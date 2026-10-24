// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Скорости плит NNR-MORVEL56: плита в точке, скорость, стрелки.
//! Перенос `tests/test_platemotion.py` модуля.

use data::platemotion::{self as pm, Motion};

/// Полюса и растр плит из файла данных модуля, если он рядом.
fn load() -> Option<Motion> {
    let Ok(text) = std::fs::read_to_string(r"C:\Dev\planetx\planetx\data\plate_motion.json") else {
        eprintln!("данных модуля нет, проверка пропущена");
        return None;
    };
    let motion = pm::read(&text);
    assert!(motion.is_ok(), "{:?}", motion.as_ref().err());
    motion.ok()
}

#[test]
fn plate_at_known_places() {
    let Some(motion) = load() else {
        return;
    };
    let cases = [
        ((0.0, -150.0), "PA"),
        ((10.0, 170.0), "PA"),
        ((-85.0, 0.0), "AN"),
        ((55.75, 37.62), "EU"),
        ((0.0, 20.0), "AF"),
        ((40.0, -100.0), "NA"),
        ((20.0, 78.0), "IN"),
        ((-25.0, 135.0), "AU"),
        ((-15.0, -60.0), "SA"),
        // Исландию режет рифт: Рейкьявик - Северная Америка,
        // восток острова - Евразия.
        ((64.1, -21.9), "NA"),
        ((65.0, -14.5), "EU"),
    ];
    for ((lat, lon), code) in cases {
        assert_eq!(motion.plate_at(lat, lon), Some(code), "{lat} {lon}");
    }
}

#[test]
fn pacific_moves_northwest_fast() {
    // Гавайи: Тихоокеанская плита идёт на северо-запад около 70
    // мм/год в NNR (DeMets et al. 2010, Argus et al. 2011).
    let Some(motion) = load() else {
        return;
    };
    let found = motion.velocity(19.7, -155.5);
    assert!(found.is_some());
    let Some((code, east, north)) = found else {
        return;
    };
    let (speed, azimuth) = pm::speed_azimuth(east, north);
    assert_eq!(code, "PA");
    assert!(60.0 < speed && speed < 80.0, "{speed}");
    assert!(280.0 < azimuth && azimuth < 310.0, "{azimuth}");
}

#[test]
fn rotation_is_rigid() {
    // Скорость у полюса вращения - ноль, на 90° от него - ω·R.
    let pole = (30.0, 40.0, 1.0);
    let (east, north) = pm::velocity(pole, 30.0, 40.0);
    assert!(east.hypot(north) < 1e-6);
    let (east, north) = pm::velocity(pole, -60.0, 40.0);
    let expected = 1.0f64.to_radians() / 1e6 * pm::RADIUS;
    assert!((east.hypot(north) - expected).abs() <= 1e-6);
}

#[test]
fn arrows_cover_view() {
    let Some(motion) = load() else {
        return;
    };
    let found = pm::arrows(&motion, 40.0, -100.0, 4.0e6);
    // Около ARROWS_ACROSS поперёк вида: десятки, не сотни.
    assert!(20 < found.len() && found.len() < 200, "{}", found.len());
    for a in &found {
        assert_eq!(a.lines.len(), 3);
        assert!(a.lat.abs() <= pm::MAX_LAT);
    }
    let whole = pm::arrows(&motion, 0.0, 0.0, 4.0e7);
    // Узлы не повторяются, -180° и 180° - одна долгота.
    let mut nodes: Vec<(i64, i64)> = whole.iter().map(|a| ((a.lat * 1e3).round() as i64, ((a.lon + 180.0).rem_euclid(360.0) * 1e3).round() as i64 % 360_000)).collect();
    let count = nodes.len();
    nodes.sort_unstable();
    nodes.dedup();
    assert_eq!(nodes.len(), count);
    assert!(30 < whole.len() && whole.len() < 400, "{}", whole.len());
}

#[test]
fn runs_round_trip() {
    let grid = [0u8, 0, 3, 3, 3, 1, 0, 0];
    let series = pm::runs(&grid);
    assert_eq!(series, [0, 2, 3, 3, 1, 1, 0, 2]);
    assert_eq!(pm::grid_of(&series, 2, 4), Ok(grid.to_vec()));
    assert!(pm::grid_of(&series, 3, 4).is_err());
    assert!(pm::grid_of(&[1, 2, 3], 1, 2).is_err());
    assert!(pm::grid_of(&[1, -2, 1, 4], 1, 2).is_err());
    assert!(pm::runs(&[]).is_empty());
}

#[test]
fn read_small_file() {
    // Шаг 90° - растр 2 × 4: север - плита A, юг - B без кода.
    let text = r#"{"step": 90, "plates": [["A", "Alpha", 90.0, 0.0, 1.0]], "runs": [1, 4, 0, 4]}"#;
    let motion = pm::read(text);
    assert!(motion.is_ok(), "{:?}", motion.as_ref().err());
    let Ok(motion) = motion else {
        return;
    };
    assert_eq!((motion.rows, motion.cols), (2, 4));
    assert_eq!(motion.plate_at(45.0, 10.0), Some("A"));
    assert_eq!(motion.plate_at(-45.0, 10.0), None);
    // Вращение вокруг северного полюса - на восток.
    let v = motion.velocity(0.5, 0.0);
    assert!(v.is_some_and(|(_, east, north)| east > 0.0 && north.abs() < 1e-9), "{v:?}");
    assert!(pm::read(r#"{"step": 0, "plates": [], "runs": []}"#).is_err());
    assert!(pm::read(r#"{"step": 1e-9, "plates": [], "runs": []}"#).is_err());
    assert_eq!(pm::step_for(4.0e7), 30.0);
    assert_eq!(pm::step_for(1.0), 0.25);
}
