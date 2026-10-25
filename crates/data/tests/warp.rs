// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Привязка по опорным точкам без GDAL: полиномы, тонкая пластина,
//! выходная сетка и отсчёт.

use data::rumsey::Warp;
use data::warp::{self, Gcp, GcpTransform};

/// Опорные точки на сетке nx × ny пикселей с шагом step по функции f.
fn grid(nx: usize, ny: usize, step: f64, f: impl Fn(f64, f64) -> (f64, f64)) -> Vec<Gcp> {
    let mut out = Vec::new();
    for j in 0..ny {
        for i in 0..nx {
            // Небольшой сдвиг - точки не на правильной сетке.
            let x = i as f64 * step + (j % 3) as f64 * 1.7;
            let y = j as f64 * step + (i % 2) as f64 * 2.3;
            let (lon, lat) = f(x, y);
            out.push((x, y, lon, lat));
        }
    }
    out
}

fn affine(x: f64, y: f64) -> (f64, f64) {
    (30.0 + 0.01 * x + 0.002 * y, 60.0 - 0.001 * x - 0.01 * y)
}

fn quadratic(x: f64, y: f64) -> (f64, f64) {
    (10.0 + 0.01 * x + 1e-5 * x * x + 2e-6 * x * y, 45.0 - 0.01 * y + 3e-6 * y * y - 4e-6 * x * x)
}

fn transform(gcps: &[Gcp], warp: Warp) -> Option<GcpTransform> {
    let t = GcpTransform::new(gcps, warp);
    assert!(t.is_ok(), "{warp:?}: {t:?}");
    t.ok()
}

/// Точки проверки между опорными.
fn probes() -> Vec<(f64, f64)> {
    (0..7).flat_map(|j| (0..7).map(move |i| (13.0 + i as f64 * 61.0, 7.0 + j as f64 * 53.0))).collect()
}

#[test]
fn affine_reproduced() {
    let gcps = grid(4, 4, 120.0, affine);
    for warp in [Warp::Polynomial(1), Warp::Polynomial(2), Warp::Polynomial(3), Warp::Tps] {
        let Some(t) = transform(&gcps, warp) else { return };
        for (x, y) in probes() {
            let (lon, lat) = t.forward(x, y);
            let (elon, elat) = affine(x, y);
            assert!((lon - elon).abs() < 1e-9 && (lat - elat).abs() < 1e-9, "{warp:?} ({x}, {y}): {lon} {lat}");
            // Обратное аффинного - аффинное, подбирается точно.
            let (bx, by) = t.inverse(elon, elat);
            assert!((bx - x).abs() < 1e-6 && (by - y).abs() < 1e-6, "{warp:?} ({x}, {y}): {bx} {by}");
        }
    }
}

#[test]
fn quadratic_reproduced() {
    let gcps = grid(5, 4, 100.0, quadratic);
    for warp in [Warp::Polynomial(2), Warp::Polynomial(3)] {
        let Some(t) = transform(&gcps, warp) else { return };
        for (x, y) in probes() {
            let (lon, lat) = t.forward(x, y);
            let (elon, elat) = quadratic(x, y);
            assert!((lon - elon).abs() < 1e-9 && (lat - elat).abs() < 1e-9, "{warp:?} ({x}, {y}): {lon} {lat}");
        }
    }
    // Первая степень квадратичное искажение не повторяет.
    let Some(t) = transform(&gcps, Warp::Polynomial(1)) else { return };
    let (lon, _) = t.forward(400.0, 300.0);
    assert!((lon - quadratic(400.0, 300.0).0).abs() > 1e-4);
}

/// Слабое искажение, как у бумажной карты: квадратичное и кубическое.
fn mild(x: f64, y: f64) -> (f64, f64) {
    (10.0 + 0.01 * x + 1e-6 * x * x + 3e-7 * x * y + 1e-9 * x * x * y, 45.0 - 0.01 * y + 4e-7 * y * y - 5e-7 * x * x)
}

#[test]
fn round_trip() {
    let gcps = grid(6, 6, 80.0, mild);
    for warp in [Warp::Polynomial(1), Warp::Polynomial(2), Warp::Polynomial(3), Warp::Tps] {
        let Some(t) = transform(&gcps, warp) else { return };
        // Первая степень на квадратичном искажении - больше допуск.
        let tolerance = if warp == Warp::Polynomial(1) { 3.0 } else { 0.5 };
        for (x, y) in probes() {
            let (lon, lat) = t.forward(x, y);
            let (bx, by) = t.inverse(lon, lat);
            assert!((bx - x).hypot(by - y) < tolerance, "{warp:?} ({x}, {y}): {bx} {by}");
        }
    }
    // Тонкая пластина проходит через опорные точки в обе стороны.
    let Some(t) = transform(&gcps, Warp::Tps) else { return };
    for g in &gcps {
        let (lon, lat) = t.forward(g.0, g.1);
        assert!((lon - g.2).abs() < 1e-9 && (lat - g.3).abs() < 1e-9);
        let (x, y) = t.inverse(g.2, g.3);
        assert!((x - g.0).abs() < 1e-6 && (y - g.1).abs() < 1e-6);
    }
}

