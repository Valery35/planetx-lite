// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Трассировка участка снимка в многоугольники по цветам, перенос
//! `core/tracing.py` модуля 1.32.
//!
//! Просьба автора модуля от 3 ноября 2026 года по образцу «Векторизации
//! растра» Inkscape - снимок раскладывается на цвета, области обводятся
//! гладкими контурами методом Potrace (Selinger 2003).
//!
//! Порядок:
//! - сглаживание снимка окном 3 × 3 ([`blur`]) - меньше шума на границах;
//! - цвета k-средними по каналам, приведённым к разбросу 1 ([`quantize`]);
//! - связные области одного цвета ([`regions`]), области меньше least
//!   клеток сливаются с соседом по самой длинной общей границе
//!   ([`merge_small`]);
//! - границы клеток между областями - цепочки между узлами, где сходятся
//!   три области и больше ([`chains`]). У соседних областей цепочка общая
//!   и сглаживается один раз, поэтому щелей и наложений между
//!   многоугольниками нет;
//! - цепочка упрощается Дугласом-Пекером и сглаживается кривыми Безье по
//!   правилу углов Potrace: вершина с alpha не ниже corner остаётся углом
//!   ([`smooth_chain`]);
//! - кольца областей собираются из цепочек ([`rings`]).
//!
//! Сетка - клетки подряд, у клетки depth каналов подряд, как у
//! `rasterclass`. Координаты - углы клеток: x - столбец, y - строка сверху
//! вниз. Случайность - [`crate::flow::Rng`], выбор цветов не совпадает
//! с numpy модуля, но повторяется при том же зерне.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::flow::Rng;
use crate::rasterclass;
use crate::segments;

/// Цветов по умолчанию.
pub const COLORS: usize = 8;
/// Проходов сглаживания снимка.
pub const SMOOTH: usize = 1;
/// Клеток в наименьшей области.
pub const LEAST: usize = 25;
/// alphamax Potrace: выше - круглее, 0 - одни углы.
pub const CORNER: f64 = 1.0;
/// Допуск упрощения ломаной по границе клеток, клетки.
pub const TOLERANCE: f64 = 0.75;
/// Клеток для поиска цветов.
pub const SAMPLE: usize = 100_000;
/// Проходов k-средних.
const ROUNDS: usize = 25;
/// Клетки между точками кривой.
const CURVE_STEP: f64 = 1.0;
/// Точек на кривую не больше.
const CURVE_POINTS: usize = 16;
/// Проходов слияния мелких областей.
const MERGE_ROUNDS: usize = 30;
/// Клеток за шаг отнесения к цветам и цепочек за шаг сглаживания.
const CELLS_STEP: usize = 262_144;
const CHAINS_STEP: usize = 2000;

/// Точка - угол клетки (x, y).
pub type Point = (f64, f64);
/// Многоугольник области: наружное кольцо и дыры, у колец первая точка
/// повторена в конце.
pub type Shape = (Vec<Point>, Vec<Vec<Point>>);

/// Среднее окна 3 × 3 по клеткам good, passes раз. Клетки вне good
/// в среднее не входят и остаются прежними.
pub fn blur(grid: &[f32], depth: usize, good: &[bool], rows: usize, cols: usize, passes: usize) -> Vec<f32> {
    let depth = depth.max(1);
    if passes == 0 || rows * cols == 0 {
        return grid.to_vec();
    }
    let mut out: Vec<f64> = grid.iter().map(|v| f64::from(*v)).collect();
    let weight: Vec<f64> = good.iter().map(|g| if *g { 1.0 } else { 0.0 }).collect();
    let count = box3(&weight, rows, cols);
    let mut plane = vec![0.0; rows * cols];
    for _ in 0..passes {
        for band in 0..depth {
            for (i, slot) in plane.iter_mut().enumerate() {
                *slot = if good.get(i).copied().unwrap_or(false) { out.get(i * depth + band).copied().unwrap_or(0.0) } else { 0.0 };
            }
            let total = box3(&plane, rows, cols);
            for i in 0..rows * cols {
                if good.get(i).copied().unwrap_or(false)
                    && let Some(slot) = out.get_mut(i * depth + band)
                {
                    *slot = total[i] / count[i].max(1.0);
                }
            }
        }
    }
    out.iter().map(|v| *v as f32).collect()
}

/// Сумма окна 3 × 3, за краем - нули.
fn box3(values: &[f64], rows: usize, cols: usize) -> Vec<f64> {
    let mut out = vec![0.0; rows * cols];
    for r in 0..rows {
        for c in 0..cols {
            let mut total = 0.0;
            for rr in r.saturating_sub(1)..(r + 2).min(rows) {
                for cc in c.saturating_sub(1)..(c + 2).min(cols) {
                    total += values[rr * cols + cc];
                }
            }
            out[r * cols + c] = total;
        }
    }
    out
}

