// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Мозаика тайлов подложки под участком для расчётов окон анализа,
//! перенос `core/tilemosaic.py` модуля.
//!
//! Вопрос Василия Швалёва от 1 ноября 2026 года - сегментировать
//! и классифицировать снимки с подложек. Окна «Классификация» и «Похожие
//! области» читают растр, поэтому тайлы подложки складываются в одну
//! картинку Web Mercator в памяти. На диск она не пишется.
//!
//! Уровень тайлов - первый, у которого пиксель не крупнее шага сетки
//! участка. Тайла нет или вместо снимка заглушка - берётся увеличенный
//! предок, как на глобусе ([`crop`]). Модуль пересчитывает мозаику на
//! сетку участка через GDAL, здесь это делает [`TileWarp`] билинейно
//! по центрам пикселей, как `rasterclass::Warp`.

use std::collections::HashMap;

use crate::aef::{self, Mosaic, ZoneIndex};

/// Пикселей на сторону тайла.
pub const SIZE: usize = 256;
/// Тайлов мозаики не больше: массового скачивания нет.
pub const MAX_TILES: usize = 1024;
/// Предел широты Web Mercator.
const LAT_LIMIT: f64 = 85.05;
/// Косинус широты у полюса, чтобы пиксель не был нулём.
const MIN_COS: f64 = 0.01;
/// Половина стороны мира Web Mercator, метры.
const MERCATOR_HALF: f64 = 20_037_508.342_789_244;

/// Сторона пикселя тайла уровня z на местности у широты lat, метры.
pub fn tile_pixel(lat: f64, z: u32) -> f64 {
    let cos = lat.to_radians().cos().max(MIN_COS);
    2.0 * MERCATOR_HALF * cos / (SIZE as f64 * f64::from(1u32 << z.min(30)))
}

/// Первый уровень от 0 до top с пикселем не крупнее pixel метров, у top -
/// даже если пиксель крупнее.
pub fn level_for(lat: f64, pixel: f64, top: u32) -> u32 {
    (0..=top).find(|z| tile_pixel(lat, *z) <= pixel).unwrap_or(top)
}

/// Доли мира по долготе и широте Web Mercator: x слева, y сверху, 0-1.
fn world(lat: f64, lon: f64) -> (f64, f64) {
    let lat = lat.clamp(-LAT_LIMIT, LAT_LIMIT);
    ((lon + 180.0) / 360.0, 0.5 - lat.to_radians().tan().asinh() / (2.0 * std::f64::consts::PI))
}

/// Тайлы уровня z под рамкой: столбцы first..=last, строки top..=bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub z: u32,
    pub first: u32,
    pub last: u32,
    pub top: u32,
    pub bottom: u32,
}

/// Тайлы уровня z под рамкой bbox (запад, юг, восток, север), градусы.
pub fn span(bbox: (f64, f64, f64, f64), z: u32) -> Frame {
    let (west, south, east, north) = bbox;
    let count = 1u64 << z.min(30);
    let cell = |share: f64| (share * count as f64).floor().clamp(0.0, (count - 1) as f64) as u32;
    let (x0, y0) = world(north, west);
    let (x1, y1) = world(south, east);
    Frame { z, first: cell(x0), last: cell(x1), top: cell(y0), bottom: cell(y1) }
}

impl Frame {
    /// Количество тайлов.
    pub fn count(&self) -> usize {
        (self.last.saturating_sub(self.first) as usize + 1) * (self.bottom.saturating_sub(self.top) as usize + 1)
    }

    /// Ключи (z, x, y) тайлов по строкам.
    pub fn keys(&self) -> Vec<(u32, u32, u32)> {
        (self.top..=self.bottom).flat_map(|y| (self.first..=self.last).map(move |x| (self.z, x, y))).collect()
    }
}

/// Предок тайла key на depth уровней выше.
pub fn ancestor(key: (u32, u32, u32), depth: u32) -> (u32, u32, u32) {
    let (z, x, y) = key;
    let depth = depth.min(z).min(31);
    (z - depth, x >> depth, y >> depth)
}

