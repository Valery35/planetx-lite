// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Википедия рядом: адреса, разбор ответов, тайлы, порядок. Перенос
//! `tests/test_wikipedia.py` модуля.

use data::infra::{tile_bounds, tile_of};
use data::wikipedia::{self as wp, Article};

const ANSWER: &str = r#"{"batchcomplete": "", "query": {"geosearch": [
    {"pageid": 423810, "ns": 0, "title": "Пермская соборная мечеть",
     "lat": 58.014088, "lon": 56.227608, "dist": 184.1, "primary": "",
     "type": "landmark", "dim": 10000},
    {"pageid": 1793088, "ns": 0, "title": "У моста", "lat": 58.0139,
     "lon": 56.2296, "dist": 297.3, "primary": "", "type": null,
     "dim": 200},
    {"pageid": 5, "ns": 0, "title": "Пермь", "lat": 58.0139,
     "lon": 56.2489, "type": "city"},
    {"pageid": 6, "ns": 0, "title": "", "lat": 58.0, "lon": 56.0},
    {"pageid": 7, "ns": 0, "title": "Без точки"}]}}"#;

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

fn rank(kind: &str) -> u32 {
    wp::TYPE_RANK.iter().find(|(k, _)| *k == kind).map_or(0, |(_, r)| *r)
}

#[test]
fn articles_and_rank() {
    let found = wp::parse_geosearch(ANSWER.as_bytes());
    assert!(found.is_ok(), "{found:?}");
    let Ok(found) = found else { return };
    let titles: Vec<&str> = found.iter().map(|a| a.title.as_str()).collect();
    assert_eq!(titles, ["Пермская соборная мечеть", "У моста", "Пермь"]);
    assert_eq!(found[0].rank, rank("landmark"));
    assert_eq!(found[1].rank, wp::OTHER_RANK);
    assert_eq!(found[2].rank, rank("city"));
    close(found[0].lat, 58.014088, 1e-12);
}

