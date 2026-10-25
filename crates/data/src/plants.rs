// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Электростанции мира, Global Power Plant Database WRI. Перенос
//! `core/plants.py` модуля.
//!
//! База WRI версии 1.3 - 34 936 электростанций с мощностью, видом
//! топлива, годом ввода и владельцем, CC BY 4.0, данные на 2021 год.
//! Архив 4.2 МБ скачивается при включении строки. У части станций место
//! в базе неверное - это ошибка источника. Цвет точки - вид топлива,
//! размер - мощность в логарифме от SIZE_MIN до SIZE_MAX.

pub const URL: &str = "https://wri-dataportal-prod.s3.amazonaws.com/manual/global_power_plant_database_v_1_3.zip";
/// Та же база версии 1.3.0 таблицей CSV в хранилище WRI на GitHub через
/// jsDelivr: архив S3 браузер страницы не читает, у него нет заголовка
/// CORS, проверено 25 октября 2026 года.
pub const WEB_URL: &str = "https://cdn.jsdelivr.net/gh/wri/global-power-plant-database@master/output_database/global_power_plant_database.csv";
pub const ATTRIBUTION: (&str, &str) = ("Power plants: WRI Global Power Plant Database, CC BY 4.0", "https://datasets.wri.org/dataset/globalpowerplantdatabase");
/// Вид топлива: ключ, значения primary_fuel, цвет. Выбор помощника модуля.
pub const FUELS: [(&str, &[&str], [u8; 3]); 9] = [
    ("coal", &["Coal", "Petcoke"], [60, 60, 60]),
    ("gas", &["Gas", "Cogeneration"], [240, 150, 40]),
    ("oil", &["Oil"], [150, 80, 40]),
    ("nuclear", &["Nuclear"], [220, 40, 160]),
    ("hydro", &["Hydro", "Wave and Tidal"], [40, 120, 230]),
    ("wind", &["Wind"], [120, 220, 230]),
    ("solar", &["Solar"], [250, 220, 40]),
    ("bio", &["Biomass", "Waste", "Geothermal"], [70, 170, 70]),
    ("other", &["Storage", "Other"], [200, 200, 200]),
];
pub const SIZE_MIN: f64 = 3.0;
pub const SIZE_MAX: f64 = 14.0;
/// Мощность, МВт, при которой точка наименьшая и наибольшая.
pub const MW_LOW: f64 = 1.0;
pub const MW_HIGH: f64 = 5000.0;

/// Номер вида топлива по значению primary_fuel.
pub fn fuel_of(name: &str) -> usize {
    FUELS.iter().position(|(_, values, _)| values.contains(&name)).unwrap_or(FUELS.len() - 1)
}

/// Размер точки по мощности, логарифм от MW_LOW до MW_HIGH.
pub fn size(capacity: f64) -> f64 {
    let mw = capacity.clamp(MW_LOW, MW_HIGH);
    let share = (mw / MW_LOW).ln() / (MW_HIGH / MW_LOW).ln();
    SIZE_MIN + share * (SIZE_MAX - SIZE_MIN)
}

/// Электростанция: место, вид топлива, мощность МВт, название, страна,
/// топливо как в базе, год ввода, владелец, адрес источника.
#[derive(Debug, Clone, PartialEq)]
pub struct Plant {
    pub lat: f64,
    pub lon: f64,
    pub fuel: usize,
    pub capacity: f64,
    pub name: String,
    pub country: String,
    pub fuel_name: String,
    pub year: String,
    pub owner: String,
    pub url: String,
}

/// Электростанции из байтов архива ZIP или CSV базы WRI.
pub fn parse(data: &[u8]) -> Result<Vec<Plant>, String> {
    let mut out = Vec::new();
    crate::csv::zip_table(data, ".csv", |col, cells| {
        let (Some(lat), Some(lon)) = (crate::csv::cell_number(cells, col("latitude")), crate::csv::cell_number(cells, col("longitude"))) else {
            return;
        };
        if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
            return;
        }
        let text = |name: &str| crate::csv::text(cells, col(name));
        let fuel_name = text("primary_fuel");
        out.push(Plant {
            lat,
            lon,
            fuel: fuel_of(&fuel_name),
            capacity: crate::csv::cell_number(cells, col("capacity_mw")).unwrap_or(0.0),
            name: text("name"),
            country: text("country_long"),
            fuel_name,
            year: crate::csv::cell_number(cells, col("commissioning_year")).map(|y| format!("{}", y.trunc())).unwrap_or_default(),
            owner: text("owner"),
            url: text("url"),
        });
    })?;
    Ok(out)
}
