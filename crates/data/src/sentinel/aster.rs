// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Снимки ASTER окна снимков: каналы, перевод значений, продукты.
//! Перенос `core/aster.py` модуля.
//!
//! ASTER 2000-2006 годов без входа даёт Microsoft Planetary Computer -
//! коллекция aster-l1t, ключ SAS - тот же. Сцена - три файла COG в UTM:
//! VNIR (каналы 1, 2, 3N, 15 м), SWIR (4-9, 30 м, у части сцен их нет),
//! TIR (10-14, 90 м). Значения - целые числа прибора без масштаба.
//!
//! Яркость L = (DN - 1) × UCC для нормального усиления. Отражение на
//! верху атмосферы - π L d² / (ESUN sin h). Тепловые каналы - яркостная
//! температура K2 / ln(K1 / L + 1).
//!
//! ASTER после 2006 года - каталог NASA CMR (AST_L1T версии 004)
//! отвечает без входа, файлы в хранилище LP DAAC - по ключу Earthdata
//! каждого пользователя ([`super::earthdata_header`]).
//!
//! Облачной маски у L1T нет: облачность над участком - облачность
//! сцены, доля данных - по каналу 1.

use serde_json::Value;

use super::{Assets, Error, Grid, Mission, Product, Raw, head, list, number, object, text};

pub const COLLECTION: &str = "aster-l1t";
pub const MISSION: Mission = Mission::Aster;
pub const NASA_MISSION: Mission = Mission::AsterNasa;
pub const NASA_SEARCH: &str = "https://cmr.earthdata.nasa.gov/search/granules.umm_json";
pub const NASA_COLLECTION: &str = "C3306888411-LPCLOUD";
pub const NASA_FIRST: &str = "2007-01-01";
/// Сцен в ответе каталога.
pub const NASA_PAGE: usize = 200;
/// Сервер входа Earthdata: переадресация на него - файл без ключа.
pub const LOGIN_HOST: &str = "urs.earthdata.nasa.gov";
pub const FIRST_DAY: &str = "2000-03-04";
pub const LAST_DAY: &str = "2006-12-31";
pub const STEPS: [f64; 3] = [15.0, 30.0, 90.0];
pub const CREDIT: &str =
    "ASTER data courtesy of NASA/METI/AIST/Japan Spacesystems and U.S./Japan ASTER Science Team";

/// Канал: файл сцены, номер канала в файле с 1, длина волны, мкм, UCC
/// нормального усиления, Вт/(м² ср мкм) на единицу значения.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AsterBand {
    pub key: &'static str,
    pub file: &'static str,
    pub index: usize,
    pub wavelength: f64,
    pub ucc: f64,
}

const fn b(
    key: &'static str,
    file: &'static str,
    index: usize,
    wavelength: f64,
    ucc: f64,
) -> AsterBand {
    AsterBand {
        key,
        file,
        index,
        wavelength,
        ucc,
    }
}

