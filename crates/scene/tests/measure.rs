// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Линейка: окружность, единицы, профиль, длина по рельефу. Перенос
//! `tests/test_measure.py` модуля. Геодезия сверена с QgsDistanceArea
//! QGIS 4.0.3 (WGS84) 23 октября 2026 года.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use globe_core::ellipsoid::EARTH;
use scene::geodesic;
use scene::measure::{self as ms, Level};

fn close(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance, "{a} != {b}");
}

#[test]
fn north_degree() {
    let (lat, lon) = ms::destination(58.0, 56.0, 0.0, 1.0_f64.to_radians() * EARTH.a);
    close(lat, 59.0, 1e-9);
    close(lon, 56.0, 1e-9);
}

#[test]
fn east_on_equator() {
    let (lat, lon) = ms::destination(0.0, 179.5, 90.0, 1.0_f64.to_radians() * EARTH.a);
    close(lat, 0.0, 1e-9);
    close(lon, -179.5, 1e-9);
}

#[test]
fn convert() {
    assert_eq!(ms::convert(2500.0, "km", &ms::LENGTH_UNITS), 2.5);
    assert_eq!(ms::convert(3.0e4, "ha", &ms::AREA_UNITS), 3.0);
}

#[test]
fn number() {
    assert_eq!(ms::number(12.345), "12.35");
    assert_eq!(ms::number(12345.67), "12 345.7");
    assert_eq!(ms::number(1234567.0), "1 234 567.0");
    assert_eq!(ms::number(0.01234), "0.0123");
    assert_eq!(ms::number(0.5), "0.5");
}

/// Линия на север от Перми длиной length метров по сфере.
fn line(length: f64) -> Vec<(f64, f64)> {
    vec![(58.0, 56.0), ms::destination(58.0, 56.0, 0.0, length)]
}

#[test]
fn sampling_follows_pixel() {
    let points = ms::sample_line(&line(1000.0), ms::PROFILE_POINTS);
    let step = 1000.0 / (points.len() - 1) as f64;
    assert!(step <= ms::pixel_size(58.0, ms::HEIGHT_LEVEL) * 1.01);
    assert!(step > ms::pixel_size(58.0, ms::HEIGHT_LEVEL) * 0.5);
}

#[test]
fn long_line_is_limited() {
    assert!(ms::sample_line(&line(2.0e6), ms::PROFILE_POINTS).len() <= ms::PROFILE_POINTS);
}

#[test]
fn ramp() {
    let latlon = ms::sample_line(&line(1000.0), ms::PROFILE_POINTS);
    let end = latlon.last().unwrap().0;
    let heights: Vec<f64> = latlon.iter().map(|p| (p.0 - 58.0) / (end - 58.0) * 100.0).collect();
    let p = ms::profile(&latlon, &heights, 0.0);
    close(p.flat, 1000.0, 5.0);
    close(p.ground, p.flat.hypot(100.0), 0.5);
    close(p.gain, 100.0, 1e-6);
    close(p.loss, 0.0, 1e-6);
    close(p.mean_slope, 0.1, 0.001);
    close(p.max_slope, 0.1, 0.002);
    assert_eq!((p.low, p.high), (0.0, 100.0));
}

#[test]
fn slope_window_ignores_roughness() {
    // Подъём 10 % с шероховатостью ±1.5 м через точку. Между соседними
    // точками уклон выходит за 50 %, на окне 50 м - нет.
    let latlon = ms::sample_line(&line(1000.0), ms::PROFILE_POINTS);
    let p0 = ms::profile(&latlon, &vec![0.0; latlon.len()], 0.0);
    let heights: Vec<f64> = p0.distance.iter().enumerate().map(|(i, d)| 0.1 * d + if i % 2 == 1 { 1.5 } else { -1.5 }).collect();
    let rough = ms::profile(&latlon, &heights, 0.0);
    let smooth = ms::profile(&latlon, &heights, 50.0);
    assert!(rough.max_slope > 0.5);
    assert!(smooth.max_slope < 0.2);
    close(smooth.mean_slope, 0.1, 0.02);
}

