// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Линейка: единицы, запись величин, точка по азимуту, профиль высот
//! и длина по рельефу. Перенос `core/measure.py` модуля.
//!
//! Длину, площадь и окружность на эллипсоиде WGS84 модуль считает через
//! QgsDistanceArea, а она - через GeographicLib. В Lite их считает
//! [`crate::geodesic`].
//!
//! Профиль и длина по рельефу считаются по точкам вдоль линии. Линия
//! сгущается по дуге большого круга с шагом в пиксель самого точного
//! уровня высот, но не больше [`PROFILE_POINTS`] точек. Длина по
//! рельефу - сумма отрезков между соседними точками на их высотах.

use globe_core::ellipsoid::{EARTH, Ellipsoid};
use globe_core::tiling::lonlat_to_tile;

use crate::features::densify;

pub const CIRCLE_POINTS: usize = 128;
/// Точек вдоль линии, не больше.
pub const PROFILE_POINTS: usize = 2000;
/// Уклон меряется на отрезке не короче стольких пикселей тайла высот:
/// между соседними точками через пиксель шероховатость данных давала на
/// Эльбрусе наибольший уклон 180 %, 30 сентября 2026 года.
pub const SLOPE_PIXELS: f64 = 3.0;
pub const TILE_PIXELS: f64 = 256.0;
/// Самый точный уровень тайлов высот, `terrain::MAX_LEVEL`.
pub const HEIGHT_LEVEL: u32 = 15;

/// Единицы длины: код и множитель от метров.
pub const LENGTH_UNITS: [(&str, f64); 4] = [("m", 1.0), ("km", 1000.0), ("mi", 1609.344), ("nmi", 1852.0)];
/// Единицы площади: код и множитель от квадратных метров.
pub const AREA_UNITS: [(&str, f64); 4] = [("m2", 1.0), ("ha", 1.0e4), ("km2", 1.0e6), ("mi2", 1609.344 * 1609.344)];

/// Точка на расстоянии distance метров по азимуту bearing, сфера
/// радиуса большой полуоси.
///
/// Для проверок и грубых построений. Точная окружность круга строится
/// на эллипсоиде ([`crate::geodesic`]).
pub fn destination(lat: f64, lon: f64, bearing: f64, distance: f64) -> (f64, f64) {
    let phi = lat.to_radians();
    let lam = lon.to_radians();
    let theta = bearing.to_radians();
    let delta = distance / EARTH.a;
    let sin_phi = phi.sin() * delta.cos() + phi.cos() * delta.sin() * theta.cos();
    let phi2 = sin_phi.clamp(-1.0, 1.0).asin();
    let lam2 = lam + (theta.sin() * delta.sin() * phi.cos()).atan2(delta.cos() - phi.sin() * sin_phi);
    let lon2 = (lam2.to_degrees() + 180.0).rem_euclid(360.0) - 180.0;
    (phi2.to_degrees(), lon2)
}

/// Величина в метрах или квадратных метрах в единицах unit из units.
/// Неизвестная единица - без перевода.
pub fn convert(value: f64, unit: &str, units: &[(&str, f64)]) -> f64 {
    value / units.iter().find(|(code, _)| *code == unit).map_or(1.0, |(_, k)| *k)
}

/// Число для окна. От 1 до 1000 - две цифры после точки. От 1000 -
/// одна цифра, тысячи разделены пробелом. Меньше 1 - три значащие цифры.
pub fn number(value: f64) -> String {
    if value >= 1000.0 {
        let text = format!("{value:.1}");
        let (whole, fraction) = text.split_once('.').unwrap_or((&text, "0"));
        let digits: Vec<char> = whole.chars().collect();
        let mut grouped = String::new();
        for (i, c) in digits.iter().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                grouped.push(' ');
            }
            grouped.push(*c);
        }
        return format!("{grouped}.{fraction}");
    }
    if value >= 1.0 {
        return format!("{value:.2}");
    }
    if value == 0.0 || !value.is_finite() {
        return format!("{value}");
    }
    // Три значащие цифры, как "{:.3g}" Python.
    let exponent = value.abs().log10().floor() as i32;
    let decimals = usize::try_from((2 - exponent).max(0)).unwrap_or(0);
    let text = format!("{value:.decimals$}");
    if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.').to_string() } else { text }
}

