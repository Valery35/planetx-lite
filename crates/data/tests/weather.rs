// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Часы и выпуски GFS, индекс, поле, сияние OVATION. Перенос
//! `tests/test_weather.py` и `tests/test_aurora.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use data::gibs::Day;
use data::weather as w;

fn utc(year: i32, month: u32, day: u32, hour: u32, min: u32) -> f64 {
    Day { year, month, day }.seconds() + f64::from(hour * 3600 + min * 60)
}

// Строки индекса выпуска 12 UTC 6 октября 2026 года, час 24.
const INDEX: &str = "1:0:d=2026100612:PRMSL:mean sea level:24 hour fcst:
581:417551237:d=2026100612:TMP:2 m above ground:24 hour fcst:
582:418063107:d=2026100612:TMP:2 m above ground:18-24 hour max fcst:
588:422157490:d=2026100612:UGRD:10 m above ground:24 hour fcst:
589:423127024:d=2026100612:VGRD:10 m above ground:24 hour fcst:
590:424101000:d=2026100612:SPFH:2 m above ground:24 hour fcst:
593:425278190:d=2026100612:PRATE:surface:24 hour fcst:
595:426753859:d=2026100612:PRATE:surface:18-24 hour ave fcst:
636:446287564:d=2026100612:TCDC:entire atmosphere:24 hour fcst:
637:447105255:d=2026100612:TCDC:entire atmosphere:18-24 hour ave fcst:
";

#[test]
fn hours() {
    let hours = w::hours();
    assert_eq!(hours[..3], [0, 1, 2]);
    assert!(hours.contains(&120) && !hours.contains(&121) && hours.contains(&123));
    assert_eq!(*hours.last().unwrap(), 384);
}

#[test]
fn runs_and_hours() {
    let now = utc(2026, 10, 6, 19, 30);
    assert_eq!(w::run_time(now), utc(2026, 10, 6, 18, 0));
    let runs = w::candidate_runs(now);
    // Через 4 часа после выпуска 12 UTC свежий - он, 18 UTC ещё нет.
    assert_eq!(runs[0], utc(2026, 10, 6, 12, 0));
    assert_eq!(runs[1], utc(2026, 10, 6, 6, 0));
    let run = runs[0];
    assert_eq!(w::pick_hour(run, utc(2026, 10, 7, 12, 0)), 24);
    assert_eq!(w::pick_hour(run, utc(2026, 10, 7, 12, 40)), 25);
    // После 120 часа - кратные трём.
    assert_eq!(w::pick_hour(run, run + 121.4 * 3600.0), 120);
    assert_eq!(w::pick_hour(run, run + 122.0 * 3600.0), 123);
    assert_eq!(w::pick_hour(run, run - 3600.0), 0);
    assert_eq!(w::step_moment(run, run + 3600.0, 1), Some(run + 7200.0));
    assert_eq!(w::step_moment(run, run, -1), None);
    // Прошлое - из выпуска не позже момента.
    assert_eq!(w::source_run(run, utc(2026, 10, 1, 7, 0)), utc(2026, 10, 1, 6, 0));
    assert_eq!(w::source_run(run, utc(2026, 10, 9, 0, 0)), run);
}

#[test]
fn file_url() {
    assert_eq!(w::file_url(utc(2026, 10, 6, 12, 0), 24), format!("{}/gfs.20261006/12/atmos/gfs.t12z.pgrb2.0p25.f024", w::BUCKET));
}

#[test]
fn ranges() {
    assert_eq!(w::byte_range(INDEX, &w::TEMPERATURE, 24), Some((417551237, Some(418063106))));
    // Ветер - U и V подряд одним участком.
    assert_eq!(w::byte_range(INDEX, &w::WIND, 24), Some((422157490, Some(424100999))));
    // Осадки - в момент, не среднее за 6 часов.
    assert_eq!(w::byte_range(INDEX, &w::PRECIPITATION, 24), Some((425278190, Some(426753858))));
    assert_eq!(w::byte_range(INDEX, &w::CLOUDS, 24), Some((446287564, Some(447105254))));
    assert_eq!(w::byte_range(INDEX, &w::TEMPERATURE, 0), None);
    assert_eq!(w::byte_range("", &w::CLOUDS, 24), None);
}

fn world(value: impl Fn(usize, usize) -> f32) -> w::Grid {
    // Сетка 1°: узлы от -180 до 179 по долготе, от 90 до -90.
    let values = (0..181).flat_map(|r| (0..360).map(move |c| (r, c))).map(|(r, c)| value(r, c)).collect();
    w::Grid { values, rows: 181, cols: 360, west: -180.0, north: 90.0, step: 1.0, run: 0.0, hour: 0 }
}

