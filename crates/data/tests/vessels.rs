// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Суда AIS Digitraffic: группы, разбор положений и сведений, размеры,
//! модель. Перенос класса TestVessels из `tests/test_movers.py` модуля
//! и свои проверки по `core/vessels.py`.

use data::vessels::{self, Info};

const LOCATIONS: &str = r#"{"type": "FeatureCollection", "features": [
    {"mmsi": 230000001, "type": "Feature",
     "geometry": {"type": "Point", "coordinates": [24.95, 60.16]},
     "properties": {"mmsi": 230000001, "sog": 10.0, "cog": 90.5, "navStat": 0,
                    "rot": 0, "posAcc": true, "raim": false, "heading": 92,
                    "timestamp": 30, "timestampExternal": 1760800000000}},
    {"mmsi": 230000002, "type": "Feature",
     "geometry": {"type": "Point", "coordinates": [25.0, 60.2]},
     "properties": {"sog": 102.3, "cog": 360.0, "heading": 511,
                    "timestampExternal": 1760800001000}},
    {"type": "Feature", "geometry": {"type": "Point", "coordinates": [25.0, 95.0]},
     "properties": {"mmsi": 3, "timestampExternal": 1760800001000}},
    {"type": "Feature", "geometry": {"type": "Point", "coordinates": [25.0]},
     "properties": {"mmsi": 4, "timestampExternal": 1760800001000}},
    {"type": "Feature", "geometry": {"type": "Point", "coordinates": [25.0, 60.0]},
     "properties": {"mmsi": 5}},
    {"type": "Feature", "geometry": {"type": "Point", "coordinates": [25.0, 60.0]},
     "properties": {"mmsi": null, "timestampExternal": 1760800001000}},
    7]}"#;

const VESSELS: &str = r#"[
    {"mmsi": 230000001, "name": "VIKING GRACE ", "callSign": "OJPU", "imo": 9606900,
     "shipType": 60, "destination": "TURKU", "referencePointA": 160,
     "referencePointB": 58, "referencePointC": 16, "referencePointD": 16},
    {"mmsi": 230000003, "name": "", "shipType": 80},
    {"name": "без номера"}, 5]"#;

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

// Записи ответов служб от 19 октября 2026 года, `test_movers.py`.
const MODULE_LOCATIONS: &str = r#"{"type": "FeatureCollection", "features": [
    {"mmsi": 230943910, "type": "Feature",
     "geometry": {"type": "Point", "coordinates": [22.385135, 60.420192]},
     "properties": {"mmsi": 230943910, "sog": 12.0, "cog": 90.0,
                    "navStat": 0, "heading": 511, "timestamp": 0,
                    "timestampExternal": 1792356241064}},
    {"mmsi": 1, "type": "Feature",
     "geometry": {"type": "Point", "coordinates": [22.0, 60.0]},
     "properties": {"mmsi": 1, "sog": 102.3, "cog": 360.0,
                    "heading": 45, "timestampExternal": 1792356241064}},
    {"mmsi": 2, "geometry": null, "properties": {"mmsi": 2}}]}"#;
const MODULE_VESSELS: &str = r#"[{"callSign": "5BFU4", "destination": "SE LLA", "imo": 9194098,
    "mmsi": 210547000, "name": "SWE-FREIGHTER ",
    "referencePointA": 83, "referencePointB": 16,
    "referencePointC": 7, "referencePointD": 7, "shipType": 70}]"#;

#[test]
fn module_locations() {
    let found = vessels::parse_locations(MODULE_LOCATIONS.as_bytes());
    assert_eq!(found.iter().map(|s| s.key).collect::<Vec<_>>(), [230943910, 1]);
    let ship = &found[0];
    close(ship.speed, 12.0 * 1852.0 / 3600.0, 1e-12);
    assert_eq!(ship.course, 90.0);
    assert!(ship.heading.is_nan());
    close(ship.t, 1792356241.064, 1e-6);
    let still = &found[1];
    assert_eq!(still.speed, 0.0);
    assert!(still.course.is_nan());
    assert_eq!(still.heading, 45.0);
}

#[test]
fn module_vessels_and_size() {
    let all = vessels::parse_vessels(MODULE_VESSELS.as_bytes());
    let info = all.get(&210547000);
    assert_eq!(info.map(|i| (i.name.as_str(), i.group())), Some(("SWE-FREIGHTER", "cargo")));
    assert_eq!(vessels::ship_size(info), (14.0, 99.0, 14.0));
    assert_eq!(vessels::ship_size(None).1, 30.0);
    assert_eq!(vessels::label(5, None), "5");
    assert_eq!(vessels::group(65), "passenger");
    assert_eq!(vessels::group(84), "tanker");
    assert_eq!(vessels::group(37), "pleasure");
}

