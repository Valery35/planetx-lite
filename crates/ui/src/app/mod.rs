// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно: глобус с подложкой и рельефом, навигация мышью и пальцами.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use eframe::{egui, egui_wgpu};
use globe_core::camera::Camera;
use globe_core::ellipsoid::Ellipsoid;
use globe_core::lod::{self, Memo, Options, Selection, TileKey};
use globe_core::navigation::{Navigator, Pose, nearest_terrain};
use globe_core::{DMat3, DVec3};
use net::TileLoader;
use render::{FrameOptions, Globe, wgpu};
use terrain::{Encoding, HeightStore};

use crate::basemap::Basemap;
use crate::check::{self, Check, Scenario, SourceStats};
use crate::credits::{self, Credits};
use crate::i18n::{Lang, search_status, tr, trf};
use crate::settings::Settings;
use crate::input::Input;
use crate::labels::{self, Labels};
use crate::marks::{self, Marks};
use crate::pictures::Pictures;
use crate::places;
use crate::relief::Relief;
use crate::timeline::TimeBar;
use crate::tours::Player;
use crate::search::{Search, place_pose};

mod rustgis;
mod addins;
use addins::ADDINS_KEY;
#[cfg(target_arch = "wasm32")]
use addins::addins_from_storage;
mod files;
mod layers;
mod analysis;
mod myplaces;
mod view;
mod live;
mod ai;
mod search;
mod selfcheck;

/// Сколько снимков тайлов держать в видеокарте, сверх нужных кадру
/// старые вытесняются. Снимок 256 × 256 занимает 256 КБ.
pub const TEXTURE_BUDGET: usize = 800;
/// Наибольшее количество точек облака в видеокарте.
pub const CLOUD_POINTS: u64 = 2_000_000;
/// Размер точки облака, логических пикселей.
pub const CLOUD_POINT_SIZE: f32 = 2.0;
/// Начальный вид - Пермь из космоса с 20 000 км, отвесно, север вверху.
/// Решение автора от 22 октября 2026 года.
pub const START: (f64, f64, f64) = (check::PERM.0, check::PERM.1, check::SPACE_DISTANCE);
/// Пол высот Земли без флажка «Глубины морей и океанов»: дно моря прижато
/// к уровню моря, подложка рисует воду.
pub const EARTH_FLOOR: f32 = 0.0;
/// Вода над дном моря: цвет в значениях экрана и непрозрачность,
/// `WATER_COLOR` модуля.
pub const WATER_COLOR: [f32; 4] = [0.09, 0.30, 0.55, 0.5];
/// Доля непрозрачности воды под картой аномалий поля, `WATER_UNDER_FIELD`
/// модуля.
pub const WATER_UNDER_FIELD: f32 = 0.15;

/// Куда отдать отчёт самопроверки: текст и итог, true - пройдена.
pub type Report = Box<dyn FnMut(&str, bool)>;

/// Куда отдать снимок окна.
pub type ShotSink = Box<dyn FnOnce(&egui::ColorImage) + Send>;

/// Кадров подряд без загрузки и движения перед снимком окна.
pub const SHOT_IDLE_FRAMES: u32 = 30;
/// Наибольшая строка «Моих меток» на странице, байт: localStorage
/// браузера держит около 5 МБ на сайт.
pub const PAGE_PLACES_LIMIT: usize = 4 << 20;
/// Секунд между позами записи тура, `RECORD_PERIOD` модуля.
pub const RECORD_PERIOD: f64 = 0.1;
/// Время на картинки линий векторной основы за кадр, секунды. Выбор
/// помощника: кадр в 60 в секунду длится 16 мс, у «Бриенца» с 6 мс было 49 кадров.
pub const VECTOR_BUDGET: f64 = 0.003;
/// Время на разбор тайлов подписей за кадр, секунды. Выбор помощника.
pub const LABEL_BUDGET: f64 = 0.003;
/// Наименьшая высота дерева меток и раздела «Слои» при перетаскивании
/// границы, точки экрана. Выбор помощника.
pub const SECTION_MIN: f32 = 60.0;
/// Строк раздела «Проект RustGIS» без прокрутки.
#[cfg(not(target_arch = "wasm32"))]
const RUSTGIS_ROWS: f32 = 8.0;
/// Строк раздела «Мои слои» без прокрутки.
const MY_LAYERS_ROWS: f32 = 10.0;
/// Снимков основы в видеокарту за кадр, как `UPLOADS_PER_FRAME`
/// в `render/view.py` модуля. Пока камера движется - один.
pub const UPLOADS_PER_FRAME: usize = 3;
/// Время на загрузку снимков за кадр, секунды, `UPLOAD_TIME` модуля.
/// Первый снимок кадра грузится всегда.
pub const UPLOAD_TIME: f64 = 0.002;
/// Значок загрузки виден через столько секунд, как у модуля.
pub const SPINNER_DELAY: f64 = 0.4;
/// Скорость шкалы в демо самосвалов, `HAUL_RATE` модуля.
pub const HAUL_RATE: f64 = 10.0;
/// Камера следом за самосвалом на сенсорном экране: расстояние, метры,
/// `FOLLOW_REACH` модуля. Издали самосвал занимает пару пикселей.
pub const FOLLOW_REACH: f64 = 200.0;
/// За сколько секунд рейса сравнивается ход самосвалов при выборе.
const HAUL_LOOK: f64 = 60.0;
/// За столько секунд до конца рейса момент возвращается к началу,
/// `HAUL_END` модуля: момент упирается в конец шкалы, и показ там встаёт.
pub const HAUL_END: f64 = 1.0;
/// Азимут и наклон камеры к охвату треков, градусы. Как вид сцены
/// самосвалов слоем QGIS модуля.
pub const TRACK_HEADING: f64 = 25.0;
pub const TRACK_TILT: f64 = 55.0;
/// Окно уже этого, логические пиксели, - узкое: панель при запуске
/// скрыта.
pub const NARROW: f32 = 700.0;

/// Снимок окна после загрузки места.
struct Shot {
    sink: Option<ShotSink>,
    done: Arc<AtomicBool>,
    idle: u32,
}

/// День ряда темы GIBS.
enum ThemeDay {
    /// Запрос ряда дат идёт.
    Asking(Box<dyn net::Request>),
    /// День ряда и готовые дни ряда: день задаёт шкала времени, как
    /// ползунок темы модуля, без шкалы - последний готовый день. Пустой
    /// ряд - день не меняется.
    Day(String, Vec<data::gibs::Interval>),
    /// Ряда нет: служба не ответила или ответ пуст.
    Failed,
}

/// Цветовая карта темы GIBS для шкалы.
enum ThemeScale {
    Asking(Box<dyn net::Request>),
    Ready(Option<data::gibs::Colormap>),
}

/// Загрузчики источников карты: первый уровень источника и загрузчик.
type MapLoaders = Vec<(u32, Box<dyn TileLoader>)>;

/// Предел ответа разового запроса, байты.
const ANSWER_BYTES: u64 = 1 << 20;
/// Половина ширины полосы, за которую берётся линия шторки, точки экрана.
const SWIPE_GRIP: f32 = 8.0;

/// Линия шторки пары текстур «после» и «до»: снимков события, без них -
/// выпусков Wayback. None - шторки пары нет.
fn pair_swipe(events: &crate::events::Events, wayback: Option<&crate::wayback::Wayback>) -> Option<f32> {
    events.swipe().or_else(|| wayback.and_then(|w| w.compare.as_ref()).filter(|_| events.layer().is_none()).map(|c| c.split))
}
/// Предел ответа всего каталога NASA GIBS, байт: 25 октября 2026 года
/// ответ - 5.8 МБ.
const GIBS_CATALOG_BYTES: u64 = 32 << 20;

/// Загрузчики тайлов платформы.
pub struct Loaders {
    /// Снимки Esri World Imagery.
    pub esri: Box<dyn TileLoader>,
    /// Карта OpenStreetMap.
    pub osm: Box<dyn TileLoader>,
    /// Высоты рельефа.
    pub terrain: Box<dyn TileLoader>,
    /// Векторные тайлы OpenFreeMap с пунктами для надписей.
    pub places: Box<dyn TileLoader>,
    /// Запросы к Nominatim.
    pub search: Box<dyn net::Request>,
    /// Картинки наложений по ссылкам, по одной.
    pub pictures: Box<dyn net::Request>,
    /// Файлы сцен по ссылкам: на странице сцены демо и `?scene=`.
    pub files: Box<dyn net::Request>,
    /// Загрузчик тайлов нового источника: карты каталога заводят свои
    /// загрузчики при включении.
    pub make: Box<dyn Fn(net::Source) -> Box<dyn TileLoader>>,
    /// Новый разовый запрос с пределом ответа в байтах: ряд дат темы
    /// GIBS, сводки слоёв. На странице предел задаёт браузер.
    pub request: Box<dyn Fn(u64) -> Box<dyn net::Request>>,
}

impl Loaders {
    /// Загрузчик основы.
    /// Загрузчик встроенной основы. У готовой подосновы загрузчик свой.
    pub fn basemap(&self, basemap: Basemap) -> Option<&dyn TileLoader> {
        match basemap {
            Basemap::Esri => Some(self.esri.as_ref()),
            Basemap::Osm => Some(self.osm.as_ref()),
            Basemap::Ready(_) | Basemap::Own(_) | Basemap::Paleo(_) | Basemap::Body(_) => None,
        }
    }
}

/// Режим «Геоид», `geoid` модуля: высоты Земли - геоид EGM2008 над
/// эллипсоидом, растянутый scale раз.
struct GeoidState {
    loader: Box<dyn TileLoader>,
    scale: f64,
}

/// Подписей живых объектов не больше, ближайших к точке взгляда.
const LIVE_LABELS: usize = 60;

/// Палеогеография, `PaleoBar` модуля: номер возраста в `AGES`, показ,
/// время прошлого шага, основа и строки векторной основы до неё.
struct PaleoState {
    index: usize,
    playing: bool,
    last: f64,
    before: Basemap,
    rows: std::collections::BTreeSet<crate::vector::Row>,
}

/// Земля на время другого тела: земные слои каталога, строки векторной
/// основы и основа, как они были. С возвратом Земли они включаются снова.
struct EarthAway {
    extras: Vec<crate::layers::Extra>,
    rows: std::collections::BTreeSet<crate::vector::Row>,
    basemap: Basemap,
}

/// Название тела по ключу [`scene::planets`], ключ перевода.
pub fn body_name(key: &str) -> &'static str {
    match key {
        "moon" => "Moon",
        "mars" => "Mars",
        "mercury" => "Mercury",
        "venus" => "Venus",
        "jupiter" => "Jupiter",
        "io" => "Io",
        "europa" => "Europa",
        "ganymede" => "Ganymede",
        "callisto" => "Callisto",
        "mimas" => "Mimas",
        "enceladus" => "Enceladus",
        "tethys" => "Tethys",
        "dione" => "Dione",
        "rhea" => "Rhea",
        "titan" => "Titan",
        "iapetus" => "Iapetus",
        "ceres" => "Ceres",
        "vesta" => "Vesta",
        _ => "Earth",
    }
}

/// Подменю меню «Тело» по ключу [`scene::planets::MENU`].
fn body_group_name(key: &str) -> &'static str {
    match key {
        "planets" => "Planets",
        "jupiter" => "Moons of Jupiter",
        "saturn" => "Moons of Saturn",
        _ => "Asteroids",
    }
}

/// Источник высот тела: у Земли Terrarium, у Марса и Луны - тайлы
/// planetx-terrain, у прочих тел высот нет.
fn terrain_source(planet: &scene::planets::Planet) -> Option<net::Source> {
    if planet.earth {
        return Some(net::TERRARIUM);
    }
    let t = planet.terrain.as_ref()?;
    let name: &'static str = Box::leak(format!("terrain_{}", planet.key()).into_boxed_str());
    let url: &'static str = Box::leak(t.url.clone().into_boxed_str());
    Some(net::Source { name, url, max_requests: 4, max_level: u32::from(t.max_level), ..net::TERRARIUM })
}

/// Секунд на шаг возраста при показе, `PLAY_PERIOD` модуля.
const PALEO_STEP: f64 = 0.9;

/// Название периода по ключу `data::paleo::period`.
fn period_name(lang: Lang, key: &str) -> &'static str {
    match key {
        "quaternary" => tr(lang, "Quaternary"),
        "neogene" => tr(lang, "Neogene"),
        "paleogene" => tr(lang, "Paleogene"),
        "cretaceous" => tr(lang, "Cretaceous"),
        "jurassic" => tr(lang, "Jurassic"),
        "triassic" => tr(lang, "Triassic"),
        "permian" => tr(lang, "Permian"),
        "carboniferous" => tr(lang, "Carboniferous"),
        "devonian" => tr(lang, "Devonian"),
        "silurian" => tr(lang, "Silurian"),
        "ordovician" => tr(lang, "Ordovician"),
        "cambrian" => tr(lang, "Cambrian"),
        _ => tr(lang, "Precambrian"),
    }
}

/// Поля окна «Новый источник тайлов», `TileSourceDialog` модуля.
#[derive(Debug, Clone)]
struct OwnDialog {
    name: String,
    url: String,
    max_level: u32,
    credit: String,
    /// Ответ GetCapabilities службы WMS по адресу asked: слои или ошибка.
    caps: Option<Result<scene::wms::Capabilities, String>>,
    asked: String,
    /// Выбранный слой службы.
    layer: usize,
}

impl Default for OwnDialog {
    fn default() -> Self {
        OwnDialog { name: String::new(), url: String::new(), max_level: 19, credit: String::new(), caps: None, asked: String::new(), layer: 0 }
    }
}

/// Хранилище высот, общее для кадра и навигатора.
/// Пол камеры под землёй, общий с рельефом навигации.
type FloorCell = Arc<RwLock<Option<crate::subsurface::Floor>>>;

#[derive(Clone, Default)]
pub struct Heights(Arc<RwLock<HeightStore>>);

impl Heights {
    pub fn read(&self) -> RwLockReadGuard<'_, HeightStore> {
        self.0.read().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, HeightStore> {
        self.0.write().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Тайлы уровней 0-2: они нужны кадру всегда, кадр без них нечем
/// закрыть Землю.
fn start_keys() -> Vec<TileKey> {
    (0..=2u32).flat_map(|z| (0..1u32 << z).flat_map(move |x| (0..1u32 << z).map(move |y| (z, x, y)))).collect()
}

/// Недостающие тайлы высот кадра и их приоритет.
///
/// Высоты нужны и тайлам, которые подложка ждёт. Иначе у самой земли
/// рисуется грубый предок, ему хватает грубых высот, и точные не просит
/// никто. Приоритет высот - экранная ошибка тайла, который их ждёт.
fn height_needs(store: &HeightStore, sel: &Selection) -> Vec<(TileKey, f64)> {
    let mut wanted: HashMap<TileKey, f64> = HashMap::new();
    let want: HashMap<TileKey, f64> = sel.want.iter().copied().collect();
    for key in sel.draw.iter().chain(sel.want.iter().map(|(k, _)| k)) {
        let height_key = store.wanted(*key);
        let p = want.get(key).copied().unwrap_or(1.0);
        if store.tile(height_key).is_none() {
            let slot = wanted.entry(height_key).or_insert(p);
            *slot = slot.max(p);
        }
        // Дно моря глубже terrain::SEA_LEVEL берётся у предка этого
        // уровня, он нужен вместе с тайлом.
        let sea = terrain::ancestor(height_key, terrain::SEA_LEVEL);
        if height_key.0 > terrain::SEA_LEVEL && store.tile(sea).is_none() {
            let slot = wanted.entry(sea).or_insert(p);
            *slot = slot.max(p);
        }
    }
    if store.tile((0, 0, 0)).is_none() {
        wanted.insert((0, 0, 0), lod::INFINITE_PRIORITY + 10.0);
    }
    wanted.into_iter().collect()
}

pub struct LiteApp {
    lang: Lang,
    render_state: Option<egui_wgpu::RenderState>,
    globe: Option<Globe>,
    texture: Option<egui::TextureId>,
    nav: Navigator,
    loaders: Loaders,
    basemap: Basemap,
    heights: Heights,
    input: Input,
    memo: Memo,
    check: Option<Check>,
    report: Report,
    shot: Option<Shot>,
    labels: Labels,
    search: Search,
    search_text: String,
    credits: Credits,
    marks: Marks,
    /// Файл, который откроется в первом кадре.
    pending_open: Vec<(Vec<u8>, String)>,
    /// Сообщение об открытии файла: название сцены или ошибка.
    open_message: Option<String>,
    /// Маршруты по дорогам, `RouteManager` модуля.
    route: crate::route::Router,
    /// Непрозрачность карт картинками по ключу, меню «⋯» строки.
    map_opacity: HashMap<String, f32>,
    /// Окно «Новый источник тайлов».
    own_dialog: Option<OwnDialog>,
    /// Запрос GetCapabilities окна «Новый источник тайлов».
    own_caps: Option<Box<dyn net::Request>>,
    /// Окно «Номенклатура листа».
    sheets_window: Option<crate::sheets::SheetsWindow>,
    /// Окно «Сайты меню «Открыть в браузере»».
    links_window: Option<crate::menulinks::LinksWindow>,
    /// Окна и слои «Видимость» и «Инсоляция».
    viewshed: Option<crate::pointcalc::Calc>,
    insolation: Option<crate::pointcalc::Calc>,
    /// Окно «Тени» и его слой.
    shadows: Option<crate::pointcalc::Calc>,
    /// Окно «Изменения местности» и его слой.
    aef_change: Option<crate::aefcalc::ChangeCalc>,
    /// Окно «Классификация» и его слой.
    aef_class: Option<crate::aefclass::ClassCalc>,
    /// Окно «Похожие области» и его слой.
    similar: Option<crate::aefclass::ClassCalc>,
    /// Окно «Трассировка».
    tracing: Option<crate::aefclass::ClassCalc>,
    /// Окно «Выделение объектов».
    object_tasks: Option<crate::aefclass::ClassCalc>,
    /// Окно «Распознавание».
    recognize: Option<crate::aefclass::ClassCalc>,
    /// Окно, которое ждёт названий классов от помощника: 0 - «Трассировка»,
    /// 1 - «Выделение объектов», 2 - «Распознавание».
    names_for: Option<u8>,
    /// Окно «Минералы по спектру».
    spectral: Option<crate::spectral::Spectral>,
    /// Окно «Минералы EMIT».
    emit: Option<crate::emit::Emit>,
    /// Предпросмотр общего слоя вида картинкой поверх картинок меток.
    preview: Option<scene::kml::Overlay>,
    /// Предпросмотр сменился: сетки картинок собираются заново.
    preview_dirty: bool,
    /// Отпечаток слоёв-картинок «Моих слоёв» в прошлом кадре.
    data_pictures_seen: u64,
    /// Охват шкалы по слоям-картинкам в прошлый раз.
    data_extent_seen: Option<(f64, f64)>,
    /// Хозяин общего слоя вида окон анализа.
    site_layer: crate::analysis::SiteLayer,
    /// Многоканальные растры: рельеф они не меняют, ждут классификации.
    band_rasters: Vec<crate::aefclass::Raster>,
    /// Общий участок окон анализа - вершины, `GlobeWindow.site` модуля.
    area_site: Option<Vec<(f64, f64)>>,
    /// Окно «Уклон и экспозиция» и его квадрат.
    relief_calc: Option<crate::reliefcalc::ReliefCalc>,
    /// Горизонтали вида в «Мои слои»: ждут высоты квадрата вида.
    view_contours: Option<crate::isolines::ViewJob>,
    /// Сессии окон анализа: номер - название папки расчёта в «Анализе
    /// участка».
    analysis_sessions: HashMap<u64, String>,
    /// Окна «Затопление», «Сток воды», «Уровень моря».
    hydro: crate::hydrocalc::Hydro,
    /// Окно «Земляные работы».
    earthwork: crate::earthwork::Earthwork,
    /// Окно «Выемка и насыпь».
    volumes: Option<crate::volumecalc::VolumeCalc>,
    /// Окно «Архив снимков Wayback».
    wayback: Option<crate::wayback::Wayback>,
    /// Окна папки: «Выдавливание» и «Батиметрия по промерам».
    extrude: Option<crate::foldercalc::ExtrudeWindow>,
    sounding: Option<crate::foldercalc::SoundingWindow>,
    /// Тайлы высот, которых ждут расчёты у точки.
    calc_keys: Vec<TileKey>,
    /// Окно «Старые карты «Это Место»».
    etomesto: Option<crate::etomesto::EtomestoWindow>,
    /// Модели живых объектов в видеокарте: строка и ключ по порядку.
    live_keys: Vec<(crate::live::Kind, String)>,
    /// Точки и подписи живых объектов.
    live_points: Vec<render::PointsMesh>,
    live_labels: Vec<(String, DVec3)>,
    /// Точки кадра и показанные спутники с местом, спутники - у слоя реестра.
    sat_points: Vec<render::PointsMesh>,
    sat_shown: Vec<(usize, DVec3)>,
    /// Загрузчики Overpass строк «Шахты и карьеры OSM» и «ЛЭП, кабели
    /// и трубопроводы OSM».
    osm_mines: Option<crate::overpass::Overpass<data::mining::Item>>,
    osm_infra: Option<crate::overpass::Overpass<data::infra::Item>>,
    overpass_mirrors: crate::overpass::Mirrors,
    /// Версия высот и кадр зданий, по которым собраны цели 3D-линейки.
    solid_stamp: Option<(u64, Vec<TileKey>)>,
    /// Неподвижные 3D-модели в видеокарте для выгрузки вида.
    export_models: Vec<render::ModelMesh>,
    /// Картинки границ полей, когда строка включена.
    field_tiles: Option<crate::fieldmap::FieldTiles>,
    building_keys: HashSet<TileKey>,
    building_only: HashSet<TileKey>,
    /// Сведения об участках кадастра под щелчком: точка и запрос, пришедшие
    /// участки для подсветки, `_ask_parcels` модуля.
    parcel_ask: Option<((f64, f64), Box<dyn net::Request>)>,
    parcels: Vec<data::cadastre::Parcel>,
    /// Включённые слои реестра [`crate::registry::ALL`] по номеру строки.
    plugged: BTreeMap<usize, Box<dyn crate::registry::Layer>>,
    /// Окно «Карты Рамсея».
    rumsey: Option<crate::rumsey::Rumsey>,
    /// Окно «Снимки OpenAerialMap».
    oam: Option<crate::oam::Oam>,
    /// Окно «Космоснимки».
    sentinel: Option<crate::sentinel::Sentinel>,
    /// Объекты живых строк на этот кадр, для окна «Объекты» и меню.
    live_objects: Vec<(crate::live::Kind, crate::live::Object)>,
    /// Камера следом за живым объектом: строка и ключ.
    live_follow: Option<(crate::live::Kind, String)>,
    /// Режим «Геоид»: загрузчик тайлов геоида и масштаб.
    geoid: Option<GeoidState>,
    /// Палеогеография: ползунок возраста и что было до неё.
    paleo: Option<PaleoState>,
    /// Тело на глобусе, `core/planets.py` модуля.
    planet: scene::planets::Planet,
    /// Земля на время другого тела.
    earth_away: Option<EarthAway>,
    /// Основа тела из мозаики COG, у Луны - LROC WAC.
    cog_base: Option<crate::cogtiles::CogTiles>,
    /// Режим полёта, свой самолёт.
    flight: Option<crate::flight::Flight>,
    /// Модель на пути, за которой идёт камера, номер в `movers`.
    followed: Option<usize>,
    /// Разрез Земли, строка каталога.
    cutaway: Option<crate::cutaway::Cutaway>,
    /// Подземный режим, `SubsurfaceManager` модуля.
    subsurface: Option<crate::subsurface::Subsurface>,
    /// Пол камеры под землёй для рельефа навигации.
    floor: FloorCell,
    /// Папка последнего открытого файла в программе.
    open_dir: Option<std::path::PathBuf>,
    /// Тайлы коридора маршрута у загрузчика векторной основы и те из них,
    /// что не нужны самой основе.
    route_keys: HashSet<TileKey>,
    route_only: HashSet<TileKey>,
    /// Тайлы воды масок прохода воды у того же загрузчика и те из них,
    /// что не нужны самой основе.
    water_tiles: crate::watermask::WaterTiles,
    water_keys: HashSet<TileKey>,
    water_only: HashSet<TileKey>,
    /// Прочитанные браузером брошенные файлы: байты или ошибка и имя.
    dropped_box: DroppedBox,
    /// «Мои слои» - векторные слои файлов и результатов в разделе «Слои».
    data_layers: crate::datalayers::DataLayers,
    /// Наборы 3D Tiles в разделе «Слои».
    tilesets: crate::tilesets::Tilesets,
    /// Список «Моих слоёв» прошлого запуска, ждёт папки данных программы.
    my_layers_saved: Option<String>,
    /// Список аддинов прошлого запуска: пакеты лежат в папке данных,
    /// она известна первым кадром.
    addins_saved: Option<String>,
    /// Страница: пакеты аддинов из хранилища до первого кадра.
    #[cfg(target_arch = "wasm32")]
    addins_store: Vec<(String, Vec<u8>)>,
    #[cfg(feature = "addins")]
    addins: crate::addins::Addins,
    /// Список наборов 3D Tiles прошлого запуска, открывается первым кадром.
    tilesets_saved: Option<String>,
    /// Наборы, которые открываются снова при запуске: путь или адрес -
    /// флажок набора. Камера к ним не летит.
    tiles_restoring: HashMap<String, bool>,
    /// Путь файла с диска, который сейчас открывается.
    opening_path: Option<std::path::PathBuf>,
    /// Папка данных пользователя: в ней лежат файлы «Моих слоёв». Не
    /// папка кэша - кэш система может очистить. У страницы её нет.
    user_dir: Option<std::path::PathBuf>,
    /// Поле имени новой папки «Моих слоёв».
    new_folder: String,
    /// Сетки выдавленных объектов «Моих слоёв».
    data_meshes: Vec<render::FeatureMesh>,
    /// Соседние файлы шейпфайла и MapInfo, пришедшие отдельно: имя
    /// и байты. Главный файл берёт свои.
    sidecars: Vec<(String, Vec<u8>)>,
    /// Главные файлы шейпфайла и MapInfo на странице, ждущие соседних:
    /// байты, имя и время прихода последнего файла.
    waiting: Vec<(Vec<u8>, String)>,
    waiting_since: f64,
    /// Снимок окна вместе с панелью.
    shot_ui: bool,
    /// Идущий тур.
    player: Option<Player>,
    /// Остановка тура прошлого кадра: на новой шкала встаёт на её время.
    tour_index: Option<usize>,
    timebar: TimeBar,
    /// Слои хозяина открытых сцен `.planetx` по названию корня меток:
    /// слои проекта QGIS или RustGIS. Lite их не открывает и при
    /// сохранении сцены пишет обратно, контракт `planetx-scene`.
    scene_layers: HashMap<String, Vec<serde_json::Map<String, serde_json::Value>>>,
    /// Картинки наложений открытой сцены.
    pictures: Pictures,
    /// Картинки наложений на экране в egui по ключу.
    screen_textures: HashMap<String, egui::TextureHandle>,
    /// Поза задана при запуске: сцена при старте её не меняет.
    keep_pose: bool,
    /// Откуда самопроверка берёт сцены демо: в программе - папка,
    /// на странице - начало адреса.
    scene_source: Option<String>,
    /// Сцена грузится по ссылке: имя файла и признак сцены самопроверки.
    fetching: Option<(String, bool)>,
    /// Папка кэша файлов демо, у страницы её нет.
    demo_cache: Option<std::path::PathBuf>,
    /// Файлы выбранного демо, которые ещё не открыты, по порядку.
    demo_queue: std::collections::VecDeque<&'static str>,
    /// Файл демо, который сейчас грузится.
    demo_fetching: Option<&'static str>,
    /// Адреса сцен из ссылки, которые ещё не открыты, по порядку.
    url_queue: std::collections::VecDeque<String>,
    /// Адрес файла из ссылки, который сейчас грузится или открывается:
    /// по нему набор 3D Tiles берёт свои куски.
    fetching_url: Option<String>,
    /// Адреса открытых сцен и файлов демо по порядку - для ссылки
    /// «Поделиться». Файлы с диска адреса не имеют.
    shared: Vec<String>,
    /// «Мои метки» изменились и ещё не записаны в хранилище.
    places_dirty: bool,
    /// Выделенная строка и переименование в «Моих метках».
    tree_state: marks::TreeState,
    /// Настройки и окно «Настройки».
    settings: Settings,
    settings_open: bool,
    /// Левая панель показана.
    panel_open: bool,
    /// Скачанные страницей шрифты с иероглифами по коду языка.
    web_fonts: HashMap<&'static str, Vec<u8>>,
    /// Шрифт, который сейчас грузится.
    font_fetching: Option<&'static str>,
    /// Шрифты поставить заново: сменился язык или пришёл шрифт.
    fonts_dirty: bool,
    /// Место панели инструментов прошлого кадра: надписи не ложатся под неё.
    toolbar_rect: Option<egui::Rect>,
    /// Настройки изменились и ещё не записаны в хранилище.
    settings_dirty: bool,
    /// Ширина окна при запуске проверена.
    layout_checked: bool,
    /// Свой рельеф: врезанные растры высот.
    relief: Relief,
    /// Открытые облака точек.
    clouds: Vec<render::PointsMesh>,
    /// Разобранные 3D-модели сцены.
    models: crate::models::Models,
    /// Модели на путях gx:Track: их место - на момент шкалы времени.
    movers: Vec<crate::models::Mover>,
    /// Линейка, новая метка или правка метки, None - инструмента нет.
    tool: Option<crate::tool::Tool>,
    /// Экранные органы навигации, как у модуля.
    navpad: crate::navpad::NavPad,
    /// Страница на телефоне или планшете: органов навигации нет, глобус
    /// ведут пальцы.
    touch: bool,
    /// «Определить объекты» включено: щелчок по глобусу - сведения
    /// о точке, как кнопка `identify` модуля.
    identify: bool,
    /// Векторная основа: строки раздела «Слои» и линии поверх снимка.
    vector: crate::vector::VectorBase,
    /// Строки слоёв каталога в разделе «Слои» и включённые из них.
    extras: std::collections::BTreeSet<crate::layers::Extra>,
    extras_on: std::collections::BTreeSet<crate::layers::Extra>,
    /// Открытое меню строки раздела «Слои».
    layers_menu: Option<String>,
    /// Каталог «Добавить данные».
    catalog: crate::layers::Catalog,
    /// Ползунок непрозрачности: выделенный путь, цвета при выделении,
    /// непрозрачность при выделении и сейчас.
    opacity: Option<OpacityState>,
    /// Высота раздела «Слои» в прошлом кадре.
    layers_height: f32,
    /// Сообщение строки состояния и момент, с которого оно видно.
    message_seen: Option<(String, f64)>,
    /// Точка поверхности под курсором: широта, долгота, высота.
    cursor_point: Option<(f64, f64, f64)>,
    /// Тайлы, которые просили подписи в прошлом кадре.
    label_keys: std::collections::HashSet<globe_core::lod::TileKey>,
    /// Загрузчики включённых карт картинками: ключ строки - (первый
    /// уровень источника, загрузчик).
    map_loaders: HashMap<&'static str, MapLoaders>,
    /// День ряда тем GIBS: запрос идёт, день выбран или ряда нет.
    theme_days: HashMap<&'static str, ThemeDay>,
    /// Запрос всего каталога NASA GIBS.
    gibs_catalog: Option<Box<dyn net::Request>>,
    /// Окно «Снимки событий» и снимки события на глобусе.
    events: crate::events::Events,
    /// Событие, которое показать, когда придёт список событий: ключ
    /// запуска `--event`.
    pending_event: Option<String>,
    /// Тайлы снимков события без снимков, по сторонам шторки.
    event_empty: [std::collections::HashSet<TileKey>; 2],
    /// Хозяин пары текстур «после» и «до»: 0 - никто, 1 - снимки события,
    /// 2 - шторка выпусков Wayback.
    pair_owner: u8,
    /// Ширина кадра глобуса в пикселях, для линии шторки.
    globe_width_px: f32,
    /// Линию шторки тянут мышью.
    swipe_drag: bool,
    /// Шторка слоёв «Моих слоёв»: линия в доле ширины вида, None - снята.
    layer_swipe: Option<f32>,
    /// Поиск и чтение облаков лидара у точки.
    lidar: Option<crate::lidarnet::Lidar>,
    /// Подписи облаков лидара на глобусе.
    lidar_credits: Vec<(&'static str, &'static str)>,
    /// Слои каталога GIBS из настроек: ложатся, когда каталог придёт.
    gibs_pending: Vec<String>,
    /// Цветовые карты тем GIBS для шкалы: запрос идёт или пришла.
    theme_scales: HashMap<&'static str, ThemeScale>,
    /// Адреса и папки кэша тем на день: строка живёт до конца программы,
    /// загрузчику нужна постоянная.
    lasting: HashMap<String, &'static str>,
    /// Загрузчик выбранной готовой подосновы.
    ready_loader: Option<(Basemap, Box<dyn TileLoader>)>,
    /// Огни городов на ночной стороне, пока включено солнце.
    lights_loader: Option<Box<dyn TileLoader>>,
    /// Каталог звёзд: запрос, пришёл (true) или не пришёл (false).
    stars: Option<Result<bool, Box<dyn net::Request>>>,
    /// Картинка неба с Млечным путём: запрос или пришла ли.
    sky_image: Option<Result<bool, Box<dyn net::Request>>>,
    /// Вид неба, `show_sky` модуля. None - на экране тело.
    sky: Option<crate::sky::Sky>,
    /// Взгляд на небо при прошлом выходе из неба.
    sky_saved: Option<data::skyview::SkyView>,
    /// Снимки неба HiPS при приближении.
    surveys: crate::skysurvey::Surveys,
    /// Пришедшие снимки основы, ждущие загрузки в видеокарту.
    pending: HashMap<TileKey, net::Image>,
    /// Тайлы, нарисованные прошлым кадром, - их снимки грузятся первыми.
    drawn: std::collections::HashSet<TileKey>,
    /// Камера двигалась в прошлом кадре.
    was_moving: bool,
    /// С какого момента идёт загрузка, для значка загрузки.
    loading_since: Option<f64>,
    /// Меню правой кнопки на глобусе.
    globe_menu: Option<crate::globemenu::GlobeMenu>,
    /// Окно «Объекты».
    objects: Option<crate::identify::Objects>,
    /// Запрос значения карты под щелчком и его итог.
    map_ask: Option<(crate::mapvalue::Ask, crate::mapvalue::Answer)>,
    /// Слои точек: землетрясения, вулканы, пожары.
    dots: crate::dots::Dots,
    /// Промежуток шкалы времени, по которому собраны точки.
    dots_window: Option<(f64, f64)>,
    /// Окно «Батиметрия водоёма».
    bathy: Option<crate::bathymetry::Dialog>,
    /// Вода растров с батиметрией и масштаб рельефа, по которому она
    /// построена. None - строить заново.
    water: Option<(Vec<render::FeatureMesh>, f64)>,
    /// Сетки «Моих меток» прошлой сборки: к ним добавляется вода.
    marks_meshes: Vec<render::FeatureMesh>,
    /// Магнитные полюса, линии склонения, скорости плит и сетки их линий.
    geo: crate::geophysics::Geo,
    geo_meshes: Vec<render::FeatureMesh>,
    /// Растущие треки точечных слоёв и сетки их путей.
    tracks: crate::tracks::Tracks,
    /// Помощник ИИ и метка, о которой его спросили «Объясни это»:
    /// путь и количество реплик разговора до вопроса.
    ai: crate::ai::Ai,
    /// «Моё место» по геолокации браузера и трек.
    gps: crate::gps::Gps,
    /// Снимки полярных шапок Esri.
    polar: crate::polar::PolarCaps,
    /// Ретранслятор MCP, не запустился ли он, и сообщение агента со
    /// снимком вида, которое ждёт картинки.
    #[cfg(not(target_arch = "wasm32"))]
    mcp: Option<crate::mcprelay::Relay>,
    #[cfg(not(target_arch = "wasm32"))]
    mcp_failed: bool,
    /// Агенты пущены флагом `--mcp` на этот запуск.
    #[cfg(not(target_arch = "wasm32"))]
    mcp_cli: bool,
    /// Издание: PlanetX Lite или RustPlanetX, `doc/EDITIONS.md`.
    edition: crate::Edition,
    /// Сообщение со снимком, куда ответить, когда пришло и снимок уже
    /// просили.
    #[cfg(not(target_arch = "wasm32"))]
    mcp_shot: Option<(String, std::sync::mpsc::Sender<Option<String>>, f64, bool)>,
    /// Связь с открытым RustGIS и флаг `--rustgis` этого запуска.
    #[cfg(not(target_arch = "wasm32"))]
    rustgis: Option<crate::rustgislink::Bridge>,
    #[cfg(not(target_arch = "wasm32"))]
    rustgis_cli: bool,
    /// Кадров подряд без загрузки и движения.
    idle_frames: u32,
    explaining: Option<(Vec<usize>, usize)>,
    /// Ответ помощника под строкой «Поиск»: реплик до вопроса.
    ai_answer_from: Option<usize>,
    /// Запрос места пользователя: не нашёлся - вопрос помощнику.
    ai_fallback: Option<String>,
    track_meshes: Vec<render::FeatureMesh>,
    /// Точки объектов треков надо собрать заново.
    track_dots_dirty: bool,
    /// Подписи объектов треков: название и точка ECEF.
    track_labels: Vec<(String, DVec3)>,
    /// Копия строк KML для пункта «Вставить».
    clip_kml: Option<String>,
    /// Вопрос перед удалением: заголовок, текст, что удалить.
    confirm: Option<(String, String, Confirmed)>,
    /// Окно «Разрез модели».
    model_section: Option<crate::modelsection::ModelSectionWindow>,
    /// Вопрос о «Временных метках» при закрытии окна, `ask_temporary`
    /// модуля.
    temporary_asking: bool,
    /// На вопрос ответили: окно закрывается.
    closing: bool,
    /// Окно «Свойства папки».
    folder_props: Option<crate::folderprops::FolderProps>,
    /// Окно свойств наложения и окно сетевой ссылки.
    overlay_props: Option<crate::overlayprops::OverlayProps>,
    /// Итоги окон панели свойств этого кадра, их разбирают окна.
    side: SideResults,
    /// Последний показ правки метки на глобусе.
    last_preview: Option<scene::kml::Place>,
    link_dialog: Option<crate::netlinks::LinkDialog>,
    netlinks: crate::netlinks::NetLinks,
    picked_box: PickedBox,
    /// Модели по ссылке на файл, в программе: путь - пакет.
    linked_models: HashMap<String, LinkedModel>,
    /// Окно названия: что сохраняется и название.
    name_dialog: Option<(NameFor, String)>,
    /// Подсказки строки «Поиск» открыты: строка была в фокусе.
    suggesting: bool,
    /// Запись тура: начало, позы (время, широта, долгота, расстояние,
    /// азимут, наклон).
    recording: Option<(f64, Vec<scene::tour::Sample>)>,
    /// Запись тура кадрами PNG, идёт вместе с проигрывателем.
    film: Option<crate::film::Film>,
    /// Снимок вида идёт: окна скрыты, кадр просят у egui.
    snapshot: Option<Snapshot>,
    /// Карточка метки, открытая щелчком по глобусу.
    card: Option<crate::card::Card>,
    /// Окно профиля высот пути или линейки.
    profile: Option<crate::profile::ProfileWindow>,
    /// Окно «Разрез» вниз вдоль пути.
    section: Option<crate::section::SectionWindow>,
    /// Действия «Моих меток» от окон, разбираются панелью в следующем
    /// кадре.
    queued: Vec<crate::marks::Action>,
    /// Модели с анимацией узлов: их матрицы - каждый кадр.
    animated: Vec<crate::models::Animated>,
    /// Номер набора наложений, по которому собраны картинки и модели.
    overlays_built: Option<u64>,
    /// Открыто демо самосвалов, рейс ещё не запущен.
    haul_pending: bool,
    /// Рейс самосвалов демо (начало, конец): шкала идёт по нему по кругу.
    haul: Option<(f64, f64)>,
}

/// Ящик прочитанных брошенных файлов.
type DroppedBox = Arc<std::sync::Mutex<Vec<(Result<Vec<u8>, String>, String)>>>;

/// Итоги окон панели свойств кадра: окно глобуса разбирает их после
/// панели.
#[derive(Default)]
struct SideResults {
    tool: Option<crate::tool::Action>,
    folder: Option<crate::folderprops::Outcome>,
    overlay: Option<crate::overlayprops::Outcome>,
    bathy: Option<crate::bathymetry::Answer>,
}

/// Огни городов NASA Black Marble на ночной стороне, `LIGHTS_URL`
/// модуля: снимок VIIRS 2016 года, уровни до 8.
const LIGHTS: net::Source = net::Source {
    name: "lights",
    url: "https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/VIIRS_Night_Lights/default/2016-01-01/GoogleMapsCompatible_Level8/{z}/{y}/{x}.png",
    max_requests: 4,
    max_level: render::LIGHTS_LEVEL,
    ext: "png",
    placeholders: false,
    raw: false,
    tilejson: None,
    region: None,
};
const LIGHTS_ATTRIBUTION: (&str, &str) = ("NASA Black Marble", "https://earthobservatory.nasa.gov/features/NightLights");

/// Для чего выбран файл картинки или модели.
#[derive(Debug, Clone, PartialEq)]
enum Pick {
    /// Новое наложение вида kind в папке по пути, как «Добавить» модуля.
    New(scene::overlays::Kind, Vec<usize>),
    /// «Обзор…» открытого окна свойств наложения.
    Browse,
    /// «3D-модель…» слоя-трека с этим номером окна «Треки».
    TrackModel(usize),
}

/// Выбранный файл: имя, путь на диске (в программе) и байты.
struct PickedFile {
    name: String,
    path: Option<String>,
    bytes: Vec<u8>,
}

/// Ящик выбранных файлов картинок и моделей: на странице браузер читает
/// их асинхронно.
type PickedBox = Arc<std::sync::Mutex<Vec<(Pick, Vec<PickedFile>)>>>;

/// Модель по ссылке на файл: время правки файла, время проверки и пакет.
type LinkedModel = (Option<f64>, f64, Option<Arc<Vec<u8>>>);

/// Количество меток в папке и вложенных папках.
fn count_places(folder: &scene::kml::Folder) -> usize {
    folder
        .children
        .iter()
        .map(|c| match c {
            scene::kml::Item::Place(_) => 1,
            scene::kml::Item::Folder(f) => count_places(f),
            _ => 0,
        })
        .sum()
}

/// Время правки файла на диске, секунды, или None. На странице файлов
/// на диске нет.
fn file_stamp_of(path: &str) -> Option<f64> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let modified = std::fs::metadata(path).ok()?.modified().ok()?;
        Some(modified.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs_f64())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        None
    }
}

