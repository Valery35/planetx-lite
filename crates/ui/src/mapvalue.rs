// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Значение карты картинками под щелчком «Определить объекты»: перенос
//! `_ask_tilemap`, `_identify_field` и `_identify_tilemap`
//! (`ui/globe/layers.py`) модуля.
//!
//! У карты значений и карты классами берётся пиксель тайла глубшего
//! уровня карты, у почв SoilGrids - ответ GetFeatureInfo службы. Запрос
//! разовый, ответ приходит позже, окно «Объекты» показывает «загрузка…».

use data::rastermaps::{self, MapValue};

use crate::i18n::{Lang, tr};
use crate::identify::Feature;

/// Что узнаётся: карта, точка и пиксель тайла (None - ответ службы).
pub struct Ask {
    pub key: &'static str,
    pub at: (f64, f64),
    pixel: Option<(u32, u32)>,
    /// Уровень карты запроса Macrostrat.
    zoom: u32,
    request: Box<dyn net::Request>,
}

/// Итог: значение словами или «нет данных».
pub enum Answer {
    Loading,
    Text(String),
    /// Подразделения геологической карты в точке или ошибка запроса.
    Geology(Result<Vec<data::geology::Unit>, String>),
}

/// Адрес запроса значения карты key в точке и пиксель тайла. None -
/// у карты значения под щелчком нет.
fn query(key: &str, lat: f64, lon: f64, zoom: u32) -> Option<(String, Option<(u32, u32)>)> {
    if key == "geology" {
        return Some((data::geology::query_url(lat, lon, f64::from(zoom)), None));
    }
    if let Some(f) = rastermaps::field(key) {
        let ((x, y), pixel) = rastermaps::pixel_of(lat, lon, f.max_level);
        return Some((net::template_url(f.url, (f.max_level, x, y)), Some(pixel)));
    }
    let m = rastermaps::tile_map(key)?;
    if let Some(url) = rastermaps::query_url(m, lat, lon) {
        return Some((url, None));
    }
    let ((x, y), pixel) = rastermaps::pixel_of(lat, lon, m.max_level);
    Some((net::template_url(rastermaps::map_template(m, m.max_level), (m.max_level, x, y)), Some(pixel)))
}

impl Ask {
    /// Начать запрос значения карты key в точке. None - у карты значения
    /// под щелчком нет.
    pub fn start(key: &'static str, lat: f64, lon: f64, zoom: u32, request: Box<dyn net::Request>) -> Option<Ask> {
        let (url, pixel) = query(key, lat, lon, zoom)?;
        request.start(&url);
        Some(Ask { key, at: (lat, lon), pixel, zoom, request })
    }

    /// Пришедший ответ, None - ещё нет. Пустой ответ Macrostrat
    /// повторяется на уровень мельче, как `_geology_arrived` модуля.
    pub fn take(&mut self, lang: Lang) -> Option<Answer> {
        let answer = self.request.take()?;
        if self.key == "geology" {
            let units = answer.and_then(|b| data::geology::parse_query(&b));
            if let Ok(found) = &units
                && found.is_empty()
                && let Some(z) = data::geology::coarser(self.zoom)
            {
                self.zoom = z;
                self.request.start(&data::geology::query_url(self.at.0, self.at.1, f64::from(z)));
                return None;
            }
            return Some(Answer::Geology(units));
        }
        Some(Answer::Text(match answer {
            Err(e) if e.contains("404") => tr(lang, "no data").to_string(),
            Err(e) => e,
            Ok(bytes) => value_text(lang, self.key, self.pixel, &bytes).unwrap_or_else(|| tr(lang, "no data").to_string()),
        }))
    }
}

