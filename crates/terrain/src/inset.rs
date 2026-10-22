// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Врезка своего рельефа: растр высот поверх Terrarium. Перенос
//! `core/inset.py` модуля, решение автора модуля от 5 октября 2026 года.
//!
//! Высоты растра заменяют Terrarium там, где у растра есть данные.
//! На полосе вдоль края данных высоты плавно переходят от растра
//! к Terrarium, ширина полосы - [`BAND_SHARE`] меньшей стороны охвата.
//! Внутри охвата уровни высот идут глубже [`crate::MAX_LEVEL`], до пикселя
//! растра, но не глубже [`crate::DEEP_LEVEL`]. Рамки - в метрах
//! Web Mercator (EPSG:3857): (запад, юг, восток, север).

use globe_core::lod::TileKey;

use crate::{DEEP_LEVEL, HeightTile, MAX_LEVEL, SIZE, sample};

/// Полуширина мира Web Mercator, метры.
pub const HALF: f64 = 20037508.342789244;
/// Полоса края - доля меньшей стороны охвата.
pub const BAND_SHARE: f64 = 0.05;

/// Рамка (запад, юг, восток, север), метры EPSG:3857.
pub type Bounds = (f64, f64, f64, f64);

/// Рамка тайла (z, x, y) в EPSG:3857.
pub fn tile_bounds(z: u32, x: u32, y: u32) -> Bounds {
    let size = 2.0 * HALF / f64::from(1u32 << z.min(31));
    let west = -HALF + f64::from(x) * size;
    let north = HALF - f64::from(y) * size;
    (west, north - size, west + size, north)
}

/// Рамка EPSG:3857 в долях мира: (u0, v0, u1, v1), v сверху вниз.
pub fn shares(b: Bounds) -> (f64, f64, f64, f64) {
    let (west, south, east, north) = b;
    let w = 2.0 * HALF;
    ((west + HALF) / w, (HALF - north) / w, (east + HALF) / w, (HALF - south) / w)
}

/// Уровень высот, пиксель которого не крупнее pixel - пикселя растра
/// в метрах EPSG:3857. Не мельче MAX_LEVEL и не глубже DEEP_LEVEL.
pub fn level_for(pixel: f64) -> u32 {
    if pixel <= 0.0 || !pixel.is_finite() {
        return DEEP_LEVEL;
    }
    let level = (2.0 * HALF / (SIZE as f64 * pixel)).log2().ceil();
    (level.max(0.0) as u32).clamp(MAX_LEVEL, DEEP_LEVEL)
}

/// Ширина полосы края в метрах EPSG:3857.
pub fn band_width(b: Bounds) -> f64 {
    let (west, south, east, north) = b;
    BAND_SHARE * (east - west).min(north - south)
}

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Доля растра по расстоянию до края данных: 0 на краю, 1 дальше
/// полосы band, между ними плавно.
pub fn edge_weight(distance: f64, band: f64) -> f32 {
    if band <= 0.0 {
        return if distance > 0.0 { 1.0 } else { 0.0 };
    }
    smoothstep(distance / band) as f32
}

/// Высоты base, в которые врезан растр dem с весом weight. dem - высоты
/// растра, NaN без данных. weight - доля растра от 0 до 1, NaN считается
/// нулём.
pub fn merge(base: &[f32], dem: &[f32], weight: &[f32]) -> Vec<f32> {
    base.iter()
        .zip(dem)
        .zip(weight)
        .map(|((b, d), w)| {
            let w = if w.is_finite() && d.is_finite() { w.clamp(0.0, 1.0) } else { 0.0 };
            if w == 0.0 { *b } else { b + w * (d - b) }
        })
        .collect()
}

/// Пересекаются ли рамки.
pub fn overlaps(a: Bounds, b: Bounds) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

/// Высоты тайла key по тайлу-предку parent, билинейно по центрам
/// пикселей.
pub fn resample(parent: &HeightTile, key: TileKey) -> Vec<f32> {
    let (z, x, y) = key;
    let n = f64::from(1u32 << z.min(31));
    let size = SIZE as f64;
    let mut out = Vec::with_capacity(SIZE * SIZE);
    for row in 0..SIZE {
        let v = (f64::from(y) + (row as f64 + 0.5) / size) / n;
        for col in 0..SIZE {
            let u = (f64::from(x) + (col as f64 + 0.5) / size) / n;
            out.push(sample(parent, u, v) as f32);
        }
    }
    out
}
