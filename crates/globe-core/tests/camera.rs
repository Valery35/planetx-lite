// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Камера и матрицы с отсчётом от глаза.
//!
//! Перенос `planetx/tests/test_camera.py` модуля. Проверки типа
//! и раскладки массива NumPy в `test_batch_equals_one_by_one` не нужны,
//! в Rust их задаёт тип `Mat4`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{PERM, Rng};
use globe_core::camera::{self as cm, Camera};
use globe_core::ellipsoid::Ellipsoid;
use globe_core::tiling::{self as tl, TileMesh};
use globe_core::{DMat3, DMat4, DVec2, DVec3, Mat4, Vec3};

fn wgs84() -> Ellipsoid {
    Ellipsoid::wgs84()
}

fn look_at(lat: f64, lon: f64, distance: f64, heading: f64, tilt: f64) -> Camera {
    Camera::look_at(wgs84(), lat, lon, distance, heading, tilt, 0.0)
}

/// Камера в 100 м над Пермью, взгляд под 60° от отвеса.
fn street_camera() -> Camera {
    look_at(PERM.0, PERM.1, 200.0, 30.0, 60.0)
}

/// Точка перед камерой и не дальше половины окна за его краем.
fn visible(cam: &Camera, px: DVec2, in_front: bool) -> bool {
    let (w, h) = (f64::from(cam.width), f64::from(cam.height));
    in_front && px.x > -0.5 * w && px.x < 1.5 * w && px.y > -0.5 * h && px.y < 1.5 * h
}

/// Путь вершины в шейдере, вся арифметика в float32.
fn shader_pixel(cam: &Camera, mvp32: &Mat4, point32: Vec3) -> DVec2 {
    let clip = *mvp32 * point32.extend(1.0);
    // В контрольном прогоне с мировыми координатами в float32 w местами
    // обращается в ноль. Такие точки дают бесконечность и в сравнение
    // попадают как ошибка.
    let ndc = clip.truncate().truncate() / clip.w;
    cam.ndc_to_pixels(ndc.as_dvec2())
}

/// Тайлы уровней 16-18 вокруг точки взгляда.
fn street_tiles() -> Vec<TileMesh> {
    let e = wgs84();
    let mut out = Vec::new();
    for z in [16, 17, 18] {
        let (x0, y0) = tl::lonlat_to_tile(PERM.0, PERM.1, z);
        for dx in [-1i64, 0, 1] {
            for dy in [-1i64, 0, 1] {
                let x = (i64::from(x0) + dx) as u32;
                let y = (i64::from(y0) + dy) as u32;
                out.push(tl::tile_mesh(&e, z, x, y));
            }
        }
    }
    out
}

fn tile_zxy(mesh: &TileMesh) -> (u32, u32, u32) {
    match mesh.id {
        tl::MeshId::Tile { z, x, y } => (z, x, y),
        tl::MeshId::PolarCap { .. } => panic!("ждали тайл"),
    }
}

/// Наибольшее расхождение пикселей шейдера с точным расчётом и количество
/// сравнённых точек. scale сжимает точный тайл к центру Земли.
fn tile_error(cam: &Camera, mesh: &TileMesh, mvp: &Mat4, points: &[Vec3], scale: f64) -> (f64, usize) {
    let (z, x, y) = tile_zxy(mesh);
    let exact = tl::grid_ecef(&cam.ellipsoid, z, x, y);
    let mut worst: f64 = 0.0;
    let mut count = 0;
    for (p, p32) in exact.iter().zip(points) {
        let (reference, in_front) = cam.project(*p * scale);
        if !visible(cam, reference, in_front) {
            continue;
        }
        count += 1;
        let got = shader_pixel(cam, mvp, *p32);
        let diff = (got - reference).length();
        // NaN и бесконечность - тоже ошибка.
        worst = if diff.is_finite() { worst.max(diff) } else { f64::INFINITY };
    }
    (worst, count)
}

fn grid_positions(mesh: &TileMesh) -> &[Vec3] {
    let s = mesh.segments as usize + 1;
    &mesh.positions[..s * s]
}

// Отсчёт от глаза.

fn street_errors(world_in_float32: bool) -> (f64, usize) {
    let cam = street_camera();
    let mut worst: f64 = 0.0;
    let mut count = 0;
    for mesh in street_tiles() {
        let (z, x, y) = tile_zxy(&mesh);
        let (mvp, points) = if world_in_float32 {
            let view = cam.rotation.transpose();
            let model_view = DMat4::from_cols(
                view.x_axis.extend(0.0),
                view.y_axis.extend(0.0),
                view.z_axis.extend(0.0),
                (-(view * cam.eye)).extend(1.0),
            );
            let mvp = (cam.projection() * model_view).as_mat4();
            let points: Vec<Vec3> = tl::grid_ecef(&cam.ellipsoid, z, x, y).iter().map(|p| p.as_vec3()).collect();
            (mvp, points)
        } else {
            (cam.tile_mvp(mesh.center, None, 1.0), grid_positions(&mesh).to_vec())
        };
        let (w, c) = tile_error(&cam, &mesh, &mvp, &points, 1.0);
        worst = worst.max(w);
        count += c;
    }
    (worst, count)
}

