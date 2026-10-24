// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Разломы GEM, границы плит PB2002, кабели OSM: разбор, ближайшая
//! линия, картинка тайла. Перенос `tests/test_faults.py`,
//! `test_plates.py` и `test_cables.py` модуля.

use data::{faults, linesets, plates};

const FAULTS: &str = r#"{"type": "FeatureCollection", "features": [
 {"type": "Feature", "geometry": {"type": "LineString", "coordinates": [[-121.9, 37.8], [-121.8, 37.9]]},
  "properties": {"name": "Mount Diablo Thrust", "slip_type": "Reverse", "net_slip_rate": "(1.55,0.8,2.22)",
   "average_dip": "(38,,)", "dip_dir": "E", "catalog_name": "UCERF3"}},
 {"type": "Feature", "geometry": {"type": "MultiLineString", "coordinates": [[[30.0, 40.0], [31.0, 40.0]], [[32.0, 40.0], [33.0, 40.2]]]},
  "properties": {"fs_name": "North Anatolian", "slip_type": "Dextral", "strike_slip_rate": "(20.0, 15.0, 25.0)"}},
 {"type": "Feature", "geometry": {"type": "Point", "coordinates": [0.0, 0.0]}, "properties": {}},
 {"type": "Feature", "geometry": {"type": "LineString", "coordinates": [[10.0, 10.0]]}, "properties": {}}]}"#;

const CABLES: &str = r#"{"date": "2026-10-11", "cables": [
 {"kind": "telecom", "name": "A", "osm": "way/1", "points": [[50.0, -10.0], [40.0, -60.0]]},
 {"kind": "power", "name": "B", "voltage": "450000", "osm": "way/2", "points": [[55.0, 8.0], [56.0, 8.0]]},
 {"kind": "pipe", "points": [[0.0, 0.0], [1.0, 1.0]]},
 {"kind": "power", "points": [[0.0, 0.0]]}]}"#;

#[test]
fn fault_kinds() {
    let cases = [
        ("Normal", "normal"),
        ("Normal-Dextral", "normal"),
        ("Reverse-Strike-Slip", "reverse"),
        ("Subduction_Thrust", "reverse"),
        ("Blind Thrust", "reverse"),
        ("Dextral_Transform", "strike"),
        ("Sinistral", "strike"),
        ("Strike-Slip", "strike"),
        ("Spreading_Ridge", "ridge"),
        ("Anticline", "fold"),
        ("Syncline", "fold"),
        ("", "other"),
        ("Unknown", "other"),
    ];
    for (slip, kind) in cases {
        assert_eq!(faults::kind_of(slip), kind, "{slip}");
    }
}

#[test]
fn fault_first_value() {
    let v = |s: &str| faults::first_value(Some(&serde_json::Value::String(s.into())));
    assert_eq!(v("(1.55,0.8,2.22)"), Some(1.55));
    assert_eq!(v("(38,,)"), Some(38.0));
    assert_eq!(v("(3.0, 1.0, 5.0)"), Some(3.0));
    assert_eq!(v("(,1600000,)"), None);
    assert_eq!(faults::first_value(None), None);
    assert_eq!(faults::first_value(Some(&serde_json::json!(2))), Some(2.0));
}

#[test]
fn faults_parse_and_nearest() {
    let (found, lines) = faults::parse(FAULTS.as_bytes()).unwrap_or_default();
    assert_eq!(found.len(), 3);
    let first = &found[0];
    assert_eq!((first.kind, first.name.as_str(), first.rate, first.dip, first.dip_dir.as_str(), first.catalog.as_str()), ("reverse", "Mount Diablo Thrust", Some(1.55), Some(38.0), "E", "UCERF3"));
    assert_eq!(lines[0].points, [(37.8, -121.9), (37.9, -121.8)]);
    // Части MultiLineString - отдельные разломы, имя из fs_name, скорость -
    // сдвиговая, если общей нет.
    assert_eq!(found[1].name, "North Anatolian");
    assert_eq!(found[2].name, "North Anatolian");
    assert_eq!(found[1].rate, Some(20.0));
    assert_eq!(found[2].kind, "strike");
    let name = |lat: f64, lon: f64, m: f64| linesets::nearest(&lines, lat, lon, m).map(|i| found[i].name.clone());
    assert_eq!(name(40.01, 30.5, 2000.0).as_deref(), Some("North Anatolian"));
    assert_eq!(name(41.0, 30.5, 2000.0), None);
    assert_eq!(name(37.85, -121.85, 500.0).as_deref(), Some("Mount Diablo Thrust"));
}

#[test]
fn cables_parse_and_nearest() {
    let (cables, lines, date) = plates::parse_cables(CABLES.as_bytes()).unwrap_or_default();
    assert_eq!(cables.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["A", "B"]);
    assert_eq!(date, "2026-10-11");
    assert_eq!(lines[1].bbox, (55.0, 8.0, 56.0, 8.0));
    // Середина отрезка в тысячи километров - вершин рядом нет.
    assert_eq!(linesets::nearest(&lines, 45.6, -33.0, 60000.0), Some(0));
    assert_eq!(linesets::nearest(&lines, 30.0, -33.0, 60000.0), None);
    assert_eq!(linesets::nearest(&lines, 55.5, 8.01, 2000.0), Some(1));
    assert_eq!(plates::kilovolts(&cables[1].voltage), Some(450.0));
    assert_eq!(plates::kilovolts("320kV;150000"), Some(320.0));
    assert_eq!(plates::kilovolts("medium"), None);
}

#[test]
fn plates_from_module_file() {
    // Файл данных модуля, если он рядом.
    let Ok(bytes) = std::fs::read(r"C:\Dev\planetx\planetx\data\plates.json") else {
        eprintln!("данных модуля нет, проверка пропущена");
        return;
    };
    let (bounds, lines, names) = plates::parse_plates(&bytes).unwrap_or_default();
    assert_eq!(names.len(), 54);
    assert!(bounds.len() > 1500);
    assert!(names.iter().any(|p| p.name == "Pacific"));
    // Японский жёлоб у эпицентра Тохоку: граница рядом, схождение.
    let near = linesets::nearest(&lines, 38.3, 143.9, 80000.0);
    assert_eq!(near.and_then(|i| plates::group(&bounds[i].code)), Some("convergent"));
    // Середина Африки - до границ сотни километров.
    assert_eq!(linesets::nearest(&lines, 5.0, 20.0, 50000.0), None);
}

#[test]
fn draw_line_on_tile() {
    let line = linesets::Line::new(vec![(0.0, -170.0), (0.0, -10.0)], [255, 0, 0, 255], 2.0);
    // Экватор тайла 1/0/0 и 1/0/1 - у их общего края, линия на краю.
    let lines = [line];
    let up = linesets::draw((1, 0, 0), &[&lines]);
    assert!(up.is_some());
    // Тайл далеко от линии - пусто.
    assert!(linesets::draw((4, 3, 2), &[&lines]).is_none());
}
