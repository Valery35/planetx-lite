// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Гидрология рельефа: D8, аккумуляция, сеть, бассейны, линии стока.
//! Перенос `tests/test_topo_flow.py` и `tests/test_topo_trace.py` модуля
//! (из Isoliner).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use data::hydro_fill::fill_depressions;
use data::topo_flow::{self as tf, PathReason};

fn uniform(seed: u64, n: usize) -> Vec<f64> {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            s ^= s >> 12;
            s ^= s << 25;
            s ^= s >> 27;
            (s.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
        })
        .collect()
}

/// Наклон на восток: сток строго E (код 1).
fn plane_east(rows: usize, cols: usize) -> Vec<f64> {
    (0..rows * cols).map(|i| 100.0 - (i % cols) as f64).collect()
}

fn filled(z: &[f64], rows: usize, cols: usize, eps: f64) -> Vec<f64> {
    fill_depressions(z, rows, cols, None, eps, None).unwrap().filled
}

// --- D8 ---------------------------------------------------------------------

#[test]
fn east_plane() {
    let (rows, cols) = (8, 10);
    let z = plane_east(rows, cols);
    let (dir, down) = tf::d8_directions(&z, rows, cols, None);
    let esri = tf::dir_to_esri(&dir);
    for r in 0..rows {
        // Весь грид, кроме восточного края, течёт на восток.
        for c in 0..cols - 1 {
            assert_eq!(esri[r * cols + c], 1);
        }
        // Восточный край уходит с грида.
        assert_eq!(dir[r * cols + cols - 1], tf::NODIR);
        assert_eq!(down[r * cols + cols - 1], -1);
    }
}

#[test]
fn diagonal_wins() {
    // Уклон по диагонали круче осевого на достаточную величину.
    let z = [3.0, 2.0, 2.0, 0.5];
    let (dir, _) = tf::d8_directions(&z, 2, 2, None);
    // SE.
    assert_eq!(tf::dir_to_esri(&dir)[0], 2);
}

#[test]
fn nodata_is_outlet() {
    let z = plane_east(4, 4);
    let mut mask = vec![false; 16];
    mask[4 + 2] = true;
    let (dir, down) = tf::d8_directions(&z, 4, 4, Some(&mask));
    assert_eq!(dir[4 + 2], tf::NODIR);
    // Западный сосед nodata льёт в неё как в слив.
    assert_eq!(down[4 + 1], -1);
}

#[test]
fn pit_is_sink() {
    let mut z = plane_east(5, 5);
    z[12] = 0.0;
    let (dir, down) = tf::d8_directions(&z, 5, 5, None);
    assert_eq!(dir[12], tf::NODIR);
    assert_eq!(down[12], -1);
}

// --- аккумуляция --------------------------------------------------------------

#[test]
fn east_plane_rows() {
    let z = plane_east(3, 6);
    let (_, down) = tf::d8_directions(&z, 3, 6, None);
    let acc = tf::flow_accumulation(&down, 3, 6, None);
    // Вдоль строки аккумуляция растёт 1..nx.
    for r in 0..3 {
        for c in 0..6 {
            assert_eq!(acc[r * 6 + c], (c + 1) as f64);
        }
    }
}

#[test]
fn total_conservation() {
    let (rows, cols) = (40, 50);
    let noise = uniform(7, rows * cols);
    let z: Vec<f64> = (0..rows * cols).map(|i| noise[i] * 10.0 + (i % cols) as f64 * 0.5).collect();
    let f = filled(&z, rows, cols, 1e-3);
    let (_, down) = tf::d8_directions(&f, rows, cols, None);
    let acc = tf::flow_accumulation(&down, rows, cols, None);
    // Каждая ячейка считает себя: минимум 1, максимум не больше N.
    assert!(acc.iter().all(|a| *a >= 1.0 && *a <= (rows * cols) as f64));
    // Каждая ячейка доходит до какого-то устья.
    let lab = tf::basins(&down, rows, cols, None, None, None, None);
    assert!(lab.iter().all(|l| *l > 0));
}

