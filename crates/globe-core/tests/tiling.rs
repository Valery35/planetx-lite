// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сетка тайлов Web Mercator и сетка вершин тайла.
//!
//! Перенос `planetx/tests/test_tiling.py` модуля без тестов рельефа
//! (`TestTerrainMesh`), они переносятся вместе с `terrain` в фазе 1.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{PERM, Rng};
use globe_core::DVec3;
use globe_core::ellipsoid::Ellipsoid;
use globe_core::tiling::{self as tl, TileMesh};

/// Эталон - QGIS 4.0.3, перевод в EPSG:3857 и деление на тайлы,
/// 26 сентября 2026. Порядок - имя, широта, долгота, z, x, y.
const QGIS_TILES: [(&str, f64, f64, u32, u32, u32); 8] = [
    ("Пермь", 58.0105, 56.2294, 5, 20, 9),
    ("Пермь", 58.0105, 56.2294, 16, 43004, 19735),
    ("Пермь", 58.0105, 56.2294, 19, 344033, 157881),
    ("Гринвич", 0.0, 0.0, 16, 32768, 32768),
    ("Сидней", -33.8568, 151.2153, 16, 60295, 39325),
    ("Сидней", -33.8568, 151.2153, 19, 482367, 314600),
    ("Анкоридж", 61.2176, -149.8997, 16, 5479, 18579),
    ("Анкоридж", 61.2176, -149.8997, 19, 43836, 148639),
];

fn wgs84() -> Ellipsoid {
    Ellipsoid::wgs84()
}

/// Тайлы всех уровней, включая крайние столбцы и строки.
fn sample_tiles(seed: u64) -> Vec<(u32, u32, u32)> {
    let mut rng = Rng(seed);
    let mut out = vec![(0, 0, 0)];
    for z in 1..=tl::MAX_LEVEL {
        let n = 1u32 << z;
        out.push((z, n - 1, 0));
        out.push((z, 0, n - 1));
        for _ in 0..3 {
            out.push((z, rng.below(n), rng.below(n)));
        }
    }
    out
}

/// Вершины в ECEF, так их соберёт шейдер из центра и смещения.
fn world(mesh: &TileMesh) -> Vec<DVec3> {
    mesh.positions.iter().map(|p| mesh.center + p.as_dvec3()).collect()
}

fn side(mesh: &TileMesh) -> usize {
    mesh.segments as usize + 1
}

fn height(e: &Ellipsoid, p: DVec3) -> f64 {
    e.ecef_to_geodetic(p).2
}

// Номера тайлов.

#[test]
fn matches_qgis() {
    for (name, lat, lon, z, x, y) in QGIS_TILES {
        assert_eq!(tl::lonlat_to_tile(lat, lon, z), (x, y), "{name} z{z}");
    }
}

#[test]
fn edges_of_the_world() {
    let n = 1u32 << 10;
    assert_eq!(tl::lonlat_to_tile(90.0, -180.0, 10), (0, 0));
    assert_eq!(tl::lonlat_to_tile(-90.0, 180.0, 10), (n - 1, n - 1));
}

#[test]
fn max_lat() {
    assert!((tl::MAX_LAT - 85.0511287798).abs() < 5e-10);
    // Постоянная записана числом, сверка с формулой.
    assert!((tl::MAX_LAT - std::f64::consts::PI.sinh().atan().to_degrees()).abs() < 1e-12);
    let b = tl::tile_bounds(0, 0, 0);
    assert_eq!((b.west, b.south, b.east, b.north), (-180.0, -tl::MAX_LAT, 180.0, tl::MAX_LAT));
}

#[test]
fn bounds_contain_the_point() {
    for (name, lat, lon, z, x, y) in QGIS_TILES {
        let b = tl::tile_bounds(z, x, y);
        assert!(b.west <= lon && lon < b.east && b.south < lat && lat <= b.north, "{name} z{z}");
    }
}

// Сетка вершин.

#[test]
fn neighbours_share_edges_bit_exact() {
    let e = wgs84();
    for (z, x, y) in sample_tiles(5) {
        let n = 1u32 << z;
        let s = tl::segments(z) as usize + 1;
        let here = tl::grid_ecef(&e, z, x, y);
        let east = tl::grid_ecef(&e, z, (x + 1) % n, y);
        for j in 0..s {
            assert_eq!(here[j * s + s - 1], east[j * s], "восточный край {z} {x} {y}");
        }
        if y + 1 < n {
            let south = tl::grid_ecef(&e, z, x, y + 1);
            assert_eq!(here[(s - 1) * s..], south[..s], "южный край {z} {x} {y}");
        }
    }
}