/// Каналы клеток good, приведённые к разбросу 1: (номера клеток,
/// значения подряд).
fn scaled(grid: &[f32], depth: usize, good: &[bool]) -> (Vec<usize>, Vec<f32>) {
    let (mean, spread) = rasterclass::scales(grid, depth, Some(good));
    let spread: Vec<f64> = spread.iter().map(|s| if *s > 0.0 { *s } else { 1.0 }).collect();
    let cells: Vec<usize> = good.iter().enumerate().filter(|(i, g)| **g && grid.get(i * depth..(i + 1) * depth).is_some()).map(|(i, _)| i).collect();
    let mut data = Vec::with_capacity(cells.len() * depth);
    for &i in &cells {
        for band in 0..depth {
            data.push(((f64::from(grid[i * depth + band]) - mean[band]) / spread[band]) as f32);
        }
    }
    (cells, data)
}

fn distance(a: &[f32], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (f64::from(*x) - y).powi(2)).sum()
}

/// Ближайший цвет точки.
fn nearest(point: &[f32], centers: &[Vec<f64>]) -> usize {
    let mut best = (f64::INFINITY, 0);
    for (n, center) in centers.iter().enumerate() {
        let d = distance(point, center);
        if d < best.0 {
            best = (d, n);
        }
    }
    best.1
}

/// Начальные цвета k-means++: каждый следующий - с вероятностью по
/// квадрату расстояния до ближайшего прежнего.
fn seeds(data: &[f32], depth: usize, count: usize, rng: &mut Rng) -> Vec<Vec<f64>> {
    let points = data.len() / depth;
    if points == 0 || count == 0 {
        return Vec::new();
    }
    let point = |n: usize| -> Vec<f64> { data[n * depth..(n + 1) * depth].iter().map(|v| f64::from(*v)).collect() };
    let first = ((rng.unit() * points as f64) as usize).min(points - 1);
    let mut centers = vec![point(first)];
    let mut far: Vec<f64> = (0..points).map(|n| distance(&data[n * depth..(n + 1) * depth], &centers[0])).collect();
    for _ in 1..count {
        let total: f64 = far.iter().sum();
        if total <= 0.0 {
            break;
        }
        let target = rng.unit() * total;
        let mut sum = 0.0;
        let mut choice = points - 1;
        for (n, f) in far.iter().enumerate() {
            sum += f;
            if sum >= target {
                choice = n;
                break;
            }
        }
        let center = point(choice);
        for (n, f) in far.iter_mut().enumerate() {
            *f = f.min(distance(&data[n * depth..(n + 1) * depth], &center));
        }
        centers.push(center);
    }
    centers
}

/// Цвета k-средними по выборке до [`SAMPLE`] клеток: центры цветов
/// в каналах с разбросом 1.
fn centers_of(data: &[f32], depth: usize, count: usize, seed: u64) -> Vec<Vec<f64>> {
    let points = data.len() / depth;
    let mut rng = Rng::new(seed);
    let pick: Vec<f32> = if points <= SAMPLE {
        data.to_vec()
    } else {
        // Выборка без повторов - частичная перестановка Фишера-Йетса.
        let mut order: Vec<usize> = (0..points).collect();
        for k in 0..SAMPLE {
            let j = k + ((rng.unit() * (points - k) as f64) as usize).min(points - k - 1);
            order.swap(k, j);
        }
        order[..SAMPLE].iter().flat_map(|n| data[n * depth..(n + 1) * depth].iter().copied()).collect()
    };
    let picked = pick.len() / depth;
    let mut centers = seeds(&pick, depth, count.min(picked), &mut rng);
    for _ in 0..ROUNDS {
        let mut sums = vec![vec![0.0; depth]; centers.len()];
        let mut sizes = vec![0usize; centers.len()];
        for point in pick.chunks(depth) {
            let n = nearest(point, &centers);
            sizes[n] += 1;
            for (s, v) in sums[n].iter_mut().zip(point) {
                *s += f64::from(*v);
            }
        }
        let moved: Vec<Vec<f64>> = centers.iter().enumerate().map(|(n, old)| if sizes[n] > 0 { sums[n].iter().map(|s| s / sizes[n] as f64).collect() } else { old.clone() }).collect();
        // Как np.allclose: |a - b| <= 1e-8 + 1e-5 |b|.
        let still = moved.iter().zip(&centers).all(|(a, b)| a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1e-8 + 1e-5 * y.abs()));
        if still {
            break;
        }
        centers = moved;
    }
    centers
}

