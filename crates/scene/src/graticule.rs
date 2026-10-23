// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Координатная сетка: параллели и меридианы с подписями. Перенос
//! `core/graticule.py` модуля.
//!
//! Как «Вид - Сетка» Google Earth. Шаг сетки подбирается по ширине
//! видимой полосы, чтобы поперёк вида шло около [`LINES_ACROSS`] линий:
//! из космоса через 30°, у земли через минуты и секунды. Крупная сетка
//! строится на весь шар, мелкая - в окне вокруг точки взгляда.
//! Параллель не дуга большого круга, поэтому её вершины идут не реже
//! [`SEGMENT`] градусов.

pub const M_PER_DEGREE: f64 = 111_320.0;
/// Линий поперёк видимой полосы, не меньше.
pub const LINES_ACROSS: f64 = 8.0;
/// Шаги сетки в градусах, от крупного к мелкому.
pub const STEPS: [f64; 18] = [
    30.0,
    20.0,
    10.0,
    5.0,
    2.0,
    1.0,
    0.5,
    0.25,
    10.0 / 60.0,
    5.0 / 60.0,
    2.0 / 60.0,
    1.0 / 60.0,
    30.0 / 3600.0,
    15.0 / 3600.0,
    10.0 / 3600.0,
    5.0 / 3600.0,
    2.0 / 3600.0,
    1.0 / 3600.0,
];
/// С этого шага и крупнее сетка строится на весь шар.
pub const GLOBAL_STEP: f64 = 5.0;
/// Окно мелкой сетки - столько ширин видимой полосы.
pub const WINDOW: f64 = 3.0;
/// Градусов между вершинами параллели, не больше.
pub const SEGMENT: f64 = 2.0;
/// Сетка кончается там же, где тайлы.
pub const MAX_LAT: f64 = 85.0;
/// Наклон оси Земли на 2026 год - 23°26′10″, по IAU 2006.
pub const TILT: f64 = 23.4362;
/// Особые параллели, как в Google Earth: экватор, тропики и полярные круги.
pub const CIRCLES: [(&str, f64); 5] =
    [("equator", 0.0), ("cancer", TILT), ("capricorn", -TILT), ("arctic", 90.0 - TILT), ("antarctic", TILT - 90.0)];

/// Линия сетки: параллель или меридиан.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Lat,
    Lon,
}

/// Шаг сетки в градусах для видимой полосы шириной width метров.
pub fn step_for(width: f64) -> f64 {
    let wanted = width / LINES_ACROSS / M_PER_DEGREE;
    STEPS.iter().copied().filter(|s| *s >= wanted).fold(f64::INFINITY, f64::min).min(STEPS[0])
}

fn range(low: f64, high: f64, step: f64) -> Vec<f64> {
    let first = (low / step - 1e-9).ceil() as i64;
    let last = (high / step + 1e-9).floor() as i64;
    (first..=last).map(|n| n as f64 * step).collect()
}

/// Окно сетки (юг, север, запад, восток) в градусах. Крупный шаг - весь
/// шар, мелкий - [`WINDOW`] ширин вокруг точки взгляда.
pub fn extent(lat: f64, lon: f64, width: f64, step: f64) -> (f64, f64, f64, f64) {
    if step >= GLOBAL_STEP {
        return (-MAX_LAT, MAX_LAT, -180.0, 180.0);
    }
    let half = WINDOW * width / 2.0 / M_PER_DEGREE;
    let cos_lat = lat.to_radians().cos().max(0.05);
    let south = (lat - half).max(-MAX_LAT);
    let north = (lat + half).min(MAX_LAT);
    let (west, east) = (lon - half / cos_lat, lon + half / cos_lat);
    if east - west >= 360.0 { (south, north, -180.0, 180.0) } else { (south, north, west, east) }
}

fn lons(west: f64, east: f64, seg: f64) -> Vec<f64> {
    let mut out = range(west, east, seg);
    if out.first().is_none_or(|f| *f > west) {
        out.insert(0, west);
    }
    if out.last().is_some_and(|l| *l < east) {
        out.push(east);
    }
    out
}

/// Линия сетки: ось, значение, вершины (широта, долгота).
pub type Line = (Axis, f64, Vec<(f64, f64)>);

