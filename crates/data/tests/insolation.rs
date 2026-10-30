// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Инсоляция по рельефу. Перенос `tests/test_insolation.py` модуля.

use data::insolation::{self as ins, Insolation, Step};
use data::viewshed::distance_bearing;

/// Пермь.
const LAT: f64 = 58.0;
const LON: f64 = 56.25;
const R: f64 = 6378137.0;

/// Секунды UTC начала суток по григорианскому календарю.
fn timegm(year: i64, month: i64, day: i64) -> f64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    ((era * 146097 + doe - 719468) * 86400) as f64
}

fn summer() -> f64 {
    timegm(2026, 6, 21)
}

fn winter() -> f64 {
    timegm(2026, 12, 21)
}

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

fn flat(points: &[(f64, f64)]) -> Vec<f64> {
    vec![0.0; points.len()]
}

/// Длина дня без рефракции по склонению солнца в полдень, часы.
fn day_length(lat: f64, unix_time: f64) -> f64 {
    let (_, dec) = ins::sun_equatorial(unix_time);
    let cos_h = -lat.to_radians().tan() * dec.tan();
    2.0 * cos_h.clamp(-1.0, 1.0).acos().to_degrees() / 15.0
}

/// Стена 200 м шириной 300 м в 400-700 м к югу от точки.
fn wall_south(points: &[(f64, f64)]) -> Vec<f64> {
    points
        .iter()
        .map(|&(lat, lon)| {
            let (dist, az) = distance_bearing(LAT, LON, lat, lon, R);
            if az > 135.0 && az < 225.0 && dist > 400.0 && dist < 700.0 { 200.0 } else { 0.0 }
        })
        .collect()
}

/// Склон вниз к северу, тангенс уклона 1.
fn north_slope(points: &[(f64, f64)]) -> Vec<f64> {
    points.iter().map(|(lat, _)| (lat - LAT) * -111000.0).collect()
}

/// Склон 63.4° вниз к северу.
fn steep_north(points: &[(f64, f64)]) -> Vec<f64> {
    points.iter().map(|(lat, _)| (lat - LAT) * -111000.0 * 2.0).collect()
}

fn compute(radius_m: f64, heights: data::viewshed::Heights, start: f64, end: f64) -> Option<Insolation> {
    ins::compute(LAT, LON, radius_m, heights, start, end, R)
}

fn median(values: &[f64]) -> f64 {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n == 0 {
        return f64::NAN;
    }
    if n % 2 == 1 { v[n / 2] } else { 0.5 * (v[n / 2 - 1] + v[n / 2]) }
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

#[test]
fn flat_is_day_length() {
    // На ровной Земле часы прямого света - длина дня. Шаг 10 мин, центр
    // суток UTC сдвинут от местного полдня: допуск 15 мин.
    for day in [summer(), winter()] {
        let result = compute(1000.0, &flat, day, day);
        assert!(result.is_some());
        let Some(result) = result else { return };
        let n = result.n;
        let centre = result.hours[(n / 2) * n + n / 2];
        close(centre, day_length(LAT, day + 43200.0), 0.25);
        let (low, high) = result.hours.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(*v), b.max(*v)));
        assert!(high - low < 0.2);
    }
}

#[test]
fn period_is_daily_mean() {
    let one = compute(1000.0, &flat, summer(), summer());
    let many = compute(1000.0, &flat, summer() - 3.0 * 86400.0, summer() + 3.0 * 86400.0);
    assert!(one.is_some() && many.is_some());
    let (Some(one), Some(many)) = (one, many) else { return };
    assert_eq!(many.days, 7);
    close(mean(&many.hours), mean(&one.hours), 0.1);
}

#[test]
fn wall_shades_north_side_in_winter() {
    // Зимой солнце в Перми не выше 9°: за стеной 200 м в 400 м к югу точка
    // в тени весь день, к северу от точки на 2 км тени нет.
    let result = compute(3000.0, &wall_south, winter(), winter());
    assert!(result.is_some());
    let Some(result) = result else { return };
    let out = result.at(&[(LAT, LON), (LAT + 0.018, LON)], R);
    assert!(out.iter().all(|(_, inside)| *inside));
    assert!(out[0].0 < 1.0, "{}", out[0].0);
    close(out[1].0, day_length(LAT, winter() + 43200.0), 0.3);
}

