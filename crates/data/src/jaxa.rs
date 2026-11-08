// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Наборы JAXA Earth: наблюдения Земли файлами Cloud Optimized GeoTIFF
//! по датам. Решение автора от 9 ноября 2026 года, взято из GeoLibre 3.7.0.
//!
//! Каталог STAC - `data.earth.jaxa.jp/stac/cog/v1/catalog.json`, файлы
//! лежат в хранилище Wasabi, оно отдаёт CORS для любого адреса. Устройство
//! набора - `collection.json`, в нём дочерние каталоги дат (`./2026-10/`),
//! в дате - уровни, в уровне - полосы долгот и файлы. Уровень 0 - два
//! полушария по 180° (`W180.00-E000.00` и `E000.00-E180.00`), уровень n -
//! квадраты по 180/2ⁿ градусов. Значение - число файла (DN), умноженное
//! на `slope` и сложенное с `offset`, как `dn2value` описания файла.
//!
//! Строки - выборка наборов на весь мир по месяцам. Глобус берёт уровень 0
//! целиком: оба полушария на сетку 0.1°, около 11 км, как поля погоды.
//! Условия данных - `earth.jaxa.jp/en/data/policy/`.

use crate::sentinel::Raw;
use crate::weather::{Field, Grid};

/// Корень наборов в хранилище.
pub const ROOT: &str = "https://s3.ap-northeast-1.wasabisys.com/je-pds/cog/v1/";
pub const ATTRIBUTION: (&str, &str) = ("JAXA Earth", "https://data.earth.jaxa.jp/");
/// Шаг сетки глобуса, градусы.
pub const STEP: f64 = 0.1;

/// Набор: ключ строки, номер набора STAC, имя канала в имени файла,
/// раскраска, пересчёт числа файла и числа «нет данных».
#[derive(Debug, PartialEq)]
pub struct Set {
    pub key: &'static str,
    pub id: &'static str,
    pub band: &'static str,
    pub field: Field,
    pub slope: f64,
    pub offset: f64,
    /// Числа файла без значения: «нет данных» и коды ошибок.
    pub empty: &'static [f64],
}

/// Температура поверхности суши днём, GCOM-C SGLI. Файл - кельвины,
/// значение - градусы Цельсия.
pub const LAND_TEMPERATURE: Set = Set {
    key: "jaxa_lst",
    id: "JAXA.G-Portal_GCOM-C.SGLI_standard.L3-LST.daytime.v3_global_monthly",
    band: "LST",
    field: Field {
        key: "jaxa_lst",
        lines: &[],
        units: "°C",
        ramp: &[(-30.0, [60, 0, 130, 255]), (-15.0, [40, 60, 220, 255]), (0.0, [230, 240, 250, 255]), (15.0, [120, 210, 90, 255]), (30.0, [250, 210, 50, 255]), (45.0, [240, 110, 30, 255]), (60.0, [150, 10, 30, 255])],
        clear: None,
        scale: 1.0,
    },
    slope: 0.02,
    offset: -273.15,
    empty: &[65535.0],
};

/// Температура поверхности моря днём, GCOM-C SGLI.
pub const SEA_TEMPERATURE: Set = Set {
    key: "jaxa_sst",
    id: "JAXA.G-Portal_GCOM-C.SGLI_standard.L3-SST.daytime.v3_global_monthly",
    band: "SST",
    field: Field {
        key: "jaxa_sst",
        lines: &[],
        units: "°C",
        // Шкала без сине-зелёной середины: при прозрачности 0.7 она
        // сливалась с синим морем подложки.
        ramp: &[(-2.0, [50, 0, 110, 255]), (6.0, [40, 90, 220, 255]), (12.0, [70, 200, 220, 255]), (18.0, [240, 230, 80, 255]), (24.0, [245, 150, 40, 255]), (28.0, [220, 50, 30, 255]), (32.0, [130, 0, 30, 255])],
        clear: None,
        scale: 1.0,
    },
    slope: 0.0012,
    offset: -10.0,
    empty: &[65535.0],
};

