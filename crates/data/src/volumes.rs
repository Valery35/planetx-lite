// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Выемка и насыпь, перенос `core/volumes.py` модуля.
//!
//! Две съёмки одного участка - разность новая минус прежняя, плюс -
//! насыпь, минус - выемка. Объём - сумма разностей на площадь ячейки,
//! разности меньше порога - шум съёмки. Земляные работы - вторая разность:
//! ровная площадка на отметке level минус рельеф участка. Отметка с равными
//! объёмами - средняя высота. Открытые высоты несут ошибку в метры, поэтому
//! объём на такой доле участка идёт с оценкой площадь × [`OPEN_ERROR`].

use crate::crs::Transformer;

use crate::grid::Grid as Raster;
use crate::hydro::{self, Grid};

/// Разность меньше по модулю, м, - шум съёмки.
pub const THRESHOLD: f64 = 0.1;
/// Пустое значение файла.
pub const NODATA: f32 = -9999.0;
/// Ошибка открытых моделей высот для оценки объёма, м.
pub const OPEN_ERROR: f64 = 3.0;
/// Доля участка на своём рельефе без оговорки.
pub const OWN_ENOUGH: f64 = 0.99;
/// Наибольшая непрозрачность раскраски на глобусе.
pub const ALPHA: u8 = 200;
/// Непрозрачность у отметки площадки.
const LEAST_ALPHA: f64 = 50.0;
pub const CUT_RGB: [u8; 3] = [200, 40, 30];
pub const FILL_RGB: [u8; 3] = [30, 90, 200];

/// Объёмы (м³), площади (м²) и крайние разности (м).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Summary {
    pub fill: f64,
    pub cut: f64,
    pub net: f64,
    pub fill_area: f64,
    pub cut_area: f64,
    pub area: f64,
    pub deepest: f64,
    pub highest: f64,
}

/// Площадка на отметке level над сеткой grid. diff - отметка минус рельеф,
/// вне участка нечисло, reach - наибольший модуль разности для шкалы.
#[derive(Debug, Clone, PartialEq)]
pub struct Earthwork {
    pub grid: Grid,
    pub diff: Vec<f64>,
    pub level: f64,
    pub reach: f64,
}

/// Разность new - old: нечисло там, где нет одной из съёмок или вне маски.
pub fn difference(old: &[f64], new: &[f64], inside: Option<&[bool]>) -> Vec<f64> {
    old.iter()
        .zip(new)
        .enumerate()
        .map(|(i, (a, b))| if inside.is_none_or(|m| m.get(i).copied().unwrap_or(false)) { b - a } else { f64::NAN })
        .collect()
}

/// Объёмы и площади по разности.
pub fn summary(diff: &[f64], cell_area: f64, threshold: f64) -> Summary {
    let mut out = Summary::default();
    for &v in diff.iter().filter(|v| v.is_finite()) {
        out.area += cell_area;
        if v >= threshold {
            out.fill += v * cell_area;
            out.fill_area += cell_area;
            out.highest = out.highest.max(v);
        } else if v <= -threshold {
            out.cut -= v * cell_area;
            out.cut_area += cell_area;
            out.deepest = out.deepest.min(v);
        }
    }
    out.net = out.fill - out.cut;
    out
}

/// Разность для файла: [`NODATA`] вместо нечисла, мелкие разности - ноль.
pub fn for_file(diff: &[f64], threshold: f64) -> Vec<f32> {
    diff.iter()
        .map(|v| match v {
            v if !v.is_finite() => NODATA,
            v if v.abs() < threshold => 0.0,
            v => *v as f32,
        })
        .collect()
}

