// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Вода по рельефу: уровень перелива, затопление, линия стока, водосбор
//! и его контур. Перенос `tests/test_hydro.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use data::hydro::{self, LineEnd, Shape};

const R: f64 = 6378137.0;

fn uniform(seed: u64, n: usize) -> Vec<f64> {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            s ^= s >> 12;
            s ^= s << 25;
            s ^= s >> 27;
            (s.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
        })
        .collect()
}

/// Уровень перелива перебором: Дейкстра с наибольшим рельефом пути
/// вместо суммы, восемь соседей.
fn minimax(z: &[f64], rows: usize, cols: usize, seed: (usize, usize)) -> Vec<f64> {
    let mut out = vec![f64::INFINITY; rows * cols];
    let s = seed.0 * cols + seed.1;
    out[s] = z[s];
    // Высоты в куче - биты положительных чисел, порядок тот же.
    let mut heap = BinaryHeap::new();
    heap.push(Reverse((z[s].to_bits(), s)));
    while let Some(Reverse((bits, i))) = heap.pop() {
        let level = f64::from_bits(bits);
        if level > out[i] {
            continue;
        }
        let (r, c) = (i / cols, i % cols);
        for dr in [-1i64, 0, 1] {
            for dc in [-1i64, 0, 1] {
                let (rr, cc) = (r as i64 + dr, c as i64 + dc);
                if (dr != 0 || dc != 0) && rr >= 0 && rr < rows as i64 && cc >= 0 && cc < cols as i64 {
                    let j = rr as usize * cols + cc as usize;
                    if z[j].is_finite() {
                        let new = level.max(z[j]);
                        if new < out[j] {
                            out[j] = new;
                            heap.push(Reverse((new.to_bits(), j)));
                        }
                    }
                }
            }
        }
    }
    out
}

/// Две котловины с дном 0 м, между ними седловина 5 м, вокруг 20 м.
fn two_pits() -> Vec<f64> {
    let (rows, cols) = (21, 41);
    let mut z = vec![20.0; rows * cols];
    for r in 0..rows {
        for c in 0..cols {
            for cx in [10.0, 30.0] {
                if (r as f64 - 10.0).hypot(c as f64 - cx) < 9.0 {
                    z[r * cols + c] = 0.0;
                }
            }
        }
    }
    for c in 18..23 {
        z[10 * cols + c] = 5.0;
    }
    z
}

fn count(mask: &[bool]) -> usize {
    mask.iter().filter(|m| **m).count()
}

#[test]
fn matches_minimax_search() {
    let (rows, cols) = (30, 37);
    let mut z: Vec<f64> = uniform(3, rows * cols).iter().map(|v| v * 50.0).collect();
    z[5 * cols + 7] = f64::NAN;
    z[12 * cols + 20] = f64::NAN;
    z[13 * cols + 20] = f64::NAN;
    let levels = hydro::spill_levels(&z, rows, cols, (15, 15));
    let reference = minimax(&z, rows, cols, (15, 15));
    for i in 0..rows * cols {
        if reference[i].is_finite() {
            assert!((levels[i] - reference[i]).abs() < 1e-9);
        } else {
            assert!(levels[i].is_infinite());
        }
    }
}

#[test]
fn ridge_keeps_other_pit_dry() {
    let z = two_pits();
    let levels = hydro::spill_levels(&z, 21, 41, (10, 10));
    let low = hydro::flood(&z, 21, 41, &levels, 4.0, 10.0);
    assert!(low.mask[10 * 41 + 10]);
    assert!(!low.mask[10 * 41 + 30]);
    assert!(!low.edge);
    let high = hydro::flood(&z, 21, 41, &levels, 6.0, 10.0);
    assert!(high.mask[10 * 41 + 30]);
    assert!((high.depth - 6.0).abs() < 1e-12);
}

#[test]
fn volume_is_depth_times_cell() {
    let mut z = vec![10.0; 25];
    z[12] = 7.0;
    z[13] = 8.0;
    let levels = hydro::spill_levels(&z, 5, 5, (2, 2));
    let f = hydro::flood(&z, 5, 5, &levels, 9.0, 2.0);
    assert_eq!(count(&f.mask), 2);
    assert!((f.area - 8.0).abs() < 1e-12);
    assert!((f.volume - 3.0 * 4.0).abs() < 1e-12);
}

