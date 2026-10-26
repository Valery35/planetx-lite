// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Морфометрия: уклон, экспозиция, вершины. Перенос
//! `tests/test_topo_surface.py` модуля (из Isoliner).

use data::topo_surface::{self as ts, Extreme};

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// Внутренние узлы сетки rows × cols без рамки в один узел.
fn inner(rows: usize, cols: usize) -> Vec<usize> {
    (1..rows - 1).flat_map(|r| (1..cols - 1).map(move |c| r * cols + c)).collect()
}

#[test]
fn east_facing_plane() {
    // Высота падает на восток: экспозиция 90, уклон atan(g).
    let (cell, g, rows, cols) = (10.0, 0.2, 20, 30);
    let z: Vec<f64> = (0..rows * cols).map(|i| 1000.0 - (i % cols) as f64 * g * cell).collect();
    let (slope, aspect) = ts::slope_aspect(&z, rows, cols, cell, None);
    let want = g.atan().to_degrees();
    for i in inner(rows, cols) {
        assert!(close(slope[i], want, 1e-6));
        assert!(close(aspect[i], 90.0, 1e-6));
    }
}

#[test]
fn north_facing_plane() {
    // Высота растёт на юг (по строкам): смотрит на север, 0 градусов.
    let (cell, rows, cols) = (5.0, 25, 15);
    let z: Vec<f64> = (0..rows * cols).map(|i| (i / cols) as f64).collect();
    let (slope, aspect) = ts::slope_aspect(&z, rows, cols, cell, None);
    for i in inner(rows, cols) {
        assert!(close(aspect[i], 0.0, 1e-6));
        assert!(slope[i] > 0.0);
    }
}

#[test]
fn flat_aspect_minus_one() {
    let z = vec![500.0; 100];
    let (slope, aspect) = ts::slope_aspect(&z, 10, 10, 30.0, None);
    assert!(slope.iter().all(|v| close(*v, 0.0, 1e-12)));
    assert!(aspect.iter().all(|v| close(*v, -1.0, 1e-12)));
}

#[test]
fn nodata_hole_gives_nan() {
    let (rows, cols) = (10, 12);
    let z: Vec<f64> = (0..rows * cols).map(|i| (i % cols) as f64).collect();
    let mut mask = vec![false; rows * cols];
    mask[5 * cols + 6] = true;
    let (slope, _aspect) = ts::slope_aspect(&z, rows, cols, 10.0, Some(&mask));
    assert!(slope[5 * cols + 6].is_nan());
    // Сосед дыры тоже NaN.
    assert!(slope[5 * cols + 5].is_nan());
    assert!(!slope[5 * cols + 3].is_nan());
}

#[test]
fn hostile_input_does_not_panic() {
    let (s, a) = ts::slope_aspect(&[1.0, 2.0], 3, 3, 10.0, None);
    assert!(s.is_empty() && a.is_empty());
    assert!(ts::find_peaks(&[], 0, 0, 10.0, 50.0, 1.0, None).is_empty());
    assert!(ts::find_extremes(&[f64::NAN; 9], 3, 3, f64::NAN, f64::NAN, 1.0, None).is_empty());
}

fn gauss(rows: usize, cols: usize, bumps: &[(f64, f64, f64, f64)], base: f64) -> Vec<f64> {
    let mut z = vec![base; rows * cols];
    for r in 0..rows {
        for c in 0..cols {
            for (h, cx, cy, s) in bumps {
                let (x, y) = (c as f64, r as f64);
                z[r * cols + c] += h * (-((x - cx).powi(2) + (y - cy).powi(2)) / s).exp();
            }
        }
    }
    z
}

fn two_hills() -> Vec<f64> {
    gauss(60, 80, &[(50.0, 20.0, 30.0, 60.0), (30.0, 60.0, 25.0, 40.0), (0.5, 40.0, 50.0, 10.0)], 0.0)
}

#[test]
fn two_hills_found_bump_filtered() {
    let z = two_hills();
    let peaks = ts::find_peaks(&z, 60, 80, 10.0, 80.0, 5.0, None);
    assert_eq!(peaks.len(), 2);
    let ((r1, c1, z1, d1), (r2, c2, z2, _d2)) = (peaks[0], peaks[1]);
    // Сортировка по убыванию высоты.
    assert!(z1 > z2);
    assert!(r1.abs_diff(30) <= 1 && c1.abs_diff(20) <= 1);
    assert!(r2.abs_diff(25) <= 1 && c2.abs_diff(60) <= 1);
    assert!(d1 > 5.0);
}

