// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Карта тренда по ряду снимков: в каждом пикселе наклон Тейла-Сена
//! и тест Манна-Кендалла по значениям сцен ряда. Перенос `core/trend.py`
//! модуля.
//!
//! Наклон Тейла-Сена - медиана наклонов по всем парам сцен пикселя,
//! в единицах величины за год. Остатки облаков и выбросы его почти
//! не сдвигают, в отличие от наклона по методу наименьших квадратов.
//! Тест Манна-Кендалла - есть ли монотонный тренд: S - сумма знаков
//! разностей пар, дисперсия - n(n-1)(2n+5)/18 без поправки на равные
//! значения, Z с поправкой на непрерывность, p - двусторонняя
//! по нормальному распределению. Сезонный ход ряда тест не вычитает:
//! ряд из сцен одного сезона даёт тренд между годами, ряд всех месяцев -
//! смешивает сезон и тренд. Пары сцен с одним временем не считаются.
//!
//! Ряд - сцены по порядку, у каждой сцены значения пикселей одной сетки
//! по строкам, NaN вне чистых пикселей. Пиксель без [`MIN_SCENES`]
//! чистых значений тренда не получает. Расчёт идёт частями по [`CHUNK`]
//! пикселей ([`Job`]), так страница не стоит на большом участке.

use crate::sentinel::percentile;

/// Чистых значений пикселя, меньше - тренда нет.
pub const MIN_SCENES: usize = 6;
/// Пикселей в части расчёта.
pub const CHUNK: usize = 4096;
/// Суток в году.
pub const YEAR: f64 = 365.25;
/// p, ниже которой тренд значим.
pub const SIGNIFICANT: f64 = 0.05;
/// Процентиль модулей наклона для пределов шкалы, [`limits`].
pub const SHARE: f64 = 98.0;

/// Тренд по пикселям сетки, по строкам: наклон за год и p теста
/// Манна-Кендалла, у пикселя без тренда - NaN, и количество чистых
/// значений.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Trend {
    pub slope: Vec<f64>,
    pub p: Vec<f64>,
    pub n: Vec<usize>,
}

/// Расчёт тренда частями. Ряд хранится внутри до конца расчёта.
#[derive(Debug, Clone)]
pub struct Job {
    stack: Vec<Vec<f64>>,
    days: Vec<f64>,
    min_scenes: usize,
    pixels: usize,
    next: usize,
    out: Trend,
    values: Vec<f64>,
    slopes: Vec<f64>,
}

impl Job {
    /// Расчёт по ряду stack - значения пикселей каждой сцены, days -
    /// сутки сцен. None, если сцен и суток разное количество или у сцен
    /// разное количество пикселей.
    pub fn new(stack: Vec<Vec<f64>>, days: &[f64], min_scenes: usize) -> Option<Self> {
        let pixels = check(&stack, days)?;
        Some(Self {
            days: days.to_vec(),
            min_scenes,
            pixels,
            next: 0,
            out: Trend {
                slope: Vec::with_capacity(pixels),
                p: Vec::with_capacity(pixels),
                n: Vec::with_capacity(pixels),
            },
            values: Vec::with_capacity(stack.len()),
            slopes: Vec::new(),
            stack,
        })
    }

    /// Посчитать ещё не больше count пикселей, true - расчёт закончен.
    pub fn step(&mut self, count: usize) -> bool {
        let stop = self.next.saturating_add(count.max(1)).min(self.pixels);
        while self.next < stop {
            let (slope, p, n) = pixel(
                &self.stack,
                &self.days,
                self.next,
                self.min_scenes,
                &mut self.values,
                &mut self.slopes,
            );
            self.out.slope.push(slope);
            self.out.p.push(p);
            self.out.n.push(n);
            self.next += 1;
        }
        self.done()
    }

    pub fn done(&self) -> bool {
        self.next >= self.pixels
    }

    /// Доля посчитанных пикселей, от 0 до 1.
    pub fn progress(&self) -> f64 {
        if self.pixels == 0 {
            return 1.0;
        }
        self.next as f64 / self.pixels as f64
    }

    /// Пикселей сетки.
    pub fn pixels(&self) -> usize {
        self.pixels
    }

    /// Досчитать остаток и отдать тренд.
    pub fn finish(mut self) -> Trend {
        while !self.step(CHUNK) {}
        self.out
    }
}

