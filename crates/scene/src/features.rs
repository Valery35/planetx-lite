// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Свои объекты глобуса: точки, линии и многоугольники по рельефу.
//! Перенос `core/features.py` модуля и нарезки крыши `buildings.roof`.
//!
//! Объект задаётся широтой и долготой вершин. Линия и контур
//! многоугольника сгущаются по дуге большого круга, чтобы длинный
//! отрезок шёл по поверхности, а не под ней. Точки садятся на рельеф
//! и переводятся в ECEF. В видеокарту уходят смещения от центра объекта
//! в float32, центр остаётся в float64, как у тайлов.
//!
//! Заливка многоугольника режется на треугольники отсечением ушей
//! в касательной плоскости у его середины. Дыры многоугольника
//! (innerBoundaryIs KML) соединяются с контуром мостами, как дворы
//! зданий. Треугольники проходят по высотам вершин контура. В горах
//! середина большого многоугольника может уйти под рельеф, облегание
//! рельефа заливкой не сделано.

use std::borrow::Cow;
use std::collections::BTreeSet;

use globe_core::DVec3;
use globe_core::ellipsoid::{EARTH, Ellipsoid};

use crate::Rgba;

/// Метров между точками сгущения.
pub const STEP: f64 = 100.0;
/// Точек на линию или контур, не больше.
pub const MAX_POINTS: usize = 4000;
/// Точек на объект глобуса, не больше. У мелкого объекта шаг остаётся
/// [`STEP`], у крупного растёт. Сгущение до 4000 точек давало у 29
/// провинций Афганистана 112 тысяч вершин вместо 2.5 тысяч, 28 сентября
/// 2026 года.
pub const SHAPE_POINTS: usize = 800;
/// Метров подъёма метки, не больше.
pub const MAX_HEIGHT: f64 = 100000.0;
/// Доля длины, с которой растущий путь уже виден.
pub const MIN_GROWN: f64 = 1.0e-3;

// Режимы высоты, как altitudeMode KML и вкладка «Высота» Google Earth.
// Поверхность - вода над морем при глубинах, иначе рельеф, дно - рельеф
// с дном моря. absolute - высота над уровнем моря.
pub const CLAMP: &str = "clampToGround";
pub const SEAFLOOR: &str = "clampToSeaFloor";
pub const RELATIVE: &str = "relativeToGround";
pub const RELATIVE_FLOOR: &str = "relativeToSeaFloor";
pub const ABSOLUTE: &str = "absolute";
pub const MODES: [&str; 5] = [CLAMP, SEAFLOOR, RELATIVE, RELATIVE_FLOOR, ABSOLUTE];

/// Вид объекта.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Point,
    Line,
    Polygon,
}

impl Kind {
    /// Имя вида: point, line, polygon.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Point => "point",
            Kind::Line => "line",
            Kind::Polygon => "polygon",
        }
    }

    pub fn parse(text: &str) -> Option<Kind> {
        [Kind::Point, Kind::Line, Kind::Polygon].into_iter().find(|k| k.as_str() == text)
    }
}

/// Вид точки под курсором, вариант highlight StyleMap KML: цвет
/// значка, значок, размер значка, цвет и размер подписи. Пустое поле -
/// как в обычном виде.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Highlight {
    pub color: Option<Rgba>,
    pub icon: Option<String>,
    pub scale: Option<f64>,
    pub label: Option<Rgba>,
    pub label_scale: Option<f64>,
}

