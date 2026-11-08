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
//!
//! Свои источники тайлов - пункт «Добавить источник тайлов…» меню
//! «Основа», окно `TileSourceDialog` модуля без служб WMS: название,
//! шаблон адреса с {z}, {x}, {y} или {-y}, уровни, подпись. Строки
//! источника живут до конца работы программы: основа - копируемое
//! значение, а источник тайлов держит адрес и имя кэша навсегда.

use std::sync::RwLock;

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
    /// Свой источник тайлов, номер в списке своих.
    Own(usize),
    /// Карта палеогеографии на возраст в млн лет, строка «Палеогеография»
    /// каталога. В выборе основы её нет.
    Paleo(u32),
    /// Снимки тела, ключ [`scene::planets`]. В выборе основы её нет, она
    /// стоит, пока на глобусе это тело.
    Body(&'static str),
}

/// Вечные строки источника снимков тела: ключ, имя кэша, адрес,
/// название, подпись, ссылка, предельный уровень.
type BodyEntry = (&'static str, &'static str, &'static str, &'static str, &'static str, &'static str, u32);

/// Источники снимков тел, заведённые за работу программы.
static BODIES: RwLock<Vec<BodyEntry>> = RwLock::new(Vec::new());

/// Строки источника снимков тела key или None, если у тела снимков нет.
fn body_entry(key: &str) -> Option<BodyEntry> {
    if let Some(found) = BODIES.read().ok().and_then(|l| l.iter().find(|e| e.0 == key).copied()) {
        return Some(found);
    }
    let planet = scene::planets::planet_by_key(key);
    let imagery = planet.imagery?;
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let entry = (planet.body.key, leak(format!("body_{key}")), leak(imagery.url), imagery.name, imagery.credit, imagery.link, u32::from(imagery.max_level));
    if let Ok(mut l) = BODIES.write() {
        l.push(entry);
    }
    Some(entry)
}

/// Адреса карт палеогеографии по возрасту, вечные строки источника.
static PALEO: RwLock<Vec<(u32, &'static str, &'static str)>> = RwLock::new(Vec::new());

/// Имя кэша и адрес карты возраста age.
fn paleo_source(age: u32) -> (&'static str, &'static str) {
    if let Some(found) = PALEO.read().ok().and_then(|l| l.iter().find(|(a, _, _)| *a == age).map(|(_, n, u)| (*n, *u))) {
        return found;
    }
    let name: &'static str = Box::leak(format!("paleo_{age}").into_boxed_str());
    let url: &'static str = Box::leak(data::paleo::TILE_URL.replace("{age}", &age.to_string()).into_boxed_str());
    if let Ok(mut l) = PALEO.write() {
        l.push((age, name, url));
    }
    (name, url)
}

/// Свой источник тайлов: название, шаблон адреса, последний уровень,
/// подпись.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Own {
    pub name: String,
    pub url: String,
    pub max_level: u32,
    pub credit: String,
}

/// Свой источник с вечными строками для источника тайлов.
#[derive(Debug)]
struct OwnEntry {
    key: &'static str,
    name: &'static str,
    url: &'static str,
    max_level: u32,
    credit: &'static str,
}

/// Свои источники по порядку добавления.
static OWN: RwLock<Vec<&'static OwnEntry>> = RwLock::new(Vec::new());

fn own_list() -> Vec<&'static OwnEntry> {
    OWN.read().map(|l| l.clone()).unwrap_or_default()
}

/// Самый глубокий уровень своих тайлов.
pub const OWN_MAX_LEVEL: u32 = 22;

/// Ошибка шаблона адреса своего источника или None, если он годится:
/// http или https, места {z}, {x} и {y} или {-y}.
pub fn template_error(url: &str) -> Option<&'static str> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Some("The address must begin with https:// or http://.");
    }
    if !(url.contains("{z}") && url.contains("{x}") && (url.contains("{y}") || url.contains("{-y}"))) {
        return Some("The address needs the places {z}, {x} and {y} or {-y}.");
    }
    if url.chars().any(char::is_whitespace) {
        return Some("The address must not contain spaces.");
    }
    None
}