#[test]
fn float32_offsets_stay_close() {
    // Смещение в float32 теряет не больше 1.2e-7 своей длины.
    let e = wgs84();
    for (z, x, y) in sample_tiles(5) {
        let mesh = tl::tile_mesh(&e, z, x, y);
        let s = side(&mesh);
        let exact = tl::grid_ecef(&e, z, x, y);
        let error = world(&mesh)[..s * s]
            .iter()
            .zip(&exact)
            .map(|(a, b)| (*a - *b).length())
            .fold(0.0, f64::max);
        assert!(error <= 1.2e-7 * mesh.radius, "{z} {x} {y}: {error}");
    }
}

#[test]
fn street_level_error_under_a_millimetre() {
    let e = wgs84();
    let (x, y) = tl::lonlat_to_tile(PERM.0, PERM.1, 16);
    let mesh = tl::tile_mesh(&e, 16, x, y);
    let s = side(&mesh);
    let exact = tl::grid_ecef(&e, 16, x, y);
    for (a, b) in world(&mesh)[..s * s].iter().zip(&exact) {
        assert!((*a - *b).abs().max_element() < 1e-3);
    }
}

#[test]
fn grid_lies_on_the_ellipsoid() {
    let e = wgs84();
    let mesh = tl::tile_mesh(&e, 7, 88, 38);
    let s = side(&mesh);
    assert_eq!(world(&mesh).len(), s * s + 4 * (s - 1));
    for p in &world(&mesh)[..s * s] {
        assert!(height(&e, *p).abs() < 0.01);
    }
}

#[test]
fn skirt_hangs_below_by_its_depth() {
    let e = wgs84();
    for z in [0, 3, 9, 17] {
        let mesh = tl::tile_mesh(&e, z, 0, 0);
        let s = side(&mesh);
        let skirt = &world(&mesh)[s * s..];
        assert!(!skirt.is_empty());
        // Допуск - погрешность смещений float32, как в тесте выше.
        let limit = f64::max(0.01, 1.2e-7 * mesh.radius);
        for p in skirt {
            assert!((height(&e, *p) + tl::skirt_depth(&e, z)).abs() < limit, "{z}");
        }
    }
}

#[test]
fn skirt_covers_coarser_neighbour_sag() {
    // Середина хорды соседа уровнем грубее не глубже юбки.
    let e = wgs84();
    for z in 1..=tl::MAX_LEVEL {
        let parent = tl::grid_ecef(&e, z - 1, 0, (1u32 << (z - 1)) / 2);
        let s = tl::segments(z - 1) as usize;
        let lowest = (0..s)
            .map(|i| height(&e, (parent[i] + parent[i + 1]) / 2.0))
            .fold(f64::INFINITY, f64::min);
        assert!(-lowest < tl::skirt_depth(&e, z), "{z}");
    }
}

fn tile_centre_latlon(e: &Ellipsoid, mesh: &TileMesh, z: u32, x: u32, y: u32) -> (f64, f64) {
    let b = tl::tile_bounds(z, x, y);
    let (lat, _, _) = e.ecef_to_geodetic(mesh.center);
    (lat, (b.west + b.east) / 2.0)
}

#[test]
fn triangles_face_outward() {
    let e = wgs84();
    for (z, x, y) in [(0, 0, 0), (2, 1, 1), (12, 2500, 1300)] {
        let mesh = tl::tile_mesh(&e, z, x, y);
        let pos = world(&mesh);
        let s = side(&mesh);
        let (lat, lon) = tile_centre_latlon(&e, &mesh, z, x, y);
        let up = e.surface_normal(lat, lon);
        let tris: Vec<[usize; 3]> = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| [t[0] as usize, t[1] as usize, t[2] as usize])
            .collect();
        assert!(!tris.is_empty());
        for t in tris {
            let (a, b, c) = (pos[t[0]], pos[t[1]], pos[t[2]]);
            let normal = (b - a).cross(c - a);
            if t.iter().all(|i| *i < s * s) {
                assert!(normal.dot((a + b + c) / 3.0) > 0.0, "{z}");
            } else if z > 0 {
                // Стенка юбки смотрит наружу от середины тайла. У тайла
                // уровня 0 нет наружной стороны.
                let mut out = (a + b + c) / 3.0 - mesh.center;
                out -= up * out.dot(up);
                assert!(normal.dot(out) > 0.0, "{z}");
            }
        }
    }
}