pub const BANDS: [AsterBand; 14] = [
    b("b1", "VNIR", 1, 0.56, 1.688),
    b("b2", "VNIR", 2, 0.66, 1.415),
    b("b3n", "VNIR", 3, 0.82, 0.862),
    b("b4", "SWIR", 1, 1.65, 0.2174),
    b("b5", "SWIR", 2, 2.165, 0.0696),
    b("b6", "SWIR", 3, 2.205, 0.0625),
    b("b7", "SWIR", 4, 2.26, 0.0597),
    b("b8", "SWIR", 5, 2.33, 0.0417),
    b("b9", "SWIR", 6, 2.395, 0.0318),
    b("b10", "TIR", 1, 8.3, 0.006822),
    b("b11", "TIR", 2, 8.65, 0.006780),
    b("b12", "TIR", 3, 9.11, 0.006590),
    b("b13", "TIR", 4, 10.6, 0.005693),
    b("b14", "TIR", 5, 11.3, 0.005225),
];
pub const ORDER: [&str; 14] = [
    "b1", "b2", "b3n", "b4", "b5", "b6", "b7", "b8", "b9", "b10", "b11", "b12", "b13", "b14",
];
/// Солнечная постоянная канала, Вт/(м² мкм).
pub const ESUN: [(&str, f64); 9] = [
    ("b1", 1845.99),
    ("b2", 1555.74),
    ("b3n", 1119.47),
    ("b4", 231.25),
    ("b5", 79.81),
    ("b6", 74.99),
    ("b7", 68.66),
    ("b8", 59.74),
    ("b9", 56.92),
];
/// Постоянные K1, Вт/(м² ср мкм), и K2, К, яркостной температуры.
pub const THERMAL: [(&str, (f64, f64)); 5] = [
    ("b10", (3040.136402, 1735.337945)),
    ("b11", (2482.375199, 1666.398761)),
    ("b12", (1935.060183, 1585.420044)),
    ("b13", (866.468575, 1350.069147)),
    ("b14", (641.326517, 1271.221673)),
];
/// Сочетания (каналы на красный, зелёный, синий).
pub const COMPOSITES: [(&str, [&str; 3]); 2] = [
    ("aster_false", ["b3n", "b2", "b1"]),
    ("aster_swir", ["b4", "b6", "b8"]),
];
pub const RATIOS: [&str; 5] = ["kaolinite", "aloh", "mgoh", "ferric", "quartz"];
pub const PRODUCTS: [&str; 8] = [
    "aster_false",
    "aster_swir",
    "kaolinite",
    "aloh",
    "mgoh",
    "ferric",
    "quartz",
    "all_bands",
];
pub const DEFAULT: [&str; 3] = ["aster_false", "aster_swir", "kaolinite"];
/// Цвета отношений от малых к большим, доля 0-1 растяжения.
pub const RAMP: [(f64, [u8; 3]); 5] = [
    (0.0, [43, 131, 186]),
    (0.25, [171, 221, 164]),
    (0.5, [255, 255, 191]),
    (0.75, [253, 174, 97]),
    (1.0, [215, 25, 28]),
];

/// ASTER открытой копии 2000-2006 или NASA после 2006 года.
pub fn is_aster(mission: Mission) -> bool {
    matches!(mission, Mission::Aster | Mission::AsterNasa)
}

/// Канал по ключу.
pub fn band_of(key: &str) -> Option<&'static AsterBand> {
    BANDS.iter().find(|b| b.key == key)
}

