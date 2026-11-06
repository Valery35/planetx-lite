// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Свой разбор облаков точек (`data::cloud`, `data::proj`) против terrio:
//! все точки файла, широта, долгота, высота и цвет. Облака - Autzen
//! (Орегон, Ламберт в футах) и Columbia Lake (UTM 11N, COPC и LAZ)
//! в `C:\Dev\pointclouds`, без них сверка пропускается.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use data::cloud::{self, Cloud};
use data::proj::Crs;
use terrio::crs::{SpatialRef, Transformer};
use terrio::pointcloud::{self, PointReadOptions};

/// Облако через terrio, как его читала программа до своего разбора.
fn terrio_cloud(bytes: Vec<u8>, name: &str) -> Cloud {
    let mut reader = pointcloud::open_points_bytes(bytes, name).unwrap();
    let total = reader.header().point_count;
    let pc = reader.read(&PointReadOptions { max_points: Some(u64::MAX), ..Default::default() }).unwrap();
    let wkt = pc.crs.to_wkt().unwrap_or_default().to_lowercase();
    let unit = if wkt.contains("us survey foot") || wkt.contains("foot_us") {
        1200.0 / 3937.0
    } else if wkt.contains("foot") || wkt.contains("feet") {
        0.3048
    } else {
        1.0
    };
    let to = Transformer::new(&pc.crs, &SpatialRef::from_epsg(4326).unwrap()).unwrap();
    let (xs, ys, zs) = (pc.coords("x").unwrap(), pc.coords("y").unwrap(), pc.coords("z").unwrap());
    let r = pc.dim("red").map(|d| d.to_f64());
    let g = pc.dim("green").map(|d| d.to_f64());
    let b = pc.dim("blue").map(|d| d.to_f64());
    let c = pc.dim("classification").map(|d| d.to_f64());
    let top = r.iter().chain(&g).chain(&b).flatten().fold(0.0f64, |m, v| m.max(*v));
    let scale = if top > 255.0 { 257.0 } else { 1.0 };
    let mut out = Cloud { total, ..Default::default() };
    for i in 0..xs.len() {
        let (lon, lat) = to.transform(xs[i], ys[i]).unwrap();
        out.points.push([lat, lon, zs[i] * unit]);
        out.colors.push(match (&r, &g, &b) {
            (Some(r), Some(g), Some(b)) => [r[i], g[i], b[i]].map(|v| (v / scale).clamp(0.0, 255.0) as u8),
            _ => cloud::class_color(c.as_ref().map_or(0, |c| c[i] as u8)),
        });
    }
    out
}

/// Точки с цветом по порядку округлённых широты, долготы (10⁻⁶°, около
/// 10 см) и высоты (1 см): порядок узлов COPC у двух разборов разный.
fn sorted(c: &Cloud) -> Vec<([i64; 3], [f64; 3], [u8; 3])> {
    let mut v: Vec<_> = c
        .points
        .iter()
        .zip(&c.colors)
        .map(|(p, col)| ([(p[0] * 1e6).round() as i64, (p[1] * 1e6).round() as i64, (p[2] * 100.0).round() as i64], *p, *col))
        .collect();
    v.sort_by_key(|a| (a.0, a.2));
    v
}

/// Облако файла своим разбором и через terrio, None - файла нет.
fn both(path: &str) -> Option<(String, Cloud, Cloud)> {
    let Ok(bytes) = std::fs::read(path) else {
        eprintln!("{path} нет, сверка пропущена");
        return None;
    };
    let name = std::path::Path::new(path).file_name().unwrap().to_string_lossy().to_string();
    let theirs = terrio_cloud(bytes.clone(), &name);
    let ours = cloud::read(bytes, &name, u64::MAX).unwrap();
    Some((name, ours, theirs))
}

