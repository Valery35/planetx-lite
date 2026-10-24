// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Высоты Terrarium.
//!
//! Перенос `planetx/tests/test_terrain.py` модуля. Тайлы в тестах -
//! полные 256 × 256, в модуле были и тайлы из нескольких пикселей.
//! Проверка кэша размахов через тождество объектов Python заменена
//! счётчиком пересчётов размаха.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{Rng, encode, flat_tile, full, ramp_tile, random_tile, tile_of};
use globe_core::ellipsoid::Ellipsoid;
use globe_core::tiling as tl;
use terrain::mesh::terrain_mesh;
use terrain::{self as tr, Encoding, HeightStore, HeightTile, Inset, SIZE};

// Раскодирование.

#[test]
fn known_values() {
    let values = [0.0, 5642.0, 123.5, 8848.0];
    let got = tr::decode(&encode(&values), 4, Some(0.0), Encoding::Terrarium);
    for (g, v) in got.iter().zip(values) {
        assert!((f64::from(*g) - v).abs() <= 1.0 / 256.0, "{g} {v}");
    }
}

#[test]
fn sea_floor_is_sea_level() {
    let got = tr::decode(&encode(&[-4000.0, -28.0, 10.0]), 4, Some(0.0), Encoding::Terrarium);
    assert_eq!(got, [0.0, 0.0, 10.0]);
}

#[test]
fn sea_depths_kept_for_profile() {
    // Море на глобусе ровное, профиль и окно «Объекты» видят дно.
    let mut heights = vec![-3000.0; SIZE * SIZE];
    heights[..SIZE * SIZE / 2].fill(200.0);
    let made = tr::make_tile_depths(0, 0, 0, &encode(&heights), 0.0, Encoding::Terrarium);
    let (tile, depths) = made.expect("tile");
    assert!(tile.heights.iter().all(|h| *h >= 0.0));
    let mut store = HeightStore::new();
    store.add(tile);
    store.add_depths((0, 0, 0), depths.expect("depths"));
    // Юг тайла - море, север - суша.
    let got = store.floor_heights_at(&[(-40.0, 10.0), (40.0, 10.0)]);
    assert!((got[0] + 3000.0).abs() < 1.0, "{got:?}");
    assert!((got[1] - 200.0).abs() < 1.0, "{got:?}");
    assert_eq!(store.heights_at(&[(-40.0, 10.0)], false), [0.0]);
    // Тайл без глубин ниже нуля глубин не несёт.
    let dry = tr::make_tile_depths(0, 0, 0, &encode(&[5.0; SIZE * SIZE]), 0.0, Encoding::Terrarium);
    assert!(dry.expect("dry").1.is_none());
}

#[test]
fn mapbox_encoding() {
    // -10000 + (R·65536 + G·256 + B)·0.1: 1000 м = 110000 = (1, 173, 176).
    let got = tr::decode(&[1, 173, 176], 3, None, Encoding::Mapbox);
    assert!((got[0] - 1000.0).abs() < 1e-3, "{got:?}");
}

#[test]
fn body_keeps_depressions() {
    // Марс и Луна: впадины ниже нуля остаются, равнина Эллада -8 км.
    let values = [-8206.0, -28.0, 21181.0];
    let tile = tile_of(0, 0, 0, &full(&values, 0.0), None);
    assert_eq!(&tile.heights[..3], &[-8206.0, -28.0, 21181.0]);
    assert_eq!(tile.low, -8206.0);
}

#[test]
fn store_depth_follows_lowest_and_scale() {
    // Подстилка опускается на глубину самой низкой высоты: равнина Исиды
    // -3.3 км при масштабе 3 закрывалась подстилкой на 3 км.
    let mut store = HeightStore::new();
    assert_eq!(store.depth(), 0.0);
    store.add(tile_of(0, 0, 0, &full(&[-3300.0, 50.0], 0.0), None));
    store.set_scale(3.0);
    assert_eq!(store.depth(), 9900.0);
    store.set_scale(0.0);
    assert_eq!(store.depth(), 0.0);
    store.clear();
    assert_eq!(store.low(), 0.0);
    // Земля: пол высот 0.
    store.set_scale(1.0);
    store.add(ramp_tile(3, 1, 2));
    assert_eq!(store.depth(), 0.0);
}

