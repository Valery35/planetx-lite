// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Номенклатура листов на опубликованных листах. Перенос
//! `tests/test_sheets.py` модуля.

use data::sheets::{self, SOUTH_MARK};

const PERM: (f64, f64) = (58.01, 56.23);

fn russian(lat: f64, lon: f64, scale: u32) -> Option<String> {
    sheets::russian(lat, lon, scale, SOUTH_MARK)
}

fn some(text: &str) -> Option<String> {
    Some(text.to_string())
}

#[test]
fn perm() {
    assert_eq!(russian(PERM.0, PERM.1, 1000000), some("O-40"));
    assert_eq!(russian(PERM.0, PERM.1, 500000), some("O-40-А"));
    assert_eq!(russian(PERM.0, PERM.1, 200000), some("O-40-XV"));
}

#[test]
fn ryazan_examples() {
    // Примеры статьи «Советская система разграфки» - Рязань.
    assert_eq!(russian(54.63, 39.74, 500000), some("N-37-Б"));
    assert_eq!(russian(54.63, 39.74, 200000), some("N-37-XVI"));
}

#[test]
fn doubled_north_of_60() {
    // Белушья Губа - R-39-А,Б.
    assert_eq!(russian(71.55, 52.3, 1000000), some("R-39,40"));
    assert_eq!(russian(71.55, 52.3, 500000), some("R-39-А,Б"));
    assert_eq!(russian(68.3, 42.5, 200000), some("R-38-XXXI,XXXII"));
}

#[test]
fn quadrupled_and_tripled_north_of_76() {
    assert_eq!(russian(80.3, 55.0, 1000000), some("U-37,38,39,40"));
    assert_eq!(russian(80.3, 55.0, 200000), some("U-40-XXXI,XXXII,XXXIII"));
}

#[test]
fn pole_and_south() {
    assert_eq!(russian(89.0, 10.0, 1000000), some("Z"));
    assert_eq!(russian(89.0, 10.0, 200000), None);
    assert_eq!(russian(-1.0, 8.0, 1000000), some("A-32 (Ю. П.)"));
}

#[test]
fn unknown_scale_gives_nothing() {
    assert_eq!(russian(PERM.0, PERM.1, 100000), None);
}

#[test]
fn roman() {
    let out: Vec<String> = [4, 9, 14, 36].into_iter().map(sheets::roman).collect();
    assert_eq!(out, ["IV", "IX", "XIV", "XXXVI"]);
}

#[test]
fn imw() {
    assert_eq!(sheets::imw(PERM.0, PERM.1), "NO 40");
    assert_eq!(sheets::imw(-33.9, 18.4), "SI 34");
    assert_eq!(sheets::imw(89.5, 0.0), "NZ");
    assert_eq!(sheets::imw(10.0, 180.0), "NC 1");
}

#[test]
fn jog_published_sheets() {
    assert_eq!(sheets::jog(PERM.0, PERM.1), some("NO 40-6"));
    assert_eq!(sheets::jog(30.18, 67.0), some("NH 42-5")); // Кветта
    assert_eq!(sheets::jog(51.18, -115.57), some("NM 11-3")); // Банф
    assert_eq!(sheets::jog(67.28, 14.40), some("NQ 33,34-1")); // Будё
    assert_eq!(sheets::jog(67.85, 20.23), some("NQ 33,34-3")); // Кируна
    assert_eq!(sheets::jog(67.68, 21.6), some("NQ 33,34-4")); // Виттанги
    assert_eq!(sheets::jog(68.44, 17.43), some("NR 33,34-11")); // Нарвик
}

#[test]
fn jog_none_far_north() {
    assert_eq!(sheets::jog(78.2, 15.6), None);
}

#[test]
fn mgrs_square() {
    let square = sheets::mgrs_square(PERM.0, PERM.1);
    assert!(square.is_some());
    let Some(square) = square else { return };
    let (zone, letters) = square.split_at(4);
    assert_eq!(zone, "40V ", "{square}");
    assert!(letters.len() == 2 && letters.chars().all(|c| c.is_ascii_uppercase()), "{square}");
    assert_eq!(sheets::mgrs_square(86.0, 0.0), None);
}

#[test]
fn rows() {
    let rows = sheets::sheets(PERM.0, PERM.1, SOUTH_MARK);
    let systems: Vec<&str> = rows.iter().map(|r| r.0).collect();
    assert_eq!(systems, ["imw", "russian", "russian", "russian", "jog", "mgrs"]);
    assert_eq!(rows[0].1, Some(1000000));
    assert_eq!(rows[5].1, None);
    assert_eq!(rows[4].2, some("NO 40-6"));
}

#[test]
fn hostile_numbers_do_not_break() {
    for (lat, lon) in [(f64::NAN, 0.0), (0.0, f64::NAN), (f64::INFINITY, f64::NEG_INFINITY), (-90.0, -180.0), (90.0, 180.0)] {
        let rows = sheets::sheets(lat, lon, SOUTH_MARK);
        assert_eq!(rows.len(), 6);
    }
}
