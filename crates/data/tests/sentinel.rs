// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Космоснимки: каталог STAC, участок, облачность, продукты, Landsat,
//! MSS, ASTER и MODIS. Перенос `tests/test_sentinel.py` модуля.
//!
//! Проверка типов чисел Python (`assertIs(type(cloud), float)`) не
//! переносится: в Rust доля облачности - всегда `f64`.

// panic! - только в ветке else у let-else, как провал проверки.
#![allow(clippy::panic)]

use std::collections::BTreeMap;

use data::sentinel::{self as s2, Asset, Assets, Bands, Grid, Href, Mission, Product, Scene};
use serde_json::{Value, json};

const BASE: &str = "https://sentinel-cogs.s3.us-west-2.amazonaws.com/sentinel-s2-l2a-\
                    cogs/40/V/DK/2026/9/S2C_40VDK_20260929_0_L2A/";

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

/// Сетка из строк.
fn grid<T: Copy>(rows: &[&[T]]) -> Grid<T> {
    let list: Vec<Vec<T>> = rows.iter().map(|r| r.to_vec()).collect();
    let Some(g) = Grid::from_rows(&list) else {
        panic!("неровные строки");
    };
    g
}

fn at<T: Copy>(g: &Grid<T>, r: usize, c: usize) -> T {
    let Some(v) = g.get(r, c) else {
        panic!("нет пикселя {r} {c}");
    };
    v
}

fn single(p: Option<Product>) -> Grid<f32> {
    let Some(Product::Single(g)) = p else {
        panic!("не один канал");
    };
    g
}

fn stack(p: Option<Product>) -> Vec<Grid<f32>> {
    let Some(Product::Stack(g)) = p else {
        panic!("не набор каналов");
    };
    g
}

fn parsed(feature: &Value) -> Scene {
    let Some(s) = s2::parse_item(feature) else {
        panic!("сцена не разобрана");
    };
    s
}

fn asset<'a>(s: &'a Scene, key: &str) -> &'a Asset {
    let Some(a) = s.assets.get(key) else {
        panic!("нет канала {key}");
    };
    a
}

fn item_on(day: &str, cloud: f64) -> Value {
    let band = json!({"scale": 0.0001, "offset": -0.1, "nodata": 0,
                      "spatial_resolution": 10});
    json!({
        "id": format!("S2C_40VDK_{}_0_L2A", day.replace('-', "")),
        "properties": {"datetime": format!("{day}T07:43:29.961000Z"),
                       "eo:cloud_cover": cloud, "proj:epsg": 32640,
                       "view:sun_elevation": 29.2,
                       "mgrs:utm_zone": 40, "mgrs:latitude_band": "V",
                       "mgrs:grid_square": "DK"},
        "assets": {
            "red": {"href": format!("{BASE}B04.tif"), "raster:bands": [band]},
            "nir": {"href": format!("{BASE}B08.tif"), "raster:bands": [band]},
            "scl": {"href": format!("{BASE}SCL.tif"),
                    "raster:bands": [{"nodata": 0}]},
            "thumbnail": {"href": format!("{BASE}preview.jpg")},
            "cloud": {"href": "s3://sentinel-s2-l2a/qi/CLD_20m.jp2"}}})
}

fn item() -> Value {
    item_on("2026-09-29", 12.5)
}

fn set_asset(feature: &mut Value, key: &str, value: Value) {
    let Some(assets) = feature.get_mut("assets").and_then(Value::as_object_mut) else {
        panic!("нет assets");
    };
    assets.insert(key.to_string(), value);
}

fn bands_of(list: &[(&str, Grid<f32>)]) -> Bands {
    list.iter()
        .map(|(k, g)| (k.to_string(), g.clone()))
        .collect()
}

// TestCatalog

#[test]
fn search_body() {
    let body = s2::search_body(
        &s2::geojson(&s2::square(58.0, 56.25, 2000.0)),
        "2025-10-01",
        "2026-09-30",
        80.0,
        s2::PAGE,
        Mission::Sentinel2,
    );
    assert_eq!(body["collections"], json!(["sentinel-2-l2a"]));
    assert_eq!(
        body["datetime"],
        json!("2025-10-01T00:00:00Z/2026-09-30T23:59:59Z")
    );
    assert_eq!(body["query"]["eo:cloud_cover"]["lte"], json!(80.0));
    assert_eq!(body["limit"], json!(100));
    assert_eq!(
        body["sortby"],
        json!([{"field": "properties.datetime", "direction": "desc"}])
    );
    let Some(ring) = body["intersects"]["coordinates"][0].as_array() else {
        panic!("нет кольца");
    };
    assert_eq!(ring.first(), ring.last());
    assert_eq!(ring.len(), 5);
}

#[test]
fn parse_page_and_next() {
    let data = json!({"features": [item(), {"id": "x", "properties": {}, "assets": {}}],
                      "links": [{"rel": "next", "body": {"next": "token"}}]});
    let (scenes, following) = s2::parse_page(&data);
    assert_eq!(scenes.len(), 1);
    let s = &scenes[0];
    assert_eq!(
        (s.when.as_str(), s.cloud, s.tile.as_str(), s.epsg),
        ("2026-09-29 07:43", 12.5, "40VDK", Some(32640))
    );
    assert_eq!(asset(s, "red").offset, -0.1);
    // Смещение уже вычтено из значений - второй раз не вычитается.
    let mut applied = item();
    applied["properties"]["earthsearch:boa_offset_applied"] = json!(true);
    assert_eq!(asset(&parsed(&applied), "red").offset, 0.0);
    assert!(!s.assets.contains("cloud")); // адрес s3 не берётся
    assert_eq!(following, Some(json!({"next": "token"})));
    assert_eq!(s2::parse_page(&json!({"features": []})).1, None);
}