/// Вегетационный индекс NDVI, GCOM-C SGLI.
pub const NDVI: Set = Set {
    key: "jaxa_ndvi",
    id: "JAXA.G-Portal_GCOM-C.SGLI_standard.L3-NDVI.daytime.v3_global_monthly",
    band: "NDVI",
    field: Field {
        key: "jaxa_ndvi",
        lines: &[],
        units: "NDVI",
        ramp: &[(-0.1, [140, 110, 80, 255]), (0.1, [210, 190, 140, 255]), (0.3, [200, 220, 110, 255]), (0.5, [110, 180, 60, 255]), (0.7, [40, 130, 40, 255]), (0.9, [0, 70, 20, 255])],
        clear: None,
        scale: 1.0,
    },
    slope: 0.0001,
    offset: -1.0,
    empty: &[65535.0],
};

/// Хлорофилл-а в воде, GCOM-C SGLI, мг/м³. Шкала по степеням десяти.
pub const CHLOROPHYLL: Set = Set {
    key: "jaxa_chla",
    id: "JAXA.G-Portal_GCOM-C.SGLI_standard.L3-CHLA.daytime.v3_global_monthly",
    band: "CHLA",
    field: Field {
        key: "jaxa_chla",
        lines: &[],
        units: "mg/m³",
        ramp: &[(0.03, [40, 20, 120, 255]), (0.1, [30, 80, 200, 255]), (0.3, [40, 170, 210, 255]), (1.0, [80, 200, 120, 255]), (3.0, [220, 220, 60, 255]), (10.0, [230, 120, 30, 255]), (30.0, [170, 20, 20, 255])],
        clear: None,
        scale: 1.0,
    },
    slope: 0.0016,
    offset: 0.0,
    empty: &[65535.0],
};

/// Осадки GSMaP, средняя интенсивность месяца, мм/ч.
pub const PRECIPITATION: Set = Set {
    key: "jaxa_precip",
    id: "JAXA.EORC_GSMaP_standard.Gauge.00Z-23Z.v6_monthly",
    band: "PRECIP",
    field: Field {
        key: "jaxa_precip",
        lines: &[],
        units: "mm/h",
        ramp: &[(0.02, [190, 220, 255, 140]), (0.1, [100, 170, 250, 200]), (0.25, [40, 110, 230, 230]), (0.5, [60, 40, 200, 245]), (1.0, [170, 40, 190, 255]), (2.0, [230, 30, 110, 255])],
        clear: Some(0.02),
        scale: 1.0,
    },
    slope: 1.0,
    offset: 0.0,
    empty: &[-999.9, -4.0, -8.0, -99.0],
};

/// Влажность почвы, GCOM-W AMSR2, проценты объёма.
pub const SOIL_MOISTURE: Set = Set {
    key: "jaxa_smc",
    id: "JAXA.G-Portal_GCOM-W.AMSR2_standard.L3-SMC.daytime.v3_global_monthly",
    band: "SMC",
    field: Field {
        key: "jaxa_smc",
        lines: &[],
        units: "%",
        ramp: &[(0.0, [170, 110, 50, 255]), (10.0, [220, 190, 110, 255]), (20.0, [180, 220, 140, 255]), (30.0, [70, 180, 170, 255]), (45.0, [30, 100, 200, 255]), (60.0, [20, 30, 140, 255])],
        clear: None,
        scale: 1.0,
    },
    slope: 0.1,
    offset: 0.0,
    empty: &[-32768.0, -32761.0, -32762.0, -32763.0, -32764.0, -32765.0, -32766.0, -32767.0],
};

/// Оптическая толща аэрозоля на 500 нм, MODIS Terra и Aqua.
pub const AEROSOL: Set = Set {
    key: "jaxa_aod",
    id: "JAXA.JASMES_Terra.MODIS-Aqua.MODIS_taua.v811_global_monthly",
    band: "T500",
    field: Field {
        key: "jaxa_aod",
        lines: &[],
        units: "AOD",
        ramp: &[(0.0, [255, 255, 210, 200]), (0.1, [250, 230, 140, 220]), (0.25, [240, 170, 60, 235]), (0.5, [210, 90, 30, 245]), (1.0, [140, 30, 30, 255]), (2.0, [70, 0, 40, 255])],
        clear: None,
        scale: 1.0,
    },
    slope: 0.001,
    offset: 0.0,
    empty: &[65535.0],
};

