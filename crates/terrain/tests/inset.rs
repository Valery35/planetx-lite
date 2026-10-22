// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Врезка своего рельефа: рамки тайлов, уровень, край, слияние,
//! глубокие уровни хранилища высот. Перенос `tests/test_inset.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use terrain::inset;
use terrain::{DEEP_LEVEL, HeightStore, HeightTile, Inset, MAX_LEVEL, SIZE};

fn flat_tile(z: u32, x: u32, y: u32, height: f32) -> HeightTile {
    HeightTile::new(z, x, y, vec![height; SIZE * SIZE]).unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-7
}

fn boxed(key: (u32, u32, u32), level: u32) -> Inset {
    let (u0, v0, u1, v1) = inset::shares(inset::tile_bounds(key.0, key.1, key.2));
    Inset { u0, v0, u1, v1, level }
}

#[test]
fn tile_bounds_cover_world() {
    let (west, south, east, north) = inset::tile_bounds(0, 0, 0);
    assert!(close(west, -inset::HALF) && close(north, inset::HALF) && close(east - west, 2.0 * inset::HALF));
    assert!(close(south, -inset::HALF));
    let (u0, v0, u1, v1) = inset::shares(inset::tile_bounds(3, 5, 2));
    assert!(close(u0, 5.0 / 8.0) && close(v0, 2.0 / 8.0) && close(u1, 6.0 / 8.0) && close(v1, 3.0 / 8.0));
}

#[test]
fn level_follows_pixel() {
    // Пиксель уровня 15 в метрах EPSG:3857 - около 4.8 м.
    let pixel15 = 2.0 * inset::HALF / (256.0 * 2f64.powi(15));
    assert_eq!(inset::level_for(pixel15), 15);
    assert_eq!(inset::level_for(pixel15 / 2.0), 16);
    assert_eq!(inset::level_for(1.0), DEEP_LEVEL);
    assert_eq!(inset::level_for(30.0), MAX_LEVEL);
    assert_eq!(inset::level_for(0.01), DEEP_LEVEL);
}

#[test]
fn edge_weight_is_smooth() {
    let w: Vec<f32> = [0.0, 25.0, 50.0, 100.0, 300.0].iter().map(|d| inset::edge_weight(*d, 100.0)).collect();
    assert_eq!(w[0], 0.0);
    assert!((w[2] - 0.5).abs() < 1e-6);
    assert_eq!(w[3], 1.0);
    assert_eq!(w[4], 1.0);
    assert!(w.windows(2).all(|p| p[1] >= p[0]));
}

#[test]
fn merge_replaces_inside_and_keeps_nodata() {
    let base = vec![100.0f32; 16];
    let mut dem = vec![40.0f32; 16];
    dem[0] = f32::NAN;
    let mut weight = vec![1.0f32; 16];
    weight[5] = 0.5;
    weight[10] = f32::NAN;
    let out = inset::merge(&base, &dem, &weight);
    assert_eq!(out[15], 40.0);
    assert_eq!(out[0], 100.0); // без данных - Terrarium
    assert_eq!(out[5], 70.0); // полоса края
    assert_eq!(out[10], 100.0);
}

#[test]
fn band_is_share_of_smaller_side() {
    assert!(close(inset::band_width((0.0, 0.0, 4000.0, 2000.0)), 100.0));
}

#[test]
fn cap_and_wanted_inside_box() {
    let mut store = HeightStore::new();
    store.deep = vec![boxed((16, 40000, 20000), 17)];
    let inside = (19, 40000 * 8 + 3, 20000 * 8 + 3);
    let outside = (19, 1000, 1000);
    assert_eq!(store.cap(inside), 17);
    assert_eq!(store.cap(outside), MAX_LEVEL);
    assert_eq!(store.wanted(inside).0, 17);
    assert_eq!(store.wanted(outside).0, MAX_LEVEL);
}

#[test]
fn deep_tile_is_used() {
    let mut store = HeightStore::new();
    let key = (17, 80000, 40000);
    store.deep = vec![boxed(key, 17)];
    store.add(flat_tile(15, 80000 >> 2, 40000 >> 2, 10.0));
    store.add(flat_tile(key.0, key.1, key.2, 55.0));
    assert_eq!(store.best((19, 80000 * 4 + 1, 40000 * 4 + 1)).unwrap().z, 17);
    let u = (80000.0 + 0.5) / 2f64.powi(17);
    let v = (40000.0 + 0.5) / 2f64.powi(17);
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * v)).sinh().atan().to_degrees();
    let lon = u * 360.0 - 180.0;
    assert!((store.height_at(lat, lon) - 55.0).abs() < 1e-3);
    assert!((store.heights_at(&[(lat, lon)], true)[0] - 55.0).abs() < 1e-3);
}

#[test]
fn resample_from_ancestor() {
    let heights: Vec<f32> = (0..SIZE * SIZE).map(|i| (i % SIZE) as f32).collect();
    let parent = HeightTile::new(15, 100, 200, heights).unwrap();
    let child = inset::resample(&parent, (16, 201, 400));
    assert_eq!(child.len(), SIZE * SIZE);
    // Правая половина предка: столбцы 128-255.
    assert!((child[0] - 127.75).abs() < 1e-3);
    assert!((child[SIZE - 1] - 255.0).abs() < 1e-3);
}

#[test]
fn flat_level_is_deeper_than_any() {
    const { assert!(terrain::FLAT_LEVEL > DEEP_LEVEL) };
}

#[test]
fn overlaps_by_box() {
    let a = (0.0, 0.0, 10.0, 10.0);
    assert!(inset::overlaps(a, (5.0, 5.0, 20.0, 20.0)));
    assert!(!inset::overlaps(a, (10.0, 0.0, 20.0, 10.0)));
}