/// Окно тайла key в картинке его предка на depth уровней выше,
/// увеличенное до стороны картинки: пиксель - квадрат пикселей, пересчёт
/// на сетку участка потом сгладит ступени. rgba - картинка предка
/// [`SIZE`] × [`SIZE`]. None - картинка не той стороны.
pub fn crop(rgba: &[u8], key: (u32, u32, u32), depth: u32) -> Option<Vec<u8>> {
    if rgba.len() != SIZE * SIZE * 4 {
        return None;
    }
    if depth == 0 {
        return Some(rgba.to_vec());
    }
    let cells = 1usize << depth.min(8);
    let side = SIZE / cells;
    let (x0, y0) = ((key.1 as usize % cells) * side, (key.2 as usize % cells) * side);
    let mut out = Vec::with_capacity(SIZE * SIZE * 4);
    for r in 0..SIZE {
        for c in 0..SIZE {
            let at = ((y0 + r / cells) * SIZE + x0 + c / cells) * 4;
            out.extend_from_slice(rgba.get(at..at + 4)?);
        }
    }
    Some(out)
}

/// Картинки тайлов рамки: ключ (x, y) - RGBA [`SIZE`] × [`SIZE`]. Тайла
/// без картинки нет, его место прозрачно.
#[derive(Debug, Clone, Default)]
pub struct Tiles {
    pub frame: Option<Frame>,
    pub images: HashMap<(u32, u32), Vec<u8>>,
}

impl Tiles {
    /// Пиксель мозаики (столбец, строка от левого верхнего тайла рамки):
    /// RGB и непрозрачность, None - вне картинок.
    fn pixel(&self, frame: &Frame, col: i64, row: i64) -> Option<[f32; 4]> {
        if col < 0 || row < 0 {
            return None;
        }
        let (tx, ty) = (u32::try_from(col / SIZE as i64).ok()? + frame.first, u32::try_from(row / SIZE as i64).ok()? + frame.top);
        let image = self.images.get(&(tx, ty))?;
        let at = (((row % SIZE as i64) as usize) * SIZE + (col % SIZE as i64) as usize) * 4;
        let p = image.get(at..at + 4)?;
        Some([f32::from(p[0]), f32::from(p[1]), f32::from(p[2]), f32::from(p[3])])
    }

    /// Цвет мозаики в точке (широта, долгота) билинейно по центрам
    /// пикселей. None - точка вне картинок или прозрачна.
    pub fn sample(&self, lat: f64, lon: f64) -> Option<[f32; 3]> {
        let frame = self.frame?;
        let world_px = (1u64 << frame.z.min(30)) as f64 * SIZE as f64;
        let (x, y) = world(lat, lon);
        let px = x * world_px - f64::from(frame.first) * SIZE as f64 - 0.5;
        let py = y * world_px - f64::from(frame.top) * SIZE as f64 - 0.5;
        let (c0, r0) = (px.floor() as i64, py.floor() as i64);
        let (fx, fy) = ((px - c0 as f64) as f32, (py - r0 as f64) as f32);
        let mut sum = [0.0f32; 3];
        let mut weight = 0.0f32;
        for (dc, dr, w) in [(0, 0, (1.0 - fx) * (1.0 - fy)), (1, 0, fx * (1.0 - fy)), (0, 1, (1.0 - fx) * fy), (1, 1, fx * fy)] {
            if w <= 0.0 {
                continue;
            }
            // Пустой сосед в смесь не входит: край тайла без соседа
            // не темнеет.
            if let Some(p) = self.pixel(&frame, c0 + dc, r0 + dr).filter(|p| p[3] > 0.0) {
                for k in 0..3 {
                    sum[k] += p[k] * w;
                }
                weight += w;
            }
        }
        (weight > 0.0).then(|| sum.map(|v| v / weight))
    }
}

/// Пересчёт мозаики на сетку участка частями: строки за кадр.
pub struct TileWarp {
    pub grid: Mosaic<f32>,
    index: ZoneIndex,
    next: usize,
}

impl TileWarp {
    /// Сетка участка bbox (запад, юг, восток, север) с шагом step метров
    /// в зоне UTM указателя index, три канала RGB, NaN без данных.
    pub fn new(index: ZoneIndex, bbox: (f64, f64, f64, f64), step: f64) -> Self {
        let (x0, y0, cols, rows) = aef::grid_frame(&index, bbox, step);
        Self { grid: Mosaic { x0, y0, step, rows, cols, depth: 3, values: vec![f32::NAN; rows * cols * 3] }, index, next: 0 }
    }

