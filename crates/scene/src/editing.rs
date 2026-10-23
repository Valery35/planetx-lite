// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Правка вершин пути и многоугольника на экране, перенос
//! `core/editing.py` модуля.
//!
//! Вершины и середины отрезков приходят уже в пикселях кадра. Модуль
//! решает, что под курсором, куда встаёт новая вершина и когда щелчок
//! по вершине завершает рисование. Попадание курсора в сохранённый
//! объект считается здесь же, в пикселях.

/// Точка экрана и видна ли она: перед камерой.
pub type Pixel = ([f64; 2], bool);

/// Что под курсором.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Picked {
    /// Вершина номер.
    Vertex(usize),
    /// Середина отрезка номер.
    Middle(usize),
}

/// Чем кончается щелчок по вершине.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    /// Щелчок по первой вершине: фигура замыкается.
    Close,
    /// Щелчок по последней вершине: путь завершён.
    Finish,
}

/// Что рисуется.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drawing {
    Point,
    Path,
    Polygon,
}

fn nearest(points: &[Pixel], px: f64, py: f64, radius: f64) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .filter(|(_, (_, front))| *front)
        .map(|(i, ([x, y], _))| (i, (x - px).hypot(y - py)))
        .filter(|(_, d)| *d <= radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Что под курсором: вершина, середина отрезка или None. Вершина
/// старше середины, если обе в радиусе.
pub fn pick(vertices: &[Pixel], middles: &[Pixel], px: f64, py: f64, radius: f64) -> Option<Picked> {
    nearest(vertices, px, py, radius).map(Picked::Vertex).or_else(|| nearest(middles, px, py, radius).map(Picked::Middle))
}

/// Точки и высоты с новой вершиной на отрезке segment. Отрезок segment
/// идёт от вершины segment к следующей, последний отрезок замкнутой
/// фигуры - к первой.
pub fn inserted<P: Clone, A: Clone>(points: &[P], alts: &[A], segment: usize, point: P, alt: A) -> (Vec<P>, Vec<A>) {
    let at = (segment + 1).min(points.len());
    let mut p = points.to_vec();
    p.insert(at, point);
    let mut a = alts.to_vec();
    a.insert(at.min(a.len()), alt);
    (p, a)
}

/// Точки и высоты без вершины index.
pub fn removed<P: Clone, A: Clone>(points: &[P], alts: &[A], index: usize) -> (Vec<P>, Vec<A>) {
    (without(points, index), without(alts, index))
}

fn without<T: Clone>(items: &[T], index: usize) -> Vec<T> {
    items.iter().enumerate().filter(|(i, _)| *i != index).map(|(_, x)| x.clone()).collect()
}

/// Чем кончается щелчок по вершине index без перетаскивания: Close,
/// Finish или None. count - количество вершин.
pub fn click_result(mode: Drawing, count: usize, index: usize, finished: bool) -> Option<Click> {
    if finished || mode == Drawing::Point {
        return None;
    }
    if index == 0 && count >= 3 {
        return Some(Click::Close);
    }
    (mode == Drawing::Path && count >= 2 && index + 1 == count).then_some(Click::Finish)
}

/// Расстояние в пикселях от курсора до ломаной. Отрезок с концом за
/// камерой не считается.
pub fn segment_distance(points: &[Pixel], px: f64, py: f64, closed: bool) -> f64 {
    if points.len() < 2 {
        return f64::INFINITY;
    }
    let mut pairs: Vec<(&Pixel, &Pixel)> = points.windows(2).filter_map(|w| Some((w.first()?, w.get(1)?))).collect();
    if closed && points.len() >= 3
        && let (Some(last), Some(first)) = (points.last(), points.first())
    {
        pairs.push((last, first));
    }
    pairs
        .into_iter()
        .filter(|((_, fa), (_, fb))| *fa && *fb)
        .map(|(([ax, ay], _), ([bx, by], _))| {
            let (dx, dy) = (bx - ax, by - ay);
            let length = dx * dx + dy * dy;
            let length = if length == 0.0 { 1.0 } else { length };
            let t = (((px - ax) * dx + (py - ay) * dy) / length).clamp(0.0, 1.0);
            (ax + dx * t - px).hypot(ay + dy * t - py)
        })
        .fold(f64::INFINITY, f64::min)
}

/// Лежит ли курсор внутри многоугольника на экране, правило чётности.
pub fn inside(points: &[[f64; 2]], px: f64, py: f64) -> bool {
    if points.len() < 3 {
        return false;
    }
    let mut odd = false;
    for (i, [x1, y1]) in points.iter().enumerate() {
        let [x2, y2] = points[(i + 1) % points.len()];
        if (*y1 > py) != (y2 > py) {
            let dy = if y2 == *y1 { 1.0 } else { y2 - y1 };
            if px < x1 + (py - y1) * (x2 - x1) / dy {
                odd = !odd;
            }
        }
    }
    odd
}

/// Вид объекта для попадания.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Point,
    Line,
    Polygon,
}

/// Попал ли курсор в объект вида kind.
pub fn hit(kind: Shape, points: &[Pixel], px: f64, py: f64, radius: f64) -> bool {
    if !points.iter().any(|(_, front)| *front) {
        return false;
    }
    match kind {
        Shape::Point => nearest(points.get(..1).unwrap_or_default(), px, py, radius).is_some(),
        Shape::Line => segment_distance(points, px, py, false) <= radius,
        Shape::Polygon => {
            segment_distance(points, px, py, true) <= radius
                || (points.iter().all(|(_, front)| *front) && inside(&points.iter().map(|(p, _)| *p).collect::<Vec<_>>(), px, py))
        }
    }
}
