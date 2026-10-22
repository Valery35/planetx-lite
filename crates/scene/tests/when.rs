// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Время меток: разбор строк KML, промежутки, видимость на шкале.
//! Перенос `tests/test_when.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use scene::when::{self, Time};

/// Секунды UTC даты, как calendar.timegm.
fn timegm(y: i64, m: i64, d: i64) -> f64 {
    // Счёт дней от 1970 года для григорианского календаря.
    let a = (14 - m) / 12;
    let yy = y + 4800 - a;
    let mm = m + 12 * a - 3;
    let jdn = d + (153 * mm + 2) / 5 + 365 * yy + yy / 4 - yy / 100 + yy / 400 - 32045;
    ((jdn - 2_440_588) * 86_400) as f64
}

fn t(a: &str, b: &str) -> Time {
    (a.to_string(), b.to_string())
}

fn day() -> f64 {
    timegm(2026, 9, 30)
}

#[test]
fn forms() {
    let day = day();
    assert_eq!(when::parse("2026-09-30"), Some(day));
    assert_eq!(when::parse("2026-09"), Some(timegm(2026, 9, 1)));
    assert_eq!(when::parse("2026"), Some(timegm(2026, 1, 1)));
    assert_eq!(when::parse("2026-09-30T12:30:00Z"), Some(day + 12.5 * 3600.0));
    assert_eq!(when::parse("2026-09-30T12:30:00"), Some(day + 12.5 * 3600.0));
    assert_eq!(when::parse("2026-09-30T17:30:00+05:00"), Some(day + 12.5 * 3600.0));
    assert_eq!(when::parse("2026-09-30T12:30:00.5Z"), Some(day + 12.5 * 3600.0 + 0.5));
}

#[test]
fn bad() {
    for text in ["", "вчера", "2026-13-01", "2026-09-30T25"] {
        assert_eq!(when::parse(text), None, "{text}");
    }
}

#[test]
fn text_round_trip() {
    assert_eq!(when::parse(&when::text(day() + 3661.0)), Some(day() + 3661.0));
}

#[test]
fn stamp_and_span() {
    let day = day();
    assert_eq!(when::interval(Some(&t("2026-09-30", "2026-09-30"))), Some((day, day)));
    let (lo, hi) = when::interval(Some(&t("2026-09-30", ""))).unwrap();
    assert_eq!(lo, day);
    assert_eq!(hi, f64::INFINITY);
    assert_eq!(when::interval(None), None);
    assert_eq!(when::interval(Some(&t("", ""))), None);
}

#[test]
fn visible() {
    let day = day();
    let span = t("2026-09-01", "2026-09-30");
    assert!(when::visible(Some(&span), day - 86400.0, day + 86400.0));
    assert!(!when::visible(Some(&span), day + 86400.0, day + 2.0 * 86400.0));
    // Метка без времени видна всегда.
    assert!(when::visible(None, 0.0, 1.0));
}

#[test]
fn extent() {
    let times = [t("2026-09-01", "2026-09-10"), t("2026-10-05", "2026-10-05"), t("", "2026-08-01")];
    let (lo, hi) = when::extent(times.iter().map(Some).chain([None])).unwrap();
    assert_eq!(lo, timegm(2026, 8, 1));
    assert_eq!(hi, timegm(2026, 10, 5));
    assert_eq!(when::extent([None, Some(&t("", ""))]), None);
}

#[test]
fn pack() {
    for time in [t("2026-09-30", "2026-09-30"), t("2026", "2027"), t("", "2026-09-30"), t("2026-09-30T12:00:00Z", "")] {
        assert_eq!(when::unpack(&when::pack(Some(&time))), Some(time.clone()));
    }
    assert_eq!(when::pack(None), "");
    assert_eq!(when::unpack(""), None);
}

// Даты до нашей эры и до 1970 года.

#[test]
fn matches_calendar() {
    for (y, m, d) in [(1, 1, 1), (100, 5, 1), (1600, 2, 29), (1969, 12, 31), (2000, 2, 29), (2026, 9, 30)] {
        assert_eq!(when::parse(&format!("{y:04}-{m:02}-{d:02}")), Some(timegm(y, m, d)), "{y}-{m}-{d}");
    }
}

#[test]
fn year_before_era() {
    // «-0001» - 1 год до н. э., он идёт сразу перед 1 годом н. э.
    assert_eq!(when::parse("0001").unwrap() - when::parse("-0001").unwrap(), 366.0 * 86400.0);
    assert!(when::parse("-0264").unwrap() < when::parse("-0146").unwrap());
    let c = when::civil(when::parse("-0216-08-02").unwrap());
    assert_eq!((c.0, c.1, c.2), (-215, 8, 2));
}

#[test]
fn text_round_trip_before_era() {
    for text in ["-0216-08-02T00:00:00Z", "0044-03-15T12:00:00Z", "1600-02-29T23:59:59Z"] {
        assert_eq!(when::text(when::parse(text).unwrap()), text);
    }
}

#[test]
fn span_is_visible() {
    let war = t("-0218", "-0201");
    assert!(when::visible(Some(&war), when::parse("-0216").unwrap(), when::parse("-0215").unwrap()));
    assert!(!when::visible(Some(&war), when::parse("-0149").unwrap(), when::parse("-0146").unwrap()));
}

#[test]
fn long_years() {
    // Годы длиннее четырёх цифр - палеогеография и глубокое время.
    assert_eq!(when::civil(when::parse("-28000").unwrap()).0, -27999);
    assert_eq!(when::civil(when::parse("12026").unwrap()).0, 12026);
}

#[test]
fn inside_span() {
    let war = t("-0218", "-0216");
    let (lo, hi) = when::interval(Some(&war)).unwrap();
    assert_eq!(when::share(Some(&war), lo), 0.0);
    assert_eq!(when::share(Some(&war), hi), 1.0);
    assert!((when::share(Some(&war), (lo + hi) / 2.0) - 0.5).abs() < 1e-12);
    assert_eq!(when::share(Some(&war), hi + 1.0e9), 1.0);
}

#[test]
fn nothing_to_grow() {
    for time in [None, Some(t("2026", "2026")), Some(t("2026", ""))] {
        assert_eq!(when::share(time.as_ref(), 0.0), 1.0, "{time:?}");
    }
}
