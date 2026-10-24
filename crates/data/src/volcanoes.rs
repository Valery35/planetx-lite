// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Вулканы голоцена, Smithsonian Global Volcanism Program. Перенос
//! `core/volcanoes.py` модуля.
//!
//! Список Volcanoes of the World - около 1200 вулканов, извергавшихся за
//! последние 12 тысяч лет. Условия Smithsonian разрешают только
//! некоммерческое использование, поэтому список не распространяется:
//! строка скачивает его запросом WFS при включении. Цвет точки - время
//! последнего извержения, размер - высота вулкана над уровнем моря.

use crate::csv::{Table, number};

pub const URL: &str = "https://webservices.volcano.si.edu/geoserver/GVP-VOTW/ows?service=WFS&version=2.0.0&request=GetFeature&typeName=GVP-VOTW:Smithsonian_VOTW_Holocene_Volcanoes&outputFormat=csv";
pub const ATTRIBUTION: (&str, &str) = ("Volcanoes: Global Volcanism Program, Smithsonian Institution", "https://volcano.si.edu/");
/// Страница вулкана по номеру.
pub const PAGE: &str = "https://volcano.si.edu/volcano.cfm?vn=";
/// Последнее извержение: ключ, с какого года включительно, цвет.
pub const PERIODS: [(&str, Option<f64>, [u8; 3]); 4] =
    [("recent", Some(1900.0), [230, 40, 30]), ("historic", Some(1500.0), [245, 140, 40]), ("holocene", Some(-10000.0), [240, 210, 70]), ("unknown", None, [170, 170, 170])];
pub const SIZE_MIN: f64 = 5.0;
pub const SIZE_MAX: f64 = 12.0;
/// Высоты наименьшей и наибольшей точки, м.
pub const HEIGHT_LOW: f64 = 0.0;
pub const HEIGHT_HIGH: f64 = 6000.0;

/// Вулкан: место, высота, год последнего извержения и поля записи.
#[derive(Debug, Clone, PartialEq)]
pub struct Volcano {
    pub lat: f64,
    pub lon: f64,
    pub elevation: Option<f64>,
    pub last: Option<f64>,
    pub number: String,
    pub name: String,
    pub kind: String,
    pub country: String,
    pub setting: String,
    pub evidence: String,
}

/// Номер периода по году последнего извержения.
pub fn period_of(year: Option<f64>) -> usize {
    let Some(year) = year.filter(|y| y.is_finite()) else {
        return PERIODS.len() - 1;
    };
    PERIODS[..PERIODS.len() - 1].iter().position(|(_, start, _)| start.is_some_and(|s| year >= s)).unwrap_or(PERIODS.len() - 2)
}

/// Размер точки по высоте вулкана.
pub fn size(elevation: Option<f64>) -> f64 {
    let h = elevation.unwrap_or(0.0).clamp(HEIGHT_LOW, HEIGHT_HIGH);
    SIZE_MIN + (h - HEIGHT_LOW) / (HEIGHT_HIGH - HEIGHT_LOW) * (SIZE_MAX - SIZE_MIN)
}

/// Вулканы из ответа WFS в CSV. Не список GVP - ошибка.
pub fn parse(data: &[u8]) -> Result<Vec<Volcano>, String> {
    let table = Table::parse(&String::from_utf8_lossy(data));
    if table.column("Volcano_Name").is_none() {
        return Err("not a GVP volcano list".into());
    }
    let col = |name: &str| table.column(name);
    let (lat, lon) = (col("Latitude"), col("Longitude"));
    let cell = |row: &Vec<String>, name: &str| col(name).and_then(|i| row.get(i)).cloned().unwrap_or_default();
    Ok(table
        .rows
        .iter()
        .filter_map(|row| {
            let y = number(lat.and_then(|i| row.get(i))).filter(|y| (-90.0..=90.0).contains(y))?;
            let x = number(lon.and_then(|i| row.get(i))).filter(|x| (-180.0..=180.0).contains(x))?;
            Some(Volcano {
                lat: y,
                lon: x,
                elevation: number(col("Elevation").and_then(|i| row.get(i))),
                last: number(col("Last_Eruption_Year").and_then(|i| row.get(i))),
                number: cell(row, "Volcano_Number"),
                name: cell(row, "Volcano_Name"),
                kind: cell(row, "Primary_Volcano_Type"),
                country: cell(row, "Country"),
                setting: cell(row, "Tectonic_Setting"),
                evidence: cell(row, "Evidence_Category"),
            })
        })
        .collect())
}
