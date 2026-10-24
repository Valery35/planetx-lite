// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Поезда Digitraffic: разбор ответов, ход между положениями, рельсы,
//! модель вагона. Перенос `tests/test_trains.py` модуля.

use data::routing;
use data::trains::{self as tr, Fleet, RailPos, Rails, Sample, TrainKey};

/// Запись ответа train-locations/latest от 18 октября 2026 года.
const LATEST: &str = r#"[{"accuracy": 0, "isGpsLocation": true,
    "location": {"type": "Point", "coordinates": [30.098746, 62.214724]},
    "speed": 138, "trainNumber": 9, "departureDate": "2026-10-18",
    "timestamp": "2026-10-18T17:31:33.000Z"},
   {"location": null, "trainNumber": 10, "timestamp": "2026-10-18T17:31:33.000Z"}]"#;
const TYPES: &str = r#"{"data": {"currentlyRunningTrains": [
    {"trainNumber": 9, "departureDate": "2026-10-18", "commuterLineid": "",
     "trainType": {"name": "S", "trainCategory": {"name": "Long-distance"}},
     "operator": {"shortCode": "vr"}},
    {"trainNumber": 9123, "departureDate": "2026-10-18", "commuterLineid": "R",
     "trainType": {"name": "HL", "trainCategory": {"name": "Commuter"}},
     "operator": {"shortCode": "vr"}}]}}"#;
/// Линия рельсов в единицах сетки.
type Line = Vec<(f64, f64)>;
/// Участок у Хельсинки.
const LAT: f64 = 60.20;

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

fn key(number: i64, date: &str) -> TrainKey {
    (number, date.to_string())
}

fn grid_line(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    points
        .iter()
        .map(|&(lat, lon)| {
            let (x, y) = routing::to_grid(lat, lon);
            (x.round(), y.round())
        })
        .collect()
}

fn sample(t: f64, lat: f64, lon: f64, kmh: f64) -> Sample {
    Sample { key: key(1, "d"), t, lat, lon, speed: kmh / 3.6 }
}

/// Единственное положение на глобусе: (широта, долгота, азимут).
fn only(fleet: &mut Fleet, now: f64, clock: f64) -> (f64, f64, f64) {
    let found = fleet.positions(now, clock);
    assert_eq!(found.len(), 1);
    found.first().map_or((f64::NAN, f64::NAN, f64::NAN), |p| (p.1, p.2, p.3))
}

// Разбор.

#[test]
fn latest() {
    let found = tr::parse_latest(LATEST.as_bytes());
    assert_eq!(found.len(), 1);
    let s = &found[0];
    assert_eq!(s.key, key(9, "2026-10-18"));
    close(s.lat, 62.214724, 1e-12);
    close(s.speed, 138.0 / 3.6, 1e-12);
    assert_eq!(Some(s.t), tr::epoch("2026-10-18T17:31:33Z"));
    assert!(tr::parse_latest(b"{}").is_empty());
}

#[test]
fn epoch_fraction() {
    assert_eq!(tr::epoch("1970-01-01T00:00:01.250Z"), Some(1.25));
    assert_eq!(tr::epoch("2000-03-01T00:00:00Z"), Some(951868800.0));
    assert_eq!(tr::epoch("вчера"), None);
    assert_eq!(tr::epoch("2026-02-30T00:00:00Z"), None);
    assert_eq!(tr::epoch("2026-10-18 17:31:33"), None);
    assert_eq!(tr::epoch("2026-10-18T25:31:33"), None);
}

#[test]
fn types_and_labels() {
    let info = tr::parse_types(TYPES.as_bytes());
    let nine = info.get(&key(9, "2026-10-18"));
    assert_eq!(nine.map(|i| i.category.as_str()), Some("Long-distance"));
    assert_eq!(tr::label(&key(9, "2026-10-18"), nine), "S 9");
    assert_eq!(tr::label(&key(9123, "2026-10-18"), info.get(&key(9123, "2026-10-18"))), "R 9123");
    assert_eq!(tr::label(&key(5, ""), None), "5");
}

#[test]
fn one_train_of_arrived() {
    // Ответ trains/2026-10-18/8208 - прибывший пригородный поезд.
    let found = tr::parse_train(
        br#"[{"trainNumber": 8208, "departureDate": "2026-10-18", "trainType": "HL",
              "trainCategory": "Commuter", "commuterLineID": "A",
              "operatorShortCode": "vr", "runningCurrently": false}]"#,
    );
    let info = found.get(&key(8208, "2026-10-18"));
    assert_eq!(info.map(|i| (i.category.as_str(), i.line.as_str())), Some(("Commuter", "A")));
    assert_eq!(tr::train_url(&key(8208, "2026-10-18")), "https://rata.digitraffic.fi/api/v1/trains/2026-10-18/8208");
}

