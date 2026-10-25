// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Струйки ветра и течений: поле, частицы, следы, заливка. Перенос
//! `tests/test_flow.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use data::flow::{self, Particles, Rng, TRAIL, VectorGrid};

/// Поле 1° на весь шар, u и v постоянны, land - строки без данных.
fn uniform_field(u: f32, v: f32, land: Option<std::ops::Range<usize>>) -> VectorGrid {
    let (rows, cols) = (181, 360);
    let mut uu = vec![u; rows * cols];
    let mut vv = vec![v; rows * cols];
    if let Some(rows) = land {
        for r in rows {
            uu[r * cols..(r + 1) * cols].fill(f32::NAN);
            vv[r * cols..(r + 1) * cols].fill(f32::NAN);
        }
    }
    VectorGrid::new(uu, vv, rows, cols, -180.0, 90.0, 1.0).unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-8 + 1e-5 * b.abs()
}

fn head_lons(p: &Particles) -> Vec<f64> {
    (0..p.count).map(|i| p.point(i, 0).unwrap().1).collect()
}

#[test]
fn sample_wraps_longitude() {
    let field = uniform_field(3.0, -2.0, None);
    for (lat, lon) in [(10.0, 179.7), (-45.5, -179.9)] {
        let (u, v) = field.sample(lat, lon);
        assert!(close(u, 3.0) && close(v, -2.0), "{u} {v}");
    }
    // За крайними строками данных нет.
    assert!(field.sample(90.5, 0.0).0.is_nan());
}

#[test]
fn particles_move_east_with_westerly() {
    let field = uniform_field(10.0, 0.0, None);
    let mut p = Particles::new(200, 1);
    p.reset(flow::rect_box(0.0, -10.0, 20.0, 10.0));
    let start = head_lons(&p);
    p.age.fill(0);
    p.life.fill(1000);
    let scale = flow::metres_per_mps(2.0e6, 10.0);
    p.step(&field, 1.0, scale);
    let mut moved: Vec<(usize, f64)> =
        head_lons(&p).iter().zip(&start).map(|(a, b)| a - b).enumerate().filter(|(_, m)| *m > 0.0).collect();
    // 12 % от 2000 км - 240 км, у экватора около 2.2°.
    assert!(moved.iter().all(|(_, m)| *m < 2.3));
    moved.sort_by(|a, b| a.1.total_cmp(&b.1));
    assert!(moved[moved.len() / 2].1 > 2.0);
    for (i, _) in moved {
        assert!(close(p.point(i, 0).unwrap().0, p.point(i, 1).unwrap().0));
    }
}

#[test]
fn particles_on_land_respawn() {
    let field = uniform_field(1.0, 1.0, Some(0..181));
    let mut p = Particles::new(50, 2);
    p.reset(flow::rect_box(0.0, -10.0, 20.0, 10.0));
    p.step(&field, 1.0, 1000.0);
    assert!(p.age.iter().all(|&a| a == 0));
}

#[test]
fn spawn_inside_box_across_date_line() {
    let mut rng = Rng::new(3);
    let frame = flow::rect_box(170.0, -5.0, -170.0, 5.0);
    let (lats, lons) = flow::spawn(1000, &frame, &mut rng);
    for (lat, lon) in lats.iter().zip(&lons) {
        assert!(flow::inside(*lat, *lon, &frame));
        assert!(*lon >= 170.0 || *lon <= -170.0);
    }
}

#[test]
fn segments_follow_trail() {
    let field = uniform_field(0.0, 5.0, None);
    let mut p = Particles::new(10, 4);
    // Рамка с запасом: за 10 шагов частица уходит на 4.5° к северу.
    p.reset(flow::rect_box(0.0, -60.0, 20.0, 30.0));
    p.age.fill(0);
    p.life.fill(1000);
    for _ in 0..TRAIL + 2 {
        p.step(&field, 1.0, 10000.0);
        p.frame = Some(flow::rect_box(0.0, -60.0, 20.0, 60.0));
    }
    let s = flow::segments(&p);
    assert_eq!(s.pairs.len(), 10 * (TRAIL - 1));
    assert!(s.pairs.iter().all(|pair| pair[0][0] > pair[1][0]));
    let top = s.share.iter().copied().fold(f64::MIN, f64::max);
    assert!(close(top, (TRAIL as f64 - 2.0) / (TRAIL as f64 - 1.0)));
    assert!(s.speed.iter().all(|&v| close(v, 5.0)));
}

#[test]
fn current_day_clamps_to_series() {
    let last = "2026-09-28";
    let at = |day: &str| flow::day_seconds(day).unwrap();
    assert_eq!(flow::current_day(None, last), last);
    assert_eq!(flow::current_day(Some(at("2020-07-15")), last), "2020-07-15");
    assert_eq!(flow::current_day(Some(at("2026-10-11")), last), last);
    assert_eq!(flow::current_day(Some(at("2001-01-01")), last), flow::CURRENTS_FIRST);
    assert_eq!(flow::current_day(Some(0.0), ""), "");
    // Полдень 1 января 1970 года и неверные даты.
    assert_eq!(flow::day_seconds("1970-01-01"), Some(43_200.0));
    assert_eq!(flow::day_seconds("2026-02-29"), None);
    assert_eq!(flow::day_seconds("2026-13-01"), None);
    assert_eq!(flow::day_seconds("2026-1-011"), None);
}