#[test]
fn too_few_or_collinear() {
    let gcps = grid(4, 4, 100.0, affine);
    for (warp, need) in [(Warp::Polynomial(1), 3), (Warp::Polynomial(2), 6), (Warp::Polynomial(3), 10), (Warp::Tps, 3)] {
        assert!(GcpTransform::new(&gcps[..need - 1], warp).is_err(), "{warp:?}");
        assert!(GcpTransform::new(&gcps[..need], warp).is_ok(), "{warp:?}");
    }
    let line: Vec<Gcp> = (0..8).map(|i| {
        let x = i as f64 * 10.0;
        let (lon, lat) = affine(x, 2.0 * x + 5.0);
        (x, 2.0 * x + 5.0, lon, lat)
    }).collect();
    for warp in [Warp::Polynomial(1), Warp::Polynomial(2), Warp::Tps] {
        assert!(GcpTransform::new(&line, warp).is_err(), "{warp:?}");
    }
    // Совпадающие точки и точки не числа.
    let mut same = gcps.clone();
    same.push(gcps[5]);
    assert!(GcpTransform::new(&same, Warp::Tps).is_err());
    let mut nan = gcps.clone();
    nan[2].2 = f64::NAN;
    assert!(GcpTransform::new(&nan, Warp::Polynomial(1)).is_err());
    assert!(GcpTransform::new(&gcps, Warp::Polynomial(4)).is_err());
}

/// Изображение 40 × 20: цвет по номеру пикселя, блок 5..10 × 4..8
/// прозрачный.
fn image() -> Vec<u8> {
    let mut out = Vec::new();
    for j in 0..20_u8 {
        for i in 0..40_u8 {
            let clear = (5..10).contains(&i) && (4..8).contains(&j);
            out.extend_from_slice(&[i * 6, j * 12, 100, if clear { 0 } else { 255 }]);
        }
    }
    out
}

#[test]
fn warp_affine_image() {
    let src = image();
    // Геопривязка: запад 20, север 50, шаг 0.01 градуса.
    let geo = |x: f64, y: f64| (20.0 + 0.01 * x, 50.0 - 0.01 * y);
    let gcps: Vec<Gcp> = [(0.0, 0.0), (40.0, 0.0), (0.0, 20.0), (40.0, 20.0), (17.0, 9.0)]
        .iter()
        .map(|&(x, y)| {
            let (lon, lat) = geo(x, y);
            (x, y, lon, lat)
        })
        .collect();
    for warp in [Warp::Polynomial(1), Warp::Tps] {
        let out = warp::warp_rgba(&src, 40, 20, &gcps, warp, 3072);
        assert!(out.is_ok(), "{out:?}");
        let Ok(out) = out else { return };
        assert_eq!((out.width, out.height), (40, 20), "{warp:?}");
        for (b, e) in out.bbox.iter().zip([20.0, 49.8, 20.4, 50.0]) {
            assert!((b - e).abs() < 1e-9, "{warp:?} {:?}", out.bbox);
        }
        assert_eq!(out.rgba.len(), 40 * 20 * 4);
        for (i, j) in [(0_usize, 0_usize), (39, 0), (0, 19), (39, 19), (12, 3), (25, 15)] {
            let k = (j * 40 + i) * 4;
            assert_eq!(out.rgba[k..k + 4], src[k..k + 4], "{warp:?} ({i}, {j})");
        }
        // Прозрачный блок остаётся прозрачным.
        let k = (5 * 40 + 7) * 4;
        assert_eq!(out.rgba[k + 3], 0, "{warp:?}");
    }
    // Большая сторона ограничена, рамка та же.
    let out = warp::warp_rgba(&src, 40, 20, &gcps, Warp::Polynomial(1), 10);
    assert!(out.is_ok(), "{out:?}");
    let Ok(out) = out else { return };
    assert_eq!((out.width, out.height), (10, 5));
    for (b, e) in out.bbox.iter().zip([20.0, 49.8, 20.4, 50.0]) {
        assert!((b - e).abs() < 1e-9, "{:?}", out.bbox);
    }
    // Неверный размер данных - ошибка.
    assert!(warp::warp_rgba(&src[4..], 40, 20, &gcps, Warp::Polynomial(1), 100).is_err());
}