/// Байты файла на диске или None. На странице файлов на диске нет.
fn read_local(path: &str) -> Option<Vec<u8>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read(path).ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        None
    }
}

/// Целое с пробелом между тысячами: 20 000.
pub(crate) fn group_thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('\u{a0}');
        }
        out.push(ch);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// Пункты меню «Демо» по разделам, как у модуля. Выбранное демо -
/// в clicked.
/// Пункты меню «Сцена»: сохранить, открыть, поделиться видом.
fn scene_items(ui: &mut egui::Ui, lang: Lang, scene: &mut Option<SceneAction>) {
    if ui.button(tr(lang, "Save Scene…")).clicked() {
        *scene = Some(SceneAction::Save);
        ui.close();
    }
    if ui.button(tr(lang, "Open Scene…")).clicked() {
        *scene = Some(SceneAction::Open);
        ui.close();
    }
    ui.separator();
    if ui.button(tr(lang, "Share View…")).on_hover_text(tr(lang, "Link to this view")).clicked() {
        *scene = Some(SceneAction::Share);
        ui.close();
    }
}

fn demo_items(ui: &mut egui::Ui, lang: Lang, clicked: &mut Option<&'static crate::demos::Demo>) {
    for (n, section) in crate::demos::MENU.iter().enumerate() {
        if n == crate::demos::BODIES_FROM {
            ui.separator();
        }
        ui.menu_button(tr(lang, section.en), |ui| {
            for demo in section.demos {
                if ui.button(tr(lang, demo.en)).clicked() {
                    *clicked = Some(demo);
                    ui.close();
                }
            }
        });
    }
}

/// Снимок вида по шагам, как «Снимок вида» модуля (`ui/snapshot.py`):
/// кадр без окон и панели инструментов, в нём глобус с подписью
/// источников. PNG ждёт сохранения в ячейке.
enum Snapshot {
    /// Окна скрыты, в этом кадре просится снимок.
    Hide(SnapshotTo),
    /// Снимок просили, ждётся картинка. Окна ещё скрыты: eframe снимает
    /// следующий кадр, панели попадали бы в снимок бледным следом.
    Wait(SnapshotTo, ShotCell),
}

/// Окна скрыты на всех шагах снимка.
fn hides_windows(snapshot: Option<&Snapshot>) -> bool {
    snapshot.is_some()
}

/// Куда идёт снимок: файл PNG («Снимок вида») или буфер обмена
/// («Копия вида», `copy_view` модуля).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotTo {
    File,
    Clipboard,
    /// Ответ агенту MCP на `snapshot`.
    Mcp,
    /// Кадр записи тура.
    Frame,
    /// Картинка в макет RustGIS, «Вид в макет QGIS» модуля.
    RustgisLayout,
}

/// Наибольшая сторона снимка вида для агента MCP, пикселей.
pub const MCP_SHOT_SIDE: u32 = 1280;

/// Окно глобуса - хозяин сервера MCP: вызовы - инструменты помощника.
#[cfg(not(target_arch = "wasm32"))]
struct McpHost<'a> {
    app: &'a mut LiteApp,
    ctx: &'a egui::Context,
    now: f64,
}

#[cfg(not(target_arch = "wasm32"))]
impl assistant::mcp::Host for McpHost<'_> {
    fn call(&mut self, call: &assistant::Call) -> assistant::mcp::Output {
        let text = self.app.ai_tool(self.ctx, call, self.now);
        // Предложенные агентом метки видны в окне помощника: запись
        // подтверждает пользователь.
        if call.name == "add_kml" && self.app.ai.proposal_text().is_some() {
            self.app.ai.open = true;
        }
        assistant::mcp::Output::Text(text)
    }
}

/// Снимок вида для агента ждёт конца перелёта и загрузки не дольше,
/// секунд.
pub const MCP_SHOT_WAIT: f64 = 10.0;
/// Качество JPEG снимка для агента.
pub const MCP_SHOT_QUALITY: u8 = 85;

/// Хозяин ответа на снимок: картинка JPEG уже снята.
#[cfg(not(target_arch = "wasm32"))]
struct ShotHost(Option<(Vec<u8>, String)>);

#[cfg(not(target_arch = "wasm32"))]
impl assistant::mcp::Host for ShotHost {
    fn call(&mut self, _: &assistant::Call) -> assistant::mcp::Output {
        match self.0.take() {
            Some((jpeg, caption)) => assistant::mcp::Output::Image(jpeg, "image/jpeg", caption),
            None => assistant::mcp::Output::Error("Снимок вида не снят.".into()),
        }
    }
}

/// Ползунок непрозрачности: путь, цвета объектов при выделении,
/// непрозрачность при выделении и сейчас.
type OpacityState = (Vec<usize>, Vec<(Vec<usize>, Vec<Option<scene::Rgba>>)>, f64, f64);

/// Сообщение строки состояния видно столько секунд, `MESSAGE_TIME` модуля.
pub const MESSAGE_TIME: f64 = 5.0;

/// Столько секунд главный файл шейпфайла на странице ждёт соседних:
/// браузер читает выбранные файлы по одному.
const SIDECAR_PAUSE: f64 = 0.5;

/// Наибольшая высота списков подсказок и найденных мест, `FOUND_HEIGHT`
/// модуля.
const FOUND_HEIGHT: f32 = 160.0;

/// Строка подсказки поиска, `hint_text` модуля: название и приписка вида -
/// метка, действие или прежний запрос.
fn hint_text(lang: Lang, item: &scene::searchbar::Suggestion) -> String {
    match item.kind {
        scene::searchbar::Kind::Place => trf(lang, "{name} - placemark", &[("name", &item.text)]),
        scene::searchbar::Kind::Action => trf(lang, "{name} - {what}", &[("name", &item.text), ("what", &item.note)]),
        scene::searchbar::Kind::History => trf(lang, "{name} - earlier query", &[("name", &item.text)]),
    }
}

/// Кнопка или клавиша панели тура.
#[derive(Debug, Clone, Copy, PartialEq)]
enum TourAct {
    Toggle,
    Stop,
    Prev,
    Next,
    Seek(f64),
    Loop,
    Captions,
    SetPause(f64),
    Film,
}

/// Описание остановки текстом для подписи: без разметки, не длиннее
/// 400 знаков, как `StopCaption` модуля.
fn caption_text(description: &str) -> String {
    let text: String = scene::balloon::pieces(description)
        .into_iter()
        .map(|p| match p {
            scene::balloon::Piece::Text(t) => t,
            scene::balloon::Piece::Link(t, _) => t,
        })
        .collect();
    let text = text.trim();
    if text.chars().count() > 400 { format!("{}…", text.chars().take(400).collect::<String>().trim_end()) } else { text.to_string() }
}

/// Что называет окно названия.
enum NameFor {
    View,
    Tour(Box<scene::kml::Place>),
    Measure(Box<scene::kml::Place>),
}

/// Что удалить после «Да».
#[derive(Debug, Clone)]
enum Confirmed {
    Remove(Vec<usize>),
    RemoveMany(Vec<Vec<usize>>),
    Clear,
    ClearTemporary,
}

/// Пункт меню «Сцена» панели значков.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SceneAction {
    Save,
    Open,
    Share,
}

/// Ячейка снимка: часть кадра или ошибка, её заполняет egui после кадра.
type ShotCell = Arc<std::sync::Mutex<Option<Result<egui::ColorImage, String>>>>;

/// Нужен ли кадру список наложений для моделей: набор сменился или
/// подошли высоты и набор сменится в этом кадре.
fn needs_models(built: Option<u64>, version: u64, due: bool) -> bool {
    built != Some(version) || due
}

/// Номер самой большой величины или None у пустого списка.
fn busiest(values: &[f64]) -> Option<usize> {
    values.iter().enumerate().filter(|(_, v)| v.is_finite()).max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i)
}

/// Часть rect снимка окна в точках, ppp - пикселей на точку.
pub(crate) fn crop(image: &egui::ColorImage, rect: egui::Rect, ppp: f32) -> Result<egui::ColorImage, String> {
    let [w, h] = image.size;
    let x0 = ((rect.min.x * ppp).round().max(0.0) as usize).min(w);
    let y0 = ((rect.min.y * ppp).round().max(0.0) as usize).min(h);
    let x1 = ((rect.max.x * ppp).round().max(0.0) as usize).clamp(x0, w);
    let y1 = ((rect.max.y * ppp).round().max(0.0) as usize).clamp(y0, h);
    if x1 == x0 || y1 == y0 {
        return Err("пустой снимок".into());
    }
    let mut pixels = Vec::with_capacity((x1 - x0) * (y1 - y0));
    for y in y0..y1 {
        pixels.extend_from_slice(image.pixels.get(y * w + x0..y * w + x1).unwrap_or_default());
    }
    Ok(egui::ColorImage::new([x1 - x0, y1 - y0], pixels))
}

/// PNG картинки снимка.
pub(crate) fn to_png(image: &egui::ColorImage) -> Result<Vec<u8>, String> {
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_srgba_unmultiplied()).collect();
    let [w, h] = image.size;
    let img = image::RgbaImage::from_raw(w as u32, h as u32, rgba).ok_or("пустой снимок")?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// Момент для солнца, секунды UTC: правый край открытой шкалы времени
/// или часы компьютера.
fn sun_moment(window: Option<(f64, f64)>) -> f64 {
    window.map_or_else(
        || web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64()),
        |w| w.1,
    )
}

/// Что нажали на панели инструментов.
#[derive(Default)]
struct ToolClicks {
    demo: Option<&'static crate::demos::Demo>,
}

