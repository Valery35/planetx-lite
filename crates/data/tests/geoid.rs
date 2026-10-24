// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Геоид EGM2008: чтение сетки GeographicLib и запись Terrarium.
//! Перенос `tests/test_geoid.py` модуля и проверки ступеней масштаба.

use data::geoid;

fn pgm(values: &[u16], width: usize, height: usize) -> Vec<u8> {
    let head = format!("P5\n# Description WGS84 EGM2008, test\n# Offset -108.0\n# Scale 0.003\n{width} {height}\n65535\n");
    let mut out = head.into_bytes();
    out.extend(values.iter().flat_map(|v| v.to_be_bytes()));
    out
}

#[test]
fn read_pgm() {
    let got = geoid::read_pgm(&pgm(&[0, 36000, 65535, 1000], 2, 2));
    assert!(got.is_ok());
    let Ok((grid, step)) = got else { return };
    assert_eq!(step, 180.0);
    assert_eq!((grid.width, grid.height), (2, 2));
    assert!((grid.at(0, 0) - -108.0).abs() < 1e-9);
    assert!(grid.at(0, 1).abs() < 1e-9);
    assert!((grid.at(1, 0) - (-108.0 + 0.003 * 65535.0)).abs() < 1e-9);
}

#[test]
fn not_pgm() {
    assert!(geoid::read_pgm(b"P2\n# Offset 1\n# Scale 1\n1 1\n255\n0").is_err());
    // Тело короче заголовка, размеры не числа, нет шкалы - ошибка.
    assert!(geoid::read_pgm(&pgm(&[1, 2, 3], 2, 2)).is_err());
    assert!(geoid::read_pgm(b"P5\n# Offset 1\n# Scale 1\nx 1\n255\n\0\0").is_err());
    assert!(geoid::read_pgm(b"P5\n# Offset 1\n1 1\n255\n\0\0").is_err());
    assert!(geoid::read_pgm(b"P5\n# Offset 1\n# Scale 1\n0 1\n255\n").is_err());
    assert!(geoid::read_pgm(b"").is_err());
}

#[test]
fn terrarium_round_trip() {
    // Тайлы геоида читает обычный разбор высот Terrarium.
    let heights = [-106.9, 85.73, 0.0, -2.66];
    for (rgb, h) in geoid::encode(&heights).iter().zip(heights) {
        let back = f64::from(rgb[0]) * 256.0 + f64::from(rgb[1]) + f64::from(rgb[2]) / 256.0 - 32768.0;
        assert!((back - h).abs() <= 0.005, "{back} != {h}");
    }
    // Нет данных - ноль.
    assert_eq!(geoid::encode(&[f64::NAN]), vec![[128, 0, 0]]);
}

#[test]
fn scale_steps() {
    assert_eq!(geoid::scale_step(geoid::SCALE), 5);
    assert_eq!(geoid::scale_step(1.0), 0);
    assert_eq!(geoid::scale_step(1e9), geoid::SCALES.len() - 1);
    assert_eq!(geoid::scale_text(10000.0), "×10 000");
    assert_eq!(geoid::scale_text(999.6), "×1 000");
    assert_eq!(geoid::scale_text(20000.0), "×20 000");
    assert!(geoid::TILE_URL.ends_with("/geoid/{z}/{x}/{y}.png"));
}
