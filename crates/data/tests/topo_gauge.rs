// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Ядро отчёта по створу. Перенос `tests/test_topo_gauge.py` модуля
//! (из Isoliner). Синтетический рельеф, эталонные цифры посчитаны руками.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use data::hydro_fill::fill_depressions;
use data::topo_flow as tf;
use data::topo_gauge::{self as tg, Extras};

const CELL: f64 = 10.0;

struct Case {
    z: Vec<f64>,
    down: Vec<i64>,
    acc: Vec<f64>,
    rows: usize,
    cols: usize,
}

/// Наклонная плоскость: вода течёт строго на восток вдоль строк.
/// Аккумуляция в ячейке (r, c) равна c + 1.
fn ramp() -> Case {
    let (rows, cols) = (4, 6);
    let z: Vec<f64> = (0..rows * cols).map(|i| (cols - i % cols) as f64).collect();
    let (_, down) = tf::d8_directions(&z, rows, cols, None);
    let acc = tf::flow_accumulation(&down, rows, cols, None);
    Case { z, down, acc, rows, cols }
}

/// V-долина с осевым водотоком по среднему столбцу, сток на юг:
/// z = |c - 2| * 10 - r.
fn valley() -> Case {
    let (rows, cols) = (5, 5);
    let z: Vec<f64> = (0..rows * cols).map(|i| ((i % cols) as f64 - 2.0).abs() * 10.0 - (i / cols) as f64).collect();
    let (_, down) = tf::d8_directions(&z, rows, cols, None);
    let acc = tf::flow_accumulation(&down, rows, cols, None);
    Case { z, down, acc, rows, cols }
}

fn close(a: Option<f64>, b: f64, tol: f64) -> bool {
    a.is_some_and(|a| (a - b).abs() <= tol)
}

// --- притяжка ----------------------------------------------------------------

#[test]
fn snap_to_max_acc() {
    let k = ramp();
    // Максимумы в окне равны, берётся первый по обходу окна - верхняя строка.
    assert_eq!(tg::snap_to_max_acc(&k.acc, k.rows, k.cols, 1, 3, 2), (0, 5));
    // Радиус 0: без притяжки.
    assert_eq!(tg::snap_to_max_acc(&k.acc, k.rows, k.cols, 1, 3, 0), (1, 3));
    // Выход за край зажимается.
    assert_eq!(tg::snap_to_max_acc(&k.acc, k.rows, k.cols, -5, 99, 0), (0, 5));
}

#[test]
fn snap_deterministic_on_ties() {
    let mut acc = vec![0.0; 9];
    acc[1] = 7.0;
    acc[7] = 7.0;
    assert_eq!(tg::snap_to_max_acc(&acc, 3, 3, 1, 1, 1), (0, 1));
}

// --- бассейн и зональная статистика ----------------------------------------------

#[test]
fn basin_mask_ramp() {
    let k = ramp();
    // Створ на восточном краю строки 2: бассейн - ровно эта строка.
    let mask = tg::basin_mask(&k.down, k.rows, k.cols, 2 * k.cols + k.cols - 1);
    assert_eq!(mask.iter().filter(|m| **m).count(), k.cols);
    assert!((0..k.cols).all(|c| mask[2 * k.cols + c] && !mask[k.cols + c] && !mask[3 * k.cols + c]));
}

#[test]
fn basin_mask_valley_full_grid() {
    let k = valley();
    let seed = (k.rows - 1) * k.cols + 2;
    let mask = tg::basin_mask(&k.down, k.rows, k.cols, seed);
    // Весь грид стекает через створ.
    assert!(mask.iter().all(|m| *m));
    assert_eq!(k.acc[seed], 25.0);
}

#[test]
fn zonal_stats_nodata_and_empty() {
    let v = [1.0, 2.0, 3.0, f64::NAN];
    let mask = [true; 4];
    assert_eq!(tg::zonal_stats(&v, &mask, None), Some((2.0, 1.0, 3.0)));
    let nodata = [false, true, false, true];
    assert_eq!(tg::zonal_stats(&v, &mask, Some(&nodata)), Some((2.0, 1.0, 3.0)));
    assert_eq!(tg::zonal_stats(&v, &[false; 4], None), None);
}

// --- главный водоток ------------------------------------------------------------

