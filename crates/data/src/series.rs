// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Ряд снимков участка: значение индекса по датам сцен. Перенос
//! `core/series.py` модуля, окно «Ряд по датам» модуля - `ui/series.py`.
//!
//! Сцены - найденные окном «Космоснимки» и видимые в его списке. У каждой
//! сцены продукт считается по участку без облаков и теней по маске
//! сцены, на грубой сетке - не больше [`SERIES_PIXELS`] пикселей
//! по стороне. Точка графика - медиана участка, полоса - от 10-го
//! до 90-го процентиля. Медиана, а не среднее - остатки облаков, полосы
//! MSS и края снимка сдвигают её меньше.
//!
//! Продукты ряда - индексы и температура, у сочетаний каналов одного
//! числа нет. У MODIS классы снега за 8 суток и день выгорания тоже
//! не величины для медианы. У ASTER маски облаков нет, ряда у него нет.

use crate::flow::{civil_from_days, days_from_civil};
use crate::sentinel::{self, Grid, INDICES, LANDSATS, Mission, modis, percentile};

/// Пикселей стороны участка при расчёте ряда.
pub const SERIES_PIXELS: f64 = 200.0;
/// Меньше этой доли чистых пикселей участка, %, - сцена без точки.
pub const MIN_CLEAR: f64 = 10.0;
/// Процентили полосы разброса.
pub const LOW: f64 = 10.0;
pub const HIGH: f64 = 90.0;
/// Продукты MODIS, у которых есть ряд.
pub const MODIS_SERIES: [&str; 5] = ["lst_day", "lst_night", "modis_ndvi", "modis_evi", "snow_cover"];
/// Шаги делений оси дат в месяцах и в сутках.
pub const MONTH_STEPS: [i64; 8] = [1, 2, 3, 6, 12, 24, 60, 120];
pub const DAY_STEPS: [f64; 5] = [1.0, 2.0, 5.0, 7.0, 14.0];
/// Делений оси дат по умолчанию.
pub const TICKS: usize = 6;

/// Сводка продукта по участку одной сцены: медиана, 10-й и 90-й
/// процентили, доля пикселей участка со значением, %.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub median: f64,
    pub low: f64,
    pub high: f64,
    pub share: f64,
}

/// Продукты ряда спутника по порядку окна снимков, пусто - ряда нет.
pub fn products(mission: Mission) -> Vec<&'static str> {
    if modis::is_modis(mission) {
        return modis::products_of(mission)
            .iter()
            .copied()
            .filter(|p| MODIS_SERIES.contains(p))
            .collect();
    }
    if mission != Mission::Sentinel2 && !LANDSATS.contains(&mission) {
        return Vec::new();
    }
    sentinel::products_of(mission)
        .into_iter()
        .filter(|p| *p == "lst" || INDICES.iter().any(|i| i.name == *p))
        .collect()
}

/// Шаг сетки ряда, м: шаг продукта спутника, но не мельче стороны
/// участка, делённой на [`SERIES_PIXELS`]. xy - кольцо участка в метрах
/// системы сцены.
pub fn step(xy: &[(f64, f64)], mission: Mission) -> f64 {
    let native = modis::step(mission).unwrap_or_else(|| sentinel::step_for(xy, mission, &[]));
    let Some(bx) = sentinel::bounds(xy, 1.0) else {
        return native;
    };
    let side = (bx[2] - bx[0]).max(bx[3] - bx[1]);
    native.max((side / SERIES_PIXELS).ceil())
}

/// Сводка продукта по участку. values - продукт, area - пиксели
/// участка, clear - пиксели без облаков, теней и пропусков. None, если
/// размеры разные или чистых значений меньше [`MIN_CLEAR`] % участка.
pub fn point(values: &Grid<f32>, area: &Grid<bool>, clear: &Grid<bool>) -> Option<Point> {
    if values.shape() != area.shape() || values.shape() != clear.shape() {
        return None;
    }
    let total = area.data().iter().filter(|&&a| a).count();
    if total == 0 {
        return None;
    }
    let mut chosen: Vec<f64> = values
        .data()
        .iter()
        .zip(area.data().iter().zip(clear.data()))
        .filter(|&(v, (&a, &c))| a && c && v.is_finite())
        .map(|(&v, _)| f64::from(v))
        .collect();
    let share = 100.0 * chosen.len() as f64 / total as f64;
    if chosen.is_empty() || share < MIN_CLEAR {
        return None;
    }
    chosen.sort_by(f64::total_cmp);
    Some(Point {
        median: percentile(&chosen, 50.0),
        low: percentile(&chosen, LOW),
        high: percentile(&chosen, HIGH),
        share,
    })
}