#[test]
fn point_above_level_floods_nothing() {
    let z = two_pits();
    let levels = hydro::spill_levels(&z, 21, 41, (0, 0));
    assert_eq!(count(&hydro::flood(&z, 21, 41, &levels, 19.0, 10.0).mask), 0);
}

#[test]
fn water_nodes_keep_narrow_channel() {
    let mut mask = vec![false; 100];
    for r in 0..10 {
        mask[r * 10 + 5] = true;
    }
    let w = hydro::water_nodes(&mask, 10, 10, 5);
    assert_eq!(w.stride, 2);
    assert_eq!((w.rows, w.cols), (5, 5));
    assert!((0..5).all(|r| w.wet[r * 5 + 2]));
    assert_eq!(count(&w.wet), 5);
}

#[test]
fn index_round_trip_and_ground_step() {
    let grid = hydro::make_grid(58.0, 56.2, 5000.0, 10.0, R);
    assert_eq!(grid.rows % 2, 1);
    let (r, c) = hydro::index_of(&grid, 58.0, 56.2, R);
    assert_eq!((r, c), (grid.rows / 2, grid.cols / 2));
    let (_lat, lon) = hydro::latlon(&grid, r as f64, c as f64 + 1.0, R);
    let east = (lon - 56.2).to_radians() * R * 58f64.to_radians().cos();
    assert!((east - grid.cell).abs() < 0.01);
    assert!(grid.rows <= hydro::SIDE + 1);
    let (lats, lons) = hydro::nodes(&grid, R, 1);
    assert_eq!(lats.len(), grid.rows * grid.cols);
    assert!((lons[c] - hydro::latlon(&grid, 0.0, c as f64, R).1).abs() < 1e-12);
}

#[test]
fn line_runs_down_ramp_to_edge() {
    let (rows, cols) = (20, 30);
    let z: Vec<f64> = (0..rows * cols).map(|i| 100.0 - (i % cols) as f64 + (i / cols) as f64 * 0.01).collect();
    let line = hydro::flow_line(&z, rows, cols, (10, 3), 5.0, None, None).unwrap();
    assert_eq!(line.end, LineEnd::Edge);
    assert_eq!(line.cols.last(), Some(&29));
    assert!((line.length - 26.0 * 5.0).abs() <= 1.0);
    assert!(line.drop > 25.0);
}

#[test]
fn line_ends_in_pit_area_after_fill() {
    let z = two_pits();
    let line = hydro::flow_line(&z, 21, 41, (10, 14), 1.0, None, None).unwrap();
    assert_eq!(line.end, LineEnd::Edge);
}

/// Долина на юг: склоны к оси x = 20, уклон оси к нижнему краю.
fn valley(n: usize) -> Vec<f64> {
    (0..n * n).map(|i| ((i % n) as f64 - (n / 2) as f64).abs() * 2.0 + (n - 1 - i / n) as f64 * 0.5 + 10.0).collect()
}

/// Удвоенная площадь кольца по формуле шнурка.
fn ring_area(ring: &[[f64; 2]]) -> f64 {
    let mut s = 0.0;
    for k in 0..ring.len() {
        let (a, b) = (ring[k], ring[(k + ring.len() - 1) % ring.len()]);
        s += a[0] * b[1] - a[1] * b[0];
    }
    0.5 * s.abs()
}

#[test]
fn watershed_of_valley_and_outline() {
    let n = 41;
    let z = valley(n);
    let basin = hydro::watershed(&z, n, n, (38, 21), 10.0, None).unwrap();
    assert_eq!(basin.seed.1, 20);
    assert!(basin.seed.0 >= 38);
    assert!(count(&basin.mask) > 41 * 30);
    assert!(basin.mask[20]);
    assert!(basin.edge);
    assert!(basin.report.get("stream_km").is_some_and(|v| v > 0.3));
    assert_eq!(basin.stream_rows.len(), basin.stream_cols.len());
    let rings = hydro::outline(&basin.mask, n, n, 0.5);
    let area = ring_area(&rings[0]);
    let cells = count(&basin.mask) as f64;
    assert!((area - cells).abs() <= 0.05 * cells, "{area} {cells}");
}