/// Цвета клеток: (номера цветов клеток, -1 вне good, средние значения
/// каналов цветов). Каналы приводятся к разбросу 1, цвета ищутся
/// k-средними по выборке до [`SAMPLE`] клеток.
pub fn quantize(grid: &[f32], depth: usize, good: &[bool], count: usize, seed: u64) -> (Vec<i32>, Vec<Vec<f64>>) {
    let depth = depth.max(1);
    let (cells, data) = scaled(grid, depth, good);
    let mut labels = vec![-1i32; good.len()];
    if cells.is_empty() {
        return (labels, Vec::new());
    }
    let centers = centers_of(&data, depth, count.max(1), seed);
    let near: Vec<usize> = data.chunks(depth).map(|p| nearest(p, &centers)).collect();
    let centers = relabel(grid, depth, &cells, &near, centers.len(), &mut labels);
    (labels, centers)
}

/// Номера только встреченных цветов подряд и средние исходных каналов
/// цветов. labels получает номера клеток cells.
fn relabel(grid: &[f32], depth: usize, cells: &[usize], near: &[usize], count: usize, labels: &mut [i32]) -> Vec<Vec<f64>> {
    let mut sums = vec![vec![0.0; depth]; count];
    let mut sizes = vec![0usize; count];
    for (&cell, &n) in cells.iter().zip(near) {
        sizes[n] += 1;
        for (band, s) in sums[n].iter_mut().enumerate() {
            *s += f64::from(grid[cell * depth + band]);
        }
    }
    let mut number = vec![-1i32; count];
    let mut out = Vec::new();
    for n in 0..count {
        if sizes[n] > 0 {
            number[n] = out.len() as i32;
            out.push(sums[n].iter().map(|s| s / sizes[n] as f64).collect());
        }
    }
    for (&cell, &n) in cells.iter().zip(near) {
        labels[cell] = number[n];
    }
    out
}

/// Связные (по сторонам клеток) области одного цвета: номера областей
/// клеток, -1 вне участка, и цвет каждой области.
pub fn regions(colors: &[i32], rows: usize, cols: usize) -> (Vec<i32>, Vec<i16>) {
    let mut cells = vec![-1i32; colors.len()];
    let mut kinds = Vec::new();
    for (slot, color) in cells.iter_mut().zip(colors) {
        if *color >= 0 {
            *slot = kinds.len() as i32;
            kinds.push(i16::try_from(*color).unwrap_or(i16::MAX));
        }
    }
    if kinds.is_empty() {
        return (cells, Vec::new());
    }
    segments::dissolve(&cells, rows, cols, &kinds)
}

/// Один проход слияния мелких областей. None - сливать больше нечего,
/// colors не тронуты.
fn merge_round(colors: &mut [i32], rows: usize, cols: usize, least: usize) -> Option<()> {
    let (parts, kinds) = regions(colors, rows, cols);
    let count = kinds.len();
    if count == 0 || least <= 1 {
        return None;
    }
    let mut sizes = vec![0usize; count];
    for p in &parts {
        if let Ok(p) = usize::try_from(*p) {
            sizes[p] += 1;
        }
    }
    let small: Vec<bool> = sizes.iter().map(|s| *s < least).collect();
    if !small.iter().any(|s| *s) {
        return None;
    }
    // Длина общей границы мелкой области a с соседом b.
    let mut shared: HashMap<(usize, usize), usize> = HashMap::new();
    let mut touch = |a: i32, b: i32| {
        if let (Ok(a), Ok(b)) = (usize::try_from(a), usize::try_from(b))
            && a != b
            && small[a]
        {
            *shared.entry((a, b)).or_insert(0) += 1;
        }
    };
    for r in 0..rows {
        for c in 0..cols {
            let at = r * cols + c;
            if c + 1 < cols {
                touch(parts[at], parts[at + 1]);
                touch(parts[at + 1], parts[at]);
            }
            if r + 1 < rows {
                touch(parts[at], parts[at + cols]);
                touch(parts[at + cols], parts[at]);
            }
        }
    }
    let biggest = sizes.iter().copied().max().unwrap_or(0);
    // Мелкий сосед берёт цвет только у мелкого с большим номером: два
    // мелких не меняются цветами по кругу. Крупный сосед главнее мелкого,
    // при равенстве - сосед с меньшим номером.
    let mut target: Vec<usize> = (0..count).collect();
    let mut best: Vec<Option<(usize, usize)>> = vec![None; count];
    for (&(a, b), &length) in &shared {
        if small[b] && b <= a {
            continue;
        }
        let score = length + if small[b] { 0 } else { biggest + 1 };
        let better = match best[a] {
            None => true,
            Some((s, other)) => score > s || (score == s && b < other),
        };
        if better {
            best[a] = Some((score, b));
        }
    }
    for (a, found) in best.iter().enumerate() {
        if let Some((_, b)) = found {
            target[a] = *b;
        }
    }
    let new: Vec<i16> = target.iter().map(|t| kinds[*t]).collect();
    if new == kinds {
        return None;
    }
    for (color, part) in colors.iter_mut().zip(&parts) {
        if let Ok(p) = usize::try_from(*part) {
            *color = i32::from(new[p]);
        }
    }
    Some(())
}

