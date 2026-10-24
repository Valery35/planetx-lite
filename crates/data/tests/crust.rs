// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Кора CRUST1.0: разбор файла границ, архив, ячейки, слои. Перенос
//! `tests/test_crust.py` модуля.
//!
//! Модель в программу не входит. Проверка на настоящем файле идёт, если
//! переменная PLANETX_CRUST1 указывает на архив crust1.0.tar.gz.

use std::io::Write;

use data::crust::{self as cr, Crust};

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

/// Отметки, по которым видна ячейка: Мохо = -(10 + строка + столбец/1000).
fn sample_bounds() -> Vec<f32> {
    let mut b = Vec::with_capacity(cr::ROWS * cr::COLS * cr::BOUNDS);
    for row in 0..cr::ROWS {
        for col in 0..cr::COLS {
            b.extend([1.0, 1.0, 1.0, 0.0, -1.25, -2.5, -3.75, -5.0]);
            b.push(-(10.0 + row as f64 + col as f64 / 1000.0) as f32);
        }
    }
    b
}

/// Скорости: западное полушарие - базальты 5.0 км/с в верхней коре,
/// восточное - 6.1 км/с.
fn sample_speeds() -> Vec<f32> {
    let mut vp = vec![6.5f32; cr::ROWS * cr::COLS * cr::BOUNDS];
    for row in 0..cr::ROWS {
        for col in 0..cr::COLS {
            vp[(row * cr::COLS + col) * cr::BOUNDS + cr::UPPER_CRUST] = if col < 180 { 5.0 } else { 6.1 };
        }
    }
    vp
}

fn as_text(values: &[f32]) -> Vec<u8> {
    let lines: Vec<String> =
        values.chunks(cr::BOUNDS).map(|cell| cell.iter().map(|v| format!("{v:7.3}")).collect::<Vec<_>>().join(" ")).collect();
    lines.join("\n").into_bytes()
}

/// Заголовок ustar члена name размером size.
fn header(name: &str, size: usize) -> [u8; 512] {
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    h[100..108].copy_from_slice(b"0000644\0");
    h[108..116].copy_from_slice(b"0000000\0");
    h[116..124].copy_from_slice(b"0000000\0");
    h[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
    h[136..148].copy_from_slice(b"00000000000\0");
    h[156] = b'0';
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|b| u32::from(*b)).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    h
}

fn as_archive(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tar = Vec::new();
    for (name, data) in members {
        tar.extend_from_slice(&header(name, data.len()));
        tar.extend_from_slice(data);
        tar.resize(tar.len().div_ceil(512) * 512, 0);
    }
    tar.resize(tar.len() + 1024, 0);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    assert!(gz.write_all(&tar).is_ok());
    gz.finish().unwrap_or_default()
}

fn model(with_speeds: bool) -> Option<Crust> {
    let bounds = as_text(&sample_bounds());
    let speeds = as_text(&sample_speeds());
    let mut members: Vec<(&str, &[u8])> = vec![("readme", b"CRUST1.0"), (cr::MEMBER, &bounds)];
    if with_speeds {
        members.push((cr::VP_MEMBER, &speeds));
    }
    let found = cr::from_archive(&as_archive(&members));
    assert!(found.is_ok(), "{found:?}");
    found.ok()
}

// Тип коры.

#[test]
fn type_by_upper_crust_speed() {
    let Some(model) = model(true) else { return };
    assert_eq!(model.oceanic(0.5, -30.0), Some(true));
    assert_eq!(model.oceanic(0.5, 30.0), Some(false));
}

#[test]
fn archive_without_speeds_has_no_type() {
    let Some(model) = model(false) else { return };
    assert!(model.vp().is_none());
    assert_eq!(model.oceanic(0.5, 0.5), None);
}

// Модель.

#[test]
fn cells_follow_file_order() {
    // Первая строка - 89.5° с. ш., первый столбец - 179.5° з. д.
    // Широта 10° - граница ячеек, точка уходит в южную, 9-10°.
    let got: Vec<(usize, usize)> = [(89.9, -179.9), (0.5, 0.5), (-89.9, 179.9), (10.0, 190.0)].map(|(la, lo)| cr::cell(la, lo)).to_vec();
    assert_eq!(got, vec![(0, 0), (89, 180), (179, 359), (80, 10)]);
}