#[test]
fn trace_ramp_row() {
    let k = ramp();
    let seed = k.cols + k.cols - 1;
    let (length, path) = tg::trace_main_stream(&k.down, &k.acc, k.rows, k.cols, seed, CELL);
    // Вся строка: 5 шагов по 10 м, путь от створа к истоку.
    assert!((length - (k.cols - 1) as f64 * CELL).abs() < 1e-9);
    assert_eq!(path[0], seed);
    assert_eq!(path.last(), Some(&k.cols));
    assert_eq!(path.len(), k.cols);
}

#[test]
fn trace_valley_axis() {
    let k = valley();
    let seed = (k.rows - 1) * k.cols + 2;
    let (length, path) = tg::trace_main_stream(&k.down, &k.acc, k.rows, k.cols, seed, CELL);
    // Вверх по оси 4 шага, потом в сильнейший приток до ячейки без притоков.
    assert!((length - 6.0 * CELL).abs() < 1e-9);
    assert_eq!(path.iter().map(|p| p % k.cols).collect::<Vec<_>>(), vec![2, 2, 2, 2, 2, 3, 4]);
    // Исток: угловая ячейка без притоков.
    assert_eq!(path.last(), Some(&4));
}

#[test]
fn trace_stops_at_source() {
    let k = ramp();
    let (length, path) = tg::trace_main_stream(&k.down, &k.acc, k.rows, k.cols, 0, CELL);
    assert!(length == 0.0 && path == vec![0]);
}

// --- сборный отчёт ----------------------------------------------------------------

#[test]
fn gauge_report_ramp_hand_numbers() {
    let k = ramp();
    let seed = 2 * k.cols + k.cols - 1;
    let rep = tg::gauge_report(&k.z, &k.down, &k.acc, k.rows, k.cols, seed, CELL, &Extras::default());
    // Площадь: 6 ячеек по 100 м² = 0.0006 км².
    assert!(close(rep.get("area_km2"), 6.0 * CELL * CELL / 1e6, 1e-12));
    assert_eq!(rep.get("cells"), Some(6.0));
    assert_eq!((rep.get("z_min"), rep.get("z_max")), (Some(1.0), Some(6.0)));
    assert!(close(rep.get("z_mean"), 3.5, 1e-12));
    assert_eq!(rep.get("z_gauge"), Some(1.0));
    // Водоток: 50 м, падение 6 - 1 = 5 м, уклон 100 промилле.
    assert!(close(rep.get("stream_km"), 0.05, 1e-12));
    assert!(close(rep.get("stream_fall_m"), 5.0, 1e-12));
    assert!(close(rep.get("stream_ppm"), 100.0, 1e-9));
    // Уклон бассейна не подавали - пусто, а не ноль.
    assert_eq!(rep.get("slope_mean"), None);
}

#[test]
fn gauge_report_valley_with_slope() {
    let k = valley();
    let seed = (k.rows - 1) * k.cols + 2;
    let slope = vec![2.5; 25];
    let extras = Extras { slope: Some(&slope), ..Extras::default() };
    let rep = tg::gauge_report(&k.z, &k.down, &k.acc, k.rows, k.cols, seed, CELL, &extras);
    assert_eq!(rep.get("cells"), Some(25.0));
    assert!(close(rep.get("slope_mean"), 2.5, 1e-12));
    // Трасса до истока (0, 4): z от -4 (створ) до 20, падение 24 м на 60 м.
    assert!(close(rep.get("stream_fall_m"), 24.0, 1e-12));
    assert!(close(rep.get("stream_km"), 0.06, 1e-12));
    assert!(close(rep.get("stream_ppm"), 400.0, 1e-9));
    assert_eq!(rep.get("z_gauge"), Some(-4.0));
}

#[test]
fn gauge_report_nodata_excluded() {
    let k = ramp();
    let mut nodata = vec![false; 24];
    // Исток строки выключен из статистики.
    nodata[2 * k.cols] = true;
    let extras = Extras { nodata: Some(&nodata), ..Extras::default() };
    let rep = tg::gauge_report(&k.z, &k.down, &k.acc, k.rows, k.cols, 2 * k.cols + k.cols - 1, CELL, &extras);
    assert_eq!(rep.get("cells"), Some(5.0));
    assert_eq!(rep.get("z_max"), Some(5.0));
}

