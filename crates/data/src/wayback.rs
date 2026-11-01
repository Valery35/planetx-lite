// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Архив снимков Esri World Imagery Wayback, перенос `core/wayback.py`
//! модуля.
//!
//! Wayback - выпуски подложки Esri World Imagery с 20 февраля 2014 года.
//! Список выпусков - открытый файл [`CONFIG_URL`]: номер выпуска, дата
//! в названии, служба сведений о снимках выпуска. Тайл выпуска - JPEG 256.
//!
//! Выпуски, где в точке ничего не менялось, отдают тайл прежнего выпуска.
//! Служба tilemap отвечает, из какого выпуска тайл на самом деле (поле
//! select). Выпуски с изменениями в точке находятся цепочкой: новейший
//! выпуск, выпуск его тайла, выпуск старше него, и так до самого старого,
//! запросов столько, сколько изменений ([`changes_next`]).

use std::collections::BTreeMap;

use serde_json::Value;

pub const CONFIG_URL: &str = "https://s3-us-west-2.amazonaws.com/config.maptiles.arcgis.com/waybackconfig.json";
const ROOT: &str = "https://wayback.maptiles.arcgis.com/arcgis/rest/services/World_Imagery";
/// Подпись - из accessInformation слоя выпуска в ArcGIS Online, условия -
/// Esri Master License Agreement, как у World Imagery.
pub const ATTRIBUTION: &str = "Wayback - Esri, Vantor, Earthstar Geographics, and the GIS User Community";
pub const ATTRIBUTION_URL: &str = "https://livingatlas.arcgis.com/wayback/";
pub const MAX_LEVEL: u32 = 19;
/// Уровень тайла, по которому ищутся изменения в точке взгляда.
pub const LOCAL_LEVEL: u32 = 16;
/// Слои службы сведений: 0 - уровень 23 … 13 - уровень 10.
const META_TOP: i64 = 23;
const META_LAYERS: i64 = 14;
/// Длина экватора Web Mercator, м.
const EQUATOR: f64 = 40075016.7;

/// Выпуск: номер, дата «ГГГГ-ММ-ДД», адрес службы сведений или пусто.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub number: u64,
    pub day: String,
    pub meta: String,
}

/// Сведения о снимке в точке.
#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    pub date: String,
    pub provider: String,
    pub sensor: String,
    pub resolution: Option<f64>,
    pub accuracy: Option<f64>,
}

/// Дата «ГГГГ-ММ-ДД» из строки или None.
fn day_in(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let digits = |r: std::ops::Range<usize>, b: &[u8]| b.get(r).is_some_and(|s| s.iter().all(u8::is_ascii_digit));
    (0..bytes.len()).find_map(|i| {
        let tail = &bytes[i..];
        (digits(0..4, tail) && tail.get(4) == Some(&b'-') && digits(5..7, tail) && tail.get(7) == Some(&b'-') && digits(8..10, tail)).then(|| String::from_utf8_lossy(&tail[..10]).into_owned())
    })
}

/// Выпуски файла [`CONFIG_URL`] по возрастанию даты. Выпуски одного дня -
/// один, с большим номером.
pub fn parse_config(data: &[u8]) -> Vec<Release> {
    let Ok(Value::Object(raw)) = serde_json::from_slice::<Value>(data) else {
        return Vec::new();
    };
    let mut by_day: BTreeMap<String, Release> = BTreeMap::new();
    for (key, item) in &raw {
        let (Ok(number), Some(day)) = (key.parse::<u64>(), item.get("itemTitle").and_then(Value::as_str).and_then(day_in)) else {
            continue;
        };
        if by_day.get(&day).is_none_or(|old| number > old.number) {
            let meta = item.get("metadataLayerUrl").and_then(Value::as_str).unwrap_or("").to_string();
            by_day.insert(day.clone(), Release { number, day, meta });
        }
    }
    by_day.into_values().collect()
}

