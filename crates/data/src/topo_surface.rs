// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Морфометрия поверхности: уклон и экспозиция по Horn (1981, ядро 3×3,
//! как в gdaldem), вершины и ямы как локальные экстремумы с фильтрами
//! по перепаду и радиусу. Перенос `core/topo_surface.py` модуля
//! (из Isoliner, `grid_isolines/topo_surface.py`, владелец тот же).
//!
//! Сетка - значения по строкам, `rows × cols`, строка 0 северная,
//! NaN - нет данных. Номер узла - `r * cols + c`.

/// Вид экстремума.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Extreme {
    /// Вершина.
    Peak,
    /// Яма.
    Pit,
}

/// Маска пустых узлов: своя маска или NaN, и то и другое пусто.
/// None - маска не той длины.
fn empty_mask(z: &[f64], nodata: Option<&[bool]>) -> Option<Vec<bool>> {
    match nodata {
        Some(m) if m.len() != z.len() => None,
        Some(m) => Some(z.iter().zip(m).map(|(v, e)| *e || !v.is_finite()).collect()),
        None => Some(z.iter().map(|v| !v.is_finite()).collect()),
    }
}

/// Уклон, градусы, и экспозиция, градусы от севера по часовой.
///
/// Horn 3×3, край продолжен копией крайнего узла. Экспозиция плоских
/// узлов (нулевой градиент) равна -1. Узлы nodata и их соседи получают
/// NaN в обоих выходах: ядро 3×3 через дыры не считается. Сетка не того
/// размера даёт пустые векторы.
pub fn slope_aspect(z: &[f64], rows: usize, cols: usize, cell: f64, nodata: Option<&[bool]>) -> (Vec<f64>, Vec<f64>) {
    let Some(mask) = empty_mask(z, nodata) else {
        return (Vec::new(), Vec::new());
    };
    if rows.checked_mul(cols) != Some(z.len()) || z.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let at = |r: usize, c: usize| {
        let i = r * cols + c;
        if mask[i] { f64::NAN } else { z[i] }
    };
    let mut slope = vec![f64::NAN; z.len()];
    let mut aspect = vec![f64::NAN; z.len()];
    for r in 0..rows {
        let (rn, rs) = (r.saturating_sub(1), (r + 1).min(rows - 1));
        for c in 0..cols {
            let i = r * cols + c;
            if mask[i] {
                continue;
            }
            let (cw, ce) = (c.saturating_sub(1), (c + 1).min(cols - 1));
            let (nw, n, ne) = (at(rn, cw), at(rn, c), at(rn, ce));
            let (w, e) = (at(r, cw), at(r, ce));
            let (sw, s, se) = (at(rs, cw), at(rs, c), at(rs, ce));
            let dzdx = ((ne + 2.0 * e + se) - (nw + 2.0 * w + sw)) / (8.0 * cell);
            // Строка 0 северная: положительный dzdy - рост на север.
            let dzdy = ((nw + 2.0 * n + ne) - (sw + 2.0 * s + se)) / (8.0 * cell);
            let value = dzdx.hypot(dzdy).atan().to_degrees();
            if !value.is_finite() {
                continue;
            }
            slope[i] = value;
            // Экспозиция - азимут спуска: вектор (-dzdx, -dzdy) в осях
            // (восток, север).
            aspect[i] = if dzdx == 0.0 && dzdy == 0.0 {
                -1.0
            } else {
                let a = (-dzdx).atan2(-dzdy).to_degrees();
                if a < 0.0 { a + 360.0 } else { a }
            };
        }
    }
    (slope, aspect)
}

/// Наибольшее по окну [c - r, c + r] линии src, за краем значений нет.
/// Блоки ширины окна с бегущими наибольшими слева и справа (van Herk,
/// Gil-Werman): время не зависит от радиуса.
fn line_max(src: &[f64], r: usize, out: &mut [f64]) {
    let n = src.len();
    let w = 2 * r + 1;
    let len = n + 2 * r;
    let val = |k: usize| if k < r || k >= r + n { f64::NEG_INFINITY } else { src[k - r] };
    let mut g = vec![f64::NEG_INFINITY; len];
    let mut h = vec![f64::NEG_INFINITY; len];
    for k in 0..len {
        g[k] = if k % w == 0 { val(k) } else { g[k - 1].max(val(k)) };
    }
    for k in (0..len).rev() {
        h[k] = if k % w == w - 1 || k == len - 1 { val(k) } else { h[k + 1].max(val(k)) };
    }
    for c in 0..n {
        out[c] = h[c].max(g[c + w - 1]);
    }
}

