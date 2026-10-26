// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Заполнение понижений с epsilon. Перенос `core/hydro_fill.py` модуля
//! (из Isoliner, `grid_isolines/hydro_fill.py`, владелец тот же).
//!
//! Модуль решает задачу Planchon-Darboux (2001) направленными проходами
//! NumPy с учётом грязных линий. Здесь - очередь с приоритетом
//! (priority-flood + epsilon, Barnes, Lehman, Mulla 2014): узлы стоков
//! кладутся в очередь, из неё каждый раз берётся самый низкий, его
//! необработанные соседи получают высоту max(z, w + epsilon). Выбор
//! помощника - время O(n log n) без проходов, сходимость не нужна.
//!
//! Ответ тот же, что у модуля, до бита: обе схемы приходят к одной
//! неподвижной точке - у каждого внутреннего узла w = max(z, наименьшее
//! w соседей + epsilon), и наименьший сосед узла из очереди - тот, от
//! которого он получил высоту. Проверка - тесты `tests/hydro_fill.rs`.
//!
//! Epsilon задаёт наименьший уклон на плоских участках, чтобы поток
//! не останавливался. Узлы на краю сетки и узлы, соседние с nodata, -
//! стоки: через них вода уходит с сетки. Свои стоки (`outlets`) заменяют
//! край, узлы у nodata остаются стоками и тогда.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

pub const DEFAULT_EPSILON: f64 = 0.001;
/// Высота узлов, до которых вода от стоков не дошла, как у модуля.
pub const BIG: f64 = 1e30;

/// Итог заполнения.
#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    /// Заполненные высоты, у nodata - исходные значения.
    pub filled: Vec<f64>,
    /// Количество поднятых узлов (подъём больше 1e-9).
    pub raised: usize,
    /// Наибольший подъём, м.
    pub max_raise: f64,
}

/// Узел в очереди: высота и номер, первым выходит низший.
#[derive(Clone, Copy)]
struct Cell {
    w: f64,
    i: usize,
}

impl PartialEq for Cell {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Cell {}

impl PartialOrd for Cell {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Cell {
    fn cmp(&self, other: &Self) -> Ordering {
        // Куча стандартной библиотеки отдаёт наибольший: порядок обратный.
        other.w.total_cmp(&self.w).then(other.i.cmp(&self.i))
    }
}

/// Заполнение понижений сетки z (rows × cols, по строкам).
///
/// nodata - true там, где данных нет, NaN тоже пусто. Значения nodata не
/// меняются и на соседей не влияют. outlets - свои стоки вместо края
/// сетки (уровень перелива от точки, `hydro::spill_levels`). Узлы,
/// до которых вода от стоков не доходит, получают BIG.
///
/// None - сетка меньше 2×2, epsilon отрицателен или не число, маски
/// не той длины.
pub fn fill_depressions(z: &[f64], rows: usize, cols: usize, nodata: Option<&[bool]>, epsilon: f64, outlets: Option<&[bool]>) -> Option<Fill> {
    let n = rows.checked_mul(cols)?;
    if rows < 2 || cols < 2 || z.len() != n || epsilon.is_nan() || epsilon < 0.0 {
        return None;
    }
    if nodata.is_some_and(|m| m.len() != n) || outlets.is_some_and(|m| m.len() != n) {
        return None;
    }
    let empty: Vec<bool> = (0..n).map(|i| !z[i].is_finite() || nodata.is_some_and(|m| m[i])).collect();
    let neighbours = |i: usize| {
        let (r, c) = ((i / cols) as i64, (i % cols) as i64);
        [(-1i64, -1i64), (-1, 0), (-1, 1), (0, -1), (0, 1), (1, -1), (1, 0), (1, 1)].into_iter().filter_map(move |(dr, dc)| {
            let (rr, cc) = (r + dr, c + dc);
            (rr >= 0 && cc >= 0 && rr < rows as i64 && cc < cols as i64).then(|| rr as usize * cols + cc as usize)
        })
    };
    let mut w = vec![BIG; n];
    let mut done = vec![false; n];
    let mut heap = BinaryHeap::new();
    for i in 0..n {
        if empty[i] {
            done[i] = true;
            continue;
        }
        let (r, c) = (i / cols, i % cols);
        let edge = r == 0 || c == 0 || r == rows - 1 || c == cols - 1;
        let own = match outlets {
            Some(m) => m[i],
            None => edge,
        };
        if own || neighbours(i).any(|j| empty[j]) {
            w[i] = z[i];
            done[i] = true;
            heap.push(Cell { w: z[i], i });
        }
    }
    while let Some(Cell { w: level, i }) = heap.pop() {
        for j in neighbours(i) {
            if done[j] {
                continue;
            }
            done[j] = true;
            let cand = level + epsilon;
            w[j] = if z[j] >= cand { z[j] } else { cand };
            heap.push(Cell { w: w[j], i: j });
        }
    }
    let mut raised = 0;
    let mut max_raise = 0.0f64;
    let filled: Vec<f64> = (0..n)
        .map(|i| {
            if empty[i] {
                return z[i];
            }
            let diff = w[i] - z[i];
            if diff > 1e-9 {
                raised += 1;
                max_raise = max_raise.max(diff);
            }
            w[i]
        })
        .collect();
    Some(Fill { filled, raised, max_raise })
}