#[test]
fn catchment_of_line_across_valley() {
    // Линия поперёк долины на юг перехватывает всё, что выше неё.
    let grid = hydro::make_grid(50.0, 30.0, 400.0, 10.0, R);
    let n = grid.rows;
    let z: Vec<f64> = (0..n * n).map(|i| ((i % n) as f64 - (n / 2) as f64).abs() * 2.0 + (n - i / n) as f64 * 0.5 + 10.0).collect();
    let a = hydro::latlon(&grid, 60.0, 10.0, R);
    let b = hydro::latlon(&grid, 60.0, (n - 11) as f64, R);
    let (seeds, length) = hydro::shape_seeds(&grid, Shape::Path, &[a, b], R);
    assert_eq!(seeds.len(), n - 20);
    assert!((length - (n - 21) as f64 * grid.cell).abs() <= 1.0);
    let result = hydro::shape_catchment(&z, n, n, &seeds, grid.cell, Some(length)).unwrap();
    assert!(result.mask[n / 2]);
    assert!(!result.mask[(n - 1) * n + n / 2]);
    assert!(result.edge);
    assert!(result.report.get("area_km2").is_some_and(|v| v > 0.0));
}

#[test]
fn polygon_seeds_cover_inside() {
    let grid = hydro::make_grid(50.0, 30.0, 200.0, 10.0, R);
    let ring: Vec<(f64, f64)> = [(10.0, 10.0), (10.0, 30.0), (30.0, 30.0), (30.0, 10.0)].iter().map(|(r, c)| hydro::latlon(&grid, *r, *c, R)).collect();
    let (seeds, _) = hydro::shape_seeds(&grid, Shape::Polygon, &ring, R);
    assert!(seeds.contains(&(20 * grid.cols + 20)));
    assert!(seeds.len() >= 21 * 21 - 5);
}

#[test]
fn rivers_join_into_higher_order() {
    // Две долины сходятся в одну: два звена порядка 1 и звено 2.
    let n = 61;
    let z: Vec<f64> = (0..n * n)
        .map(|i| {
            let (y, x) = ((i / n) as f64, (i % n) as f64);
            let v = if y < 40.0 { (x - 15.0).abs().min((x - 45.0).abs()) } else { (x - 30.0).abs() };
            v * 2.0 + (n as f64 - y)
        })
        .collect();
    let streams = hydro::rivers(&z, n, n, 10.0, 20.0 * 100.0, 1);
    let mut orders: Vec<u32> = streams.iter().map(|s| s.order).collect();
    orders.sort();
    orders.dedup();
    assert!(orders.contains(&1) && orders.contains(&2));
    assert!(streams.iter().all(|s| s.rows.len() > 1));
    // Порядок от 2 оставляет только звенья ниже слияния, порядок у них
    // прежний - он считается по всей сети.
    let trunk = hydro::rivers(&z, n, n, 10.0, 20.0 * 100.0, 2);
    assert!(!trunk.is_empty());
    let mut high: Vec<u32> = trunk.iter().map(|s| s.order).collect();
    high.sort();
    high.dedup();
    assert_eq!(high, orders.iter().copied().filter(|o| *o >= 2).collect::<Vec<_>>());
    assert!(trunk.len() < streams.len());
}

#[test]
fn line_stops_on_stream() {
    // Долина на юг: линия со склона кончается на оси долины.
    let n = 41;
    let z = valley(n);
    let whole = hydro::flow_line(&z, n, n, (10, 5), 10.0, None, None).unwrap();
    assert_eq!(whole.end, LineEnd::Edge);
    let line = hydro::flow_line(&z, n, n, (10, 5), 10.0, Some(60.0 * 100.0), None).unwrap();
    assert_eq!(line.end, LineEnd::Stream);
    assert!(line.cols.last().is_some_and(|c| c.abs_diff(20) <= 1));
    assert!(line.length < whole.length);
}

