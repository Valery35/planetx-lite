// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Поиск места по названию через Nominatim, геокодер OpenStreetMap,
//! перенос `core/geocode.py` модуля.
//!
//! Здесь адрес запроса и разбор ответа, без сети. Правила Nominatim
//! соблюдает окно: запрос только по Enter, не чаще раза
//! в [`SEARCH_INTERVAL`], заголовок программы, ответы запоминаются.
//! <https://operations.osmfoundation.org/policies/nominatim/>

use serde_json::Value;

pub const NOMINATIM: &str = "https://nominatim.openstreetmap.org/search";
/// Секунд между запросами, требование Nominatim.
pub const SEARCH_INTERVAL: f64 = 1.0;
/// Мест в ответе, не больше.
pub const SEARCH_LIMIT: usize = 8;
pub const REVERSE: &str = "https://nominatim.openstreetmap.org/reverse";
/// Подробность адреса обратного запроса: 18 - здание.
pub const REVERSE_ZOOM: u32 = 18;
/// Метров на пиксель уровня 0 у экватора, тайл 256 пикселей.
pub const EQUATOR_METRES: f64 = 156_543.033_92;
/// Ширина окна браузера, по ней берётся масштаб ссылки.
pub const LINK_PIXELS: f64 = 1000.0;

/// Найденное место. name - короткое название, detail - остаток полного
/// названия, например область и страна. lat, lon в градусах. box_ -
/// охват (запад, юг, восток, север) в градусах или None.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: String,
    pub detail: String,
    pub lat: f64,
    pub lon: f64,
    pub bbox: Option<(f64, f64, f64, f64)>,
}

/// Строка запроса без лишних пробелов, ключ памяти ответов.
pub fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Значение для строки запроса, как urlencode Python: пробел - плюс,
/// буквы, цифры и `-._~` как есть, остальное - %XX байтов UTF-8.
fn encode(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn query(base: &str, pairs: &[(&str, String)]) -> String {
    let parts: Vec<String> = pairs.iter().map(|(k, v)| format!("{k}={}", encode(v))).collect();
    format!("{base}?{}", parts.join("&"))
}

/// Адрес запроса к Nominatim. language - код языка названий.
pub fn search_url(text: &str, language: Option<&str>, limit: usize) -> String {
    let mut pairs = vec![("q", normalize(text)), ("format", "jsonv2".to_string()), ("limit", limit.to_string())];
    if let Some(l) = language.filter(|l| !l.is_empty()) {
        pairs.push(("accept-language", l.to_string()));
    }
    query(NOMINATIM, &pairs)
}

fn number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::String(s) => s.trim().parse::<f64>().ok(),
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
    .filter(|v| v.is_finite())
}

/// Охват из boundingbox Nominatim: юг, север, запад, восток.
fn bbox(item: &Value) -> Option<(f64, f64, f64, f64)> {
    let values: Vec<Option<f64>> = item.get("boundingbox")?.as_array()?.iter().map(|v| number(Some(v))).collect();
    match values.as_slice() {
        [Some(south), Some(north), Some(west), Some(east)] => Some((*west, *south, *east, *north)),
        _ => None,
    }
}

fn text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// Места из ответа Nominatim в формате jsonv2. Записи без координат
/// пропускаются. Порядок - как в ответе, Nominatim ставит первыми
/// важные места.
pub fn parse_places(data: &Value) -> Vec<Place> {
    let Some(items) = data.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items.iter().filter(|i| i.is_object()) {
        let (Some(lat), Some(lon)) = (number(item.get("lat")), number(item.get("lon"))) else {
            continue;
        };
        let full = text(item.get("display_name"));
        let mut parts: Vec<String> =
            full.split(',').map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).collect();
        let mut name = text(item.get("name"));
        if name.is_empty() {
            name = parts.first().cloned().unwrap_or_default();
        }
        if parts.first() == Some(&name) {
            parts.remove(0);
        }
        out.push(Place { name, detail: parts.join(", "), lat, lon, bbox: bbox(item) });
    }
    out
}

/// Адрес обратного запроса: что находится в точке.
pub fn reverse_url(lat: f64, lon: f64, language: Option<&str>, zoom: u32) -> String {
    let mut pairs = vec![
        ("lat", format!("{lat:.6}")),
        ("lon", format!("{lon:.6}")),
        ("format", "jsonv2".to_string()),
        ("zoom", zoom.to_string()),
    ];
    if let Some(l) = language.filter(|l| !l.is_empty()) {
        pairs.push(("accept-language", l.to_string()));
    }
    query(REVERSE, &pairs)
}

/// Адрес из ответа обратного запроса или пустая строка - адреса нет.
pub fn parse_reverse(data: &Value) -> String {
    match data.as_object() {
        Some(o) if o.get("error").is_none_or(|e| e.is_null() || e == &Value::Bool(false)) => {
            text(o.get("display_name"))
        }
        _ => String::new(),
    }
}

/// Уровень масштаба карты OpenStreetMap, на котором в окне шириной
/// [`LINK_PIXELS`] видна полоса width метров на широте lat, от 1 до 19.
pub fn link_zoom(lat: f64, width: f64) -> u32 {
    let per_pixel = width.max(1.0) / LINK_PIXELS;
    let zoom = (EQUATOR_METRES * lat.to_radians().cos() / per_pixel).log2();
    if zoom.is_nan() { 1 } else { zoom.round().clamp(1.0, 19.0) as u32 }
}

/// Ссылка на точку на карте OpenStreetMap с меткой в ней.
pub fn osm_link(lat: f64, lon: f64, width: f64) -> String {
    let zoom = link_zoom(lat, width);
    format!("https://www.openstreetmap.org/?mlat={lat:.6}&mlon={lon:.6}#map={zoom}/{lat:.5}/{lon:.5}")
}

/// Строка списка найденных мест.
pub fn place_text(place: &Place) -> String {
    if place.detail.is_empty() { place.name.clone() } else { format!("{}, {}", place.name, place.detail) }
}
