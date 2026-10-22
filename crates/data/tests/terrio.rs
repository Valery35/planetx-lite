// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сверка terrio 0.4.1 на крейте Rust с GDAL 3.13 из QGIS 4.0.3 на данных
//! демо модуля, 23 октября 2026 года. Эталон - `gdal.Open`, `ogr.Open`
//! и `osr.CoordinateTransformation` на тех же файлах, порядок осей
//! «долгота, широта».
//!
//! Папка демо - `PLANETX_DEMO` или `C:\Dev\planetx\planetx\demo`. Без неё
//! тест сообщает об этом и проходит: на машине CI модуля нет.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use data::grid::{self, Grid};
use data::layers::{self, Shape};

fn demo(name: &str) -> Option<Vec<u8>> {
    let dir = std::env::var_os("PLANETX_DEMO").map_or_else(|| PathBuf::from(r"C:\Dev\planetx\planetx\demo"), PathBuf::from);
    let bytes = std::fs::read(dir.join(name)).ok();
    if bytes.is_none() {
        eprintln!("файла демо {name} нет, проверка пропущена");
    }
    bytes
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// Растр: размер, привязка, EPSG, статистика, точки - как у GDAL.
fn check(name: &str, size: (usize, usize), gt: [f64; 6], epsg: u32, stats: (f64, f64, f64), samples: &[(usize, usize, Option<f32>)]) {
    let Some(bytes) = demo(name) else {
        return;
    };
    let file = name.rsplit('/').next().unwrap();
    let g = Grid::read(bytes, file).unwrap();
    assert_eq!((g.width, g.height), size, "{name}");
    for (a, b) in g.transform.iter().zip(gt) {
        assert!(close(*a, b, 1e-6), "{name}: {:?} против {gt:?}", g.transform);
    }
    assert_eq!(g.epsg, Some(epsg), "{name}");
    let (lo, hi, mean) = g.stats().unwrap();
    // Как в таблице «Проверки terrio»: до 4-го знака.
    assert!(close(lo, stats.0, 1e-4) && close(hi, stats.1, 1e-4) && close(mean, stats.2, 1e-4), "{name}: {lo} {hi} {mean}");
    for (col, row, v) in samples {
        assert_eq!(g.value(*col, *row), *v, "{name} ({col}, {row})");
    }
}

#[test]
fn quarry_lerc_dem_like_gdal() {
    check(
        "quarry/quarry_dem.tif",
        (2400, 2400),
        [497100.9822753188, 1.0, 0.0, 6595816.25184939, 0.0, -1.0],
        32640,
        (80.98999786376953, 273.8800048828125, 208.7108519989502),
        &[(0, 0, Some(228.86)), (1200, 1200, Some(80.99)), (2399, 2399, Some(204.76)), (800, 600, Some(256.26))],
    );
}

#[test]
fn brienz_bed_like_gdal() {
    check(
        "brienz/brienzersee_bed.tif",
        (1400, 807),
        [2632872.0, 10.0, 0.0, 1178863.0, 0.0, -10.0],
        2056,
        (307.880126953125, 742.5406494140625, 413.0336451750738),
        &[(0, 0, None), (700, 403, Some(308.448_82)), (1399, 806, None), (466, 201, None)],
    );
}

#[test]
fn roof_like_gdal() {
    check(
        "subsurface/roof_01.tif",
        (81, 61),
        [491175.0, 50.0, 0.0, 6591675.0, 0.0, -50.0],
        32640,
        (131.53683471679688, 156.7684326171875, 149.64735947473665),
        &[(0, 0, Some(151.633_04)), (40, 30, Some(152.533_14)), (80, 60, Some(147.202_64)), (27, 15, Some(151.871_48))],
    );
}

#[test]
fn coordinates_like_gdal() {
    // (EPSG, x, y) -> (долгота, широта) GDAL.
    let cases = [
        (32640, 500000.0, 6500000.0, 57.000000000000014, 58.64029694025095),
        (2056, 2640000.0, 1175000.0, 7.961881765100159, 46.725003271283086),
        (28410, 10500000.0, 6500000.0, 56.99842553691176, 58.616328726928735),
        (3857, 6260000.0, 7950000.0, 56.23453678588204, 57.91746759255054),
    ];
    for (code, x, y, lon, lat) in cases {
        let (la, lo) = grid::lat_lon_of(code, x, y).unwrap();
        // Как в таблице: до 1e-12°. Пулково-42 - через параметры датума,
        // допуск 1e-7° (около 1 см).
        let tol = if code == 28410 { 1e-7 } else { 1e-9 };
        assert!(close(la, lat, tol) && close(lo, lon, tol), "EPSG:{code}: {la} {lo} против {lat} {lon}");
    }
}

#[test]
fn geopackages_like_gdal() {
    if let Some(bytes) = demo("haul/haul_kachkanar.gpkg") {
        let l = layers::read(bytes, "haul_kachkanar.gpkg").unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!((l[0].name.as_str(), l[0].items.len(), l[0].epsg), ("trucks", 5406, Some(4326)));
        assert_eq!(l[0].fields, ["truck", "time", "loaded"]);
        assert_eq!(l[0].items[0].shape, Shape::Point);
        let (lat, lon) = l[0].items[0].parts[0][0];
        assert!(close(lat, 58.7520433458308, 1e-12) && close(lon, 59.5043198661564, 1e-12));
    }
    if let Some(bytes) = demo("vegas/vegas_loop.gpkg") {
        let l = layers::read(bytes, "vegas_loop.gpkg").unwrap();
        assert_eq!((l[0].name.as_str(), l[0].items.len()), ("tunnels", 50));
        assert_eq!(l[0].items[0].shape, Shape::Line);
        let (lat, lon) = l[0].items[0].parts[0][0];
        assert!(close(lat, 36.1286484, 1e-9) && close(lon, -115.146274, 1e-9));
    }
    if let Some(bytes) = demo("subsurface/perm_subsurface.gpkg") {
        let l = layers::read(bytes, "perm_subsurface.gpkg").unwrap();
        let summary: Vec<(&str, usize)> = l.iter().map(|x| (x.name.as_str(), x.items.len())).collect();
        for expected in [("collar", 24), ("sections", 2), ("cut", 1), ("images", 1), ("interval", 240), ("survey", 236), ("beds", 10)] {
            assert!(summary.contains(&expected), "{summary:?} без {expected:?}");
        }
        let cut = l.iter().find(|x| x.name == "cut").unwrap();
        assert_eq!(cut.items[0].shape, Shape::Polygon);
        assert_eq!(cut.epsg, Some(32640));
        // Угол многоугольника (493200, 6590150) UTM 40N - в WGS84.
        let (lat, lon) = cut.items[0].parts[0][0];
        let (la, lo) = grid::lat_lon_of(32640, 493200.0, 6590150.0).unwrap();
        assert!(close(lat, la, 1e-12) && close(lon, lo, 1e-12));
    }
}

#[test]
fn quarry_dem_samples_tiles_like_the_raster() {
    // Тайл уровня 17 над серединой «Карьера»: высоты выборки совпадают
    // с растром в той же точке, в середине вес 1.
    let Some(bytes) = demo("quarry/quarry_dem.tif") else {
        return;
    };
    let dem = data::dem::Dem::new(Grid::read(bytes, "quarry_dem.tif").unwrap(), 0.05).unwrap();
    let (w, s, e, n) = dem.bounds;
    assert!(w < e && s < n);
    // Точка растра 1 м в UTM 40N на 58.6° - около 1.9 м EPSG:3857.
    assert!((dem.pixel - 1.92).abs() < 0.05, "{}", dem.pixel);
    let (cx, cy) = ((w + e) / 2.0, (s + n) / 2.0);
    let half = 2.0 * 20037508.342789244 / f64::from(1u32 << 17) / 2.0;
    let (heights, weights) = dem.sample((cx - half, cy - half, cx + half, cy + half), 256);
    assert_eq!(heights.len(), 256 * 256);
    assert!(heights.iter().all(|h| h.is_finite()));
    assert!(weights.iter().all(|w| (*w - 1.0).abs() < 1e-6));
    // Середина тайла - точка растра под (cx, cy).
    let g = dem.grid();
    let t = g.transformer_from(3857).unwrap();
    let (x, y) = t.transform(cx + half / 256.0, cy - half / 256.0).unwrap();
    let (col, row) = g.pixel_of(x, y).unwrap();
    let exact = g.bilinear(col, row);
    let got = heights[128 * 256 + 128];
    assert!((got - exact).abs() < 0.05, "{got} против {exact}");
}

#[test]
fn hostile_bytes_are_an_error() {
    assert!(Grid::read(b"not a tiff".to_vec(), "x.tif").is_err());
    assert!(layers::read(b"not a geopackage".to_vec(), "x.gpkg").is_err());
}
