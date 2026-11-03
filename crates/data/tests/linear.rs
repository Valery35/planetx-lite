// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Линейные объекты, перенос `tests/test_linear.py` модуля 1.33.

use data::linear;

fn total(paths: &[Vec<(f64, f64)>]) -> f64 {
    paths.iter().map(|p| linear::length_m(p, 1.0)).sum()
}

/// Прямоугольник: строки и столбцы от и до.
type Rect = ((usize, usize), (usize, usize));

/// Маска rows × cols с прямоугольниками (строки, столбцы).
fn mask(rows: usize, cols: usize, boxes: &[Rect]) -> Vec<bool> {
    let mut out = vec![false; rows * cols];
    for ((r0, r1), (c0, c1)) in boxes {
        for r in *r0..*r1 {
            for c in *c0..*c1 {
                out[r * cols + c] = true;
            }
        }
    }
    out
}

#[test]
fn strip_is_thin_field_is_not() {
    // Дорога в 3 клетки и поле.
    let m = mask(80, 120, &[((10, 13), (5, 115)), ((30, 75), (20, 70))]);
    let part = linear::thin(&m, 80, 120, 3.0);
    assert!(part[11 * 120 + 60]);
    assert!(!part[50 * 120 + 45]);
}

#[test]
fn road_becomes_one_line() {
    let m = mask(40, 120, &[((10, 13), (5, 115))]);
    let found = linear::lines(&m, 40, 120, 3.0, 20.0);
    assert_eq!(found.len(), 1);
    assert!(total(&found) > 95.0);
    assert!(found[0].iter().all(|(_, y)| (10.0..=13.0).contains(y)));
}

#[test]
fn field_gives_no_line() {
    let m = mask(80, 80, &[((10, 70), (10, 70))]);
    assert!(linear::lines(&m, 80, 80, 3.0, 20.0).is_empty());
}

#[test]
fn short_piece_dropped() {
    let m = mask(40, 40, &[((10, 12), (5, 15))]);
    assert!(linear::lines(&m, 40, 40, 3.0, 20.0).is_empty());
}

#[test]
fn crossing_roads_one_group() {
    let m = mask(101, 101, &[((49, 52), (5, 96)), ((5, 96), (49, 52))]);
    let found = linear::lines(&m, 101, 101, 3.0, 20.0);
    assert!(found.len() >= 4);
    assert!(total(&found) > 170.0);
}

#[test]
fn road_into_field_keeps_road() {
    // Поле и дорога от поля.
    let m = mask(80, 160, &[((20, 60), (0, 40)), ((39, 42), (40, 150))]);
    let found = linear::lines(&m, 80, 160, 3.0, 20.0);
    assert!(total(&found) > 95.0);
    assert!(found.iter().flatten().map(|p| p.0).fold(0.0, f64::max) > 140.0);
}

#[test]
fn one_cell_wide() {
    let m = mask(20, 40, &[((5, 10), (3, 37))]);
    let skel = linear::skeleton(&m, 20, 40);
    assert!(skel.iter().any(|v| *v));
    assert!((0..20).filter(|r| skel[r * 40 + 20]).count() <= 1);
}