/// Ключ своего источника по адресу: «own» и короткий хэш, имя папки кэша.
fn own_key(url: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in url.trim().bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("own_{h:016x}")
}

/// Поставить свой источник в список. Тот же адрес - прежняя основа.
/// Неверный шаблон - None.
pub fn register(own: &Own) -> Option<Basemap> {
    if template_error(&own.url).is_some() {
        return None;
    }
    let key = own_key(&own.url);
    let mut list = OWN.write().ok()?;
    if let Some(i) = list.iter().position(|e| e.key == key) {
        return Some(Basemap::Own(i));
    }
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ").trim().to_string();
    let name = clean(&own.name);
    let entry = OwnEntry {
        key: Box::leak(key.into_boxed_str()),
        name: Box::leak(if name.is_empty() { "Tiles".to_string() } else { name }.into_boxed_str()),
        url: Box::leak(own.url.trim().to_string().into_boxed_str()),
        max_level: own.max_level.clamp(1, OWN_MAX_LEVEL),
        credit: Box::leak(clean(&own.credit).into_boxed_str()),
    };
    list.push(Box::leak(Box::new(entry)));
    Some(Basemap::Own(list.len() - 1))
}

/// Свой источник основы для настроек.
pub fn own_of(b: Basemap) -> Option<Own> {
    let Basemap::Own(n) = b else {
        return None;
    };
    own_list().get(n).map(|e| Own { name: e.name.to_string(), url: e.url.to_string(), max_level: e.max_level, credit: e.credit.to_string() })
}

/// Группа готовых подоснов, `READY_GROUPS` модуля.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadyGroup {
    Osm,
    Esri,
    NasaUsgs,
    /// Открытые национальные карты, `national` модуля 1.41.
    National,
}

impl ReadyGroup {
    pub const ALL: [ReadyGroup; 4] = [ReadyGroup::Osm, ReadyGroup::Esri, ReadyGroup::NasaUsgs, ReadyGroup::National];