/// Наборы строк каталога по порядку.
pub const SETS: [&Set; 7] = [&LAND_TEMPERATURE, &SEA_TEMPERATURE, &NDVI, &CHLOROPHYLL, &PRECIPITATION, &SOIL_MOISTURE, &AEROSOL];

/// Адрес описания набора.
pub fn collection_url(set: &Set) -> String {
    format!("{ROOT}{}/collection.json", set.id)
}

/// Адрес файла полушария уровня 0 на дату date: east - восточное.
pub fn half_url(set: &Set, date: &str, east: bool) -> String {
    let (lons, name) = if east { ("E000.00-E180.00", "E000.00-S90.00-E180.00-N90.00") } else { ("W180.00-E000.00", "W180.00-S90.00-E000.00-N90.00") };
    format!("{ROOT}{}/{date}/0/{lons}/{name}-{}.tiff", set.id, set.band)
}

/// Начало даты набора в секундах UTC: «ГГГГ», «ГГГГ-ММ» или «ГГГГ-ММ-ДД».
pub fn date_seconds(date: &str) -> Option<f64> {
    let full = match date.len() {
        4 => format!("{date}-01-01"),
        7 => format!("{date}-01"),
        10 => date.to_string(),
        _ => return None,
    };
    crate::gibs::Day::parse(&full).map(crate::gibs::Day::seconds)
}

/// Даты набора из `collection.json`: дочерние каталоги вида
/// `./2026-10/catalog.json`, по возрастанию. Ответ считается враждебным.
pub fn parse_dates(text: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let mut dates: Vec<String> = value
        .get("links")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|l| l.get("rel").and_then(serde_json::Value::as_str) == Some("child"))
        .filter_map(|l| l.get("href").and_then(serde_json::Value::as_str))
        .filter_map(|h| h.strip_prefix("./")?.strip_suffix("/catalog.json"))
        .filter(|d| date_seconds(d).is_some())
        .map(str::to_string)
        .collect();
    dates.sort_by(|a, b| date_seconds(a).partial_cmp(&date_seconds(b)).unwrap_or(std::cmp::Ordering::Equal));
    dates.dedup();
    dates
}

/// Дата для момента moment, секунды UTC: последняя, что началась не позже
/// него, раньше первой - первая. Без момента - последняя.
pub fn pick_date(dates: &[String], moment: Option<f64>) -> Option<&str> {
    let Some(moment) = moment else {
        return dates.last().map(String::as_str);
    };
    dates.iter().rev().find(|d| date_seconds(d).is_some_and(|s| s <= moment)).or_else(|| dates.first()).map(String::as_str)
}

/// Значение числа файла dn, NaN - нет данных.
pub fn value(set: &Set, dn: f64) -> f64 {
    if !dn.is_finite() || set.empty.iter().any(|e| (dn - e).abs() < 1e-3) {
        return f64::NAN;
    }
    dn * set.slope + set.offset
}

/// Сетка мира из двух полушарий одного шага: запад - долготы -180..0,
/// восток - 0..180, ряды с севера. date - начало даты в секундах UTC.
pub fn world_grid(set: &Set, west: &Raw, east: &Raw, date: f64) -> Option<Grid> {
    let (rows, cols) = west.shape();
    if east.shape() != (rows, cols) || rows < 2 || cols == 0 {
        return None;
    }
    let step = 180.0 / cols as f64;
    if ((180.0 / rows as f64) - step).abs() > step * 1e-6 {
        return None;
    }
    let mut values = Vec::with_capacity(rows.checked_mul(cols * 2)?);
    for r in 0..rows {
        for half in [west, east] {
            let row = half.data().get(r * cols..(r + 1) * cols)?;
            values.extend(row.iter().map(|dn| value(set, *dn) as f32));
        }
    }
    // Узел сетки погоды - точка, у полушария - центр пикселя.
    Some(Grid { values, rows, cols: cols * 2, west: -180.0 + step / 2.0, north: 90.0 - step / 2.0, step, run: date, hour: 0, vectors: None })
}