/// Тренд всего ряда сразу. Ряд и None - как у [`Job::new`].
pub fn trend(stack: &[Vec<f64>], days: &[f64], min_scenes: usize) -> Option<Trend> {
    let pixels = check(stack, days)?;
    let mut out = Trend {
        slope: Vec::with_capacity(pixels),
        p: Vec::with_capacity(pixels),
        n: Vec::with_capacity(pixels),
    };
    let (mut values, mut slopes) = (Vec::with_capacity(stack.len()), Vec::new());
    for index in 0..pixels {
        let (slope, p, n) = pixel(stack, days, index, min_scenes, &mut values, &mut slopes);
        out.slope.push(slope);
        out.p.push(p);
        out.n.push(n);
    }
    Some(out)
}

/// Количество пикселей ряда или None, если размеры не сходятся.
fn check(stack: &[Vec<f64>], days: &[f64]) -> Option<usize> {
    if stack.len() != days.len() {
        return None;
    }
    let pixels = stack.first().map_or(0, Vec::len);
    stack.iter().all(|s| s.len() == pixels).then_some(pixels)
}

/// Наклон за год, p и количество чистых значений пикселя index.
fn pixel(
    stack: &[Vec<f64>],
    days: &[f64],
    index: usize,
    min_scenes: usize,
    values: &mut Vec<f64>,
    slopes: &mut Vec<f64>,
) -> (f64, f64, usize) {
    values.clear();
    values.extend(stack.iter().map(|s| s.get(index).copied().unwrap_or(f64::NAN)));
    let n = values.iter().filter(|v| v.is_finite()).count();
    if n < min_scenes {
        return (f64::NAN, f64::NAN, n);
    }
    slopes.clear();
    let mut s = 0.0;
    for (i, (&a, &da)) in values.iter().zip(days).enumerate() {
        for (&b, &db) in values.iter().zip(days).skip(i + 1) {
            let gap = (db - da) / YEAR;
            if gap == 0.0 {
                continue;
            }
            let diff = b - a;
            let slope = diff / gap;
            if !slope.is_nan() {
                slopes.push(slope);
            }
            if diff > 0.0 {
                s += 1.0;
            } else if diff < 0.0 {
                s -= 1.0;
            }
        }
    }
    (median(slopes), p_value(s, n), n)
}

/// Медиана списка, NaN у пустого. Порядок списка меняется.
fn median(list: &mut [f64]) -> f64 {
    let len = list.len();
    if len == 0 {
        return f64::NAN;
    }
    let mid = len / 2;
    let (below, &mut middle, _) = list.select_nth_unstable_by(mid, f64::total_cmp);
    if len % 2 == 1 {
        return middle;
    }
    let lower = below.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (lower + middle) / 2.0
}

/// Двусторонняя p теста Манна-Кендалла по S и количеству n.
fn p_value(s: f64, n: usize) -> f64 {
    let n = n as f64;
    let variance = n * (n - 1.0) * (2.0 * n + 5.0) / 18.0;
    let shifted = if s > 0.0 {
        s - 1.0
    } else if s < 0.0 {
        s + 1.0
    } else {
        0.0
    };
    let z = shifted / variance.max(1e-12).sqrt();
    erfc(z.abs() / std::f64::consts::SQRT_2)
}

/// Дополнительная функция ошибок, приближение Чебышёва из Numerical
/// Recipes, относительная ошибка меньше 1.2e-7.
fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let poly = -z * z - 1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07
                            + t * (-1.135_203_98
                                + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    let ans = t * poly.exp();
    if x >= 0.0 { ans } else { 2.0 - ans }
}

/// Симметричные пределы шкалы наклона: от -a до a, a - процентиль share
/// модулей наклона, или наибольший модуль, если процентиль ноль. Нет
/// наклонов или все нулевые - от -1 до 1.
pub fn limits(slope: &[f64], share: f64) -> (f64, f64) {
    let mut values: Vec<f64> = slope.iter().filter(|v| v.is_finite()).map(|v| v.abs()).collect();
    if values.is_empty() {
        return (-1.0, 1.0);
    }
    values.sort_by(f64::total_cmp);
    let mut top = percentile(&values, share);
    if top == 0.0 {
        top = values.last().copied().unwrap_or(0.0);
    }
    if top == 0.0 || top.is_nan() {
        top = 1.0;
    }
    (-top, top)
}
