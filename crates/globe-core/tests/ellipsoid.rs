// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Эллипсоид WGS84, пересчёт в ECEF и пересечение луча.
//!
//! Перенос `planetx/tests/test_ellipsoid.py` и тестов тел из
//! `planetx/tests/test_body.py` модуля. Тест формы массива
//! (`test_shape_follows_input`) не переносится - функции Lite
//! принимают одну точку. Тесты мер, тайлов и пределов камеры на других
//! телах переносятся вместе с их модулями.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{PERM, Rng};
use globe_core::DVec3;
use globe_core::ellipsoid::{self as el, Ellipsoid};

const MM: f64 = 1e-3;

/// Эталон - PROJ из QGIS 4.0.3, EPSG:4979 в EPSG:4978, 26 сентября 2026.
const PROJ: [(f64, f64, f64, [f64; 3]); 4] = [
    (58.0105, 56.2294, 150.0, [1882812.758132668, 2815636.425723567, 5386484.577109812]),
    (-89.9, 10.0, 0.0, [10999.70400604673, 1939.5445960730267, -6356742.567109314]),
    (0.0, 0.0, 20000000.0, [26378137.0, 0.0, 0.0]),
    (45.0, -120.0, -400.0, [-2258654.0180682275, -3912103.516013765, 4487065.566153445]),
];

fn wgs84() -> Ellipsoid {
    Ellipsoid::wgs84()
}

fn random_points(count: usize, seed: u64) -> Vec<(f64, f64, f64)> {
    let mut rng = Rng(seed);
    let mut out: Vec<(f64, f64, f64)> = (0..count)
        .map(|_| {
            let lat = rng.uniform(-1.0, 1.0).asin().to_degrees();
            let lon = rng.uniform(-180.0, 180.0);
            let h = rng.uniform(-11000.0, 40e6);
            (lat, lon, h)
        })
        .collect();
    // Особые точки - полюса, экватор, линия смены дат.
    let special = [
        (90.0, 0.0, 0.0),
        (-90.0, 123.0, 0.0),
        (0.0, 180.0, 0.0),
        (0.0, -180.0, 1e7),
        (89.999999, 17.0, 5.0),
        (-45.0, 0.0, -11000.0),
    ];
    for (slot, point) in out.iter_mut().zip(special) {
        *slot = point;
    }
    out
}

fn max_abs_diff(a: DVec3, b: DVec3) -> f64 {
    (a - b).abs().max_element()
}

// Пересчёт широты, долготы и высоты в ECEF.

#[test]
fn geodetic_to_ecef_matches_proj() {
    let e = wgs84();
    for (lat, lon, h, expected) in PROJ {
        let got = e.geodetic_to_ecef(lat, lon, h);
        let d = max_abs_diff(got, DVec3::from_array(expected));
        assert!(d < MM, "{lat} {lon} {h}: {d}");
    }
}

#[test]
fn poles_and_equator() {
    let e = wgs84();
    let north = e.geodetic_to_ecef(90.0, 0.0, 0.0);
    assert!((north.z - e.b).abs() < MM);
    let east = e.geodetic_to_ecef(0.0, 90.0, 0.0);
    assert!((east.y - e.a).abs() < MM);
}

// Обратный пересчёт.

#[test]
fn round_trip_under_one_millimetre() {
    let e = wgs84();
    for (lat, lon, h) in random_points(200_000, 1) {
        let p = e.geodetic_to_ecef(lat, lon, h);
        let (lat2, lon2, h2) = e.ecef_to_geodetic(p);
        let back = e.geodetic_to_ecef(lat2, lon2, h2);
        assert!((back - p).length() < MM, "{lat} {lon} {h}");
        assert!((h2 - h).abs() < MM, "{lat} {lon} {h}");
    }
}

#[test]
fn latitude_and_height_recovered() {
    let e = wgs84();
    for (lat, lon, h) in random_points(20_000, 2) {
        let (lat2, _, h2) = e.ecef_to_geodetic(e.geodetic_to_ecef(lat, lon, h));
        // Угол 1e-8 градуса на поверхности - около 1 мм.
        assert!((lat2 - lat).abs() < 1e-8, "{lat} {lon} {h}");
        assert!((h2 - h).abs() < MM, "{lat} {lon} {h}");
    }
}

#[test]
fn near_axis_high_above_pole() {
    // Глаз почти над полюсом с 4000-40 000 км. В модуле до 6 октября
    // 2026 года подкоренное уходило в минус, широта и высота выходили
    // NaN, и кадр падал в nearest_terrain.
    let e = wgs84();
    for z in [1e7, 2e7, 4e7, 4.6e7, 1e8] {
        for p in [0.0, 1e-3, 0.5, 30.0] {
            for sign in [1.0, -1.0] {
                let xyz = DVec3::new(p, 0.0, sign * z);
                let (lat, lon, h) = e.ecef_to_geodetic(xyz);
                assert!(lat.is_finite() && h.is_finite(), "{p} {z}");
                let back = e.geodetic_to_ecef(lat, lon, h);
                assert!(max_abs_diff(back, xyz) < MM, "{p} {z}");
            }
        }
    }
}

#[test]
fn ecef_to_geodetic_matches_proj() {
    let e = wgs84();
    for (lat, lon, h, xyz) in PROJ {
        let (lat2, lon2, h2) = e.ecef_to_geodetic(DVec3::from_array(xyz));
        assert!((lat2 - lat).abs() < 1e-8, "{lat}");
        assert!((h2 - h).abs() < MM, "{lat}");
        if lat.abs() < 90.0 {
            assert!((lon2 - lon).abs() < 1e-8, "{lat}");
        }
    }
}