#[test]
fn min_drop_zero_keeps_bump() {
    let z = two_hills();
    assert_eq!(ts::find_peaks(&z, 60, 80, 10.0, 80.0, 0.1, None).len(), 3);
}

#[test]
fn plateau_gives_single_peak() {
    let mut z = vec![0.0; 900];
    for r in 10..13 {
        for c in 10..13 {
            // Плоская макушка 3x3.
            z[r * 30 + c] = 7.0;
        }
    }
    let peaks = ts::find_peaks(&z, 30, 30, 10.0, 50.0, 1.0, None);
    assert_eq!(peaks.len(), 1);
    // Остаётся северо-западная ячейка плато.
    assert_eq!((peaks[0].0, peaks[0].1), (10, 10));
}

#[test]
fn radius_separates() {
    let mut z = vec![0.0; 20 * 60];
    z[10 * 60 + 10] = 5.0;
    z[10 * 60 + 40] = 6.0;
    assert_eq!(ts::find_peaks(&z, 20, 60, 10.0, 100.0, 0.5, None).len(), 2);
    assert_eq!(ts::find_peaks(&z, 20, 60, 10.0, 400.0, 0.5, None).len(), 1);
}

/// Вершина и яма разом. Пришло из чата сообщества: поверхность в АвтоКАДе
/// по одним горизонталям кладёт плоскую шапку на каждую замкнутую
/// горизонталь, и пикеты нужны сразу на ямы и вершины.
fn hill_and_pit() -> Vec<f64> {
    gauss(60, 80, &[(50.0, 20.0, 30.0, 60.0), (-40.0, 60.0, 30.0, 40.0)], 100.0)
}

#[test]
fn both_kinds_found() {
    let got = ts::find_extremes(&hill_and_pit(), 60, 80, 10.0, 150.0, 5.0, None);
    let mut kinds: Vec<Extreme> = got.iter().map(|t| t.4).collect();
    kinds.sort();
    assert_eq!(kinds, vec![Extreme::Peak, Extreme::Pit]);
}

#[test]
fn pit_keeps_true_elevation_and_positive_depth() {
    // Обращение рельефа - деталь реализации, наружу знаки выходят исходные.
    let z = hill_and_pit();
    let got = ts::find_extremes(&z, 60, 80, 10.0, 150.0, 5.0, None);
    let pit = got.iter().find(|t| t.4 == Extreme::Pit).copied();
    let Some((r, c, zv, dv, _)) = pit else {
        unreachable!("яма не найдена");
    };
    assert!(close(zv, z[r * 80 + c], 1e-9));
    // Яма ниже равнины, глубина положительная.
    assert!(zv < 100.0);
    assert!(dv > 5.0);
}

#[test]
fn peak_half_matches_find_peaks() {
    let z = hill_and_pit();
    let mut old = ts::find_peaks(&z, 60, 80, 10.0, 150.0, 5.0, None);
    let mut new: Vec<(usize, usize, f64, f64)> =
        ts::find_extremes(&z, 60, 80, 10.0, 150.0, 5.0, None).into_iter().filter(|t| t.4 == Extreme::Peak).map(|t| (t.0, t.1, t.2, t.3)).collect();
    old.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    new.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    assert_eq!(old, new);
}

#[test]
fn flat_field_gives_nothing() {
    let z = vec![7.0; 1600];
    assert!(ts::find_extremes(&z, 40, 40, 10.0, 100.0, 1.0, None).is_empty());
}

#[test]
fn nodata_respected() {
    let z = hill_and_pit();
    let mut mask = vec![false; z.len()];
    for r in 20..40 {
        for c in 50..70 {
            // Яма вырезана из данных.
            mask[r * 80 + c] = true;
        }
    }
    let got = ts::find_extremes(&z, 60, 80, 10.0, 150.0, 5.0, Some(&mask));
    assert_eq!(got.iter().map(|t| t.4).collect::<Vec<_>>(), vec![Extreme::Peak]);
}