    /// Название подменю.
    pub fn name(self) -> &'static str {
        match self {
            ReadyGroup::Osm => "OpenStreetMap",
            ReadyGroup::Esri => "Esri",
            ReadyGroup::NasaUsgs => "NASA and USGS",
            ReadyGroup::National => "National maps",
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
    /// Охват районной карты, у мировой - None.
    pub region: Option<net::Region>,
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
    Ready { key, name, url, max_level, credit, terms, group, region: None }
}

/// Районная карта страны, вид `regional` модуля: охват в сотых долях
/// градуса (запад, юг, восток, север), уровень, с которого берётся сама
/// карта, и подложка вокруг.
const fn regional(key: &'static str, name: &'static str, url: &'static str, credit: &'static str, terms: &'static str, area: ([i32; 4], u32), base: &'static str) -> Ready {
    Ready { key, name, url, max_level: 19, credit, terms, group: ReadyGroup::National, region: Some(net::Region { bbox: area.0, min_level: area.1, base }) }
}

const ESRI_TOPO: &str = esri!("World_Topo_Map");
const ESRI_IMAGERY: &str = esri!("World_Imagery");
const TOPPLUS_CREDIT: &str = "© BKG (2026) dl-de/by-2-0";
const TOPPLUS_TERMS: &str = "https://gdz.bkg.bund.de/index.php/default/wmts-topplusopen-wmts-topplus-open.html";
const BASEMAPDE_CREDIT: &str = "© GeoBasis-DE / BKG (2026) CC BY 4.0";
const BASEMAPDE_TERMS: &str = "https://basemap.de/open-data/";
const SWISS_CREDIT: &str = "© swisstopo";
const SWISS_TERMS: &str = "https://www.geo.admin.ch/en/general-terms-of-use-fsdi";
const PDOK_CREDIT: &str = "© Kadaster, PDOK, CC BY 4.0";
const PDOK_TERMS: &str = "https://www.pdok.nl/copyright";
/// Охваты стран с запасом и уровни, REGIONS модуля.
const GERMANY: ([i32; 4], u32) = ([550, 4720, 1560, 5510], 7);
const SWISS: ([i32; 4], u32) = ([590, 4580, 1050, 4785], 8);
const HOLLAND: ([i32; 4], u32) = ([320, 5070, 730, 5360], 7);

/// Слой basemap.de по стилю.
macro_rules! basemapde {
    ($style:literal) => {
        concat!("https://sgx.geodatenzentrum.de/wmts_basemapde/tile/1.0.0/de_basemapde_web_raster_", $style, "/default/GLOBAL_WEBMERCATOR/{z}/{y}/{x}.png")
    };
}

/// Слой swisstopo по имени.
macro_rules! swiss {
    ($layer:literal) => {
        concat!("https://wmts.geo.admin.ch/1.0.0/ch.swisstopo.", $layer, "/default/current/3857/{z}/{x}/{y}.jpeg")
    };
}

/// Фоновая карта PDOK по стилю.
macro_rules! pdok {
    ($style:literal) => {
        concat!("https://service.pdok.nl/brt/achtergrondkaart/wmts/v2_0/", $style, "/EPSG:3857/{z}/{x}/{y}.png")
    };
}

/// Готовые подосновы в порядке подменю модуля. Адреса проверены модулем
/// тайлом у Перми (USGS - у Денвера) 13 и 18 октября 2026 года.
pub const READY: [Ready; 30] = [
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
    // Открытые национальные карты модуля 1.41, список opengeos/qgis-basemaps
    // (MIT). Адреса и уровни проверены модулем тайлами Берлина, Берна
    // и Амстердама 6 ноября 2026 года. Сверка - planetx-scene/data/sources.json.
    ready("topplus", "TopPlusOpen (BKG)", "https://sgx.geodatenzentrum.de/wmts_topplus_open/tile/1.0.0/web/default/WEBMERCATOR/{z}/{y}/{x}.png", 18, TOPPLUS_CREDIT, TOPPLUS_TERMS, ReadyGroup::National),
    ready("topplus_grey", "TopPlusOpen Grey (BKG)", "https://sgx.geodatenzentrum.de/wmts_topplus_open/tile/1.0.0/web_grau/default/WEBMERCATOR/{z}/{y}/{x}.png", 18, TOPPLUS_CREDIT, TOPPLUS_TERMS, ReadyGroup::National),
    regional("basemapde", "basemap.de (Germany)", basemapde!("farbe"), BASEMAPDE_CREDIT, BASEMAPDE_TERMS, GERMANY, ESRI_TOPO),
    regional("basemapde_grey", "basemap.de Grey (Germany)", basemapde!("grau"), BASEMAPDE_CREDIT, BASEMAPDE_TERMS, GERMANY, ESRI_TOPO),
    regional("swiss_map", "swisstopo National Map (Switzerland)", swiss!("pixelkarte-farbe"), SWISS_CREDIT, SWISS_TERMS, SWISS, ESRI_TOPO),
    regional("swiss_grey", "swisstopo National Map Grey (Switzerland)", swiss!("pixelkarte-grau"), SWISS_CREDIT, SWISS_TERMS, SWISS, ESRI_TOPO),
    regional("swissimage", "SWISSIMAGE (Switzerland)", swiss!("swissimage"), SWISS_CREDIT, SWISS_TERMS, SWISS, ESRI_IMAGERY),
    regional("pdok_map", "PDOK Map (Netherlands)", pdok!("standaard"), PDOK_CREDIT, PDOK_TERMS, HOLLAND, ESRI_TOPO),
    regional("pdok_grey", "PDOK Map Grey (Netherlands)", pdok!("grijs"), PDOK_CREDIT, PDOK_TERMS, HOLLAND, ESRI_TOPO),
    regional("pdok_pastel", "PDOK Map Pastel (Netherlands)", pdok!("pastel"), PDOK_CREDIT, PDOK_TERMS, HOLLAND, ESRI_TOPO),
    regional(
        "pdok_photo",
        "PDOK Aerial Photo (Netherlands)",
        "https://service.pdok.nl/hwh/luchtfotorgb/wmts/v1_0/Actueel_ortho25/EPSG:3857/{z}/{x}/{y}.jpeg",
        "© Beeldmateriaal Nederland, PDOK, CC BY 4.0",
        PDOK_TERMS,
        HOLLAND,
        ESRI_IMAGERY,
    ),
];

impl Basemap {
    /// Встроенные основы.
    pub const ALL: [Basemap; 2] = [Basemap::Esri, Basemap::Osm];