// Нормаль.

#[test]
fn normal_is_unit_and_perpendicular_to_surface() {
    let e = wgs84();
    let step = 1e-6;
    for (lat, lon, _) in random_points(1000, 3) {
        let lat = lat.clamp(-89.9, 89.9);
        let n = e.surface_normal(lat, lon);
        assert!((n.length() - 1.0).abs() < 1e-12, "{lat} {lon}");
        let d_lat = e.geodetic_to_ecef(lat + step, lon, 0.0)
            - e.geodetic_to_ecef(lat - step, lon, 0.0);
        let d_lon = e.geodetic_to_ecef(lat, lon + step, 0.0)
            - e.geodetic_to_ecef(lat, lon - step, 0.0);
        for d in [d_lat, d_lon] {
            assert!(n.dot(d.normalize()).abs() < 1e-6, "{lat} {lon}");
        }
    }
}

// Пересечение луча.

#[test]
fn straight_down_from_100_m_over_perm() {
    let e = wgs84();
    let target = e.geodetic_to_ecef(PERM.0, PERM.1, 0.0);
    let normal = e.surface_normal(PERM.0, PERM.1);
    let eye = e.geodetic_to_ecef(PERM.0, PERM.1, 100.0);
    let t = e.ray_intersect(eye, -normal, 0.0).unwrap();
    assert!((t - 100.0).abs() < MM);
    let hit = eye - normal * t;
    assert!((hit - target).length() < MM);
}

#[test]
fn from_space_toward_centre() {
    let e = wgs84();
    let eye = DVec3::new(e.a + 20e6, 0.0, 0.0);
    let t = e.ray_intersect(eye, DVec3::NEG_X, 0.0).unwrap();
    assert!((t - 20e6).abs() < MM);
}

#[test]
fn direction_need_not_be_unit() {
    let e = wgs84();
    let eye = DVec3::new(e.a + 1000.0, 0.0, 0.0);
    let t = e.ray_intersect(eye, DVec3::new(-10.0, 0.0, 0.0), 0.0).unwrap();
    assert!((t - 100.0).abs() < 1e-6);
}

#[test]
fn miss_and_behind() {
    let e = wgs84();
    let eye = DVec3::new(e.a + 1000.0, 0.0, 0.0);
    assert_eq!(e.ray_intersect(eye, DVec3::Z, 0.0), None);
    assert_eq!(e.ray_intersect(eye, DVec3::X, 0.0), None);
}

#[test]
fn grazing_ray_near_horizon() {
    // Сечение по экватору - окружность радиуса A. Луч под углом beta
    // к направлению на центр встречает её на расстоянии
    // r cos(beta) - sqrt(A^2 - r^2 sin^2(beta)).
    let e = wgs84();
    let r = e.a + 1000.0;
    let eye = DVec3::new(r, 0.0, 0.0);
    let tangent = (e.a / r).asin();
    for beta in [tangent - 1e-4, tangent - 1e-7] {
        let d = DVec3::new(-beta.cos(), beta.sin(), 0.0);
        let expected = r * beta.cos() - (e.a * e.a - (r * beta.sin()).powi(2)).sqrt();
        let t = e.ray_intersect(eye, d, 0.0).unwrap();
        assert!((t - expected).abs() < MM, "{beta}");
    }
    let above = tangent + 1e-7;
    let d = DVec3::new(-above.cos(), above.sin(), 0.0);
    assert_eq!(e.ray_intersect(eye, d, 0.0), None);
}

#[test]
fn inflated_ellipsoid() {
    let e = wgs84();
    let eye = e.geodetic_to_ecef(PERM.0, PERM.1, 1000.0);
    let normal = e.surface_normal(PERM.0, PERM.1);
    let t = e.ray_intersect(eye, -normal, 300.0).unwrap();
    let (_, _, h) = e.ecef_to_geodetic(eye - normal * t);
    // Раздутый эллипсоид отличается от поверхности на высоте 300 м
    // на доли метра.
    assert!((h - 300.0).abs() < 1.0);
}

// Тела, перенос из test_body.py.

#[test]
fn radius_of_each_body() {
    for (body, radius) in [(el::EARTH, 6378137.0), (el::MARS, 3396190.0), (el::MOON, 1737400.0)] {
        let p = Ellipsoid::new(body).geodetic_to_ecef(0.0, 0.0, 0.0);
        assert!((p.length() - radius).abs() < 1e-3, "{}", body.key);
    }
}

#[test]
fn round_trip_on_spheres() {
    for body in [el::MARS, el::MOON] {
        let e = Ellipsoid::new(body);
        // Олимп на Марсе.
        let (lat, lon, h) = (18.65, -133.8, 21900.0);
        let (lat2, lon2, h2) = e.ecef_to_geodetic(e.geodetic_to_ecef(lat, lon, h));
        assert!((lat2 - lat).abs() < 1e-9, "{}", body.key);
        assert!((lon2 - lon).abs() < 1e-9, "{}", body.key);
        assert!((h2 - h).abs() < 1e-4, "{}", body.key);
    }
}

#[test]
fn unknown_key_is_earth() {
    assert_eq!(el::body_by_key("vulcan"), &el::EARTH);
    assert_eq!(el::body_by_key("mars"), &el::MARS);
    assert_eq!(el::body_by_key("vesta"), &el::VESTA);
}
