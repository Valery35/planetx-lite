// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! ЛЭП и трубопроводы OSM: напряжение, классы, запрос и разбор ответа
//! Overpass, тайлы вида. Перенос `tests/test_infra.py` модуля и проверки
//! общего кода Overpass `ui/overpass.py`.

use data::infra::{self, Kind};

const ANSWER: &str = r#"{"elements": [
    {"type": "way", "id": 1,
     "geometry": [{"lat": 59.0, "lon": 56.0}, {"lat": 59.1, "lon": 56.1}],
     "tags": {"power": "line", "voltage": "500000", "name": "ВЛ 500 кВ"}},
    {"type": "way", "id": 2,
     "geometry": [{"lat": 59.0, "lon": 56.0}, {"lat": 59.2, "lon": 56.2}],
     "tags": {"man_made": "pipeline", "substance": "gas", "operator": "Газпром"}},
    {"type": "way", "id": 3, "geometry": [{"lat": 59.0, "lon": 56.0}],
     "tags": {"power": "line"}},
    {"type": "way", "id": 4, "center": {"lat": 59.3, "lon": 56.3},
     "tags": {"power": "substation", "voltage": "110000"}},
    {"type": "way", "id": 1,
     "geometry": [{"lat": 59.0, "lon": 56.0}, {"lat": 59.1, "lon": 56.1}],
     "tags": {"power": "line"}},
    {"type": "way", "id": 5,
     "geometry": [{"lat": 59.0, "lon": 56.0}, {"lat": 59.0, "lon": 56.3}],
     "tags": {"power": "cable", "voltage": "220000", "location": "underwater"}},
    {"type": "way", "id": 6,
     "geometry": [{"lat": 59.0, "lon": 56.0}, {"lat": 59.4, "lon": 56.0}],
     "tags": {"communication": "line"}}]}"#;

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

#[test]
fn kilovolts() {
    assert_eq!(infra::kilovolts("500000"), Some(500.0));
    assert_eq!(infra::kilovolts("220000;110000"), Some(220.0));
    assert_eq!(infra::kilovolts("110 kV"), Some(110.0));
    assert_eq!(infra::kilovolts("10000,35 KV"), Some(35.0));
    assert_eq!(infra::kilovolts(""), None);
    assert_eq!(infra::kilovolts("высокое"), None);
    assert_eq!(infra::kilovolts("1e6"), None);
}

#[test]
fn classes() {
    assert_eq!(infra::voltage_class(Some(500.0)), "v500");
    assert_eq!(infra::voltage_class(Some(330.0)), "v220");
    assert_eq!(infra::voltage_class(Some(110.0)), "v110");
    assert_eq!(infra::voltage_class(Some(35.0)), "v35");
    assert_eq!(infra::voltage_class(None), "v35");
    assert_eq!(infra::substance_class("gas"), "gas");
    assert_eq!(infra::substance_class("Crude_Oil"), "oil");
    assert_eq!(infra::substance_class(""), "other");
}

