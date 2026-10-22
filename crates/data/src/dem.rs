// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Свой рельеф для врезки: растр высот, выбираемый на тайлы Web Mercator.
//!
//! Модуль пересчитывает растр в EPSG:3857 файлом GDAL. Здесь растр
//! остаётся в своей системе, а точки тайла пересчитываются в неё: узлы
//! сетки [`NODES`] × [`NODES`] на тайл - движком terrio, точки между
//! ними - линейно. Отклонение от точного пересчёта внутри клетки тайла
//! уровня 15 и глубже - доли миллиметра.
//!
//! Вес края - доля растра от 0 на краю данных до 1 дальше полосы края,
//! как `core/inset.edge_weight` модуля. Расстояние до края считается
//! по маске данных на сетке не крупнее [`MASK_SIDE`] по стороне,
//! рамка растра - тоже край.

use terrio::crs::Transformer;

use crate::grid::Grid;

/// Узлов сетки пересчёта на сторону тайла.
pub const NODES: usize = 17;
/// Наибольшая сторона маски для расстояния до края.
pub const MASK_SIDE: usize = 1024;
/// Точек на сторону рамки растра при её пересчёте в EPSG:3857.
const EDGE_POINTS: usize = 32;

/// Растр высот для врезки.
pub struct Dem {
    grid: Grid,
    from_mercator: Transformer,
    /// Рамка в метрах EPSG:3857: запад, юг, восток, север.
    pub bounds: (f64, f64, f64, f64),
    /// Размер точки растра в метрах EPSG:3857 у середины.
    pub pixel: f64,
    /// Расстояние до края данных на маске, в точках растра.
    distance: Vec<f32>,
    mask_w: usize,
    mask_h: usize,
    /// Точек растра на клетку маски.
    step: usize,
    /// Ширина полосы края в точках растра.
    band: f64,
}

impl std::fmt::Debug for Dem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dem").field("grid", &self.grid).field("bounds", &self.bounds).field("pixel", &self.pixel).finish()
    }
}

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Расстояние до ближайшей клетки без данных, двухпроходный обход
/// с весами 3 и 4 (шаг по стороне и по диагонали), в клетках.
fn chamfer(valid: &[bool], w: usize, h: usize) -> Vec<f32> {
    const FAR: f32 = 1.0e9;
    let mut d: Vec<f32> = valid.iter().map(|v| if *v { FAR } else { 0.0 }).collect();
    let at = |d: &[f32], x: isize, y: isize| -> f32 {
        if x < 0 || y < 0 || x >= w as isize || y >= h as isize { 0.0 } else { d[y as usize * w + x as usize] }
    };
    for y in 0..h as isize {
        for x in 0..w as isize {
            let i = y as usize * w + x as usize;
            if d[i] == 0.0 {
                continue;
            }
            let best = [(-1, 0, 3.0), (0, -1, 3.0), (-1, -1, 4.0), (1, -1, 4.0)]
                .iter()
                .map(|(dx, dy, c)| at(&d, x + dx, y + dy) + c)
                .fold(d[i], f32::min);
            d[i] = best;
        }
    }
    for y in (0..h as isize).rev() {
        for x in (0..w as isize).rev() {
            let i = y as usize * w + x as usize;
            if d[i] == 0.0 {
                continue;
            }
            let best = [(1, 0, 3.0), (0, 1, 3.0), (1, 1, 4.0), (-1, 1, 4.0)]
                .iter()
                .map(|(dx, dy, c)| at(&d, x + dx, y + dy) + c)
                .fold(d[i], f32::min);
            d[i] = best;
        }
    }
    d.into_iter().map(|v| v / 3.0).collect()
}

