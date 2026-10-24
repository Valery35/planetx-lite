// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Границы литосферных плит PB2002 и подводные кабели OpenStreetMap -
//! файлы данных модуля. Перенос `core/plates.py` и `core/cables.py`.
//!
//! Модель Peter Bird 2003, данные в GIS - Hugo Ahlenius, Nordpil, Open
//! Data Commons Attribution 1.0. Кабели - OpenStreetMap под ODbL,
//! линии связи и силовые кабели под водой. Файлы data/plates.json
//! и data/cables.json собирают инструменты модуля, Lite берёт их из
//! хранилища модуля по сети и в себе не несёт.

use crate::linesets::Line;

/// Файлы данных модуля.
pub const PLATES_URL: &str = "https://raw.githubusercontent.com/Valery35/planetx/main/planetx/data/plates.json";
pub const CABLES_URL: &str = "https://raw.githubusercontent.com/Valery35/planetx/main/planetx/data/cables.json";
pub const PLATES_ATTRIBUTION: (&str, &str) = ("Plate boundaries: Bird 2003, PB2002, Hugo Ahlenius, Nordpil", "https://github.com/fraxen/tectonicplates");
pub const CABLES_ATTRIBUTION: (&str, &str) = ("Submarine cables: © OpenStreetMap contributors", "https://www.openstreetmap.org/copyright");
/// Толщина линий, пиксели картинки тайла.
pub const PLATE_WIDTH: f32 = 2.5;
pub const CABLE_WIDTH: f32 = 2.0;

/// Группа класса шага границы: раздвиг, сдвиг, схождение.
pub fn group(code: &str) -> Option<&'static str> {
    match code {
        "OSR" | "CRB" => Some("divergent"),
        "OTF" | "CTF" => Some("transform"),
        "SUB" | "OCB" | "CCB" => Some("convergent"),
        _ => None,
    }
}

/// Цвет группы границы.
pub fn plate_color(group: &str) -> [u8; 4] {
    match group {
        "divergent" => [230, 70, 50, 255],
        "transform" => [60, 200, 90, 255],
        _ => [70, 140, 255, 255],
    }
}

/// Граница: класс шага, пара плит «AF-AN», скорость мм/год.
#[derive(Debug, Clone, PartialEq)]
pub struct Boundary {
    pub code: String,
    pub pair: String,
    pub speed: f64,
}

/// Плита: код, название, место подписи.
#[derive(Debug, Clone, PartialEq)]
pub struct Plate {
    pub code: String,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

fn points(item: &serde_json::Value) -> Vec<(f64, f64)> {
    item.get("points")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let p = p.as_array()?;
            Some((p.first()?.as_f64()?, p.get(1)?.as_f64()?))
        })
        .collect()
}

fn text(item: &serde_json::Value, key: &str) -> String {
    match item.get(key) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// Границы, их линии и плиты.
pub type Plates = (Vec<Boundary>, Vec<Line>, Vec<Plate>);

/// Границы, их линии и плиты из текста data/plates.json.
pub fn parse_plates(data: &[u8]) -> Result<Plates, String> {
    let json: serde_json::Value = serde_json::from_slice(data).map_err(|e| e.to_string())?;
    let mut bounds = Vec::new();
    let mut lines = Vec::new();
    for item in json.get("boundaries").and_then(|b| b.as_array()).into_iter().flatten() {
        let code = text(item, "class");
        let pts = points(item);
        let Some(g) = group(&code) else {
            continue;
        };
        if pts.len() < 2 {
            continue;
        }
        lines.push(Line::new(pts, plate_color(g), PLATE_WIDTH));
        bounds.push(Boundary { code, pair: text(item, "pair"), speed: item.get("speed").and_then(|s| s.as_f64()).unwrap_or(0.0) });
    }
    let plates = json
        .get("plates")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let p = p.as_array()?;
            Some(Plate { code: p.first()?.as_str()?.to_string(), name: p.get(1)?.as_str()?.to_string(), lat: p.get(2)?.as_f64()?, lon: p.get(3)?.as_f64()? })
        })
        .collect();
    Ok((bounds, lines, plates))
}

/// Кабель: вид (telecom, power), название, владелец, напряжение из OSM,
/// объект OSM «way/номер».
#[derive(Debug, Clone, PartialEq)]
pub struct Cable {
    pub kind: &'static str,
    pub name: String,
    pub operator: String,
    pub voltage: String,
    pub osm: String,
}

/// Цвет вида кабеля.
pub fn cable_color(kind: &str) -> [u8; 4] {
    if kind == "power" { [190, 70, 210, 255] } else { [0, 190, 200, 255] }
}

/// Кабели, их линии и дата выборки из текста data/cables.json.
pub fn parse_cables(data: &[u8]) -> Result<(Vec<Cable>, Vec<Line>, String), String> {
    let json: serde_json::Value = serde_json::from_slice(data).map_err(|e| e.to_string())?;
    let mut cables = Vec::new();
    let mut lines = Vec::new();
    for item in json.get("cables").and_then(|b| b.as_array()).into_iter().flatten() {
        let kind = match item.get("kind").and_then(|k| k.as_str()) {
            Some("telecom") => "telecom",
            Some("power") => "power",
            _ => continue,
        };
        let pts = points(item);
        if pts.len() < 2 {
            continue;
        }
        lines.push(Line::new(pts, cable_color(kind), CABLE_WIDTH));
        cables.push(Cable { kind, name: text(item, "name"), operator: text(item, "operator"), voltage: text(item, "voltage"), osm: text(item, "osm") });
    }
    Ok((cables, lines, text(&json, "date")))
}

/// Наибольшее напряжение тега voltage в кВ или None, `infra.kilovolts`
/// модуля. Значения через точку с запятой - цепи разного напряжения на
/// одной опоре, без «kV» - вольты.
pub fn kilovolts(voltage: &str) -> Option<f64> {
    let mut best: Option<f64> = None;
    for part in voltage.replace(',', ";").split(';') {
        let part = part.trim().to_lowercase().replace(' ', "");
        let (scale, number) = match part.strip_suffix("kv") {
            Some(n) => (1.0, n.to_string()),
            None => (0.001, part),
        };
        // Целое или десятичная дробь с цифрами по обе стороны точки.
        let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
        let ok = match number.split_once('.') {
            Some((a, b)) => digits(a) && digits(b),
            None => digits(&number),
        };
        let Some(value) = number.parse::<f64>().ok().filter(|_| ok) else {
            continue;
        };
        let kv = value * scale;
        best = Some(best.map_or(kv, |b: f64| b.max(kv)));
    }
    best
}
