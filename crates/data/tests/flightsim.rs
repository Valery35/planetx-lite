// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Режим полёта: ход самолёта по клавишам и деления прибора. Перенос
//! `tests/test_flightsim.py` модуля.

use data::flightsim::{self as fs, Controls, Plane};

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

fn plane() -> Plane {
    Plane::new(60.0, 25.0, 1000.0, 90.0, None, None)
}

fn keys(pitch: f64, roll: f64, throttle: f64) -> Controls {
    Controls { pitch, roll, throttle }
}

fn none() -> Controls {
    Controls::default()
}

/// Расстояние по шару радиуса WGS84, как `_metres` в `core/trains.py`.
fn metres(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dl = (lon2 - lon1).to_radians();
    let a = ((p2 - p1) / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * fs::EARTH_RADIUS * a.sqrt().min(1.0).asin()
}

#[test]
fn level_flight_keeps_height_course_and_speed() {
    let mut p = plane();
    fs::step(&mut p, 10.0, none(), 0.0);
    close(p.alt, 1000.0, 1e-6);
    close(p.heading, 90.0, 1e-9);
    close(p.speed, fs::START_SPEED, 1e-6);
    close(metres(60.0, 25.0, p.lat, p.lon), 10.0 * fs::START_SPEED, 1.0);
    assert!(p.lon > 25.0);
}

#[test]
fn right_bank_turns_right_and_levels_back() {
    let mut p = plane();
    fs::step(&mut p, 1.0, keys(0.0, 1.0, 0.0), 0.0);
    close(p.roll, fs::ROLL_RATE, 1e-7);
    fs::step(&mut p, 3.0, none(), 0.0);
    assert!(p.heading > 90.5);
    assert_eq!(p.roll, 0.0);
    let mut p = plane();
    fs::step(&mut p, 1.0, keys(0.0, -1.0, 0.0), 0.0);
    fs::step(&mut p, 1.0, none(), 0.0);
    assert!(p.heading < 89.5);
}

#[test]
fn nose_up_climbs_and_slows() {
    let mut p = plane();
    fs::step(&mut p, 1.0, keys(1.0, 0.0, 0.0), 0.0);
    close(p.pitch, fs::PITCH_RATE, 1e-7);
    fs::step(&mut p, 5.0, none(), 0.0);
    assert!(p.alt > 1050.0);
    assert!(p.climb > 0.0);
    assert!(p.speed < fs::START_SPEED);
    fs::step(&mut p, 10.0, keys(1.0, 0.0, 0.0), 0.0);
    assert_eq!(p.pitch, fs::PITCH_MAX);
}

#[test]
fn throttle_accelerates_to_top_speed() {
    let mut p = plane();
    fs::step(&mut p, 3.0, keys(0.0, 0.0, 1.0), 0.0);
    assert_eq!(p.throttle, 1.0);
    fs::step(&mut p, 300.0, none(), 0.0);
    close(p.speed, fs::MAX_SPEED, 0.5);
}

#[test]
fn stall_drops_nose() {
    let mut p = plane();
    p.speed = fs::STALL - 5.0;
    p.throttle = 0.0;
    fs::step(&mut p, 1.0, none(), 0.0);
    assert!(p.pitch < 0.0);
}

#[test]
fn touching_ground_ends_flight() {
    let mut p = plane();
    fs::step(&mut p, 2.0, keys(-1.0, 0.0, 0.0), 0.0);
    fs::step(&mut p, 60.0, none(), 900.0);
    assert!(p.crashed);
    assert_eq!(p.alt, 900.0);
    let lat = p.lat;
    fs::step(&mut p, 5.0, none(), 0.0);
    assert_eq!(p.lat, lat);
}

#[test]
fn readout() {
    let p = plane();
    let out = fs::readout(&p, 200.0);
    close(out.speed, 500.0, 1e-7);
    assert_eq!(out.height, 800.0);
    close(out.throttle, 100.0 * p.throttle, 1e-12);
}

#[test]
fn hostile_time_step() {
    let mut p = plane();
    let before = p.clone();
    fs::step(&mut p, f64::NAN, none(), 0.0);
    fs::step(&mut p, -5.0, none(), 0.0);
    assert_eq!(p, before);
    fs::step(&mut p, f64::INFINITY, none(), 0.0);
    assert!(p.lat.is_finite() && p.lon.is_finite());
}

#[test]
fn tape_marks_around_value() {
    let marks = fs::tape(1013.0, 100.0, 500.0);
    let values: Vec<f64> = marks.iter().map(|m| m.0).collect();
    assert_eq!(values, [600.0, 700.0, 800.0, 900.0, 1000.0, 1100.0, 1200.0, 1300.0, 1400.0, 1500.0]);
    let at = marks.iter().find(|m| m.0 == 1000.0).map_or(f64::NAN, |m| m.1);
    close(at, -13.0, 1e-9);
    assert!(fs::tape(0.0, 0.0, 10.0).is_empty());
    assert!(fs::tape(0.0, 1e-9, 1e9).is_empty());
}

#[test]
fn heading_tape_wraps_over_north() {
    let marks = fs::heading_tape(355.0, 10.0, 20.0);
    let values: Vec<f64> = marks.iter().map(|m| m.0).collect();
    assert_eq!(values, [340.0, 350.0, 0.0, 10.0]);
    close(marks[2].1, 5.0, 1e-9);
}

#[test]
fn ladder_follows_pitch() {
    let steps = fs::ladder(12.0, 5.0, 10.0);
    let values: Vec<f64> = steps.iter().map(|m| m.0).collect();
    assert_eq!(values, [5.0, 10.0, 15.0, 20.0]);
    let at = steps.iter().find(|m| m.0 == 10.0).map_or(f64::NAN, |m| m.1);
    close(at, -2.0, 1e-9);
    assert_eq!(fs::ladder(85.0, 5.0, 10.0).last().map(|m| m.0), Some(90.0));
}

#[test]
fn horizon_offset_matches_projection() {
    // Горизонт при тангаже 10° вверх стоит ниже середины кадра на
    // focal · tg 10°.
    let focal = 500.0;
    close(fs::ladder_offset(0.0 - 10.0, focal), -focal * 10f64.to_radians().tan(), 1e-9);
}

#[test]
fn fighter_and_light_top_speeds() {
    let mut tops = Vec::new();
    for kind in [fs::FIGHTER, fs::LIGHT] {
        let mut p = Plane::new(60.0, 25.0, 9000.0, 90.0, None, Some(kind));
        assert_eq!(p.speed, p.spec.start);
        p.throttle = 1.0;
        fs::step(&mut p, 600.0, keys(0.0, 0.0, 1.0), 0.0);
        tops.push(p.speed * 3.6);
    }
    close(tops[0], 1998.0, 5.0);
    close(tops[1], 342.0, 3.0);
}

#[test]
fn fighter_banks_faster() {
    let mut rolls = Vec::new();
    for kind in [fs::FIGHTER, fs::LIGHT] {
        let mut p = Plane::new(60.0, 25.0, 1000.0, 90.0, None, Some(kind));
        fs::step(&mut p, 0.5, keys(0.0, 1.0, 0.0), 0.0);
        rolls.push(p.roll);
    }
    assert!(rolls[0] > rolls[1]);
    assert_eq!(fs::kind_spec(Some("unknown")), fs::MIDDLE);
}