#[test]
fn report_lines() {
    let k = ramp();
    let rep = tg::gauge_report(&k.z, &k.down, &k.acc, k.rows, k.cols, 2 * k.cols + k.cols - 1, CELL, &Extras::default());
    let lines = tg::report_lines(&rep);
    assert_eq!(lines.len(), 2);
    let text: Vec<String> = lines.iter().map(tg::render).collect();
    assert!(text[0].contains("0.001") && text[0].contains("1.00...6.00"), "{}", text[0]);
    assert!(text[1].contains("100.0"), "{}", text[1]);
    // Переводчику отдаются шаблоны с подстановками.
    assert!(lines.iter().all(|(template, _)| template.contains('{')));
}

#[test]
fn report_keys_order_stable() {
    assert_eq!(tg::REPORT_KEYS[0], "area_km2");
    assert_eq!(tg::REPORT_KEYS[tg::REPORT_KEYS.len() - 1], "cells");
    let mut keys = tg::REPORT_KEYS.to_vec();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), tg::REPORT_KEYS.len());
    let rep = tg::Report::new(&tg::REPORT_KEYS);
    assert_eq!(rep.entries().iter().map(|e| e.0).collect::<Vec<_>>(), tg::REPORT_KEYS.to_vec());
}

#[test]
fn network_and_contour_lengths() {
    let k = valley();
    let mask = vec![true; 25];
    // Сеть от аккумуляции 2: ось долины и угловые цепочки.
    let net = tg::network_length_in_mask(&k.down, &k.acc, k.rows, k.cols, &mask, CELL, 2.0, None);
    assert!(net.is_some_and(|m| m > 0.0));
    assert_eq!(tg::network_length_in_mask(&k.down, &k.acc, k.rows, k.cols, &mask, CELL, 0.0, None), None);
    // Ось долины падает на 1 м за строку: горизонтали через 1 м между
    // строками средней колонки, переходы есть.
    let iso = tg::contour_length_in_mask(&k.z, k.rows, k.cols, &mask, CELL, 1.0, None);
    assert!(iso > 0.0);
    assert_eq!(tg::contour_length_in_mask(&k.z, k.rows, k.cols, &mask, CELL, 0.0, None), 0.0);
    assert!(tg::sp_slope_ppm(iso, 2500.0, 1.0).is_some_and(|v| v > 0.0));
    assert_eq!(tg::sp_slope_ppm(iso, 0.0, 1.0), None);
}

#[test]
fn sp_stream_of_straight_profile() {
    // Прямой профиль: средневзвешенный уклон равен простому.
    let z = [10.0, 8.0, 6.0, 4.0];
    assert!(tg::sp_stream_ppm(&z, 10.0, None).is_some_and(|v| (v - 200.0).abs() < 1e-9));
    assert_eq!(tg::sp_stream_ppm(&[1.0], 10.0, None), None);
    assert_eq!(tg::sp_stream_ppm(&[1.0, 1.0, 1.0], 10.0, None), None);
}

#[test]
fn mask_report_and_outlet() {
    let k = valley();
    let mask = vec![true; 25];
    let seed = tg::outlet_in_mask(&k.z, &mask, Some(&k.acc), None);
    assert_eq!(seed, Some(22));
    assert_eq!(tg::outlet_in_mask(&k.z, &mask, None, None), Some(22));
    let rep = tg::mask_report(&k.z, k.rows, k.cols, &mask, CELL, Some((&k.down, &k.acc)), None, &Extras::default(), true);
    assert_eq!(rep.get("cells"), Some(25.0));
    assert!(close(rep.get("stream_km"), 0.06, 1e-12));
    let rep = tg::mask_report(&k.z, k.rows, k.cols, &mask, CELL, None, None, &Extras::default(), true);
    assert_eq!(rep.get("stream_km"), None);
    assert_eq!(tg::outlet_in_mask(&k.z, &[false; 25], None, None), None);
}

// --- линейный водосбор (канава) ---------------------------------------------------

#[test]
fn cells_along_polyline_straight() {
    // Линия вдоль строки 1 через весь грид 4x6.
    let got = tg::cells_along_polyline(&[(5.0, -15.0), (55.0, -15.0)], 0.0, 0.0, CELL, 4, 6, 0.5);
    assert_eq!(got, (0..6).map(|c| 6 + c).collect::<Vec<_>>());
}

