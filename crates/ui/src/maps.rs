// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Карты картинками поверх снимка - строки каталога «Добавить данные»
//! из `data::rastermaps`: карты значений геофизики, готовые карты
//! и наложения. Названия, подсказки и группы - `layer_names` и `LAYERS`
//! из `ui/nasamaps.py` модуля.
//!
//! Картинки карт ложатся в картинку наложения тайла под линиями
//! векторной основы (`vector::VectorBase`). На глобусе одна карта
//! значений или готовая карта, наложений - сколько угодно, как у модуля.

use data::gibs::{self, Theme};
use data::rastermaps::{self, Field, Overlay, TileMap};

/// Вид карты.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Field(&'static Field),
    Map(&'static TileMap),
    Overlay(&'static Overlay),
    /// Тема GIBS: адрес тайла - на день ряда, его даёт запрос ряда дат.
    Theme(&'static Theme),
    /// Горизонтали рельефа по высотам Terrarium, цвета - по подложке.
    Contours(data::contours::Palette),
    /// Уклон или экспозиция по высотам Terrarium.
    Terrain(Analysis),
}

/// Анализ рельефа: на глобусе одна из двух раскрасок, как у модуля.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Analysis {
    Slope,
    Aspect,
}

/// Группа каталога, как у модуля.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Map,
    Terrain,
    Fire,
    Gases,
    Water,
    Life,
    Depths,
    Soil,
    Tiles,
}

/// Строка каталога: ключ, название и подсказка (английский ключ
/// перевода), группа, источник второй строкой.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Entry {
    pub key: &'static str,
    pub name: &'static str,
    pub tip: &'static str,
    pub group: Group,
    pub source: &'static str,
}

