// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Похожие области: образец, похожесть объектов, порог. Перенос
//! `tests/test_similar.py` модуля.

use data::{segments, similar};
use segments::{LEAST, Params, SHAPE};

/// Простой генератор: тесту нужен шум, а не ряд NumPy.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f32) / (1u64 << 31) as f32
    }

    fn normal(&mut self) -> f32 {
        (0..12).map(|_| self.next()).sum::<f32>() - 6.0
    }
}

const SIDE: usize = 90;
const WIDTH: usize = 6;

/// Сетка 90 × 90: фон и три пруда, рядом с первым луг цвета пруда
/// с крупной рябью.
fn field() -> Vec<f32> {
    let mut random = Noise(3);
    let pond = |r: usize, c: usize, row: std::ops::Range<usize>, col: std::ops::Range<usize>| row.contains(&r) && col.contains(&c);
    let mut grid = Vec::with_capacity(3 * SIDE * SIDE);
    for r in 0..SIDE {
        for c in 0..SIDE {
            let ripple = if random.next() < 0.5 { -40.0 } else { 40.0 };
            let base = if pond(r, c, 60..80, 60..80) {
                40.0 + ripple
            } else if pond(r, c, 10..30, 10..30) || pond(r, c, 10..30, 60..80) || pond(r, c, 60..80, 10..30) {
                40.0
            } else {
                120.0
            };
            for _ in 0..3 {
                grid.push(base + 2.0 * random.normal());
            }
        }
    }
    grid
}

fn cut(grid: &[f32], weight: f64) -> Vec<i32> {
    segments::segment(grid, 3, SIDE, SIDE, None, Params { scale: 10.0, weight, least: LEAST, shape: SHAPE })
}

fn square(rows: std::ops::Range<usize>, cols: std::ops::Range<usize>) -> Vec<bool> {
    (0..SIDE * SIDE).map(|i| rows.contains(&(i / SIDE)) && cols.contains(&(i % SIDE))).collect()
}

/// Найденное: (объекты, похожесть, номера клеток).
fn found(grid: &[f32], mask: &[bool], weight: f64, limit: f64) -> (Vec<i32>, Vec<f64>, Vec<i32>) {
    let parts = cut(grid, weight);
    let table = segments::table(grid, 3, &parts);
    let score = similar::scores(&table, WIDTH, &segments::areas(&parts), &similar::sample_rows(&table, WIDTH, &parts, mask), weight).unwrap_or_default();
    let labels = similar::pick(&parts, SIDE, SIDE, &score, limit, true).0;
    (parts, score, labels)
}

fn at(labels: &[i32], r: usize, c: usize) -> i32 {
    labels[r * SIDE + c]
}

#[test]
fn finds_other_ponds_not_background() {
    let labels = found(&field(), &square(14..26, 14..26), 0.3, 0.7).2;
    assert!(at(&labels, 20, 20) >= 0);
    assert!(at(&labels, 20, 70) >= 0);
    assert!(at(&labels, 70, 20) >= 0);
    assert_eq!(at(&labels, 45, 45), -1);
}

#[test]
fn texture_tells_rippled_field_from_pond() {
    let labels = found(&field(), &square(14..26, 14..26), 0.5, 0.7).2;
    assert_eq!(at(&labels, 70, 70), -1);
}

#[test]
fn outline_with_some_foreign_cells_still_works() {
    // Контур чуть шире пруда: образец смешан, поиск остаётся.
    let labels = found(&field(), &square(9..31, 9..31), 0.3, 0.5).2;
    assert!(at(&labels, 20, 70) >= 0);
    assert_eq!(at(&labels, 45, 45), -1);
}

#[test]
fn nearest_of_two_samples() {
    let grid = field();
    let parts = cut(&grid, 0.3);
    let table = segments::table(&grid, 3, &parts);
    let sizes = segments::areas(&parts);
    let first = similar::sample_rows(&table, WIDTH, &parts, &square(14..26, 14..26));
    let one = similar::scores(&table, WIDTH, &sizes, &first, 0.3).unwrap_or_default();
    let both = [first.clone(), similar::sample_rows(&table, WIDTH, &parts, &square(40..50, 40..50))].concat();
    let two = similar::scores(&table, WIDTH, &sizes, &both, 0.3).unwrap_or_default();
    assert!(two.iter().zip(&one).all(|(a, b)| *a >= b - 1e-9));
    let middle = at(&parts, 45, 45) as usize;
    assert!(two[middle] > one[middle]);
}