/// Области меньше least клеток перекрашиваются в цвет соседа по самой
/// длинной общей границе, крупный сосед главнее мелкого. Итог - (номера
/// областей, цвета областей, цвета клеток). Мелкая область без соседей
/// на участке остаётся.
pub fn merge_small(colors: &[i32], rows: usize, cols: usize, least: usize) -> (Vec<i32>, Vec<i16>, Vec<i32>) {
    let mut colors = colors.to_vec();
    for _ in 0..MERGE_ROUNDS {
        if merge_round(&mut colors, rows, cols, least).is_none() {
            break;
        }
    }
    let (parts, kinds) = regions(&colors, rows, cols);
    (parts, kinds, colors)
}

/// Цепочка границы клеток между областями. Направление - от начала
/// к концу, «слева» - на экране со строками сверху вниз, -1 - вне участка.
#[derive(Debug, Clone, PartialEq)]
pub struct Chain {
    /// Углы клеток (x, y).
    pub points: Vec<Point>,
    pub left: i32,
    pub right: i32,
    pub closed: bool,
    /// Узлы начала и конца, у замкнутой - None.
    pub start: Option<usize>,
    pub end: Option<usize>,
    /// Клетка слева у начала и у конца, номер в сетке с полями.
    pub pix_start: usize,
    pub pix_end: usize,
}

/// Цепочки границ клеток между областями: у соседних областей цепочка
/// общая, узлы - углы, где сходятся не две границы.
pub fn chains(parts: &[i32], rows: usize, cols: usize) -> Vec<Chain> {
    let width = cols + 2;
    let span = cols + 1;
    let padded = |r: usize, c: usize| -> i32 { if r == 0 || c == 0 || r > rows || c > cols { -1 } else { parts.get((r - 1) * cols + c - 1).copied().unwrap_or(-1) } };
    let (mut u, mut v, mut left, mut right, mut lpix, mut rpix) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    // Отвесные трещины: между клетками (r, c - 1) и (r, c), вниз от угла
    // (r, c) к (r + 1, c). Слева при ходе вниз - восточная клетка.
    for r in 0..rows {
        for c in 0..=cols {
            let (east, west) = (padded(r + 1, c + 1), padded(r + 1, c));
            if east != west {
                u.push(r * span + c);
                v.push((r + 1) * span + c);
                left.push(east);
                right.push(west);
                lpix.push((r + 1) * width + c + 1);
                rpix.push((r + 1) * width + c);
            }
        }
    }
    // Ровные трещины: между (r - 1, c) и (r, c), вправо от угла (r, c)
    // к (r, c + 1). Слева при ходе вправо - северная клетка.
    for r in 0..=rows {
        for c in 0..cols {
            let (north, south) = (padded(r, c + 1), padded(r + 1, c + 1));
            if north != south {
                u.push(r * span + c);
                v.push(r * span + c + 1);
                left.push(north);
                right.push(south);
                lpix.push(r * width + c + 1);
                rpix.push((r + 1) * width + c + 1);
            }
        }
    }
    let total = u.len();
    if total == 0 {
        return Vec::new();
    }
    // Рёбра у каждого угла по порядку номеров рёбер.
    let mut ends: Vec<(usize, usize)> = (0..total).map(|e| (u[e], e)).chain((0..total).map(|e| (v[e], e))).collect();
    ends.sort_by_key(|p| p.0);
    let edge: Vec<usize> = ends.iter().map(|p| p.1).collect();
    let mut slot: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    for (at, (corner, _)) in ends.iter().enumerate() {
        slot.entry(*corner).and_modify(|s| s.1 += 1).or_insert((at, 1));
    }
    let mut seen = vec![false; total];
    let mut out = Vec::new();
    let walk = |start: usize, mut e: usize, seen: &mut [bool]| -> (Vec<usize>, usize, bool, usize, bool) {
        let mut points = vec![start];
        let mut cur = start;
        let (first_e, first_fw) = (e, u[e] == start);
        loop {
            seen[e] = true;
            let forward = u[e] == cur;
            let next = if forward { v[e] } else { u[e] };
            points.push(next);
            let (at, deg) = slot.get(&next).copied().unwrap_or((0, 0));
            if deg != 2 || next == start {
                return (points, first_e, first_fw, e, forward);
            }
            let (a, b) = (edge[at], edge[at + 1]);
            e = if a == e { b } else { a };
            cur = next;
        }
    };
    let record = |out: &mut Vec<Chain>, walked: (Vec<usize>, usize, bool, usize, bool), closed: bool| {
        let (points, fe, ffw, le, lfw) = walked;
        out.push(Chain {
            points: points.iter().map(|p| ((p % span) as f64, (p / span) as f64)).collect(),
            left: if ffw { left[fe] } else { right[fe] },
            right: if ffw { right[fe] } else { left[fe] },
            closed,
            start: (!closed).then(|| points.first().copied()).flatten(),
            end: (!closed).then(|| points.last().copied()).flatten(),
            pix_start: if ffw { lpix[fe] } else { rpix[fe] },
            pix_end: if lfw { lpix[le] } else { rpix[le] },
        });
    };
    for (&corner, &(at, deg)) in &slot {
        if deg == 2 {
            continue;
        }
        for &e in &edge[at..at + deg] {
            if !seen[e] {
                let walked = walk(corner, e, &mut seen);
                record(&mut out, walked, false);
            }
        }
    }
    for e in 0..total {
        if !seen[e] {
            let walked = walk(u[e], e, &mut seen);
            record(&mut out, walked, true);
        }
    }
    out
}

