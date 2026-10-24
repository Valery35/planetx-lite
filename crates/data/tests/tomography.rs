// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Томография мантии: выборка, раскраска, мантия на гранях разреза, файл
//! профиля. Перенос `tests/test_tomography.py` модуля.

use data::cutaway::{self as cw, Faces};
use data::tomography::{self as tm, Tomo};

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

/// Западное полушарие медленнее на 2 %, восточное быстрее, сетка файла -
/// широты с севера, долготы 0-359.
fn model() -> Option<Tomo> {
    let depths: Vec<f64> = (1..=57).map(|k| 50.0 * k as f64).collect();
    let lats: Vec<f64> = (0..179).map(|i| 89.0 - i as f64).collect();
    let lons: Vec<f64> = (0..360).map(f64::from).collect();
    let row: Vec<f32> = lons.iter().map(|lon| if *lon >= 180.0 { -2.0 } else { 2.0 }).collect();
    let dvs: Vec<f32> = row.iter().copied().cycle().take(depths.len() * lats.len() * lons.len()).collect();
    let tomo = tm::make(&depths, &lats, &lons, &dvs);
    assert!(tomo.is_ok(), "{tomo:?}");
    tomo.ok()
}

fn has(colors: &[[u8; 3]], color: [u8; 3]) -> bool {
    colors.contains(&color)
}

#[test]
fn make_sorts_latitudes() {
    let Some(tomo) = model() else { return };
    assert!(tomo.lats[0] < tomo.lats[tomo.lats.len() - 1]);
    assert_eq!((tomo.depths.len(), tomo.lats.len(), tomo.lons.len()), (57, 179, 360));
    assert_eq!(tomo.dvs.len(), 57 * 179 * 360);
}

#[test]
fn make_reorders_grid_and_drops_fill() {
    // Широты с севера, долготы -180..180: строки и столбцы следом за осями.
    let lats = [10.0, 0.0];
    let lons = [-90.0, 0.0, 90.0];
    let dvs = [1.0, 2.0, 3.0, 4.0, 5.0, 99999.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let found = tm::make(&[100.0, 200.0], &lats, &lons, &dvs);
    assert!(found.is_ok(), "{found:?}");
    let Ok(tomo) = found else { return };
    assert_eq!(tomo.lats, [0.0, 10.0]);
    assert_eq!(tomo.lons, [0.0, 90.0, 270.0]);
    let first: Vec<f32> = tomo.dvs[..6].to_vec();
    assert_eq!(first[0], 5.0);
    assert!(first[1].is_nan());
    assert_eq!(first[2..], [4.0, 2.0, 3.0, 1.0]);
    // Сетка не по осям и слишком малая - ошибка.
    assert!(tm::make(&[100.0, 200.0], &lats, &lons, &dvs[..11]).is_err());
    assert!(tm::make(&[100.0], &lats, &lons, &dvs[..6]).is_err());
}

#[test]
fn sample_and_colors() {
    let Some(tomo) = model() else { return };
    let values = tm::sample(&tomo, &[0.0, 0.0, 10.0], &[-90.0, 90.0, 270.0], 300.0);
    for (got, want) in values.iter().zip([-2.0, 2.0, -2.0]) {
        close(*got, want, 1e-8);
    }
    let rgb = tm::colors(&values);
    assert_eq!(rgb[0], tm::STOPS[0].1);
    assert_eq!(rgb[1], tm::STOPS[4].1);
    // Ниже последней глубины модели - край модели.
    let deep = tm::sample(&tomo, &[0.0], &[90.0], 2890.0);
    close(deep[0], 2.0, 1e-7);
    // NaN - цвет нуля.
    assert_eq!(tm::color(f64::NAN), tm::STOPS[2].1);
}

#[test]
fn faces_paint_mantle() {
    // Сектор у 0° долготы - обе половины модели на гранях.
    let Some(tomo) = model() else { return };
    let mesh = cw::faces(&cw::wedge_at(10.0, 0.0, cw::HALF), &Faces { tomo: Some(&tomo), ..Faces::default() });
    assert!(mesh.is_some());
    let Some(mesh) = mesh else { return };
    let rgb: Vec<[u8; 3]> = mesh.vertices.iter().map(|v| [v.color[0], v.color[1], v.color[2]]).collect();
    assert!(has(&rgb, tm::STOPS[0].1));
    assert!(has(&rgb, tm::STOPS[4].1));
    // Оболочек мантии PREM на гранях нет, ядро есть.
    for shell in cw::SHELLS.iter().filter(|s| cw::MANTLE.contains(&s.key)) {
        assert!(!has(&rgb, shell.color), "{}", shell.key);
    }
    assert!(has(&rgb, cw::SHELLS[1].color));
}

#[test]
fn rows_end_at_core() {
    let Some(tomo) = model() else { return };
    let rows = tm::rows(&tomo);
    assert_eq!(rows.len(), 58);
    assert_eq!(rows.last(), Some(&tm::CMB));
}

#[test]
fn save_load() {
    let Some(tomo) = model() else { return };
    let saved = tm::save(&tomo);
    assert!(saved.is_ok(), "{saved:?}");
    let Ok(bytes) = saved else { return };
    let back = tm::load(&bytes);
    assert!(back.is_ok(), "{back:?}");
    let Ok(back) = back else { return };
    assert_eq!((back.depths.clone(), back.lats.clone(), back.lons.clone()), (tomo.depths.clone(), tomo.lats.clone(), tomo.lons.clone()));
    assert!(back.dvs.iter().zip(&tomo.dvs).all(|(a, b)| (a - b).abs() <= 1e-2));
    assert!(tm::load(b"not npz").is_err());
}

#[test]
fn half_precision_rounds_like_numpy() {
    // float16: шаг у 1.0 - 2^-10, у 2.0 - 2^-9, предел 65504.
    let mut tomo = Tomo { depths: vec![0.0, 1.0], lats: vec![0.0, 1.0], lons: vec![0.0, 1.0], dvs: vec![0.0; 8] };
    tomo.dvs[..6].copy_from_slice(&[1.0 + 1.0 / 2048.0, 1.0 + 3.0 / 2048.0, -2.0, 1.0e-7, 0.1, f32::NAN]);
    let saved = tm::save(&tomo).unwrap_or_default();
    let back = tm::load(&saved);
    assert!(back.is_ok(), "{back:?}");
    let Ok(back) = back else { return };
    // Посередине - к чётному.
    assert_eq!(back.dvs[0], 1.0);
    assert_eq!(back.dvs[1], 1.0 + 2.0 / 1024.0);
    assert_eq!(back.dvs[2], -2.0);
    // Денормализованное число и 0.1 - ближайшие значения float16.
    assert_eq!(back.dvs[3], 2.0f32.powi(-24) * 2.0);
    assert_eq!(back.dvs[4], 1638.0 / 16384.0);
    assert!(back.dvs[5].is_nan());
}