#[test]
fn north_slope_dark_in_winter() {
    // Склон 45° на север зимой не получает прямого света. Летом солнце
    // в полдень выше 55°, он освещён почти весь день. Склон 63.4° круче
    // полуденного солнца, около полудня он в тени, утром и вечером солнце
    // на севере светит на него.
    let winter = compute(500.0, &north_slope, winter(), winter());
    let summer_slope = compute(500.0, &north_slope, summer(), summer());
    let steep = compute(500.0, &steep_north, summer(), summer());
    assert!(winter.is_some() && summer_slope.is_some() && steep.is_some());
    let (Some(winter), Some(summer_slope), Some(steep)) = (winter, summer_slope, steep) else { return };
    assert!(median(&winter.hours) < 0.01);
    let length = day_length(LAT, summer() + 43200.0);
    close(median(&summer_slope.hours), length, 0.3);
    let steep = median(&steep.hours);
    assert!(steep > 6.0 && steep < length - 4.0, "{steep} {length}");
}

#[test]
fn colors_and_outside() {
    let result = compute(1000.0, &flat, summer(), summer());
    assert!(result.is_some());
    let Some(result) = result else { return };
    let rgba = ins::tile_rgba(&result, &[(LAT, LON), (LAT + 1.0, LON)], R);
    assert!(rgba[0][3] > 0);
    assert_eq!(rgba[1][3], 0);
    let high = result.hours.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert_eq!(result.top(), high.ceil());
}

#[test]
fn month_10_km_is_fast() {
    // Время проверяется только в оптимизированной сборке.
    let started = std::time::Instant::now();
    assert!(compute(10000.0, &flat, summer(), summer() + 30.0 * 86400.0).is_some());
    if !cfg!(debug_assertions) {
        assert!(started.elapsed().as_secs_f64() < 20.0);
    }
}

#[test]
fn work_in_parts_reports_progress() {
    let heights = vec![0.0; ins::GRID * ins::GRID];
    let work = ins::Work::new(LAT, LON, 1000.0, &heights, summer(), summer(), R);
    assert!(work.is_some());
    let Some(mut work) = work else { return };
    let mut last = 0.0;
    let mut steps = 0;
    while let Step::Going(share) = work.step() {
        assert!(share >= last && share <= 1.0);
        last = share;
        steps += 1;
    }
    assert!(steps > ins::AZIMUTHS);
    assert_eq!(work.step(), Step::Done);
    assert!(work.result().is_some_and(|r| r.days == 1));
}

#[test]
fn sample_times_pick_days_evenly() {
    let (times, weight) = ins::sample_times(0.0, 0.0);
    assert_eq!(times.len(), 144);
    close(weight, 600.0 / 3600.0, 1e-12);
    close(times[0], 300.0, 1e-9);
    let (times, weight) = ins::sample_times(0.0, 99.0 * 86400.0);
    assert_eq!(times.len(), 15 * 144);
    close(weight, 600.0 / 3600.0 / 15.0, 1e-12);
    close(times[times.len() - 1], 99.0 * 86400.0 + 86400.0 - 300.0, 1e-6);
    // Конец раньше начала - одни сутки.
    assert_eq!(ins::sample_times(86400.0, 0.0).0.len(), 144);
}

#[test]
fn hostile_input_gives_nothing() {
    assert!(compute(0.0, &flat, summer(), summer()).is_none());
    assert!(compute(f64::NAN, &flat, summer(), summer()).is_none());
    assert!(compute(1000.0, &flat, f64::NAN, summer()).is_none());
    let short = |points: &[(f64, f64)]| vec![0.0; points.len() - 1];
    assert!(compute(1000.0, &short, summer(), summer()).is_none());
    assert!(ins::ray_steps(0.0, 100.0).is_empty());
    assert!(ins::ray_steps(f64::NAN, 100.0).is_empty());
}

#[test]
fn ring_limits_hours_and_raster() {
    // Участок многоугольником: вне него часов нет, растр с севера.
    let result = compute(1000.0, &flat, summer(), summer());
    assert!(result.is_some());
    let Some(mut result) = result else { return };
    result.ring = Some(data::site::square_ring(LAT, LON, 600.0));
    let out = result.at(&[(LAT, LON), (LAT + 0.005, LON)], R);
    assert!(out[0].1 && out[0].0 > 10.0);
    assert!(!out[1].1 && out[1].0.is_nan());
    let (grid, hours) = result.raster(R);
    assert_eq!((grid.cols, grid.rows), (result.n, result.n));
    assert!(grid.north > LAT && grid.west < LON);
    assert!(hours[0].is_nan());
    let middle = hours[(result.n / 2) * result.n + result.n / 2];
    assert!(middle > 10.0, "{middle}");
}