/// Сверка с terrio. ordered - точки у обоих разборов в порядке файла
/// (LAS и LAZ без дерева), тогда сверка поточечная.
fn compare(path: &str, ordered: bool) {
    let Some((name, ours, theirs)) = both(path) else {
        return;
    };
    assert_eq!(ours.total, theirs.total, "{name}");
    assert_eq!(ours.points.len(), theirs.points.len(), "{name}");
    // Средняя точка не зависит от порядка: систематический сдвиг.
    let mean = |c: &Cloud| c.points.iter().fold([0.0; 3], |m, p| [m[0] + p[0], m[1] + p[1], m[2] + p[2]]).map(|v| v / c.points.len() as f64);
    let (ma, mb) = (mean(&ours), mean(&theirs));
    let shift = [ma[0] - mb[0], ma[1] - mb[1]];
    eprintln!("{name}: сдвиг средней {:.2e}° {:.2e}°, высота {:.4} м", shift[0], shift[1], ma[2] - mb[2]);
    // Сдвиг датума не учитывается - не больше 2 м, 2·10⁻⁵°.
    assert!(shift[0].abs() < 2e-5 && shift[1].abs() < 3e-5, "{name}: {shift:?}");
    // Высота в футах: американский фут против международного - 2·10⁻⁶.
    assert!((ma[2] - mb[2]).abs() < 1e-3 + 1e-5 * mb[2].abs(), "{name}: {ma:?} {mb:?}");
    let mut shifted = theirs.clone();
    for p in &mut shifted.points {
        p[0] += shift[0];
        p[1] += shift[1];
    }
    if ordered {
        let mut worst = [0.0f64; 3];
        for (p, q) in ours.points.iter().zip(&shifted.points) {
            worst[0] = worst[0].max((p[0] - q[0]).abs());
            worst[1] = worst[1].max((p[1] - q[1]).abs() * p[0].to_radians().cos());
            worst[2] = worst[2].max((p[2] - q[2]).abs());
        }
        eprintln!("{name}: {} точек по порядку, после сдвига расхождение {:.2e}° {:.2e}° {:.4} м", ours.points.len(), worst[0], worst[1], worst[2]);
        // 10⁻⁷° - около 1 см: остаток сдвига датума внутри плитки.
        assert!(worst[0] < 1e-7 && worst[1] < 1e-7 && worst[2] < 1e-3, "{name}: {worst:?}");
        assert_eq!(ours.colors, theirs.colors, "{name}");
        return;
    }
    same_points(&name, &ours, &shifted);
}

/// Одни и те же точки в разном порядке: пары по клеткам 10⁻⁶°.
fn same_points(name: &str, ours: &Cloud, shifted: &Cloud) {
    let (a, b) = (sorted(ours), sorted(shifted));
    let mut worst = [0.0f64; 3];
    let (mut colors, mut apart) = (0usize, 0usize);
    for (p, q) in a.iter().zip(&b) {
        // Точка у границы округления встаёт в паре с соседней.
        if p.0 != q.0 {
            apart += 1;
            continue;
        }
        // В паре - точки одной клетки 10⁻⁶°, не обязательно одна точка:
        // поточечный допуск - размер клетки.
        worst[0] = worst[0].max((p.1[0] - q.1[0]).abs());
        worst[1] = worst[1].max((p.1[1] - q.1[1]).abs() * p.1[0].to_radians().cos());
        worst[2] = worst[2].max((p.1[2] - q.1[2]).abs());
        colors += usize::from(p.2 != q.2);
    }
    eprintln!("{name}: {} точек, расхождение {:.2e}° {:.2e}° {:.4} м, не в паре {apart}, цвет разный у {colors}", a.len(), worst[0], worst[1], worst[2]);
    assert!(worst[0] < 2e-6 && worst[1] < 2e-6, "{name}: {worst:?}");
    assert!(worst[2] < 0.02, "{name}: {worst:?}");
    assert!(apart * 100 < a.len(), "{name}: не в паре {apart}");
    // Точки с одинаковыми координатами могут встать в другом порядке.
    assert!(colors * 1000 < a.len(), "{name}: цвет разный у {colors}");
}

#[test]
fn autzen_matches_terrio() {
    compare(r"C:\Dev\pointclouds\autzen\autzen-classified.copc.laz", false);
}

#[test]
fn columbia_laz_matches_terrio() {
    compare(r"C:\Dev\pointclouds\columbia_lake\SRMT_final_580000_5568000.laz", true);
}

