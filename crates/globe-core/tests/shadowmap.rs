// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Матрицы карты теней кадра, перенос `tests/test_shadowmap.py` модуля.

#![allow(clippy::unwrap_used)]

use globe_core::ellipsoid::Ellipsoid;
use globe_core::shadowmap as sm;
use globe_core::{DVec3, DVec4};

const LAT: f64 = 43.35;
const LON: f64 = 42.45;

/// Центр, восток, север, верх и солнце на высоте 20° с севера.
fn place() -> (DVec3, DVec3, DVec3, DVec3, DVec3) {
    let center = Ellipsoid::wgs84().geodetic_to_ecef(LAT, LON, 2000.0);
    let (la, lo) = (LAT.to_radians(), LON.to_radians());
    let east = DVec3::new(-lo.sin(), lo.cos(), 0.0);
    let north = DVec3::new(-la.sin() * lo.cos(), -la.sin() * lo.sin(), la.cos());
    let up = DVec3::new(la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin());
    let sun = north * 20f64.to_radians().cos() + up * 20f64.to_radians().sin();
    (center, east, north, up, sun)
}

#[test]
fn radius_steps_and_limits() {
    assert_eq!(sm::radius_for(sm::FROM * 2.0), None);
    assert_eq!(sm::radius_for(10.0), sm::radius_for(100.0));
    assert!(sm::radius_for(30000.0).unwrap() <= sm::REACH);
    // Небольшой сдвиг расстояния не меняет радиус.
    assert_eq!(sm::radius_for(1000.0), sm::radius_for(1001.0));
}

#[test]
fn center_maps_to_middle_and_sun_side_is_nearer() {
    let (center, east, _, up, sun) = place();
    let frame = sm::LightFrame::new(center, sun, 1000.0, sm::MAP_SIZE);
    assert!(frame.active());
    let size = f64::from(frame.size);
    let middle = frame.project(center);
    assert!(middle.x.abs() < 2.0 / size);
    assert!(middle.y.abs() < 2.0 / size);
    // Точка над центром ближе к солнцу - глубина меньше.
    assert!(frame.project(center + up * 100.0).z < middle.z);
    // Точка на востоке на 900 м - у края квадрата по x.
    let e = frame.project(center + east * 900.0);
    assert!(((e.x - middle.x).abs() - 0.9).abs() < 0.01);
}

#[test]
fn matrices_match_projection() {
    let (center, east, north, _, sun) = place();
    let frame = sm::LightFrame::new(center, sun, 2000.0, sm::MAP_SIZE);
    let offset = DVec3::new(12.0, -5.0, 40.0);
    for c in [center + east * 300.0, center - north * 700.0] {
        let got = frame.matrix(c) * DVec4::new(offset.x, offset.y, offset.z, 1.0);
        let want = frame.project(c + offset);
        assert!((got.truncate() - want).length() < 1e-9, "{got} {want}");
    }
}

#[test]
fn float32_matrices_keep_precision() {
    // Матрица - в f32, как в видеокарте: ошибка меньше тексела.
    let (center, _, north, _, sun) = place();
    let frame = sm::LightFrame::new(center, sun, 1000.0, sm::MAP_SIZE);
    let tile = center + north * 1500.0;
    let matrix = frame.matrix(tile).as_mat4().as_dmat4();
    let offset = DVec3::new(30.0, 20.0, -10.0);
    let got = matrix * DVec4::new(offset.x, offset.y, offset.z, 1.0);
    let want = frame.project(tile + offset);
    let worst = (got.x - want.x).abs().max((got.y - want.y).abs());
    assert!(worst * f64::from(frame.size) / 2.0 < 0.01, "{worst}");
}

#[test]
fn snapped_center_moves_by_texels() {
    let (center, east, _, _, sun) = place();
    let a = sm::LightFrame::new(center, sun, 1000.0, sm::MAP_SIZE);
    let b = sm::LightFrame::new(center + east * 1.3, sun, 1000.0, sm::MAP_SIZE);
    let shift = b.project(a.center) * f64::from(a.size) / 2.0;
    assert!((shift.x - shift.x.round()).abs() < 1e-6);
    assert!((shift.y - shift.y.round()).abs() < 1e-6);
}

#[test]
fn sun_below_horizon_is_inactive() {
    let (center, _, _, _, sun) = place();
    assert!(!sm::LightFrame::new(center, -sun, 1000.0, sm::MAP_SIZE).active());
}
