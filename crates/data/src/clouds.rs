// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Облака из NASA GIBS: дата снимка, адрес тайла, прозрачность. Перенос
//! `core/clouds.py` модуля.
//!
//! Слой - VIIRS SNPP Corrected Reflectance True Color, снимок за сутки
//! в цвете, как его видит глаз. Отдельного слоя облаков в цвете у GIBS
//! нет. Облако - белое и серое, поэтому прозрачность тайла считается
//! по белизне пикселя: светлый пиксель без насыщенности становится
//! облаком, суша, море, пустыня и чёрные полосы между проходами спутника
//! пропадают. Снег и лёд тоже белые и остаются, на подложке они того же
//! цвета. Решение помощника модуля от 29 сентября 2026 года, его
//! утверждает автор.
//!
//! Снимок за вчерашние сутки UTC GIBS собирает до утра. Поэтому берутся
//! сутки, начало которых было не меньше LAG секунд назад: до полудня
//! UTC - позавчерашние, после - вчерашние.

use std::f64::consts::PI;

use crate::gibs::Day;

pub const LAYER: &str = "VIIRS_SNPP_CorrectedReflectance_TrueColor";
/// Адрес тайла: поле `{date}` - дата снимка, поля `{z}`, `{y}`, `{x}` -
/// тайл.
pub const URL: &str = "https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/VIIRS_SNPP_CorrectedReflectance_TrueColor/default/{date}/GoogleMapsCompatible_Level9/{z}/{y}/{x}.jpg";
/// GoogleMapsCompatible_Level9, пиксель около 300 м.
pub const MAX_LEVEL: u32 = 9;
/// Секунд от начала суток снимка до показа.
pub const LAG: f64 = 36.0 * 3600.0;
pub const ATTRIBUTION: (&str, &str) = ("NASA GIBS, VIIRS", "https://www.earthdata.nasa.gov/engage/open-data-services-software/earthdata-developer-portal/gibs-api");
/// Белизна: наименьший из трёх каналов, доля от 0 до 1. Ниже LOW облака
/// нет, выше HIGH оно непрозрачно.
pub const LOW: f64 = 0.42;
pub const HIGH: f64 = 0.85;
/// Насыщенность (наибольший - наименьший) / наибольший. Облако серое,
/// песок и сухая трава желтее. Выше GREY_END облака нет.
pub const GREY_START: f64 = 0.10;
pub const GREY_END: f64 = 0.28;
/// Широты угасания облаков у полюсов, градусы.
pub const FADE_START: f64 = 80.0;
pub const FADE_END: f64 = 85.0;
/// Предел момента по модулю, секунды: около 270 миллионов лет, дальше
/// календарь не считается.
const TIME_LIMIT: f64 = 8.64e15;

/// Дата снимка «YYYY-MM-DD» для момента unix_time. Момент не число
/// считается нулём, слишком далёкий ограничивается TIME_LIMIT.
pub fn image_date(unix_time: f64) -> String {
    let moment = if unix_time.is_nan() { 0.0 } else { (unix_time - LAG).clamp(-TIME_LIMIT, TIME_LIMIT) };
    Day::from_seconds(moment).text()
}

/// Адрес тайла для загрузчика, с полями `{z}`, `{x}`, `{y}`.
pub fn url_template(unix_time: f64) -> String {
    URL.replace("{date}", &image_date(unix_time))
}

fn smooth(edge0: f64, edge1: f64, value: f64) -> f64 {
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Широты середин строк пикселей тайла key (z, x, y), градусы.
pub fn row_latitudes(key: (u32, u32, u32), size: usize) -> Vec<f64> {
    let (z, _, y) = key;
    let count = 2f64.powi(z.min(1023) as i32);
    (0..size)
        .map(|row| {
            let share = (f64::from(y) + (row as f64 + 0.5) / size as f64) / count;
            (PI * (1.0 - 2.0 * share)).sinh().atan().to_degrees()
        })
        .collect()
}

/// Тайл снимка RGBA шириной width пикселей в облака: RGBA с премноженной
/// альфой. None - длина не кратна строке.
///
/// Цвет облака - цвет пикселя, так видны тени и толщина облака. key -
/// тайл снимка. С ним облака гаснут от FADE_START до FADE_END широты.
/// Выше кончается Web Mercator и лежит одноцветная шапка, без угасания
/// облака обрывались у неё ровным кругом.
pub fn cloud_rgba(rgba: &[u8], width: usize, key: Option<(u32, u32, u32)>) -> Option<Vec<u8>> {
    let stride = width.checked_mul(4).filter(|s| *s > 0)?;
    if !rgba.len().is_multiple_of(stride) {
        return None;
    }
    let rows = rgba.len() / stride;
    let fade: Vec<f64> = match key {
        Some(key) => row_latitudes(key, rows).into_iter().map(|lat| 1.0 - smooth(FADE_START, FADE_END, lat.abs())).collect(),
        None => vec![1.0; rows],
    };
    let mut out = vec![0u8; rgba.len()];
    for (row, (line, target)) in rgba.chunks_exact(stride).zip(out.chunks_exact_mut(stride)).enumerate() {
        for (pixel, to) in line.as_chunks::<4>().0.iter().zip(target.as_chunks_mut::<4>().0.iter_mut()) {
            let rgb = [0, 1, 2].map(|k| f64::from(pixel[k]) / 255.0);
            let low = rgb[0].min(rgb[1]).min(rgb[2]);
            let high = rgb[0].max(rgb[1]).max(rgb[2]);
            let grey = (high - low) / high.max(1e-3);
            let alpha = smooth(LOW, HIGH, low) * (1.0 - smooth(GREY_START, GREY_END, grey)) * fade[row];
            for (channel, value) in to.iter_mut().zip(rgb) {
                *channel = (value * alpha * 255.0).round_ties_even() as u8;
            }
            to[3] = (alpha * 255.0).round_ties_even() as u8;
        }
    }
    Some(out)
}

/// Тайл источника для тайла глобуса: сам тайл или предок уровня
/// max_level.
pub fn source_key(key: (u32, u32, u32), max_level: u32) -> (u32, u32, u32) {
    let (z, x, y) = key;
    if z <= max_level {
        return key;
    }
    let shift = z - max_level;
    (max_level, x.checked_shr(shift).unwrap_or(0), y.checked_shr(shift).unwrap_or(0))
}
