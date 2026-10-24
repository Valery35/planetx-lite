// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Линии векторной основы: разбор тайла по стилям модуля, уровни,
//! группы и картинка тайла.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::sync::Arc;

use data::lines::{self, Group, Shape, Tile};

fn all() -> HashSet<Group> {
    [Group::Borders, Group::Rivers, Group::Water, Group::Roads, Group::Railways, Group::Parks, Group::Airports].into()
}

fn varint(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn bytes_field(number: u64, data: &[u8], out: &mut Vec<u8>) {
    varint(number << 3 | 2, out);
    varint(data.len() as u64, out);
    out.extend_from_slice(data);
}

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// Объект тайла: тип геометрии, класс, вершины.
type Feature<'a> = (u64, &'a str, &'a [(i64, i64)]);

/// Тайл из одного слоя name.
fn tile(name: &str, features: &[Feature<'_>]) -> Vec<u8> {
    let mut layer = Vec::new();
    bytes_field(1, name.as_bytes(), &mut layer);
    let mut classes: Vec<&str> = Vec::new();
    for (kind, class, points) in features {
        let index = classes.iter().position(|c| c == class).unwrap_or_else(|| {
            classes.push(class);
            classes.len() - 1
        });
        let mut feature = Vec::new();
        let mut tags = Vec::new();
        varint(0, &mut tags);
        varint(index as u64, &mut tags);
        bytes_field(2, &tags, &mut feature);
        varint(3 << 3, &mut feature);
        varint(*kind, &mut feature);
        let mut geometry = Vec::new();
        let (mut x, mut y) = (0, 0);
        varint(1 | 1 << 3, &mut geometry);
        varint(zigzag(points[0].0 - x), &mut geometry);
        varint(zigzag(points[0].1 - y), &mut geometry);
        (x, y) = points[0];
        varint(2 | ((points.len() - 1) as u64) << 3, &mut geometry);
        for p in &points[1..] {
            varint(zigzag(p.0 - x), &mut geometry);
            varint(zigzag(p.1 - y), &mut geometry);
            (x, y) = *p;
        }
        if *kind == 3 {
            varint(7 | 1 << 3, &mut geometry);
        }
        bytes_field(4, &geometry, &mut feature);
        bytes_field(2, &feature, &mut layer);
    }
    bytes_field(3, b"class", &mut layer);
    for class in classes {
        let mut value = Vec::new();
        bytes_field(1, class.as_bytes(), &mut value);
        bytes_field(4, &value, &mut layer);
    }
    varint(5 << 3, &mut layer);
    varint(4096, &mut layer);
    let mut out = Vec::new();
    bytes_field(3, &layer, &mut out);
    out
}

#[test]
fn styles_by_class_and_rail() {
    let data = tile(
        "transportation",
        &[(2, "motorway", &[(0, 0), (4096, 4096)]), (2, "rail", &[(0, 4096), (4096, 0)]), (2, "path", &[(0, 0), (10, 10)])],
    );
    let t = lines::decode(&data, &all()).unwrap();
    let styles: Vec<usize> = t.shapes.iter().map(|s| s.style).collect();
    // Магистраль - пятый стиль модуля, железная дорога - свой, тропа
    // не рисуется.
    assert_eq!(styles, [4, lines::RAIL_STYLE]);
    assert_eq!(t.shapes[0].parts, [vec![[0.0, 0.0], [1.0, 1.0]]]);
}

#[test]
fn water_polygon_ring_is_closed() {
    let data = tile("water", &[(3, "lake", &[(0, 0), (4096, 0), (4096, 4096)])]);
    let t = lines::decode(&data, &all()).unwrap();
    assert_eq!(t.shapes.len(), 1);
    assert_eq!(t.shapes[0].style, 3);
    let ring = &t.shapes[0].parts[0];
    assert_eq!(ring.first(), ring.last());
}

#[test]
fn deep_tiles_take_level_14() {
    assert_eq!(lines::vector_key((16, 100, 200)), (14, 25, 50));
    assert_eq!(lines::vector_key((5, 3, 4)), (5, 3, 4));
    let rails: HashSet<Group> = [Group::Railways].into();
    // Тайл уровня 7 с железными дорогами берёт четыре тайла уровня 8.
    assert_eq!(lines::sources((7, 1, 2), &rails), [(7, 1, 2), (8, 2, 4), (8, 3, 4), (8, 2, 5), (8, 3, 5)]);
    assert_eq!(lines::sources((7, 1, 2), &HashSet::new()), [(7, 1, 2)]);
}

#[test]
fn rail_width_falls_when_far() {
    assert!((lines::rail_scale(7) - 0.5).abs() < 1e-6);
    assert_eq!(lines::rail_scale(12), 1.0);
    assert_eq!(lines::rail_scale(3), 0.4);
}

fn diagonal(style: usize) -> Arc<Tile> {
    Arc::new(Tile { shapes: vec![Shape { style, parts: vec![vec![[0.0, 0.5], [1.0, 0.5]]] }] })
}

fn alpha_at(rgba: &[u8], x: usize, y: usize) -> u8 {
    rgba[(y * lines::TILE_SIZE as usize + x) * 4 + 3]
}

#[test]
fn picture_has_the_line_and_respects_levels_and_groups() {
    let roads: HashSet<Group> = [Group::Roads].into();
    // Магистраль с уровня 6: на тайле уровня 6 линия по середине.
    let t = diagonal(4);
    let rgba = lines::draw((6, 0, 0), &|_| Some(t.clone()), &roads).unwrap();
    // Линия толщиной 1.5 по границе пикселей: строки 127 и 128 закрыты
    // на три четверти.
    assert!(alpha_at(&rgba, 128, 127) > 150 && alpha_at(&rgba, 128, 128) > 150);
    assert_eq!(alpha_at(&rgba, 128, 10), 0);
    // Уровень 5 - рано, выключенная группа - тоже ничего.
    assert!(lines::draw((5, 0, 0), &|_| Some(t.clone()), &roads).is_none());
    assert!(lines::draw((6, 0, 0), &|_| Some(t.clone()), &HashSet::new()).is_none());
    // Тайл глубже 14 берёт долю тайла 14: линия y = 0.5 тайла 14 попадает
    // в верхний край нижней половины детей.
    let rgba = lines::draw((15, 0, 1), &|k| (k == (14, 0, 0)).then(|| t.clone()), &roads).unwrap();
    let a = alpha_at(&rgba, 128, 0);
    assert!(a > 100, "{a} {}", alpha_at(&rgba, 128, 1));
}