/// Значение из ответа: пиксель тайла или ответ GetFeatureInfo.
fn value_text(lang: Lang, key: &str, pixel: Option<(u32, u32)>, bytes: &[u8]) -> Option<String> {
    if let Some(f) = rastermaps::field(key) {
        let (col, row) = pixel?;
        let tile = net::decode_tile(bytes).ok()?;
        let v = rastermaps::value_at(f, &tile.rgba, col, row)?;
        let unit = field_texts(key).map_or("", |t| t.2);
        let sign = if f.stops.first().is_some_and(|s| s.0 < 0.0) && v > 0.0 { "+" } else { "" };
        return Some(format!("{sign}{v:.prec$} {}", tr(lang, unit), prec = f.digits));
    }
    let m = rastermaps::tile_map(key)?;
    match pixel {
        None => {
            let v = rastermaps::parse_query(m, bytes)?;
            Some(format!("{v:.prec$} {}", m.unit, prec = m.digits).trim().to_string())
        }
        Some((col, row)) => {
            let tile = net::decode_tile(bytes).ok()?;
            let at = ((row.min(255) * 256 + col.min(255)) * 4) as usize;
            let px = tile.rgba.get(at..at + 4)?;
            match rastermaps::value_of(m, [px[0], px[1], px[2], px[3]])? {
                MapValue::Class(code) => Some(tr(lang, class_name(code)).to_string()),
                MapValue::Percent(p) => Some(format!("{p} %")),
            }
        }
    }
}

/// Тексты карты значений: группа окна, строка, единица, пояснения.
type FieldText = (&'static str, &'static str, &'static str, &'static [(&'static str, &'static str)]);

/// Карта значений: группа окна, строка, единица, строки пояснений,
/// `field_texts` модуля. Ключи перевода - английские.
fn field_texts(key: &str) -> Option<FieldText> {
    Some(match key {
        "emag" => ("Magnetic anomaly", "Field anomaly", "nT", &[("Grid height", "4 km above the ellipsoid"), ("Dataset", "EMAG2v3, NOAA NCEI")]),
        "seafloor_age" => ("Ocean floor", "Crust age", "Ma", &[("Dataset", "Seton et al. 2020, EarthByte")]),
        "gravity" => ("Gravity", "Free-air anomaly", "mGal", &[("Dataset", "XGM2019e, Zingerle et al. 2020")]),
        "bouguer" => ("Gravity", "Bouguer anomaly", "mGal", &[("Dataset", "XGM2019e, ETOPO 2022"), ("Rock density", "2670 kg/m³")]),
        "heatflow" => ("Heat flow", "Heat flow|value", "mW/m²", &[("Dataset", "IHFC 2024, Petrunin et al. 2026")]),
        _ => return None,
    })
}

/// Название класса карты по коду, `class_names` модуля.
pub fn class_name(code: &str) -> &'static str {
    match code {
        "su" => "Unconsolidated sediments",
        "ss" => "Siliciclastic sedimentary rocks",
        "sm" => "Mixed sedimentary rocks",
        "sc" => "Carbonate sedimentary rocks",
        "ev" => "Evaporites",
        "py" => "Pyroclastics",
        "va" => "Acid volcanic rocks",
        "vi" => "Intermediate volcanic rocks",
        "vb" => "Basic volcanic rocks",
        "pa" => "Acid plutonic rocks",
        "pi" => "Intermediate plutonic rocks",
        "pb" => "Basic plutonic rocks",
        "mt" => "Metamorphic rocks",
        "ig" => "Ice and glaciers",
        "permanent" => "Permanent water",
        "new_permanent" => "New permanent water",
        "lost_permanent" => "Lost permanent water",
        "seasonal" => "Seasonal water",
        "new_seasonal" => "New seasonal water",
        "lost_seasonal" => "Lost seasonal water",
        "seasonal_to_permanent" => "Seasonal to permanent",
        "permanent_to_seasonal" => "Permanent to seasonal",
        "ephemeral_permanent" => "Ephemeral permanent water",
        "ephemeral_seasonal" => "Ephemeral seasonal water",
        "tree" => "Forest",
        "shrub" => "Shrubland",
        "grass" => "Grassland",
        "crop" => "Cropland",
        "built" => "Built-up",
        "bare" => "Bare and sparse vegetation",
        "snow" => "Snow and ice",
        "water" => "Permanent water bodies",
        "wetland" => "Herbaceous wetland",
        "mangrove" => "Mangroves",
        "moss" => "Moss and lichen",
        _ => "",
    }
}

/// Заголовок шкалы карты, `field_titles` и `class_titles` модуля.
pub fn title(key: &str) -> &'static str {
    match key {
        "emag" => "Magnetic anomaly, nT",
        "seafloor_age" => "Ocean floor age, Ma",
        "gravity" => "Gravity anomaly, mGal",
        "bouguer" => "Bouguer anomaly, mGal",
        "heatflow" => "Heat flow, mW/m²",
        "water_occurrence" => "Water occurrence 1984-2021, %",
        "soil_clay" => "Clay in soil 0-5 cm, %",
        "soil_sand" => "Sand in soil 0-5 cm, %",
        "soil_soc" => "Soil organic carbon 0-5 cm, g/kg",
        "soil_ph" => "Soil pH 0-5 cm, in water",
        "glim" => "GLiM lithology",
        "water_transitions" => "Water transitions 1984-2021, JRC",
        "worldcover" => "ESA WorldCover 2021 land cover",
        _ => "",
    }
}

