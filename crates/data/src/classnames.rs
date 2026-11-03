// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Названия классов трассировки от ИИ-помощника, перенос
//! `core/classnames.py` модуля 1.33.
//!
//! Помощник получает таблицу классов - средние значения каналов с их
//! ролями, долю участка, количество и средний размер областей, место
//! и месяц съёмки, название по правилам окна «Выделение объектов», если
//! оно есть, - и предлагает названия. Снимок, имена слоёв и файлов
//! в модель не уходят. Ответ - JSON {"names": [...]} по порядку классов.
//!
//! Текст вопроса - по-русски у русского интерфейса, иначе по-английски,
//! названия - на языке интерфейса.

use serde_json::Value;

/// Знаков в названии, длиннее обрезается.
pub const NAME_LIMIT: usize = 40;

/// Названия ролей каналов (русское, английское).
const ROLE_NAMES: [(&str, &str, &str); 12] = [
    ("blue", "синий", "blue"),
    ("green", "зелёный", "green"),
    ("red", "красный", "red"),
    ("nir", "ближний ИК", "near infrared"),
    ("swir1", "SWIR 1,6 мкм", "SWIR 1.6 µm"),
    ("swir2", "SWIR 2,2 мкм", "SWIR 2.2 µm"),
    ("coastal", "береговой", "coastal"),
    ("re1", "край красного 1", "red edge 1"),
    ("re2", "край красного 2", "red edge 2"),
    ("re3", "край красного 3", "red edge 3"),
    ("vapour", "водяной пар", "water vapour"),
    ("thermal", "тепловой, K", "thermal, K"),
];

/// Название роли канала на языке окна.
pub fn role_name(role: &str, ru: bool) -> String {
    let (base, more) = match role.split_once('@') {
        Some((b, _)) => (b, true),
        None => (role, false),
    };
    let name = ROLE_NAMES.iter().find(|r| r.0 == base).map_or(base, |r| if ru { r.1 } else { r.2 });
    match (more, ru) {
        (false, _) => name.to_string(),
        (true, true) => format!("{name} второго снимка"),
        (true, false) => format!("{name} of the second image"),
    }
}

/// Системная подсказка: дешифровщик снимка, ответ только JSON. language -
/// английское название языка названий.
pub fn system_text(ru: bool, language: &str) -> String {
    if ru {
        format!(
            "Ты помогаешь дешифрировать снимок Земли. Тебе дают классы цветов, на которые трассировка разложила участок снимка: средние значения каналов, долю участка, количество и средний размер областей. Назови, что вероятнее всего изображает каждый класс, - вода, хвойный лес, лиственный лес, луг, посевы, пашня, рубка, гарь, застройка, дорога, болото, снег, облака, тень и подобное. Учитывай роли каналов и место съёмки. Если два класса - одно и то же, дай им одно название. Если не понять, напиши «неясно». Названия - на {} языке, от одного до трёх слов. Ответь только JSON вида {{\"names\": [\"...\", ...]}} - по названию на класс в том же порядке, без пояснений.",
            if language == "English" { "английском" } else { "русском" }
        )
    } else {
        format!(
            "You help to interpret an image of the Earth. You are given the colour classes into which tracing split an area of the image: mean band values, share of the area, count and mean size of the areas. Name what each class most likely shows - water, coniferous forest, broadleaf forest, meadow, crops, ploughland, clearcut, burnt area, buildings, road, bog, snow, clouds, shadow and the like. Take into account the band roles and the place. If two classes are the same thing, give them one name. If it is unclear, write \"unclear\". Names in {language}, one to three words. Answer only with JSON of the form {{\"names\": [\"...\", ...]}} - one name per class in the same order, without explanations."
        )
    }
}

fn number(value: f64) -> String {
    if value.abs() >= 10.0 { format!("{value:.1}") } else { format!("{value:.3}") }
}

/// Строка таблицы классов.
pub struct Row {
    /// Средние по каналам.
    pub means: Vec<f64>,
    /// Доля участка, 0-1.
    pub share: f64,
    /// Областей.
    pub regions: usize,
    /// Средняя область, га.
    pub area_ha: f64,
    /// Название по правилам отражения.
    pub rule: Option<String>,
}

/// Текст вопроса. roles - роли каналов или None («nir@2» - канал второго
/// снимка), units - что за значения, place - (широта, долгота), month -
/// месяц.
pub fn question(rows: &[Row], roles: Option<&[Option<String>]>, units: &str, place: Option<(f64, f64)>, month: Option<u32>, ru: bool) -> String {
    let mut lines = Vec::new();
    if let Some((lat, lon)) = place {
        lines.push(if ru { format!("Место - широта {lat:.2}°, долгота {lon:.2}°.") } else { format!("Place - latitude {lat:.2}°, longitude {lon:.2}°.") });
    }
    if let Some(m) = month {
        lines.push(if ru { format!("Месяц съёмки - {m}.") } else { format!("Month of the image - {m}.") });
    }
    let count = rows.first().map_or(0, |r| r.means.len());
    let band = |n: usize| if ru { format!("канал {}", n + 1) } else { format!("band {}", n + 1) };
    let names: Vec<String> = (0..count).map(|n| roles.and_then(|r| r.get(n)).and_then(|r| r.as_deref()).map_or_else(|| band(n), |r| role_name(r, ru))).collect();
    lines.push(if ru { format!("Каналы - {}. Значения - {units}.", names.join(", ")) } else { format!("Bands - {}. Values - {units}.", names.join(", ")) });
    for (n, row) in rows.iter().enumerate() {
        let means: Vec<String> = row.means.iter().map(|v| number(*v)).collect();
        let mut text = if ru {
            format!("Класс {} - {}, доля участка {:.0} %, областей {}, средняя область {} га", n + 1, means.join(", "), 100.0 * row.share, row.regions, number(row.area_ha))
        } else {
            format!("Class {} - {}, share of the area {:.0} %, areas {}, mean area {} ha", n + 1, means.join(", "), 100.0 * row.share, row.regions, number(row.area_ha))
        };
        if let Some(rule) = &row.rule {
            text += &if ru { format!(", по правилам отражения - {rule}") } else { format!(", by the reflectance rules - {rule}") };
        }
        lines.push(text + ".");
    }
    lines.join("\n")
}

/// Названия из ответа модели: count строк или None. Берётся первый объект
/// JSON с полем names или первый список строк.
pub fn parse_names(text: &str, count: usize) -> Option<Vec<String>> {
    for (start, _) in text.char_indices().filter(|(_, c)| *c == '[' || *c == '{') {
        let mut stream = serde_json::Deserializer::from_str(&text[start..]).into_iter::<Value>();
        let Some(Ok(value)) = stream.next() else {
            continue;
        };
        let value = match value {
            Value::Object(mut map) => map.remove("names").unwrap_or(Value::Null),
            other => other,
        };
        let Value::Array(items) = value else {
            continue;
        };
        let names: Option<Vec<String>> = items.iter().map(|v| v.as_str().map(str::to_string)).collect();
        let Some(names) = names.filter(|n| !n.is_empty()) else {
            continue;
        };
        if names.len() < count {
            return None;
        }
        return Some(names.into_iter().take(count).map(|n| n.trim().chars().take(NAME_LIMIT).collect()).collect());
    }
    None
}