#[test]
fn graphql_body_as_module() {
    // json.dumps модуля: разделитель «: ».
    assert_eq!(
        tr::types_body(),
        "{\"query\": \"{ currentlyRunningTrains { trainNumber departureDate commuterLineid \
         trainType { name trainCategory { name } } operator { shortCode } } }\"}"
    );
    assert_eq!(tr::GRAPHQL_HEADERS, [("Digitraffic-User", "PlanetX"), ("Content-Type", "application/json")]);
}

// Ход.

#[test]
fn heading_and_dead_reckoning_without_rails() {
    let mut fleet = Fleet::default();
    fleet.update(vec![sample(1000.0, LAT, 25.0, 72.0)], 0.0, 1000.0);
    fleet.update(vec![sample(1010.0, LAT + 0.001, 25.0, 72.0)], 0.0, 1010.0);
    // Переход кончился, 10 с хода по 20 м/с на север - 200 м.
    let (lat, lon, heading) = only(&mut fleet, 10.0, 1020.0);
    close(heading, 0.0, 1e-3);
    close(tr::metres(LAT + 0.001, 25.0, lat, lon), 200.0, 0.5);
    // Ход по скорости - не дольше EXTRAPOLATE_MAX.
    let (lat, lon, _) = only(&mut fleet, 10.0, 5000.0);
    close(tr::metres(LAT + 0.001, 25.0, lat, lon), 20.0 * tr::EXTRAPOLATE_MAX, 1.0);
}

#[test]
fn new_sample_blends_without_jump() {
    let mut fleet = Fleet::default();
    fleet.update(vec![sample(1000.0, LAT, 25.0, 72.0)], 0.0, 1000.0);
    fleet.update(vec![sample(1010.0, LAT + 0.001, 25.0, 72.0)], 0.0, 1010.0);
    let before = only(&mut fleet, 5.0, 1025.0);
    fleet.update(vec![sample(1020.0, LAT + 0.002, 25.0, 72.0)], 5.0, 1025.0);
    let after = only(&mut fleet, 5.0, 1025.0);
    close(before.0, after.0, 1e-9);
    let later = only(&mut fleet, 5.0 + tr::BLEND, 1025.0);
    let target = tr::destination(LAT + 0.002, 25.0, 0.0, 20.0 * 5.0);
    close(later.0, target.0, 1e-7);
}

#[test]
fn gone_and_stale_trains_dropped() {
    let mut fleet = Fleet::default();
    fleet.update(vec![sample(1000.0, LAT, 25.0, 72.0)], 0.0, 1000.0);
    fleet.update(Vec::new(), 0.0, 1010.0);
    assert!(fleet.trains.is_empty());
    fleet.update(vec![sample(1000.0, LAT, 25.0, 72.0)], 0.0, 1000.0 + tr::STALE + 1.0);
    assert!(fleet.trains.is_empty());
}

// Рельсы.

/// Путь на восток, в конце - стрелка: прямо на восток и отворот на
/// северо-восток.
fn lines() -> (Line, Line, Line) {
    (
        grid_line(&[(LAT, 25.00), (LAT, 25.01)]),
        grid_line(&[(LAT, 25.01), (LAT, 25.02)]),
        grid_line(&[(LAT, 25.01), (LAT + 0.005, 25.02)]),
    )
}

#[test]
fn snap_and_walk_through_switch() {
    let (a, east, north) = lines();
    let rails = Rails::new(vec![a, east, north]);
    let (x, y) = routing::to_grid(LAT + 0.0002, 25.005); // 22 м от пути
    let reach = tr::SNAP_REACH / routing::unit_metres(LAT);
    let found = rails.snap(x, y, reach);
    assert!(found.is_some());
    let Some((line, s)) = found else { return };
    assert_eq!(line, 0);
    // Курс на восток: за стрелкой ход идёт прямо, не на север.
    let pos = rails.walk(RailPos { line, s, direction: 1 }, 1000.0 / routing::unit_metres(LAT));
    assert_eq!(pos.line, 1);
    let point = rails.point(pos.line, pos.s);
    assert!(point.is_some());
    let Some((xy, tangent)) = point else { return };
    let (lat, _) = routing::to_latlon(xy.0, xy.1);
    close(lat, LAT, 1e-5);
    close(tr::grid_heading(tangent), 90.0, 0.5);
    let (x, y) = routing::to_grid(LAT + 0.01, 25.005);
    assert!(rails.snap(x, y, reach).is_none());
}

