// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Суда Балтики вживую по AIS Digitraffic. Перенос `core/vessels.py`
//! модуля.
//!
//! Положения - открытый ответ Fintraffic Digitraffic ais/v1/locations
//! (GeoJSON, MMSI, скорость и курс по земле, курс корпуса, время),
//! сведения - ais/v1/vessels (название, позывной, тип AIS, назначение,
//! размеры от точки антенны A, B, C, D). Условия - CC BY 4.0. Охват -
//! Финский и Ботнический заливы, Балтика у Финляндии. Заголовок запросов -
//! [`HEADERS`].
//!
//! Модель - условный корпус длиной 1 вдоль оси Y модели, его растягивает
//! масштаб по размерам судна ([`unit_ship_parts`], [`ship_size`]).

use std::collections::BTreeMap;

use serde_json::Value;

pub use crate::movers::KNOT;
use crate::movers::Sample;
use crate::trains::{Part, integer, number, opaque, string};

pub const LOCATIONS_URL: &str = "https://meri.digitraffic.fi/api/ais/v1/locations";
pub const VESSELS_URL: &str = "https://meri.digitraffic.fi/api/ais/v1/vessels";
pub const ATTRIBUTION: (&str, &str) = ("Ships: Fintraffic / digitraffic.fi, CC BY 4.0", "https://www.digitraffic.fi/en/marine-traffic/");
/// Заголовки запросов Digitraffic, `HEADERS` окна модуля.
pub const HEADERS: [(&str, &str); 1] = crate::trains::HEADERS;
/// Секунд между запросами положений.
pub const POLL: f64 = 20.0;
/// Секунд между запросами сведений о судах.
pub const INFO_POLL: f64 = 600.0;
/// Секунд хода своим курсом после положения.
pub const EXTRAPOLATE: f64 = 120.0;
/// Положение старше этого не показывается, секунды.
pub const STALE: f64 = 1800.0;
/// Значения AIS «нет данных»: скорость по земле, курс, азимут корпуса.
pub const NO_SPEED: f64 = 102.3;
pub const NO_COURSE: f64 = 360.0;
pub const NO_HEADING: f64 = 511.0;
/// Группы типов AIS: ключ, цвет 0-1, длина по умолчанию в метрах. Цвета -
/// привычные для карт AIS, выбор помощника модуля.
pub const GROUPS: [(&str, [f64; 3], f64); 8] = [
    ("passenger", [0.20, 0.50, 0.92], 150.0),
    ("cargo", [0.22, 0.68, 0.30], 120.0),
    ("tanker", [0.86, 0.20, 0.20], 150.0),
    ("fishing", [0.96, 0.58, 0.14], 25.0),
    ("service", [0.10, 0.78, 0.80], 30.0),
    ("fast", [0.96, 0.86, 0.20], 30.0),
    ("pleasure", [0.72, 0.36, 0.86], 12.0),
    ("other", [0.78, 0.78, 0.78], 30.0),
];
pub const WHITE: [f64; 3] = [0.94, 0.94, 0.92];
pub const DARK: [f64; 3] = [0.18, 0.19, 0.21];

/// Группа по коду типа AIS.
pub fn group(ship_type: i64) -> &'static str {
    match ship_type {
        60..=69 => "passenger",
        70..=79 => "cargo",
        80..=89 => "tanker",
        30 => "fishing",
        31..=35 | 50..=55 | 58 | 59 => "service",
        40..=49 => "fast",
        36 | 37 => "pleasure",
        _ => "other",
    }
}

fn group_row(key: &str) -> (&'static str, [f64; 3], f64) {
    GROUPS.iter().copied().find(|g| g.0 == key).unwrap_or(GROUPS[GROUPS.len() - 1])
}

/// Цвет группы 0-1, неизвестной - цвет «other».
pub fn group_color(key: &str) -> [f64; 3] {
    group_row(key).1
}

/// Длина судна группы по умолчанию, метры.
pub fn default_length(key: &str) -> f64 {
    group_row(key).2
}

/// Положения из GeoJSON ais/v1/locations байтами JSON. Ключ - MMSI.
/// Ответ не JSON - пустой список.
pub fn parse_locations(data: &[u8]) -> Vec<Sample<i64>> {
    serde_json::from_slice::<Value>(data).map(|json| parse_locations_json(&json)).unwrap_or_default()
}

/// Положения из уже разобранного ответа, как [`parse_locations`].
pub fn parse_locations_json(data: &Value) -> Vec<Sample<i64>> {
    let Some(features) = data.get("features").and_then(Value::as_array) else { return Vec::new() };
    features.iter().filter_map(location).collect()
}