/// Объект глобуса.
///
/// points - вершины (широта, долгота) в градусах, у многоугольника без
/// повтора первой. color - цвет линии, width - толщина в логических
/// пикселях, fill - цвет заливки многоугольника или None. name -
/// подпись. height - подъём над рельефом в метрах, как «относительно
/// земли» у Google Earth. extrude - стена от объекта до земли, у точки -
/// стойка. icon - значок точки из [`crate::icons`], окрашенный цветом
/// color. alts - высоты точек над эллипсоидом в метрах или None. С ними
/// объект 3D-линейки стоит в пространстве, как «абсолютная высота» KML:
/// отрезки прямые, на рельеф не садятся, height не действует. flow -
/// по пути бегут штрихи потока. mode - режим высоты KML ([`MODES`]),
/// пустой - прежняя запись: с подъёмом relativeToGround, без него
/// clampToGround. holes - дыры многоугольника: кольца (широта, долгота)
/// без повтора первой вершины или None. icon_scale - размер значка
/// точки, доля обычного. label_color - цвет подписи точки или None -
/// белый, label_scale - размер подписи, доля обычного, 0 - подписи нет.
/// icon_heading - поворот значка по часовой стрелке от севера, градусы.
/// highlight - вид точки под курсором или None.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub kind: Kind,
    pub points: Vec<(f64, f64)>,
    pub color: Rgba,
    pub width: f64,
    pub fill: Option<Rgba>,
    pub name: String,
    pub height: f64,
    pub extrude: bool,
    pub icon: String,
    pub alts: Option<Vec<f64>>,
    pub flow: bool,
    pub mode: String,
    pub holes: Option<Vec<Vec<(f64, f64)>>>,
    pub icon_scale: f64,
    pub label_color: Option<Rgba>,
    pub label_scale: f64,
    pub icon_heading: f64,
    pub highlight: Option<Highlight>,
}

impl Shape {
    /// Объект со свойствами по умолчанию: жёлтая линия толщиной 2,
    /// на земле, без заливки.
    pub fn new(kind: Kind, points: Vec<(f64, f64)>) -> Self {
        Self {
            kind,
            points,
            color: [255, 255, 0, 255],
            width: 2.0,
            fill: None,
            name: String::new(),
            height: 0.0,
            extrude: false,
            icon: crate::icons::DEFAULT.into(),
            alts: None,
            flow: false,
            mode: String::new(),
            holes: None,
            icon_scale: 1.0,
            label_color: None,
            label_scale: 1.0,
            icon_heading: 0.0,
            highlight: None,
        }
    }
}

/// Режим высоты объекта из [`MODES`].
pub fn altitude_mode(shape: &Shape) -> &str {
    if MODES.contains(&shape.mode.as_str()) {
        return &shape.mode;
    }
    if shape.height > 0.0 { RELATIVE } else { CLAMP }
}

/// Действует ли высота метки в режиме mode.
pub fn uses_height(mode: &str) -> bool {
    [RELATIVE, RELATIVE_FLOOR, ABSOLUTE].contains(&mode)
}

/// Отсчёт от дна моря, а не от поверхности.
pub fn on_floor(mode: &str) -> bool {
    [SEAFLOOR, RELATIVE_FLOOR].contains(&mode)
}

fn unit(lat: f64, lon: f64) -> DVec3 {
    let (sl, cl) = lat.to_radians().sin_cos();
    let (so, co) = lon.to_radians().sin_cos();
    DVec3::new(cl * co, cl * so, sl)
}

fn latlon(u: DVec3) -> (f64, f64) {
    (u.z.atan2(u.x.hypot(u.y)).to_degrees(), u.y.atan2(u.x).to_degrees())
}

fn arc(p: DVec3, q: DVec3) -> f64 {
    p.dot(q).clamp(-1.0, 1.0).acos()
}

/// Вершины со вставками по дуге большого круга.
///
/// Шаг step метров. Если точек вышло бы больше max_points, шаг растёт.
/// closed - контур, последняя вершина соединяется с первой.
pub fn densify(points: &[(f64, f64)], closed: bool, step: f64, max_points: usize) -> Vec<(f64, f64)> {
    if points.len() < 2 {
        return points.to_vec();
    }
    let mut ring = points.to_vec();
    if closed {
        ring.push(points[0]);
    }
    let units: Vec<DVec3> = ring.iter().map(|p| unit(p.0, p.1)).collect();
    let angles: Vec<f64> = units.windows(2).map(|w| arc(w[0], w[1])).collect();
    let total = angles.iter().sum::<f64>() * EARTH.a;
    let room = (max_points as i64 - ring.len() as i64).max(1) as f64;
    let step = step.max(total / room);
    let mut out = Vec::new();
    for (i, angle) in angles.iter().enumerate() {
        let (p, q) = (units[i], units[i + 1]);
        let n = ((angle * EARTH.a / step).ceil()).max(1.0) as usize;
        out.push(ring[i]); // исходная вершина точно, без перевода туда-обратно
        for k in 1..n {
            let t = k as f64 / n as f64;
            let u = if *angle < 1e-12 {
                p
            } else {
                (p * ((1.0 - t) * angle).sin() + q * (t * angle).sin()) / angle.sin()
            };
            out.push(latlon(u));
        }
    }
    if !closed && let Some(last) = ring.last() {
        out.push(*last);
    }
    out
}

