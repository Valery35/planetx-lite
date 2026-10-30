// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Тени на момент по рельефу. Перенос `tests/test_shadows.py` модуля.

use std::sync::Arc;

use data::shadows::{self, Horizon, OUTSIDE, SHADE, SUN, Shadows};
use data::viewshed::distance_bearing;

/// Пермь.
const LAT: f64 = 58.0;
const LON: f64 = 56.25;
const R: f64 = 6378137.0;

/// Секунды UTC по григорианскому календарю.
fn timegm(year: i64, month: i64, day: i64, hour: i64, minute: i64) -> f64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    ((era * 146097 + doe - 719468) * 86400 + hour * 3600 + minute * 60) as f64
}

/// Местный полдень 21 декабря - около 8:15 UTC, солнце на юге не выше 9°.
fn winter_noon() -> f64 {
    timegm(2026, 12, 21, 8, 15)
}

fn summer_noon() -> f64 {
    timegm(2026, 6, 21, 8, 15)
}

fn night() -> f64 {
    timegm(2026, 6, 21, 22, 0)
}

fn flat(points: &[(f64, f64)]) -> Vec<f64> {
    vec![0.0; points.len()]
}

/// Стена 200 м шириной 300 м в 400-700 м к югу от точки.
fn wall_south(points: &[(f64, f64)]) -> Vec<f64> {
    points
        .iter()
        .map(|&(lat, lon)| {
            let (dist, az) = distance_bearing(LAT, LON, lat, lon, R);
            if az > 135.0 && az < 225.0 && dist > 400.0 && dist < 700.0 { 200.0 } else { 0.0 }
        })
        .collect()
}

/// Холм 300 м радиусом 1 км в 1.5 км к югу от точки.
fn hill(points: &[(f64, f64)]) -> Vec<f64> {
    points
        .iter()
        .map(|&(lat, lon)| {
            let (dist, az) = distance_bearing(LAT, LON, lat, lon, R);
            let north = dist * az.to_radians().cos();
            let east = dist * az.to_radians().sin();
            let r = (north + 1500.0).hypot(east);
            if r < 1000.0 { 300.0 * (1.0 - r / 1000.0) } else { 0.0 }
        })
        .collect()
}

fn horizon(heights: data::viewshed::Heights, ring: Option<Vec<(f64, f64)>>, radius_m: f64) -> Option<Arc<Horizon>> {
    shadows::compute(LAT, LON, radius_m, heights, R, ring)
}

fn centre_lit(result: &Shadows) -> bool {
    let n = result.horizon.n;
    result.lit[(n / 2) * n + n / 2]
}

#[test]
fn flat_ground_is_lit_by_day() {
    let h = horizon(&flat, None, 1000.0);
    assert!(h.is_some());
    let Some(h) = h else { return };
    let result = h.at(summer_noon());
    assert!(result.lit.iter().all(|l| *l));
    assert!(result.elevation > 50.0, "{}", result.elevation);
}

#[test]
fn sun_below_horizon_shades_everything() {
    let Some(h) = horizon(&flat, None, 1000.0) else { return };
    let result = h.at(night());
    assert!(result.elevation < 0.0);
    assert!(!result.lit.iter().any(|l| *l));
}

#[test]
fn wall_shades_north_of_it_in_winter_noon() {
    let h = horizon(&wall_south, None, 1000.0);
    assert!(h.is_some());
    let Some(h) = h else { return };
    assert!(!centre_lit(&h.at(winter_noon())));
    // Летом солнце выше 50°, стена 200 м в 400 м тени не даёт.
    assert!(centre_lit(&h.at(summer_noon())));
}

#[test]
fn hill_shades_point_but_not_far_north() {
    let h = horizon(&hill, None, 1000.0);
    assert!(h.is_some());
    let Some(h) = h else { return };
    let winter = h.at(winter_noon());
    assert!(!centre_lit(&winter));
    // 900 м к северу от точки угол на холм меньше высоты солнца.
    let n = h.n;
    let north = n / 2 + (900.0 / h.cell) as usize;
    assert!(winter.lit[north * n + n / 2]);
}

#[test]
fn raster_and_tile() {
    let ring = vec![(LAT - 0.002, LON - 0.002), (LAT - 0.002, LON + 0.002), (LAT + 0.002, LON + 0.002), (LAT + 0.002, LON - 0.002)];
    let h = horizon(&wall_south, Some(ring), 500.0);
    assert!(h.is_some());
    let Some(h) = h else { return };
    let result = h.at(winter_noon());
    let (grid, values) = result.raster();
    assert!(grid.north > LAT && grid.west < LON);
    assert!(values.iter().all(|v| [SUN, SHADE, OUTSIDE].contains(v)));
    assert!(values.contains(&OUTSIDE) && values.contains(&SHADE));
    let points: Vec<(f64, f64)> = (0..8).flat_map(|i| (0..8).map(move |j| (LAT - 0.01 + 0.02 * f64::from(i) / 7.0, LON - 0.01 + 0.02 * f64::from(j) / 7.0))).collect();
    let rgba = shadows::tile_rgba(&result, &points, R);
    assert_eq!(rgba.len(), 64);
    assert_eq!(rgba[0][3], 0);
    let share = result.share();
    assert!((0.0..=1.0).contains(&share));
}

#[test]
fn azimuths_between_bins_interpolate() {
    // 4 азимута, 4 узла: на севере горизонт 45°, солнце на 30°.
    let mut hor = vec![0.0f32; 4 * 4];
    hor[..4].fill(1.0);
    let normals = vec![[0.0, 0.0, 1.0]; 4];
    let t = 30f64.to_radians().tan();
    assert!(!shadows::lit_map(&hor, &normals, [0.0, 1.0, t]).iter().any(|l| *l));
    assert!(shadows::lit_map(&hor, &normals, [1.0, 0.0, t]).iter().all(|l| *l));
}

#[test]
fn hostile_input_gives_nothing() {
    assert!(horizon(&flat, None, f64::NAN).is_none());
    assert!(shadows::lit_map(&[], &[], [0.0, 0.0, 1.0]).is_empty());
    assert_eq!(shadows::lit_map(&[1.0], &[[0.0, 0.0, 1.0]; 2], [0.0, 0.0, 1.0]), vec![false, false]);
}