#[test]
fn v_valley_outlet() {
    // V-долина, падающая на юг: вся вода в одной ячейке выхода.
    let (rows, cols) = (20, 11);
    let z: Vec<f64> = (0..rows * cols).map(|i| ((i % cols) as f64 - 5.0).abs() * 2.0 + (rows - i / cols) as f64 * 0.5).collect();
    let f = filled(&z, rows, cols, 1e-4);
    let (_, down) = tf::d8_directions(&f, rows, cols, None);
    let acc = tf::flow_accumulation(&down, rows, cols, None);
    assert_eq!(acc[(rows - 1) * cols + 5], (rows * cols) as f64);
}

// --- речная сеть --------------------------------------------------------------

/// Два истока сливаются в один ствол (форма Y, падение на юг).
fn y_network() -> (Vec<i64>, Vec<f64>, usize, usize) {
    let (rows, cols) = (30, 21);
    let mut z: Vec<f64> = (0..rows * cols).map(|i| ((i % cols) as f64 - 10.0).abs() * 3.0 + (rows - i / cols) as f64).collect();
    // Два боковых оврага в верхней половине.
    for r in 0..12 {
        z[r * cols + 4 + r / 3] -= 2.5;
        z[r * cols + 16 - r / 3] -= 2.5;
    }
    let f = filled(&z, rows, cols, 1e-4);
    let (_, down) = tf::d8_directions(&f, rows, cols, None);
    let acc = tf::flow_accumulation(&down, rows, cols, None);
    (down, acc, rows, cols)
}

#[test]
fn strahler_increases() {
    let (down, acc, rows, cols) = y_network();
    let links = tf::river_network(&down, &acc, 8.0, rows, cols);
    assert!(links.len() > 1);
    let orders: Vec<u32> = links.iter().map(|l| l.order).collect();
    assert_eq!(orders.iter().min(), Some(&1));
    assert!(orders.iter().max().is_some_and(|m| *m >= 2));
    // Звенья направлены вниз по течению: аккумуляция не убывает.
    for link in &links {
        assert!(link.cells.len() > 1);
        assert!(acc[link.cells[0]] <= acc[link.cells[link.cells.len() - 1]] + 1e-9);
    }
}

#[test]
fn threshold_prunes() {
    let (down, acc, rows, cols) = y_network();
    let many: usize = tf::river_network(&down, &acc, 5.0, rows, cols).iter().map(|l| l.cells.len()).sum();
    let few: usize = tf::river_network(&down, &acc, 100.0, rows, cols).iter().map(|l| l.cells.len()).sum();
    assert!(many > few);
}

// --- бассейны -----------------------------------------------------------------

#[test]
fn ridge_splits_two() {
    // Хребет по центру: западная половина льёт на запад, восточная на восток.
    let (rows, cols) = (10, 21);
    let z: Vec<f64> = (0..rows * cols).map(|i| -((i % cols) as f64 - 10.0).abs() * 2.0 + 100.0 + (i / cols) as f64 * 1e-6).collect();
    let f = filled(&z, rows, cols, 1e-6);
    let (_, down) = tf::d8_directions(&f, rows, cols, None);
    let acc = tf::flow_accumulation(&down, rows, cols, None);
    let top = acc.iter().copied().fold(0.0, f64::max);
    let lab = tf::basins(&down, rows, cols, None, None, Some(&acc), Some(top * 0.4));
    let mut got: Vec<i32> = lab.iter().copied().filter(|l| *l != 0).collect();
    got.sort();
    got.dedup();
    assert!(got.len() >= 2);
}

#[test]
fn seed_catches_upstream() {
    let (rows, cols) = (6, 12);
    let z = plane_east(rows, cols);
    let (_, down) = tf::d8_directions(&z, rows, cols, None);
    // Семя в середине строки 2: всё западнее в его бассейне.
    let lab = tf::basins(&down, rows, cols, Some(&[(2 * 12 + 6, 5)]), None, None, None);
    assert_eq!(lab[2 * 12 + 6], 5);
    assert!((0..6).all(|c| lab[2 * 12 + c] == 5));
    assert!((0..12).all(|c| lab[3 * 12 + c] != 5));
    // Восточнее семени вода уже прошла: метки нет.
    assert!((7..12).all(|c| lab[2 * 12 + c] == 0));
}

