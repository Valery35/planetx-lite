// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сегментация снимка: объекты по цвету и текстуре, классы объектов.
//! Перенос `tests/test_segments.py` модуля, тест остановки расчёта
//! заменён проходами [`segments::Cut`], тесты границ объектов - свои.

use data::{rasterclass, segments};
use segments::{LEAST, Params, SHAPE, WEIGHT};

/// Простой генератор с равномерным распределением: тесту нужен шум,
/// а не ряд NumPy.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f32) / (1u64 << 31) as f32
    }

    /// Нормальный шум, сумма двенадцати равномерных.
    fn normal(&mut self) -> f32 {
        (0..12).map(|_| self.next()).sum::<f32>() - 6.0
    }
}

const SIDE: usize = 120;

/// Снимок 120 × 120: тёмный запад, светлый восток, в востоке пруд.
fn scene(noise: f32) -> Vec<f32> {
    let mut random = Noise(5);
    let mut out = Vec::with_capacity(SIDE * SIDE * 3);
    for r in 0..SIDE {
        for c in 0..SIDE {
            let colour = if (30..60).contains(&r) && (80..110).contains(&c) {
                [30.0, 30.0, 30.0]
            } else if c < 60 {
                [40.0, 90.0, 50.0]
            } else {
                [190.0, 170.0, 110.0]
            };
            out.extend(colour.map(|v: f32| v + noise * random.normal()));
        }
    }
    out
}

fn segment(grid: &[f32], depth: usize, side: usize, scale: f64, weight: f64, least: u32) -> Vec<i32> {
    segments::segment(grid, depth, side, side, None, Params { scale, weight, least, shape: SHAPE })
}

/// Главный объект части и его доля.
fn main_part(labels: &[i32], rows: std::ops::Range<usize>, cols: std::ops::Range<usize>) -> (i32, f64) {
    let mut seen = std::collections::HashMap::new();
    let mut total = 0;
    for r in rows {
        for c in cols.clone() {
            *seen.entry(labels[r * SIDE + c]).or_insert(0usize) += 1;
            total += 1;
        }
    }
    let (object, most) = seen.into_iter().max_by_key(|v| v.1).unwrap_or((-1, 0));
    (object, most as f64 / total as f64)
}

/// Клетки объекта связны по сторонам.
fn single(labels: &[i32], side: usize, object: i32) -> bool {
    let cells: Vec<usize> = (0..labels.len()).filter(|i| labels[*i] == object).collect();
    let Some(&first) = cells.first() else {
        return true;
    };
    let mut seen = std::collections::HashSet::from([first]);
    let mut queue = vec![first];
    while let Some(at) = queue.pop() {
        let (r, c) = (at / side, at % side);
        let near = [(r > 0).then(|| at - side), (r + 1 < side).then(|| at + side), (c > 0).then(|| at - 1), (c + 1 < side).then(|| at + 1)];
        for n in near.into_iter().flatten() {
            if labels[n] == object && seen.insert(n) {
                queue.push(n);
            }
        }
    }
    seen.len() == cells.len()
}

#[test]
fn finds_parts_of_scene() {
    let labels = segment(&scene(4.0), 3, SIDE, 25.0, WEIGHT, LEAST);
    let west = main_part(&labels, 60..120, 0..50);
    let east = main_part(&labels, 70..120, 70..120);
    let pond = main_part(&labels, 35..55, 85..105);
    // Главный объект части покрывает её почти целиком.
    for part in [west, east, pond] {
        assert!(part.1 > 0.95, "{part:?}");
    }
    let unique: std::collections::HashSet<i32> = [west.0, east.0, pond.0].into();
    assert_eq!(unique.len(), 3);
}

#[test]
fn objects_are_connected() {
    let labels = segment(&scene(4.0), 3, SIDE, 25.0, WEIGHT, LEAST);
    for object in 0..segments::count(&labels) as i32 {
        assert!(single(&labels, SIDE, object));
    }
}

#[test]
fn two_ponds_of_one_color_stay_apart() {
    let mut bands = vec![100.0f32; 40 * 40];
    for (rows, cols) in [(5..12, 5..12), (25..35, 25..35)] {
        for r in rows {
            for c in cols.clone() {
                bands[r * 40 + c] = 10.0;
            }
        }
    }
    let labels = segment(&bands, 1, 40, 5.0, 0.0, 1);
    assert_ne!(labels[8 * 40 + 8], labels[30 * 40 + 30]);
}