#[test]
fn indices_and_uv() {
    let e = wgs84();
    for z in [0, 1, 2, 10] {
        let mesh = tl::tile_mesh(&e, z, 0, 0);
        let seg = mesh.segments as usize;
        let count = (seg + 1) * (seg + 1) + 4 * seg;
        assert_eq!(mesh.positions.len(), count);
        assert_eq!(mesh.uv.len(), count);
        assert_eq!(mesh.indices.len(), 6 * seg * seg + 24 * seg);
        assert!((*mesh.indices.iter().max().unwrap() as usize) < count);
        assert_eq!(mesh.uv[0].to_array(), [0.0, 0.0]);
        assert_eq!(mesh.uv[(seg + 1) * (seg + 1) - 1].to_array(), [1.0, 1.0]);
    }
}

#[test]
fn uv_follows_the_picture() {
    // Северо-западный угол картинки - первая вершина сетки.
    let e = wgs84();
    let mesh = tl::tile_mesh(&e, 3, 5, 2);
    let b = tl::tile_bounds(3, 5, 2);
    let corner = e.geodetic_to_ecef(b.north, b.west, 0.0);
    assert!((world(&mesh)[0] - corner).length() < 1.0);
}

#[test]
fn segments_by_level() {
    let got: Vec<u32> = (0..5).map(tl::segments).collect();
    assert_eq!(got, [64, 32, 16, 16, 16]);
}

// Шапки у полюсов.

#[test]
fn cap_rim_and_pole() {
    let e = wgs84();
    for north in [true, false] {
        let cap = tl::polar_cap_mesh(&e, north, 256);
        let pos = world(&cap);
        assert_eq!(pos.len(), 257);
        let sign = if north { 1.0 } else { -1.0 };
        let (lat0, _, _) = e.ecef_to_geodetic(pos[0]);
        assert!((lat0 - sign * 90.0).abs() < 5e-7);
        for p in &pos[1..] {
            let (lat, _, h) = e.ecef_to_geodetic(*p);
            assert!((lat - sign * tl::MAX_LAT).abs() < 1e-6);
            assert!(h.abs() < 1.0);
        }
    }
}

#[test]
fn cap_faces_outward() {
    let e = wgs84();
    for north in [true, false] {
        let cap = tl::polar_cap_mesh(&e, north, 256);
        let pos = world(&cap);
        assert_eq!(cap.indices.len(), 3 * 256);
        for t in cap.indices.as_chunks::<3>().0 {
            let (a, b, c) = (pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
            assert!((a + b + c).dot((b - a).cross(c - a)) > 0.0, "{north}");
        }
    }
}

#[test]
fn cap_meets_the_tiles() {
    // Край шапки совпадает с краем сетки тайлов уровня 0.
    let e = wgs84();
    let grid = tl::grid_ecef(&e, 0, 0, 0);
    let seg = tl::segments(0);
    let cap = tl::polar_cap_mesh(&e, true, seg);
    let rim = &world(&cap)[1..];
    assert_eq!(rim.len(), seg as usize);
    for (a, b) in rim.iter().zip(&grid) {
        assert!((*a - *b).abs().max_element() < 2.0);
    }
}

// Полоса счётчика дыр у края диска покрывает просадку хорд.

fn measured_sag(e: &Ellipsoid, z: u32) -> f64 {
    // Середины сторон и центры треугольников всех тайлов уровня,
    // без юбки: у её треугольников вершины ниже поверхности.
    let weights = [
        [1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0],
        [0.5, 0.5, 0.0],
        [0.0, 0.5, 0.5],
        [0.5, 0.0, 0.5],
    ];
    let mut low: f64 = 0.0;
    let n = 1u32 << z;
    for x in 0..n {
        for y in 0..n {
            let m = tl::tile_mesh(e, z, x, y);
            let p = world(&m);
            let hv: Vec<f64> = p.iter().map(|q| height(e, *q)).collect();
            for t in m.indices.as_chunks::<3>().0 {
                let t = [t[0] as usize, t[1] as usize, t[2] as usize];
                if t.iter().any(|i| hv[*i].abs() >= 1.0) {
                    continue;
                }
                for w in weights {
                    let q = p[t[0]] * w[0] + p[t[1]] * w[1] + p[t[2]] * w[2];
                    low = low.min(height(e, q));
                }
            }
        }
    }
    -low
}

#[test]
fn cell_sag_matches_mesh() {
    let e = wgs84();
    for z in [2, 3] {
        let measured = measured_sag(&e, z);
        assert!(measured <= tl::cell_sag(&e, z) * 1.001, "{z}");
        assert!(measured > tl::cell_sag(&e, z) * 0.95, "{z}");
    }
}

#[test]
fn margin_covers_level_2_and_underlay() {
    // Кадр без уровней глубже 2 бывает в новом окне, пока идёт
    // загрузка. В модуле полоса 10 км давала там ложные дыры,
    // 26 сентября 2026.
    let e = wgs84();
    assert!(tl::HOLE_MARGIN > tl::cell_sag(&e, 2) + tl::UNDERLAY_DEPTH);
}
