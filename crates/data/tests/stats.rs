// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сводка значений участка, перенос `tests/test_stats.py` модуля.

#![allow(clippy::expect_used)]

use data::stats::{describe, rounded};

#[test]
fn values_of_known_row() {
    let row: Vec<f64> = (1..=101).map(f64::from).collect();
    let rep = describe(&row).expect("сводка");
    assert_eq!(rep.count, 101);
    assert!((rep.mean - 51.0).abs() < 1e-9);
    assert!((rep.std - 850.0_f64.sqrt()).abs() < 1e-9);
    assert_eq!((rep.low, rep.high), (1.0, 101.0));
    assert_eq!((rep.p10, rep.q1, rep.median, rep.q3, rep.p90), (11.0, 26.0, 51.0, 76.0, 91.0));
}

#[test]
fn quantile_between_values_is_interpolated() {
    let rep = describe(&[0.0, 10.0]).expect("сводка");
    assert_eq!((rep.p10, rep.q1, rep.median, rep.q3, rep.p90), (1.0, 2.5, 5.0, 7.5, 9.0));
}

#[test]
fn empty_cells_stay_outside() {
    let rep = describe(&[1.0, f64::NAN, f64::INFINITY, 3.0]).expect("сводка");
    assert_eq!(rep.count, 2);
    assert_eq!(rep.mean, 2.0);
}

#[test]
fn nothing_to_describe() {
    assert!(describe(&[f64::NAN, f64::NEG_INFINITY]).is_none());
    assert!(describe(&[]).is_none());
}

#[test]
fn three_digits() {
    assert_eq!(rounded(0.0284), "0.03");
    assert_eq!(rounded(28.74), "28.7");
    assert_eq!(rounded(-1234.5), "-1234");
}