#[test]
fn cells_along_polyline_diagonal_no_gaps() {
    let got = tg::cells_along_polyline(&[(5.0, -5.0), (45.0, -45.0)], 0.0, 0.0, CELL, 5, 5, 0.5);
    assert_eq!(got, vec![0, 6, 12, 18, 24]);
}

#[test]
fn cells_along_polyline_clips_and_dedups() {
    // Уходит за левый край и возвращается: наружные точки отброшены.
    let got = tg::cells_along_polyline(&[(-100.0, -15.0), (25.0, -15.0), (25.0, -15.0)], 0.0, 0.0, CELL, 4, 6, 0.5);
    assert_eq!(got, vec![6, 7, 8]);
}

#[test]
fn cells_in_polygon_square() {
    // Квадрат 3x3 ячейки: внутренность строчной развёрткой, ничего лишнего.
    let ring = vec![(22.0, -22.0), (48.0, -22.0), (48.0, -48.0), (22.0, -48.0), (22.0, -22.0)];
    let mut got = tg::cells_in_polygon(&[ring], 0.0, 0.0, CELL, 6, 6);
    got.sort();
    let want: Vec<usize> = (2..5).flat_map(|r| (2..5).map(move |c| r * 6 + c)).collect();
    assert_eq!(got, want);
}

#[test]
fn cells_in_polygon_tiny_boundary_only() {
    // Полигон меньше ячейки: ни один центр не внутри, ячейку даёт контур.
    let ring = vec![(21.0, -21.0), (23.0, -21.0), (23.0, -23.0), (21.0, -23.0)];
    assert_eq!(tg::cells_in_polygon(&[ring], 0.0, 0.0, CELL, 6, 6), vec![14]);
}

#[test]
fn cells_in_polygon_clips_to_grid() {
    let ring = vec![(-50.0, 50.0), (80.0, 50.0), (80.0, -80.0), (-50.0, -80.0)];
    let mut got = tg::cells_in_polygon(&[ring], 0.0, 0.0, CELL, 3, 3);
    got.sort();
    assert_eq!(got, (0..9).collect::<Vec<_>>());
}

#[test]
fn polygon_seed_catchment_over_depression() {
    // Канал рядов 2..4 падает на запад, огорожен гребнями в рядах 1 и 5,
    // в середине канала вырыта яма глубиной 5.
    let (rows, cols) = (7, 7);
    let mut z: Vec<f64> = (0..49).map(|i| (i % cols) as f64 * 0.1).collect();
    for c in 0..cols {
        z[cols + c] += 100.0;
        z[5 * cols + c] += 100.0;
        z[c] += 0.05;
        z[6 * cols + c] += 0.05;
    }
    for r in 2..5 {
        for c in 2..5 {
            z[r * cols + c] -= 5.0;
        }
    }
    let zf = fill_depressions(&z, rows, cols, None, 1e-3, None).unwrap().filled;
    let (_, down) = tf::d8_directions(&zf, rows, cols, None);
    let ring = vec![(22.0, -22.0), (48.0, -22.0), (48.0, -48.0), (22.0, -48.0)];
    let seeds = tg::cells_in_polygon(&[ring], 0.0, 0.0, CELL, rows, cols);
    let mut sorted = seeds.clone();
    sorted.sort();
    assert_eq!(sorted, (2..5).flat_map(|r| (2..5).map(move |c| r * cols + c)).collect::<Vec<_>>());
    let m = tg::catchment_mask(&down, rows, cols, &seeds);
    let mut want = vec![false; 49];
    for r in 0..rows {
        for c in 0..cols {
            // Затравка, канал восточнее ямы, гребни над и под каналом.
            want[r * cols + c] = ((2..5).contains(&r) && c >= 2) || ((r == 1 || r == 5) && c >= 2);
        }
    }
    assert_eq!(m, want);
}

#[test]
fn ditch_report_seed_km2() {
    let k = valley();
    let seeds: Vec<usize> = (0..k.cols).map(|c| 3 * k.cols + c).collect();
    let rep = tg::ditch_report(&k.z, &k.down, k.rows, k.cols, &seeds, CELL, None, &Extras::default());
    assert!(close(rep.get("seed_km2"), k.cols as f64 * CELL * CELL / 1e6, 1e-12));
    assert!(tg::DITCH_KEYS.contains(&"seed_km2"));
    let mut keys = tg::DITCH_KEYS.to_vec();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), tg::DITCH_KEYS.len());
}