    /// Все основы: встроенные и готовые.
    pub fn all() -> Vec<Basemap> {
        let mut out = Self::ALL.to_vec();
        out.extend((0..READY.len()).map(Basemap::Ready));
        out.extend((0..own_list().len()).map(Basemap::Own));
        out
    }

    /// Свои источники.
    pub fn owns() -> Vec<Basemap> {
        (0..own_list().len()).map(Basemap::Own).collect()
    }

    fn own(self) -> Option<&'static OwnEntry> {
        match self {
            Basemap::Own(n) => own_list().get(n).copied(),
            _ => None,
        }
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
                Some(r) => Source { name: r.key, url: r.url, max_requests: 4, max_level: r.max_level, ext: "png", placeholders: false, raw: false, tilejson: None, region: r.region },
                None => net::ESRI,
            },
            Basemap::Own(_) => match self.own() {
                Some(e) => Source { name: e.key, url: e.url, max_requests: 2, max_level: e.max_level, ext: "png", placeholders: false, raw: false, tilejson: None, region: None },
                None => net::ESRI,
            },
            Basemap::Paleo(age) => {
                let (name, url) = paleo_source(age);
                Source { name, url, max_requests: 4, max_level: data::paleo::MAX_LEVEL, ext: "jpg", placeholders: false, raw: false, tilejson: None, region: None }
            }
            Basemap::Body(key) => match body_entry(key) {
                Some((_, name, url, _, _, _, max_level)) => {
                    let ext = if url.ends_with(".jpg") { "jpg" } else { "png" };
                    Source { name, url, max_requests: 4, max_level, ext, placeholders: false, raw: false, tilejson: None, region: None }
                }
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
            Basemap::Own(_) => self.own().map_or("esri", |e| e.key),
            Basemap::Paleo(_) => "paleo",
            Basemap::Body(_) => "body",
        }
    }

    /// Основа по ключу.
    pub fn from_key(key: &str) -> Option<Basemap> {
        Self::all().into_iter().find(|b| b.key() == key)
    }

    /// Основа сцены: сначала по ключу `view.basemap_key`, потом по названию
    /// `view.basemap` на английском или ключу в нём - решение 7
    /// `planetx-scene/DECISIONS.md`. Незнакомая основа - None.
    pub fn from_scene(key: Option<&str>, name: Option<&str>) -> Option<Basemap> {
        key.and_then(Self::from_key).or_else(|| name.and_then(|n| Self::all().into_iter().find(|b| b.name(Lang::EN) == n || b.key() == n)))
    }

    /// Название в выборе основы.
    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Basemap::Esri => tr(lang, "Esri World Imagery"),
            Basemap::Osm => tr(lang, "OpenStreetMap map"),
            Basemap::Ready(_) => self.ready().map_or("", |r| tr(lang, r.name)),
            Basemap::Own(_) => self.own().map_or("", |e| e.name),
            Basemap::Paleo(_) => tr(lang, "Paleogeography"),
            Basemap::Body(key) => body_entry(key).map_or("", |e| e.3),
        }
    }

    /// Подпись подложки вокруг районной карты: вне своей страны и издалека
    /// на глобусе тайлы Esri. У мировой основы - None.
    pub fn around_attribution(self, lang: Lang) -> Option<(&'static str, &'static str)> {
        let region = self.ready()?.region?;
        let around = READY.iter().find(|r| r.url == region.base).map(|r| (r.credit, r.terms));
        Some(around.unwrap_or_else(|| Basemap::Esri.attribution(lang)))
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
            Basemap::Own(_) => self.own().map_or(("", ""), |e| (e.credit, "")),
            Basemap::Paleo(_) => data::paleo::ATTRIBUTION,
            Basemap::Body(key) => body_entry(key).map_or(("", ""), |e| (e.4, e.5)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Basemap, READY};

    #[test]
    fn own_source_registers_once() {
        use super::{Own, register, template_error};
        assert!(template_error("ftp://x/{z}/{x}/{y}").is_some());
        assert!(template_error("https://x/{z}/{x}.png").is_some());
        assert!(template_error("https://x/{z}/{x}/{-y}.png").is_none());
        let own = Own { name: "Topo\tmap".into(), url: "https://tiles.example.org/{z}/{x}/{y}.png".into(), max_level: 99, credit: "Example".into() };
        let got = register(&own);
        assert!(got.is_some());
        let Some(b) = got else {
            return;
        };
        assert_eq!(register(&own), Some(b));
        assert_eq!(Basemap::from_key(b.key()), Some(b));
        assert_eq!(b.source().max_level, super::OWN_MAX_LEVEL);
        assert_eq!(b.name(crate::i18n::Lang::EN), "Topo map");
        assert_eq!(b.attribution(crate::i18n::Lang::EN).0, "Example");
        assert!(register(&Own { url: "nonsense".into(), ..own }).is_none());
    }

    /// Готовые подосновы и наложения Lite совпадают с каталогом источников
    /// линейки `planetx-scene/data/sources.json`, его копия -
    /// `crates/ui/data/sources.json`. Копия сверяется с хранилищем, если
    /// оно лежит рядом с Lite или указано в `PLANETX_SCENE`.
    #[test]
    #[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    fn sources_match_line_catalog() {
        let copy: serde_json::Value = serde_json::from_str(include_str!("../data/sources.json")).expect("копия sources.json");
        let list = copy["sources"].as_array().expect("sources");
        let find = |key: &str| list.iter().find(|s| s["key"] == key).unwrap_or_else(|| panic!("{key} нет в sources.json"));
        let text = |s: &serde_json::Value, k: &str| s[k].as_str().unwrap_or_default().to_string();
        for r in READY {
            let s = find(r.key);
            assert_eq!(text(s, "url"), r.url, "{}", r.key);
            assert_eq!(s["max_level"].as_u64(), Some(u64::from(r.max_level)), "{}", r.key);
            assert_eq!(text(s, "credit"), r.credit, "{}", r.key);
            assert_eq!(text(s, "terms"), r.terms, "{}", r.key);
            assert_eq!(text(s, "name_en"), r.name, "{}", r.key);
            match r.region {
                Some(region) => {
                    let bbox: Vec<i32> = s["bbox"].as_array().expect("bbox").iter().map(|v| (v.as_f64().unwrap_or(f64::NAN) * 100.0).round() as i32).collect();
                    assert_eq!(bbox, region.bbox.to_vec(), "{}", r.key);
                    assert_eq!(s["from_level"].as_u64(), Some(u64::from(region.min_level)), "{}", r.key);
                    assert_eq!(text(find(&text(s, "fallback")), "url"), region.base, "{}", r.key);
                }
                None => assert!(s.get("bbox").is_none(), "{} - районная карта в каталоге", r.key),
            }
        }
        // Национальные карты каталога - все в Lite.
        for s in list.iter().filter(|s| s["group"] == "national") {
            let key = text(s, "key");
            assert!(READY.iter().any(|r| r.key == key), "{key} нет в READY");
        }
        // Наложения каталога и Lite - одни и те же.
        let overlays: Vec<&serde_json::Value> = list.iter().filter(|s| s["kind"] == "overlay").collect();
        assert_eq!(overlays.len(), data::rastermaps::OVERLAYS.len());
        for o in data::rastermaps::OVERLAYS {
            let s = find(o.key);
            assert_eq!(text(s, "url"), o.url, "{}", o.key);
            assert_eq!(s["max_level"].as_u64(), Some(u64::from(o.max_level)), "{}", o.key);
            assert_eq!(text(s, "credit"), o.attribution, "{}", o.key);
            assert_eq!(text(s, "terms"), o.terms, "{}", o.key);
            assert!(crate::maps::kind(o.key).is_some(), "{} нет в каталоге карт", o.key);
        }
        // Копия совпадает с хранилищем линейки.
        let dir = std::env::var_os("PLANETX_SCENE").map_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../planetx-scene"), std::path::PathBuf::from);
        match std::fs::read_to_string(dir.join("data/sources.json")) {
            Ok(original) => {
                let original: serde_json::Value = serde_json::from_str(&original).expect("sources.json хранилища");
                assert_eq!(original, copy, "копия crates/ui/data/sources.json отстала от planetx-scene");
            }
            Err(_) => eprintln!("хранилища planetx-scene нет, сверка копии пропущена"),
        }
    }

    /// Районная карта вне своей страны и мельче своего уровня берёт тайл
    /// подложки вокруг, `RegionalXyz` модуля.
    #[test]
    #[allow(clippy::panic)]
    fn regional_map_falls_back_outside_its_country() {
        let Some(b) = Basemap::from_key("swiss_map") else {
            panic!("нет swiss_map");
        };
        let source = b.source();
        let Some(region) = source.region else {
            panic!("у swiss_map нет охвата");
        };
        // Берн, уровень 12 - тайл swisstopo.
        let bern = |z: u32| {
            let n = f64::from(1u32 << z);
            let x = ((7.44 + 180.0) / 360.0 * n) as u32;
            let lat = 46.95f64.to_radians();
            let y = ((1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * n) as u32;
            (z, x, y)
        };
        assert!(net::tile_url(&source, bern(12)).contains("wmts.geo.admin.ch"));
        // Мельче уровня 8 и вне Швейцарии - Esri.
        assert!(net::tile_url(&source, bern(5)).contains("arcgisonline"));
        assert!(net::tile_url(&source, (12, 0, 0)).contains("arcgisonline"));
        // Превью каталога - тайл внутри страны.
        assert!(net::tile_url(&source, region.preview()).contains("wmts.geo.admin.ch"));
        // Подпись подложки вокруг: карта - Esri Topo, аэрофото - снимки Esri,
        // у мировой основы её нет.
        let en = crate::i18n::Lang::EN;
        assert_eq!(b.around_attribution(en).map(|a| a.0), Some("Esri and the GIS User Community"));
        let photo = Basemap::from_key("swissimage").and_then(|b| b.around_attribution(en));
        assert_eq!(photo, Some(Basemap::Esri.attribution(en)));
        assert_eq!(Basemap::Osm.around_attribution(en), None);
    }

    #[test]
    fn ready_keys_round_trip() {
        for b in Basemap::all() {
            assert_eq!(Basemap::from_key(b.key()), Some(b));
            assert!(b.source().url.starts_with("https://"));
        }
        assert!(Basemap::all().len() >= 2 + READY.len());
    }

    #[test]
    fn scene_basemap_by_key_then_by_name() {
        // Ключ сильнее названия: название могло смениться.
        assert_eq!(Basemap::from_scene(Some("osm"), Some("Esri World Imagery")), Some(Basemap::Osm));
        // Незнакомый ключ - по названию, как прежние сцены без ключа.
        assert_eq!(Basemap::from_scene(Some("nonsense"), Some("Esri World Imagery")), Some(Basemap::Esri));
        assert_eq!(Basemap::from_scene(None, Some("OpenStreetMap map")), Some(Basemap::Osm));
        // Готовая подоснова - по ключу.
        if let Some(b) = Basemap::all().into_iter().find(|b| matches!(b, Basemap::Ready(_))) {
            assert_eq!(Basemap::from_scene(Some(b.key()), None), Some(b));
        }
        assert_eq!(Basemap::from_scene(None, Some("Неизвестная основа")), None);
        assert_eq!(Basemap::from_scene(None, None), None);
    }
}
