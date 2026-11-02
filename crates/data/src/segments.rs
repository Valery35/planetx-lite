// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сегментация снимка: участок делится на объекты, похожие по цвету
//! и текстуре, классы получают объекты, а не клетки. Перенос
//! `core/segments.py` модуля.
//!
//! Способ - слияние областей, как многомасштабная сегментация Baatz
//! и Schäpe 2000 года. Сначала каждая клетка - своя область. Цена слияния
//! соседей - прирост разнородности n1 n2 / (n1 + n2) · |m1 - m2|² (мера
//! Уорда) плюс прирост неровности формы. За проход сливаются пары, где
//! каждая область - лучший сосед другой, пока цена не больше порога,
//! порог растёт ступенями до квадрата масштаба. Области соседние, поэтому
//! объект всегда связный.
//!
//! Каналы приводятся к разбросу 1 ([`crate::rasterclass`]) и делятся
//! на корень из их количества: масштаб не зависит от количества каналов.
//! Каналов больше [`PARTS`] (64 эмбеддинга AlphaEarth, съёмка многих
//! каналов) сводятся к главным компонентам. Текстура - местный разброс
//! яркости в окне клетки, ещё один признак с долей weight.
//!
//! Модуль считает в рабочем потоке, у страницы потоков нет. Поэтому расчёт
//! идёт проходами [`Cut::step`], окно делает их за кадры. Сетка - значения
//! клеток по depth подряд, NaN без данных, как у [`crate::rasterclass`].
//! Многоугольники объектов модуль получает из GDAL, здесь их собирает
//! [`outlines`] по рёбрам клеток.

use crate::rasterclass;

/// Масштаб объектов по умолчанию.
pub const SCALE: u32 = 15;
/// Доля текстуры среди признаков.
pub const WEIGHT: f64 = 0.3;
/// Клеток в наименьшем объекте.
pub const LEAST: u32 = 16;
/// Окно текстуры 5 × 5 клеток.
const RADIUS: usize = 2;
/// Вес неровности формы в цене слияния.
pub const SHAPE: f64 = 0.2;
/// Ступеней роста порога, каждая вчетверо ниже следующей.
const STAGES: u32 = 4;
/// Главных компонент у многоканальной сетки.
pub const PARTS: usize = 8;
/// Клеток для оценки компонент. У модуля 200 тысяч, здесь оценка идёт
/// в кадре.
const SAMPLE: usize = 50_000;
/// Каналов в оценке компонент: дальше ковариация не поместилась бы в кадр.
const MOST_BANDS: usize = 256;
/// Клеток участка, к которым отнесён масштаб окна.
const SCALE_CELLS: f64 = 800.0 * 800.0;
/// Клеток участка: рёбер вдвое больше, память.
const MAX_CELLS: u128 = 2000 * 2000;

/// Настройки деления: масштаб расчёта, доля текстуры, наименьший объект,
/// вес формы.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    pub scale: f64,
    pub weight: f64,
    pub least: u32,
    pub shape: f64,
}

/// Масштаб расчёта по масштабу окна: цена слияния растёт с клетками
/// объекта, поэтому при другом пикселе тот же масштаб окна даёт объекты
/// той же доли участка. Иначе на сетке 40 × 40 весь участок сливался
/// в один объект.
pub fn site_scale(scale: f64, cells: usize) -> f64 {
    scale * (cells as f64 / SCALE_CELLS).sqrt()
}

/// Сетка слишком велика для сегментации.
pub fn too_big(rows: usize, cols: usize) -> bool {
    (rows as u128).saturating_mul(cols as u128) > MAX_CELLS
}

/// Клетка с данными во всех каналах.
fn usable(cell: &[f32]) -> bool {
    cell.iter().all(|v| v.is_finite())
}

/// Сумма по окну (2 radius + 1)² с обрезкой у краёв сетки.
pub fn box_sum(values: &[f64], rows: usize, cols: usize, radius: usize) -> Vec<f64> {
    let mut total = vec![0.0; (rows + 1) * (cols + 1)];
    for r in 0..rows {
        let mut line = 0.0;
        for c in 0..cols {
            line += values.get(r * cols + c).copied().unwrap_or(0.0);
            total[(r + 1) * (cols + 1) + c + 1] = total[r * (cols + 1) + c + 1] + line;
        }
    }
    let at = |r: usize, c: usize| total[r * (cols + 1) + c];
    let mut out = Vec::with_capacity(rows * cols);
    for r in 0..rows {
        let (top, bottom) = (r.saturating_sub(radius), (r + radius + 1).min(rows));
        for c in 0..cols {
            let (left, right) = (c.saturating_sub(radius), (c + radius + 1).min(cols));
            out.push(at(bottom, right) - at(top, right) - at(bottom, left) + at(top, left));
        }
    }
    out
}

