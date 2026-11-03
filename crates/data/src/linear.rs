// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Линейные объекты участка осевыми линиями, перенос `core/linear.py`
//! модуля 1.33.
//!
//! Дороги, просеки, реки и лесополосы - линиями, а не площадями. Порядок:
//! - маска вида (почва, вода, растительность) на сетке участка;
//! - тонкие части маски - то, что убирает морфологическое открытие
//!   квадратом ([`thin`]): узкая полоса уходит, поле остаётся;
//! - скелет тонких частей - утончение Чжана-Суэня 1984 года ([`skeleton`]);
//! - цепочки скелета от узла до узла ([`chains`]), связный кусок скелета
//!   короче наименьшей длины отбрасывается, короткие отростки - тоже;
//! - ломаные упрощаются Дугласом-Пекером ([`crate::tracing::simplify`]).
//!
//! Маски - клетки подряд по строкам. Координаты линий - середины клеток:
//! x - столбец, y - строка сверху.

use std::collections::{BTreeMap, BTreeSet};

use crate::tracing;

/// Линейный объект не шире, метры - выбор помощника модуля.
pub const WIDTH: f64 = 40.0;
/// Связный кусок линии не короче, метры.
pub const LENGTH: f64 = 200.0;
/// Отросток короче SPUR ширин линии отбрасывается.
pub const SPUR: f64 = 3.0;
/// Соседи P2..P9 по часовой стрелке от верхнего.
const STEPS: [(i64, i64); 8] = [(-1, 0), (-1, 1), (0, 1), (1, 1), (1, 0), (1, -1), (0, -1), (-1, -1)];

type Cell = (usize, usize);

/// Значение соседа (r + dr, c + dc), за краем - ложь.
fn near(mask: &[bool], rows: usize, cols: usize, r: usize, c: usize, (dr, dc): (i64, i64)) -> bool {
    let (y, x) = (r as i64 + dr, c as i64 + dc);
    y >= 0 && x >= 0 && (y as usize) < rows && (x as usize) < cols && mask[y as usize * cols + x as usize]
}

/// Сжатие квадратом: клетка остаётся, если она и все восемь соседей - да.
pub fn erode(mask: &[bool], rows: usize, cols: usize, radius: usize) -> Vec<bool> {
    let mut out = mask.to_vec();
    for _ in 0..radius {
        let prev = out.clone();
        for r in 0..rows {
            for c in 0..cols {
                let k = r * cols + c;
                out[k] = prev[k] && STEPS.iter().all(|s| near(&prev, rows, cols, r, c, *s));
            }
        }
    }
    out
}

/// Расширение квадратом.
pub fn dilate(mask: &[bool], rows: usize, cols: usize, radius: usize) -> Vec<bool> {
    let mut out = mask.to_vec();
    for _ in 0..radius {
        let prev = out.clone();
        for r in 0..rows {
            for c in 0..cols {
                let k = r * cols + c;
                out[k] = prev[k] || STEPS.iter().any(|s| near(&prev, rows, cols, r, c, *s));
            }
        }
    }
    out
}

/// Части маски уже примерно cells клеток: маска без её открытия квадратом
/// стороны 2r + 1, r = cells / 2.
pub fn thin(mask: &[bool], rows: usize, cols: usize, cells: f64) -> Vec<bool> {
    let radius = ((cells / 2.0).round() as usize).max(1);
    let opened = dilate(&erode(mask, rows, cols, radius), rows, cols, radius);
    mask.iter().zip(opened).map(|(m, o)| *m && !o).collect()
}

/// Утончение Чжана-Суэня до линии в клетку, связность 8.
pub fn skeleton(mask: &[bool], rows: usize, cols: usize) -> Vec<bool> {
    let mut img = mask.to_vec();
    loop {
        let mut changed = false;
        for phase in 0..2 {
            let mut gone = Vec::new();
            for r in 0..rows {
                for c in 0..cols {
                    if !img[r * cols + c] {
                        continue;
                    }
                    let p: Vec<bool> = STEPS.iter().map(|s| near(&img, rows, cols, r, c, *s)).collect();
                    let count = p.iter().filter(|v| **v).count();
                    let turns = (0..8).filter(|n| !p[*n] && p[(n + 1) % 8]).count();
                    let (p2, p4, p6, p8) = (p[0], p[2], p[4], p[6]);
                    let cut = if phase == 0 { !(p2 && p4 && p6) && !(p4 && p6 && p8) } else { !(p2 && p4 && p8) && !(p2 && p6 && p8) };
                    if (2..=6).contains(&count) && turns == 1 && cut {
                        gone.push(r * cols + c);
                    }
                }
            }
            if !gone.is_empty() {
                changed = true;
                for k in gone {
                    img[k] = false;
                }
            }
        }
        if !changed {
            return img;
        }
    }
}

fn neighbours(cells: &BTreeSet<Cell>, (r, c): Cell) -> Vec<Cell> {
    STEPS
        .iter()
        .filter_map(|(dr, dc)| {
            let (y, x) = (r as i64 + dr, c as i64 + dc);
            (y >= 0 && x >= 0).then_some((y as usize, x as usize)).filter(|p| cells.contains(p))
        })
        .collect()
}