#[test]
fn scale_changes_amount() {
    let grid = scene(20.0);
    let few = segments::count(&segment(&grid, 3, SIDE, 40.0, WEIGHT, LEAST));
    let many = segments::count(&segment(&grid, 3, SIDE, 3.0, WEIGHT, 1));
    assert!(many > few, "{many} {few}");
}

#[test]
fn texture_separates_same_color() {
    // Лес и луг одного среднего цвета: без текстуры - один объект.
    let mut random = Noise(1);
    let mut bands = vec![100.0f32; 3 * 100 * 100];
    for r in 0..100 {
        for c in 50..100 {
            let spot = if random.next() < 0.5 { -60.0 } else { 60.0 };
            for b in 0..3 {
                bands[(r * 100 + c) * 3 + b] += spot;
            }
        }
    }
    let labels = segment(&bands, 3, 100, 25.0, 0.8, LEAST);
    assert_ne!(labels[50 * 100 + 20], labels[50 * 100 + 80]);
}

#[test]
fn least_removes_small_objects() {
    let mut grid = scene(4.0);
    for r in 100..102 {
        for c in 10..12 {
            for b in 0..3 {
                grid[(r * SIDE + c) * 3 + b] = 250.0;
            }
        }
    }
    let kept = segment(&grid, 3, SIDE, 1.0, WEIGHT, 1);
    let gone = segment(&grid, 3, SIDE, 1.0, WEIGHT, 16);
    assert!(segments::areas(&kept).into_iter().min() < Some(16));
    assert!(segments::areas(&gone).into_iter().min() >= Some(16));
}

#[test]
fn no_data_and_outside_have_no_object() {
    let mut grid = scene(4.0);
    for r in 10..20 {
        for c in 10..20 {
            grid[(r * SIDE + c) * 3 + 1] = f32::NAN;
        }
    }
    let inside: Vec<bool> = (0..SIDE * SIDE).map(|i| i / SIDE >= 5).collect();
    let labels = segments::segment(&grid, 3, SIDE, SIDE, Some(&inside), Params { scale: 25.0, weight: WEIGHT, least: LEAST, shape: SHAPE });
    assert_eq!(labels[15 * SIDE + 15], -1);
    assert!(labels[..5 * SIDE].iter().all(|l| *l == -1));
    assert!((30..SIDE).all(|r| (30..SIDE).all(|c| labels[r * SIDE + c] >= 0)));
}

#[test]
fn flat_raster_is_one_object() {
    let labels = segment(&vec![7.0; 2 * 30 * 30], 2, 30, segments::SCALE as f64, WEIGHT, LEAST);
    assert!(labels.iter().all(|l| *l == 0));
}

#[test]
fn nothing_usable() {
    assert!(segment(&vec![f32::NAN; 2 * 8 * 8], 2, 8, 15.0, WEIGHT, LEAST).iter().all(|l| *l == -1));
}

#[test]
fn cut_goes_by_passes() {
    let grid = scene(4.0);
    let mut cut = segments::Cut::new(&grid, 3, SIDE, SIDE, None, Params { scale: 25.0, weight: WEIGHT, least: LEAST, shape: SHAPE });
    let at_start = cut.edges();
    let mut passes = 0;
    while !cut.step() {
        passes += 1;
    }
    assert!(passes > 1, "{passes}");
    assert!(cut.edges() < at_start);
    assert_eq!(cut.finish(), segment(&grid, 3, SIDE, 25.0, WEIGHT, LEAST));
}

#[test]
fn embedding_bands_reduce_and_find_parts() {
    // 64 канала: две части сцены различаются смесью всех каналов.
    let mut random = Noise(7);
    let mix: Vec<f32> = (0..64).map(|_| random.normal()).collect();
    let mut grid = Vec::with_capacity(64 * 80 * 80);
    for _ in 0..80 {
        for c in 0..80 {
            let side = if c < 40 { 1.0 } else { -1.0 };
            grid.extend(mix.iter().map(|m| m * side + 0.3 * random.normal()));
        }
    }
    let good = vec![true; 80 * 80];
    let data = segments::features(&grid, 64, &good, 80, 80, 0.0).unwrap_or_default();
    assert_eq!(data.len(), segments::PARTS);
    let var = |row: &Vec<f32>| {
        let n = row.len() as f64;
        let mean = row.iter().map(|v| f64::from(*v)).sum::<f64>() / n;
        row.iter().map(|v| (f64::from(*v) - mean).powi(2)).sum::<f64>() / n
    };
    assert!((data.iter().map(var).sum::<f64>() - 1.0).abs() < 0.05);
    let labels = segment(&grid, 64, 80, 15.0, 0.0, LEAST);
    assert_ne!(labels[40 * 80 + 15], labels[40 * 80 + 65]);
}

