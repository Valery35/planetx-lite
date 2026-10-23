// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Ход момента шкалы времени, перенос `core/clock.py` модуля.
//!
//! Показ идёт со скоростью из [`RATES`] - во сколько раз быстрее
//! часов, или проходит весь охват за [`FIT_SECONDS`]. Часы - окно
//! [`HALF`] вокруг момента, у модуля они для солнца и спутников. Дойдя
//! до края окна, охват сдвигается вслед за моментом. Без часов показ
//! проходит охват данных и встаёт на его конце или идёт по кругу.

/// Полуширина окна часов вокруг момента, секунды.
pub const HALF: f64 = 86_400.0;
/// Скорости показа: во сколько раз время шкалы идёт быстрее часов.
pub const RATES: [f64; 6] = [1.0, 10.0, 60.0, 600.0, 3600.0, 86_400.0];
/// Вся шкала проходится за столько секунд, когда скорость не задана.
pub const FIT_SECONDS: f64 = 20.0;

/// Окно часов вокруг момента, левый бегунок lo в нём остаётся.
pub fn window(moment: f64, lo: Option<f64>) -> (f64, f64) {
    let start = moment - HALF;
    (lo.map_or(start, |lo| start.min(lo)), moment + HALF)
}

/// Охват шкалы: охват данных data вместе с окном часов вокруг момента,
/// если часы нужны (on). Без часов - охват данных.
pub fn extent(data: Option<(f64, f64)>, moment: Option<f64>, lo: Option<f64>, on: bool) -> Option<(f64, f64)> {
    let Some(moment) = moment.filter(|_| on) else {
        return data;
    };
    let (a, b) = window(moment, lo);
    Some(match data {
        None => (a, b),
        Some((c, d)) => (a.min(c), b.max(d)),
    })
}

/// Сдвиг момента за dt секунд часов. rate None - охват span проходится
/// за seconds.
pub fn shift(span: (f64, f64), rate: Option<f64>, dt: f64, seconds: f64) -> f64 {
    match rate {
        Some(rate) => rate * dt,
        None => (span.1 - span.0) * dt / seconds,
    }
}

/// Показ при промежутке на всю шкалу: левый край lo стоит, правый hi
/// идёт на step к концу охвата span. С looping с конца - снова с lo.
/// Возвращает (hi, идёт ли показ).
pub fn sweep(lo: f64, hi: f64, span: (f64, f64), step: f64, looping: bool) -> (f64, bool) {
    let hi = hi + step;
    if hi < span.1 {
        (hi, true)
    } else if looping {
        (lo, true)
    } else {
        (span.1, false)
    }
}

/// Промежуток (lo, hi) после сдвига на step при охвате span.
///
/// grow - часы: на краю охвата показ идёт дальше, охват None значит,
/// что его надо построить заново вокруг нового момента. Иначе looping -
/// промежуток той же ширины начинает с начала охвата, без него встаёт
/// на конце. Возвращает (lo, hi, охват, идёт ли показ).
pub fn advance(lo: f64, hi: f64, span: (f64, f64), step: f64, grow: bool, looping: bool) -> (f64, f64, Option<(f64, f64)>, bool) {
    let (a, b) = span;
    let (lo, hi) = (lo + step, hi + step);
    if hi <= b {
        return (lo, hi, Some(span), true);
    }
    if grow {
        return (lo, hi, None, true);
    }
    let width = hi - lo;
    if looping { (a, a + width, Some(span), true) } else { (b - width, b, Some(span), false) }
}
