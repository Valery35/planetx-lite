// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Непрозрачность выделенных меток ползунком. Перенос
//! `tests/test_opacity.py` модуля.

use scene::opacity;

#[test]
fn selection_is_most_opaque() {
    let v = opacity::selection_opacity(&[vec![Some([255, 0, 0, 51]), Some([255, 0, 0, 25])], vec![Some([0, 0, 0, 102]), None]]);
    assert!((v - 0.4).abs() < 1e-9);
    assert_eq!(opacity::selection_opacity(&[]), 1.0);
}

#[test]
fn scaling_keeps_fill_share() {
    let out = opacity::scaled(&[Some([10, 20, 30, 200]), Some([1, 2, 3, 100])], 200.0 / 255.0, 0.5 * 200.0 / 255.0);
    assert_eq!(out, [Some([10, 20, 30, 100]), Some([1, 2, 3, 50])]);
}

#[test]
fn back_to_start_is_exact() {
    let colors = [Some([10, 20, 30, 137]), Some([1, 2, 3, 61])];
    let base = opacity::opacity_of(&colors);
    assert_ne!(opacity::scaled(&colors, base, 0.1), colors);
    // Ползунок считает от цветов при выделении, а не от уже уменьшенных,
    // поэтому возврат точный.
    assert_eq!(opacity::scaled(&colors, base, base), colors);
}

#[test]
fn from_transparent() {
    let out = opacity::scaled(&[Some([1, 1, 1, 0]), Some([2, 2, 2, 0]), None], 0.0, 0.8);
    assert_eq!(out[0].map(|c| c[3]), Some(204));
    assert_eq!(out[1].map(|c| c[3]), Some(102));
    assert_eq!(out[2], None);
}

#[test]
fn clipped() {
    assert_eq!(opacity::scaled(&[Some([0, 0, 0, 200])], 0.5, 1.0)[0].map(|c| c[3]), Some(255));
}
