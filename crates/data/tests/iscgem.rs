// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Исторические землетрясения ISC-GEM: промежутки, разбор, файл. Перенос
//! `tests/test_iscgem.py` модуля.

use data::iscgem;

/// Начало ответа ComCat catalog=iscgem 17 октября 2026 года, сокращено.
const CSV: &str = "time,latitude,longitude,depth,mag,magType,nst,gap,dmin,rms,net,id,updated,place,type\n\
1904-04-04T10:02:34.560Z,41.802,23.108,15,7.02,mw,,,,,iscgem,iscgem16957905,2022-04-26T18:08:12.000Z,\"1 km NE of Kresna, Bulgaria\",earthquake\n\
1906-04-18T13:12:26Z,37.75,-122.55,,7.9,mw,,,,,iscgem,iscgem16957602,2022-04-26T18:08:12.000Z,San Francisco,earthquake\n\
1910-01-01T00:00:00.000Z,,,10,6.0,mw,,,,,iscgem,bad,,,earthquake\n";

#[test]
fn periods_cover_years_without_gaps() {
    let periods = iscgem::periods(2026);
    assert_eq!(periods.first(), Some(&("1900-01-01".to_string(), "1960-01-01".to_string())));
    assert_eq!(periods.last().map(|p| p.1.as_str()), Some("2027-01-01"));
    for pair in periods.windows(2) {
        assert_eq!(pair[0].1, pair[1].0);
    }
    assert!(iscgem::query_url("1900-01-01", "1960-01-01").contains("&starttime=1900-01-01&endtime=1960-01-01"));
}

#[test]
fn parse_skips_rows_without_place() {
    let events = iscgem::parse(CSV.as_bytes());
    assert_eq!(events.len(), 2);
    let [first, second] = events.as_slice() else { return };
    assert!((first.lat - 41.802).abs() < 1e-9);
    assert!((first.mag - 7.02).abs() < 1e-9);
    assert_eq!(first.place, "1 km NE of Kresna, Bulgaria");
    assert!(first.url.ends_with("/iscgem16957905"));
    // 1904-04-04 10:02:34.56 UTC.
    let time = first.time.unwrap_or(f64::NAN);
    assert!((time - -2_074_687_045.44).abs() < 1e-3, "{time}");
    // Секунды до 1970 года отрицательны, время без долей тоже читается.
    assert!(second.time.is_some_and(|t| t < 0.0));
    assert_eq!(second.depth, 0.0);
}

#[test]
fn bad_time_is_none() {
    let text = "time,latitude,longitude,depth,mag\n1904-13-04T10:02:34Z,1,2,3,4\nвчера,1,2,3,4\n";
    let events = iscgem::parse(text.as_bytes());
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|q| q.time.is_none()));
}

#[test]
fn file_round_trip() {
    let events = iscgem::parse(CSV.as_bytes());
    let bytes = iscgem::save(&events);
    assert!(bytes.is_ok());
    let Ok(bytes) = bytes else { return };
    assert_eq!(iscgem::load(&bytes), Ok(events));
    let empty = iscgem::save(&[]);
    assert!(empty.is_ok());
    let Ok(empty) = empty else { return };
    assert_eq!(iscgem::load(&empty), Ok(Vec::new()));
}

#[test]
fn hostile_file_is_error() {
    assert!(iscgem::load(b"PK\x03\x04 not a zip").is_err());
    assert!(iscgem::load(b"").is_err());
}