#[test]
fn tile_range() {
    let tile = ramp_tile(3, 1, 2);
    assert_eq!((tile.low, tile.high), (0.0, 2550.0));
}

#[test]
fn wrong_size_is_refused() {
    assert!(tr::make_tile(0, 0, 0, &[0, 0, 0, 255], Some(0.0), Encoding::Terrarium).is_none());
    assert!(HeightTile::new(0, 0, 0, vec![0.0; 10]).is_none());
}

#[test]
fn store_limits_wanted_level() {
    // У источника до уровня 5 глубже тайлы не просятся.
    let mut store = HeightStore::new();
    assert_eq!(store.wanted((14, 9000, 5000)).0, 12);
    store.max_level = 5;
    assert_eq!(store.wanted((14, 9000, 5000)), (5, 9000 >> 9, 5000 >> 9));
}

// Выборка.

#[test]
fn pixel_centres() {
    let tile = ramp_tile(0, 0, 0);
    for col in [0.0, 10.0, 200.0] {
        let u = (col + 0.5) / SIZE as f64;
        assert!((tr::sample(&tile, u, 0.5) - col * 10.0).abs() < 5e-7);
    }
}

#[test]
fn between_centres_is_linear() {
    let tile = ramp_tile(0, 0, 0);
    let u = (10.5 + 0.25) / SIZE as f64;
    assert!((tr::sample(&tile, u, 0.3) - 102.5).abs() < 5e-7);
}

#[test]
fn outside_is_clamped() {
    let tile = ramp_tile(4, 5, 6);
    let west = 5.0 / 16.0 - 0.001;
    assert!(tr::sample(&tile, west, 6.5 / 16.0).abs() < 5e-7);
}

#[test]
fn child_tile_nodes_agree_with_parent_samples() {
    let tile = ramp_tile(5, 10, 12);
    let shares = tr::grid_shares(8, 80, 96, 16, 0);
    // Узлы тайла уровня 8 лежат внутри тайла высот уровня 5.
    assert_eq!(shares.len(), 17 * 17);
    let direct: Vec<f64> = shares.iter().map(|(u, v)| tr::sample(&tile, *u, *v)).collect();
    for row in direct.chunks(17) {
        assert!(row.windows(2).all(|w| w[1] >= w[0]));
    }
}

#[test]
fn neighbours_share_edge_heights() {
    let tile = ramp_tile(6, 20, 20);
    let a = tr::grid_shares(9, 160, 161, 16, 0);
    let b = tr::grid_shares(9, 161, 161, 16, 0);
    for row in 0..17 {
        let (u1, v1) = a[row * 17 + 16];
        let (u2, v2) = b[row * 17];
        assert_eq!(tr::sample(&tile, u1, v1), tr::sample(&tile, u2, v2));
    }
}

#[test]
fn grid_shares_match_tiling() {
    let shares = tr::grid_shares(7, 88, 38, 16, 0);
    let latlon = tl::grid_latlon(7, 88, 38, 0);
    for ((u, v), (lat, lon)) in shares.iter().zip(&latlon) {
        assert!((u * 360.0 - 180.0 - lon).abs() < 1e-9);
        assert!((tl::lat_of_row(*v) - lat).abs() < 1e-9);
    }
}

#[test]
fn border_extends_the_grid() {
    let shares = tr::grid_shares(7, 88, 38, 16, 1);
    assert_eq!(shares.len(), 19 * 19);
    assert_eq!(shares[19 + 1], tr::grid_shares(7, 88, 38, 16, 0)[0]);
}

// Хранилище.

