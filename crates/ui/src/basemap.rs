// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Основа глобуса: космоснимки Esri, карта OpenStreetMap и готовые
//! подосновы модуля.
//!
//! Esri и OpenStreetMap встроены, как в модуле. Основа по умолчанию -
//! Esri World Imagery, решение автора от 22 октября 2026 года, как
//! в модуле. Готовые подосновы - `READY` из `core/basemap.py` модуля без
//! группы «с предупреждением»: Google, Bing, Яндекс и 2ГИС модуль ставит
//! только через окно согласия с их условиями. Подпись источника основы
//! всегда на экране.

use net::Source;

use crate::i18n::{Lang, tr};

/// Основа глобуса.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Basemap {
    #[default]
    Esri,
    Osm,
    /// Готовая подоснова, номер в [`READY`].
    Ready(usize),
}

/// Группа готовых подоснов, `READY_GROUPS` модуля.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadyGroup {
    Osm,
    Esri,
    NasaUsgs,
}

impl ReadyGroup {
    pub const ALL: [ReadyGroup; 3] = [ReadyGroup::Osm, ReadyGroup::Esri, ReadyGroup::NasaUsgs];

    /// Название подменю.
    pub fn name(self) -> &'static str {
        match self {
            ReadyGroup::Osm => "OpenStreetMap",
            ReadyGroup::Esri => "Esri",
            ReadyGroup::NasaUsgs => "NASA and USGS",
        }
    }
}

/// Готовая подоснова: ключ, английское название (ключ перевода), шаблон
/// адреса, предельный уровень, подпись, ссылка на условия, группа.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ready {
    pub key: &'static str,
    pub name: &'static str,
    pub url: &'static str,
    pub max_level: u32,
    pub credit: &'static str,
    pub terms: &'static str,
    pub group: ReadyGroup,
}

const ESRI_TERMS: &str = "https://www.esri.com/en-us/legal/terms/full-master-agreement";
const OSMFR_TERMS: &str = "https://www.openstreetmap.fr/fonds-de-carte/";
const OSMF_TERMS: &str = "https://operations.osmfoundation.org/policies/tiles/";
const GIBS_TERMS: &str = "https://www.earthdata.nasa.gov/engage/open-data-services-software-policies";
const USGS_TERMS: &str = "https://www.usgs.gov/information-policies-and-instructions/copyrights-and-credits";
const USGS_CREDIT: &str = "USGS The National Map";

/// Слой Esri по пути службы.
macro_rules! esri {
    ($path:literal) => {
        concat!("https://server.arcgisonline.com/ArcGIS/rest/services/", $path, "/MapServer/tile/{z}/{y}/{x}")
    };
}

/// Слой USGS The National Map по имени службы.
macro_rules! usgs {
    ($path:literal) => {
        concat!("https://basemap.nationalmap.gov/arcgis/rest/services/", $path, "/MapServer/tile/{z}/{y}/{x}")
    };
}

const fn ready(
    key: &'static str,
    name: &'static str,
    url: &'static str,
    max_level: u32,
    credit: &'static str,
    terms: &'static str,
    group: ReadyGroup,
) -> Ready {
    Ready { key, name, url, max_level, credit, terms, group }
}

