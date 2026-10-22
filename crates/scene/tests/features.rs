// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Свои объекты глобуса: сгущение, посадка, треугольники, дыры, режимы
//! высоты. Перенос `tests/test_features.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::borrow::Cow;

use globe_core::DVec3;
use globe_core::ellipsoid::{EARTH, Ellipsoid};
use scene::features::{self as ft, Kind, Shape};

fn close(a: f64, b: f64, places: i32) -> bool {
    (a - b).abs() < 0.5 * 10f64.powi(-places)
}

fn dense(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    ft::densify(points, false, ft::STEP, ft::MAX_POINTS)
}

#[test]
fn step_along_arc() {
    // Градус меридиана - около 111 км, при шаге 100 м ~1113 точек.
    let out = dense(&[(58.0, 56.0), (59.0, 56.0)]);
    assert_eq!(out[0], (58.0, 56.0));
    assert!(close(out[out.len() - 1].0, 59.0, 7));
    assert!(1100 < out.len() && out.len() < 1130, "{}", out.len());
    assert!(out.windows(2).all(|w| w[1].0 > w[0].0));
}

#[test]
fn great_circle_bulges_to_pole() {
    // Дуга между точками одной параллели идёт севернее её.
    let out = ft::densify(&[(60.0, 0.0), (60.0, 40.0)], false, 10000.0, ft::MAX_POINTS);
    assert!(out.iter().map(|p| p.0).fold(f64::MIN, f64::max) > 61.0);
}

#[test]
fn point_cap() {
    let out = ft::densify(&[(0.0, 0.0), (0.0, 90.0)], false, ft::STEP, 500);
    assert!(out.len() <= 500);
}

#[test]
fn closed_ring_returns_to_start_without_repeat() {
    let ring = [(0.0, 0.0), (0.0, 0.01), (0.01, 0.01)];
    let out = ft::densify(&ring, true, 200.0, ft::MAX_POINTS);
    assert_eq!(out[0], (0.0, 0.0));
    let last = out[out.len() - 1];
    assert!(last.0.abs() > 1e-8 || last.1.abs() > 1e-8);
}

#[test]
fn short() {
    assert_eq!(dense(&[(1.0, 2.0)]).len(), 1);
}

#[test]
fn heights_and_center() {
    let latlon = [(58.0, 56.0), (58.01, 56.01)];
    let xyz = ft::lift(&latlon, Some(&|_, _| 150.0), 0.0, None);
    let (_, _, h) = Ellipsoid::wgs84().ecef_to_geodetic(xyz[0]);
    assert!(close(h, 150.0, 3));
    let (center, offsets) = ft::centered(&xyz);
    // Смещения малы, float32 их держит до сантиметров.
    let worst = xyz
        .iter()
        .zip(&offsets)
        .map(|(p, o)| (center + DVec3::new(f64::from(o[0]), f64::from(o[1]), f64::from(o[2])) - *p).abs().max_element())
        .fold(0.0, f64::max);
    assert!(worst < 0.01);
}

#[test]
fn offset_above_terrain() {
    let xyz = ft::lift(&[(58.0, 56.0)], Some(&|_, _| 150.0), 40.0, None);
    assert!(close(Ellipsoid::wgs84().ecef_to_geodetic(xyz[0]).2, 190.0, 3));
}

#[test]
fn wall_quads() {
    // Линия из трёх точек: два отрезка, по два треугольника.
    let tri = ft::walls(3, false);
    assert_eq!(tri.len(), 12);
    assert_eq!(tri[..6], [0, 1, 4, 0, 4, 3]);
    // Замкнутый контур добавляет стену от последней точки к первой.
    let closed = ft::walls(3, true);
    assert_eq!(closed.len(), 18);
    assert_eq!(closed[12..], [2, 0, 3, 2, 3, 5]);
    assert!(ft::walls(1, false).is_empty());
}

#[test]
fn shape_defaults_on_ground() {
    let shape = Shape::new(Kind::Line, vec![(58.0, 56.0), (58.1, 56.1)]);
    assert_eq!((shape.height, shape.extrude), (0.0, false));
}

#[test]
fn open_and_closed() {
    assert_eq!(ft::segments(3, false), [0, 1, 1, 2]);
    assert_eq!(ft::segments(3, true), [0, 1, 1, 2, 2, 0]);
}