/// Местный разброс яркости bright по клеткам good в окне клетки.
pub fn texture(bright: &[f64], good: &[bool], rows: usize, cols: usize, radius: usize) -> Vec<f64> {
    let value: Vec<f64> = bright.iter().zip(good).map(|(v, g)| if *g { *v } else { 0.0 }).collect();
    let square: Vec<f64> = value.iter().map(|v| v * v).collect();
    let weight: Vec<f64> = good.iter().map(|g| f64::from(u8::from(*g))).collect();
    let count = box_sum(&weight, rows, cols, radius);
    let first = box_sum(&value, rows, cols, radius);
    let second = box_sum(&square, rows, cols, radius);
    count
        .iter()
        .zip(first.iter().zip(&second))
        .map(|(n, (f, s))| {
            let n = n.max(1.0);
            let mean = f / n;
            (s / n - mean * mean).max(0.0).sqrt()
        })
        .collect()
}

/// Собственные числа и векторы симметричной матрицы n × n, вращения Якоби.
/// Векторы - столбцы: vectors[i * n + k] - строка i вектора k.
fn eigen(matrix: &[f64], n: usize) -> (Vec<f64>, Vec<f64>) {
    let mut a = matrix.to_vec();
    let mut v = vec![0.0; n * n];
    for i in 0..n {
        v[i * n + i] = 1.0;
    }
    for _ in 0..32 {
        let off: f64 = (0..n).flat_map(|p| (p + 1..n).map(move |q| (p, q))).map(|(p, q)| a[p * n + q] * a[p * n + q]).sum();
        let whole: f64 = a.iter().map(|x| x * x).sum();
        if off <= 1e-20 * whole.max(1e-300) {
            break;
        }
        for p in 0..n {
            for q in p + 1..n {
                let pq = a[p * n + q];
                if pq == 0.0 {
                    continue;
                }
                let theta = (a[q * n + q] - a[p * n + p]) / (2.0 * pq);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..n {
                    let (kp, kq) = (a[k * n + p], a[k * n + q]);
                    a[k * n + p] = c * kp - s * kq;
                    a[k * n + q] = s * kp + c * kq;
                }
                for k in 0..n {
                    let (pk, qk) = (a[p * n + k], a[q * n + k]);
                    a[p * n + k] = c * pk - s * qk;
                    a[q * n + k] = s * pk + c * qk;
                }
                for k in 0..n {
                    let (kp, kq) = (v[k * n + p], v[k * n + q]);
                    v[k * n + p] = c * kp - s * kq;
                    v[k * n + q] = s * kp + c * kq;
                }
            }
        }
    }
    ((0..n).map(|i| a[i * n + i]).collect(), v)
}

/// Главные компоненты признаков rows (канал - вектор значений клеток):
/// первые [`PARTS`], сумма разбросов итога - 1. Оценка - по части клеток.
fn reduce(rows: &[Vec<f32>]) -> Vec<Vec<f32>> {
    let rows = &rows[..rows.len().min(MOST_BANDS)];
    let n = rows.len();
    let count = rows.first().map_or(0, Vec::len);
    let step = (count / SAMPLE).max(1);
    let means: Vec<f64> = rows.iter().map(|r| r.iter().step_by(step).map(|v| f64::from(*v)).sum::<f64>() / count.div_ceil(step).max(1) as f64).collect();
    let mut cov = vec![0.0; n * n];
    let mut cell = vec![0.0; n];
    let mut used = 0.0;
    for at in (0..count).step_by(step) {
        for (b, row) in rows.iter().enumerate() {
            cell[b] = f64::from(row[at]) - means[b];
        }
        for p in 0..n {
            for q in p..n {
                cov[p * n + q] += cell[p] * cell[q];
            }
        }
        used += 1.0;
    }
    for p in 0..n {
        for q in p..n {
            cov[p * n + q] /= f64::max(used, 1.0);
            cov[q * n + p] = cov[p * n + q];
        }
    }
    let (values, vectors) = eigen(&cov, n);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|a, b| values[*b].total_cmp(&values[*a]));
    order.truncate(PARTS);
    let kept: f64 = order.iter().map(|k| values[*k]).sum();
    let norm = kept.max(1e-12).sqrt();
    order
        .iter()
        .map(|k| {
            let mut out = vec![0.0f32; count];
            for (b, row) in rows.iter().enumerate() {
                let w = (vectors[b * n + k] / norm) as f32;
                for (o, v) in out.iter_mut().zip(row) {
                    *o += w * v;
                }
            }
            out
        })
        .collect()
}