/// Места исходных вершин pts в сгущённом контуре ring. Вершины стоят
/// в контуре по порядку, каждая - первой точкой своей стороны,
/// [`densify`] ставит их точно.
fn starts(ring: &[(f64, f64)], pts: &[(f64, f64)]) -> Vec<usize> {
    let mut out = Vec::with_capacity(pts.len());
    let mut position = 0;
    for vertex in pts {
        while position < ring.len() && ring[position] != *vertex {
            position += 1;
        }
        out.push(position);
    }
    out
}

fn pick(starts: &[usize], tri: &[u32]) -> Vec<u32> {
    tri.iter().map(|i| starts.get(*i as usize).map_or(u32::MAX, |s| *s as u32)).collect()
}

/// Сгущённый контур многоугольника и треугольники заливки.
///
/// Режутся только исходные вершины, номера переносятся на сгущённый
/// контур: вершина i стоит в нём в начале своей стороны. Резка всего
/// сгущённого контура шла 1.1 с на 800 точек при каждом движении мыши,
/// у больших многоугольников - минуты, 27 сентября 2026 года. Точки
/// между вершинами лежат на сторонах и в заливку не входят.
pub fn fill(points: &[(f64, f64)], step: f64, max_points: usize) -> (Vec<(f64, f64)>, Vec<u32>) {
    let ring = densify(points, true, step, max_points);
    let at = starts(&ring, points);
    let tri = triangulate(&plane(points));
    let numbers = pick(&at, &tri);
    (ring, numbers)
}

/// Дыры многоугольника не меньше чем из трёх вершин.
pub fn holes_of(shape: &Shape) -> Vec<Vec<(f64, f64)>> {
    if shape.kind != Kind::Polygon {
        return Vec::new();
    }
    shape.holes.iter().flatten().filter(|h| h.len() >= 3).cloned().collect()
}

/// Заливка многоугольника с дырами, номера вершин колец подряд (контур,
/// потом дыры).
///
/// Нарезка - как крыша здания: контур против часовой стрелки, дыры
/// по часовой, мосты от дыр к контуру. Дыра без моста закрыта заливкой,
/// как двор здания.
pub fn holed_triangles(rings: &[Vec<(f64, f64)>]) -> Vec<u32> {
    let all: Vec<(f64, f64)> = rings.iter().flatten().copied().collect();
    let points = plane(&all);
    let mut numbers = Vec::with_capacity(rings.len());
    let mut offset = 0;
    for (n, ring) in rings.iter().enumerate() {
        let mut index: Vec<usize> = (offset..offset + ring.len()).collect();
        offset += ring.len();
        let part: Vec<(f64, f64)> = index.iter().map(|i| points[*i]).collect();
        let outward = area2(&part) > 0.0;
        if outward != (n == 0) {
            index.reverse();
        }
        numbers.push(index);
    }
    roof(&points, &numbers)
}

/// Стены колец подряд: верх кольца - его вершины, низ - те же номера
/// плюс total.
fn ring_walls(sizes: &[usize], total: usize) -> Vec<u32> {
    let mut out = Vec::new();
    let mut offset = 0;
    for size in sizes {
        for k in 0..*size {
            let a = (offset + k) as u32;
            let b = (offset + (k + 1) % size) as u32;
            let t = total as u32;
            out.extend([a, b, b + t, a, b + t, a + t]);
        }
        offset += size;
    }
    out
}

