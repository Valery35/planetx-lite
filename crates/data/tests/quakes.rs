// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Землетрясения: разбор сводки USGS, цвета, размеры, время. Перенос
//! `tests/test_quakes.py` модуля.

use data::quakes;

const FEED: &str = r#"{"type": "FeatureCollection", "features": [
 {"type": "Feature", "properties": {"mag": 6.1, "place": "Kuril Islands", "time": 1790000000000, "url": "https://x/1"},
  "geometry": {"type": "Point", "coordinates": [150.0, 45.0, 35.0]}},
 {"type": "Feature", "properties": {"mag": null}, "geometry": {"type": "Point", "coordinates": [1.0, 2.0, 3.0]}},
 {"type": "Feature", "properties": {"mag": 4.7}, "geometry": {"type": "Point", "coordinates": [140.0, 30.0]}},
 {"type": "Feature", "properties": {"mag": 5.0, "time": null}, "geometry": {"type": "Point", "coordinates": [-70.0, -20.0, 600.0]}}
]}"#;

#[test]
fn parse_skips_incomplete() {
    let got = quakes::parse(FEED.as_bytes());
    assert_eq!(got.len(), 2);
    assert_eq!((got[0].lat, got[0].lon, got[0].depth), (45.0, 150.0, 35.0));
    assert_eq!(got[0].time, Some(1_790_000_000.0));
    assert_eq!(got[1].time, None);
    assert!(quakes::parse(b"null").is_empty());
    assert!(quakes::parse(b"{\"features\": 5}").is_empty());
}

#[test]
fn colors_and_sizes() {
    assert_eq!(quakes::depth_color(0.0), [230, 40, 30]);
    assert_eq!(quakes::depth_color(700.0), quakes::depth_color(900.0));
    assert_eq!([4.5, 6.5, 11.0].map(quakes::size), [4.0, 10.0, 22.0]);
}

#[test]
fn window_keeps_events_without_time() {
    let span = Some((150.0, 250.0));
    let got: Vec<bool> = [Some(100.0), Some(200.0), None, Some(300.0)].iter().map(|t| quakes::in_window(*t, span)).collect();
    assert_eq!(got, [false, true, true, false]);
    assert!(quakes::in_window(Some(1.0), None));
}

#[test]
fn span_ignores_missing_time() {
    let events = quakes::parse(FEED.as_bytes());
    assert_eq!(quakes::span(&events), Some((1_790_000_000.0, 1_790_000_000.0)));
    assert_eq!(quakes::span(&events[1..]), None);
}