/// Признаки клеток good: вектор на признак, значения клеток good подряд.
/// Сумма разбросов цветовых признаков - 1 - weight, текстуры - weight.
/// None - каналы не меняются.
pub fn features(grid: &[f32], depth: usize, good: &[bool], rows: usize, cols: usize, weight: f64) -> Option<Vec<Vec<f32>>> {
    let (mean, spread) = rasterclass::scales(grid, depth, Some(good));
    let used: Vec<usize> = (0..depth).filter(|b| spread[*b] > 0.0).collect();
    if used.is_empty() {
        return None;
    }
    let cells = good.len();
    let mut bright = vec![0.0; cells];
    let mut bands = Vec::with_capacity(used.len());
    for band in &used {
        let mut row = Vec::with_capacity(cells);
        for i in 0..cells {
            if good[i] {
                let v = (f64::from(grid[i * depth + band]) - mean[*band]) / spread[*band];
                bright[i] += v;
                row.push(v as f32);
            }
        }
        bands.push(row);
    }
    let mut out: Vec<Vec<f32>> = if used.len() > PARTS {
        let share = (1.0 - weight).sqrt() as f32;
        reduce(&bands).into_iter().map(|r| r.into_iter().map(|v| v * share).collect()).collect()
    } else {
        let colour = ((1.0 - weight) / used.len() as f64).sqrt() as f32;
        for row in &mut bands {
            for v in row.iter_mut() {
                *v *= colour;
            }
        }
        bands
    };
    if weight > 0.0 {
        let scaled: Vec<f64> = bright.iter().map(|v| v / used.len() as f64).collect();
        let rough: Vec<f64> = texture(&scaled, good, rows, cols, RADIUS).into_iter().zip(good).filter(|(_, g)| **g).map(|(v, _)| v).collect();
        let count = rough.len().max(1) as f64;
        let average = rough.iter().sum::<f64>() / count;
        let spread = (rough.iter().map(|v| (v - average) * (v - average)).sum::<f64>() / count).sqrt();
        if spread > 0.0 {
            let share = weight.sqrt();
            out.push(rough.iter().map(|v| ((v - average) / spread * share) as f32).collect());
        }
    }
    Some(out)
}

/// Номера рёбер, где обе области - лучшие соседи друг друга. Ранг вместо
/// цены: при равных ценах каждая область входит в одну пару.
pub fn pairs(a: &[usize], b: &[usize], cost: &[f64], regions: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..cost.len()).collect();
    order.sort_by(|x, y| cost[*x].total_cmp(&cost[*y]));
    let mut rank = vec![0; cost.len()];
    for (place, edge) in order.iter().enumerate() {
        rank[*edge] = place;
    }
    let mut best = vec![usize::MAX; regions];
    for (edge, place) in rank.iter().enumerate() {
        let (p, q) = (a[edge], b[edge]);
        best[p] = best[p].min(*place);
        best[q] = best[q].min(*place);
    }
    (0..cost.len()).filter(|e| best[a[*e]] == rank[*e] && best[b[*e]] == rank[*e]).collect()
}

/// Ступень расчёта.
#[derive(Clone, Copy, PartialEq)]
enum Stage {
    /// Слияния до порога с формой, порог растёт ступенями.
    Grow(u32),
    /// Последний проход - по цвету: форма держала границы ровными, но
    /// не пускала большие однородные области срастись, река выходила сотами.
    Colour,
    /// Слияние областей меньше наименьшей.
    Small,
    Done,
}

/// Расчёт объектов проходами.
pub struct Cut {
    /// Суммы признаков областей: по feats на область.
    total: Vec<f64>,
    feats: usize,
    size: Vec<f64>,
    edge: Vec<f64>,
    parent: Vec<usize>,
    /// Рёбра между областями и длины их границ.
    a: Vec<usize>,
    b: Vec<usize>,
    shared: Vec<f64>,
    stage: Stage,
    limit: f64,
    target: f64,
    shape: f64,
    least: f64,
    good: Vec<bool>,
}

