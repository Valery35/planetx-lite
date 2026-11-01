// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Отметки Z объектов слоя доходят до разреза модели.

#![allow(clippy::expect_used)]

use data::layers::{self, Shape};

const GEOJSON: &str = r#"{"type": "FeatureCollection", "features": [
 {"type": "Feature", "properties": {"name": "с Z"}, "geometry": {"type": "LineString", "coordinates": [[56.0, 58.0, -120.5], [56.1, 58.0, -130.0]]}},
 {"type": "Feature", "properties": {"name": "без Z"}, "geometry": {"type": "LineString", "coordinates": [[56.0, 58.0], [56.1, 58.0]]}}
]}"#;

#[test]
fn heights_follow_the_points() {
    let read = layers::read(GEOJSON.as_bytes().to_vec(), "lines.geojson").expect("слой");
    let items = &read.first().expect("слой").items;
    assert_eq!(items[0].shape, Shape::Line);
    assert_eq!(items[0].heights, [vec![-120.5, -130.0]]);
    // terrio 0.6 считает пересчёт координат с ошибкой в последнем знаке.
    let (lat, lon) = items[0].parts[0][1];
    assert!((lat - 58.0).abs() < 1e-12 && (lon - 56.1).abs() < 1e-12, "{lat} {lon}");
    // Часть без отметок - пустая строка отметок, объект остаётся.
    assert!(items[1].heights[0].is_empty());
}