#[test]
fn current_colors() {
    let colors = flow::current_colors(&[0.0, 0.4, 10.0]);
    assert_eq!(colors[0], [20.0, 40.0, 110.0]);
    assert_eq!(colors[1], [40.0, 200.0, 170.0]);
    assert_eq!(colors[2], [230.0, 40.0, 70.0]);
}

#[test]
fn speed_fill_tile() {
    // Заливка скорости под струйками течений: суша прозрачна, быстрая
    // вода непрозрачнее медленной, цвет с премноженной альфой.
    let row = [0.05f32, 0.05, 1.0, 1.0, f32::NAN, f32::NAN];
    let u: Vec<f32> = row.iter().chain(row.iter()).copied().collect();
    let grid = VectorGrid::new(u, vec![0.0; 12], 2, 6, 0.0, 1.0, 1.0).unwrap();
    let rgba = flow::speed_rgba(&flow::fill_job(Arc::new(grid)), &[0.5, 0.5, 0.5], &[0.5, 2.5, 4.5]);
    assert_eq!(rgba.len(), 3);
    let (slow, fast, land) = (rgba[0], rgba[1], rgba[2]);
    assert_eq!(land[3], 0);
    assert!(slow[3] < fast[3]);
    assert!(fast[..3].iter().all(|&c| c <= fast[3]));
}

#[test]
fn cap_spawn_stays_in_visible_circle() {
    // Издалека частицы рождаются в видимом круге Земли. Прежде
    // прямоугольник широт и долгот с 12 800 км захватывал обратную
    // сторону шара, видна была треть частиц, 12 октября 2026 года.
    let mut rng = Rng::new(3);
    let frame = flow::cap_box(35.0, -40.0, 60.0);
    let (lats, lons) = flow::spawn(20000, &frame, &mut rng);
    assert!(lats.iter().zip(&lons).all(|(a, b)| flow::inside(*a, *b, &frame)));
    // Равномерно по площади: в круге половинной площади - половина.
    let half = (0.5 * (1.0 + 60f64.to_radians().cos())).acos().to_degrees();
    let small = flow::cap_box(35.0, -40.0, half);
    let share = lats.iter().zip(&lons).filter(|(a, b)| flow::inside(**a, **b, &small)).count() as f64 / 20000.0;
    assert!((share - 0.5).abs() <= 0.02, "{share}");
    // Точка за кругом снаружи, круг через линию перемены дат.
    assert!(!flow::inside(35.0, 140.0, &frame));
    assert!(flow::inside(0.0, -175.0, &flow::cap_box(0.0, 179.0, 10.0)));
}

#[test]
fn far_view_step_stays_inside_cell_share() {
    // Издалека типичная скорость за шаг проходит не больше CELL_SHARE
    // клетки поля, иначе след петлял через вихри, снимок автора
    // от 12 октября 2026 года.
    let (dt, typical, step) = (0.04, 0.3, 0.5f64);
    let cell = step.to_radians() * flow::RADIUS;
    let far = flow::display_metres(1.3e7, typical, step, dt, Some(flow::CELL_SHARE));
    assert!(far * typical * dt / cell <= flow::CELL_SHARE + 1e-9);
    // Вблизи предел не действует: доля ширины вида прежняя.
    let near = flow::display_metres(5.0e5, typical, step, dt, Some(flow::CELL_SHARE));
    assert!(close(near, flow::metres_per_mps(5.0e5, typical)));
    // Без доли - прежняя скорость.
    assert!(close(flow::display_metres(1.3e7, typical, step, dt, None), flow::metres_per_mps(1.3e7, typical)));
}

#[test]
fn particles_repeat_with_same_seed() {
    let field = uniform_field(0.3, 0.1, None);
    let run = |seed| {
        let mut p = Particles::new(100, seed);
        p.reset(flow::cap_box(10.0, 20.0, 30.0));
        for _ in 0..50 {
            p.step(&field, 0.04, 1.0e6);
        }
        p.lats
    };
    assert_eq!(run(7), run(7));
    assert_ne!(run(7), run(8));
}

#[test]
fn currents_url_by_day() {
    let last = flow::currents_url(None);
    assert!(last.starts_with("https://coastwatch.pfeg.noaa.gov/erddap/griddap/nesdisSSH1day.nc?ugos%5B(last)%5D"));
    assert!(last.contains(",vgos%5B(last)%5D%5B(-89.875):2:(89.875)%5D%5B(-179.875):2:(179.875)%5D"));
    assert_eq!(flow::currents_url(Some("")), last);
    let day = flow::currents_url(Some("2024-05-01"));
    assert_eq!(day.matches("%5B(2024-05-01T00:00:00Z)%5D").count(), 2);
}