#[test]
fn rotated_image_outside_transparent() {
    // Поворот на 30°: углы выходной рамки вне источника - прозрачные.
    let src = image();
    let (c, s) = (30_f64.to_radians().cos(), 30_f64.to_radians().sin());
    let geo = |x: f64, y: f64| (20.0 + 0.01 * (c * x + s * y), 50.0 - 0.01 * (-s * x + c * y));
    let gcps: Vec<Gcp> = [(0.0, 0.0), (40.0, 0.0), (0.0, 20.0), (40.0, 20.0)]
        .iter()
        .map(|&(x, y)| {
            let (lon, lat) = geo(x, y);
            (x, y, lon, lat)
        })
        .collect();
    let out = warp::warp_rgba(&src, 40, 20, &gcps, Warp::Polynomial(1), 3072);
    assert!(out.is_ok(), "{out:?}");
    let Ok(out) = out else { return };
    let (w, h) = (out.width as usize, out.height as usize);
    assert_eq!(out.rgba[3], 0);
    assert_eq!(out.rgba[(w * h - 1) * 4 + 3], 0);
    // Рамка охватывает образы четырёх углов. Запад и север - точно,
    // восток и юг - с округлением до целого количества пикселей, как
    // у gdalwarp.
    let res = (out.bbox[2] - out.bbox[0]) / w as f64;
    assert!(((out.bbox[3] - out.bbox[1]) / h as f64 - res).abs() < 1e-12);
    for g in &gcps {
        assert!(g.2 >= out.bbox[0] - 1e-9 && g.2 <= out.bbox[2] + res / 2.0);
        assert!(g.3 >= out.bbox[1] - res / 2.0 && g.3 <= out.bbox[3] + 1e-9);
    }
    let lons: Vec<f64> = gcps.iter().map(|g| g.2).collect();
    let lats: Vec<f64> = gcps.iter().map(|g| g.3).collect();
    assert!((lons.iter().fold(f64::INFINITY, |a, b| a.min(*b)) - out.bbox[0]).abs() < 1e-9);
    assert!((lats.iter().fold(f64::NEG_INFINITY, |a, b| a.max(*b)) - out.bbox[3]).abs() < 1e-9);
    // Центр результата - образ точки источника, цвет её пикселя.
    let lon = (out.bbox[0] + out.bbox[2]) / 2.0;
    let lat = (out.bbox[1] + out.bbox[3]) / 2.0;
    let i = ((lon - out.bbox[0]) / (out.bbox[2] - out.bbox[0]) * w as f64) as usize;
    let j = ((out.bbox[3] - lat) / (out.bbox[3] - out.bbox[1]) * h as f64) as usize;
    assert_eq!(out.rgba[(j * w + i) * 4 + 3], 255);
}

#[test]
fn sample_premultiplied() {
    // Красный непрозрачный и зелёный прозрачный: на границе - красный
    // с половиной альфы, зелёный не подмешивается.
    let src = [255, 0, 0, 255, 0, 255, 0, 0];
    assert_eq!(warp::sample(&src, 2, 1, 1.0, 0.5), [255, 0, 0, 128]);
    assert_eq!(warp::sample(&src, 2, 1, 0.5, 0.5), [255, 0, 0, 255]);
    assert_eq!(warp::sample(&src, 2, 1, 1.5, 0.5), [0, 0, 0, 0]);
    // У края сосед за краем отбрасывается.
    assert_eq!(warp::sample(&src, 2, 1, 0.1, 0.9), [255, 0, 0, 255]);
    // Вне изображения - прозрачный.
    assert_eq!(warp::sample(&src, 2, 1, -0.1, 0.5), [0; 4]);
    assert_eq!(warp::sample(&src, 2, 1, 2.0, 0.5), [0; 4]);
}

/// Время на изображении 3000 × 3000, запуск:
/// `cargo test --release -p data --test warp -- --ignored --nocapture`.
#[test]
#[ignore]
fn large_image_time() {
    let side = 3000_u32;
    let src = vec![200_u8; (side * side * 4) as usize];
    let gcps = grid(8, 8, 420.0, |x, y| {
        let (lon, lat) = quadratic(x / 10.0, y / 10.0);
        (lon + 1e-7 * x * y, lat)
    });
    for warp in [Warp::Polynomial(3), Warp::Tps] {
        let start = std::time::Instant::now();
        let out = warp::warp_rgba(&src, side, side, &gcps, warp, 3072);
        assert!(out.is_ok(), "{out:?}");
        let Ok(out) = out else { return };
        println!("{warp:?}: {} × {} за {:?}", out.width, out.height, start.elapsed());
    }
}
