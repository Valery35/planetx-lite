// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Скважины: ствол по минимальной кривизне, разбор таблиц. Перенос
//! `tests/test_drillholes.py` модуля.

use std::collections::{BTreeMap, HashMap};

use data::drillholes::{self as dh, Role, Row, Value};

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

fn row(pairs: &[(&str, Value)]) -> Row {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

// Ствол.

#[test]
fn vertical_without_survey() {
    let nodes = dh::axis(150.0, 80.0, &[]);
    assert_eq!(nodes.last(), Some(&[80.0, 0.0, 0.0, 70.0]));
}

#[test]
fn straight_inclined() {
    // Зенит 30°, азимут 90° (восток), 100 м по стволу.
    let nodes = dh::axis(0.0, 100.0, &[(0.0, 30.0, 90.0), (100.0, 30.0, 90.0)]);
    let Some(&[_, east, north, z]) = nodes.last() else { return };
    close(east, 50.0, 1e-6);
    close(north, 0.0, 1e-6);
    close(z, -100.0 * 30f64.to_radians().cos(), 1e-6);
}

#[test]
fn arc_of_constant_curvature() {
    // Набор зенита от 0 до 90° на дуге радиуса 100 м к северу: забой на 100 м
    // к северу и на 100 м ниже устья. Минимальная кривизна на дуге точна.
    let length = 100.0 * std::f64::consts::PI / 2.0;
    let nodes = dh::axis(0.0, length, &[(0.0, 0.0, 0.0), (length, 90.0, 0.0)]);
    let Some(&[_, east, north, z]) = nodes.last() else { return };
    close(north, 100.0, 1e-6);
    close(z, -100.0, 1e-6);
    close(east, 0.0, 1e-6);
    // Середина дуги лежит на окружности, а не на хорде.
    let mid = dh::point_at(&nodes, length / 2.0);
    assert!(mid.is_some());
    let Some(mid) = mid else { return };
    close((mid[1] - 100.0).hypot(mid[2]), 100.0, 0.05);
}

#[test]
fn extends_below_last_station() {
    let nodes = dh::axis(0.0, 200.0, &[(0.0, 0.0, 0.0), (100.0, 0.0, 0.0)]);
    let Some(last) = nodes.last() else { return };
    close(last[3], -200.0, 1e-9);
}

#[test]
fn piece_has_ends() {
    let nodes = dh::axis(0.0, 100.0, &[]);
    let part = dh::piece(&nodes, 20.0, 35.0);
    assert_eq!(part.first(), Some(&[0.0, 0.0, -20.0]));
    assert_eq!(part.last(), Some(&[0.0, 0.0, -35.0]));
}

#[test]
fn hostile_stations_give_finite_axis() {
    // Станции с NaN отбрасываются, пустой ствол точки не даёт.
    let nodes = dh::axis(0.0, 50.0, &[(f64::NAN, 10.0, 0.0), (10.0, f64::INFINITY, 0.0)]);
    assert!(nodes.iter().flatten().all(|v| v.is_finite()));
    assert_eq!(nodes.last(), Some(&[50.0, 0.0, 0.0, -50.0]));
    assert!(dh::point_at(&[], 10.0).is_none());
    assert!(dh::piece(&[], 0.0, 10.0).is_empty());
}

// Углы наклона.

#[test]
fn zenith_conventions() {
    assert_eq!(dh::zeniths(&[-90.0, -60.0], "dip"), [0.0, 30.0]);
    assert_eq!(dh::zeniths(&[90.0, 60.0], "DIP"), [0.0, 30.0]);
    assert_eq!(dh::zeniths(&[0.0, 10.0], "inc"), [0.0, 10.0]);
    assert_eq!(dh::zeniths(&[2.0, 5.0], "dip"), [2.0, 5.0]);
}

// Разбор таблиц.

#[test]
fn synonyms_swapped_and_orphans() {
    let collars = [
        row(&[
            ("BHID", "A".into()),
            ("ELEV", 120.0.into()),
            ("TD", 50.0.into()),
            ("x", 1.into()),
            ("y", 2.into()),
        ]),
        row(&[("BHID", 7.0.into()), ("ELEV", 100.0.into()), ("TD", Value::Null), ("x", 3.into()), ("y", 4.into())]),
    ];
    let intervals = [
        row(&[("bhid", "A".into()), ("from_m", 10.0.into()), ("to_m", 0.0.into()), ("lith", "Q".into())]),
        row(&[
            ("bhid", "A".into()),
            ("from_m", 10.0.into()),
            ("to_m", 30.0.into()),
            ("lith", "ПЦТ".into()),
            ("grade", 5.into()),
        ]),
        row(&[("bhid", 7.into()), ("from_m", 0.0.into()), ("to_m", 60.0.into()), ("lith", "Q".into())]),
        row(&[("bhid", "Z".into()), ("from_m", 0.0.into()), ("to_m", 5.0.into()), ("lith", "Q".into())]),
    ];
    let surveys =
        [row(&[("hole", "A".into()), ("depth", 0.0.into()), ("azimuth", 90.0.into()), ("dip", (-60.0).into())])];
    let (holes, skipped) = dh::assemble(&collars, &intervals, &surveys);
    let ids: Vec<&str> = holes.iter().map(|h| h.hole_id.as_str()).collect();
    assert_eq!(ids, ["A", "7"]);
    let [a, seven] = holes.as_slice() else { return };
    let got: Vec<(f64, f64, &str)> = a.intervals.iter().map(|i| (i.start, i.end, i.code.as_str())).collect();
    assert_eq!(got, [(0.0, 10.0, "Q"), (10.0, 30.0, "ПЦТ")]);
    assert_eq!(a.intervals[1].extra, row(&[("grade", 5.into())]));
    assert_eq!((a.x, a.y), (Some(1.0), Some(2.0)));
    // Забоя нет - он по нижнему интервалу.
    assert_eq!(seven.eoh, 60.0);
    // dip -60 - зенит 30, на восток.
    let Some(last) = a.axis.last() else { return };
    close(last[1], 50.0 * 0.5, 1e-6);
    let want: BTreeMap<&str, usize> = [("interval_swapped", 1), ("interval_no_collar", 1)].into();
    assert_eq!(skipped, want);
}

#[test]
fn missing_fields_are_counted() {
    // Устье без отметки, инклинометрия без азимута, интервал без глубин.
    let collars = [row(&[("hole_id", "A".into()), ("z", "нет".into()), ("eoh", 10.0.into())])];
    let intervals = [row(&[("hole_id", "A".into()), ("code", "Q".into())])];
    let surveys = [row(&[("hole_id", "A".into()), ("md", 0.0.into()), ("dip", (-90.0).into())])];
    let (holes, skipped) = dh::assemble(&collars, &intervals, &surveys);
    assert!(holes.is_empty());
    let want: BTreeMap<&str, usize> =
        [("survey_fields", 1), ("interval_fields", 1), ("collar_fields", 1)].into();
    assert_eq!(skipped, want);
}

#[test]
fn find_field_ignores_case() {
    assert_eq!(dh::find_field(["X", "Hole_ID", "BHID"], Role::HoleId), Some("Hole_ID"));
    assert_eq!(dh::find_field(["x", "y"], Role::Z), None);
}

#[test]
fn code_color_is_stable() {
    assert_eq!(dh::code_color("КрII", None), dh::code_color("КрII", None));
    let palette: HashMap<String, String> = [("Q".to_string(), "#ff0080".to_string())].into();
    assert_eq!(dh::code_color("Q", Some(&palette)), [255, 0, 128]);
}

#[test]
fn code_color_matches_module() {
    // Цвета модуля PlanetX для QGIS, посчитаны его `code_color`.
    assert_eq!(dh::code_color("КрII", None), [53, 194, 87]);
    assert_eq!(dh::code_color("Q", None), [203, 49, 104]);
    assert_eq!(dh::code_color("", None), [212, 87, 213]);
    assert_eq!(dh::code_color("ПЦТ", None), [111, 75, 202]);
    assert_eq!(dh::code_color(&"a".repeat(70), None), [205, 115, 66]);
    // Неверный цвет палитры - цвет по коду, как без палитры.
    let palette: HashMap<String, String> = [("Q".to_string(), "#zz".to_string())].into();
    assert_eq!(dh::code_color("Q", Some(&palette)), [203, 49, 104]);
}