#[test]
fn sample_bilinear_and_wrap() {
    let grid = world(|r, _| 90.0 - r as f32);
    assert!((w::sample(&grid, 45.5, 10.0) - 45.5).abs() < 1e-4);
    let grid = world(|_, c| c as f32);
    // Между 179° и -180°: середина значений 359 и 0.
    assert!((w::sample(&grid, 0.0, 179.5) - 179.5).abs() < 1e-3);
    assert!((w::sample(&grid, 0.0, -179.5) - 0.5).abs() < 1e-3);
}

#[test]
fn wind_speed_and_units() {
    assert_eq!(w::values_of(&w::WIND, &[vec![3.0; 4], vec![4.0; 4]]), vec![5.0; 4]);
    let rate = w::values_of(&w::PRECIPITATION, &[vec![1.0 / 3600.0; 4]]);
    assert!(rate.iter().all(|v| (v - 1.0).abs() < 1e-6));
}

#[test]
fn colors() {
    assert_eq!(w::colorize(&w::PRECIPITATION, 0.0)[3], 0);
    assert_eq!(w::colorize(&w::PRECIPITATION, 0.05)[3], 0);
    assert!(w::colorize(&w::PRECIPITATION, 1.0)[3] > 200);
    assert_eq!(w::colorize(&w::PRECIPITATION, f64::NAN)[3], 0);
}

#[test]
fn tile_of_a_world_field() {
    let grid = world(|_, _| 20.0);
    let tile = w::tile_rgba(&w::TEMPERATURE, &grid, (2, 1, 1)).expect("тайл");
    assert_eq!(tile.len(), 256 * 256 * 4);
    assert!(tile[3] > 0);
    assert!(w::tile_rgba(&w::PRECIPITATION, &world(|_, _| 0.0), (2, 1, 1)).is_none());
}

/// Файл в разметке OVATION: тройки долгота, широта, вероятность.
fn aurora_file() -> Vec<u8> {
    let mut triples = Vec::new();
    for lon in 0..360 {
        for lat in -90..=90 {
            triples.push(format!("[{lon},{lat},{}]", if lat == 67 && lon == 30 { 40 } else { 0 }));
        }
    }
    format!(r#"{{"Observation Time": "2026-10-12T03:10:00Z", "Forecast Time": "2026-10-12T03:45:00Z", "coordinates": [{}]}}"#, triples.join(",")).into_bytes()
}

#[test]
fn aurora_rows_north_to_south() {
    let grid = w::parse_aurora(&aurora_file()).expect("поле");
    assert_eq!((grid.rows, grid.cols), (181, 360));
    assert_eq!(grid.values[(90 - 67) * 360 + 30], 40.0);
    assert_eq!(grid.values.iter().sum::<f32>(), 40.0);
    assert_eq!(grid.run, utc(2026, 10, 12, 3, 45));
}

#[test]
fn aurora_sample_and_west_longitude() {
    let grid = w::parse_aurora(&aurora_file()).expect("поле");
    assert!((w::sample(&grid, 67.0, 30.0) - 40.0).abs() < 1e-6);
    // Западная долгота -330° - та же точка.
    assert!((w::sample(&grid, 67.0, -330.0) - 40.0).abs() < 1e-6);
}

#[test]
fn weak_aurora_is_clear() {
    assert_eq!(w::colorize(&w::AURORA, 1.0)[3], 0);
    assert!(w::colorize(&w::AURORA, 50.0)[3] > 200);
}

#[test]
fn bad_aurora_files_refused() {
    assert!(w::parse_aurora(b"<html>").is_none());
    assert!(w::parse_aurora(b"{}").is_none());
    assert!(w::parse_aurora(br#"{"coordinates": [[0, 0, 1]]}"#).is_none());
}

#[test]
fn forecast_time_forms() {
    assert_eq!(w::forecast_seconds("2026-10-12T03:45:00Z"), w::forecast_seconds("2026-10-12 03:45"));
    assert_eq!(w::forecast_seconds("вчера"), None);
}

/// Живой файл GRIB: путь в переменной PLANETX_GRIB, одно поле TMP.
#[test]
#[ignore = "нужен файл GRIB2"]
fn real_grib_reads() {
    let path = std::env::var("PLANETX_GRIB").expect("путь");
    let grid = w::read_grib(std::fs::read(path).expect("файл"), &w::TEMPERATURE, 0.0, 3).expect("поле");
    println!("{} × {}, запад {}, север {}, шаг {}", grid.cols, grid.rows, grid.west, grid.north, grid.step);
    println!("Пермь {}, экватор {}, Антарктида {}", w::sample(&grid, 58.0, 56.2), w::sample(&grid, 0.0, 20.0), w::sample(&grid, -80.0, 0.0));
    assert_eq!((grid.cols, grid.rows), (1440, 721));
}