impl Cut {
    /// grid - значения клеток по depth подряд, NaN без данных, inside -
    /// клетки участка.
    pub fn new(grid: &[f32], depth: usize, rows: usize, cols: usize, inside: Option<&[bool]>, params: Params) -> Self {
        let Params { scale, weight, least, shape } = params;
        let cells = rows * cols;
        let depth = depth.max(1);
        let good: Vec<bool> = (0..cells).map(|i| grid.get(i * depth..(i + 1) * depth).is_some_and(usable) && inside.is_none_or(|m| m.get(i).copied().unwrap_or(false))).collect();
        let count = good.iter().filter(|g| **g).count();
        // Каналы не меняются - один объект на связную часть.
        let data = if count > 0 { features(grid, depth, &good, rows, cols, weight) } else { None }.unwrap_or_else(|| vec![vec![0.0; count]]);
        let feats = data.len();
        let mut total = vec![0.0; feats * count];
        for (k, row) in data.iter().enumerate() {
            for (i, v) in row.iter().enumerate() {
                total[i * feats + k] = f64::from(*v);
            }
        }
        // Номера клеток и рёбра соседних клеток: сначала по строкам,
        // потом по столбцам.
        let mut number = vec![usize::MAX; cells];
        let mut next = 0;
        for (slot, keep) in number.iter_mut().zip(&good) {
            if *keep {
                *slot = next;
                next += 1;
            }
        }
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for r in 0..rows {
            for c in 0..cols.saturating_sub(1) {
                let (p, q) = (number[r * cols + c], number[r * cols + c + 1]);
                if p != usize::MAX && q != usize::MAX {
                    a.push(p);
                    b.push(q);
                }
            }
        }
        for r in 0..rows.saturating_sub(1) {
            for c in 0..cols {
                let (p, q) = (number[r * cols + c], number[(r + 1) * cols + c]);
                if p != usize::MAX && q != usize::MAX {
                    a.push(p);
                    b.push(q);
                }
            }
        }
        // Периметр клетки - только к соседям на участке: край участка
        // не делает область неровной.
        let mut edge = vec![0.0; count];
        for (p, q) in a.iter().zip(&b) {
            edge[*p] += 1.0;
            edge[*q] += 1.0;
        }
        let target = scale * scale;
        let shared = vec![1.0; a.len()];
        Self { total, feats, size: vec![1.0; count], edge, parent: (0..count).collect(), a, b, shared, stage: Stage::Grow(0), limit: target / 4f64.powi(STAGES as i32), target, shape, least: f64::from(least), good }
    }

    /// Рёбер осталось.
    pub fn edges(&self) -> usize {
        self.a.len()
    }

    /// Цена слияния областей ea и eb: прирост разнородности плюс shape ×
    /// прирост неровности формы sqrt(n) · периметр.
    fn cost(&self, ea: usize, eb: usize, length: f64, shape: f64) -> f64 {
        let (na, nb) = (self.size[ea], self.size[eb]);
        let (pa, pb) = (&self.total[ea * self.feats..(ea + 1) * self.feats], &self.total[eb * self.feats..(eb + 1) * self.feats]);
        let sum: f64 = pa.iter().zip(pb).map(|(x, y)| (x / na - y / nb) * (x / na - y / nb)).sum();
        let mut cost = sum * na * nb / (na + nb);
        if shape > 0.0 {
            let whole = (na + nb).sqrt() * (self.edge[ea] + self.edge[eb] - 2.0 * length);
            cost += shape * (whole - na.sqrt() * self.edge[ea] - nb.sqrt() * self.edge[eb]);
        }
        cost
    }

