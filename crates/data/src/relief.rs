// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Анализ рельефа квадрата у точки: уклон и экспозиция на сетке
//! [`crate::hydro`] по истинным высотам, вершины и ямы. Перенос
//! `core/relief.py` модуля.
//!
//! Уклон и экспозиция - ядро Horn 3×3 ([`crate::topo_surface`],
//! инструмент 2.08 Isoliner), вершины и ямы - find_extremes (2.09). Шаг
//! сетки на местности один по обеим осям, поэтому ядро годится как есть.
//! Экспозиция ровного места (уклон меньше [`slope::FLAT`]) - NaN, как
//! у слоя уклона вида. Классы уклона и цвета - [`crate::slope`], общие
//! со слоем.

use crate::contours;
use crate::hydro::{self, Grid};
use crate::slope;
use crate::topo_surface::{self, Extreme};

/// Вершины и ямы: радиус окна и наименьший перепад по умолчанию, метры.
/// Выбор помощника модуля, утверждает автор.
pub const PEAK_RADIUS: f64 = 300.0;
pub const PEAK_DROP: f64 = 20.0;
/// Стороны света экспозиции по 45° с серединой на направлении.
pub const SIDES: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];

/// Что показывает слой.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Slope,
    Aspect,
}

/// Уклон и экспозиция сетки grid, градусы, экспозиция ровного - NaN.
#[derive(Debug, Clone, PartialEq)]
pub struct Relief {
    pub grid: Grid,
    pub slope: Vec<f64>,
    pub aspect: Vec<f64>,
    /// Точка и полусторона квадрата, м.
    pub lat: f64,
    pub lon: f64,
    pub radius_m: f64,
    /// Рамка (юг, север, запад, восток), градусы.
    pub bbox: (f64, f64, f64, f64),
    pub mode: Mode,
}

/// Уклон и экспозиция высот z сетки grid (по строкам), radius - радиус
/// тела.
pub fn analyse(z: &[f64], grid: &Grid, radius: f64, mode: Mode) -> Relief {
    let (values, aspect) = topo_surface::slope_aspect(z, grid.rows, grid.cols, grid.cell, None);
    let aspect = values.iter().zip(aspect).map(|(s, a)| if *s < slope::FLAT { f64::NAN } else { a }).collect();
    let (north, west) = hydro::latlon(grid, 0.0, 0.0, radius);
    let (south, east) = hydro::latlon(grid, grid.rows.saturating_sub(1) as f64, grid.cols.saturating_sub(1) as f64, radius);
    Relief { grid: *grid, slope: values, aspect, lat: grid.lat, lon: grid.lon, radius_m: grid.rows.saturating_sub(1) as f64 * grid.cell / 2.0, bbox: (south, north, west, east), mode }
}

/// Сводка уклона.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// Наименьший, медиана, наибольший, градусы.
    pub low: f64,
    pub median: f64,
    pub high: f64,
    /// (нижняя граница класса, площадь м²) по slope::SLOPE_STOPS.
    pub classes: Vec<(f64, f64)>,
    /// Доли площади с экспозицией по SIDES.
    pub sides: [f64; 8],
    /// Доля ровного места.
    pub flat: f64,
}

/// Сводка уклона и экспозиции по узлам с данными. None - данных нет.
pub fn summary(relief: &Relief) -> Option<Summary> {
    let cell2 = relief.grid.cell * relief.grid.cell;
    let mut good: Vec<f64> = Vec::new();
    let mut counts = vec![0usize; slope::SLOPE_STOPS.len()];
    let mut sectors = [0usize; 8];
    let mut no_aspect = 0usize;
    for (i, v) in relief.slope.iter().enumerate() {
        if !v.is_finite() {
            continue;
        }
        good.push(*v);
        let class = slope::SLOPE_STOPS.iter().filter(|s| s.0 <= *v).count().clamp(1, slope::SLOPE_STOPS.len());
        counts[class - 1] += 1;
        match relief.aspect.get(i).copied().filter(|a| a.is_finite()) {
            Some(a) => sectors[(((a + 22.5).rem_euclid(360.0) / 45.0).floor() as usize).min(7)] += 1,
            None => no_aspect += 1,
        }
    }
    if good.is_empty() {
        return None;
    }
    good.sort_by(f64::total_cmp);
    let m = good.len();
    let median = if m % 2 == 1 { good[m / 2] } else { (good[m / 2 - 1] + good[m / 2]) / 2.0 };
    Some(Summary {
        low: good[0],
        median,
        high: good[m - 1],
        classes: slope::SLOPE_STOPS.iter().zip(&counts).map(|(s, n)| (s.0, *n as f64 * cell2)).collect(),
        sides: sectors.map(|s| s as f64 / m as f64),
        flat: no_aspect as f64 / m as f64,
    })
}

