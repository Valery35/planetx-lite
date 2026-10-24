// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! ЛЭП, подстанции, кабели и трубопроводы OpenStreetMap. Перенос
//! `core/infra.py` и чистой части `ui/overpass.py` модуля, тайлы вида -
//! из `core/mining.py`.
//!
//! Линии power=line, power=cable, communication=line и man_made=pipeline
//! с геометрией, подстанции power=substation серединой - запросом
//! Overpass по тайлам вида. Класс ЛЭП - по наибольшему напряжению тега
//! voltage, класс трубопровода - по тегу substance. Классы, цвета
//! и толщины - выбор помощника модуля, утверждает автор.
//!
//! Общий код Overpass: тайлы уровня [`ZOOM`] у точки взгляда ниже
//! [`MAX_ALTITUDE`], недостающие тайлы - одним запросом POST формы
//! ([`form_body`]) по общей рамке ([`union`]). Серверы - [`SERVERS`],
//! к одному серверу не чаще [`GAP`] секунд. Пользователю при отказе -
//! короткая фраза, в журнал - ошибка без адресов ([`brief_error`]).

use serde_json::Value;

/// Предел запроса на сервере Overpass, секунды.
pub const TIMEOUT: u32 = 90;
/// Классы ЛЭП: ключ, наименьшее напряжение в кВ, цвет, толщина
/// в пикселях картинки тайла.
pub const VOLTAGES: [(&str, f64, [u8; 3], f64); 4] = [
    ("v500", 500.0, [220, 30, 30], 3.5),
    ("v220", 220.0, [240, 120, 20], 2.8),
    ("v110", 110.0, [230, 200, 30], 2.0),
    ("v35", 0.0, [170, 170, 170], 1.4),
];
/// Классы трубопроводов: ключ, значения substance, цвет.
pub const SUBSTANCES: [(&str, &[&str], [u8; 3]); 4] = [
    ("gas", &["gas", "natural_gas", "cng", "lng", "fuel_gas"], [245, 215, 60]),
    ("oil", &["oil", "crude_oil", "fuel", "petroleum", "diesel", "gasoline", "kerosene"], [120, 70, 40]),
    ("water", &["water", "drinking_water", "hot_water", "steam", "heat", "wastewater", "sewage"], [60, 140, 230]),
    ("other", &[], [170, 90, 200]),
];
/// Толщина трубопровода, пиксели картинки тайла.
pub const PIPE_WIDTH: f64 = 2.4;
/// Кабели: ключ, цвет, толщина в пикселях картинки тайла. Рисуются
/// пунктиром.
pub const CABLES: [(&str, [u8; 3], f64); 2] = [("cable", [150, 60, 170], 2.4), ("telecom", [20, 170, 170], 1.6)];

/// Серверы Overpass, главный первым, `mining.OSM_SERVERS` модуля.
pub const SERVERS: [&str; 3] = crate::mining::OSM_SERVERS;
/// Заголовок тела запроса Overpass.
pub const CONTENT_TYPE: &str = "application/x-www-form-urlencoded";

// Тайлы вида, рамка, подпись OSM, уровень, предел высоты и пауза между
// запросами - общие с шахтами OSM, `core/mining.py` модуля. Подпись
// модуль ставит и под ЛЭП, текст её начинается с «Mines:».
pub use crate::mining::{Bounds, GAP, MAX_ALTITUDE, MAX_TILES, OSM_ATTRIBUTION, ZOOM, tile_bounds, tile_of, union, view_tiles};

/// Вид объекта.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// ЛЭП power=line.
    Line,
    /// Силовой кабель power=cable.
    Cable,
    /// Линия связи communication=line.
    Telecom,
    /// Трубопровод man_made=pipeline.
    Pipe,
    /// Подстанция power=substation.
    Substation,
}

impl Kind {
    /// Имя вида, как поле kind модуля.
    pub fn name(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Cable => "cable",
            Self::Telecom => "telecom",
            Self::Pipe => "pipe",
            Self::Substation => "substation",
        }
    }
}

