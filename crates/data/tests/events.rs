// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Проверки `events.rs`, перенос TestCatalog из `tests/test_events.py`
//! модуля и проверки враждебных документов.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use data::events::{self, Item};
use data::gibs::Day;
use serde_json::{Value, json};

fn vantor_item() -> Value {
    json!({
        "type": "Feature", "id": "103001012263FC00",
        "bbox": [-5.35, 36.65, -4.96, 36.86],
        "assets": {"visual": {
            "href": "https://example.org/events/E/103001012263FC00.tif",
            "type": "image/tiff; application=geotiff; profile=cloud-optimized"}},
        "properties": {"datetime": "2026-02-15T11:41:24.955033Z",
                       "phase": "post", "pan_gsd": 0.83, "vehicle_name": "WV02"}})
}

fn planet_item() -> Value {
    json!({
        "type": "Feature", "id": "585-1059", "bbox": [-77.17, 6.14, -76.99, 6.31],
        "assets": {"visual": {"href": "./585-1059_visual.tif",
                              "type": "image/tiff; application=geotiff"}},
        "properties": {"datetime": "2026-05-16T12:00:00Z", "gsd": 4.77,
                       "pl:mosaic": "global_quarterly_2026q2_mosaic"}})
}

#[test]
fn vantor_item_fields() {
    let item = events::parse_item(&vantor_item(), "https://example.org/e.json", events::VANTOR, "").unwrap();
    assert_eq!(item.day, "2026-02-15");
    assert_eq!(item.phase, events::POST);
    assert!((item.gsd - 0.83).abs() < 1e-12);
    assert_eq!(item.platform, "WV02");
    assert_eq!(item.ident, "103001012263FC00");
    assert_eq!(item.source, events::VANTOR);
    assert_eq!(item.bbox, [-5.35, 36.65, -4.96, 36.86]);
    assert_eq!(item.time, Day::parse("2026-02-15").unwrap().seconds() + 11.0 * 3600.0 + 41.0 * 60.0 + 24.0);
}

#[test]
fn planet_item_phase_from_path_and_relative_href() {
    let url = "https://data.example/planet/e/pre-event/basemap/items/585-1059/585-1059.json";
    let item = events::parse_item(&planet_item(), url, events::PLANET, events::phase_of(url)).unwrap();
    assert_eq!(item.phase, events::PRE);
    assert_eq!(item.href, "https://data.example/planet/e/pre-event/basemap/items/585-1059/585-1059_visual.tif");
    assert_eq!(item.platform, "Planet basemap");
    assert_eq!(events::phase_of("https://x/post-event/a.json"), events::POST);
    assert_eq!(events::phase_of("https://x/a.json"), "");
}

#[test]
fn item_without_visual_is_skipped() {
    let mut doc = vantor_item();
    doc["assets"] = json!({"thumbnail": {"href": "t.png"}});
    assert!(events::parse_item(&doc, "u", events::VANTOR, "").is_none());
    // Картинка не в TIFF, без момента, без рамки - тоже мимо.
    let mut doc = vantor_item();
    doc["assets"]["visual"]["type"] = json!("image/png");
    assert!(events::parse_item(&doc, "u", events::VANTOR, "").is_none());
    let mut doc = vantor_item();
    doc["properties"]["datetime"] = json!(null);
    assert!(events::parse_item(&doc, "u", events::VANTOR, "").is_none());
    let mut doc = vantor_item();
    doc["bbox"] = json!([1, 2, 3]);
    assert!(events::parse_item(&doc, "u", events::VANTOR, "").is_none());
}

#[test]
fn item_numbers_are_tolerant() {
    let mut doc = vantor_item();
    doc["bbox"] = json!([-5, "36.65", -4.96, 37]);
    doc["properties"] = json!({"start_datetime": "2026-02-15", "gsd": "0.5", "constellation": "worldview"});
    let item = events::parse_item(&doc, "u", events::VANTOR, "pre").unwrap();
    assert_eq!(item.bbox, [-5.0, 36.65, -4.96, 37.0]);
    assert_eq!(item.gsd, 0.5);
    assert_eq!(item.platform, "worldview");
    assert_eq!(item.phase, "pre");
    assert_eq!(item.time, Day::parse("2026-02-15").unwrap().seconds());
}

