// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Границы муниципальных образований России СберИндекса: разбор файла
//! слоя, линии границ и муниципалитет у точки.
//!
//! Файл собирает `tools/build_munis.py` из «Версионного справочника
//! границ и преобразований муниципальных образований России» СберИндекса
//! (CC BY-SA 4.0). Данные лежат в хранилище planetx-terrain под той же
//! лицензией, рядом - её текст. Файл - JSON, сжатый gzip. В `units` -
//! муниципалитеты: номер, название с типом, тип, ОКТМО, регион, центр, год
//! начала, точка подписи и кольца границы, точки подряд «широта,
//! долгота…».
//!
//! Точка внутри муниципалитета, если луч от неё на восток пересекает его
//! кольца нечётное количество раз: внешние кольца и дыры одинаково.

use std::io::Read;

use crate::linesets::Line;

/// Файл слоя в хранилище planetx-terrain.
pub const URL: &str = "https://raw.githubusercontent.com/Valery35/planetx-terrain/main/sberindex/munis.json.gz";
/// Подпись источника на экране и страница набора. Без запятой: короткая
/// подпись источников отрезает текст после неё, а лицензия должна быть
/// видна.
pub const ATTRIBUTION: (&str, &str) = ("SberIndex - CC BY-SA 4.0", "https://sberindex.ru/ru/research/russian_municipal_districts_boundaries");
/// Предел сжатого файла, байт.
pub const BYTES: u64 = 32 << 20;
/// Предел разжатого файла, байт: входной файл враждебен.
const UNPACKED: u64 = 256 << 20;
/// Цвет границ RGBA.
pub const COLOR: [u8; 4] = [200, 60, 150, 255];
/// Толщина границ, пиксели картинки тайла.
pub const WIDTH: f32 = 1.4;
/// Долгота шва 180°: отрезок по шву - разрез Чукотки, не граница.
const SEAM: f64 = 179.999;

/// Муниципальное образование.
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    pub id: String,
    /// Название с типом: «Частинский м.о.».
    pub name: String,
    /// Тип: муниципальный район, округ, городской округ.
    pub kind: String,
    pub oktmo: String,
    pub region: String,
    pub center: String,
    /// Год начала нынешних границ, 0 - нет.
    pub from: i64,
    /// Точка подписи (широта, долгота).
    pub lat: f64,
    pub lon: f64,
    /// Кольца границы, точки (широта, долгота).
    pub rings: Vec<Vec<(f64, f64)>>,
    /// Рамка колец (юг, запад, север, восток).
    pub bbox: (f64, f64, f64, f64),
}

/// Набор муниципалитетов.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Munis {
    pub units: Vec<Unit>,
}

fn text(item: &serde_json::Value, key: &str) -> String {
    match item.get(key) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// Разжать gzip с пределом. Несжатый JSON идёт как есть.
fn unpack(data: &[u8]) -> Result<Vec<u8>, String> {
    if !data.starts_with(&[0x1f, 0x8b]) {
        return Ok(data.to_vec());
    }
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(data).take(UNPACKED + 1).read_to_end(&mut out).map_err(|e| e.to_string())?;
    if out.len() as u64 > UNPACKED {
        return Err("file too large".into());
    }
    Ok(out)
}

/// Кольцо из плоского массива «широта, долгота…».
fn ring(value: &serde_json::Value) -> Vec<(f64, f64)> {
    let flat: Vec<f64> = value.as_array().into_iter().flatten().filter_map(|v| v.as_f64()).collect();
    flat.as_chunks::<2>().0.iter().map(|[lat, lon]| (*lat, *lon)).filter(|(lat, lon)| lat.abs() <= 90.0 && lon.abs() <= 180.0).collect()
}

/// Набор из байтов файла, сжатого gzip или нет.
pub fn parse(data: &[u8]) -> Result<Munis, String> {
    let bytes = unpack(data)?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let mut units = Vec::new();
    for item in json.get("units").and_then(|u| u.as_array()).into_iter().flatten() {
        let rings: Vec<Vec<(f64, f64)>> = item.get("rings").and_then(|r| r.as_array()).into_iter().flatten().map(ring).filter(|r| r.len() >= 3).collect();
        let (Some(lat), Some(lon)) = (item.get("lat").and_then(|v| v.as_f64()), item.get("lon").and_then(|v| v.as_f64())) else {
            continue;
        };
        if rings.is_empty() || !lat.is_finite() || !lon.is_finite() {
            continue;
        }
        let (mut s, mut w, mut n, mut e) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for p in rings.iter().flatten() {
            s = s.min(p.0);
            n = n.max(p.0);
            w = w.min(p.1);
            e = e.max(p.1);
        }
        units.push(Unit {
            id: text(item, "id"),
            name: text(item, "name"),
            kind: text(item, "kind"),
            oktmo: text(item, "oktmo"),
            region: text(item, "region"),
            center: text(item, "center"),
            from: item.get("from").and_then(|v| v.as_i64()).unwrap_or(0),
            lat,
            lon,
            rings,
            bbox: (s, w, n, e),
        });
    }
    if units.is_empty() {
        return Err("no units".into());
    }
    Ok(Munis { units })
}

/// Куски кольца без отрезков по шву 180°.
fn pieces(ring: &[(f64, f64)]) -> Vec<Vec<(f64, f64)>> {
    let mut out = Vec::new();
    let mut piece: Vec<(f64, f64)> = Vec::new();
    for pair in ring.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if a.1.abs() >= SEAM && b.1.abs() >= SEAM {
            if piece.len() >= 2 {
                out.push(std::mem::take(&mut piece));
            }
            piece.clear();
            continue;
        }
        if piece.is_empty() {
            piece.push(a);
        }
        piece.push(b);
    }
    if piece.len() >= 2 {
        out.push(piece);
    }
    out
}

impl Munis {
    /// Линии границ для картинки тайла.
    pub fn lines(&self) -> Vec<Line> {
        self.units.iter().flat_map(|u| u.rings.iter()).flat_map(|r| pieces(r)).map(|p| Line::new(p, COLOR, WIDTH)).collect()
    }

    /// Муниципалитет, внутри которого точка (широта, долгота).
    pub fn unit_at(&self, lat: f64, lon: f64) -> Option<&Unit> {
        self.units.iter().find(|u| {
            let (s, w, n, e) = u.bbox;
            lat >= s && lat <= n && lon >= w && lon <= e && inside(&u.rings, lat, lon)
        })
    }
}

/// Чётность пересечений луча на восток с кольцами.
fn inside(rings: &[Vec<(f64, f64)>], lat: f64, lon: f64) -> bool {
    let mut odd = false;
    for ring in rings {
        for pair in ring.windows(2) {
            let ((la, lo), (lb, lob)) = (pair[0], pair[1]);
            if (la > lat) != (lb > lat) {
                let cross = lo + (lat - la) / (lb - la) * (lob - lo);
                if cross > lon {
                    odd = !odd;
                }
            }
        }
    }
    odd
}
