// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Похожие области: образец обводится на глобусе, программа находит
//! объекты участка, похожие на него по цвету и текстуре. Перенос
//! `core/similar.py` модуля.
//!
//! Объекты участка - те же, что у классификации ([`crate::segments`]).
//! Образец - объекты, занимающие контур: контур обводится неточно, а клетки
//! соседнего леса в контуре вокруг пруда уводили бы и средний цвет,
//! и разброс. Контур через границу двух объектов даёт два образца.
//! Признаки объектов приводятся к среднему 0, доля цвета в сумме разбросов -
//! 1 - weight, доля текстуры (разброса внутри объекта) - weight. Среднее
//! и разброс считаются с весом площади: мелкие объекты города не решают
//! масштаб признаков всего участка.
//!
//! Похожесть - exp(-d²), d - расстояние до ближайшего образца. Сумма
//! разбросов признаков - 1, поэтому у двух случайных объектов участка d²
//! около 2 и похожесть около 14 %, у совпавших - 100 %. Порог ползунка
//! сравнивается с готовой похожестью и расчёта заново не требует.
//!
//! Таблица объектов - [`crate::segments::table`]: width чисел на объект,
//! сначала средние каналов, потом разбросы.

use crate::segments;

/// Порог похожести по умолчанию, проценты.
pub const LIMIT: u32 = 70;
/// Доля контура, с которой объект входит в образец.
const SAMPLE_SHARE: f64 = 0.25;

/// Признаки образца: строки table объектов, занимающих не меньше
/// [`SAMPLE_SHARE`] клеток контура mask, иначе объекта с наибольшей долей.
/// Пусто - в контуре нет объектов.
pub fn sample_rows(table: &[f32], width: usize, parts: &[i32], mask: &[bool]) -> Vec<Vec<f32>> {
    let mut counts = vec![0usize; segments::count(parts)];
    let mut total = 0usize;
    for (part, inside) in parts.iter().zip(mask) {
        if *inside && let Ok(object) = usize::try_from(*part) {
            counts[object] += 1;
            total += 1;
        }
    }
    if total == 0 {
        return Vec::new();
    }
    let mut keep: Vec<usize> = (0..counts.len()).filter(|o| counts[*o] as f64 >= SAMPLE_SHARE * total as f64).collect();
    if keep.is_empty() {
        let mut best = 0;
        for (object, count) in counts.iter().enumerate() {
            if *count > counts[best] {
                best = object;
            }
        }
        keep.push(best);
    }
    keep.iter().filter_map(|o| table.get(o * width..(o + 1) * width).map(<[f32]>::to_vec)).collect()
}

/// Похожесть объектов от 0 до 1. table - [`segments::table`], sizes - клеток
/// в объектах, samples - признаки образцов, weight - доля текстуры. None -
/// признаки на участке не меняются.
pub fn scores(table: &[f32], width: usize, sizes: &[u64], samples: &[Vec<f32>], weight: f64) -> Option<Vec<f64>> {
    let bands = width / 2;
    let total: u64 = sizes.iter().sum();
    if total == 0 || width == 0 {
        return None;
    }
    let share: Vec<f64> = sizes.iter().map(|s| *s as f64 / total as f64).collect();
    let mut mean = vec![0.0; width];
    let mut spread = vec![0.0; width];
    for (row, part) in table.chunks(width).zip(&share) {
        for (m, v) in mean.iter_mut().zip(row) {
            *m += f64::from(*v) * part;
        }
    }
    for (row, part) in table.chunks(width).zip(&share) {
        for (k, v) in row.iter().enumerate() {
            let shift = f64::from(*v) - mean[k];
            spread[k] += shift * shift * part;
        }
    }
    for s in &mut spread {
        *s = s.sqrt();
    }
    let colour = spread[..bands].iter().filter(|s| **s > 0.0).count();
    let rough = spread[bands..].iter().filter(|s| **s > 0.0).count();
    if colour == 0 && rough == 0 {
        return None;
    }
    let weight = if rough == 0 {
        0.0
    } else if colour == 0 {
        1.0
    } else {
        weight
    };
    let factor: Vec<f64> = (0..width)
        .map(|k| {
            if spread[k] <= 0.0 {
                0.0
            } else if k < bands {
                ((1.0 - weight) / colour as f64).sqrt() / spread[k]
            } else {
                (weight / rough as f64).sqrt() / spread[k]
            }
        })
        .collect();
    let points: Vec<Vec<f64>> = samples.iter().map(|s| (0..width).map(|k| (f64::from(s.get(k).copied().unwrap_or(0.0)) - mean[k]) * factor[k]).collect()).collect();
    let out = table
        .chunks(width)
        .map(|row| {
            let far = points.iter().map(|point| (0..width).map(|k| ((f64::from(row[k]) - mean[k]) * factor[k] - point[k]).powi(2)).sum::<f64>()).fold(f64::INFINITY, f64::min);
            (-far).exp()
        })
        .collect();
    Some(out)
}

/// Найденные объекты: (номера клеток с 0, -1 вне найденного, похожесть
/// каждого в долях). limit - порог в долях, join - соседние найденные
/// объекты сливаются, похожесть слитого - среднее по площади.
pub fn pick(parts: &[i32], rows: usize, cols: usize, score: &[f64], limit: f64, join: bool) -> (Vec<i32>, Vec<f64>) {
    let kept: Vec<bool> = score.iter().map(|s| *s >= limit).collect();
    let chosen = |part: i32| usize::try_from(part).ok().filter(|o| kept.get(*o).copied().unwrap_or(false));
    let labels = if join {
        let kinds: Vec<i16> = kept.iter().map(|k| if *k { 0 } else { -1 }).collect();
        segments::dissolve(parts, rows, cols, &kinds).0
    } else {
        parts.to_vec()
    };
    let mut roots: Vec<i32> = parts.iter().zip(&labels).filter(|(p, _)| chosen(**p).is_some()).map(|(_, l)| *l).collect();
    roots.sort_unstable();
    roots.dedup();
    let mut out = vec![-1; parts.len()];
    let mut sum = vec![0.0; roots.len()];
    let mut size = vec![0.0f64; roots.len()];
    for (i, (part, label)) in parts.iter().zip(&labels).enumerate() {
        let (Some(object), Ok(n)) = (chosen(*part), roots.binary_search(label)) else {
            continue;
        };
        out[i] = n as i32;
        sum[n] += score[object];
        size[n] += 1.0;
    }
    let mean = sum.iter().zip(&size).map(|(s, n)| s / n.max(1.0)).collect();
    (out, mean)
}