#[test]
fn event_title_and_day() {
    let doc = json!({"type": "Catalog", "id": "colombia-earthquake-2026-08-10",
                     "title": "Planet Crisis Response — Colombia Earthquake (2026)", "description": "M7.4"});
    let event = events::parse_event(&doc, "u", events::PLANET).unwrap();
    assert_eq!(event.title, "Colombia Earthquake (2026)");
    assert_eq!(event.day, "2026-08-10");
    assert_eq!(event.description, "M7.4");
    let mut doc = json!({"type": "Collection", "id": "Spain-Portugal-Flooding-Feb-2026",
                         "title": "", "extent": {
                             "spatial": {"bbox": [[-5.3, 36.6, -4.9, 36.8]]},
                             "temporal": {"interval": [["2026-02-15T11:41Z", null]]}}});
    let event = events::parse_event(&doc, "u", events::VANTOR).unwrap();
    assert_eq!(event.title, "Spain Portugal Flooding Feb 2026");
    assert_eq!(event.day, "2026-02-15");
    assert_eq!(event.bbox, Some([-5.3, 36.6, -4.9, 36.8]));
    assert_eq!(event.key, "vantor/Spain-Portugal-Flooding-Feb-2026");
    // Название, равное id, - тоже без дефисов.
    doc["title"] = doc["id"].clone();
    assert_eq!(events::parse_event(&doc, "u", events::VANTOR).unwrap().title, "Spain Portugal Flooding Feb 2026");
    // Дефис и двоеточие после начала Planet тоже снимаются.
    let doc = json!({"type": "Catalog", "id": "x", "title": "Planet Crisis Response: Fire"});
    assert_eq!(events::parse_event(&doc, "u", events::PLANET).unwrap().title, "Fire");
    let doc = json!({"type": "Catalog", "id": "x", "title": "Planet Crisis Response-Fire"});
    assert_eq!(events::parse_event(&doc, "u", events::PLANET).unwrap().title, "Fire");
    assert!(events::parse_event(&json!({"type": "Feature"}), "u", events::PLANET).is_none());
    // Без id ключ - по адресу.
    let event = events::parse_event(&json!({"type": "Catalog"}), "https://h/c.json", events::PLANET).unwrap();
    assert_eq!(event.key, "planet/https://h/c.json");
    assert_eq!(event.bbox, None);
}

#[test]
fn links_are_absolute() {
    let doc = json!({"links": [{"rel": "child", "href": "./a/catalog.json", "title": "A"},
                               {"rel": "item", "href": "../b.json"},
                               {"rel": "license", "href": "https://x/l"},
                               {"rel": "child", "href": ""},
                               {"rel": "child", "href": 5},
                               "junk"]});
    let found = events::children(&doc, "https://h/root/sub/catalog.json");
    assert_eq!(found, vec![("child".to_string(), "https://h/root/sub/a/catalog.json".to_string()), ("item".to_string(), "https://h/root/b.json".to_string())]);
    let parsed = events::parse_events(&doc, "https://h/root/sub/catalog.json");
    assert_eq!(parsed, vec![("https://h/root/sub/a/catalog.json".to_string(), "A".to_string())]);
    assert_eq!(events::links(&doc, "https://h/").len(), 3);
    assert!(events::links(&json!({"links": 3}), "https://h/").is_empty());
}

#[test]
fn join_like_urljoin() {
    let base = "https://h/a/b/c.json?q=1#f";
    for (href, want) in [
        ("https://o/x.json", "https://o/x.json"),
        ("/x.json", "https://h/x.json"),
        ("./x.json", "https://h/a/b/x.json"),
        ("x.json", "https://h/a/b/x.json"),
        ("../x.json", "https://h/a/x.json"),
        ("../../../../x.json", "https://h/x.json"),
        ("x.json#part", "https://h/a/b/x.json"),
        ("//o/y/../z", "https://o/z"),
        ("?p=2", "https://h/a/b/c.json?p=2"),
        ("d/./e/../f?k=v", "https://h/a/b/d/f?k=v"),
        ("", "https://h/a/b/c.json?q=1"),
        ("..", "https://h/a/"),
    ] {
        assert_eq!(events::join_url(base, href), want, "{href}");
    }
    assert_eq!(events::join_url("https://h", "x"), "https://h/x");
    assert_eq!(events::join_url("u", "x"), "x");
    // Враждебные строки - без паники.
    for s in ["", "#", "?", "://", ":", "//", "../..", "ы://ж/../", "https://", "a:b:c", "%%%/..//.."] {
        for t in ["", "#", "?", "://", "//", "../..", "ж/./..", "/../..", "x:/y"] {
            let _ = events::join_url(s, t);
        }
    }
}

#[test]
fn moment_of_stamps() {
    let day = Day::parse("2026-02-15").unwrap().seconds();
    assert_eq!(events::moment("2026-02-15"), Some(day));
    assert_eq!(events::moment("2026-02-15T01:02:03Z"), Some(day + 3723.0));
    assert_eq!(events::moment("2026-02-15T11:41Z"), Some(day));
    assert_eq!(events::moment("2026-02-30"), None);
    assert_eq!(events::moment("2026-02-15T25:00:00"), None);
    assert_eq!(events::moment("x2026-02-15"), None);
    assert_eq!(events::moment(""), None);
    assert_eq!(events::moment("2026-02-1ж"), None);
}

fn item(day: &str, phase: &str, gsd: f64, t: f64) -> Item {
    Item {
        ident: format!("{day}{phase}"),
        source: events::PLANET,
        day: day.to_string(),
        time: t,
        phase: phase.to_string(),
        bbox: [0.0, 0.0, 1.0, 1.0],
        href: "h".to_string(),
        gsd,
        platform: String::new(),
    }
}