/// Крутой склон 30° на 20 ячеек, дальше равнина 0.5°.
fn steep_then_flat(cols: usize, cell: f64) -> Vec<f64> {
    let (steep, gentle) = (30f64.to_radians().tan(), 0.5f64.to_radians().tan());
    (0..21 * cols)
        .map(|i| {
            let (r, x) = ((i / cols) as f64, (i % cols) as f64);
            let top = if x < 20.0 { (20.0 - x) * cell * steep } else { 0.0 };
            top - x * cell * gentle + (r - 10.0).abs() * 0.001
        })
        .collect()
}

#[test]
fn line_stops_on_flattening() {
    let z = steep_then_flat(60, 10.0);
    let whole = hydro::flow_line(&z, 21, 60, (10, 2), 10.0, None, None).unwrap();
    let line = hydro::flow_line(&z, 21, 60, (10, 2), 10.0, None, Some((2.0, 50.0))).unwrap();
    assert_eq!(line.end, LineEnd::Flat);
    assert!(line.length < whole.length);
    assert!(line.cols.last().is_some_and(|c| c.abs_diff(20) <= 2));
}

#[test]
fn flattening_with_step_not_dividing_window() {
    // Шаг 16.7 м не делит окно 50 м: прежде уклон окна не мерился.
    let cell = 16.7;
    let z = steep_then_flat(80, cell);
    let line = hydro::flow_line(&z, 21, 80, (10, 2), cell, None, Some((2.0, 50.0))).unwrap();
    assert_eq!(line.end, LineEnd::Flat);
    assert!(line.cols.last().is_some_and(|c| c.abs_diff(20) <= 2));
}

#[test]
fn lines_from_shape_merge() {
    // Линии от многих узлов поперёк склона долины сливаются в одну.
    let n = 41;
    let z = valley(n);
    let seeds: Vec<usize> = (2..39).map(|c| 10 * 41 + c).collect();
    let apart = hydro::flow_lines(&z, n, n, &seeds, 10.0, None, None, false);
    let joined = hydro::flow_lines(&z, n, n, &seeds, 10.0, None, None, true);
    assert_eq!(apart.len(), seeds.len());
    assert!(joined.len() <= seeds.len());
    assert!(joined.iter().any(|l| l.end == LineEnd::Merge));
    assert!(joined.iter().any(|l| l.end == LineEnd::Edge));
    // Узел принадлежит одной линии, кроме узла слияния.
    let mut all: Vec<(usize, usize)> = Vec::new();
    let mut total = 0;
    for line in &joined {
        let mut cells: Vec<(usize, usize)> = line.rows.iter().copied().zip(line.cols.iter().copied()).collect();
        cells.sort();
        cells.dedup();
        total += cells.len();
        all.extend(cells);
    }
    all.sort();
    all.dedup();
    let merged = joined.iter().filter(|l| l.end == LineEnd::Merge).count();
    assert_eq!(all.len(), total - merged);
}

#[test]
fn outlet_basins_split_square() {
    // Хребет посередине: две долины, два устья у нижнего края.
    let n = 41;
    let z: Vec<f64> = (0..n * n)
        .map(|i| {
            let (y, x) = ((i / n) as f64, (i % n) as f64);
            (x - 10.0).abs().min((x - 30.0).abs()) * 2.0 + (n as f64 - y) * 0.5
        })
        .collect();
    let regions = hydro::outlet_basins(&z, n, n, 10.0, 100.0 * 100.0);
    assert_eq!(regions.len(), 2);
    let mean_col = |mask: &[bool]| {
        let cols: Vec<f64> = mask.iter().enumerate().filter(|(_, m)| **m).map(|(i, _)| (i % n) as f64).collect();
        cols.iter().sum::<f64>() / cols.len() as f64
    };
    let (left, right) = if mean_col(&regions[0].mask) < mean_col(&regions[1].mask) { (&regions[0], &regions[1]) } else { (&regions[1], &regions[0]) };
    assert!(left.mask[10]);
    assert!(right.mask[30]);
    assert!(!left.mask[30]);
    assert!((left.area - count(&left.mask) as f64 * 100.0).abs() < 1e-9);
}

