// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Видимость из точки по рельефу. Перенос `tests/test_viewshed.py`
//! модуля.

use data::viewshed::{self as vs, REFRACTION};

const LAT: f64 = 45.0;
const LON: f64 = 10.0;
const R: f64 = 6378137.0;

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

fn flat(points: &[(f64, f64)]) -> Vec<f64> {
    vec![0.0; points.len()]
}

/// Стена высотой 100 м в 1-2 км к востоку от точки.
fn wall(points: &[(f64, f64)]) -> Vec<f64> {
    points
        .iter()
        .map(|&(lat, lon)| {
            let (dist, az) = vs::distance_bearing(LAT, LON, lat, lon, R);
            if az > 45.0 && az < 135.0 && dist > 1000.0 && dist < 2000.0 { 100.0 } else { 0.0 }
        })
        .collect()
}

fn compute(radius_m: f64, cell: f64, heights: vs::Heights, observer: f64) -> Option<vs::Viewshed> {
    let params = vs::Params { observer, target: 0.0, refraction: REFRACTION, radius: R };
    vs::compute(LAT, LON, radius_m, cell, heights, &params)
}

fn along(bearing: f64, distances: &[f64]) -> Vec<(f64, f64)> {
    vs::destinations(LAT, LON, &[bearing], distances, R)
}

#[test]
fn flat_horizon() {
    // На ровной Земле глаз на 2 м видит землю до горизонта sqrt(2·R'·h),
    // с рефракцией 0.13 около 5.4 км.
    let result = compute(10000.0, 50.0, &flat, 2.0);
    assert!(result.is_some());
    let Some(result) = result else { return };
    let horizon = (2.0 * R / (1.0 - REFRACTION) * 2.0).sqrt();
    let seen = result.ray(0);
    let last = seen.iter().rposition(|v| *v).map_or(0.0, |i| (i + 1) as f64 * result.step);
    close(last, horizon, 2.0 * result.step);
}

#[test]
fn wall_hides_behind() {
    let result = compute(5000.0, 20.0, &wall, 2.0);
    assert!(result.is_some());
    let Some(result) = result else { return };
    let points = along(90.0, &[900.0, 1500.0, 3000.0]);
    let out = result.at(&points, R);
    assert!(out.iter().all(|(_, inside)| *inside));
    // Перед стеной видно. Плоский верх стены выше глаза на 2 м не виден,
    // за стеной тоже. К западу видно до 3 км.
    let seen: Vec<bool> = out.iter().map(|(s, _)| *s).collect();
    assert_eq!(seen, [true, false, false]);
    let west = result.at(&along(270.0, &[3000.0]), R);
    assert!(west.first().is_some_and(|(s, _)| *s));
}

#[test]
fn mast_sees_over_wall() {
    let result = compute(5000.0, 20.0, &wall, 300.0);
    assert!(result.is_some());
    let Some(result) = result else { return };
    // С мачты 300 м видны и верх стены, и земля за ней.
    let out = result.at(&along(90.0, &[1500.0, 3000.0]), R);
    assert!(out.iter().all(|(s, _)| *s));
}

#[test]
fn outside_and_colors() {
    let result = compute(1000.0, 20.0, &flat, 2.0);
    assert!(result.is_some());
    let Some(result) = result else { return };
    let rgba = vs::tile_rgba(&result, &[(LAT, LON + 0.001), (LAT + 1.0, LON)], R);
    assert!(rgba[0][3] > 0);
    assert_eq!(rgba[1][3], 0);
    assert!(result.share() > 0.99);
}

#[test]
fn height_tiles_cover_circle() {
    // 10 км, шаг 20 м: пиксель уровня не крупнее шага, тайлов не больше
    // MAX_TILES, рамка круга внутри тайлов.
    let (level, keys) = vs::height_tiles(LAT, LON, 10000.0, 20.0, R, 15);
    let ground = 2.0 * std::f64::consts::PI * R * LAT.to_radians().cos() / 256.0;
    assert!(keys.len() <= vs::MAX_TILES);
    let (_south, _north, west, east) = vs::circle_box(LAT, LON, 10000.0, R);
    let n = f64::from(1u32 << level);
    let xs: Vec<u32> = keys.iter().map(|k| k.1).collect();
    let (min, max) = (xs.iter().min().copied().unwrap_or(0), xs.iter().max().copied().unwrap_or(0));
    assert!(f64::from(min) / n * 360.0 - 180.0 <= west);
    assert!(f64::from(max + 1) / n * 360.0 - 180.0 >= east);
    // Уровень мог уменьшиться ради количества тайлов, но не ниже того,
    // где пиксель вдвое крупнее шага на каждую ступень.
    assert!(level > 8);
    assert!(ground / f64::from(1u32 << (level + 1)) < 20.0);
    // Предел уровня источника соблюдается.
    let (level, _) = vs::height_tiles(LAT, LON, 1000.0, 2.0, R, 5);
    assert_eq!(level, 5);
}

#[test]
fn fifty_km_is_fast() {
    // Критерий плана: радиус 50 км не дольше 5 с, расчёт без загрузки.
    // Время проверяется только в оптимизированной сборке.
    let started = std::time::Instant::now();
    assert!(compute(50000.0, 100.0, &flat, 2.0).is_some());
    if !cfg!(debug_assertions) {
        assert!(started.elapsed().as_secs_f64() < 5.0);
    }
}

#[test]
fn hostile_input_gives_nothing() {
    assert!(compute(0.0, 20.0, &flat, 2.0).is_none());
    assert!(compute(-5.0, 20.0, &flat, 2.0).is_none());
    assert!(compute(f64::NAN, 20.0, &flat, 2.0).is_none());
    assert!(compute(1000.0, 0.0, &flat, 2.0).is_some());
    let short = |points: &[(f64, f64)]| vec![0.0; points.len() / 2];
    assert!(compute(1000.0, 20.0, &short, 2.0).is_none());
    let (level, keys) = vs::height_tiles(LAT, LON, f64::INFINITY, 0.0, R, 40);
    assert!(level <= 30 && keys.len() <= vs::MAX_TILES.max(4), "{level} {}", keys.len());
}
