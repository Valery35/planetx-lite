// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Выемка и насыпь. Перенос `tests/test_earthwork.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use data::hydro::{self, Grid};
use data::volumes;

const RADIUS: f64 = 6378137.0;

fn plain(rows: usize) -> Grid {
    hydro::make_grid(58.0, 56.0, 50.0 * (rows - 1) as f64, 100.0, RADIUS)
}

fn flat(grid: &Grid, value: f64) -> Vec<f64> {
    vec![value; grid.rows * grid.cols]
}

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6 * b.abs().max(1.0), "{a} {b}");
}

#[test]
fn hill_above_level_is_cut() {
    let grid = plain(11);
    let mut z = flat(&grid, 100.0);
    z[5 * grid.cols + 5] = 130.0;
    let work = volumes::earthwork(&z, &grid, 100.0);
    let out = volumes::summary(&work.diff, grid.cell.powi(2), volumes::THRESHOLD);
    close(out.cut, 30.0 * 100.0f64.powi(2));
    assert_eq!(out.fill, 0.0);
}

#[test]
fn pit_below_level_is_fill() {
    let grid = plain(11);
    let mut z = flat(&grid, 100.0);
    z[2 * grid.cols + 3] = 90.0;
    let work = volumes::earthwork(&z, &grid, 100.0);
    let out = volumes::summary(&work.diff, grid.cell.powi(2), volumes::THRESHOLD);
    close(out.fill, 10.0 * 100.0f64.powi(2));
    assert_eq!(out.cut, 0.0);
}

#[test]
fn balance_level_gives_equal_volumes() {
    let grid = plain(11);
    let mut z: Vec<f64> = (0..grid.rows).flat_map(|r| std::iter::repeat_n(100.0 + 3.0 * r as f64, grid.cols)).collect();
    z[0] = f64::NAN;
    let level = volumes::balance_level(&z).expect("есть высоты");
    let work = volumes::earthwork(&z, &grid, level);
    let out = volumes::summary(&work.diff, grid.cell.powi(2), 0.0);
    assert!(out.net.abs() < 1e-3, "{}", out.net);
    assert!(out.cut > 0.0);
}

#[test]
fn no_heights_no_level() {
    assert_eq!(volumes::balance_level(&[f64::NAN; 9]), None);
}

#[test]
fn outside_area_is_not_counted() {
    let grid = plain(11);
    let mut z = flat(&grid, 90.0);
    for r in 0..grid.rows {
        for c in 5..grid.cols {
            z[r * grid.cols + c] = f64::NAN;
        }
    }
    let work = volumes::earthwork(&z, &grid, 100.0);
    let out = volumes::summary(&work.diff, grid.cell.powi(2), volumes::THRESHOLD);
    close(out.fill, 10.0 * 5.0 * grid.rows as f64 * 100.0f64.powi(2));
}

#[test]
fn share_and_error() {
    let grid = plain(11);
    let z = flat(&grid, 100.0);
    let work = volumes::earthwork(&z, &grid, 100.0);
    let half = grid.x0 + ((grid.cols / 2) as f64 + 0.5) * grid.step;
    let bounds = (grid.x0 - 1.0, grid.y0 - 1e6, half, grid.y0 + 1.0);
    close(volumes::own_share(&work, &[bounds]), 6.0 / 11.0);
    let whole = (grid.rows * grid.cols) as f64 * 100.0f64.powi(2);
    close(volumes::open_error(&work, 0.0), whole * volumes::OPEN_ERROR);
    assert_eq!(volumes::open_error(&work, 1.0), 0.0);
    assert_eq!(volumes::own_share(&work, &[]), 0.0);
}

#[test]
fn colors_follow_sign() {
    let grid = plain(11);
    let mut z = flat(&grid, 100.0);
    z[2 * grid.cols + 5] = 150.0; // выемка
    z[8 * grid.cols + 5] = 50.0; // насыпь
    z[5 * grid.cols + 8] = f64::NAN;
    let work = volumes::earthwork(&z, &grid, 100.0);
    let at = |r: f64, c: f64| hydro::latlon(&grid, r, c, RADIUS);
    let rgba = volumes::points_rgba(&work, &[at(2.0, 5.0), at(8.0, 5.0), at(5.0, 8.0), at(5.0, 2.0), at(40.0, 5.0)], RADIUS);
    let plain_color = |p: [u8; 4]| [0, 1, 2].map(|i| (f64::from(p[i]) * 255.0 / f64::from(p[3])).round() as u8);
    assert_eq!(rgba[0][3], volumes::ALPHA);
    // Премножение округляет цвет на единицу.
    let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 1);
    assert!(near(plain_color(rgba[0]), volumes::CUT_RGB), "{:?}", rgba[0]);
    assert!(near(plain_color(rgba[1]), volumes::FILL_RGB), "{:?}", rgba[1]);
    assert_eq!(rgba[2][3], 0); // вне участка
    assert_eq!(rgba[3][3], 0); // на отметке площадки
    assert_eq!(rgba[4][3], 0); // за сеткой
}

#[test]
fn difference_skips_gaps_and_outside() {
    let old = [1.0, f64::NAN, 3.0, 4.0];
    let new = [2.0, 2.0, 1.0, 9.0];
    let diff = volumes::difference(&old, &new, Some(&[true, true, true, false]));
    assert_eq!(diff[0], 1.0);
    assert!(diff[1].is_nan());
    assert_eq!(diff[2], -2.0);
    assert!(diff[3].is_nan());
}

#[test]
fn summary_of_two_surveys() {
    let out = volumes::summary(&[2.0, -1.0, 0.05, f64::NAN], 4.0, 0.1);
    assert_eq!((out.fill, out.cut, out.net), (8.0, 4.0, 4.0));
    assert_eq!((out.fill_area, out.cut_area, out.area), (4.0, 4.0, 12.0));
    assert_eq!((out.deepest, out.highest), (-1.0, 2.0));
}

#[test]
fn file_values_mark_gaps_and_noise() {
    let out = volumes::for_file(&[f64::NAN, 0.05, 2.0], 0.1);
    assert_eq!(out, vec![volumes::NODATA, 0.0, 2.0]);
}
