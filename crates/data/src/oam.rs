// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Снимки с дронов и самолётов OpenAerialMap: поиск по рамке вида. Перенос
//! `core/oam.py` модуля.
//!
//! Служба api.openaerialmap.org/meta отвечает без ключа, 21 487 снимков
//! на 1 ноября 2026 года. Снимок ложится тайлами службы
//! tiles.openaerialmap.org, файл не скачивается. Шаблон тайлов был у всех
//! 400 проверенных снимков, у старых адрес начинается с http.
//!
//! Общая запись ответа называет CC-BY 4.0, но у снимка своя лицензия
//! в поле properties.license. Из 300 новых снимков 73 - CC BY-NC 4.0, 5 -
//! CC BY-SA 4.0, поэтому показывается и пишется лицензия снимка. Запись
//! лицензии в данных разная, «CC-BY-NC 4.0» и «CC BY-NC 4.0». Даты бывают
//! с ошибкой ввода, например год 0206, такая дата не показывается.
//!
//! Служба разрешает чтение только своему сайту (заголовок CORS, проверено
//! 1 ноября 2026 года), поэтому окно есть только у программы.

use serde_json::Value;

use crate::rumsey::Bbox;

const SEARCH: &str = "https://api.openaerialmap.org/meta";
/// Подпись источника.
pub const ATTRIBUTION: &str = "OpenAerialMap (Open Imagery Network)";
/// Снимков на страницу ответа.
pub const PAGE: u32 = 100;
/// Раньше дата считается ошибкой ввода.
const FIRST_YEAR: u32 = 1990;
/// Часть адреса тайлов снимка.
const TILE_HOST: &str = "tiles.openaerialmap.org";
/// Глубже служба даёт увеличенные пиксели.
pub const MAX_LEVEL: u32 = 22;
/// Экватор Web Mercator, метры.
const CIRCLE: f64 = 40_075_016.686;

/// Снимок: день съёмки «ГГГГ-ММ-ДД» или пусто, пиксель в метрах, байт
/// в файле, лицензия снимка, рамка, адрес файла, адрес картинки или пусто,
/// шаблон тайлов XYZ или пусто.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub title: String,
    pub provider: String,
    pub day: String,
    pub gsd: f64,
    pub size: u64,
    pub license: String,
    pub bbox: Bbox,
    pub url: String,
    pub thumbnail: String,
    pub tiles: String,
}

/// Адрес поиска снимков в рамке, новые первыми. Рамка через линию перемены
/// дат берётся всеми долготами.
pub fn search_url(bbox: Bbox, page: u32) -> String {
    let [mut west, south, mut east, north] = bbox;
    if west > east {
        (west, east) = (-180.0, 180.0);
    }
    format!("{SEARCH}?bbox={west:.5}%2C{south:.5}%2C{east:.5}%2C{north:.5}&limit={PAGE}&page={page}&order_by=acquisition_end&sort=desc")
}

/// Единая запись лицензии снимка.
pub fn license_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ").replace("CC-BY", "CC BY")
}

/// День съёмки из записи ISO, пусто при ошибке ввода года.
pub fn day_of(text: &str) -> String {
    let day: String = text.chars().take(10).collect();
    let year = day.get(..4).filter(|y| y.chars().all(|c| c.is_ascii_digit())).and_then(|y| y.parse::<u32>().ok());
    if year.is_some_and(|y| y >= FIRST_YEAR) { day } else { String::new() }
}

fn spaced(row: &Value, key: &str) -> String {
    row.get(key).and_then(Value::as_str).unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ")
}

fn image(row: &Value) -> Option<Image> {
    let url = row.get("uuid").and_then(Value::as_str).filter(|u| u.starts_with("https://"))?;
    let bbox: Vec<f64> = row.get("bbox")?.as_array()?.iter().filter_map(Value::as_f64).collect();
    let bbox = Bbox::try_from(bbox).ok()?;
    let extra = row.get("properties");
    let text = |key: &str| extra.and_then(|e| e.get(key)).and_then(Value::as_str).unwrap_or("");
    let tiles = text("tms");
    let tiles = match tiles.strip_prefix("http://") {
        Some(rest) => format!("https://{rest}"),
        None => tiles.to_string(),
    };
    let thumbnail = text("thumbnail");
    Some(Image {
        title: spaced(row, "title"),
        provider: spaced(row, "provider"),
        day: day_of(row.get("acquisition_end").and_then(Value::as_str).unwrap_or("")),
        gsd: row.get("gsd").and_then(Value::as_f64).unwrap_or(0.0),
        size: row.get("file_size").and_then(Value::as_u64).unwrap_or(0),
        license: license_text(text("license")),
        bbox,
        url: url.to_string(),
        thumbnail: if thumbnail.starts_with("https://") { thumbnail.to_string() } else { String::new() },
        tiles: if tiles.starts_with("https://") { tiles } else { String::new() },
    })
}

/// Ответ службы: (снимки, найдено всего).
pub fn parse(data: &Value) -> (Vec<Image>, u64) {
    let found = data.get("meta").and_then(|m| m.get("found")).and_then(Value::as_u64).unwrap_or(0);
    let images = data.get("results").and_then(Value::as_array).into_iter().flatten().filter_map(image).collect();
    (images, found)
}

/// Строка прав слоя: у снимка своя лицензия и свой поставщик.
pub fn rights(image: &Image) -> String {
    let parts = [image.provider.as_str(), image.license.as_str(), "OpenAerialMap, contributors of Open Imagery Network"];
    parts.iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(", ")
}

/// Уровень тайлов с пикселем не крупнее пикселя снимка.
pub fn max_level(image: &Image) -> u32 {
    if image.gsd <= 0.0 {
        return MAX_LEVEL;
    }
    let lat = (image.bbox[1] + image.bbox[3]) / 2.0;
    let metres = CIRCLE / 256.0 * lat.to_radians().cos().max(0.05);
    ((metres / image.gsd).log2().ceil().max(0.0) as u32).min(MAX_LEVEL)
}

/// Источник слоя - тайлы снимка OpenAerialMap.
pub fn is_layer(source: &str) -> bool {
    source.contains(TILE_HOST)
}
