// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Аддин-пример PlanetX Lite: буфер вокруг объекта - команда ABI 2.
//!
//! Объект переводится в метры местной азимутальной равнопромежуточной
//! проекцией на шаре с центром в объекте. Расстояния от центра в ней
//! верны, поэтому буфер верен и у полюса, и через линию перемены дат.
//! Буфер строит крейт geo, круглые углы и концы - по дуге через 0.1
//! радиана. Затем вершины возвращаются в широту и долготу.
//!
//! Единственное место `unsafe` - атрибут `#[unsafe(no_mangle)]`
//! у экспортов: без него функцию не экспортировать из модуля
//! WebAssembly. Сам код аддина - безопасный Rust.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::todo)]

use std::sync::{Mutex, MutexGuard};

use addin_api::{ABI_VERSION, Feature, Kind, decode_request, encode_features};
use geo::algorithm::buffer::{Buffer, BufferStyle, LineCap, LineJoin};
use geo::{Coord, LineString, MultiLineString, MultiPoint, MultiPolygon, Point, Polygon};

/// Средний радиус Земли, м (IUGG).
const R: f64 = 6_371_008.8;
/// Наибольшее расстояние буфера, м: дальше местная проекция искажает
/// форму заметно.
pub const MAX_DISTANCE: f64 = 1_000_000.0;
/// Наибольшее количество вершин входа.
const MAX_VERTICES: usize = 500_000;
/// Шаг дуги углов и концов буфера, радиан.
const ARC: f64 = 0.1;
/// Наибольший вход, байт.
const MAX_INPUT: usize = 64 << 20;

static INPUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());
static OUTPUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// Описание аддина - тот же файл `addin.json`, что лежит в пакете.
pub const MANIFEST: &str = include_str!("../addin.json");

fn lock(m: &Mutex<Vec<u8>>) -> MutexGuard<'_, Vec<u8>> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Положить в выход, ответ - длина.
fn put(bytes: Vec<u8>) -> i32 {
    let len = i32::try_from(bytes.len()).unwrap_or(i32::MAX);
    *lock(&OUTPUT) = bytes;
    len
}

#[unsafe(no_mangle)]
pub extern "C" fn pxa_abi() -> i32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn pxa_manifest() -> i32 {
    put(MANIFEST.as_bytes().to_vec())
}

