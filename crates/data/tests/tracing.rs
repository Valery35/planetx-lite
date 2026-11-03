// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Трассировка: цвета, слияние мелких областей, общие цепочки границ,
//! сглаживание Potrace, кольца без щелей. Перенос `tests/test_tracing.py`
//! модуля, тест шагов [`tracing::Tracer`] - свой.

use data::tracing::{self, Params, Shape};

/// Простой генератор: тесту нужен шум, а не ряд NumPy.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f32) / (1u64 << 31) as f32
    }

    /// Нормальный шум, сумма двенадцати равномерных.
    fn normal(&mut self) -> f32 {
        (0..12).map(|_| self.next()).sum::<f32>() - 6.0
    }
}

const ROWS: usize = 60;
const COLS: usize = 80;

/// Сетка 60 × 80 с тремя каналами: фон, круг, квадрат с дырой и шум по
/// пикселям.
fn picture() -> Vec<f32> {
    let mut noise = Noise(5);
    let mut grid = vec![0.0f32; ROWS * COLS * 3];
    for r in 0..ROWS {
        for c in 0..COLS {
            let (dr, dc) = (r as i64 - 30, c as i64 - 20);
            let mut cell = [200.0f32; 3];
            if dr * dr + dc * dc < 144 {
                cell = [40.0; 3];
            }
            if (10..50).contains(&r) && (45..75).contains(&c) {
                cell = [120.0, 30.0, 30.0];
            }
            if (25..35).contains(&r) && (55..65).contains(&c) {
                cell = [200.0; 3];
            }
            for (k, v) in cell.iter().enumerate() {
                grid[(r * COLS + c) * 3 + k] = v + 6.0 * noise.normal();
            }
        }
    }
    grid
}

fn at(r: usize, c: usize) -> usize {
    r * COLS + c
}

fn total_area(shapes: &std::collections::BTreeMap<usize, Shape>) -> f64 {
    shapes.values().map(|(outer, holes)| tracing::ring_area(outer).abs() - holes.iter().map(|h| tracing::ring_area(h).abs()).sum::<f64>()).sum()
}

fn three() -> Params {
    Params { colors: 3, least: 10, ..Params::default() }
}

#[test]
fn three_colors_found() {
    let grid = picture();
    let good = vec![true; ROWS * COLS];
    let smooth = tracing::blur(&grid, 3, &good, ROWS, COLS, tracing::SMOOTH);
    let (classes, centers) = tracing::quantize(&smooth, 3, &good, 3, 0);
    assert_eq!(centers.len(), 3);
    let mut seen = classes.clone();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen, vec![0, 1, 2]);
    // Центр круга и угол фона - разные цвета, дыра квадрата - фон.
    assert_ne!(classes[at(30, 20)], classes[at(2, 2)]);
    assert_eq!(classes[at(30, 60)], classes[at(2, 2)]);
}

#[test]
fn outside_cells_stay_empty() {
    let grid = picture();
    let good: Vec<bool> = (0..ROWS * COLS).map(|i| i % COLS >= 10).collect();
    let (classes, _) = tracing::quantize(&grid, 3, &good, 3, 0);
    assert!((0..ROWS).all(|r| (0..10).all(|c| classes[at(r, c)] == -1)));
}

#[test]
fn small_spots_join_neighbour() {
    let (rows, cols) = (20, 20);
    let mut colors = vec![0i32; rows * cols];
    colors[5 * cols + 5] = 1;
    for r in 10..12 {
        for c in 10..12 {
            colors[r * cols + c] = 1;
        }
    }
    for slot in &mut colors[14 * cols..] {
        *slot = 2;
    }
    let (_, kinds, out) = tracing::merge_small(&colors, rows, cols, 10);
    assert_eq!(out[5 * cols + 5], 0);
    assert_eq!(out[10 * cols + 10], 0);
    let mut kinds = kinds;
    kinds.sort_unstable();
    assert_eq!(kinds, vec![0, 2]);
}

#[test]
fn square_in_field_is_closed_chain() {
    let mut parts = vec![0i32; 100];
    for r in 3..6 {
        for c in 3..6 {
            parts[r * 10 + c] = 1;
        }
    }
    let found = tracing::chains(&parts, 10, 10);
    let inner: Vec<_> = found.iter().filter(|c| [c.left, c.right].contains(&1) && [c.left, c.right].contains(&0)).collect();
    assert_eq!(inner.len(), 1);
    assert!(inner[0].closed);
    assert_eq!(inner[0].points.len() - 1, 12);
}

#[test]
fn left_side_is_region_on_screen_left() {
    // Отвесная граница: слева на экране (запад) область 0, справа 1.
    let parts = vec![0, 1, 0, 1];
    let found = tracing::chains(&parts, 2, 2);
    let middle = found.iter().find(|c| (c.left, c.right) == (0, 1) || (c.left, c.right) == (1, 0));
    let Some(middle) = middle else {
        return assert!(middle.is_some());
    };
    let going_down = middle.points[middle.points.len() - 1].1 > middle.points[0].1;
    assert_eq!(middle.left, if going_down { 1 } else { 0 });
}