    /// Готовых строк.
    pub fn done(&self) -> usize {
        self.next
    }

    /// Следующие count строк. true - готовы все.
    pub fn rows(&mut self, tiles: &Tiles, count: usize) -> bool {
        let (step, x0, y0, cols) = (self.grid.step, self.grid.x0, self.grid.y0, self.grid.cols);
        let end = self.next.saturating_add(count).min(self.grid.rows);
        for r in self.next..end {
            let y = y0 - (r as f64 + 0.5) * step;
            for c in 0..cols {
                let (lat, lon) = aef::from_utm(self.index.number, self.index.south, x0 + (c as f64 + 0.5) * step, y);
                if let Some(rgb) = tiles.sample(lat, lon)
                    && let Some(cell) = self.grid.values.get_mut((r * cols + c) * 3..(r * cols + c) * 3 + 3)
                {
                    cell.copy_from_slice(&rgb);
                }
            }
        }
        self.next = end;
        self.next >= self.grid.rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_follows_pixel() {
        // Пиксель уровня 17 у экватора около 1.19 м.
        assert!((tile_pixel(0.0, 17) - 1.194).abs() < 0.01);
        assert_eq!(level_for(0.0, 1.2, 19), 17);
        assert_eq!(level_for(60.0, 1.2, 19), 16);
        assert_eq!(level_for(0.0, 0.01, 19), 19);
    }

    #[test]
    fn span_covers_box() {
        let f = span((56.0, 57.9, 56.1, 58.0), 12);
        assert!(f.first <= f.last && f.top <= f.bottom);
        assert_eq!(f.keys().len(), f.count());
        assert!(f.keys().iter().all(|k| k.0 == 12));
        // Рамка в одной точке - один тайл.
        assert_eq!(span((56.0, 58.0, 56.0, 58.0), 10).count(), 1);
    }

    #[test]
    fn crop_takes_ancestor_quarter() {
        // Предок: левая половина красная, правая синяя. Правый тайл уровнем
        // ниже - синий целиком.
        let mut rgba = vec![0u8; SIZE * SIZE * 4];
        for (i, px) in rgba.chunks_mut(4).enumerate() {
            px.copy_from_slice(if i % SIZE < SIZE / 2 { &[255, 0, 0, 255] } else { &[0, 0, 255, 255] });
        }
        let part = crop(&rgba, (5, 3, 2), 1).unwrap_or_default();
        assert_eq!(part.len(), SIZE * SIZE * 4);
        assert!(part.chunks(4).all(|p| p == [0, 0, 255, 255]));
        assert_eq!(crop(&rgba[..10], (5, 3, 2), 1), None);
    }

    #[test]
    fn sample_and_warp_read_the_tile() {
        let key = (14, 10_800, 4_900);
        let frame = Frame { z: key.0, first: key.1, last: key.1, top: key.2, bottom: key.2 };
        let mut tiles = Tiles { frame: Some(frame), images: HashMap::new() };
        let image: Vec<u8> = (0..SIZE * SIZE).flat_map(|_| [10u8, 200, 30, 255]).collect();
        tiles.images.insert((key.1, key.2), image);
        // Середина тайла.
        let n = f64::from(1u32 << key.0);
        let lon = (f64::from(key.1) + 0.5) / n * 360.0 - 180.0;
        let lat = (std::f64::consts::PI * (1.0 - 2.0 * (f64::from(key.2) + 0.5) / n)).sinh().atan().to_degrees();
        let found = tiles.sample(lat, lon).unwrap_or_default();
        assert!(found.iter().zip([10.0, 200.0, 30.0]).all(|(a, b)| (a - b).abs() < 1e-3), "{found:?}");
        assert_eq!(tiles.sample(lat + 1.0, lon), None);
        let index = ZoneIndex::bare(&aef::zone_of(lon, lat));
        assert!(index.is_some());
        let Some(index) = index else {
            return;
        };
        let d = 0.0005;
        let mut warp = TileWarp::new(index, (lon - d, lat - d, lon + d, lat + d), 10.0);
        assert!(!warp.rows(&tiles, 1) || warp.grid.rows == 1);
        while !warp.rows(&tiles, 4) {}
        let good = warp.grid.values.chunks(3).filter(|c| (c[1] - 200.0).abs() < 1e-3).count();
        assert_eq!(good, warp.grid.rows * warp.grid.cols);
    }
}
