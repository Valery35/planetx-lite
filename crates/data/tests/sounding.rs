// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Батиметрия по промерам: шаг грида, рамка, маска, триангуляция. Перенос
//! `tests/test_sounding.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use data::sounding;

#[test]
fn nice_step() {
    assert_eq!(sounding::nice_step(4.47), 5.0);
    assert_eq!(sounding::nice_step(2.0), 2.0);
    assert_eq!(sounding::nice_step(0.3), 0.5);
    assert_eq!(sounding::nice_step(11.0), 20.0);
}

#[test]
fn tracks_twenty_meters_apart_give_five_meter_cell() {
    // Галсы через 20 м, точка через метр, участок 1 × 1 км.
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    for row in 0..50 {
        for col in 0..1000 {
            xs.push(f64::from(col));
            ys.push(f64::from(row) * 20.0);
        }
    }
    assert_eq!(sounding::auto_cell(&xs, &ys), 5.0);
}

#[test]
fn long_lake_in_empty_frame() {
    // Полоса 14 × 2 км по диагонали рамки 14 × 16 км: галсы через 50 м,
    // точка через 5 м. По рамке шаг вышел бы 50 м.
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    for row in 0..40 {
        for col in 0..2800 {
            let x = f64::from(col) * 5.0;
            xs.push(x);
            ys.push(x + f64::from(row) * 50.0);
        }
    }
    assert_eq!(sounding::auto_cell(&xs, &ys), 20.0);
}

#[test]
fn cell_limited_by_grid_size() {
    let xs: Vec<f64> = [0.0, 1.0e5, 0.0, 1.0e5].repeat(1000);
    let ys: Vec<f64> = [0.0, 0.0, 1.0e5, 1.0e5].repeat(1000);
    let cell = sounding::auto_cell(&xs, &ys);
    assert!((1.0e5 / cell).powi(2) <= sounding::MAX_CELLS as f64);
}

#[test]
fn frame_is_multiple_of_cell_and_covers_points() {
    let ((w, s, e, n), (cols, rows)) = sounding::grid_frame(&[3.2, 47.9], &[11.0, 18.5], 5.0);
    assert_eq!((w, s, e, n), (-5.0, 5.0, 55.0, 25.0));
    assert_eq!((cols, rows), (12, 4));
}

#[test]
fn negative_depth_and_nan() {
    let (xs, _, zs) = sounding::clean(&[0.0, 1.0, 2.0], &[0.0, 1.0, 2.0], &[-3.0, f64::NAN, 4.0]);
    assert_eq!(zs, [3.0, 4.0]);
    assert_eq!(xs, [0.0, 2.0]);
}

#[test]
fn plane_is_interpolated_and_far_cells_stay_empty() {
    // Глубина растёт к востоку на 1 м на 10 м, две точки промеров рядом,
    // и остров промеров далеко.
    let mut pts: Vec<(f64, f64, f64)> = Vec::new();
    for y in [0.0, 100.0] {
        for x in [0.0, 50.0, 100.0] {
            pts.push((x, y, x / 10.0));
        }
    }
    pts.push((1000.0, 50.0, 100.0));
    let (xs, ys, zs): (Vec<f64>, Vec<f64>, Vec<f64>) = (pts.iter().map(|p| p.0).collect(), pts.iter().map(|p| p.1).collect(), pts.iter().map(|p| p.2).collect());
    let mut build = sounding::Build::new(&xs, &ys, &zs, 10.0, 60.0).expect("триангуляция");
    while !build.step(16) {}
    let at = |x: f64, y: f64| {
        let col = ((x - build.west) / build.cell).floor() as usize;
        let row = ((build.north - y) / build.cell).floor() as usize;
        build.depths[row * build.cols + col]
    };
    // Середина ячейки у (55, 55): глубина по плоскости 5.5 м.
    assert!((at(55.0, 55.0) - 5.5).abs() < 1e-6, "{}", at(55.0, 55.0));
    // Между промерами и дальней точкой триангуляция есть, промера рядом нет.
    assert!(at(500.0, 50.0).is_nan());
    assert!(build.known() > 0);
}

#[test]
fn too_few_points_give_nothing() {
    assert!(sounding::Build::new(&[0.0, 1.0], &[0.0, 1.0], &[1.0, 2.0], 1.0, 10.0).is_none());
}
