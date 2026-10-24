// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Уклон и экспозиция поверхности по тайлу высот. Перенос
//! `core/slope.py` модуля.
//!
//! Тайл высот Terrarium - 256×256 пикселей в сетке Web Mercator. Шаг
//! пикселя на местности по широте и долготе один в пределах строки
//! и равен 2πR·cos φ / (256·2^z). Производные высоты по востоку
//! и северу - разности, делённые на шаг строки, на краях тайла
//! односторонние. Уклон - угол поверхности к горизонту, 0-90°.
//! Экспозиция - азимут направления вниз по склону, 0° - север,
//! по часовой стрелке. У почти ровного места (уклон меньше FLAT)
//! экспозиции нет. Шкала уклона и цвета сторон света утверждены автором
//! модуля 2 октября 2026 года.

use crate::contours::{Grid, gradient};

pub const SIZE: usize = 256;
/// Градусов: ровнее - экспозиции нет.
pub const FLAT: f64 = 0.5;
pub const OPACITY: f64 = 0.7;
/// Шкала уклона: нижняя граница класса, °, и цвет. Зелёный - ровно,
/// красный и лиловый - круто.
pub const SLOPE_STOPS: [(f64, [u8; 3]); 7] =
    [(0.0, [26, 150, 65]), (2.0, [166, 217, 106]), (5.0, [255, 255, 191]), (10.0, [253, 174, 97]), (15.0, [244, 109, 67]), (25.0, [215, 25, 28]), (35.0, [128, 0, 128])];
/// Цвета сторон света для экспозиции, по часовой стрелке от севера.
pub const ASPECT_STOPS: [(f64, [u8; 3]); 9] = [
    (0.0, [230, 25, 75]),
    (45.0, [245, 130, 48]),
    (90.0, [255, 225, 25]),
    (135.0, [60, 180, 75]),
    (180.0, [70, 240, 240]),
    (225.0, [0, 130, 200]),
    (270.0, [145, 30, 180]),
    (315.0, [240, 50, 230]),
    (360.0, [230, 25, 75]),
];
pub const FLAT_COLOR: [u8; 3] = [200, 200, 200];

/// Широта центра строки row тайла (z, y), градусы.
pub fn row_latitude(z: u32, y: u32, row: usize, size: usize) -> f64 {
    let n = f64::from(1u32 << z.min(30));
    let t = (f64::from(y) + (row as f64 + 0.5) / size as f64) / n;
    (std::f64::consts::PI * (1.0 - 2.0 * t)).sinh().atan().to_degrees()
}

/// Шаг пикселя на местности в строке row тайла (z, y), метры.
pub fn pixel_size(z: u32, y: u32, row: usize, radius: f64, size: usize) -> f64 {
    2.0 * std::f64::consts::PI * radius * row_latitude(z, y, row, size).to_radians().cos() / (size as f64 * f64::from(1u32 << z.min(30)))
}

/// Уклон, градусы, и экспозиция, градусы от севера или NaN.
pub fn slope_aspect(heights: &Grid, z: u32, y: u32, radius: f64) -> (Grid, Grid) {
    let (d_row, d_col) = gradient(heights);
    let mut slope = Grid::new(heights.rows, heights.cols, 0.0);
    let mut aspect = Grid::new(heights.rows, heights.cols, f64::NAN);
    for r in 0..heights.rows {
        let step = pixel_size(z, y, r, radius, heights.rows);
        for c in 0..heights.cols {
            let i = r * heights.cols + c;
            // Строки тайла идут с севера на юг.
            let east = d_col.data[i] / step;
            let north = -d_row.data[i] / step;
            let s = east.hypot(north).atan().to_degrees();
            slope.data[i] = s;
            // Вниз по склону - против градиента высоты.
            if s >= FLAT {
                aspect.data[i] = (-east).atan2(-north).to_degrees().rem_euclid(360.0);
            }
        }
    }
    (slope, aspect)
}

/// Цвет класса по нижним границам шкалы.
fn class(value: f64, stops: &[(f64, [u8; 3])]) -> [f64; 3] {
    let n = stops.iter().filter(|s| s.0 <= value).count().clamp(1, stops.len());
    stops[n - 1].1.map(f64::from)
}

/// Цвет по шкале с плавным переходом между точками.
fn blend(value: f64, stops: &[(f64, [u8; 3])]) -> [f64; 3] {
    if value <= stops[0].0 {
        return stops[0].1.map(f64::from);
    }
    for pair in stops.windows(2) {
        if value <= pair[1].0 {
            let t = (value - pair[0].0) / (pair[1].0 - pair[0].0);
            return [0, 1, 2].map(|k| f64::from(pair[0].1[k]) + (f64::from(pair[1].1[k]) - f64::from(pair[0].1[k])) * t);
        }
    }
    stops[stops.len() - 1].1.map(f64::from)
}

fn premultiplied(rgb: [f64; 3], alpha: f64, out: &mut Vec<u8>) {
    for c in rgb {
        out.push((c * alpha).round_ties_even() as u8);
    }
    out.push((alpha * 255.0).round_ties_even() as u8);
}

/// Раскраска уклона классами SLOPE_STOPS: RGBA с премноженной альфой.
pub fn slope_rgba(slope: &Grid) -> Vec<u8> {
    let mut out = Vec::with_capacity(slope.data.len() * 4);
    for v in &slope.data {
        premultiplied(class(*v, &SLOPE_STOPS), f64::from(OPACITY as f32), &mut out);
    }
    out
}

/// Раскраска экспозиции цветами сторон света, ровное - серое.
pub fn aspect_rgba(aspect: &Grid) -> Vec<u8> {
    let mut out = Vec::with_capacity(aspect.data.len() * 4);
    for v in &aspect.data {
        if v.is_nan() {
            premultiplied(FLAT_COLOR.map(f64::from), f64::from((OPACITY * 0.4) as f32), &mut out);
        } else {
            premultiplied(blend(*v, &ASPECT_STOPS), f64::from(OPACITY as f32), &mut out);
        }
    }
    out
}