/// Разность двух съёмок на сетке новой: прежняя пересчитывается на неё
/// билинейно, как `gdal.Warp` окна модуля. Считается строками за кадры.
pub struct Surveys {
    pub new: Raster,
    old: Raster,
    to_old: Option<Transformer>,
    /// Многоугольник участка (широта, долгота) или None - весь охват.
    ring: Option<Vec<(f64, f64)>>,
    row: usize,
    /// Новая минус прежняя по строкам, нечисло - нет разности.
    pub diff: Vec<f64>,
}

impl Surveys {
    pub fn new(new: Raster, old: Raster, ring: Option<Vec<(f64, f64)>>) -> Self {
        let to_old = new.transformer_into(&old);
        let diff = Vec::with_capacity(new.width.saturating_mul(new.height));
        Self { new, old, to_old, ring, row: 0, diff }
    }

    /// Доля посчитанных строк, 0-1.
    pub fn done(&self) -> f64 {
        self.row as f64 / self.new.height.max(1) as f64
    }

    /// Следующие rows строк. Истина - все посчитаны.
    pub fn step(&mut self, rows: usize) -> bool {
        let end = (self.row + rows).min(self.new.height);
        for row in self.row..end {
            let inside = self.row_inside(row);
            for col in 0..self.new.width {
                let new = self.new.values.get(row * self.new.width + col).copied().unwrap_or(f32::NAN);
                let old = if new.is_finite() && inside.as_ref().is_none_or(|m| m.get(col).copied().unwrap_or(false)) { self.old_at(col, row) } else { f64::NAN };
                self.diff.push(f64::from(new) - old);
            }
        }
        self.row = end;
        self.row >= self.new.height
    }

    /// Маска участка строки.
    fn row_inside(&self, row: usize) -> Option<Vec<bool>> {
        let ring = self.ring.as_ref()?;
        let points: Vec<(f64, f64)> = (0..self.new.width)
            .map(|col| {
                let (x, y) = self.new.at(col as f64 + 0.5, row as f64 + 0.5);
                self.new.lat_lon(x, y).unwrap_or((f64::NAN, f64::NAN))
            })
            .collect();
        Some(crate::site::inside(&points, ring))
    }

    /// Прежняя съёмка в середине точки новой.
    fn old_at(&self, col: usize, row: usize) -> f64 {
        let (mut x, mut y) = self.new.at(col as f64 + 0.5, row as f64 + 0.5);
        if let Some(t) = &self.to_old {
            match t.transform(x, y) {
                Some(p) => (x, y) = p,
                None => return f64::NAN,
            }
        }
        self.old.pixel_of(x, y).map_or(f64::NAN, |(c, r)| f64::from(self.old.bilinear(c, r)))
    }

    /// Площадь точки новой съёмки на местности, м²: у растра в градусах
    /// шаг сетки в метры не переводится.
    pub fn cell_area(&self) -> f64 {
        let t = &self.new.transform;
        let plain = (t[1] * t[5] - t[2] * t[4]).abs();
        let (c, r) = (self.new.width as f64 / 2.0, self.new.height as f64 / 2.0);
        let at = |col: f64, row: f64| {
            let (x, y) = self.new.at(col, row);
            self.new.lat_lon(x, y)
        };
        let (Some(a), Some(b), Some(d)) = (at(c, r), at(c + 1.0, r), at(c, r + 1.0)) else {
            return plain;
        };
        let metres = |p: (f64, f64), q: (f64, f64)| {
            let east = (q.1 - p.1) * p.0.to_radians().cos();
            (q.0 - p.0).hypot(east) * crate::site::METERS_PER_DEGREE
        };
        metres(a, b) * metres(a, d)
    }
}

/// Отметка площадки с равными выемкой и насыпью - средняя высота, None
/// без высот.
pub fn balance_level(z: &[f64]) -> Option<f64> {
    let (sum, count) = z.iter().filter(|v| v.is_finite()).fold((0.0, 0usize), |(s, n), v| (s + v, n + 1));
    (count > 0).then(|| sum / count as f64)
}