/// Шаг и линии сетки.
pub fn lines(lat: f64, lon: f64, width: f64) -> (f64, Vec<Line>) {
    let step = step_for(width);
    let (south, north, west, east) = extent(lat, lon, width, step);
    let mut out = Vec::new();
    let seg = step.min(SEGMENT);
    let along = lons(west, east, seg);
    // Экватор рисуется особой параллелью, см. circles.
    for value in range(south, north, step).into_iter().filter(|v| v.abs() <= MAX_LAT && v.abs() > 1e-9) {
        out.push((Axis::Lat, value, along.iter().map(|x| (value, *x)).collect()));
    }
    let mut lats = range(south, north, seg);
    if lats.first().is_none_or(|f| *f > south) {
        lats.insert(0, south);
    }
    if lats.last().is_some_and(|l| *l < north) {
        lats.push(north);
    }
    // На весь шар меридиан 180° совпадает с -180°, он идёт один раз.
    let stop = if east - west < 360.0 { east } else { east - step / 2.0 };
    for value in range(west, stop, step) {
        out.push((Axis::Lon, value, lats.iter().map(|y| (*y, value)).collect()));
    }
    (step, out)
}

/// Особая параллель: имя, широта, вершины.
pub type Circle = (&'static str, f64, Vec<(f64, f64)>);

/// Особые параллели в окне сетки.
pub fn circles(lat: f64, lon: f64, width: f64) -> Vec<Circle> {
    let step = step_for(width);
    let (south, north, west, east) = extent(lat, lon, width, step);
    let along = lons(west, east, step.min(SEGMENT));
    CIRCLES
        .iter()
        .filter(|(_, v)| south <= *v && *v <= north)
        .map(|(name, v)| (*name, *v, along.iter().map(|x| (*v, *x)).collect()))
        .collect()
}

/// Подписи особых параллелей: (широта, долгота, имя), между
/// меридианами возле точки взгляда.
pub fn circle_labels(lat: f64, lon: f64, width: f64) -> Vec<(f64, f64, &'static str)> {
    let step = step_for(width);
    let at_lon = ((lon / step).round() + 0.5) * step;
    circles(lat, lon, width).into_iter().map(|(name, v, _)| (v, at_lon, name)).collect()
}

/// Подписи: (широта, долгота, ось, значение). Параллели подписаны на
/// меридиане возле точки взгляда, меридианы - на параллели.
pub fn labels(lat: f64, lon: f64, width: f64) -> Vec<(f64, f64, Axis, f64)> {
    let step = step_for(width);
    let (south, north, west, east) = extent(lat, lon, width, step);
    let at_lon = (lon / step).round() * step;
    let at_lat = ((lat / step).round() * step).clamp(-MAX_LAT, MAX_LAT);
    let mut out: Vec<(f64, f64, Axis, f64)> =
        range(south, north, step).into_iter().filter(|v| v.abs() <= MAX_LAT && v.abs() > 1e-9).map(|v| (v, at_lon, Axis::Lat, v)).collect();
    let stop = if east - west < 360.0 { east } else { east - step / 2.0 };
    out.extend(range(west, stop, step).into_iter().map(|v| (at_lat, v, Axis::Lon, v)));
    out
}

/// Ключ перестройки: сетка та же, пока ключ не меняется.
pub fn key(lat: f64, lon: f64, width: f64) -> (u64, i64, i64) {
    let step = step_for(width);
    if step >= GLOBAL_STEP {
        return (step.to_bits(), 0, 0);
    }
    (step.to_bits(), (lat / step).round() as i64, (lon / step).round() as i64)
}

/// Угол без знака: градусы, минуты и секунды по нужде шага. Полушарие
/// подписывает окно, буквы зависят от языка.
pub fn angle_text(value: f64, step: f64) -> String {
    let seconds = (value.abs() * 3600.0).round() as i64;
    let (d, rest) = (seconds / 3600, seconds % 3600);
    let (m, s) = (rest / 60, rest % 60);
    if step >= 1.0 {
        format!("{d}°")
    } else if step >= 1.0 / 60.0 - 1e-12 {
        format!("{d}°{m:02}′")
    } else {
        format!("{d}°{m:02}′{s:02}″")
    }
}

/// Долгота в пределах от -180 до 180.
pub fn normal_lon(value: f64) -> f64 {
    (value + 180.0).rem_euclid(360.0) - 180.0
}