/// Контур многоугольника с дырами: кольца подряд, отрезки каждого
/// кольца, заливка мостами, стены по всем кольцам.
fn holed(shape: &Shape, raised: bool) -> Geometry {
    let mut rings = vec![shape.points.clone()];
    rings.extend(holes_of(shape));
    let budget = (SHAPE_POINTS / rings.len()).max(16);
    let (mut ring, mut at, mut lines, mut sizes) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut offset = 0;
    for points in &rings {
        let dense = densify(points, true, STEP, budget);
        at.extend(starts(&dense, points).into_iter().map(|s| offset + s));
        lines.extend(segments(dense.len(), true).into_iter().map(|i| i + offset as u32));
        sizes.push(dense.len());
        offset += dense.len();
        ring.extend(dense);
    }
    let triangles = if shape.fill.is_some() { pick(&at, &holed_triangles(&rings)) } else { Vec::new() };
    let wall = if raised { ring_walls(&sizes, offset) } else { Vec::new() };
    Geometry { ring, lines, triangles, wall, stem: false, alts: None }
}

/// Высоты рельефа массива точек одним вызовом: широты, долготы.
pub type HeightsAt<'a> = &'a dyn Fn(&[f64], &[f64]) -> Vec<f64>;
/// Высота рельефа одной точки.
pub type HeightAt<'a> = &'a dyn Fn(f64, f64) -> f64;

/// Точки (широта, долгота) на рельефе плюс offset метров в ECEF.
///
/// heights_at - высоты массива точек одним вызовом, у него приоритет
/// перед height_at по одной точке.
pub fn lift(latlon: &[(f64, f64)], height_at: Option<HeightAt<'_>>, offset: f64, heights_at: Option<HeightsAt<'_>>) -> Vec<DVec3> {
    let heights: Vec<f64> = match (heights_at, height_at) {
        (Some(batch), _) => {
            let lats: Vec<f64> = latlon.iter().map(|p| p.0).collect();
            let lons: Vec<f64> = latlon.iter().map(|p| p.1).collect();
            batch(&lats, &lons)
        }
        (None, Some(one)) => latlon.iter().map(|p| one(p.0, p.1)).collect(),
        (None, None) => Vec::new(),
    };
    let e = Ellipsoid::wgs84();
    latlon
        .iter()
        .enumerate()
        .map(|(i, p)| e.geodetic_to_ecef(p.0, p.1, heights.get(i).copied().unwrap_or(0.0) + offset))
        .collect()
}

/// Место ползунка высоты от 0 (земля) до 1 ([`MAX_HEIGHT`]), шкала
/// логарифмическая, как ползунок «Поверхность земли - Космос» Google
/// Earth. Метры у земли и километры в небе ставятся одинаково точно.
pub fn height_share(height: f64) -> f64 {
    let height = height.clamp(0.0, MAX_HEIGHT);
    (height + 1.0).log10() / (MAX_HEIGHT + 1.0).log10()
}

/// Высота по месту ползунка, обратная [`height_share`].
pub fn share_height(share: f64) -> f64 {
    (MAX_HEIGHT + 1.0).powf(share.clamp(0.0, 1.0)) - 1.0
}

/// Контур объекта для видеокарты, от высот не зависит.
///
/// ring - вершины (широта, долгота) после сгущения. lines, triangles,
/// wall - индексы отрезков, заливки и стены. У выдавленного объекта
/// вершины идут дважды: поднятые, потом на земле. stem - стойка точки.
/// alts - высоты вершин над эллипсоидом у 3D-объекта, иначе None.
#[derive(Debug, Clone, PartialEq)]
pub struct Geometry {
    pub ring: Vec<(f64, f64)>,
    pub lines: Vec<u32>,
    pub triangles: Vec<u32>,
    pub wall: Vec<u32>,
    pub stem: bool,
    pub alts: Option<Vec<f64>>,
}