#[test]
fn street_level_error_under_hundredth_of_pixel() {
    let (worst, count) = street_errors(false);
    assert!(count > 500, "{count}");
    assert!(worst < 0.01, "{worst}");
}

#[test]
fn world_coordinates_in_float32_fail() {
    // Сторож приёма. Без отсчёта от глаза ошибка больше пикселя.
    let (worst, _) = street_errors(true);
    assert!(worst > 1.0, "{worst}");
}

#[test]
fn space_view_error_under_hundredth_of_pixel() {
    let e = wgs84();
    let cam = look_at(PERM.0, PERM.1, 2.0e7, 0.0, 0.0);
    let mut worst: f64 = 0.0;
    let mut count = 0;
    for x in 0..4 {
        for y in 0..4 {
            let mesh = tl::tile_mesh(&e, 2, x, y);
            let mvp = cam.tile_mvp(mesh.center, None, 1.0);
            let (w, c) = tile_error(&cam, &mesh, &mvp, grid_positions(&mesh), 1.0);
            worst = worst.max(w);
            count += c;
        }
    }
    assert!(count > 0);
    assert!(worst < 0.01, "{worst}");
}

// Набор матриц за один вызов.

#[test]
fn batch_equals_one_by_one() {
    let e = wgs84();
    let cam = street_camera();
    let meshes = street_tiles();
    let centers: Vec<DVec3> = meshes.iter().map(|m| m.center).collect();
    let mut scales = vec![1.0; meshes.len() - 3];
    scales.extend([1.0 - 1000.0 / e.a; 3]);
    let batch = cam.tiles_mvp(&centers, Some(&scales), None);
    assert_eq!(batch.len(), meshes.len());
    for (i, (c, s)) in centers.iter().zip(&scales).enumerate() {
        let single = cam.tile_mvp(*c, None, *s);
        for (a, b) in batch[i].to_cols_array().iter().zip(single.to_cols_array()) {
            assert!((a - b).abs() <= 1e-6 + 1e-6 * b.abs(), "{i}");
        }
    }
}

#[test]
fn batch_keeps_street_precision() {
    let cam = street_camera();
    let meshes = street_tiles();
    let centers: Vec<DVec3> = meshes.iter().map(|m| m.center).collect();
    let batch = cam.tiles_mvp(&centers, None, None);
    assert_eq!(batch.len(), meshes.len());
    let mut worst: f64 = 0.0;
    for (mvp, mesh) in batch.iter().zip(&meshes) {
        let (w, _) = tile_error(&cam, mesh, mvp, grid_positions(mesh), 1.0);
        worst = worst.max(w);
    }
    assert!(worst < 0.01, "{worst}");
}

// Сжатый тайл.

#[test]
fn scale_shrinks_towards_the_centre() {
    // Подстилка: тайл, сжатый к центру Земли, ложится туда же,
    // куда лёг бы тайл из точек, умноженных на scale.
    let e = wgs84();
    let cam = look_at(PERM.0, PERM.1, 2.0e6, 0.0, 30.0);
    let scale = 1.0 - 1000.0 / e.a;
    let mesh = tl::tile_mesh(&e, 2, 2, 1);
    let mvp = cam.tile_mvp(mesh.center, None, scale);
    let (worst, count) = tile_error(&cam, &mesh, &mvp, grid_positions(&mesh), scale);
    assert!(count > 20, "{count}");
    assert!(worst < 0.01, "{worst}");
}

// Ориентация.

#[test]
fn rotation_is_proper() {
    for heading in [0.0, 45.0, 200.0] {
        for tilt in [0.0, 30.0, 89.0] {
            let r = cm::orientation(PERM.0, PERM.1, heading, tilt);
            let d = r.transpose() * r - DMat3::IDENTITY;
            assert!(d.to_cols_array().iter().all(|v| v.abs() < 1e-12));
            assert!((r.determinant() - 1.0).abs() < 5e-13);
        }
    }
}

#[test]
fn target_is_in_the_centre() {
    let e = wgs84();
    for (heading, tilt) in [(0.0, 0.0), (30.0, 60.0), (250.0, 80.0)] {
        let cam = look_at(PERM.0, PERM.1, 5000.0, heading, tilt);
        let (px, front) = cam.project(e.geodetic_to_ecef(PERM.0, PERM.1, 0.0));
        assert!(front);
        assert!((px.x - f64::from(cam.width) / 2.0).abs() < 1e-6);
        assert!((px.y - f64::from(cam.height) / 2.0).abs() < 1e-6);
    }
}

#[test]
fn heading_turns_north() {
    let e = wgs84();
    let north = e.geodetic_to_ecef(PERM.0 + 0.001, PERM.1, 0.0);
    let cam = look_at(PERM.0, PERM.1, 5000.0, 0.0, 0.0);
    let (px, _) = cam.project(north);
    assert!(px.y < f64::from(cam.height) / 2.0 - 10.0);
    assert!((px.x - f64::from(cam.width) / 2.0).abs() < 0.5);
    // Верх экрана смотрит на восток, север уходит влево.
    let cam = look_at(PERM.0, PERM.1, 5000.0, 90.0, 0.0);
    let (px, _) = cam.project(north);
    assert!(px.x < f64::from(cam.width) / 2.0 - 10.0);
}

