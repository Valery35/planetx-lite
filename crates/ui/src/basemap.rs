// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Основа глобуса: космоснимки Esri или карта OpenStreetMap.
//!
//! Обе встроены, как в модуле. Основа по умолчанию - Esri World Imagery,
//! решение автора от 22 октября 2026 года, как в модуле. Подпись
//! источника основы всегда на экране.

use net::Source;

use crate::i18n::Lang;

/// Основа глобуса.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Basemap {
    #[default]
    Esri,
    Osm,
}

impl Basemap {
    pub const ALL: [Basemap; 2] = [Basemap::Esri, Basemap::Osm];

    pub fn source(self) -> Source {
        match self {
            Basemap::Esri => net::ESRI,
            Basemap::Osm => net::OSM,
        }
    }

    /// Название в выборе основы.
    pub fn name(self, lang: Lang) -> &'static str {
        match (self, lang) {
            (Basemap::Esri, Lang::Ru) => "Снимки Esri World Imagery",
            (Basemap::Esri, Lang::En) => "Esri World Imagery",
            (Basemap::Osm, Lang::Ru) => "Карта OpenStreetMap",
            (Basemap::Osm, Lang::En) => "OpenStreetMap map",
        }
    }

    /// Подпись источника и ссылка на условия. Подпись Esri - как
    /// в описании слоя в ArcGIS Online, сверено модулем 26 сентября
    /// 2026 года, ссылка - Esri Master License Agreement.
    pub fn attribution(self, lang: Lang) -> (&'static str, &'static str) {
        match (self, lang) {
            (Basemap::Esri, _) => (
                "Esri, Vantor, Earthstar Geographics, and the GIS User Community",
                "https://www.esri.com/en-us/legal/terms/full-master-agreement",
            ),
            (Basemap::Osm, Lang::Ru) => ("© участники OpenStreetMap", "https://www.openstreetmap.org/copyright"),
            (Basemap::Osm, Lang::En) => ("© OpenStreetMap contributors", "https://www.openstreetmap.org/copyright"),
        }
    }
}