#[test]
fn service_error_is_error() {
    let error = wp::parse_geosearch(br#"{"error": {"code": "toobig", "info": "Bounding box is too big"}}"#);
    assert_eq!(error, Err("toobig: Bounding box is too big".to_string()));
    assert!(wp::parse_geosearch(b"[]").is_err());
    assert!(wp::parse_geosearch(b"not json").is_err());
}

#[test]
fn summary() {
    assert_eq!(wp::parse_summary(r#"{"extract": " Пермь - город. "}"#.as_bytes()), Ok("Пермь - город.".to_string()));
    assert_eq!(
        wp::parse_summary(r#"{"extract": "Центральный  — микрорайон."}"#.as_bytes()),
        Ok("Центральный — микрорайон.".to_string())
    );
    // Перевод строки между абзацами остаётся.
    assert_eq!(wp::parse_summary(br#"{"extract": "a\n\nb"}"#), Ok("a\n\nb".to_string()));
    assert_eq!(wp::parse_summary(br#"{"type": "not_found"}"#), Err("not_found".to_string()));
    assert!(wp::parse_summary(b"[1]").is_err());
}

#[test]
fn bbox_is_tile_north_west_south_east() {
    let tile = tile_of(58.01, 56.23, wp::ZOOM);
    let url = wp::geosearch_url("ru", tile);
    let (south, west, north, east) = tile_bounds(tile.0, tile.1, wp::ZOOM);
    assert!(url.starts_with("https://ru.wikipedia.org/w/api.php?"));
    assert!(url.contains(&format!("gsbbox={north:.6}|{west:.6}|{south:.6}|{east:.6}")), "{url}");
    assert!(url.contains("gslimit=500"));
    assert!(url.ends_with("&gsprop=type|dim"));
}

#[test]
fn tile_is_under_service_limit() {
    // Служба отвергает рамку 0.18° × 0.18° у экватора (около 400 км²)
    // и принимает 0.16°. Тайл уровня ZOOM у экватора уже 0.16°.
    let tile = tile_of(0.0, 30.0, wp::ZOOM);
    let (south, west, north, east) = tile_bounds(tile.0, tile.1, wp::ZOOM);
    assert!(north - south < 0.16);
    assert!(east - west < 0.16);
}

#[test]
fn titles_are_quoted() {
    assert_eq!(wp::page_url("en", "Perm Opera and Ballet Theatre"), "https://en.wikipedia.org/wiki/Perm_Opera_and_Ballet_Theatre");
    assert!(wp::summary_url("en", "AC/DC").contains("/page/summary/AC%2FDC"));
    assert!(wp::page_url("ru", "Пермь").contains("ru.wikipedia.org/wiki/%D0%9F"));
    assert_eq!(wp::summary_url("xx/../", "a~b.c-d_e"), "https://en.wikipedia.org/api/rest_v1/page/summary/a~b.c-d_e");
}

#[test]
fn language() {
    assert_eq!(wp::language("ru"), "ru");
    assert_eq!(wp::language("RU"), "ru");
    assert_eq!(wp::language(""), "en");
    assert_eq!(wp::language("ru/../x"), "en");
    assert_eq!(wp::language("abcd"), "en");
}

#[test]
fn view_tiles_near_and_far() {
    let tiles = wp::view_tiles(58.01, 56.23, 5000.0, wp::VIEW_TILES);
    assert_eq!(tiles.len(), wp::VIEW_TILES);
    assert_eq!(tiles[0], tile_of(58.01, 56.23, wp::ZOOM));
    let mut unique = tiles.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), tiles.len());
    assert!(wp::view_tiles(58.01, 56.23, wp::MAX_ALTITUDE + 1.0, wp::VIEW_TILES).is_empty());
    assert!(wp::view_tiles(58.01, 56.23, f64::NAN, wp::VIEW_TILES).is_empty());
}

#[test]
fn view_tiles_wrap_date_line_and_stop_at_pole() {
    let n = 1u32 << wp::ZOOM;
    let tiles = wp::view_tiles(0.0, 179.99, 1000.0, wp::VIEW_TILES);
    assert_eq!(tiles.len(), wp::VIEW_TILES);
    assert!(tiles.iter().any(|t| t.0 == 0) && tiles.iter().all(|t| t.0 < n));
    // У полюса строк выше верхней нет: 3 строки по 5 тайлов.
    assert_eq!(wp::view_tiles(89.0, 0.0, 1000.0, wp::VIEW_TILES).len(), 15);
}

#[test]
fn order_rank_then_distance_without_repeats() {
    let article = |pageid, lat, lon, rank| Article { pageid, title: String::new(), lat, lon, rank };
    let a = article(1, 58.5, 56.2, 3);
    let b = article(2, 58.01, 56.23, 5);
    let c = article(3, 58.02, 56.23, 3);
    let got = wp::order(&[a.clone(), b.clone(), c.clone(), b.clone()], 58.01, 56.23, wp::MAX_MARKS);
    assert_eq!(got.iter().map(|x| x.pageid).collect::<Vec<_>>(), [3, 1, 2]);
    assert_eq!(wp::order(&[a, b, c], 58.0, 56.0, 2).len(), 2);
}

#[test]
fn hostile_records_are_skipped() {
    let answer = r#"{"query": {"geosearch": [1, "x", null,
        {"pageid": "8", "title": "  Строка  ", "lat": "58.1", "lon": "56.1"},
        {"pageid": 9, "title": "Бесконечность", "lat": "nan", "lon": 0},
        {"pageid": 10, "title": "За полюсом", "lat": 91, "lon": 0}]}}"#;
    let found = wp::parse_geosearch(answer.as_bytes());
    assert!(found.is_ok(), "{found:?}");
    let Ok(found) = found else { return };
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].pageid, found[0].title.as_str()), (8, "Строка"));
    assert_eq!(wp::parse_geosearch(br#"{"query": {"geosearch": {}}}"#), Ok(Vec::new()));
}
