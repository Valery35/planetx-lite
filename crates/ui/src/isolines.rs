// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Горизонтали вида и изолинии своего растра слоем «Моих слоёв», как
//! «Горизонтали вида в проект QGIS…» и «Изолинии в проект QGIS…» модуля
//! (`ui/contours.py`, `ContourExport`).
//!
//! Слой - линии с полем отметки [`FIELD`], как слой `contours` модуля.
//! Горизонтали вида снимаются с тайлов высот квадрата вида
//! ([`crate::gridheights`]): тайлы просятся и ждутся, как у окон анализа,
//! высоты сглаживаются, как у горизонталей на глобусе. Сечение - по шагу
//! сетки тем же правилом, что у тайла горизонталей. Изолинии растра -
//! по его значениям с шагом около [`RASTER_LINES`] линий на размах.

use data::layers::{Item, LayerData, Shape};
use globe_core::lod::TileKey;

use crate::gridheights::{GridHeights, Step};
use crate::i18n::{Lang, tr, trf};

/// Поле отметки линии, как `elev` модуля.
pub const FIELD: &str = "elev";
/// Изолиний растра на размах значений, `grid_step` модуля.
pub const RASTER_LINES: f64 = 12.0;
/// Узлов растра по стороне не больше: крупный растр прореживается.
pub const RASTER_SIDE: usize = 1024;
/// Узлов сетки горизонталей вида по стороне не больше: около двух узлов
/// на пиксель окна, как сетка горизонталей глобуса на экране.
pub const VIEW_SIDE: usize = 2048;
/// Толщина линий слоя, пиксели.
pub const WIDTH: f32 = 1.2;

/// Ломаная изолинии: отметка и точки (широта, долгота).
pub type Line = (f64, Vec<(f64, f64)>);

/// Слой линий name с полем отметки.
pub fn layer(name: &str, lines: Vec<Line>) -> LayerData {
    let items = lines
        .into_iter()
        .map(|(level, points)| Item { shape: Shape::Line, parts: vec![points], heights: Vec::new(), values: vec![data::contours::label_text(level)] })
        .collect();
    LayerData { name: name.to_string(), epsg: Some(4326), fields: vec![FIELD.to_string()], numeric: vec![true], items }
}

/// Изолинии растра: сечение и линии в широтах и долготах. Растр крупнее
/// [`RASTER_SIDE`] узлов по стороне прореживается. Ошибка - ключ
/// перевода.
pub fn raster(grid: &data::grid::Grid) -> Result<(f64, Vec<Line>), &'static str> {
    let every = grid.width.max(grid.height).div_ceil(RASTER_SIDE).max(1);
    let (rows, cols) = (grid.height.div_ceil(every), grid.width.div_ceil(every));
    let z = data::contours::Grid::from_fn(rows, cols, |r, c| grid.value(c * every, r * every).map_or(f64::NAN, f64::from));
    let step = data::contours::grid_step(&z.data, RASTER_LINES);
    if step <= 0.0 {
        return Err("The raster has no spread of values.");
    }
    // Узел - середина точки растра.
    let at = |p: [f64; 2]| {
        let (x, y) = grid.at(p[1] * every as f64 + 0.5, p[0] * every as f64 + 0.5);
        grid.lat_lon(x, y)
    };
    let mut lines = Vec::new();
    for (level, line) in data::contours::lines(&z, step) {
        let points: Option<Vec<(f64, f64)>> = line.into_iter().map(at).collect();
        lines.push((level, points.ok_or("The coordinate system of the raster is unknown.")?));
    }
    Ok((step, lines))
}

/// Горизонтали квадрата высот: сглаживание, если пустых узлов нет, и
/// линии через step в широтах и долготах.
pub fn grid_lines(grid: &data::hydro::Grid, z: &[f64], step: f64, radius: f64) -> Vec<Line> {
    let mut heights = data::contours::Grid { rows: grid.rows, cols: grid.cols, data: z.to_vec() };
    if heights.data.len() != grid.rows * grid.cols {
        return Vec::new();
    }
    // Пустые узлы сглаживание растащило бы на соседей, тогда как есть.
    if heights.data.iter().all(|h| h.is_finite()) {
        heights = data::contours::smooth(&heights);
    }
    data::contours::lines(&heights, step).into_iter().map(|(level, line)| (level, line.into_iter().map(|p| data::hydro::latlon(grid, p[0], p[1], radius)).collect())).collect()
}