#[test]
fn query_and_parse() {
    let text = infra::query((59.0, 56.0, 60.0, 57.0));
    assert!(text.contains(r#"way["power"="line"](59.00000,56.00000,60.00000,57.00000);"#));
    assert!(text.contains("out geom tags;"));
    assert!(text.contains(r#"way["power"="cable"]"#));
    // Текст запроса - как у модуля, знак в знак.
    let b = "(59.00000,56.00000,60.00000,57.00000)";
    assert_eq!(
        text,
        format!(
            "[out:json][timeout:90];(way[\"power\"=\"line\"]{b};way[\"man_made\"=\"pipeline\"]{b};\
             way[\"power\"=\"cable\"]{b};way[\"communication\"=\"line\"]{b};);out geom tags;\
             (nwr[\"power\"=\"substation\"]{b};);out center tags;"
        )
    );
    let items = infra::unique(infra::parse(ANSWER.as_bytes()));
    let kinds: Vec<&str> = items.iter().map(|i| i.kind.name()).collect();
    assert_eq!(kinds, ["line", "pipe", "substation", "cable", "telecom"]);
    assert_eq!(items[3].location, "underwater");
    assert_eq!(items[3].voltage, Some(220.0));
    assert_eq!(items[3].style, "cable");
    assert_eq!(items[0].style, "v500");
    assert_eq!(items[0].name, "ВЛ 500 кВ");
    assert_eq!(items[0].osm, "way/1");
    assert_eq!(items[1].style, "gas");
    assert_eq!(items[1].operator, "Газпром");
    assert_eq!(items[2].style, "v110");
    assert_eq!(items[2].kind, Kind::Substation);
    assert_eq!(items[2].points, [(59.3, 56.3)]);
}

#[test]
fn hostile_answers() {
    assert!(infra::parse(b"not json").is_empty());
    assert!(infra::parse(b"[]").is_empty());
    assert!(infra::parse(br#"{"elements": {}}"#).is_empty());
    let answer = r#"{"elements": [7, null,
        {"type": "node", "id": 9, "lat": 59.0, "lon": 56.0, "tags": {"power": "substation"}},
        {"type": "node", "id": 10, "lat": 59.0, "tags": {"power": "substation"}},
        {"type": "way", "id": 11, "geometry": [null, {"lat": 59.0, "lon": 56.0}, {"lat": 59.1}, {"lat": 59.2, "lon": 56.2}],
         "tags": {"power": "line", "name": "A", "name:ru": "Б"}},
        {"type": "way", "id": 12, "geometry": [{"lat": 59.0, "lon": 56.0}, {"lat": 59.2, "lon": 56.2}],
         "tags": {"highway": "road"}}]}"#;
    let items = infra::parse(answer.as_bytes());
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].kind, Kind::Substation);
    assert_eq!(items[0].osm, "node/9");
    assert_eq!(items[1].points.len(), 2);
    assert_eq!(items[1].name, "Б");
}

#[test]
fn overpass_tiles_and_box() {
    let (x, y) = infra::tile_of(58.01, 56.23, infra::ZOOM);
    let (south, west, north, east) = infra::tile_bounds(x, y, infra::ZOOM);
    assert!(south <= 58.01 && 58.01 <= north && west <= 56.23 && 56.23 <= east);
    let tiles = infra::view_tiles(58.01, 56.23, 1000.0, infra::ZOOM, infra::MAX_TILES);
    assert_eq!(tiles.len(), infra::MAX_TILES);
    assert_eq!(tiles[0], (x, y));
    assert!(infra::view_tiles(58.01, 56.23, infra::MAX_ALTITUDE + 1.0, infra::ZOOM, infra::MAX_TILES).is_empty());
    let boxes: Vec<_> = tiles.iter().map(|t| infra::tile_bounds(t.0, t.1, infra::ZOOM)).collect();
    let all = infra::union(&boxes);
    assert!(all.is_some());
    let Some(all) = all else { return };
    close(all.1, infra::tile_bounds(x - 1, y, infra::ZOOM).1, 1e-12);
    close(all.2, infra::tile_bounds(x, y - 1, infra::ZOOM).2, 1e-12);
    assert_eq!(infra::union(&[]), None);
    // Тайлы через линию перемены дат - первая рамка.
    let date_line = [infra::tile_bounds(1023, 300, infra::ZOOM), infra::tile_bounds(0, 300, infra::ZOOM)];
    assert_eq!(infra::union(&date_line), Some(date_line[0]));
}

#[test]
fn overpass_form_body() {
    // urlencode модуля: пробел - «+», прочее вне букв, цифр и «_.-~» -
    // %XX.
    assert_eq!(infra::form_body("[out:json];a b\"=é"), "data=%5Bout%3Ajson%5D%3Ba+b%22%3D%C3%A9");
    assert_eq!(infra::SERVERS[0], "https://overpass-api.de/api/interpreter");
}

#[test]
fn brief_error() {
    assert_eq!(
        infra::brief_error("Error transferring https://overpass-api.de/api/interpreter?data=%5B - server replied: Gateway"),
        "Error transferring overpass-api.de - server replied: Gateway"
    );
    assert_eq!(infra::brief_error("http://a.b/c http://d.e"), "a.b d.e");
    assert_eq!(infra::brief_error("http:///x"), "http:///x");
    assert_eq!(infra::brief_error(""), "");
}
