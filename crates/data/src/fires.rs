// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Пожары: разбор сводки NASA FIRMS, цвет и размер по мощности
//! излучения. Перенос `core/fires.py` модуля.
//!
//! Открытая сводка FIRMS прибора VIIRS спутника NOAA-20 за 24 часа,
//! около 78 тысяч очагов, 375 м. Строка сводки - широта, долгота, дата
//! и время снимка UTC, мощность излучения FRP в мегаваттах,
//! достоверность, день или ночь. Цвет - шкала FRP_STOPS в логарифме,
//! слабые жёлтые, мощные тёмно-красные.

use crate::csv::{Table, number};

pub const FEED: &str = "https://firms.modaps.eosdis.nasa.gov/data/active_fire/noaa-20-viirs-c2/csv/J1_VIIRS_C2_Global_24h.csv";
/// Копия для страницы в хранилище planetx-live: у службы нет заголовка
/// CORS, браузер её ответ не читает. Копии раз в 3 часа обновляет задача
/// GitHub Actions того хранилища, с 25 октября 2026 года.
pub const FEED_WEB: &str = "https://raw.githubusercontent.com/Valery35/planetx-live/data/fires/J1_VIIRS_C2_Global_24h.csv";
pub const ATTRIBUTION: (&str, &str) = ("Fires: NASA FIRMS VIIRS NOAA-20", "https://firms.modaps.eosdis.nasa.gov/");
/// Мощность излучения, МВт - цвет.
pub const FRP_STOPS: [(f64, [u8; 3]); 4] = [(1.0, [255, 230, 80]), (10.0, [255, 150, 30]), (100.0, [230, 40, 20]), (1000.0, [120, 0, 0])];
/// Подписи шкалы, МВт.
pub const FRP_TICKS: [f64; 4] = [1.0, 10.0, 100.0, 1000.0];
pub const MIN_SIZE: f64 = 3.0;
pub const SIZE_PER_DECADE: f64 = 2.5;
pub const MAX_SIZE: f64 = 11.0;

/// Очаг: место, мощность, время снимка, достоверность, ночной снимок.
#[derive(Debug, Clone, PartialEq)]
pub struct Fire {
    pub lat: f64,
    pub lon: f64,
    pub frp: f64,
    pub time: Option<f64>,
    pub confidence: String,
    pub night: bool,
}

/// Секунды UTC по дате ГГГГ-ММ-ДД и времени ЧЧММ.
fn seconds(date: &str, hhmm: &str) -> Option<f64> {
    let day = crate::gibs::Day::parse(date.trim())?;
    let t: u32 = hhmm.trim().parse().ok()?;
    Some(day.seconds() + f64::from(t / 100) * 3600.0 + f64::from(t % 100) * 60.0)
}

/// Очаги из текста CSV сводки FIRMS. Строка без координат или мощности
/// пропускается.
pub fn parse(text: &str) -> Vec<Fire> {
    let table = Table::parse(text);
    let (Some(i_lat), Some(i_lon), Some(i_frp), Some(i_date), Some(i_time)) =
        (table.column("latitude"), table.column("longitude"), table.column("frp"), table.column("acq_date"), table.column("acq_time"))
    else {
        return Vec::new();
    };
    let (i_conf, i_dn) = (table.column("confidence"), table.column("daynight"));
    table
        .rows
        .iter()
        .filter_map(|row| {
            let (lat, lon, frp) = (number(row.get(i_lat))?, number(row.get(i_lon))?, number(row.get(i_frp))?);
            let time = seconds(row.get(i_date)?, row.get(i_time)?);
            let text = |i: Option<usize>| i.and_then(|i| row.get(i)).map_or(String::new(), |s| s.trim().to_string());
            Some(Fire { lat, lon, frp, time, confidence: text(i_conf), night: text(i_dn) == "N" })
        })
        .collect()
}

/// Цвет очага по мощности, шкала FRP_STOPS в логарифме мощности.
pub fn color(frp: f64) -> [u8; 3] {
    let v = frp.max(1e-3).log10();
    let stops = &FRP_STOPS;
    let first = stops[0];
    if v <= first.0.log10() {
        return first.1;
    }
    for pair in stops.windows(2) {
        let (a, b) = (pair[0].0.log10(), pair[1].0.log10());
        if v <= b {
            let t = (v - a) / (b - a);
            return [0, 1, 2].map(|k| (f64::from(pair[0].1[k]) + (f64::from(pair[1].1[k]) - f64::from(pair[0].1[k])) * t).round() as u8);
        }
    }
    stops[stops.len() - 1].1
}

/// Размер точки в логических пикселях по мощности.
pub fn size(frp: f64) -> f64 {
    (MIN_SIZE + SIZE_PER_DECADE * frp.max(1.0).log10()).clamp(MIN_SIZE, MAX_SIZE)
}