#[test]
fn hostile_catalog_does_not_break() {
    for data in [
        json!(null),
        json!([1, 2]),
        json!({"features": "x", "links": 5}),
        json!({"features": [null, 1, {"assets": [1]}, {"properties": 7, "assets": {"red": 3}}]}),
    ] {
        let (scenes, following) = s2::parse_page(&data);
        assert!(scenes.is_empty());
        assert!(following.is_none());
    }
    assert!(s2::cmr_scenes(&json!({"items": [null, {"umm": 5}]})).is_empty());
}

// TestArea

#[test]
fn square_side_in_metres() {
    let ring = s2::square(58.0, 56.25, 2000.0);
    let Ok(xy) = s2::ring_utm(&ring, 32640) else {
        panic!("вне UTM");
    };
    close(xy[1].0 - xy[0].0, 2000.0, 15.0);
    close(xy[3].1 - xy[0].1, 2000.0, 15.0);
    let Ok(centre) = s2::ring_utm(&[(58.0, 56.25)], 32640) else {
        panic!("вне UTM");
    };
    let n = xy.len() as f64;
    let middle = (
        xy.iter().map(|p| p.0).sum::<f64>() / n,
        xy.iter().map(|p| p.1).sum::<f64>() / n,
    );
    close(middle.0, centre[0].0, 5.0);
    close(middle.1, centre[0].1, 5.0);
}

#[test]
fn utm_matches_qgis() {
    // Эталоны QGIS из tests/coords.rs крейта scene.
    let Ok(north) = s2::ring_utm(&[(58.0105, 56.2294)], 32640) else {
        panic!("вне UTM");
    };
    close(north[0].0, 454464.3794, 0.001);
    close(north[0].1, 6430138.879, 0.001);
    let Ok(south) = s2::ring_utm(&[(-33.8688, 151.2093)], 32756) else {
        panic!("вне UTM");
    };
    close(south[0].0, 334368.6336, 0.001);
    close(south[0].1, 6250948.3454, 0.001);
    // Система сцены одна: точка к северу от экватора в южной системе.
    let Ok(cross) = s2::ring_utm(&[(0.001, 56.0)], 32740) else {
        panic!("вне UTM");
    };
    assert!(cross[0].1 > 10_000_000.0);
    assert!(s2::ring_utm(&[(85.0, 56.0)], 32640).is_err());
    assert!(s2::ring_utm(&[(58.0, 56.0)], 4326).is_err());
}

#[test]
fn mask_of_triangle() {
    let xy = [(0.0, 0.0), (100.0, 0.0), (0.0, 100.0)];
    let Some(bx) = s2::bounds(&xy, 10.0) else {
        panic!("нет рамки");
    };
    let Some(inside) = s2::mask(&xy, bx, 10.0) else {
        panic!("нет маски");
    };
    assert_eq!(inside.shape(), (10, 10));
    // Треугольник - половина квадрата, по центрам пикселей 45 из 100.
    assert_eq!(inside.count(), 45);
    assert!(at(&inside, 9, 0)); // юго-запад внутри
    assert!(!at(&inside, 0, 9)); // северо-восток снаружи
    assert!(s2::mask(&xy, bx, f64::NAN).is_none());
    assert!(s2::mask(&xy, [0.0, 0.0, 1e12, 1e12], 1.0).is_none());
}

#[test]
fn step_grows_with_area() {
    let m = Mission::Sentinel2;
    assert_eq!(
        s2::step_for(&[(0.0, 0.0), (30000.0, 30000.0)], m, &[]),
        10.0
    );
    assert_eq!(s2::step_for(&[(0.0, 0.0), (60000.0, 1.0)], m, &[]), 20.0);
    assert_eq!(s2::step_for(&[(0.0, 0.0), (200000.0, 1.0)], m, &[]), 60.0);
}

// TestCloud

#[test]
fn cloud_share_over_area() {
    let mut scl = Grid::filled(10, 10, 4.0); // растительность
    for r in 0..10 {
        for c in 0..3 {
            scl.set(r, c, 9.0); // облака высокой вероятности
        }
        scl.set(r, 9, 0.0); // нет данных
    }
    let inside = Grid::filled(10, 10, true);
    let Some((cloud, valid)) = s2::cloud_share(&scl, &inside) else {
        panic!("размеры");
    };
    close(cloud, 100.0 * 30.0 / 90.0, 1e-7);
    close(valid, 90.0, 1e-7);
    // Участок справа от облаков - чистый.
    let mut right = Grid::filled(10, 10, false);
    for r in 0..10 {
        for c in 4..9 {
            right.set(r, c, true);
        }
    }
    assert_eq!(s2::cloud_share(&scl, &right), Some((0.0, 100.0)));
    assert_eq!(s2::cloud_share(&scl, &Grid::filled(2, 2, true)), None);
}

// TestProducts

#[test]
fn reflectance_offset_and_nodata() {
    let a = Asset::new("x", 0.0001, -0.1, Some(0.0), Some(10.0));
    let out = s2::reflectance(&grid(&[&[0.0, 1000.0, 3000.0]]), &a);
    assert!(at(&out, 0, 0).is_nan());
    close(f64::from(at(&out, 0, 1)), 0.0, 1e-6);
    close(f64::from(at(&out, 0, 2)), 0.2, 1e-6);
}

