// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Участок окон анализа участка - многоугольник (широта, долгота), как
//! у Google Earth. Геометрия `square`, `square_ring`, `ring_box`
//! (`core/aef.py`) и `Area` (`ui/area.py`) модуля без окна.
//!
//! Решение автора от 28 октября 2026 года: при открытии окна участок -
//! квадрат с серединой в точке, его вершины тянутся мышью, участок можно
//! нарисовать заново щелчками. Расчёт идёт по рамке участка, вне
//! многоугольника результатов нет.

/// Метров в градусе широты, как у модуля.
pub const METERS_PER_DEGREE: f64 = 111320.0;

/// Рамка квадрата side метров с серединой в точке: (запад, юг, восток,
/// север), градусы.
pub fn square(lat: f64, lon: f64, side: f64) -> (f64, f64, f64, f64) {
    let half_lat = side / 2.0 / METERS_PER_DEGREE;
    let half_lon = half_lat / lat.to_radians().cos().max(0.01);
    (lon - half_lon, lat - half_lat, lon + half_lon, lat + half_lat)
}

/// Вершины квадрата side метров с серединой в точке, (широта, долгота)
/// против часовой стрелки с юго-запада.
pub fn square_ring(lat: f64, lon: f64, side: f64) -> Vec<(f64, f64)> {
    let (west, south, east, north) = square(lat, lon, side);
    vec![(south, west), (south, east), (north, east), (north, west)]
}

/// Рамка многоугольника в градусах (запад, юг, восток, север) и его
/// наибольший размер в метрах. None - вершин нет.
pub fn ring_box(points: &[(f64, f64)]) -> Option<((f64, f64, f64, f64), f64)> {
    let first = points.first()?;
    let (mut south, mut north, mut west, mut east) = (first.0, first.0, first.1, first.1);
    for &(lat, lon) in points {
        south = south.min(lat);
        north = north.max(lat);
        west = west.min(lon);
        east = east.max(lon);
    }
    let middle = ((south + north) / 2.0).to_radians();
    let side = ((north - south) * METERS_PER_DEGREE).max((east - west) * METERS_PER_DEGREE * middle.cos());
    Some(((west, south, east, north), side))
}

/// Признак «внутри многоугольника» для точек, правило чётности.
pub fn inside(points: &[(f64, f64)], ring: &[(f64, f64)]) -> Vec<bool> {
    let ring: Vec<[f64; 2]> = ring.iter().map(|&(a, b)| [a, b]).collect();
    let (lats, lons): (Vec<f64>, Vec<f64>) = points.iter().copied().unzip();
    crate::subsurface::inside(&lats, &lons, &ring)
}

/// Высоты узлов сетки grid вне многоугольника - нечисло: вне участка
/// высот нет, как у `GridHeights` модуля. radius - радиус тела.
pub fn mask_grid(z: &mut [f64], grid: &crate::hydro::Grid, ring: &[(f64, f64)], radius: f64) {
    if ring.len() < 3 {
        return;
    }
    let points: Vec<(f64, f64)> = (0..grid.rows).flat_map(|r| (0..grid.cols).map(move |c| (r, c))).map(|(r, c)| crate::hydro::latlon(grid, r as f64, c as f64, radius)).collect();
    for (value, keep) in z.iter_mut().zip(inside(&points, ring)) {
        if !keep {
            *value = f64::NAN;
        }
    }
}

/// Сетка grid в метрах Web Mercator для GeoTIFF.
pub fn mercator_of(grid: &crate::hydro::Grid) -> crate::geotiff::MercatorGrid {
    crate::geotiff::MercatorGrid { x0: grid.x0, y0: grid.y0, step: grid.step, cols: grid.cols, rows: grid.rows }
}

/// Почему по участку считать нельзя.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Problem {
    /// Участок не замкнут: вершин меньше трёх или он ещё рисуется.
    Open,
    /// Участок больше предела, километры.
    TooLarge(f64),
}

/// Участок окна: середина квадрата, вершины, сторона квадрата
/// и наибольший размер, рисуется ли он щелчками.
#[derive(Debug, Clone, PartialEq)]
pub struct Site {
    pub point: (f64, f64),
    pub ring: Vec<(f64, f64)>,
    /// Сторона квадрата, километры.
    pub side_km: f64,
    /// Наибольший размер участка, километры.
    pub most_km: f64,
    pub drawing: bool,
}

impl Site {
    /// Квадрат side_km с серединой в точке, сторона не больше most_km.
    pub fn new(lat: f64, lon: f64, side_km: f64, most_km: f64) -> Self {
        let mut site = Self { point: (lat, lon), ring: Vec::new(), side_km: side_km.clamp(0.1, most_km.max(0.1)), most_km, drawing: false };
        site.reset_square();
        site
    }

    /// Участок - квадрат стороной side_km у точки окна.
    pub fn reset_square(&mut self) {
        self.side_km = self.side_km.clamp(0.1, self.most_km.max(0.1));
        self.ring = square_ring(self.point.0, self.point.1, self.side_km * 1000.0);
        self.drawing = false;
    }

    /// Новая точка окна: квадрат прежней стороны у неё.
    pub fn move_to(&mut self, lat: f64, lon: f64) {
        self.point = (lat, lon);
        self.reset_square();
    }

    /// Участок рисуется заново щелчками.
    pub fn start_drawing(&mut self) {
        self.ring.clear();
        self.drawing = true;
    }

    /// Замкнут ли участок.
    pub fn closed(&self) -> bool {
        !self.drawing && self.ring.len() >= 3
    }

    /// Рамка участка (запад, юг, восток, север): замкнутого - по вершинам,
    /// иначе - квадрат у точки.
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        match ring_box(&self.ring) {
            Some((b, _)) if self.closed() => b,
            _ => square(self.point.0, self.point.1, self.side_km * 1000.0),
        }
    }

    /// Наибольший размер участка, метры.
    pub fn size(&self) -> f64 {
        match ring_box(&self.ring) {
            Some((_, s)) if self.closed() => s,
            _ => self.side_km * 1000.0,
        }
    }

    /// Середина рамки (широта, долгота).
    pub fn centre(&self) -> (f64, f64) {
        let (west, south, east, north) = self.bbox();
        ((south + north) / 2.0, (west + east) / 2.0)
    }

    /// Расстояние от середины рамки до дальней вершины, метры.
    pub fn reach(&self) -> f64 {
        let (lat, lon) = self.centre();
        let scale = lat.to_radians().cos();
        self.ring.iter().map(|p| ((p.0 - lat) * METERS_PER_DEGREE).hypot((p.1 - lon) * METERS_PER_DEGREE * scale)).fold(0.0, f64::max)
    }

    /// Точки внутри участка.
    pub fn contains(&self, points: &[(f64, f64)]) -> Vec<bool> {
        inside(points, &self.ring)
    }

    /// Почему считать нельзя или None.
    pub fn check(&self) -> Option<Problem> {
        if !self.closed() {
            return Some(Problem::Open);
        }
        if self.size() > self.most_km * 1000.0 + 1.0 {
            return Some(Problem::TooLarge(self.most_km));
        }
        None
    }
}
