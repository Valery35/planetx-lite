// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Форматы координат: UTM, MGRS, градусы-минуты-секунды, разбор строки.
//! Перенос `tests/test_coords.py` модуля.
//!
//! Эталоны UTM - пересчёт QGIS 4.0.3 из EPSG:4326 в EPSG:326xx и 327xx,
//! 30 сентября 2026 года. Эталон MGRS - пример статьи Military Grid
//! Reference System, Гонолулу, «4Q FJ 1234 6789».

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use scene::coords::{self as co, Format};

/// Широта, долгота, зона, восток, север по QGIS.
const QGIS_UTM: [(f64, f64, u32, f64, f64); 8] = [
    (58.0105, 56.2294, 40, 454464.3794, 6430138.879),
    (60.0, 5.0, 32, 276979.9264, 6658157.2024),
    (78.0, 15.0, 33, 500000.0, 8658369.5858),
    (-33.8688, 151.2093, 56, 334368.6336, 6250948.3454),
    (-0.5, -78.5, 17, 778265.7783, 9944681.96),
    (10.0, 0.0, 31, 171071.2639, 1106908.8542),
    (70.0, 179.9, 60, 610660.4649, 7768505.4523),
    (43.3499, 42.4453, 38, 292952.0158, 4802841.4832),
];

fn honolulu() -> (f64, f64) {
    (
        21.0 + 24.0 / 60.0 + 35.43062452380 / 3600.0,
        -(157.0 + 54.0 / 60.0 + 57.89102787040 / 3600.0),
    )
}

fn close(a: f64, b: f64, tolerance: f64, what: &str) {
    assert!((a - b).abs() <= tolerance, "{a} != {b}: {what}");
}

// TestUtm

#[test]
fn utm_matches_qgis() {
    for (lat, lon, zone, east, north) in QGIS_UTM {
        let got = co::to_utm(lat, lon, None).unwrap();
        assert_eq!(got.0, zone, "{lat} {lon}");
        close(got.2, east, 0.001, "восток");
        close(got.3, north, 0.001, "север");
    }
}

#[test]
fn utm_round_trip() {
    for (lat, lon, ..) in QGIS_UTM {
        let (zone, letter, east, north) = co::to_utm(lat, lon, None).unwrap();
        let back = co::from_utm(zone, letter < 'N', east, north);
        // places=9 в unittest - разница меньше 5e-10.
        close(back.0, lat, 5e-10, "широта");
        close(back.1, lon, 5e-10, "долгота");
    }
}

#[test]
fn outside_utm() {
    assert_eq!(co::to_utm(85.0, 10.0, None), None);
    assert_eq!(
        co::format_point(85.0, 10.0, Format::Utm, co::HEMISPHERES, co::DIGITS),
        "85.00000, 10.00000"
    );
}

// TestMgrs

#[test]
fn mgrs_honolulu() {
    let (lat, lon) = honolulu();
    assert_eq!(co::to_mgrs(lat, lon, 4).as_deref(), Some("4Q FJ 1234 6789"));
}

#[test]
fn mgrs_parse_honolulu() {
    let (lat, lon) = co::parse_point("4QFJ12346789").unwrap();
    let (h_lat, h_lon) = honolulu();
    // Юго-западный угол квадрата 10 м.
    close(lat, h_lat, 10.0 / 111000.0, "широта");
    close(lon, h_lon, 15.0 / 104000.0, "долгота");
}

#[test]
fn mgrs_round_trip() {
    for (lat, lon, ..) in QGIS_UTM {
        let text = co::to_mgrs(lat, lon, co::MGRS_DIGITS).unwrap();
        let back = co::parse_point(&text);
        let Some(back) = back else {
            panic!("{text}");
        };
        // Метровый квадрат: угол не дальше 1.5 м от точки.
        close(back.0, lat, 1.5 / 111000.0, &text);
        close(back.1, lon, 1.5 / 30000.0, &text);
    }
}

// TestDms

#[test]
fn dms_text() {
    assert_eq!(
        co::format_point(58.0105, -56.2294, Format::Dms, co::HEMISPHERES, co::DIGITS),
        "58°00′37.8″ N, 56°13′45.8″ W"
    );
}

#[test]
fn dms_rounding_carries() {
    assert_eq!(co::dms_text(10.99999999, "N", "S", co::DMS_DIGITS), "11°00′00.0″ N");
}

#[test]
fn dms_parse_forms() {
    for text in [
        "58°00′37.8″ N, 56°13′45.8″ E",
        "58°00'37.8\"N 56°13'45.8\"E",
        "58 00 37.8 N 56 13 45.8 E",
        "56°13′45.8″ E 58°00′37.8″ N",
        "58°00′37,8″ с. ш., 56°13′45,8″ в. д.",
    ] {
        let Some((lat, lon)) = co::parse_point(text) else {
            panic!("{text}");
        };
        // places=4 в unittest - разница меньше 5e-5.
        close(lat, 58.0105, 5e-5, text);
        close(lon, 56.2294, 5e-5, text);
    }
}

#[test]
fn dms_west_south() {
    let (lat, _) = co::parse_point("33°52′08″ S 151°12′33″ E").unwrap();
    assert!(lat < 0.0);
    let (_, lon) = co::parse_point("21°24′35″ N 157°54′58″ W").unwrap();
    assert!(lon < 0.0);
}

// TestParse

#[test]
fn parse_decimal_with_height() {
    // Строка координат самого глобуса, её вставляли в поиск.
    assert_eq!(co::parse_point("59.593416, 56.806003, высота 207 м"), Some((59.593416, 56.806003)));
}

#[test]
fn parse_utm() {
    // Восток и север округлены до метра.
    let (lat, lon) = co::parse_point("40V 454464 6430139").unwrap();
    close(lat, 58.0105, 1e-5, "широта");
    close(lon, 56.2294, 2e-5, "долгота");
    let (lat, _) = co::parse_point("56H 334369 6250948").unwrap();
    // places=5 в unittest - разница меньше 5e-6.
    close(lat, -33.8688, 5e-6, "широта");
}

#[test]
fn parse_place_names_are_not_points() {
    for text in ["Москва", "Пермь", "Route 66", "улица Ленина 5 в", "New York", "Sea of Japan"] {
        assert_eq!(co::parse_point(text), None, "{text}");
    }
}

#[test]
fn parse_all_formats_round_trip() {
    for (lat, lon, ..) in QGIS_UTM {
        for fmt in co::FORMATS {
            let text = co::format_point(lat, lon, fmt, co::HEMISPHERES, co::DIGITS);
            let Some(back) = co::parse_point(&text) else {
                panic!("{text}");
            };
            close(back.0, lat, 3e-5, &text);
            close(back.1, lon, 6e-5, &text);
        }
    }
}

// Ключи форматов - строки настроек модуля.

#[test]
fn format_keys() {
    let keys: Vec<&str> = co::FORMATS.iter().map(|f| f.key()).collect();
    assert_eq!(keys, ["decimal", "dms", "utm", "mgrs"]);
    for fmt in co::FORMATS {
        assert_eq!(Format::from_key(fmt.key()), Some(fmt));
    }
    assert_eq!(Format::from_key("ddm"), None);
}