#[test]
fn pointer_doubling_long_chain() {
    // Длинная цепочка из 5000 ячеек в одну сторону.
    let z = plane_east(1, 5000);
    let (_, down) = tf::d8_directions(&z, 1, 5000, None);
    let lab = tf::basins(&down, 1, 5000, None, None, None, None);
    assert!(lab.iter().all(|l| *l == lab[4999]));
    assert!(lab[0] > 0);
}

#[test]
fn hostile_pointers_do_not_panic() {
    // Указатели за пределы решётки и круг.
    let down = [5, 99, 0, -7, 2, 3];
    let acc = tf::flow_accumulation(&down, 2, 3, None);
    assert_eq!(acc.len(), 6);
    let lab = tf::basins(&down, 2, 3, None, None, None, None);
    assert_eq!(lab.len(), 6);
    let _ = tf::river_network(&down, &acc, 1.0, 2, 3);
    assert!(tf::flow_accumulation(&down, 5, 5, None).is_empty());
    assert!(tf::d8_directions(&[1.0], 3, 3, None).1.is_empty());
}

// --- линии стока: ход вниз по D8 от заданных ячеек -------------------------------

fn slope(rows: usize, cols: usize, step: f64) -> Vec<f64> {
    (0..rows * cols).map(|i| 200.0 - (i / cols) as f64 * step).collect()
}

#[test]
fn trace_runs_downhill_to_the_edge() {
    let z = slope(20, 20, 1.0);
    let (_, ds) = tf::d8_directions(&z, 20, 20, None);
    let path = &tf::trace_downhill(&ds, &[10], 20, 20, None, None, None)[0];
    assert_eq!(path.len(), 20);
    // Трасса обязана идти строго вниз.
    assert_eq!(path.iter().map(|i| i / 20).collect::<Vec<_>>(), (0..20).collect::<Vec<_>>());
    assert_eq!(tf::path_reason(path, &ds, 20, 20, None, None), PathReason::Edge);
}

#[test]
fn trace_stops_in_a_pit() {
    // Незаполненная впадина обрывает трассу: ниже некуда.
    let mut z = slope(20, 20, 1.0);
    z[10 * 20 + 10] = 150.0;
    let (_, ds) = tf::d8_directions(&z, 20, 20, None);
    let path = &tf::trace_downhill(&ds, &[5 * 20 + 10], 20, 20, None, None, None)[0];
    assert_eq!(path.last(), Some(&(10 * 20 + 10)));
    assert_eq!(tf::path_reason(path, &ds, 20, 20, None, None), PathReason::Sink);
}

#[test]
fn trace_stops_at_a_receiver() {
    // Водоток или водоём принимает трассу и дальше её не ведёт.
    let z = slope(20, 20, 1.0);
    let (_, ds) = tf::d8_directions(&z, 20, 20, None);
    let mut stop = vec![false; 400];
    for c in 0..20 {
        stop[12 * 20 + c] = true;
    }
    let path = &tf::trace_downhill(&ds, &[10], 20, 20, Some(&stop), None, None)[0];
    assert_eq!(path.last(), Some(&(12 * 20 + 10)));
    assert_eq!(tf::path_reason(path, &ds, 20, 20, Some(&stop), None), PathReason::Receiver);
}