impl LiteApp {
    /// scenario - самопроверка или None, её отчёт уходит в report.
    pub fn new(cc: &eframe::CreationContext<'_>, loaders: Loaders, scenario: Option<Scenario>, report: Report) -> Self {
        // Настройки прошлого запуска. Самопроверка - с настройками
        // по умолчанию.
        let settings = if scenario.is_none() {
            cc.storage.and_then(|s| s.get_string(crate::settings::KEY)).map(|t| Settings::from_text(&t)).unwrap_or_default()
        } else {
            Settings::default()
        };
        let lang = settings.lang();
        let vector_text = settings.vector.clone();
        let gibs_pending: Vec<String> = settings.maps.iter().filter(|k| k.starts_with(data::gibs::CATALOG_PREFIX)).cloned().collect();
        let extras_on: std::collections::BTreeSet<crate::layers::Extra> = [(crate::layers::Extra::Air, settings.air), (crate::layers::Extra::Stars, settings.stars), (crate::layers::Extra::Sun, settings.sun), (crate::layers::Extra::Grid, settings.grid)]
            .into_iter()
            .filter_map(|(e, on)| on.then_some(e))
            .chain(settings.maps.iter().filter_map(|k| crate::maps::index(k)).map(crate::layers::Extra::Map))
            .chain(settings.dots.iter().filter_map(|k| crate::dots::Layer::from_key(k)).map(crate::layers::Extra::Dots))
            .collect();
        // Тёмная тема в цветах модуля по умолчанию, решения автора
        // от 23 октября 2026 года.
        crate::theme::apply(&cc.egui_ctx, settings.dark);
        let render_state = cc.wgpu_render_state.clone();
        let globe = render_state.as_ref().map(|rs| Globe::new(&rs.device, &rs.queue));
        let camera = Camera::new(Ellipsoid::wgs84(), DVec3::ZERO, DMat3::IDENTITY);
        let pose = match scenario {
            Some(s) => s.start_pose(),
            None => Pose::new(START.0, START.1, START.2, 0.0, 0.0),
        };
        let heights = Heights::default();
        // Снятый флажок «Рельефа» - Земля гладкая, как у модуля.
        let scale = if settings.relief { settings.scale } else { 0.0 };
        if (scale - 1.0).abs() > f64::EPSILON {
            heights.write().set_scale(scale);
        }
        let mut nav = Navigator::new(camera, pose);
        let source = heights.clone();
        // Камера под землёй: в рамке подземной модели рельеф навигации -
        // её низ, `GlobeView.floor` модуля.
        let floor: FloorCell = Arc::new(RwLock::new(None));
        let below = floor.clone();
        nav.set_terrain(Some(Arc::new(move |lat, lon| {
            let store = source.read();
            let h = store.height_at(lat, lon);
            let f = *below.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            f.and_then(|f| f.at(lat, lon, store.scale(), || store.heights_at(&[(lat, lon)], false).first().copied().unwrap_or(0.0))).map_or(h, |low| low.min(h))
        })));
        let labels = Labels::new(lang, &settings.labels);
        let mut app = Self {
            lang,
            render_state,
            globe,
            texture: None,
            nav,
            loaders,
            basemap: scenario.map_or(Basemap::default(), Scenario::start_basemap),
            heights,
            input: Input::default(),
            memo: Memo::new(),
            check: scenario.map(Check::new),
            report,
            shot: None,
            labels,
            search: Search::new(Some(lang.code())),
            settings: settings.clone(),
            settings_open: false,
            panel_open: true,
            web_fonts: HashMap::new(),
            font_fetching: None,
            fonts_dirty: false,
            toolbar_rect: None,
            settings_dirty: false,
            layout_checked: false,
            search_text: String::new(),
            credits: Credits::default(),
            marks: Marks::default(),
            pending_open: Vec::new(),
            open_message: None,
            map_opacity: settings.opacity.iter().cloned().collect(),
            subsurface: None,
            cutaway: None,
            own_dialog: None,
            own_caps: None,
            sheets_window: None,
            links_window: None,
            viewshed: None,
            insolation: None,
            shadows: None,
            aef_change: None,
            aef_class: None,
            similar: None,
            tracing: None,
            object_tasks: None,
            recognize: None,
            names_for: None,
            spectral: None,
            emit: None,
            preview: None,
            preview_dirty: false,
            data_pictures_seen: 0,
            data_extent_seen: None,
            site_layer: crate::analysis::SiteLayer::default(),
            band_rasters: Vec::new(),
            area_site: None,
            relief_calc: None,
            view_contours: None,
            analysis_sessions: HashMap::new(),
            hydro: crate::hydrocalc::Hydro::new(),
            earthwork: crate::earthwork::Earthwork::default(),
            volumes: None,
            wayback: None,
            extrude: None,
            sounding: None,
            calc_keys: Vec::new(),
            etomesto: None,
            paleo: None,
            planet: scene::planets::planet_by_key("earth"),
            earth_away: None,
            cog_base: None,
            geoid: None,
            live_keys: Vec::new(),
            live_points: Vec::new(),
            live_labels: Vec::new(),
            sat_points: Vec::new(),
            sat_shown: Vec::new(),
            osm_mines: None,
            osm_infra: None,
            overpass_mirrors: crate::overpass::Mirrors::default(),
            solid_stamp: None,
            export_models: Vec::new(),
            field_tiles: None,
            building_keys: HashSet::new(),
            building_only: HashSet::new(),
            parcel_ask: None,
            parcels: Vec::new(),
            plugged: BTreeMap::new(),
            rumsey: None,
            oam: None,
            sentinel: None,
            live_objects: Vec::new(),
            live_follow: None,
            followed: None,
            flight: None,
            floor,
            open_dir: None,
            route_keys: HashSet::new(),
            route_only: HashSet::new(),
            water_tiles: crate::watermask::WaterTiles::default(),
            water_keys: HashSet::new(),
            water_only: HashSet::new(),
            route: crate::route::Router::new(data::routing::Mode::from_name(&settings.route).unwrap_or(data::routing::Mode::Car)),
            dropped_box: DroppedBox::default(),
            data_layers: crate::datalayers::DataLayers::default(),
            tilesets: crate::tilesets::Tilesets::default(),
            my_layers_saved: None,
            addins_saved: None,
            #[cfg(target_arch = "wasm32")]
            addins_store: Vec::new(),
            #[cfg(feature = "addins")]
            addins: crate::addins::Addins::default(),
            tilesets_saved: None,
            tiles_restoring: HashMap::new(),
            opening_path: None,
            user_dir: None,
            new_folder: String::new(),
            data_meshes: Vec::new(),
            sidecars: Vec::new(),
            waiting: Vec::new(),
            waiting_since: 0.0,
            shot_ui: false,
            player: None,
            tour_index: None,
            timebar: TimeBar::default(),
            scene_layers: HashMap::new(),
            pictures: Pictures::default(),
            screen_textures: HashMap::new(),
            keep_pose: false,
            scene_source: None,
            fetching: None,
            demo_cache: None,
            demo_queue: std::collections::VecDeque::new(),
            demo_fetching: None,
            url_queue: std::collections::VecDeque::new(),
            fetching_url: None,
            shared: Vec::new(),
            places_dirty: false,
            tree_state: marks::TreeState::default(),
            relief: Relief::default(),
            clouds: Vec::new(),
            models: crate::models::Models::default(),
            movers: Vec::new(),
            animated: Vec::new(),
            tool: None,
            suggesting: false,
            navpad: crate::navpad::NavPad::default(),
            touch: crate::share::touch_screen(),
            identify: false,
            vector: crate::vector::VectorBase::from_text(&vector_text),
            extras: extras_on.clone(),
            extras_on,
            layers_menu: None,
            catalog: crate::layers::Catalog::default(),
            opacity: None,
            layers_height: 120.0,
            message_seen: None,
            cursor_point: None,
            name_dialog: None,
            clip_kml: None,
            globe_menu: None,
            loading_since: None,
            label_keys: std::collections::HashSet::new(),
            pending: HashMap::new(),
            map_loaders: HashMap::new(),
            ready_loader: None,
            lights_loader: None,
            stars: None,
            sky_image: None,
            sky: None,
            sky_saved: None,
            surveys: crate::skysurvey::Surveys::default(),
            theme_days: HashMap::new(),
            gibs_catalog: None,
            events: crate::events::Events::default(),
            pending_event: None,
            event_empty: Default::default(),
            pair_owner: 0,
            globe_width_px: 0.0,
            swipe_drag: false,
            layer_swipe: None,
            lidar: None,
            lidar_credits: Vec::new(),
            gibs_pending,
            theme_scales: HashMap::new(),
            lasting: HashMap::new(),
            drawn: std::collections::HashSet::new(),
            was_moving: false,
            objects: None,
            map_ask: None,
            dots: crate::dots::Dots::default(),
            dots_window: None,
            bathy: None,
            water: None,
            marks_meshes: Vec::new(),
            geo: crate::geophysics::Geo::default(),
            geo_meshes: Vec::new(),
            tracks: crate::tracks::Tracks::default(),
            ai: crate::ai::Ai::new(),
            gps: crate::gps::Gps::default(),
            polar: crate::polar::PolarCaps::default(),
            #[cfg(not(target_arch = "wasm32"))]
            mcp: None,
            #[cfg(not(target_arch = "wasm32"))]
            mcp_failed: false,
            #[cfg(not(target_arch = "wasm32"))]
            mcp_cli: false,
            edition: crate::Edition::Lite,
            #[cfg(not(target_arch = "wasm32"))]
            mcp_shot: None,
            #[cfg(not(target_arch = "wasm32"))]
            rustgis: None,
            #[cfg(not(target_arch = "wasm32"))]
            rustgis_cli: false,
            idle_frames: 0,
            film: None,
            explaining: None,
            ai_answer_from: None,
            ai_fallback: None,
            track_meshes: Vec::new(),
            track_dots_dirty: false,
            track_labels: Vec::new(),
            confirm: None,
            model_section: None,
            temporary_asking: false,
            closing: false,
            folder_props: None,
            overlay_props: None,
            side: SideResults::default(),
            last_preview: None,
            link_dialog: None,
            netlinks: crate::netlinks::NetLinks::default(),
            picked_box: PickedBox::default(),
            linked_models: HashMap::new(),
            recording: None,
            snapshot: None,
            card: None,
            profile: None,
            section: None,
            queued: Vec::new(),
            overlays_built: None,
            haul_pending: false,
            haul: None,
        };
        // «Мои метки» прошлого запуска. Самопроверка начинает с пустых.
        if app.check.is_none()
            && let Some(root) = cc.storage.and_then(|s| s.get_string(places::KEY)).and_then(|t| places::from_text(&t))
        {
            let mut root = root;
            root.name = marks::root_name(lang).to_string();
            app.marks.set(root, "");
            app.marks_changed();
            app.places_dirty = false;
        }
        // «Мои слои» прошлого запуска. Файлы их объектов лежат в папке
        // данных, она известна после запуска - слои читаются первым кадром.
        if app.check.is_none() {
            app.my_layers_saved = cc.storage.and_then(|s| s.get_string(crate::datalayers::KEY));
            app.addins_saved = cc.storage.and_then(|s| s.get_string(ADDINS_KEY));
            #[cfg(target_arch = "wasm32")]
            {
                app.addins_store = addins_from_storage(cc.storage, app.addins_saved.as_deref());
            }
            app.tilesets_saved = cc.storage.and_then(|s| s.get_string(crate::tilesets::KEY));
        }
        // «Временные метки» прошлого запуска, строка выделена: новое
        // ложится туда, `select_temporary` модуля. Самопроверка - без них.
        if app.check.is_none() {
            let saved = cc.storage.and_then(|s| s.get_string(places::TEMP_KEY)).and_then(|t| places::from_text(&t));
            let mut folder = saved.unwrap_or_else(|| scene::kml::Folder::new(""));
            folder.name = tr(lang, "Temporary Places").to_string();
            app.marks.set_temporary(folder, marks::root_name(lang));
            app.tree_state.selected = Some(marks::TEMPORARY.to_vec());
            app.marks_changed();
            app.places_dirty = false;
        }
        app.install_fonts(&cc.egui_ctx);
        if let Some(g) = crate::glyphs::digits() {
            app.vector.set_glyphs(Arc::new(g));
        }
        app.sync_maps();
        for layer in crate::dots::Layer::ALL {
            if app.extras_on.contains(&crate::layers::Extra::Dots(layer)) {
                let loaders = &app.loaders;
                app.dots.enable(layer, &|| (loaders.request)(layer.bytes()));
            }
        }
        // Основа прошлого запуска. Самопроверка ставит свою.
        if app.check.is_none()
            && let Some(b) = Basemap::from_key(&app.settings.basemap)
        {
            app.set_basemap(b);
        }
        app
    }