#[test]
fn burn_sloped_bottom_is_monotone_to_outfall() {
    // Дно жёлоба монотонно падает к стоку, перевала через борт нет.
    let (rows, cols) = (3, 10);
    let mut z = vec![0.0; rows * cols];
    let profile = [10.0, 9.0, 8.0, 9.5, 11.0, 8.0, 7.0, 8.5, 6.0, 5.0];
    z[cols..2 * cols].copy_from_slice(&profile);
    let run: Vec<usize> = (0..cols).map(|c| cols + c).collect();
    let out = tg::burn_trace_sloped(&z, std::slice::from_ref(&run), 1.0, 0.01, CELL, None);
    let bottom: Vec<f64> = run.iter().map(|i| out[*i]).collect();
    // Сток к концу, падение строго монотонное, дно не выше врезки.
    assert!(bottom.windows(2).all(|w| w[1] - w[0] <= -0.01 * CELL + 1e-12));
    assert!(bottom.iter().zip(profile).all(|(b, p)| *b <= p - 1.0 + 1e-12));
    // Чужие ячейки не тронуты.
    assert!(out[..cols] == z[..cols] && out[2 * cols..] == z[2 * cols..]);
}

#[test]
fn burn_sloped_direction_follows_low_end() {
    let z: Vec<f64> = (0..8).map(|c| 2.0 + c as f64).collect();
    let run: Vec<usize> = (0..8).collect();
    let out = tg::burn_trace_sloped(&z, &[run], 0.5, 0.01, CELL, None);
    // Падает к началу.
    assert!(out.windows(2).all(|w| w[1] - w[0] >= 0.01 * CELL - 1e-12));
}

#[test]
fn burn_sloped_zero_slope_still_monotone() {
    let z = [5.0, 3.0, 6.0, 2.0];
    let out = tg::burn_trace_sloped(&z, &[vec![0, 1, 2, 3]], 1.0, 0.0, CELL, None);
    assert!(out.windows(2).all(|w| w[1] - w[0] <= 1e-12));
    assert_eq!(out[3], 1.0);
}

#[test]
fn burn_sloped_nodata_and_degenerate() {
    let z = [5.0, 4.0, 3.0];
    let mask = [false, true, false];
    let out = tg::burn_trace_sloped(&z, &[vec![0, 1, 2]], 1.0, 0.01, CELL, Some(&mask));
    // Nodata не тронута.
    assert_eq!(out[1], 4.0);
    assert!(out[0] == 4.0 && out[2] <= 2.0);
    // Пустые входы - копия без изменений.
    assert_eq!(tg::burn_trace_sloped(&z, &[], 1.0, 0.01, CELL, None), z.to_vec());
    assert_eq!(tg::burn_trace_sloped(&z, &[vec![0]], 0.0, 0.01, CELL, None), z.to_vec());
    // Номер за решёткой не роняет.
    assert_eq!(tg::burn_trace_sloped(&z, &[vec![0, 7]], 1.0, 0.01, CELL, None)[0], 4.0);
}

#[test]
fn polyline_length() {
    assert!((tg::polyline_length(&[(0.0, 0.0), (30.0, 40.0)]) - 50.0).abs() < 1e-9);
    assert_eq!(tg::polyline_length(&[(0.0, 0.0)]), 0.0);
}

#[test]
fn catchment_of_line_across_valley() {
    // Канава поперёк долины перехватывает всё, что выше неё по стоку.
    let k = valley();
    let seeds: Vec<usize> = (0..k.cols).map(|c| 3 * k.cols + c).collect();
    let mask = tg::catchment_mask(&k.down, k.rows, k.cols, &seeds);
    assert!(mask[..20].iter().all(|m| *m));
    assert!(!mask[20..].iter().any(|m| *m));
}

#[test]
fn catchment_of_single_cell_matches_gauge() {
    let k = ramp();
    let seed = 2 * k.cols + k.cols - 1;
    assert_eq!(tg::catchment_mask(&k.down, k.rows, k.cols, &[seed]), tg::basin_mask(&k.down, k.rows, k.cols, seed));
}

#[test]
fn catchment_empty_seeds() {
    let k = ramp();
    assert!(!tg::catchment_mask(&k.down, k.rows, k.cols, &[]).iter().any(|m| *m));
}

