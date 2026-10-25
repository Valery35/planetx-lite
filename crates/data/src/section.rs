// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Разрез вниз вдоль линии, перенос `core/section.py` модуля.
//!
//! Вдоль пути из «Моих меток» собирается вертикальный разрез Земли
//! до заданной глубины:
//! - кора CRUST1.0 по точкам линии, без модели - слой PREM до Мохо 24.4 км;
//! - оболочки PREM ниже Мохо ([`crate::cutaway::SHELLS`]);
//! - погружающиеся плиты Slab2 полосой с толщиной ([`crate::slabs::band`]);
//! - очаги землетрясений в полосе шириной width по обе стороны линии,
//!   перенесённые на разрез по ближайшей точке линии.
//!
//! Точки линии идут по дугам большого круга между вершинами, равномерно
//! по длине. Расстояния - по шару среднего радиуса Земли, на разрезах
//! в сотни и тысячи километров ошибка от сжатия меньше полупроцента.

use crate::crust::{BOUNDS, Crust};
use crate::cutaway::{self, PREM_RADIUS, SHELLS, SLAB_COLOR};
use crate::quakes::{self, Quake};
use crate::slabs::{self, Slab};
use crate::subsurface::{self, Mesh, Part};

/// Точек вдоль линии.
pub const SAMPLES: usize = 600;
/// Непрозрачность стенки на глобусе, выбор помощника модуля.
pub const WALL_ALPHA: u8 = 220;
/// Глубины низа разреза на выбор, км.
pub const DEPTHS: [f64; 5] = [100.0, 300.0, 700.0, 2891.0, PREM_RADIUS];
/// Глубина по умолчанию - низ переходной зоны, км.
pub const DEPTH: f64 = 700.0;
/// Полоса очагов по обе стороны линии вместе, км.
pub const WIDTH: f64 = 100.0;

/// Очаг на разрезе: км вдоль линии, глубина км, магнитуда, номер
/// в списке, из которого он взят.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Near {
    pub along: f64,
    pub depth: f64,
    pub mag: f64,
    pub index: usize,
}

/// Разрез. distance - км от начала линии, lats, lons - точки линии,
/// bounds - отметки границ коры CRUST1.0 в км или None, slab_top,
/// slab_bottom - плита в км глубины или NaN, quakes - свежие очаги
/// в полосе, depth - глубина низа разреза в км, oceanic - океаническая
/// ли кора по точкам или None, history - исторические очаги ISC-GEM.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub distance: Vec<f64>,
    pub lats: Vec<f64>,
    pub lons: Vec<f64>,
    pub bounds: Option<Vec<[f64; BOUNDS]>>,
    pub slab_top: Vec<f64>,
    pub slab_bottom: Vec<f64>,
    pub quakes: Vec<Near>,
    pub depth: f64,
    pub oceanic: Option<Vec<bool>>,
    pub history: Vec<Near>,
}

/// Оболочка ниже Мохо на разрезе: ключ, верх и низ в км глубины, цвет.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layer {
    pub key: &'static str,
    pub top: f64,
    pub bottom: f64,
    pub color: [u8; 3],
}