/// Вершины и ямы: (строка, столбец, отметка, перепад, вид), по убыванию
/// перепада.
pub fn extremes(z: &[f64], grid: &Grid, radius_m: f64, drop: f64) -> Vec<(usize, usize, f64, f64, Extreme)> {
    topo_surface::find_extremes(z, grid.rows, grid.cols, grid.cell, radius_m, drop, None)
}

/// Значение слоя (по mode) ближайшего узла к точке, None - вне сетки.
pub fn sample(relief: &Relief, lat: f64, lon: f64, radius: f64) -> Option<f64> {
    let grid = &relief.grid;
    let (x, y) = hydro::mercator(lat, lon, radius);
    let c = ((x - grid.x0) / grid.step).round_ties_even();
    let r = ((grid.y0 - y) / grid.step).round_ties_even();
    if !(r >= 0.0 && c >= 0.0 && r < grid.rows as f64 && c < grid.cols as f64) {
        return None;
    }
    values(relief).get(r as usize * grid.cols + c as usize).copied()
}

fn values(relief: &Relief) -> &[f64] {
    match relief.mode {
        Mode::Slope => &relief.slope,
        Mode::Aspect => &relief.aspect,
    }
}

/// Раскраска значений слоя: None - вне сетки, прозрачно. У уклона NaN
/// тоже прозрачен, у экспозиции NaN - серое ровное место.
fn paint(relief: &Relief, found: &[Option<f64>]) -> Vec<u8> {
    let data: Vec<f64> = found.iter().map(|v| v.unwrap_or(match relief.mode {
        Mode::Slope => 0.0,
        Mode::Aspect => f64::NAN,
    })).collect();
    let field = contours::Grid { rows: 1, cols: data.len(), data };
    let mut rgba = match relief.mode {
        Mode::Slope => slope::slope_rgba(&field),
        Mode::Aspect => slope::aspect_rgba(&field),
    };
    for (k, v) in found.iter().enumerate() {
        let clear = match relief.mode {
            Mode::Slope => !v.is_some_and(f64::is_finite),
            Mode::Aspect => v.is_none(),
        };
        if clear {
            rgba[k * 4..k * 4 + 4].fill(0);
        }
    }
    rgba
}

/// Раскраска слоя в точках (широта, долгота): RGBA с премноженной
/// альфой на точку, вне сетки прозрачно.
pub fn points_rgba(relief: &Relief, points: &[(f64, f64)], radius: f64) -> Vec<u8> {
    let found: Vec<Option<f64>> = points.iter().map(|(lat, lon)| sample(relief, *lat, *lon, radius)).collect();
    paint(relief, &found)
}

/// Картинка тайла key (уровень, x, y) Web Mercator слоя уклона или
/// экспозиции: TILE × TILE RGBA с премноженной альфой, как у слоя уклона
/// вида, вне сетки прозрачно. None - тайл не задевает сетку.
pub fn tile_rgba(relief: &Relief, key: (u32, u32, u32), radius: f64) -> Option<Vec<u8>> {
    let cells = hydro::tile_cells(&relief.grid, key, radius)?;
    let data = values(relief);
    let found: Vec<Option<f64>> = cells.iter().map(|c| c.and_then(|i| data.get(i).copied())).collect();
    Some(paint(relief, &found))
}