impl Dem {
    /// Врезка из растра. band_share - полоса края, доля меньшей стороны.
    pub fn new(grid: Grid, band_share: f64) -> Result<Dem, String> {
        let from_mercator = grid.transformer_from(3857).ok_or("у растра нет системы координат")?;
        let to_mercator = grid.transformer_to(3857).ok_or("у растра нет системы координат")?;
        let (w, h) = (grid.width as f64, grid.height as f64);
        let mut edge = Vec::new();
        for k in 0..=EDGE_POINTS {
            let s = k as f64 / EDGE_POINTS as f64;
            edge.extend([(s * w, 0.0), (s * w, h), (0.0, s * h), (w, s * h)]);
        }
        let mut b = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for (c, r) in edge {
            let (x, y) = grid.at(c, r);
            let (mx, my) = to_mercator.transform(x, y).map_err(|e| e.to_string())?;
            b = (b.0.min(mx), b.1.min(my), b.2.max(mx), b.3.max(my));
        }
        if !(b.0 < b.2 && b.1 < b.3) {
            return Err("рамка растра пуста".into());
        }
        let (cx, cy) = grid.at(w / 2.0, h / 2.0);
        let (nx, ny) = grid.at(w / 2.0 + 1.0, h / 2.0);
        let p0 = to_mercator.transform(cx, cy).map_err(|e| e.to_string())?;
        let p1 = to_mercator.transform(nx, ny).map_err(|e| e.to_string())?;
        let pixel = (p1.0 - p0.0).hypot(p1.1 - p0.1);
        let step = grid.width.max(grid.height).div_ceil(MASK_SIDE).max(1);
        let (mask_w, mask_h) = (grid.width.div_ceil(step), grid.height.div_ceil(step));
        let valid: Vec<bool> = (0..mask_h)
            .flat_map(|my| (0..mask_w).map(move |mx| (mx, my)))
            .map(|(mx, my)| {
                let col = (mx * step + step / 2).min(grid.width - 1);
                let row = (my * step + step / 2).min(grid.height - 1);
                grid.value(col, row).is_some()
            })
            .collect();
        let distance = chamfer(&valid, mask_w, mask_h).into_iter().map(|d| d * step as f32).collect();
        let band = band_share * w.min(h);
        Ok(Dem { grid, from_mercator, bounds: b, pixel, distance, mask_w, mask_h, step, band })
    }

    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    /// Высоты и вес края для тайла с рамкой tile (метры EPSG:3857: запад,
    /// юг, восток, север), size × size точек по строкам с севера. Высота
    /// NaN - у растра нет данных.
    pub fn sample(&self, tile: (f64, f64, f64, f64), size: usize) -> (Vec<f32>, Vec<f32>) {
        let (west, south, east, north) = tile;
        let cells = (NODES - 1) as f64;
        let nodes: Vec<Option<(f64, f64)>> = (0..NODES)
            .flat_map(|j| (0..NODES).map(move |i| (i, j)))
            .map(|(i, j)| {
                let mx = west + (east - west) * i as f64 / cells;
                let my = north - (north - south) * j as f64 / cells;
                let (x, y) = self.from_mercator.transform(mx, my).ok()?;
                self.grid.pixel_of(x, y)
            })
            .collect();
        let mut heights = Vec::with_capacity(size * size);
        let mut weights = Vec::with_capacity(size * size);
        for row in 0..size {
            let fy = (row as f64 + 0.5) / size as f64 * cells;
            let j = (fy.floor() as usize).min(NODES - 2);
            let ty = fy - j as f64;
            for col in 0..size {
                let fx = (col as f64 + 0.5) / size as f64 * cells;
                let i = (fx.floor() as usize).min(NODES - 2);
                let tx = fx - i as f64;
                let corner = |di: usize, dj: usize| nodes.get((j + dj) * NODES + i + di).copied().flatten();
                let (Some(a), Some(b), Some(c), Some(d)) = (corner(0, 0), corner(1, 0), corner(0, 1), corner(1, 1)) else {
                    heights.push(f32::NAN);
                    weights.push(0.0);
                    continue;
                };
                let lerp = |p: (f64, f64), q: (f64, f64), t: f64| (p.0 + (q.0 - p.0) * t, p.1 + (q.1 - p.1) * t);
                let (pc, pr) = lerp(lerp(a, b, tx), lerp(c, d, tx), ty);
                heights.push(self.grid.bilinear(pc, pr));
                weights.push(self.weight(pc, pr));
            }
        }
        (heights, weights)
    }

    /// Вес края в точке растра (столбец, строка) с долями.
    fn weight(&self, col: f64, row: f64) -> f32 {
        if col < 0.0 || row < 0.0 {
            return 0.0;
        }
        let (mx, my) = ((col / self.step as f64) as usize, (row / self.step as f64) as usize);
        if mx >= self.mask_w || my >= self.mask_h {
            return 0.0;
        }
        let d = f64::from(self.distance.get(my * self.mask_w + mx).copied().unwrap_or(0.0));
        if self.band <= 0.0 {
            return if d > 0.0 { 1.0 } else { 0.0 };
        }
        smoothstep(d / self.band) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::chamfer;

    #[test]
    fn chamfer_counts_cells_to_the_edge() {
        // Квадрат 7 × 7: в середине до края 4 клетки, у края 1.
        let d = chamfer(&[true; 49], 7, 7);
        assert_eq!((d[24], d[0], d[3]), (4.0, 1.0, 1.0));
        assert_eq!(d[8], 2.0);
        let mut hole = vec![true; 25];
        hole[12] = false;
        let d = chamfer(&hole, 5, 5);
        assert_eq!(d[12], 0.0);
        assert_eq!(d[11], 1.0);
    }
}