/// Объект ответа: вид, точки (широта, долгота), ключ класса стиля,
/// название, владелец, напряжение в кВ, вещество, прокладка (тег
/// location), объект OSM «тип/номер».
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub kind: Kind,
    pub points: Vec<(f64, f64)>,
    pub style: &'static str,
    pub name: String,
    pub operator: String,
    pub voltage: Option<f64>,
    pub substance: String,
    pub location: String,
    pub osm: String,
}

/// Текст запроса Overpass QL для рамки (юг, запад, север, восток):
/// линии с геометрией и подстанции серединой.
pub fn query(bounds: Bounds) -> String {
    let (s, w, n, e) = bounds;
    let b = format!("({s:.5},{w:.5},{n:.5},{e:.5})");
    format!(
        "[out:json][timeout:{TIMEOUT}];(way[\"power\"=\"line\"]{b};way[\"man_made\"=\"pipeline\"]{b};\
         way[\"power\"=\"cable\"]{b};way[\"communication\"=\"line\"]{b};);out geom tags;\
         (nwr[\"power\"=\"substation\"]{b};);out center tags;"
    )
}

/// Число вида «цифры» или «цифры.цифры».
fn plain_number(text: &str) -> bool {
    let mut parts = text.splitn(2, '.');
    let digits = |s: Option<&str>| s.is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()));
    let whole = parts.next();
    match parts.next() {
        None => digits(whole),
        fraction => digits(whole) && digits(fraction),
    }
}

/// Наибольшее напряжение тега voltage в кВ или None. Значения через точку
/// с запятой или запятую - цепи разного напряжения на одной опоре. Число
/// без «kV» - в вольтах.
pub fn kilovolts(text: &str) -> Option<f64> {
    let mut best: Option<f64> = None;
    for part in text.split([';', ',']) {
        let part = part.trim().to_lowercase().replace(' ', "");
        let (number, scale) = match part.strip_suffix("kv") {
            Some(rest) => (rest, 1.0),
            None => (part.as_str(), 0.001),
        };
        if !plain_number(number) {
            continue;
        }
        let Ok(value) = number.parse::<f64>() else { continue };
        let value = value * scale;
        best = Some(best.map_or(value, |b| b.max(value)));
    }
    best
}

/// Ключ класса ЛЭП по напряжению в кВ, None - нижний класс.
pub fn voltage_class(kv: Option<f64>) -> &'static str {
    let lowest = VOLTAGES[VOLTAGES.len() - 1].0;
    let Some(kv) = kv else { return lowest };
    VOLTAGES.iter().find(|(_, low, _, _)| kv >= *low).map_or(lowest, |v| v.0)
}

/// Ключ класса трубопровода по тегу substance.
pub fn substance_class(text: &str) -> &'static str {
    let value = text.to_lowercase();
    let other = SUBSTANCES[SUBSTANCES.len() - 1].0;
    SUBSTANCES.iter().find(|(_, values, _)| values.contains(&value.as_str())).map_or(other, |s| s.0)
}

/// Строка значения тега, иначе пусто. Теги OSM - строки.
fn tag<'a>(tags: Option<&'a Value>, key: &str) -> &'a str {
    tags.and_then(|t| t.get(key)).and_then(Value::as_str).unwrap_or("")
}

/// Значение JSON строкой, как `str()` модуля: null и нет значения -
/// «None».
fn py_str(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "None".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(true)) => "True".to_string(),
        Some(Value::Bool(false)) => "False".to_string(),
        Some(other) => other.to_string(),
    }
}

/// Точка (широта, долгота) из объекта с полями lat и lon или None.
fn latlon(value: &Value) -> Option<(f64, f64)> {
    let lat = value.get("lat")?.as_f64()?;
    let lon = value.get("lon")?.as_f64()?;
    (lat.is_finite() && lon.is_finite()).then_some((lat, lon))
}

