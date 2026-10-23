// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Координатная сетка: шаг, окно, линии, подписи. Перенос
//! `tests/test_graticule.py` модуля.

use scene::graticule::{self as gr, Axis};

fn close(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance, "{a} != {b}");
}

#[test]
fn step_follows_width() {
    assert_eq!(gr::step_for(2.0e7), 30.0);
    assert_eq!(gr::step_for(1.0e6), 2.0);
    close(gr::step_for(20000.0), 2.0 / 60.0, 1e-12);
    close(gr::step_for(300.0), 2.0 / 3600.0, 1e-12);
}

#[test]
fn about_eight_lines_across() {
    for width in [5.0e6, 2.0e5, 8000.0] {
        let step = gr::step_for(width);
        let across = width / (step * gr::M_PER_DEGREE);
        assert!(across >= 3.0, "{width}");
        assert!(across <= gr::LINES_ACROSS + 0.01, "{width}");
    }
}

#[test]
fn global_grid() {
    let (step, found) = gr::lines(58.0, 56.0, 2.0e7);
    assert_eq!(step, 30.0);
    let parallels: Vec<f64> = found.iter().filter(|l| l.0 == Axis::Lat).map(|l| l.1).collect();
    let meridians = found.iter().filter(|l| l.0 == Axis::Lon).count();
    // Экватор - особая параллель, в общей сетке его нет.
    assert_eq!(parallels, [-60.0, -30.0, 30.0, 60.0]);
    assert_eq!(meridians, 12);
    // Параллель гнётся по вершинам не реже SEGMENT градусов.
    let points = &found.iter().find(|l| l.0 == Axis::Lat).map(|l| l.2.clone()).unwrap_or_default();
    let gap = points.windows(2).map(|w| w[1].1 - w[0].1).fold(0.0, f64::max);
    assert!(gap <= gr::SEGMENT + 1e-9);
}

#[test]
fn local_window_near_view() {
    let (step, found) = gr::lines(58.0, 56.2, 20000.0);
    close(step, 2.0 / 60.0, 1e-12);
    for (axis, value, _) in &found {
        match axis {
            Axis::Lat => assert!((value - 58.0).abs() < 1.0),
            Axis::Lon => assert!((value - 56.2).abs() < 2.0),
        }
    }
    assert!(found.len() < 80);
}

#[test]
fn labels_and_key() {
    let spots = gr::labels(58.0, 56.2, 20000.0);
    assert!(spots.iter().any(|s| s.2 == Axis::Lat));
    assert!(spots.iter().any(|s| s.2 == Axis::Lon));
    assert_eq!(gr::key(58.0, 56.2, 20000.0), gr::key(58.001, 56.201, 20000.0));
    assert_ne!(gr::key(58.0, 56.2, 20000.0), gr::key(58.0, 56.2, 2.0e6));
}

#[test]
fn all_circles_from_space() {
    let found = gr::circles(58.0, 56.0, 2.0e7);
    assert_eq!(found.iter().map(|c| c.0).collect::<Vec<_>>(), ["equator", "cancer", "capricorn", "arctic", "antarctic"]);
    close(found[3].1, 66.5638, 1e-4);
    close(found[2].1, -23.4362, 1e-4);
    assert_eq!(gr::circle_labels(58.0, 56.0, 2.0e7).len(), 5);
}

#[test]
fn only_circles_in_window() {
    assert!(gr::circles(58.0, 56.2, 20000.0).is_empty());
    assert_eq!(gr::circles(66.5, 40.0, 50000.0).iter().map(|c| c.0).collect::<Vec<_>>(), ["arctic"]);
}

#[test]
fn angle_text() {
    assert_eq!(gr::angle_text(58.0, 10.0), "58°");
    assert_eq!(gr::angle_text(-58.5, 0.5), "58°30′");
    assert_eq!(gr::angle_text(56.2275, 1.0 / 3600.0), "56°13′39″");
}
