// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Точки линейки и новой метки, перенос класса `Ruler` из
//! `ui/measure.py` модуля.
//!
//! Точки ставятся щелчками по глобусу, перетаскивание по-прежнему
//! двигает Землю. Точку можно перетащить мышью, Backspace убирает
//! последнюю. За курсором тянется резинка к следующей точке. Длина,
//! периметр и площадь - на эллипсоиде WGS84 (`scene::geodesic`), длина
//! по рельефу - по точкам вдоль линии (`scene::measure`). 3D-виды
//! модуля (точки на крышах зданий) в Lite не перенесены: зданий нет.

use scene::features::Kind;
use scene::{geodesic, measure};

/// Вид линейки или новой метки.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Метка - одна точка, только у новой метки.
    Point,
    Line,
    Path,
    Polygon,
    Circle,
}

impl Mode {
    /// Точек у вида: метка - одна, линия и круг - две.
    fn limit(self) -> Option<usize> {
        match self {
            Self::Point => Some(1),
            Self::Line | Self::Circle => Some(2),
            Self::Path | Self::Polygon => None,
        }
    }

    /// Вид для правки вершин [`scene::editing`].
    pub fn drawing(self) -> scene::editing::Drawing {
        match self {
            Self::Point => scene::editing::Drawing::Point,
            Self::Polygon | Self::Circle => scene::editing::Drawing::Polygon,
            Self::Line | Self::Path => scene::editing::Drawing::Path,
        }
    }
}

/// Фигура для экрана или для «Моих меток»: вид, точки.
#[derive(Debug, Clone, PartialEq)]
pub struct Figure {
    pub kind: Kind,
    pub points: Vec<(f64, f64)>,
}

/// Величины линейки. Нет величины - None. ground_ready - высоты для
/// длины по рельефу все на месте.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Values {
    pub length: Option<f64>,
    pub ground: Option<f64>,
    pub heading: Option<f64>,
    pub perimeter: Option<f64>,
    pub area: Option<f64>,
    pub radius: Option<f64>,
    pub ground_ready: bool,
}

/// Точки линейки.
#[derive(Debug, Clone)]
pub struct Ruler {
    pub mode: Mode,
    pub points: Vec<(f64, f64)>,
    /// Точка под курсором или None. Резинка тянется к ней.
    pub cursor: Option<(f64, f64)>,
    /// Рисование завершено: щелчок точек не добавляет, резинки нет.
    pub finished: bool,
}

impl Ruler {
    pub fn new(mode: Mode) -> Self {
        Self { mode, points: Vec::new(), cursor: None, finished: false }
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.clear();
    }

    pub fn clear(&mut self) {
        self.points.clear();
        self.finished = false;
    }

    /// Щелчок по глобусу: новая точка по правилам вида. Третий щелчок
    /// у линии и круга начинает заново.
    pub fn add(&mut self, lat: f64, lon: f64) {
        if self.finished {
            return;
        }
        if self.mode.limit().is_some_and(|limit| self.points.len() >= limit) {
            self.points.clear();
        }
        self.points.push((lat, lon));
    }

    /// Точку index перетащили.
    pub fn move_to(&mut self, index: usize, lat: f64, lon: f64) {
        if let Some(p) = self.points.get_mut(index) {
            *p = (lat, lon);
        }
    }

    /// Backspace: убрать последнюю точку.
    pub fn remove_last(&mut self) {
        self.points.pop();
        if self.points.is_empty() {
            self.finished = false;
        }
    }

    /// Новая вершина перед точкой index, середина отрезка.
    pub fn insert(&mut self, index: usize, lat: f64, lon: f64) {
        self.points.insert(index.min(self.points.len()), (lat, lon));
    }

    /// Убрать вершину index.
    pub fn remove(&mut self, index: usize) {
        if index < self.points.len() {
            self.points.remove(index);
            if self.points.is_empty() {
                self.finished = false;
            }
        }
    }

    /// Рисование завершено, вершины остаются для правки.
    pub fn finish(&mut self) {
        self.finished = true;
    }

    /// Продолжить рисование: щелчки снова добавляют точки.
    pub fn resume(&mut self) {
        self.finished = false;
    }

    fn rubber(&self, rubber: bool) -> Option<(f64, f64)> {
        let full = self.mode.limit().is_some_and(|limit| self.points.len() >= limit);
        (rubber && !self.finished && !self.points.is_empty() && !full).then_some(self.cursor).flatten()
    }

    /// Точки с резинкой к курсору, если rubber.
    pub fn points(&self, rubber: bool) -> Vec<(f64, f64)> {
        let mut out = self.points.clone();
        out.extend(self.rubber(rubber));
        out
    }

    /// Фигура для глобуса или для «Моих меток», или None.
    pub fn figure(&self, rubber: bool) -> Option<Figure> {
        let points = self.points(rubber);
        match self.mode {
            Mode::Point => points.first().map(|p| Figure { kind: Kind::Point, points: vec![*p] }),
            Mode::Circle => {
                let [center, edge, ..] = points.as_slice() else {
                    return None;
                };
                let radius = geodesic::length(&[*center, *edge]);
                Some(Figure { kind: Kind::Polygon, points: geodesic::ring(center.0, center.1, radius) })
            }
            _ if points.len() < 2 => None,
            Mode::Polygon if points.len() >= 3 => Some(Figure { kind: Kind::Polygon, points }),
            _ => Some(Figure { kind: Kind::Line, points }),
        }
    }