    /// Один проход слияния. true - слияния на этой ступени кончились.
    fn pass(&mut self, limit: Option<f64>, shape: f64, small: Option<f64>) -> bool {
        if self.a.is_empty() {
            return true;
        }
        let near: Vec<usize> = match small {
            Some(s) => (0..self.a.len()).filter(|e| self.size[self.a[*e]] < s || self.size[self.b[*e]] < s).collect(),
            None => (0..self.a.len()).collect(),
        };
        if near.is_empty() {
            return true;
        }
        let mut edges = Vec::with_capacity(near.len());
        let mut costs = Vec::with_capacity(near.len());
        for e in near {
            let cost = self.cost(self.a[e], self.b[e], self.shared[e], shape);
            if limit.is_none_or(|l| cost <= l) {
                edges.push(e);
                costs.push(cost);
            }
        }
        if edges.is_empty() {
            return true;
        }
        let ea: Vec<usize> = edges.iter().map(|e| self.a[*e]).collect();
        let eb: Vec<usize> = edges.iter().map(|e| self.b[*e]).collect();
        for at in pairs(&ea, &eb, &costs, self.parent.len()) {
            let (keep, drop) = (ea[at], eb[at]);
            for k in 0..self.feats {
                self.total[keep * self.feats + k] += self.total[drop * self.feats + k];
            }
            self.size[keep] += self.size[drop];
            self.edge[keep] += self.edge[drop] - 2.0 * self.shared[edges[at]];
            self.parent[drop] = keep;
        }
        // Рёбра слившихся областей: общие длины складываются.
        let regions = self.parent.len() as u64;
        let mut keyed: Vec<(u64, f64)> = Vec::with_capacity(self.a.len());
        for ((p, q), length) in self.a.iter().zip(&self.b).zip(&self.shared) {
            let (p, q) = (self.parent[*p] as u64, self.parent[*q] as u64);
            if p != q {
                keyed.push((p.min(q) * regions + p.max(q), *length));
            }
        }
        keyed.sort_unstable_by_key(|k| k.0);
        self.a.clear();
        self.b.clear();
        self.shared.clear();
        let mut last = u64::MAX;
        for (key, length) in keyed {
            if key == last {
                if let Some(s) = self.shared.last_mut() {
                    *s += length;
                }
            } else {
                last = key;
                self.a.push((key / regions) as usize);
                self.b.push((key % regions) as usize);
                self.shared.push(length);
            }
        }
        false
    }

    /// Один проход. true - объекты готовы.
    pub fn step(&mut self) -> bool {
        match self.stage {
            Stage::Grow(n) => {
                if self.pass(Some(self.limit), self.shape, None) {
                    // Порог растёт ступенями: области растут вровень,
                    // а не одна за счёт соседей.
                    self.limit *= 4.0;
                    self.stage = if n >= STAGES { Stage::Colour } else { Stage::Grow(n + 1) };
                }
            }
            Stage::Colour => {
                if self.pass(Some(self.target), 0.0, None) {
                    self.stage = if self.least > 1.0 { Stage::Small } else { Stage::Done };
                }
            }
            Stage::Small => {
                if self.pass(None, self.shape, Some(self.least)) {
                    self.stage = Stage::Done;
                }
            }
            Stage::Done => {}
        }
        self.stage == Stage::Done
    }

    /// Номера объектов клеток с 0, -1 без объекта.
    pub fn finish(self) -> Vec<i32> {
        let mut parent = self.parent;
        loop {
            let up: Vec<usize> = parent.iter().map(|p| parent[*p]).collect();
            if up == parent {
                break;
            }
            parent = up;
        }
        let mut roots = parent.clone();
        roots.sort_unstable();
        roots.dedup();
        let mut number = vec![0i32; parent.len()];
        for (n, root) in roots.iter().enumerate() {
            number[*root] = n as i32;
        }
        let mut out = vec![-1; self.good.len()];
        let mut next = 0;
        for (slot, keep) in out.iter_mut().zip(&self.good) {
            if *keep {
                *slot = number[parent[next]];
                next += 1;
            }
        }
        out
    }
}

/// Объекты сетки целиком, см. [`Cut::new`].
pub fn segment(grid: &[f32], depth: usize, rows: usize, cols: usize, inside: Option<&[bool]>, params: Params) -> Vec<i32> {
    let mut cut = Cut::new(grid, depth, rows, cols, inside, params);
    while !cut.step() {}
    cut.finish()
}

/// Количество объектов.
pub fn count(labels: &[i32]) -> usize {
    labels.iter().copied().max().map_or(0, |m| usize::try_from(m + 1).unwrap_or(0))
}

