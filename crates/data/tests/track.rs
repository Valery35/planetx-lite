// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Растущие треки: путь к моменту, положение, азимут, высота. Перенос
//! `tests/test_track.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use data::track::{Item, Position, Track};

/// Машина идёт на север 100 с, потом на восток. Лось стартует позже.
fn items() -> Vec<Item> {
    vec![
        Item::new("car", 0.0, 58.00, 56.00),
        Item::new("car", 100.0, 58.10, 56.00),
        Item::new("car", 200.0, 58.10, 56.20),
        Item::new("car", 100.0, 58.10, 56.00),
        Item::new("elk", 150.0, 57.00, 55.00),
        Item::new("elk", 250.0, 57.00, 55.10),
    ]
}

fn by_key(track: &Track, t: Option<f64>) -> BTreeMap<String, Position> {
    track.at(t).into_iter().map(|p| (p.key.clone(), p)).collect()
}

fn close(a: f64, b: f64, delta: f64) -> bool {
    (a - b).abs() <= delta
}

#[test]
fn height_between_points_and_along_path() {
    let track = Track::new([
        Item::new("plane", 0.0, 58.0, 56.0).with_alt(0.0),
        Item::new("plane", 100.0, 58.1, 56.0).with_alt(1000.0),
        Item::new("plane", 200.0, 58.2, 56.0).with_alt(3000.0),
        Item::new("car", 0.0, 57.0, 55.0),
    ]);
    let found = by_key(&track, Some(150.0));
    let plane = &found["plane"];
    assert!(close(plane.alt, 2000.0, 1e-7));
    assert_eq!(plane.alts, Some(vec![0.0, 1000.0, 2000.0]));
    assert_eq!(plane.alts.as_ref().map(Vec::len), Some(plane.path.len()));
    assert!(found["car"].alt.is_nan());
    assert!(found["car"].alts.is_none());
}

#[test]
fn range_and_duplicates() {
    let track = Track::new(items());
    assert_eq!((track.start, track.end), (Some(0.0), Some(250.0)));
    assert_eq!(track.object("car").unwrap().times.len(), 3);
    // Из двух точек с одним временем остаётся первая во входе.
    let twice = Track::new([Item::new("x", 0.0, 1.0, 1.0), Item::new("x", 0.0, 2.0, 2.0)]);
    assert_eq!(twice.object("x").unwrap().lats, vec![1.0]);
}

#[test]
fn growing_path_and_interpolation() {
    let track = Track::new(items());
    let car = &by_key(&track, Some(50.0))["car"];
    assert!(close(car.lat, 58.05, 1e-7));
    assert_eq!(car.path.len(), 2);
    assert_eq!(car.path.last(), Some(&(car.lat, car.lon)));
    let later = &by_key(&track, Some(150.0))["car"];
    assert_eq!(later.path.len(), 3);
    assert!(close(later.lon, 56.10, 1e-7));
}

#[test]
fn heading_follows_motion() {
    let track = Track::new(items());
    assert!(by_key(&track, Some(50.0))["car"].heading.abs() % 360.0 < 0.5);
    assert!(close(by_key(&track, Some(180.0))["car"].heading, 90.0, 1.0));
}

#[test]
fn not_started_and_finished() {
    let track = Track::new(items());
    assert!(!by_key(&track, Some(100.0)).contains_key("elk"));
    let end = &by_key(&track, Some(1000.0))["car"];
    assert_eq!((end.lat, end.lon), (58.10, 56.20));
    let whole = by_key(&track, None);
    assert_eq!(whole["car"].path.len(), 3);
    assert_eq!(whole["elk"].path.len(), 2);
}

#[test]
fn standing_object_has_no_heading() {
    let one = Track::new([Item::new("x", 0.0, 1.0, 2.0)]).at(Some(10.0));
    assert!(one[0].heading.is_nan());
    assert_eq!(Track::new([]).start, None);
}

#[test]
fn hostile_values_are_skipped() {
    let track = Track::new([
        Item::new("x", f64::NAN, 1.0, 2.0),
        Item::new("x", 5.0, f64::INFINITY, 2.0),
        Item::new("x", 1.0, 1.0, 2.0),
    ]);
    assert_eq!(track.object("x").unwrap().times, vec![1.0]);
    assert!(track.at(Some(f64::NAN)).is_empty());
    assert_eq!(track.at(Some(f64::INFINITY)).len(), 1);
}
