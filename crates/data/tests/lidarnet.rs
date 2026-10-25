// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Облака точек лидара по сети: запросы и разбор каталогов IGN и 3DEP,
//! выбор плиток у точки, ключ SAS. Перенос `tests/test_lidarnet.py`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use data::lidarnet;
use serde_json::{Value, json};

/// Ответ WFS IGN у Гренобля 25 октября 2026 года, сокращён.
fn ign() -> Value {
    json!({"type": "FeatureCollection", "features": [
        {"geometry": {"type": "MultiPolygon", "coordinates": [[[
            [5.70086541, 45.20668645], [5.71359534, 45.2063778],
            [5.71315777, 45.19737892], [5.70042989, 45.19768752],
            [5.70086541, 45.20668645]]]]},
         "properties": {"coordonnees_nw": "0912-6460",
                        "nombre_points": 16072567,
                        "date_debut_acquisition": "2023-09-27Z",
                        "url_npl": "http://data.geopf.fr/telechargement/download/LiDARHD-NUALID/x/LHD_FXX_0912_6460_PTS_LAMB93_IGN69.copc.laz"}},
        {"geometry": {"type": "Polygon", "coordinates": [[
            [5.71315777, 45.19737892], [5.7258855, 45.19706887],
            [5.72544602, 45.18807005], [5.71315777, 45.19737892]]]},
         "properties": {"coordonnees_nw": "0913-6459", "nombre_points": 9,
                        "url_npl": "https://data.geopf.fr/x/b.copc.laz"}},
        {"geometry": null, "properties": {"url_npl": "https://x/c.copc.laz"}}]})
}

/// Запись поиска STAC 3dep-lidar-copc, рамка с отметками.
fn pc() -> Value {
    json!({"features": [{
        "id": "USGS_LPC_UT_StatewideSouth_2020_A20_12SUH7021",
        "bbox": [-112.48332701, 38.12750163, 2475.74, -112.48220744, 38.13247722, 2754.14],
        "assets": {"data": {"href": "https://usgslidareuwest.blob.core.windows.net/usgs-3dep-copc/usgs-copc/a/copc/x.copc.laz"}},
        "properties": {"pc:count": 376166, "start_datetime": "2020-01-01T00:00:00Z"}}]})
}

/// Значение параметра строки запроса, раскодированное.
fn query_value(url: &str, key: &str) -> String {
    let query = url.split_once('?').unwrap().1;
    let raw = query.split('&').find_map(|p| p.strip_prefix(&format!("{key}="))).unwrap();
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                out.push(u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap(), 16).unwrap());
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8(out).unwrap()
}

#[test]
fn ign_request_and_parse() {
    let bbox = lidarnet::box_around(45.19, 5.725, lidarnet::SIDE);
    let url = lidarnet::ign_url(bbox);
    assert!(url.starts_with(lidarnet::IGN_WFS));
    let frame = query_value(&url, "BBOX");
    let v: Vec<f64> = frame.split(',').take(4).map(|s| s.parse().unwrap()).collect();
    let (south, _west, north, east) = (v[0], v[1], v[2], v[3]);
    assert!(south < 45.19);
    assert!(east > 5.725);
    assert!(((north - south) * 111_320.0 - lidarnet::SIDE).abs() <= 1.0);
    assert!(frame.ends_with(",urn:ogc:def:crs:EPSG::4326"));
    assert_eq!(query_value(&url, "TYPENAMES"), lidarnet::IGN_LAYER);
    // Как urlencode Python: двоеточие и запятая закодированы.
    assert!(url.contains("TYPENAMES=IGNF_LIDAR-HD_METADONNEE%3Ametadata"));
    let tiles = lidarnet::parse_ign(ign().to_string().as_bytes()).unwrap();
    let names: Vec<&str> = tiles.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["LiDAR HD 0912-6460", "LiDAR HD 0913-6459"]);
    assert!(tiles[0].url.starts_with("https://data.geopf.fr/"));
    assert_eq!(tiles[0].date, "2023-09-27");
    assert_eq!(tiles[0].points, 16_072_567);
    assert!((tiles[0].bbox[0] - 5.70042989).abs() < 1e-12);
    assert_eq!(tiles[0].source, lidarnet::IGN);
}

