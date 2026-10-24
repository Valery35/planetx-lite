// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Живые объекты: ход по курсу и дуге, переход, давность, наклон модели,
//! камера следом. Перенос класса TestMovers из `tests/test_movers.py`
//! модуля, классы TestVessels и TestAircraft - в тестах `vessels`
//! и `aircraft`.

use data::movers::{self, Movers, Sample};
use data::trains::metres;

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

/// Положение по позиционным полям Sample модуля: широта, долгота,
/// скорость, курс, азимут, высота, подъём.
fn moving(key: &str, t: f64, v: [f64; 7]) -> Sample<String> {
    let [lat, lon, speed, course, heading, alt, climb] = v;
    Sample { speed, course, heading, alt, climb, ..Sample::new(key.to_string(), t, lat, lon) }
}

fn only(fleet: &mut Movers<String, ()>, now: f64, clock: f64) -> (f64, f64, f64, f64) {
    let found = fleet.positions(now, clock);
    assert_eq!(found.len(), 1);
    found.first().map_or((f64::NAN, f64::NAN, f64::NAN, f64::NAN), |p| (p.1, p.2, p.3, p.4))
}

#[test]
fn course_speed_and_climb() {
    let mut fleet: Movers<String, ()> = Movers::new(30.0, movers::STALE);
    fleet.update(vec![moving("a", 1000.0, [60.0, 25.0, 100.0, 90.0, f64::NAN, 1000.0, 5.0])], 0.0, 1000.0, false);
    let (lat, lon, heading, alt) = only(&mut fleet, 0.0, 1010.0);
    close(metres(60.0, 25.0, lat, lon), 1000.0, 0.5);
    assert_eq!(heading, 90.0);
    close(alt, 1050.0, 1e-9);
    // Не дольше extrapolate.
    let (lat, lon, _, alt) = only(&mut fleet, 0.0, 2000.0);
    close(metres(60.0, 25.0, lat, lon), 3000.0, 1.0);
    close(alt, 1150.0, 1e-9);
}

#[test]
fn view_part_keeps_others_until_stale() {
    let mut fleet: Movers<String, ()> = Movers::new(movers::EXTRAPOLATE, 100.0);
    fleet.update(vec![Sample::new("a".to_string(), 1000.0, 60.0, 25.0)], 0.0, 1000.0, false);
    fleet.update(vec![Sample::new("b".to_string(), 1050.0, 61.0, 25.0)], 0.0, 1050.0, true);
    assert_eq!(fleet.items.keys().collect::<Vec<_>>(), ["a", "b"]);
    fleet.update(Vec::new(), 0.0, 1101.0, true);
    assert_eq!(fleet.items.keys().collect::<Vec<_>>(), ["b"]);
    fleet.update(Vec::new(), 0.0, 1102.0, false);
    assert_eq!(fleet.len(), 0);
}

#[test]
fn chase_puts_object_on_view_ray() {
    // Самолёт на 10 км, наклон 60°, курс на восток, камера в 4 км позади:
    // точка взгляда впереди на 10 км · tan 60°, расстояние до неё
    // 4 км + 10 км / cos 60°.
    let (lat, lon, look, distance) = movers::chase(60.0, 25.0, 90.0, 10000.0, 4000.0, 60.0);
    close(metres(60.0, 25.0, lat, lon), 10000.0 * 60f64.to_radians().tan(), 1.0);
    assert!(lon > 25.0);
    close(distance, 24000.0, 1e-6);
    // Азимут в точке взгляда - продолжение дуги: на 60° с. ш. за 17 км на
    // восток курс больше 90° на схождение меридианов.
    assert!(look > 90.0 && look < 90.5);
    // Поезд на земле: точка взгляда - он сам, курс прежний.
    assert_eq!(movers::chase(60.0, 25.0, 30.0, f64::NAN, 2000.0, 60.0), (60.0, 25.0, 30.0, 2000.0));
    // Камера не ближе CHASE_MIN.
    assert_eq!(movers::chase(60.0, 25.0, 30.0, 0.0, 1.0, 60.0).3, movers::CHASE_MIN);
}

#[test]
fn chase_on_takes_height_over_look_point() {
    // Рельеф растёт на восток на 10 м на км: точка взгляда впереди выше,
    // высота над ней меньше, точка ближе, чем над ровным.
    let ground = |_lat: f64, lon: f64| (lon - 25.0) * 0.5567 * 1000.0 * 10.0 / 1000.0 * 100.0;
    let flat = movers::chase(60.0, 25.0, 90.0, 1000.0, 150.0, 75.0);
    let (lat, lon, _, _) = movers::chase_on(60.0, 25.0, 90.0, 1000.0, &ground, 150.0, 75.0);
    let ahead = metres(60.0, 25.0, lat, lon);
    let height = 1000.0 - ground(lat, lon);
    assert!(ahead < metres(60.0, 25.0, flat.0, flat.1));
    close(ahead, height * 75f64.to_radians().tan(), 5.0);
}