/// Объекты ответа Overpass байтами JSON. Ответ не JSON - пустой список.
pub fn parse(data: &[u8]) -> Vec<Item> {
    serde_json::from_slice::<Value>(data).map(|json| parse_json(&json)).unwrap_or_default()
}

/// Объекты уже разобранного ответа Overpass, как [`parse`].
pub fn parse_json(answer: &Value) -> Vec<Item> {
    let mut out = Vec::new();
    let Some(elements) = answer.get("elements").and_then(Value::as_array) else {
        return out;
    };
    for element in elements.iter().filter(|e| e.is_object()) {
        let tags = element.get("tags").filter(|t| t.is_object());
        let name = match tag(tags, "name:ru") {
            "" => tag(tags, "name"),
            ru => ru,
        };
        let voltage = kilovolts(tag(tags, "voltage"));
        let substance = tag(tags, "substance");
        let item = |kind, points, style| Item {
            kind,
            points,
            style,
            name: name.to_string(),
            operator: tag(tags, "operator").to_string(),
            voltage,
            substance: substance.to_string(),
            location: tag(tags, "location").to_string(),
            osm: format!("{}/{}", py_str(element.get("type")), py_str(element.get("id"))),
        };
        if tag(tags, "power") == "substation" {
            let center = element.get("center").filter(|c| c.as_object().is_some_and(|o| !o.is_empty())).unwrap_or(element);
            if let Some(point) = latlon(center) {
                out.push(item(Kind::Substation, vec![point], voltage_class(voltage)));
            }
            continue;
        }
        let points: Vec<(f64, f64)> =
            element.get("geometry").and_then(Value::as_array).map(|g| g.iter().filter_map(latlon).collect()).unwrap_or_default();
        if points.len() < 2 {
            continue;
        }
        if tag(tags, "power") == "line" {
            out.push(item(Kind::Line, points, voltage_class(voltage)));
        } else if tag(tags, "power") == "cable" {
            out.push(item(Kind::Cable, points, "cable"));
        } else if tag(tags, "communication") == "line" {
            out.push(item(Kind::Telecom, points, "telecom"));
        } else if tag(tags, "man_made") == "pipeline" {
            out.push(item(Kind::Pipe, points, substance_class(substance)));
        }
    }
    out
}

/// Объекты без повторов одного объекта из соседних запросов, первый
/// остаётся.
pub fn unique(items: Vec<Item>) -> Vec<Item> {
    let mut seen = std::collections::HashSet::new();
    items.into_iter().filter(|item| seen.insert(item.osm.clone())).collect()
}

// Общий код Overpass.

/// Тело POST запроса Overpass: поле data формы, кодировка `urlencode`
/// модуля - пробел «+», буквы, цифры и «_.-~» как есть, прочие байты
/// UTF-8 - %XX.
pub fn form_body(query: &str) -> String {
    let mut out = String::with_capacity(query.len() * 3 + 5);
    out.push_str("data=");
    for byte in query.bytes() {
        match byte {
            b' ' => out.push('+'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => out.push(char::from(byte)),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Ошибка сети без адресов запроса: от адреса http(s) остаётся имя
/// сервера. Адрес запроса Overpass - в сотни знаков.
pub fn brief_error(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let scheme = ["https://", "http://"].into_iter().find(|s| rest.starts_with(s));
        if let Some(scheme) = scheme {
            let after = &rest[scheme.len()..];
            let host_end = after.find(|c: char| c == '/' || c.is_whitespace()).unwrap_or(after.len());
            if host_end > 0 {
                out.push_str(&after[..host_end]);
                let tail = &after[host_end..];
                let url_end = tail.find(char::is_whitespace).unwrap_or(tail.len());
                rest = &tail[url_end..];
                continue;
            }
        }
        let mut chars = rest.chars();
        if let Some(c) = chars.next() {
            out.push(c);
        }
        rest = chars.as_str();
    }
    out
}