/// Ломаная Дугласа-Пекера. У замкнутой первая точка повторена в конце,
/// её место - как у открытой.
pub fn simplify(points: &[Point], tolerance: f64, closed: bool) -> Vec<Point> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    if closed {
        // Замкнутую делит пополам самая дальняя от начала точка.
        let first = points[0];
        let mut far = 0;
        let mut most = 0.0;
        for (n, p) in points.iter().enumerate() {
            let d = (p.0 - first.0).powi(2) + (p.1 - first.1).powi(2);
            if d > most {
                most = d;
                far = n;
            }
        }
        if far == 0 {
            return vec![first];
        }
        let mut one = simplify(&points[..=far], tolerance, false);
        let two = simplify(&points[far..], tolerance, false);
        one.extend_from_slice(two.get(1..).unwrap_or(&[]));
        return one;
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0, points.len() - 1)];
    while let Some((lo, hi)) = stack.pop() {
        if hi - lo < 2 {
            continue;
        }
        let (a, b) = (points[lo], points[hi]);
        let (sx, sy) = (b.0 - a.0, b.1 - a.1);
        let length = sx.hypot(sy);
        let mut best = (-1.0, lo + 1);
        for (k, p) in points.iter().enumerate().take(hi).skip(lo + 1) {
            let d = if length == 0.0 { (p.0 - a.0).hypot(p.1 - a.1) } else { (sx * (p.1 - a.1) - sy * (p.0 - a.0)).abs() / length };
            if d > best.0 {
                best = (d, k);
            }
        }
        if best.0 > tolerance {
            keep[best.1] = true;
            stack.push((lo, best.1));
            stack.push((best.1, hi));
        }
    }
    points.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// alpha вершины по правилу Potrace: отход вершины от прямой соседей,
/// отнесённый к длине L1 этой прямой. 0 - прямая, больше - острее.
pub fn corner_alpha(prev: Point, vertex: Point, next: Point) -> f64 {
    let (dx, dy) = (next.0 - prev.0, next.1 - prev.1);
    let denom = dx.abs() + dy.abs();
    if denom == 0.0 {
        return 4.0 / 3.0;
    }
    let cross = ((vertex.0 - prev.0) * dy - (vertex.1 - prev.1) * dx).abs();
    let dd = cross / denom;
    if dd > 1.0 { (1.0 - 1.0 / dd) / 0.75 } else { 0.0 }
}

fn bezier(p0: Point, p1: Point, p2: Point, p3: Point) -> Vec<Point> {
    let chord = (p3.0 - p0.0).hypot(p3.1 - p0.1) + (p1.0 - p0.0).hypot(p1.1 - p0.1) + (p3.0 - p2.0).hypot(p3.1 - p2.1);
    let count = ((chord / CURVE_STEP) as usize).clamp(2, CURVE_POINTS);
    (1..=count)
        .map(|k| {
            let t = k as f64 / count as f64;
            let s = 1.0 - t;
            let at = |a: f64, b: f64, c: f64, d: f64| s * s * s * a + 3.0 * s * s * t * b + 3.0 * s * t * t * c + t * t * t * d;
            (at(p0.0, p1.0, p2.0, p3.0), at(p0.1, p1.1, p2.1, p3.1))
        })
        .collect()
}