#[test]
fn same_object_scores_full() {
    let mut grid: Vec<f32> = (0..100).map(|i| if i % 10 >= 5 { 10.0 } else { 0.0 }).collect();
    grid[0] = 1.0;
    let parts: Vec<i32> = (0..100).map(|i| i32::from(i % 10 >= 5)).collect();
    let table = segments::table(&grid, 1, &parts);
    let score = similar::scores(&table, 2, &segments::areas(&parts), &[table[..2].to_vec()], segments::WEIGHT).unwrap_or_default();
    assert!((score[0] - 1.0).abs() < 1e-9);
    assert!(score[1] < 0.5);
}

#[test]
fn flat_site_has_no_scores() {
    let grid = vec![5.0f32; 2 * 36];
    let parts: Vec<i32> = (0..36).map(|i| i32::from(i % 6 >= 3)).collect();
    let table = segments::table(&grid, 2, &parts);
    assert!(similar::scores(&table, 4, &segments::areas(&parts), &[table[..4].to_vec()], segments::WEIGHT).is_none());
}

#[test]
fn area_weight_keeps_small_objects_from_scale() {
    // Тысяча мелких объектов не сжимает различие двух больших частей.
    let mut table = vec![0.0f32; 2 * 1002];
    table[2] = 100.0;
    let mut sizes = vec![1u64; 1002];
    sizes[0] = 10000;
    sizes[1] = 10000;
    let score = similar::scores(&table, 2, &sizes, &[table[..2].to_vec()], 0.0).unwrap_or_default();
    assert!(score[1] < 0.1);
}

/// Таблица модуля 4 × 1 × 3: значение - признак · 3 + объект.
fn numbers() -> Vec<f32> {
    (0..3).flat_map(|o| (0..4).map(move |f| (f * 3 + o) as f32)).collect()
}

#[test]
fn objects_filling_outline() {
    let parts = [0, 0, 0, 0, 0, 0, 0, 1, 1, 2];
    let rows = similar::sample_rows(&numbers(), 4, &parts, &[true; 10]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][1], 3.0);
}

#[test]
fn outline_over_two_objects_gives_two() {
    assert_eq!(similar::sample_rows(&numbers(), 4, &[0, 0, 1, 1], &[true; 4]).len(), 2);
}

#[test]
fn scattered_outline_takes_largest() {
    let mut parts: Vec<i32> = (0..12).map(|i| i % 6).collect();
    parts[0] = 1;
    let mut table = vec![0.0f32; 2 * 6];
    table[2] = 5.0;
    let rows = similar::sample_rows(&table, 2, &parts, &[true; 12]);
    assert_eq!(rows.iter().map(|r| r[0]).collect::<Vec<_>>(), [5.0]);
}

#[test]
fn outline_without_objects() {
    assert!(similar::sample_rows(&numbers(), 4, &[-1; 4], &[true; 4]).is_empty());
}

const PARTS: [i32; 8] = [0, 0, 1, 2, 0, 0, 1, 2];

#[test]
fn limit_moves_without_new_scores() {
    let score = [0.9, 0.5, 0.2];
    let wide = similar::pick(&PARTS, 2, 4, &score, 0.4, true).0;
    let tight = similar::pick(&PARTS, 2, 4, &score, 0.8, true).0;
    assert_eq!(wide.iter().filter(|l| **l >= 0).count(), 6);
    assert_eq!(tight.iter().filter(|l| **l >= 0).count(), 4);
}

#[test]
fn join_merges_neighbours_area_weighted() {
    let score = [0.9, 0.6, 0.2];
    let (out, mean) = similar::pick(&PARTS, 2, 4, &score, 0.5, true);
    assert_eq!(out.iter().max(), Some(&0));
    assert!((mean[0] - (4.0 * 0.9 + 2.0 * 0.6) / 6.0).abs() < 1e-9);
    let (out, _) = similar::pick(&PARTS, 2, 4, &score, 0.5, false);
    assert_eq!(out.iter().max(), Some(&1));
}

#[test]
fn nothing_found() {
    let (out, mean) = similar::pick(&PARTS, 2, 4, &[0.0; 3], 0.5, true);
    assert_eq!(out.iter().max(), Some(&-1));
    assert!(mean.is_empty());
}

#[test]
fn no_object_cells_stay_empty() {
    let (out, _) = similar::pick(&[0, -1, 1], 1, 3, &[1.0, 1.0], 0.5, false);
    assert_eq!(out[1], -1);
}