#[test]
fn nan_heights_become_zero() {
    // NaN в тайле высот ронял кадр модуля в nearest_terrain, 17 октября
    // 2026 года. Хранилище берёт тайл с нулём вместо NaN.
    let mut heights = vec![120.0f32; SIZE * SIZE];
    for row in 100..110 {
        for col in 100..110 {
            heights[row * SIZE + col] = f32::NAN;
        }
    }
    let mut store = HeightStore::new();
    store.add(HeightTile { z: 0, x: 0, y: 0, heights, low: 0.0, high: 0.0 });
    let kept = &store.tile((0, 0, 0)).unwrap().heights;
    assert!(kept.iter().all(|h| h.is_finite()));
    assert_eq!(kept[105 * SIZE + 105], 0.0);
    assert_eq!(kept[0], 120.0);
    for i in 0..33 {
        for j in 0..33 {
            let lat = -80.0 + 5.0 * f64::from(i);
            let lon = -179.0 + 358.0 / 32.0 * f64::from(j);
            assert!(store.height_at(lat, lon).is_finite());
        }
    }
    let mut empty = HeightStore::new();
    empty.add(HeightTile { z: 1, x: 0, y: 0, heights: vec![f32::NAN; SIZE * SIZE], low: 0.0, high: 0.0 });
    assert!(empty.is_empty());
}

#[test]
fn levels() {
    let got: Vec<u32> = [0, 2, 3, 10, 17, 19].iter().map(|z| tr::height_level(*z, tr::MAX_LEVEL)).collect();
    assert_eq!(got, [0, 0, 1, 8, 15, 15]);
}

#[test]
fn best_walks_up() {
    let mut store = HeightStore::new();
    assert!(store.best((10, 600, 300)).is_none());
    store.add(ramp_tile(0, 0, 0));
    assert_eq!(store.best((10, 600, 300)).unwrap().z, 0);
    store.add(ramp_tile(4, 600 >> 6, 300 >> 6));
    assert_eq!(store.best((10, 600, 300)).unwrap().z, 4);
    assert_eq!(store.wanted((10, 600, 300)), (8, 150, 75));
}

#[test]
fn height_at_uses_finest() {
    let mut store = HeightStore::new();
    store.add(flat_tile(0, 0, 0, 0.0));
    let (u, v) = tr::mercator_share(43.3499, 42.4453);
    let key = (10, (u * 1024.0) as u32, (v * 1024.0) as u32);
    store.add(flat_tile(key.0, key.1, key.2, 5642.0));
    assert!((store.height_at(43.3499, 42.4453) - 5642.0).abs() < 5e-4);
    assert_eq!(store.height_at(0.0, 0.0), 0.0);
}

#[test]
fn range_for_uses_the_covered_block() {
    let mut store = HeightStore::new();
    assert_eq!(store.range_for((12, 1, 1)), (0.0, 0.0));
    store.add(ramp_tile(0, 0, 0));
    // Тайл 0 целиком.
    assert_eq!(store.range_for((0, 0, 0)), (0.0, 2550.0));
    // Правая половина: столбцы 128-255 и ещё один слева на выборку.
    assert_eq!(store.range_for((1, 1, 0)), (1270.0, 2550.0));
    // Мельче пикселя: пиксель 0 и сосед справа.
    assert_eq!(store.range_for((12, 1, 1)), (0.0, 10.0));
}

#[test]
fn range_follows_finer_heights_without_full_reset() {
    let mut store = HeightStore::new();
    store.add(ramp_tile(0, 0, 0));
    let key = (12, 1, 1);
    let far = (12, 3000, 3000);
    assert_eq!(store.range_for(key), (0.0, 10.0));
    let far_range = store.range_for(far);
    // Тайл высот в другом месте не трогает посчитанный размах.
    store.add(flat_tile(8, 180, 180, 700.0));
    let before = store.range_computations();
    assert_eq!(store.range_for(far), far_range);
    assert_eq!(store.range_computations(), before);
    // Тайл высот точнее над key меняет его размах.
    store.add(flat_tile(4, 0, 0, 50.0));
    assert_eq!(store.range_for(key), (50.0, 50.0));
}

