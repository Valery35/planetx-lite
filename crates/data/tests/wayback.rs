// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Архив снимков Esri Wayback. Перенос `tests/test_wayback.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use data::wayback as wb;

const CONFIG: &str = r#"{
 "63116": {"itemTitle": "World Imagery (Wayback 2026-09-24)", "metadataLayerUrl": "https://metadata.example/2026_r09/MapServer"},
 "22252": {"itemTitle": "World Imagery (Wayback 2020-03-11)", "metadataLayerUrl": ""},
 "22000": {"itemTitle": "World Imagery (Wayback 2020-03-11)", "metadataLayerUrl": ""},
 "10": {"itemTitle": "World Imagery (Wayback 2014-02-20)", "metadataLayerUrl": ""},
 "bad": {"itemTitle": "World Imagery (Wayback 2015-01-01)"}}"#;
// Ответы tilemap у Перми 18 октября 2026 года.
const TILEMAP_NEW: &[u8] = br#"{"data":[1],"location":{"height":1,"left":21612,"top":9449,"width":1},"select":[22252],"size":[21325],"valid":true}"#;
const TILEMAP_OWN: &[u8] = br#"{"data":[1],"location":{"height":1,"left":21612,"top":9449,"width":1},"size":[13041],"valid":true}"#;
const METADATA: &[u8] = br#"{"features": [{"attributes": {"SRC_DATE2": 1747180800000, "NICE_DESC": "Vantor", "SRC_DESC": "LG02", "SRC_RES": 0.34, "SRC_ACC": 8.47}}]}"#;

fn releases() -> Vec<wb::Release> {
    wb::parse_config(CONFIG.as_bytes())
}

#[test]
fn config_sorted_by_day_one_per_day() {
    let list = releases();
    assert_eq!(list.iter().map(|r| r.number).collect::<Vec<_>>(), [10, 22252, 63116]);
    assert_eq!(list[2].day, "2026-09-24");
    assert!(list[2].meta.ends_with("MapServer"));
}

#[test]
fn tile_url_keeps_loader_places() {
    assert!(wb::tile_url(63116).ends_with("/tile/63116/{z}/{y}/{x}"));
}

#[test]
fn tilemap_and_change_chain() {
    assert_eq!(wb::parse_tilemap(TILEMAP_NEW, 63116), Some(22252));
    assert_eq!(wb::parse_tilemap(TILEMAP_OWN, 10), Some(10));
    assert_eq!(wb::parse_tilemap(br#"{"data":[0]}"#, 10), None);
    // Тайл новейшего выпуска - из 22252, следующий запрос - старше.
    let list = releases();
    assert_eq!(wb::changes_next(&list, 22252).unwrap().number, 10);
    assert!(wb::changes_next(&list, 10).is_none());
}

#[test]
fn tile_of() {
    assert_eq!(wb::tile_of(0.0, 0.0, 1), (1, 1));
    assert_eq!(wb::tile_of(60.0, -170.0, 2), (0, 1));
}

#[test]
fn search_level_follows_view_width() {
    assert_eq!(wb::search_level(600.0), wb::LOCAL_LEVEL);
    assert_eq!(wb::search_level(2000.0), 15);
    assert_eq!(wb::search_level(2.0e6), 5);
    assert_eq!(wb::search_level(4.0e7), 2);
}

#[test]
fn metadata() {
    assert_eq!(wb::metadata_layer(23), 0);
    assert_eq!(wb::metadata_layer(16), 7);
    assert_eq!(wb::metadata_layer(5), 13);
    let list = releases();
    let url = wb::metadata_url(&list[2], 58.01, 56.25, 16).unwrap();
    assert!(url.contains("/MapServer/7/query?geometry=56.2500000,58.0100000"), "{url}");
    assert!(wb::metadata_url(&list[0], 0.0, 0.0, 16).is_none());
    let info = wb::parse_metadata(METADATA).unwrap();
    assert_eq!(info.date, "2025-05-14");
    assert_eq!(info.provider, "Vantor");
    assert_eq!(info.resolution, Some(0.34));
    assert!(wb::parse_metadata(br#"{"features":[]}"#).is_none());
}