#[test]
fn train_follows_rails() {
    let (a, east, north) = lines();
    let mut fleet = Fleet::default();
    fleet.set_rails(Some(Rails::new(vec![a, east, north])));
    let first = Sample { key: key(1, "d"), t: 1000.0, lat: LAT + 0.0002, lon: 25.004, speed: 20.0 };
    fleet.update(vec![first], 0.0, 1000.0);
    let moved = Sample { key: key(1, "d"), t: 1010.0, lat: LAT + 0.0002, lon: 25.0076, speed: 20.0 };
    fleet.update(vec![moved], 0.0, 1010.0);
    let (lat, lon, heading) = only(&mut fleet, 10.0, 1020.0);
    // Притянут к пути и прошёл 200 м по нему на восток.
    close(lat, LAT, 1e-5);
    let start = tr::destination(LAT, 25.0076, 90.0, 0.0);
    close(tr::metres(start.0, start.1, lat, lon), 200.0, 3.0);
    close(heading, 90.0, 0.5);
}

#[test]
fn cars_bend_along_rails() {
    // Поезд прошёл стрелку и идёт на северо-восток, хвост ещё на пути
    // с запада: вагон в 300 м позади головы смотрит на восток.
    let (a, _, north) = lines();
    let mut fleet = Fleet::default();
    fleet.set_rails(Some(Rails::new(vec![a, north])));
    let one = Sample { key: key(1, "d"), t: 1000.0, lat: LAT + 0.0005, lon: 25.011, speed: 20.0 };
    fleet.update(vec![one], 0.0, 1000.0);
    let two = Sample { key: key(1, "d"), t: 1010.0, lat: LAT + 0.00075, lon: 25.0115, speed: 20.0 };
    fleet.update(vec![two], 0.0, 1010.0);
    let head = fleet.place(&key(1, "d"), 10.0, 1010.0, 0.0);
    let tail = fleet.place(&key(1, "d"), 10.0, 1010.0, 300.0);
    assert!(head.is_some() && tail.is_some());
    let (Some(head), Some(tail)) = (head, tail) else { return };
    close(head.2, 44.9, 1.0);
    close(tail.2, 90.0, 0.5);
    close(tail.0, LAT, 1e-5);
    // По пути от хвоста до головы - 300 м: до стрелки на восток и от
    // стрелки на северо-восток.
    close(tr::metres(LAT, 25.01, tail.0, tail.1), 300.0 - tr::metres(LAT, 25.01, head.0, head.1), 3.0);
}

#[test]
fn end_of_line_stops() {
    let (a, east, north) = lines();
    let rails = Rails::new(vec![a, east, north]);
    let pos = rails.walk(RailPos { line: 1, s: 0.0, direction: 1 }, 1e9);
    assert_eq!((pos.line, Some(pos.s)), (1, rails.length(1)));
    // Чужой номер линии - место не меняется.
    let odd = RailPos { line: 9, s: 1.0, direction: 1 };
    assert_eq!(rails.walk(odd, 10.0), odd);
    assert!(rails.point(9, 0.0).is_none());
}

#[test]
fn empty_rails_are_none() {
    let mut fleet = Fleet::default();
    fleet.set_rails(Some(Rails::new(vec![vec![(1.0, 1.0)]])));
    assert!(fleet.rails.is_none());
    assert!(Rails::new(Vec::new()).snap(0.0, 0.0, 1e9).is_none());
}

// Модель.

#[test]
fn car_points_along_y() {
    let color = tr::category_color("Cargo");
    let parts = tr::train_parts(color, true);
    let all: Vec<[f64; 3]> = parts.iter().flat_map(|p| p.positions.iter().copied()).collect();
    let min = |k: usize| all.iter().map(|v| v[k]).fold(f64::MAX, f64::min);
    let max = |k: usize| all.iter().map(|v| v[k]).fold(f64::MIN, f64::max);
    close(max(1) - min(1), tr::CAR_LENGTH, 0.1);
    close(min(2), 0.0, 1e-12);
    assert!(parts.iter().any(|p| p.rgba[..3] == color));
    assert_eq!(tr::category_color("?"), tr::OTHER);
    // Вагон без головы - без лобового стекла, ровно CAR_LENGTH.
    let car = tr::train_parts(color, false);
    let ys = car.iter().flat_map(|p| p.positions.iter().map(|v| v[1]));
    let (lo, hi) = ys.fold((f64::MAX, f64::MIN), |(lo, hi), y| (lo.min(y), hi.max(y)));
    close(hi - lo, tr::CAR_LENGTH, 1e-12);
    assert_eq!(tr::cars("Cargo"), 12);
    assert_eq!(tr::cars("?"), tr::CARS_OTHER);
}