/// K1, K2 теплового канала.
pub fn thermal(key: &str) -> Option<(f64, f64)> {
    THERMAL.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

fn esun(key: &str) -> Option<f64> {
    ESUN.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// Канал есть у сцены: файл канала (NASA) или файл группы каналов
/// (Planetary Computer).
pub fn has(key: &str, assets: &Assets) -> bool {
    assets.contains(key) || band_of(key).is_some_and(|b| assets.contains(b.file))
}

/// Окончание имени файла канала NASA: «VNIR_B03N», «TIR_B10».
pub fn file_suffix(key: &str) -> Option<String> {
    let band = band_of(key)?;
    let name = key.get(1..)?.to_uppercase();
    let number = name.trim_end_matches('N');
    let rest = name.get(number.len()..)?;
    let value: u32 = number.parse().ok()?;
    Some(format!("{}_B{value:02}{rest}", band.file))
}

/// Кодирование значения запроса, как quote_plus Python.
fn quote_plus(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                out.push(char::from(byte));
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Число, как формат «{:g}» Python: 6 значащих цифр, без хвостовых
/// нулей.
pub fn format_g(value: f64) -> String {
    if !value.is_finite() {
        return if value.is_nan() {
            "nan".into()
        } else if value > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        };
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0".into()
        } else {
            "0".into()
        };
    }
    let sci = format!("{value:.5e}");
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let trim = |s: String| {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    };
    if (-4..6).contains(&exp) {
        let digits = usize::try_from(5 - exp).unwrap_or(0);
        trim(format!("{value:.digits$}"))
    } else {
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{}e{sign}{:02}", trim(mantissa.to_string()), exp.abs())
    }
}

/// Адрес поиска сцен NASA CMR: рамка кольца (широта, долгота), дни
/// «ГГГГ-ММ-ДД», облачность сцены, новые первыми. None у пустого кольца.
pub fn cmr_url(
    ring: &[(f64, f64)],
    start: &str,
    end: &str,
    max_cloud: f64,
    page_size: usize,
) -> Option<String> {
    if ring.is_empty() {
        return None;
    }
    let lat0 = ring.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let lat1 = ring.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let lon0 = ring.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let lon1 = ring.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    let query = [
        ("collection_concept_id", NASA_COLLECTION.to_string()),
        (
            "bounding_box",
            format!("{lon0:.6},{lat0:.6},{lon1:.6},{lat1:.6}"),
        ),
        ("temporal", format!("{start}T00:00:00Z,{end}T23:59:59Z")),
        ("cloud_cover", format!("0,{}", format_g(max_cloud))),
        ("sort_key", "-start_date".to_string()),
        ("page_size", page_size.to_string()),
    ];
    let pairs: Vec<String> = query
        .iter()
        .map(|(k, v)| format!("{}={}", quote_plus(k), quote_plus(v)))
        .collect();
    Some(format!("{NASA_SEARCH}?{}", pairs.join("&")))
}

/// Сцена ответа CMR.
#[derive(Debug, Clone, PartialEq)]
pub struct CmrItem {
    pub id: String,
    pub when: String,
    pub cloud: f64,
    pub epsg: Option<u32>,
    pub sun: f64,
    pub preview: String,
    /// {канал: адрес} в порядке каналов.
    pub files: Vec<(String, String)>,
}

/// Сцены ответа CMR. Ночные сцены (солнце ниже горизонта, только
/// тепловые каналы) отброшены.
pub fn parse_cmr(data: &Value) -> Vec<CmrItem> {
    let mut out = Vec::new();
    for item in list(data.get("items")) {
        let Some(umm) = object(item.get("umm")) else {
            continue;
        };
        let mut attrs: Vec<(String, Option<&Value>)> = Vec::new();
        for a in list(umm.get("AdditionalAttributes")) {
            let name = a
                .get("Name")
                .map(super::py_str)
                .unwrap_or_else(|| "None".into());
            let first = list(a.get("Values")).first();
            match attrs.iter_mut().find(|(n, _)| *n == name) {
                Some(slot) => slot.1 = first,
                None => attrs.push((name, first)),
            }
        }
        let attr = |name: &str| attrs.iter().find(|(n, _)| n == name).and_then(|(_, v)| *v);
        let (Some(sun), Some(zone)) =
            (number(attr("SolarElevationAngle")), number(attr("UTMZone")))
        else {
            continue;
        };
        if sun.is_nan() || sun <= 0.0 || !zone.is_finite() || zone.abs() > 1e6 {
            continue;
        }
        let zone = zone.trunc() as i64;
        let mut files: Vec<(String, String)> = Vec::new();
        let mut preview = String::new();
        for link in list(umm.get("RelatedUrls")) {
            let url = text(link.get("URL"));
            if !url.starts_with("https://") {
                continue;
            }
            let kind = link.get("Type").and_then(Value::as_str);
            if kind == Some("GET RELATED VISUALIZATION") {
                preview = url;
                continue;
            }
            if kind != Some("GET DATA") {
                continue;
            }
            for key in ORDER {
                let Some(suffix) = file_suffix(key) else {
                    continue;
                };
                if url.ends_with(&format!("_{suffix}.tif")) {
                    match files.iter_mut().find(|(k, _)| k == key) {
                        Some(slot) => slot.1 = url.clone(),
                        None => files.push((key.to_string(), url.clone())),
                    }
                }
            }
        }
        if !files.iter().any(|(k, _)| k == "b1") {
            continue;
        }
        let geometry = umm
            .get("SpatialExtent")
            .and_then(|s| s.get("HorizontalSpatialDomain"))
            .and_then(|h| h.get("Geometry"));
        let mut lats: Vec<f64> = Vec::new();
        for polygon in list(geometry.and_then(|g| g.get("GPolygons"))) {
            for point in list(polygon.get("Boundary").and_then(|b| b.get("Points"))) {
                lats.push(number(point.get("Latitude")).unwrap_or(0.0));
            }
        }
        let north = lats.is_empty() || lats.iter().sum::<f64>() / lats.len() as f64 >= 0.0;
        let base = if north { 32600 } else { 32700 };
        let when = text(
            umm.get("TemporalExtent")
                .and_then(|t| t.get("SingleDateTime")),
        );
        let cloud = match umm.get("CloudCover") {
            None | Some(Value::Null) => 100.0,
            v => number(v).unwrap_or(100.0),
        };
        out.push(CmrItem {
            id: text(umm.get("GranuleUR")),
            when: head(&when.replace('T', " "), 16),
            cloud,
            epsg: u32::try_from(base + zone).ok(),
            sun,
            preview,
            files,
        });
    }
    out
}

/// Каналы для продуктов по порядку номеров. С assets - только каналы
/// сцены: «Все каналы» у сцены без SWIR - 8 каналов.
pub fn needed(products: &[&str], assets: Option<&Assets>) -> Vec<String> {
    let mut keys: Vec<&str> = Vec::new();
    for name in products {
        let add: &[&str] = match *name {
            "aster_false" => &["b3n", "b2", "b1"],
            "aster_swir" => &["b4", "b6", "b8"],
            "kaolinite" => &["b4", "b5", "b6"],
            "aloh" => &["b5", "b6", "b7"],
            "mgoh" => &["b7", "b8", "b9"],
            "ferric" => &["b1", "b2"],
            "quartz" => &["b10", "b11", "b12"],
            "all_bands" => &ORDER,
            _ => &[],
        };
        keys.extend_from_slice(add);
    }
    ORDER
        .iter()
        .filter(|k| keys.contains(k) && assets.is_none_or(|a| has(k, a)))
        .map(|k| k.to_string())
        .collect()
}

/// Файлы сцены для каналов: {файл: [каналы]} в порядке появления.
pub fn files_of(keys: &[String]) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for key in keys {
        let Some(band) = band_of(key) else {
            continue;
        };
        match out.iter_mut().find(|(f, _)| f == band.file) {
            Some(slot) => slot.1.push(key.clone()),
            None => out.push((band.file.to_string(), vec![key.clone()])),
        }
    }
    out
}