fn unit(lat: f64, lon: f64) -> [f64; 3] {
    let (lat, lon) = (lat.to_radians(), lon.to_radians());
    [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Средний радиус Земли, км: (2a + b) / 3.
pub fn radius_km() -> f64 {
    (2.0 * subsurface::A + subsurface::B) / 3.0 / 1000.0
}

/// Точки вдоль ломаной points [(широта, долгота)] по дугам большого
/// круга: (км от начала, широты, долготы). Шаг одинаковый по длине.
/// None - точек меньше двух, длина нулевая или count меньше двух.
pub fn along(points: &[(f64, f64)], count: usize) -> Option<(Vec<f64>, Vec<f64>, Vec<f64>)> {
    if points.len() < 2 || count < 2 {
        return None;
    }
    let units: Vec<[f64; 3]> = points.iter().map(|p| unit(p.0, p.1)).collect();
    let angles: Vec<f64> = units.windows(2).map(|w| dot(w[0], w[1]).clamp(-1.0, 1.0).acos()).collect();
    let total: f64 = angles.iter().sum();
    if total.is_nan() || total <= 0.0 {
        return None;
    }
    let mut marks = vec![0.0];
    for a in &angles {
        marks.push(marks[marks.len() - 1] + a);
    }
    let last = angles.len() - 1;
    let (mut distance, mut lats, mut lons) = (Vec::with_capacity(count), Vec::with_capacity(count), Vec::with_capacity(count));
    for i in 0..count {
        let want = total * i as f64 / (count - 1) as f64;
        // Отрезок, на который попала точка, как searchsorted справа.
        let k = marks.partition_point(|m| *m <= want).saturating_sub(1).min(last);
        let (a, b, omega) = (units[k], units[k + 1], angles[k]);
        let p = if omega < 1e-12 {
            a
        } else {
            let t = (want - marks[k]) / omega;
            let (sa, sb) = (((1.0 - t) * omega).sin() / omega.sin(), (t * omega).sin() / omega.sin());
            [sa * a[0] + sb * b[0], sa * a[1] + sb * b[1], sa * a[2] + sb * b[2]]
        };
        distance.push(want * radius_km());
        lats.push(p[2].clamp(-1.0, 1.0).asin().to_degrees());
        lons.push(p[1].atan2(p[0]).to_degrees());
    }
    Some((distance, lats, lons))
}

/// Очаги в промежутке шкалы времени span = (от, до) в секундах UTC или
/// все при span None - тот же отбор, что у очагов на глобусе. Очаг без
/// времени остаётся.
pub fn in_time(quakes: &[Quake], span: Option<(f64, f64)>) -> Vec<&Quake> {
    quakes.iter().filter(|q| quakes::in_window(q.time, span)).collect()
}

/// Очаги в полосе ширины width км вдоль точек линии. Ближайшая точка
/// линии ищется среди точек разреза, их шаг меньше ширины полосы на
/// любой разумной линии. Очаг за концом линии не берётся: его ближайшая
/// точка - конец, но он лежит дальше конца на расстояние больше полосы.
pub fn near(quakes: &[&Quake], lats: &[f64], lons: &[f64], distance: &[f64], width: f64) -> Vec<Near> {
    let line: Vec<[f64; 3]> = lats.iter().zip(lons).map(|(la, lo)| unit(*la, *lo)).collect();
    let radius = radius_km();
    let mut out = Vec::new();
    for (index, q) in quakes.iter().enumerate() {
        let e = unit(q.lat, q.lon);
        let Some((i, best)) = line.iter().map(|p| dot(e, *p)).enumerate().fold(None, |acc: Option<(usize, f64)>, (i, d)| match acc {
            Some((_, b)) if b >= d => acc,
            _ => Some((i, d)),
        }) else {
            break;
        };
        let gap = best.clamp(-1.0, 1.0).acos() * radius;
        if gap <= width / 2.0
            && let Some(along) = distance.get(i)
        {
            out.push(Near { along: *along, depth: q.depth, mag: q.mag, index });
        }
    }
    out
}

/// Разрез вдоль points или None, если точек меньше двух. history -
/// исторические очаги, они идут в полосе отдельным списком.
#[allow(clippy::too_many_arguments)]
pub fn build(points: &[(f64, f64)], crust: Option<&Crust>, zones: &[&Slab], quakes: &[&Quake], depth: f64, width: f64, count: usize, history: &[&Quake]) -> Option<Section> {
    let (distance, lats, lons) = along(points, count)?;
    let pairs = || lats.iter().zip(&lons);
    let bounds = crust.map(|c| pairs().map(|(la, lo)| c.at(*la, *lo)).collect());
    let oceanic = crust.and_then(|c| pairs().map(|(la, lo)| c.oceanic(*la, *lo)).collect());
    let (slab_top, slab_bottom) = if zones.is_empty() {
        (vec![f64::NAN; lats.len()], vec![f64::NAN; lats.len()])
    } else {
        slabs::band(zones.iter().copied(), &lats, &lons)
    };
    let quakes = near(quakes, &lats, &lons, &distance, width);
    let history = near(history, &lats, &lons, &distance, width);
    Some(Section { distance, lats, lons, bounds, slab_top, slab_bottom, quakes, depth, oceanic, history })
}

/// Глубина Мохо вдоль линии, км: по CRUST1.0 или 24.4 км PREM.
pub fn moho(section: &Section) -> Vec<f64> {
    match &section.bounds {
        Some(b) => b.iter().map(|row| -row[BOUNDS - 1]).collect(),
        None => vec![PREM_RADIUS - cutaway::crust_low(); section.distance.len()],
    }
}

/// Цвет коры PREM.
pub fn crust_shell_color() -> [u8; 3] {
    SHELLS.iter().find(|s| s.key == "crust").map_or([120, 95, 75], |s| s.color)
}

/// Оболочки PREM ниже Мохо в пределах глубины разреза, сверху вниз.
/// У верхней мантии верх - Мохо, его даёт [`moho`].
pub fn shells_below(section: &Section) -> Vec<Layer> {
    SHELLS
        .iter()
        .rev()
        .filter(|s| s.key != "crust")
        .map(|s| (s, PREM_RADIUS - s.high, PREM_RADIUS - s.low))
        .filter(|(_, top, _)| *top < section.depth)
        .map(|(s, top, bottom)| Layer { key: s.key, top, bottom: bottom.min(section.depth), color: s.color })
        .collect()
}

/// Стенка разреза на глобусе: вертикальные полосы слоёв вдоль линии
/// от поверхности до глубины разреза или None. Глубины настоящие,
/// в метрах ниже уровня моря. Слой без толщины в точке пропускается.
pub fn wall_mesh(section: &Section, alpha: u8) -> Option<Mesh> {
    let n = section.distance.len();
    let limit = section.depth;
    let (lats, lons) = (&section.lats, &section.lons);
    // Глубины км вниз - высоты м, обрезка по глубине разреза. Пустое
    // место - нулевая толщина, такую полосу wall пропускает.
    let part = |top: &[f64], bottom: &[f64], color: [u8; 3]| {
        let mut tops = Vec::with_capacity(n);
        let mut bottoms = Vec::with_capacity(n);
        for (t, b) in top.iter().zip(bottom) {
            let (t, b) = (t.min(limit), b.min(limit));
            let empty = !(t.is_finite() && b.is_finite());
            tops.push(if empty { 0.0 } else { -t * 1000.0 });
            bottoms.push(if empty { 0.0 } else { -b * 1000.0 });
        }
        subsurface::wall(lats, lons, &tops, &bottoms, [color[0], color[1], color[2], alpha])
    };
    let mut parts: Vec<Part> = Vec::new();
    let base = moho(section);
    for layer in shells_below(section) {
        let upper = if layer.key == "upper_mantle" { base.clone() } else { vec![layer.top; n] };
        parts.extend(part(&upper, &vec![layer.bottom; n], layer.color));
    }
    match &section.bounds {
        Some(bounds) => {
            for (k, key) in cutaway::crust_layers().iter().enumerate() {
                let top: Vec<f64> = bounds.iter().map(|b| -b[k]).collect();
                let bottom: Vec<f64> = bounds.iter().map(|b| -b[k + 1]).collect();
                let colors = cutaway::layer_colors(key, section.oceanic.as_deref());
                let Some(first) = colors.first().copied() else {
                    continue;
                };
                if let Some(mut piece) = part(&top, &bottom, first) {
                    if colors.len() == n {
                        // Цвет по точкам: континентальная и океаническая кора.
                        for (i, c) in piece.colors.iter_mut().enumerate() {
                            let rgb = colors[i % n];
                            *c = [rgb[0], rgb[1], rgb[2], alpha];
                        }
                    }
                    parts.push(piece);
                }
            }
        }
        None => parts.extend(part(&vec![0.0; n], &base, crust_shell_color())),
    }
    parts.extend(part(&section.slab_top, &section.slab_bottom, SLAB_COLOR));
    subsurface::merge(&parts)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn quake(lat: f64, lon: f64, depth: f64, mag: f64, time: Option<f64>) -> Quake {
        Quake { lat, lon, depth, mag, time, place: String::new(), url: String::new() }
    }

    const LINE: [(f64, f64); 2] = [(0.0, 0.0), (0.0, 10.0)];

    #[test]
    fn along_equator() {
        let (distance, lats, lons) = along(&LINE, 11).unwrap();
        assert!(lats.iter().all(|l| l.abs() < 1e-9));
        assert!(lons.iter().enumerate().all(|(i, l)| (l - i as f64).abs() < 1e-9));
        // 10° дуги - около 1112 км.
        assert!((distance[10] - 1111.95).abs() < 2.0);
        assert!(along(&LINE[..1], SAMPLES).is_none());
    }

    #[test]
    fn along_two_legs_even_step() {
        let (distance, lats, lons) = along(&[(0.0, 0.0), (0.0, 5.0), (5.0, 5.0)], 101).unwrap();
        let step = distance[1] - distance[0];
        assert!(distance.windows(2).all(|w| ((w[1] - w[0]) / step - 1.0).abs() < 1e-9));
        assert!(lats[50].abs() < 1e-6);
        assert!((lons[100] - 5.0).abs() < 1e-6);
    }

    #[test]
    fn quakes_in_band() {
        let events = [quake(0.3, 5.0, 50.0, 6.0, None), quake(2.0, 5.0, 10.0, 5.0, None), quake(0.0, 20.0, 10.0, 5.0, None)];
        let refs: Vec<&Quake> = events.iter().collect();
        let s = build(&LINE, None, &[], &refs, DEPTH, 100.0, SAMPLES, &[]).unwrap();
        assert_eq!(s.quakes.len(), 1);
        let q = s.quakes[0];
        assert!((q.along - 556.0).abs() < 5.0);
        assert_eq!((q.depth, q.mag, q.index), (50.0, 6.0, 0));
    }

    #[test]
    fn history_in_band() {
        // Исторические очаги - своим списком.
        let mut events: Vec<Quake> = (0..10000).map(|k| quake(0.1, 1.0 + 8.0 * f64::from(k) / 9999.0, 30.0, 6.0, None)).collect();
        events.push(quake(3.0, 5.0, 30.0, 6.0, None));
        let refs: Vec<&Quake> = events.iter().collect();
        let s = build(&LINE, None, &[], &[], DEPTH, 100.0, SAMPLES, &refs).unwrap();
        assert_eq!(s.history.len(), 10000);
        assert!(s.quakes.is_empty());
        assert!(build(&LINE, None, &[], &[], DEPTH, WIDTH, SAMPLES, &[]).unwrap().history.is_empty());
    }

    #[test]
    fn in_time_follows_time_slider() {
        let events = [quake(0.0, 1.0, 10.0, 5.0, Some(100.0)), quake(0.0, 2.0, 10.0, 5.0, Some(500.0)), quake(0.0, 3.0, 10.0, 5.0, None)];
        assert_eq!(in_time(&events, None).len(), 3);
        // Очаг без времени остаётся, как на глобусе.
        let kept: Vec<f64> = in_time(&events, Some((0.0, 200.0))).iter().map(|q| q.lon).collect();
        assert_eq!(kept, [1.0, 3.0]);
        assert!(in_time(&[], Some((0.0, 1.0))).is_empty());
    }

    #[test]
    fn shells_cut_at_depth() {
        let s = build(&LINE, None, &[], &[], 700.0, WIDTH, SAMPLES, &[]).unwrap();
        let layers = shells_below(&s);
        let keys: Vec<&str> = layers.iter().map(|l| l.key).collect();
        assert_eq!(keys, ["upper_mantle", "transition_zone", "lower_mantle"]);
        assert_eq!(layers[2].bottom, 700.0);
        assert!(moho(&s).iter().all(|m| (m - 24.4).abs() < 1e-9));
    }

    fn heights(mesh: &Mesh) -> Vec<f64> {
        mesh.vertices
            .iter()
            .map(|v| {
                let p = [f64::from(v.position[0]) + mesh.center[0], f64::from(v.position[1]) + mesh.center[1], f64::from(v.position[2]) + mesh.center[2]];
                subsurface::ecef_to_geodetic(p)[2]
            })
            .collect()
    }

    #[test]
    fn wall_reaches_section_depth() {
        let s = build(&LINE, None, &[], &[], 700.0, WIDTH, 21, &[]).unwrap();
        let mesh = wall_mesh(&s, WALL_ALPHA).unwrap();
        let h = heights(&mesh);
        assert!(h.iter().all(|x| x.is_finite()));
        let (low, high) = h.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), x| (a.min(*x), b.max(*x)));
        assert!((low + 700000.0).abs() < 50.0);
        assert!(high.abs() < 50.0);
        assert!(mesh.vertices.iter().all(|v| v.color[3] == WALL_ALPHA));
    }

    #[test]
    fn wall_skips_missing_slab() {
        let plain = build(&LINE, None, &[], &[], 100.0, WIDTH, 21, &[]).unwrap();
        let mut slab = plain.clone();
        slab.slab_top = (0..21).map(|i| if i < 5 { 30.0 } else { f64::NAN }).collect();
        slab.slab_bottom = (0..21).map(|i| if i < 5 { 80.0 } else { f64::NAN }).collect();
        let (a, b) = (wall_mesh(&plain, WALL_ALPHA).unwrap(), wall_mesh(&slab, WALL_ALPHA).unwrap());
        assert!(b.indices.len() > a.indices.len());
        assert!(heights(&b).iter().all(|x| x.is_finite()));
    }
}
