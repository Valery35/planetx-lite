// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Слой «Моих слоёв» сохраняется текстом GeoJSON и читается обратно
//! теми же объектами, полями и признаками числовых полей.

#![allow(clippy::expect_used)]

use data::layers::{self, Item, LayerData, Shape};

fn item(shape: Shape, parts: Vec<Vec<(f64, f64)>>, heights: Vec<Vec<f64>>, values: &[&str]) -> Item {
    Item { shape, parts, heights, values: values.iter().map(|v| v.to_string()).collect() }
}

/// Кольцо без замыкающей точки - так terrio отдаёт кольца файлов.
fn square(lat: f64, lon: f64, side: f64) -> Vec<(f64, f64)> {
    vec![(lat, lon), (lat, lon + side), (lat + side, lon + side), (lat + side, lon)]
}

fn close(a: &[Vec<(f64, f64)>], b: &[Vec<(f64, f64)>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(p, q)| p.len() == q.len() && p.iter().zip(q).all(|(x, y)| (x.0 - y.0).abs() < 1e-9 && (x.1 - y.1).abs() < 1e-9))
}

#[test]
fn layer_survives_geojson() {
    let layer = LayerData {
        name: "ponds".into(),
        epsg: Some(4326),
        fields: vec!["name".into(), "h".into(), "note".into()],
        numeric: vec![false, true, false],
        items: vec![
            // Два многоугольника, у второго дыра.
            item(Shape::Polygon, vec![square(58.0, 56.0, 0.1), square(58.0, 57.0, 1.0), square(58.4, 57.4, 0.2)], vec![], &["Пруды", "3", ""]),
            item(Shape::Point, vec![vec![(60.0, 50.0)]], vec![vec![-12.5]], &["Скважина", "", "глубокая"]),
            item(Shape::Line, vec![vec![(61.0, 50.0), (61.0, 51.0)], vec![(62.0, 50.0), (62.0, 51.0)]], vec![], &["Дороги", "2.75", ""]),
        ],
    };
    let text = layers::to_geojson(&layer);
    let read = layers::read(text.into_bytes(), "ponds.geojson").expect("слой");
    let back = read.first().expect("слой");
    assert_eq!(back.fields, layer.fields);
    // Числовое поле с пустым значением осталось числовым.
    assert_eq!(back.numeric, layer.numeric);
    assert_eq!(back.items.len(), 3);
    for (a, b) in layer.items.iter().zip(&back.items) {
        assert_eq!(a.shape, b.shape);
        assert!(close(&a.parts, &b.parts), "{:?}\n{:?}", a.parts, b.parts);
    }
    assert_eq!(back.items[0].values[0], "Пруды");
    assert_eq!(back.items[0].values[1], "3");
    assert_eq!(back.items[2].values[1], "2.75");
    assert_eq!(back.items[1].values[2], "глубокая");
    // Отметка точки сохранилась.
    assert_eq!(back.items[1].heights, [vec![-12.5]]);
}