/// Шаблон адреса тайла выпуска для загрузчика: {z}, {x}, {y}.
pub fn tile_url(number: u64) -> String {
    format!("{ROOT}/WMTS/1.0.0/default028mm/MapServer/tile/{number}/{{z}}/{{y}}/{{x}}")
}

/// Уровень тайла поиска изменений по ширине вида width, м: тайл около
/// половины ширины, не подробнее [`LOCAL_LEVEL`].
pub fn search_level(width: f64) -> u32 {
    let level = (EQUATOR / width.max(1.0)).log2().round() as i64 + 1;
    level.clamp(2, i64::from(LOCAL_LEVEL)) as u32
}

pub fn tilemap_url(number: u64, z: u32, x: u32, y: u32) -> String {
    format!("{ROOT}/MapServer/tilemap/{number}/{z}/{y}/{x}")
}

/// Тайл (x, y) Web Mercator уровня z под точкой.
pub fn tile_of(lat: f64, lon: f64, z: u32) -> (u32, u32) {
    let lat = lat.clamp(-85.0511, 85.0511).to_radians();
    let n = f64::from(1u32 << z);
    let x = ((lon + 180.0) / 360.0 * n) as i64;
    let s = (lat.tan() + 1.0 / lat.cos()).ln();
    let y = ((1.0 - s / std::f64::consts::PI) / 2.0 * n) as i64;
    let top = (1i64 << z) - 1;
    (x.clamp(0, top) as u32, y.clamp(0, top) as u32)
}

/// Выпуск, из которого тайл выпуска number на самом деле, или None - у
/// выпуска тайла здесь нет.
pub fn parse_tilemap(data: &[u8], number: u64) -> Option<u64> {
    let raw: Value = serde_json::from_slice(data).ok()?;
    let has = raw.get("data").and_then(|d| d.get(0)).and_then(Value::as_i64).is_some_and(|v| v != 0);
    if !has {
        return None;
    }
    Some(raw.get("select").and_then(|s| s.get(0)).and_then(Value::as_u64).unwrap_or(number))
}

/// Следующий выпуск цепочки изменений: самый новый выпуск старше выпуска
/// effective, или None - старше нет. effective - номер выпуска, откуда
/// пришёл тайл.
pub fn changes_next(releases: &[Release], effective: u64) -> Option<&Release> {
    let day = &releases.iter().find(|r| r.number == effective)?.day;
    releases.iter().rfind(|r| r.day < *day)
}

/// Слой службы сведений для уровня масштаба z.
pub fn metadata_layer(z: u32) -> i64 {
    (META_TOP - i64::from(z)).clamp(0, META_LAYERS - 1)
}

/// Запрос сведений о снимке выпуска в точке или None без службы.
pub fn metadata_url(release: &Release, lat: f64, lon: f64, z: u32) -> Option<String> {
    if release.meta.is_empty() {
        return None;
    }
    Some(format!(
        "{}/{}/query?geometry={lon:.7},{lat:.7}&geometryType=esriGeometryPoint&inSR=4326&spatialRel=esriSpatialRelIntersects&outFields=SRC_DATE2,NICE_DESC,SRC_DESC,SRC_RES,SRC_ACC&returnGeometry=false&f=json",
        release.meta.trim_end_matches('/'),
        metadata_layer(z)
    ))
}

/// Сведения о снимке или None.
pub fn parse_metadata(data: &[u8]) -> Option<Info> {
    let raw: Value = serde_json::from_slice(data).ok()?;
    let attrs = raw.get("features")?.get(0)?.get("attributes")?;
    let date = attrs.get("SRC_DATE2").and_then(Value::as_f64).map_or(String::new(), |ms| crate::gibs::Day::from_seconds(ms / 1000.0).text());
    let text = |name: &str| attrs.get(name).and_then(Value::as_str).unwrap_or("").to_string();
    let number = |name: &str| attrs.get(name).and_then(Value::as_f64);
    Some(Info { date, provider: text("NICE_DESC"), sensor: text("SRC_DESC"), resolution: number("SRC_RES"), accuracy: number("SRC_ACC") })
}