/// Строки: темы GIBS в порядке `gallery_items` модуля, за ними слои
/// в порядке `LAYERS`. «Повторяемости воды» в списке
/// каталога модуля нет, её нет и здесь.
pub const ALL: [Entry; 41] = [
    Entry {
        key: "contours",
        name: "Contours",
        tip: "Lines of equal terrain height, as on a topographic map. The interval follows the view scale - hundreds of metres from afar, 5-10 m close up, every fifth contour is thicker and labelled with its height. Below sea level they are blue isobaths. They are built from the globe heights and from an own terrain raster of the project.",
        group: Group::Map,
        source: "Terrarium",
    },
    Entry {
        key: "smoke",
        name: "Smoke, aerosol index",
        tip: "Absorbing aerosol - smoke of fires and dust - from OMPS per day, since 2012.",
        group: Group::Fire,
        source: "NASA GIBS",
    },
    Entry {
        key: "aerosol",
        name: "Aerosol optical depth",
        tip: "How much the air dims light because of smoke, dust and smog, MODIS per day, since 2017.",
        group: Group::Fire,
        source: "NASA GIBS",
    },
    Entry {
        key: "co",
        name: "Carbon monoxide",
        tip: "Carbon monoxide mixing ratio at about 5 km from AIRS per day, since 2002. Fire plumes are seen over thousands of kilometres.",
        group: Group::Fire,
        source: "NASA GIBS",
    },
    Entry {
        key: "co_emission",
        name: "Carbon monoxide emission",
        tip: "Surface carbon monoxide emission from the MERRA-2 reanalysis per month, since 1980. The series lags by several months.",
        group: Group::Fire,
        source: "NASA GIBS",
    },
    Entry {
        key: "rain",
        name: "Precipitation",
        tip: "IMERG precipitation rate per day, since 2000.",
        group: Group::Water,
        source: "NASA GIBS",
    },
    Entry {
        key: "soil",
        name: "Soil moisture",
        tip: "Moisture of the top centimetres of soil from SMAP per day, since 2015. The series has missing days.",
        group: Group::Water,
        source: "NASA GIBS",
    },
    Entry {
        key: "snow",
        name: "Snow cover",
        tip: "Snow from MODIS Terra imagery per day, since 2000. There are gaps under clouds and in the polar night, seas and lakes are not coloured.",
        group: Group::Water,
        source: "NASA GIBS",
    },
    Entry {
        key: "snow_8day",
        name: "Snow over 8 days",
        tip: "Snow and lake ice from MODIS Terra imagery, an 8-day summary, since 2000. There are fewer gaps under clouds than in the daily layer.",
        group: Group::Water,
        source: "NASA GIBS",
    },
    Entry {
        key: "snow_mass",
        name: "Snow mass",
        tip: "Snow amount in kilograms per square metre from the SMAP Level 4 model per day, since 2015, a cell of about 9 km. It is a model calculation from satellite data, clouds leave no gaps.",
        group: Group::Water,
        source: "NASA GIBS",
    },
    Entry {
        key: "sea_ice",
        name: "Sea ice",
        tip: "Sea ice concentration from GHRSST MUR per day, since 2002.",
        group: Group::Water,
        source: "NASA GIBS",
    },
    Entry {
        key: "vapor",
        name: "Water vapour",
        tip: "Water vapour in the atmospheric column from MODIS Terra per day, since 2000.",
        group: Group::Water,
        source: "NASA GIBS",
    },
    Entry {
        key: "chlorophyll",
        name: "Chlorophyll",
        tip: "Algae chlorophyll in the surface layer of the sea from PACE per day, since 2024.",
        group: Group::Water,
        source: "NASA GIBS",
    },
    Entry {
        key: "salinity",
        name: "Sea salinity",
        tip: "Sea surface salinity from SMAP, an 8-day running mean, since 2015.",
        group: Group::Water,
        source: "NASA GIBS",
    },
    Entry {
        key: "flood",
        name: "Floods",
        tip: "Water beyond its usual banks from MODIS over 3 days, since 2021.",
        group: Group::Water,
        source: "NASA GIBS",
    },
    Entry {
        key: "no2",
        name: "Nitrogen dioxide",
        tip: "Tropospheric nitrogen dioxide from TROPOMI per day, since 2018. Cities, roads and power plants are seen.",
        group: Group::Gases,
        source: "NASA GIBS",
    },
    Entry {
        key: "so2",
        name: "Sulphur dioxide",
        tip: "Near-surface sulphur dioxide from OMPS per day, since 2012. Volcanoes and plants are seen.",
        group: Group::Gases,
        source: "NASA GIBS",
    },
    Entry {
        key: "methane",
        name: "Methane",
        tip: "Methane mixing ratio at about 7 km from AIRS per month, since 2002.",
        group: Group::Gases,
        source: "NASA GIBS",
    },
    Entry {
        key: "co2",
        name: "Carbon dioxide",
        tip: "Column-average carbon dioxide from OCO-2, orbit swaths per day, since 2014. The series lags by several months.",
        group: Group::Gases,
        source: "NASA GIBS",
    },
    Entry {
        key: "ozone",
        name: "Ozone",
        tip: "Total column ozone from OMI per day, since 2004. The ozone hole over Antarctica is seen.",
        group: Group::Gases,
        source: "NASA GIBS",
    },
    Entry {
        key: "ndvi",
        name: "Vegetation",
        tip: "NDVI vegetation index from MODIS Terra over 16 days, since 2000. The series lags by a month.",
        group: Group::Life,
        source: "NASA GIBS",
    },
    Entry {
        key: "dust",
        name: "Dust",
        tip: "Dust in the atmosphere from AIRS per day, since 2002.",
        group: Group::Life,
        source: "NASA GIBS",
    },
    Entry {
        key: "night",
        name: "Night lights",
        tip: "VIIRS night image per day, since 2021. City lights, fires, gas flares and auroras are seen.",
        group: Group::Life,
        source: "NASA GIBS",
    },
    Entry {
        key: "geology",
        name: "Geological map",
        tip: "The Macrostrat geological map of the world compiled from maps of different scales. The colour shows the age of the rocks. In Russia and Europe the most detailed map is 1:5 000 000, in the USA the maps are much more detailed. A click in the Identify features mode shows the unit, the lithology, the age in Ma and the source map. The globe shows one such map - the geological map or a geophysical value map.",
        group: Group::Depths,
        source: "Macrostrat",
    },
    Entry {
        key: "glim",
        name: "Lithology",
        tip: "The GLiM lithological map of the world - surface rocks in 16 classes: unconsolidated sediments, sedimentary, volcanic, plutonic and metamorphic rocks, ice. It is compiled from 92 regional maps at a scale of about 1:3 750 000. A click in Identify features mode shows the rock class. The globe shows one such map - lithological, geological, soil, water or a geophysical value map.",
        group: Group::Depths,
        source: "GLiM, Hartmann & Moosdorf 2012",
    },
    Entry {
        key: "emag",
        name: "Magnetic anomalies",
        tip: "Anomalies of the Earth magnetic field after the NOAA NCEI EMAG2v3 grid - the departure of the field from the normal one at 4 km above the ellipsoid, a step of about 4 km. Red means a stronger field, blue a weaker one. The anomalies reveal the crust - ridges and stripes of the ocean floor, ancient shields, ore districts. Empty places have no survey. A click in the Identify features mode shows the value in nT.",
        group: Group::Depths,
        source: "EMAG2v3, NOAA NCEI",
    },
    Entry {
        key: "gravity",
        name: "Gravity anomalies",
        tip: "Free-air gravity anomaly after the XGM2019e model to degree 719, about 28 km spacing. Red marks gravity above normal, blue marks gravity below normal. Deep-sea trenches show lows down to -300 mGal, volcanic islands and mountains show highs. A click in the Identify features mode shows the anomaly in mGal.",
        group: Group::Depths,
        source: "XGM2019e, ICGEM",
    },
    Entry {
        key: "bouguer",
        name: "Bouguer anomalies",
        tip: "Gravity anomaly minus the attraction of the relief - the Bouguer correction after the ETOPO 2022 relief with a rock density of 2670 kg/m³, water and ice with their own densities, and the terrain correction for the surrounding relief within 167 km. The anomaly shows the density of the interior without the effect of mountains and ocean depths. Under mountains it is negative, which is thick light crust, over the ocean it is positive. A click in the Identify features mode shows the anomaly in mGal.",
        group: Group::Depths,
        source: "XGM2019e, ETOPO 2022",
    },
    Entry {
        key: "seafloor_age",
        name: "Ocean floor age",
        tip: "Age of the oceanic crust after the grid of Seton et al. 2020, 0.1° spacing. Young floor at mid-ocean ridges is red, old floor at ocean margins is blue and purple. Land and continental crust are not coloured. The globe shows one value map at a time - ocean floor age, magnetic anomalies, gravity or heat flow. A click in the Identify features mode shows the age in Ma.",
        group: Group::Depths,
        source: "Seton et al. 2020",
    },
    Entry {
        key: "heatflow",
        name: "Heat flow",
        tip: "Heat flow from the interior through the Earth's surface on a 0.5° grid built from 91 thousand measurements of the IHFC 2024 database. Blue marks cold old shields, red marks rifts, ridges and volcanic areas. Far from measurements the grid is smoothed. A click in the Identify features mode shows the flow in mW/m².",
        group: Group::Depths,
        source: "IHFC 2024",
    },
    Entry {
        key: "worldcover",
        name: "Land cover",
        tip: "The ESA WorldCover land cover map for 2021 from Sentinel-1 and Sentinel-2 images with a 10 m pixel - forest, shrubland, grassland, cropland, built-up, bare land, snow and ice, water, wetland, mangroves, moss and lichen. A click in Identify features mode shows the class.",
        group: Group::Life,
        source: "ESA WorldCover 2021",
    },
    Entry {
        key: "water_transitions",
        name: "Water transitions",
        tip: "What happened to water from 1984 to 2021 in Landsat images - permanent and seasonal water, new and lost, transitions between them, JRC Global Surface Water. New reservoirs, shrunken lakes and migrating channels are visible. A click in Identify features mode shows the class.",
        group: Group::Water,
        source: "JRC Global Surface Water",
    },
    Entry {
        key: "soil_clay",
        name: "Clay in soil",
        tip: "The share of clay particles in the soil layer 0-5 cm from ISRIC SoilGrids - a model built on 240 thousand soil profiles, a 250 m grid. A click in Identify features mode asks SoilGrids for the value.",
        group: Group::Soil,
        source: "SoilGrids, ISRIC",
    },
    Entry {
        key: "soil_sand",
        name: "Sand in soil",
        tip: "The share of sand particles in the soil layer 0-5 cm from ISRIC SoilGrids, a 250 m grid. A click in Identify features mode asks SoilGrids for the value.",
        group: Group::Soil,
        source: "SoilGrids, ISRIC",
    },
    Entry {
        key: "soil_soc",
        name: "Soil organic carbon",
        tip: "Organic carbon content of the soil layer 0-5 cm from ISRIC SoilGrids, g/kg, a 250 m grid. The highest values are in the peatlands of the tundra and taiga. A click in Identify features mode asks SoilGrids for the value.",
        group: Group::Soil,
        source: "SoilGrids, ISRIC",
    },
    Entry {
        key: "soil_ph",
        name: "Soil pH",
        tip: "Soil pH in water, layer 0-5 cm, from ISRIC SoilGrids, a 250 m grid. Yellow soils are acid, green neutral, turquoise alkaline. A click in Identify features mode asks SoilGrids for the value.",
        group: Group::Soil,
        source: "SoilGrids, ISRIC",
    },
    Entry {
        key: "sea_marks",
        name: "Sea marks",
        tip: "OpenSeaMap navigation marks - buoys, lighthouses, leading lines, fairways and anchorages. The marks are seen close up, from map scale about 1:500 000. The map does not replace a nautical chart.",
        group: Group::Tiles,
        source: "OpenSeaMap",
    },
    Entry {
        key: "hiking",
        name: "Hiking routes",
        tip: "Waymarked hiking routes from OpenStreetMap by Waymarked Trails - international, national, regional and local trails in the colour of their level.",
        group: Group::Tiles,
        source: "Waymarked Trails",
    },
    Entry {
        key: "cycling",
        name: "Cycling routes",
        tip: "Waymarked cycling routes from OpenStreetMap by Waymarked Trails - from international cycle routes to local ones.",
        group: Group::Tiles,
        source: "Waymarked Trails",
    },
    Entry {
        key: "slope",
        name: "Slope",
        tip: "Surface slope from the terrain heights, in classes from flat to steeper than 35°. The scale is in the bottom left corner of the view. Available on the Earth, Mars and the Moon.",
        group: Group::Terrain,
        source: "Terrarium",
    },
    Entry {
        key: "aspect",
        name: "Aspect",
        tip: "Which way a slope faces - the colour of the compass direction, flat ground is grey. It replaces the slope when switched on.",
        group: Group::Terrain,
        source: "Terrarium",
    },
];

