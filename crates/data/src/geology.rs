// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Геологическая карта Macrostrat. Перенос `core/geology.py` модуля.
//!
//! Macrostrat (Peters и соавторы, университет Висконсин-Мэдисон) сводит
//! геологические карты мира разных масштабов в одну, данные под CC BY
//! 4.0. Тайлы PNG 512 × 512 без ключа, цвет - возраст пород. Сведения
//! о породе в точке - запрос mobile/map_query: подразделение,
//! литология, возраст и карта-источник. Глубже уровня самой подробной
//! карты места тайл приходит пустым, на его месте рисуется предок.

/// Тайлы карты.
pub const TILES: &str = "https://tiles.macrostrat.org/carto/{z}/{x}/{y}.png";
pub const MAX_LEVEL: u32 = 14;
pub const ATTRIBUTION: (&str, &str) = ("Geology: Macrostrat, CC BY 4.0", "https://macrostrat.org/");
pub const OPACITY: f64 = 0.75;
/// Глубже карты места map_query отвечает пустой записью, тогда запрос
/// повторяется на столько уровней мельче.
pub const QUERY_STEP: u32 = 3;

/// Адрес сведений о породе в точке на уровне карты z.
pub fn query_url(lat: f64, lon: f64, z: f64) -> String {
    let z = z.round().clamp(0.0, 16.0) as u32;
    format!("https://macrostrat.org/api/v2/mobile/map_query?lat={lat:.5}&lng={lon:.5}&z={z}")
}

/// Уровень следующего запроса, когда на уровне z карта в точке пуста:
/// на QUERY_STEP мельче, None - мельче спрашивать некуда.
pub fn coarser(z: u32) -> Option<u32> {
    (z > 1).then(|| z.saturating_sub(QUERY_STEP).max(1))
}

/// Страница карты Macrostrat у точки.
pub fn page_url(lat: f64, lon: f64) -> String {
    format!("https://macrostrat.org/map/loc/{lon:.4}/{lat:.4}")
}

/// Подразделение карты в точке.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Unit {
    pub name: String,
    pub strat_name: String,
    pub lith: String,
    pub descrip: String,
    pub comments: String,
    /// Названия верхней и нижней границ возраста.
    pub top: String,
    pub bottom: String,
    /// Возраст границ, млн лет.
    pub top_age: Option<f64>,
    pub bottom_age: Option<f64>,
    pub source: String,
    pub authors: String,
    pub year: String,
    pub url: String,
}

/// Подразделения карты в точке из ответа map_query. Пустой список -
/// карта в точке ничего не показывает. Ответ не JSON - ошибка.
pub fn parse_query(data: &[u8]) -> Result<Vec<Unit>, String> {
    let answer: serde_json::Value = serde_json::from_slice(data).map_err(|e| e.to_string())?;
    let items = answer.pointer("/success/data/burwell").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let text = |v: &serde_json::Value, k: &str| -> String {
        match v.get(k) {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Number(n)) => n.to_string(),
            _ => String::new(),
        }
    };
    let age = |v: &serde_json::Value, k: &str| -> Option<f64> {
        match v.get(k) {
            Some(serde_json::Value::Number(n)) => n.as_f64(),
            Some(serde_json::Value::String(s)) => s.parse().ok(),
            _ => None,
        }
    };
    Ok(items
        .iter()
        .filter(|i| !(text(i, "name").is_empty() && text(i, "lith").is_empty() && text(i, "t_int_name").is_empty()))
        .map(|i| {
            let r = i.get("ref").cloned().unwrap_or_default();
            Unit {
                name: text(i, "name"),
                strat_name: text(i, "strat_name"),
                lith: text(i, "lith"),
                descrip: text(i, "descrip"),
                comments: text(i, "comments"),
                top: text(i, "t_int_name"),
                bottom: text(i, "b_int_name"),
                top_age: age(i, "t_int_age"),
                bottom_age: age(i, "b_int_age"),
                source: text(&r, "name"),
                authors: text(&r, "authors"),
                year: text(&r, "ref_year"),
                url: text(&r, "url"),
            }
        })
        .collect())
}

/// Число, как `{:g}` модуля после округления до десятых.
fn short(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r == r.trunc() { format!("{}", r as i64) } else { format!("{r}") }
}

/// Возраст подразделения: названия интервалов и пределы в млн лет без
/// единицы, например («Late Permian», «274.4-251.9»).
pub fn age_text(unit: &Unit) -> (String, String) {
    let names = if unit.bottom.is_empty() || unit.bottom == unit.top { unit.top.clone() } else { format!("{} - {}", unit.bottom, unit.top) };
    match (unit.top_age, unit.bottom_age) {
        (Some(low), Some(high)) => (names, format!("{}-{}", short(high), short(low))),
        _ => (names, String::new()),
    }
}
