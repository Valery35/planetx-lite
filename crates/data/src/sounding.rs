// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Батиметрия по промерам эхолота: точки глубин в грид, перенос
//! `core/sounding.py` модуля.
//!
//! Промеры - обычно галсы: линии точек через метры вдоль хода и через
//! десятки метров между галсами. Грид строит триангуляция Делоне в метрах
//! зоны UTM середины промеров, у модуля это `gdal.Grid` linear. Ячейки
//! дальше предела от ближайшего промера пустые, так грид не заливает берег
//! и заливы без промеров.
//!
//! Шаг грида по умолчанию - корень из площади промеров на точку, округлённый
//! до 1, 2 или 5 × 10ⁿ м. Предел - [`MAX_GAP_CELLS`] шагов.

use std::collections::HashSet;

use spade::handles::FixedDirectedEdgeHandle;
use spade::{DelaunayTriangulation, HasPosition, Point2, PositionInTriangulation, Triangulation};

/// Самый мелкий шаг, м.
pub const MIN_CELL: f64 = 0.5;
/// Наибольшее количество ячеек грида.
pub const MAX_CELLS: usize = 4000 * 4000;
/// Предел от ближайшего промера, шагов грида.
pub const MAX_GAP_CELLS: f64 = 10.0;

/// Ближайшее сверху из 1, 2, 2.5, 5 × 10ⁿ.
pub fn nice_step(value: f64) -> f64 {
    if value.is_nan() || value <= 0.0 {
        return 1.0;
    }
    let power = 10.0f64.powf(value.log10().floor());
    [1.0, 2.0, 2.5, 5.0, 10.0].iter().map(|b| b * power).find(|v| *v >= value * (1.0 - 1e-9)).unwrap_or(10.0 * power)
}

/// Наименьшее и наибольшее значение.
fn span(values: &[f64]) -> (f64, f64) {
    values.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(*v), b.max(*v)))
}

/// Шаг грида в метрах по рамке и количеству точек.
pub fn auto_cell(xs: &[f64], ys: &[f64]) -> f64 {
    if xs.len() < 3 {
        return 1.0;
    }
    let ((x0, x1), (y0, y1)) = (span(xs), span(ys));
    let area = ((x1 - x0) * (y1 - y0)).max(1.0);
    let count = xs.len() as f64;
    // Площадь промеров - занятые клетки вдвое крупнее среднего шага по
    // рамке: у вытянутого озера рамка в основном пустая.
    let coarse = 2.0 * (area / count).sqrt();
    let cells: HashSet<(i64, i64)> = xs.iter().zip(ys).map(|(x, y)| (((x - x0) / coarse).floor() as i64, ((y - y0) / coarse).floor() as i64)).collect();
    let cell = (area.min(cells.len() as f64 * coarse * coarse) / count).sqrt();
    // Грид не больше MAX_CELLS ячеек.
    nice_step(cell.max((area / MAX_CELLS as f64).sqrt()).max(MIN_CELL))
}

/// Рамка (запад, юг, восток, север) по точкам с полями в шаг и размеры
/// (столбцов, строк). Края кратны шагу.
pub fn grid_frame(xs: &[f64], ys: &[f64], cell: f64) -> ((f64, f64, f64, f64), (usize, usize)) {
    let ((x0, x1), (y0, y1)) = (span(xs), span(ys));
    let west = (x0 / cell).floor() * cell - cell;
    let south = (y0 / cell).floor() * cell - cell;
    let east = (x1 / cell).ceil() * cell + cell;
    let north = (y1 / cell).ceil() * cell + cell;
    ((west, south, east, north), (((east - west) / cell).round() as usize, ((north - south) / cell).round() as usize))
}

/// Точки с конечными координатами и глубиной, глубина - модуль: у эхолотов
/// она бывает и со знаком минус.
pub fn clean(xs: &[f64], ys: &[f64], zs: &[f64]) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let (mut a, mut b, mut c) = (Vec::new(), Vec::new(), Vec::new());
    for ((x, y), z) in xs.iter().zip(ys).zip(zs) {
        if x.is_finite() && y.is_finite() && z.is_finite() {
            a.push(*x);
            b.push(*y);
            c.push(z.abs());
        }
    }
    (a, b, c)
}

/// Вершина триангуляции: глубина в точке.
#[derive(Clone, Copy)]
struct Sounding {
    position: Point2<f64>,
    depth: f64,
}

impl HasPosition for Sounding {
    type Scalar = f64;

    fn position(&self) -> Point2<f64> {
        self.position
    }
}

