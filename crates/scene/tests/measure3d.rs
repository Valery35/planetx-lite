// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! 3D-путь и 3D-многоугольник. Перенос `tests/test_measure3d.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use globe_core::DVec3;
use globe_core::ellipsoid::Ellipsoid;
use scene::measure3d as m3;

/// Треугольники коробки над плоскостью z = 0: крыша и четыре стены.
fn boxed(x0: f64, y0: f64, x1: f64, y1: f64, h: f64) -> Vec<m3::Triangle> {
    let top = [DVec3::new(x0, y0, h), DVec3::new(x1, y0, h), DVec3::new(x1, y1, h), DVec3::new(x0, y1, h)];
    let low = top.map(|p| DVec3::new(p.x, p.y, 0.0));
    let mut tris = vec![[top[0], top[1], top[2]], [top[0], top[2], top[3]]];
    for i in 0..4 {
        let j = (i + 1) % 4;
        tris.push([low[i], low[j], top[j]]);
        tris.push([low[i], top[j], top[i]]);
    }
    tris
}

fn close(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() < tolerance, "{a} {b}");
}

#[test]
fn roof_is_hit_from_above() {
    let t = m3::ray_triangles(DVec3::new(5.0, 5.0, 100.0), DVec3::NEG_Z, &boxed(0.0, 0.0, 10.0, 10.0, 20.0));
    close(t.unwrap(), 80.0, 1e-9);
}

#[test]
fn wall_is_hit_from_the_side() {
    let t = m3::ray_triangles(DVec3::new(-30.0, 5.0, 10.0), DVec3::X, &boxed(0.0, 0.0, 10.0, 10.0, 20.0));
    close(t.unwrap(), 30.0, 1e-9);
}

#[test]
fn nearest_of_two_buildings() {
    let mut tris = boxed(0.0, 0.0, 10.0, 10.0, 20.0);
    tris.extend(boxed(20.0, 0.0, 30.0, 10.0, 50.0));
    close(m3::ray_triangles(DVec3::new(-10.0, 5.0, 15.0), DVec3::X, &tris).unwrap(), 10.0, 1e-9);
}

#[test]
fn miss_and_behind() {
    let tris = boxed(0.0, 0.0, 10.0, 10.0, 20.0);
    assert!(m3::ray_triangles(DVec3::new(50.0, 50.0, 100.0), DVec3::NEG_Z, &tris).is_none());
    assert!(m3::ray_triangles(DVec3::new(5.0, 5.0, 100.0), DVec3::Z, &tris).is_none());
    assert!(m3::ray_triangles(DVec3::new(5.0, 5.0, 100.0), DVec3::NEG_Z, &[]).is_none());
}

#[test]
fn sphere_filter() {
    assert!(m3::ray_sphere(DVec3::new(0.0, 0.0, 100.0), DVec3::NEG_Z, DVec3::ZERO, 5.0));
    assert!(!m3::ray_sphere(DVec3::new(0.0, 0.0, 100.0), DVec3::Z, DVec3::ZERO, 5.0));
    assert!(!m3::ray_sphere(DVec3::new(20.0, 0.0, 100.0), DVec3::NEG_Z, DVec3::ZERO, 5.0));
}

/// Точка ECEF в метрах от (lat, lon) по осям восток, север, вверх.
fn local(east: f64, north: f64, up: f64) -> DVec3 {
    let (lat, lon) = (58.0f64, 56.0f64);
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    let e = DVec3::new(-lo.sin(), lo.cos(), 0.0);
    let n = DVec3::new(-la.sin() * lo.cos(), -la.sin() * lo.sin(), la.cos());
    let u = DVec3::new(la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin());
    Ellipsoid::wgs84().geodetic_to_ecef(lat, lon, 0.0) + e * east + n * north + u * up
}

#[test]
fn path_is_straight_segments() {
    close(m3::path_length(&[local(0.0, 0.0, 0.0), local(30.0, 0.0, 40.0), local(30.0, 0.0, 0.0)]), 90.0, 1e-6);
}

#[test]
fn flat_roof() {
    let p = [(0.0, 0.0), (20.0, 0.0), (20.0, 10.0), (0.0, 10.0)].map(|(x, y)| local(x, y, 25.0));
    let (perimeter, area, tilt) = m3::polygon(&Ellipsoid::wgs84(), &p);
    close(perimeter, 60.0, 1e-6);
    close(area, 200.0, 1e-4);
    assert!(tilt < 0.01);
}

#[test]
fn slope_of_30_degrees() {
    // Скат 10 × 20 м под 30°: проекция на карту 10 × 17.32 м, настоящая
    // площадь 200 м².
    let rise = 20.0 * 30.0f64.to_radians().sin();
    let run = 20.0 * 30.0f64.to_radians().cos();
    let p = [local(0.0, 0.0, 0.0), local(10.0, 0.0, 0.0), local(10.0, run, rise), local(0.0, run, rise)];
    let (_, area, tilt) = m3::polygon(&Ellipsoid::wgs84(), &p);
    close(area, 200.0, 1e-3);
    close(tilt, 30.0, 1e-3);
}

#[test]
fn wall() {
    let p = [local(0.0, 0.0, 0.0), local(12.0, 0.0, 0.0), local(12.0, 0.0, 9.0), local(0.0, 0.0, 9.0)];
    let (_, area, tilt) = m3::polygon(&Ellipsoid::wgs84(), &p);
    close(area, 108.0, 1e-3);
    close(tilt, 90.0, 1e-3);
}

#[test]
fn two_points() {
    let (perimeter, area, tilt) = m3::polygon(&Ellipsoid::wgs84(), &[local(0.0, 0.0, 0.0), local(3.0, 4.0, 0.0)]);
    close(perimeter, 10.0, 1e-6);
    assert_eq!((area, tilt), (0.0, 0.0));
}