#[test]
fn days_order_and_level() {
    let items = [item("2026-05-16", "pre", 4.77, 1.0), item("2026-08-10", "post", 3.8, 2.0), item("2026-08-11", "post", 0.66, 3.0)];
    assert_eq!(events::pre_day(&items).as_deref(), Some("2026-05-16"));
    assert_eq!(events::post_day(&items).as_deref(), Some("2026-08-11"));
    assert_eq!(events::intervals(&items).len(), 3);
    let d = Day::parse("2026-08-10").unwrap();
    assert!(events::intervals(&items).contains(&(d, d, 1, 'D')));
    assert_eq!(events::days(&items), vec!["2026-05-16", "2026-08-10", "2026-08-11"]);
    let shown: Vec<&str> = events::shown(&items, "2026-08-10").iter().map(|i| i.day.as_str()).collect();
    assert_eq!(shown, vec!["2026-08-10", "2026-05-16"]);
    // 0.66 м у экватора - уровень 18 (0.6 м).
    assert_eq!(events::max_level(&items), 18);
    assert_eq!(events::max_level(&[]), events::MAX_LEVEL);
    assert_eq!(events::max_level(&[item("2026-01-01", "", 0.0, 0.0)]), events::MAX_LEVEL);
    assert_eq!(events::bbox_of(&items), Some([0.0, 0.0, 1.0, 1.0]));
    assert_eq!(events::bbox_of(&[]), None);
    // Только до события - день события по всем снимкам.
    let pre = [item("2026-05-16", "pre", 1.0, 1.0)];
    assert_eq!(events::post_day(&pre).as_deref(), Some("2026-05-16"));
    assert_eq!(events::pre_day(&items[1..]), None);
}

#[test]
fn shown_same_moment_order() {
    // При равном моменте: без размера пикселя, затем подробнее.
    let items = [item("2026-01-01", "", 3.0, 5.0), item("2026-01-01", "", 0.5, 5.0), item("2026-01-01", "", 0.0, 5.0), item("2026-01-01", "", 1.0, 9.0)];
    let gsd: Vec<f64> = events::shown(&items, "2026-12-31").iter().map(|i| i.gsd).collect();
    assert_eq!(gsd, vec![1.0, 0.0, 0.5, 3.0]);
}

#[test]
fn tiles_and_touch() {
    let b = events::tile_box(0, 0, 0);
    assert_eq!((b[0], b[2]), (-180.0, 180.0));
    assert!((b[3] - 85.0511287798).abs() < 1e-9 && (b[1] + 85.0511287798).abs() < 1e-9);
    let b = events::tile_box(1, 1, 0);
    assert_eq!((b[0], b[1], b[2]), (0.0, 0.0, 180.0));
    assert!(events::touches([0.0, 0.0, 1.0, 1.0], [1.0, 1.0, 2.0, 2.0]));
    assert!(!events::touches([0.0, 0.0, 1.0, 1.0], [1.5, 0.0, 2.0, 1.0]));
    let _ = events::tile_box(64, u32::MAX, u32::MAX);
}

#[test]
fn names_and_credits() {
    assert_eq!(events::source_name(events::VANTOR), "Vantor");
    assert_eq!(events::source_name(events::PLANET), "Planet");
    assert_eq!(events::credit(events::PLANET).0, "Planet Labs PBC, CC BY-NC 4.0");
    assert_eq!(events::ROOTS[0].0, events::VANTOR);
}

#[test]
fn hostile_documents_never_panic() {
    let docs = [
        json!(null),
        json!(5),
        json!("x"),
        json!([]),
        json!({"type": "Feature", "assets": 3}),
        json!({"type": "Feature", "assets": {"visual": {"type": 7, "href": [1]}}, "properties": {"datetime": 5}}),
        json!({"type": "Feature", "assets": {"visual": {"type": "tiff", "href": "a"}}, "properties": {"datetime": "2026-01-01"}, "bbox": ["a", 1, 2, 3]}),
        json!({"type": "Collection", "id": 12, "title": 7, "extent": {"temporal": {"interval": "x"}, "spatial": {"bbox": [1, 2]}}}),
        json!({"type": "Collection", "extent": {"temporal": {"interval": [[]]}, "spatial": {"bbox": [[1, 2, "x", 4]]}}}),
        json!({"type": "Catalog", "extent": [1], "links": [{"href": {}}, null]}),
    ];
    for doc in &docs {
        let _ = events::parse_event(doc, "u", events::VANTOR);
        let _ = events::parse_item(doc, "u", events::VANTOR, "");
        let _ = events::links(doc, "u");
    }
    let event = events::parse_event(&docs[7], "u", events::VANTOR).unwrap();
    assert_eq!(event.key, "vantor/12");
    assert_eq!(event.title, "7");
    assert_eq!(event.bbox, None);
}
