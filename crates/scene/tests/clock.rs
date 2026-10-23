// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Ход шкалы времени, перенос `tests/test_clock.py` модуля.

use scene::clock::{self, HALF};

const NOW: f64 = 1.8e9;

#[test]
fn clock_alone_is_window_around_moment() {
    assert_eq!(clock::extent(None, Some(NOW), None, true), Some((NOW - HALF, NOW + HALF)));
}

#[test]
fn without_clock_data_extent_only() {
    assert_eq!(clock::extent(None, Some(NOW), None, false), None);
    assert_eq!(clock::extent(Some((1.0, 2.0)), Some(NOW), None, false), Some((1.0, 2.0)));
}

#[test]
fn union_with_data_and_left_handle() {
    let (lo, hi) = clock::extent(Some((NOW - 10.0 * HALF, NOW - 9.0 * HALF)), Some(NOW), Some(NOW - 5.0 * HALF), true).unwrap_or_default();
    assert_eq!(lo, NOW - 10.0 * HALF);
    assert_eq!(hi, NOW + HALF);
    assert_eq!(clock::window(NOW, Some(NOW - 5.0 * HALF)).0, NOW - 5.0 * HALF);
}

#[test]
fn right_edge_runs_from_left_to_end() {
    let span = (0.0, 100.0);
    assert_eq!(clock::sweep(0.0, 0.0, span, 30.0, false), (30.0, true));
    assert_eq!(clock::sweep(0.0, 90.0, span, 30.0, false), (100.0, false));
    assert_eq!(clock::sweep(0.0, 90.0, span, 30.0, true), (0.0, true));
}

#[test]
fn full_range_does_not_move_by_advance() {
    // Так показ стоял: промежуток на всю шкалу сразу на краю.
    assert!(!clock::advance(0.0, 100.0, (0.0, 100.0), 5.0, false, false).3);
}

#[test]
fn rate_is_real_time_multiple() {
    assert_eq!(clock::shift((0.0, 100.0), Some(3600.0), 0.04, clock::FIT_SECONDS), 144.0);
    // Вся шкала за 20 с: за секунду - двадцатая доля охвата.
    assert_eq!(clock::shift((0.0, 100.0), None, 1.0, clock::FIT_SECONDS), 5.0);
}

#[test]
fn clock_runs_past_extent() {
    assert_eq!(clock::advance(NOW, NOW, (NOW - 5.0, NOW + 5.0), 10.0, true, false), (NOW + 10.0, NOW + 10.0, None, true));
}

#[test]
fn data_stops_or_loops_at_end() {
    assert_eq!(clock::advance(8.0, 9.0, (0.0, 10.0), 3.0, false, false), (9.0, 10.0, Some((0.0, 10.0)), false));
    assert_eq!(clock::advance(8.0, 9.0, (0.0, 10.0), 3.0, false, true), (0.0, 1.0, Some((0.0, 10.0)), true));
    assert_eq!(clock::advance(1.0, 2.0, (0.0, 10.0), 3.0, false, false), (4.0, 5.0, Some((0.0, 10.0)), true));
}
