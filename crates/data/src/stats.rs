// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сводка значений участка: среднее, разброс и квантили, перенос
//! `core/stats.py` модуля.
//!
//! Значения - растр расчёта любого окна анализа, нечисло и бесконечность
//! в сводку не идут.

/// Сводка: count - значений, mean - среднее, std - стандартное
/// отклонение, low и high - края, p10, q1, median, q3, p90 - квантили
/// 10, 25, 50, 75 и 90 %.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub count: usize,
    pub mean: f64,
    pub std: f64,
    pub low: f64,
    pub p10: f64,
    pub q1: f64,
    pub median: f64,
    pub q3: f64,
    pub p90: f64,
    pub high: f64,
}

/// Квантиль q процентов упорядоченных значений, линейно между соседними,
/// как `numpy.percentile`.
fn quantile(sorted: &[f64], q: f64) -> f64 {
    let at = q / 100.0 * sorted.len().saturating_sub(1) as f64;
    let low = at.floor() as usize;
    match (sorted.get(low), sorted.get(low + 1)) {
        (Some(a), Some(b)) => a + (b - a) * (at - at.floor()),
        (Some(a), None) => *a,
        _ => f64::NAN,
    }
}

/// Сводка конечных значений или None, если их нет.
pub fn describe(values: &[f64]) -> Option<Summary> {
    let mut data: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if data.is_empty() {
        return None;
    }
    data.sort_by(f64::total_cmp);
    let count = data.len() as f64;
    let mean = data.iter().sum::<f64>() / count;
    let std = (data.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / count).sqrt();
    Some(Summary {
        count: data.len(),
        mean,
        std,
        low: *data.first()?,
        p10: quantile(&data, 10.0),
        q1: quantile(&data, 25.0),
        median: quantile(&data, 50.0),
        q3: quantile(&data, 75.0),
        p90: quantile(&data, 90.0),
        high: *data.last()?,
    })
}

/// Запись значения - три значащие цифры, большие без дробной части.
pub fn rounded(value: f64) -> String {
    let size = value.abs();
    if size >= 100.0 {
        format!("{value:.0}")
    } else if size >= 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.2}")
    }
}