#[test]
fn groups() {
    assert_eq!(vessels::group(60), "passenger");
    assert_eq!(vessels::group(79), "cargo");
    assert_eq!(vessels::group(84), "tanker");
    assert_eq!(vessels::group(30), "fishing");
    assert_eq!(vessels::group(52), "service");
    assert_eq!(vessels::group(45), "fast");
    assert_eq!(vessels::group(37), "pleasure");
    assert_eq!(vessels::group(0), "other");
    assert_eq!(vessels::group_color("tanker"), [0.86, 0.20, 0.20]);
    assert_eq!(vessels::group_color("?"), vessels::GROUPS[7].1);
    assert_eq!(vessels::default_length("pleasure"), 12.0);
    assert_eq!(vessels::default_length("?"), 30.0);
}

#[test]
fn locations() {
    let found = vessels::parse_locations(LOCATIONS.as_bytes());
    assert_eq!(found.len(), 2);
    let a = &found[0];
    assert_eq!(a.key, 230000001);
    close(a.t, 1760800000.0, 1e-6);
    close(a.lat, 60.16, 1e-12);
    close(a.lon, 24.95, 1e-12);
    close(a.speed, 10.0 * vessels::KNOT, 1e-12);
    close(a.course, 90.5, 1e-12);
    close(a.heading, 92.0, 1e-12);
    assert!(a.alt.is_nan() && a.turn.is_nan());
    assert_eq!((a.climb, a.roll), (0.0, 0.0));
    // «Нет данных» AIS: скорость 102.3, курс 360, азимут 511. MMSI - из
    // записи, когда его нет в свойствах.
    let b = &found[1];
    assert_eq!(b.key, 230000002);
    assert_eq!(b.speed, 0.0);
    assert!(b.course.is_nan() && b.heading.is_nan());
    assert!(vessels::parse_locations(b"[]").is_empty());
    assert!(vessels::parse_locations(b"not json").is_empty());
}

#[test]
fn vessel_info() {
    let info = vessels::parse_vessels(VESSELS.as_bytes());
    assert_eq!(info.len(), 2);
    let Some(grace) = info.get(&230000001) else {
        assert!(info.contains_key(&230000001));
        return;
    };
    assert_eq!(grace.name, "VIKING GRACE");
    assert_eq!(grace.call, "OJPU");
    assert_eq!(grace.imo, 9606900);
    assert_eq!(grace.destination, "TURKU");
    assert_eq!((grace.length, grace.width), (218.0, 32.0));
    assert_eq!(grace.group(), "passenger");
    assert_eq!(vessels::label(230000001, Some(grace)), "VIKING GRACE");
    assert_eq!(vessels::label(230000003, info.get(&230000003)), "230000003");
    assert_eq!(vessels::label(7, None), "7");
    assert!(vessels::parse_vessels(br#"{"mmsi": 1}"#).is_empty());
}

#[test]
fn sizes() {
    let grace = Info { ship_type: 60, length: 218.0, width: 32.0, ..Info::default() };
    assert_eq!(vessels::ship_size(Some(&grace)), (32.0, 218.0, 32.0));
    // Без размеров AIS - длина группы, ширина - шестая часть длины.
    let tanker = Info { ship_type: 80, ..Info::default() };
    assert_eq!(vessels::ship_size(Some(&tanker)), (25.0, 150.0, 25.0));
    assert_eq!(vessels::ship_size(None), (5.0, 30.0, 5.0));
    let odd = Info { ship_type: 70, length: 900.0, width: 0.5, ..Info::default() };
    assert_eq!(vessels::ship_size(Some(&odd)), (20.0, 120.0, 20.0));
}

#[test]
fn unit_ship() {
    let color = vessels::group_color("cargo");
    let parts = vessels::unit_ship_parts(color);
    assert_eq!(parts.len(), 3);
    assert!(parts.iter().all(|p| p.positions.len() % 3 == 0 && !p.positions.is_empty()));
    assert_eq!(parts[1].rgba, [color[0], color[1], color[2], 1.0]);
    let all = parts.iter().flat_map(|p| p.positions.iter());
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for p in all {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    // Длина 1 вдоль Y, нос на +Y, ширина 1, днище на нуле.
    assert_eq!((lo[1], hi[1]), (-0.5, 0.5));
    assert_eq!((lo[0], hi[0]), (-0.5, 0.5));
    assert_eq!((lo[2], hi[2]), (0.0, 0.95));
    // Призма корпуса из пятиугольника: 3 треугольника крышки и днища
    // по 2, 5 боковых граней по 2 треугольника.
    assert_eq!(parts[0].positions.len(), (3 * 2 + 5 * 2) * 3);
}
