// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Поиск места через Nominatim: адрес запроса и разбор ответа, перенос
//! `tests/test_geocode.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;

use data::geocode as gc;
use serde_json::json;

/// Сокращённый ответ Nominatim jsonv2 на запрос «Пермь».
fn perm() -> serde_json::Value {
    json!([
        {"lat": "58.0103211", "lon": "56.2341778", "name": "Пермь",
         "display_name": "Пермь, городской округ Пермь, Пермский край, Приволжский федеральный округ, Россия",
         "boundingbox": ["57.8637", "58.1766", "55.8046", "56.6573"]},
        {"lat": "59.3", "lon": "56.6", "name": "Пермский край",
         "display_name": "Пермский край, Россия",
         "boundingbox": ["56.1", "61.6", "51.8", "59.6"]},
        {"lon": "1.0", "name": "без широты"}
    ])
}

/// Разбор строки запроса: «+» - пробел, %XX - байты UTF-8.
fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                out.push(u8::from_str_radix(&value[i + 1..i + 3], 16).unwrap());
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8(out).unwrap()
}

fn query(url: &str) -> HashMap<String, String> {
    let (_, q) = url.split_once('?').unwrap();
    let q = q.split('#').next().unwrap();
    q.split('&').map(|p| p.split_once('=').unwrap()).map(|(k, v)| (k.to_string(), decode(v))).collect()
}

#[test]
fn query_and_language() {
    let q = query(&gc::search_url("  Пермь   край ", Some("ru"), gc::SEARCH_LIMIT));
    assert_eq!(q["q"], "Пермь край");
    assert_eq!(q["accept-language"], "ru");
    assert_eq!(q["format"], "jsonv2");
    assert_eq!(q["limit"], gc::SEARCH_LIMIT.to_string());
}

#[test]
fn local_names_send_no_language() {
    assert!(!query(&gc::search_url("Perm", None, gc::SEARCH_LIMIT)).contains_key("accept-language"));
}

#[test]
fn goes_to_nominatim() {
    assert!(gc::search_url("x", None, gc::SEARCH_LIMIT).starts_with(&format!("{}?", gc::NOMINATIM)));
}

#[test]
fn places_in_answer_order() {
    let places = gc::parse_places(&perm());
    assert_eq!(places.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Пермь", "Пермский край"]);
}

#[test]
fn detail_drops_name() {
    let perm = &gc::parse_places(&perm())[0];
    assert!(perm.detail.starts_with("городской округ Пермь"));
    assert_eq!(gc::place_text(perm).split(", ").next(), Some("Пермь"));
}

#[test]
fn box_is_west_south_east_north() {
    let perm = &gc::parse_places(&perm())[0];
    assert_eq!(perm.bbox, Some((55.8046, 57.8637, 56.6573, 58.1766)));
    assert!((perm.lat - 58.0103211).abs() < 1e-9);
}

#[test]
fn bad_answers() {
    assert!(gc::parse_places(&serde_json::Value::Null).is_empty());
    assert!(gc::parse_places(&json!({"error": "x"})).is_empty());
    let broken = json!([{"lat": "1", "lon": "2", "display_name": "A, B", "boundingbox": ["x"]}]);
    let place = &gc::parse_places(&broken)[0];
    assert_eq!((place.name.as_str(), place.detail.as_str(), place.bbox), ("A", "B", None));
}

#[test]
fn reverse_url() {
    let url = gc::reverse_url(58.0105, 56.2294, Some("ru"), gc::REVERSE_ZOOM);
    let q = query(&url);
    assert!(url.starts_with(gc::REVERSE));
    assert_eq!(q["lat"], "58.010500");
    assert_eq!(q["lon"], "56.229400");
    assert_eq!(q["accept-language"], "ru");
    assert_eq!(q["format"], "jsonv2");
}

#[test]
fn parse_reverse() {
    assert_eq!(gc::parse_reverse(&json!({"display_name": "Эспланада, Пермь, Россия"})), "Эспланада, Пермь, Россия");
    assert_eq!(gc::parse_reverse(&json!({"error": "Unable to geocode"})), "");
    assert_eq!(gc::parse_reverse(&json!([])), "");
}

#[test]
fn osm_link_zoom_follows_view() {
    // Полоса 1 км на экваторе в окне 1000 пикселей - метр на пиксель,
    // уровень 17. Полоса 1000 км - уровень 7.
    assert_eq!(gc::link_zoom(0.0, 1000.0), 17);
    assert_eq!(gc::link_zoom(0.0, 1e6), 7);
    assert_eq!(gc::link_zoom(0.0, 1e9), 1);
    assert_eq!(gc::link_zoom(0.0, 0.1), 19);
    let link = gc::osm_link(58.0105, 56.2294, 3000.0);
    assert!(link.starts_with("https://www.openstreetmap.org/?mlat=58.010500&mlon=56.229400#map="));
    assert!(link.ends_with("/58.01050/56.22940"));
}