fn location(feature: &Value) -> Option<Sample<i64>> {
    let props = feature.get("properties").filter(|p| p.is_object());
    let coords = feature.pointer("/geometry/coordinates")?.as_array()?;
    // MMSI свойств главнее, null в свойствах - нет MMSI.
    let mmsi = match props.and_then(|p| p.get("mmsi")) {
        Some(value) => integer(Some(value))?,
        None => integer(feature.get("mmsi"))?,
    };
    let prop = |key: &str| number(props.and_then(|p| p.get(key)));
    let stamp = prop("timestampExternal")?;
    let lon = number(coords.first())?;
    let lat = number(coords.get(1))?;
    if !(-90.0..=90.0).contains(&lat) {
        return None;
    }
    let speed = prop("sog").filter(|v| *v < NO_SPEED).map_or(0.0, |v| v * KNOT);
    let course = prop("cog").filter(|v| *v < NO_COURSE).unwrap_or(f64::NAN);
    let heading = prop("heading").filter(|v| *v < 360.0).unwrap_or(f64::NAN);
    Some(Sample { speed, course, heading, ..Sample::new(mmsi, stamp / 1000.0, lat, lon) })
}

/// Сведения о судне: название, позывной, IMO, тип AIS, назначение,
/// длина и ширина в метрах (0 - нет).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Info {
    pub name: String,
    pub call: String,
    pub imo: i64,
    pub ship_type: i64,
    pub destination: String,
    pub length: f64,
    pub width: f64,
}

impl Info {
    /// Группа по типу AIS.
    pub fn group(&self) -> &'static str {
        group(self.ship_type)
    }
}

/// {MMSI: сведения} из ответа ais/v1/vessels байтами JSON.
pub fn parse_vessels(data: &[u8]) -> BTreeMap<i64, Info> {
    serde_json::from_slice::<Value>(data).map(|json| parse_vessels_json(&json)).unwrap_or_default()
}

/// Сведения из уже разобранного ответа, как [`parse_vessels`].
pub fn parse_vessels_json(data: &Value) -> BTreeMap<i64, Info> {
    let mut out = BTreeMap::new();
    for item in data.as_array().into_iter().flatten() {
        let Some(mmsi) = integer(item.get("mmsi")) else { continue };
        let [a, b, c, d] = ["A", "B", "C", "D"].map(|k| number(item.get(format!("referencePoint{k}").as_str())).unwrap_or(0.0));
        out.insert(
            mmsi,
            Info {
                name: string(item.get("name")),
                call: string(item.get("callSign")),
                imo: integer(item.get("imo")).unwrap_or(0),
                ship_type: integer(item.get("shipType")).unwrap_or(0),
                destination: string(item.get("destination")),
                length: a + b,
                width: c + d,
            },
        );
    }
    out
}

/// Подпись судна: название или MMSI.
pub fn label(key: i64, info: Option<&Info>) -> String {
    match info {
        Some(info) if !info.name.is_empty() => info.name.clone(),
        _ => key.to_string(),
    }
}

/// (ширина, длина, высота) модели в метрах: размеры AIS, без них - длина
/// группы, ширина - шестая часть длины.
pub fn ship_size(info: Option<&Info>) -> (f64, f64, f64) {
    let key = info.map_or("other", Info::group);
    let length = info.map(|i| i.length).filter(|l| (5.0..=500.0).contains(l)).unwrap_or_else(|| default_length(key));
    let width = info.map(|i| i.width).filter(|w| (1.0..=80.0).contains(w)).unwrap_or(length / 6.0);
    (width, length, width)
}

/// Треугольники призмы над многоугольником points (против часовой
/// стрелки) от z0 до z1, крышка веером.
fn prism(points: &[(f64, f64)], z0: f64, z1: f64) -> Vec<[f64; 3]> {
    let mut out = Vec::new();
    let Some(&a) = points.first() else { return out };
    for pair in points.get(1..).unwrap_or_default().windows(2) {
        let (b, c) = (pair[0], pair[1]);
        out.extend([[a.0, a.1, z1], [b.0, b.1, z1], [c.0, c.1, z1]]);
        out.extend([[a.0, a.1, z0], [c.0, c.1, z0], [b.0, b.1, z0]]);
    }
    for (k, &a) in points.iter().enumerate() {
        let b = points[(k + 1) % points.len()];
        out.extend([[a.0, a.1, z0], [b.0, b.1, z0], [b.0, b.1, z1], [a.0, a.1, z0], [b.0, b.1, z1], [a.0, a.1, z1]]);
    }
    out
}

/// Условное судно длиной 1 (ось Y, нос - +Y), шириной 1, высотой в долях
/// ширины: корпус цвета группы с заострённым носом, надстройка белая
/// у кормы, ватерлиния тёмная.
pub fn unit_ship_parts(color: [f64; 3]) -> Vec<Part> {
    let hull = [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.28), (0.0, 0.5), (-0.5, 0.28)];
    let house = [(-0.38, -0.46), (0.38, -0.46), (0.38, -0.22), (-0.38, -0.22)];
    vec![
        Part { positions: prism(&hull, 0.0, 0.08), rgba: opaque(DARK) },
        Part { positions: prism(&hull, 0.08, 0.35), rgba: opaque(color) },
        Part { positions: prism(&house, 0.35, 0.95), rgba: opaque(WHITE) },
    ]
}
