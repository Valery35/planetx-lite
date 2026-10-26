// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Ряд снимков участка: продукты, шаг, сводка, деления оси дат, CSV.
//! Перенос `tests/test_series.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use data::sentinel::{Grid, Mission};
use data::series::{self, Point, Row, SERIES_PIXELS, TICKS};

fn mission(name: &str) -> Mission {
    Mission::from_name(name).unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-7
}

/// Процентиль по правилу numpy, для сверки.
fn reference(values: &[f64], p: f64) -> f64 {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let at = p / 100.0 * (v.len() - 1) as f64;
    let (i, j) = (at.floor() as usize, at.ceil() as usize);
    v[i] + (v[j] - v[i]) * (at - i as f64)
}

#[test]
fn indices_only() {
    assert_eq!(series::products(mission("sentinel2")), ["ndvi", "ndwi", "ndmi", "nbr", "ndsi"]);
    assert_eq!(series::products(mission("landsat")).last(), Some(&"lst"));
    assert_eq!(series::products(mission("mss")), ["ndvi", "ndwi"]);
}

#[test]
fn modis_without_classes_and_burn_day() {
    assert_eq!(series::products(mission("modis_veg")), ["modis_ndvi", "modis_evi"]);
    assert!(series::products(mission("modis_snow8")).is_empty());
    assert!(series::products(mission("modis_burn")).is_empty());
}

#[test]
fn aster_has_no_series() {
    assert!(series::products(mission("aster")).is_empty());
    assert!(series::products(mission("aster_nasa")).is_empty());
}

#[test]
fn small_area_keeps_native_step() {
    let xy = [(0.0, 0.0), (1000.0, 0.0), (1000.0, 1000.0), (0.0, 1000.0)];
    assert_eq!(series::step(&xy, mission("sentinel2")), 10.0);
    assert_eq!(series::step(&xy, mission("landsat")), 30.0);
}

#[test]
fn large_area_is_coarse() {
    let xy = [(0.0, 0.0), (50000.0, 0.0), (50000.0, 50000.0), (0.0, 50000.0)];
    let res = series::step(&xy, mission("sentinel2"));
    assert_eq!(res, 250.0);
    assert!(50000.0 / res <= SERIES_PIXELS);
}

#[test]
fn median_and_band_over_clear_pixels() {
    let values = Grid::from_vec(10, 10, (0..100).map(|v| v as f32).collect()).unwrap();
    let area = Grid::filled(10, 10, true);
    let mut clear = Grid::filled(10, 10, true);
    for r in 0..10 {
        for c in 5..10 {
            clear.set(r, c, false); // облако на правой половине
        }
    }
    let p = series::point(&values, &area, &clear).unwrap();
    assert!(close(p.share, 50.0));
    let expected: Vec<f64> = (0..10).flat_map(|r| (0..5).map(move |c| (r * 10 + c) as f64)).collect();
    assert!(close(p.median, reference(&expected, 50.0)));
    assert!(close(p.low, reference(&expected, 10.0)));
    assert!(close(p.high, reference(&expected, 90.0)));
}

#[test]
fn nan_counts_as_missing() {
    let mut values = Grid::filled(10, 10, f32::NAN);
    for c in 0..9 {
        values.set(0, c, 0.5);
    }
    let area = Grid::filled(10, 10, true);
    assert!(series::point(&values, &area, &area).is_none());
    for c in 0..10 {
        values.set(1, c, 0.7);
    }
    let p = series::point(&values, &area, &area).unwrap();
    assert!(close(p.share, 19.0));
}

#[test]
fn outside_area_is_ignored() {
    let mut values = Grid::filled(4, 4, 0.0_f32);
    values.set(0, 0, 100.0);
    let mut area = Grid::filled(4, 4, true);
    area.set(0, 0, false);
    let p = series::point(&values, &area, &Grid::filled(4, 4, true)).unwrap();
    assert_eq!(p.high, 0.0);
}

#[test]
fn empty_area() {
    let none = series::point(&Grid::filled(2, 2, 0.0), &Grid::filled(2, 2, false), &Grid::filled(2, 2, true));
    assert!(none.is_none());
    // Разные размеры - сводки нет.
    let odd = series::point(&Grid::filled(2, 2, 0.0), &Grid::filled(2, 3, true), &Grid::filled(2, 2, true));
    assert!(odd.is_none());
}

#[test]
fn day_number() {
    assert_eq!(series::day_number("1970-01-02"), Some(1.0));
    assert!(close(series::day_number("1970-01-01 12:00").unwrap(), 0.5));
    assert_eq!(series::day_number("2024-02-30"), None);
    assert_eq!(series::day_number("20x4-01-01"), None);
    assert_eq!(series::day_number("2024"), None);
    assert_eq!(series::day_number("2024-01-01 1x:00"), None);
}

#[test]
fn year_ticks() {
    let first = series::day_number("2016-03-05").unwrap();
    let last = series::day_number("2026-10-01").unwrap();
    let ticks = series::date_ticks(first, last, TICKS);
    assert!(!ticks.is_empty());
    assert!(ticks.iter().all(|(_, label)| label.bytes().all(|c| c.is_ascii_digit())));
    assert!(ticks.len() <= 6);
    assert!(ticks[0].0 >= first);
    assert!(ticks[ticks.len() - 1].0 <= last);
}

#[test]
fn month_ticks_start_on_first_day() {
    let first = series::day_number("2024-01-17").unwrap();
    let last = series::day_number("2024-12-20").unwrap();
    let ticks = series::date_ticks(first, last, TICKS);
    assert_eq!(ticks[0].1, "2024-03");
    assert!(ticks.iter().all(|(_, label)| label.len() == 7));
}

#[test]
fn day_ticks() {
    let first = series::day_number("2024-06-01").unwrap();
    let last = series::day_number("2024-06-20").unwrap();
    let ticks = series::date_ticks(first, last, TICKS);
    assert_eq!(ticks[0].1, "2024-06-01");
    assert!(ticks.len() <= 6);
}

#[test]
fn hostile_tick_range() {
    assert!(series::date_ticks(f64::NAN, 10.0, TICKS).is_empty());
    let wide = series::date_ticks(-1e300, 1e300, TICKS);
    assert!(!wide.is_empty() && wide.len() <= 1000);
}

#[test]
fn csv_rows() {
    let rows = [
        Row {
            when: "2024-06-01 10:00".into(),
            scene: "S2A_1".into(),
            value: Some(Point {
                median: 0.5,
                low: 0.4,
                high: 0.6,
                share: 80.0,
            }),
        },
        Row {
            when: "2024-06-06 10:00".into(),
            scene: "S2B_2".into(),
            value: None,
        },
    ];
    let text = series::csv_text(&rows, "NDVI");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1], "2024-06-01 10:00,S2A_1,0.5000,0.4000,0.6000,80.0,\"NDVI\"");
    assert_eq!(lines[2], "2024-06-06 10:00,S2B_2,,,,,\"NDVI\"");
}
