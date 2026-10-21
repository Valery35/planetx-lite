// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Перелёт по методу van Wijk и Nuij.
//!
//! Перенос `planetx/tests/test_flight.py` модуля. Вращения вокруг точки
//! (`Spin`) модуль не проверял, тест вращения добавлен в Lite.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::PERM;
use globe_core::camera::Camera;
use globe_core::ellipsoid::Ellipsoid;
use globe_core::flight::{self as fl, Flight, Motion, Path, Spin};
use globe_core::navigation::{Navigator, Pose};
use globe_core::{DMat3, DVec3};

const MOSCOW: (f64, f64) = (55.7558, 37.6173);

fn scale() -> f64 {
    2.0 * 22.5f64.to_radians().tan()
}

fn cases() -> [(&'static str, f64, f64, f64); 5] {
    let s = scale();
    [
        ("Москва - Пермь на 3 км", 1.15e6, 3000.0 * s, 3000.0 * s),
        ("космос - улица", 0.0, 2e7 * s, 300.0 * s),
        ("улица - улица через полпланеты", 1.9e7, 300.0 * s, 300.0 * s),
        ("улица - космос рядом", 5e4, 300.0 * s, 1e7 * s),
        ("соседний дом", 150.0, 500.0 * s, 500.0 * s),
    ]
}

fn wgs84() -> Ellipsoid {
    Ellipsoid::wgs84()
}

fn linspace(a: f64, b: f64, count: usize) -> impl Iterator<Item = f64> {
    (0..count).map(move |i| a + (b - a) * i as f64 / (count - 1) as f64)
}

/// Приращения ds по равным шагам s, в метрике статьи.
///
/// ds² = (ρ²·du² + dw²/ρ²) / w². Из формул пути du/ds = w·sech/ρ
/// и dw/ds = -ρ·w·tanh, и тогда ds'² = sech² + tanh² = 1.
fn metric_steps(path: &Path, count: usize) -> Vec<f64> {
    let uw: Vec<(f64, f64)> = linspace(0.0, path.length, count + 1).map(|s| path.at(s)).collect();
    let rho = fl::RHO;
    uw.windows(2)
        .map(|p| {
            let (du, dw) = (p[1].0 - p[0].0, p[1].1 - p[0].1);
            let w = (p[0].1 * p[1].1).sqrt();
            (rho * rho * du * du + dw * dw / (rho * rho)).sqrt() / w
        })
        .collect()
}

// Путь.

#[test]
fn ends() {
    for (name, u1, w0, w1) in cases() {
        let path = Path::new(u1, w0, w1);
        let (u, w) = path.at(0.0);
        assert!(u.abs() < 1e-6 * u1.max(1.0), "{name}");
        assert!((w - w0).abs() < 1e-9 * w0, "{name}");
        let (u, w) = path.at(path.length);
        let target_u = if path.zoom_only { 0.0 } else { u1 };
        assert!((u - target_u).abs() < 1e-6 * u1.max(1.0), "{name}");
        assert!((w - w1).abs() < 1e-6 * w1, "{name}");
    }
}

#[test]
fn pure_zoom_length() {
    let s = scale();
    let path = Path::new(0.0, 2e7 * s, 300.0 * s);
    assert!(path.zoom_only);
    assert!((path.length - (300.0f64 / 2e7).ln().abs() / fl::RHO).abs() < 5e-13);
}

#[test]
fn constant_speed_in_the_metric() {
    for (name, u1, w0, w1) in cases() {
        let path = Path::new(u1, w0, w1);
        let steps = metric_steps(&path, 2000);
        let total: f64 = steps.iter().sum();
        let mean = total / steps.len() as f64;
        assert!(steps.iter().all(|s| (s / mean - 1.0).abs() < 0.01), "{name}");
        assert!((total - path.length).abs() < 0.01 * path.length, "{name}");
    }
}

#[test]
fn long_flight_rises() {
    let s = scale();
    let path = Path::new(1.15e6, 3000.0 * s, 3000.0 * s);
    let top = linspace(0.0, path.length, 200).map(|x| path.at(x).1).fold(f64::NEG_INFINITY, f64::max);
    assert!(top > 50.0 * 3000.0 * s, "{top}");
}

#[test]
fn no_nan() {
    for (name, u1, w0, w1) in cases() {
        let path = Path::new(u1, w0, w1);
        for s in linspace(0.0, path.length, 50) {
            let (u, w) = path.at(s);
            assert!(u.is_finite() && w.is_finite(), "{name}");
            assert!(w > 0.0, "{name}");
        }
    }
}

// Перелёт.

fn moscow_to_perm() -> Flight {
    let start = Pose::new(MOSCOW.0, MOSCOW.1, 3000.0, 120.0, 45.0);
    Flight::new(&wgs84(), &start, Pose::new(PERM.0, PERM.1, 2000.0, 0.0, 0.0), 45.0)
}

#[test]
fn starts_and_ends_exactly() {
    let f = moscow_to_perm();
    let p = f.pose_at(0.0);
    assert!((p.lat - MOSCOW.0).abs() < 5e-10);
    assert!((p.lon - MOSCOW.1).abs() < 5e-10);
    assert!((p.distance - 3000.0).abs() < 1e-6);
    assert!((p.heading - 120.0).abs() < 5e-10);
    let p = f.pose_at(f.duration);
    assert_eq!((p.lat, p.lon, p.distance, p.heading, p.tilt), (PERM.0, PERM.1, 2000.0, 0.0, 0.0));
}

#[test]
fn no_jump_before_the_end() {
    let f = moscow_to_perm();
    let p = f.pose_at(f.duration * 0.999_999);
    assert!((p.lat - PERM.0).abs() + (p.lon - PERM.1).abs() < 1e-4);
    assert!((p.distance - 2000.0).abs() < 1.0);
}

#[test]
fn time_runs_evenly_along_the_path() {
    let f = moscow_to_perm();
    for share in [0.1, 0.25, 0.5, 0.9] {
        assert!((f.s_at(share * f.duration) - share * f.path.length).abs() < 5e-10);
    }
}

#[test]
fn duration() {
    let f = moscow_to_perm();
    assert!((f.duration - f.path.length / fl::SPEED).abs() < 5e-8);
    assert!(3.0 < f.duration && f.duration < 6.0, "{}", f.duration);
    let start = Pose::new(PERM.0, PERM.1, 1000.0, 0.0, 0.0);
    let near = Flight::new(&wgs84(), &start, Pose::new(PERM.0 + 1e-5, PERM.1, 1000.0, 0.0, 0.0), 45.0);
    assert_eq!(near.duration, fl::MIN_TIME);
}

#[test]
fn heading_takes_short_way() {
    let start = Pose::new(PERM.0, PERM.1, 1000.0, 350.0, 0.0);
    let f = Flight::new(&wgs84(), &start, Pose::new(MOSCOW.0, MOSCOW.1, 1000.0, 10.0, 0.0), 45.0);
    let mid = f.pose_at(f.duration / 2.0).heading;
    assert!(!(5.0..=355.0).contains(&mid), "{mid}");
}

#[test]
fn across_the_antimeridian() {
    let start = Pose::new(60.0, 170.0, 5000.0, 0.0, 0.0);
    let f = Flight::new(&wgs84(), &start, Pose::new(60.0, -170.0, 5000.0, 0.0, 0.0), 45.0);
    assert!(f.duration > 0.0);
    for t in linspace(0.0, f.duration, 20) {
        let lon = f.pose_at(t).lon;
        assert!(lon >= 170.0 || lon <= -170.0, "{lon}");
    }
}

#[test]
fn spin_turns_after_the_flight() {
    // Вращение вокруг точки: сначала перелёт с прежним расстоянием,
    // отвесный вид наклоняется на SPIN_TILT, потом азимут растёт.
    let start = Pose::new(MOSCOW.0, MOSCOW.1, 3000.0, 20.0, 0.0);
    let spin = Spin::new(&wgs84(), &start, PERM.0, PERM.1, 45.0);
    let motion = Motion::Spin(spin.clone());
    assert!(motion.duration().is_infinite());
    let d = spin.flight.duration;
    assert!(d >= fl::MIN_TIME);
    let p = motion.pose_at(d + 10.0);
    assert_eq!((p.lat, p.lon, p.distance, p.tilt), (PERM.0, PERM.1, 3000.0, fl::SPIN_TILT));
    assert!((p.heading - (20.0 + 10.0 * fl::SPIN_RATE)).abs() < 1e-9);
}

// Перелёт в навигаторе.

fn navigator() -> Navigator {
    let cam = Camera::new(wgs84(), DVec3::ZERO, DMat3::IDENTITY).with_viewport(1920, 1080);
    Navigator::new(cam, Pose::new(MOSCOW.0, MOSCOW.1, 3000.0, 0.0, 0.0))
}

#[test]
fn flight_runs_and_press_stops_it() {
    let mut n = navigator();
    let f = Flight::new(&wgs84(), &n.pose, Pose::new(PERM.0, PERM.1, 2000.0, 0.0, 0.0), 45.0);
    n.start_flight(Motion::Flight(f), 0.0);
    assert!(n.step(1.0));
    assert_ne!((n.pose.lat, n.pose.lon), MOSCOW);
    n.press(960.0, 540.0, 1.1);
    assert!(!n.has_flight());
}

#[test]
fn flight_ends_at_target() {
    let mut n = navigator();
    let f = Flight::new(&wgs84(), &n.pose, Pose::new(PERM.0, PERM.1, 2000.0, 0.0, 0.0), 45.0);
    n.start_flight(Motion::Flight(f), 0.0);
    assert!(n.has_flight());
    let mut t = 0.0;
    while n.step(t) {
        t += 1.0 / 60.0;
        assert!(t < 20.0);
    }
    assert_eq!((n.pose.lat, n.pose.lon), PERM);
}

// Ввод координат.

#[test]
fn parse_formats() {
    for text in ["58.0105, 56.2294", "58.0105 56.2294", " 58.0105,56.2294 ", "58,0105; 56,2294"] {
        assert_eq!(fl::parse_latlon(text), Some(PERM), "{text}");
    }
}

#[test]
fn parse_rejects() {
    for text in ["", "58.0105", "91, 10", "10, 181", "север, юг", "1, 2, 3", "nan, 10", "inf, 10"] {
        assert_eq!(fl::parse_latlon(text), None, "{text}");
    }
}

// Вид на охват.

#[test]
fn extent_fits_the_frame() {
    // Охват растра «В_top» у Соликамска и вытянутый охват.
    let e = wgs84();
    for (west, south, east, north) in [(56.6973, 59.5166, 56.9287, 59.6038), (40.0, 55.0, 60.0, 56.0)] {
        for (width, height) in [(1600u32, 900u32), (900, 1600)] {
            let (lat, lon, distance) =
                fl::fit_view(west, south, east, north, 45.0, f64::from(width) / f64::from(height));
            let cam = Camera::look_at(e, lat, lon, distance, 0.0, 0.0, 0.0).with_viewport(width, height);
            for la in [south, north] {
                for lo in [west, east] {
                    let (px, front) = cam.project(e.geodetic_to_ecef(la, lo, 0.0));
                    assert!(front);
                    assert!(px.x > 0.0 && px.x < f64::from(width), "{west} {width}");
                    assert!(px.y > 0.0 && px.y < f64::from(height), "{west} {width}");
                }
            }
        }
    }
}

#[test]
fn point_layer_is_not_too_close() {
    let (_, _, distance) = fl::fit_view(56.2, 58.0, 56.2, 58.0, 45.0, 1.5);
    assert_eq!(distance, fl::MIN_FIT_DISTANCE);
}

// Путь автора «Мой путь 3» через линию перемены дат, 18 октября
// 2026 года: в модуле перелёт уходил на другую сторону Земли.

const LONS: [f64; 3] = [-174.25, -178.14, 179.33];
const LATS: [f64; 3] = [-16.51, -27.44, -35.09];

#[test]
fn span_across_date_line() {
    let (west, east) = fl::lon_span(&LONS);
    assert!((west - 179.33).abs() < 5e-7);
    assert!((east - (360.0 - 174.25)).abs() < 5e-7);
}

#[test]
fn span_ordinary() {
    assert_eq!(fl::lon_span(&[10.0, -5.0, 30.0]), (-5.0, 30.0));
    assert_eq!(fl::lon_span(&[42.0]), (42.0, 42.0));
    assert_eq!(fl::lon_span(&[]), (0.0, 0.0));
}

#[test]
fn fit_view_looks_at_the_path() {
    let (west, east) = fl::lon_span(&LONS);
    let south = LATS.iter().copied().fold(f64::INFINITY, f64::min);
    let north = LATS.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let (_, lon, distance) = fl::fit_view(west, south, east, north, 45.0, 1.5);
    // Середина пути - около 177.5° з. д., а не 0°.
    assert!((lon.abs() - 177.46).abs() < 0.01, "{lon}");
    assert!(distance < 5.0e6);
    // Восток меньше запада - та же рамка в записи Region KML.
    let (_, lon2, d2) = fl::fit_view(179.33, -35.09, -174.25, -16.51, 45.0, 1.5);
    assert!((lon2 - lon).abs() < 5e-7);
    assert!((d2 - distance).abs() < 5e-4);
}