#[test]
fn moho_and_thickness_by_cell() {
    let Some(model) = model(false) else { return };
    close(model.moho(0.5, 0.5), 10.0 + 89.0 + 0.180, 5e-4);
    close(model.thickness(0.5, 0.5), 1.0 + 99.180, 5e-4);
}

#[test]
fn layers_skip_empty() {
    let Some(model) = model(false) else { return };
    let layers = model.layers(0.5, 0.5);
    let keys: Vec<&str> = layers.iter().map(|(k, _, _)| *k).collect();
    // Вода и лёд нулевой толщины пропущены.
    assert!(!keys.contains(&"water"));
    assert!(!keys.contains(&"ice"));
    assert_eq!(keys.last(), Some(&"crust_lower"));
    for pair in layers.windows(2) {
        let ((_, top, bottom), (_, below, _)) = (pair[0], pair[1]);
        assert!(top > bottom);
        assert_eq!(bottom, below);
    }
}

#[test]
fn surfaces_in_metres_by_cell() {
    // Семь кровель сверху вниз, метры, ячейка точки - та же строка
    // и столбец сетки, что у cell.
    let Some(model) = model(false) else { return };
    let found = model.surfaces();
    let keys: Vec<&str> = found.iter().map(|(k, _)| *k).collect();
    assert_eq!(keys, ["sediments_upper", "sediments_middle", "sediments_lower", "crust_upper", "crust_middle", "crust_lower", "moho"]);
    let (row, col) = cr::cell(0.5, 0.5);
    let value = |key: &str, row: usize, col: usize| {
        found.iter().find(|(k, _)| *k == key).and_then(|(_, v)| v.get(row * cr::COLS + col)).copied().unwrap_or(f64::NAN)
    };
    close(value("moho", row, col), -1000.0 * model.moho(0.5, 0.5), 0.05);
    assert_eq!(value("sediments_upper", 0, 0), 1000.0);
    assert!(found.iter().all(|(_, v)| v.len() == cr::ROWS * cr::COLS));
    // Середина ячейки [row, col] - GRID_NORTH - row, GRID_WEST + col.
    let lat = cr::GRID_NORTH - row as f64 * cr::GRID_STEP;
    let lon = cr::GRID_WEST + col as f64 * cr::GRID_STEP;
    assert_eq!((lat, lon), (0.5, 0.5));
}

#[test]
fn bad_files_raise() {
    assert!(cr::parse_bounds(b"1 2 3").is_err());
    assert!(cr::parse_bounds(b"1 2 x").is_err());
    assert!(cr::from_archive(b"not an archive").is_err());
    // Архив без файла границ и обрезанный архив.
    assert!(cr::from_archive(&as_archive(&[("readme", b"text")])).is_err());
    let whole = as_archive(&[(cr::MEMBER, &as_text(&sample_bounds()))]);
    assert!(cr::from_archive(&whole[..whole.len() / 2]).is_err());
    assert!(Crust::new(vec![0.0; 10], None).is_err());
}

// Настоящая модель.

fn real() -> Option<Crust> {
    let path = std::env::var("PLANETX_CRUST1").ok()?;
    let data = std::fs::read(path).ok()?;
    let model = cr::from_archive(&data);
    assert!(model.is_ok(), "{model:?}");
    model.ok()
}

#[test]
fn ocean_is_thin_and_continent_thick() {
    let Some(model) = real() else { return };
    // Тихий океан, Тибет, Пермский край.
    assert!(model.thickness(0.5, -150.5) < 10.0);
    assert!(model.moho(0.5, -150.5) > 8.0);
    assert!(model.thickness(32.5, 88.5) > 60.0);
    let perm = model.thickness(59.5, 56.5);
    assert!(35.0 < perm && perm < 50.0);
    let layers = model.layers(0.5, -150.5);
    let Some((key, _, bottom)) = layers.first() else { return };
    assert_eq!(*key, "water");
    assert!(-bottom > 3.0);
}

#[test]
fn crust_types() {
    let Some(model) = real() else { return };
    // Тихий и Атлантический океаны, Исландия - океаническая кора,
    // Пермский край, Тибет, шельф Северного моря - континентальная.
    let points = [(0.5, -140.5), (30.5, -40.5), (65.0, -18.0), (58.5, 56.5), (32.5, 88.5), (56.5, 3.5)];
    let got: Vec<Option<bool>> = points.iter().map(|(la, lo)| model.oceanic(*la, *lo)).collect();
    assert_eq!(got, [true, true, true, false, false, false].map(Some));
}
