// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Заглушки вместо снимка и окно тайла в картинке предка.
//!
//! Перенос `planetx/tests/test_placeholder.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use net::placeholder as ph;

fn tile(rgb: [u8; 3]) -> Vec<u8> {
    (0..256 * 256).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect()
}

/// Генератор SplitMix64.
struct Rng(u64);

impl Rng {
    fn below(&mut self, lo: u8, hi: u8) -> u8 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        lo + (z % u64::from(hi - lo)) as u8
    }
}

#[test]
fn esri_gray_with_text() {
    // Как у Esri: 204 серого, по середине белая надпись, 94 %.
    let mut rgba = tile([204, 204, 204]);
    for row in 110..145 {
        for col in 60..196 {
            let i = (row * 256 + col) * 4;
            rgba[i..i + 3].copy_from_slice(&[255, 255, 255]);
        }
    }
    assert!(ph::is_placeholder(&rgba));
}

#[test]
fn imagery_is_not() {
    let mut rng = Rng(1);
    let rgba: Vec<u8> =
        (0..256 * 256).flat_map(|_| [rng.below(60, 120), rng.below(60, 120), rng.below(60, 120), 255]).collect();
    assert!(!ph::is_placeholder(&rgba));
}

#[test]
fn flat_water_is_not() {
    // Ровная вода не серая: каналы различаются.
    assert!(!ph::is_placeholder(&tile([20, 60, 110])));
}

#[test]
fn noisy_snow_is_not() {
    // Снег в JPEG с шумом: серый, но не одного точного цвета.
    let mut rng = Rng(2);
    let rgba: Vec<u8> = (0..256 * 256)
        .flat_map(|_| {
            let v = rng.below(240, 255);
            [v, v, v, 255]
        })
        .collect();
    assert!(!ph::is_placeholder(&rgba));
}

#[test]
fn empty() {
    assert!(!ph::is_placeholder(&[]));
}

#[test]
fn ancestor() {
    assert_eq!(ph::ancestor((19, 345884, 152543), 2), (17, 86471, 38135));
}

#[test]
fn quarter_of_parent() {
    assert_eq!(ph::crop_window((5, 0, 0), 1, 256), (0, 0, 128));
    assert_eq!(ph::crop_window((5, 1, 0), 1, 256), (128, 0, 128));
    assert_eq!(ph::crop_window((5, 0, 1), 1, 256), (0, 128, 128));
}

#[test]
fn deep_window() {
    // Три уровня вверх: кусок 32 пикселя, место по младшим битам.
    assert_eq!(ph::crop_window((10, 13, 6), 3, 256), (5 * 32, 6 * 32, 32));
}