#[test]
fn ndvi_and_composite() {
    let bands = bands_of(&[
        ("nir", grid(&[&[0.5f32, 0.3]])),
        ("red", grid(&[&[0.1f32, 0.3]])),
        ("green", grid(&[&[0.2f32, 0.2]])),
    ]);
    let ndvi = single(s2::product("ndvi", &bands, Mission::Sentinel2));
    close(f64::from(at(&ndvi, 0, 0)), 0.4 / 0.6, 1e-5);
    close(f64::from(at(&ndvi, 0, 1)), 0.0, 1e-6);
    // Малая или отрицательная сумма - не число, а не -700.
    let Some(odd) =
        s2::normalized_difference(&grid(&[&[0.05f32, 0.0]]), &grid(&[&[-0.0499f32, 0.0]]))
    else {
        panic!("размеры");
    };
    assert!(odd.data().iter().all(|v| v.is_nan()));
    let rgb = stack(s2::product("infrared", &bands, Mission::Sentinel2));
    assert_eq!(rgb.len(), 3);
    assert_eq!(rgb[0].shape(), (1, 2));
    close(f64::from(at(&rgb[0], 0, 0)), 0.5, 1e-7); // ИК - красным
}

#[test]
fn needed_bands_once() {
    assert_eq!(
        s2::needed(&["natural", "ndvi"], Mission::Sentinel2, false),
        ["red", "green", "blue", "nir"]
    );
    for name in s2::PRODUCTS {
        assert!(!s2::needed(&[name], Mission::Sentinel2, false).is_empty());
    }
}

#[test]
fn names() {
    let s = parsed(&item());
    assert_eq!(s2::file_name(&s, "ndvi"), "S2_20260929_40VDK_ndvi");
    assert_eq!(
        s2::credit(&s),
        "Contains modified Copernicus Sentinel data 2026"
    );
}

const LANDSAT: &str = "https://landsateuwest.blob.core.windows.net/landsat-c2/level-2/\
                       standard/oli-tirs/2025/166/020/LC09_L2SP_166020_20250923_\
                       20250924_02_T1/LC09_L2SP_166020_20250923_20250924_02_T1_";

fn landsat_item(platform: &str, thermal: &str) -> Value {
    let optical = json!({"scale": 2.75e-05, "offset": -0.2, "nodata": 0,
                         "spatial_resolution": 30});
    let mut assets = serde_json::Map::new();
    for k in ["blue", "green", "red", "nir08", "swir16", "swir22"] {
        assets.insert(
            k.to_string(),
            json!({"href": format!("{LANDSAT}{k}.TIF"), "raster:bands": [optical]}),
        );
    }
    assets.insert(
        thermal.to_string(),
        json!({"href": format!("{LANDSAT}ST_B10.TIF"), "raster:bands": [
            {"scale": 0.00341802, "offset": 149, "nodata": 0}]}),
    );
    assets.insert(
        "qa_pixel".to_string(),
        json!({"href": format!("{LANDSAT}QA_PIXEL.TIF"), "raster:bands": [{"nodata": 1}]}),
    );
    assets.insert(
        "rendered_preview".to_string(),
        json!({"href": "https://planetarycomputer.microsoft.com/api/data/v1/item/\
                        preview.png?collection=landsat-c2-l2&item=x"}),
    );
    json!({"id": "LC09_L2SP_166020_20250923_02_T1",
           "properties": {"datetime": "2025-09-23T07:14:45.55Z",
                          "platform": platform, "proj:epsg": 32640,
                          "eo:cloud_cover": 24.97,
                          "view:sun_elevation": 31.7,
                          "landsat:wrs_path": "166",
                          "landsat:wrs_row": "020"},
           "assets": assets})
}

// TestThermalFill

#[test]
fn formula_inverts_radiance_and_keeps_usgs() {
    // Излучение сцены, построенное из 300 К и ε травы, формула
    // обращает обратно. Где USGS дал значение, оно остаётся.
    let Some((k1, k2)) = s2::thermal_k("9") else {
        panic!("нет K1 K2");
    };
    let ndvi = 0.7;
    let eps = f64::from(at(&s2::emissivity(&grid(&[&[ndvi as f32]])), 0, 0));
    close(eps, 0.99, 1e-4);
    let black = k1 / ((k2 / 300.0f64).exp() - 1.0);
    let (tau, up, down) = (0.8, 1.2, 2.0);
    let trad = tau * eps * black + up + tau * (1.0 - eps) * down;
    let (red, nir) = (0.03, 0.03 * (1.0 + ndvi) / (1.0 - ndvi));
    let full = |v: f64| Grid::filled(1, 2, v as f32);
    let bands = bands_of(&[
        ("lwir", grid(&[&[f32::NAN, 290.0]])),
        ("trad", full(trad)),
        ("atran", full(tau)),
        ("urad", full(up)),
        ("drad", full(down)),
        ("red", full(red)),
        ("nir08", full(nir)),
    ]);
    let Some((kelvin, share)) = s2::thermal_fill(&bands, "9") else {
        panic!("нет каналов");
    };
    close(f64::from(at(&kelvin, 0, 0)), 300.0, 0.005);
    assert_eq!(at(&kelvin, 0, 1), 290.0);
    close(share, 0.5, 1e-7);
    assert!(
        s2::needed(&["lst"], Mission::Landsat, true)
            .iter()
            .any(|k| k == "trad")
    );
    assert!(
        !s2::needed(&["lst"], Mission::Landsat, false)
            .iter()
            .any(|k| k == "trad")
    );
}

#[test]
fn emissivity_classes() {
    let eps = s2::emissivity(&grid(&[&[-0.3f32, 0.1, 0.35, 0.8]]));
    let rounded: Vec<f64> = eps
        .data()
        .iter()
        .map(|e| (f64::from(*e) * 1000.0).round() / 1000.0)
        .collect();
    assert_eq!(rounded, [0.991, 0.973, 0.987, 0.99]);
}

