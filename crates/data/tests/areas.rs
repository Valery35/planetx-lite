// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Многоугольники в картинке тайла и под точкой.

use data::areas::{Area, Areas, draw};

fn square(s: f64, w: f64, n: f64, e: f64) -> Vec<(f64, f64)> {
    vec![(s, w), (s, e), (n, e), (n, w)]
}

#[test]
fn hole_is_outside() {
    let a = Area::new(vec![square(0.0, 0.0, 10.0, 10.0), square(4.0, 4.0, 6.0, 6.0)]);
    assert!(a.contains(2.0, 2.0));
    assert!(!a.contains(5.0, 5.0));
    assert!(!a.contains(20.0, 5.0));
}

#[test]
fn index_finds_by_frame() {
    let areas = Areas::new(vec![Area::new(vec![square(58.0, 59.0, 58.1, 59.2)]), Area::new(vec![square(-30.0, 20.0, -29.0, 21.0)])]);
    assert_eq!(areas.within(57.0, 58.0, 59.0, 60.0), vec![0]);
    assert_eq!(areas.at(58.05, 59.1), vec![0]);
    assert!(areas.at(0.0, 0.0).is_empty());
    // Рамка во весь мир - оба.
    assert_eq!(areas.within(-90.0, -180.0, 90.0, 180.0), vec![0, 1]);
}

#[test]
fn tile_gets_fill_and_tiny_areas_show() {
    let big = Area::new(vec![square(-40.0, -40.0, 40.0, 40.0)]);
    let tiny = Area::new(vec![square(10.0, 100.0, 10.001, 100.001)]);
    let areas = Areas::new(vec![big, tiny]);
    let fill = [235, 125, 35, 90];
    let line = [205, 85, 20, 255];
    let drawn = draw((0, 0, 0), &areas, fill, line, 1.3);
    assert!(drawn.is_some());
    let Some(rgba) = drawn else {
        return;
    };
    // Середина тайла - заливка, у точки крошечного контура - обвод.
    let at = |x: usize, y: usize| rgba[(y * 256 + x) * 4 + 3];
    assert!(at(128, 128) > 0);
    let x = ((100.0 + 180.0) / 360.0 * 256.0) as usize;
    let y = 121;
    assert!((y - 2..=y + 2).any(|yy| (x - 2..=x + 2).any(|xx| at(xx, yy) > 0)));
    // Тайл далеко от контуров - пусто.
    assert!(draw((6, 0, 63), &areas, fill, line, 1.3).is_none());
}
