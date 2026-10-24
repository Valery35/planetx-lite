// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Геологическая карта Macrostrat: ответ о породе, уровни запроса.
//! Перенос `tests/test_geology.py` модуля.

use data::geology;

/// Ответ map_query у Перми 17 октября 2026 года, сокращён.
const ANSWER: &str = r##"{"success": {"v": 2, "license": "CC-BY 4.0", "data": {
 "elevation": 166, "burwell": [{"map_id": 3154618, "name": "East European plate", "strat_name": "",
 "lith": "clay", "descrip": "", "comments": "", "t_int_name": "Late Permian", "t_int_age": 251.902,
 "b_int_name": "Late Permian", "b_int_age": 274.4, "color": "#FBA794", "scale": "medium",
 "ref": {"name": "Europe 1:5M", "authors": "Asch, K.", "ref_year": "2003", "url": "https://www.bgr.bund.de/igme5000"}}],
 "macrostrat": {}, "lines": {}}}}"##;

#[test]
fn parse_query() {
    let units = geology::parse_query(ANSWER.as_bytes()).unwrap_or_default();
    assert_eq!(units.len(), 1);
    let unit = &units[0];
    assert_eq!(unit.name, "East European plate");
    assert_eq!(unit.lith, "clay");
    assert_eq!(unit.source, "Europe 1:5M");
    assert_eq!(geology::age_text(unit), ("Late Permian".into(), "274.4-251.9".into()));
    let other = geology::Unit { bottom: "Early Permian".into(), top_age: None, ..unit.clone() };
    assert_eq!(geology::age_text(&other), ("Early Permian - Late Permian".into(), String::new()));
    assert_eq!(geology::parse_query(br#"{"success": {"data": {"burwell": []}}}"#), Ok(Vec::new()));
    // Глубже карты места - запись {} у Перми на уровне 10.
    assert_eq!(geology::parse_query(br#"{"success": {"data": {"burwell": [{}]}}}"#), Ok(Vec::new()));
    assert!(geology::parse_query(b"<html>").is_err());
}

#[test]
fn coarser() {
    assert_eq!(geology::coarser(13), Some(10));
    assert_eq!(geology::coarser(3), Some(1));
    assert_eq!(geology::coarser(1), None);
}

#[test]
fn urls() {
    let url = geology::query_url(58.0, 56.25, 8.4);
    assert!(url.contains("lat=58.00000") && url.contains("lng=56.25000"));
    assert!(url.ends_with("&z=8"));
    assert!(geology::query_url(0.0, 0.0, 30.0).ends_with("&z=16"));
    assert!(geology::TILES.contains("{z}"));
}
