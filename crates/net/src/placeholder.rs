// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Заглушки вместо снимка и окно тайла в картинке предка.
//!
//! Esri World Imagery там, где снимков крупного уровня нет, отвечает
//! не ошибкой, а картинкой «Map data not yet available». Это серый
//! квадрат 204, 204, 204 с надписью, 94 % пикселей одного точного цвета,
//! замер модуля 27 сентября 2026 года. Настоящий снимок, даже снег или
//! вода, так однороден не бывает, шум сжатия JPEG даёт разброс.
//!
//! Вместо заглушки глобус рисует ближайшего предка с настоящим снимком,
//! его долю, которую занимает тайл, увеличенную до стороны тайла. Так
//! делает Google Earth.
//!
//! Перенос `planetx/core/placeholder.py` модуля.

use globe_core::lod::TileKey;

/// Доля пикселей одного точного серого цвета, с которой тайл - заглушка.
pub const PLACEHOLDER_SHARE: f64 = 0.8;
/// Самый глубокий предок, из которого берётся тайл. На уровень 6 вверх
/// от тайла 256 пикселей приходится кусок 4 × 4.
pub const MAX_FILL_DEPTH: u32 = 6;

/// Заглушка ли картинка RGBA по 8 бит: серый цвет одного значения
/// не меньше чем на [`PLACEHOLDER_SHARE`] пикселей.
pub fn is_placeholder(rgba: &[u8]) -> bool {
    let pixels = rgba.len() / 4;
    if pixels == 0 {
        return false;
    }
    let mut counts = [0usize; 256];
    let mut gray = 0usize;
    for px in rgba.as_chunks::<4>().0 {
        if px[0] == px[1] && px[1] == px[2] {
            gray += 1;
            counts[usize::from(px[0])] += 1;
        }
    }
    let need = PLACEHOLDER_SHARE * pixels as f64;
    gray as f64 >= need && counts.iter().copied().max().unwrap_or(0) as f64 >= need
}

/// Предок тайла key на depth уровней выше.
pub fn ancestor(key: TileKey, depth: u32) -> TileKey {
    let (z, x, y) = key;
    let depth = depth.min(z).min(31);
    (z - depth, x >> depth, y >> depth)
}

/// Окно тайла key в картинке предка на depth уровней выше:
/// (x0, y0, сторона) в пикселях картинки предка стороной size. Первая
/// строка картинки - северный край.
pub fn crop_window(key: TileKey, depth: u32, size: u32) -> (u32, u32, u32) {
    let (_, x, y) = key;
    let depth = depth.min(31);
    let cells = 1u32 << depth;
    let side = size / cells;
    ((x % cells) * side, (y % cells) * side, side)
}