/// Продукты, файлы которых есть у сцены, - у части сцен нет SWIR.
pub fn available<'a>(products: &[&'a str], assets: &Assets) -> Vec<&'a str> {
    products
        .iter()
        .copied()
        .filter(|p| {
            (*p == "all_bands" && has("b1", assets))
                || needed(&[p], None).iter().all(|k| has(k, assets))
        })
        .collect()
}

/// Расстояние Земля - Солнце в астрономических единицах.
pub fn sun_distance(day_of_year: f64) -> f64 {
    1.0 - 0.01672 * (0.9856 * (day_of_year - 4.0)).to_radians().cos()
}

/// День года даты «ГГГГ-ММ-ДД» или None.
pub fn day_of_year(date: &str) -> Option<u32> {
    let mut parts = date.get(..10)?.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: usize = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let lengths = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let length = *lengths.get(month.checked_sub(1)?)?;
    if day == 0 || day > length {
        return None;
    }
    Some(lengths.iter().take(month - 1).sum::<u32>() + day)
}

/// Яркость канала, нет данных (0) - NaN, float32.
pub fn radiance(dn: &Raw, key: &str) -> Result<Grid<f32>, Error> {
    let ucc = band_of(key).ok_or(Error::UnknownBand)?.ucc as f32;
    Ok(dn.map(|v| {
        if v == 0.0 {
            f32::NAN
        } else {
            ((v as f32) - 1.0) * ucc
        }
    }))
}