/// Первые и последние сутки, которые ось дат умеет подписать, - годы
/// от 1 до 9999.
fn day_range() -> (f64, f64) {
    (days_from_civil(1, 1, 1) as f64, days_from_civil(9999, 12, 31) as f64)
}

/// Сутки от 1970-01-01 по времени сцены «ГГГГ-ММ-ДД ЧЧ:ММ», с долей
/// суток. Время можно не указывать. None, если дата не разобрана.
pub fn day_number(when: &str) -> Option<f64> {
    let num = |a: usize, b: usize| -> Option<i64> {
        let part = when.get(a..b)?;
        if !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    if !(1..=12).contains(&month) || day < 1 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    if civil_from_days(days) != (year, month, day) {
        return None;
    }
    let mut hours = 0.0;
    if when.len() >= 16 {
        let (h, m) = (num(11, 13)?, num(14, 16)?);
        hours = h as f64 + m as f64 / 60.0;
    }
    Some(days as f64 + hours / 24.0)
}

/// Дата «ГГГГ-ММ-ДД» суток day.
fn iso_date(day: f64) -> String {
    let (y, m, d) = civil_from_days(day.floor() as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Деления оси дат между сутками first и last: [(сутки, подпись)].
/// Шаг - сутки (1, 2, 5, 7, 14) при коротком промежутке, иначе месяцы
/// (1, 2, 3, 6, 12 и больше), около count делений. Подпись - год у шага
/// в год и больше, год и месяц у шага в месяцы, дата у шага в сутки.
/// Сутки за годами 1-9999 сдвигаются на край, не число - делений нет.
pub fn date_ticks(first: f64, last: f64, count: usize) -> Vec<(f64, String)> {
    if !(first.is_finite() && last.is_finite()) {
        return Vec::new();
    }
    let (lo, hi) = day_range();
    let (first, last) = (first.clamp(lo, hi), last.clamp(lo, hi));
    let count = count as f64;
    let span = (last - first).max(1.0);
    let mut out = Vec::new();
    if span / 30.0 < count {
        let size = DAY_STEPS
            .iter()
            .copied()
            .find(|size| span / size <= count)
            .unwrap_or(14.0);
        let mut day = (first / size).ceil() * size;
        while day <= last {
            out.push((day, iso_date(day)));
            day += size;
        }
        return out;
    }
    let months = span / 30.44;
    let size = MONTH_STEPS
        .iter()
        .copied()
        .find(|&m| months / m as f64 <= count)
        .unwrap_or(120);
    let (year, month, day) = civil_from_days(first.floor() as i64);
    let mut index = year * 12 + month - 1;
    if day > 1 {
        index += 1;
    }
    index = (index + size - 1) / size * size;
    loop {
        let (year, month) = (index / 12, index % 12 + 1);
        if year > 9999 {
            return out;
        }
        let day = days_from_civil(year, month, 1) as f64;
        if day > last {
            return out;
        }
        let label = if size % 12 == 0 {
            year.to_string()
        } else {
            format!("{year:04}-{month:02}")
        };
        out.push((day, label));
        index += size;
    }
}

/// Строка ряда для таблицы: время сцены, имя сцены и сводка или None.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub when: String,
    pub scene: String,
    pub value: Option<Point>,
}

/// Ряд для таблицы: строки в текст CSV с заголовком. Сцена без значения -
/// пустые поля. Название продукта в кавычках, кавычки внутри удваиваются.
pub fn csv_text(rows: &[Row], title: &str) -> String {
    let title = title.replace('"', "\"\"");
    let mut text = String::from("date,scene,median,p10,p90,clear_percent,product\n");
    for row in rows {
        let line = match row.value {
            None => format!("{},{},,,,,\"{title}\"\n", row.when, row.scene),
            Some(v) => format!(
                "{},{},{:.4},{:.4},{:.4},{:.1},\"{title}\"\n",
                row.when, row.scene, v.median, v.low, v.high, v.share
            ),
        };
        text.push_str(&line);
    }
    text
}