#[test]
fn neighbors_of_one_class_merge() {
    let labels = [0, 0, 1, 1, 2, 0, 0, 1, 1, 2, 3, 3, 3, -1, 2];
    let (out, joined) = segments::dissolve(&labels, 3, 5, &[0, 0, 1, 0]);
    // 0, 1 и 3 связаны, 2 - другой класс.
    assert_eq!(segments::count(&out), 2);
    assert_eq!(out[0], out[2]);
    assert_eq!(out[0], out[10]);
    assert_ne!(out[0], out[4]);
    assert_eq!(out[13], -1);
    let mut sorted = joined.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, [0, 1]);
}

#[test]
fn far_objects_of_one_class_stay_apart() {
    let (out, joined) = segments::dissolve(&[0, 1, 2], 1, 3, &[0, 1, 0]);
    assert_eq!(segments::count(&out), 3);
    assert_eq!(joined, [0, 1, 0]);
}

#[test]
fn no_class_does_not_merge() {
    let (out, _) = segments::dissolve(&[0, 1], 1, 2, &[-1, -1]);
    assert_eq!(segments::count(&out), 2);
}

#[test]
fn long_chain_joins() {
    let labels: Vec<i32> = (0..500).rev().collect();
    let (out, joined) = segments::dissolve(&labels, 1, 500, &[0; 500]);
    assert!(out.iter().all(|l| *l == 0));
    assert_eq!(joined, [0]);
}

#[test]
fn each_region_in_one_pair_on_equal_costs() {
    let (a, b) = ([0, 1, 2, 3], [1, 2, 3, 4]);
    let chosen = segments::pairs(&a, &b, &[0.0; 4], 5);
    let used: Vec<usize> = chosen.iter().flat_map(|e| [a[*e], b[*e]]).collect();
    let unique: std::collections::HashSet<usize> = used.iter().copied().collect();
    assert_eq!(used.len(), unique.len());
    assert!(!chosen.is_empty());
}

#[test]
fn mutual_best_only() {
    // Область 1 ближе к 2, поэтому пара 0-1 ждёт.
    assert_eq!(segments::pairs(&[0, 1], &[1, 2], &[5.0, 1.0], 3), [1]);
}

#[test]
fn box_sum_matches_direct() {
    let mut random = Noise(2);
    let value: Vec<f64> = (0..99).map(|_| f64::from(random.normal())).collect();
    let got = segments::box_sum(&value, 9, 11, 2);
    let direct = |rows: std::ops::Range<usize>, cols: std::ops::Range<usize>| rows.flat_map(|r| cols.clone().map(move |c| (r, c))).map(|(r, c)| value[r * 11 + c]).sum::<f64>();
    assert!((got[4 * 11 + 5] - direct(2..7, 3..8)).abs() < 1e-9);
    assert!((got[0] - direct(0..3, 0..3)).abs() < 1e-9);
    assert!((got[8 * 11 + 10] - direct(6..9, 8..11)).abs() < 1e-9);
}

#[test]
fn flat_has_no_texture() {
    let out = segments::texture(&[3.0; 36], &[true; 36], 6, 6, 2);
    assert!(out.iter().all(|v| *v < 1e-6));
}

#[test]
fn masked_cells_do_not_count() {
    // Яркий выброс вне участка не делает клетку шероховатой.
    let mut value = [1.0; 49];
    value[24] = 1e6;
    let mut good = [true; 49];
    good[24] = false;
    assert!(segments::texture(&value, &good, 7, 7, 2).iter().all(|v| *v < 1e-6));
}

