// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Звёзды: звёздное время, поворот неба, вид точки, файл каталога.
//! Перенос `tests/test_stars.py` модуля без картинки неба.

use std::f64::consts::PI;

use data::stars;

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

#[test]
fn gmst_at_j2000() {
    close(stars::gmst(stars::UNIX_J2000).to_degrees(), 280.46061837, 1e-6);
}

#[test]
fn sidereal_day_is_shorter() {
    // За звёздные сутки, 86164.09 с, небо делает полный оборот.
    let start = 1.8e9;
    let turn = stars::gmst(start + 86164.0905) - stars::gmst(start);
    let rest = turn - (turn / (2.0 * PI)).round() * 2.0 * PI;
    close(rest, 0.0, 1e-4);
}

#[test]
fn star_at_gmst_is_over_greenwich() {
    let when = 1.8e9;
    let ra = stars::gmst(when);
    let ecef = stars::to_ecef(stars::sky_direction(ra, 0.3), when);
    close(ecef[1].atan2(ecef[0]), 0.0, 1e-9);
    close(ecef[2].asin(), 0.3, 1e-9);
}

#[test]
fn look() {
    let (a, b, c) = (stars::size(-1.46), stars::size(1.0), stars::size(6.0));
    assert!(a > b && b > c);
    close(c, stars::SIZE_FAINT, 1e-12);
    let (blue, red) = (stars::color(-0.3), stars::color(1.6));
    assert!(blue[2] > blue[0] && red[0] > red[2]);
    assert!(stars::color(f64::NAN).iter().all(|v| v.is_finite()));
    assert_eq!(stars::fade(1000.0), 0.0);
    assert_eq!(stars::fade(stars::FADE_HIGH), 1.0);
    let mid = stars::fade(70000.0);
    assert!(0.0 < mid && mid < 1.0);
}

#[test]
fn npy_is_checked() {
    assert!(stars::read_npy(b"not numpy").is_err());
    // Таблица 1 × 4 float32 вручную.
    let header = "{'descr': '<f4', 'fortran_order': False, 'shape': (1, 4), }";
    let mut data = b"\x93NUMPY\x01\x00".to_vec();
    let pad = 64 - (10 + header.len() + 1) % 64;
    let text = format!("{header}{}\n", " ".repeat(pad));
    data.extend_from_slice(&(text.len() as u16).to_le_bytes());
    data.extend_from_slice(text.as_bytes());
    for v in [1.0f32, 0.5, 2.0, 0.3] {
        data.extend_from_slice(&v.to_le_bytes());
    }
    let rows = stars::read_npy(&data).unwrap_or_default();
    assert_eq!(rows, vec![[1.0, 0.5, 2.0, f64::from(0.3f32)]]);
}

#[test]
fn file_has_sirius() {
    let Ok(bytes) = std::fs::read(r"C:\Dev\planetx\planetx\data\stars.npy") else {
        eprintln!("данных модуля нет, проверка пропущена");
        return;
    };
    let table = stars::read_npy(&bytes).unwrap_or_default();
    assert!(table.len() > 5000);
    assert!(table.iter().all(|r| r[2] <= stars::MAX_MAG + 1e-6));
    let sirius = table.iter().min_by(|a, b| a[2].total_cmp(&b[2])).copied().unwrap_or_default();
    close(sirius[0].to_degrees(), 101.287, 0.01);
    close(sirius[1].to_degrees(), -16.716, 0.01);
    assert_eq!(stars::parse(&bytes).unwrap_or_default().len(), table.len());
}
