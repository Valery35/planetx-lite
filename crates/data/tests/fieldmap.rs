// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Границы полей Fields of the World: разбор тайла, оценка, цвет.
//! Перенос `tests/test_fieldmap.py` модуля.

use data::fieldmap::{self, FULL, MIN_SCORE, STOPS};
use data::mvt::Value;

fn varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    while value >= 0x80 {
        out.push((value & 0x7F) as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
    out
}

fn field(number: u64, payload: &[u8]) -> Vec<u8> {
    [varint(number << 3 | 2), varint(payload.len() as u64), payload.to_vec()].concat()
}

fn number(no: u64, value: u64) -> Vec<u8> {
    [varint(no << 3), varint(value)].concat()
}

fn zigzag(value: i64) -> u64 {
    if value >= 0 { (value as u64) << 1 } else { ((-value as u64) << 1) - 1 }
}

/// Геометрия MVT квадрата от начала координат тайла.
fn square(x: i64, y: i64, side: i64) -> Vec<u8> {
    let cells = [1 << 3 | 1, zigzag(x), zigzag(y), 3 << 3 | 2, zigzag(side), 0, 0, zigzag(side), zigzag(-side), 0, 7 << 3 | 7];
    cells.iter().flat_map(|c| varint(*c)).collect()
}

fn feature(label: u64, confidence: u64, x: i64) -> Vec<u8> {
    let tags: Vec<u8> = [0, label, 1, confidence].iter().flat_map(|v| varint(*v)).collect();
    [field(2, &tags), number(3, 3), field(4, &square(x, 100, 400))].concat()
}

fn tile(title: &str, extent: u64) -> Vec<u8> {
    let layer = [
        field(1, title.as_bytes()),
        field(3, b"label"),
        field(3, b"confidence_mean"),
        field(4, &field(1, b"field")),
        field(4, &field(1, b"field_boundaries")),
        field(4, &field(1, b"0.55")),
        field(4, &field(1, b"0.2")),
        field(2, &feature(0, 2, 100)),
        field(2, &feature(1, 2, 600)),
        field(2, &feature(0, 3, 1100)),
        number(5, extent),
    ]
    .concat();
    field(3, &layer)
}

#[test]
fn only_confident_fields_of_layer() {
    let found = fieldmap::decode(&tile("fields", 4096), "fields").unwrap_or_default();
    // Полоса и низкая оценка отброшены.
    assert_eq!(found.len(), 1);
    assert!((found[0].score - 0.55 / FULL * 100.0).abs() < 1e-6);
}

#[test]
fn rings_are_tile_shares() {
    let found = fieldmap::decode(&tile("fields", 1024), "fields").unwrap_or_default();
    let ring = &found[0].rings[0];
    let least = ring.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
    let most = ring.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max);
    assert!((least - 100.0 / 1024.0).abs() < 1e-9);
    assert!((most - 500.0 / 1024.0).abs() < 1e-9);
}

#[test]
fn other_layer_gives_nothing() {
    assert_eq!(fieldmap::decode(&tile("fields", 4096), "2024"), Ok(Vec::new()));
}

#[test]
fn hostile_tile_is_an_error() {
    assert!(fieldmap::decode(&[0xFF; 32], "fields").is_err());
}

#[test]
fn full_value_is_hundred() {
    let score = fieldmap::score_of(&Value::Str(FULL.to_string()));
    assert!(score.is_some_and(|s| (s - 100.0).abs() < 1e-9));
    assert_eq!(fieldmap::score_of(&Value::Str("0.9".into())), Some(100.0));
    assert_eq!(fieldmap::score_of(&Value::Str("нет".into())), None);
}

#[test]
fn color_follows_stops() {
    assert_eq!(fieldmap::color(MIN_SCORE), STOPS[0].1);
    assert_eq!(fieldmap::color(100.0), STOPS[3].1);
    assert_eq!(fieldmap::color(90.0), [207, 236, 176]);
}

#[test]
fn every_year_has_key_address_and_layer() {
    for year in fieldmap::YEARS {
        assert_eq!(fieldmap::year_of(fieldmap::key(year)), Some(year));
        assert!(fieldmap::url(year).ends_with(".pmtiles"));
        assert!(!fieldmap::layer(year).is_empty());
    }
}

/// Поле-квадрат долями тайла: запад 0.25, восток 0.75.
fn plot() -> fieldmap::Field {
    fieldmap::Field { rings: vec![vec![[0.25, 0.25], [0.75, 0.25], [0.75, 0.75], [0.25, 0.75]]], score: 90.0 }
}

#[test]
fn score_under_point() {
    let mut one = plot();
    assert_eq!(fieldmap::score_at(std::slice::from_ref(&one), 0.5, 0.5), Some(90.0));
    assert_eq!(fieldmap::score_at(std::slice::from_ref(&one), 0.1, 0.5), None);
    // В дыре поля оценки нет.
    one.rings.push(vec![[0.4, 0.4], [0.6, 0.4], [0.6, 0.6], [0.4, 0.6]]);
    assert_eq!(fieldmap::score_at(&[one], 0.5, 0.5), None);
}

#[test]
fn point_in_tile() {
    let ((x, y), (dx, dy)) = fieldmap::tile_point(0.0, 0.0, 10);
    assert_eq!((x, y), (512, 512));
    assert!(dx.abs() < 1e-9 && dy.abs() < 1e-9);
    // Счёт вручную: (1 - asinh(tan 58°) / π) / 2 · 1024 = 308.3.
    let ((x, y), (dx, dy)) = fieldmap::tile_point(58.0, 56.25, 10);
    assert_eq!((x, y), (672, 308));
    assert!((0.0..1.0).contains(&dx) && (0.0..1.0).contains(&dy));
}

#[test]
fn picture_of_tile() {
    let rgba = fieldmap::paint(&[plot()], (14, 8, 8), (14, 8, 8)).unwrap_or_default();
    assert_eq!(rgba.len(), 256 * 256 * 4);
    let at = |x: usize, y: usize| rgba[(y * 256 + x) * 4 + 3];
    assert!(at(128, 128) > 0 && at(128, 128) < 255);
    assert_eq!(at(10, 10), 0);
    assert!(fieldmap::paint(&[], (14, 8, 8), (14, 8, 8)).is_none());
}

#[test]
fn deeper_picture_takes_part_of_source() {
    // Правая нижняя четверть тайла архива на уровне глубже: поле занимает
    // левый верхний её угол.
    assert_eq!(fieldmap::source_of((15, 17, 19)), (14, 8, 9));
    let rgba = fieldmap::paint(&[plot()], (14, 8, 9), (15, 17, 19)).unwrap_or_default();
    let at = |x: usize, y: usize| rgba[(y * 256 + x) * 4 + 3];
    assert!(at(100, 100) > 0);
    assert_eq!(at(200, 200), 0);
    // Левая верхняя четверть: поле занимает правый нижний угол.
    let rgba = fieldmap::paint(&[plot()], (14, 8, 9), (15, 16, 18)).unwrap_or_default();
    assert_eq!(rgba[(10 * 256 + 10) * 4 + 3], 0);
    assert!(rgba[(200 * 256 + 200) * 4 + 3] > 0);
    // Четверть без поля.
    let far = fieldmap::Field { rings: vec![vec![[0.1, 0.1], [0.2, 0.1], [0.2, 0.2], [0.1, 0.2]]], score: 90.0 };
    assert!(fieldmap::paint(&[far], (14, 8, 9), (15, 17, 19)).is_none());
}