/// Размер пикселя тайла высот уровня level на широте lat, м.
pub fn pixel_size(lat: f64, level: u32) -> f64 {
    2.0 * std::f64::consts::PI * EARTH.a * lat.to_radians().cos() / (TILE_PIXELS * f64::from(1u32 << level.min(30)))
}

/// Длина ломаной по дугам большого круга на сфере радиуса большой
/// полуоси, м.
pub fn sphere_length(points: &[(f64, f64)]) -> f64 {
    points
        .windows(2)
        .map(|w| {
            let ((a, b), (c, d)) = (w[0], w[1]);
            let (p1, p2) = (a.to_radians(), c.to_radians());
            let cos_d = p1.sin() * p2.sin() + p1.cos() * p2.cos() * (d - b).to_radians().cos();
            cos_d.clamp(-1.0, 1.0).acos()
        })
        .sum::<f64>()
        * EARTH.a
}

fn mean_lat(points: &[(f64, f64)]) -> f64 {
    if points.is_empty() { 0.0 } else { points.iter().map(|p| p.0).sum::<f64>() / points.len() as f64 }
}

/// Точки вдоль линии для профиля. Шаг - пиксель самого точного уровня
/// высот на средней широте, при длинной линии больше, чтобы точек было
/// не больше limit.
pub fn sample_line(points: &[(f64, f64)], limit: usize) -> Vec<(f64, f64)> {
    if points.len() < 2 {
        return points.to_vec();
    }
    let step = pixel_size(mean_lat(points), HEIGHT_LEVEL).max(sphere_length(points) / limit as f64);
    densify(points, false, step, limit)
}

/// Уровень тайлов высот, пиксель которого не мельче шага выборки.
pub fn height_level(points: &[(f64, f64)], limit: usize) -> u32 {
    let lat = mean_lat(points);
    let step = pixel_size(lat, HEIGHT_LEVEL).max(sphere_length(points) / limit as f64);
    let mut level = HEIGHT_LEVEL;
    while level > 0 && pixel_size(lat, level) < step {
        level -= 1;
    }
    level
}

/// Ключи тайлов высот уровня level под точками, по порядку, без
/// повторов: (уровень, x, y).
pub fn tiles_along(latlon: &[(f64, f64)], level: u32) -> Vec<(u32, u32, u32)> {
    let mut out: Vec<(u32, u32, u32)> = Vec::new();
    for (lat, lon) in latlon {
        let (x, y) = lonlat_to_tile(*lat, *lon, level);
        if !out.contains(&(level, x, y)) {
            out.push((level, x, y));
        }
    }
    out
}

/// Профиль высот вдоль линии.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Profile {
    pub latlon: Vec<(f64, f64)>,
    /// Расстояние от начала по карте, м.
    pub distance: Vec<f64>,
    pub height: Vec<f64>,
    /// Длина по карте и по рельефу.
    pub flat: f64,
    pub ground: f64,
    pub low: f64,
    pub high: f64,
    /// Набор и потеря высоты.
    pub gain: f64,
    pub loss: f64,
    /// Средний и наибольший уклон в долях (0.1 - 10 %).
    pub mean_slope: f64,
    pub max_slope: f64,
    /// Длина отрезка для уклонов, м.
    pub window: f64,
}

