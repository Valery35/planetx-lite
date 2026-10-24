// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Свои пункты меню на глобусе: адрес-шаблон точки для браузера. Перенос
//! `core/menulinks.py` модуля.
//!
//! Пункт - название и адрес с местами {lat}, {lon} и {zoom}: широта
//! и долгота точки под курсором и уровень масштаба карты по ширине вида.
//! Адрес открывает браузер, модуль сам в сеть не ходит. Список хранится
//! в настройках строкой JSON.
//!
//! Подстановка повторяет `str.format` Python для этих мест: `{{` и `}}` -
//! скобки, другое место в фигурных скобках и одиночная скобка - ошибка.
//! Формат места внутри скобок (`{lat:.2f}`, `{lat!r}`) - тоже ошибка.

/// Места шаблона.
pub const FIELDS: [&str; 3] = ["lat", "lon", "zoom"];
/// Допустимые начала адреса.
pub const SCHEMES: [&str; 2] = ["https://", "http://"];
/// Карта Windy для встраивания: на ней единицы задаёт адрес - °C и м/с,
/// у windy.com их задают только настройки сайта в браузере.
pub const EXAMPLE: (&str, &str) = (
    "Windy",
    "https://embed.windy.com/embed.html?type=map&location=coordinates&metricTemp=%C2%B0C&metricWind=m%2Fs&overlay=temp&product=ecmwf&level=surface&zoom={zoom}&lat={lat}&lon={lon}&detailLat={lat}&detailLon={lon}&detail=true",
);

/// Готовые сайты окна списка: название по-русски и по-английски, адрес.
/// Адреса - форматы ссылок из описаний сайтов. Названия сайтов - данные,
/// а не строки интерфейса.
pub const PRESETS: [(&str, &str, &str); 5] = [
    ("Google Карты", "Google Maps", "https://www.google.com/maps/@?api=1&map_action=map&center={lat},{lon}&zoom={zoom}&basemap=satellite"),
    ("Яндекс Карты", "Yandex Maps", "https://yandex.ru/maps/?ll={lon},{lat}&z={zoom}&pt={lon},{lat}"),
    ("2ГИС", "2GIS", "https://2gis.ru/?m={lon},{lat}/{zoom}"),
    ("OpenStreetMap", "OpenStreetMap", "https://www.openstreetmap.org/?mlat={lat}&mlon={lon}#map={zoom}/{lat}/{lon}"),
    ("Windy", "Windy", EXAMPLE.1),
];

/// Причина, по которой пункт не годится, или None - годится. Строки
/// причин - ключи перевода окна: «name», «scheme», «place», «braces».
pub fn check(name: &str, template: &str) -> Option<&'static str> {
    if name.trim().is_empty() {
        return Some("name");
    }
    let lower = template.trim().to_lowercase();
    if !SCHEMES.iter().any(|s| lower.starts_with(s)) {
        return Some("scheme");
    }
    if !template.contains("{lat}") || !template.contains("{lon}") {
        return Some("place");
    }
    if fill(template, 0.0, 0.0, 1.0).is_err() {
        return Some("braces");
    }
    None
}

/// Адрес точки: места {lat} и {lon} - градусы с шестью знаками, {zoom} -
/// уровень масштаба, целая часть. Другие места в фигурных скобках
/// и лишняя скобка - ошибка с текстом причины.
pub fn fill(template: &str, lat: f64, lon: f64, zoom: f64) -> Result<String, String> {
    let lat = format!("{lat:.6}");
    let lon = format!("{lon:.6}");
    // Целая часть с отбрасыванием дробной, как int() Python. Нечисло - 0.
    let zoom = (zoom.trunc() as i64).to_string();
    let mut out = String::new();
    let mut chars = template.trim().chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                if chars.peek() == Some(&'{') {
                    chars.next();
                    out.push('{');
                    continue;
                }
                let mut field = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some('{') => return Err("unexpected '{' in field name".to_string()),
                        Some(ch) => field.push(ch),
                        None => return Err("expected '}' before end of string".to_string()),
                    }
                }
                match field.as_str() {
                    "lat" => out.push_str(&lat),
                    "lon" => out.push_str(&lon),
                    "zoom" => out.push_str(&zoom),
                    _ => return Err(format!("unknown field {field:?}")),
                }
            }
            '}' => {
                if chars.peek() == Some(&'}') {
                    chars.next();
                    out.push('}');
                } else {
                    return Err("single '}' encountered".to_string());
                }
            }
            _ => out.push(c),
        }
    }
    Ok(out)
}

/// Пункты из строки настроек: список пар (название, шаблон). Испорченная
/// строка и негодные пункты пропускаются, края строк обрезаются.
pub fn load(text: &str) -> Vec<(String, String)> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let [name, template] = item.as_array()?.as_slice() else {
                return None;
            };
            let (name, template) = (name.as_str()?, template.as_str()?);
            check(name, template).is_none().then(|| (name.trim().to_string(), template.trim().to_string()))
        })
        .collect()
}

/// Строка настроек для пунктов, JSON без экранирования кириллицы.
pub fn dump(items: &[(String, String)]) -> String {
    let list: Vec<[&str; 2]> = items.iter().map(|(n, t)| [n.as_str(), t.as_str()]).collect();
    serde_json::to_string(&list).unwrap_or_default()
}