/// Признаки объектов для классов: float32 по 2 depth на объект - среднее
/// и разброс каналов внутри объекта. Форма сетки rasterclass: объект -
/// клетка. Каналы с NaN у объекта быть не может.
pub fn table(grid: &[f32], depth: usize, labels: &[i32]) -> Vec<f32> {
    let objects = count(labels);
    let depth = depth.max(1);
    let mut first = vec![0.0f64; objects * depth];
    let mut second = vec![0.0f64; objects * depth];
    let mut size = vec![0.0f64; objects];
    for (i, label) in labels.iter().enumerate() {
        let Ok(object) = usize::try_from(*label) else {
            continue;
        };
        size[object] += 1.0;
        for band in 0..depth {
            let v = f64::from(grid.get(i * depth + band).copied().unwrap_or(0.0));
            first[object * depth + band] += v;
            second[object * depth + band] += v * v;
        }
    }
    let mut out = vec![0.0f32; 2 * depth * objects];
    for object in 0..objects {
        let n = size[object].max(1.0);
        for band in 0..depth {
            let mean = first[object * depth + band] / n;
            out[object * 2 * depth + band] = mean as f32;
            out[object * 2 * depth + depth + band] = (second[object * depth + band] / n - mean * mean).max(0.0).sqrt() as f32;
        }
    }
    out
}

/// Признаков рисунка объекта у [`pattern`].
pub const PATTERN: usize = 4;

/// Рисунок объектов, `pattern` модуля: по [`PATTERN`] чисел на объект.
/// Разброс цвета внутри объекта ([`table`]) не отличает ряды вырубки от
/// пятен леса или травы с тем же разбросом. Здесь яркость (каналы,
/// приведённые к разбросу 1) даёт четыре числа по клеткам самого объекта,
/// соседние объекты в них не входят:
/// - перепад - средний модуль перепада яркости между соседними клетками;
/// - зерно - перепад, делённый на разброс яркости по объекту: мелкий
///   рисунок - около 1, крупные пятна и плавный переход - меньше;
/// - направленность - согласованность направлений перепадов по объекту
///   (тензор структуры): ряды и полосы - ближе к 1, кроны и трава - к 0;
/// - доля перепадов - доля пар соседних клеток с перепадом больше
///   разброса по участку: редкие резкие края против частой ряби.
pub fn pattern(grid: &[f32], depth: usize, labels: &[i32], rows: usize, cols: usize) -> Vec<f32> {
    let objects = count(labels);
    let mut out = vec![0.0f32; PATTERN * objects];
    let depth = depth.max(1);
    let cells = rows * cols;
    if objects == 0 || labels.len() < cells {
        return out;
    }
    let has: Vec<bool> = labels.iter().take(cells).map(|l| *l >= 0).collect();
    let (mean, spread) = rasterclass::scales(grid, depth, Some(&has));
    let used: Vec<usize> = (0..depth).filter(|b| spread[*b] > 0.0).collect();
    if used.is_empty() {
        return out;
    }
    let bright: Vec<f64> = (0..cells)
        .map(|i| {
            if !has[i] {
                return 0.0;
            }
            let sum: f64 = used
                .iter()
                .map(|&b| {
                    let v = f64::from(grid.get(i * depth + b).copied().unwrap_or(0.0));
                    (if v.is_finite() { v } else { 0.0 } - mean[b]) / spread[b]
                })
                .sum();
            sum / used.len() as f64
        })
        .collect();
    // Соседи одного объекта по строке (с левым) и по столбцу (с верхним).
    let same_x = |i: usize| !i.is_multiple_of(cols) && has[i] && labels[i] == labels[i - 1];
    let same_y = |i: usize| i >= cols && has[i] && labels[i] == labels[i - cols];
    let gx = |i: usize| if same_x(i) { bright[i] - bright[i - 1] } else { 0.0 };
    let gy = |i: usize| if same_y(i) { bright[i] - bright[i - cols] } else { 0.0 };
    let sharp = {
        let n = has.iter().filter(|h| **h).count().max(1) as f64;
        let m = (0..cells).filter(|i| has[*i]).map(|i| bright[i]).sum::<f64>() / n;
        ((0..cells).filter(|i| has[*i]).map(|i| (bright[i] - m).powi(2)).sum::<f64>() / n).sqrt()
    };
    let mut sums = vec![[0.0f64; 9]; objects];
    for i in 0..cells {
        let Ok(object) = usize::try_from(labels[i]) else {
            continue;
        };
        let (x, y) = (gx(i), gy(i));
        // Центральные разности: у прямых разностей по строке и столбцу
        // общая клетка, и у шума выходило бы 0.5, а не 0.
        let (c, r) = (i % cols, i / cols);
        let cx = if c + 1 < cols && same_x(i) && same_x(i + 1) { bright[i + 1] - bright[i - 1] } else { 0.0 };
        let cy = if r + 1 < rows && same_y(i) && same_y(i + cols) { bright[i + cols] - bright[i - cols] } else { 0.0 };
        let s = &mut sums[object];
        s[0] += 1.0;
        s[1] += f64::from(u8::from(same_x(i))) + f64::from(u8::from(same_y(i)));
        s[2] += x.abs() + y.abs();
        s[3] += bright[i];
        s[4] += bright[i] * bright[i];
        s[5] += cx * cx;
        s[6] += cy * cy;
        s[7] += cx * cy;
        s[8] += f64::from(u8::from(x.abs() > sharp)) + f64::from(u8::from(y.abs() > sharp));
    }
    for (object, s) in sums.iter().enumerate() {
        let (size, links) = (s[0].max(1.0), s[1].max(1.0));
        let step = s[2] / links;
        let first = s[3] / size;
        let whole = (s[4] / size - first * first).max(0.0).sqrt();
        let grain = if whole > 1e-6 { (step / whole.max(1e-6)).min(2.0) } else { 0.0 };
        let energy = s[5] + s[6];
        let line = if energy > 0.0 { ((s[5] - s[6]).powi(2) + 4.0 * s[7] * s[7]).sqrt() / energy.max(1e-12) } else { 0.0 };
        let edges = s[8] / links;
        for (k, v) in [step, grain, line, edges].into_iter().enumerate() {
            out[object * PATTERN + k] = v as f32;
        }
    }
    out
}