#[unsafe(no_mangle)]
pub extern "C" fn pxa_input(len: i32) -> i32 {
    let Ok(n) = usize::try_from(len) else {
        return 0;
    };
    if n > MAX_INPUT {
        return 0;
    }
    let mut input = lock(&INPUT);
    *input = vec![0; n.max(1)];
    i32::try_from(input.as_ptr() as usize).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn pxa_output() -> i32 {
    i32::try_from(lock(&OUTPUT).as_ptr() as usize).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn pxa_command(len: i32) -> i32 {
    let input = lock(&INPUT);
    let n = usize::try_from(len).unwrap_or(0).min(input.len());
    let result = decode_request(input.get(..n).unwrap_or(&[])).and_then(|(c, p, f)| run(&c, &p, &f));
    drop(input);
    match result {
        Ok(features) => put(encode_features(&features)),
        Err(e) => -put(e.into_bytes()).max(1),
    }
}

/// Выполнить команду: имя, параметры JSON, объекты.
pub fn run(command: &str, params: &str, features: &[Feature]) -> Result<Vec<Feature>, String> {
    if command != "buffer" {
        return Err(format!("нет команды {command}"));
    }
    let p: serde_json::Value = serde_json::from_str(params).map_err(|e| format!("параметры не JSON: {e}"))?;
    let d = p.get("distance").and_then(serde_json::Value::as_f64).ok_or("нет расстояния")?;
    if !d.is_finite() || d.abs() < 1e-3 || d.abs() > MAX_DISTANCE {
        return Err(format!("расстояние должно быть от 0.001 до {MAX_DISTANCE} м по модулю"));
    }
    let vertices: usize = features.iter().flat_map(|f| f.parts.iter()).map(Vec::len).sum();
    if vertices > MAX_VERTICES {
        return Err(format!("вершин больше {MAX_VERTICES}"));
    }
    let mut out = Vec::new();
    for f in features {
        if d < 0.0 && f.kind != Kind::Polygon {
            return Err("внутренний буфер - только у полигона".into());
        }
        out.extend(buffer(f, d));
    }
    Ok(out)
}

/// Местная азимутальная равнопромежуточная проекция на шаре.
struct Aeqd {
    lat0: f64,
    lon0: f64,
}

impl Aeqd {
    /// Центр объекта: средняя широта и средняя долгота, долготы
    /// развёрнуты от первой вершины, чтобы объект через линию перемены
    /// дат не дал центр на другой стороне Земли.
    fn around(f: &Feature) -> Option<Aeqd> {
        let first = f.parts.iter().flatten().next()?;
        let (mut sl, mut so, mut n) = (0.0, 0.0, 0.0);
        for v in f.parts.iter().flatten() {
            sl += v[0];
            so += first[1] + wrap(v[1] - first[1]);
            n += 1.0;
        }
        Some(Aeqd { lat0: (sl / n).to_radians(), lon0: wrap(so / n).to_radians() })
    }

    fn forward(&self, lat: f64, lon: f64) -> Coord<f64> {
        let (p, l) = (lat.to_radians(), lon.to_radians() - self.lon0);
        let cos_c = (self.lat0.sin() * p.sin() + self.lat0.cos() * p.cos() * l.cos()).clamp(-1.0, 1.0);
        let c = cos_c.acos();
        let az = (l.sin() * p.cos()).atan2(self.lat0.cos() * p.sin() - self.lat0.sin() * p.cos() * l.cos());
        Coord { x: R * c * az.sin(), y: R * c * az.cos() }
    }

    /// Обратно в широту и долготу. Долгота - от центра, без скачка через
    /// ±180, чтобы кольцо осталось непрерывным.
    fn inverse(&self, c: Coord<f64>) -> [f64; 3] {
        let dist = c.x.hypot(c.y) / R;
        let az = c.x.atan2(c.y);
        let lat = (self.lat0.sin() * dist.cos() + self.lat0.cos() * dist.sin() * az.cos()).clamp(-1.0, 1.0).asin();
        let dl = (az.sin() * dist.sin() * self.lat0.cos()).atan2(dist.cos() - self.lat0.sin() * lat.sin());
        [lat.to_degrees(), (self.lon0 + dl).to_degrees(), 0.0]
    }
}

/// Угол в пределы -180..180.
fn wrap(a: f64) -> f64 {
    (a + 180.0).rem_euclid(360.0) - 180.0
}

fn line(a: &Aeqd, part: &[[f64; 3]]) -> LineString<f64> {
    LineString(part.iter().map(|v| a.forward(v[0], v[1])).collect())
}

/// Буфер одного объекта: полигоны результата.
fn buffer(f: &Feature, d: f64) -> Vec<Feature> {
    let Some(a) = Aeqd::around(f) else {
        return Vec::new();
    };
    let style = || BufferStyle::new(d).line_join(LineJoin::Round(ARC)).line_cap(LineCap::Round(ARC));
    let mp: MultiPolygon<f64> = match f.kind {
        Kind::Point => {
            let pts: Vec<Point<f64>> = f.parts.iter().flatten().map(|v| Point(a.forward(v[0], v[1]))).collect();
            MultiPoint(pts).buffer_with_style(style())
        }
        Kind::Line => MultiLineString(f.parts.iter().filter(|p| !p.is_empty()).map(|p| line(&a, p)).collect()).buffer_with_style(style()),
        Kind::Polygon => {
            let mut rings = f.parts.iter().filter(|p| p.len() >= 3).map(|p| line(&a, p));
            let Some(outer) = rings.next() else {
                return Vec::new();
            };
            Polygon::new(outer, rings.collect()).buffer_with_style(style())
        }
    };
    mp.0.iter()
        .map(|poly| {
            let ring = |r: &LineString<f64>| {
                let mut v: Vec<[f64; 3]> = r.0.iter().map(|c| a.inverse(*c)).collect();
                // Кольцо geo замкнуто, у объекта Lite - нет.
                if v.len() > 1 && v.first() == v.last() {
                    v.pop();
                }
                v
            };
            let mut parts = vec![ring(poly.exterior())];
            parts.extend(poly.interiors().iter().map(ring));
            Feature { kind: Kind::Polygon, parts, name: f.name.clone() }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{Feature, Kind, R, run};

    /// Площадь кольца, м², по формуле Гаусса в местной проекции с центром
    /// у первой вершины. Для колец в несколько километров искажение
    /// площади проекции меньше 10⁻⁶.
    fn area(ring: &[[f64; 3]]) -> f64 {
        let a = super::Aeqd { lat0: ring[0][0].to_radians(), lon0: ring[0][1].to_radians() };
        let p: Vec<_> = ring.iter().map(|v| a.forward(v[0], v[1])).collect();
        let mut s = 0.0;
        for i in 0..p.len() {
            let (u, v) = (p[i], p[(i + 1) % p.len()]);
            s += u.x * v.y - v.x * u.y;
        }
        (s / 2.0).abs()
    }

    fn one(kind: Kind, parts: Vec<Vec<[f64; 3]>>, d: f64) -> Vec<Feature> {
        run("buffer", &format!(r#"{{"distance":{d}}}"#), &[Feature { kind, parts, name: "x".into() }]).unwrap()
    }

    #[test]
    fn point_buffer_is_a_circle() {
        for (lat, lon) in [(58.0, 56.2), (89.99, 0.0), (0.0, 179.999), (-45.0, -70.0)] {
            let out = one(Kind::Point, vec![vec![[lat, lon, 0.0]]], 1000.0);
            assert_eq!(out.len(), 1);
            let a = area(&out[0].parts[0]);
            let want = std::f64::consts::PI * 1000.0f64.powi(2);
            assert!((a / want - 1.0).abs() < 0.01, "{lat},{lon}: {a} vs {want}");
        }
    }

    #[test]
    fn segment_buffer_is_a_stadium() {
        // Отрезок 10 км по экватору, буфер 500 м: прямоугольник и два
        // полукруга.
        let len = 10_000.0 / R;
        let out = one(Kind::Line, vec![vec![[0.0, 0.0, 0.0], [0.0, len.to_degrees(), 0.0]]], 500.0);
        let want = 10_000.0 * 1000.0 + std::f64::consts::PI * 500.0f64.powi(2);
        let a = area(&out[0].parts[0]);
        assert!((a / want - 1.0).abs() < 0.01, "{a} vs {want}");
    }

    #[test]
    fn polygon_inward_and_errors() {
        // Квадрат 2x2 км, внутрь на 500 м - квадрат 1x1 км.
        let k = 1000.0 / R;
        let sq = vec![vec![[-k.to_degrees(), -k.to_degrees(), 0.0], [-k.to_degrees(), k.to_degrees(), 0.0], [k.to_degrees(), k.to_degrees(), 0.0], [k.to_degrees(), -k.to_degrees(), 0.0]]];
        let out = one(Kind::Polygon, sq, -500.0);
        let a = area(&out[0].parts[0]);
        assert!((a / 1e6 - 1.0).abs() < 0.01, "{a}");
        let p = Feature { kind: Kind::Point, parts: vec![vec![[0.0, 0.0, 0.0]]], name: String::new() };
        assert!(run("buffer", r#"{"distance":-5}"#, std::slice::from_ref(&p)).is_err());
        assert!(run("buffer", r#"{"distance":0}"#, std::slice::from_ref(&p)).is_err());
        assert!(run("buffer", r#"{"distance":5e7}"#, std::slice::from_ref(&p)).is_err());
        assert!(run("other", r#"{"distance":5}"#, std::slice::from_ref(&p)).is_err());
        assert!(run("buffer", "x", std::slice::from_ref(&p)).is_err());
    }
}
