// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Плиты Slab2 на разрезах: файл зоны, рамки, полоса на гранях. Перенос
//! `tests/test_slabs.py` модуля.
//!
//! Файлы зон лежат в хранилище planetx-terrain. Проверка на них идёт,
//! если переменная PLANETX_TERRAIN_DIR указывает на его локальную копию.

use std::collections::BTreeMap;
use std::io::Write;

use data::cutaway as cw;
use data::slabs::{self as sl, Frame, Slab};

const INDEX: &str = r"C:\Dev\planetx\planetx\data\slab2_index.json";

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

/// Файл .npy версии 1.0.
fn npy(descr: &str, shape: &str, body: &[u8]) -> Vec<u8> {
    let mut header = format!("{{'descr': '{descr}', 'fortran_order': False, 'shape': {shape}, }}");
    while (10 + header.len() + 1) % 64 != 0 {
        header.push(' ');
    }
    header.push('\n');
    let mut out = b"\x93NUMPY\x01\x00".to_vec();
    out.extend_from_slice(&(header.len() as u16).to_le_bytes());
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(body);
    out
}

fn npz(members: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in members {
        let started = writer.start_file(format!("{name}.npy"), zip::write::SimpleFileOptions::default());
        assert!(started.is_ok());
        assert!(writer.write_all(bytes).is_ok());
    }
    writer.finish().map(|c| c.into_inner()).unwrap_or_default()
}

/// Зона 3 × 4 узла у линии перемены дат, одна клетка пустая.
fn sample_bytes() -> Vec<u8> {
    let depth: [i16; 12] = [100, 200, 300, -32768, 150, 250, 350, 450, 200, 300, 400, 500];
    let i2 = |values: &[i16]| values.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
    let origin: Vec<u8> = [179.9f64, 10.0, 0.1].iter().flat_map(|v| v.to_le_bytes()).collect();
    npz(&[
        ("origin", npy("<f8", "(3,)", &origin)),
        ("depth", npy("<i2", "(3, 4)", &i2(&depth))),
        ("thickness", npy("<i2", "(3, 4)", &i2(&[300; 12]))),
        ("dip", npy("|u1", "(3, 4)", &[60; 12])),
    ])
}

fn sample() -> Option<Slab> {
    let slab = sl::load(&sample_bytes(), "tst");
    assert!(slab.is_ok(), "{slab:?}");
    slab.ok()
}

#[test]
fn load_decodes_depth() {
    let Some(slab) = sample() else { return };
    assert_eq!(slab.code, "tst");
    assert_eq!((slab.rows, slab.cols), (3, 4));
    assert_eq!(slab.depth[0], 10.0);
    assert_eq!(slab.thickness[4 + 1], 30.0);
    assert!(slab.depth[3].is_nan());
    assert_eq!(slab.dip[0], 60.0);
    assert!(sl::load(b"not npz", "bad").is_err());
    // Сетки разного размера и рамка не из трёх чисел - ошибка.
    let bad = npz(&[
        ("origin", npy("<f8", "(1,)", &1.0f64.to_le_bytes())),
        ("depth", npy("<i2", "(1, 1)", &[0, 0])),
        ("thickness", npy("<i2", "(1, 1)", &[0, 0])),
        ("dip", npy("|u1", "(1, 1)", &[0])),
    ]);
    assert!(sl::load(&bad, "bad").is_err());
}

#[test]
fn depth_wraps_dateline() {
    let Some(slab) = sample() else { return };
    assert_eq!(sl::depth_at(&slab, 9.9, -179.9), 35.0);
    assert!(sl::depth_at(&slab, 0.0, 0.0).is_nan());
}

#[test]
fn band_is_apparent_thickness() {
    let Some(slab) = sample() else { return };
    let (top, bottom) = sl::band([&slab], &[9.9, 0.0], &[-179.9, 0.0]);
    assert_eq!(top[0], 35.0);
    // Толщина 30 км при падении 60° - 60 км по вертикали.
    close(bottom[0] - top[0], 60.0, 1e-6);
    assert!(top[1].is_nan());
}

#[test]
fn box_across_dateline() {
    let frame = Frame { south: 40.0, north: 60.0, west: 170.0, east: -170.0 };
    let got: Vec<bool> = [(50.0, 175.0), (50.0, -175.0), (50.0, 0.0), (30.0, 175.0)].iter().map(|(la, lo)| sl::in_box(&frame, *la, *lo)).collect();
    assert_eq!(got, [true, true, false, false]);
    let index = BTreeMap::from([("a".to_string(), frame), ("b".to_string(), Frame { south: 0.0, north: 10.0, west: 0.0, east: 10.0 })]);
    assert_eq!(sl::touched(&index, &[50.0], &[179.0]), ["a"]);
}

