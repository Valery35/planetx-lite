// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Анализ рельефа квадрата: уклон, сводка, раскраска тайла. Перенос
//! `tests/test_relief.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use data::hydro;
use data::relief::{self, Mode};
use data::topo_surface::Extreme;

const R: f64 = 6371000.0;

fn grid() -> hydro::Grid {
    hydro::make_grid(55.0, 37.0, 500.0, 10.0, R)
}

/// Высоты, падающие на восток с уклоном gradient.
fn east_plane(grid: &hydro::Grid, gradient: f64) -> Vec<f64> {
    (0..grid.rows * grid.cols).map(|i| 1000.0 - (i % grid.cols) as f64 * gradient * grid.cell).collect()
}

#[test]
fn east_plane_slope_and_aspect() {
    let g = grid();
    let result = relief::analyse(&east_plane(&g, 0.2), &g, R, Mode::Slope);
    let want = 0.2f64.atan().to_degrees();
    for r in 1..g.rows - 1 {
        for c in 1..g.cols - 1 {
            assert!((result.slope[r * g.cols + c] - want).abs() < 1e-8);
            assert!((result.aspect[r * g.cols + c] - 90.0).abs() < 1e-8);
        }
    }
    let (south, north, west, east) = result.bbox;
    assert!(south < 55.0 && north > 55.0 && west < 37.0 && east > 37.0);
}

#[test]
fn flat_has_no_aspect() {
    let g = grid();
    let result = relief::analyse(&vec![200.0; g.rows * g.cols], &g, R, Mode::Slope);
    assert!(result.aspect.iter().all(|a| a.is_nan()));
    let rep = relief::summary(&result).unwrap();
    assert_eq!(rep.flat, 1.0);
    assert_eq!(rep.sides.iter().sum::<f64>(), 0.0);
}

#[test]
fn summary_areas_and_sides() {
    let g = grid();
    let result = relief::analyse(&east_plane(&g, 0.2), &g, R, Mode::Slope);
    let rep = relief::summary(&result).unwrap();
    let total = (g.rows * g.cols) as f64 * g.cell * g.cell;
    assert!((rep.classes.iter().map(|c| c.1).sum::<f64>() - total).abs() < 1e-6);
    // 11.3° - класс от 10°, у крайних столбцов ядро даёт половину.
    let area = rep.classes.iter().find(|c| c.0 == 10.0).map(|c| c.1).unwrap();
    let edge = 2.0 * g.rows as f64 * g.cell * g.cell;
    assert!((area - (total - edge)).abs() < 1e-6);
    let east = relief::SIDES.iter().position(|s| *s == "E").unwrap();
    assert!((rep.sides[east] - 1.0).abs() < 1e-12);
    assert!((rep.median - 0.2f64.atan().to_degrees()).abs() < 1e-9);
}

#[test]
fn points_inside_and_outside() {
    let g = grid();
    let mut result = relief::analyse(&east_plane(&g, 0.2), &g, R, Mode::Slope);
    let points = [(55.0, 37.0), (56.0, 37.0)];
    let rgba = relief::points_rgba(&result, &points, R);
    assert!(rgba[3] > 0);
    assert_eq!(rgba[7], 0);
    result.mode = Mode::Aspect;
    let aspect = relief::points_rgba(&result, &points, R);
    assert!(aspect[3] > 0);
    assert_eq!(aspect[7], 0);
    assert!(relief::sample(&result, 56.0, 37.0, R).is_none());
    assert!(relief::sample(&result, 55.0, 37.0, R).is_some_and(|v| (v - 90.0).abs() < 1e-8));
}

#[test]
fn tile_inside_and_outside() {
    let g = grid();
    let result = relief::analyse(&east_plane(&g, 0.2), &g, R, Mode::Slope);
    let zoom = 15u32;
    let scale = f64::from(1u32 << zoom);
    let x = ((37.0 + 180.0) / 360.0 * scale) as u32;
    let lat = 55f64.to_radians();
    let y = ((1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * scale) as u32;
    let rgba = relief::tile_rgba(&result, (zoom, x, y), R).unwrap();
    assert_eq!(rgba.len(), 256 * 256 * 4);
    assert!(rgba.chunks(4).any(|p| p[3] > 0));
    assert!(relief::tile_rgba(&result, (zoom, x + 40, y), R).is_none());
}

#[test]
fn extremes_of_hill() {
    let g = grid();
    let z: Vec<f64> = (0..g.rows * g.cols)
        .map(|i| {
            let (r, c) = ((i / g.cols) as f64, (i % g.cols) as f64);
            100.0 + 60.0 * (-((r - 50.0).powi(2) + (c - 40.0).powi(2)) / 200.0).exp()
        })
        .collect();
    let found = relief::extremes(&z, &g, 200.0, 10.0);
    let peaks: Vec<_> = found.iter().filter(|f| f.4 == Extreme::Peak).collect();
    assert_eq!(peaks.len(), 1);
    assert_eq!((peaks[0].0, peaks[0].1), (50, 40));
}
