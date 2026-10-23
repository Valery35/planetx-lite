// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сетка тайла на рельефе.
//!
//! Перенос `TestTerrainMesh` из `planetx/tests/test_tiling.py` и теста
//! памяти узлов при приходе высот из `planetx/tests/test_lod.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{PERM_LIKE, Rng, flat_tile, height_tile};
use globe_core::camera::Camera;
use globe_core::ellipsoid::Ellipsoid;
use globe_core::lod::{self, Memo, Options, TileKey};
use globe_core::tiling as tl;
use globe_core::{DVec3, Vec3};
use terrain::mesh::{TerrainMesh, polar_cap, terrain_mesh};
use terrain::{self as tr, HeightStore, HeightTile};

fn wgs84() -> Ellipsoid {
    Ellipsoid::wgs84()
}

/// Склон: высота растёт на юго-восток, 0-3000 м на тайл уровня 8.
fn slope() -> HeightTile {
    height_tile(8, 160, 90, |u, v| ((u * 256.0 - 160.0) + (v * 256.0 - 90.0)) * 1500.0)
}

fn world(m: &TerrainMesh) -> Vec<DVec3> {
    m.mesh.positions.iter().map(|p| m.mesh.center + p.as_dvec3()).collect()
}

fn side(m: &TerrainMesh) -> usize {
    m.mesh.segments as usize + 1
}

