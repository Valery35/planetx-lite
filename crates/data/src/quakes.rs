// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Землетрясения: разбор сводки USGS, цвет по глубине, размер по
//! магнитуде. Перенос `core/quakes.py` модуля.
//!
//! Источник - сводка каталога USGS в GeoJSON (FEED), данные USGS
//! в общественном достоянии США. Координаты события - долгота, широта
//! и глубина очага в километрах ниже уровня моря. Цвет по глубине -
//! шкала DEPTH_STOPS: мелкие очаги красные, глубокие синие и лиловые,
//! как на картах USGS. Размер точки растёт с магнитудой.

/// Сводка: события магнитудой от 4.5 за 30 суток.
pub const FEED: &str = "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/4.5_month.geojson";
pub const ATTRIBUTION: (&str, &str) = ("Earthquakes: USGS", "https://earthquake.usgs.gov/earthquakes/feed/");
/// Глубина очага, км - цвет. Выбор помощника модуля, утверждает автор.
pub const DEPTH_STOPS: [(f64, [u8; 3]); 6] =
    [(0.0, [230, 40, 30]), (70.0, [255, 150, 0]), (150.0, [250, 225, 0]), (300.0, [60, 190, 80]), (500.0, [40, 120, 230]), (700.0, [120, 60, 200])];
/// Подписи шкалы, км.
pub const DEPTH_TICKS: [f64; 5] = [0.0, 150.0, 300.0, 500.0, 700.0];
/// Магнитуда самой маленькой точки.
pub const MIN_MAG: f64 = 4.5;
/// Размер точки, логические пиксели.
pub const MIN_SIZE: f64 = 4.0;
pub const SIZE_PER_MAG: f64 = 3.0;
pub const MAX_SIZE: f64 = 22.0;

/// Событие: широта, долгота, глубина очага в км, магнитуда, время
/// в секундах UTC, название места, адрес страницы события.
#[derive(Debug, Clone, PartialEq)]
pub struct Quake {
    pub lat: f64,
    pub lon: f64,
    pub depth: f64,
    pub mag: f64,
    pub time: Option<f64>,
    pub place: String,
    pub url: String,
}

/// Число JSON, но не логическое значение.
fn number(v: Option<&serde_json::Value>) -> Option<f64> {
    v.and_then(serde_json::Value::as_f64).filter(|x| x.is_finite())
}

/// События из GeoJSON сводки USGS. Событие без координат или магнитуды
/// пропускается. Ответ считается враждебным.
pub fn parse(data: &[u8]) -> Vec<Quake> {
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(data) else {
        return Vec::new();
    };
    let Some(features) = json.get("features").and_then(|f| f.as_array()) else {
        return Vec::new();
    };
    features
        .iter()
        .filter_map(|f| {
            let coords = f.get("geometry")?.get("coordinates")?.as_array()?;
            let props = f.get("properties")?;
            let (lon, lat, depth) = (number(coords.first())?, number(coords.get(1))?, number(coords.get(2))?);
            let mag = number(props.get("mag"))?;
            let text = |k: &str| props.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
            Some(Quake { lat, lon, depth, mag, time: number(props.get("time")).map(|t| t / 1000.0), place: text("place"), url: text("url") })
        })
        .collect()
}

/// Цвет очага по глубине, шкала DEPTH_STOPS.
pub fn depth_color(depth: f64) -> [u8; 3] {
    let stops = &DEPTH_STOPS;
    if depth <= stops[0].0 {
        return stops[0].1;
    }
    for pair in stops.windows(2) {
        let ((a, ca), (b, cb)) = (pair[0], pair[1]);
        if depth <= b {
            let t = (depth - a) / (b - a);
            return [0, 1, 2].map(|k| (f64::from(ca[k]) + (f64::from(cb[k]) - f64::from(ca[k])) * t).round() as u8);
        }
    }
    stops[stops.len() - 1].1
}

/// Размер точки в логических пикселях по магнитуде.
pub fn size(mag: f64) -> f64 {
    (MIN_SIZE + SIZE_PER_MAG * (mag - MIN_MAG)).clamp(MIN_SIZE, MAX_SIZE)
}

/// Событие в промежутке шкалы времени span (от, до) или шкала закрыта -
/// None. Событие без времени видно всегда, как метка без времени.
pub fn in_window(time: Option<f64>, span: Option<(f64, f64)>) -> bool {
    match (time, span) {
        (Some(t), Some((lo, hi))) => t >= lo && t <= hi,
        _ => true,
    }
}

/// Самое раннее и самое позднее время событий.
pub fn span(quakes: &[Quake]) -> Option<(f64, f64)> {
    let times = quakes.iter().filter_map(|q| q.time);
    let lo = times.clone().fold(f64::INFINITY, f64::min);
    let hi = times.fold(f64::NEG_INFINITY, f64::max);
    (lo.is_finite() && hi.is_finite()).then_some((lo, hi))
}