fn area(p: &[(f64, f64)], tri: &[u32]) -> f64 {
    tri.as_chunks::<3>()
        .0
        .iter()
        .map(|t| {
            let (a, b, c) = (p[t[0] as usize], p[t[1] as usize], p[t[2] as usize]);
            ((b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)).abs() / 2.0
        })
        .sum()
}

#[test]
fn square_both_orders() {
    let sq = vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    let reversed: Vec<(f64, f64)> = sq.iter().rev().copied().collect();
    for p in [sq, reversed] {
        let tri = ft::triangulate(&p);
        assert_eq!(tri.len(), 6);
        assert!(close(area(&p, &tri), 1.0, 7));
    }
}

#[test]
fn concave_l_shape() {
    let p = [(0.0, 0.0), (2.0, 0.0), (2.0, 1.0), (1.0, 1.0), (1.0, 2.0), (0.0, 2.0)];
    let tri = ft::triangulate(&p);
    assert_eq!(tri.len(), 12);
    assert!(close(area(&p, &tri), 3.0, 7));
}

#[test]
fn fill_of_large_polygon_is_fast() {
    // Многоугольник 0.2° × 0.3° сгущается до 800 точек. Резка всего
    // контура шла 1.1 с на каждое движение мыши. Режутся только
    // вершины, 27 сентября 2026 года.
    let corners = [(58.0, 56.0), (58.0, 56.3), (58.2, 56.3), (58.2, 56.0)];
    let start = std::time::Instant::now();
    let (ring, tri) = ft::fill(&corners, ft::STEP, ft::MAX_POINTS);
    assert!(start.elapsed().as_secs_f64() < 0.2);
    assert_eq!(tri.len(), 6);
    // Номера треугольников указывают на вершины в сгущённом контуре.
    for i in tri {
        assert!(corners.contains(&ring[i as usize]));
    }
}

#[test]
fn plane_of_small_polygon() {
    let xy = ft::plane(&[(58.0, 56.0), (58.0, 56.01), (58.01, 56.01)]);
    // 0.01° долготы на 58° - около 590 м.
    assert!(((xy[1].0 - xy[0].0).abs() - 590.0).abs() <= 5.0);
}

#[test]
fn ends_and_round_trip() {
    assert_eq!(ft::height_share(0.0), 0.0);
    assert!(close(ft::height_share(ft::MAX_HEIGHT), 1.0, 7));
    assert_eq!(ft::height_share(-5.0), 0.0);
    for h in [1.0, 36.0, 3600.0, 50000.0] {
        assert!(close(ft::share_height(ft::height_share(h)), h, 6));
    }
}

#[test]
fn low_heights_get_room() {
    // Первая пятая ползунка - до 9 м, половина - до 316 м.
    assert!(ft::share_height(0.2) < 10.0);
    assert!(ft::share_height(0.5) < 320.0);
}

fn line() -> Shape {
    Shape::new(Kind::Line, vec![(0.0, 0.0), (0.0, 10.0), (0.0, 30.0)])
}

#[test]
fn full_is_same_object() {
    // Тот же объект - буферы видеокарты не пересобираются.
    assert!(matches!(ft::grown(&line(), 1.0), Cow::Borrowed(_)));
}

#[test]
fn cut_inside_second_segment() {
    let shape = line();
    let grown = ft::grown(&shape, 0.5);
    let points = &grown.points;
    assert_eq!(points[..2], [(0.0, 0.0), (0.0, 10.0)]);
    assert!(close(points[2].1, 15.0, 6));
    assert!(close(points[2].0, 0.0, 6));
}

#[test]
fn cut_inside_first_segment() {
    let shape = line();
    let grown = ft::grown(&shape, 0.1);
    assert_eq!(grown.points.len(), 2);
    assert!(close(grown.points[1].1, 3.0, 6));
}

#[test]
fn start_is_still_a_line() {
    let shape = line();
    let grown = ft::grown(&shape, 0.0);
    assert_eq!(grown.points.len(), 2);
    assert!(grown.points[1].1 < 0.1);
}

#[test]
fn other_kinds_untouched() {
    let polygon = Shape::new(Kind::Polygon, vec![(0.0, 0.0), (0.0, 1.0), (1.0, 1.0)]);
    assert!(matches!(ft::grown(&polygon, 0.3), Cow::Borrowed(_)));
}

// Многоугольник с дырой: заливка обходит дыру, контур и стены - у каждого
// кольца.