fn height(p: DVec3) -> f64 {
    wgs84().ecef_to_geodetic(p).2
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

#[test]
fn vertices_sit_at_sampled_heights() {
    let tile = slope();
    let m = terrain_mesh(&wgs84(), 10, 641, 361, Some(&tile), 1.0);
    let s = side(&m);
    let shares = tr::grid_shares(10, 641, 361, m.mesh.segments, 0);
    for (p, (u, v)) in world(&m)[..s * s].iter().zip(&shares) {
        assert!((height(*p) - tr::sample(&tile, *u, *v)).abs() < 0.01);
    }
}

#[test]
fn neighbours_share_edges_with_same_heights() {
    let tile = slope();
    let a = terrain_mesh(&wgs84(), 10, 641, 361, Some(&tile), 1.0);
    let b = terrain_mesh(&wgs84(), 10, 642, 361, Some(&tile), 1.0);
    let s = side(&a);
    let (wa, wb) = (world(&a), world(&b));
    for row in 0..s {
        assert!((wa[row * s + s - 1] - wb[row * s]).abs().max_element() < 0.01);
        assert!((a.shade[row * s + s - 1] - b.shade[row * s]).abs() < 1e-5);
    }
}

#[test]
fn sea_lowers_mesh_to_sea_level() {
    // Проход воды опускает вершину на sea.w вдоль sea.xyz: узлы и юбка
    // ложатся на эллипсоид, высота в sea - высота узла.
    let tile = slope();
    let m = terrain_mesh(&wgs84(), 10, 641, 361, Some(&tile), 2.0);
    assert_eq!(m.sea.len(), m.mesh.positions.len());
    let pos = world(&m);
    for (p, sea) in pos.iter().zip(&m.sea) {
        let flat = *p - sea.truncate().as_dvec3() * f64::from(sea.w);
        assert!(height(flat).abs() < 0.05);
    }
    let s = side(&m);
    for (p, sea) in pos[..s * s].iter().zip(&m.sea) {
        assert!((height(*p) - f64::from(sea.w)).abs() < 0.05);
    }
    let cap = polar_cap(&wgs84(), true, 256, None);
    assert!(cap.sea.iter().all(|s| s.w == 0.0));
    assert_eq!(cap.sea.len(), cap.mesh.positions.len());
}

#[test]
fn flat_ground_is_not_shaded() {
    let flat = height_tile(8, 160, 90, |_, _| 0.0);
    let m = terrain_mesh(&wgs84(), 10, 641, 361, Some(&flat), 1.0);
    assert!(m.shade.iter().all(|s| (s - 1.0).abs() < 1e-4));
    let plain = terrain_mesh(&wgs84(), 10, 641, 361, None, 1.0);
    assert!(!plain.shade.is_empty());
    assert!(plain.shade.iter().all(|s| *s == 1.0));
}

#[test]
fn slopes_face_the_light_or_not() {
    // Высота растёт на юго-восток: склон смотрит на северо-запад, к свету.
    // Обратный склон смотрит на юго-восток, в тень.
    let lit = terrain_mesh(&wgs84(), 10, 641, 361, Some(&slope()), 1.0);
    let back = height_tile(8, 160, 90, |u, v| 3000.0 - ((u * 256.0 - 160.0) + (v * 256.0 - 90.0)) * 1500.0);
    let dark = terrain_mesh(&wgs84(), 10, 641, 361, Some(&back), 1.0);
    // Уклон около 3 %, разница небольшая, но знак обязан быть верным.
    assert!(median(lit.shade.iter().map(|s| f64::from(*s)).collect()) > 1.005);
    assert!(median(dark.shade.iter().map(|s| f64::from(*s)).collect()) < 0.995);
    // Крутой склон, около 40 %, затенён заметно.
    let steep_tile =
        height_tile(8, 160, 90, |u, v| 20.0 * (3000.0 - ((u * 256.0 - 160.0) + (v * 256.0 - 90.0)) * 1500.0));
    let steep = terrain_mesh(&wgs84(), 10, 641, 361, Some(&steep_tile), 1.0);
    assert!(median(steep.shade.iter().map(|s| f64::from(*s)).collect()) < 0.8);
}

#[test]
fn normals_for_the_sun() {
    // Нормали единичные. У гладкого тайла - нормаль эллипсоида, у склона
    // на юго-восток нормаль наклонена к северо-западу, у соседей на общем
    // краю нормаль одна.
    let e = wgs84();
    let plain = terrain_mesh(&e, 10, 641, 361, None, 1.0);
    let s = side(&plain);
    assert!(plain.normals.iter().all(|n| (n.length() - 1.0).abs() < 1e-5));
    let latlon = tl::grid_latlon(10, 641, 361, 0);
    for (n, (lat, lon)) in plain.normals[..s * s].iter().zip(&latlon) {
        let up = e.surface_normal(*lat, *lon).as_vec3();
        assert!((*n - up).abs().max_element() < 1e-6);
    }
    let a = terrain_mesh(&e, 10, 641, 361, Some(&slope()), 1.0);
    let b = terrain_mesh(&e, 10, 642, 361, Some(&slope()), 1.0);
    for row in 0..s {
        assert!((a.normals[row * s + s - 1] - b.normals[row * s]).abs().max_element() < 1e-5);
    }
    let mut to_north = Vec::new();
    let mut to_east = Vec::new();
    for (n, (lat, lon)) in a.normals[..s * s].iter().zip(&latlon) {
        let (sl, cl) = lat.to_radians().sin_cos();
        let (so, co) = lon.to_radians().sin_cos();
        let north = Vec3::new((-sl * co) as f32, (-sl * so) as f32, cl as f32);
        let east = Vec3::new(-so as f32, co as f32, 0.0);
        to_north.push(f64::from(n.dot(north)));
        to_east.push(f64::from(n.dot(east)));
    }
    assert!(median(to_north) > 0.0);
    assert!(median(to_east) < 0.0);
    assert_eq!(a.normals.len(), a.mesh.positions.len());
}

#[test]
fn skirt_goes_below_the_height_spread() {
    let m = terrain_mesh(&wgs84(), 10, 641, 361, Some(&slope()), 1.0);
    let s = side(&m);
    let pos = world(&m);
    let grid: Vec<f64> = pos[..s * s].iter().map(|p| height(*p)).collect();
    let top = grid.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let bottom = grid.iter().copied().fold(f64::INFINITY, f64::min);
    let skirt_top = pos[s * s..].iter().map(|p| height(*p)).fold(f64::NEG_INFINITY, f64::max);
    assert!(skirt_top < top - (top - bottom) + 1.0);
}

#[test]
fn radius_covers_heights() {
    let m = terrain_mesh(&wgs84(), 10, 641, 361, Some(&slope()), 1.0);
    let far = m.mesh.positions.iter().map(|p| f64::from(p.length())).fold(0.0, f64::max);
    assert!(far <= m.mesh.radius + 1e-3);
}

#[test]
fn plain_mesh_matches_tiling() {
    // Без высот сетка та же, что у гладкого эллипсоида.
    let plain = terrain_mesh(&wgs84(), 7, 88, 38, None, 1.0);
    let smooth = tl::tile_mesh(&wgs84(), 7, 88, 38);
    assert_eq!(plain.mesh, smooth);
}

// Память узлов выбора тайлов при приходе высот.

fn view(lat: f64, lon: f64, distance: f64, heading: f64, tilt: f64) -> Camera {
    Camera::look_at(wgs84(), lat, lon, distance, heading, tilt, 0.0).with_viewport(1920, 1080)
}

#[test]
fn memo_partial_reset_when_heights_arrive() {
    // Высоты приходят по одному тайлу, камера стоит. Память забывает
    // только узлы под пришедшим тайлом и даёт тот же выбор, что расчёт
    // с нуля.
    let cam = view(PERM_LIKE.0, PERM_LIKE.1, 2000.0, 30.0, 70.0);
    let mut store = HeightStore::new();
    store.add(flat_tile(0, 0, 0, 0.0));
    let mut memo = Memo::new();
    let mut rng = Rng(2);
    let everything = |_: TileKey| true;
    for step in 0..8u32 {
        let added = store.take_added();
        let values = memo.bind(&cam, lod::THRESHOLD, store.version(), &added);
        let heights = |k: TileKey| store.range_for(k);
        let with_memo =
            lod::select(&cam, &everything, Options { heights: Some(&heights), memo: Some(values), ..Options::default() });
        let plain = lod::select(&cam, &everything, Options { heights: Some(&heights), ..Options::default() });
        assert_eq!(with_memo.draw, plain.draw, "{step}");
        assert_eq!(with_memo.keep, plain.keep, "{step}");
        // Следующий тайл высот - над одним из нарисованных тайлов.
        let key = with_memo.draw[rng.below(with_memo.draw.len() as u32) as usize];
        let hz = tr::height_level(key.0, tr::MAX_LEVEL);
        let shift = key.0 - hz;
        store.add(flat_tile(hz, key.1 >> shift, key.2 >> shift, f64::from(step + 1) * 4.0 * 256.0));
    }
}