#[test]
fn object_table_and_classes() {
    let grid = scene(4.0);
    let labels = segment(&grid, 3, SIDE, 25.0, WEIGHT, LEAST);
    let table = segments::table(&grid, 3, &labels);
    assert_eq!(table.len(), 6 * segments::count(&labels));
    let object = |r: usize, c: usize| labels[r * SIDE + c] as usize;
    let row = |n: usize| table[n * 6..(n + 1) * 6].to_vec();
    let (west, east, pond) = (object(60, 20), object(90, 90), object(45, 95));
    assert!((row(west)[0] - 40.0).abs() < 2.0);
    assert!(row(west)[3] < 10.0);
    let (kinds, _) = rasterclass::classify(&table, 6, &[row(west), row(east), row(pond)], &[0, 1, 2], None, 3);
    assert_eq!((kinds[west], kinds[east], kinds[pond]), (0, 1, 2));
}

#[test]
fn borders_of_objects() {
    let labels = [0, 0, 1, 0, 0, 1, -1, -1, 1];
    let edge = segments::borders(&labels, 3, 3);
    assert!(edge[1]);
    assert!(!edge[0]);
    assert!(!edge[6]);
}

#[test]
fn site_scale_keeps_share_of_site() {
    // Та же сцена на сетке вчетверо грубее: объекты те же.
    let grid = scene(2.0);
    let fine = segment(&grid, 3, SIDE, segments::site_scale(600.0, SIDE * SIDE), WEIGHT, 4);
    let side = SIDE / 4;
    let coarse: Vec<f32> = (0..side).flat_map(|r| (0..side).map(move |c| (r, c))).flat_map(|(r, c)| grid[(r * 4 * SIDE + c * 4) * 3..(r * 4 * SIDE + c * 4) * 3 + 3].to_vec()).collect();
    let rough = segment(&coarse, 3, side, segments::site_scale(600.0, side * side), WEIGHT, 4);
    assert_eq!(segments::count(&fine), segments::count(&rough));
}

#[test]
fn too_big_grid() {
    assert!(!segments::too_big(800, 800));
    assert!(segments::too_big(3000, 3000));
}

/// Площадь колец со знаком: внешнее минус дыры.
fn area(rings: &[Vec<[f64; 2]>]) -> f64 {
    let one = |ring: &Vec<[f64; 2]>| {
        let n = ring.len();
        ((0..n).map(|i| ring[i][0] * ring[(i + 1) % n][1] - ring[(i + 1) % n][0] * ring[i][1]).sum::<f64>() / 2.0).abs()
    };
    rings.iter().enumerate().map(|(n, r)| if n == 0 { one(r) } else { -one(r) }).sum()
}

#[test]
fn outline_of_square_has_four_corners() {
    let rings = segments::outlines(&[0; 9], 3, 3);
    assert_eq!(rings.len(), 1);
    assert_eq!(rings[0].len(), 1);
    assert_eq!(rings[0][0].len(), 4);
    assert_eq!(area(&rings[0]), 9.0);
}

#[test]
fn outline_with_hole_keeps_area() {
    // Объект 0 с объектом 1 внутри и клеткой без данных в дальнем углу.
    let labels = [0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, -1];
    let rings = segments::outlines(&labels, 5, 5);
    assert_eq!(rings[0].len(), 2);
    assert_eq!(area(&rings[0]), 20.0);
    assert_eq!(area(&rings[1]), 4.0);
    // Первое кольцо - внешнее.
    assert_eq!(rings[1].len(), 1);
    assert!(rings[0][0].contains(&[0.0, 0.0]));
}

#[test]
fn outlines_cover_every_cell() {
    // На сцене площади всех объектов совпадают с клетками, как у модуля
    // сошлись площади слоя и клеток.
    let labels = segment(&scene(4.0), 3, SIDE, 25.0, WEIGHT, LEAST);
    let sizes = segments::areas(&labels);
    for (rings, size) in segments::outlines(&labels, SIDE, SIDE).iter().zip(sizes) {
        assert_eq!(area(rings) as u64, size);
    }
}

#[test]
fn outline_touching_at_corner() {
    // Дыра объекта 0 касается угла клетки без данных: кольцо одно,
    // оно проходит узел дважды, площадь верная.
    let labels = [-1, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let rings = segments::outlines(&labels, 5, 5);
    assert_eq!(rings[0].len(), 1);
    assert_eq!(area(&rings[0]), 20.0);
    assert_eq!(area(&rings[1]), 4.0);
}