/// Чем кончился шаг горизонталей вида.
pub enum Done {
    /// Слой, сечение и загружены ли все высоты.
    Layer(LayerData, f64, bool),
    Failed(String),
}

/// Горизонтали вида: ожидание тайлов высот квадрата вида, потом линии.
pub struct ViewJob {
    heights: GridHeights,
    step: f64,
    name: String,
    /// Строка хода для сообщения.
    pub status: String,
}

impl ViewJob {
    /// Квадрат со стороной side метров у точки взгляда, сечение step - как
    /// у горизонталей на глобусе в этом виде (`contours::view_step`).
    /// Шаг сетки - пиксель уровня, с которого глобус рисует горизонтали,
    /// не больше [`VIEW_SIDE`] узлов поперёк.
    #[allow(clippy::too_many_arguments)]
    pub fn new(lang: Lang, lat: f64, lon: f64, side: f64, step: f64, radius: f64, max_level: u32, now: f64) -> Self {
        let cell = step / data::contours::STEP_PIXELS;
        let heights = GridHeights::with_side(lat, lon, side / 2.0, cell, VIEW_SIDE, radius, max_level, now);
        let name = trf(lang, "Contours {step} m", &[("step", &data::contours::label_text(step))]);
        Self { heights, step, name, status: String::new() }
    }

    pub fn wanted(&self, has: &dyn Fn(TileKey) -> bool) -> Vec<TileKey> {
        self.heights.wanted(has)
    }

    /// Шаг: None - ещё идёт, ход в [`ViewJob::status`].
    pub fn step(&mut self, lang: Lang, now: f64, has: &dyn Fn(TileKey) -> bool, heights: data::viewshed::Heights) -> Option<Done> {
        match self.heights.step(lang, now, has, heights) {
            Step::Going(text) => {
                self.status = text;
                None
            }
            Step::Failed(text) => Some(Done::Failed(text)),
            Step::Done(z) => {
                let lines = grid_lines(&self.heights.grid, &z, self.step, self.heights.radius);
                if lines.is_empty() {
                    return Some(Done::Failed(tr(lang, "The view has no contours: the terrain is flat or the heights are not loaded.").to_string()));
                }
                Some(Done::Layer(layer(&self.name, lines), self.step, self.heights.partial))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_heights_give_contour_lines_in_degrees() {
        let r = 6378137.0;
        let grid = data::hydro::make_grid(58.0, 56.25, 1000.0, 20.0, r);
        // Склон на восток: 1 м на узел, линии - меридианы.
        let z: Vec<f64> = (0..grid.rows).flat_map(|_| (0..grid.cols).map(|c| c as f64)).collect();
        let lines = grid_lines(&grid, &z, 25.0, r);
        assert!(!lines.is_empty());
        for (level, line) in &lines {
            assert!(level.rem_euclid(25.0) == 0.0 && *level > 0.0);
            let lon0 = line[0].1;
            assert!(line.iter().all(|(lat, lon)| (lon - lon0).abs() < 1e-6 && (lat - 58.0).abs() < 0.02), "{level}");
        }
        let data = layer("Горизонтали 25 м", lines.clone());
        assert_eq!((data.fields.as_slice(), data.numeric.as_slice()), (&[FIELD.to_string()][..], &[true][..]));
        assert_eq!(data.items.len(), lines.len());
        assert_eq!(data.items[0].values, [data::contours::label_text(lines[0].0)]);
        assert_eq!(data.items[0].shape, Shape::Line);
        // Пустая сетка - без линий.
        assert!(grid_lines(&grid, &vec![f64::NAN; z.len()], 25.0, r).is_empty());
    }
}
