// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Правка вершин: что под курсором, вставка, замыкание, попадание.
//! Перенос `tests/test_editing.py` модуля.

use scene::editing::{self as ed, Click, Drawing, Picked, Pixel, Shape};

fn visible(points: &[[f64; 2]]) -> Vec<Pixel> {
    points.iter().map(|p| (*p, true)).collect()
}

fn square() -> Vec<Pixel> {
    visible(&[[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]])
}

fn middles() -> Vec<Pixel> {
    visible(&[[50.0, 0.0], [100.0, 50.0], [50.0, 100.0], [0.0, 50.0]])
}

#[test]
fn pick_vertex() {
    assert_eq!(ed::pick(&square(), &middles(), 98.0, 3.0, 8.0), Some(Picked::Vertex(1)));
}

#[test]
fn pick_middle() {
    assert_eq!(ed::pick(&square(), &middles(), 52.0, 99.0, 8.0), Some(Picked::Middle(2)));
}

#[test]
fn vertex_before_middle() {
    // Короткий отрезок: вершина и середина обе в радиусе.
    let vertices = visible(&[[0.0, 0.0], [6.0, 0.0]]);
    let middles = visible(&[[3.0, 0.0]]);
    assert_eq!(ed::pick(&vertices, &middles, 4.0, 0.0, 8.0), Some(Picked::Vertex(1)));
}

#[test]
fn pick_nothing() {
    assert_eq!(ed::pick(&square(), &middles(), 50.0, 50.0, 8.0), None);
}

#[test]
fn behind_camera_is_skipped() {
    let mut square = square();
    square[0].1 = false;
    assert_eq!(ed::pick(&square, &[], 1.0, 1.0, 8.0), None);
}

#[test]
fn pick_empty() {
    assert_eq!(ed::pick(&[], &[], 1.0, 1.0, 8.0), None);
}

#[test]
fn insert_inside() {
    let (points, alts) = ed::inserted(&[(0, 0), (0, 2)], &[None::<f64>, None], 0, (0, 1), None);
    assert_eq!(points, [(0, 0), (0, 1), (0, 2)]);
    assert_eq!(alts, [None, None, None]);
}

#[test]
fn insert_on_closing_segment() {
    let (points, _) = ed::inserted(&[(0, 0), (0, 2), (2, 2)], &[None::<f64>; 3], 2, (1, 1), None);
    assert_eq!(points, [(0, 0), (0, 2), (2, 2), (1, 1)]);
}

#[test]
fn remove() {
    let (points, alts) = ed::removed(&[(0, 0), (0, 1), (0, 2)], &[1.0, 2.0, 3.0], 1);
    assert_eq!(points, [(0, 0), (0, 2)]);
    assert_eq!(alts, [1.0, 3.0]);
}

#[test]
fn first_vertex_closes() {
    assert_eq!(ed::click_result(Drawing::Path, 3, 0, false), Some(Click::Close));
    assert_eq!(ed::click_result(Drawing::Polygon, 4, 0, false), Some(Click::Close));
}

#[test]
fn two_points_do_not_close() {
    assert_eq!(ed::click_result(Drawing::Path, 2, 0, false), None);
}

#[test]
fn last_vertex_finishes_path() {
    assert_eq!(ed::click_result(Drawing::Path, 3, 2, false), Some(Click::Finish));
    assert_eq!(ed::click_result(Drawing::Polygon, 3, 2, false), None);
}

#[test]
fn finished_figure_ignores_click() {
    assert_eq!(ed::click_result(Drawing::Path, 3, 0, true), None);
}

#[test]
fn point_mode() {
    assert_eq!(ed::click_result(Drawing::Point, 1, 0, false), None);
}

#[test]
fn segment_distance() {
    assert!((ed::segment_distance(&square(), 50.0, 5.0, false) - 5.0).abs() < 1e-12);
}

#[test]
fn closing_segment_counts_only_when_closed() {
    assert!((ed::segment_distance(&square(), -3.0, 50.0, true) - 3.0).abs() < 1e-12);
    assert!(ed::segment_distance(&square(), -3.0, 50.0, false) > 40.0);
}

#[test]
fn inside() {
    let p: Vec<[f64; 2]> = square().iter().map(|(p, _)| *p).collect();
    assert!(ed::inside(&p, 50.0, 50.0));
    assert!(!ed::inside(&p, 150.0, 50.0));
}

#[test]
fn hit_kinds() {
    assert!(ed::hit(Shape::Polygon, &square(), 50.0, 50.0, 8.0));
    assert!(!ed::hit(Shape::Line, &square(), 50.0, 50.0, 8.0));
    assert!(ed::hit(Shape::Line, &square(), 50.0, 4.0, 8.0));
    assert!(ed::hit(Shape::Point, &square(), 3.0, 3.0, 8.0));
    assert!(!ed::hit(Shape::Point, &square(), 98.0, 3.0, 8.0));
}

#[test]
fn hidden_object_is_not_hit() {
    let hidden: Vec<Pixel> = square().iter().map(|(p, _)| (*p, false)).collect();
    assert!(!ed::hit(Shape::Polygon, &hidden, 50.0, 50.0, 8.0));
}