    /// Панель значков в левом верхнем углу глобуса, `ui/toolbar.py`
    /// модуля. Кнопки и их порядок - как у модуля, решение автора
    /// от 24 октября 2026 года «порядок кнопок возьми тем же». Без
    /// кнопок QGIS: синхронизации с картой и вида в макет.
    fn toolbar(&mut self, ctx: &egui::Context, view: egui::Rect, now: f64) {
        use crate::theme::{ICON_SIDE, Tool, tool_button, tool_item};
        let lang = self.lang;
        // Новое касание убирает подсказку долгого нажатия.
        if ctx.input(|i| i.pointer.any_pressed()) {
            crate::theme::clear_touch_tip(ctx);
        }
        // F11 - во весь экран и обратно, Esc - выход из него, как у модуля.
        let on = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        let (f11, esc) = ctx.input(|i| (i.key_pressed(egui::Key::F11), i.key_pressed(egui::Key::Escape)));
        let typing = ctx.memory(|m| m.focused().is_some());
        if f11 || (esc && on && !typing) {
            self.set_fullscreen(ctx, !on);
        }
        // Во весь экран панель значков прячется, органы навигации, шкала
        // времени и тур остаются.
        if on {
            self.toolbar_rect = None;
            return;
        }
        let mut clicks = ToolClicks::default();
        let mut panel = self.panel_open;
        let mut settings = self.settings_open;
        let mut identify = self.identify;
        let mut time = self.timebar.open;
        let time_ready = self.timebar.extent.is_some();
        let mut new_tool: Option<crate::tool::Tool> = None;
        let mut about = false;
        let mut full = false;
        let mut record = false;
        let mut save_view = false;
        let mut snapshot: Option<SnapshotTo> = None;
        let mut scene: Option<SceneAction> = None;
        let mut body: Option<&'static str> = None;
        let mut sky_flags = (self.settings.constellations, self.settings.deepsky, self.settings.sky_imagery);
        let mut sky_changed = false;
        let recording = self.recording.is_some();
        let measuring = matches!(self.tool.as_ref().map(|t| &t.purpose), Some(crate::tool::Purpose::Ruler));
        let other_body = !self.planet.earth;
        // Синхронизация с картой RustGIS - кнопка-переключатель, пока есть
        // связь, как значок синхронизации модуля. На других телах её нет.
        #[cfg(not(target_arch = "wasm32"))]
        let sync_button = self.rustgis.as_ref().map(|b| (b.sync, b.connected && !other_body));
        #[cfg(target_arch = "wasm32")]
        let sync_button: Option<(bool, bool)> = None;
        let mut sync_toggle = false;
        // «Вид в макет RustGIS» - пока есть связь, как «Вид в макет QGIS»
        // модуля при открытом проекте.
        let layout_button = sync_button.is_some_and(|(_, ready)| ready);
        let current =if self.sky.is_some() { "sky" } else { self.planet.key() };
        // Узкий экран, телефон: все значки панели в ширину вида не входят.
        // На панели остаются частые, прочие - строками с названиями
        // в меню «…».
        let buttons = if cfg!(target_arch = "wasm32") { 14.0 } else { 15.0 };
        let narrow = view.width() < buttons * (ICON_SIDE + 8.0) + 60.0;
        // Уровень панели - как у окон: открытое окно встаёт поверх неё.
        // На уровне Foreground панель закрывала окно «Источники данных»,
        // замечание автора от 23 октября 2026 года.
        let area = egui::Area::new(egui::Id::unique("planetx-toolbar"))
            .order(egui::Order::Middle)
            .fixed_pos(view.left_top() + egui::vec2(8.0, 8.0))
            .show(ctx, |ui| {
                // Значки от «Сохранить вид» до «Демо»: на панели
                // или строками меню «…» (rows).
                let mut extra = |ui: &mut egui::Ui, rows: bool| {
                    if tool_item(ui, rows, Tool::Bookmark, false, true, tr(lang, "Save view"), tr(lang, "Save view. The point of view becomes a placemark in My Places, flying to it returns the height, heading and tilt.")).clicked() {
                        save_view = true;
                    }
                    if tool_item(ui, rows, Tool::Record, recording, true, tr(lang, "Record a tour"), tr(lang, "Record a tour. Camera motion by mouse, keys or flights becomes a tour in My Places, a second click stops the recording.")).clicked() {
                        record = true;
                    }
                    // Шкала времени доступна, когда у видимых меток есть
                    // время, как `set_time_available` модуля.
                    if tool_item(ui, rows, Tool::Time, time && time_ready, time_ready, tr(lang, "Time slider"), tr(lang, "Time slider. While it is open, data outside its period is hidden. The closed slider shows all placemarks.")).clicked() {
                        time = !time;
                    }
                    if tool_item(ui, rows, Tool::Camera, false, true, tr(lang, "Snapshot of the view"), tr(lang, "Snapshot of the view to a PNG file.")).clicked() {
                        snapshot = Some(SnapshotTo::File);
                    }
                    if tool_item(ui, rows, Tool::Copy, false, true, tr(lang, "Copy the view to the clipboard"), tr(lang, "Copy of the view to the clipboard - the globe with labels, without the toolbar, the navigation controls and the time slider.")).clicked() {
                        snapshot = Some(SnapshotTo::Clipboard);
                    }
                    if layout_button && tool_item(ui, rows, Tool::Layout, false, true, tr(lang, "View to the RustGIS layout"), tr(lang, "View to a RustGIS layout as a fixed picture stored in the project. The picture goes to the only layout, otherwise to the open one, otherwise to a new A4 layout.")).clicked() {
                        snapshot = Some(SnapshotTo::RustgisLayout);
                    }
                    let scene_tip = tr(lang, "Scene - the camera and the selected folder of My Places with its tour. It is saved to a file and opens on another computer.");
                    let demo_tip = tr(lang, "Demo: prepared scenes with placemarks and tours on the Earth, Mars, the Moon and in the sky. The placemarks go to a new folder of My Places, the ▶ button under the list plays the tour.");
                    if rows {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            crate::theme::menu_icon(ui, Tool::Scene);
                            ui.menu_button(tr(lang, "Scene"), |ui| scene_items(ui, lang, &mut scene)).response.on_hover_text(scene_tip);
                        });
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            crate::theme::menu_icon(ui, Tool::Demo);
                            ui.menu_button(tr(lang, "Demo"), |ui| demo_items(ui, lang, &mut clicks.demo)).response.on_hover_text(demo_tip);
                        });
                    } else {
                        let scene_button = tool_button(ui, Tool::Scene, false, true, true, scene_tip);
                        egui::Popup::menu(&scene_button).show(|ui| scene_items(ui, lang, &mut scene));
                        let demo = tool_button(ui, Tool::Demo, false, true, true, demo_tip);
                        egui::Popup::menu(&demo).show(|ui| demo_items(ui, lang, &mut clicks.demo));
                    }
                };
                // Во весь экран и «О программе» - в конце панели.
                let mut tail = |ui: &mut egui::Ui, rows: bool| {
                    // У страницы полный экран - средствами браузера, команда
                    // окна eframe там ничего не делает: кнопки нет.
                    if !cfg!(target_arch = "wasm32") && tool_item(ui, rows, Tool::Fullscreen, false, true, tr(lang, "Full screen"), tr(lang, "Full screen (F11) - the globe view for a show. Esc or F11 returns the window.")).clicked() {
                        full = true;
                    }
                    if tool_item(ui, rows, Tool::Info, false, true, tr(lang, "About"), if rows { "" } else { tr(lang, "About") }).clicked() {
                        about = true;
                    }
                };
                crate::theme::plate().inner_margin(egui::Margin::same(3)).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
                    ui.horizontal(|ui| {
                        let tip = if panel { tr(lang, "Hide sidebar") } else { tr(lang, "Show sidebar") };
                        if tool_button(ui, Tool::Sidebar, false, false, true, tip).clicked() {
                            panel = !panel;
                        }
                        if let Some((on, ready)) = sync_button
                            && tool_button(ui, Tool::Sync, on, false, ready, tr(lang, "Synchronisation with the RustGIS map window. The direction is in the view properties.")).clicked()
                        {
                            sync_toggle = true;
                        }
                        if tool_button(ui, Tool::Identify, identify, false, true, tr(lang, "Identify. A click on the globe shows the coordinates, the height or depth of the point and the placemarks there.")).clicked() {
                            identify = !identify;
                        }
                        if tool_button(ui, Tool::Ruler, false, false, true, tr(lang, "Ruler. Length, perimeter and area on the ellipsoid, the measurement can be saved to My Places.")).clicked() {
                            new_tool = Some(if measuring { crate::tool::Tool::closed() } else { crate::tool::Tool::ruler() });
                        }
                        // Новая метка - меню вида объекта, пункт открывает
                        // окно «Новая метка» на своей вкладке, как у модуля.
                        let place = tool_button(ui, Tool::Place, false, true, true, tr(lang, "New placemark, path, polygon or circle in My Places."));
                        egui::Popup::menu(&place).show(|ui| {
                            for (mode, title) in [
                                (crate::ruler::Mode::Point, "Placemark"),
                                (crate::ruler::Mode::Path, "Path"),
                                (crate::ruler::Mode::Polygon, "Polygon"),
                                (crate::ruler::Mode::Circle, "Circle"),
                            ] {
                                if ui.button(tr(lang, title)).clicked() {
                                    new_tool = Some(crate::tool::Tool::place(mode));
                                    ui.close();
                                }
                            }
                        });
                        if !narrow {
                            extra(ui, false);
                        }
                        // Тело глобуса, меню `MENU` модуля: Земля сверху, под
                        // разделителем Луна и Марс, прочие тела - подменю.
                        let body_button = tool_button(ui, Tool::Body, other_body, true, true, tr(lang, "Body of the globe - a planet, a moon or a dwarf planet. Each body has its own imagery, Mars and the Moon also have terrain. Earth layers, search and buildings are off on other bodies."));
                        egui::Popup::menu(&body_button).show(|ui| {
                            for (n, (group, keys)) in scene::planets::MENU.iter().enumerate() {
                                let mut items = |ui: &mut egui::Ui| {
                                    for key in keys.iter() {
                                        if ui.radio(current == *key, tr(lang, body_name(key))).clicked() {
                                            body = Some(*key);
                                            ui.close();
                                        }
                                    }
                                };
                                if group.is_empty() {
                                    if n > 0 {
                                        ui.separator();
                                    }
                                    items(ui);
                                } else {
                                    ui.menu_button(tr(lang, body_group_name(group)), items);
                                }
                            }
                            ui.separator();
                            if ui.radio(current == "sky", tr(lang, "Sky")).on_hover_text(tr(lang, "The starry sky from the centre of the celestial sphere: stars, constellations, the Sun, the Moon and the planets. Dragging turns the sky, the wheel changes the field of view.")).clicked() {
                                body = Some("sky");
                                ui.close();
                            }
                            // Флажки неба вне группы тел, как у модуля.
                            ui.separator();
                            if ui.checkbox(&mut sky_flags.0, tr(lang, "Constellations")).on_hover_text(tr(lang, "Figure lines and names of the constellations in the sky. Without them the stars, their names and the Sun, the Moon and the planets remain.")).changed() {
                                sky_changed = true;
                            }
                            if ui.checkbox(&mut sky_flags.2, tr(lang, "Sky imagery when zooming in")).on_hover_text(tr(lang, "Closer than a field of view of about 50° the detailed NASA sky map appears instead of the Milky Way picture, closer than 6° - the DSS survey images. Without the check the sky is the Milky Way picture and the catalogue stars.")).changed() {
                                sky_changed = true;
                            }
                            if ui.checkbox(&mut sky_flags.1, tr(lang, "Deep-sky objects")).on_hover_text(tr(lang, "Labels of galaxies, nebulae and clusters of the Messier, NGC and IC catalogues. A click on a label opens the card of the object.")).changed() {
                                sky_changed = true;
                            }
                        });
                        if tool_button(ui, Tool::Settings, settings, false, true, tr(lang, "View properties: relief scale, language of labels, appearance.")).clicked() {
                            settings = !settings;
                        }
                        if narrow {
                            let more = tool_button(ui, Tool::More, false, true, true, tr(lang, "More tools"));
                            egui::Popup::menu(&more).show(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                extra(ui, true);
                                ui.separator();
                                tail(ui, true);
                            });
                        } else {
                            tail(ui, false);
                        }
                    });
                });
            });
        self.toolbar_rect = Some(area.response.rect);
        // Подсказка долгого нажатия - под панелью, в ширину экрана.
        if let Some(text) = crate::theme::touch_tip(ctx) {
            let width = (view.width() - 16.0).clamp(80.0, 360.0);
            egui::Area::new(egui::Id::unique("planetx-touch-tip-area"))
                .order(egui::Order::Tooltip)
                .fixed_pos(area.response.rect.left_bottom() + egui::vec2(0.0, 6.0))
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(width);
                        ui.label(text);
                    });
                });
        }
        self.spinner(ctx, area.response.rect, now);
        #[cfg(not(target_arch = "wasm32"))]
        if sync_toggle && let Some(b) = &mut self.rustgis {
            let on = !b.sync;
            b.set_sync(on);
        }
        #[cfg(target_arch = "wasm32")]
        let _ = sync_toggle;
        self.panel_open = panel;
        self.settings_open = settings;
        self.identify = identify;
        self.timebar.open = time && time_ready;
        self.credits.about_request |= about;
        if full {
            self.set_fullscreen(ctx, true);
        }
        if let Some(to) = snapshot {
            self.snapshot = Some(Snapshot::Hide(to));
        }
        if record {
            self.toggle_record(now);
        }
        if sky_changed {
            (self.settings.constellations, self.settings.deepsky, self.settings.sky_imagery) = sky_flags;
            self.settings_dirty = true;
        }
        if let Some(key) = body {
            // «Небо» - вид из центра сферы, тело под ним остаётся.
            if key == "sky" {
                self.enter_sky();
            } else {
                self.leave_sky();
                self.set_body(key);
            }
        }
        if save_view {
            let name = scene::placetree::numbered_name(tr(lang, "View"), &self.marks.names());
            self.name_dialog = Some((NameFor::View, name));
        }
        if let Some(tool) = new_tool {
            // Повторное нажатие «Линейки» закрывает её. Открытая правка
            // метки закрывается «Отменой».
            self.close_side();
            self.tool = tool.open.then_some(tool);
        }
        match scene {
            Some(SceneAction::Save) => {
                let folder = self.tree_state.selected.clone().unwrap_or_default();
                self.save_marks(&folder, crate::export::Kind::Scene);
            }
            Some(SceneAction::Open) => self.pick_file(ctx, now),
            Some(SceneAction::Share) => self.share(ctx),
            None => {}
        }
        if let Some(demo) = clicks.demo {
            self.open_demo(demo, now);
        }
    }

    /// Значок загрузки справа от панели значков, как `BusySpinner`
    /// модуля: дуга цвета выделения, видна через 0.4 с после начала
    /// загрузки.
    fn spinner(&self, ctx: &egui::Context, bar: egui::Rect, now: f64) {
        let Some(since) = self.loading_since.filter(|s| now - s >= SPINNER_DELAY) else {
            return;
        };
        let side = 26.0;
        let center = egui::pos2(bar.right() + 8.0 + side / 2.0, bar.center().y);
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::unique("planetx-spinner")));
        let start = ((now - since) * std::f64::consts::TAU) as f32;
        let points: Vec<egui::Pos2> = (0..=24).map(|i| {
            let a = start + i as f32 / 24.0 * 4.4;
            center + egui::vec2(a.cos(), a.sin()) * (side / 2.0 - 3.0)
        }).collect();
        painter.add(egui::Shape::line(points, egui::Stroke::new(3.0, crate::theme::pal().accent)));
        ctx.request_repaint();
    }

    /// Во весь экран или обратно, с сообщением, как `toggle_fullscreen`
    /// модуля.
    fn set_fullscreen(&mut self, ctx: &egui::Context, on: bool) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
        if on {
            self.open_message = Some(tr(self.lang, "Full screen. Exit - Esc or F11.").to_string());
        }
    }

    /// Меню на глобусе в точке pos окна.
    fn make_globe_menu(&self, pos: egui::Pos2, rect: egui::Rect, ppp: f64) -> crate::globemenu::GlobeMenu {
        let (px, py) = (f64::from(pos.x - rect.min.x) * ppp, f64::from(pos.y - rect.min.y) * ppp);
        let point = globe_core::navigation::ground_under(&self.nav.camera, px, py, self.nav.pose.terrain.as_ref()).map(|p| {
            let (lat, lon, _) = self.nav.camera.ellipsoid.ecef_to_geodetic(p);
            (lat, lon)
        });
        let coords = point.map_or(String::new(), |(lat, lon)| self.point_text(lat, lon, None, 6));
        let place = self.marks.tree.as_ref().and_then(|t| crate::card::pick(t, &self.nav, px, py, crate::card::PICK_PIXELS * ppp)).and_then(|path| {
            let name = self.marks.place(&path).map(|p| p.name.clone())?;
            Some((path, name))
        });
        let drawing = self.tool.as_ref().filter(|t| t.open && !t.ruler.points.is_empty()).map(|t| {
            let radius = crate::card::PICK_PIXELS * ppp;
            let vertex = t.ruler.points.iter().position(|(lat, lon)| {
                let h = self.nav.pose.terrain.as_ref().map_or(0.0, |f| f(*lat, *lon));
                let (q, front) = self.nav.camera.project(self.nav.camera.ellipsoid.geodetic_to_ecef(*lat, *lon, h));
                front && (q.x - px).hypot(q.y - py) <= radius
            });
            crate::globemenu::Drawing {
                vertex,
                finished: t.ruler.finished,
                can_save: t.ruler.figure(false).is_some(),
                point: t.ruler.mode == crate::ruler::Mode::Point,
            }
        });
        // Модель на пути под курсором, иначе та, за которой идёт камера.
        let store = self.heights.read();
        let window = self.timebar.window();
        let radius = crate::card::PICK_PIXELS * ppp * 2.0;
        let under = self.movers.iter().position(|m| {
            let (q, front) = self.nav.camera.project(m.place(window, &store).center);
            front && (q.x - px).hypot(q.y - py) <= radius
        });
        let mover = under.or(self.followed).and_then(|i| self.movers.get(i).map(|m| (i, m.name.clone(), self.followed == Some(i))));
        // Живой объект под курсором, иначе тот, за которым идёт камера.
        let e = self.nav.camera.ellipsoid;
        let live = self
            .live_objects
            .iter()
            .filter_map(|(kind, o)| {
                let h = if o.alt.is_finite() { o.alt * store.scale() } else { store.height_at(o.lat, o.lon) };
                let (q, front) = self.nav.camera.project(e.geodetic_to_ecef(o.lat, o.lon, h));
                let gap = (q.x - px).hypot(q.y - py);
                (front && gap <= crate::live::PICK_PIXELS * ppp * 2.0).then_some((gap, kind, o))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, kind, o)| (*kind, o.key.clone(), o.label.clone()))
            .or_else(|| {
                let (kind, key) = self.live_follow.clone()?;
                let label = self.live_objects.iter().find(|(k, o)| *k == kind && o.key == key).map_or(key.clone(), |(_, o)| o.label.clone());
                Some((kind, key, label))
            })
            .map(|(kind, key, label)| {
                let on = self.live_follow.as_ref().is_some_and(|(k, f)| *k == kind && *f == key);
                (kind, key, label, on)
            });
        let sheets = point.map_or(Vec::new(), |(lat, lon)| crate::sheets::items(self.lang, lat, lon));
        let links = self.settings.links.clone();
        // Спутник под курсором, иначе выбранный.
        let satellite = self.plugged.values().find_map(|l| l.satellites()).and_then(|s| {
            let number = self.satellite_under(px, py, ppp).or(s.selected)?;
            let name = s.name_of(number).unwrap_or_else(|| number.to_string());
            Some((number, name, s.selected == Some(number), s.follow && s.selected == Some(number)))
        });
        let earth = self.planet.earth;
        let relief = earth || self.planet.terrain.is_some();
        let rasters = !self.open_rasters().is_empty();
        let contours = self.vector.contours_on();
        crate::globemenu::GlobeMenu { at: pos, point, coords, place, drawing, mover, live, sheets, links, satellite, earth, relief, rasters, contours }
    }

    /// Выбор в меню на глобусе.
    fn globe_menu_choice(&mut self, ctx: &egui::Context, choice: crate::globemenu::Choice, point: Option<(f64, f64)>, place: Option<(Vec<usize>, String)>, now: f64) {
        use crate::globemenu::Choice;
        match choice {
            Choice::CopyCoords(text) => {
                ctx.copy_text(text.clone());
                self.open_message = Some(trf(self.lang, "Copied: {number}", &[("number", &text)]));
            }
            Choice::RemoveVertex(i) => {
                if let Some(t) = &mut self.tool
                    && i < t.ruler.points.len()
                {
                    t.ruler.points.remove(i);
                }
            }
            Choice::Finish => {
                if let Some(t) = &mut self.tool {
                    t.ruler.finished = true;
                }
            }
            Choice::Continue => {
                if let Some(t) = &mut self.tool {
                    t.ruler.finished = false;
                }
            }
            Choice::Save => {
                if let Some(t) = &mut self.tool {
                    t.save_request = true;
                }
            }
            Choice::Clear => {
                if let Some(t) = &mut self.tool {
                    t.ruler.clear();
                }
            }
            Choice::Properties(path) => self.queued.push(crate::marks::Action::Properties(path)),
            Choice::AddPlacemark | Choice::Measure => {
                let Some(p) = point else {
                    return;
                };
                let mut tool = if choice == Choice::AddPlacemark { crate::tool::Tool::place(crate::ruler::Mode::Point) } else { crate::tool::Tool::ruler() };
                tool.ruler.points.push(p);
                self.set_tool(tool);
            }
            Choice::FlyHere => {
                if let Some((lat, lon)) = point {
                    let (distance, heading, tilt) = (self.nav.pose.distance, self.nav.pose.heading, self.nav.pose.tilt);
                    crate::input::fly(&mut self.nav, Pose::new(lat, lon, distance, heading, tilt), now);
                }
            }
            Choice::Spin => {
                if let Some((lat, lon)) = point {
                    let spin = globe_core::flight::Spin::new(&self.nav.camera.ellipsoid, &self.nav.pose, lat, lon, self.nav.camera.fov_y);
                    self.nav.start_flight(globe_core::flight::Motion::Spin(spin), now);
                }
            }
            Choice::WhatHere => {
                if let Some((lat, lon)) = point {
                    self.search.reverse(lat, lon);
                    self.ask_map_value(lat, lon);
                    self.objects = Some(self.make_objects(lat, lon, None, Some(tr(self.lang, "loading").to_string())));
                }
            }
            Choice::FlyFrom(kind) => {
                if let Some((lat, lon)) = point {
                    let ground = self.heights.read().heights_at(&[(lat, lon)], false).first().copied().unwrap_or(0.0).max(0.0);
                    let heading = self.nav.pose.heading;
                    self.live_follow = None;
                    self.followed = None;
                    self.flight = Some(crate::flight::Flight::start(lat, lon, ground, heading, kind, now));
                    self.nav.show(Pose::new(lat, lon, crate::flight::CHASE_REACH, heading, crate::flight::CHASE_TILT));
                }
            }
            Choice::Follow(index) => {
                self.followed = index;
                if index.is_some() {
                    self.live_follow = None;
                }
            }
            Choice::FollowLive(target) => {
                if let Some((kind, _)) = &target
                    && let Some(live) = self.plugged.values_mut().filter_map(|l| l.live_mut()).find(|l| l.kind == *kind)
                {
                    live.reach = crate::live::START_REACH;
                    self.followed = None;
                }
                self.live_follow = target;
                self.live_keys.clear();
            }
            Choice::CopySheet(number) => {
                ctx.copy_text(number.clone());
                self.open_message = Some(trf(self.lang, "Copied: {number}", &[("number", &number)]));
            }
            Choice::SheetsWindow => {
                if let Some((lat, lon)) = point {
                    let place = self.point_text(lat, lon, None, 6);
                    self.sheets_window = Some(crate::sheets::SheetsWindow::new(self.lang, lat, lon, place));
                }
            }
            Choice::OpenLink(template) => {
                if let Some((lat, lon)) = point {
                    let zoom = data::geocode::link_zoom(lat, self.view_width());
                    match data::menulinks::fill(&template, lat, lon, f64::from(zoom)) {
                        Ok(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
                        Err(e) => self.open_message = Some(e),
                    }
                }
            }
            Choice::EditLinks => self.links_window = Some(crate::menulinks::LinksWindow::new(self.settings.links.clone())),
            Choice::ViewContours => {
                // Квадрат по большей стороне вида у точки взгляда, как
                // участок `export_tiles` модуля.
                let width = self.view_width();
                let aspect = self.nav.camera.aspect().max(0.1);
                let side = width.max(width / aspect);
                let max_level = terrain_source(&self.planet).map_or(terrain::MAX_LEVEL, |s| s.max_level);
                let pose = &self.nav.pose;
                let (lat, lon, radius) = (pose.lat, pose.lon, self.nav.camera.ellipsoid.a);
                // Сечение - как у горизонталей на глобусе и в их легенде.
                let height_px = f64::from(self.globe_width_px) / aspect;
                let step = data::contours::view_step(pose.distance, lat, self.nav.camera.fov_y.to_radians(), height_px, radius, data::contours::LEVEL);
                self.view_contours = Some(crate::isolines::ViewJob::new(self.lang, lat, lon, side, step, radius, max_level, now));
                self.open_message = Some(tr(self.lang, "Contours of the view are built: the heights are loaded first.").to_string());
            }
            Choice::Viewshed | Choice::Insolation | Choice::Shadows => {
                let Some((lat, lon)) = point else {
                    return;
                };
                let name = place.as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
                let coords = format!("{lat:.5}, {lon:.5}");
                let title = if name.is_empty() { coords } else { format!("{name}  {coords}") };
                let (slot, kind) = match choice {
                    Choice::Viewshed => (&mut self.viewshed, crate::pointcalc::Kind::Viewshed),
                    Choice::Shadows => (&mut self.shadows, crate::pointcalc::Kind::Shadows),
                    _ => (&mut self.insolation, crate::pointcalc::Kind::Insolation),
                };
                match slot {
                    Some(c) => c.set_point(lat, lon, title),
                    None => {
                        let unix = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
                        *slot = Some(crate::pointcalc::Calc::new(kind, lat, lon, title, unix));
                    }
                }
                self.join_site(&choice, lat, lon);
            }
            Choice::Changes => {
                let Some((lat, lon)) = point else {
                    return;
                };
                let name = place.as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
                let coords = format!("{lat:.5}, {lon:.5}");
                let title = if name.is_empty() { coords } else { format!("{name}  {coords}") };
                match &mut self.aef_change {
                    Some(c) => c.set_point(lat, lon, title),
                    None => self.aef_change = Some(crate::aefcalc::ChangeCalc::new(lat, lon, title)),
                }
                self.join_site(&choice, lat, lon);
            }
            Choice::Classes => {
                let Some((lat, lon)) = point else {
                    return;
                };
                let name = place.as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
                let coords = format!("{lat:.5}, {lon:.5}");
                let title = if name.is_empty() { coords } else { format!("{name}  {coords}") };
                match &mut self.aef_class {
                    Some(c) => c.set_point(lat, lon, title),
                    None => self.aef_class = Some(crate::aefclass::ClassCalc::new(lat, lon, title, self.open_rasters(), self.lang)),
                }
                self.join_site(&choice, lat, lon);
            }
            Choice::Similar => {
                let Some((lat, lon)) = point else {
                    return;
                };
                let name = place.as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
                let coords = format!("{lat:.5}, {lon:.5}");
                let title = if name.is_empty() { coords } else { format!("{name}  {coords}") };
                match &mut self.similar {
                    Some(c) => c.set_point(lat, lon, title),
                    None => self.similar = Some(crate::aefclass::ClassCalc::similar(lat, lon, title, self.open_rasters(), self.lang)),
                }
                self.join_site(&choice, lat, lon);
            }
            Choice::Trace => {
                let Some((lat, lon)) = point else {
                    return;
                };
                let name = place.as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
                let coords = format!("{lat:.5}, {lon:.5}");
                let title = if name.is_empty() { coords } else { format!("{name}  {coords}") };
                match &mut self.tracing {
                    Some(c) => c.set_point(lat, lon, title),
                    None => self.tracing = Some(crate::aefclass::ClassCalc::tracing(lat, lon, title, self.open_rasters(), self.lang)),
                }
                self.join_site(&choice, lat, lon);
            }
            Choice::Recognize => {
                let Some((lat, lon)) = point else {
                    return;
                };
                let name = place.as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
                let coords = format!("{lat:.5}, {lon:.5}");
                let title = if name.is_empty() { coords } else { format!("{name}  {coords}") };
                match &mut self.recognize {
                    Some(c) => c.set_point(lat, lon, title),
                    None => self.recognize = Some(crate::aefclass::ClassCalc::recognition(lat, lon, title, self.open_rasters(), self.lang)),
                }
                self.join_site(&choice, lat, lon);
            }
            Choice::Objects => {
                let Some((lat, lon)) = point else {
                    return;
                };
                let name = place.as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
                let coords = format!("{lat:.5}, {lon:.5}");
                let title = if name.is_empty() { coords } else { format!("{name}  {coords}") };
                match &mut self.object_tasks {
                    Some(c) => c.set_point(lat, lon, title),
                    None => self.object_tasks = Some(crate::aefclass::ClassCalc::objects(lat, lon, title, self.open_rasters(), self.lang)),
                }
                self.join_site(&choice, lat, lon);
            }
            Choice::Emit => {
                let Some((lat, lon)) = point else {
                    return;
                };
                let name = place.as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
                let coords = format!("{lat:.5}, {lon:.5}");
                let title = if name.is_empty() { coords } else { format!("{name}  {coords}") };
                match &mut self.emit {
                    Some(c) => c.set_point(lat, lon, title),
                    None => self.emit = Some(crate::emit::Emit::new(lat, lon, title)),
                }
                self.join_site(&choice, lat, lon);
            }
            Choice::Spectral => {
                let Some((lat, lon)) = point else {
                    return;
                };
                let name = place.as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
                let coords = format!("{lat:.5}, {lon:.5}");
                let title = if name.is_empty() { coords } else { format!("{name}  {coords}") };
                match &mut self.spectral {
                    Some(c) => c.set_point(lat, lon, title),
                    None => self.spectral = Some(crate::spectral::Spectral::new(lat, lon, title, self.open_rasters())),
                }
                self.join_site(&choice, lat, lon);
            }
            Choice::SlopeHere => {
                let Some((lat, lon)) = point else {
                    return;
                };
                let name = place.as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
                let coords = format!("{lat:.5}, {lon:.5}");
                let title = if name.is_empty() { coords } else { format!("{name}  {coords}") };
                match &mut self.relief_calc {
                    Some(c) => c.set_point(lat, lon, title),
                    None => self.relief_calc = Some(crate::reliefcalc::ReliefCalc::new(lat, lon, title)),
                }
                self.join_site(&choice, lat, lon);
            }
            Choice::Flood | Choice::Flow | Choice::SeaLevel | Choice::Earthwork => self.hydro_choice(choice, point, place),
            Choice::SatPath(number) => {
                if let Some(s) = self.plugged.values_mut().find_map(|l| l.satellites_mut()) {
                    s.select(number);
                }
            }
            Choice::SatFollow(number) => {
                if let Some(s) = self.plugged.values_mut().find_map(|l| l.satellites_mut()) {
                    if let Some(n) = number {
                        s.select(Some(n));
                        s.follow = true;
                        self.live_follow = None;
                        self.followed = None;
                    } else {
                        s.follow = false;
                    }
                }
            }
            Choice::Lidar => {
                if let Some((lat, lon)) = point {
                    self.lidar = Some(crate::lidarnet::Lidar::search(lat, lon, &*self.loaders.request));
                    self.open_message = Some(tr(self.lang, "Searching the lidar catalogs IGN LiDAR HD and USGS 3DEP near the point…").to_string());
                }
            }
            Choice::Sentinel => {
                if let Some((lat, lon)) = point {
                    match &mut self.sentinel {
                        Some(s) => s.show_point(lat, lon),
                        None => {
                            let unix = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
                            self.sentinel = Some(crate::sentinel::Sentinel::new(lat, lon, self.view_width(), unix));
                        }
                    }
                }
            }
            Choice::Rumsey => {
                if let Some((lat, lon)) = point {
                    let view = self.view_box();
                    match &mut self.rumsey {
                        Some(r) => r.show_view(lat, lon, view),
                        None => {
                            let loaders = &self.loaders;
                            self.rumsey = Some(crate::rumsey::Rumsey::new(lat, lon, view, &|limit: u64| (loaders.request)(limit)));
                        }
                    }
                }
            }
            Choice::Wayback => self.open_wayback(),
            Choice::Oam => {
                let view = self.view_box();
                let loaders = &self.loaders;
                let request = |limit: u64| (loaders.request)(limit);
                match &mut self.oam {
                    Some(o) => o.show_view(view, &request),
                    None => self.oam = Some(crate::oam::Oam::new(view, &request)),
                }
            }
            Choice::Etomesto => {
                if let Some((lat, lon)) = point {
                    let loaders = &self.loaders;
                    let request = |limit: u64| (loaders.request)(limit);
                    self.etomesto = Some(crate::etomesto::EtomestoWindow::new(lat, lon, &request));
                }
            }
            Choice::RouteFrom | Choice::RouteTo => {
                // Точечная метка под курсором даёт свою точку и название,
                // как `route_items` модуля.
                let picked = place.and_then(|(path, _)| self.marks.place(&path).filter(|p| p.kind == scene::features::Kind::Point).and_then(|p| p.points.first().map(|q| (q.0, q.1, p.name.clone()))));
                let Some(end) = picked.or_else(|| point.map(|(lat, lon)| (lat, lon, String::new()))) else {
                    return;
                };
                if choice == Choice::RouteFrom {
                    self.route.set_origin(self.lang, end, now);
                } else {
                    self.route.set_target(self.lang, end, now);
                }
            }
        }
    }

    /// Пол высот Земли: 0 - высоты ниже нуля обнуляются, минус
    /// бесконечность - дно моря остаётся, `_earth_floor` модуля.
    fn earth_floor(&self) -> f32 {
        // У других тел моря нет, впадины остаются.
        if self.settings.sea || !self.planet.earth { f32::NEG_INFINITY } else { EARTH_FLOOR }
    }

    /// Вид неба, `show_sky` модуля: поза глобуса запоминается, взгляд -
    /// прежний или начальный.
    fn enter_sky(&mut self) {
        if self.sky.is_some() {
            return;
        }
        self.nav.stop();
        self.close_side();
        self.tool = None;
        let view = self.sky_saved.unwrap_or_default();
        self.sky = Some(crate::sky::Sky::new(view, self.nav.pose.clone()));
    }

    /// Выход из неба, `_leave_sky` модуля: поза глобуса возвращается.
    fn leave_sky(&mut self) {
        if let Some(sky) = self.sky.take() {
            self.sky_saved = Some(sky.view);
            self.nav.stop();
            self.nav.set_pose(sky.globe_pose);
            if let Some(globe) = &mut self.globe {
                globe.set_sky_lines(&[]);
                globe.set_sky_bodies(&[]);
            }
        }
    }

    /// Кадр неба: ввод, данные, отрисовка, подписи, панель значков.
    fn sky_frame(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, rect: egui::Rect, response: &egui::Response, ppp: f64, now: f64) {
        let (width, height) = (self.nav.camera.width, self.nav.camera.height);
        let moment = sun_moment(self.timebar.window());
        let loaders = &self.loaders;
        let Some(sky) = &mut self.sky else {
            return;
        };
        let moved = sky.input(ctx, response, ppp, f64::from(height));
        let lines = sky.step_data(&|limit| (loaders.request)(limit), self.settings.constellations);
        let bodies = sky.bodies_at.is_none_or(|t| (moment - t).abs() > crate::sky::BODIES_PERIOD).then(|| {
            sky.bodies_at = Some(moment);
            crate::sky::body_points(moment)
        });
        let busy = sky.busy() || matches!(self.stars, Some(Err(_)) | None) || matches!(self.sky_image, Some(Err(_)) | None) || self.surveys.busy();
        let view = sky.view;
        // Камера неба: глаз не важен, звёзды и линии - направления, оси -
        // взгляда на небо.
        let mut camera = self.nav.camera.clone();
        let [r, u, b] = view.rotation();
        camera.rotation = globe_core::DMat3::from_cols(globe_core::DVec3::from_array(r), globe_core::DVec3::from_array(u), globe_core::DVec3::from_array(b));
        camera.fov_y = view.fov;
        if let Some(globe) = &mut self.globe {
            if let Some(lines) = lines {
                globe.set_sky_lines(&lines);
            }
            // Снимки неба: карта неба гасит Млечный путь, DSS - звёзды
            // каталога, как у модуля.
            let aspect = f64::from(width) / f64::from(height.max(1));
            self.surveys.step(globe, &view, (aspect, f64::from(height)), self.settings.sky_imagery, now, &|limit| (loaders.request)(limit));
            let identity = globe_core::DMat3::IDENTITY;
            globe.set_sky_map_frame(Some((identity, (1.0 - self.surveys.share(0)) as f32)));
            globe.set_star_frame(Some((identity, (1.0 - self.surveys.share(1)) as f32)));
            if let Some(points) = bodies {
                globe.set_sky_bodies(&points);
            }
            let recreated = globe.resize(width, height);
            globe.render(&camera, &globe_core::lod::Selection::default(), None, FrameOptions { count_holes: false, show_holes: false, air: false, ratio: ppp as f32, sun: None, water: None, tint: None, sky: true, sea_level: 0.0, focus: None, relief: None });
            self.show_target(recreated || self.texture.is_none());
        }
        if let Some(id) = self.texture {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(id, rect, uv, egui::Color32::WHITE);
        }
        let painter = ui.painter().with_clip_rect(rect);
        let shown = crate::sky::Shown { constellations: self.settings.constellations, deep: self.settings.deepsky };
        if let Some(sky) = &mut self.sky {
            sky.labels(&painter, rect, ppp, self.lang, moment, shown);
            sky.card(ctx, self.lang);
            if let Some(name) = &sky.not_found {
                painter.text(rect.center_bottom() + egui::vec2(0.0, -40.0), egui::Align2::CENTER_BOTTOM, trf(self.lang, "Nothing found in the sky: {name}", &[("name", name)]), egui::FontId::proportional(14.0), egui::Color32::from_rgb(230, 160, 120));
            }
            // Направление под курсором - строкой у нижнего края, как у модуля.
            if let Some(pos) = response.hover_pos() {
                let d = sky.view.direction_at(f64::from(pos.x - rect.min.x) * ppp, f64::from(pos.y - rect.min.y) * ppp, f64::from(width), f64::from(height));
                painter.text(rect.left_bottom() + egui::vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM, data::skyview::ra_dec_text(d), egui::FontId::proportional(13.0), egui::Color32::from_gray(200));
            }
        }
        let mut parts = vec![data::stars::ATTRIBUTION, data::stars::SKY_ATTRIBUTION, data::skydata::ATTRIBUTION];
        if self.settings.deepsky {
            parts.push(data::deepsky::ATTRIBUTION);
        }
        if self.surveys.share(0) > 0.0 {
            parts.push((data::hips::STARMAP_CREDIT, data::hips::STARMAP_LINK));
        }
        if self.surveys.share(1) > 0.0 {
            parts.push((data::hips::DSS_CREDIT, data::hips::DSS_TERMS));
        }
        self.credits.show(ctx, rect, self.lang, &parts, 2);
        // Снимок вида неба - тем же путём, что у глобуса. Раньше кадр
        // неба до него не доходил: «Снимок вида», «Копия вида» и снимок
        // агента MCP не снимались, 28 октября 2026 года.
        let hiding = hides_windows(self.snapshot.as_ref());
        self.request_snapshot(ctx, rect);
        if self.shows_ui() && !hiding {
            self.toolbar(ctx, rect, now);
            self.settings_window(ctx);
            self.dialogs(ctx, now);
        }
        self.step_shot(ctx, busy);
        if moved || busy || response.dragged() {
            ctx.request_repaint();
        }
    }

    /// Сменить тело глобуса, `_set_body` модуля: размеры тела, снимки,
    /// высоты, воздух и земные слои сразу. Камера встаёт над домашней
    /// точкой тела. Неизвестный ключ - Земля.
    fn set_body(&mut self, key: &str) {
        use crate::layers::Extra;
        let planet = scene::planets::planet_by_key(key);
        if planet.key() == self.planet.key() {
            return;
        }
        self.area_site = None;
        // Земные слои и векторная основа приостанавливаются, как
        // `_apply_extra` модуля на другом теле.
        if self.planet.earth {
            let extras: Vec<Extra> = self.extras_on.iter().copied().filter(|e| e.earth_only()).collect();
            for extra in &extras {
                self.set_extra(*extra, false);
            }
            let rows = self.vector.rows.clone();
            self.set_vector_rows(std::collections::BTreeSet::new());
            self.earth_away = Some(EarthAway { extras, rows, basemap: self.basemap });
        }
        // Разрез вниз и профиль считались по прежнему телу.
        self.section = None;
        self.profile = None;
        self.planet = planet;
        self.nav.camera.ellipsoid = Ellipsoid::new(self.planet.body);
        // Высоты прежнего тела не годятся.
        let source = terrain_source(&self.planet);
        self.loaders.terrain = (self.loaders.make)(source.unwrap_or(net::TERRARIUM));
        {
            let mut store = self.heights.write();
            store.clear();
            store.max_level = source.map_or(0, |s| s.max_level).min(terrain::MAX_LEVEL);
        }
        if let Some(globe) = &mut self.globe {
            globe.change_body();
            globe.set_subsurface("section", None);
        }
        self.pending.clear();
        if self.planet.earth {
            let away = self.earth_away.take();
            self.set_basemap(away.as_ref().map_or(Basemap::default(), |a| a.basemap));
            if let Some(away) = away {
                self.set_vector_rows(away.rows);
                for extra in away.extras {
                    self.set_extra(extra, true);
                }
            }
        } else {
            self.set_basemap(Basemap::Body(self.planet.key()));
        }
        let (lat, lon, distance) = self.planet.home;
        self.nav.stop();
        self.nav.set_pose(Pose::new(lat, lon, distance, 0.0, 0.0));
    }

    /// Окно «Объекты» для точки: метки под пикселем, если он дан, и место
    /// с адресом.
    fn make_objects(&self, lat: f64, lon: f64, pixel: Option<(f64, f64, f64)>, address: Option<String>) -> crate::identify::Objects {
        let height = (self.true_heights())(&[(lat, lon)]).first().copied();
        let mut groups = Vec::new();
        let mut hits = Vec::new();
        let paths = match (pixel, self.marks.tree.as_ref()) {
            (Some((px, py, ppp)), Some(tree)) => crate::card::pick_all(tree, &self.nav, px, py, crate::card::PICK_PIXELS * ppp),
            _ => Vec::new(),
        };
        let places: Vec<&scene::kml::Place> = paths.iter().filter_map(|p| self.marks.place(p)).collect();
        groups.extend(crate::identify::places_group(self.lang, &places));
        // Объекты слоёв «Моих данных», как группы слоёв проекта модуля.
        // Допуск - PICK_PIXELS пикселей кадра, у линий - в метрах у точки.
        if let Some((px, py, ppp)) = pixel {
            let radius = crate::card::PICK_PIXELS * ppp;
            let camera = &self.nav.camera;
            let ground = |la: f64, lo: f64| self.nav.pose.terrain.as_ref().map_or(0.0, |t| t(la, lo));
            let p = camera.ellipsoid.geodetic_to_ecef(lat, lon, ground(lat, lon));
            let scale = 2.0 * (camera.fov_y.to_radians() / 2.0).tan() / f64::from(camera.height.max(1));
            let metres = radius * (camera.eye - p).length() * scale;
            let gap = |la: f64, lo: f64| {
                let (q, front) = camera.project(camera.ellipsoid.geodetic_to_ecef(la, lo, ground(la, lo)));
                let gap = (q.x - px).hypot(q.y - py);
                (front && gap <= radius).then_some(gap / radius.max(1e-9))
            };
            groups.extend(self.data_layers.identify(self.lang, lat, lon, metres, &gap));
            hits = self.data_layers.hits_at(lat, lon, metres, &gap);
            // Объект слоя RustGIS выделяется и на карте RustGIS, как
            // определение объектов слоёв проекта QGIS модулем.
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(sync) = self.rustgis.as_ref().filter(|s| s.connected)
                && let Some((layer, row)) = self.data_layers.live_hit(lat, lon, metres, &gap)
            {
                sync.link.ask(crate::rustgislink::Ask::Select { layer, rows: vec![row] });
            }
        }
        if let Some((px, py, ppp)) = pixel {
            groups.extend(self.dots.identify(self.lang, &self.nav, (lat, lon), (px, py), crate::card::PICK_PIXELS * ppp * 2.0, self.timebar.window()));
        }
        groups.extend(self.geo.identify(&self.dots, self.lang, lat, lon, self.magnetic_year().0));
        // Живые объекты у пикселя, `LiveManager.identify` модуля.
        if let Some((px, py, ppp)) = pixel {
            let store = self.heights.read();
            let e = self.nav.camera.ellipsoid;
            for live in self.plugged.values().filter_map(|l| l.live()) {
                let kind = &live.kind;
                let mut near: Vec<(f64, &crate::live::Object)> = self
                    .live_objects
                    .iter()
                    .filter(|(k, _)| k == kind)
                    .filter_map(|(_, o)| {
                        let h = if o.alt.is_finite() { o.alt * store.scale() } else { store.height_at(o.lat, o.lon) };
                        let (q, front) = self.nav.camera.project(e.geodetic_to_ecef(o.lat, o.lon, h));
                        let gap = (q.x - px).hypot(q.y - py);
                        let radius = crate::live::PICK_PIXELS * ppp * if crate::live::near(*kind, self.view_width()) { 3.0 } else { 1.0 };
                        (front && gap <= radius).then_some((gap, o))
                    })
                    .collect();
                near.sort_by(|a, b| a.0.total_cmp(&b.0));
                let features: Vec<crate::identify::Feature> = near.into_iter().take(5).map(|(_, o)| (o.label.clone(), live.values(self.lang, &o.key))).collect();
                if !features.is_empty() {
                    groups.push((tr(self.lang, kind.name()).to_string(), features));
                }
            }
        }
        // Слои реестра под точкой: контур горной разработки,
        // `_identify_mining` модуля, и другие.
        groups.extend(self.plugged.values().filter_map(|l| l.identify(self.lang, lat, lon)));
        // Спутник у пикселя, `SatelliteManager.identify` модуля.
        if let (Some((px, py, ppp)), Some(sats)) = (pixel, self.plugged.values().find_map(|l| l.satellites()))
            && let Some(number) = self.satellite_under(px, py, ppp)
            && let Some(index) = sats.swarm.index_of(number)
            && let Some(feature) = sats.values(self.lang, index, self.satellite_moment())
        {
            groups.push((tr(self.lang, "Satellites").to_string(), vec![feature]));
        }
        // Объекты слоёв реестра у пикселя: статьи Википедии,
        // `_identify_wiki` модуля, и другие.
        if let Some(pixel) = pixel {
            groups.extend(self.plugged.values().filter_map(|l| l.pick(self.lang, &self.nav.camera, pixel)));
        }
        groups.push(crate::identify::place_group(self.lang, height.filter(|_| self.settings.relief), address));
        if let Some((ask, answer)) = &self.map_ask
            && ask.at == (lat, lon)
        {
            groups.extend(crate::mapvalue::group(self.lang, ask.key, answer, ask.at));
        }
        groups.extend(self.field_tiles.as_ref().and_then(|f| f.group(self.lang, lat, lon)));
        crate::identify::Objects { point: self.point_text(lat, lon, height, 6), groups, at: (lat, lon), hits, select: None }
    }

    /// Год с дробью для геофизики: правый бегунок открытой шкалы времени,
    /// без неё - часы компьютера, и год левого бегунка - начало следа
    /// полюсов, как `magnetic_year` и `_magnetic_start` модуля.
    fn magnetic_year(&self) -> (f64, Option<f64>) {
        let now = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
        match self.timebar.window() {
            Some((lo, hi)) if hi.is_finite() => (data::geomag::year_of(hi), (lo.is_finite() && lo < hi).then(|| data::geomag::year_of(lo))),
            _ => (data::geomag::year_of(now), None),
        }
    }

    /// Ширина вида в метрах у точки взгляда.
    fn view_width(&self) -> f64 {
        2.0 * self.nav.pose.distance * (self.nav.camera.fov_y.to_radians() / 2.0).tan() * self.nav.camera.aspect()
    }

    /// Рамка вида (запад, юг, восток, север) по ширине и высоте вида у точки
    /// взгляда, `oam_box` модуля.
    fn view_box(&self) -> data::rumsey::Bbox {
        let width = self.view_width();
        let height = width / self.nav.camera.aspect().max(0.1);
        let (pl, plon) = (self.nav.pose.lat, self.nav.pose.lon);
        let dlat = (height / 2.0 / 111_320.0).min(85.0);
        let dlon = (width / 2.0 / (111_320.0 * pl.to_radians().cos().max(0.05))).min(180.0);
        [plon - dlon, (pl - dlat).max(-90.0), plon + dlon, (pl + dlat).min(90.0)]
    }

    /// Значение карты значений или готовой карты под точкой, как
    /// `_ask_tilemap` модуля. Ответ ложится в окно «Объекты».
    fn ask_map_value(&mut self, lat: f64, lon: f64) {
        let key = self.vector.map_keys().into_iter().find(|k| crate::maps::kind(k).and_then(|m| m.slot()) == Some(1));
        // Уровень карты запроса Macrostrat - по ширине вида, `link_zoom` модуля.
        let pose = &self.nav.pose;
        let width = 2.0 * pose.distance * (self.nav.camera.fov_y.to_radians() / 2.0).tan() * self.nav.camera.aspect();
        let zoom = data::geocode::link_zoom(lat, width);
        self.map_ask = key.and_then(|k| crate::mapvalue::Ask::start(k, lat, lon, zoom, (self.loaders.request)(ANSWER_BYTES))).map(|a| (a, crate::mapvalue::Answer::Loading));
    }

    /// Уровень воды по умолчанию: высота рельефа у края растра name -
    /// середина рамки каждой стороны и углы чуть снаружи, медиана.
    fn shore_level(&self, name: &str) -> f64 {
        let Some((w, s, e, n)) = self.relief.bounds(name) else {
            return 0.0;
        };
        let (dx, dy) = ((e - w) * 0.02, (n - s) * 0.02);
        let xs = [w - dx, (w + e) / 2.0, e + dx];
        let ys = [s - dy, (s + n) / 2.0, n + dy];
        let points: Vec<(f64, f64)> = xs.iter().flat_map(|x| ys.iter().map(move |y| (*x, *y))).filter(|(x, y)| (*x, *y) != ((w + e) / 2.0, (s + n) / 2.0)).map(|(x, y)| data::bathymetry::mercator_latlon(x, y)).collect();
        let mut heights: Vec<f64> = self.heights.read().heights_at(&points, false).into_iter().filter(|h| h.is_finite()).collect();
        if heights.is_empty() {
            return 0.0;
        }
        heights.sort_by(f64::total_cmp);
        (heights[heights.len() / 2] * 10.0).round() / 10.0
    }

    /// Выбор строк после перестройки дерева: пути устарели. Текущая строка
    /// теперь «Временные метки», новое ложится туда.
    fn reset_selection(&mut self) {
        self.tree_state.chosen.clear();
        self.tree_state.selected = self.marks.temporary.then(|| marks::TEMPORARY.to_vec());
    }

    /// Окно вопроса перед удалением и окно свойств папки.
    fn dialogs(&mut self, ctx: &egui::Context, now: f64) {
        let lang = self.lang;
        // Окно «Снимки событий».
        let mut event_actions = Vec::new();
        self.events.window(ctx, lang, &mut event_actions);
        self.event_actions(event_actions, now);
        if let Some(menu) = &self.globe_menu {
            let (choice, open) = menu.show(ctx, lang);
            let point = menu.point;
            let place = menu.place.clone();
            if !open {
                self.globe_menu = None;
            }
            if let Some(c) = choice {
                self.globe_menu_choice(ctx, c, point, place, now);
            }
        }
        // Пришёл адрес «Сведения о точке» - строка «Адрес» в группе «Место».
        if let Some(objects) = &self.objects
            && let Some((at, answer)) = self.search.address.clone()
            && at == objects.at
        {
            let (lat, lon) = at;
            let text = match answer {
                Ok(a) if !a.is_empty() => a,
                Ok(_) => tr(lang, "none").to_string(),
                Err(e) => trf(lang, "not received: {error}", &[("error", &e)]),
            };
            self.search.address = None;
            self.objects = Some(self.make_objects(lat, lon, None, Some(text)));
        }
        // Пришло значение карты - строка группы карты.
        if let Some((ask, answer)) = &mut self.map_ask
            && let Some(got) = ask.take(lang)
        {
            *answer = got;
            if let Some(objects) = &mut self.objects
                && objects.at == ask.at
                && let Some(group) = crate::mapvalue::group(lang, ask.key, answer, ask.at)
            {
                match objects.groups.iter_mut().find(|(name, _)| *name == group.0) {
                    Some(slot) => *slot = group,
                    None => objects.groups.push(group),
                }
            }
        }
        // Окно «Батиметрия водоёма».
        if let Some(dialog) = &mut self.bathy
            && let Some(answer) = self.side.bathy.take()
        {
            let name = dialog.name.clone();
            self.bathy = None;
            let mut store = self.heights.write();
            let result = match answer {
                crate::bathymetry::Answer::Set(kind, level) => self.relief.set_lake(&name, Some((kind, level)), &mut store),
                crate::bathymetry::Answer::Remove => {
                    self.relief.remove(&name, &mut store);
                    Ok(())
                }
                crate::bathymetry::Answer::Cancel => Ok(()),
            };
            drop(store);
            if let Err(e) = result {
                self.open_message = Some(trf(lang, "The raster did not open: {name}: {e}", &[("name", &name), ("e", &e)]));
            }
            self.water = None;
        }
        if let Some(objects) = &mut self.objects {
            let open = objects.window(ctx, lang);
            // «Выделить на глобусе» и «Снять выделение».
            match objects.select.take() {
                Some(true) => self.data_layers.select(&objects.hits),
                Some(false) => self.data_layers.select(&[]),
                None => {}
            }
            if !open {
                self.objects = None;
            }
        }
        if let Some((title, text, _)) = &self.confirm {
            let mut answer = None;
            egui::Window::new(title.as_str()).collapsible(false).resizable(false).pivot(egui::Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).show(ctx, |ui| {
                ui.label(text.as_str());
                ui.horizontal(|ui| {
                    if ui.button(tr(lang, "Yes")).clicked() {
                        answer = Some(true);
                    }
                    if ui.button(tr(lang, "No")).clicked() {
                        answer = Some(false);
                    }
                });
            });
            if let Some(yes) = answer
                && let Some((_, _, what)) = self.confirm.take()
                && yes
            {
                match what {
                    Confirmed::Remove(path) => {
                        if self.marks.remove(&path) {
                            self.reset_selection();
                            self.player = None;
                            self.marks_changed();
                        }
                    }
                    Confirmed::RemoveMany(paths) => {
                        if self.marks.remove_many(&paths) > 0 {
                            self.reset_selection();
                            self.player = None;
                            self.marks_changed();
                        }
                    }
                    Confirmed::Clear => self.clear_marks(),
                    Confirmed::ClearTemporary => {
                        self.marks.clear_temporary();
                        self.reset_selection();
                        self.player = None;
                        self.marks_changed();
                    }
                }
            }
        }
        let p = &self.nav.pose;
        let current = [p.lat, p.lon, p.distance, p.heading, p.tilt];
        if let Some(props) = &mut self.folder_props {
            let _ = current;
            match self.side.folder.take().unwrap_or(crate::folderprops::Outcome::Open) {
                crate::folderprops::Outcome::Open => {}
                crate::folderprops::Outcome::Cancel => self.folder_props = None,
                crate::folderprops::Outcome::Ok => {
                    let region = props.region(&self.marks.folder_points(&props.path));
                    let view = if props.has_view { scene::lookat::make(props.view[0], props.view[1], props.view[2], props.view[3], props.view[4]) } else { None };
                    let (name, description, expandable, radio) = (props.name.trim().to_string(), props.description.trim().to_string(), props.expandable, props.radio && props.expandable);
                    let path = props.path.clone();
                    self.folder_props = None;
                    if self.marks.edit_folder(&path, |f| {
                        if !name.is_empty() {
                            f.name = name;
                        }
                        f.description = description;
                        f.expandable = expandable;
                        f.radio = radio;
                        f.view = view;
                        f.region = region;
                    }) {
                        self.marks_changed();
                    }
                }
            }
        }
    }

    /// Окна папки «Моих меток».
    fn folder_windows(&mut self, ctx: &egui::Context, lang: Lang) {
        if let Some(window) = &mut self.extrude
            && let Some(answer) = window.show(ctx, lang)
        {
            let path = window.path.clone();
            self.extrude = None;
            if let Some((field, factor)) = answer {
                let radius = self.nav.camera.ellipsoid.a;
                let mut count = 0;
                self.marks.edit_folder(&path, |f| count = crate::foldercalc::extrude_folder(f, &field, factor, radius));
                self.marks_changed();
                self.open_message = Some(trf(lang, "Objects extruded: {count}.", &[("count", &count.to_string())]));
            }
        }
        if let Some(window) = &mut self.sounding {
            let open = window.show(ctx, lang);
            let done = window.done.take();
            if !open {
                self.sounding = None;
            }
            if let Some((tiff, name, level)) = done
                && self.open_relief(&tiff, &name, None).is_ok()
            {
                let mut store = self.heights.write();
                let _ = self.relief.set_lake(&name, Some((data::bathymetry::Kind::Depth, level)), &mut store);
                drop(store);
                self.water = None;
            }
        }
    }

    /// Настоящие высоты точек, без масштаба рельефа, для линейки
    /// и профиля.
    fn true_heights(&self) -> impl Fn(&[(f64, f64)]) -> Vec<f64> + use<> {
        let source = self.heights.clone();
        move |points: &[(f64, f64)]| -> Vec<f64> {
            // Без масштаба и с дном моря, как высоты профиля модуля
            // с глубинами морей.
            source.read().floor_heights_at(points)
        }
    }

    /// Окно профиля высот. Профиль линейки идёт за её точками.
    fn profile_window(&mut self, ctx: &egui::Context) {
        let heights_at = self.true_heights();
        let ready = !self.loaders.terrain.busy();
        let lakes = self.relief.lakes();
        let ruler_points = self.tool.as_ref().filter(|t| t.purpose == crate::tool::Purpose::Ruler).map(|t| t.ruler.points(true));
        let Some(profile) = &mut self.profile else {
            return;
        };
        if profile.ruler {
            match ruler_points {
                Some(points) => profile.points = points,
                None => {
                    self.profile = None;
                    return;
                }
            }
        }
        let mut open = true;
        profile.window(ctx, self.lang, &heights_at, &lakes, ready, &mut open);
        if !open {
            self.profile = None;
        }
    }

    /// Окно «Разрез»: загрузки, сборка, стенка на глобусе. Закрыли -
    /// стенка уходит.
    fn section_window(&mut self, ctx: &egui::Context) {
        let Some(s) = &mut self.section else {
            return;
        };
        let loaders = &self.loaders;
        let (wall, saved) = s.step(&|limit| (loaders.request)(limit), self.dots.quakes(crate::dots::Layer::Quakes), self.dots.quakes(crate::dots::Layer::Iscgem), self.timebar.window());
        if let (Some(wall), Some(globe)) = (wall, &mut self.globe) {
            globe.set_subsurface("section", wall.as_ref());
        }
        let mut open = true;
        s.window(ctx, self.lang, &mut open);
        if let Some(bytes) = saved {
            self.store_crust(&bytes);
        }
        if !open {
            self.section = None;
            if let Some(globe) = &mut self.globe {
                globe.set_subsurface("section", None);
            }
        }
    }

    /// Окно «Разрез модели»: разрез заново и точка на глобусе, свои цвета -
    /// в настройки.
    fn model_section_window(&mut self, ctx: &egui::Context, now: f64) {
        let Some(w) = &mut self.model_section else {
            return;
        };
        let version = self.heights.read().version();
        let heights = &self.heights;
        let ground = |pts: &[(f64, f64)]| heights.read().heights_at(pts, false);
        let sub = self.subsurface.as_ref();
        w.step(sub.and_then(|s| s.model.as_ref()), sub.map_or(0, |s| s.generation), &ground, version, now);
        let mut open = true;
        w.window(ctx, self.lang, &mut open);
        if std::mem::take(&mut w.colors_changed) {
            self.settings.section_colors = w.colors();
            self.settings_dirty = true;
        }
        if !open {
            self.model_section = None;
        }
    }

    /// Карточка метки. Метки по пути больше нет - карточка закрывается.
    fn card_window(&mut self, ctx: &egui::Context) {
        let Some(card) = &self.card else {
            return;
        };
        let Some(place) = self.marks.place(&card.path) else {
            self.card = None;
            return;
        };
        let mut open = true;
        let answer = self.explaining.as_ref().filter(|(p, _)| *p == card.path).map(|(_, from)| self.ai.answer_since(*from));
        let action = card.window(ctx, self.lang, place, answer.as_deref(), &mut open);
        match action {
            Some(crate::card::Action::Fly(path)) => self.queued.push(crate::marks::Action::Fly(path)),
            Some(crate::card::Action::Properties(path)) => self.queued.push(crate::marks::Action::Properties(path)),
            Some(crate::card::Action::Explain(path)) => {
                let question = crate::card::explain_question(self.lang, place);
                let from = self.ai.lines.len();
                let context = self.ai_context();
                let loaders = &self.loaders;
                let request = |limit: u64| (loaders.request)(limit);
                if self.ai.ask(self.lang, &question, &self.settings.ai, &context, &request) {
                    self.explaining = Some((path, from));
                }
            }
            None => {}
        }
        if !open {
            self.card = None;
        }
    }

    /// Окно инструмента и его итог: новая метка ложится в выделенную
    /// папку «Моих меток» или в корень, правка заменяет метку.
    fn tool_window(&mut self, ctx: &egui::Context) {
        if self.tool.is_none() {
            return;
        }
        let names = self.marks.names();
        // Длина по рельефу - по настоящим высотам, без масштаба рельефа.
        let heights_at = self.true_heights();
        let ready = !self.loaders.terrain.busy();
        let pose = &self.nav.pose;
        let current = [pose.lat, pose.lon, pose.distance, pose.heading, pose.tilt];
        let Some(tool) = &mut self.tool else {
            return;
        };
        tool.current_view = current;
        let edit = match &tool.purpose {
            crate::tool::Purpose::Edit(path) => Some(path.clone()),
            _ => None,
        };
        // Окно свойств метки - в панели свойств, его итог пришёл оттуда.
        let action = if edit.is_some() { self.side.tool.take() } else { tool.window(ctx, self.lang, &names, Some((&heights_at, ready))) };
        // Правка метки видна на глобусе сразу, как предпросмотр модуля:
        // метка заменяется меткой с правками, «Отмена» возвращает прежнюю.
        let mut show = None;
        if let Some(path) = &edit {
            if tool.open {
                let preview = tool.preview();
                if preview.is_some() && preview != self.last_preview {
                    self.last_preview = preview.clone();
                    show = preview.map(|p| (path.clone(), p));
                }
            } else if !matches!(action, Some(crate::tool::Action::Update(..))) {
                show = tool.original().cloned().map(|p| (path.clone(), p));
            }
        }
        if !tool.open {
            self.tool = None;
            self.last_preview = None;
        }
        if let Some((path, place)) = show
            && self.marks.set_place(&path, place)
        {
            self.marks_changed();
        }
        let root = marks::root_name(self.lang);
        match action {
            Some(crate::tool::Action::Measure(place)) => {
                let name = place.name.clone();
                self.name_dialog = Some((NameFor::Measure(Box::new(place)), name));
            }
            Some(crate::tool::Action::Add(place)) => {
                let folder = self.tree_state.selected.clone().unwrap_or_default();
                self.marks.insert(&folder, scene::kml::Item::Place(place), root);
                self.marks_changed();
            }
            Some(crate::tool::Action::Update(path, place)) => {
                if self.marks.set_place(&path, place) {
                    self.marks_changed();
                }
            }
            Some(crate::tool::Action::Profile) => {
                self.profile = Some(crate::profile::ProfileWindow { title: String::new(), points: Vec::new(), hovered: None, ruler: true });
            }
            None => {}
        }
    }

    /// Окно «Настройки» и перемены по нему: язык, подписи, оформление,
    /// масштаб рельефа.
    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let before = self.settings.clone();
        let mut open = self.settings_open;
        let mut basemap = self.basemap;
        let (changed, sources, under) = crate::settings::window(ctx, self.lang, &mut open, &mut self.settings, &mut basemap);
        self.settings_open = open;
        self.credits.sources_open |= sources;
        if under {
            match &mut self.subsurface {
                Some(sub) => sub.window_open = true,
                None => self.open_message = Some(tr(self.lang, "No subsurface data: open a GeoPackage file with drill holes, bed roofs or tunnels.").to_string()),
            }
        }
        self.set_basemap(basemap);
        if !changed {
            return;
        }
        self.settings_dirty = true;
        let s = self.settings.clone();
        if s.dark != before.dark {
            crate::theme::apply(ctx, s.dark);
        }
        if (s.scale - before.scale).abs() > f64::EPSILON {
            self.heights.write().set_scale(if s.relief { s.scale } else { 0.0 });
        }
        if s.sea != before.sea {
            self.sea_changed();
        }
        if s.language != before.language || s.labels != before.labels {
            let old_root = marks::root_name(self.lang).to_string();
            self.lang = s.lang();
            self.labels = Labels::new(self.lang, &s.labels);
            self.labels.kinds = self.vector.label_kinds();
            self.labels.set_plates(&self.dots.plate_names());
            self.search = Search::new(Some(self.lang.code()));
            // Корень «Мои метки» - на новом языке, если его не переименовали.
            let new_root = marks::root_name(self.lang);
            if let Some(root) = self.marks.tree.as_mut().filter(|r| r.name == old_root) {
                root.name = new_root.to_string();
            }
            let temporary = tr(self.lang, "Temporary Places");
            self.marks.edit_folder(&marks::TEMPORARY, |f| f.name = temporary.to_string());
            self.marks_to_labels();
            self.fonts_dirty = true;
        }
    }

    /// Язык, которому нужны иероглифы: интерфейса, иначе подписей.
    fn cjk_code(&self) -> Option<&'static str> {
        if self.lang.needs_cjk() {
            return Some(self.lang.code());
        }
        ["zh", "ja", "ko"].into_iter().find(|c| *c == self.settings.labels)
    }

    /// Шрифты интерфейса и подписей по языку. Программа берёт шрифт
    /// с иероглифами из системы, страница скачивает свой из
    /// planetx-terrain и ставит его по приходу.
    fn install_fonts(&mut self, ctx: &egui::Context) {
        let code = self.cjk_code();
        #[cfg(not(target_arch = "wasm32"))]
        let cjk = code.and_then(crate::fonts::system_cjk);
        #[cfg(target_arch = "wasm32")]
        let cjk = code.and_then(|c| self.web_fonts.get(c).cloned().map(|b| (b, 0)));
        #[cfg(target_arch = "wasm32")]
        if let Some(c) = code.filter(|c| !self.web_fonts.contains_key(*c))
            && self.fetching.is_none()
            && let Some(url) = crate::fonts::web_cjk_url(c)
        {
            self.loaders.files.start(&url);
            self.fetching = Some((c.to_string(), false));
            self.font_fetching = Some(c);
            self.open_message = Some(tr(self.lang, "Loading the font for this language…").to_string());
        }
        crate::fonts::install(ctx, cjk);
    }

    /// Картинки на экране, ScreenOverlay: поверх кадра, под надписями
    /// интерфейса, в порядке drawOrder. Возвращает их места.
    fn screen_overlays(&mut self, ctx: &egui::Context, painter: &egui::Painter, rect: egui::Rect) -> Vec<egui::Rect> {
        let mut places = Vec::new();
        let mut screen: Vec<&scene::kml::Overlay> =
            self.marks.overlays().iter().filter(|o| o.kind() == scene::overlays::Kind::Screen).collect();
        screen.sort_by_key(|o| o.overlay.order);
        for o in screen {
            let key = crate::pictures::key_of(o);
            let Some(image) = self.pictures.image(&key) else {
                continue;
            };
            let texture = self.screen_textures.entry(key.clone()).or_insert_with(|| {
                let size = [image.width as usize, image.height as usize];
                ctx.load_texture(format!("overlay {key}"), egui::ColorImage::from_rgba_unmultiplied(size, &image.rgba), egui::TextureOptions::LINEAR)
            });
            let (x, y, w, h) = scene::overlays::screen_rect(
                &o.overlay,
                f64::from(rect.width()),
                f64::from(rect.height()),
                f64::from(image.width),
                f64::from(image.height),
            );
            let place = egui::Rect::from_min_size(rect.min + egui::vec2(x as f32, y as f32), egui::vec2(w as f32, h as f32));
            let [r, g, b, a] = o.overlay.color;
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(texture.id(), place, uv, egui::Color32::from_rgba_unmultiplied(r, g, b, a));
            places.push(place);
        }
        places
    }

    /// Линия шторки снимков события на виде rect: рисуется поверх глобуса,
    /// тянется мышью. Истина - мышь у линии, глобус её не получает.
    fn swipe_input(&mut self, ctx: &egui::Context, rect: egui::Rect) -> bool {
        // Шторка пары текстур (снимки события или выпуски Wayback)
        // важнее шторки слоёв: ручка у неё одна.
        let pair = pair_swipe(&self.events, self.wayback.as_ref());
        let events = pair.is_some();
        let Some(share) = pair.or(self.layer_swipe) else {
            self.swipe_drag = false;
            return false;
        };
        let x = rect.left() + share * rect.width();
        let grip = egui::Rect::from_x_y_ranges(x - SWIPE_GRIP..=x + SWIPE_GRIP, rect.y_range());
        let (pos, pressed, down) = ctx.input(|i| (i.pointer.interact_pos(), i.pointer.primary_pressed(), i.pointer.primary_down()));
        let near = pos.is_some_and(|p| grip.contains(p));
        if pressed && near {
            self.swipe_drag = true;
        }
        if !down {
            self.swipe_drag = false;
        }
        if self.swipe_drag
            && let Some(p) = pos
        {
            let share = ((p.x - rect.left()) / rect.width().max(1.0)).clamp(0.02, 0.98);
            if self.events.swipe().is_some() {
                self.events.split = share;
            } else if let Some(c) = self.wayback.as_mut().and_then(|w| w.compare.as_mut()).filter(|_| events) {
                c.split = share;
            } else {
                self.layer_swipe = Some(share);
            }
        }
        if near || self.swipe_drag {
            ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        near || self.swipe_drag
    }

    /// Линия шторки и ручка с днями сторон поверх кадра глобуса.
    fn draw_swipe(&self, painter: &egui::Painter, rect: egui::Rect) {
        let Some(share) = pair_swipe(&self.events, self.wayback.as_ref()).or(self.layer_swipe) else {
            return;
        };
        let x = rect.left() + share * rect.width();
        let p = crate::theme::pal();
        painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], egui::Stroke::new(2.0, egui::Color32::WHITE));
        let mid = egui::pos2(x, rect.center().y);
        painter.circle(mid, 11.0, p.plate, egui::Stroke::new(2.0, egui::Color32::WHITE));
        painter.text(mid, egui::Align2::CENTER_CENTER, "⇆", egui::FontId::proportional(14.0), p.plate_text);
        // Дни сторон: снимков события или выпусков Wayback.
        let days = match self.events.shown.as_ref().filter(|_| self.events.swipe().is_some()) {
            Some(s) => Some((s.before.clone().unwrap_or_default(), s.day.clone())),
            None => self.wayback.as_ref().and_then(|w| w.compare.as_ref()).filter(|_| self.events.layer().is_none()).map(|c| (c.left.day.clone(), c.right.day.clone())),
        };
        if let Some((left, right)) = days {
            let font = egui::FontId::proportional(13.0);
            let shadow = egui::Color32::from_black_alpha(160);
            for (text, anchor, at) in [(left, egui::Align2::RIGHT_TOP, egui::pos2(x - 8.0, rect.top() + 8.0)), (right, egui::Align2::LEFT_TOP, egui::pos2(x + 8.0, rect.top() + 8.0))] {
                painter.text(at + egui::vec2(1.0, 1.0), anchor, &text, font.clone(), shadow);
                painter.text(at, anchor, &text, font.clone(), egui::Color32::WHITE);
            }
        }
    }

    /// Картинка на поверхности - слоем-картинкой «Моих слоёв» в папку
    /// folder_name, как слой проекта у модуля: снимки «Космоснимков»
    /// и карты тренда. Время снимка идёт по шкале. bbox - запад, юг,
    /// восток, север.
    #[allow(clippy::too_many_arguments)]
    fn picture_to_layers(&mut self, folder_name: &str, name: &str, png: Vec<u8>, bbox: [f64; 4], time: Option<scene::when::Time>, description: String) {
        let [west, south, east, north] = bbox;
        let mut o = scene::overlays::Overlay::new(scene::overlays::Kind::Ground);
        o.set_box([north, south, east, west, 0.0]);
        let mut overlay = scene::kml::Overlay::new(name, o);
        overlay.time = time;
        overlay.description = description;
        overlay.image = Some(png);
        // RustPlanetX: «Моих слоёв» нет, снимок - в проект RustGIS.
        if self.edition.linked()
            && let Some(text) = self.layers_to_rustgis(folder_name, Vec::new(), vec![overlay.clone()])
        {
            self.open_message = Some(text);
            return;
        }
        self.data_layers.add_picture(overlay, folder_name);
    }

    /// Новый инструмент: открытая правка метки закрывается «Отменой».
    fn set_tool(&mut self, tool: crate::tool::Tool) {
        self.close_side();
        self.tool = Some(tool);
    }

    /// Закрыть окно панели свойств «Отменой»: в панели одно окно, новое
    /// закрывает прежнее, как `SidePanel.host` модуля.
    fn close_side(&mut self) {
        if let Some(tool) = self.tool.take() {
            if let (crate::tool::Purpose::Edit(path), Some(original)) = (&tool.purpose, tool.original()) {
                if self.marks.set_place(path, original.clone()) {
                    self.marks_changed();
                }
            } else {
                self.tool = Some(tool);
            }
        }
        self.last_preview = None;
        self.folder_props = None;
        self.bathy = None;
        if let Some(props) = self.overlay_props.take() {
            self.marks.set_overlay(&props.path, props.original);
            self.marks_changed();
        }
    }

    /// Панель свойств справа от глобуса, как `SidePanel` модуля:
    /// заголовок, ✕ - «Отмена», тело окна с прокруткой во всю высоту.
    /// Ширину задаёт разделитель.
    fn side_panel(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang;
        let editing = self.tool.as_ref().is_some_and(|t| matches!(t.purpose, crate::tool::Purpose::Edit(_)));
        let title = if editing {
            self.tool.as_ref().map(|t| t.edit_title(lang)).unwrap_or_default()
        } else if let Some(p) = &self.overlay_props {
            p.title(lang).to_string()
        } else if self.folder_props.is_some() {
            tr(lang, "Folder properties").to_string()
        } else if self.bathy.is_some() {
            tr(lang, "Water body bathymetry").to_string()
        } else {
            return;
        };
        let names = self.marks.names();
        let p = &self.nav.pose;
        let current = [p.lat, p.lon, p.distance, p.heading, p.tilt];
        let mut close = false;
        egui::Panel::right("planetx-properties").resizable(true).default_size(360.0).min_size(260.0).show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("✕").on_hover_text(tr(lang, "Close the properties without saving the edits, like the Cancel button.")).clicked() {
                        close = true;
                    }
                    ui.add(egui::Label::new(egui::RichText::new(&title).strong()).wrap());
                });
            });
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                if editing {
                    if let Some(tool) = &mut self.tool {
                        self.side.tool = tool.edit_body(ui, lang, &names);
                    }
                } else if let Some(props) = &mut self.overlay_props {
                    self.side.overlay = Some(props.body(ui, lang));
                } else if let Some(props) = &mut self.folder_props {
                    self.side.folder = Some(props.body(ui, lang, current));
                } else if let Some(dialog) = &mut self.bathy {
                    self.side.bathy = dialog.body(ui, lang);
                }
            });
        });
        if close {
            self.close_side();
        }
    }

    /// Загрузить в видеокарту часть ожидающих снимков, как `_upload`
    /// в `render/view.py` модуля. Первыми идут уровни 0-2, за ними тайлы
    /// прошлого кадра, потом мелкие уровни. Стартовые уровни грузятся
    /// все сразу. Снимок окна ждёт всё и грузит всё.
    fn upload_pending(&mut self) {
        let Some(globe) = &mut self.globe else {
            return;
        };
        if self.pending.is_empty() {
            return;
        }
        let start_missing = start_keys().into_iter().any(|k| !globe.has_texture(k));
        let all = start_missing || self.shot.is_some();
        let (budget, time_limit) = if all {
            (self.pending.len(), f64::INFINITY)
        } else if self.was_moving {
            (1, 0.0)
        } else {
            (UPLOADS_PER_FRAME, UPLOAD_TIME)
        };
        let mut order: Vec<TileKey> = self.pending.keys().copied().collect();
        order.sort_by_key(|k| (k.0 > 2, !self.drawn.contains(k), k.0));
        let started = web_time::Instant::now();
        for (count, key) in order.into_iter().take(budget).enumerate() {
            if count > 0 && key.0 > 2 && started.elapsed().as_secs_f64() > time_limit {
                break;
            }
            if let Some(image) = self.pending.remove(&key) {
                globe.upload(key, image.width, image.height, &image.rgba);
            }
        }
    }

    /// Снимки и высоты пришедших тайлов - в видеокарту и в хранилище.
    fn upload(&mut self) {
        if let Some(c) = &mut self.cog_base {
            let loaders = &self.loaders;
            c.step(&|limit| (loaders.request)(limit));
            for (key, tile) in c.take() {
                match tile {
                    crate::cogtiles::Tile::Image(image) => {
                        self.pending.insert(key, image);
                    }
                    crate::cogtiles::Tile::Missing => {
                        if let Some(globe) = &mut self.globe {
                            globe.mark_missing(key);
                        }
                    }
                }
            }
        }
        let bases: Vec<Basemap> = Basemap::ALL.into_iter().chain(self.ready_loader.as_ref().map(|(b, _)| *b)).collect();
        for basemap in bases {
            // Тайлы прежней основы, пришедшие после смены, не нужны.
            let arrived = self.base_loader(basemap).map(|l| l.take()).unwrap_or_default();
            let Some(globe) = &mut self.globe else {
                continue;
            };
            if basemap != self.basemap {
                continue;
            }
            for loaded in arrived {
                if loaded.missing {
                    globe.mark_missing(loaded.key);
                    if let Some(check) = &mut self.check {
                        check.note_missing();
                    }
                } else if let Ok(net::Content::Image(image)) = loaded.content {
                    self.pending.insert(loaded.key, image);
                }
            }
        }
        self.upload_pending();
        // Векторный тайл нужен и подписям, и линиям векторной основы.
        for loaded in self.loaders.places.take() {
            // Байты общие: копия для линий стоила кадру на крупных тайлах.
            let bytes = match loaded.content {
                Ok(net::Content::Bytes(b)) => Some(Arc::new(b)),
                Ok(net::Content::Image(_)) => None,
                Err(_) if loaded.missing => None,
                Err(_) => continue,
            };
            if self.label_keys.contains(&loaded.key) {
                self.labels.add(loaded.key, bytes.clone());
            }
            if self.building_keys.contains(&loaded.key)
                && let Some(b) = self.plugged.values_mut().find_map(|l| l.buildings_mut())
            {
                b.add(loaded.key, bytes.clone());
                // Тайл только зданий в основу не идёт.
                if self.building_only.contains(&loaded.key) {
                    continue;
                }
            }
            if self.route_keys.contains(&loaded.key) {
                self.route.add_tile(self.lang, loaded.key, bytes.as_deref().map(Vec::as_slice));
                // Тайл только маршрута в основу не идёт.
                if self.route_only.contains(&loaded.key) {
                    continue;
                }
            }
            if self.water_keys.contains(&loaded.key) {
                self.water_tiles.add(loaded.key, bytes.as_ref());
                // Тайл только маски воды в основу не идёт.
                if self.water_only.contains(&loaded.key) {
                    continue;
                }
            }
            self.vector.add(loaded.key, bytes);
        }
        // Ряды дат тем GIBS: выбран день - тема ложится на глобус.
        let mut days_changed = false;
        let now_utc = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
        for state in self.theme_days.values_mut() {
            if let ThemeDay::Asking(request) = state
                && let Some(answer) = request.take()
            {
                let intervals = answer.map(|b| data::gibs::parse_domains(&String::from_utf8_lossy(&b))).unwrap_or_default();
                let ready = data::gibs::ready_intervals(&intervals, now_utc);
                *state = data::gibs::pick_day(&ready, None).map_or(ThemeDay::Failed, |day| ThemeDay::Day(day, ready));
                days_changed = true;
            }
        }
        // День темы - по шкале времени: правый край окна шкалы.
        let moment = match self.timebar.window() {
            Some((_, hi)) if hi.is_finite() => Some(hi),
            _ => None,
        };
        for (id, state) in &mut self.theme_days {
            if let ThemeDay::Day(day, ready) = state
                && let Some(picked) = data::gibs::pick_day(ready, moment)
                && picked != *day
            {
                *day = picked;
                days_changed = true;
                // Загрузчик темы - на прежний день, его место займёт новый.
                self.map_loaders.remove(id);
            }
        }
        if days_changed {
            self.sync_maps();
        }
        // Весь каталог NASA GIBS: строки карт, слои из настроек - на глобус.
        if !self.gibs_pending.is_empty() {
            self.ask_gibs_catalog();
        }
        if let Some(request) = &self.gibs_catalog
            && let Some(answer) = request.take()
        {
            self.gibs_catalog = None;
            match answer {
                Ok(bytes) => {
                    crate::maps::register(data::gibs::parse_capabilities(&String::from_utf8_lossy(&bytes)));
                    for key in std::mem::take(&mut self.gibs_pending) {
                        if let Some(n) = crate::maps::index(&key) {
                            self.extras.insert(crate::layers::Extra::Map(n));
                            self.set_extra(crate::layers::Extra::Map(n), true);
                        }
                    }
                }
                Err(e) => self.open_message = Some(crate::i18n::trf(self.lang, "The NASA GIBS catalog did not load: {error}", &[("error", &e)])),
            }
        }
        for state in self.theme_scales.values_mut() {
            if let ThemeScale::Asking(request) = state
                && let Some(answer) = request.take()
            {
                *state = ThemeScale::Ready(answer.ok().and_then(|b| data::gibs::parse_colormap(&String::from_utf8_lossy(&b))));
            }
        }
        // Сводки слоёв точек и точки в видеокарту.
        if let Some(message) = self.dots.poll(self.lang) {
            self.open_message = Some(message);
        }
        // Геофизика на год шкалы времени и у вида.
        let (year, start) = self.magnetic_year();
        let view = crate::geophysics::View { lat: self.nav.pose.lat, lon: self.nav.pose.lon, width: self.view_width() };
        let loaders = &self.loaders;
        if let Some(message) = self.geo.update(&self.dots, self.lang, year, start, &view, &|| (loaders.request)(1 << 20)) {
            self.open_message = Some(message);
        }
        if self.geo.shapes_dirty || self.geo.lines_dirty {
            self.labels.set_geo(&self.geo.labels());
            // Точки полюсов - вместе с точками слоёв.
            self.dots.dirty |= self.geo.shapes_dirty;
        }
        // Линии слоёв линий и линии склонения - в картинки наложения,
        // названия плит - в надписи.
        // Слои «Моих данных»: точки - вместе с точками слоёв, подписи
        // по полю - у точки взгляда, `layer_labels` модуля.
        self.dots.dirty |= std::mem::take(&mut self.data_layers.dots_dirty);
        let key = crate::datalayers::label_key(self.nav.pose.lat, self.nav.pose.lon, self.view_width());
        if std::mem::take(&mut self.data_layers.labels_dirty) || self.data_layers.labels_key != Some(key) {
            self.data_layers.labels_key = Some(key);
            let (labels, sides) = self.data_layers.labels_with_sides(self.nav.pose.lat, self.nav.pose.lon);
            self.labels.set_layers_sided(&labels, &sides);
        }
        let data_lines = std::mem::take(&mut self.data_layers.lines_dirty);
        if self.dots.dirty || self.geo.lines_dirty || data_lines {
            self.geo.lines_dirty = false;
            self.labels.set_plates(&self.dots.plate_names());
            let mut sets = self.dots.line_sets();
            sets.extend(self.geo.line_set());
            sets.extend(self.data_layers.line_set());
            if self.vector.set_line_sets(sets)
                && let Some(globe) = &mut self.globe
            {
                globe.clear_overlays();
            }
        }
        let window = self.timebar.window();
        if (self.dots.dirty || self.track_dots_dirty || window != self.dots_window)
            && let Some(globe) = &mut self.globe
        {
            self.dots.dirty = false;
            self.dots_window = window;
            let terrain = self.nav.pose.terrain.clone();
            let height = move |lat: f64, lon: f64| terrain.as_ref().map_or(0.0, |t| t(lat, lon));
            let mut all = self.clouds.clone();
            all.extend(self.live_points.iter().cloned());
            all.extend(self.sat_points.iter().cloned());

            all.extend(self.dots.meshes(&self.nav.camera.ellipsoid, &height, window));
            // Точки слоёв «Моих данных» - кругами на рельефе.
            for ((dots, color, size, alpha), side) in self.data_layers.dot_sets() {
                let points: Vec<DVec3> = dots.iter().map(|(lat, lon)| self.nav.camera.ellipsoid.geodetic_to_ecef(*lat, *lon, height(*lat, *lon))).collect();
                let center = points.iter().fold(DVec3::ZERO, |a, p| a + *p) / points.len() as f64;
                all.push(render::PointsMesh {
                    side,
                    center,
                    offsets: points.iter().map(|p| (*p - center).as_vec3().to_array()).collect(),
                    colors: vec![color; points.len()],
                    size,
                    sizes: Vec::new(),
                    dot: Some(render::Dot { alpha, lift: crate::dots::LIFT, space: false }),
                });
            }
            // Объекты треков: точка над землёй или на своей высоте над
            // уровнем моря, подписи - у тех же точек.
            self.track_dots_dirty = false;
            let scale = self.heights.read().scale();
            let e = &self.nav.camera.ellipsoid;
            let level = |d: &crate::tracks::Dot| {
                let ground = height(d.lat, d.lon);
                match (d.alt.is_finite(), d.relative) {
                    (false, _) => ground,
                    (true, true) => ground + d.alt * scale,
                    (true, false) => d.alt * scale,
                }
            };
            self.track_labels = self.tracks.labels().iter().map(|d| (d.name.clone(), e.geodetic_to_ecef(d.lat, d.lon, level(d)))).collect();
            let mut dots = self.tracks.dots().into_iter().cloned().collect::<Vec<_>>();
            // «Моё место» - точкой по земле.
            if let Some(f) = self.gps.fix.filter(|_| self.gps.on) {
                dots.push(crate::tracks::Dot { name: String::new(), lat: f.lat, lon: f.lon, alt: f64::NAN, relative: false, color: crate::gps::COLOUR });
            }
            if !dots.is_empty() {
                let points: Vec<DVec3> = dots.iter().map(|d| e.geodetic_to_ecef(d.lat, d.lon, level(d))).collect();
                let center = points.iter().fold(DVec3::ZERO, |a, p| a + *p) / points.len() as f64;
                all.push(render::PointsMesh { side: render::Side::Both,
                    center,
                    offsets: points.iter().map(|p| (*p - center).as_vec3().to_array()).collect(),
                    colors: dots.iter().map(|d| [d.color[0], d.color[1], d.color[2]]).collect(),
                    size: crate::live::POINT_SIZE,
                    sizes: Vec::new(),
                    dot: Some(render::Dot { alpha: 1.0, lift: crate::dots::LIFT, space: false }),
                });
            }
            let poles = self.geo.points();
            if !poles.is_empty() {
                let points: Vec<globe_core::DVec3> = poles.iter().map(|(lat, lon, _)| self.nav.camera.ellipsoid.geodetic_to_ecef(*lat, *lon, height(*lat, *lon))).collect();
                let center = points.iter().fold(globe_core::DVec3::ZERO, |a, p| a + *p) / points.len() as f64;
                all.push(render::PointsMesh { side: render::Side::Both,
                    center,
                    offsets: points.iter().map(|p| (*p - center).as_vec3().to_array()).collect(),
                    colors: poles.iter().map(|p| p.2).collect(),
                    size: crate::geophysics::POLE_DOT,
                    sizes: Vec::new(),
                    dot: Some(render::Dot { alpha: 1.0, lift: crate::dots::LIFT, space: false }),
                });
            }
            globe.set_points(&all);
        }
        // Звёзды: каталог просится при первом показе, как у модуля.
        if (self.settings.stars || self.sky.is_some()) && self.stars.is_none() {
            let r = (self.loaders.request)(ANSWER_BYTES);
            r.start(data::stars::URL);
            self.stars = Some(Err(r));
        }
        if let Some(Err(r)) = &self.stars
            && let Some(answer) = r.take()
        {
            let found = answer.and_then(|b| data::stars::parse(&b));
            let ok = found.is_ok();
            if let (Ok(list), Some(globe)) = (found, &mut self.globe) {
                let points: Vec<render::StarPoint> = list
                    .iter()
                    .map(|s| render::StarPoint {
                        dir: s.dir.map(|v| v as f32),
                        size: s.size as f32,
                        color: [s.color[0] as f32, s.color[1] as f32, s.color[2] as f32, s.brightness as f32],
                    })
                    .collect();
                globe.set_stars(&points);
            }
            self.stars = Some(Ok(ok));
        }
        // Картинка неба - вместе со звёздами, раскладывается один раз.
        if (self.settings.stars || self.sky.is_some()) && self.sky_image.is_none() {
            let r = (self.loaders.request)(data::stars::SKY_BYTES);
            r.start(data::stars::SKY_URL);
            self.sky_image = Some(Err(r));
        }
        if let Some(Err(r)) = &self.sky_image
            && let Some(answer) = r.take()
        {
            let image = answer.and_then(|b| image::load_from_memory(&b).map_err(|e| e.to_string()));
            let ok = image.is_ok();
            if let (Ok(img), Some(globe)) = (image, &mut self.globe) {
                let rgba = img.to_rgba8();
                let (w, h) = rgba.dimensions();
                globe.set_sky_image(w, h, rgba.as_raw());
            }
            self.sky_image = Some(Ok(ok));
        }
        if let Some(globe) = &mut self.globe {
            // Небо - в своей системе J2000, без поворота и всегда видно.
            let frame = if self.sky.is_some() { Some((globe_core::DMat3::IDENTITY, 1.0)) } else { self.settings.stars.then(|| {
                let m = data::stars::sky_rotation(sun_moment(self.timebar.window()));
                let rows = globe_core::DMat3::from_cols_array_2d(&m).transpose();
                (rows, data::stars::fade(self.nav.camera.altitude()) as f32)
            }) };
            globe.set_star_frame(frame);
            globe.set_sky_map_frame(frame);
        }
        // Огни городов, как у модуля: при солнце на Земле.
        if self.settings.sun != self.lights_loader.is_some() {
            self.lights_loader = self.settings.sun.then(|| (self.loaders.make)(LIGHTS));
            if !self.settings.sun
                && let Some(globe) = &mut self.globe
            {
                globe.clear_lights();
            }
        }
        if let (Some(loader), Some(globe)) = (&self.lights_loader, &mut self.globe) {
            for loaded in loader.take() {
                if let Ok(net::Content::Image(image)) = loaded.content {
                    globe.set_lights(loaded.key, image.width, image.height, &image.rgba);
                }
            }
        }
        // Тайлы карт картинками.
        for (id, loaders) in &self.map_loaders {
            for (_, loader) in loaders {
                for loaded in loader.take() {
                    let rgba = match loaded.content {
                        Ok(net::Content::Image(image)) => Some(image.rgba),
                        _ => None,
                    };
                    self.vector.add_map(id, loaded.key, rgba);
                }
            }
        }
        // Разбор подписей - в бюджете кадра.
        if self.labels.decode_queued(LABEL_BUDGET) {
            self.labels.pending = true;
        }
        // Геоид: высоты - тайлы геоида, без дна моря и своего рельефа.
        if let Some(g) = &self.geoid {
            let arrived = g.loader.take();
            if arrived.is_empty() {
                return;
            }
            let mut store = self.heights.write();
            for loaded in arrived {
                let (z, x, y) = loaded.key;
                if let Ok(net::Content::Image(img)) = loaded.content
                    && let Some(tile) = terrain::make_tile(z, x, y, &img.rgba, None, Encoding::Terrarium)
                {
                    store.add(tile);
                }
            }
            drop(store);
            self.nav.keep_clear();
            return;
        }
        let arrived = self.loaders.terrain.take();
        if arrived.is_empty() && !self.relief.busy() {
            return;
        }
        let mut restale = Vec::new();
        {
            let mut store = self.heights.write();
            for loaded in arrived {
                let (z, x, y) = loaded.key;
                let Ok(net::Content::Image(img)) = loaded.content else {
                    continue;
                };
                if img.rgba.len() != terrain::SIZE * terrain::SIZE * 4 {
                    continue;
                }
                // Дно моря глубже уровня terrain::SEA_LEVEL - из предка.
                let mut raw = terrain::decode(&img.rgba, 4, None, Encoding::Terrarium);
                store.merge_sea(loaded.key, &mut raw);
                if let Some((tile, depths)) = terrain::tile_depths(z, x, y, raw, self.earth_floor()) {
                    // Свой рельеф врезается в тайл до хранилища. Глубины
                    // моря - для профиля высот и окна «Объекты».
                    let sea = depths.is_some();
                    if let Some(d) = depths {
                        store.add_depths(loaded.key, d);
                    }
                    store.add(self.relief.patch(tile));
                    // Потомки, пришедшие раньше, берут дно у этого тайла.
                    if sea {
                        let relief = &self.relief;
                        let floor = self.earth_floor();
                        let done = store.remerge_sea(loaded.key, floor, &|t| relief.patch(t));
                        if !done.is_empty() {
                            restale.push(loaded.key);
                        }
                    }
                }
            }
            self.relief.build(&mut store);
        }
        if let Some(globe) = &mut self.globe
            && !restale.is_empty()
        {
            globe.stale_meshes(Some(&restale));
        }
        // Точные высоты могли прийти выше глаза.
        self.nav.keep_clear();
    }

    /// Показ кадра глобуса в egui, текстура регистрируется заново после
    /// смены размера.
    fn show_target(&mut self, recreated: bool) {
        let (Some(rs), Some(globe)) = (&self.render_state, &self.globe) else {
            return;
        };
        let Some(view) = globe.color_view() else {
            return;
        };
        let mut renderer = rs.renderer.write();
        match self.texture {
            Some(id) if recreated => {
                renderer.update_egui_texture_from_wgpu_texture(&rs.device, view, wgpu::FilterMode::Nearest, id);
            }
            Some(_) => {}
            None => {
                self.texture = Some(renderer.register_native_texture(&rs.device, view, wgpu::FilterMode::Nearest));
            }
        }
    }

    /// Знак PlanetX и подпись источников, как в модуле: подложка
    /// и рельеф целиком, дальше векторная основа.
    fn attribution(&mut self, ctx: &egui::Context, view: egui::Rect) {
        // На другом теле - его снимки и высоты, земных подписей мест нет.
        let mut parts = if self.planet.earth {
            let mut p = vec![self.basemap.attribution(self.lang)];
            // Вокруг районной карты - тайлы Esri и их подпись.
            p.extend(self.basemap.around_attribution(self.lang));
            p.extend([credits::TERRAIN, labels::ATTRIBUTION]);
            p
        } else {
            let mut p = vec![self.basemap.attribution(self.lang)];
            p.extend(self.planet.terrain.as_ref().map(|t| (t.credit, "")));
            p
        };
        // Подписи включённых карт картинками.
        parts.extend(self.vector.map_keys().into_iter().filter_map(crate::maps::kind).map(|k| (k.attribution(), k.link())));
        parts.extend(self.dots.keys().into_iter().map(|l| l.attribution()));
        if self.lights_loader.is_some() {
            parts.push(LIGHTS_ATTRIBUTION);
        }
        if self.settings.stars && self.nav.camera.altitude() > data::stars::FADE_LOW {
            parts.push(data::stars::ATTRIBUTION);
            if matches!(self.sky_image, Some(Ok(true))) {
                parts.push(data::stars::SKY_ATTRIBUTION);
            }
        }
        parts.extend(self.plugged.values().filter_map(|l| l.attribution()));
        if let Some(layer) = self.events.layer() {
            parts.push(layer.credit);
        }
        parts.extend(self.lidar_credits.iter().copied());
        if self.rumsey.is_some() {
            parts.push(data::rumsey::ATTRIBUTION);
        }
        if self.oam.is_some() {
            parts.push((data::oam::ATTRIBUTION, ""));
        }
        if self.cutaway.is_some() || self.section.is_some() {
            parts.push(data::crust::CITATION);
            parts.push(data::slabs::ATTRIBUTION);
        }
        if self.section.is_some() {
            parts.push(data::quakes::ATTRIBUTION);
        }
        self.credits.show(ctx, view, self.lang, &parts, 2);
    }

    /// Строка состояния панели, как у модуля: высота глаза и количество
    /// идущих запросов.
    fn status_text(&self) -> String {
        let e = &self.nav.camera.ellipsoid;
        let (_, _, height) = e.ecef_to_geodetic(self.nav.camera.eye);
        let loaders = &self.loaders;
        let loading: usize = [
            loaders.esri.stats(),
            loaders.osm.stats(),
            loaders.terrain.stats(),
            loaders.places.stats(),
            loaders.pictures.stats(),
            loaders.files.stats(),
        ]
        .iter()
        .map(|s| s.requests_now.load(Ordering::SeqCst))
        .sum();
        let (km, m) = (tr(self.lang, "km"), tr(self.lang, "m"));
        let height = if height.abs() >= 10_000.0 {
            format!("{} {km}", group_thousands((height / 1000.0).round() as i64))
        } else {
            format!("{} {m}", group_thousands(height.round() as i64))
        };
        // Первая строка, как `_show_state` модуля: сообщение пять секунд,
        // иначе высота обзора и хвост загрузки.
        let first = match &self.message_seen {
            Some((text, _)) => text.clone(),
            None => {
                let mut line = trf(self.lang, "View from {height}", &[("height", &height)]);
                if loading > 0 {
                    line += &format!(" · {}", trf(self.lang, "loading {count}", &[("count", &loading.to_string())]));
                }
                line
            }
        };
        match self.cursor_point {
            Some((lat, lon, h)) => format!("{first}\n{}", self.point_text(lat, lon, Some(h), 5)),
            None => first,
        }
    }

    /// Точка словами, `point_text` модуля: координаты в формате свойств
    /// вида и высота или глубина. С выключенным рельефом - координаты.
    fn point_text(&self, lat: f64, lon: f64, height: Option<f64>, digits: usize) -> String {
        let coords = match self.settings.coords.as_str() {
            "dms" | "utm" | "mgrs" => crate::coords_text::format(self.lang, lat, lon, &self.settings.coords, digits),
            _ => format!("{lat:.digits$}, {lon:.digits$}"),
        };
        match height.filter(|_| self.settings.relief) {
            // Ниже -0.5 м - глубина, выше - высота не меньше нуля.
            Some(h) if h < -0.5 => trf(self.lang, "{coords}, depth {d} m", &[("coords", &coords), ("d", &(-h).round().to_string())]),
            Some(h) => trf(self.lang, "{coords}, height {h} m", &[("coords", &coords), ("h", &h.max(0.0).round().to_string())]),
            None => coords,
        }
    }

    /// Окна и органы управления на экране: не самопроверка и не снимок
    /// без интерфейса.
    fn shows_ui(&self) -> bool {
        self.check.is_none() && (self.shot.is_none() || self.shot_ui)
    }

}