#[test]
fn turning_object_follows_arc() {
    // 100 м/с, поворот 3°/с: за 30 с курс 90° -> 180°, хорда - на азимут
    // 135°, длина 2R·sin 45°, R = v / ω.
    let mut fleet: Movers<String, ()> = Movers::new(60.0, movers::STALE);
    let sample = Sample { roll: f64::NAN, turn: 3.0, ..moving("a", 1000.0, [60.0, 25.0, 100.0, 90.0, 90.0, 1000.0, 0.0]) };
    fleet.update(vec![sample], 0.0, 1000.0, false);
    let (lat, lon, heading, _) = only(&mut fleet, 0.0, 1030.0);
    close(heading, 180.0, 1e-6);
    let radius = 100.0 / 3f64.to_radians();
    close(metres(60.0, 25.0, lat, lon), 2.0 * radius * 45f64.to_radians().sin(), 2.0);
    assert!(lat < 60.0 && lon > 25.0);
}

#[test]
fn turn_from_two_samples_and_attitude() {
    let mut fleet: Movers<String, ()> = Movers::default();
    let first = Sample { roll: f64::NAN, ..moving("a", 1000.0, [60.0, 25.0, 100.0, 90.0, f64::NAN, 1000.0, 5.0]) };
    fleet.update(vec![first], 0.0, 1000.0, false);
    let second = Sample { roll: f64::NAN, ..moving("a", 1010.0, [60.0, 25.02, 100.0, 110.0, f64::NAN, 1050.0, 5.0]) };
    fleet.update(vec![second], 1.0, 1010.0, false);
    let key = "a".to_string();
    close(fleet.items.get(&key).map_or(f64::NAN, |i| i.sample.turn), 2.0, 1e-12);
    let attitude = fleet.attitude(&key);
    assert!(attitude.is_some());
    let Some((pitch, roll)) = attitude else { return };
    close(pitch, 5f64.atan2(100.0).to_degrees(), 1e-12);
    close(roll, (2f64.to_radians() * 100.0 / movers::G).atan().to_degrees(), 1e-12);
    // Через полкруга от 350° к 10° - поворот вправо, не влево.
    let a = Sample { speed: 50.0, course: 350.0, ..Sample::new("b", 0.0, 0.0, 0.0) };
    let b = Sample { speed: 50.0, course: 10.0, ..Sample::new("b", 10.0, 0.0, 0.0) };
    close(movers::turn_between(&a, &b), 2.0, 1e-12);
    // Данный крен главнее, на земле крена нет.
    if let Some(item) = fleet.items.get_mut(&key) {
        item.sample.roll = -7.0;
    }
    assert_eq!(fleet.attitude(&key).map(|a| a.1), Some(-7.0));
    if let Some(item) = fleet.items.get_mut(&key) {
        item.sample.roll = f64::NAN;
        item.sample.alt = f64::NAN;
    }
    assert_eq!(fleet.attitude(&key), Some((0.0, 0.0)));
    assert_eq!(fleet.attitude(&"нет".to_string()), None);
}

#[test]
fn new_sample_blends() {
    let mut fleet: Movers<String, ()> = Movers::default();
    let step = |t: f64, lat: f64| Sample { speed: 10.0, course: 0.0, ..Sample::new("a".to_string(), t, lat, 25.0) };
    fleet.update(vec![step(1000.0, 60.0)], 0.0, 1000.0, false);
    let before = only(&mut fleet, 5.0, 1010.0);
    fleet.update(vec![step(1010.0, 60.002)], 5.0, 1010.0, false);
    let after = only(&mut fleet, 5.0, 1010.0);
    close(before.0, after.0, 1e-9);
}

#[test]
fn arc_and_bank() {
    assert_eq!(movers::arc(90.0, 10.0, 0.0, 5.0), (90.0, 50.0));
    let (chord, run) = movers::arc(350.0, 10.0, 6.0, 10.0);
    close(chord, 20.0, 1e-9);
    assert!(run < 100.0);
    assert_eq!(movers::bank(1000.0, 300.0), movers::ROLL_MAX);
    assert!(movers::near(60.0, 25.0, 60.0, 25.001, 100.0));
    assert!(!movers::near(60.0, 25.0, 60.1, 25.0, 100.0));
}
