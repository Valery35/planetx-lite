// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Тур: остановки, время, непрерывность позы, путь, записанный тур.
//! Перенос `tests/test_tour.py` модуля. Тест позы модели по gx:Track
//! (`core/track.py`) не перенесён: модуль трека ещё не перенесён.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use globe_core::navigation::Pose;
use scene::tour::{self as tr, Stop, Tour, Update};
use serde_json::{Map, Value, json};

fn stops() -> Vec<Stop> {
    vec![
        Stop::new("Пермь", 58.01, 56.23, 20000.0, 30.0, 45.0),
        Stop::new("Кунгур", 57.43, 56.94, 5000.0, 0.0, 0.0),
        Stop::new("Эльбрус", 43.35, 42.44, 30000.0, 180.0, 60.0),
    ]
}

fn same(pose: &Pose, stop: &Stop) -> bool {
    (pose.lat - stop.lat).abs() < 1e-9
        && (pose.lon - stop.lon).abs() < 1e-9
        && (pose.distance - stop.distance).abs() < 1e-6
        && ((pose.heading - stop.heading + 180.0).rem_euclid(360.0) - 180.0).abs() < 1e-9
        && (pose.tilt - stop.tilt).abs() < 1e-9
}

fn close(a: f64, b: f64, places: i32) -> bool {
    (a - b).abs() < 0.5 * 10f64.powi(-places)
}

fn start() -> Pose {
    Pose::new(55.75, 37.62, 2.0e6, 0.0, 0.0)
}

fn tour() -> Tour {
    Tour::new(&start(), stops(), 2.0)
}

#[test]
fn caption_only_on_stop() {
    // Подпись остановки видна на остановке, в перелёте её нет,
    // проявляется и гаснет за CAPTION_FADE.
    let t = tour();
    let mid = t.arrivals[1] + t.pause / 2.0;
    assert_eq!(t.caption_at(mid), Some((1, 1.0)));
    let flying = (t.starts[1] + t.glides[1]) / 2.0;
    assert_eq!(t.caption_at(flying), None);
    let just = t.caption_at(t.glides[0] + tr::CAPTION_FADE / 2.0).unwrap();
    assert_eq!(just.0, 0);
    assert!(close(just.1, 0.5, 7));
    let leaving = t.caption_at(t.arrivals[2] + t.pause - tr::CAPTION_FADE / 4.0).unwrap();
    assert!(close(leaving.1, 0.25, 7));
}

#[test]
fn own_pause_and_speed() {
    // Вторая остановка - своя пауза 7 с и перелёт вдвое быстрее, прочие -
    // пауза тура. Перелёт кончается в той же позе.
    let mut own = stops();
    own[1].pause = Some(7.0);
    own[1].speed = 2.0;
    let tour = Tour::new(&start(), own.clone(), 2.0);
    let plain = Tour::new(&start(), stops(), 2.0);
    assert_eq!(tour.pauses, [2.0, 7.0, 2.0]);
    let flight = plain.glides[1] - plain.starts[1];
    assert!(close(tour.glides[1] - tour.starts[1], flight / 2.0, 6));
    assert!(close(tour.starts[2] - tour.arrivals[1], 7.0, 6));
    let end = tour.pose_at(tour.glides[1]);
    assert!(same(&end, &own[1]));
    assert_eq!(tour.caption_at(tour.arrivals[1] + 3.5), Some((1, 1.0)));
}

#[test]
fn pause_end_only_on_stop() {
    // Камера стоит на остановке - конец её паузы, в перелёте и после
    // конца тура - None. Камеру, взятую на паузе, тур не теряет.
    let tour = Tour::new(&start(), stops(), 4.0);
    assert_eq!(tour.pause_end(tour.starts[1] + 0.01), None);
    assert!(close(tour.pause_end(tour.arrivals[1] + 1.0).unwrap(), tour.arrivals[1] + 4.0, 9));
    assert_eq!(tour.pause_end(tour.duration), None);
    assert_eq!(Tour::new(&start(), stops(), 0.0).pause_end(tour.arrivals[0]), None);
}

#[test]
fn no_caption_without_pause() {
    let tour = Tour::new(&start(), stops(), 0.0);
    assert_eq!(tour.caption_at(tour.arrivals[1]), None);
}

#[test]
fn duration_is_flights_and_pauses() {
    let t = tour();
    let flights: f64 = t.flights.iter().map(|f| f.duration).sum();
    assert!(close(t.duration, flights + 3.0 * 2.0, 7));
}