impl eframe::App for LiteApp {
    /// «Мои метки» - в хранилище, если изменились. eframe зовёт это
    /// раз в 30 с и при выходе.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.check.is_some() {
            return;
        }
        if self.settings_dirty {
            storage.set_string(crate::settings::KEY, self.settings.to_text());
            self.settings_dirty = false;
        }
        self.save_my_layers(storage);
        self.save_addins(storage);
        // Список наборов прошлого запуска ещё не открыт - не затирать его.
        if self.tilesets.save_dirty && self.tilesets_saved.is_none() {
            storage.set_string(crate::tilesets::KEY, self.tilesets.index());
            self.tilesets.save_dirty = false;
        }
        if !self.places_dirty {
            return;
        }
        let Some(root) = self.marks.export_tree(&[]) else {
            return;
        };
        // «Временные метки» - своим ключом: в корень они не входят.
        let temporary = self.marks.temporary_folder().map_or(Ok(String::new()), places::to_text);
        match places::to_text(&root).and_then(|own| temporary.map(|t| (own, t))) {
            // На странице предел localStorage около 5 МБ.
            Ok((own, t)) if cfg!(target_arch = "wasm32") && own.len() + t.len() > PAGE_PLACES_LIMIT => {
                self.open_message = Some(tr(self.lang, "My Places exceed 4 MB, the page cannot save them").to_string());
            }
            Ok((own, t)) => {
                storage.set_string(places::KEY, own);
                if self.marks.temporary {
                    storage.set_string(places::TEMP_KEY, t);
                }
            }
            Err(e) => self.open_message = Some(format!("{}: {e}", marks::root_name(self.lang))),
        }
        self.places_dirty = false;
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.restore_my_layers();
        self.restore_addins();
        // Файлы «Моих слоёв» на диске изменились - строки «Обновить».
        if self.data_layers.check_sources(now, &crate::datalayers::file_modified) {
            ctx.request_repaint();
        }
        self.restore_tilesets();
        self.ask_temporary(&ctx);
        self.upload();
        if std::mem::take(&mut self.fonts_dirty) {
            self.install_fonts(&ctx);
        }
        // На узком экране, телефоне стоймя, панель при запуске скрыта:
        // глобусу остаётся весь экран, панель открывает первая кнопка
        // панели инструментов. Замечание автора от 23 октября 2026 года.
        if !std::mem::replace(&mut self.layout_checked, true) && ctx.content_rect().width() < NARROW {
            self.panel_open = false;
        }
        if self.check.is_none() && (self.shot.is_none() || self.shot_ui) {
            self.marks_panel(ui, now);
            self.side_panel(ui);
        }

        let rect = ui.available_rect_before_wrap();
        let globe_response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        // Тур останавливает своя навигация: протяжка, колесо, жест.
        let scrolled = ctx.input(|i| i.smooth_scroll_delta != egui::Vec2::ZERO || i.multi_touch().is_some());
        if self.player.is_some() && (globe_response.drag_started() || (scrolled && globe_response.hovered())) {
            self.player = None;
        }
        // Пикселей кадра глобуса на логический пиксель. Кадр больше
        // наибольшей стороны текстуры видеокарты рисуется в меньшем
        // разрешении, экран телефона бывает больше 4096 пикселей.
        let max_side = f64::from(self.globe.as_ref().map_or(u32::MAX, Globe::max_side));
        let ppp = f64::from(ctx.pixels_per_point());
        let longest = f64::from(rect.width().max(rect.height())) * ppp;
        let ppp = if longest > max_side { ppp * max_side / longest } else { ppp };
        let width = ((f64::from(rect.width()) * ppp).floor() as u32).max(1);
        let height = ((f64::from(rect.height()) * ppp).floor() as u32).max(1);
        self.nav.camera.width = width;
        self.nav.camera.height = height;
        if self.sky.is_some() {
            // Помощник и агенты MCP работают и в небе. Раньше их шаг
            // стоял ниже выхода: в небе сообщения не шли, снимок
            // агента не снимался, 28 октября 2026 года.
            self.step_ai(&ctx, now);
            #[cfg(not(target_arch = "wasm32"))]
            self.step_mcp(&ctx, now);
            self.sky_frame(&ctx, ui, rect, &globe_response, ppp, now);
            return;
        }

        let loading = self.base_loader(self.basemap).is_some_and(|l| l.busy())
            || !self.pending.is_empty()
            || self.map_loaders.values().flatten().any(|(_, l)| l.busy())
            || self.lights_loader.as_ref().is_some_and(|l| l.busy())
            || self.theme_days.values().any(|d| matches!(d, ThemeDay::Asking(_)))
            || self.dots.busy()
            || self.route.busy()
            || self.loaders.terrain.busy()
            || self.loaders.places.busy()
            || self.labels.pending
            || self.search.busy()
            || self.pictures.busy()
            || self.fetching.is_some()
            || self.tilesets.busy()
            || self.parcel_ask.is_some()
            || self.plugged.values().any(|l| l.busy())
            || self.rumsey.as_ref().is_some_and(crate::rumsey::Rumsey::busy)
            || self.oam.as_ref().is_some_and(crate::oam::Oam::busy)
            || self.wayback.as_ref().is_some_and(crate::wayback::Wayback::busy)
            || self.sentinel.as_ref().is_some_and(crate::sentinel::Sentinel::busy)
            || self.plugged.values().filter_map(|l| l.buildings()).any(|b| b.busy(self.heights.read().version()))
            || self.field_tiles.as_ref().is_some_and(crate::fieldmap::FieldTiles::busy)
            || self.gibs_catalog.is_some()
            || self.events.busy()
            || self.lidar.as_ref().is_some_and(crate::lidarnet::Lidar::busy)
            || self.relief_calc.as_ref().is_some_and(crate::reliefcalc::ReliefCalc::busy)
            || self.hydro.busy()
            || self.earthwork.busy()
            || self.volumes.as_ref().is_some_and(crate::volumecalc::VolumeCalc::busy)
            || self.osm_mines.as_ref().is_some_and(|o| o.busy(now, &self.overpass_mirrors))
            || self.osm_infra.as_ref().is_some_and(|o| o.busy(now, &self.overpass_mirrors))
            || self.viewshed.as_ref().is_some_and(crate::pointcalc::Calc::busy)
            || self.insolation.as_ref().is_some_and(crate::pointcalc::Calc::busy)
            || self.shadows.as_ref().is_some_and(crate::pointcalc::Calc::busy)
            || self.aef_change.as_ref().is_some_and(crate::aefcalc::ChangeCalc::busy)
            || self.aef_class.as_ref().is_some_and(crate::aefclass::ClassCalc::busy)
            || self.similar.as_ref().is_some_and(crate::aefclass::ClassCalc::busy)
            || self.tracing.as_ref().is_some_and(crate::aefclass::ClassCalc::busy)
            || self.object_tasks.as_ref().is_some_and(|c| c.busy() || c.finding())
            || self.recognize.as_ref().is_some_and(|c| c.busy() || c.finding())
            || self.spectral.as_ref().is_some_and(crate::spectral::Spectral::busy)
            || self.emit.as_ref().is_some_and(crate::emit::Emit::busy)
            || self.section.as_ref().is_some_and(crate::section::SectionWindow::busy)
            || self.cog_base.as_ref().is_some_and(crate::cogtiles::CogTiles::busy)
            || self.relief.busy();
        self.loading_since = if loading { self.loading_since.or(Some(now)) } else { None };
        let base_busy = self.base_loader(self.basemap).is_some_and(|l| l.busy());
        if let Some(check) = &mut self.check {
            let parts = [
                ("basemap", base_busy),
                ("terrain", self.loaders.terrain.busy()),
                ("places", self.loaders.places.busy()),
                ("labels", self.labels.pending),
                ("search", self.search.busy()),
                ("pictures", self.pictures.busy()),
                ("fetching", self.fetching.is_some()),
            ];
            let loaders = &self.loaders;
            let requests = [loaders.esri.stats(), loaders.osm.stats(), loaders.terrain.stats(), loaders.places.stats(), loaders.pictures.stats(), loaders.files.stats()]
                .iter()
                .map(|s| s.requests_total.load(Ordering::SeqCst))
                .sum();
            check.note_busy(parts.iter().filter(|(_, b)| *b).map(|(n, _)| *n).collect::<Vec<_>>().join(","), requests);
            check.note_tour(self.player.is_some());
            let mut basemap = self.basemap;
            check.before_frame(&mut self.nav, now, loading, &mut basemap);
            self.set_basemap(basemap);
        } else {
            // Органы навигации и вершина инструмента держат мышь - Землю
            // она не двигает.
            let padded = self.shows_ui() && !self.touch && self.navpad.input(&ctx, rect, globe_response.layer_id, &mut self.nav, now);
            self.globe_width_px = rect.width() * ppp as f32;
            let swiped = self.swipe_input(&ctx, rect);
            let busy = padded
                || swiped
                || self.tool.as_mut().is_some_and(|t| t.input(&ctx, &globe_response, rect, ppp, &self.nav))
                || self.overlay_props.as_mut().is_some_and(|p| p.input(&ctx, &globe_response, rect, ppp, &self.nav))
                || {
                    // Участки окон анализа: вершины тянутся мышью.
                    let nav = &self.nav;
                    [&mut self.viewshed, &mut self.insolation, &mut self.shadows]
                        .into_iter()
                        .flatten()
                        .filter_map(crate::pointcalc::Calc::site_edit)
                        .chain(self.relief_calc.as_mut().and_then(crate::reliefcalc::ReliefCalc::site_edit))
                        .chain(self.hydro.site_edits())
                        .chain(self.earthwork.site_edit())
                        .chain(self.aef_change.as_mut().and_then(crate::aefcalc::ChangeCalc::site_edit))
                        .any(|s| s.input(&ctx, &globe_response, rect, ppp, nav))
                }
                || {
                    // Классификация: участок и контрольные точки щелчком.
                    let nav = &self.nav;
                    self.aef_class.as_mut().is_some_and(|c| c.input(&ctx, &globe_response, rect, ppp, nav)) || self.similar.as_mut().is_some_and(|c| c.input(&ctx, &globe_response, rect, ppp, nav))
                        || self.tracing.as_mut().is_some_and(|c| c.input(&ctx, &globe_response, rect, ppp, nav))
                        || self.object_tasks.as_mut().is_some_and(|c| c.input(&ctx, &globe_response, rect, ppp, nav))
                        || self.recognize.as_mut().is_some_and(|c| c.input(&ctx, &globe_response, rect, ppp, nav))
                        || self.spectral.as_mut().is_some_and(|c| c.input(&ctx, &globe_response, rect, ppp, nav, self.lang))
                        || self.emit.as_mut().is_some_and(|c| c.input(&ctx, &globe_response, rect, ppp, nav))
                };
            // Изменённый участок - общий для открытых окон анализа.
            let edits = [&mut self.viewshed, &mut self.insolation, &mut self.shadows]
                .into_iter()
                .flatten()
                .filter_map(crate::pointcalc::Calc::site_edit)
                .chain(self.relief_calc.as_mut().and_then(crate::reliefcalc::ReliefCalc::site_edit))
                .chain(self.hydro.site_edits())
                .chain(self.earthwork.site_edit())
                .chain(self.aef_change.as_mut().and_then(crate::aefcalc::ChangeCalc::site_edit))
                .chain(self.aef_class.as_mut().and_then(crate::aefclass::ClassCalc::site_edit))
                .chain(self.similar.as_mut().and_then(crate::aefclass::ClassCalc::site_edit))
                .chain(self.tracing.as_mut().and_then(crate::aefclass::ClassCalc::site_edit))
                .chain(self.object_tasks.as_mut().and_then(crate::aefclass::ClassCalc::site_edit))
                .chain(self.recognize.as_mut().and_then(crate::aefclass::ClassCalc::site_edit))
                .chain(self.spectral.as_mut().and_then(crate::spectral::Spectral::site_edit))
                .chain(self.emit.as_mut().and_then(crate::emit::Emit::site_edit))
                .collect::<Vec<_>>();
            // Участок из «Моих меток»: кнопка «Из меток» и щелчок выбора.
            let mut edits = edits;
            crate::analysis::sites_from_places(self.lang, &mut edits, self.marks.tree.as_ref(), &self.tree_state.picked());
            crate::analysis::share_sites(&mut self.area_site, edits);
            // Камера следом: колесо - расстояние до объекта, Esc отпускает,
            // `ChaseControl` модуля.
            if let Some((kind, _)) = self.live_follow.clone()
                && globe_response.hovered()
            {
                let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0
                    && let Some(live) = self.plugged.values_mut().filter_map(|l| l.live_mut()).find(|l| l.kind == kind)
                {
                    live.wheel(f64::from(scroll) / 50.0);
                    ctx.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.live_follow = None;
                }
            }
            // В полёте клавиши и колесо - у самолёта, камеру ставит полёт.
            let flying = self.flight.is_some();
            if let Some(flight) = &mut self.flight
                && globe_response.hovered()
            {
                let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    flight.wheel(f64::from(scroll) / 50.0);
                }
            }
            if !busy && !flying {
                self.input.handle(&ctx, rect, globe_response.layer_id, &mut self.nav, now, ppp);
            }
            // Точка под курсором - вторая строка состояния, как у модуля.
            let heights_at = self.true_heights();
            self.cursor_point = globe_response.hover_pos().and_then(|pos| {
                let (px, py) = (f64::from(pos.x - rect.min.x) * ppp, f64::from(pos.y - rect.min.y) * ppp);
                let p = globe_core::navigation::ground_under(&self.nav.camera, px, py, self.nav.pose.terrain.as_ref())?;
                let (lat, lon, _) = self.nav.camera.ellipsoid.ecef_to_geodetic(p);
                Some((lat, lon, heights_at(&[(lat, lon)]).first().copied().unwrap_or(0.0)))
            });
            // Правая кнопка без протяжки - меню на глобусе, как у модуля.
            if !padded
                && globe_response.secondary_clicked()
                && let Some(pos) = globe_response.interact_pointer_pos()
            {
                self.globe_menu = Some(self.make_globe_menu(pos, rect, ppp));
            }
            // «Определить объекты» включено: щелчок - окно «Объекты».
            if self.identify
                && self.tool.is_none()
                && !padded
                && globe_response.clicked()
                && let Some(pos) = globe_response.interact_pointer_pos()
            {
                let (px, py) = (f64::from(pos.x - rect.min.x) * ppp, f64::from(pos.y - rect.min.y) * ppp);
                if let Some(p) = globe_core::navigation::ground_under(&self.nav.camera, px, py, self.nav.pose.terrain.as_ref()) {
                    let (lat, lon, _) = self.nav.camera.ellipsoid.ecef_to_geodetic(p);
                    self.ask_map_value(lat, lon);
                    self.objects = Some(self.make_objects(lat, lon, Some((px, py, ppp)), None));
                    // Сведения об участках, когда показан кадастр. Один
                    // запрос на щелчок, прежний снимается.
                    self.parcels.clear();
                    self.parcel_ask = None;
                    if self.vector.map_keys().contains(&"cadastre") {
                        let ask = (self.loaders.request)(1 << 20);
                        ask.send(&data::cadastre::info_url(lat, lon), &[("Referer", data::cadastre::REFERER)], None);
                        self.parcel_ask = Some(((lat, lon), ask));
                    }
                    // Найденный щелчком спутник становится выбранным.
                    if let Some(number) = self.satellite_under(px, py, ppp)
                        && let Some(s) = self.plugged.values_mut().find_map(|l| l.satellites_mut())
                    {
                        s.select(Some(number));
                    }
                    // Слои реестра дозапрашивают сведения: первый абзац
                    // ближней статьи у щелчка.
                    self.objects_clicked();
                }
            }
            // Щелчок по метке без инструмента - её карточка, как у модуля.
            if self.tool.is_none()
                && !self.identify
                && !padded
                && !busy
                && globe_response.clicked()
                && let (Some(pos), Some(tree)) = (globe_response.interact_pointer_pos(), self.marks.tree.as_ref())
            {
                let (px, py) = (f64::from(pos.x - rect.min.x) * ppp, f64::from(pos.y - rect.min.y) * ppp);
                if let Some(path) = crate::card::pick(tree, &self.nav, px, py, crate::card::PICK_PIXELS * ppp) {
                    self.tree_state.selected = Some(path.clone());
                    self.card = Some(crate::card::Card { path });
                }
            }
            // Щелчок по объекту слоя реестра без инструмента - окно
            // «Объекты» у его точки. У статьи Википедии - ссылка и первый
            // абзац, как `open_article_card` модуля.
            if self.tool.is_none()
                && !self.identify
                && !padded
                && !busy
                && self.card.is_none()
                && globe_response.clicked()
                && let Some(pos) = globe_response.interact_pointer_pos()
            {
                let pixel = (f64::from(pos.x - rect.min.x) * ppp, f64::from(pos.y - rect.min.y) * ppp, ppp);
                let hit = self
                    .plugged
                    .values()
                    .filter_map(|l| l.hit(&self.nav.camera, pixel))
                    .filter_map(|p| crate::registry::gap(&self.nav.camera, p, pixel.0, pixel.1, f64::INFINITY).map(|g| (g, p)))
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                if let Some((_, p)) = hit {
                    let (lat, lon, _) = self.nav.camera.ellipsoid.ecef_to_geodetic(p);
                    let (q, _) = self.nav.camera.project(p);
                    self.objects = Some(self.make_objects(lat, lon, Some((q.x, q.y, ppp)), None));
                    self.objects_clicked();
                }
            }
        }
        if let Some(name) = self.check.as_mut().and_then(Check::take_scene_request) {
            self.load_check_scene(&name, now);
        }
        if let Some(index) = self.check.as_mut().and_then(Check::take_tour_request) {
            self.start_check_tour(index, now);
        }
        if let Some(line) = self.check.as_mut().and_then(Check::take_progress) {
            (self.report)(&line, false);
        }
        self.step_fetch(now);
        if let Some(query) = self.check.as_mut().and_then(Check::take_search) {
            self.search_text = query.clone();
            if let Some(place) = self.search.submit(&query) {
                self.fly_to_found(&place, now);
            }
        }
        if let Some(place) = self.search.step(now, self.loaders.search.as_ref()) {
            self.fly_to_found(&place, now);
        }
        self.pictures.step(self.loaders.pictures.as_ref());
        self.pictures.due(now);
        self.picked(now);
        self.step_links(now);
        self.step_route(now);
        self.step_live(now);
        self.step_flight(&ctx, now);
        self.step_subsurface(&ctx, now);
        self.own_window(&ctx);
        self.point_windows(&ctx);
        self.step_calcs(&ctx, now);
        self.step_satellites(now);
        self.step_overpass(now);
        self.step_parcels();
        self.step_plugged(&ctx, now);
        self.step_lidar(now);
        self.step_events(now);
        self.step_rumsey(&ctx, now);
        self.step_oam(&ctx, now);
        self.step_wayback(&ctx);
        self.step_sentinel(&ctx, now);
        self.step_tracks(&ctx);
        self.step_ai(&ctx, now);
        self.step_gps(&ctx, now);
        #[cfg(not(target_arch = "wasm32"))]
        self.step_mcp(&ctx, now);
        #[cfg(not(target_arch = "wasm32"))]
        self.step_rustgis(&ctx, now);
        self.paleo_bar(&ctx, now);
        self.step_cutaway();
        self.check_linked_models(now);
        for (bytes, name) in std::mem::take(&mut self.pending_open) {
            let _ = self.open_file(&bytes, &name, None);
        }
        self.dropped(&ctx, now);
        self.step_film(&ctx, now);
        self.step_tour(now);
        self.step_haul(now);
        if self.recording.is_some() {
            self.record_sample(now);
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(RECORD_PERIOD));
        }
        self.timebar.step(now);
        if self.marks.set_window(self.timebar.window()) {
            self.marks_to_labels();
        }
        // Камера записи стоит: иначе вид не считался бы загруженным.
        let moving = self.nav.step(now) || (self.player.is_some() && self.film.is_none()) || self.timebar.playing();
        // Анимация моделей просит кадры, пока модель видна, как
        // `render/view.py` модуля. Снимок её не ждёт: она идёт по кругу.
        let animating = !self.animated.is_empty();
        // Ближняя плоскость - от ближайшего рельефа.
        let eye = self.nav.camera.eye;
        self.nav.camera.nearest =
            Some(nearest_terrain(&self.nav.camera.ellipsoid, eye, self.nav.pose.terrain.as_ref()));
        // Наложения для моделей, с пакетами моделей по ссылке, - только
        // когда набор сменился.
        // Треки на момент шкалы, сменились модели треков - модели заново.
        self.tracks.step(self.timebar.window().map(|w| w.1));
        let tracks_models = std::mem::take(&mut self.tracks.models_dirty);
        // Слои-картинки «Моих слоёв» сменились - сетки картинок заново.
        let pictures_version = self.data_layers.pictures_version(self.timebar.window());
        if pictures_version != self.data_pictures_seen {
            self.data_pictures_seen = pictures_version;
            self.preview_dirty = true;
            self.pictures_changed();
        }
        // Слои со временем идут за шкалой. Охват шкалы по снимкам и слоям
        // со временем - заново, только если он сменился, иначе шкала
        // сбрасывалась бы при каждом шаге.
        self.data_layers.set_window(self.timebar.window());
        let extent = self.data_layers.extent();
        if extent != self.data_extent_seen {
            self.data_extent_seen = extent;
            self.time_extent();
        }
        let preview_dirty = std::mem::take(&mut self.preview_dirty);
        if tracks_models || preview_dirty {
            self.overlays_built = None;
        }
        // Список нужен и в кадре, где подошли высоты: сетки меток ниже
        // поднимут номер набора, и модели собрались бы из пустого списка -
        // они исчезали, как только приходили высоты.
        let due = self.marks.heights_due(self.heights.read().version(), now);
        let listed = needs_models(self.overlays_built, self.marks.overlays_version(), due);
        let model_list = if listed { self.model_overlays(now) } else { Vec::new() };

        let Some(globe) = &mut self.globe else {
            ui.centered_and_justified(|ui| ui.label(tr(self.lang, "The graphics card does not support wgpu, the globe cannot be drawn.")));
            return;
        };
        let added = self.heights.write().take_added();
        let store = self.heights.read();
        let selection = {
            let ready = |k: TileKey| k.0 <= 2 || globe.has_texture(k);
            let ranges = |k: TileKey| store.range_for(k);
            let memo = self.memo.bind(&self.nav.camera, lod::THRESHOLD, store.version(), &added);
            lod::select(
                &self.nav.camera,
                &ready,
                Options { heights: Some(&ranges), memo: Some(memo), ..Options::default() },
            )
        };
        let mut wanted: Vec<(TileKey, f64)> = start_keys()
            .into_iter()
            .filter(|k| !globe.has_texture(*k))
            .map(|k| (k, lod::INFINITE_PRIORITY + 10.0 - f64::from(k.0)))
            .collect();
        // Пришедший, но не загруженный снимок второй раз не просится.
        wanted.extend(selection.want.iter().copied().filter(|(k, _)| !self.pending.contains_key(k)));
        self.was_moving = moving;
        self.drawn = selection.draw.iter().copied().collect();
        let bases: Vec<Basemap> = Basemap::ALL.into_iter().chain(self.ready_loader.as_ref().map(|(b, _)| *b)).collect();
        for basemap in bases {
            let loader = self.loaders.basemap(basemap).or_else(|| self.ready_loader.as_ref().filter(|(b, _)| *b == basemap).map(|(_, l)| l.as_ref()));
            if let Some(loader) = loader {
                // Уровни подробной мозаики идут не загрузчику тайлов.
                let base: Vec<(TileKey, f64)> = match &self.cog_base {
                    Some(c) if basemap == self.basemap => wanted.iter().copied().filter(|(k, _)| k.0 < c.from_level()).collect(),
                    _ => Vec::new(),
                };
                loader.want(if basemap != self.basemap { &[] } else if self.cog_base.is_some() { &base } else { &wanted });
            }
        }
        if let Some(c) = &mut self.cog_base {
            c.want(&wanted);
        }
        // Высоты глубже уровня 15 у своего рельефа собираются здесь.
        let mut needs = self.relief.route(height_needs(&store, &selection), &store);
        // Высоты расчётов у точки - вперёд тайлов кадра.
        needs.extend(self.calc_keys.iter().filter(|k| store.tile(**k).is_none()).map(|k| (*k, lod::INFINITE_PRIORITY + 5.0)));
        match &self.geoid {
            Some(g) => {
                g.loader.want(&needs);
                self.loaders.terrain.want(&[]);
            }
            None if terrain_source(&self.planet).is_some() => self.loaders.terrain.want(&needs),
            None => self.loaders.terrain.want(&[]),
        }
        let mut vector_tiles = if self.planet.earth { self.labels.wanted(&selection.draw) } else { Vec::new() };
        // Подписям - только их тайлы: разбор пунктов всех тайлов линий
        // стоил главному потоку до 40 кадров в секунду.
        self.label_keys = vector_tiles.iter().map(|(k, _)| *k).collect();
        let asked = self.label_keys.clone();
        vector_tiles.extend(self.vector.wanted(&selection.draw).into_iter().filter(|(k, _)| !asked.contains(k)));
        // Тайлы воды масок прохода воды - тем же загрузчиком, только для
        // тайлов кадра с рельефом ниже нуля, пока вода по уровню моря.
        let water_on = self.settings.sea && self.planet.earth && self.geoid.is_none() && self.hydro.sea_level <= 0.0;
        let water_list = if water_on {
            let low = |k: TileKey| store.range_for(k).0 < 0.0;
            self.water_tiles.wanted(&selection.draw, &low, &|k| globe.has_water_mask(k))
        } else {
            Vec::new()
        };
        self.water_keys = water_list.iter().map(|(k, _)| *k).collect();
        self.water_only = self.water_keys.iter().copied().filter(|k| !vector_tiles.iter().any(|(v, _)| v == k)).collect();
        vector_tiles.extend(water_list.into_iter().filter(|(k, _)| self.water_only.contains(k)));
        // Тайлы дорог маршрута - тем же загрузчиком, раньше основы.
        self.route_keys = self.route.wanted().into_iter().collect();
        self.route_only = self.route_keys.iter().copied().filter(|k| !vector_tiles.iter().any(|(v, _)| v == k)).collect();
        vector_tiles.extend(self.route_only.iter().map(|k| (*k, lod::INFINITE_PRIORITY)));
        // Тайлы зданий у глаза - у слоя реестра, который строит здания,
        // `render/buildings.py` модуля. Тайлы основы - тем же загрузчиком.
        if let Some(layer) = self.plugged.values_mut().find(|l| l.buildings().is_some())
            && let Some(wanted) = {
                let cam = &self.nav.camera;
                let (lat, lon, _) = cam.ellipsoid.ecef_to_geodetic(cam.eye);
                let forward = -cam.rotation.z_axis;
                let ground = store.height_at(lat, lon);
                layer.buildings_mut().map(|b| b.want(cam.eye.to_array(), forward.to_array(), ground))
            }
        {
            // Свои тайлы слоя, у Overture - загрузчиком архива PMTiles.
            let keys: Vec<TileKey> = wanted.iter().map(|(k, _)| *k).collect();
            let own = layer.building_tiles(&keys, now, &*self.loaders.request);
            if let Some(b) = layer.buildings_mut() {
                if let Some(own) = own {
                    self.building_keys.clear();
                    self.building_only.clear();
                    for (key, bytes) in own {
                        b.add(key, bytes);
                    }
                } else {
                    self.building_keys = b.wanted.iter().copied().collect();
                    self.building_only = wanted.iter().map(|(k, _)| *k).filter(|k| !vector_tiles.iter().any(|(v, _)| v == k)).collect();
                    vector_tiles.extend(wanted.into_iter().filter(|(k, _)| self.building_only.contains(k)));
                }
                let heights = |lats: &[f64], lons: &[f64]| -> Vec<f64> {
                    let points: Vec<(f64, f64)> = lats.iter().copied().zip(lons.iter().copied()).collect();
                    store.heights_at(&points, true)
                };
                b.step(globe, store.version(), &heights);
                // Здания для луча 3D-линейки: только при смене кадра зданий.
                if let Some(tool) = self.tool.as_mut().filter(|t| t.solid_on) {
                    let stamp = (store.version(), b.wanted.clone());
                    if self.solid_stamp.as_ref() != Some(&stamp) {
                        tool.targets = b.meshes(&heights).iter().map(crate::viewexport::solid_tile).collect();
                        self.solid_stamp = Some(stamp);
                    }
                }
            }
        } else {
            self.building_keys.clear();
            self.building_only.clear();
        }
        self.loaders.places.want(&vector_tiles);
        // Уровень высот горизонталей: открытое море - глубины с уровня
        // `contours::SEA_LEVEL`, как `sea_high` модуля.
        let sea = |key: TileKey| -> Option<f64> {
            if key.0 < data::contours::SEA_LEVEL {
                return None;
            }
            (5..=data::contours::SEA_LEVEL).rev().find_map(|level| {
                let parent = data::contours::ancestor(key, level);
                let tile = store.tile(parent)?;
                let side = (tile.heights.len() as f64).sqrt() as usize;
                let grid = data::contours::Grid { rows: side, cols: side, data: tile.heights.iter().map(|h| f64::from(*h)).collect() };
                Some(data::contours::footprint_high(&grid, parent, key))
            })
        };
        self.vector.note_contour_levels(&selection.draw, &sea);
        // Тайлы карт: каждому источнику - его уровни.
        let map_tiles = self.vector.maps_wanted(&selection.draw);
        // Пару текстур «после» и «до» занимают снимки события, без них -
        // шторка выпусков Wayback. Хозяин сменился - пара забывается.
        let event_layer = self.events.layer();
        let wayback_pair = event_layer.is_none() && self.wayback.as_ref().is_some_and(|w| w.compare.is_some());
        if let Some(w) = &mut self.wayback {
            w.blocked = event_layer.is_some();
        }
        let owner = if event_layer.is_some() { 1 } else if wayback_pair { 2 } else { 0 };
        if owner != self.pair_owner {
            self.pair_owner = owner;
            for side in 0..2 {
                globe.clear_events(side);
                self.event_empty[side].clear();
            }
        }
        if let Some(w) = self.wayback.as_mut().filter(|_| wayback_pair)
            && let Some(compare) = &mut w.compare
        {
            for side in 0..2 {
                let mut keys: Vec<TileKey> = Vec::new();
                for key in &selection.draw {
                    let k = terrain::ancestor(*key, key.0.min(data::wayback::MAX_LEVEL));
                    if !globe.has_event(side, k) && !self.event_empty[side].contains(&k) && !keys.contains(&k) {
                        keys.push(k);
                    }
                }
                for (key, rgba) in compare.tiles(side, &keys, &*self.loaders.request) {
                    match rgba.and_then(|r| data::rastermaps::overlay_rgba(&r)) {
                        Some(pic) => globe.set_event(side, key, 256, 256, &pic),
                        None => {
                            self.event_empty[side].insert(key);
                        }
                    }
                }
            }
        }
        // Снимки события «после» и «до» шторки: тайлы кадра не глубже
        // уровня снимков, своей парой текстур тайла.
        if let Some(layer) = event_layer {
            for side in 0..2 {
                let mut keys: Vec<TileKey> = Vec::new();
                for key in &selection.draw {
                    let k = terrain::ancestor(*key, key.0.min(layer.level));
                    if !globe.has_event(side, k) && !self.event_empty[side].contains(&k) && !keys.contains(&k) {
                        keys.push(k);
                    }
                }
                for (key, rgba) in self.events.tiles(side, &keys, &*self.loaders.request) {
                    match rgba.and_then(|r| data::rastermaps::overlay_rgba(&r)) {
                        Some(pic) => globe.set_event(side, key, 256, 256, &pic),
                        None => {
                            self.event_empty[side].insert(key);
                        }
                    }
                }
            }
        }
        globe.set_split(pair_swipe(&self.events, self.wayback.as_ref()).map(|f| f * self.globe_width_px));
        // Шторка слоёв - пока у слоёв есть стороны.
        let swipe = self.layer_swipe.filter(|_| self.data_layers.has_sides()).map(|f| f * self.globe_width_px);
        globe.set_swipe(swipe);
        self.labels.swipe = swipe;
        // Снимки полюсов - при подложке-снимке на Земле, как у модуля.
        let imagery = self.planet.earth && data::contours::is_imagery(self.basemap.key(), self.basemap.source().url, false);
        if imagery {
            let altitude = self.nav.camera.ellipsoid.ecef_to_geodetic(self.nav.camera.eye).2;
            let loaders = &self.loaders;
            self.polar.step(self.nav.pose.lat, altitude, now, &|limit| (loaders.request)(limit));
        }
        for north in [true, false] {
            let image = self.polar.images[usize::from(!north)].as_ref().filter(|_| imagery);
            if image.is_some() != globe.has_polar_image(north) {
                globe.set_polar_image(north, image.cloned());
            }
        }
        if let Some(fields) = &mut self.field_tiles {
            let id = data::fieldmap::key(fields.year);
            let keys: Vec<TileKey> = map_tiles.iter().filter(|(k, _, _)| *k == id).map(|(_, key, _)| *key).collect();
            for (key, rgba) in fields.step(&keys, now, &*self.loaders.request) {
                self.vector.add_map(id, key, rgba);
            }
        }
        for (id, loaders) in &self.map_loaders {
            for (n, (level, loader)) in loaders.iter().enumerate() {
                let next = loaders.get(n + 1).map_or(u32::MAX, |(l, _)| *l);
                let list: Vec<(TileKey, f64)> = map_tiles.iter().filter(|(k, key, _)| k == id && key.0 >= *level && key.0 < next).map(|(_, key, p)| (*key, *p)).collect();
                loader.want(&list);
            }
        }
        // Тайлы огней городов: предки тайлов кадра не глубже их уровня.
        if let Some(loader) = &self.lights_loader {
            let mut list: Vec<(TileKey, f64)> = Vec::new();
            for key in &selection.draw {
                let want = terrain::ancestor(*key, key.0.min(render::LIGHTS_LEVEL));
                if !globe.has_lights(want) && !list.iter().any(|(k, _)| *k == want) {
                    list.push((want, 50.0 - f64::from(want.0)));
                }
            }
            loader.want(&list);
        }
        // Линии векторной основы - не дольше VECTOR_BUDGET за кадр.
        self.vector.step(&selection.draw, VECTOR_BUDGET, globe);
        let lines_pending = !self.vector.ready(&selection.draw);
        // Маски воды - в том же бюджете кадра. Снимок --shot ждёт их,
        // как линий основы.
        let mut water_pending = water_on && !self.water_keys.is_empty();
        if water_on {
            let low = |k: TileKey| store.range_for(k).0 < 0.0;
            if self.water_tiles.step(&selection.draw, &low, VECTOR_BUDGET, globe) {
                water_pending = true;
                ctx.request_repaint();
            }
        }

        for key in std::mem::take(&mut self.pictures.fresh) {
            if let Some(image) = self.pictures.image(&key) {
                globe.set_picture_image(&key, image.width, image.height, &image.rgba);
            }
        }
        // Вода растров с батиметрией - вместе с сетками меток. Без рельефа
        // воды нет: дно и вода легли бы в одну плоскость, как у модуля.
        let scale = store.scale();
        let water_stale = self.water.as_ref().is_none_or(|(_, s)| *s != scale);
        if water_stale {
            let water = if scale > 0.0 { self.relief.water(&self.nav.camera.ellipsoid, scale) } else { Vec::new() };
            self.water = Some((water, scale));
        }
        let fresh = self.marks.meshes(&store, now);
        let geo_stale = self.geo.shapes_dirty;
        let tracks_stale = self.tracks.dirty || self.gps.dirty;
        let data_stale = std::mem::take(&mut self.data_layers.shapes_dirty);
        if fresh.is_some() || water_stale || geo_stale || tracks_stale || data_stale {
            // Выдавленные объекты «Моих данных» стоят на рельефе:
            // строятся заново и с новыми высотами.
            if data_stale || fresh.is_some() {
                let radius = self.nav.camera.ellipsoid.a;
                self.data_meshes = self.data_layers.shapes(radius).iter().filter_map(|(s, side)| crate::marks::mesh(s, &store).map(|m| render::FeatureMesh { side: *side, ..m })).collect();
            }
            if let Some(meshes) = &fresh {
                self.marks_meshes = meshes.clone();
            }
            // Линии геофизики и пути треков лежат на рельефе: строятся
            // заново и с новыми высотами.
            self.geo.shapes_dirty = false;
            self.geo_meshes = self.geo.shapes().iter().filter_map(|s| crate::marks::mesh(s, &store)).collect();
            if tracks_stale || fresh.is_some() {
                self.tracks.dirty = false;
                self.track_meshes = self.tracks.shapes().iter().filter_map(|s| crate::marks::mesh(s, &store)).collect();
                // Трек «Моего места» - той же линией на рельефе.
                self.gps.dirty = false;
                if self.gps.track.len() >= 2 {
                    let mut line = scene::features::Shape::new(scene::features::Kind::Line, self.gps.track.iter().map(|f| (f.lat, f.lon)).collect());
                    line.color = crate::gps::COLOUR;
                    line.width = 3.0;
                    self.track_meshes.extend(crate::marks::mesh(&line, &store));
                }
                self.track_dots_dirty = true;
            }
            let mut all = self.marks_meshes.clone();
            all.extend(self.water.iter().flat_map(|(w, _)| w.iter().cloned()));
            all.extend(self.geo_meshes.iter().cloned());
            all.extend(self.track_meshes.iter().cloned());
            all.extend(self.data_meshes.iter().cloned());
            globe.set_features(&all);
        }
        if listed && (fresh.is_some() || tracks_models || preview_dirty) && self.overlays_built != Some(self.marks.overlays_version()) {
            self.overlays_built = Some(self.marks.overlays_version());
            let mut meshes = self.marks.picture_meshes(&store);
            meshes.extend(self.data_layers.picture_sides(self.timebar.window()).into_iter().filter_map(|(o, side)| crate::pictures::ground_mesh(o, &crate::pictures::key_of(o), &store).map(|m| render::PictureMesh { side, ..m })));
            meshes.extend(self.preview.as_ref().and_then(|o| crate::pictures::ground_mesh(o, &crate::pictures::key_of(o), &store)));
            globe.set_pictures(&meshes);
            let built = self.models.meshes(&model_list, &store, now);
            globe.set_models(&built.meshes);
            self.export_models = built.meshes;
            self.movers = built.movers;
            self.animated = built.animated;
        }
        // Куски 3D Tiles по экранной ошибке, пока не прочитаны все нужные.
        if self.tilesets.update(globe, &self.nav.camera, &*self.loaders.request) {
            ui.ctx().request_repaint();
        }
        let window = self.timebar.window();
        for mover in &self.movers {
            globe.place_model(mover.index, mover.place(window, &store));
        }
        // Камера следом за моделью на пути: точка взгляда - модель,
        // расстояние, азимут и наклон прежние, `set_follow` модуля.
        if self.followed.is_some_and(|i| i >= self.movers.len()) {
            self.followed = None;
        }
        if let Some(m) = self.followed.and_then(|i| self.movers.get(i)) {
            let (lat, lon, _) = self.nav.camera.ellipsoid.ecef_to_geodetic(m.place(window, &store).center);
            let p = &self.nav.pose;
            let pose = Pose::new(lat, lon, p.distance, p.heading, p.tilt);
            self.nav.show(pose);
        }
        for model in &self.animated {
            globe.animate_model(model.index, model.deltas(now));
        }
        // Солнце на момент шкалы времени или сейчас, как у модуля.
        let sun = self.settings.sun.then(|| scene::sun::direction(sun_moment(window)));
        globe.set_model_sun(sun.map(|s| [s[0] as f32, s[1] as f32, s[2] as f32]));
        let recreated = globe.resize(width, height);
        // Вода над дном моря, под картой аномалий поля прозрачнее: синяя
        // вода делала красное молодое дно у хребтов фиолетовым.
        // В режиме геоида воды нет, высоты - геоид, а не дно.
        // Уровень моря окна «Уровень моря»: вода и без глубин морей, когда
        // уровень выше нынешнего.
        let sea_level = if self.planet.earth && self.geoid.is_none() { self.hydro.sea_level as f32 } else { 0.0 };
        let water = ((self.settings.sea || sea_level > 0.0) && self.geoid.is_none() && self.planet.earth).then(|| {
            // Под картой аномалий поля и температурой поверхности вода почти
            // прозрачна: у модуля их слои рисуются над водой, у Lite картинки
            // карт лежат под ней, и синий воды менял цвета шкалы моря.
            let under_field = if self.settings.maps.iter().any(|m| m == "emag" || m == crate::maps::TEMPERATURE) { WATER_UNDER_FIELD } else { 1.0 };
            [WATER_COLOR[0], WATER_COLOR[1], WATER_COLOR[2], WATER_COLOR[3] * under_field]
        });
        let count_holes = self.check.as_ref().is_some_and(Check::count_holes);
        if self.check.as_mut().is_some_and(Check::take_grab_request) {
            globe.grab();
        }
        // Тени в кадре у точки взгляда, карта теней стоит на рельефе.
        let pose = &self.nav.pose;
        let focus = self.nav.camera.ellipsoid.geodetic_to_ecef(pose.lat, pose.lon, store.height_at(pose.lat, pose.lon)).to_array();
        let focus = Some((focus, pose.distance));
        let stats =
            globe.render(&self.nav.camera, &selection, Some(&store), FrameOptions { count_holes, show_holes: false, air: self.settings.air && self.planet.air.is_some(), ratio: ppp as f32, sun, water, tint: self.planet.air, sky: false, sea_level, focus, relief: Some([self.settings.light_azimuth as f32, self.settings.shading as f32]) });
        drop(store);
        if let Some(check) = &mut self.check
            && check.waits_frame()
            && let Some(frame) = globe.take_grab()
        {
            let camera = &self.nav.camera;
            let (distance, a, fov) = (camera.eye.length(), camera.ellipsoid.a, camera.fov_y);
            check.frame(&frame, &mut self.nav, distance, a, fov, now);
        }
        globe.evict(&selection.keep, TEXTURE_BUDGET);
        self.show_target(recreated || self.texture.is_none());
        if let Some(id) = self.texture {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(id, rect, uv, egui::Color32::WHITE);
        }
        // Надписи только в кадре и не под подписью источников.
        // Картинки на экране - раньше надписей, надписи не ложатся под них.
        let painter = ui.painter().with_clip_rect(rect);
        self.draw_swipe(&painter, rect);
        let mut blocked = self.screen_overlays(&ctx, &painter, rect);
        blocked.extend(self.credits.rect);
        blocked.extend(self.toolbar_rect);
        let hiding = hides_windows(self.snapshot.as_ref());
        let pad = self.shows_ui() && !hiding && !self.touch;
        if pad {
            blocked.push(crate::navpad::rect(rect));
        }
        // В палеогеографии и на других телах подписей нет, как у модуля.
        if self.paleo.is_none() && self.planet.earth {
            self.labels.draw(&painter, rect, ppp as f32, &blocked, &self.nav.camera, &selection.draw, &self.heights.read());
        }
        // Приборы и строка полёта.
        if let Some(flight) = &self.flight {
            let ground = {
                let store = self.heights.read();
                let s = store.scale();
                if s != 0.0 { (store.height_at(flight.plane.lat, flight.plane.lon) / s).max(0.0) } else { 0.0 }
            };
            let focal = rect.height() / 2.0 / (self.nav.camera.fov_y.to_radians() as f32 / 2.0).tan();
            flight.hud(&painter, rect, self.lang, ground, focal);
            let lines = flight.lines(self.lang, ground);
            let font = egui::FontId::proportional(15.0);
            let galleys: Vec<_> = lines.iter().enumerate().map(|(n, l)| painter.layout_no_wrap(l.clone(), font.clone(), if n == 0 && lines.len() > 1 { egui::Color32::WHITE } else { egui::Color32::from_rgb(200, 210, 222) })).collect();
            let width = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max) + 20.0;
            let height = galleys.iter().map(|g| g.size().y).sum::<f32>() + 20.0;
            let back = egui::Rect::from_min_size(egui::pos2(rect.center().x - width / 2.0, rect.bottom() - 44.0 - height), egui::vec2(width, height));
            painter.rect_filled(back, 6.0, egui::Color32::from_rgba_unmultiplied(15, 25, 40, 175));
            let mut y = back.top() + 10.0;
            for g in galleys {
                let h = g.size().y;
                painter.galley(egui::pos2(back.center().x - g.size().x / 2.0, y), g, egui::Color32::WHITE);
                y += h;
            }
        }
        // Подписи живых объектов и статей у их точек, на обращённой к глазу
        // стороне, без наложения: ближние к точке взгляда первыми.
        let mut taken: Vec<egui::Rect> = blocked.clone();
        let plugged_labels = self.plugged.values().flat_map(|l| {
            let dot = l.dot();
            l.labels().iter().map(move |(text, p)| (text, p, dot))
        });
        for (text, p, dot) in self.live_labels.iter().chain(&self.track_labels).map(|(text, p)| (text, p, None)).chain(plugged_labels) {
            let (q, front) = self.nav.camera.project(*p);
            let facing = (*p - self.nav.camera.eye).dot(p.normalize_or_zero()) < 0.0;
            if !front || !facing {
                continue;
            }
            let at = rect.min + egui::vec2((q.x / ppp) as f32 + 6.0, (q.y / ppp) as f32 - 6.0);
            if !rect.contains(at) {
                continue;
            }
            let font = egui::FontId::proportional(11.0);
            let galley = painter.layout_no_wrap(text.clone(), font.clone(), egui::Color32::WHITE);
            let place = egui::Rect::from_min_size(at - egui::vec2(0.0, galley.size().y), galley.size()).expand(2.0);
            if taken.iter().any(|r| r.intersects(place)) {
                continue;
            }
            taken.push(place);
            // Точка у подписи слоя реестра, у статьи Википедии - как пункт
            // подписей модуля.
            if let Some(([r, g, b], size)) = dot {
                painter.circle(at - egui::vec2(6.0, -6.0), size / 2.0, egui::Color32::from_rgb(r, g, b), egui::Stroke::new(1.0, egui::Color32::WHITE));
            }
            painter.text(at + egui::vec2(1.0, 1.0), egui::Align2::LEFT_BOTTOM, text, font.clone(), egui::Color32::from_black_alpha(200));
            painter.text(at, egui::Align2::LEFT_BOTTOM, text, font, egui::Color32::WHITE);
        }
        // Рамки карт окна «Карты Рамсея» и снимков OpenAerialMap, выбранная -
        // ярче.
        for (boxes, selected) in self.rumsey.as_ref().map(crate::rumsey::Rumsey::footprints).into_iter().chain(self.oam.as_ref().map(crate::oam::Oam::footprints)) {
            let e = self.nav.camera.ellipsoid;
            let store = self.heights.read();
            let frame = |b: &data::rumsey::Bbox, stroke: egui::Stroke| {
                let [w, s, ea, n] = *b;
                let ring = [(s, w), (s, ea), (n, ea), (n, w), (s, w)];
                let mut points: Vec<Option<egui::Pos2>> = Vec::new();
                for pair in ring.windows(2) {
                    for k in 0..=8 {
                        let t = f64::from(k) / 8.0;
                        let (la, lo) = (pair[0].0 + (pair[1].0 - pair[0].0) * t, pair[0].1 + (pair[1].1 - pair[0].1) * t);
                        let p = e.geodetic_to_ecef(la, lo, store.height_at(la, lo));
                        let (q, front) = self.nav.camera.project(p);
                        let facing = (p - self.nav.camera.eye).dot(p.normalize_or_zero()) < 0.0;
                        points.push((front && facing).then(|| rect.min + egui::vec2((q.x / ppp) as f32, (q.y / ppp) as f32)));
                    }
                }
                for pair in points.windows(2) {
                    if let [Some(a), Some(b)] = pair {
                        painter.line_segment([*a, *b], stroke);
                    }
                }
            };
            for b in &boxes {
                frame(b, egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 200, 60, 140)));
            }
            if let Some(b) = selected {
                frame(&b, egui::Stroke::new(2.5, egui::Color32::from_rgb(255, 120, 30)));
            }
        }
        // Рисунки слоёв реестра поверх глобуса: струйки течений и ветра,
        // числа поля погоды.
        let view = (self.nav.pose.lat, self.nav.pose.lon, self.view_width());
        for layer in self.plugged.values() {
            layer.paint(&painter, rect, ppp, &self.nav.camera, view);
        }
        // Контуры участков кадастра под щелчком.
        if !self.parcels.is_empty() {
            let store = self.heights.read();
            let e = self.nav.camera.ellipsoid;
            let stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 230, 40));
            for ring in self.parcels.iter().flat_map(|p| &p.rings) {
                let points: Vec<Option<egui::Pos2>> = ring
                    .iter()
                    .chain(ring.first())
                    .map(|(la, lo)| {
                        let (q, front) = self.nav.camera.project(e.geodetic_to_ecef(*la, *lo, store.height_at(*la, *lo)));
                        front.then(|| rect.min + egui::vec2((q.x / ppp) as f32, (q.y / ppp) as f32))
                    })
                    .collect();
                for pair in points.windows(2) {
                    if let [Some(a), Some(b)] = pair {
                        painter.line_segment([*a, *b], stroke);
                    }
                }
            }
        }
        // Виток орбиты и след выбранного спутника, цвет группы.
        let moment = self.satellite_moment();
        if let Some(sats) = self.plugged.values_mut().find_map(|l| l.satellites_mut()) {
            let color = sats.selected.and_then(|n| sats.swarm.index_of(n)).and_then(|i| sats.swarm.group_of.get(i)).map_or([255; 3], |g| data::satellites::color(g));
            if let Some(path) = sats.path(moment) {
                let e = self.nav.camera.ellipsoid;
                let eye = self.nav.camera.eye;
                let orbit: Vec<DVec3> = path.lat.iter().zip(&path.lon).zip(&path.h).map(|((la, lo), h)| e.geodetic_to_ecef(*la, *lo, *h)).collect();
                let track: Vec<DVec3> = path.tlat.iter().zip(&path.tlon).map(|(la, lo)| e.geodetic_to_ecef(*la, *lo, crate::satellites::TRACK_LIFT)).collect();
                for (points, alpha) in [(orbit, 255u8), (track, crate::satellites::TRACK_ALPHA)] {
                    let raw: Vec<[f64; 3]> = points.iter().map(|p| p.to_array()).collect();
                    let hidden = data::satellites::hidden(eye.to_array(), &raw);
                    let stroke = egui::Stroke::new(crate::satellites::PATH_WIDTH, egui::Color32::from_rgba_unmultiplied(color[0], color[1], color[2], alpha));
                    for (k, pair) in points.windows(2).enumerate() {
                        if hidden.get(k).copied().unwrap_or(true) || hidden.get(k + 1).copied().unwrap_or(true) {
                            continue;
                        }
                        let (a, fa) = self.nav.camera.project(pair[0]);
                        let (b, fb) = self.nav.camera.project(pair[1]);
                        if fa && fb {
                            painter.line_segment([rect.min + egui::vec2((a.x / ppp) as f32, (a.y / ppp) as f32), rect.min + egui::vec2((b.x / ppp) as f32, (b.y / ppp) as f32)], stroke);
                        }
                    }
                }
            }
        }
        if self.settings.grid {
            crate::grid::paint(&painter, rect, ppp, &self.nav, self.lang);
        }
        if let Some(tool) = &self.tool {
            tool.paint(&painter, rect, ppp, &self.nav, self.lang);
        }
        if let Some(props) = &self.overlay_props {
            props.paint(&painter, rect, ppp, &self.nav, self.lang);
        }
        for calc in [&self.viewshed, &self.insolation, &self.shadows].into_iter().flatten() {
            if calc.open
                && let Some(site) = &calc.site
            {
                site.paint(&painter, rect, ppp, &self.nav);
            }
        }
        if let Some(calc) = self.relief_calc.as_ref().filter(|c| c.open) {
            calc.site.paint(&painter, rect, ppp, &self.nav);
        }
        for site in self.hydro.sites().into_iter().chain(self.earthwork.site()) {
            site.paint(&painter, rect, ppp, &self.nav);
        }
        if let Some(calc) = self.aef_change.as_ref().filter(|c| c.open) {
            calc.site.paint(&painter, rect, ppp, &self.nav);
        }
        for calc in [&self.aef_class, &self.similar, &self.tracing, &self.object_tasks, &self.recognize].into_iter().flatten() {
            calc.paint(&painter, rect, ppp, &self.nav);
        }
        if let Some(calc) = &self.spectral {
            calc.paint(&painter, rect, ppp, &self.nav);
        }
        if let Some(calc) = &self.emit {
            calc.paint(&painter, rect, ppp, &self.nav);
        }
        // Точка пути под курсором графика профиля, как у модуля.
        if let Some((lat, lon)) = self.profile.as_ref().and_then(|p| p.hovered) {
            let h = self.nav.pose.terrain.as_ref().map_or(0.0, |t| t(lat, lon));
            let (px, front) = self.nav.camera.project(self.nav.camera.ellipsoid.geodetic_to_ecef(lat, lon, h));
            if front {
                let at = rect.min + egui::vec2((px.x / ppp) as f32, (px.y / ppp) as f32);
                painter.circle(at, 5.0, egui::Color32::from_rgb(220, 60, 40), egui::Stroke::new(1.5, egui::Color32::WHITE));
            }
        }
        // Точка разреза модели под курсором графика, расстояние в метрах.
        if let Some((lat, lon, metres)) = self.model_section.as_ref().and_then(|s| s.hovered) {
            let h = self.nav.pose.terrain.as_ref().map_or(0.0, |t| t(lat, lon));
            let (px, front) = self.nav.camera.project(self.nav.camera.ellipsoid.geodetic_to_ecef(lat, lon, h));
            if front {
                let at = rect.min + egui::vec2((px.x / ppp) as f32, (px.y / ppp) as f32);
                painter.circle(at, 5.0, egui::Color32::from_rgb(220, 60, 40), egui::Stroke::new(1.5, egui::Color32::WHITE));
                let text = trf(self.lang, "{value} m", &[("value", &format!("{metres:.0}"))]);
                painter.text(at + egui::vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM, text, egui::FontId::proportional(13.0), egui::Color32::WHITE);
            }
        }
        // Точка разреза под курсором графика с расстоянием вдоль линии.
        if let Some((lat, lon, km)) = self.section.as_ref().and_then(|s| s.hovered) {
            let h = self.nav.pose.terrain.as_ref().map_or(0.0, |t| t(lat, lon));
            let (px, front) = self.nav.camera.project(self.nav.camera.ellipsoid.geodetic_to_ecef(lat, lon, h));
            if front {
                let at = rect.min + egui::vec2((px.x / ppp) as f32, (px.y / ppp) as f32);
                painter.circle(at, 5.0, egui::Color32::from_rgb(220, 60, 40), egui::Stroke::new(1.5, egui::Color32::WHITE));
                let text = trf(self.lang, "{value} km", &[("value", &format!("{km:.0}"))]);
                painter.text(at + egui::vec2(8.0, -8.0), egui::Align2::LEFT_BOTTOM, text, egui::FontId::proportional(13.0), egui::Color32::WHITE);
            }
        }
        if pad {
            self.navpad.paint(&painter, rect, &self.nav, self.settings.dark);
            self.navpad.tooltip(ui, rect, self.lang);
        }
        self.attribution(&ctx, rect);
        // Шкалы карт над подписью источников, крестик убирает карту.
        let maps: Vec<(&'static str, crate::maps::Kind)> = self.vector.map_list().to_vec();
        let theme = maps.iter().find_map(|(k, m)| match (m, self.theme_days.get(k)) {
            (crate::maps::Kind::Theme(_), Some(ThemeDay::Day(day, _))) => {
                let name = crate::maps::index(k).and_then(crate::maps::entry).map_or("", |e| tr(self.lang, e.name));
                let scale = match self.theme_scales.get(k) {
                    Some(ThemeScale::Ready(s)) => s.clone(),
                    _ => None,
                };
                let units = scale.as_ref().map_or("", |s| s.units.as_str());
                let title = if units.is_empty() {
                    trf(self.lang, "{name} · {day}", &[("name", name), ("day", day)])
                } else {
                    trf(self.lang, "{name}, {units} · {day}", &[("name", name), ("units", units), ("day", day)])
                };
                Some((*k, title, scale))
            }
            _ => None,
        });
        let dot_layers = self.dots.keys();
        // Шкала горизонталей: сечение в середине вида, цвет - по подложке.
        let contours = maps.iter().find_map(|(_, k)| match k {
            crate::maps::Kind::Contours(palette) => {
                let ppp = f64::from(ctx.pixels_per_point());
                let pose = &self.nav.pose;
                let step = data::contours::view_step(pose.distance, pose.lat, self.nav.camera.fov_y.to_radians(), f64::from(rect.height()) * ppp, globe_core::ellipsoid::EARTH.a, data::contours::LEVEL);
                let color = if *palette == data::contours::Palette::Imagery { [255, 214, 0] } else { data::contours::LAND };
                Some((step, color))
            }
            _ => None,
        });
        // Шкала слоя реестра - одна, первого слоя со шкалой.
        let plugged_legend = self.plugged.iter().find_map(|(n, l)| Some((*n, l.legend(self.lang)?)));
        let weather = plugged_legend.as_ref().map(|(_, legend)| legend.clone());
        let site = self.site_scale();
        match crate::legend::show(&ctx, self.lang, rect, self.credits.rect, crate::legend::Shown { maps: &maps, theme, dots: &dot_layers, contours, weather, site }) {
            Some(crate::legend::Closed::Site) => self.drop_site_owner(),
            Some(crate::legend::Closed::Map(key)) => {
                if let Some(n) = crate::maps::index(key) {
                    self.set_extra(crate::layers::Extra::Map(n), false);
                }
            }
            Some(crate::legend::Closed::Dots(layer)) => self.set_extra(crate::layers::Extra::Dots(layer), false),
            Some(crate::legend::Closed::Weather) => {
                if let Some((n, _)) = plugged_legend {
                    self.set_extra(crate::layers::Extra::Plugged(n), false);
                }
            }
            None => {}
        }
        self.request_snapshot(&ctx, rect);
        if !hiding {
            self.overlays(&ctx, rect, now);
        } else if self.film.is_some() {
            // Подпись остановки входит в кадр записи, как у модуля.
            self.stop_caption(&ctx, rect, now);
        }
        if !hiding && self.shows_ui() {
            self.toolbar(&ctx, rect, now);
            self.settings_window(&ctx);
            self.tool_window(&ctx);
            self.card_window(&ctx);
            self.profile_window(&ctx);
            self.section_window(&ctx);
            self.model_section_window(&ctx, now);
            self.name_window(&ctx);
            self.dialogs(&ctx, now);
            self.overlay_windows(&ctx);
            let mut out = Vec::new();
            let mut catalog = std::mem::take(&mut self.catalog);
            catalog.window(&ctx, self.lang, &self.layers_state(), &*self.loaders.request, &mut out);
            self.catalog = catalog;
            self.apply_layers(out, now);
        }

        if let Some(check) = &mut self.check {
            check.after_frame(&stats, &self.nav, &self.heights.read());
            check.note_labels(&self.labels.drawn, rect);
        }
        self.finish_check(&ctx);
        self.step_shot(&ctx, moving || loading || lines_pending || water_pending);
        if moving || animating || loading || lines_pending || self.labels.pending || self.check.is_some() {
            ctx.request_repaint();
        }
    }
}