#[test]
fn shape_grid_covers_shape() {
    let (lat, _lon, side) = hydro::shape_grid(&[(50.0, 30.0), (50.01, 30.0)], R, 3.0, 2000.0, 50000.0).unwrap();
    assert!((lat - 50.005).abs() < 1e-9);
    assert!((side - 2000.0).abs() < 1e-9);
    let (_, _, side) = hydro::shape_grid(&[(50.0, 30.0), (50.2, 30.0)], R, 3.0, 2000.0, 50000.0).unwrap();
    assert!((side - 3.0 * 0.1f64.to_radians() * R).abs() <= 1.0);
    assert!(hydro::shape_grid(&[], R, 3.0, 2000.0, 50000.0).is_none());
}

#[test]
fn simplify_keeps_corners_on_line_of_ring_ends() {
    // Кольцо начинается посередине длинной стороны, концы цепочки соседние.
    // Дуглас-Пекер до прямой через них выбрасывал углы на этой прямой.
    let mut ring: Vec<[f64; 2]> = (5..101).map(|x| [0.0, x as f64]).collect();
    ring.extend([[1.0, 100.0], [2.0, 100.0]]);
    ring.extend((0..100).rev().map(|x| [2.0, x as f64]));
    ring.push([1.0, 0.0]);
    ring.extend((0..5).map(|x| [0.0, x as f64]));
    let kept = hydro::simplify(&ring, 0.5);
    for corner in [[0.0, 100.0], [2.0, 100.0], [2.0, 0.0], [0.0, 0.0]] {
        assert!(kept.contains(&corner), "{corner:?}");
    }
}

#[test]
fn outline_of_square_is_four_corners() {
    let mut mask = vec![false; 100];
    for r in 2..6 {
        for c in 3..8 {
            mask[r * 10 + c] = true;
        }
    }
    let ring = &hydro::outline(&mask, 10, 10, 0.5)[0];
    assert!(ring.len() <= 8);
    assert!((ring.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min) - 1.5).abs() < 1e-9);
    assert!((ring.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max) - 7.5).abs() < 1e-9);
}

#[test]
fn flood_tile_paints_water_only() {
    let grid = hydro::make_grid(55.0, 37.0, 500.0, 10.0, R);
    let n = grid.rows;
    let z = vec![100.0; n * n];
    let levels = hydro::spill_levels(&z, n, n, (n / 2, n / 2));
    let flood = hydro::flood(&z, n, n, &levels, 101.0, grid.cell);
    assert_eq!(count(&flood.mask), n * n);
    // Тайл уровня 15 у точки: вода есть.
    let zoom = 15u32;
    let scale = f64::from(1u32 << zoom);
    let x = ((37.0 + 180.0) / 360.0 * scale) as u32;
    let lat = 55f64.to_radians();
    let y = ((1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * scale) as u32;
    let rgba = hydro::flood_tile_rgba(&grid, &flood, &z, 101.0, (zoom, x, y), R).unwrap();
    assert_eq!(rgba.len(), 256 * 256 * 4);
    assert!(rgba.chunks(4).any(|p| p[3] > 0));
    // Тайл далеко от сетки не задевает её.
    assert!(hydro::flood_tile_rgba(&grid, &flood, &z, 101.0, (zoom, x + 50, y), R).is_none());
}

#[test]
fn hostile_input_does_not_panic() {
    assert!(hydro::spill_levels(&[1.0; 3], 2, 2, (0, 0)).is_empty());
    assert!(hydro::flow_line(&[1.0; 4], 2, 2, (9, 9), 1.0, None, None).is_none());
    assert!(hydro::watershed(&[f64::NAN; 9], 3, 3, (1, 1), 1.0, None).is_some());
    assert!(hydro::rivers(&[1.0], 1, 1, 1.0, 1.0, 1).is_empty());
    assert!(hydro::outline(&[true; 3], 2, 2, 0.5).is_empty());
    let grid = hydro::make_grid(f64::NAN, f64::NAN, f64::NAN, f64::NAN, R);
    assert!(grid.rows >= 1);
}