/// Таблица объектов с рисунком: за строкой [`table`] шириной width идут
/// [`PATTERN`] чисел [`pattern`]. Ширина итога - width + PATTERN.
pub fn with_pattern(table: &[f32], width: usize, drawing: &[f32]) -> Vec<f32> {
    let objects = table.len().checked_div(width).unwrap_or(0);
    let mut out = Vec::with_capacity(objects * (width + PATTERN));
    for object in 0..objects {
        out.extend_from_slice(&table[object * width..(object + 1) * width]);
        out.extend((0..PATTERN).map(|k| drawing.get(object * PATTERN + k).copied().unwrap_or(0.0)));
    }
    out
}

/// Соседние объекты одного класса - один объект: (новые номера клеток,
/// классы новых объектов). kinds - классы объектов, -1 без класса, такие
/// объекты не сливаются.
pub fn dissolve(labels: &[i32], rows: usize, cols: usize, kinds: &[i16]) -> (Vec<i32>, Vec<i16>) {
    let mut parent: Vec<usize> = (0..kinds.len()).collect();
    fn root(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let mut join = |p: i32, q: i32| {
        let (Ok(p), Ok(q)) = (usize::try_from(p), usize::try_from(q)) else {
            return;
        };
        if p != q && kinds[p] == kinds[q] && kinds[p] >= 0 {
            let (x, y) = (root(&mut parent, p), root(&mut parent, q));
            parent[x.max(y)] = x.min(y);
        }
    };
    for r in 0..rows {
        for c in 0..cols {
            let at = r * cols + c;
            if c + 1 < cols {
                join(labels[at], labels[at + 1]);
            }
            if r + 1 < rows {
                join(labels[at], labels[at + cols]);
            }
        }
    }
    let roots: Vec<usize> = (0..kinds.len()).map(|n| root(&mut parent, n)).collect();
    let mut unique = roots.clone();
    unique.sort_unstable();
    unique.dedup();
    let mut number = vec![0i32; kinds.len()];
    for (n, r) in unique.iter().enumerate() {
        number[*r] = n as i32;
    }
    let out = labels.iter().map(|l| usize::try_from(*l).map_or(-1, |l| number[roots[l]])).collect();
    (out, unique.iter().map(|r| kinds[*r]).collect())
}

/// Клеток в объектах.
pub fn areas(labels: &[i32]) -> Vec<u64> {
    let mut out = vec![0; count(labels)];
    for label in labels {
        if let Ok(object) = usize::try_from(*label) {
            out[object] += 1;
        }
    }
    out
}

/// Клетки на границе объектов: сосед справа или снизу из другого объекта.
pub fn borders(labels: &[i32], rows: usize, cols: usize) -> Vec<bool> {
    let mut edge = vec![false; labels.len()];
    for r in 0..rows {
        for c in 0..cols {
            let at = r * cols + c;
            let right = c + 1 < cols && labels[at] != labels[at + 1];
            let below = r + 1 < rows && labels[at] != labels[at + cols];
            edge[at] = (right || below) && labels[at] >= 0;
        }
    }
    edge
}

/// Узел сетки и направление ребра: 0 - на восток, 1 - на юг, 2 - на запад,
/// 3 - на север. Клетка ребра лежит справа по ходу.
fn cell_of_edge(x: usize, y: usize, dir: u8, cols: usize, rows: usize) -> Option<usize> {
    let (row, col) = match dir {
        0 => (y, x),
        1 => (y, x.checked_sub(1)?),
        2 => (y.checked_sub(1)?, x.checked_sub(1)?),
        _ => (y.checked_sub(1)?, x),
    };
    (row < rows && col < cols).then_some(row * cols + col)
}

/// Кольца каждого объекта по рёбрам клеток: первое - внешнее. Точка -
/// узел сетки (столбец, строка), первая точка не повторяется, точки на
/// одной прямой убраны. Объект связен по сторонам клеток, поэтому внешнее
/// кольцо одно. Касание углами кольцо не рвёт: на таком узле обход идёт
/// вокруг той же клетки.
pub fn outlines(labels: &[i32], rows: usize, cols: usize) -> Vec<Vec<Vec<[f64; 2]>>> {
    let stride = cols + 1;
    let mut mask = vec![0u8; (rows + 1) * stride];
    let label = |r: isize, c: isize| -> i32 {
        if r < 0 || c < 0 || r >= rows as isize || c >= cols as isize { -1 } else { labels[r as usize * cols + c as usize] }
    };
    for r in 0..rows {
        for c in 0..cols {
            let own = labels[r * cols + c];
            if own < 0 {
                continue;
            }
            let (ir, ic) = (r as isize, c as isize);
            if label(ir - 1, ic) != own {
                mask[r * stride + c] |= 1;
            }
            if label(ir, ic + 1) != own {
                mask[r * stride + c + 1] |= 2;
            }
            if label(ir + 1, ic) != own {
                mask[(r + 1) * stride + c + 1] |= 4;
            }
            if label(ir, ic - 1) != own {
                mask[(r + 1) * stride + c] |= 8;
            }
        }
    }
    let mut out: Vec<Vec<Vec<[f64; 2]>>> = vec![Vec::new(); count(labels)];
    for start in 0..mask.len() {
        for first in 0..4u8 {
            if mask[start] & (1 << first) == 0 {
                continue;
            }
            let (sx, sy) = (start % stride, start / stride);
            let Some(cell) = cell_of_edge(sx, sy, first, cols, rows) else {
                continue;
            };
            let own = labels[cell];
            let (mut x, mut y, mut dir) = (sx, sy, first);
            let mut ring: Vec<[f64; 2]> = Vec::new();
            loop {
                mask[y * stride + x] &= !(1 << dir);
                let before = dir;
                (x, y) = match dir {
                    0 => (x + 1, y),
                    1 => (x, y + 1),
                    2 => (x - 1, y),
                    _ => (x, y - 1),
                };
                // Сначала направо - вокруг той же клетки.
                let next = [(dir + 1) % 4, dir, (dir + 3) % 4].into_iter().find(|d| mask[y * stride + x] & (1 << d) != 0 && cell_of_edge(x, y, *d, cols, rows).is_some_and(|c| labels[c] == own));
                let Some(next) = next else {
                    // Кольцо замкнулось поворотом: узел начала - угол.
                    if before != first {
                        ring.push([x as f64, y as f64]);
                    }
                    break;
                };
                if next != before {
                    ring.push([x as f64, y as f64]);
                }
                dir = next;
            }
            if ring.len() >= 3 && let Some(rings) = usize::try_from(own).ok().and_then(|o| out.get_mut(o)) {
                rings.push(ring);
            }
        }
    }
    for rings in &mut out {
        // Обход с клеткой справа в координатах строк: у внешнего кольца
        // площадь наибольшая.
        rings.sort_by(|p, q| ring_area(q).total_cmp(&ring_area(p)));
    }
    out
}

fn ring_area(ring: &[[f64; 2]]) -> f64 {
    let n = ring.len();
    let sum: f64 = (0..n).map(|i| ring[i][0] * ring[(i + 1) % n][1] - ring[(i + 1) % n][0] * ring[i][1]).sum();
    sum.abs() / 2.0
}
