// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Горные разработки: тайлы вида, запрос Overpass, разбор ответа.
//! Перенос `tests/test_mining.py` модуля.

use data::{deposits, mining};

fn near() -> Vec<(u32, u32)> {
    mining::view_tiles(59.41, 56.80, 30000.0, mining::ZOOM, mining::MAX_TILES)
}

#[test]
fn tile_contains_point() {
    let (x, y) = mining::tile_of(59.41, 56.80, mining::ZOOM);
    let (south, west, north, east) = mining::tile_bounds(x, y, mining::ZOOM);
    assert!(south <= 59.41 && 59.41 <= north);
    assert!(west <= 56.80 && 56.80 <= east);
    // Долгота замкнута, широта у полюса - крайний ряд.
    assert_eq!(mining::tile_of(89.9, 180.0, 2), (0, 0));
    // Целая часть - к нулю, как int() модуля.
    assert_eq!(mining::tile_of(-89.9, -181.0, 2), (0, 3));
    assert_eq!(mining::tile_of(-89.9, -271.0, 2), (3, 3));
}

#[test]
fn view_tiles_near_only() {
    let tiles = near();
    assert_eq!(tiles.len(), mining::MAX_TILES);
    assert_eq!(tiles.first(), Some(&mining::tile_of(59.41, 56.80, mining::ZOOM)));
    assert!(mining::view_tiles(59.41, 56.80, 500000.0, mining::ZOOM, mining::MAX_TILES).is_empty());
    // Ряд у полюса: соседей сверху нет.
    assert_eq!(mining::view_tiles(85.05, 0.0, 1000.0, mining::ZOOM, mining::MAX_TILES).len(), 6);
}

#[test]
fn union_of_view_tiles() {
    let tiles = near();
    let boxes: Vec<_> = tiles.iter().map(|t| mining::tile_bounds(t.0, t.1, mining::ZOOM)).collect();
    let got = mining::union(&boxes);
    assert!(got.is_some());
    let Some(b) = got else { return };
    for (south, west, north, east) in boxes {
        assert!(b.0 <= south && b.2 >= north);
        assert!(b.1 <= west && b.3 >= east);
    }
    // Через линию перемены дат - первая рамка.
    assert_eq!(mining::union(&[(0.0, 179.0, 1.0, 180.0), (0.0, -180.0, 1.0, -179.0)]), Some((0.0, 179.0, 1.0, 180.0)));
    assert_eq!(mining::union(&[]), None);
}

#[test]
fn query_has_all_kinds_and_box() {
    let text = mining::query((59.0, 56.0, 60.0, 57.0));
    assert!(text.contains(r#"nwr["landuse"="quarry"](59.00000,56.00000,60.00000,57.00000);"#));
    assert!(text.contains(r#"nwr["man_made"="mineshaft"]"#));
    assert!(text.starts_with("[out:json][timeout:60]"));
    assert!(text.ends_with("out center tags;"));
}

const ANSWER: &str = r#"{"elements": [
 {"type": "way", "id": 1, "center": {"lat": 59.4, "lon": 56.8},
  "tags": {"landuse": "quarry", "name": "Карьер", "resource": "sand;gravel"}},
 {"type": "node", "id": 2, "lat": 59.6, "lon": 56.7,
  "tags": {"man_made": "mine", "name:ru": "Рудник СКРУ-1", "name": "SKRU-1", "resource": "potash", "operator": "Уралкалий"}},
 {"type": "way", "id": 1, "center": {"lat": 59.4, "lon": 56.8}, "tags": {"landuse": "quarry"}},
 {"type": "relation", "id": 3, "tags": {"landuse": "quarry"}}]}"#;

#[test]
fn parse_and_points() {
    let items = mining::parse(ANSWER.as_bytes());
    assert_eq!(items.len(), 3); // без середины - пропущен
    assert_eq!(items[1].name, "Рудник СКРУ-1");
    assert_eq!(items[0].resource, "sand, gravel");
    assert_eq!(items[0].kind, "quarry");
    assert_eq!(items[2].osm, "way/1");
    let data = mining::to_deposits(&items);
    assert_eq!(data.len(), 2); // повтор way/1 отброшен
    assert_eq!(deposits::GROUPS[data[1].group].0, "industrial"); // калий
    let fields = data[1].fields();
    assert_eq!(fields.len(), 10);
    assert_eq!(fields[6], "Уралкалий");
    assert_eq!(fields[9], "node/2");
    assert_eq!(mining::osm_page(fields[9]).as_deref(), Some("https://www.openstreetmap.org/node/2"));
}

#[test]
fn hostile_answer() {
    assert!(mining::parse(b"[1, 2]").is_empty());
    assert!(mining::parse(b"not json").is_empty());
    assert!(mining::parse(br#"{"elements": [5, {"lat": "x", "lon": 1}, {"lat": 1, "lon": 2, "tags": []}]}"#).len() == 1);
    assert_eq!(mining::osm_page("node"), None);
}