// TestSide

#[test]
fn side_follows_view_within_limits() {
    assert_eq!(s2::side_for_view(16240.0), 16.2);
    assert_eq!(s2::side_for_view(300.0), s2::SIDE_MIN);
    assert_eq!(s2::side_for_view(5e6), s2::SIDE_MAX);
}

// TestLandsat

#[test]
fn landsat_item_parsed() {
    let s = parsed(&landsat_item("landsat-9", "lwir11"));
    assert_eq!(s.mission, Mission::Landsat);
    assert_eq!(s.tile, "L9 166/020");
    assert_eq!(s.epsg, Some(32640));
    assert_eq!(s.when, "2025-09-23 07:14");
    assert!(s.thumbnail.contains("preview.png"));
    close(asset(&s, "red").offset, -0.2, 1e-12);
    // Тепловой канал Landsat 8-9 доступен под общим ключом lwir.
    assert_eq!(asset(&s, "lwir").href, asset(&s, "lwir11").href);
    close(asset(&s, "lwir").offset, 149.0, 1e-12);
}

#[test]
fn old_landsat_thermal() {
    let s = parsed(&landsat_item("landsat-5", "lwir"));
    assert_eq!(s.tile, "L5 166/020");
    assert!(s.assets.contains("lwir"));
}

#[test]
fn landsat_search_body() {
    let body = s2::search_body(
        &json!({"type": "Point", "coordinates": [56, 58]}),
        "1990-01-01",
        "1990-12-31",
        50.0,
        s2::PAGE,
        Mission::Landsat,
    );
    assert_eq!(body["collections"], json!(["landsat-c2-l2"]));
    assert_eq!(s2::search_url(Mission::Landsat), s2::LANDSAT_SEARCH);
}

#[test]
fn qa_clouds() {
    // Чистый 21824, облако 22280 (биты 3 и 1), тень 23888 (бит 4),
    // нет данных 1 (бит 0).
    let qa = grid(&[&[21824.0, 22280.0], &[23888.0, 1.0]]);
    let inside = Grid::filled(2, 2, true);
    let Some((cloud, valid)) = s2::cloud_share_qa(&qa, &inside) else {
        panic!("размеры");
    };
    close(cloud, 200.0 / 3.0, 1e-7);
    close(valid, 75.0, 1e-7);
    let scene = parsed(&landsat_item("landsat-9", "lwir11"));
    let clear = s2::clear_mask(&scene, &qa);
    assert_eq!(clear, grid(&[&[true, false], &[false, false]]));
}

#[test]
fn products_use_nir08() {
    assert_eq!(
        s2::needed(&["infrared", "ndvi"], Mission::Landsat, false),
        ["nir08", "red", "green"]
    );
    assert_eq!(s2::needed(&["lst"], Mission::Landsat, false), ["lwir"]);
    assert!(s2::products_of(Mission::Landsat).contains(&"lst"));
    assert!(!s2::products_of(Mission::Sentinel2).contains(&"lst"));
    let bands = bands_of(&[
        ("nir08", Grid::filled(1, 1, 0.4f32)),
        ("red", Grid::filled(1, 1, 0.1f32)),
    ]);
    let ndvi = single(s2::product("ndvi", &bands, Mission::Landsat));
    close(f64::from(at(&ndvi, 0, 0)), 0.6, 1e-5);
}

#[test]
fn surface_temperature() {
    let scene = parsed(&landsat_item("landsat-9", "lwir11"));
    let kelvin = s2::reflectance(&grid(&[&[0.0, 43636.0]]), asset(&scene, "lwir"));
    let lst = single(s2::product(
        "lst",
        &bands_of(&[("lwir", kelvin)]),
        Mission::Landsat,
    ));
    assert!(at(&lst, 0, 0).is_nan());
    // 43636 * 0.00341802 + 149 = 298.15 K = 25 °C.
    close(f64::from(at(&lst, 0, 1)), 25.0, 0.05);
}

#[test]
fn landsat_names_and_credit() {
    let s = parsed(&landsat_item("landsat-9", "lwir11"));
    assert_eq!(s2::file_name(&s, "ndvi"), "L9_20250923_166-020_ndvi");
    assert_eq!(
        s2::credit(&s),
        "Landsat 9 image courtesy of the U.S. Geological Survey"
    );
    assert_eq!(
        s2::step_for(&[(0.0, 0.0), (3000.0, 3000.0)], Mission::Landsat, &[]),
        30.0
    );
}

#[test]
fn sign() {
    assert_eq!(
        s2::sign_one("https://a/b.TIF", "st=1&sig=2"),
        "https://a/b.TIF?st=1&sig=2"
    );
    assert_eq!(
        s2::sign_one("https://a/b.TIF?x=1", "sig=2"),
        "https://a/b.TIF?x=1&sig=2"
    );
    assert_eq!(s2::sign_one("https://a/b.TIF", ""), "https://a/b.TIF");
    let tiles = Href::Tiles(vec!["https://a/1.tif".into(), "https://a/2.tif?x".into()]);
    assert_eq!(
        s2::sign(&tiles, "k=1"),
        Href::Tiles(vec![
            "https://a/1.tif?k=1".into(),
            "https://a/2.tif?x&k=1".into()
        ])
    );
}

