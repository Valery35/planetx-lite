// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Свои пункты меню на глобусе. Перенос `tests/test_menulinks.py` модуля.

use data::menulinks::{self as ml, EXAMPLE, PRESETS};

fn pair(name: &str, template: &str) -> (String, String) {
    (name.to_string(), template.to_string())
}

#[test]
fn presets_are_valid() {
    for (ru, _en, template) in PRESETS {
        assert_eq!(ml::check(ru, template), None, "{ru}");
        let url = ml::fill(template, 58.0, 56.2, 12.0);
        assert!(url.as_deref().is_ok_and(|u| u.contains("58.000000")), "{url:?}");
    }
}

#[test]
fn fill_puts_point_and_zoom() {
    let url = ml::fill("https://example.org/?q={lat},{lon}&z={zoom}", 58.0105, 56.2294, 12.6);
    assert_eq!(url, Ok("https://example.org/?q=58.010500,56.229400&z=12".to_string()));
    let url = ml::fill(EXAMPLE.1, 1.5, -2.25, 9.0).unwrap_or_default();
    assert!(url.starts_with("https://embed.windy.com/"));
    assert!(url.contains("metricTemp=%C2%B0C"));
    assert!(url.contains("&zoom=9&lat=1.500000&lon=-2.250000&detailLat=1.500000&detailLon=-2.250000"));
}

#[test]
fn doubled_braces_are_literal() {
    assert_eq!(ml::fill(" https://x/{{a}}/{lat} ", 1.0, 2.0, 3.0), Ok("https://x/{a}/1.000000".to_string()));
    assert!(ml::fill("https://x/}", 1.0, 2.0, 3.0).is_err());
    assert!(ml::fill("https://x/{}", 1.0, 2.0, 3.0).is_err());
    assert!(ml::fill("https://x/{a{lat}", 1.0, 2.0, 3.0).is_err());
}

#[test]
fn check_names_the_reason() {
    let good = "https://example.org/{lat}/{lon}";
    assert_eq!(ml::check("Пример", good), None);
    assert_eq!(ml::check(" ", good), Some("name"));
    assert_eq!(ml::check("Пример", "ftp://x/{lat}/{lon}"), Some("scheme"));
    assert_eq!(ml::check("Пример", "file:///{lat}/{lon}"), Some("scheme"));
    assert_eq!(ml::check("Пример", "HTTPS://x/{lat}/{lon}"), None);
    assert_eq!(ml::check("Пример", "https://x/{lat}"), Some("place"));
    assert_eq!(ml::check("Пример", "https://x/{lat}/{lon}/{alt}"), Some("braces"));
    assert_eq!(ml::check("Пример", "https://x/{lat}/{lon}/{"), Some("braces"));
}

#[test]
fn settings_round_trip_skips_bad_items() {
    let items = vec![pair("Windy", EXAMPLE.1), pair("Карта", "https://x/{lat}/{lon}")];
    assert_eq!(ml::load(&ml::dump(&items)), items);
    let mut more = items.clone();
    more.push(pair("Плохой", "javascript:{lat}{lon}"));
    assert_eq!(ml::load(&ml::dump(&more)), items);
    assert_eq!(ml::load("{не json"), Vec::new());
    assert_eq!(ml::load(r#"{"a": 1}"#), Vec::new());
    assert_eq!(ml::load(""), Vec::new());
    assert_eq!(ml::load(r#"[["a", "https://x/{lat}/{lon}", "c"], [1, 2], " b ", [" Имя ", " https://x/{lat}/{lon} "]]"#), vec![pair("Имя", "https://x/{lat}/{lon}")]);
}

#[test]
fn dump_keeps_cyrillic() {
    assert!(ml::dump(&[pair("Карта", "https://x/{lat}/{lon}")]).contains("Карта"));
}