/// Номер строки по ключу.
pub fn index(key: &str) -> Option<usize> {
    ALL.iter().position(|e| e.key == key)
}

/// Вид карты строки.
pub fn kind(key: &str) -> Option<Kind> {
    match key {
        "slope" => return Some(Kind::Terrain(Analysis::Slope)),
        "aspect" => return Some(Kind::Terrain(Analysis::Aspect)),
        _ => {}
    }
    if key == "contours" {
        return Some(Kind::Contours(data::contours::Palette::Map));
    }
    if let Some(t) = gibs::theme_of(key) {
        return Some(Kind::Theme(t));
    }
    if let Some(f) = rastermaps::field(key) {
        return Some(Kind::Field(f));
    }
    if let Some(m) = rastermaps::tile_map(key) {
        return Some(Kind::Map(m));
    }
    rastermaps::OVERLAYS.iter().find(|o| o.key == key).map(Kind::Overlay)
}

impl Kind {
    /// Место карты на глобусе: на месте одна карта, как у модуля. Тема
    /// GIBS - одна карта каталога, карта значений или готовая карта -
    /// одна карта слоя вида, наложений сколько угодно - None.
    pub fn slot(self) -> Option<u8> {
        match self {
            Kind::Theme(_) => Some(0),
            Kind::Field(_) | Kind::Map(_) => Some(1),
            Kind::Terrain(_) => Some(2),
            Kind::Overlay(_) | Kind::Contours(_) => None,
        }
    }

