// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Облака: дата снимка, адрес, прозрачность по белизне, тайл снимка.
//! Перенос `tests/test_clouds.py` модуля.

use data::clouds;
use data::gibs::Day;

fn utc(year: i32, month: u32, day: u32, hour: u32) -> f64 {
    Day { year, month, day }.seconds() + f64::from(hour) * 3600.0
}

#[test]
fn yesterday_after_noon() {
    assert_eq!(clouds::image_date(utc(2026, 9, 29, 13)), "2026-09-28");
}

#[test]
fn day_before_in_the_morning() {
    // Вчерашние сутки GIBS утром ещё собирает.
    assert_eq!(clouds::image_date(utc(2026, 9, 29, 6)), "2026-09-27");
}

#[test]
fn url_has_date_and_fields() {
    let url = clouds::url_template(utc(2026, 9, 29, 13));
    assert!(url.contains("/2026-09-28/GoogleMapsCompatible_Level9/"));
    assert!(url.ends_with("/{z}/{y}/{x}.jpg"));
    assert!(url.contains(clouds::LAYER));
}

#[test]
fn bad_time_still_gives_a_date() {
    // Не число - нулевой момент, далёкий - без переполнения.
    assert_eq!(clouds::image_date(f64::NAN), "1970-01-01");
    for t in [f64::INFINITY, -1e300] {
        assert!(clouds::image_date(t).len() >= 10);
    }
}

fn alpha(rgb: [u8; 3]) -> u8 {
    let out = clouds::cloud_rgba(&[rgb[0], rgb[1], rgb[2], 255], 1, None).unwrap_or_default();
    out.get(3).copied().unwrap_or(77)
}

#[test]
fn white_is_cloud() {
    assert_eq!(alpha([250, 250, 250]), 255);
}

#[test]
fn dark_and_gap_are_clear() {
    assert_eq!(alpha([0, 0, 0]), 0);
    assert_eq!(alpha([20, 60, 110]), 0);
    assert_eq!(alpha([60, 90, 40]), 0);
}

#[test]
fn sand_is_clear() {
    assert_eq!(alpha([225, 195, 150]), 0);
}

#[test]
fn colour_is_premultiplied() {
    let out = clouds::cloud_rgba(&[170; 4], 1, None).unwrap_or_default();
    assert_eq!(out.len(), 4);
    assert!(out[3] < 255);
    assert!(out[0] <= out[3]);
}

#[test]
fn bad_shape_is_none() {
    assert!(clouds::cloud_rgba(&[1, 2, 3], 1, None).is_none());
    assert!(clouds::cloud_rgba(&[1, 2, 3, 4], 0, None).is_none());
    assert!(clouds::cloud_rgba(&[1; 8], 4, None).is_none());
}

#[test]
fn clouds_fade_towards_the_cap() {
    // Тайл 3/0/0 - от 85° до 79.2° северной широты, сверху вниз.
    let white = vec![250u8; 256 * 256 * 4];
    let out = clouds::cloud_rgba(&white, 256, Some((3, 0, 0))).unwrap_or_default();
    assert_eq!(out.len(), white.len());
    let column: Vec<u8> = (0..256).map(|row| out[row * 256 * 4 + 3]).collect();
    assert_eq!(column[255], 255);
    assert!(column[0] < 5);
    assert!(column.windows(2).all(|w| w[1] >= w[0]));
}

#[test]
fn south_is_symmetric() {
    let lat = clouds::row_latitudes((3, 0, 7), 4);
    assert!(lat[3] < -80.0);
    let top = clouds::row_latitudes((0, 0, 0), 2);
    assert!((top[0] + top[1]).abs() < 1e-9);
}

#[test]
fn middle_latitudes_untouched() {
    let white = vec![250u8; 8 * 8 * 4];
    let out = clouds::cloud_rgba(&white, 8, Some((3, 4, 2))).unwrap_or_default();
    assert_eq!(out.len(), white.len());
    assert!(out.iter().skip(3).step_by(4).all(|a| *a == 255));
}

#[test]
fn deep_tile_takes_level_nine() {
    assert_eq!(clouds::source_key((12, 2800, 1200), clouds::MAX_LEVEL), (9, 350, 150));
    assert_eq!(clouds::source_key((5, 20, 9), clouds::MAX_LEVEL), (5, 20, 9));
    assert_eq!(clouds::source_key((60, 7, 7), 0), (0, 0, 0));
}