#[test]
fn pc_parse_and_key() {
    let body: Value = serde_json::from_str(&lidarnet::pc_body([1.0, 2.0, 3.0, 4.0])).unwrap();
    assert_eq!(body["collections"], json!([lidarnet::PC_COLLECTION]));
    assert_eq!(body["intersects"]["coordinates"][0][2], json!([3.0, 4.0]));
    let tiles = lidarnet::parse_pc(pc().to_string().as_bytes()).unwrap();
    assert_eq!(tiles.len(), 1);
    let tile = &tiles[0];
    assert_eq!(tile.bbox, [-112.48332701, 38.12750163, -112.48220744, 38.13247722]);
    assert_eq!((tile.points, tile.date.as_str()), (376_166, "2020"));
    assert_eq!(tile.source, lidarnet::USGS);
    assert!(lidarnet::needs_key(&tile.url));
    let keyed = lidarnet::with_key(&tile.url, "sv=1&sig=a");
    assert!(keyed.ends_with(".copc.laz?sv=1&sig=a"));
    assert_eq!(lidarnet::with_key(&keyed, "sv=2"), format!("{}?sv=2", tile.url));
    let ign = "https://data.geopf.fr/x/b.copc.laz";
    assert!(!lidarnet::needs_key(ign));
    assert_eq!(lidarnet::with_key(ign, "sv=2"), ign);
}

#[test]
fn choose_covering_first() {
    let tiles = lidarnet::parse_ign(ign().to_string().as_bytes()).unwrap();
    let twice: Vec<_> = tiles.iter().chain(&tiles).cloned().collect();
    let found = lidarnet::choose(&twice, 45.193, 5.718, 4);
    let names: Vec<&str> = found.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["LiDAR HD 0913-6459", "LiDAR HD 0912-6460"]);
    assert_eq!(lidarnet::choose(&tiles, 45.2, 5.71, 1).len(), 1);
}

#[test]
fn token_and_credit() {
    assert_eq!(lidarnet::parse_token(br#"{"msft:expiry": "2026-10-27T00:00:00Z", "token": "sv=1&sig=a"}"#).as_deref(), Some("sv=1&sig=a"));
    assert_eq!(lidarnet::parse_token(br#"{"token": 5}"#), None);
    assert_eq!(lidarnet::parse_token(b"<html>"), None);
    assert!(lidarnet::credit(lidarnet::IGN).0.contains("IGN"));
    assert!(lidarnet::credit(lidarnet::USGS).1.starts_with("https://"));
    assert_eq!(lidarnet::credit("x"), ("", ""));
}

#[test]
fn hostile_answers() {
    assert!(lidarnet::parse_ign(b"not json").is_err());
    assert!(lidarnet::parse_pc(b"\xff\xfe").is_err());
    assert_eq!(lidarnet::parse_ign(b"{}").unwrap(), []);
    assert_eq!(lidarnet::parse_pc(b"[1, 2]").unwrap(), []);
    // Глубокая вложенность, неполные точки, рамка не из чисел.
    let deep = json!({"features": [{"geometry": {"coordinates": [[[[[[[[[[[[1.0, 2.0]]]]]]]]]]]]}, "properties": {"url_npl": "https://a/b.copc.laz"}},
        {"geometry": {"coordinates": [[1.0]]}, "properties": {"url_npl": "https://a/c.copc.laz"}}]});
    assert_eq!(lidarnet::parse_ign(deep.to_string().as_bytes()).unwrap(), []);
    let bad = json!({"features": [{"bbox": ["a", 1, 2, 3], "assets": {"data": {"href": "https://a/b.copc.laz"}}},
        {"bbox": [1, 2, 3, 4], "assets": {"data": {"href": "http://a/b.copc.laz"}}}]});
    assert_eq!(lidarnet::parse_pc(bad.to_string().as_bytes()).unwrap(), []);
}