/// Глубина в точке внутри треугольника a, b, c.
fn plane(a: &Sounding, b: &Sounding, c: &Sounding, p: Point2<f64>) -> f64 {
    let (pa, pb, pc) = (a.position, b.position, c.position);
    let area = (pb.y - pc.y) * (pa.x - pc.x) + (pc.x - pb.x) * (pa.y - pc.y);
    let u = ((pb.y - pc.y) * (p.x - pc.x) + (pc.x - pb.x) * (p.y - pc.y)) / area;
    let v = ((pc.y - pa.y) * (p.x - pc.x) + (pa.x - pc.x) * (p.y - pc.y)) / area;
    u * a.depth + v * b.depth + (1.0 - u - v) * c.depth
}

/// Сборка грида строками за кадры. depths - глубины ячеек с севера,
/// нечисло - пусто.
pub struct Build {
    tri: DelaunayTriangulation<Sounding>,
    pub west: f64,
    pub north: f64,
    pub cell: f64,
    pub cols: usize,
    pub rows: usize,
    limit: f64,
    row: usize,
    pub depths: Vec<f64>,
}

impl Build {
    /// Триангуляция точек (x, y, глубина) в метрах. None - точек меньше
    /// трёх, все на одной прямой или грид больше [`MAX_CELLS`].
    pub fn new(xs: &[f64], ys: &[f64], zs: &[f64], cell: f64, limit: f64) -> Option<Self> {
        if xs.len() < 3 || cell <= 0.0 {
            return None;
        }
        let ((west, _, _, north), (cols, rows)) = grid_frame(xs, ys, cell);
        if cols.saturating_mul(rows) > MAX_CELLS {
            return None;
        }
        let vertices: Vec<Sounding> = xs.iter().zip(ys).zip(zs).map(|((x, y), z)| Sounding { position: Point2::new(*x, *y), depth: *z }).collect();
        let tri = DelaunayTriangulation::<Sounding>::bulk_load(vertices).ok()?;
        (tri.num_inner_faces() > 0).then(|| Self { tri, west, north, cell, cols, rows, limit, row: 0, depths: Vec::with_capacity(cols * rows) })
    }

    /// Доля посчитанных строк, 0-1.
    pub fn done(&self) -> f64 {
        self.row as f64 / self.rows.max(1) as f64
    }

    /// Ячеек с глубиной.
    pub fn known(&self) -> usize {
        self.depths.iter().filter(|d| d.is_finite()).count()
    }

    /// Глубина в точке по плоскости треугольника, None - вне триангуляции.
    fn linear(&self, point: Point2<f64>) -> Option<f64> {
        match self.tri.locate(point) {
            PositionInTriangulation::OnVertex(v) => Some(self.tri.vertex(v).data().depth),
            PositionInTriangulation::OnFace(f) => {
                let [a, b, c] = self.tri.face(f).vertices().map(|v| *v.data());
                Some(plane(&a, &b, &c, point))
            }
            PositionInTriangulation::OnEdge(e) => self.on_edge(e, point),
            PositionInTriangulation::OutsideOfConvexHull(_) | PositionInTriangulation::NoTriangulation => None,
        }
    }

    /// Глубина точки на ребре: между его концами.
    fn on_edge(&self, edge: FixedDirectedEdgeHandle, point: Point2<f64>) -> Option<f64> {
        let [a, b] = self.tri.directed_edge(edge).vertices().map(|v| *v.data());
        let (dx, dy) = (b.position.x - a.position.x, b.position.y - a.position.y);
        let length = dx * dx + dy * dy;
        (length > 0.0).then(|| {
            let t = ((point.x - a.position.x) * dx + (point.y - a.position.y) * dy) / length;
            a.depth + (b.depth - a.depth) * t.clamp(0.0, 1.0)
        })
    }

    /// Следующие rows строк. Истина - все посчитаны.
    pub fn step(&mut self, rows: usize) -> bool {
        let end = (self.row + rows).min(self.rows);
        for row in self.row..end {
            let y = self.north - (row as f64 + 0.5) * self.cell;
            for col in 0..self.cols {
                let point = Point2::new(self.west + (col as f64 + 0.5) * self.cell, y);
                let near = self.tri.nearest_neighbor(point).is_some_and(|v| v.data().position.distance_2(point).sqrt() <= self.limit);
                let depth = if near { self.linear(point) } else { None };
                self.depths.push(depth.unwrap_or(f64::NAN));
            }
        }
        self.row = end;
        self.row >= self.rows
    }
}
