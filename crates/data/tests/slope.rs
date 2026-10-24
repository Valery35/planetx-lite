// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Уклон и экспозиция по тайлу высот. Перенос `tests/test_slope.py`
//! модуля.

use data::contours::Grid;
use data::slope as sl;

const R: f64 = 6378137.0;

/// Высоты наклонной плоскости: подъём east на метр к востоку и north
/// на метр к северу, по настоящему шагу пикселей строки.
fn plane(z: u32, y: u32, east: f64, north: f64) -> Grid {
    let steps: Vec<f64> = (0..sl::SIZE).map(|r| sl::pixel_size(z, y, r, R, sl::SIZE)).collect();
    let mut rows = vec![0.0; sl::SIZE];
    for r in 1..sl::SIZE {
        // Строки идут к югу, расстояние к северу - со знаком минус.
        rows[r] = rows[r - 1] - steps[r];
    }
    Grid::from_fn(sl::SIZE, sl::SIZE, |r, c| east * c as f64 * steps[r] + north * rows[r])
}

fn inner_mean(g: &Grid) -> f64 {
    let mut sum = 0.0;
    let mut n = 0.0;
    for r in 5..sl::SIZE - 5 {
        for c in 5..sl::SIZE - 5 {
            sum += g.at(r, c);
            n += 1.0;
        }
    }
    sum / n
}

#[test]
fn plane_rising_east() {
    // Подъём 1 м на 10 м к востоку: уклон atan(0.1), склон смотрит вниз -
    // на запад, 270°.
    let (s, a) = sl::slope_aspect(&plane(14, 5000, 0.1, 0.0), 14, 5000, R);
    assert!((inner_mean(&s) - 0.1f64.atan().to_degrees()).abs() < 0.01);
    assert!((inner_mean(&a) - 270.0).abs() < 0.1);
}

#[test]
fn plane_rising_north_faces_south() {
    let (s, a) = sl::slope_aspect(&plane(12, 1300, 0.0, 1.0), 12, 1300, R);
    assert!((inner_mean(&s) - 45.0).abs() < 0.1);
    assert!((inner_mean(&a) - 180.0).abs() < 0.1);
}

#[test]
fn flat_has_no_aspect() {
    let (s, a) = sl::slope_aspect(&Grid::new(256, 256, 120.0), 10, 300, R);
    assert_eq!(s.data.iter().fold(0.0f64, |m, v| m.max(*v)), 0.0);
    assert!(a.data.iter().all(|v| v.is_nan()));
}

#[test]
fn pixel_size_shrinks_to_pole() {
    assert!(sl::pixel_size(3, 0, 0, R, 256) < sl::pixel_size(3, 0, 255, R, 256));
    let middle = sl::pixel_size(0, 0, 128, R, 256);
    assert!((middle - 2.0 * std::f64::consts::PI * R / 256.0).abs() < 200.0);
}

#[test]
fn colors() {
    let rgba = sl::slope_rgba(&Grid { rows: 1, cols: 3, data: vec![1.0, 7.0, 40.0] });
    assert_eq!(rgba.len(), 12);
    // round модуля - к чётному: 178.5 даёт 178.
    assert_eq!(rgba[3], (sl::OPACITY * 255.0).round_ties_even() as u8);
    // Ровное - зелёное, крутое - лиловое, премноженная альфа.
    assert!(rgba[1] > rgba[0]);
    assert!(rgba[10] > rgba[9]);
    let a = sl::aspect_rgba(&Grid { rows: 1, cols: 2, data: vec![f64::NAN, 90.0] });
    assert!(a[3] < a[7]);
}
