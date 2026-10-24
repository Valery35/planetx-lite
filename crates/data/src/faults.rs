// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Активные разломы GEM Global Active Faults. Перенос `core/faults.py`
//! модуля.
//!
//! База GEM Global Active Faults (Styron, Pagani 2020), лицензия CC BY-SA
//! 4.0. Файл gem_active_faults_harmonized.geojson (около 10.6 МБ,
//! 13 696 линий) скачивается при включении строки. Цвет - вид смещения
//! по полю slip_type. Классы и цвета - выбор помощника модуля,
//! утверждает автор.

use crate::linesets::Line;

pub const URL: &str = "https://raw.githubusercontent.com/GEMScienceTools/gem-global-active-faults/master/geojson/gem_active_faults_harmonized.geojson";
pub const ATTRIBUTION: (&str, &str) =
    ("Active faults: GEM Global Active Faults, Styron and Pagani 2020, CC BY-SA 4.0", "https://github.com/GEMScienceTools/gem-global-active-faults");
pub const PAGE: &str = "https://doi.org/10.1177/8755293020944182";
/// Виды смещения: сброс, взброс и надвиг, сдвиг, срединный хребет,
/// складка, прочее.
pub const KINDS: [&str; 6] = ["normal", "reverse", "strike", "ridge", "fold", "other"];
/// Толщина линии, пиксели картинки тайла.
pub const WIDTH: f32 = 1.6;

/// Цвет вида смещения.
pub fn color(kind: &str) -> [u8; 4] {
    match kind {
        "normal" => [40, 120, 230, 255],
        "reverse" => [225, 45, 40, 255],
        "strike" => [30, 170, 70, 255],
        "ridge" => [240, 150, 30, 255],
        "fold" => [150, 90, 200, 255],
        _ => [120, 120, 120, 255],
    }
}

/// Разлом: вид, название, slip_type как в базе, скорость смещения
/// мм/год, падение в градусах, сторона падения, каталог источника.
#[derive(Debug, Clone, PartialEq)]
pub struct Fault {
    pub kind: &'static str,
    pub name: String,
    pub slip_type: String,
    pub rate: Option<f64>,
    pub dip: Option<f64>,
    pub dip_dir: String,
    pub catalog: String,
}

/// Вид смещения по полю slip_type базы.
pub fn kind_of(slip_type: &str) -> &'static str {
    let text = slip_type.to_lowercase();
    if text.is_empty() {
        "other"
    } else if text.starts_with("spreading") {
        "ridge"
    } else if text == "anticline" || text == "syncline" {
        "fold"
    } else if text.starts_with("normal") {
        "normal"
    } else if ["reverse", "subduction", "blind"].iter().any(|p| text.starts_with(p)) {
        "reverse"
    } else if ["dextral", "sinistral", "strike"].iter().any(|p| text.starts_with(p)) {
        "strike"
    } else {
        "other"
    }
}

/// Наиболее вероятное значение записи вида «(1.55,0.8,2.22)» или None.
pub fn first_value(value: Option<&serde_json::Value>) -> Option<f64> {
    match value? {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().trim_matches(|c| c == '(' || c == ')').split(',').next()?.trim().parse::<f64>().ok().filter(|v| v.is_finite()),
        _ => None,
    }
}

/// Разломы и их линии из байтов GeoJSON базы. Части MultiLineString идут
/// отдельными разломами с теми же полями.
pub fn parse(data: &[u8]) -> Result<(Vec<Fault>, Vec<Line>), String> {
    let json: serde_json::Value = serde_json::from_slice(data).map_err(|e| e.to_string())?;
    let mut faults = Vec::new();
    let mut lines = Vec::new();
    for feature in json.get("features").and_then(|f| f.as_array()).into_iter().flatten() {
        let geometry = feature.get("geometry").cloned().unwrap_or_default();
        let props = feature.get("properties").cloned().unwrap_or_default();
        let coords = geometry.get("coordinates").cloned().unwrap_or_default();
        let parts: Vec<serde_json::Value> = match geometry.get("type").and_then(|t| t.as_str()) {
            Some("LineString") => vec![coords],
            Some("MultiLineString") => coords.as_array().cloned().unwrap_or_default(),
            _ => continue,
        };
        let text = |k: &str| props.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let slip = text("slip_type");
        let name = if text("name").is_empty() { text("fs_name") } else { text("name") };
        let rate = first_value(props.get("net_slip_rate")).or_else(|| first_value(props.get("strike_slip_rate")));
        let dip = first_value(props.get("average_dip"));
        for part in parts {
            let points: Vec<(f64, f64)> = part
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| {
                    let p = p.as_array()?;
                    Some((p.get(1)?.as_f64()?, p.first()?.as_f64()?))
                })
                .collect();
            if points.len() < 2 {
                continue;
            }
            let kind = kind_of(&slip);
            lines.push(Line::new(points, color(kind), WIDTH));
            faults.push(Fault { kind, name: name.clone(), slip_type: slip.clone(), rate, dip, dip_dir: text("dip_dir"), catalog: text("catalog_name") });
        }
    }
    Ok((faults, lines))
}