#[test]
fn sas_service_and_token() {
    // Ключ хранилища файла, без файла - ключ коллекции.
    assert_eq!(
        s2::sas_url(Mission::Landsat, Some(LANDSAT)),
        "https://planetarycomputer.microsoft.com/api/sas/v1/token/landsateuwest/landsat-c2"
    );
    assert_eq!(s2::sas_url(Mission::Landsat, None), s2::LANDSAT_SAS_URL);
    assert_eq!(
        s2::sas_url(Mission::ModisVeg, Some("https://example.com/a/b")),
        "https://planetarycomputer.microsoft.com/api/sas/v1/token/modis-13Q1-061"
    );
    assert_eq!(
        s2::parse_token(br#"{"msft:expiry": "x", "token": "st=1&sig=2"}"#),
        Some("st=1&sig=2".to_string())
    );
    assert_eq!(s2::parse_token(b"{}"), None);
    assert_eq!(s2::parse_token(b"\xff"), None);
    assert!(!s2::needs_key(Mission::Sentinel2));
    assert!(s2::needs_key(Mission::Landsat));
    assert!(!s2::needs_key(Mission::AsterNasa));
}

// TestAllBands

const S2_FILES: [&str; 12] = [
    "B01", "B02", "B03", "B04", "B05", "B06", "B07", "B08", "B8A", "B09", "B11", "B12",
];

#[test]
fn sentinel_all_bands() {
    let keys = s2::all_bands(Mission::Sentinel2);
    let mut data = item();
    for (key, name) in keys.iter().zip(S2_FILES) {
        set_asset(
            &mut data,
            key,
            json!({"href": format!("{BASE}{name}.tif"), "raster:bands": [{"scale": 0.0001}]}),
        );
    }
    let s = parsed(&data);
    assert!(s2::products_of(Mission::Sentinel2).contains(&"all_bands"));
    assert_eq!(s2::all_keys(&s), keys);
    assert_eq!(s2::band_label(&s, "nir08"), "B8A nir08");
    assert_eq!(s2::band_label(&s, "red"), "B4 red");
    assert_eq!(s2::needed(&["all_bands"], Mission::Sentinel2, false), keys);
    let list: Vec<(&str, Grid<f32>)> = keys
        .iter()
        .enumerate()
        .map(|(n, k)| (*k, Grid::filled(1, 1, n as f32)))
        .collect();
    let all = stack(s2::product(
        "all_bands",
        &bands_of(&list),
        Mission::Sentinel2,
    ));
    assert_eq!(all.len(), 12);
    assert_eq!(at(&all[8], 0, 0), 8.0);
    // 30 км на 10 м - 3000 пикселей: с «Всеми каналами» шаг 20 м.
    let side = [(0.0, 0.0), (30000.0, 30000.0)];
    assert_eq!(s2::step_for(&side, Mission::Sentinel2, &[]), 10.0);
    assert_eq!(
        s2::step_for(&side, Mission::Sentinel2, &["all_bands"]),
        20.0
    );
}

#[test]
fn landsat_without_coastal() {
    let mut data = landsat_item("landsat-5", "lwir");
    set_asset(
        &mut data,
        "blue",
        json!({"href": format!("{LANDSAT}SR_B1.TIF"), "raster:bands": [{"scale": 2.75e-05}]}),
    );
    set_asset(
        &mut data,
        "red",
        json!({"href": format!("{LANDSAT}SR_B3.TIF"), "raster:bands": [{"scale": 2.75e-05}]}),
    );
    data["assets"]["lwir"]["href"] = json!(format!("{LANDSAT}ST_B6.TIF"));
    let s = parsed(&data);
    assert_eq!(
        s2::all_keys(&s),
        ["blue", "green", "red", "nir08", "swir16", "swir22", "lwir"]
    );
    assert_eq!(s2::band_label(&s, "lwir"), "B6 lwir, K");
    assert_eq!(s2::band_label(&s, "red"), "B3 red");
    assert!(s2::products_of(Mission::Landsat).contains(&"all_bands"));
    assert!(!s2::products_of(Mission::Mss).contains(&"all_bands"));
}

fn scene(mission: Mission, list: &[(&str, Href)]) -> Scene {
    let mut assets = Assets::new();
    for (k, h) in list {
        assets.insert(
            k,
            Asset {
                href: h.clone(),
                scale: 1.0,
                offset: 0.0,
                nodata: Some(0.0),
                res: Some(30.0),
            },
        );
    }
    Scene {
        id: "x".into(),
        when: "2026-04-05 17:08".into(),
        cloud: 0.0,
        tile: "T".into(),
        epsg: Some(32611),
        sun: Some(60.0),
        thumbnail: String::new(),
        assets,
        mission,
    }
}

fn one(h: &str) -> Href {
    Href::One(h.to_string())
}

// TestWhole

#[test]
fn whole_sentinel_bands_then_cloud_mask() {
    let keys = s2::all_bands(Mission::Sentinel2);
    let mut data = item();
    for key in keys {
        set_asset(
            &mut data,
            key,
            json!({"href": format!("{BASE}{key}.tif"), "raster:bands": [{"scale": 0.0001}]}),
        );
    }
    let s = parsed(&data);
    let files = s2::whole_files(&s);
    assert_eq!(&files[..12], keys);
    assert_eq!(&files[12..], ["scl"]);
    let bands = s2::whole_bands(&s);
    let names: Vec<&str> = bands.iter().map(|b| b.key.as_str()).collect();
    assert_eq!(names, keys);
    assert!(bands.iter().all(|b| b.file == b.key && b.index == 1));
}

#[test]
fn whole_aster_group_files_split_into_bands() {
    let s = scene(
        Mission::Aster,
        &[("VNIR", one("v.tif")), ("TIR", one("t.tif"))],
    );
    assert_eq!(s2::whole_files(&s), ["VNIR", "TIR"]);
    let bands = s2::whole_bands(&s);
    let names: Vec<&str> = bands.iter().map(|b| b.key.as_str()).collect();
    assert_eq!(
        names,
        ["b1", "b2", "b3n", "b10", "b11", "b12", "b13", "b14"]
    );
    assert_eq!((bands[2].file.as_str(), bands[2].index), ("VNIR", 3));
    assert_eq!((bands[7].file.as_str(), bands[7].index), ("TIR", 5));
    assert_eq!(bands[2].label, "B3N 0.82 um");
    assert_eq!(bands[7].label, "B14 11.3 um, K");
}

#[test]
fn whole_aster_nasa_file_per_band() {
    let s = scene(
        Mission::AsterNasa,
        &[
            ("b1", one("1.tif")),
            ("b2", one("2.tif")),
            ("b3n", one("3.tif")),
        ],
    );
    assert_eq!(s2::whole_files(&s), ["b1", "b2", "b3n"]);
    let bands = s2::whole_bands(&s);
    let got: Vec<(&str, usize)> = bands.iter().map(|b| (b.file.as_str(), b.index)).collect();
    assert_eq!(got, [("b1", 1), ("b2", 1), ("b3n", 1)]);
}

#[test]
fn whole_modis_tiles_without_hdf() {
    let tiles = |a: &str, b: &str| Href::Tiles(vec![a.to_string(), b.to_string()]);
    let s = scene(
        Mission::ModisLst1,
        &[
            ("LST_Day_1km", tiles("a/day.tif", "b/day.tif")),
            ("LST_Night_1km", tiles("a/night.tif?sig", "b/night.tif")),
            ("hdf", one("a/file.hdf")),
        ],
    );
    assert_eq!(s2::whole_files(&s), ["LST_Day_1km", "LST_Night_1km"]);
    let names: Vec<String> = s2::whole_bands(&s).into_iter().map(|b| b.key).collect();
    assert_eq!(names, ["LST_Day_1km", "LST_Night_1km"]);
}

const MSS: &str = "https://landsateuwest.blob.core.windows.net/landsat-c2/level-1/\
                   standard/mss/1973/181/019/LM01_L1TP_181019_19730606_20200909_02_T2/\
                   LM01_L1TP_181019_19730606_20200909_02_T2_";

/// Сцена Landsat 1 MSS у Перми 6 июня 1973 года, как в каталоге
/// Planetary Computer 12 октября 2026 года.
fn mss_item() -> Value {
    let bands = [
        ("green", "B4", 0.95591, -18.55591),
        ("red", "B5", 0.64843, -0.74843),
        ("nir08", "B6", 0.65236, -0.75236),
        ("nir09", "B7", 0.60866, -0.60866),
    ];
    let mut assets = serde_json::Map::new();
    for (k, b, scale, offset) in bands {
        assets.insert(
            k.to_string(),
            json!({"href": format!("{MSS}{b}.TIF"), "raster:bands": [
                {"scale": scale, "offset": offset, "nodata": 0}]}),
        );
    }
    assets.insert(
        "qa_pixel".to_string(),
        json!({"href": format!("{MSS}QA_PIXEL.TIF"), "raster:bands": [{"nodata": 1}]}),
    );
    assets.insert(
        "mtl.json".to_string(),
        json!({"href": format!("{MSS}MTL.json")}),
    );
    json!({"id": "LM01_L1TP_181019_19730606_02_T2",
           "collection": "landsat-c2-l1",
           "properties": {"datetime": "1973-06-06T07:00:36.5Z",
                          "platform": "landsat-1", "proj:epsg": 32640,
                          "eo:cloud_cover": 8.0,
                          "view:sun_elevation": 51.38128591,
                          "landsat:wrs_path": "181",
                          "landsat:wrs_row": "019"},
           "assets": assets})
}

/// Выдержка из mtl.json той же сцены.
fn mss_mtl() -> Value {
    json!({"LANDSAT_METADATA_FILE": {
        "LEVEL1_RADIOMETRIC_RESCALING": {
            "REFLECTANCE_MULT_BAND_4": "1.7269E-03",
            "REFLECTANCE_MULT_BAND_5": "1.3650E-03",
            "REFLECTANCE_MULT_BAND_6": "1.6568E-03",
            "REFLECTANCE_MULT_BAND_7": "2.3270E-03",
            "REFLECTANCE_ADD_BAND_4": "-0.033522",
            "REFLECTANCE_ADD_BAND_5": "-0.001575",
            "REFLECTANCE_ADD_BAND_6": "-0.001911",
            "REFLECTANCE_ADD_BAND_7": "-0.002327"},
        "IMAGE_ATTRIBUTES": {"SUN_ELEVATION": "51.38128591"}}})
}

// TestMss

#[test]
fn mss_item_is_mss() {
    let s = parsed(&mss_item());
    assert_eq!(s.mission, Mission::Mss);
    assert_eq!(s.tile, "L1 181/019");
    assert_eq!(s2::file_name(&s, "ndvi"), "LM1_19730606_181-019_ndvi");
    assert_eq!(
        s2::credit(&s),
        "Landsat 1 image courtesy of the U.S. Geological Survey"
    );
    assert!(s2::needs_key(Mission::Mss));
}

#[test]
fn mss_search_and_steps() {
    let body = s2::search_body(
        &json!({"type": "Point", "coordinates": [56, 58]}),
        "1975-05-01",
        "1975-09-30",
        50.0,
        s2::PAGE,
        Mission::Mss,
    );
    assert_eq!(body["collections"], json!(["landsat-c2-l1"]));
    assert_eq!(
        s2::step_for(&[(0.0, 0.0), (3000.0, 3000.0)], Mission::Mss, &[]),
        60.0
    );
    assert_eq!(s2::cloud_asset(Mission::Mss), ("qa_pixel", 60.0));
}

#[test]
fn mss_toa_reflectance() {
    // Значение 100 красного канала: (1.365e-3 × 100 - 0.001575)
    // / sin(51.38°) = 0.1727. Каталог давал бы яркость 64.1.
    let Ok(s) = s2::toa(&parsed(&mss_item()), &mss_mtl()) else {
        panic!("нет солнца");
    };
    let value = s2::reflectance(&grid(&[&[0.0, 100.0]]), asset(&s, "red"));
    assert!(at(&value, 0, 0).is_nan());
    let sine = 51.38128591f64.to_radians().sin();
    let expected = (1.365e-3 * 100.0 - 0.001575) / sine;
    close(f64::from(at(&value, 0, 1)), expected, 1e-5);
    // mtl.json не масштабируется, у зелёного - свой канал B4.
    assert_eq!(asset(&s, "mtl.json").scale, 1.0);
    close(asset(&s, "green").offset * sine, -0.033522, 1e-6);
    // Без высоты солнца и под горизонтом - ошибка.
    let mut dark = parsed(&mss_item());
    dark.sun = None;
    assert!(s2::toa(&dark, &json!({})).is_err());
    let night = json!({"LANDSAT_METADATA_FILE": {"IMAGE_ATTRIBUTES": {"SUN_ELEVATION": -3}}});
    assert!(s2::toa(&parsed(&mss_item()), &night).is_err());
}

#[test]
fn mss_products() {
    assert_eq!(
        s2::products_of(Mission::Mss),
        ["infrared", "mss_bands", "ndvi", "ndwi"]
    );
    assert_eq!(
        s2::needed(&["infrared", "mss_bands"], Mission::Mss, false),
        ["nir08", "red", "green", "nir09"]
    );
    let bands = bands_of(&[
        ("green", Grid::filled(1, 1, 0.1f32)),
        ("red", Grid::filled(1, 1, 0.2f32)),
        ("nir08", Grid::filled(1, 1, 0.3f32)),
        ("nir09", Grid::filled(1, 1, 0.4f32)),
    ]);
    let all = stack(s2::product("mss_bands", &bands, Mission::Mss));
    assert_eq!(all.len(), 4);
    close(f64::from(at(&all[3], 0, 0)), 0.4, 1e-6);
    let ndvi = single(s2::product("ndvi", &bands, Mission::Mss));
    close(f64::from(at(&ndvi, 0, 0)), 0.2, 1e-5);
}

#[test]
fn mss_saturated_line_is_filled() {
    // Строка насыщенного зелёного (бит 3 у B4 Landsat 1-3) среди
    // ровного поля 0.1 заменяется соседями, сплошная насыщенная
    // область без целых соседей остаётся.
    let s = parsed(&mss_item());
    let Some(bit) = s2::radsat_bit(asset(&s, "green")) else {
        panic!("нет бита");
    };
    assert_eq!(bit, 3);
    assert_eq!(s2::radsat_bit(asset(&s, "nir09")), Some(6));
    // Полоса - строки 3-4 насыщены, строки 2 и 5 провалены в тёмное,
    // как на сцене 21 сентября 1975 года.
    let mut values = Grid::filled(8, 5, 0.1f32);
    let mut radsat = Grid::filled(8, 5, 0.0f64);
    let mut shifted = Grid::filled(8, 5, 0.0f64);
    for c in 0..5 {
        for r in 3..5 {
            values.set(r, c, 0.9);
            radsat.set(r, c, f64::from(1u32 << bit));
            shifted.set(r, c, f64::from(1u32 << (bit + 1)));
        }
        values.set(2, c, 0.03);
        values.set(5, c, 0.03);
    }
    let Some((filled, count)) = s2::fill_saturated(&values, &radsat, bit) else {
        panic!("размеры");
    };
    assert_eq!(count, 20);
    assert!(filled.data().iter().all(|v| (v - 0.1).abs() < 1e-6));
    // Насыщение другого канала зелёный не трогает.
    let Some((same, none)) = s2::fill_saturated(&values, &shifted, bit) else {
        panic!("размеры");
    };
    assert_eq!(none, 0);
    close(f64::from(at(&same, 3, 2)), 0.9, 1e-6);
    // Облако выше STRIPE_ROWS строк остаётся целиком.
    let mut cloud = Grid::filled(9, 5, 0.1f32);
    let mut flags = Grid::filled(9, 5, 0.0f64);
    for r in 1..8 {
        for c in 0..5 {
            cloud.set(r, c, 0.9);
            flags.set(r, c, f64::from(1u32 << bit));
        }
    }
    let Some((kept, none)) = s2::fill_saturated(&cloud, &flags, bit) else {
        panic!("размеры");
    };
    assert_eq!(none, 0);
    for r in 1..8 {
        for c in 0..5 {
            close(f64::from(at(&kept, r, c)), 0.9, 1e-6);
        }
    }
}

#[test]
fn mss_clean_zone() {
    // Насыщенный пиксель зелёного, рядом в зоне полосы - провал
    // зелёного без отметки, далеко от полосы - такой же провал,
    // он настоящий и остаётся.
    let mut bands: Bands = s2::MSS_BANDS
        .iter()
        .map(|k| (k.to_string(), Grid::filled(12, 20, 0.1f32)))
        .collect();
    let Some(green) = bands.get_mut("green") else {
        panic!("нет зелёного");
    };
    green.set(5, 5, 0.9);
    green.set(6, 8, 0.03);
    green.set(10, 17, 0.03);
    let mut radsat = Grid::filled(12, 20, 0.0f64);
    radsat.set(5, 5, f64::from(1u32 << 3));
    let bits: BTreeMap<String, u32> = [("green", 3), ("red", 4), ("nir08", 5), ("nir09", 6)]
        .into_iter()
        .map(|(k, b)| (k.to_string(), b))
        .collect();
    let Some((out, done)) = s2::clean_mss(&bands, &radsat, &bits) else {
        panic!("размеры");
    };
    let (Some(g), Some(red)) = (out.get("green"), out.get("red")) else {
        panic!("нет каналов");
    };
    close(f64::from(at(g, 5, 5)), 0.1, 1e-6);
    close(f64::from(at(g, 6, 8)), 0.1, 1e-6);
    close(f64::from(at(g, 10, 17)), 0.03, 1e-6);
    assert!(red.data().iter().all(|v| (v - 0.1).abs() < 1e-6));
    assert!(done >= 2);
}

#[test]
fn mss_single_band_spike_is_found() {
    // Всплеск зелёного на ровном поле - полоса, яркая крыша во всех
    // каналах - нет.
    let mut bands: Bands = s2::MSS_BANDS
        .iter()
        .map(|k| (k.to_string(), Grid::filled(5, 5, 0.1f32)))
        .collect();
    for (key, g) in bands.iter_mut() {
        if key == "green" {
            g.set(2, 1, 0.4);
        }
        g.set(2, 3, 0.4);
    }
    let Some(found) = s2::spikes(&bands, &s2::MSS_BANDS) else {
        panic!("нет каналов");
    };
    let (Some(green), Some(red)) = (found.get("green"), found.get("red")) else {
        panic!("нет масок");
    };
    let mut hits = Vec::new();
    for r in 0..5 {
        for c in 0..5 {
            if at(green, r, c) {
                hits.push((r, c));
            }
        }
    }
    assert_eq!(hits, [(2, 1)]);
    assert_eq!(red.count(), 0);
    let Some(values) = bands.get("green") else {
        panic!("нет зелёного");
    };
    let Some((fixed, count)) = s2::fill_mask(values, green, 1) else {
        panic!("размеры");
    };
    assert_eq!(count, 1);
    close(f64::from(at(&fixed, 2, 1)), 0.1, 1e-6);
    close(f64::from(at(&fixed, 2, 3)), 0.4, 1e-6);
}

#[test]
fn mss_qa_cloud_bit() {
    // Бит 3 - облако, бит 0 - нет данных, как у qa_pixel MSS.
    let qa = grid(&[&[0.0, 8.0], &[1.0, 0.0]]);
    let Some((cloud, valid)) = s2::clouds_of(&parsed(&mss_item()), &qa, &Grid::filled(2, 2, true))
    else {
        panic!("размеры");
    };
    close(cloud, 100.0 / 3.0, 1e-7);
    close(valid, 75.0, 1e-7);
}

// ASTER и MODIS, которые зовёт модуль снимков.

#[test]
fn aster_cmr_url_and_names() {
    use data::sentinel::aster;
    let ring = [(58.0, 56.0), (58.5, 56.5)];
    let Some(url) = aster::cmr_url(&ring, "2010-06-01", "2010-08-31", 80.0, aster::NASA_PAGE)
    else {
        panic!("нет адреса");
    };
    assert_eq!(
        url,
        "https://cmr.earthdata.nasa.gov/search/granules.umm_json?collection_concept_id=\
         C3306888411-LPCLOUD&bounding_box=56.000000%2C58.000000%2C56.500000%2C58.500000\
         &temporal=2010-06-01T00%3A00%3A00Z%2C2010-08-31T23%3A59%3A59Z&cloud_cover=0%2C80\
         &sort_key=-start_date&page_size=200"
    );
    assert_eq!(aster::file_suffix("b3n").as_deref(), Some("VNIR_B03N"));
    assert_eq!(aster::file_suffix("b10").as_deref(), Some("TIR_B10"));
    assert_eq!(aster::format_g(12.5), "12.5");
    assert_eq!(aster::format_g(0.1234567), "0.123457");
    assert_eq!(aster::format_g(1e7), "1e+07");
    assert_eq!(aster::day_of_year("2024-03-01"), Some(61));
    assert_eq!(aster::day_of_year("2023-02-30"), None);
}

#[test]
fn modis_tiles_merge_into_day() {
    use data::sentinel::modis;
    let tile = |when: &str, tile: &str, href: &str| {
        let mut s = scene(
            Mission::ModisVeg,
            &[("NDVI", Href::Tiles(vec![href.to_string()]))],
        );
        s.when = when.to_string();
        s.tile = tile.to_string();
        s
    };
    let merged = modis::merge_tiles(&[
        tile("2026-01-01", "Terra h21v03", "a"),
        tile("2026-01-17", "Terra h21v03", "b"),
        tile("2026-01-01", "Terra h22v03", "c"),
    ]);
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0].when, "2026-01-17");
    assert_eq!(merged[1].tile, "Terra h21v03,h22v03");
    let Some(a) = merged[1].assets.get("NDVI") else {
        panic!("нет канала");
    };
    assert_eq!(a.href, Href::Tiles(vec!["a".into(), "c".into()]));
    assert_eq!(modis::utm_epsg(&[(58.0, 56.25)]), Some(32640));
    assert_eq!(modis::utm_epsg(&[(-10.0, -179.0)]), Some(32701));
    assert_eq!(
        modis::credit("MOD10A1.A2026001.h21v03.061"),
        "MOD10A1 v061, NASA EOSDIS NSIDC DAAC"
    );
    let Some(lst) = modis::value("lst_day", &grid(&[&[0.0, 14000.0]])) else {
        panic!("нет продукта");
    };
    assert!(at(&lst, 0, 0).is_nan());
    close(f64::from(at(&lst, 0, 1)), 6.85, 1e-4);
}