const OUTER: [(f64, f64); 4] = [(0.0, 0.0), (0.0, 0.01), (0.01, 0.01), (0.01, 0.0)];
const HOLE: [(f64, f64); 4] = [(0.004, 0.004), (0.004, 0.006), (0.006, 0.006), (0.006, 0.004)];

fn fill_area(geo: &ft::Geometry) -> f64 {
    area(&ft::plane(&geo.ring), &geo.triangles)
}

#[test]
fn fill_leaves_hole_open() {
    let mut whole = Shape::new(Kind::Polygon, OUTER.to_vec());
    whole.fill = Some([255, 255, 255, 60]);
    let mut holed = whole.clone();
    holed.holes = Some(vec![HOLE.to_vec()]);
    let (whole, holed) = (ft::geometry(&whole).unwrap(), ft::geometry(&holed).unwrap());
    let share = fill_area(&holed) / fill_area(&whole);
    assert!(close(share, 0.96, 2), "{share}");
    // Середина дыры не лежит ни в одном треугольнике заливки.
    let mut both = holed.ring.clone();
    both.push((0.005, 0.005));
    let both = ft::plane(&both);
    let (xy, middle) = (&both[..both.len() - 1], both[both.len() - 1]);
    for t in holed.triangles.as_chunks::<3>().0 {
        let (a, b, c) = (xy[t[0] as usize], xy[t[1] as usize], xy[t[2] as usize]);
        assert!(!(ft::inside(middle, a, b, c) || ft::inside(middle, a, c, b)));
    }
    // Отрезки: контур и дыра замкнуты каждый сам по себе.
    assert_eq!(holed.lines.len() / 2, holed.ring.len());
}

#[test]
fn walls_go_round_every_ring() {
    let mut shape = Shape::new(Kind::Polygon, OUTER.to_vec());
    shape.height = 10.0;
    shape.extrude = true;
    shape.holes = Some(vec![HOLE.to_vec()]);
    let geo = ft::geometry(&shape).unwrap();
    let n = geo.ring.len();
    assert_eq!(geo.wall.len() / 6, n);
    assert_eq!(geo.wall.iter().max().copied(), Some(2 * n as u32 - 1));
}

// Режимы высоты меток. Рельеф с дном моря -100 м, поверхность - вода
// на нуле.

fn floor(lats: &[f64], _: &[f64]) -> Vec<f64> {
    vec![-100.0; lats.len()]
}

fn surface(lats: &[f64], _: &[f64]) -> Vec<f64> {
    vec![0.0; lats.len()]
}

fn top(mode: &str, height: f64, scale: f64) -> (f64, f64) {
    let mut shape = Shape::new(Kind::Point, vec![(0.0, 0.0)]);
    shape.height = height;
    shape.extrude = true;
    shape.mode = mode.into();
    let geo = ft::geometry(&shape).unwrap();
    let xyz = ft::vertices(&geo, height, Some(&floor), mode, Some(&surface), scale);
    (xyz[0].length() - EARTH.a, xyz[1].length() - EARTH.a)
}

#[test]
fn heights_by_mode() {
    for (mode, height, expected) in [(ft::RELATIVE, 30.0, 30.0), (ft::RELATIVE_FLOOR, 30.0, -70.0), (ft::ABSOLUTE, 30.0, 60.0)] {
        let (top, foot) = top(mode, height, 2.0);
        assert!(close(top, expected, 3), "{mode}");
        // Стена идёт до поверхности, над морем - до воды.
        assert!(close(foot, 0.0, 3), "{mode}");
    }
}

#[test]
fn clamp_modes_ignore_height() {
    let mut line = Shape::new(Kind::Line, vec![(0.0, 0.0), (0.0, 0.001)]);
    line.height = 500.0;
    line.mode = ft::SEAFLOOR.into();
    let geo = ft::geometry(&line).unwrap();
    let xyz = ft::vertices(&geo, 500.0, Some(&floor), ft::SEAFLOOR, Some(&surface), 1.0);
    assert!(close(xyz[0].length() - EARTH.a, -100.0, 3));
}

#[test]
fn old_records_keep_meaning() {
    // Метка без режима: с подъёмом - относительно поверхности.
    let mut raised = Shape::new(Kind::Point, vec![(0.0, 0.0)]);
    raised.height = 10.0;
    assert_eq!(ft::altitude_mode(&raised), ft::RELATIVE);
    assert_eq!(ft::altitude_mode(&Shape::new(Kind::Point, vec![(0.0, 0.0)])), ft::CLAMP);
}