/// Группа карты окна «Объекты».
pub fn group(lang: Lang, key: &str, answer: &Answer, at: (f64, f64)) -> Option<(String, Vec<Feature>)> {
    if key == "geology" {
        return Some((tr(lang, "Geology").to_string(), geology_features(lang, at, answer)));
    }
    let value = match answer {
        Answer::Loading => tr(lang, "loading…").to_string(),
        Answer::Text(t) => t.clone(),
        Answer::Geology(_) => String::new(),
    };
    let row = |name: &'static str| (tr(lang, name).to_string(), value.clone());
    if let Some((group, line, _, notes)) = field_texts(key) {
        let mut values = vec![row("Value")];
        if matches!(answer, Answer::Text(_)) {
            values.extend(notes.iter().map(|(a, b)| (tr(lang, a).to_string(), tr(lang, b).to_string())));
        }
        return Some((tr(lang, group).to_string(), vec![(tr(lang, line).to_string(), values)]));
    }
    let m = rastermaps::tile_map(key)?;
    let classes = matches!(m.reading, rastermaps::Reading::Classes(_));
    let group = match key {
        "glim" => "Lithology",
        "worldcover" => "Land cover",
        "water_occurrence" | "water_transitions" => "Water",
        _ => "Soil",
    };
    Some((tr(lang, group).to_string(), vec![(tr(lang, title(key)).to_string(), vec![row(if classes { "Class" } else { "Value" })])]))
}

/// Объекты группы «Геология» для точки at, `_identify_geology` модуля.
fn geology_features(lang: Lang, at: (f64, f64), answer: &Answer) -> Vec<Feature> {
    let units = match answer {
        Answer::Geology(Ok(u)) => u,
        Answer::Geology(Err(e)) => return vec![(tr(lang, "Macrostrat did not answer").to_string(), vec![(tr(lang, "Error").to_string(), e.clone())])],
        _ => return vec![(tr(lang, "Asking Macrostrat…").to_string(), Vec::new())],
    };
    if units.is_empty() {
        return vec![(tr(lang, "The map is empty at the point").to_string(), Vec::new())];
    }
    units
        .iter()
        .map(|unit| {
            let (names, ages) = data::geology::age_text(unit);
            let mut values = Vec::new();
            let mut row = |name: &'static str, value: &str| {
                if !value.is_empty() {
                    values.push((tr(lang, name).to_string(), value.to_string()));
                }
            };
            row("Age", &names);
            row("Age, Ma", &ages);
            row("Lithology", &unit.lith);
            row("Formation", &unit.strat_name);
            row("Description", &unit.descrip);
            row("Note", &unit.comments);
            let source = [unit.source.as_str(), unit.authors.as_str(), unit.year.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(", ");
            row("Map", &source);
            row("Map link", &unit.url);
            row("Macrostrat page", &data::geology::page_url(at.0, at.1));
            let title = if !unit.name.is_empty() { unit.name.clone() } else if !unit.lith.is_empty() { unit.lith.clone() } else { tr(lang, "Untitled").to_string() };
            (title, values)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Answer, group, query};
    use crate::i18n::Lang;

    #[test]
    fn query_kinds() {
        // Карта значений - тайл глубшего уровня и пиксель точки.
        let (url, pixel) = query("emag", 58.0, 56.25, 8).unwrap_or_default();
        assert!(url.ends_with("emag/5/21/9.png"), "{url}");
        assert!(pixel.is_some());
        // Почвы - ответ службы.
        let (url, pixel) = query("soil_clay", 58.0, 56.25, 8).unwrap_or_default();
        assert!(url.contains("GetFeatureInfo") && pixel.is_none());
        // У наложений значения нет.
        assert!(query("hiking", 0.0, 0.0, 8).is_none());
        let g = group(Lang::EN, "glim", &Answer::Loading, (0.0, 0.0)).unwrap_or_default();
        assert_eq!(g.0, "Lithology");
        assert_eq!(g.1[0].1[0].0, "Class");
    }
}