#[test]
fn up_and_down() {
    let latlon = ms::sample_line(&line(2000.0), ms::PROFILE_POINTS);
    let end = latlon.last().unwrap().0;
    let heights: Vec<f64> = latlon.iter().map(|p| 200.0 - 150.0 * (2.0 * (p.0 - 58.0) / (end - 58.0) - 1.0).abs()).collect();
    let p = ms::profile(&latlon, &heights, 0.0);
    close(p.gain, 150.0, 0.5);
    close(p.loss, 150.0, 0.5);
}

#[test]
fn ground_length_flat() {
    let ground = ms::ground_length(&line(5000.0), &|p: &[(f64, f64)]| vec![0.0; p.len()], ms::PROFILE_POINTS);
    // Хорды через пиксель на эллипсоиде против дуги на сфере.
    close(ground, 5000.0, 20.0);
}

#[test]
fn level_and_tiles() {
    assert_eq!(ms::height_level(&line(1000.0), ms::PROFILE_POINTS), 15);
    let long = line(1.0e6);
    let level = ms::height_level(&long, ms::PROFILE_POINTS);
    assert!(level < 15);
    assert!(ms::pixel_size(58.0, level) >= 1.0e6 / ms::PROFILE_POINTS as f64 * 0.99);
    let keys = ms::tiles_along(&ms::sample_line(&long, ms::PROFILE_POINTS), level);
    let mut unique = keys.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(keys.len(), unique.len());
    assert!(keys.iter().all(|k| k.0 == level));
}

#[test]
fn bearing() {
    close(ms::bearing(58.0, 56.0, 59.0, 56.0), 0.0, 1e-9);
    close(ms::bearing(0.0, 0.0, 0.0, 1.0), 90.0, 1e-9);
}

#[test]
fn nearest_vertex() {
    let pixels = [([100.0, 100.0], true), ([200.0, 105.0], true), ([300.0, 100.0], true)];
    assert_eq!(ms::nearest_vertex(&pixels, 198.0, 100.0, 8.0), Some(1));
    assert_eq!(ms::nearest_vertex(&pixels, 150.0, 100.0, 8.0), None);
    // Точка за камерой не хватается.
    let hidden = [pixels[0], ([200.0, 105.0], false), pixels[2]];
    assert_eq!(ms::nearest_vertex(&hidden, 198.0, 100.0, 8.0), None);
}

#[test]
fn midpoints() {
    let mids = ms::segment_midpoints(&[(58.0, 56.0), (58.0, 57.0), (59.0, 57.0)], true);
    assert_eq!(mids.len(), 3);
    close(mids[0].1, 56.5, 1e-6);
    close(mids[1].0, 58.5, 1e-3);
    // Геодезическая, а не геоцентрическая широта.
    let mid = ms::segment_midpoints(&[(45.0, 10.0), (45.0, 10.002)], false)[0];
    close(mid.0, 45.0, 1e-6);
}

#[test]
fn below_sea_is_depth() {
    assert_eq!(ms::surface_level(-10565.4), Level::Depth(10565));
    assert_eq!(ms::surface_level(-0.3), Level::Height(0));
    assert_eq!(ms::surface_level(207.6), Level::Height(208));
}

#[test]
fn geodesic_matches_qgis_distance_area() {
    // Значения QgsDistanceArea, WGS84, scratchpad tools/geod.py.
    let path = [(58.0, 56.0), (58.1, 56.2), (58.05, 56.4)];
    close(geodesic::length(&path), 29283.351351041827, 1e-6);
    close(geodesic::bearing(path[0], path[1]), 46.593106168898004, 1e-9);
    let polygon = [(58.0, 56.0), (58.0, 56.1), (58.05, 56.1), (58.05, 56.0)];
    let (area, perimeter) = geodesic::area_perimeter(&polygon);
    close(area, 32907560.791229248, 0.01);
    close(perimeter, 22956.135346972766, 1e-6);
    // Обход по часовой стрелке - та же площадь, а не остаток Земли.
    let reversed: Vec<(f64, f64)> = polygon.iter().rev().copied().collect();
    close(geodesic::area_perimeter(&reversed).0, 32907560.791229248, 0.01);
    let (lat, lon) = geodesic::project(58.0, 56.0, 30.0, 1000.0);
    close(lat, 58.00777529051154, 1e-11);
    close(lon, 56.008457368574554, 1e-11);
    assert_eq!(geodesic::ring(58.0, 56.0, 1000.0).len(), ms::CIRCLE_POINTS);
}