#[test]
fn camera_stands_on_each_stop_during_pause() {
    let t = tour();
    for (i, stop) in stops().iter().enumerate() {
        for dt in [0.0, 1.0, 1.99] {
            let at = t.arrivals[i] + dt;
            assert_eq!(t.index_at(at), Some(i));
            assert!(t.arrived(at));
            assert!(same(&t.pose_at(at), stop));
        }
    }
}

#[test]
fn starts_from_camera() {
    let t = tour();
    let pose = t.pose_at(0.0);
    assert!(close(pose.lat, t.start.lat, 7));
    assert!(close(pose.distance, t.start.distance, 7));
    assert!(!t.arrived(0.0));
}

#[test]
fn pose_is_continuous_at_leg_start() {
    // Перелёт к следующей остановке начинается с позы предыдущей.
    let t = tour();
    let at = t.starts[1];
    let before = t.pose_at(at - 1e-6);
    let after = t.pose_at(at + 1e-6);
    assert!(close(before.lat, after.lat, 5));
    assert!(close(before.lon, after.lon, 5));
    assert!((before.distance - after.distance).abs() / before.distance < 1e-4);
}

#[test]
fn end_and_empty() {
    let t = tour();
    let end = t.pose_at(t.duration + 5.0);
    assert!(same(&end, &stops()[2]));
    let empty = Tour::new(&start(), Vec::new(), tr::PAUSE);
    assert_eq!(empty.duration, 0.0);
    assert_eq!(empty.index_at(0.0), None);
}

#[test]
fn minutes_and_hours() {
    assert_eq!(tr::clock(0.0), "0:00");
    assert_eq!(tr::clock(65.4), "1:05");
    assert_eq!(tr::clock(3725.0), "1:02:05");
    assert_eq!(tr::clock(-3.0), "0:00");
}

// Остановка-путь: 20 км на север, потом 20 км на восток.

fn path() -> Stop {
    Stop::path("Путь", &[(58.0, 56.0), (58.18, 56.0), (58.18, 56.34)], tr::PATH_TILT)
}

#[test]
fn length_distance_and_time() {
    let p = path();
    assert!((p.length() - 40000.0).abs() <= 300.0);
    assert!(close(p.distance, p.length() / 8.0, 7));
    // Проезд пути длиннее 2.4 км - 32 с при любой длине.
    assert!(close(p.glide, 32.0, 7));
}

#[test]
fn heading_follows_path() {
    let p = path();
    let first = p.pose_at(0.2 * p.glide);
    let last = p.pose_at(0.9 * p.glide);
    assert!(first.heading.min(360.0 - first.heading) < 1.0);
    assert!((last.heading - 90.0).abs() <= 1.5);
    assert_eq!(first.tilt, tr::PATH_TILT);
}

#[test]
fn ends_at_last_point() {
    let end = path().end_pose();
    assert!(close(end.lat, 58.18, 6));
    assert!(close(end.lon, 56.34, 6));
}

#[test]
fn tour_glides_after_flight_and_continues() {
    let tour = Tour::new(&start(), vec![path(), stops().remove(0)], 1.0);
    let g = tour.glides[0];
    let at_arrival = tour.pose_at(g - 1e-6);
    let gliding = tour.pose_at(g + 1e-6);
    assert!(close(at_arrival.lat, gliding.lat, 6));
    assert!(close(at_arrival.heading, gliding.heading, 3));
    assert!(!tour.arrived(g + 1.0)); // едет, не стоит
    assert!(tour.arrived(tour.arrivals[0] + 0.5));
    // Следующий перелёт начинается с конца пути.
    let leg = tour.pose_at(tour.starts[1] + 1e-6);
    assert!(close(leg.lat, 58.18, 4));
    assert!(close(leg.lon, 56.34, 4));
}

// Изменения меток записанного тура и перелёт с подъёмом.

#[test]
fn bounce_rises_between_poses() {
    let samples = [[0.0, 58.0, 56.0, 2000.0, 0.0, 0.0], [10.0, 58.0, 57.0, 2000.0, 0.0, 0.0]];
    let flat = Stop::recorded("ровно", &samples, &[], Vec::new());
    let high = Stop::recorded("подъём", &samples, &[1], Vec::new());
    assert!(close(flat.pose_at(5.0).distance, 2000.0, 3));
    assert!(high.pose_at(5.0).distance > 4000.0);
    assert!(close(high.pose_at(10.0).lon, 57.0, 6));
}