/// COPC и LAZ одной плитки - одни и те же точки своим разбором.
#[test]
fn columbia_copc_matches_its_laz() {
    let dir = r"C:\Dev\pointclouds\columbia_lake\";
    let (Ok(copc), Ok(laz)) = (std::fs::read(format!("{dir}SRMT_final_580000_5568000.copc.laz")), std::fs::read(format!("{dir}SRMT_final_580000_5568000.laz"))) else {
        eprintln!("облака Columbia Lake нет, сверка пропущена");
        return;
    };
    let a = cloud::read(copc, "a.copc.laz", u64::MAX).unwrap();
    let b = cloud::read(laz, "a.laz", u64::MAX).unwrap();
    assert_eq!((a.total, a.points.len(), a.epsg), (b.total, b.points.len(), b.epsg));
    same_points("Columbia COPC и LAZ", &a, &b);
}

/// Системы по коду EPSG для старых файлов LAS с ключами GeoTIFF.
#[test]
fn epsg_codes_match_terrio() {
    let wgs = SpatialRef::from_epsg(4326).unwrap();
    for code in [32631, 32760, 26910, 3157, 2955, 2962, 3761, 6328, 6340, 25832, 28355, 7855, 2154] {
        let crs = data::proj::from_epsg(code).unwrap_or_else(|| panic!("EPSG:{code}"));
        let to = Transformer::new(&SpatialRef::from_epsg(code).unwrap(), &wgs).unwrap();
        let (x, y) = if code == 2154 { (650_000.0, 6_700_000.0) } else if [32760, 28355, 7855].contains(&code) { (450_000.0, 6_200_000.0) } else { (550_000.0, 5_500_000.0) };
        let (lon, lat) = to.transform(x, y).unwrap();
        let (la, lo) = crs.to_lat_lon(x, y).unwrap();
        // Сдвиг датума не учитывается: у этих систем он меньше 2 м.
        assert!((la - lat).abs() < 2e-5 && (lo - lon).abs() < 3e-5, "EPSG:{code}: {la}, {lo} против {lat}, {lon}");
    }
}

/// Строки WKT систем облаков: пересчёт против terrio в точках у края
/// и в середине области.
#[test]
fn projections_match_terrio() {
    let cases: [(u32, [(f64, f64); 3]); 6] = [
        // UTM 31N, Гренобль.
        (32631, [(714_000.0, 5_008_000.0), (500_000.0, 0.0), (300_000.0, 7_000_000.0)]),
        // UTM 33S.
        (32733, [(400_000.0, 7_500_000.0), (700_000.0, 9_900_000.0), (500_000.0, 6_000_000.0)]),
        // Lambert-93.
        (2154, [(700_000.0, 6_600_000.0), (900_000.0, 6_450_000.0), (200_000.0, 6_900_000.0)]),
        // Орегон Ламберт, международные футы (Autzen).
        (2992, [(4_200_000.0, 600_000.0), (7_000_000.0, 1_000_000.0), (1_000_000.0, 200_000.0)]),
        // CONUS Альберс USGS.
        (5070, [(0.0, 0.0), (-2_000_000.0, 2_500_000.0), (2_000_000.0, 500_000.0)]),
        // Гаусс-Крюгер, Пулково 1942, зона 10.
        (28410, [(10_500_000.0, 6_600_000.0), (10_300_000.0, 5_500_000.0), (10_700_000.0, 7_500_000.0)]),
    ];
    let wgs = SpatialRef::from_epsg(4326).unwrap();
    for (code, points) in cases {
        let sr = SpatialRef::from_epsg(code).unwrap();
        let to = Transformer::new(&sr, &wgs).unwrap();
        for wkt in [sr.to_wkt().unwrap()] {
            let crs = Crs::from_wkt(&wkt).unwrap_or_else(|e| panic!("EPSG:{code}: {e}\n{wkt}"));
            assert_eq!(crs.epsg, Some(code), "{wkt}");
            for (x, y) in points {
                let (lon, lat) = to.transform(x, y).unwrap();
                let (la, lo) = crs.to_lat_lon(x, y).unwrap();
                // Без сдвига датума: Пулково 1942 расходится с WGS84 на
                // сотню метров, сверка - с той же системой без сдвига.
                let tol = if code == 28410 { 3e-3 } else { 1e-8 };
                assert!((la - lat).abs() < tol && (lo - lon).abs() < tol, "EPSG:{code} ({x}, {y}): {la}, {lo} против {lat}, {lon}");
            }
        }
    }
}