#[test]
fn merging_traces_walk_each_cell_once() {
    // С обрывом при слиянии выходит дерево, а не пачка копий русла.
    let z: Vec<f64> = slope(30, 30, 1.0).iter().enumerate().map(|(i, v)| v + ((i % 30) as f64 - 15.0).abs() * 0.2).collect();
    let (_, ds) = tf::d8_directions(&z, 30, 30, None);
    let starts: Vec<i64> = (10..21).map(|c| 2 * 30 + c).collect();
    let loose = tf::trace_downhill(&ds, &starts, 30, 30, None, None, None);
    let mut seen = vec![false; 900];
    let tree = tf::trace_downhill(&ds, &starts, 30, 30, None, Some(&mut seen), None);
    assert!(loose.iter().map(Vec::len).sum::<usize>() > tree.iter().map(Vec::len).sum::<usize>());
    // Трасса доводится до места слияния и там обрывается.
    let inner: Vec<usize> = tree.iter().flat_map(|p| p[..p.len().saturating_sub(1)].iter().copied()).collect();
    let mut unique = inner.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(inner.len(), unique.len(), "ячейка пройдена дважды");
    assert!(tree.iter().all(|p| !p.is_empty()));
}

#[test]
fn metrics_measure_the_path() {
    let z = slope(11, 11, 2.0);
    let (_, ds) = tf::d8_directions(&z, 11, 11, None);
    let path = &tf::trace_downhill(&ds, &[5], 11, 11, None, None, None)[0];
    let m = tf::path_metrics(path, &z, 11, 11, 10.0, None);
    assert_eq!(m.cells, 11);
    // Десять шагов по 10 м.
    assert!((m.length - 100.0).abs() < 1e-9);
    assert!((m.drop - 20.0).abs() < 1e-9);
    assert!((m.slope - 0.2).abs() < 1e-9);
    assert!(m.z_start > m.z_end);
}

#[test]
fn metrics_take_a_rectangular_cell() {
    let z = slope(5, 5, 1.0);
    let (_, ds) = tf::d8_directions(&z, 5, 5, None);
    let path = &tf::trace_downhill(&ds, &[2], 5, 5, None, None, None)[0];
    let m = tf::path_metrics(path, &z, 5, 5, 10.0, Some(25.0));
    assert!((m.length - 100.0).abs() < 1e-9, "{}", m.length);
}

#[test]
fn single_cell_path_is_not_an_error() {
    let z = slope(20, 20, 1.0);
    let m = tf::path_metrics(&[19 * 20 + 3], &z, 20, 20, 5.0, None);
    assert!(m.cells == 1 && m.length == 0.0 && m.slope == 0.0);
}

#[test]
fn start_outside_the_grid_gives_an_empty_path() {
    let z = slope(20, 20, 1.0);
    let (_, ds) = tf::d8_directions(&z, 20, 20, None);
    assert_eq!(tf::trace_downhill(&ds, &[-1, 1_000_000], 20, 20, None, None, None), vec![Vec::<usize>::new(), Vec::new()]);
}

#[test]
fn loop_in_a_foreign_grid_does_not_hang() {
    // Указатели из чужого инструмента могут зациклиться, ход всё равно
    // обязан закончиться.
    let ds = [1, 0];
    assert_eq!(tf::trace_downhill(&ds, &[0], 1, 2, None, None, None)[0], vec![0, 1]);
}

#[test]
fn flattening_cut_and_steep_run() {
    // Склон 1:1 на 10 ячеек, дальше ровно: обрезка по началу окна 30 м,
    // в котором уклона уже нет, - у подножия.
    let cols = 30;
    let z: Vec<f64> = (0..cols).map(|c| if c < 10 { (10 - c) as f64 * 10.0 } else { 0.0 }).collect();
    let path: Vec<usize> = (0..cols).collect();
    let (cut, done) = tf::cut_on_flattening(&path, &z, 1, cols, 10.0, tf::slope_from_degrees(2.0), 30.0, None);
    assert!(done);
    assert_eq!(cut, (0..11).collect::<Vec<_>>());
    // Средний уклон 0.5 держится от вершины до 200 м: 100 м падения.
    let (len, i, j) = tf::steep_run(&path, &z, 1, cols, 10.0, 0.5, None);
    assert_eq!((i, j), (0, 20));
    assert!((len - 200.0).abs() < 1e-9);
    assert!((tf::slope_from_degrees(45.0) - 1.0).abs() < 1e-12);
}