/// Путь, выросший на долю share своей длины, от 0 до 1. Доля 1, не
/// линия и 3D-объект - сам shape. Отрезок режется по той же дуге,
/// по какой его рисует [`densify`].
pub fn grown(shape: &Shape, share: f64) -> Cow<'_, Shape> {
    if share >= 1.0 || shape.kind != Kind::Line || shape.points.len() < 2 || shape.alts.is_some() {
        return Cow::Borrowed(shape);
    }
    let units: Vec<DVec3> = shape.points.iter().map(|p| unit(p.0, p.1)).collect();
    let arcs: Vec<f64> = units.windows(2).map(|w| arc(w[0], w[1])).collect();
    let total: f64 = arcs.iter().sum();
    if total.is_nan() || total <= 0.0 {
        return Cow::Borrowed(shape);
    }
    let target = share.max(MIN_GROWN) * total;
    let ends: Vec<f64> = arcs.iter().scan(0.0, |s, a| {
        *s += a;
        Some(*s)
    }).collect();
    let n = ends.iter().position(|e| *e >= target).unwrap_or(ends.len()).min(arcs.len() - 1);
    let done = if n > 0 { ends[n - 1] } else { 0.0 };
    let piece = arcs[n];
    let mut points: Vec<(f64, f64)> = shape.points[..=n].to_vec();
    if piece > 0.0 {
        let t = ((target - done) / piece).clamp(0.0, 1.0);
        let s = piece.sin();
        let u = if s > 1.0e-12 {
            (units[n] * ((1.0 - t) * piece).sin() + units[n + 1] * (t * piece).sin()) / s
        } else {
            units[n]
        };
        points.push(latlon(u));
    }
    if points.len() < 2 {
        points.push(points[0]);
    }
    let mut out = shape.clone();
    out.points = points;
    Cow::Owned(out)
}

/// Стоит ли объект в пространстве по своим высотам.
pub fn has_alts(shape: &Shape) -> bool {
    shape.alts.as_ref().is_some_and(|a| !a.is_empty() && a.len() == shape.points.len())
}

/// Контур объекта или None для точки без стойки и пустого объекта.
pub fn geometry(shape: &Shape) -> Option<Geometry> {
    let raised = shape.extrude && uses_height(altitude_mode(shape));
    if shape.kind == Kind::Point {
        if !raised || shape.points.is_empty() {
            return None;
        }
        let ring = shape.points[..1].to_vec();
        return Some(Geometry { ring, lines: vec![0, 1], triangles: Vec::new(), wall: Vec::new(), stem: true, alts: None });
    }
    if shape.points.len() < 2 {
        return None;
    }
    let closed = shape.kind == Kind::Polygon && shape.points.len() >= 3;
    if has_alts(shape) {
        // Прямые отрезки в пространстве, без сгущения по дуге.
        let ring = shape.points.clone();
        let triangles = if closed && shape.fill.is_some() { triangulate(&plane(&ring)) } else { Vec::new() };
        let lines = segments(ring.len(), closed);
        return Some(Geometry { ring, lines, triangles, wall: Vec::new(), stem: false, alts: shape.alts.clone() });
    }
    if closed && !holes_of(shape).is_empty() {
        return Some(holed(shape, raised));
    }
    let (ring, triangles) = if closed && shape.fill.is_some() {
        fill(&shape.points, STEP, SHAPE_POINTS)
    } else {
        (densify(&shape.points, closed, STEP, SHAPE_POINTS), Vec::new())
    };
    let wall = if raised { walls(ring.len(), closed) } else { Vec::new() };
    let lines = segments(ring.len(), closed);
    Some(Geometry { ring, lines, triangles, wall, stem: false, alts: None })
}

/// Вершины контура в ECEF: поднятые, у стены и стойки ещё земля.
///
/// heights_at - рельеф с дном моря, surface_at - поверхность (вода над
/// морем), без неё - тот же рельеф. scale - масштаб рельефа на экране,
/// им растягивается абсолютная высота. Стена и стойка идут до
/// поверхности.
pub fn vertices(
    geo: &Geometry,
    height: f64,
    heights_at: Option<HeightsAt<'_>>,
    mode: &str,
    surface_at: Option<HeightsAt<'_>>,
    scale: f64,
) -> Vec<DVec3> {
    if let Some(alts) = &geo.alts {
        let e = Ellipsoid::wgs84();
        return geo
            .ring
            .iter()
            .enumerate()
            .map(|(i, p)| e.geodetic_to_ecef(p.0, p.1, alts.get(i).copied().unwrap_or(0.0)))
            .collect();
    }
    let surface = surface_at.or(heights_at);
    let mut top = if mode == ABSOLUTE {
        lift(&geo.ring, None, height * scale, None)
    } else {
        let offset = if uses_height(mode) { height } else { 0.0 };
        lift(&geo.ring, None, offset, if on_floor(mode) { heights_at } else { surface })
    };
    if !geo.wall.is_empty() || geo.stem {
        top.extend(lift(&geo.ring, None, 0.0, surface));
    }
    top
}

