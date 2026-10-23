// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Положение солнца против таблиц и свет поверхности. Перенос
//! `tests/test_sun.py` модуля.

use scene::sun;

/// Секунды Unix по дате UTC.
fn utc(y: i64, mo: i64, d: i64, h: i64, mi: i64) -> f64 {
    // Дни от 1970-01-01 по алгоритму Говарда Хиннанта.
    let (y, mo) = if mo <= 2 { (y - 1, mo + 12) } else { (y, mo) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (mo - 3) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    (days * 86400 + h * 3600 + mi * 60) as f64
}

fn close(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance, "{a} != {b}");
}

#[test]
fn solstice_and_equinox_declination() {
    close(sun::subsolar(utc(2024, 6, 20, 20, 51)).0, 23.44, 0.02);
    close(sun::subsolar(utc(2024, 3, 20, 3, 6)).0, 0.0, 0.02);
    close(sun::subsolar(utc(2024, 12, 21, 9, 21)).0, -23.44, 0.02);
}

#[test]
fn equation_of_time() {
    for ((y, m, d), minutes) in [((2025, 11, 3), 16.4), ((2025, 2, 11), -14.2), ((2025, 6, 13), 0.0)] {
        let (_, lon) = sun::subsolar(utc(y, m, d, 12, 0));
        close(lon, -minutes / 4.0, 0.08);
    }
}

#[test]
fn noon_elevation_in_perm() {
    let best = (0..60).map(|m| sun::elevation(58.01, 56.23, utc(2024, 6, 20, 8, 0) + 60.0 * f64::from(m))).fold(f64::MIN, f64::max);
    close(best, 55.43, 0.05);
}

#[test]
fn direction_is_unit_and_points_to_subsolar() {
    let t = utc(2026, 9, 30, 6, 0);
    let d = sun::direction(t);
    close((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt(), 1.0, 1e-12);
    let (lat, lon) = sun::subsolar(t);
    close(sun::elevation(lat, lon, t), 90.0, 1e-4);
}

#[test]
fn night_on_the_opposite_side() {
    let t = utc(2026, 9, 30, 6, 0);
    let (lat, lon) = sun::subsolar(t);
    close(sun::elevation(-lat, lon + 180.0, t), -90.0, 1e-4);
}

#[test]
fn flat_ground_at_45_degrees_is_one() {
    close(sun::brightness(45.0f64.to_radians().sin()), 1.0, 1e-12);
}

#[test]
fn night_is_dark_and_day_is_bright() {
    close(sun::brightness(-0.5), sun::NIGHT / sun::flat(), 1e-12);
    assert!(sun::brightness(1.0) > 1.0);
}

#[test]
fn twilight_is_monotonic() {
    let values: Vec<f64> = (-40..=40).map(|c| sun::brightness(f64::from(c) / 100.0)).collect();
    assert!(values.windows(2).all(|w| w[0] <= w[1]));
}