#[test]
fn bands_on_faces() {
    let Some(slab) = sample() else { return };
    // Западная грань - по 180°, через зону.
    let wedge = cw::make_wedge(-180.0, -90.0, 0.0, 90.0);
    let mesh = cw::slab_bands(&wedge, &[&slab], 1.0, cw::SLAB_STEP, None);
    assert!(mesh.is_some());
    let Some(mesh) = mesh else { return };
    assert!(mesh.vertices.iter().all(|v| v.color[..3] == cw::SLAB_COLOR));
    // Без плиты на дугах сетки нет.
    let far = cw::make_wedge(-45.0, 45.0, 0.0, 90.0);
    assert!(cw::slab_bands(&far, &[&slab], 1.0, cw::SLAB_STEP, None).is_none());
}

#[test]
fn band_does_not_wedge_to_surface() {
    // Плита в первых двух точках дуги, дальше её нет (доля 1.0 -
    // поверхность). Ячейка от конца плиты к точке без плиты шла клином
    // до поверхности, 4 октября 2026 года на демо модуля «Японский жёлоб».
    let edge = [[1.0, 0.0, 0.0], [0.99, 0.14, 0.0], [0.96, 0.28, 0.0], [0.92, 0.4, 0.0]];
    let low = [0.90, 0.91, 1.0, 1.0];
    let high = [0.95, 0.96, 1.0, 1.0];
    let part = cw::band(&edge, [0.0; 3], &low, &high, &[cw::SLAB_COLOR], true);
    assert!(part.is_some());
    let Some(part) = part else { return };
    let mut used: Vec<u32> = part.indices.iter().flatten().copied().collect();
    used.sort_unstable();
    used.dedup();
    // Задействованы только точки 0 и 1 и их пары верхнего края.
    assert_eq!(used, [0, 1, 4, 5]);
    // Слои коры выклиниваются, у них краевая ячейка остаётся.
    let part = cw::band(&edge, [0.0; 3], &low, &high, &[cw::SLAB_COLOR], false);
    assert!(part.is_some_and(|p| p.indices.iter().flatten().any(|i| *i == 2)));
}

#[test]
fn index_in_module() {
    let Ok(bytes) = std::fs::read(INDEX) else { return };
    let index = sl::read_index(&bytes);
    assert!(index.is_ok(), "{index:?}");
    let Ok(index) = index else { return };
    assert_eq!(index.len(), 27);
    // Разрез по 143.5° в. д. касается Курильской и Японской зоны.
    let lats: Vec<f64> = (0..181).map(|i| i as f64 * 0.5).collect();
    let codes = sl::touched(&index, &lats, &vec![143.5; lats.len()]);
    assert!(codes.iter().any(|c| c == "kur"));
    assert!(!codes.iter().any(|c| c == "sam"));
    assert!(sl::read_index(b"[1, 2]").is_err());
    assert!(sl::read_index(br#"{"a": {"south": 1}}"#).is_err());
}

#[test]
fn urls() {
    assert_eq!(sl::url_of("kur", None), "https://raw.githubusercontent.com/Valery35/planetx-terrain/main/slab2/kur.npz");
    assert_eq!(sl::url_of("kur", Some("file:///d/{code}.npz")), "file:///d/kur.npz");
    assert_eq!(format!("{}{}", sl::TERRAIN_REPOS[1], sl::PATH.replace("{code}", "kur")), "https://cdn.jsdelivr.net/gh/Valery35/planetx-terrain@main/slab2/kur.npz");
}

#[test]
fn zone_files() {
    let Ok(dir) = std::env::var("PLANETX_TERRAIN_DIR") else { return };
    let Ok(bytes) = std::fs::read(INDEX) else { return };
    let Ok(index) = sl::read_index(&bytes) else { return };
    let read = |code: &str| {
        let path = std::path::Path::new(&dir).join("slab2").join(format!("{code}.npz"));
        let slab = std::fs::read(path).map_err(|e| sl::SlabError(e.to_string())).and_then(|b| sl::load(&b, code));
        assert!(slab.is_ok(), "{code}: {slab:?}");
        slab.ok()
    };
    for code in index.keys() {
        let Some(slab) = read(code) else { return };
        assert!(slab.depth.iter().any(|d| d.is_finite()), "{code}");
    }
    let Some(kur) = read("kur") else { return };
    // Плита под Хоккайдо на 80-160 км, на западе глубже 600 км.
    let hokkaido = sl::depth_at(&kur, 43.0, 143.0);
    assert!(80.0 < hokkaido && hokkaido < 160.0);
    assert!(kur.depth.iter().copied().filter(|d| d.is_finite()).fold(f64::MIN, f64::max) > 600.0);
}