#[test]
fn staircase_becomes_straight() {
    let mut steps = vec![(0.0, 0.0)];
    for k in 0..10 {
        let k = f64::from(k);
        steps.push((k + 1.0, k));
        steps.push((k + 1.0, k + 1.0));
    }
    assert_eq!(tracing::simplify(&steps, tracing::TOLERANCE, false).len(), 2);
}

#[test]
fn long_right_angle_stays_corner() {
    assert!(tracing::corner_alpha((0.0, 0.0), (20.0, 0.0), (20.0, 20.0)) >= tracing::CORNER);
    assert!(tracing::corner_alpha((0.0, 0.0), (3.0, 0.0), (3.0, 3.0)) < tracing::CORNER);
}

#[test]
fn open_chain_keeps_ends() {
    let out = tracing::smooth_chain(&[(0.0, 0.0), (5.0, 0.0), (5.0, 5.0), (10.0, 5.0)], false, tracing::CORNER, tracing::TOLERANCE);
    assert_eq!(out.first(), Some(&(0.0, 0.0)));
    assert_eq!(out.last(), Some(&(10.0, 5.0)));
}

#[test]
fn regions_tile_the_site_without_gaps() {
    let grid = picture();
    let good = vec![true; ROWS * COLS];
    let out = tracing::trace(&grid, 3, ROWS, COLS, &good, three());
    let Some(out) = out else {
        return assert!(out.is_some());
    };
    assert_eq!(out.shapes.len(), out.kinds.len());
    // Общие цепочки: сумма площадей областей - весь участок.
    assert!((total_area(&out.shapes) - (ROWS * COLS) as f64).abs() <= 1.0);
}

#[test]
fn square_has_hole() {
    let grid = picture();
    let good = vec![true; ROWS * COLS];
    let out = tracing::trace(&grid, 3, ROWS, COLS, &good, three());
    let Some(out) = out else {
        return assert!(out.is_some());
    };
    let square = usize::try_from(out.parts[at(15, 60)]).unwrap_or(usize::MAX);
    let shape = out.shapes.get(&square);
    let Some((_, holes)) = shape else {
        return assert!(shape.is_some());
    };
    assert_eq!(holes.len(), 1);
    assert!((tracing::ring_area(&holes[0]).abs() - 100.0).abs() <= 8.0);
}

#[test]
fn area_outside_site_is_not_traced() {
    let grid = picture();
    let good: Vec<bool> = (0..ROWS * COLS).map(|i| (5..55).contains(&(i / COLS)) && (5..75).contains(&(i % COLS))).collect();
    let out = tracing::trace(&grid, 3, ROWS, COLS, &good, three());
    let Some(out) = out else {
        return assert!(out.is_some());
    };
    assert!((total_area(&out.shapes) - 50.0 * 70.0).abs() <= 1.0);
}

#[test]
fn tracer_steps_give_the_same_result() {
    let grid = picture();
    let good = vec![true; ROWS * COLS];
    let whole = tracing::trace(&grid, 3, ROWS, COLS, &good, three());
    let mut tracer = tracing::Tracer::new(&grid, 3, ROWS, COLS, &good, three());
    let mut steps = 1;
    while !tracer.step() {
        steps += 1;
    }
    assert!(steps > 3);
    assert_eq!(tracer.finish(), whole);
}

#[test]
fn byte_colors_as_is() {
    assert_eq!(tracing::display_colors(&[vec![10.0, 20.0, 30.0], vec![200.0, 0.0, 5.0]]), vec![[10, 20, 30], [200, 0, 5]]);
}

#[test]
fn reflectance_stretched() {
    assert_eq!(tracing::display_colors(&[vec![0.1], vec![0.3]]), vec![[0, 0, 0], [255, 255, 255]]);
}

#[test]
fn untouched_colors_stay() {
    let (lookup, keep) = tracing::join_colors(&[-1, -1, -1]);
    assert_eq!(lookup, vec![0, 1, 2]);
    assert_eq!(keep, vec![0, 1, 2]);
}

#[test]
fn chain_goes_to_free_color() {
    // 0 слит с 2, 2 - с 3: оба уходят в 3.
    let (lookup, keep) = tracing::join_colors(&[2, -1, 3, -1]);
    assert_eq!(keep, vec![1, 3]);
    assert_eq!(lookup, vec![1, 0, 1, 1]);
}

#[test]
fn circle_is_one_class() {
    let (lookup, keep) = tracing::join_colors(&[1, 2, 0, -1]);
    assert_eq!(keep, vec![0, 3]);
    assert_eq!(lookup, vec![0, 0, 0, 1]);
}

#[test]
fn cells_and_regions_follow() {
    let (lookup, _) = tracing::join_colors(&[-1, 0, -1]);
    let (classes, kinds) = tracing::joined(&[0, 1, 2, -1], &[1, 2, 0], &lookup);
    assert_eq!(classes, vec![0, 0, 1, -1]);
    assert_eq!(kinds, vec![0, 1, 0]);
}