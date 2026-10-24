// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Палеогеография: возрасты, периоды, раскраска и отмывка карт.
//! Перенос `tests/test_paleo.py` модуля.

use data::paleo;

#[test]
fn ages() {
    assert_eq!(paleo::AGES.len(), 109);
    assert_eq!((paleo::AGES[0], paleo::AGES[108]), (0, 540));
}

#[test]
fn nearest() {
    assert_eq!(paleo::nearest(252.0), 250);
    assert_eq!(paleo::nearest(1000.0), 540);
    assert_eq!(paleo::nearest(-3.0), 0);
    // Из двух равных - младший возраст, пустой набор - весь набор.
    assert_eq!(paleo::nearest_in(7.5, &[10, 5]), 5);
    assert_eq!(paleo::nearest_in(252.0, &[]), 250);
    assert_eq!(paleo::nearest(f64::NAN), 0);
}

#[test]
fn tile_url_has_age_and_tile() {
    assert!(paleo::tile_url(250, 2, 1, 3).ends_with("/paleomap/250/2/1/3.jpg"));
}

#[test]
fn period() {
    assert_eq!(paleo::period(0.0), "quaternary");
    assert_eq!(paleo::period(100.0), "cretaceous");
    assert_eq!(paleo::period(280.0), "permian");
    assert_eq!(paleo::period(600.0), "precambrian");
}

#[test]
fn sea_is_blue_and_land_is_not() {
    let got = paleo::colors(&[-3000.0, 500.0]);
    let [sea, land] = got.as_slice() else { return };
    assert!(sea[2] > sea[0]);
    assert!(land[1] > land[2]);
    // Края шкалы держат крайний цвет.
    assert_eq!(paleo::colors(&[-1e6, 1e6]), vec![[8, 28, 66], [236, 236, 236]]);
}

#[test]
fn flat_ground_is_not_shaded() {
    let shade = paleo::hillshade(&[100.0; 20 * 40], 40, 0.1).unwrap_or_default();
    assert_eq!(shade.len(), 800);
    assert!(shade.iter().all(|v| (v - 1.0).abs() < 1e-9));
}

#[test]
fn slope_to_sun_is_lighter() {
    // Высота растёт к юго-востоку: склон смотрит на северо-запад,
    // к солнцу, и светлее равнины. Обратный склон темнее.
    let (rows, cols) = (20, 40);
    let rise: Vec<f64> = (0..rows * cols).map(|i| ((i / cols) + (i % cols)) as f64 * 50.0).collect();
    let fall: Vec<f64> = rise.iter().map(|v| -v).collect();
    let towards = paleo::hillshade(&rise, cols, 0.1).unwrap_or_default();
    let away = paleo::hillshade(&fall, cols, 0.1).unwrap_or_default();
    assert_eq!((towards.len(), away.len()), (800, 800));
    assert!(towards[10 * cols + 20] > 1.0);
    assert!(away[10 * cols + 20] < 1.0);
}

#[test]
fn picture_shape() {
    let image = paleo::picture(&[0.0; 10 * 20], 20, 0.1).unwrap_or_default();
    assert_eq!(image.len(), 200);
}

#[test]
fn bad_shape_is_none() {
    assert!(paleo::hillshade(&[0.0; 7], 2, 0.1).is_none());
    assert!(paleo::hillshade(&[0.0; 4], 0, 0.1).is_none());
    assert!(paleo::picture(&[0.0; 3], 2, 0.1).is_none());
    // Одна и две строки - без наклона по широте.
    assert_eq!(paleo::hillshade(&[5.0; 4], 4, 0.1).map(|s| s.len()), Some(4));
    assert_eq!(paleo::hillshade(&[5.0, 6.0], 1, 0.1).map(|s| s.len()), Some(2));
}