/// Профиль по точкам вдоль линии и их высотам. window - длина отрезка
/// для уклонов, м: уклон в точке - перепад до первой точки не ближе
/// window, делённый на расстояние до неё.
pub fn profile(latlon: &[(f64, f64)], heights: &[f64], window: f64) -> Profile {
    let e = Ellipsoid::wgs84();
    let n = latlon.len().min(heights.len());
    let (latlon, heights) = (&latlon[..n], &heights[..n]);
    let flat_xyz: Vec<_> = latlon.iter().map(|(a, b)| e.geodetic_to_ecef(*a, *b, 0.0)).collect();
    let ground_xyz: Vec<_> = latlon.iter().zip(heights).map(|((a, b), h)| e.geodetic_to_ecef(*a, *b, *h)).collect();
    let mut distance = vec![0.0];
    for w in flat_xyz.windows(2) {
        distance.push(distance.last().copied().unwrap_or(0.0) + (w[1] - w[0]).length());
    }
    distance.truncate(n.max(1));
    let ground = ground_xyz.windows(2).map(|w| (w[1] - w[0]).length()).sum();
    let rise: Vec<f64> = heights.windows(2).map(|w| w[1] - w[0]).collect();
    let mut slopes = Vec::new();
    for i in 0..n {
        let target = distance[i] + window;
        let ahead = distance.partition_point(|d| *d < target).max(i + 1);
        if ahead < n {
            let run = distance[ahead] - distance[i];
            if run > 0.0 {
                slopes.push((heights[ahead] - heights[i]).abs() / run);
            }
        }
    }
    let fold = |f: fn(f64, f64) -> f64, start: f64| heights.iter().copied().fold(start, f);
    Profile {
        latlon: latlon.to_vec(),
        flat: if n > 0 { distance.last().copied().unwrap_or(0.0) } else { 0.0 },
        distance: if n > 0 { distance } else { Vec::new() },
        height: heights.to_vec(),
        ground,
        low: if n > 0 { fold(f64::min, f64::INFINITY) } else { 0.0 },
        high: if n > 0 { fold(f64::max, f64::NEG_INFINITY) } else { 0.0 },
        gain: rise.iter().filter(|r| **r > 0.0).sum(),
        loss: -rise.iter().filter(|r| **r < 0.0).sum::<f64>(),
        mean_slope: if slopes.is_empty() { 0.0 } else { slopes.iter().sum::<f64>() / slopes.len() as f64 },
        max_slope: slopes.iter().copied().fold(0.0, f64::max),
        window,
    }
}

/// Высоты точек (широта, долгота), м.
pub type HeightsAt<'a> = dyn Fn(&[(f64, f64)]) -> Vec<f64> + 'a;

/// Длина по рельефу ломаной points, м. heights_at - высоты точек.
pub fn ground_length(points: &[(f64, f64)], heights_at: &HeightsAt<'_>, limit: usize) -> f64 {
    let latlon = sample_line(points, limit);
    if latlon.len() < 2 {
        return 0.0;
    }
    profile(&latlon, &heights_at(&latlon), 0.0).ground
}

/// Начальный азимут с севера по часовой, градусы 0-360, сфера.
pub fn bearing(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    crate::tour::bearing(lat1, lon1, lat2, lon2)
}

/// Номер точки, ближайшей к пикселю (px, py) не дальше radius, или
/// None. Точка за камерой (front false) не хватается.
pub fn nearest_vertex(pixels: &[([f64; 2], bool)], px: f64, py: f64, radius: f64) -> Option<usize> {
    crate::editing::pick(pixels, &[], px, py, radius).map(|p| match p {
        crate::editing::Picked::Vertex(i) | crate::editing::Picked::Middle(i) => i,
    })
}

/// Середины отрезков ломаной, (широта, долгота). closed - и отрезок от
/// последней точки к первой.
pub fn segment_midpoints(points: &[(f64, f64)], closed: bool) -> Vec<(f64, f64)> {
    let e = Ellipsoid::wgs84();
    let mut pairs: Vec<((f64, f64), (f64, f64))> = points.windows(2).map(|w| (w[0], w[1])).collect();
    if closed && points.len() >= 3
        && let (Some(last), Some(first)) = (points.last(), points.first())
    {
        pairs.push((*last, *first));
    }
    pairs
        .into_iter()
        .map(|(a, b)| {
            // Середина хорды лежит под поверхностью, её геодезические
            // широта и долгота - середина дуги. Направление из центра
            // Земли дало бы геоцентрическую широту, она расходится
            // с геодезической до 0.19°.
            let middle = (e.geodetic_to_ecef(a.0, a.1, 0.0) + e.geodetic_to_ecef(b.0, b.1, 0.0)) * 0.5;
            let (lat, lon, _) = e.ecef_to_geodetic(middle);
            (lat, lon)
        })
        .collect()
}

/// Высота точки для строки состояния: глубина ниже уровня моря или
/// высота. Глубина меньше полуметра - высота 0, иначе ещё не
/// загруженная высота печаталась бы как «-0 м».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Depth(i64),
    Height(i64),
}

pub fn surface_level(height: f64) -> Level {
    if height < -0.5 { Level::Depth((-height).round() as i64) } else { Level::Height(height.max(0.0).round() as i64) }
}