/// Треугольники стены между верхом и низом.
///
/// Вершины 0..n-1 - поднятая линия, n..2n-1 - те же точки на земле.
/// На отрезок - два треугольника.
pub fn walls(n: usize, closed: bool) -> Vec<u32> {
    if n < 2 {
        return Vec::new();
    }
    let mut pairs: Vec<(u32, u32)> = (0..n - 1).map(|a| (a as u32, a as u32 + 1)).collect();
    if closed {
        pairs.push((n as u32 - 1, 0));
    }
    let m = n as u32;
    pairs.into_iter().flat_map(|(a, b)| [a, b, b + m, a, b + m, a + m]).collect()
}

/// Центр в float64 и смещения от него в float32.
pub fn centered(xyz: &[DVec3]) -> (DVec3, Vec<[f32; 3]>) {
    let Some(first) = xyz.first() else {
        return (DVec3::ZERO, Vec::new());
    };
    let (lo, hi) = xyz.iter().fold((*first, *first), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let center = (lo + hi) * 0.5;
    (center, xyz.iter().map(|p| (*p - center).as_vec3().to_array()).collect())
}

/// Индексы отрезков ломаной из n точек для линий видеокарты.
pub fn segments(n: usize, closed: bool) -> Vec<u32> {
    if n < 2 {
        return Vec::new();
    }
    let mut out: Vec<u32> = (0..n as u32 - 1).flat_map(|a| [a, a + 1]).collect();
    if closed {
        out.extend([n as u32 - 1, 0]);
    }
    out
}

/// Удвоенная площадь со знаком: больше нуля - обход против часовой.
pub fn area2(p: &[(f64, f64)]) -> f64 {
    let n = p.len();
    (0..n).map(|i| p[i].0 * p[(i + 1) % n].1 - p[(i + 1) % n].0 * p[i].1).sum()
}

/// Точки в касательной плоскости у середины, метры: восток, север.
pub fn plane(latlon: &[(f64, f64)]) -> Vec<(f64, f64)> {
    if latlon.is_empty() {
        return Vec::new();
    }
    let count = latlon.len() as f64;
    let lat0 = latlon.iter().map(|p| p.0).sum::<f64>() / count;
    let lon0 = latlon.iter().map(|p| p.1).sum::<f64>() / count;
    let k = lat0.to_radians().cos();
    latlon
        .iter()
        .map(|p| {
            let dlon = (p.1 - lon0 + 180.0).rem_euclid(360.0) - 180.0;
            (dlon.to_radians() * EARTH.a * k, (p.0 - lat0).to_radians() * EARTH.a)
        })
        .collect()
}

fn side(u: (f64, f64), v: (f64, f64), w: (f64, f64)) -> f64 {
    (v.0 - u.0) * (w.1 - u.1) - (v.1 - u.1) * (w.0 - u.0)
}

/// Лежит ли точка p в треугольнике abc против часовой стрелки.
pub fn inside(p: (f64, f64), a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> bool {
    side(a, b, p) >= 0.0 && side(b, c, p) >= 0.0 && side(c, a, p) >= 0.0
}

/// Треугольники простого многоугольника отсечением ушей, тройки номеров
/// подряд.
///
/// points - вершины на плоскости без повтора первой, в любом обходе.
/// Многоугольник без дыр и самопересечений.
pub fn triangulate(points: &[(f64, f64)]) -> Vec<u32> {
    let n = points.len();
    if n < 3 {
        return Vec::new();
    }
    let mut order: Vec<usize> = if area2(points) > 0.0 { (0..n).collect() } else { (0..n).rev().collect() };
    let mut out = Vec::new();
    let mut guard = 0;
    while order.len() > 3 && guard < 2 * n * n {
        guard += 1;
        let m = order.len();
        let mut cut = false;
        for i in 0..m {
            let (ia, ib, ic) = (order[(i + m - 1) % m], order[i], order[(i + 1) % m]);
            let (a, b, c) = (points[ia], points[ib], points[ic]);
            let cross = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
            if cross <= 0.0 || cross.is_nan() {
                continue; // вогнутая вершина или вырожденная
            }
            if order.iter().any(|j| ![ia, ib, ic].contains(j) && inside(points[*j], a, b, c)) {
                continue;
            }
            out.extend([ia as u32, ib as u32, ic as u32]);
            order.remove(i);
            cut = true;
            break;
        }
        if !cut {
            break; // многоугольник неправильный, остаток не режется
        }
    }
    if order.len() == 3 {
        out.extend(order.iter().map(|i| *i as u32));
    }
    out
}

// Нарезка крыши с дворами, перенос `buildings.roof` модуля.

fn orient(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> f64 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

/// Треугольники кольца против часовой стрелки, тройки номеров.
///
/// Ухо проверяется только по вогнутым вершинам. Точки, совпавшие
/// с вершинами уха, его не закрывают, так проходят мосты к дворам. Если
/// ухо не нашлось, остаток режется веером.
fn ear_clip(points: &[(f64, f64)]) -> Vec<[usize; 3]> {
    let n = points.len();
    if n < 3 {
        return Vec::new();
    }
    let mut prev: Vec<usize> = (0..n).map(|i| (i + n - 1) % n).collect();
    let mut nxt: Vec<usize> = (0..n).map(|i| (i + 1) % n).collect();
    let cross = |a: usize, b: usize, c: usize| {
        let (pa, pb, pc) = (points[a], points[b], points[c]);
        (pb.0 - pa.0) * (pc.1 - pb.1) - (pb.1 - pa.1) * (pc.0 - pb.0)
    };
    let mut reflex: BTreeSet<usize> = (0..n).filter(|i| cross(prev[*i], *i, nxt[*i]) <= 0.0).collect();
    let blocked = |a: usize, b: usize, c: usize, reflex: &BTreeSet<usize>| {
        let (pa, pb, pc) = (points[a], points[b], points[c]);
        let (left, right) = (pa.0.min(pb.0).min(pc.0), pa.0.max(pb.0).max(pc.0));
        let (low, high) = (pa.1.min(pb.1).min(pc.1), pa.1.max(pb.1).max(pc.1));
        reflex.iter().any(|r| {
            let p = points[*r];
            if p.0 < left || p.0 > right || p.1 < low || p.1 > high {
                return false;
            }
            if p == pa || p == pb || p == pc {
                return false;
            }
            orient(pa, pb, p) >= 0.0 && orient(pb, pc, p) >= 0.0 && orient(pc, pa, p) >= 0.0
        })
    };
    let mut out = Vec::new();
    let mut remaining = n;
    let mut i = 0;
    let mut stall = 0;
    while remaining > 3 && stall < remaining {
        let (a, c) = (prev[i], nxt[i]);
        if !reflex.contains(&i) && !blocked(a, i, c, &reflex) {
            out.push([a, i, c]);
            nxt[a] = c;
            prev[c] = a;
            remaining -= 1;
            for v in [a, c] {
                if reflex.contains(&v) && cross(prev[v], v, nxt[v]) > 0.0 {
                    reflex.remove(&v);
                }
            }
            i = c;
            stall = 0;
        } else {
            i = nxt[i];
            stall += 1;
        }
    }
    let a = i;
    let mut b = nxt[a];
    let mut c = nxt[b];
    let mut guard = 0;
    while c != a && guard < n {
        out.push([a, b, c]);
        b = c;
        c = nxt[c];
        guard += 1;
    }
    out
}

/// Пересекает ли отрезок pq какой-нибудь из отрезков внутри или проходит
/// ли через вершину, кроме своих концов. Мост через вершину двора
/// ложится на его сторону, нарезка на таком кольце вставала. Так было
/// у здания в Берлине, 29 сентября 2026 года.
fn crosses(p: (f64, f64), q: (f64, f64), starts: &[(f64, f64)], ends: &[(f64, f64)]) -> bool {
    let len2 = (q.0 - p.0).powi(2) + (q.1 - p.1).powi(2);
    let through = starts.iter().zip(ends).any(|(s, e)| {
        let (d1, d2, d3, d4) = (orient(*s, *e, p), orient(*s, *e, q), orient(p, q, *s), orient(p, q, *e));
        d1 * d2 < 0.0 && d3 * d4 < 0.0
    });
    through
        || starts.iter().any(|s| {
            let along = (s.0 - p.0) * (q.0 - p.0) + (s.1 - p.1) * (q.1 - p.1);
            orient(p, q, *s) == 0.0 && along > 0.0 && along < len2
        })
}

/// Контур с дворами одним кольцом через мосты, номера точек.
///
/// outer - номера точек контура против часовой стрелки, holes - номера
/// точек дворов по часовой. Мост идёт от самой правой точки двора
/// к ближайшей точке кольца, если он не пересекает стороны. Двор без
/// такого моста пропускается, крыша тогда его закрывает.
fn bridge(points: &[(f64, f64)], outer: &[usize], holes: &[Vec<usize>]) -> Vec<usize> {
    let mut ring = outer.to_vec();
    let right = |hole: &Vec<usize>| hole.iter().map(|i| points[*i].0).fold(f64::NEG_INFINITY, f64::max);
    let mut holes: Vec<&Vec<usize>> = holes.iter().filter(|h| !h.is_empty()).collect();
    holes.sort_by(|a, b| right(b).total_cmp(&right(a)));
    for number in 0..holes.len() {
        let hole = holes[number];
        let mut m = hole[0];
        for i in hole {
            if points[*i].0 > points[m].0 {
                m = *i;
            }
        }
        let mut edges: Vec<&[usize]> = vec![&ring];
        edges.extend(holes[number..].iter().map(|h| h.as_slice()));
        let starts: Vec<(f64, f64)> = edges.iter().flat_map(|e| e.iter().map(|i| points[*i])).collect();
        let ends: Vec<(f64, f64)> =
            edges.iter().flat_map(|e| (0..e.len()).map(|k| points[e[(k + 1) % e.len()]])).collect();
        let pm = points[m];
        let mut order: Vec<usize> = (0..ring.len()).collect();
        let dist = |k: usize| (points[ring[k]].0 - pm.0).powi(2) + (points[ring[k]].1 - pm.1).powi(2);
        order.sort_by(|a, b| dist(*a).total_cmp(&dist(*b)));
        for position in order {
            let v = ring[position];
            if !crosses(pm, points[v], &starts, &ends) {
                let start = hole.iter().position(|i| *i == m).unwrap_or(0);
                let mut lane: Vec<usize> = hole[start..].to_vec();
                lane.extend_from_slice(&hole[..=start]);
                let mut next = ring[..=position].to_vec();
                next.extend(lane);
                next.extend_from_slice(&ring[position..]);
                ring = next;
                break;
            }
        }
    }
    ring
}

fn cut(points: &[(f64, f64)], ring: &[usize]) -> Vec<u32> {
    let part: Vec<(f64, f64)> = ring.iter().map(|i| points[*i]).collect();
    ear_clip(&part).into_iter().flat_map(|t| t.map(|k| ring[k] as u32)).collect()
}

/// Треугольники крыши, номера точек.
///
/// rings - списки номеров точек колец, первое - контур. Если с дворами
/// вышел вывернутый треугольник, крыша режется по контуру без дворов
/// и закрывает их. Вывернутый треугольник лёг бы за контур.
fn roof(points: &[(f64, f64)], rings: &[Vec<usize>]) -> Vec<u32> {
    let Some(outer) = rings.first() else {
        return Vec::new();
    };
    if rings.len() > 1 {
        let tri = cut(points, &bridge(points, outer, &rings[1..]));
        let flipped = tri.as_chunks::<3>().0.iter().any(|t| {
            orient(points[t[0] as usize], points[t[1] as usize], points[t[2] as usize]) < 0.0
        });
        if !flipped {
            return tri;
        }
    }
    cut(points, outer)
}