/// Наибольшее по квадратному окну 2r+1, раздельно по осям. За краем
/// сетки значений нет.
fn window_max(z: &[f64], rows: usize, cols: usize, r: usize) -> Vec<f64> {
    let mut across = vec![f64::NEG_INFINITY; z.len()];
    for row in 0..rows {
        line_max(&z[row * cols..(row + 1) * cols], r, &mut across[row * cols..(row + 1) * cols]);
    }
    let mut out = vec![f64::NEG_INFINITY; z.len()];
    let mut column = vec![0.0; rows];
    let mut done = vec![0.0; rows];
    for c in 0..cols {
        for row in 0..rows {
            column[row] = across[row * cols + c];
        }
        line_max(&column, r, &mut done);
        for row in 0..rows {
            out[row * cols + c] = done[row];
        }
    }
    out
}

/// Вершины: локальные максимумы в радиусе с фильтром перепада.
///
/// Узел - вершина, если он не ниже всех узлов квадратного окна радиуса
/// radius_m и выше наименьшего узла окна не меньше чем на min_drop. Из
/// узлов одного окна остаётся высший, при равных - первый по строкам,
/// то есть северо-западный узел плато. Ответ - (строка, столбец,
/// высота, перепад) по убыванию высоты.
pub fn find_peaks(z: &[f64], rows: usize, cols: usize, cell: f64, radius_m: f64, min_drop: f64, nodata: Option<&[bool]>) -> Vec<(usize, usize, f64, f64)> {
    let Some(mask) = empty_mask(z, nodata) else {
        return Vec::new();
    };
    if rows.checked_mul(cols) != Some(z.len()) || z.is_empty() {
        return Vec::new();
    }
    let ratio = (radius_m / cell).round_ties_even();
    let r = if ratio.is_finite() && ratio >= 1.0 { (ratio as usize).min(rows.max(cols)) } else { 1 };
    let high: Vec<f64> = z.iter().zip(&mask).map(|(v, e)| if *e { f64::NEG_INFINITY } else { *v }).collect();
    let wmax = window_max(&high, rows, cols, r);
    let low: Vec<f64> = z.iter().zip(&mask).map(|(v, e)| if *e { f64::NEG_INFINITY } else { -*v }).collect();
    let wmin: Vec<f64> = window_max(&low, rows, cols, r).iter().map(|v| -v).collect();
    let mut found: Vec<(usize, f64)> = Vec::new();
    for i in 0..z.len() {
        if mask[i] || high[i] < wmax[i] {
            continue;
        }
        let drop = high[i] - if wmin[i].is_finite() { wmin[i] } else { high[i] };
        if drop >= min_drop {
            found.push((i, drop));
        }
    }
    found.sort_by(|a, b| z[b.0].total_cmp(&z[a.0]));
    let mut out: Vec<(usize, usize, f64, f64)> = Vec::new();
    for (i, drop) in found {
        let (rr, cc) = (i / cols, i % cols);
        if out.iter().any(|p| p.0.abs_diff(rr) <= r && p.1.abs_diff(cc) <= r) {
            continue;
        }
        out.push((rr, cc, z[i], drop));
    }
    out
}

/// Вершины и ямы разом: (строка, столбец, высота, перепад, вид).
///
/// Яма ищется тем же find_peaks на обращённом рельефе. Высота и перепад
/// возвращаются в исходных знаках: у ямы высота - её настоящая отметка,
/// перепад - положительная глубина. Порядок - по убыванию перепада, при
/// равных вершины раньше ям.
///
/// Обе половины нужны одновременно: поверхность, построенная по одним
/// горизонталям, кладёт плоскую шапку на каждую замкнутую горизонталь,
/// и ошибка объёма на вершине и в яме одна и та же с разным знаком.
pub fn find_extremes(z: &[f64], rows: usize, cols: usize, cell: f64, radius_m: f64, min_drop: f64, nodata: Option<&[bool]>) -> Vec<(usize, usize, f64, f64, Extreme)> {
    let mut out: Vec<(usize, usize, f64, f64, Extreme)> = find_peaks(z, rows, cols, cell, radius_m, min_drop, nodata).into_iter().map(|(r, c, v, d)| (r, c, v, d, Extreme::Peak)).collect();
    let minus: Vec<f64> = z.iter().map(|v| -v).collect();
    for (r, c, _, d) in find_peaks(&minus, rows, cols, cell, radius_m, min_drop, nodata) {
        out.push((r, c, z[r * cols + c], d, Extreme::Pit));
    }
    out.sort_by(|a, b| b.3.abs().total_cmp(&a.3.abs()));
    out
}
