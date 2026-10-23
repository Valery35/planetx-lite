// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Экранные органы навигации: кольцо, джойстики, ползунок. Перенос
//! `tests/test_navpad.py` модуля.

use globe_core::ellipsoid::EARTH;
use globe_core::navpad as np;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn angles_clockwise_from_top() {
    assert!(close(np::ring_angle(0.0, -1.0), 0.0));
    assert!(close(np::ring_angle(1.0, 0.0), 90.0));
    assert!(close(np::ring_angle(0.0, 1.0), 180.0));
    assert!(close(np::ring_angle(-1.0, 0.0), 270.0));
}

#[test]
fn delta_is_shortest() {
    assert!(close(np::angle_delta(350.0, 10.0), 20.0));
    assert!(close(np::angle_delta(10.0, 350.0), -20.0));
}

#[test]
fn north_follows_ring() {
    // Кольцо тянут по часовой на 30° - буква N уходит туда же.
    let heading = 40.0;
    let turned = heading + np::ring_turn(0.0, 30.0);
    assert!(close(np::angle_delta(np::north_angle(heading), np::north_angle(turned)), 30.0));
}

#[test]
fn stick_is_clamped() {
    let (sx, sy) = np::stick(30.0, 40.0, 10.0);
    assert!(close(sx.hypot(sy), 1.0));
    assert_eq!(np::stick(5.0, 0.0, 10.0), (0.5, 0.0));
}

#[test]
fn look_up_raises_tilt() {
    let (dh, dt) = np::look_step(0.0, -1.0, 1.0);
    assert_eq!(dh, 0.0);
    assert!(close(dt, np::LOOK_RATE));
}

#[test]
fn move_up_goes_forward() {
    let (forward, right) = np::move_step(0.0, -1.0, 2.0, 1000.0, EARTH.a);
    assert!(close(forward, 2.0 * np::MOVE_RATE * 1000.0));
    assert_eq!(right, 0.0);
}

#[test]
fn move_from_space_is_capped() {
    // С 20 000 км полоса около 23 000 км, скорость - от радиуса тела.
    let (forward, _) = np::move_step(0.0, -1.0, 1.0, 2.3e7, EARTH.a);
    assert!(close(forward, np::MOVE_RATE * np::MOVE_WIDTH_RADII * EARTH.a));
}

#[test]
fn plus_halves_in_a_second() {
    assert!(close(np::zoom_factor(1.0, 1.0), 0.5));
    assert!(close(np::zoom_factor(-1.0, 1.0), 2.0));
}

#[test]
fn slider_round_trip_and_ends() {
    let far = np::slider_far(EARTH.a);
    for d in [150.0, 3000.0, 1.0e6] {
        assert!(close(np::slider_distance(np::slider_share(d, np::NEAR, far), np::NEAR, far) / d, 1.0));
    }
    assert_eq!(np::slider_share(1.0, np::NEAR, far), 0.0);
    assert_eq!(np::slider_share(1.0e9, np::NEAR, far), 1.0);
}