    /// Карта занимает место, на котором бывает одна.
    pub fn is_single(self) -> bool {
        self.slot().is_some()
    }

    /// Глубший уровень тайлов карты.
    pub fn max_level(self) -> u32 {
        match self {
            Kind::Field(f) => f.max_level,
            Kind::Map(m) => m.max_level,
            Kind::Overlay(o) => o.max_level,
            Kind::Theme(t) => t.level,
            Kind::Contours(_) => data::contours::LEVEL,
            Kind::Terrain(_) => net::TERRARIUM.max_level,
        }
    }

    /// Подпись источника на экране.
    pub fn attribution(self) -> &'static str {
        match self {
            Kind::Field(f) => f.attribution.0,
            Kind::Map(m) => m.attribution.0,
            Kind::Overlay(o) => o.attribution,
            Kind::Theme(_) => gibs::ATTRIBUTION.0,
            // Подпись высот Terrarium и так стоит на экране.
            Kind::Contours(_) | Kind::Terrain(_) => "",
        }
    }

    /// Адрес источника или условий.
    pub fn link(self) -> &'static str {
        match self {
            Kind::Field(f) => f.attribution.1,
            Kind::Map(m) => m.attribution.1,
            Kind::Overlay(o) => o.terms,
            Kind::Theme(_) => gibs::ATTRIBUTION.1,
            Kind::Contours(_) | Kind::Terrain(_) => "",
        }
    }

    /// Пришедший тайл RGBA в картинку слоя с премноженной альфой, None -
    /// рисовать нечего.
    pub fn picture(self, rgba: &[u8], key: globe_core::lod::TileKey) -> Option<Vec<u8>> {
        match self {
            Kind::Field(f) => rastermaps::field_rgba(f, rgba),
            Kind::Map(m) => rastermaps::map_rgba(m, rgba, m.opacity),
            Kind::Overlay(_) => rastermaps::overlay_rgba(rgba),
            Kind::Theme(t) => gibs::overlay_rgba(rgba, gibs::OPACITY, t.clear),
            // Картинку горизонталей рисует задание по тайлу высот.
            Kind::Contours(_) => None,
            // Уклон - по высотам самого тайла, как slope_prepare модуля.
            Kind::Terrain(a) => {
                let heights = data::contours::terrarium(rgba);
                let (slope, aspect) = data::slope::slope_aspect(&heights, key.0, key.2, globe_core::ellipsoid::EARTH.a);
                Some(match a {
                    Analysis::Slope => data::slope::slope_rgba(&slope),
                    Analysis::Aspect => data::slope::aspect_rgba(&aspect),
                })
            }
        }
    }

    /// Источники тайлов: (первый уровень источника, источник). У карты
    /// с мелкими уровнями из planetx-terrain их два.
    pub fn sources(self, key: &'static str) -> Vec<(u32, net::Source)> {
        let source = |name: &'static str, url: &'static str, max_level: u32| net::Source {
            name,
            url,
            max_requests: 4,
            max_level,
            ext: "png",
            placeholders: false,
            raw: false,
            tilejson: None,
        };
        match self {
            Kind::Field(f) => vec![(0, source(key, f.url, f.max_level))],
            Kind::Overlay(o) => vec![(0, source(key, o.url, o.max_level))],
            Kind::Map(m) if !m.coarse.is_empty() && m.split > 0 => {
                vec![(0, source(coarse_name(key), m.coarse, m.split - 1)), (m.split, source(key, m.url, m.max_level))]
            }
            Kind::Map(m) => vec![(0, source(key, m.url, m.max_level))],
            // Адрес темы - на день, источник заводит окно по ряду дат.
            Kind::Theme(_) => Vec::new(),
            // Высоты Terrarium до уровня съёмки, кэш - общий с рельефом.
            Kind::Contours(_) => vec![(0, net::Source { max_level: data::contours::SOURCE_LEVEL, ..net::TERRARIUM })],
            Kind::Terrain(_) => vec![(0, net::TERRARIUM)],
        }
    }
}

/// Папка кэша мелких уровней карты.
fn coarse_name(key: &str) -> &'static str {
    match key {
        "worldcover" => "worldcover-terrain",
        _ => "map-coarse",
    }
}

#[cfg(test)]
mod tests {
    use super::{ALL, kind};

    #[test]
    fn every_row_has_map() {
        for e in &ALL {
            let k = kind(e.key);
            assert!(k.is_some(), "{} без карты", e.key);
            let Some(k) = k else {
                return;
            };
            let sources = k.sources(e.key);
            assert!(!sources.is_empty() || matches!(k, super::Kind::Theme(_)));
            assert!(sources.iter().all(|(_, s)| s.url.starts_with("https://")));
        }
        // У покрытия земель мелкие уровни - из planetx-terrain.
        let cover = kind("worldcover").map(|k| k.sources("worldcover")).unwrap_or_default();
        assert_eq!(cover.len(), 2);
        assert_eq!(cover[0].1.max_level + 1, cover[1].0);
    }
}