/// Готовые подосновы в порядке подменю модуля. Адреса проверены модулем
/// тайлом у Перми (USGS - у Денвера) 13 и 18 октября 2026 года.
pub const READY: [Ready; 19] = [
    ready(
        "opentopomap",
        "OpenTopoMap",
        "https://a.tile.opentopomap.org/{z}/{x}/{y}.png",
        17,
        "© OpenStreetMap contributors, SRTM, © OpenTopoMap (CC-BY-SA)",
        "https://opentopomap.org/about",
        ReadyGroup::Osm,
    ),
    ready(
        "osm_hot",
        "OSM Humanitarian",
        "https://a.tile.openstreetmap.fr/hot/{z}/{x}/{y}.png",
        19,
        "© OpenStreetMap contributors, HOT, OpenStreetMap France",
        OSMFR_TERMS,
        ReadyGroup::Osm,
    ),
    ready(
        "osm_fr",
        "OSM France",
        "https://a.tile.openstreetmap.fr/osmfr/{z}/{x}/{y}.png",
        19,
        "© OpenStreetMap France, © OpenStreetMap contributors",
        OSMFR_TERMS,
        ReadyGroup::Osm,
    ),
    ready(
        "cyclosm",
        "CyclOSM",
        "https://a.tile-cyclosm.openstreetmap.fr/cyclosm/{z}/{x}/{y}.png",
        19,
        "CyclOSM, © OpenStreetMap contributors",
        OSMF_TERMS,
        ReadyGroup::Osm,
    ),
    ready("esri_topo", "Esri World Topo Map", esri!("World_Topo_Map"), 19, "Esri and the GIS User Community", ESRI_TERMS, ReadyGroup::Esri),
    ready("esri_street", "Esri World Street Map", esri!("World_Street_Map"), 19, "Esri and its data partners", ESRI_TERMS, ReadyGroup::Esri),
    ready(
        "esri_light_gray",
        "Esri Light Gray Canvas",
        esri!("Canvas/World_Light_Gray_Base"),
        16,
        "Esri, HERE, Garmin, © OpenStreetMap contributors",
        ESRI_TERMS,
        ReadyGroup::Esri,
    ),
    ready(
        "esri_dark_gray",
        "Esri Dark Gray Canvas",
        esri!("Canvas/World_Dark_Gray_Base"),
        16,
        "Esri, HERE, Garmin, © OpenStreetMap contributors",
        ESRI_TERMS,
        ReadyGroup::Esri,
    ),
    ready(
        "esri_natgeo",
        "Esri National Geographic",
        esri!("NatGeo_World_Map"),
        16,
        "National Geographic, Esri, GEBCO, NOAA, iPC",
        ESRI_TERMS,
        ReadyGroup::Esri,
    ),
    ready("esri_ocean", "Esri Ocean", esri!("Ocean/World_Ocean_Base"), 13, "Esri, GEBCO, NOAA, National Geographic", ESRI_TERMS, ReadyGroup::Esri),
    ready("esri_terrain", "Esri World Terrain", esri!("World_Terrain_Base"), 13, "Esri, USGS, NPS", ESRI_TERMS, ReadyGroup::Esri),
    ready("esri_shaded", "Esri Shaded Relief", esri!("World_Shaded_Relief"), 13, "Esri", ESRI_TERMS, ReadyGroup::Esri),
    ready("esri_physical", "Esri Physical", esri!("World_Physical_Map"), 8, "Esri, US National Park Service", ESRI_TERMS, ReadyGroup::Esri),
    ready(
        "esri_hillshade",
        "Esri World Hillshade",
        esri!("Elevation/World_Hillshade"),
        16,
        "Esri, Airbus DS, USGS, NGA, NASA, CGIAR",
        ESRI_TERMS,
        ReadyGroup::Esri,
    ),
    ready(
        "nasa_blue_marble",
        "NASA Blue Marble",
        "https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/BlueMarble_ShadedRelief_Bathymetry/default/GoogleMapsCompatible_Level8/{z}/{y}/{x}.jpeg",
        8,
        "NASA Blue Marble, NASA GIBS",
        GIBS_TERMS,
        ReadyGroup::NasaUsgs,
    ),
    ready("usgs_imagery", "USGS Imagery (USA)", usgs!("USGSImageryOnly"), 16, USGS_CREDIT, USGS_TERMS, ReadyGroup::NasaUsgs),
    ready("usgs_imagery_topo", "USGS Imagery Topo (USA)", usgs!("USGSImageryTopo"), 16, USGS_CREDIT, USGS_TERMS, ReadyGroup::NasaUsgs),
    ready("usgs_topo", "USGS Topo (USA)", usgs!("USGSTopo"), 16, USGS_CREDIT, USGS_TERMS, ReadyGroup::NasaUsgs),
    ready("usgs_relief", "USGS Shaded Relief (USA)", usgs!("USGSShadedReliefOnly"), 16, USGS_CREDIT, USGS_TERMS, ReadyGroup::NasaUsgs),
];

impl Basemap {
    /// Встроенные основы.
    pub const ALL: [Basemap; 2] = [Basemap::Esri, Basemap::Osm];

    /// Все основы: встроенные и готовые.
    pub fn all() -> Vec<Basemap> {
        let mut out = Self::ALL.to_vec();
        out.extend((0..READY.len()).map(Basemap::Ready));
        out
    }

    /// Готовая подоснова.
    pub fn ready(self) -> Option<&'static Ready> {
        match self {
            Basemap::Ready(n) => READY.get(n),
            _ => None,
        }
    }

    pub fn source(self) -> Source {
        match self {
            Basemap::Esri => net::ESRI,
            Basemap::Osm => net::OSM,
            Basemap::Ready(_) => match self.ready() {
                Some(r) => Source { name: r.key, url: r.url, max_requests: 4, max_level: r.max_level, ext: "png", placeholders: false, raw: false, tilejson: None },
                None => net::ESRI,
            },
        }
    }

    /// Ключ основы для действий строки «Поиск» и настроек.
    pub fn key(self) -> &'static str {
        match self {
            Basemap::Esri => "esri",
            Basemap::Osm => "osm",
            Basemap::Ready(_) => self.ready().map_or("esri", |r| r.key),
        }
    }

    /// Основа по ключу.
    pub fn from_key(key: &str) -> Option<Basemap> {
        Self::all().into_iter().find(|b| b.key() == key)
    }

    /// Название в выборе основы.
    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Basemap::Esri => tr(lang, "Esri World Imagery"),
            Basemap::Osm => tr(lang, "OpenStreetMap map"),
            Basemap::Ready(_) => self.ready().map_or("", |r| tr(lang, r.name)),
        }
    }

    /// Подпись источника и ссылка на условия. Подпись Esri - как
    /// в описании слоя в ArcGIS Online, сверено модулем 26 сентября
    /// 2026 года, ссылка - Esri Master License Agreement.
    pub fn attribution(self, lang: Lang) -> (&'static str, &'static str) {
        match self {
            Basemap::Esri => (
                "Esri, Vantor, Earthstar Geographics, and the GIS User Community",
                "https://www.esri.com/en-us/legal/terms/full-master-agreement",
            ),
            Basemap::Osm => (tr(lang, "© OpenStreetMap contributors"), "https://www.openstreetmap.org/copyright"),
            Basemap::Ready(_) => self.ready().map_or(("", ""), |r| (r.credit, r.terms)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Basemap, READY};

    #[test]
    fn ready_keys_round_trip() {
        for b in Basemap::all() {
            assert_eq!(Basemap::from_key(b.key()), Some(b));
            assert!(b.source().url.starts_with("https://"));
        }
        assert_eq!(Basemap::all().len(), 2 + READY.len());
    }
}