fn mid(a: Point, b: Point) -> Point {
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

/// Путь от середины start до середины end мимо вершины: угол или кривая
/// Безье с контрольными точками к вершине на долю alpha.
fn around(prev: Point, vertex: Point, next: Point, start: Point, end: Point, corner: f64) -> Vec<Point> {
    let alpha = corner_alpha(prev, vertex, next);
    if alpha >= corner {
        return vec![vertex, end];
    }
    let alpha = alpha.clamp(0.55, 1.0);
    let c1 = (start.0 + alpha * (vertex.0 - start.0), start.1 + alpha * (vertex.1 - start.1));
    let c2 = (end.0 + alpha * (vertex.0 - end.0), end.1 + alpha * (vertex.1 - end.1));
    bezier(start, c1, c2, end)
}

/// Гладкая цепочка: упрощение и кривые Безье Potrace. У открытой концы на
/// месте - это узлы, общие с другими цепочками. У замкнутой первая точка
/// повторена в конце.
pub fn smooth_chain(points: &[Point], closed: bool, corner: f64, tolerance: f64) -> Vec<Point> {
    let poly = simplify(points, tolerance, closed);
    if closed {
        let ring = &poly[..poly.len().saturating_sub(1)];
        if ring.len() < 3 {
            return poly;
        }
        let n = ring.len();
        let mids: Vec<Point> = (0..n).map(|i| mid(ring[i], ring[(i + 1) % n])).collect();
        let mut out = vec![mids[n - 1]];
        for j in 0..n {
            let back = (j + n - 1) % n;
            out.extend(around(ring[back], ring[j], ring[(j + 1) % n], mids[back], mids[j], corner));
        }
        return out;
    }
    if poly.len() <= 2 {
        return poly;
    }
    let mids: Vec<Point> = poly.windows(2).map(|w| mid(w[0], w[1])).collect();
    let mut out = vec![poly[0], mids[0]];
    for j in 1..poly.len() - 1 {
        out.extend(around(poly[j - 1], poly[j], poly[j + 1], mids[j - 1], mids[j], corner));
    }
    out.push(poly[poly.len() - 1]);
    out
}

/// Площадь кольца со знаком, в клетках.
pub fn ring_area(ring: &[Point]) -> f64 {
    let n = ring.len();
    0.5 * (0..n).map(|i| ring[i].0 * ring[(i + 1) % n].1 - ring[(i + 1) % n].0 * ring[i].1).sum::<f64>()
}

/// Направленная цепочка: номер и ход от начала к концу.
type Directed = (usize, bool);

/// Многоугольники областей из цепочек found и их гладких точек smooth.
pub fn rings(found: &[Chain], smooth: &[Vec<Point>]) -> BTreeMap<usize, Shape> {
    let mut starts: HashMap<(usize, i32), Vec<Directed>> = HashMap::new();
    for (n, item) in found.iter().enumerate() {
        if item.closed {
            continue;
        }
        for forward in [true, false] {
            let region = if forward { item.left } else { item.right };
            let node = if forward { item.start } else { item.end };
            if let (true, Some(node)) = (region >= 0, node) {
                starts.entry((node, region)).or_default().push((n, forward));
            }
        }
    }
    let mut loops: BTreeMap<usize, Vec<Vec<Point>>> = BTreeMap::new();
    let mut used: HashSet<Directed> = HashSet::new();
    for (n, item) in found.iter().enumerate() {
        let Some(points) = smooth.get(n) else {
            continue;
        };
        for forward in [true, false] {
            let region = if forward { item.left } else { item.right };
            let Ok(key) = usize::try_from(region) else {
                continue;
            };
            if item.closed {
                let pts = if forward { points.clone() } else { points.iter().rev().copied().collect() };
                loops.entry(key).or_default().push(pts);
                continue;
            }
            if used.contains(&(n, forward)) {
                continue;
            }
            if let Some(ring) = assemble(found, smooth, &starts, &mut used, (n, forward), region) {
                loops.entry(key).or_default().push(ring);
            }
        }
    }
    let mut out = BTreeMap::new();
    for (region, mut all) in loops {
        all.sort_by(|a, b| ring_area(b).abs().total_cmp(&ring_area(a).abs()));
        let mut all = all.into_iter();
        if let Some(outer) = all.next() {
            out.insert(region, (outer, all.collect()));
        }
    }
    out
}

/// Кольцо области region из цепочек от направленной цепочки first: на
/// узле следующая - цепочка с той же областью слева, при двух - та, что
/// огибает ту же клетку.
fn assemble(found: &[Chain], smooth: &[Vec<Point>], starts: &HashMap<(usize, i32), Vec<Directed>>, used: &mut HashSet<Directed>, first: Directed, region: i32) -> Option<Vec<Point>> {
    let mut ring: Vec<Point> = Vec::new();
    let mut cur = first;
    for _ in 0..=found.len() {
        used.insert(cur);
        let (k, fw) = cur;
        let pts = smooth.get(k)?;
        let count = pts.len().saturating_sub(1);
        if fw {
            ring.extend_from_slice(&pts[..count]);
        } else {
            ring.extend(pts.iter().rev().take(count));
        }
        let item = found.get(k)?;
        let node = (if fw { item.end } else { item.start })?;
        let pixel = if fw { item.pix_end } else { item.pix_start };
        let options: Vec<Directed> = starts.get(&(node, region)).map(|all| all.iter().filter(|d| !used.contains(d) || **d == first).copied().collect()).unwrap_or_default();
        let mut next = *options.first()?;
        if options.len() > 1 {
            for d in &options {
                let other = found.get(d.0)?;
                if (if d.1 { other.pix_start } else { other.pix_end }) == pixel {
                    next = *d;
                    break;
                }
            }
        }
        if next == first {
            let head = *ring.first()?;
            ring.push(head);
            return Some(ring);
        }
        cur = next;
    }
    None
}

/// Настройки трассировки.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    pub colors: usize,
    pub smooth: usize,
    pub least: usize,
    pub corner: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self { colors: COLORS, smooth: SMOOTH, least: LEAST, corner: CORNER }
    }
}

