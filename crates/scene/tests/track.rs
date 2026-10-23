// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Поза модели по gx:Track, перенос `TestModelPose` из `tests/test_tour.py`
//! модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use scene::overlays::TrackRow;
use scene::track::model_pose;

fn row(t: f64, lat: f64, lon: f64, alt: f64, heading: Option<f64>, tilt: f64, roll: f64) -> TrackRow {
    TrackRow { t, lat, lon, alt, heading, tilt, roll }
}

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-7, "{a} != {b}");
}

#[test]
fn angles_and_heading_from_motion() {
    let rows = [row(0.0, 58.0, 56.0, 1000.0, Some(350.0), 0.0, 0.0), row(10.0, 58.0, 56.1, 2000.0, Some(10.0), 10.0, 20.0)];
    let p = model_pose(&rows, 5.0).unwrap();
    close(p.lon, 56.05);
    close(p.alt, 1500.0);
    // Кратчайший поворот.
    close(p.heading, 0.0);
    close(p.tilt, 5.0);
    close(p.roll, 10.0);
    assert!(model_pose(&rows, 11.0).is_none());
    let bare: Vec<TrackRow> = rows.iter().map(|r| TrackRow { heading: None, tilt: 0.0, roll: 0.0, ..*r }).collect();
    let h = model_pose(&bare, 5.0).unwrap().heading;
    assert!((h - 90.0).abs() < 0.1, "{h}");
}

#[test]
fn ends_and_standing_point() {
    let rows = [row(0.0, 58.0, 56.0, 0.0, None, 0.0, 0.0), row(10.0, 58.1, 56.0, 0.0, None, 0.0, 0.0)];
    // На концах трека модель есть, до начала - нет.
    assert!(model_pose(&rows, -0.1).is_none());
    let end = model_pose(&rows, 10.0).unwrap();
    close(end.lat, 58.1);
    // В последней точке азимут - по последнему отрезку, на север.
    assert!(end.heading.abs() < 1e-6 || (end.heading - 360.0).abs() < 1e-6, "{}", end.heading);
    assert!(model_pose(&[], 0.0).is_none());
    // Одна точка - азимут 0.
    let one = model_pose(&rows[..1], 0.0).unwrap();
    close(one.heading, 0.0);
}
