// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Самолёты ADS-B adsb.lol: адрес запроса, группы, разбор ответа,
//! модель. Перенос класса TestAircraft из `tests/test_movers.py` модуля
//! и свои проверки по `core/aircraft.py`.

use data::aircraft::{self, FOOT, KNOT};

const ANSWER: &str = r#"{"now": 1760900000000, "total": 5, "ac": [
    {"hex": "4CA7B5 ", "type": "adsb_icao", "flight": "RYR1AB  ", "r": "EI-DCL",
     "t": "B738", "alt_baro": 37000, "alt_geom": 37500, "gs": 450.2,
     "track": 271.3, "true_heading": 268.0, "baro_rate": -64, "geom_rate": 32,
     "squawk": "1000", "category": "A3", "lat": 60.1, "lon": 24.9,
     "seen_pos": 1.5, "seen": 0.2, "roll": -2.5, "track_rate": 0.1},
    {"hex": "461f2a", "alt_baro": "ground", "alt_geom": 50, "gs": 12,
     "category": "A7", "lat": 60.3, "lon": 24.96, "seen": 3, "baro_rate": 500},
    {"hex": "abc123", "lat": 60.0, "lon": 25.0, "alt_baro": true},
    {"hex": "", "lat": 60.0, "lon": 25.0},
    {"hex": "def456", "lat": 60.0},
    {"hex": "fed654", "lat": 91.0, "lon": 25.0},
    "x"]}"#;

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

/// Запись ответа от 19 октября 2026 года, `test_movers.py`.
const PLANES: &str = r#"{"now": 1792356307501.0, "ac": [
    {"hex": "4ACA82", "flight": "NSZ1GT  ", "r": "SE-RTB", "t": "B38M",
     "alt_baro": 12375, "gs": 306.0, "track": 87.54,
     "true_heading": 87.63, "baro_rate": -1856, "roll": -0.35,
     "category": "A3", "lat": 60.194554, "lon": 23.450472,
     "seen_pos": 2.5},
    {"hex": "abc", "alt_baro": "ground", "gs": 3.0, "track": 10.0,
     "lat": 60.3, "lon": 24.96, "seen": 1.0},
    {"hex": "nopos", "alt_baro": 1000}]}"#;

#[test]
fn module_parse() {
    let (samples, info) = aircraft::parse(PLANES.as_bytes());
    assert_eq!(samples.iter().map(|s| s.key.as_str()).collect::<Vec<_>>(), ["4aca82", "abc"]);
    let plane = &samples[0];
    close(plane.alt, 12375.0 * 0.3048, 1e-9);
    close(plane.climb, -1856.0 * 0.3048 / 60.0, 1e-12);
    close(plane.t, 1792356307.501 - 2.5, 1e-6);
    assert_eq!(plane.heading, 87.63);
    assert_eq!(aircraft::label("4aca82", info.get("4aca82")), "NSZ1GT");
    assert_eq!(info.get("4aca82").map(aircraft::Info::group), Some("large"));
    assert!(samples[1].alt.is_nan());
    assert!(info.get("abc").is_some_and(|i| i.ground));
    assert_eq!(aircraft::label("abc", info.get("abc")), "ABC");
}

#[test]
fn module_url_radius() {
    assert!(aircraft::point_url(60.0, 25.0, 1e7).ends_with("/250"));
    assert!(aircraft::point_url(60.0, 25.0, 100.0).ends_with("/5"));
    assert!(aircraft::point_url(60.0, 25.0, 1e4).contains("/60.0000/25.0000/"));
}

#[test]
fn point_url() {
    assert_eq!(aircraft::point_url(60.1, 24.9, 20000.0), "https://api.adsb.lol/v2/point/60.1000/24.9000/11");
    assert_eq!(aircraft::point_url(-33.94612, 151.17722, 1.0), "https://api.adsb.lol/v2/point/-33.9461/151.1772/5");
    assert_eq!(aircraft::point_url(0.0, 0.0, 1e9), "https://api.adsb.lol/v2/point/0.0000/0.0000/250");
    assert_eq!(aircraft::point_url(0.0, 0.0, f64::NAN), "https://api.adsb.lol/v2/point/0.0000/0.0000/5");
}