    /// Величины: длина на карте и по рельефу, курс, периметр, площадь,
    /// радиус. heights - высоты точек или None, тогда длины по рельефу
    /// нет. ready - все тайлы высот на месте.
    pub fn values(&self, rubber: bool, heights: Option<(&measure::HeightsAt<'_>, bool)>) -> Values {
        let points = self.points(rubber);
        let mut out = Values { ground_ready: true, ..Values::default() };
        if points.len() < 2 {
            return out;
        }
        match self.mode {
            Mode::Line | Mode::Path => {
                out.length = Some(geodesic::length(&points));
                if let Some((heights_at, ready)) = heights {
                    out.ground = Some(measure::ground_length(&points, heights_at, measure::PROFILE_POINTS));
                    out.ground_ready = ready;
                }
                if self.mode == Mode::Line {
                    out.heading = Some(geodesic::bearing(points[0], points[1]));
                }
            }
            Mode::Circle => {
                let radius = geodesic::length(&points[..2]);
                let (area, perimeter) = geodesic::area_perimeter(&geodesic::ring(points[0].0, points[0].1, radius));
                (out.radius, out.area, out.perimeter) = (Some(radius), Some(area), Some(perimeter));
            }
            Mode::Polygon | Mode::Point => {
                let (area, perimeter) = geodesic::area_perimeter(&points);
                (out.area, out.perimeter) = (Some(area), Some(perimeter));
            }
        }
        out
    }
}

/// Какие строки видны у вида линейки, `SHOWN` модуля.
pub fn shown(mode: Mode) -> &'static [Row] {
    match mode {
        Mode::Line => &[Row::Length, Row::Ground, Row::Heading],
        Mode::Path => &[Row::Length, Row::Ground],
        Mode::Polygon | Mode::Point => &[Row::Perimeter, Row::Area],
        Mode::Circle => &[Row::Radius, Row::Perimeter, Row::Area],
    }
}

/// Строка величины в окне линейки.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Length,
    Ground,
    Heading,
    Perimeter,
    Area,
    Radius,
}

impl Row {
    pub fn value(self, v: &Values) -> Option<f64> {
        match self {
            Self::Length => v.length,
            Self::Ground => v.ground,
            Self::Heading => v.heading,
            Self::Perimeter => v.perimeter,
            Self::Area => v.area,
            Self::Radius => v.radius,
        }
    }

    pub fn is_area(self) -> bool {
        self == Self::Area
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{Mode, Row, Ruler, shown};
    use scene::features::Kind;

    #[test]
    fn line_restarts_on_third_click_and_has_heading() {
        let mut r = Ruler::new(Mode::Line);
        r.add(58.0, 56.0);
        r.add(58.1, 56.2);
        let v = r.values(false, None);
        // Длина и курс - как QgsDistanceArea, `scene::geodesic`.
        assert!((v.length.unwrap() - 16233.59).abs() < 0.01, "{:?}", v.length);
        assert!(v.heading.unwrap() > 40.0 && v.heading.unwrap() < 55.0);
        r.add(59.0, 57.0);
        assert_eq!(r.points, [(59.0, 57.0)]);
    }

    #[test]
    fn rubber_follows_cursor_until_finished() {
        let mut r = Ruler::new(Mode::Path);
        r.add(58.0, 56.0);
        r.cursor = Some((58.0, 56.1));
        assert_eq!(r.points(true).len(), 2);
        assert_eq!(r.figure(true).unwrap().kind, Kind::Line);
        assert!(r.figure(false).is_none());
        r.add(58.0, 56.1);
        r.finish();
        r.add(58.2, 56.2);
        assert_eq!(r.points(true).len(), 2);
        r.resume();
        r.add(58.2, 56.2);
        assert_eq!(r.points.len(), 3);
    }

    #[test]
    fn polygon_area_and_circle_radius() {
        let mut r = Ruler::new(Mode::Polygon);
        for p in [(58.0, 56.0), (58.0, 56.1), (58.05, 56.1), (58.05, 56.0)] {
            r.add(p.0, p.1);
        }
        let v = r.values(false, None);
        assert!((v.area.unwrap() - 32907560.79).abs() < 1.0);
        assert_eq!(r.figure(false).unwrap().kind, Kind::Polygon);
        let mut c = Ruler::new(Mode::Circle);
        c.add(58.0, 56.0);
        c.add(58.0 + 1000.0 / 111_400.0, 56.0);
        let v = c.values(false, None);
        assert!((v.radius.unwrap() - 1000.0).abs() < 10.0);
        // Площадь круга почти πr².
        let ratio = v.area.unwrap() / (std::f64::consts::PI * v.radius.unwrap().powi(2));
        assert!((ratio - 1.0).abs() < 0.001, "{ratio} {v:?}");
        assert_eq!(shown(Mode::Circle), [Row::Radius, Row::Perimeter, Row::Area]);
    }

    #[test]
    fn remove_and_insert() {
        let mut r = Ruler::new(Mode::Path);
        r.add(0.0, 0.0);
        r.add(0.0, 2.0);
        r.insert(1, 0.0, 1.0);
        assert_eq!(r.points, [(0.0, 0.0), (0.0, 1.0), (0.0, 2.0)]);
        r.remove(1);
        r.remove_last();
        r.finish();
        r.remove_last();
        assert!(r.points.is_empty() && !r.finished);
    }
}
