// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Карта тренда: наклон Тейла-Сена и тест Манна-Кендалла по пикселям.
//! Перенос `tests/test_trend.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use data::trend::{self, Job, MIN_SCENES, SHARE, SIGNIFICANT, Trend, YEAR};

/// Свой генератор случайных чисел: ряд повторяется от запуска к запуску.
struct Rng(u64);

impl Rng {
    fn uniform(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }

    fn normal(&mut self, mean: f64, sd: f64) -> f64 {
        let (a, b) = (self.uniform(), self.uniform());
        mean + sd * (-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * b).cos()
    }
}

const SCENES: usize = 30;
const COLS: usize = 3;

/// Ряд 30 сцен за 6 лет на сетке 2 × 3, сутки сцен.
fn setup() -> (Vec<Vec<f64>>, Vec<f64>) {
    let mut rng = Rng(7);
    let mut days: Vec<f64> = (0..SCENES).map(|_| rng.uniform() * 6.0 * YEAR).collect();
    days.sort_by(f64::total_cmp);
    let mut stack = vec![vec![f64::NAN; 2 * COLS]; SCENES];
    for (k, scene) in stack.iter_mut().enumerate() {
        let years = days[k] / YEAR;
        scene[0] = 0.3 + 0.05 * years + rng.normal(0.0, 0.01);
        scene[1] = 0.5 - 0.02 * years + rng.normal(0.0, 0.01);
        scene[2] = rng.normal(0.4, 0.05);
        if k < 4 {
            scene[COLS] = 0.1 * (k + 1) as f64;
        }
        // Каждая третья сцена - выброс, облако.
        scene[COLS + 1] = if k % 3 == 0 { 5.0 } else { 0.3 + 0.05 * years };
        if k % 2 == 0 {
            scene[COLS + 2] = 0.2 + 0.1 * years;
        }
    }
    (stack, days)
}

fn run() -> (Trend, Vec<Vec<f64>>, Vec<f64>) {
    let (stack, days) = setup();
    let out = trend::trend(&stack, &days, MIN_SCENES).unwrap();
    (out, stack, days)
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    let m = v.len() / 2;
    if v.len() % 2 == 1 { v[m] } else { (v[m - 1] + v[m]) / 2.0 }
}

#[test]
fn slope_and_significance() {
    let (t, _, _) = run();
    assert!((t.slope[0] - 0.05).abs() <= 0.005, "{}", t.slope[0]);
    assert!((t.slope[1] + 0.02).abs() <= 0.005, "{}", t.slope[1]);
    assert!(t.p[0] < 0.001);
    assert!(t.p[1] < 0.001);
    assert!(t.p[2] > SIGNIFICANT, "{}", t.p[2]);
    assert_eq!(t.n[COLS], 4);
    assert!(t.slope[COLS].is_nan() && t.p[COLS].is_nan());
    assert_eq!(t.n[COLS + 2], 15);
    assert!((t.slope[COLS + 2] - 0.1).abs() <= 1e-9);
}

#[test]
fn outliers_do_not_move_sen_slope() {
    let (t, _, _) = run();
    assert!((t.slope[COLS + 1] - 0.05).abs() <= 0.02, "{}", t.slope[COLS + 1]);
}

#[test]
fn matches_direct_pairs() {
    let (t, stack, days) = run();
    let values: Vec<f64> = stack.iter().map(|s| s[2]).collect();
    let mut pairs = Vec::new();
    for i in 0..SCENES {
        for j in i + 1..SCENES {
            pairs.push((values[j] - values[i]) / ((days[j] - days[i]) / YEAR));
        }
    }
    assert!((t.slope[2] - median(pairs)).abs() <= 1e-12);
}

#[test]
fn chunks_give_same_result() {
    let (stack, days) = setup();
    let big: Vec<Vec<f64>> = stack.iter().map(|s| s.iter().flat_map(|&v| [v; 50]).collect()).collect();
    let full = trend::trend(&big, &days, MIN_SCENES).unwrap();
    let mut job = Job::new(big, &days, MIN_SCENES).unwrap();
    let mut steps = 0;
    while !job.step(7) {
        steps += 1;
        assert!(job.progress() < 1.0);
    }
    assert_eq!(steps, job.pixels().div_ceil(7) - 1);
    let parts = job.finish();
    assert_eq!(full.n, parts.n);
    for (a, b) in full.slope.iter().zip(&parts.slope) {
        assert!(a == b || (a.is_nan() && b.is_nan()));
    }
    for (a, b) in full.p.iter().zip(&parts.p) {
        assert!(a == b || (a.is_nan() && b.is_nan()));
    }
}

#[test]
fn mann_kendall_p_of_monotone_series() {
    // Шесть растущих значений: S = 15, дисперсия 6·5·17/18, p = erfc(|Z|/√2).
    let stack: Vec<Vec<f64>> = (0..6).map(|k| vec![k as f64]).collect();
    let days: Vec<f64> = (0..6).map(|k| k as f64 * YEAR).collect();
    let t = trend::trend(&stack, &days, MIN_SCENES).unwrap();
    assert!((t.slope[0] - 1.0).abs() <= 1e-12);
    // Z = 14 / sqrt(28.333...) = 2.6301, p = 0.008535 по таблице нормального закона.
    assert!((t.p[0] - 0.008535).abs() <= 1e-5, "{}", t.p[0]);
}

#[test]
fn same_day_pairs_are_skipped() {
    let stack: Vec<Vec<f64>> = [1.0, 2.0, 3.0].iter().map(|&v| vec![v]).collect();
    let t = trend::trend(&stack, &[0.0, 0.0, YEAR], 0).unwrap();
    assert!((t.slope[0] - 1.5).abs() <= 1e-12);
}

#[test]
fn shapes_must_match() {
    assert!(trend::trend(&[vec![1.0], vec![1.0, 2.0]], &[0.0, 1.0], 0).is_none());
    assert!(trend::trend(&[vec![1.0]], &[0.0, 1.0], 0).is_none());
    assert_eq!(trend::trend(&[], &[], 0), Some(Trend::default()));
}

#[test]
fn limits_symmetric() {
    let (low, high) = trend::limits(&[-0.2, 0.05, f64::NAN, 0.1], SHARE);
    assert_eq!(low, -high);
    assert!(high > 0.1);
    assert_eq!(trend::limits(&[f64::NAN], SHARE), (-1.0, 1.0));
    assert_eq!(trend::limits(&[0.0, 0.0], SHARE), (-1.0, 1.0));
}