#[test]
fn groups_and_sizes() {
    assert_eq!(aircraft::group("A1"), "light");
    assert_eq!(aircraft::group("A6"), "small");
    assert_eq!(aircraft::group("A4"), "large");
    assert_eq!(aircraft::group("A5"), "heavy");
    assert_eq!(aircraft::group("A7"), "rotor");
    assert_eq!(aircraft::group(""), "other");
    assert_eq!(aircraft::plane_size("A5"), (64.0, 64.0, 64.0));
    assert_eq!(aircraft::plane_size("B1"), (30.0, 30.0, 30.0));
    assert_eq!(aircraft::group_color("rotor"), [0.96, 0.58, 0.14]);
    assert_eq!(aircraft::group_color("?"), [0.75, 0.75, 0.75]);
}

#[test]
fn parse_answer() {
    let (samples, info) = aircraft::parse(ANSWER.as_bytes());
    assert_eq!(samples.len(), 3);
    assert_eq!(info.len(), 3);
    let a = &samples[0];
    assert_eq!(a.key, "4ca7b5");
    close(a.t, 1760900000.0 - 1.5, 1e-6);
    close(a.speed, 450.2 * KNOT, 1e-9);
    close(a.course, 271.3, 1e-12);
    close(a.heading, 268.0, 1e-12);
    close(a.alt, 37500.0 * FOOT, 1e-9);
    close(a.climb, 32.0 * FOOT / 60.0, 1e-12);
    close(a.roll, -2.5, 1e-12);
    close(a.turn, 0.1, 1e-12);
    let Some(first) = info.get("4ca7b5") else {
        assert!(info.contains_key("4ca7b5"));
        return;
    };
    assert_eq!(first.flight, "RYR1AB");
    assert_eq!(first.reg, "EI-DCL");
    assert_eq!(first.kind, "B738");
    assert_eq!(first.squawk, "1000");
    assert_eq!(first.group(), "large");
    assert!(!first.ground);
    // На земле: высоты нет, подъёма нет, давность - seen.
    let g = &samples[1];
    assert!(g.alt.is_nan());
    assert_eq!(g.climb, 0.0);
    close(g.t, 1760900000.0 - 3.0, 1e-6);
    assert!(g.course.is_nan() && g.heading.is_nan() && g.roll.is_nan() && g.turn.is_nan());
    assert!(info.get("461f2a").is_some_and(|i| i.ground));
    // Логическое значение - не число.
    assert!(samples[2].alt.is_nan());
    assert_eq!(samples[2].speed, 0.0);
}

#[test]
fn hostile_answers() {
    assert_eq!(aircraft::parse(b"not json").0.len(), 0);
    assert_eq!(aircraft::parse(b"[]").0.len(), 0);
    // Без времени ответа положений нет.
    assert_eq!(aircraft::parse(br#"{"ac": [{"hex": "a", "lat": 1, "lon": 2}]}"#).0.len(), 0);
    assert_eq!(aircraft::parse(br#"{"now": 1, "ac": {"hex": "a"}}"#).0.len(), 0);
    // Отрицательная высота - на нуле.
    let (s, _) = aircraft::parse(br#"{"now": 1000, "ac": [{"hex": "a", "lat": 1, "lon": 2, "alt_baro": -200}]}"#);
    assert_eq!(s.first().map(|s| s.alt), Some(0.0));
}

#[test]
fn labels() {
    let (_, info) = aircraft::parse(ANSWER.as_bytes());
    assert_eq!(aircraft::label("4ca7b5", info.get("4ca7b5")), "RYR1AB");
    let reg = aircraft::Info { reg: "OH-LVA".into(), ..aircraft::Info::default() };
    assert_eq!(aircraft::label("461f2a", Some(&reg)), "OH-LVA");
    assert_eq!(aircraft::label("461f2a", info.get("461f2a")), "461F2A");
    assert_eq!(aircraft::label("abc", None), "ABC");
}

#[test]
fn unit_plane() {
    let parts = aircraft::unit_plane_parts([0.1, 0.2, 0.3]);
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[2].rgba, [0.1, 0.2, 0.3, 1.0]);
    // Фюзеляж - две коробки по 36 вершин.
    assert_eq!(parts[0].positions.len(), 72);
    let ys: Vec<f64> = parts.iter().flat_map(|p| p.positions.iter().map(|v| v[1])).collect();
    let xs: Vec<f64> = parts.iter().flat_map(|p| p.positions.iter().map(|v| v[0])).collect();
    assert_eq!(ys.iter().copied().fold(f64::MAX, f64::min), -0.5);
    assert_eq!(ys.iter().copied().fold(f64::MIN, f64::max), 0.5);
    assert_eq!(xs.iter().copied().fold(f64::MAX, f64::min), -0.5);
    assert_eq!(xs.iter().copied().fold(f64::MIN, f64::max), 0.5);
}