#[test]
fn remembered_range_matches_fresh_store() {
    // Запомненные размахи совпадают с размахами нового хранилища с теми
    // же тайлами, в том числе после тайла высот над ключом, смены врезок
    // и предельного уровня.
    let mut rng = Rng(7);
    let mut keys: Vec<(u32, u32, u32)> = (0..400)
        .map(|_| {
            let z = 1 + rng.below(16);
            (z, rng.below(1 << z), rng.below(1 << z))
        })
        .collect();
    keys.extend([(12, 1, 1), (14, 5, 6), (9, 0, 1)]);
    let mut tiles = vec![ramp_tile(0, 0, 0), flat_tile(4, 0, 0, 50.0), random_tile(8, 0, 0, 900.0, 8)];
    let mut store = HeightStore::new();
    let check = |store: &HeightStore, added: &[HeightTile]| {
        let mut fresh = HeightStore::new();
        fresh.deep = store.deep.clone();
        fresh.max_level = store.max_level;
        for t in added {
            fresh.add(t.clone());
        }
        for key in &keys {
            assert_eq!(store.range_for(*key), fresh.range_for(*key), "{key:?}");
        }
    };
    for n in 0..tiles.len() {
        store.add(tiles[n].clone());
        check(&store, &tiles[..=n]);
    }
    store.deep = vec![Inset { u0: 0.0, v0: 0.0, u1: 0.01, v1: 0.01, level: 17 }];
    check(&store, &tiles);
    store.max_level = 3;
    check(&store, &tiles);
    // Больше LOG_CHECK тайлов после подсчёта и тайл над ключом среди
    // немногих новых.
    store.max_level = tr::MAX_LEVEL;
    for n in 0..HeightStore::LOG_CHECK as u32 + 8 {
        let t = flat_tile(9, 100 + n, 200, f64::from(n));
        store.add(t.clone());
        tiles.push(t);
    }
    check(&store, &tiles);
    let t = flat_tile(11, 2, 3, 321.0);
    store.add(t.clone());
    tiles.push(t);
    check(&store, &tiles);
}

#[test]
fn range_is_never_narrower_than_the_mesh() {
    // Узлы сетки тайла лежат внутри размаха, иначе сфера тайла не накрыла
    // бы его вершины.
    let mut store = HeightStore::new();
    let tile = random_tile(3, 4, 2, 4000.0, 5);
    store.add(tile.clone());
    for key in [(5, 17, 9), (7, 70, 38), (9, 280, 150)] {
        let (low, high) = store.range_for(key);
        for (u, v) in tr::grid_shares(key.0, key.1, key.2, 16, 0) {
            let h = tr::sample(&tile, u, v);
            assert!(h >= low - 1e-6 && h <= high + 1e-6, "{key:?}");
        }
    }
}

// Высоты в наборе точек.

#[test]
fn heights_at_matches_single_points() {
    let mut store = HeightStore::new();
    store.add(ramp_tile(0, 0, 0));
    store.add(ramp_tile(3, 5, 2));
    store.add(ramp_tile(6, 42, 18));
    let mut rng = Rng(3);
    let mut points: Vec<(f64, f64)> = (0..300).map(|_| (rng.uniform(-80.0, 80.0), rng.uniform(-179.0, 179.0))).collect();
    points.extend((0..300).map(|_| (rng.uniform(56.5, 58.5), rng.uniform(55.0, 57.5))));
    let many = store.heights_at(&points, true);
    assert_eq!(many.len(), points.len());
    for (h, (lat, lon)) in many.iter().zip(&points) {
        assert!((h - store.height_at(*lat, *lon)).abs() < 1e-6);
    }
}