/// Строка, которая живёт до конца программы: адрес и папка кэша темы на
/// день. Каждая разная строка заводится один раз.
fn lasting(cache: &mut HashMap<String, &'static str>, text: String) -> &'static str {
    if let Some(s) = cache.get(&text) {
        return s;
    }
    let s: &'static str = Box::leak(text.clone().into_boxed_str());
    cache.insert(text, s);
    s
}

/// Место слоя каталога на глобусе, на месте бывает один слой: темы GIBS
/// и пожары - одна карта витрины, карта значений или готовая карта -
/// одна карта слоя вида. None - слоёв сколько угодно.
fn extra_slot(extra: crate::layers::Extra) -> Option<u8> {
    match extra {
        crate::layers::Extra::Map(_) => extra.map().and_then(|e| crate::maps::kind(e.key)).and_then(|k| k.slot()),
        crate::layers::Extra::Dots(l) => l.slot(),
        crate::layers::Extra::Plugged(n) => crate::registry::get(n).and_then(|e| e.slot()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Snapshot, SnapshotTo, busiest, hides_windows, needs_models};

    /// Снимается кадр после запроса: панели, вернувшиеся на шаге ожидания,
    /// попадали в снимок и в кадры записи тура бледным следом.
    #[test]
    fn windows_stay_hidden_until_the_shot_arrives() {
        let cell = std::sync::Arc::new(std::sync::Mutex::new(None));
        assert!(hides_windows(Some(&Snapshot::Hide(SnapshotTo::File))));
        assert!(hides_windows(Some(&Snapshot::Wait(SnapshotTo::Frame, cell))));
        assert!(!hides_windows(None));
    }

    /// Кадр, где подошли высоты, собирает модели из настоящего списка:
    /// из пустого самосвалы демо исчезали с глобуса.
    #[test]
    fn models_are_listed_when_heights_arrive() {
        assert!(needs_models(Some(5), 5, true));
        assert!(needs_models(Some(4), 5, false));
        assert!(needs_models(None, 5, false));
        assert!(!needs_models(Some(5), 5, false));
    }

    /// Камера идёт за тем самосвалом, который едет, а не стоит на погрузке.
    #[test]
    fn busiest_truck_is_followed() {
        assert_eq!(busiest(&[0.0, 120.0, 30.0]), Some(1));
        assert_eq!(busiest(&[f64::NAN, 5.0]), Some(1));
        assert_eq!(busiest(&[]), None);
    }
}