/// Итог трассировки.
#[derive(Debug, Clone, PartialEq)]
pub struct Traced {
    /// Цвета клеток после слияния, -1 вне участка.
    pub classes: Vec<i32>,
    /// Цвета областей.
    pub kinds: Vec<i16>,
    /// Номера областей клеток.
    pub parts: Vec<i32>,
    /// Средние каналы цветов.
    pub centers: Vec<Vec<f64>>,
    /// Многоугольники областей по номеру области.
    pub shapes: BTreeMap<usize, Shape>,
}

/// Шаг трассировки.
enum Stage {
    Colors,
    Label { cells: Vec<usize>, data: Vec<f32>, centers: Vec<Vec<f64>>, near: Vec<usize> },
    Merge { centers: Vec<Vec<f64>>, round: usize },
    Chains { centers: Vec<Vec<f64>> },
    Smooth { centers: Vec<Vec<f64>>, parts: Vec<i32>, kinds: Vec<i16>, found: Vec<Chain>, smooth: Vec<Vec<Point>> },
    Done(Box<Traced>),
}

/// Трассировка частями за кадры: [`Tracer::step`] делает шаг, итог -
/// [`Tracer::finish`].
pub struct Tracer {
    grid: Vec<f32>,
    depth: usize,
    rows: usize,
    cols: usize,
    good: Vec<bool>,
    params: Params,
    colors: Vec<i32>,
    stage: Stage,
}

impl Tracer {
    /// Трассировка сетки grid (клетки подряд, depth каналов у клетки) по
    /// клеткам good.
    pub fn new(grid: &[f32], depth: usize, rows: usize, cols: usize, good: &[bool], params: Params) -> Self {
        Self { grid: grid.to_vec(), depth: depth.max(1), rows, cols, good: good.to_vec(), params, colors: Vec::new(), stage: Stage::Colors }
    }

    /// Шаг расчёта, true - готово.
    pub fn step(&mut self) -> bool {
        let stage = std::mem::replace(&mut self.stage, Stage::Colors);
        self.stage = match stage {
            Stage::Colors => {
                let values = blur(&self.grid, self.depth, &self.good, self.rows, self.cols, self.params.smooth);
                let (cells, data) = scaled(&values, self.depth, &self.good);
                let centers = centers_of(&data, self.depth, self.params.colors.max(1), 0);
                self.grid = values;
                Stage::Label { cells, data, centers, near: Vec::new() }
            }
            Stage::Label { cells, data, centers, mut near } => {
                let from = near.len();
                let to = (from + CELLS_STEP).min(cells.len());
                near.extend(data[from * self.depth..to * self.depth].chunks(self.depth).map(|p| nearest(p, &centers)));
                if near.len() < cells.len() {
                    Stage::Label { cells, data, centers, near }
                } else {
                    self.colors = vec![-1; self.good.len()];
                    let centers = relabel(&self.grid, self.depth, &cells, &near, centers.len(), &mut self.colors);
                    Stage::Merge { centers, round: 0 }
                }
            }
            Stage::Merge { centers, round } => {
                if round < MERGE_ROUNDS && merge_round(&mut self.colors, self.rows, self.cols, self.params.least).is_some() {
                    Stage::Merge { centers, round: round + 1 }
                } else {
                    Stage::Chains { centers }
                }
            }
            Stage::Chains { centers } => {
                let (parts, kinds) = regions(&self.colors, self.rows, self.cols);
                let found = chains(&parts, self.rows, self.cols);
                Stage::Smooth { centers, parts, kinds, found, smooth: Vec::new() }
            }
            Stage::Smooth { centers, parts, kinds, found, mut smooth } => {
                let from = smooth.len();
                let to = (from + CHAINS_STEP).min(found.len());
                smooth.extend(found[from..to].iter().map(|c| smooth_chain(&c.points, c.closed, self.params.corner, TOLERANCE)));
                if smooth.len() < found.len() {
                    Stage::Smooth { centers, parts, kinds, found, smooth }
                } else {
                    let shapes = rings(&found, &smooth);
                    Stage::Done(Box::new(Traced { classes: std::mem::take(&mut self.colors), kinds, parts, centers, shapes }))
                }
            }
            Stage::Done(done) => Stage::Done(done),
        };
        matches!(self.stage, Stage::Done(_))
    }