/// Цепочки скелета от узла до узла: списки клеток (строка, столбец).
/// Узел - клетка не с двумя соседями, кольцо без узлов - одна цепочка.
pub fn chains(skel: &[bool], rows: usize, cols: usize) -> Vec<Vec<Cell>> {
    let cells: BTreeSet<Cell> = (0..rows * cols).filter(|k| skel[*k]).map(|k| (k / cols, k % cols)).collect();
    let links: BTreeMap<Cell, Vec<Cell>> = cells.iter().map(|c| (*c, neighbours(&cells, *c))).collect();
    let mut nodes: BTreeSet<Cell> = links.iter().filter(|(_, n)| n.len() != 2).map(|(c, _)| *c).collect();
    let mut used: BTreeSet<(Cell, Cell)> = BTreeSet::new();
    let mut out = Vec::new();
    let mut order: Vec<Cell> = nodes.iter().copied().collect();
    order.extend(cells.iter().filter(|c| !nodes.contains(c)));
    for start in order {
        for &next in links.get(&start).map_or(&[][..], Vec::as_slice) {
            if used.contains(&(start, next)) {
                continue;
            }
            let mut path = vec![start, next];
            used.insert((start, next));
            used.insert((next, start));
            loop {
                let here = path[path.len() - 1];
                if nodes.contains(&here) || here == start {
                    break;
                }
                let step = links.get(&here).and_then(|l| l.iter().copied().find(|c| !used.contains(&(here, *c))));
                let Some(step) = step else {
                    break;
                };
                used.insert((here, step));
                used.insert((step, here));
                path.push(step);
            }
            out.push(path);
        }
        // Кольцо обойдено, дальше его клетка - узел.
        nodes.insert(start);
    }
    out
}

fn length(path: &[(f64, f64)]) -> f64 {
    path.windows(2).map(|w| (w[0].0 - w[1].0).hypot(w[0].1 - w[1].1)).sum()
}

fn cell_length(path: &[Cell]) -> f64 {
    path.windows(2).map(|w| (w[0].0 as f64 - w[1].0 as f64).hypot(w[0].1 as f64 - w[1].1 as f64)).sum()
}

/// Связные группы цепочек по общим концам.
fn groups(paths: &[Vec<Cell>]) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..paths.len()).collect();
    fn root(parent: &mut [usize], mut n: usize) -> usize {
        while parent[n] != n {
            parent[n] = parent[parent[n]];
            n = parent[n];
        }
        n
    }
    let mut owner: BTreeMap<Cell, usize> = BTreeMap::new();
    for (n, path) in paths.iter().enumerate() {
        for cell in [path[0], path[path.len() - 1]] {
            match owner.get(&cell) {
                Some(&m) => {
                    let (a, b) = (root(&mut parent, n), root(&mut parent, m));
                    parent[a] = b;
                }
                None => {
                    owner.insert(cell, n);
                }
            }
        }
    }
    let mut out: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for n in 0..paths.len() {
        let r = root(&mut parent, n);
        out.entry(r).or_default().push(n);
    }
    out.into_values().collect()
}

/// Осевые линии тонких частей маски: ломаные (x, y) в клетках (x -
/// столбец, y - строка, середины клеток). Отростки короче
/// [`SPUR`] × width_cells, связные куски короче least_cells
/// отбрасываются.
pub fn lines(mask: &[bool], rows: usize, cols: usize, width_cells: f64, least_cells: f64) -> Vec<Vec<(f64, f64)>> {
    let skel = skeleton(&thin(mask, rows, cols, width_cells), rows, cols);
    if !skel.iter().any(|v| *v) {
        return Vec::new();
    }
    let found = chains(&skel, rows, cols);
    let mut ends: BTreeMap<Cell, usize> = BTreeMap::new();
    for path in &found {
        for end in [path[0], path[path.len() - 1]] {
            *ends.entry(end).or_default() += 1;
        }
    }
    let spur = SPUR * width_cells.max(1.0);
    let end = |c: &Cell| ends.get(c).copied().unwrap_or(0);
    let kept: Vec<Vec<Cell>> = found
        .into_iter()
        .filter(|p| {
            let (a, b) = (end(&p[0]), end(&p[p.len() - 1]));
            !((a == 1 || b == 1) && cell_length(p) < spur && !(a == 1 && b == 1))
        })
        .collect();
    let mut out = Vec::new();
    for group in groups(&kept) {
        if group.iter().map(|n| cell_length(&kept[*n])).sum::<f64>() < least_cells {
            continue;
        }
        for n in group {
            let points: Vec<(f64, f64)> = kept[n].iter().map(|(r, c)| (*c as f64 + 0.5, *r as f64 + 0.5)).collect();
            out.push(tracing::simplify(&points, 1.0, false));
        }
    }
    out
}

/// Клетки сетки под ломаными - для показа линий на глобусе.
pub fn cells_of(paths: &[Vec<(f64, f64)>], rows: usize, cols: usize) -> Vec<bool> {
    let mut out = vec![false; rows * cols];
    for path in paths {
        for w in path.windows(2) {
            let ((x0, y0), (x1, y1)) = (w[0], w[1]);
            let steps = (((x1 - x0).hypot(y1 - y0) * 2.0) as usize + 1).max(2);
            for k in 0..steps {
                let t = k as f64 / (steps - 1) as f64;
                let (x, y) = (x0 + t * (x1 - x0), y0 + t * (y1 - y0));
                if x >= 0.0 && y >= 0.0 && (x as usize) < cols && (y as usize) < rows {
                    out[y as usize * cols + x as usize] = true;
                }
            }
        }
    }
    out
}

/// Длина ломаной в метрах по шагу сетки.
pub fn length_m(points: &[(f64, f64)], step: f64) -> f64 {
    length(points) * step
}