#[test]
fn heights_at_flat_and_empty() {
    let mut store = HeightStore::new();
    store.add(ramp_tile(0, 0, 0));
    store.set_scale(0.0);
    assert_eq!(store.heights_at(&[(10.0, 20.0)], true), [0.0]);
    assert!(HeightStore::new().heights_at(&[], true).is_empty());
}

// Вертикальный масштаб рельефа и выключенный рельеф.

fn scaled_setup() -> (HeightStore, HeightTile) {
    let mut store = HeightStore::new();
    let tile = random_tile(3, 4, 2, 4000.0, 7);
    store.add(tile.clone());
    (store, tile)
}

const KEY: (u32, u32, u32) = (7, 70, 38);

#[test]
fn heights_and_ranges_follow_scale() {
    let (mut store, _) = scaled_setup();
    let height = store.height_at(50.0, 20.0);
    let (low, high) = store.range_for(KEY);
    assert!(height > 0.0);
    store.set_scale(2.5);
    assert!((store.height_at(50.0, 20.0) - 2.5 * height).abs() < 5e-7);
    assert_eq!(store.range_for(KEY), (2.5 * low, 2.5 * high));
}

#[test]
fn scaled_range_covers_scaled_mesh() {
    // Сфера тайла строится по размаху, вершины сетки с тем же масштабом
    // обязаны лежать внутри него.
    let (mut store, tile) = scaled_setup();
    store.set_scale(4.0);
    let (low, high) = store.range_for(KEY);
    let e = Ellipsoid::wgs84();
    let mesh = terrain_mesh(&e, KEY.0, KEY.1, KEY.2, Some(&tile), 4.0).mesh;
    let side = mesh.segments as usize + 1;
    let latlon = tl::grid_latlon(KEY.0, KEY.1, KEY.2, 0);
    let mut top: f64 = f64::NEG_INFINITY;
    for (p, (lat, lon)) in mesh.positions[..side * side].iter().zip(&latlon) {
        let world = mesh.center + p.as_dvec3();
        let h = (world - e.geodetic_to_ecef(*lat, *lon, 0.0)).dot(e.surface_normal(*lat, *lon));
        assert!(h >= low - 1.0 && h <= high + 1.0, "{h} {low} {high}");
        top = top.max(h);
    }
    assert!(top > 3.0 * f64::from(tile.high) / 4.0);
}

#[test]
fn scale_changes_version() {
    let (mut store, _) = scaled_setup();
    let version = store.version();
    store.set_scale(2.0);
    assert!(store.version() > version);
}

#[test]
fn off_means_smooth_earth() {
    let (mut store, _) = scaled_setup();
    store.set_scale(0.0);
    assert_eq!(store.height_at(50.0, 20.0), 0.0);
    assert_eq!(store.range_for(KEY), (0.0, 0.0));
    let (tile, level) = store.for_mesh(KEY);
    assert!(tile.is_none());
    assert_eq!(level, tr::FLAT_LEVEL as i32);
}

#[test]
fn for_mesh() {
    let (store, tile) = scaled_setup();
    let (none, level) = HeightStore::new().for_mesh(KEY);
    assert!(none.is_none());
    assert_eq!(level, -1);
    let (got, level) = store.for_mesh(KEY);
    assert_eq!(*got.unwrap(), tile);
    assert_eq!(level, 3);
    // Сетка без рельефа не пересобирается ни от каких высот.
    const _: () = assert!(tr::FLAT_LEVEL > tr::MAX_LEVEL);
}

#[test]
fn version_and_added() {
    let mut store = HeightStore::new();
    let v0 = store.version();
    store.add(ramp_tile(0, 0, 0));
    store.add(ramp_tile(1, 1, 0));
    assert_eq!(store.version(), v0 + 2);
    assert_eq!(store.take_added(), [(0, 0, 0), (1, 1, 0)]);
    assert!(store.take_added().is_empty());
    assert_eq!(store.len(), 2);
}