    /// Итог, None - расчёт не закончен.
    pub fn finish(self) -> Option<Traced> {
        match self.stage {
            Stage::Done(done) => Some(*done),
            _ => None,
        }
    }
}

/// Трассировка целиком, [`Tracer`] без деления на кадры.
pub fn trace(grid: &[f32], depth: usize, rows: usize, cols: usize, good: &[bool], params: Params) -> Option<Traced> {
    let mut tracer = Tracer::new(grid, depth, rows, cols, good, params);
    while !tracer.step() {}
    tracer.finish()
}

/// Цвета трассировки в классы, `join_colors` модуля 1.33: target[n] -
/// цвет, с которым слит цвет n, сам n или -1 - не слит. Цепочка слияний
/// доходит до цвета, который не слит, у круга слияний класс - наименьший
/// цвет круга. Итог - (lookup, keep): lookup[n] - номер класса цвета n,
/// keep - цвета-классы по порядку.
pub fn join_colors(target: &[i32]) -> (Vec<usize>, Vec<usize>) {
    let count = target.len();
    let next = |m: usize| usize::try_from(target[m]).ok().filter(|t| *t < count && *t != m);
    let mut roots = Vec::with_capacity(count);
    for n in 0..count {
        let mut path = vec![n];
        let mut m = n;
        while let Some(t) = next(m) {
            m = t;
            if let Some(at) = path.iter().position(|p| *p == m) {
                m = path[at..].iter().copied().min().unwrap_or(m);
                break;
            }
            path.push(m);
        }
        roots.push(m);
    }
    let mut keep = roots.clone();
    keep.sort_unstable();
    keep.dedup();
    let lookup = roots.iter().map(|r| keep.binary_search(r).unwrap_or(0)).collect();
    (lookup, keep)
}

/// Цвета клеток и областей после слияния в классы, -1 остаётся,
/// `joined` модуля.
pub fn joined(classes: &[i32], kinds: &[i16], lookup: &[usize]) -> (Vec<i32>, Vec<i16>) {
    let map = |c: i64| usize::try_from(c).ok().and_then(|c| lookup.get(c)).map_or(-1, |k| *k as i64);
    (classes.iter().map(|c| map(i64::from(*c)) as i32).collect(), kinds.iter().map(|k| map(i64::from(*k)) as i16).collect())
}

/// Цвета RGB 0-255 цветов трассировки: первые три канала (один канал -
/// серый). Значения 0-255 берутся как есть, отражение 0-1 и прочие -
/// растяжением по цветам.
pub fn display_colors(centers: &[Vec<f64>]) -> Vec<[u8; 3]> {
    if centers.is_empty() {
        return Vec::new();
    }
    let rgb: Vec<[f64; 3]> = centers
        .iter()
        .map(|c| {
            let at = |k: usize| c.get(k).or_else(|| c.first()).copied().unwrap_or(0.0);
            if c.len() >= 3 { [at(0), at(1), at(2)] } else { [at(0), at(0), at(0)] }
        })
        .collect();
    let all = rgb.iter().flatten();
    let (lowest, highest) = all.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    let stretch = lowest < 0.0 || highest > 255.0 || highest <= 1.5;
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for row in &rgb {
        for k in 0..3 {
            low[k] = low[k].min(row[k]);
            high[k] = high[k].max(row[k]);
        }
    }
    rgb.iter()
        .map(|row| {
            let mut out = [0u8; 3];
            for k in 0..3 {
                let v = if stretch { 255.0 * (row[k] - low[k]) / if high[k] > low[k] { high[k] - low[k] } else { 1.0 } } else { row[k] };
                out[k] = v.round().clamp(0.0, 255.0) as u8;
            }
            out
        })
        .collect()
}
