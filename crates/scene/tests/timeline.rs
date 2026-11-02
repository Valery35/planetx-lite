// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Шкала времени записи: кадры, позы, время данных. Перенос
//! `tests/test_timeline.py` модуля и проверка файла `frames.json`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use scene::timeline::{self as tl, FPS, Timeline};
use scene::tour::{FOV_Y, Stop};
use serde_json::Value;

fn stops() -> Vec<Stop> {
    vec![Stop::new("A", 58.0, 56.2, 5000.0, 30.0, 45.0), Stop::new("B", 58.1, 56.4, 8000.0, 0.0, 0.0)]
}

fn line(data: Option<(f64, f64)>) -> Timeline {
    Timeline::new(stops(), 1.0, data, FPS, FOV_Y).expect("остановки есть")
}

#[test]
fn frame_count_and_repeat() {
    let (a, b) = (line(None), line(None));
    assert_eq!(a.count, (a.tour.duration * f64::from(FPS)) as usize + 1);
    assert_eq!(a.count, b.count);
    for n in [0, a.count / 2, a.count - 1] {
        let (pa, pb) = (a.frame(n).pose, b.frame(n).pose);
        assert_eq!((pa.lat, pa.lon, pa.distance, pa.heading, pa.tilt), (pb.lat, pb.lon, pb.distance, pb.heading, pb.tilt));
    }
}

#[test]
fn starts_at_first_stop() {
    let first = line(None).frame(0).pose;
    assert!((first.lat - 58.0).abs() < 1e-9);
    assert!((first.heading - 30.0).abs() < 1e-9);
}

#[test]
fn data_time_spans_tour() {
    let with_data = line(Some((1000.0, 2000.0)));
    assert_eq!(with_data.frame(0).moment, Some(1000.0));
    let last = with_data.frame(with_data.count - 1).moment.expect("есть время данных");
    assert!((last - 2000.0).abs() <= 1000.0 / with_data.count as f64 + 1e-6, "{last}");
    assert_eq!(line(None).frame(3).moment, None);
}

#[test]
fn needs_stops() {
    assert!(Timeline::new(Vec::new(), 1.0, None, FPS, FOV_Y).is_none());
}

#[test]
fn frame_time_is_number_over_fps() {
    let timeline = line(None);
    let frame = timeline.frame(50);
    assert_eq!(frame.number, 50);
    assert!((frame.t - 2.0).abs() < 1e-12);
}

#[test]
fn frame_names_keep_order_of_files() {
    assert_eq!(tl::frame_name(0), "frame_00000.png");
    assert_eq!(tl::frame_name(1234), "frame_01234.png");
}

#[test]
fn index_has_size_poses_and_incomplete_frames() {
    let timeline = line(Some((0.0, 10.0)));
    let frames: Vec<_> = (0..2).map(|n| timeline.frame(n)).collect();
    let text = timeline.index([1920, 1080], &[1], &frames);
    let value: Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(value["fps"], f64::from(FPS));
    assert_eq!(value["count"], timeline.count);
    assert_eq!(value["width"], 1920);
    assert_eq!(value["height"], 1080);
    assert_eq!(value["incomplete"], serde_json::json!([1]));
    let list = value["frames"].as_array().expect("список кадров");
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["pose"].as_array().expect("поза").len(), 5);
    assert_eq!(list[0]["pose"][0], frames[0].pose.lat);
    assert_eq!(list[0]["moment"], 0.0);
    // Без времени данных момента нет, как None у модуля.
    let plain = line(None);
    let value: Value = serde_json::from_str(&plain.index([2, 2], &[], &[plain.frame(0)])).expect("JSON");
    assert!(value["frames"][0]["moment"].is_null());
}
