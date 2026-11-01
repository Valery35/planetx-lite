// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Исторические карты по годам. Перенос `tests/test_oldmaps.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use scene::oldmaps;
use scene::when;

fn at(year: i64) -> f64 {
    when::parse(&oldmaps::year_time(year).0).unwrap()
}

fn entries() -> Vec<(&'static str, f64)> {
    vec![("city", at(1850)), ("city", at(1900)), ("city", at(1950)), ("region", at(1920))]
}

fn list(moment: Option<f64>) -> Vec<usize> {
    oldmaps::shown(&entries(), moment).into_iter().collect()
}

#[test]
fn latest_not_later_than_moment_per_group() {
    assert_eq!(list(Some(at(1925))), [1, 3]);
}

#[test]
fn moment_on_map_year_shows_that_map() {
    assert_eq!(list(Some(at(1950))), [2, 3]);
}

#[test]
fn before_first_map_nothing() {
    assert!(list(Some(at(1800))).is_empty());
}

#[test]
fn closed_bar_shows_all() {
    assert_eq!(list(None), [0, 1, 2, 3]);
}

#[test]
fn steps_by_map_years() {
    let moments = [at(1850), at(1900), at(1900), at(1950)];
    assert_eq!(oldmaps::step(&moments, at(1900), 1), Some(at(1950)));
    assert_eq!(oldmaps::step(&moments, at(1900), -1), Some(at(1850)));
    assert_eq!(oldmaps::step(&moments, at(1925), -1), Some(at(1900)));
    assert_eq!(oldmaps::step(&moments, at(1950), 1), None);
}

#[test]
fn year_text_and_label() {
    assert_eq!(oldmaps::year_time(1890), ("1890".to_string(), "1890".to_string()));
    assert_eq!(oldmaps::year_label(at(1890)), "1890");
    let year = oldmaps::parse_year("-500").unwrap().unwrap();
    assert_eq!(oldmaps::year_label(at(year)), "-500");
}

#[test]
fn parse_year() {
    assert_eq!(oldmaps::parse_year(" 1890 "), Ok(Some(1890)));
    assert_eq!(oldmaps::parse_year(""), Ok(None));
    assert!(oldmaps::parse_year("0").is_err());
    assert!(oldmaps::parse_year("около 1890").is_err());
}

#[test]
fn only_stamp_is_map_moment() {
    assert_eq!(oldmaps::start(Some(&("1890".to_string(), "1890".to_string()))), Some(at(1890)));
    assert_eq!(oldmaps::start(Some(&("1890".to_string(), "1910".to_string()))), None);
    assert_eq!(oldmaps::start(None), None);
}
