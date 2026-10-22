// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Вид метки: пять чисел, прежние три, запись, метка «Сохранить вид».
//! Перенос `tests/test_lookat.py` модуля, тесты Camera и слоёв вида
//! добавлены.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use globe_core::ellipsoid::Ellipsoid;
use scene::lookat as la;

#[test]
fn round_trip() {
    let view = la::make(58.24, 56.57, 50257.0, 370.0, 95.0).unwrap();
    assert_eq!(view, [58.24, 56.57, 50257.0, 10.0, la::MAX_TILT]);
    assert_eq!(la::parse(&la::text(Some(&view)), None), Some(view));
    assert_eq!(la::text(Some(&view)), "58.24,56.57,50257.0,10.0,85.0");
}

#[test]
fn old_three_numbers_look_at_the_place() {
    assert_eq!(la::parse("2500.0,30.0,45.0", Some((58.01, 56.23))), Some([58.01, 56.23, 2500.0, 30.0, 45.0]));
    assert_eq!(la::parse("2500.0,30.0,45.0", None), None);
}

#[test]
fn bad_fields() {
    assert_eq!(la::parse("", None), None);
    assert_eq!(la::parse("a,b,c,d,e", None), None);
    assert_eq!(la::parse("58,56,0,0,0", None), None);
    assert_eq!(la::text(None), "");
}

#[test]
fn view_mark() {
    let view = la::make(58.0, 56.0, 1000.0, 0.0, 0.0).unwrap();
    assert!(la::is_view_mark("point", &[(58.0, 56.0)], Some(&view)));
    assert!(!la::is_view_mark("point", &[(58.1, 56.0)], Some(&view)));
    assert!(!la::is_view_mark("line", &[(58.0, 56.0)], Some(&view)));
    assert!(!la::is_view_mark("point", &[(58.0, 56.0)], None));
}

#[test]
fn camera_straight_down_looks_below_itself() {
    let e = Ellipsoid::wgs84();
    let v = la::from_camera(&e, 58.0, 56.0, 1000.0, 0.0, 0.0).unwrap();
    assert!((v[0] - 58.0).abs() < 1e-6 && (v[1] - 56.0).abs() < 1e-6);
    assert!((v[2] - 1000.0).abs() < 1e-3);
    assert!(v[4] < 1e-3);
}

#[test]
fn camera_above_horizon_falls_back() {
    // Наклон 120° от отвеса смотрит в небо: берётся близкий к горизонту.
    let e = Ellipsoid::wgs84();
    let v = la::from_camera(&e, 58.0, 56.0, 2000.0, 90.0, 120.0).unwrap();
    assert!(v[2] > 2000.0);
    assert!((v[3] - 90.0).abs() < 1.0);
    assert!(v[4] <= la::MAX_TILT);
}

#[test]
fn layers_round_trip_and_state() {
    let text = r#"{"grid":1,"stars":false,"@wedge":[1,2,3,4],"@ground":{"opacity":2,"cut":1},"@bad":5,"@opacity":{"x":0.5}}"#;
    let layers = la::layers_parse(text).unwrap();
    assert_eq!(layers.get("grid"), Some(&serde_json::Value::Bool(true)));
    assert!(!layers.contains_key("@bad"));
    let back = la::layers_text(Some(&layers));
    assert!(back.contains(r#""@ground":{"cut":true,"opacity":1.0}"#) || back.contains(r#""@ground":{"opacity":1.0,"cut":true}"#), "{back}");
    assert!(back.contains(r#""grid":1"#) && back.contains(r#""stars":0"#), "{back}");
    let sw = la::switches(Some(&layers));
    assert_eq!(sw.len(), 2);
    assert_eq!(la::layers_parse("[1]"), None);
    assert_eq!(la::layers_parse(""), None);
}