/// Значение канала в выдаче: отражение на верху атмосферы у VNIR и SWIR,
/// яркостная температура, К, у TIR.
pub fn value(
    dn: &Raw,
    key: &str,
    sun_elevation: f64,
    day_of_year: f64,
) -> Result<Grid<f32>, Error> {
    let lum = radiance(dn, key)?;
    if let Some((k1, k2)) = thermal(key) {
        return Ok(lum.map(|l| {
            if l > 0.0 {
                (k2 as f32) / ((k1 as f32) / l + 1.0).ln()
            } else {
                f32::NAN
            }
        }));
    }
    let sine = sun_elevation.to_radians().sin();
    if sine.is_nan() || sine <= 0.0 {
        return Err(Error::SunBelowHorizon);
    }
    let d = sun_distance(day_of_year);
    let factor =
        (std::f64::consts::PI * d * d / (esun(key).ok_or(Error::UnknownBand)? * sine)) as f32;
    Ok(lum.map(|l| l * factor))
}

/// a / b, бесконечность - NaN. None при разных размерах.
fn ratio(a: &Grid<f32>, b: &Grid<f32>) -> Option<Grid<f32>> {
    a.zip_map(b, |a, b| {
        let v = a / b;
        if v.is_finite() { v } else { f32::NAN }
    })
}

fn add(a: &Grid<f32>, b: &Grid<f32>) -> Option<Grid<f32>> {
    a.zip_map(b, |a, b| a + b)
}

fn mul(a: &Grid<f32>, b: &Grid<f32>) -> Option<Grid<f32>> {
    a.zip_map(b, |a, b| a * b)
}

/// Массив продукта по значениям каналов ([`value`]). Кварц - по
/// яркости тепловых каналов, она передаётся под ключами «L10», «L11»,
/// «L12». None у неизвестного продукта или без канала.
pub fn product(name: &str, bands: &super::Bands) -> Option<Product> {
    let g = |k: &str| bands.get(k);
    if let Some((_, keys)) = COMPOSITES.iter().find(|(n, _)| *n == name) {
        return Some(Product::Stack(
            keys.iter().map(|k| g(k).cloned()).collect::<Option<_>>()?,
        ));
    }
    let single = match name {
        "kaolinite" => ratio(&add(g("b4")?, g("b6")?)?, g("b5")?)?,
        "aloh" => ratio(&add(g("b5")?, g("b7")?)?, g("b6")?)?,
        "mgoh" => ratio(&add(g("b7")?, g("b9")?)?, g("b8")?)?,
        "ferric" => ratio(g("b2")?, g("b1")?)?,
        "quartz" => ratio(&mul(g("L11")?, g("L11")?)?, &mul(g("L10")?, g("L12")?)?)?,
        "all_bands" => {
            return Some(Product::Stack(
                ORDER.iter().filter_map(|k| g(k).cloned()).collect(),
            ));
        }
        _ => return None,
    };
    Some(Product::Single(single))
}

/// Подпись канала в файле «Все каналы»: «B3N 0.82 um», у тепловых -
/// с единицей.
pub fn label(key: &str) -> String {
    let name = format!("B{}", key.get(1..).unwrap_or("").to_uppercase());
    let Some(band) = band_of(key) else {
        return name;
    };
    let text = format!("{name} {} um", format_g(band.wavelength));
    if thermal(key).is_some() {
        text + ", K"
    } else {
        text
    }
}

/// Имя файла продукта: «AST_20060815_aloh».
pub fn file_name(day: &str, name: &str) -> String {
    format!("AST_{}_{name}", day.replace('-', ""))
}
