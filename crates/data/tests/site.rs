// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Участок окон анализа: квадрат, рамка, размер, проверка перед расчётом.

use data::site::{self, Problem, Site};

const LAT: f64 = 58.0;
const LON: f64 = 56.25;

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

#[test]
fn square_has_its_side() {
    let ring = site::square_ring(LAT, LON, 3000.0);
    assert_eq!(ring.len(), 4);
    let found = site::ring_box(&ring);
    assert!(found.is_some());
    let Some(((west, south, east, north), side)) = found else { return };
    close(side, 3000.0, 1.0);
    close((south + north) / 2.0, LAT, 1e-12);
    close((west + east) / 2.0, LON, 1e-12);
    assert!(site::ring_box(&[]).is_none());
}

#[test]
fn inside_follows_ring() {
    let ring = site::square_ring(LAT, LON, 2000.0);
    let got = site::inside(&[(LAT, LON), (LAT + 0.02, LON)], &ring);
    assert_eq!(got, vec![true, false]);
}

#[test]
fn site_checks_before_building() {
    let mut s = Site::new(LAT, LON, 3.0, 40.0);
    assert!(s.closed());
    assert_eq!(s.check(), None);
    close(s.reach(), 1500.0 * 2f64.sqrt(), 5.0);
    s.start_drawing();
    assert_eq!(s.check(), Some(Problem::Open));
    // Рамка у рисуемого участка - квадрат у точки.
    let (w, _, e, _) = s.bbox();
    assert!(w < LON && e > LON);
    s.ring = site::square_ring(LAT, LON, 50000.0);
    s.drawing = false;
    assert_eq!(s.check(), Some(Problem::TooLarge(40.0)));
    s.move_to(LAT + 1.0, LON);
    assert_eq!(s.check(), None);
    close(s.centre().0, LAT + 1.0, 1e-9);
    // Сторона не выходит за предел.
    assert_eq!(Site::new(LAT, LON, 99.0, 40.0).side_km, 40.0);
}