#[test]
fn burn_trace_lowers_only_trace() {
    let k = valley();
    let seeds: Vec<usize> = (0..k.cols).map(|c| 3 * k.cols + c).collect();
    let zb = tg::burn_trace(&k.z, &seeds, 5.0, None);
    assert!(seeds.iter().all(|i| zb[*i] == k.z[*i] - 5.0));
    assert!((0..k.cols).all(|c| zb[c] == k.z[c]));
}

#[test]
fn burn_trace_respects_nodata_and_zero_depth() {
    let k = valley();
    let seeds: Vec<usize> = (0..k.cols).map(|c| 3 * k.cols + c).collect();
    let mut nod = vec![false; 25];
    nod[15] = true;
    let zb = tg::burn_trace(&k.z, &seeds, 5.0, Some(&nod));
    assert_eq!(zb[15], k.z[15]);
    assert_eq!(zb[16], k.z[16] - 5.0);
    assert_eq!(tg::burn_trace(&k.z, &seeds, 0.0, None), k.z);
}

#[test]
fn ditch_report_hand_numbers() {
    let k = valley();
    let seeds: Vec<usize> = (0..k.cols).map(|c| 3 * k.cols + c).collect();
    let slope = vec![1.5; 25];
    let extras = Extras { slope: Some(&slope), ..Extras::default() };
    let rep = tg::ditch_report(&k.z, &k.down, k.rows, k.cols, &seeds, CELL, Some(4.0 * CELL), &extras);
    assert_eq!(rep.get("cells"), Some(20.0));
    assert!(close(rep.get("area_km2"), 20.0 * CELL * CELL / 1e6, 1e-12));
    assert_eq!(rep.get("trace_cells"), Some(5.0));
    assert!(close(rep.get("trace_km"), 0.04, 1e-12));
    assert!(close(rep.get("slope_mean"), 1.5, 1e-12));
    // Высоты только по водосбору: строки 0..3 долины.
    assert_eq!((rep.get("z_max"), rep.get("z_min")), (Some(20.0), Some(-3.0)));
}

#[test]
fn ditch_report_nodata_excluded() {
    let k = valley();
    let seeds: Vec<usize> = (0..k.cols).map(|c| 3 * k.cols + c).collect();
    let mut nod = vec![false; 25];
    nod[..5].fill(true);
    let extras = Extras { nodata: Some(&nod), ..Extras::default() };
    let rep = tg::ditch_report(&k.z, &k.down, k.rows, k.cols, &seeds, CELL, None, &extras);
    assert_eq!(rep.get("cells"), Some(15.0));
}

#[test]
fn ditch_report_lines() {
    let k = valley();
    let seeds: Vec<usize> = (0..k.cols).map(|c| 3 * k.cols + c).collect();
    let rep = tg::ditch_report(&k.z, &k.down, k.rows, k.cols, &seeds, CELL, Some(40.0), &Extras::default());
    let lines = tg::ditch_report_lines(&rep);
    assert_eq!(lines.len(), 2);
    let text: Vec<String> = lines.iter().map(tg::render).collect();
    assert!(text[0].contains("0.002"), "{}", text[0]);
    assert!(text[1].contains("0.040") && text[1].contains("trace cells 5"), "{}", text[1]);
    assert!(lines.iter().all(|(template, _)| template.contains('{')));
}

#[test]
fn hostile_input_does_not_panic() {
    let rep = tg::gauge_report(&[1.0], &[0], &[1.0], 3, 3, 99, CELL, &Extras::default());
    assert_eq!(rep.get("cells"), Some(0.0));
    assert!(tg::basin_mask(&[0, 1], 1, 2, 7).iter().all(|m| !*m));
    assert_eq!(tg::trace_main_stream(&[1, 0], &[1.0, 1.0], 1, 2, 9, CELL).1, Vec::<usize>::new());
    assert!(tg::cells_along_polyline(&[(f64::NAN, 0.0), (1e300, -1e300)], 0.0, 0.0, 0.0, 3, 3, 0.5).len() <= 9);
    assert!(tg::cells_in_polygon(&[vec![(0.0, 0.0), (f64::NAN, 1.0), (1e308, -1e308)]], 0.0, 0.0, 1.0, 3, 3).len() <= 9);
}