#[test]
fn tilt() {
    let e = wgs84();
    let up = e.surface_normal(PERM.0, PERM.1);
    let down = look_at(PERM.0, PERM.1, 1000.0, 0.0, 0.0);
    assert!((down.forward().dot(up) + 1.0).abs() < 5e-13);
    let level = look_at(PERM.0, PERM.1, 1000.0, 0.0, 90.0);
    assert!(level.forward().dot(up).abs() < 5e-13);
}

#[test]
fn distance_and_altitude() {
    assert!((street_camera().altitude() - 100.0).abs() < 0.01);
}

// Луч через точку окна.

#[test]
fn centre_ray_is_forward() {
    let cam = street_camera();
    let (_, d) = cam.ray(f64::from(cam.width) / 2.0, f64::from(cam.height) / 2.0);
    assert!((d - cam.forward()).abs().max_element() < 1e-12);
}

#[test]
fn ray_and_project_agree() {
    let cam = street_camera();
    let mut rng = Rng(7);
    for _ in 0..50 {
        let px = rng.uniform(0.0, f64::from(cam.width));
        let py = rng.uniform(0.0, f64::from(cam.height));
        let (origin, d) = cam.ray(px, py);
        let (back, front) = cam.project(origin + d * 150.0);
        assert!(front);
        assert!((back.x - px).abs() < 1e-6, "{px} {py}");
        assert!((back.y - py).abs() < 1e-6, "{px} {py}");
    }
}

#[test]
fn ray_hits_the_target() {
    let e = wgs84();
    let cam = street_camera();
    let (origin, d) = cam.ray(f64::from(cam.width) / 2.0, f64::from(cam.height) / 2.0);
    let t = e.ray_intersect(origin, d, 0.0).unwrap();
    let hit = origin + d * t;
    assert!((hit - e.geodetic_to_ecef(PERM.0, PERM.1, 0.0)).length() < 1e-3);
}

// Маска диска Земли.

fn mask_at(cam: &Camera, mask: &[bool], row: usize, col: usize) -> bool {
    mask[row * cam.width as usize + col]
}

#[test]
fn disk_area_from_space() {
    // Видимый радиус диска на расстоянии d - arcsin(R / d).
    let e = wgs84();
    let cam = look_at(0.0, 0.0, 2.0e7, 0.0, 0.0).with_viewport(800, 600);
    let mask = cam.earth_mask(0);
    assert_eq!(mask.len(), 800 * 600);
    let d = 2.0e7 + e.a;
    let half = (e.a / d).asin();
    let focal = f64::from(cam.height) / 2.0 / (cam.fov_y.to_radians() / 2.0).tan();
    let radius = focal * half.tan();
    let area = mask.iter().filter(|m| **m).count() as f64;
    assert!((area / (std::f64::consts::PI * radius * radius) - 1.0).abs() < 0.01);
    assert!(mask_at(&cam, &mask, 300, 400));
    assert!(!mask_at(&cam, &mask, 0, 0));
}

#[test]
fn erode_removes_the_rim() {
    let cam = look_at(0.0, 0.0, 2.0e7, 0.0, 0.0).with_viewport(400, 300);
    let full = cam.earth_mask(0);
    let inner = cam.earth_mask(2);
    assert_eq!(full.len(), inner.len());
    assert!(inner.iter().zip(&full).all(|(i, f)| !*i || *f));
    let count = |m: &[bool]| m.iter().filter(|v| **v).count();
    assert!(count(&inner) < count(&full));
}

#[test]
fn street_view_is_all_earth() {
    let cam = look_at(PERM.0, PERM.1, 300.0, 0.0, 0.0).with_viewport(160, 90);
    let mask = cam.earth_mask(0);
    assert_eq!(mask.len(), 160 * 90);
    assert!(mask.iter().all(|m| *m));
}

// Плоскости отсечения.

#[test]
fn ratio_and_coverage() {
    let e = wgs84();
    for h in [50.0, 100.0, 300.0, 1e3, 1e4, 1e5, 1e6, 2e7, 4e7] {
        let (near, far) = cm::clip_range(&e, h, None, None);
        assert!(far / near <= cm::MAX_DEPTH_RATIO * 1.000001, "{h}");
        // Ближе высоты камеры поверхности нет, она не срезается.
        assert!(near <= h, "{h}");
        let horizon = (h * (2.0 * e.a + h)).sqrt();
        assert!(far > horizon, "{h}");
    }
}

#[test]
fn far_plane_follows_true_altitude() {
    // Глаз на 5 км в 50 м над склоном. Дальняя плоскость считается
    // от высоты над эллипсоидом, иначе далёкие хребты срезаются.
    let e = wgs84();
    let (near, far) = cm::clip_range(&e, 5000.0, Some(50.0), None);
    assert!(far > (5000.0 * 2.0 * e.a).sqrt());
    assert!(near <= cm::NEAR_SHARE * 50.0);
}