fn object(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

#[test]
fn animated_changes() {
    let base: tr::State =
        [("a".to_string(), object(json!({"visible": false, "color": [255, 0, 0, 255], "icon_scale": 1.0, "points": [[58.0, 56.0]]})))]
            .into();
    let updates = vec![
        Update {
            t: 2.0,
            d: 4.0,
            ids: vec!["a".into()],
            set: object(json!({"visible": true, "icon_scale": 3.0, "color": [0, 0, 255, 255], "points": [[58.0, 57.0]]})),
        },
        Update { t: 9.0, d: 0.0, ids: vec!["zz".into()], set: object(json!({"visible": true})) },
    ];
    assert_eq!(tr::animated(&updates, 1.0, &base), base);
    let half = &tr::animated(&updates, 4.0, &base)["a"];
    assert_eq!(half["visible"], json!(true));
    assert!(close(half["icon_scale"].as_f64().unwrap(), 2.0, 7));
    assert_eq!(half["color"], json!([128, 0, 128, 255]));
    assert!(close(half["points"][0][1].as_f64().unwrap(), 56.5, 7));
    let end = &tr::animated(&updates, 10.0, &base)["a"];
    assert_eq!(end["color"], json!([0, 0, 255, 255]));
    assert_eq!(base["a"]["icon_scale"], json!(1.0)); // base не тронут
}

// Записанный тур: позы между моментами, переходы через 180°,
// прореживание, место в туре.

const SAMPLES: [tr::Sample; 3] = [
    [0.0, 58.0, 179.0, 1000.0, 350.0, 30.0],
    [1.0, 58.0, -179.0, 4000.0, 10.0, 50.0],
    [3.0, 58.2, -179.0, 4000.0, 10.0, 50.0],
];

#[test]
fn interpolation() {
    let stop = Stop::recorded("Запись", &SAMPLES, &[], Vec::new());
    assert_eq!(stop.glide, 3.0);
    let mid = stop.pose_at(0.5);
    // Через линию перемены дат, а не назад через весь шар.
    assert!(close(mid.lon.abs(), 180.0, 6));
    // Через север, а не через юг.
    let h = mid.heading.rem_euclid(360.0);
    assert!(close(h.min(360.0 - h), 0.0, 6));
    assert!(close(mid.distance, 2000.0, 6));
    assert!(close(mid.tilt, 40.0, 6));
    assert!(close(stop.end_pose().lat, 58.2, 9));
}

#[test]
fn key_points_spread_along_path() {
    let samples: Vec<tr::Sample> = (0..101).map(|t| [f64::from(t), 58.0 + 0.001 * f64::from(t), 56.0, 3000.0, 0.0, 45.0]).collect();
    assert_eq!(tr::key_indices(&samples, 11), (0..=100).step_by(10).collect::<Vec<_>>());
    // Камера вращалась на месте - опорные по времени.
    let still: Vec<tr::Sample> = (0..50).map(|t| [f64::from(t), 58.0, 56.0, 3000.0, 3.6 * f64::from(t), 45.0]).collect();
    assert_eq!(tr::key_indices(&still, 5).len(), 5);
}

#[test]
fn edit_moves_neighbours_smoothly() {
    let samples: Vec<tr::Sample> = (0..21).map(|t| [f64::from(t), 58.0 + 0.001 * f64::from(t), 56.0, 3000.0, 10.0, 45.0]).collect();
    let keys = [0, 10, 20];
    let mut points: Vec<(f64, f64)> = keys.iter().map(|k| (samples[*k][1], samples[*k][2])).collect();
    points[1].1 = 56.01; // середина на восток
    let out = tr::edit_samples(&samples, &keys, &points, 2.0);
    assert!(close(out[10][2], 56.01, 7));
    assert!(close(out[5][2], 56.005, 7));
    assert!(close(out[0][2], 56.0, 7));
    assert!(close(out[20][2], 56.0, 7));
    assert!(out.iter().all(|o| o[3] == 6000.0));
    assert!(out.iter().zip(&samples).all(|(o, s)| o[0] == s[0]));
    assert!(out.iter().all(|o| o[4] == 10.0));
}

#[test]
fn in_tour() {
    let stop = Stop::recorded("Запись", &SAMPLES, &[], Vec::new());
    let tour = Tour::new(&Pose::new(55.0, 37.0, 2.0e6, 0.0, 0.0), vec![stop], 1.0);
    let arrive = tour.glides[0];
    assert!(close(tour.duration, arrive + 3.0 + 1.0, 7));
    assert!(close(tour.pose_at(arrive + 3.0).lat, 58.2, 7));
}

#[test]
fn thin() {
    let mut samples: Vec<tr::Sample> = (0..5).map(|t| [f64::from(t), 58.0, 56.0, 1000.0, 0.0, 0.0]).collect();
    samples.push([5.0, 58.1, 56.0, 1000.0, 0.0, 0.0]);
    let out = tr::thin(&samples);
    assert_eq!(out.iter().map(|s| s[0]).collect::<Vec<_>>(), [0.0, 4.0, 5.0]);
}