/// Площадка на отметке level над высотами z сетки grid. Разность шкалы
/// не меньше метра.
pub fn earthwork(z: &[f64], grid: &Grid, level: f64) -> Earthwork {
    let diff: Vec<f64> = z.iter().map(|h| level - h).collect();
    let reach = diff.iter().filter(|v| v.is_finite()).fold(0.0f64, |a, v| a.max(v.abs()));
    Earthwork { grid: *grid, diff, level, reach: reach.max(1.0) }
}

/// Доля участка на своём рельефе. boxes - рамки растров (x0, y0, x1, y1)
/// в метрах Web Mercator.
pub fn own_share(work: &Earthwork, boxes: &[(f64, f64, f64, f64)]) -> f64 {
    let g = &work.grid;
    let (mut known, mut own) = (0usize, 0usize);
    for (i, _) in work.diff.iter().enumerate().filter(|(_, v)| v.is_finite()) {
        known += 1;
        let x = g.x0 + (i % g.cols.max(1)) as f64 * g.step;
        let y = g.y0 - (i / g.cols.max(1)) as f64 * g.step;
        if boxes.iter().any(|b| x >= b.0 && x <= b.2 && y >= b.1 && y <= b.3) {
            own += 1;
        }
    }
    if known == 0 { 0.0 } else { own as f64 / known as f64 }
}

/// Оценка ошибки объёма, м³: площадь на открытых высотах × [`OPEN_ERROR`].
pub fn open_error(work: &Earthwork, share: f64) -> f64 {
    let known = work.diff.iter().filter(|v| v.is_finite()).count();
    known as f64 * work.grid.cell.powi(2) * (1.0 - share) * OPEN_ERROR
}

/// Цвет разности с премноженной альфой: выемка красная, насыпь синяя,
/// ближе к площадке прозрачнее, у отметки и без значения прозрачно.
pub fn color(value: f64, reach: f64) -> [u8; 4] {
    if !value.is_finite() || value.abs() < THRESHOLD {
        return [0; 4];
    }
    let rgb = if value < 0.0 { CUT_RGB } else { FILL_RGB };
    let alpha = (LEAST_ALPHA + (f64::from(ALPHA) - LEAST_ALPHA) * (value.abs() / reach).min(1.0)).floor();
    let dim = |c: u8| (f64::from(c) * alpha / 255.0).round() as u8;
    [dim(rgb[0]), dim(rgb[1]), dim(rgb[2]), alpha as u8]
}

/// Картинка тайла слоя: RGBA с премноженной альфой, None - тайл мимо сетки.
pub fn tile_rgba(work: &Earthwork, key: (u32, u32, u32), radius: f64) -> Option<Vec<u8>> {
    let cells = hydro::tile_cells(&work.grid, key, radius)?;
    let mut out = vec![0u8; hydro::TILE * hydro::TILE * 4];
    for (k, cell) in cells.iter().enumerate() {
        if let Some(value) = cell.and_then(|i| work.diff.get(i)) {
            out[k * 4..k * 4 + 4].copy_from_slice(&color(*value, work.reach));
        }
    }
    Some(out)
}

/// Раскраска в точках (широта, долгота), как у [`tile_rgba`], за сеткой
/// прозрачно.
pub fn points_rgba(work: &Earthwork, points: &[(f64, f64)], radius: f64) -> Vec<[u8; 4]> {
    let g = &work.grid;
    points
        .iter()
        .map(|&(lat, lon)| {
            let (x, y) = hydro::mercator(lat, lon, radius);
            let c = ((x - g.x0) / g.step).round_ties_even();
            let r = ((g.y0 - y) / g.step).round_ties_even();
            if !(r >= 0.0 && c >= 0.0 && r < g.rows as f64 && c < g.cols as f64) {
                return [0; 4];
            }
            work.diff.get(r as usize * g.cols + c as usize).map_or([0; 4], |v| color(*v, work.reach))
        })
        .collect()
}
