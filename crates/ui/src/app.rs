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
    /// Последний готовый день ряда.
    Day(String),
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
    /// Окно «Уклон и экспозиция» и его квадрат.
    relief_calc: Option<crate::reliefcalc::ReliefCalc>,
    /// Окна «Затопление», «Сток воды», «Уровень моря».
    hydro: crate::hydrocalc::Hydro,
    /// Тайлы высот, которых ждут расчёты у точки.
    calc_keys: Vec<TileKey>,
    /// Окно «Старые карты «Это Место»».
    etomesto: Option<crate::etomesto::EtomestoWindow>,
    /// Живые объекты по строкам.
    live: BTreeMap<crate::live::Kind, crate::live::Live>,
    /// Модели живых объектов в видеокарте: строка и ключ по порядку.
    live_keys: Vec<(crate::live::Kind, String)>,
    /// Точки и подписи живых объектов.
    live_points: Vec<render::PointsMesh>,
    live_labels: Vec<(String, DVec3)>,
    /// «Википедия рядом»: статьи, точки и подписи, время сборки.
    wiki: Option<crate::wiki::Wiki>,

    wiki_labels: Vec<(String, DVec3)>,
    wiki_built: f64,
    /// Спутники: группы, точки кадра, показанные спутники с местом.
    satellites: Option<crate::satellites::Satellites>,
    sat_points: Vec<render::PointsMesh>,
    sat_shown: Vec<(usize, DVec3)>,
    /// Загрузчики Overpass строк «Шахты и карьеры OSM» и «ЛЭП, кабели
    /// и трубопроводы OSM».
    osm_mines: Option<crate::overpass::Overpass<data::mining::Item>>,
    osm_infra: Option<crate::overpass::Overpass<data::infra::Item>>,
    overpass_mirrors: crate::overpass::Mirrors,
    /// Строка «Горные разработки».
    mining: Option<crate::mining::Mining>,
    /// Строка «3D-здания», тайлы зданий загрузчика OpenFreeMap и те из них,
    /// что нужны только зданиям.
    buildings: Option<crate::buildings::Buildings>,
    /// Загрузчик зданий Overture, когда здания из него.
    overture: Option<crate::overture::Overture>,
    building_keys: HashSet<TileKey>,
    building_only: HashSet<TileKey>,
    /// Сведения об участках кадастра под щелчком: точка и запрос, пришедшие
    /// участки для подсветки, `_ask_parcels` модуля.
    parcel_ask: Option<((f64, f64), Box<dyn net::Request>)>,
    parcels: Vec<data::cadastre::Parcel>,
    /// Строка «Течения».
    currents: Option<crate::currents::Currents>,
    /// Окно «Карты Рамсея».
    rumsey: Option<crate::rumsey::Rumsey>,
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
    /// Прочитанные браузером брошенные файлы: байты или ошибка и имя.
    dropped_box: DroppedBox,
    /// Снимок окна вместе с панелью.
    shot_ui: bool,
    /// Идущий тур.
    player: Option<Player>,
    /// Остановка тура прошлого кадра: на новой шкала встаёт на её время.
    tour_index: Option<usize>,
    timebar: TimeBar,
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
    /// Ширина кадра глобуса в пикселях, для линии шторки.
    globe_width_px: f32,
    /// Линию шторки тянут мышью.
    swipe_drag: bool,
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
    /// Сообщение со снимком, куда ответить, когда пришло и снимок уже
    /// просили.
    #[cfg(not(target_arch = "wasm32"))]
    mcp_shot: Option<(String, std::sync::mpsc::Sender<Option<String>>, f64, bool)>,
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
    /// Снимок просили, ждётся картинка.
    Wait(SnapshotTo, ShotCell),
}

/// Куда идёт снимок: файл PNG («Снимок вида») или буфер обмена
/// («Копия вида», `copy_view` модуля).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotTo {
    File,
    Clipboard,
    /// Ответ агенту MCP на `snapshot`.
    Mcp,
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
    Clear,
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

/// Часть rect снимка окна в точках, ppp - пикселей на точку.
fn crop(image: &egui::ColorImage, rect: egui::Rect, ppp: f32) -> Result<egui::ColorImage, String> {
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
fn to_png(image: &egui::ColorImage) -> Result<Vec<u8>, String> {
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
            relief_calc: None,
            hydro: crate::hydrocalc::Hydro::new(),
            calc_keys: Vec::new(),
            etomesto: None,
            paleo: None,
            planet: scene::planets::planet_by_key("earth"),
            earth_away: None,
            cog_base: None,
            geoid: None,
            live: BTreeMap::new(),
            live_keys: Vec::new(),
            live_points: Vec::new(),
            live_labels: Vec::new(),
            wiki: None,

            wiki_labels: Vec::new(),
            wiki_built: f64::NEG_INFINITY,
            satellites: None,
            sat_points: Vec::new(),
            sat_shown: Vec::new(),
            osm_mines: None,
            osm_infra: None,
            overpass_mirrors: crate::overpass::Mirrors::default(),
            mining: None,
            buildings: None,
            overture: None,
            building_keys: HashSet::new(),
            building_only: HashSet::new(),
            parcel_ask: None,
            parcels: Vec::new(),
            currents: None,
            rumsey: None,
            sentinel: None,
            live_objects: Vec::new(),
            live_follow: None,
            followed: None,
            flight: None,
            floor,
            open_dir: None,
            route_keys: HashSet::new(),
            route_only: HashSet::new(),
            route: crate::route::Router::new(data::routing::Mode::from_name(&settings.route).unwrap_or(data::routing::Mode::Car)),
            dropped_box: DroppedBox::default(),
            shot_ui: false,
            player: None,
            tour_index: None,
            timebar: TimeBar::default(),
            pictures: Pictures::default(),
            screen_textures: HashMap::new(),
            keep_pose: false,
            scene_source: None,
            fetching: None,
            demo_cache: None,
            demo_queue: std::collections::VecDeque::new(),
            demo_fetching: None,
            url_queue: std::collections::VecDeque::new(),
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
            globe_width_px: 0.0,
            swipe_drag: false,
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
            #[cfg(not(target_arch = "wasm32"))]
            mcp_shot: None,
            idle_frames: 0,
            explaining: None,
            ai_answer_from: None,
            ai_fallback: None,
            track_meshes: Vec::new(),
            track_dots_dirty: false,
            track_labels: Vec::new(),
            confirm: None,
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
        let current = if self.sky.is_some() { "sky" } else { self.planet.key() };
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

    /// Окно названия: «Сохранить вид», «Сохранить тур», «Сохранить
    /// измерение» модуля. Название по умолчанию с номером, объект ложится
    /// в выделенную папку или в папку выделенной метки.
    fn name_window(&mut self, ctx: &egui::Context) {
        let Some((what, name)) = &mut self.name_dialog else {
            return;
        };
        let lang = self.lang;
        let title = match what {
            NameFor::View => tr(lang, "Save view"),
            NameFor::Tour(_) => tr(lang, "Save tour"),
            NameFor::Measure(_) => tr(lang, "Save measurement"),
        };
        let mut open = true;
        let mut done: Option<bool> = None;
        egui::Window::new(title).open(&mut open).collapsible(false).resizable(false).pivot(egui::Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).show(ctx, |ui| {
            ui.label(tr(lang, "Name"));
            let edit = ui.text_edit_singleline(name);
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                done = Some(true);
            }
            ui.horizontal(|ui| {
                if ui.button(tr(lang, "OK")).clicked() {
                    done = Some(true);
                }
                if ui.button(tr(lang, "Cancel")).clicked() {
                    done = Some(false);
                }
            });
        });
        if !open {
            done = Some(false);
        }
        match done {
            Some(true) => {
                let Some((what, name)) = self.name_dialog.take() else {
                    return;
                };
                let folder = self.tree_state.selected.clone().unwrap_or_default();
                let name = name.trim().to_string();
                match what {
                    NameFor::View => {
                        let name = if name.is_empty() { scene::placetree::numbered_name(tr(lang, "View"), &self.marks.names()) } else { name };
                        self.add_view_place(&folder, &name);
                    }
                    NameFor::Tour(mut place) | NameFor::Measure(mut place) => {
                        if !name.is_empty() {
                            place.name = name;
                        }
                        let at = self.marks.insert(&folder, scene::kml::Item::Place(*place), marks::root_name(lang));
                        self.tree_state.selected = Some(at);
                        self.marks_changed();
                    }
                }
            }
            Some(false) => self.name_dialog = None,
            None => {}
        }
    }
    /// Метка вида камеры в папке по пути folder.
    fn add_view_place(&mut self, folder: &[usize], name: &str) {
        let pose = &self.nav.pose;
        let mut place = scene::kml::Place::new(name, scene::features::Kind::Point, vec![(pose.lat, pose.lon)]);
        place.view = scene::lookat::make(pose.lat, pose.lon, pose.distance, pose.heading, pose.tilt);
        let at = self.marks.insert(folder, scene::kml::Item::Place(place), marks::root_name(self.lang));
        self.tree_state.selected = Some(at);
        self.marks_changed();
    }

    /// Кнопка записи тура, как `_record_toggled` модуля: начать или
    /// закончить. Позы прореживаются (`scene::tour::thin`), тур ложится
    /// в выделенную папку «Моих меток». Камера не двигалась - тура нет.
    fn toggle_record(&mut self, now: f64) {
        let Some((_, samples)) = self.recording.take() else {
            self.recording = Some((now, Vec::new()));
            self.record_sample(now);
            return;
        };
        let samples = scene::tour::thin(&samples);
        if samples.len() < 2 || samples.last().is_none_or(|s| s[0] <= 0.0) {
            self.open_message = Some(tr(self.lang, "The tour is not recorded, the camera did not move.").to_string());
            return;
        }
        // Название тура спрашивает окно «Сохранить тур», как у модуля.
        let name = scene::placetree::numbered_name(tr(self.lang, "Tour"), &self.marks.names());
        let points = samples.iter().map(|s| (s[1], s[2])).collect();
        let mut place = scene::kml::Place::new(&name, scene::features::Kind::Line, points);
        place.tour = Some(samples);
        self.name_dialog = Some((NameFor::Tour(Box::new(place)), name));
        self.open_message = None;
    }

    /// Поза камеры в запись тура не чаще [`RECORD_PERIOD`].
    fn record_sample(&mut self, now: f64) {
        let Some((start, samples)) = &mut self.recording else {
            return;
        };
        let t = now - *start;
        if samples.last().is_some_and(|s| t - s[0] < RECORD_PERIOD) {
            return;
        }
        let p = &self.nav.pose;
        samples.push([t, p.lat, p.lon, p.distance, p.heading, p.tilt]);
        self.open_message = Some(trf(
            self.lang,
            "Recording a tour {clock}. Another click on the record button ends it.",
            &[("clock", &scene::tour::clock(t))],
        ));
    }

    /// Строка текстом KML в буфер обмена, как `copy_places` модуля: её
    /// вставляют в «Мои метки» или в Google Earth. Копия остаётся и для
    /// пункта меню «Вставить».
    fn copy_marks(&mut self, ctx: &egui::Context, path: &[usize]) {
        let text = self.marks.export_tree(path).and_then(|tree| crate::export::tree_bytes(&tree, false).ok());
        if let Some(text) = text.and_then(|b| String::from_utf8(b).ok()) {
            ctx.copy_text(text.clone());
            self.clip_kml = Some(text);
            self.open_message = Some(tr(self.lang, "Copied as KML.").to_string());
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
        let satellite = self.satellites.as_ref().and_then(|s| {
            let number = self.satellite_under(px, py, ppp).or(s.selected)?;
            let name = s.name_of(number).unwrap_or_else(|| number.to_string());
            Some((number, name, s.selected == Some(number), s.follow && s.selected == Some(number)))
        });
        let earth = self.planet.earth;
        let relief = earth || self.planet.terrain.is_some();
        crate::globemenu::GlobeMenu { at: pos, point, coords, place, drawing, mover, live, sheets, links, satellite, earth, relief }
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
                    && let Some(live) = self.live.get_mut(kind)
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
            Choice::Viewshed | Choice::Insolation => {
                let Some((lat, lon)) = point else {
                    return;
                };
                let name = place.as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
                let coords = format!("{lat:.5}, {lon:.5}");
                let title = if name.is_empty() { coords } else { format!("{name}  {coords}") };
                let (slot, kind) = if choice == Choice::Viewshed { (&mut self.viewshed, crate::pointcalc::Kind::Viewshed) } else { (&mut self.insolation, crate::pointcalc::Kind::Insolation) };
                match slot {
                    Some(c) => c.set_point(lat, lon, title),
                    None => {
                        let unix = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
                        *slot = Some(crate::pointcalc::Calc::new(kind, lat, lon, title, unix));
                    }
                }
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
            }
            Choice::Flood | Choice::Flow | Choice::SeaLevel => self.hydro_choice(choice, point, place),
            Choice::SatPath(number) => {
                if let Some(s) = &mut self.satellites {
                    s.select(number);
                }
            }
            Choice::SatFollow(number) => {
                if let Some(s) = &mut self.satellites {
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
                    // Рамка вида - по ширине и высоте вида у точки взгляда.
                    let width = self.view_width();
                    let height = width / self.nav.camera.aspect().max(0.1);
                    let (pl, plon) = (self.nav.pose.lat, self.nav.pose.lon);
                    let dlat = (height / 2.0 / 111_320.0).min(85.0);
                    let dlon = (width / 2.0 / (111_320.0 * pl.to_radians().cos().max(0.05))).min(180.0);
                    let view = [plon - dlon, (pl - dlat).max(-90.0), plon + dlon, (pl + dlat).min(90.0)];
                    match &mut self.rumsey {
                        Some(r) => r.show_view(lat, lon, view),
                        None => {
                            let loaders = &self.loaders;
                            self.rumsey = Some(crate::rumsey::Rumsey::new(lat, lon, view, &|limit: u64| (loaders.request)(limit)));
                        }
                    }
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
            globe.render(&camera, &globe_core::lod::Selection::default(), None, FrameOptions { count_holes: false, show_holes: false, air: false, ratio: ppp as f32, sun: None, water: None, tint: None, sky: true, sea_level: 0.0 });
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
        let hiding = matches!(self.snapshot, Some(Snapshot::Hide(_)));
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
        let paths = match (pixel, self.marks.tree.as_ref()) {
            (Some((px, py, ppp)), Some(tree)) => crate::card::pick_all(tree, &self.nav, px, py, crate::card::PICK_PIXELS * ppp),
            _ => Vec::new(),
        };
        let places: Vec<&scene::kml::Place> = paths.iter().filter_map(|p| self.marks.place(p)).collect();
        groups.extend(crate::identify::places_group(self.lang, &places));
        if let Some((px, py, ppp)) = pixel {
            groups.extend(self.dots.identify(self.lang, &self.nav, (lat, lon), (px, py), crate::card::PICK_PIXELS * ppp * 2.0, self.timebar.window()));
        }
        groups.extend(self.geo.identify(&self.dots, self.lang, lat, lon, self.magnetic_year().0));
        // Живые объекты у пикселя, `LiveManager.identify` модуля.
        if let Some((px, py, ppp)) = pixel {
            let store = self.heights.read();
            let e = self.nav.camera.ellipsoid;
            for (kind, live) in &self.live {
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
        // Контур горной разработки под точкой, `_identify_mining` модуля.
        if let Some(group) = self.mining.as_ref().and_then(|m| m.identify(self.lang, lat, lon)) {
            groups.push(group);
        }
        // Спутник у пикселя, `SatelliteManager.identify` модуля.
        if let (Some((px, py, ppp)), Some(sats)) = (pixel, &self.satellites)
            && let Some(number) = self.satellite_under(px, py, ppp)
            && let Some(index) = sats.swarm.index_of(number)
            && let Some(feature) = sats.values(self.lang, index, self.satellite_moment())
        {
            groups.push((tr(self.lang, "Satellites").to_string(), vec![feature]));
        }
        // Статьи Википедии у пикселя, _identify_wiki модуля.
        if let (Some((px, py, ppp)), Some(wiki)) = (pixel, &self.wiki) {
            let radius = crate::wiki::PICK_PIXELS * ppp;
            let mut near: Vec<(f64, &String)> = self
                .wiki_labels
                .iter()
                .filter_map(|(title, p)| {
                    let (q, front) = self.nav.camera.project(*p);
                    let gap = (q.x - px).hypot(q.y - py);
                    (front && gap <= radius).then_some((gap, title))
                })
                .collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            let features: Vec<crate::identify::Feature> = near
                .iter()
                .take(10)
                .map(|(_, title)| {
                    let mut fields = vec![(tr(self.lang, "Article").to_string(), data::wikipedia::page_url(wiki.lang(), title))];
                    if let Some(text) = wiki.summary_of(title).filter(|t| !t.is_empty()) {
                        fields.insert(0, (tr(self.lang, "Summary").to_string(), text.to_string()));
                    }
                    ((*title).clone(), fields)
                })
                .collect();
            if !features.is_empty() {
                groups.push((tr(self.lang, "Wikipedia").to_string(), features));
            }
        }
        groups.push(crate::identify::place_group(self.lang, height.filter(|_| self.settings.relief), address));
        if let Some((ask, answer)) = &self.map_ask
            && ask.at == (lat, lon)
        {
            groups.extend(crate::mapvalue::group(self.lang, ask.key, answer, ask.at));
        }
        crate::identify::Objects { point: self.point_text(lat, lon, height, 6), groups, at: (lat, lon) }
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

    /// Удалить всё из «Моих меток» после вопроса.
    fn clear_marks(&mut self) {
        self.tree_state.selected = None;
        self.marks.clear();
        self.shared.clear();
        self.player = None;
        self.open_message = None;
        self.marks_changed();
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
        // Пришёл адрес «Что здесь?» - строка «Адрес» в группе «Место».
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
        if let Some(objects) = &mut self.objects
            && !objects.window(ctx, lang)
        {
            self.objects = None;
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
                            self.tree_state.selected = None;
                            self.player = None;
                            self.marks_changed();
                        }
                    }
                    Confirmed::Clear => self.clear_marks(),
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

    /// Вставить метки из текста KML в выделенную папку или корень.
    /// Не KML - сообщение, «Мои метки» не меняются.
    fn paste_kml(&mut self, text: &str) {
        let folder = self.tree_state.selected.clone().unwrap_or_default();
        self.paste_kml_into(&folder, text);
    }

    /// Вставить метки из текста KML в папку folder.
    fn paste_kml_into(&mut self, folder: &[usize], text: &str) {
        let looks_kml = text.contains("<kml") || text.contains("<Placemark") || text.contains("<Folder") || text.contains("<Document");
        if !looks_kml {
            return;
        }
        match marks::open(text.as_bytes(), "clipboard.kml") {
            Ok(opened) => {
                let root = marks::root_name(self.lang);
                for item in opened.tree.children {
                    self.marks.insert(folder, item, root);
                }
                self.marks_changed();
                self.open_message = Some(tr(self.lang, "Pasted from the clipboard.").to_string());
            }
            Err(e) => self.open_message = Some(trf(self.lang, "Not pasted: {e}", &[("e", &e)])),
        }
    }

    /// Пришёл снимок вида - файлом PNG или в буфер обмена.
    /// Снимок вида: в кадре со скрытыми окнами - запрос картинки кадра,
    /// в следующих - запись готовой картинки.
    fn request_snapshot(&mut self, ctx: &egui::Context, rect: egui::Rect) {
        if let Some(Snapshot::Hide(to)) = self.snapshot {
            // Окна скрыты в этом кадре: снимок - глобус с подписью
            // источников.
            let cell = Arc::new(std::sync::Mutex::new(None));
            let sink = cell.clone();
            let ppp_points = ctx.pixels_per_point();
            ctx.request_screenshot(move |image| {
                if let Ok(mut slot) = sink.lock() {
                    *slot = Some(crop(&image, rect, ppp_points));
                }
            });
            self.snapshot = Some(Snapshot::Wait(to, cell));
        } else {
            self.save_snapshot(ctx);
        }
    }

    fn save_snapshot(&mut self, ctx: &egui::Context) {
        let Some(Snapshot::Wait(to, cell)) = &self.snapshot else {
            return;
        };
        let to = *to;
        let Some(result) = cell.lock().ok().and_then(|mut slot| slot.take()) else {
            return;
        };
        self.snapshot = None;
        if to == SnapshotTo::Mcp {
            let shot = result.ok().and_then(|image| {
                let png = to_png(&image).ok()?;
                // Большой снимок - уменьшенный, ответ агенту не раздувается.
                let img = image::load_from_memory(&png).ok()?;
                let img = if img.width().max(img.height()) > MCP_SHOT_SIDE { img.resize(MCP_SHOT_SIDE, MCP_SHOT_SIDE, image::imageops::FilterType::Triangle) } else { img };
                // JPEG: снимок снимков земли в PNG весит мегабайты.
                let mut out = Vec::new();
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, MCP_SHOT_QUALITY).encode_image(&img.to_rgb8()).ok()?;
                let p = &self.nav.pose;
                Some((out, format!("Точка взгляда {:.4}, {:.4}, расстояние {:.1} км, азимут {:.0}°, наклон {:.0}°.", p.lat, p.lon, p.distance / 1000.0, p.heading, p.tilt)))
            });
            #[cfg(not(target_arch = "wasm32"))]
            if let Some((message, reply, _, _)) = self.mcp_shot.take() {
                let server = assistant::mcp::Server::new(&crate::basemap::READY.map(|r| r.key));
                let _ = reply.send(server.handle(&mut ShotHost(shot), &message));
            }
            #[cfg(target_arch = "wasm32")]
            let _ = shot;
            return;
        }
        self.open_message = match (to, result) {
            (SnapshotTo::Clipboard, Ok(image)) => {
                ctx.copy_image(image);
                Some(tr(self.lang, "The view is copied to the clipboard.").to_string())
            }
            (SnapshotTo::File, Ok(image)) => match to_png(&image).and_then(|png| crate::export::save_bytes("planetx.png", ("PNG", "png"), &png)) {
                Ok(Some(path)) => Some(trf(self.lang, "Saved: {to}", &[("to", &path)])),
                Ok(None) => None,
                Err(e) => Some(trf(self.lang, "Not saved: {e}", &[("e", &e)])),
            },
            (_, Err(e)) => Some(trf(self.lang, "Not saved: {e}", &[("e", &e)])),
            (SnapshotTo::Mcp, Ok(_)) => None,
        };
    }

    /// Ретранслятор MCP: запуск и остановка по настройке, сообщения
    /// агентов - в этом кадре, по одному. Снимок вида ждёт кадра без
    /// окон, пока он не снят, другие сообщения ждут.
    #[cfg(not(target_arch = "wasm32"))]
    fn step_mcp(&mut self, ctx: &egui::Context, now: f64) {
        if !self.settings.ai.mcp && !self.mcp_cli {
            self.mcp = None;
            self.mcp_failed = false;
            return;
        }
        if self.mcp.is_none() && !self.mcp_failed {
            let port = crate::mcprelay::port();
            let wake_ctx = ctx.clone();
            match crate::mcprelay::Relay::start(port, crate::mcprelay::random_token(), crate::mcprelay::token_path(port), Arc::new(move || wake_ctx.request_repaint())) {
                Ok(relay) => self.mcp = Some(relay),
                Err(e) => {
                    self.mcp_failed = true;
                    self.open_message = Some(trf(self.lang, "MCP agents are not let in: {error}", &[("error", &e)]));
                }
            }
        }
        // Снимок - когда перелёт кончился и место загрузилось, как
        // `--shot`, но не позже [`MCP_SHOT_WAIT`].
        if let Some((_, _, since, false)) = &self.mcp_shot {
            if self.idle_frames >= SHOT_IDLE_FRAMES || now - since > MCP_SHOT_WAIT {
                if self.snapshot.is_none() {
                    self.snapshot = Some(Snapshot::Hide(SnapshotTo::Mcp));
                    if let Some(shot) = &mut self.mcp_shot {
                        shot.3 = true;
                    }
                }
            } else {
                ctx.request_repaint();
            }
        }
        while self.mcp_shot.is_none() {
            let Some(job) = self.mcp.as_ref().and_then(crate::mcprelay::Relay::take) else {
                break;
            };
            if assistant::mcp::is_snapshot(&job.message) {
                self.mcp_shot = Some((job.message, job.reply, now, false));
                self.idle_frames = 0;
                ctx.request_repaint();
                break;
            }
            let server = assistant::mcp::Server::new(&crate::basemap::READY.map(|r| r.key));
            let reply = server.handle(&mut McpHost { app: self, ctx, now }, &job.message);
            let _ = job.reply.send(reply);
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
            self.marks_to_labels();
            self.fonts_dirty = true;
        }
    }

    /// Флажок «Глубины морей и океанов» сменился: высоты заново из
    /// глубин, сетки заново, как `_set_sea_floor` модуля.
    fn sea_changed(&mut self) {
        let floor = self.earth_floor();
        let relief = &self.relief;
        self.heights.write().refloor(floor, &|tile| relief.patch(tile));
        if let Some(globe) = &mut self.globe {
            globe.stale_meshes(None);
        }
    }

    /// Действия раздела «Слои» и каталога «Добавить данные».
    fn apply_layers(&mut self, actions: Vec<crate::layers::Action>) {
        use crate::layers::Action;
        for action in actions {
            match action {
                Action::Basemap(b) => self.set_basemap(b),
                Action::Sea(on) => {
                    self.settings.sea = on;
                    self.settings_dirty = true;
                    self.sea_changed();
                }
                Action::AddSource => self.own_dialog = Some(OwnDialog::default()),
                Action::GibsCatalog => self.ask_gibs_catalog(),
                Action::Events => self.events.open(&*self.loaders.request),
                Action::RemoveOwn(b) => {
                    if let Some(own) = crate::basemap::own_of(b) {
                        self.settings.own.retain(|o| *o != own);
                        self.settings_dirty = true;
                    }
                    if self.basemap == b {
                        self.set_basemap(Basemap::Esri);
                    }
                }
                Action::GeoidScale(scale) => {
                    self.settings.geoid_scale = scale;
                    self.settings_dirty = true;
                    if let Some(g) = &mut self.geoid {
                        g.scale = scale;
                        self.heights.write().set_scale(scale);
                    }
                }
                Action::Stretch(on) => {
                    self.settings.stretch = on;
                    self.settings_dirty = true;
                    if let Some(c) = &mut self.cutaway {
                        c.set_stretch(on);
                    }
                }
                Action::Opacity(key, alpha) => {
                    self.map_opacity.insert(key.to_string(), alpha);
                    self.settings.opacity = self.map_opacity.iter().map(|(k, v)| (k.clone(), *v)).collect();
                    self.settings_dirty = true;
                    if self.vector.set_opacity(key, alpha)
                        && let Some(globe) = &mut self.globe
                    {
                        globe.clear_overlays();
                    }
                }
                Action::Ground(alpha) => {
                    if let Some(sub) = &mut self.subsurface {
                        sub.options.opacity = alpha;
                    }
                }
                Action::Subsurface => {
                    if let Some(sub) = &mut self.subsurface {
                        sub.window_open = true;
                    }
                }
                Action::Relief(on) => {
                    self.settings.relief = on;
                    self.heights.write().set_scale(if on { self.settings.scale } else { 0.0 });
                }
                Action::Extra(e, on) => {
                    self.extras.insert(e);
                    self.set_extra(e, on);
                }
                Action::RemoveExtra(e) => {
                    self.extras.remove(&e);
                    self.set_extra(e, false);
                }
                Action::Rows(rows) => self.set_vector_rows(rows),
                Action::RemoveGroup(n) => {
                    let mut rows = self.vector.rows.clone();
                    rows.retain(|r| !crate::vector::TREE[n].2.iter().any(|(k, _, _)| k == r));
                    self.vector.seen.remove(&n);
                    self.set_vector_rows(rows);
                }
                Action::Sources => self.credits.sources_open = true,
            Action::SatGroups(groups) => {
                self.settings.sat_groups = groups.clone();
                self.settings_dirty = true;
                if let Some(s) = &mut self.satellites {
                    s.set_groups(groups.into_iter().collect());
                }
            }
                Action::Catalog => self.catalog.open = !self.catalog.open,
                Action::Bathymetry(name) => {
                    self.close_side();
                    let level = self.shore_level(&name);
                    self.bathy = Some(crate::bathymetry::Dialog::new(&name, self.relief.lake(&name), level));
                }
            }
        }
    }

    /// Слой каталога включить или выключить.
    fn set_extra(&mut self, extra: crate::layers::Extra, on: bool) {
        use crate::layers::Extra;
        match extra {
            Extra::Air => self.settings.air = on,
            Extra::Stars => self.settings.stars = on,
            Extra::Sun => self.settings.sun = on,
            Extra::Grid => self.settings.grid = on,
            Extra::Map(_) => {}
            Extra::Live(kind) => {
                if on {
                    self.live.entry(kind).or_insert_with(|| crate::live::Live::new(kind));
                } else if self.live.remove(&kind).is_some() {
                    if self.live_follow.as_ref().is_some_and(|(k, _)| *k == kind) {
                        self.live_follow = None;
                    }
                    self.live_keys.clear();
                    if let Some(globe) = &mut self.globe {
                        globe.set_live_models(&[]);
                    }
                    self.live_points.clear();
                    self.live_labels.clear();
                    self.dots.dirty = true;
                }
            }
            Extra::Currents => {
                if on {
                    self.currents.get_or_insert_with(crate::currents::Currents::default);
                } else if self.currents.take().is_some() {
                    self.refresh_painters();
                }
            }
            // Здания основы и Overture - одно место: включение строки
            // снимает другую (`extra_slot`), разобранные тайлы прежнего
            // источника забываются.
            Extra::Buildings | Extra::Overture => {
                // Снятие строки, которая уже не на глобусе, ничего не трогает.
                let active = self.buildings.is_some() && self.overture.is_some() == (extra == Extra::Overture);
                if (on || active)
                    && let Some(mut b) = self.buildings.take()
                    && let Some(globe) = &mut self.globe
                {
                    b.clear(globe);
                }
                if on || active {
                    self.overture = None;
                }
                if on {
                    self.buildings = Some(crate::buildings::Buildings::default());
                    if extra == Extra::Overture {
                        self.overture = Some(crate::overture::Overture::new(&*self.loaders.request));
                    }
                }
            }
            Extra::Mining => {
                if on {
                    if self.mining.is_none() {
                        let path = self.data_dir().map(|d| d.join(data::mining::POLYGONS_FILE));
                        let loaders = &self.loaders;
                        let (m, message) = crate::mining::Mining::new(self.lang, path, &|limit: u64| (loaders.request)(limit));
                        self.mining = Some(m);
                        if message.is_some() {
                            self.open_message = message;
                        }
                    }
                } else if self.mining.take().is_some() {
                    self.refresh_painters();
                }
            }
            Extra::Satellites => {
                if on {
                    if self.satellites.is_none() {
                        self.satellites = Some(crate::satellites::Satellites::new(&self.settings.sat_groups, self.satellite_dir()));
                    }
                } else {
                    self.satellites = None;
                    self.sat_points.clear();
                    self.sat_shown.clear();
                    self.dots.dirty = true;
                }
            }
            Extra::Wiki => {
                if on {
                    self.wiki.get_or_insert_with(crate::wiki::Wiki::default);
                } else {
                    self.wiki = None;
                    self.wiki_labels.clear();
                }
            }
            Extra::Geoid => {
                if on && self.geoid.is_none() {
                    let source = net::Source { name: "geoid", url: data::geoid::TILE_URL, max_requests: 4, max_level: data::geoid::MAX_LEVEL, ext: "png", placeholders: false, raw: false, tilejson: None };
                    self.geoid = Some(GeoidState { loader: (self.loaders.make)(source), scale: self.settings.geoid_scale });
                    let mut store = self.heights.write();
                    store.clear();
                    store.max_level = data::geoid::MAX_LEVEL;
                    store.set_scale(self.settings.geoid_scale);
                } else if !on && self.geoid.take().is_some() {
                    let mut store = self.heights.write();
                    store.clear();
                    store.max_level = terrain::MAX_LEVEL;
                    store.set_scale(if self.settings.relief { self.settings.scale } else { 0.0 });
                }
                if let Some(globe) = &mut self.globe {
                    globe.stale_meshes(None);
                }
            }
            Extra::Paleo => {
                if on && self.paleo.is_none() {
                    let state = PaleoState { index: 0, playing: false, last: 0.0, before: self.basemap, rows: self.vector.rows.clone() };
                    self.paleo = Some(state);
                    self.set_vector_rows(std::collections::BTreeSet::new());
                    self.heights.write().set_scale(0.0);
                    self.set_basemap(Basemap::Paleo(data::paleo::AGES[0]));
                } else if !on && let Some(state) = self.paleo.take() {
                    self.set_basemap(state.before);
                    self.set_vector_rows(state.rows);
                    let scale = if self.settings.relief { self.settings.scale } else { 0.0 };
                    self.heights.write().set_scale(scale);
                }
            }
            Extra::Cutaway => {
                // Сектор под точкой взгляда в момент включения, как
                // `_set_cutaway` модуля.
                self.cutaway = on.then(|| crate::cutaway::Cutaway::new(self.nav.pose.lat, self.nav.pose.lon, self.settings.stretch, self.crust_cache().as_deref()));
                self.nav.camera.deep = on;
                if !on {
                    self.labels.wedge = None;
                }
                if let Some(globe) = &mut self.globe
                    && !on
                {
                    globe.set_wedge(None);
                    globe.set_subsurface("cutaway", None);
                    globe.set_subsurface("slabs", None);
                }
            }
            Extra::Dots(layer) => {
                if on {
                    let loaders = &self.loaders;
                    self.dots.enable(layer, &|| (loaders.request)(layer.bytes()));
                } else {
                    self.dots.disable(layer);
                }

                self.settings.dots = crate::dots::Layer::ALL.into_iter().filter(|l| if *l == layer { on } else { self.extras_on.contains(&Extra::Dots(*l)) }).map(|l| l.key().to_string()).collect();
            }
        }
        if on {
            // Карта значений или готовая карта на глобусе одна, как
            // у модуля: включение другой снимает прежнюю.
            if let Some(slot) = extra_slot(extra) {
                let others: Vec<Extra> = self.extras_on.iter().copied().filter(|e| *e != extra && extra_slot(*e) == Some(slot)).collect();
                for other in others {
                    self.set_extra(other, false);
                }
            }
            self.extras_on.insert(extra);
            self.extras.insert(extra);
        } else {
            self.extras_on.remove(&extra);
        }
        if matches!(extra, Extra::Map(_)) {
            self.sync_maps();
        }
        self.settings_dirty = true;
    }

    /// Попросить весь каталог NASA GIBS, один раз за запуск.
    fn ask_gibs_catalog(&mut self) {
        if self.gibs_catalog.is_none() && !crate::maps::has_catalog() {
            let request = (self.loaders.request)(GIBS_CATALOG_BYTES);
            request.start(data::gibs::CAPABILITIES);
            self.gibs_catalog = Some(request);
        }
    }

    /// Карты картинками на глобусе - по включённым строкам: снизу карта
    /// значений или готовая карта, над ней наложения. Загрузчики заводятся
    /// и убираются вместе со строками.
    fn sync_maps(&mut self) {
        let mut on: Vec<(&'static str, crate::maps::Kind)> = (0..crate::maps::count())
            .filter(|n| self.extras_on.contains(&crate::layers::Extra::Map(*n)))
            .filter_map(crate::maps::entry)
            .filter_map(|e| crate::maps::kind(e.key).map(|k| (e.key, k)))
            .collect();
        // Слои каталога GIBS, который ещё не пришёл, остаются в настройках.
        self.settings.maps = on.iter().map(|(id, _)| id.to_string()).chain(self.gibs_pending.iter().cloned()).collect();
        // Тема GIBS ложится, когда известен день ряда: до ответа ряда
        // дат картинки тайлов ждали бы её и не рисовались. У слоя всего
        // каталога ряд пришёл с каталогом.
        let now_utc = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
        for (id, kind) in &on {
            if let crate::maps::Kind::Theme(theme) = kind
                && !self.theme_days.contains_key(id)
            {
                let state = if theme.intervals.is_some() {
                    theme.catalog_day(now_utc).map_or(ThemeDay::Failed, ThemeDay::Day)
                } else {
                    let request = (self.loaders.request)(ANSWER_BYTES);
                    request.start(&theme.domains_url());
                    ThemeDay::Asking(request)
                };
                self.theme_days.insert(id, state);
            }
        }
        on.retain(|(id, kind)| !matches!(kind, crate::maps::Kind::Theme(_)) || matches!(self.theme_days.get(id), Some(ThemeDay::Day(_))));
        // Цвета горизонталей - по подложке: на снимке жёлтые, на карте
        // коричневые, как у модуля.
        let source = self.basemap.source();
        let palette = if data::contours::is_imagery(self.basemap.key(), source.url, false) { data::contours::Palette::Imagery } else { data::contours::Palette::Map };
        for (_, kind) in &mut on {
            if let crate::maps::Kind::Contours(p) = kind {
                *p = palette;
            }
        }
        on.sort_by_key(|(_, k)| !k.is_single());
        self.map_loaders.retain(|k, _| on.iter().any(|(id, _)| id == k));
        for (id, kind) in &on {
            if !self.map_loaders.contains_key(id) {
                let mut sources = kind.sources(id);
                if let (crate::maps::Kind::Theme(theme), Some(ThemeDay::Day(day))) = (kind, self.theme_days.get(id)) {
                    let url = lasting(&mut self.lasting, theme.url(day));
                    let name = lasting(&mut self.lasting, format!("gibs-{}-{day}", theme.key));
                    let max_level = theme.level;
                    let ext = if theme.ext == "jpeg" { "jpg" } else { "png" };
                    sources = vec![(0, net::Source { name, url, max_requests: 4, max_level, ext, placeholders: false, raw: false, tilejson: None })];
                    // Шкала темы - по её цветовой карте.
                    if !self.theme_scales.contains_key(id) {
                        let state = match theme.colormap_url() {
                            Some(url) => {
                                let request = (self.loaders.request)(ANSWER_BYTES);
                                request.start(&url);
                                ThemeScale::Asking(request)
                            }
                            None => ThemeScale::Ready(None),
                        };
                        self.theme_scales.insert(id, state);
                    }
                }
                let loaders = sources.into_iter().map(|(level, source)| (level, (self.loaders.make)(source))).collect();
                self.map_loaders.insert(id, loaders);
            }
        }
        // Непрозрачность карт из настроек.
        for (key, alpha) in &self.map_opacity {
            if let Some(entry) = crate::maps::index(key).and_then(crate::maps::entry) {
                self.vector.set_opacity(entry.key, *alpha);
            }
        }
        if self.vector.set_maps(on)
            && let Some(globe) = &mut self.globe
        {
            globe.clear_overlays();
        }
    }

    /// Новые строки векторной основы: линии рисуются заново, подписи - по
    /// новым классам.
    fn set_vector_rows(&mut self, rows: std::collections::BTreeSet<crate::vector::Row>) {
        if self.vector.set_rows(rows)
            && let Some(globe) = &mut self.globe
        {
            globe.clear_overlays();
        }
        self.labels.kinds = self.vector.label_kinds();
        self.settings.vector = self.vector.to_text();
        self.settings_dirty = true;
    }

    /// Состояние для раздела «Слои» и каталога.
    fn layers_state(&self) -> crate::layers::State<'_> {
        crate::layers::State {
            basemap: self.basemap,
            relief: self.settings.relief,
            extras: &self.extras,
            on: &self.extras_on,
            vector: &self.vector,
            own: self.relief.names().into_iter().map(str::to_string).collect(),
            sea: self.settings.sea,
            ground: self.subsurface.as_ref().map(|s| s.options.opacity),
            opacity: &self.map_opacity,
            stretch: self.settings.stretch,
            geoid_scale: self.settings.geoid_scale,
            sat_groups: &self.settings.sat_groups,
            earth: self.planet.earth,
            events: self.events.shown.is_some(),
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

    /// Перелёт к найденному месту и метка на нём.
    fn fly_to_found(&mut self, place: &data::geocode::Place, now: f64) {
        let camera = &self.nav.camera;
        let pose = place_pose(place, camera.fov_y, camera.aspect());
        crate::input::fly(&mut self.nav, pose, now);
        self.labels.set_search(Some((&place.name, place.lat, place.lon)));
        if let Some(check) = &mut self.check {
            check.note_search(place.lat, place.lon);
        }
    }

    /// Открыть сцену `.planetx`, KML или KMZ при старте: камера сразу
    /// встаёт в позу сцены.
    pub fn with_file(mut self, bytes: Vec<u8>, name: &str) -> Self {
        self.pending_open.push((bytes, name.to_string()));
        self
    }

    /// Откуда самопроверка `scenes` берёт сцены демо: в программе -
    /// папка, на странице - начало адреса, к нему добавляется
    /// «имя.planetx».
    pub fn with_scenes(mut self, source: &str) -> Self {
        self.scene_source = Some(source.to_string());
        self
    }

    /// Пускать агентов MCP в этом запуске, `planetx-lite --mcp`, без
    /// записи в настройки.
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    pub fn with_mcp(mut self) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.mcp_cli = true;
        }
        self
    }

    /// Папка кэша файлов демо.
    pub fn with_demo_cache(mut self, dir: std::path::PathBuf) -> Self {
        self.demo_cache = Some(dir);
        self
    }

    /// Открыть демо: файлы по порядку, из кэша или по ссылке.
    fn open_demo(&mut self, demo: &crate::demos::Demo, now: f64) {
        // Одна загрузка за раз: ответ прежней пришёл бы новому демо.
        if self.fetching.is_some() {
            return;
        }
        self.demo_queue = demo.files.iter().copied().collect();
        self.demo_fetching = None;
        self.haul_pending = crate::demos::is_haul(demo);
        self.next_demo_file(now);
    }

    /// Открыть следующие файлы демо из кэша, первый без кэша - начать
    /// грузить.
    fn next_demo_file(&mut self, now: f64) {
        while let Some(path) = self.demo_queue.pop_front() {
            let name = crate::demos::name(path).to_string();
            self.remember_url(&crate::demos::url(path));
            if let Some(bytes) = self.cached_demo(path) {
                // Ошибку открытия показывает панель.
                let _ = self.open_file(&bytes, &name, Some(now));
                continue;
            }
            self.loaders.files.start(&crate::demos::url(path));
            self.open_message = Some(trf(self.lang, "Loading the demo: {name}…", &[("name", &name)]));
            self.fetching = Some((name, false));
            self.demo_fetching = Some(path);
            return;
        }
    }

    /// Файл демо из кэша программы.
    #[cfg(not(target_arch = "wasm32"))]
    fn cached_demo(&self, path: &str) -> Option<Vec<u8>> {
        crate::demos::cached(self.demo_cache.as_deref()?, path)
    }

    /// У страницы своего кэша нет, файлы хранит кэш браузера.
    #[cfg(target_arch = "wasm32")]
    fn cached_demo(&self, _path: &str) -> Option<Vec<u8>> {
        None
    }

    /// Открыть сцену по ссылке, когда она загрузится. Страница так
    /// открывает `?scene=адрес`. Сцены из нескольких вызовов
    /// открываются по порядку.
    pub fn with_scene_url(mut self, url: &str) -> Self {
        self.url_queue.push_back(url.to_string());
        if self.fetching.is_none() {
            self.next_scene_url();
        }
        self
    }

    /// Начать грузить следующую сцену из ссылки.
    fn next_scene_url(&mut self) {
        let Some(url) = self.url_queue.pop_front() else {
            return;
        };
        let name = url.rsplit('/').next().unwrap_or(&url).split(['?', '#']).next().unwrap_or("").to_string();
        self.loaders.files.start(&url);
        self.remember_url(&url);
        self.fetching = Some((name, false));
    }

    /// Рейс самосвалов демо, как `_play_model_trucks` и `_haul_loop`
    /// модуля: шкала-момент в начале рейса, ×[`HAUL_RATE`], показ идёт.
    /// Момент у конца рейса или до его начала - снова начало. Пути
    /// сменились - демо снято, рейс больше не ведётся.
    fn step_haul(&mut self, now: f64) {
        let span = crate::models::span(&self.movers);
        if self.haul_pending && let Some(span) = span {
            self.haul_pending = false;
            self.haul = Some(span);
            self.timebar.open = true;
            self.timebar.rate = Some(HAUL_RATE);
            self.timebar.set_point(true, Some(span.0));
            if !self.timebar.playing() {
                self.timebar.toggle(now);
            }
            return;
        }
        let Some(haul) = self.haul else {
            return;
        };
        if span.map(|s| s.0) != Some(haul.0) {
            self.haul = None;
            return;
        }
        let moment = self.timebar.range.1;
        if moment >= haul.1 - HAUL_END || moment < haul.0 {
            self.timebar.set_point(true, Some(haul.0));
            if !self.timebar.playing() {
                self.timebar.toggle(now);
            }
        }
    }

    /// Адрес открытого файла - в ссылку «Поделиться», без повторов.
    fn remember_url(&mut self, url: &str) {
        if !self.shared.iter().any(|u| u == url) {
            self.shared.push(url.to_string());
        }
    }

    /// Ссылка на вид: адреса открытых сцен и поза камеры. На странице -
    /// её собственный адрес, у программы - страница на GitHub Pages.
    fn share_link(&self) -> String {
        crate::share::link(&crate::share::page_address(), &self.shared, &self.nav.pose)
    }

    /// Поделиться ссылкой на вид. На телефоне - окно «Поделиться»
    /// системы, иначе ссылка в буфер обмена.
    fn share(&mut self, ctx: &egui::Context) {
        let link = self.share_link();
        if !crate::share::system_share(&link) {
            ctx.copy_text(link);
        }
        let mut message = tr(self.lang, "The link to this view is copied.").to_string();
        if self.shared.is_empty() && self.marks.tree.as_ref().is_some_and(|t| !t.children.is_empty()) {
            message.push(' ');
            message.push_str(tr(self.lang, "Files opened from the disk are not in the link."));
        }
        self.open_message = Some(message);
    }

    /// Сцена самопроверки по имени: из папки сразу, по ссылке - позже.
    fn load_check_scene(&mut self, name: &str, now: f64) {
        let file = format!("{name}.planetx");
        let Some(source) = self.scene_source.clone() else {
            if let Some(check) = &mut self.check {
                check.scene_opened(Err("не задано, откуда брать сцены демо".into()), now);
            }
            return;
        };
        if source.starts_with("http://") || source.starts_with("https://") || cfg!(target_arch = "wasm32") {
            self.loaders.files.start(&format!("{source}{file}"));
            self.fetching = Some((file, true));
        } else {
            self.read_check_scene(&source, &file, now);
        }
    }

    /// Сцена самопроверки из папки.
    #[cfg(not(target_arch = "wasm32"))]
    fn read_check_scene(&mut self, dir: &str, file: &str, now: f64) {
        match std::fs::read(std::path::Path::new(dir).join(file)) {
            Ok(bytes) => self.open_check_scene(&bytes, file, now),
            Err(e) => {
                if let Some(check) = &mut self.check {
                    check.scene_opened(Err(format!("{file}: {e}")), now);
                }
            }
        }
    }

    /// На странице папок нет, сцены приходят по ссылке.
    #[cfg(target_arch = "wasm32")]
    fn read_check_scene(&mut self, _dir: &str, _file: &str, _now: f64) {}

    /// Ответ на загрузку сцены по ссылке.
    fn step_fetch(&mut self, now: f64) {
        if self.fetching.is_none() {
            return;
        }
        let Some(answer) = self.loaders.files.take() else {
            return;
        };
        let Some((name, for_check)) = self.fetching.take() else {
            return;
        };
        // Пришёл шрифт с иероглифами: он ставится в следующем кадре.
        if let Some(code) = self.font_fetching.take() {
            match answer {
                Ok(bytes) if crate::fonts::is_font(&bytes) => {
                    self.web_fonts.insert(code, bytes);
                    self.fonts_dirty = true;
                    self.open_message = None;
                }
                Ok(_) => self.open_message = Some(format!("{name}: не шрифт")),
                Err(e) => self.open_message = Some(format!("{name}: {e}")),
            }
            return;
        }
        match (answer, for_check) {
            (Ok(bytes), true) => self.open_check_scene(&bytes, &name, now),
            (Ok(bytes), false) => match self.demo_fetching.take() {
                Some(path) => {
                    #[cfg(not(target_arch = "wasm32"))]
                    if let Some(dir) = &self.demo_cache {
                        // Кэш не записался - демо всё равно открывается.
                        let _ = crate::demos::store(dir, path, &bytes);
                    }
                    let _ = path;
                    let _ = self.open_file(&bytes, &name, Some(now));
                    self.next_demo_file(now);
                }
                None => {
                    let _ = self.open_file(&bytes, &name, None);
                    self.next_scene_url();
                }
            },
            (Err(e), true) => {
                if let Some(check) = &mut self.check {
                    check.scene_opened(Err(format!("{name}: {e}")), now);
                }
            }
            (Err(e), false) => {
                self.demo_fetching = None;
                self.demo_queue.clear();
                self.url_queue.clear();
                self.open_message = Some(format!("{name}: {e}"));
            }
        }
    }

    /// Открыть сцену самопроверки и отдать итог проверке: строки
    /// дерева, поза, количество туров.
    fn open_check_scene(&mut self, bytes: &[u8], name: &str, now: f64) {
        // Самопроверка сверяет одну сцену: «Мои метки» перед ней пусты.
        self.marks.clear();
        let opened = self.open_file(bytes, name, None).map(|pose| {
            let p = &self.nav.pose;
            let pose_ok = pose.is_some_and(|s| {
                (p.lat - s.lat).abs() < check::POSE_TOLERANCE
                    && (p.lon - s.lon).abs() < check::POSE_TOLERANCE
                    && (p.distance / s.distance - 1.0).abs() < check::POSE_TOLERANCE
                    && (p.heading - s.heading).abs() < check::POSE_TOLERANCE
                    && (p.tilt - s.tilt).abs() < check::POSE_TOLERANCE
            });
            let camera = &self.nav.camera;
            let (counts, tours) = self.marks.last().map_or(([0; 4], 0), |t| {
                (crate::tours::counts(t), crate::tours::scene_tours(t, camera.fov_y, camera.aspect()).len())
            });
            (counts, pose_ok, tours)
        });
        if let Some(check) = &mut self.check {
            check.scene_opened(opened, now);
        }
    }

    /// Тур сцены самопроверки по номеру.
    fn start_check_tour(&mut self, index: usize, now: f64) {
        let camera = &self.nav.camera;
        let tour = self
            .marks
            .last()
            .and_then(|t| crate::tours::scene_tours(t, camera.fov_y, camera.aspect()).into_iter().nth(index));
        self.player = tour.and_then(|(name, stops)| Player::new(&name, &self.nav.pose, stops, camera.fov_y, now));
        self.tour_index = None;
        self.nav.stop();
        // Длинный тур проигрывается целиком, но ускоренно.
        let started = self.player.as_mut().map(|p| {
            p.rate = (p.tour.duration / check::TOUR_LIMIT).max(1.0);
            (p.tour.duration / p.rate, p.rate)
        });
        if let Some(check) = &mut self.check {
            check.tour_started(started);
        }
    }

    /// Облако точек LAS, LAZ или COPC, камера - к нему.
    fn open_cloud(&mut self, bytes: &[u8], name: &str, fly: Option<f64>) -> Result<Option<Pose>, String> {
        let cloud = match data::cloud::read(bytes.to_vec(), name, CLOUD_POINTS) {
            Ok(c) if !c.points.is_empty() => c,
            Ok(_) => Err("в облаке нет точек".to_string())?,
            Err(e) => {
                self.open_message =
                    Some(trf(self.lang, "The point cloud did not open: {name}: {e}", &[("name", name), ("e", &e)]));
                return Err(e);
            }
        };
        Ok(self.show_cloud(&cloud, name, fly))
    }

    /// Пункты подменю «Гидрология» меню на глобусе.
    fn hydro_choice(&mut self, choice: crate::globemenu::Choice, point: Option<(f64, f64)>, place: Option<(Vec<usize>, String)>) {
        use crate::globemenu::Choice;
        if choice == Choice::SeaLevel {
            self.hydro.sea_open = true;
            return;
        }
        let Some((lat, lon)) = point else {
            return;
        };
        let name = place.as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
        let coords = format!("{lat:.5}, {lon:.5}");
        let title = if name.is_empty() { coords } else { format!("{name}  {coords}") };
        if choice == Choice::Flood {
            self.hydro.flood_here(lat, lon, title);
        } else {
            self.hydro.flow_here(lat, lon, title);
        }
    }

    /// Шаг снимков событий в каждом кадре, с окнами и без: каталоги,
    /// обход события, событие ключа `--event`.
    fn step_events(&mut self, now: f64) {
        let lang = self.lang;
        let mut actions = self.events.step(lang, &*self.loaders.request);
        if let Some(key) = self.pending_event.clone()
            && !self.events.events.is_empty()
            && !self.events.busy()
        {
            self.pending_event = None;
            if !self.events.show_key(lang, &key, &mut actions) {
                self.open_message = Some(trf(lang, "There is no event {key} in the catalogs.", &[("key", &key)]));
            }
        }
        self.event_actions(actions, now);
    }

    /// Просьбы снимков событий: перелёт к событию, смена снимков стороны.
    fn event_actions(&mut self, actions: Vec<crate::events::Action>, now: f64) {
        for action in actions {
            match action {
                crate::events::Action::Fly([w, s, e, n]) => {
                    let camera = &self.nav.camera;
                    let (lat, lon, distance) = globe_core::flight::fit_view(w, s, e, n, camera.fov_y, camera.aspect());
                    if lat.is_finite() && lon.is_finite() && distance.is_finite() {
                        crate::input::fly(&mut self.nav, Pose::new(lat, lon, distance, 0.0, 0.0), now);
                    }
                }
                crate::events::Action::Changed(side) => {
                    if let Some(globe) = &mut self.globe {
                        globe.clear_events(side);
                    }
                    if let Some(empty) = self.event_empty.get_mut(side) {
                        empty.clear();
                    }
                }
            }
        }
    }

    /// Линия шторки снимков события на виде rect: рисуется поверх глобуса,
    /// тянется мышью. Истина - мышь у линии, глобус её не получает.
    fn swipe_input(&mut self, ctx: &egui::Context, rect: egui::Rect) -> bool {
        let Some(share) = self.events.swipe() else {
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
            self.events.split = ((p.x - rect.left()) / rect.width().max(1.0)).clamp(0.02, 0.98);
        }
        if near || self.swipe_drag {
            ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        near || self.swipe_drag
    }

    /// Линия шторки и ручка с днями сторон поверх кадра глобуса.
    fn draw_swipe(&self, painter: &egui::Painter, rect: egui::Rect) {
        if self.events.swipe().is_none() {
            return;
        }
        let x = rect.left() + self.events.split * rect.width();
        let p = crate::theme::pal();
        painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], egui::Stroke::new(2.0, egui::Color32::WHITE));
        let mid = egui::pos2(x, rect.center().y);
        painter.circle(mid, 11.0, p.plate, egui::Stroke::new(2.0, egui::Color32::WHITE));
        painter.text(mid, egui::Align2::CENTER_CENTER, "⇆", egui::FontId::proportional(14.0), p.plate_text);
        if let Some(s) = &self.events.shown {
            let font = egui::FontId::proportional(13.0);
            let shadow = egui::Color32::from_black_alpha(160);
            for (text, anchor, at) in [(s.before.clone().unwrap_or_default(), egui::Align2::RIGHT_TOP, egui::pos2(x - 8.0, rect.top() + 8.0)), (s.day.clone(), egui::Align2::LEFT_TOP, egui::pos2(x + 8.0, rect.top() + 8.0))] {
                painter.text(at + egui::vec2(1.0, 1.0), anchor, &text, font.clone(), shadow);
                painter.text(at, anchor, &text, font.clone(), egui::Color32::WHITE);
            }
        }
    }

    /// Шаг облаков лидара по сети: пришедшие облака - на глобус.
    fn step_lidar(&mut self, now: f64) {
        let Some(lidar) = &mut self.lidar else {
            return;
        };
        let arrived = lidar.step(now, &*self.loaders.request);
        let done = !lidar.busy();
        let mut first = true;
        for a in arrived {
            match a {
                crate::lidarnet::Arrived::Cloud(tile, cloud) if !cloud.points.is_empty() => {
                    let name = format!("{} {}", tile.name, tile.date).trim().to_string();
                    let credit = data::lidarnet::credit(tile.source);
                    if !self.lidar_credits.contains(&credit) {
                        self.lidar_credits.push(credit);
                    }
                    // Камера летит к первому облаку.
                    self.show_cloud(&cloud, &name, first.then_some(now));
                    first = false;
                }
                crate::lidarnet::Arrived::Cloud(tile, _) => self.open_message = Some(trf(self.lang, "The point cloud did not open: {name}: {e}", &[("name", &tile.name), ("e", "0")])),
                crate::lidarnet::Arrived::Message(text) => self.open_message = Some(trf(self.lang, "Lidar point clouds: {error}", &[("error", &text)])),
                crate::lidarnet::Arrived::Nothing(errors) => {
                    self.open_message = Some(if errors.is_empty() { tr(self.lang, "There are no lidar tiles of IGN LiDAR HD and USGS 3DEP near the point.").to_string() } else { trf(self.lang, "Lidar point clouds: {error}", &[("error", &errors)]) });
                }
            }
        }
        if done {
            self.lidar = None;
        }
    }

    /// Облако на глобус, камера - к нему, если fly или поза не задана.
    fn show_cloud(&mut self, cloud: &data::cloud::Cloud, name: &str, fly: Option<f64>) -> Option<Pose> {
        let e = Ellipsoid::wgs84();
        let xyz: Vec<DVec3> = cloud.points.iter().map(|p| e.geodetic_to_ecef(p[0], p[1], p[2])).collect();
        let (center, offsets) = scene::features::centered(&xyz);
        self.clouds.push(render::PointsMesh { center, offsets, colors: cloud.colors.clone(), size: CLOUD_POINT_SIZE, ..Default::default() });
        if let Some(globe) = &mut self.globe {
            globe.set_points(&self.clouds);
            self.dots.dirty = true;
        }
        // Охват облака - углы его рамки.
        let (mut lat0, mut lon0, mut lat1, mut lon1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in &cloud.points {
            (lat0, lon0, lat1, lon1) = (lat0.min(p[0]), lon0.min(p[1]), lat1.max(p[0]), lon1.max(p[1]));
        }
        let camera = &self.nav.camera;
        let pose = marks::fit_points(&[(lat0, lon0), (lat1, lon1)], camera.fov_y, camera.aspect()).map(|mut p| {
            // Облако видно сбоку: наклон, как у перелёта к пути.
            p.tilt = 55.0;
            p
        });
        if let Some(pose) = pose.clone().filter(|_| fly.is_some() || !self.keep_pose) {
            match fly {
                Some(now) => crate::input::fly(&mut self.nav, pose, now),
                None => self.nav.show(pose),
            }
        }
        self.open_message = Some(trf(
            self.lang,
            "Point cloud {name}: {shown} of {total} points",
            &[("name", name), ("shown", &cloud.points.len().to_string()), ("total", &cloud.total.to_string())],
        ));
        pose
    }

    /// Растр высот GeoTIFF - врезка своего рельефа, камера - к нему.
    fn open_relief(&mut self, bytes: &[u8], name: &str, fly: Option<f64>) -> Result<Option<Pose>, String> {
        let opened = data::grid::Grid::read(bytes.to_vec(), name)
            .and_then(|grid| data::dem::Dem::new(grid, terrain::inset::BAND_SHARE));
        let dem = match opened {
            Ok(dem) => dem,
            Err(e) => {
                self.open_message = Some(trf(self.lang, "The raster did not open: {name}: {e}", &[("name", name), ("e", &e)]));
                return Err(e);
            }
        };
        let corners = dem.grid().corners();
        {
            let mut store = self.heights.write();
            self.relief.add(name, dem, bytes.to_vec(), &mut store);
            // Дно озера демо - сразу батиметрия, как у модуля.
            if let Some(lake) = crate::demos::lake(name) {
                let _ = self.relief.set_lake(name, Some(lake), &mut store);
                self.water = None;
            }
        }
        let camera = &self.nav.camera;
        let pose = corners.and_then(|c| marks::fit_points(&c, camera.fov_y, camera.aspect()));
        if let Some(pose) = pose.clone().filter(|_| fly.is_some() || !self.keep_pose) {
            match fly {
                Some(now) => crate::input::fly(&mut self.nav, pose, now),
                None => self.nav.show(pose),
            }
        }
        self.open_message = Some(trf(self.lang, "Own terrain: {names}", &[("names", &self.relief.names().join(", "))]));
        Ok(pose)
    }

    /// Открыть файл. fly - лететь к позе сцены, иначе встать в неё сразу.
    /// Возвращает позу сцены или ошибку.
    fn open_file(&mut self, bytes: &[u8], name: &str, fly: Option<f64>) -> Result<Option<Pose>, String> {
        let lower = name.to_lowercase();
        // Растр кровли, которого ждёт подземная модель, - ей.
        if let Some(sub) = self.subsurface.as_mut().filter(|s| s.wants(name)) {
            sub.add_side(name, bytes.to_vec());
            return Ok(None);
        }
        if lower.ends_with(".gpkg")
            && let Ok(layers) = data::layers::read(bytes.to_vec(), name)
        {
            if crate::subsurface::is_subsurface(&layers) {
                return Ok(self.open_subsurface(layers, name, fly));
            }
            if let Some(pose) = self.open_tracks(layers, name, fly) {
                return Ok(pose);
            }
        }
        // Модель самосвала демо ждёт слой самосвалов.
        if name == crate::tracks::DEMO_MODEL {
            let file = PickedFile { name: name.to_string(), path: None, bytes: bytes.to_vec() };
            let (main, _, package) = Self::model_package(std::slice::from_ref(&file)).ok_or_else(|| trf(self.lang, "The model cannot be read: {name}", &[("name", name)]))?;
            self.tracks.demo_model = Some(crate::tracks::Model { name: main, package: Arc::new(package) });
            return Ok(None);
        }
        if lower.ends_with(".tif") || lower.ends_with(".tiff") {
            return self.open_relief(bytes, name, fly);
        }
        if lower.ends_with(".las") || lower.ends_with(".laz") {
            return self.open_cloud(bytes, name, fly);
        }
        match marks::open(bytes, name) {
            Ok(opened) => {
                // Сцена ложится папкой в «Мои метки», прежние остаются.
                self.marks.add(opened.tree, &opened.name, marks::root_name(self.lang));
                self.player = None;
                self.marks_changed();
                // Тело сцены - до её позы, как `ui/scene.py` модуля. Сцена
                // неба открывает небо со своим взглядом.
                match opened.body.as_deref() {
                    Some("sky") => {
                        self.enter_sky();
                        if let (Some([ra, dec, fov]), Some(sky)) = (opened.sky, &mut self.sky) {
                            sky.view.set(ra, dec, Some(fov));
                        }
                        self.open_message = None;
                        return Ok(None);
                    }
                    Some(body) => {
                        self.leave_sky();
                        self.set_body(body);
                    }
                    None => {}
                }
                // Слои сцены, как `_apply_extra` модуля: известные строки
                // каталога включаются или выключаются. Земные строки на
                // другом теле не трогаются.
                for (key, on) in &opened.extras {
                    if let Some(extra) = crate::layers::Extra::from_key(key)
                        && (self.planet.earth || !extra.earth_only())
                        && self.extras_on.contains(&extra) != *on
                    {
                        self.extras.insert(extra);
                        self.set_extra(extra, *on);
                    }
                }
                // Поза, заданная при запуске, сильнее позы сцены.
                if let Some(pose) = opened.pose.clone().filter(|_| fly.is_some() || !self.keep_pose) {
                    match fly {
                        Some(now) => crate::input::fly(&mut self.nav, pose, now),
                        None => self.nav.show(pose),
                    }
                }
                // Название сцены видно в «Моих метках», строка
                // сообщения - для ошибок и загрузки.
                self.open_message = None;
                Ok(opened.pose)
            }
            Err(e) => {
                self.open_message = Some(trf(self.lang, "The file did not open: {name}: {e}", &[("name", name), ("e", &e)]));
                Err(e)
            }
        }
    }

    /// Открыть подземную модель из слоёв файла name, `start` модуля:
    /// прежняя модель уходит, окно режима открывается, камера летит
    /// к модели.
    /// Точечные слои файла с полем времени - треки, остальные слои -
    /// папкой в «Мои метки». None - треков в файле нет, иначе поза
    /// к охвату точек треков.
    fn open_tracks(&mut self, layers: Vec<data::layers::LayerData>, name: &str, fly: Option<f64>) -> Option<Option<Pose>> {
        let (found, rest) = crate::tracks::Tracks::split(&layers, name);
        if found.is_empty() {
            return None;
        }
        let points: Vec<(f64, f64)> = found.iter().flat_map(crate::tracks::Layer::places).collect();
        let others: Vec<data::layers::LayerData> = layers.into_iter().enumerate().filter(|(n, _)| rest.contains(n)).map(|(_, l)| l).collect();
        if others.iter().any(|l| l.items.iter().any(|i| i.shape != data::layers::Shape::None)) {
            self.marks.add(crate::gpkg::folder(others, name), name, marks::root_name(self.lang));
            self.marks_changed();
        }
        let count = found.len();
        let demo = self.tracks.demo_model.is_some();
        self.tracks.add(found, name);
        self.time_extent();
        // Окно «Треки» открывается у своего файла, у демо - нет.
        self.tracks.open |= !demo;
        self.open_message = Some(trf(self.lang, "Tracks: {count}. The time slider grows the paths.", &[("count", &count.to_string())]));
        let camera = &self.nav.camera;
        let pose = marks::fit_points(&points, camera.fov_y, camera.aspect()).map(|p| Pose::new(p.lat, p.lon, p.distance, TRACK_HEADING, TRACK_TILT));
        if let Some(pose) = pose.clone().filter(|_| fly.is_some() || !self.keep_pose) {
            match fly {
                Some(now) => crate::input::fly(&mut self.nav, pose, now),
                None => self.nav.show(pose),
            }
        }
        Some(pose)
    }

    /// Охват шкалы времени - время меток и треков.
    fn time_extent(&mut self) {
        let marks = self.marks.extent();
        let tracks = self.tracks.span();
        let extent = match (marks, tracks) {
            (Some(a), Some(b)) => Some((a.0.min(b.0), a.1.max(b.1))),
            (a, b) => a.or(b),
        };
        self.timebar.set_extent(extent);
    }

    fn open_subsurface(&mut self, layers: Vec<data::layers::LayerData>, name: &str, fly: Option<f64>) -> Option<Pose> {
        let now = fly.unwrap_or(0.0);
        let mut sub = crate::subsurface::Subsurface::new(layers, name, self.open_dir.clone(), now);
        sub.window_open = true;
        let pose = sub.middle().map(|(lat, lon, extent)| Pose::new(lat, lon, (extent * 1.5).max(2000.0), 30.0, 60.0));
        self.subsurface = Some(sub);
        if let Some(globe) = &mut self.globe {
            globe.clear_subsurface();
        }
        if let Some(pose) = pose.clone().filter(|_| fly.is_some() || !self.keep_pose) {
            match fly {
                Some(now) => crate::input::fly(&mut self.nav, pose, now),
                None => self.nav.show(pose),
            }
        }
        self.open_message = None;
        pose
    }

    /// Ползунок возраста палеогеографии внизу слева, `PaleoBar` модуля:
    /// прошлое слева, настоящее справа, показ к настоящему.
    fn paleo_bar(&mut self, ctx: &egui::Context, now: f64) {
        let lang = self.lang;
        let Some(state) = &mut self.paleo else {
            return;
        };
        let ages = &data::paleo::AGES;
        let last = ages.len() - 1;
        let mut changed = false;
        if state.playing && now - state.last >= PALEO_STEP {
            state.last = now;
            if state.index == 0 {
                state.playing = false;
            } else {
                state.index -= 1;
                changed = true;
            }
        }
        if state.playing {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(PALEO_STEP / 3.0));
        }
        egui::Area::new(egui::Id::unique("paleo-bar")).order(egui::Order::Foreground).anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -70.0)).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Прошлое слева: ползунок идёт от последнего возраста.
                    let mut position = last - state.index.min(last);
                    let slider = ui.add(egui::Slider::new(&mut position, 0..=last).show_value(false)).on_hover_text(tr(lang, "Age in millions of years ago. The map of relief and depths for this age - PaleoDEM PALEOMAP, Scotese and Wright 2018."));
                    if slider.changed() {
                        state.index = last - position;
                        changed = true;
                    }
                    let play = if state.playing { "⏸" } else { "▶" };
                    if ui.button(play).on_hover_text(tr(lang, "Show from the chosen age to the present through all maps of the set.")).clicked() {
                        state.playing = !state.playing && state.index > 0;
                        state.last = now;
                    }
                    let age = ages[state.index.min(last)];
                    let text = if age == 0 {
                        tr(lang, "Present").to_string()
                    } else {
                        trf(lang, "{age} million years ago, {period}", &[("age", &age.to_string()), ("period", period_name(lang, data::paleo::period(f64::from(age))))])
                    };
                    ui.label(text);
                });
            });
        });
        if changed {
            let age = ages[state.index.min(last)];
            self.set_basemap(Basemap::Paleo(age));
        }
    }

    /// Окно «Карты Рамсея»: каталог, сборка, готовая карта - картинкой
    /// на поверхности в «Мои метки», папка «Карты Рамсея».
    fn step_rumsey(&mut self, ctx: &egui::Context, now: f64) {
        let lang = self.lang;
        let Some(r) = &mut self.rumsey else {
            return;
        };
        {
            let loaders = &self.loaders;
            let request = |limit: u64| (loaders.request)(limit);
            r.step(lang, &request);
            r.show(ctx, lang, &request);
        }
        if r.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if let Some(b) = r.fly.take() {
            let distance = data::rumsey::fly_distance(&b, self.nav.camera.fov_y);
            crate::input::fly(&mut self.nav, Pose::new((b[1] + b[3]) / 2.0, (b[0] + b[2]) / 2.0, distance, 0.0, 0.0), now);
        }
        let built = r.built.take();
        if !r.open && !r.busy() {
            self.rumsey = None;
        }
        let Some(b) = built else {
            return;
        };
        let year = b.year.map_or(String::new(), |y| y.to_string());
        let name = if year.is_empty() { b.title.clone() } else { format!("{year} {}", b.title) };
        self.add_picture(tr(lang, "Rumsey maps"), &name, &format!("rumsey/{}.png", b.number), b.png, b.bbox, (!year.is_empty()).then(|| (year.clone(), year)), String::new());
    }

    /// Окно «Космоснимки»: поиск, чтение каналов, готовый снимок -
    /// картинкой на поверхности в «Мои метки», папка «Космоснимки».
    fn step_sentinel(&mut self, ctx: &egui::Context, now: f64) {
        let lang = self.lang;
        let Some(s) = &mut self.sentinel else {
            return;
        };
        {
            let loaders = &self.loaders;
            let request = |limit: u64| (loaders.request)(limit);
            s.step(lang, now, &request);
            s.show(ctx, lang, &request);
        }
        if s.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let built = s.built.take();
        let trend = s.series.as_mut().and_then(|x| x.built.take());
        if !s.shown() && !s.busy() {
            self.sentinel = None;
        }
        let stem = |name: &str| name.chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).collect::<String>();
        // Карта тренда - без времени: она о всём ряде.
        if let Some(b) = trend {
            self.add_picture(tr(lang, "Trend maps"), &b.name, &format!("trend/{}.png", stem(&b.name)), b.png, b.bbox, None, b.credit);
        }
        let Some(b) = built else {
            return;
        };
        self.add_picture(tr(lang, "Satellite images"), &b.name, &format!("images/{}.png", stem(&b.name)), b.png, b.bbox, Some((b.day.clone(), b.day.clone())), b.credit);
    }

    /// Окно «Треки»: смена охвата времени треков - новый охват шкалы,
    /// кнопка «3D-модель…» - выбор файла модели.
    fn step_tracks(&mut self, ctx: &egui::Context) {
        let before = self.tracks.span();
        self.tracks.show(ctx, self.lang);
        if self.tracks.span() != before {
            self.time_extent();
        }
        if let Some(n) = self.tracks.pick_model.take() {
            self.pick_for(ctx, Pick::TrackModel(n));
        }
    }

    /// Контекст вида для модели, `assistant_context` модуля: точка
    /// взгляда, тело, включённые строки, сектор разреза, количество
    /// выделенных меток, время. Названия меток не уходят.
    fn ai_context(&self) -> serde_json::Value {
        let p = &self.nav.pose;
        let round = |v: f64, k: f64| (v * k).round() / k;
        let mut on: Vec<String> = Vec::new();
        if self.settings.relief {
            on.push("relief".into());
        }
        on.extend(self.extras_on.iter().map(|e| e.key().to_string()));
        let mut keys = vec!["relief".to_string()];
        keys.extend(crate::layers::Extra::all().into_iter().filter(|e| !e.desktop_only() || !cfg!(target_arch = "wasm32")).map(|e| e.key().to_string()));
        let mut bodies = vec!["sky".to_string()];
        bodies.extend(scene::planets::MENU.iter().flat_map(|(_, keys)| keys.iter().map(|k| k.to_string())));
        let wedge = self.cutaway.as_ref().map(|c| [c.wedge.west, c.wedge.east, c.wedge.south, c.wedge.north].map(|v| round(v, 100.0)));
        serde_json::json!({
            "view": {"lat": round(p.lat, 1e4), "lon": round(p.lon, 1e4), "distance_km": round(p.distance / 1000.0, 10.0), "heading": round(p.heading, 10.0), "tilt": round(p.tilt, 10.0)},
            "body": if self.sky.is_some() { "sky".to_string() } else { self.planet.key().to_string() },
            "body_keys": bodies,
            "layers_on": on,
            "layer_keys": keys,
            "cutaway": wedge,
            "selected_places": usize::from(self.tree_state.selected.is_some()),
            "utc": scene::when::text(web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())),
        })
    }

    /// Окно помощника: ответы модели, вызовы инструментов, запись
    /// предложенных и созданных меток.
    fn step_ai(&mut self, ctx: &egui::Context, now: f64) {
        let lang = self.lang;
        let context = self.ai_context();
        {
            let loaders = &self.loaders;
            let request = |limit: u64| (loaders.request)(limit);
            self.ai.step(lang, now, &self.settings.ai, &context, &request);
            if self.shows_ui() && self.ai.show(ctx, lang, &mut self.settings.ai, &context, &request) {
                self.settings_dirty = true;
            }
        }
        // Вызовы без разрешения исполняются подряд, `_step` модуля.
        for _ in 0..assistant::MAX_ROUNDS * 4 {
            let Some(call) = self.ai.run.take() else {
                break;
            };
            let text = self.ai_tool(ctx, &call, now);
            let context = self.ai_context();
            let loaders = &self.loaders;
            let request = |limit: u64| (loaders.request)(limit);
            self.ai.result(lang, &text, &self.settings.ai, &context, &request);
        }
        if let Some((kml, made)) = self.ai.write.take() {
            let note = self.write_ai_kml(&kml, made);
            self.ai.say(crate::ai::Who::Note, &note);
        }
        if self.ai.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// Документ помощника - новой папкой в «Мои метки», перелёт к ней.
    /// У созданного одним запросом шкала времени открывается на даты
    /// меток, `_show_made_time` модуля.
    fn write_ai_kml(&mut self, kml: &str, made: bool) -> String {
        let lang = self.lang;
        let opened = match marks::open(kml.as_bytes(), "assistant.kml") {
            Ok(o) => o,
            Err(e) => return trf(lang, "Not pasted: {e}", &[("e", &e)]),
        };
        let count = count_places(&opened.tree);
        let mut tree = opened.tree;
        if tree.name.trim().is_empty() || tree.name == "assistant.kml" {
            tree.name = tr(lang, "Assistant").to_string();
        }
        let at = self.marks.insert(&[], scene::kml::Item::Folder(tree), marks::root_name(lang));
        self.marks_changed();
        self.tree_state.selected = Some(at.clone());
        self.queued.push(crate::marks::Action::Fly(at));
        if made && self.timebar.extent.is_some() {
            self.timebar.open = true;
        }
        trf(lang, "Placemarks written to My Places: {count}.", &[("count", &count.to_string())])
    }

    /// Исполнить вызов инструмента модели, ответ - текст для модели,
    /// `assistant_tool` модуля. Ошибка в аргументах - тоже ответ, модель
    /// может исправиться. Тексты - для модели, по-русски, как описания
    /// инструментов.
    fn ai_tool(&mut self, ctx: &egui::Context, call: &assistant::Call, now: f64) -> String {
        let args = &call.args;
        let num = |k: &str| args.get(k).and_then(serde_json::Value::as_f64).filter(|v| v.is_finite());
        let text = |k: &str| args.get(k).and_then(serde_json::Value::as_str).unwrap_or("").trim().to_string();
        match call.name.as_str() {
            "fly_to" => {
                let (Some(lat), Some(lon)) = (num("lat"), num("lon")) else {
                    return "Ошибка в аргументах fly_to: нужны lat и lon.".into();
                };
                let distance = num("distance_km").unwrap_or(500.0).clamp(0.05, 50_000.0) * 1000.0;
                let (heading, tilt) = (self.nav.pose.heading, self.nav.pose.tilt);
                crate::input::fly(&mut self.nav, Pose::new(lat.clamp(-90.0, 90.0), lon, distance, heading, tilt), now);
                format!("Перелёт к {lat:.3}, {lon:.3}.")
            }
            "search_place" => {
                let query = text("query");
                if query.is_empty() {
                    return "Ошибка в аргументах search_place: нужен query.".into();
                }
                self.search_text = query.clone();
                if let Some(place) = self.search.submit(&query) {
                    self.fly_to_found(&place, now);
                }
                format!("Поиск запущен: {query}.")
            }
            "set_body" => {
                let body = text("body");
                if body == "sky" {
                    self.enter_sky();
                } else if scene::planets::MENU.iter().any(|(_, keys)| keys.contains(&body.as_str())) {
                    self.leave_sky();
                    self.set_body(&body);
                } else {
                    return format!("Неизвестное тело: {body}.");
                }
                format!("Тело: {body}.")
            }
            "set_layer" => {
                let key = text("key");
                let on = args.get("on").and_then(serde_json::Value::as_bool).unwrap_or(true);
                if key == "relief" {
                    self.settings.relief = on;
                    self.settings_dirty = true;
                } else if let Some(extra) = crate::layers::Extra::from_key(&key).filter(|e| !e.desktop_only() || !cfg!(target_arch = "wasm32")) {
                    self.set_extra(extra, on);
                } else {
                    return format!("Неизвестная строка: {key}.");
                }
                format!("Строка {key}: {}.", if on { "включена" } else { "выключена" })
            }
            "list_actions" => assistant::actions_text(&assistant::assistant_actions(&self.ai_actions())),
            "run_action" => {
                let key = text("key");
                let Some(action) = assistant::assistant_actions(&self.ai_actions()).into_iter().find(|a| a.key == key) else {
                    return format!("Нет такого действия: {key}. Ключ берётся из list_actions.");
                };
                let Some(command) = crate::commands::Command::from_key(&key) else {
                    return format!("Нет такого действия: {key}. Ключ берётся из list_actions.");
                };
                self.run_command(ctx, command, now);
                format!("Выполнено: {} - {}.", action.text, action.note)
            }
            "set_opacity" => {
                let key = text("key");
                let Some(value) = num("value") else {
                    return "Ошибка в аргументах set_opacity: нужен value.".into();
                };
                let value = value.clamp(0.0, 1.0) as f32;
                let entry = crate::layers::Extra::from_key(&key).filter(|e| self.extras_on.contains(e)).and_then(|e| match e {
                    crate::layers::Extra::Map(n) => crate::maps::entry(n),
                    _ => None,
                });
                let Some(entry) = entry else {
                    return format!("Слоя {key} нет среди включённых карт.");
                };
                self.map_opacity.insert(entry.key.to_string(), value);
                self.settings.opacity = self.map_opacity.iter().map(|(k, v)| (k.clone(), *v)).collect();
                self.settings_dirty = true;
                if self.vector.set_opacity(entry.key, value)
                    && let Some(globe) = &mut self.globe
                {
                    globe.clear_overlays();
                }
                format!("Непрозрачность {key}: {value:.2}.")
            }
            "set_time" => {
                let (start, end) = (text("start"), text("end"));
                if start.is_empty() && end.is_empty() {
                    self.timebar.open = false;
                    return "Шкала времени закрыта.".into();
                }
                let Some(extent) = self.timebar.extent else {
                    return "Шкалы времени нет: у видимых меток нет времени.".into();
                };
                let lo = if start.is_empty() { Some(extent.0) } else { scene::when::parse(&start) };
                let hi = if end.is_empty() { Some(extent.1) } else { scene::when::parse(&end) };
                let (Some(lo), Some(hi)) = (lo, hi) else {
                    return format!("Даты не разобраны: {start} - {end}.");
                };
                self.timebar.open = true;
                self.timebar.range = (lo.min(hi), lo.max(hi));
                format!("Шкала времени: {start} - {end}.")
            }
            "earth_cutaway" => {
                if args.get("off").and_then(serde_json::Value::as_bool).unwrap_or(false) {
                    self.set_extra(crate::layers::Extra::Cutaway, false);
                    return "Разрез Земли убран.".into();
                }
                if !self.planet.earth {
                    return "Разрез Земли есть только у Земли.".into();
                }
                if self.cutaway.is_none() {
                    self.set_extra(crate::layers::Extra::Cutaway, true);
                }
                if let (Some(c), Some(west), Some(east)) = (&mut self.cutaway, num("west"), num("east")) {
                    c.wedge = data::cutaway::Wedge { west, east, south: num("south").unwrap_or(0.0).clamp(-90.0, 90.0), north: num("north").unwrap_or(90.0).clamp(-90.0, 90.0) };
                }
                match &self.cutaway {
                    Some(c) => format!("Сектор: {:.1}, {:.1}, {:.1}, {:.1}.", c.wedge.west, c.wedge.east, c.wedge.south, c.wedge.north),
                    None => "Разрез не построен.".into(),
                }
            }
            "section_down" => {
                let points: Vec<(f64, f64)> = args
                    .get("points")
                    .and_then(serde_json::Value::as_array)
                    .map(|a| a.iter().filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?))).filter(|(a, b)| a.is_finite() && b.is_finite()).collect())
                    .unwrap_or_default();
                if points.len() < 2 {
                    return "Для разреза нужны хотя бы две точки.".into();
                }
                let title = tr(self.lang, "Assistant section").to_string();
                match &mut self.section {
                    Some(s) => s.set_source(title, points.clone()),
                    None => self.section = Some(crate::section::SectionWindow::new(title, points.clone(), self.crust_cache().as_deref())),
                }
                format!("Разрез открыт: точек {}, очаги и строение коры в его окне.", points.len())
            }
            "point_info" => {
                let (Some(lat), Some(lon)) = (num("lat"), num("lon")) else {
                    return "Ошибка в аргументах point_info: нужны lat и lon.".into();
                };
                let objects = self.make_objects(lat, lon, None, None);
                let mut lines = vec![objects.point];
                for (group, features) in &objects.groups {
                    for (name, values) in features {
                        let fields: Vec<String> = values.iter().map(|(k, v)| format!("{k}: {v}")).collect();
                        lines.push(format!("{group}, {name}: {}", fields.join("; ")));
                    }
                }
                lines.join("\n")
            }
            "quakes_summary" => {
                let (Some(south), Some(north), Some(west), Some(east)) = (num("south"), num("north"), num("west"), num("east")) else {
                    return "Ошибка в аргументах quakes_summary: нужны south, north, west, east.".into();
                };
                let Some(quakes) = self.dots.quakes(crate::dots::Layer::Quakes) else {
                    self.set_extra(crate::layers::Extra::Dots(crate::dots::Layer::Quakes), true);
                    return "Сводка землетрясений загружается, повтори запрос через несколько секунд.".into();
                };
                let list: Vec<assistant::Quake> = quakes.iter().map(|q| assistant::Quake { lat: q.lat, lon: q.lon, depth: q.depth, mag: q.mag, place: q.place.clone() }).collect();
                assistant::quakes_text(&list, (south, north, west, east), 3)
            }
            "add_basemap" => {
                let ready = text("ready");
                if ready.is_empty() {
                    return "Свою подложку по адресу помощник у PlanetX Lite не ставит. Готовые источники - ready.".into();
                }
                let Some(n) = crate::basemap::READY.iter().position(|r| r.key == ready) else {
                    return format!("Нет готового источника {ready}.");
                };
                self.set_basemap(Basemap::Ready(n));
                format!("Подложка поставлена: {}.", crate::basemap::READY[n].name)
            }
            "add_kml" => {
                let raw = text("kml");
                let kml = assistant::extract_kml(&raw).unwrap_or(raw);
                let tree = match scene::kml::read_kml(kml.as_bytes(), "assistant.kml") {
                    Ok(t) => t,
                    Err(e) => return format!("KML не разобран: {e:?}. Исправь документ."),
                };
                let count = count_places(&tree);
                if count == 0 {
                    return "В документе KML нет меток.".into();
                }
                let name = if tree.name.trim().is_empty() { tr(self.lang, "Assistant").to_string() } else { tree.name.clone() };
                self.ai.propose(trf(self.lang, "The assistant offers the folder «{name}», placemarks {count}.", &[("name", &name), ("count", &count.to_string())]), kml);
                format!("Документ показан пользователю, меток {count}. Запись - после его подтверждения.")
            }
            "get_kml" => {
                let p = &self.nav.pose;
                let mut out = vec![assistant::look_at_kml(p.lat, p.lon, p.distance, p.heading, p.tilt)];
                let selected = self.tree_state.selected.clone().unwrap_or_default();
                let folder = match (self.marks.folder(&selected), self.marks.place(&selected)) {
                    (Some(f), _) if !selected.is_empty() => Some(f.clone()),
                    (_, Some(place)) => {
                        let mut f = scene::kml::Folder::new(&place.name);
                        f.children.push(scene::kml::Item::Place(place.clone()));
                        Some(f)
                    }
                    _ => None,
                };
                match folder.map(|f| crate::export::tree_bytes(&f, false)) {
                    Some(Ok(bytes)) => out.push(String::from_utf8_lossy(&bytes).into_owned()),
                    _ => out.push("Выделенных меток нет.".into()),
                }
                out.join("\n")
            }
            other => format!("Нет такого инструмента: {other}."),
        }
    }

    /// «Моё место»: кнопка у правого нижнего края вида, перелёт к первому
    /// месту, камера следом, окно с записью трека.
    fn step_gps(&mut self, ctx: &egui::Context, now: f64) {
        if !crate::gps::Gps::available() || !self.shows_ui() {
            return;
        }
        let lang = self.lang;
        let mut toggle = false;
        egui::Area::new(egui::Id::unique("planetx-gps"))
            .order(egui::Order::Middle)
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -40.0))
            .show(ctx, |ui| {
                crate::theme::plate().inner_margin(egui::Margin::same(3)).show(ui, |ui| {
                    let text = egui::RichText::new("◎").size(22.0).color(if self.gps.on { egui::Color32::from_rgb(30, 140, 255) } else { ui.visuals().text_color() });
                    toggle = ui.add(egui::Button::new(text).frame(false)).on_hover_text(tr(lang, "My location: the location of this device on the globe, the camera follows, a track can be recorded.")).clicked();
                });
            });
        if toggle {
            if self.gps.on {
                self.gps.open = !self.gps.open;
            } else {
                self.gps.start(ctx);
            }
        }
        if let Some(fix) = self.gps.step() {
            let p = &self.nav.pose;
            let (distance, heading, tilt) = (p.distance, p.heading, p.tilt);
            if !self.gps.flown {
                self.gps.flown = true;
                crate::input::fly(&mut self.nav, Pose::new(fix.lat, fix.lon, crate::gps::FIX_DISTANCE, heading, tilt), now);
            } else if self.gps.follow {
                self.nav.show(Pose::new(fix.lat, fix.lon, distance, heading, tilt));
            }
        }
        if self.gps.window(ctx, lang) == Some(crate::gps::Action::Save)
            && let Some(place) = self.gps.track_place(lang)
        {
            let name = place.name.clone();
            let at = self.marks.insert(&[], scene::kml::Item::Place(place), marks::root_name(lang));
            self.tree_state.selected = Some(at);
            self.marks_changed();
            self.gps.track.clear();
            self.gps.dirty = true;
            self.open_message = Some(trf(lang, "The track is in My Places: {name}.", &[("name", &name)]));
        }
    }

    /// Вопрос помощнику из строки «Поиск»: ответ встаёт под строкой.
    fn ask_from_search(&mut self, text: &str) {
        let from = self.ai.lines.len();
        let context = self.ai_context();
        let loaders = &self.loaders;
        let request = |limit: u64| (loaders.request)(limit);
        if self.ai.ask(self.lang, text, &self.settings.ai, &context, &request) {
            self.ai_answer_from = Some(from);
        }
    }

    /// Действия строки «Поиск» для помощника.
    fn ai_actions(&self) -> Vec<assistant::Action> {
        crate::commands::actions(self.lang, &self.command_state()).into_iter().map(|s| assistant::Action { key: s.key, text: s.text, note: s.note }).collect()
    }

    /// Картинка на поверхности в «Мои метки», в папку folder_name: у
    /// программы - файлом file в папке данных, у страницы - в метках.
    #[allow(clippy::too_many_arguments)]
    fn add_picture(&mut self, folder_name: &str, name: &str, file: &str, png: Vec<u8>, bbox: [f64; 4], time: Option<scene::when::Time>, description: String) {
        let [west, south, east, north] = bbox;
        let mut o = scene::overlays::Overlay::new(scene::overlays::Kind::Ground);
        o.set_box([north, south, east, west, 0.0]);
        let mut overlay = scene::kml::Overlay::new(name, o);
        overlay.time = time;
        overlay.description = description;
        let path = self.data_dir().map(|d| d.join(file));
        match path.filter(|p| p.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok()) && std::fs::write(p, &png).is_ok()) {
            Some(p) => overlay.href = p.to_string_lossy().into_owned(),
            None => {
                overlay.href = file.replace('/', "-");
                overlay.image = Some(png);
            }
        }
        let root = marks::root_name(self.lang);
        let tree = self.marks.root(root);
        let at = tree.children.iter().position(|i| matches!(i, scene::kml::Item::Folder(f) if f.name == folder_name));
        let at = match at {
            Some(n) => n,
            None => {
                tree.children.insert(0, scene::kml::Item::Folder(scene::kml::Folder::new(folder_name)));
                0
            }
        };
        self.marks.insert(&[at], scene::kml::Item::Overlay(overlay), root);
        self.marks_changed();
    }

    /// Шаг течений: сутки по бегунку шкалы времени, загрузка, частицы.
    fn step_currents(&mut self, ctx: &egui::Context, now: f64) {
        let moment = match self.timebar.window() {
            Some((_, hi)) if hi.is_finite() => Some(hi),
            _ => None,
        };
        let altitude = self.nav.camera.ellipsoid.ecef_to_geodetic(self.nav.camera.eye).2;
        let view = (self.nav.pose.lat, self.nav.pose.lon, altitude, self.view_width());
        let lang = self.lang;
        let Some(c) = &mut self.currents else {
            return;
        };
        let loaders = &self.loaders;
        if let Some(message) = c.step(lang, now, moment, view, &|limit: u64| (loaders.request)(limit)) {
            self.open_message = Some(message);
        }
        let changed = std::mem::take(&mut c.changed);
        // Струйки бегут - кадры идут по такту частиц.
        ctx.request_repaint_after(std::time::Duration::from_millis(u64::from(data::flow::TICK_MS)));
        if changed {
            self.refresh_painters();
        }
    }

    /// Пришли сведения об участках: группа окна «Объекты» и подсветка
    /// контуров, `_parcels_arrived` модуля.
    fn step_parcels(&mut self) {
        let Some(((lat, lon), ask)) = &self.parcel_ask else {
            return;
        };
        let Some(answer) = ask.take() else {
            return;
        };
        let (lat, lon) = (*lat, *lon);
        self.parcel_ask = None;
        let lang = self.lang;
        let result = answer.and_then(|d| data::cadastre::parse_info(&d));
        let features: Vec<crate::identify::Feature> = match &result {
            Err(e) => vec![(trf(lang, "not received: {error}", &[("error", e)]), Vec::new())],
            Ok(list) if list.is_empty() => vec![(tr(lang, "No parcels under the point").to_string(), Vec::new())],
            Ok(list) => list
                .iter()
                .map(|p| {
                    let mut values: Vec<(String, String)> = p
                        .values
                        .iter()
                        .map(|(k, v)| {
                            let unit = data::cadastre::FIELDS.iter().find(|(f, _)| f == k).map_or("", |(_, u)| *u);
                            let text = if unit.is_empty() { v.clone() } else { format!("{v} {unit}") };
                            (tr(lang, crate::identify::parcel_label(k)).to_string(), text)
                        })
                        .collect();
                    values.push((tr(lang, "NSPD map").to_string(), data::cadastre::map_link(lat, lon)));
                    (trf(lang, "Land parcel {number}", &[("number", &p.number)]), values)
                })
                .collect(),
        };
        self.parcels = result.unwrap_or_default();
        if let Some(objects) = &mut self.objects
            && objects.at == (lat, lon)
        {
            objects.groups.insert(0, (tr(lang, "Rosreestr cadastre").to_string(), features));
        }
    }

    /// Шаг загрузчиков Overpass: тайлы у точки взгляда, пришедшие
    /// объекты - в слои точек и линий.
    fn step_overpass(&mut self, now: f64) {
        // Загрузчик идёт за включённой строкой: строки включаются и из
        // настроек при запуске, мимо щелчка по строке.
        let keys = self.dots.keys();
        let mines_on = keys.contains(&crate::dots::Layer::OsmMines);
        let infra_on = keys.contains(&crate::dots::Layer::InfraOsm);
        if mines_on != self.osm_mines.is_some() {
            self.osm_mines = mines_on.then(|| crate::overpass::Overpass::new(data::mining::query, data::mining::parse));
        }
        if infra_on != self.osm_infra.is_some() {
            self.osm_infra = infra_on.then(|| crate::overpass::Overpass::new(data::infra::query, data::infra::parse));
        }
        if self.osm_mines.is_none() && self.osm_infra.is_none() {
            return;
        }
        let altitude = self.nav.camera.ellipsoid.ecef_to_geodetic(self.nav.camera.eye).2;
        let view = (self.nav.pose.lat, self.nav.pose.lon, altitude);
        let loaders = &self.loaders;
        let request = |limit: u64| (loaders.request)(limit);
        let mut failed = false;
        if let Some(o) = &mut self.osm_mines {
            o.step(now, view, &mut self.overpass_mirrors, &request);
            failed |= o.failed;
            if std::mem::take(&mut o.dirty) {
                self.dots.set_data(crate::dots::Layer::OsmMines, crate::dots::Data::Mines(data::mining::to_deposits(&o.all())));
            }
        }
        if let Some(o) = &mut self.osm_infra {
            o.step(now, view, &mut self.overpass_mirrors, &request);
            failed |= o.failed;
            if std::mem::take(&mut o.dirty) {
                self.dots.set_data(crate::dots::Layer::InfraOsm, crate::dots::infra_data(o.all()));
            }
        }
        if failed {
            self.open_message = Some(tr(self.lang, "The OpenStreetMap server did not answer, the request will repeat by itself.").to_string());
        }
    }

    /// Папка файлов групп спутников, у страницы None.
    #[cfg(not(target_arch = "wasm32"))]
    fn satellite_dir(&self) -> Option<std::path::PathBuf> {
        Some(self.demo_cache.as_ref()?.parent()?.join("satellites"))
    }

    #[cfg(target_arch = "wasm32")]
    fn satellite_dir(&self) -> Option<std::path::PathBuf> {
        None
    }

    /// Момент спутников: правый бегунок открытой шкалы времени, иначе
    /// часы, `moment` модуля.
    fn satellite_moment(&self) -> f64 {
        match self.timebar.window() {
            Some((_, hi)) if hi.is_finite() => hi,
            _ => web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64()),
        }
    }

    /// Шаг спутников: загрузка групп, точки по такту, камера следом.
    fn step_satellites(&mut self, now: f64) {
        let moment = self.satellite_moment();
        let lang = self.lang;
        let Some(sats) = &mut self.satellites else {
            return;
        };
        {
            let loaders = &self.loaders;
            sats.step(lang, moment, &|limit: u64| (loaders.request)(limit));
        }
        let tick = if sats.follow { crate::satellites::FOLLOW_TICK } else { crate::satellites::TICK };
        if now - sats.shown_at < tick && !self.sat_points.is_empty() {
            return;
        }
        sats.shown_at = now;
        let eye = self.nav.camera.eye;
        let shown = sats.points(moment, eye.to_array());
        let colors: Vec<[u8; 3]> = shown.iter().map(|(i, _)| sats.swarm.colors.get(*i).map_or([255; 3], |c| [(c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8])).collect();
        let sizes: Vec<f32> = shown.iter().map(|(i, _)| sats.swarm.sizes.get(*i).copied().unwrap_or(data::satellites::SIZE)).collect();
        self.sat_shown = shown.iter().map(|(i, p)| (*i, DVec3::from_array(*p))).collect();
        self.sat_points.clear();
        if !self.sat_shown.is_empty() {
            let center = self.sat_shown.iter().fold(DVec3::ZERO, |a, (_, p)| a + *p) / self.sat_shown.len() as f64;
            self.sat_points.push(render::PointsMesh {
                center,
                offsets: self.sat_shown.iter().map(|(_, p)| (*p - center).as_vec3().to_array()).collect(),
                colors,
                size: data::satellites::SIZE,
                sizes,
                dot: Some(render::Dot { alpha: 1.0, lift: 0.0, space: true }),
            });
        }
        self.dots.dirty = true;
        // Камера следом: точка под спутником, азимут по курсу, расстояние
        // и наклон остаются у пользователя.
        if sats.follow
            && let Some(index) = sats.selected.and_then(|n| sats.swarm.index_of(n))
            && let Some((point, velocity)) = sats.swarm.motion(index, moment)
        {
            let (lat, lon, _) = self.nav.camera.ellipsoid.ecef_to_geodetic(DVec3::from_array(point));
            let heading = data::satellites::heading(point, velocity);
            let (distance, tilt) = (self.nav.pose.distance, self.nav.pose.tilt);
            self.nav.show(Pose::new(lat, lon, distance, heading, tilt));
        }
    }

    /// Спутник под пикселем кадра: номер NORAD.
    fn satellite_under(&self, px: f64, py: f64, ppp: f64) -> Option<i64> {
        let sats = self.satellites.as_ref()?;
        let radius = crate::satellites::PICK_PIXELS * ppp;
        self.sat_shown
            .iter()
            .filter_map(|(i, p)| {
                let (q, front) = self.nav.camera.project(*p);
                let gap = (q.x - px).hypot(q.y - py);
                (front && gap <= radius).then_some((gap, *i))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .and_then(|(_, i)| sats.swarm.numbers.get(i).copied())
    }

    /// «Википедия рядом»: запросы тайлов у точки взгляда, точки и подписи
    /// статей, первый абзац статьи в окне «Объекты».
    fn step_wiki(&mut self, now: f64) {
        let Some(wiki) = &mut self.wiki else {
            return;
        };
        let e = self.nav.camera.ellipsoid;
        let altitude = e.ecef_to_geodetic(self.nav.camera.eye).2;
        let (lat, lon) = (self.nav.pose.lat, self.nav.pose.lon);
        {
            let loaders = &self.loaders;
            let request = |limit: u64| (loaders.request)(limit);
            wiki.step(now, (lat, lon, altitude), self.lang.code(), &request);
        }
        if wiki.failed {
            self.open_message = Some(tr(self.lang, "Wikipedia did not answer, retry in 30 s.").to_string());
        }
        // Первый абзац пришёл - в группу «Википедия» окна «Объекты».
        if let Some(objects) = &mut self.objects {
            let wiki_name = tr(self.lang, "Wikipedia");
            let summary = tr(self.lang, "Summary");
            for (group, features) in &mut objects.groups {
                if group != wiki_name {
                    continue;
                }
                for (title, fields) in features.iter_mut() {
                    if let Some(text) = wiki.summary_of(title).filter(|t| !t.is_empty())
                        && !fields.iter().any(|(k, _)| k == summary)
                    {
                        fields.insert(0, (summary.to_string(), text.to_string()));
                    }
                }
            }
        }
        // Набор и порядок подписей - по приходу статей и движению вида.
        if !wiki.dirty && now - self.wiki_built < 0.5 {
            return;
        }
        wiki.dirty = false;
        self.wiki_built = now;
        let shown = altitude <= data::wikipedia::MAX_ALTITUDE;
        let articles = if shown { wiki.articles(lat, lon) } else { Vec::new() };
        let store = self.heights.read();
        let points: Vec<DVec3> = articles.iter().take(crate::wiki::LABELS).map(|a| e.geodetic_to_ecef(a.lat, a.lon, store.height_at(a.lat, a.lon))).collect();
        drop(store);
        self.wiki_labels = articles.iter().zip(&points).take(crate::wiki::LABELS).map(|(a, p)| (a.title.clone(), *p)).collect();
    }

    /// Расчёты у точки: окна, шаги, тайлы высот, слои над картами,
    /// шкала инсоляции.
    fn step_calcs(&mut self, ctx: &egui::Context, now: f64) {
        let lang = self.lang;
        if let Some(w) = &mut self.etomesto
            && !w.show(ctx, lang)
        {
            self.etomesto = None;
        }
        let mut keys = Vec::new();
        let mut changed = false;
        {
            let store = self.heights.clone();
            let has = |k: TileKey| store.read().tile(k).is_some();
            let heights = self.true_heights();
            for calc in [&mut self.viewshed, &mut self.insolation].into_iter().flatten() {
                calc.show(ctx, lang, now);
                keys.extend(calc.step(lang, now, &has, &heights));
                changed |= std::mem::take(&mut calc.changed);
            }
            // Квадраты анализа рельефа и гидрологии: радиус тела и уровень
            // тайлов высот - у тела на глобусе.
            let radius = self.nav.camera.ellipsoid.a;
            let max_level = terrain_source(&self.planet).map_or(terrain::MAX_LEVEL, |s| s.max_level);
            if let Some(calc) = &mut self.relief_calc {
                calc.show(ctx, lang, now, radius, max_level);
                calc.step(lang, now, &has, &heights);
                keys.extend(calc.wanted(&has));
                changed |= std::mem::take(&mut calc.changed);
            }
            keys.extend(self.hydro.step(ctx, lang, now, radius, max_level, &has, &heights));
            changed |= std::mem::take(&mut self.hydro.changed);
        }
        self.calc_keys = keys;
        if let Some(text) = self.hydro.message.take() {
            self.open_message = Some(text);
        }
        // Уровень ниже нынешнего открывает шельф: нужны глубины морей,
        // они включаются, как `set_sea_level` модуля.
        if self.hydro.sea_level < 0.0 && !self.settings.sea && self.planet.earth && self.geoid.is_none() {
            self.settings.sea = true;
            self.settings_dirty = true;
            self.sea_changed();
        }
        // Вершины и ямы, линии стока и водосборы - в «Мои метки».
        let kml: Vec<String> = self.relief_calc.as_mut().and_then(|c| c.kml.take()).into_iter().chain(self.hydro.kml.drain(..)).collect();
        for text in kml {
            self.paste_kml(&text);
        }
        // Квадрат уклона занимает место строк «Уклон» и «Экспозиция».
        if self.relief_calc.as_mut().is_some_and(|c| std::mem::take(&mut c.built)) {
            for key in ["slope", "aspect"] {
                if let Some(n) = crate::maps::index(key)
                    && self.extras_on.contains(&crate::layers::Extra::Map(n))
                {
                    self.set_extra(crate::layers::Extra::Map(n), false);
                }
            }
        }
        if let Some(m) = &mut self.mining {
            if let Some(message) = m.step(lang) {
                self.open_message = Some(message);
            }
            changed |= std::mem::take(&mut m.changed);
        }
        if changed {
            self.refresh_painters();
        }
        // Окно закрыто и слоя нет - расчёт забыт.
        for slot in [&mut self.viewshed, &mut self.insolation] {
            if slot.as_ref().is_some_and(|c| !c.open && c.layer.is_none() && !c.busy()) {
                *slot = None;
            }
        }
        if self.relief_calc.as_ref().is_some_and(|c| !c.open && c.layer().is_none() && !c.busy()) {
            self.relief_calc = None;
        }
        if let Some(crate::pointcalc::Layer::Insolation(r)) = self.insolation.as_ref().and_then(|c| c.layer.as_ref()) {
            let rect = ctx.content_rect();
            crate::pointcalc::legend(ctx, lang, r.top(), egui::pos2(rect.left() + 12.0, rect.bottom() - 120.0));
        }
    }

    /// Слои над картами по готовым результатам: видимость, инсоляция,
    /// контуры горных разработок.
    fn refresh_painters(&mut self) {
        let mut painters: Vec<(&'static str, crate::vector::Painter)> = Vec::new();
        if let Some(grid) = self.currents.as_ref().and_then(crate::currents::Currents::grid) {
            painters.push(("currents", crate::currents::painter(grid)));
        }
        if let Some(mines) = self.mining.as_ref().and_then(crate::mining::Mining::mines) {
            painters.push(("mining", crate::mining::painter(mines)));
        }
        for (id, c) in [("viewshed", &self.viewshed), ("insolation", &self.insolation)] {
            if let Some(l) = c.as_ref().and_then(|c| c.layer.as_ref()) {
                painters.push((id, crate::pointcalc::painter(l)));
            }
        }
        if let Some((r, radius)) = self.relief_calc.as_ref().and_then(crate::reliefcalc::ReliefCalc::layer) {
            painters.push(("relief", crate::reliefcalc::painter(r, radius)));
        }
        painters.extend(self.hydro.painters());
        self.vector.set_painters(painters);
    }

    /// Папка данных программы, у страницы None.
    #[cfg(not(target_arch = "wasm32"))]
    fn data_dir(&self) -> Option<std::path::PathBuf> {
        self.demo_cache.as_ref()?.parent().map(std::path::Path::to_path_buf)
    }

    #[cfg(target_arch = "wasm32")]
    fn data_dir(&self) -> Option<std::path::PathBuf> {
        None
    }

    /// Окна точки меню на глобусе: номера листов и список своих сайтов.
    fn point_windows(&mut self, ctx: &egui::Context) {
        if let Some(w) = &mut self.sheets_window
            && !w.show(ctx, self.lang).1
        {
            self.sheets_window = None;
        }
        if let Some(w) = &mut self.links_window {
            let (items, open) = w.show(ctx, self.lang);
            if let Some(items) = items {
                self.settings.links = items;
                self.settings_dirty = true;
            }
            if !open {
                self.links_window = None;
            }
        }
    }

    /// Окно «Новый источник тайлов»: название, адрес, уровни, подпись.
    /// «Добавить» ставит источник основой и в настройки.
    fn own_window(&mut self, ctx: &egui::Context) {
        let lang = self.lang;
        let Some(d) = &mut self.own_dialog else {
            self.own_caps = None;
            return;
        };
        // Ответ GetCapabilities: слои службы, название и подпись - из него.
        if let Some(request) = &self.own_caps
            && let Some(answer) = request.take()
        {
            self.own_caps = None;
            let caps = answer.and_then(|b| scene::wms::parse(&b).map_err(|e| e.to_string()));
            if let Ok(c) = &caps {
                d.layer = 0;
                if d.name.trim().is_empty() {
                    d.name = c.layers.first().map_or_else(|| c.title.clone(), |l| l.title.clone());
                }
                if d.credit.trim().is_empty() {
                    d.credit = if c.attribution.is_empty() { c.title.clone() } else { c.attribution.clone() };
                }
            }
            d.caps = Some(caps);
        }
        let wms = scene::wms::is_wms(&d.url);
        let mut ask_caps = false;
        let mut open = true;
        let mut done = None;
        egui::Window::new(tr(lang, "New tile source")).open(&mut open).collapsible(false).resizable(false).default_width(420.0).show(ctx, |ui| {
            egui::Grid::new("own-source").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Address"));
                ui.add(egui::TextEdit::singleline(&mut d.url).desired_width(300.0).hint_text("https://example.org/{z}/{x}/{y}.png"))
                    .on_hover_text(tr(lang, "Address template of the tiles with the places {z}, {x} and {y}. {-y} - rows from bottom to top, as in TMS. Or the address of a WMS service - the window knows it by the words WMSServer, /wms or service=WMS and reads the list of its layers."));
                ui.end_row();
                if wms {
                    ui.label(tr(lang, "Layer"));
                    ui.horizontal(|ui| {
                        if let Some(Ok(caps)) = &d.caps {
                            let title = |l: &scene::wms::Layer| if l.group { format!("{} - {}", l.title, tr(lang, "all layers")) } else { l.title.clone() };
                            let current = caps.layers.get(d.layer).map(title).unwrap_or_default();
                            egui::ComboBox::from_id_salt("own-wms-layer").width(240.0).selected_text(current).show_ui(ui, |ui| {
                                for (n, l) in caps.layers.iter().enumerate() {
                                    if ui.selectable_label(d.layer == n, title(l)).clicked() {
                                        d.layer = n;
                                        d.name = l.title.clone();
                                    }
                                }
                            });
                        }
                        let waiting = self.own_caps.is_some();
                        if ui.add_enabled(!waiting, egui::Button::new(tr(lang, "Read layers"))).on_hover_text(tr(lang, "Read the list of layers of the service, the GetCapabilities request.")).clicked() {
                            ask_caps = true;
                        }
                        if waiting {
                            ui.spinner();
                        }
                    });
                    ui.end_row();
                }
                ui.label(tr(lang, "Name"));
                ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(300.0));
                ui.end_row();
                if !wms {
                    ui.label(tr(lang, "Levels up to"));
                    ui.add(egui::DragValue::new(&mut d.max_level).range(1..=crate::basemap::OWN_MAX_LEVEL));
                    ui.end_row();
                }
                ui.label(tr(lang, "Credit"));
                ui.add(egui::TextEdit::singleline(&mut d.credit).desired_width(300.0)).on_hover_text(tr(lang, "Credit of the source on the screen, as its terms of use require."));
                ui.end_row();
            });
            let error = if wms {
                match &d.caps {
                    Some(Ok(_)) if d.asked == d.url.trim() => None,
                    Some(Err(e)) if d.asked == d.url.trim() => {
                        ui.colored_label(crate::theme::pal().accent, crate::i18n::trf(lang, "The service did not give its layers: {error}", &[("error", e)]));
                        Some("")
                    }
                    _ => Some(""),
                }
            } else {
                crate::basemap::template_error(&d.url)
            };
            if let Some(e) = error.filter(|e| !d.url.trim().is_empty() && !e.is_empty()) {
                ui.colored_label(crate::theme::pal().accent, tr(lang, e));
            }
            if wms {
                ui.label(egui::RichText::new(tr(lang, "A WMS map lies over the base map of the globe, outside the area of the layer the base map is seen. The service draws each picture anew, so the map loads slower than tiles.")).weak());
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(error.is_none(), egui::Button::new(tr(lang, "Add"))).clicked() {
                    done = Some(true);
                }
                if ui.button(tr(lang, "Cancel")).clicked() {
                    done = Some(false);
                }
            });
        });
        if !open {
            done = Some(false);
        }
        if ask_caps {
            d.asked = d.url.trim().to_string();
            d.caps = None;
            let request = (self.loaders.request)(8 << 20);
            request.start(&scene::wms::capabilities_url(&scene::wms::base_url(&d.asked)));
            self.own_caps = Some(request);
        }
        // Служба WMS - карта поверх подложки, `wms.source` модуля.
        if done == Some(true)
            && wms
            && let Some(Ok(caps)) = &d.caps
            && let Some(layer) = caps.layers.get(d.layer)
        {
            let base = scene::wms::base_url(&d.asked);
            let template = scene::wms::getmap_template(&base, &caps.version, &layer.names, scene::wms::mercator_code(&layer.crs), scene::wms::pick_format(&caps.formats));
            let saved = crate::maps::WmsSaved { name: d.name.clone(), template, bbox: layer.bbox, credit: d.credit.clone() };
            let layer_box = layer.bbox;
            self.own_dialog = None;
            if let Some(n) = crate::maps::register_wms(&saved) {
                if !self.settings.wms.contains(&saved) {
                    self.settings.wms.push(saved);
                }
                self.extras.insert(crate::layers::Extra::Map(n));
                self.set_extra(crate::layers::Extra::Map(n), true);
                // Перелёт к району службы.
                if let Some(b) = layer_box.filter(|b| !scene::wms::is_world(Some(*b))) {
                    let (lat, lon) = ((b[1] + b[3]) / 2.0, (b[0] + b[2]) / 2.0);
                    let span = globe_core::ellipsoid::EARTH.a * (b[3] - b[1]).max((b[2] - b[0]) * lat.to_radians().cos()).to_radians();
                    let now = ctx.input(|i| i.time);
                    crate::input::fly(&mut self.nav, Pose::new(lat, lon, span.clamp(2000.0, 4.0e6) * 1.5, 0.0, 0.0), now);
                }
            }
            return;
        }
        match done {
            Some(true) => {
                let own = crate::basemap::Own { name: d.name.clone(), url: d.url.trim().to_string(), max_level: d.max_level, credit: d.credit.clone() };
                self.own_dialog = None;
                if let Some(b) = crate::basemap::register(&own) {
                    if !self.settings.own.contains(&own) {
                        self.settings.own.push(own);
                    }
                    self.settings_dirty = true;
                    self.set_basemap(b);
                }
            }
            Some(false) => self.own_dialog = None,
            None => {}
        }
    }

    /// Шаг разреза Земли: загрузки, грани и полосы плит, сектор
    /// в шейдере тайла.
    fn step_cutaway(&mut self) {
        let Some(c) = &mut self.cutaway else {
            return;
        };
        let loaders = &self.loaders;
        let (meshes, saved) = c.step(self.nav.pose.distance, &|limit| (loaders.request)(limit));
        let wedge = c.uniform();
        self.labels.wedge = Some(c.wedge);
        if let Some(globe) = &mut self.globe {
            globe.set_wedge(Some(wedge));
            if let Some(m) = meshes {
                globe.set_subsurface("cutaway", m.faces.as_ref());
                globe.set_subsurface("slabs", m.slabs.as_ref());
            }
        }
        if let Some(bytes) = saved {
            self.store_crust(&bytes);
        }
    }

    /// Архив CRUST1.0 из кэша программы, как `_crust_path` модуля.
    #[cfg(not(target_arch = "wasm32"))]
    fn crust_cache(&self) -> Option<Vec<u8>> {
        std::fs::read(self.demo_cache.as_ref()?.parent()?.join(data::crust::FILE)).ok()
    }

    #[cfg(target_arch = "wasm32")]
    fn crust_cache(&self) -> Option<Vec<u8>> {
        None
    }

    /// Пришедший архив CRUST1.0 - в кэш программы.
    #[cfg(not(target_arch = "wasm32"))]
    fn store_crust(&self, bytes: &[u8]) {
        if let Some(dir) = self.demo_cache.as_ref().and_then(|d| d.parent()) {
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(dir.join(data::crust::FILE), bytes);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn store_crust(&self, _bytes: &[u8]) {}

    /// Шаг подземного режима: сетки заново, маска, прозрачность, пол
    /// камеры, окно.
    fn step_subsurface(&mut self, ctx: &egui::Context, now: f64) {
        let lang = self.lang;
        let mut remove = false;
        if let Some(sub) = &mut self.subsurface {
            let (changed, gone) = if self.check.is_none() && (self.shot.is_none() || self.shot_ui) { sub.window(ctx, lang) } else { (false, false) };
            remove = gone;
            let _ = changed;
            let (version, scale) = {
                let s = self.heights.read();
                (s.version(), s.scale())
            };
            *self.floor.write().unwrap_or_else(std::sync::PoisonError::into_inner) = sub.floor();
            if let Some(globe) = &mut self.globe {
                globe.set_opacity(sub.options.opacity);
                if sub.due(now, version, scale)
                    && let Some(model) = &sub.model
                {
                    let heights = &self.heights;
                    let ground = |pts: &[(f64, f64)]| heights.read().heights_at(pts, false);
                    let (meshes, count) = crate::subsurface::build(model, scale, &ground, sub.cut_on());
                    for (name, mesh) in &meshes {
                        globe.set_subsurface(name, mesh.as_ref());
                    }
                    sub.built(lang, now, version, scale, count);
                }
                if sub.mask_dirty {
                    let made = sub.model.as_ref().and_then(|m| crate::subsurface::mask(m, sub.cut_on()));
                    globe.set_cut(made.as_ref().map(|(b, w, h, rgba)| (*b, *w, *h, rgba.as_slice())));
                    sub.mask_dirty = false;
                }
            }
        }
        if remove {
            self.subsurface = None;
            *self.floor.write().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
            if let Some(globe) = &mut self.globe {
                globe.clear_subsurface();
                globe.set_cut(None);
                globe.set_opacity(1.0);
            }
        }
    }

    /// «Мои метки» изменились: картинки и модели удалённых меток
    /// забываются, шкала времени и надписи - по новому дереву.
    fn marks_changed(&mut self) {
        self.pictures.set(&self.marks.all_overlays());
        let keys: Vec<String> = self.pictures.states.keys().cloned().collect();
        self.screen_textures.retain(|k, _| keys.contains(k));
        if let Some(globe) = &mut self.globe {
            globe.keep_picture_images(&keys);
        }
        if self.marks.all_overlays().is_empty() {
            self.models.clear();
        }
        self.time_extent();
        self.marks_to_labels();
        self.places_dirty = true;
    }

    /// Шаг тура: камера в позе тура, на новой остановке с датой шкала
    /// встаёт на её время, в конце тур снимается.
    fn step_tour(&mut self, now: f64) {
        let Some(player) = &self.player else {
            return;
        };
        if player.done(now) {
            // Тур по кругу начинается с первой остановки.
            if player.looped {
                let pose = self.nav.pose.clone();
                if let Some(p) = &mut self.player {
                    p.play_from(0, &pose, now);
                }
                self.tour_index = None;
                return;
            }
            self.nav.show(player.pose(now));
            self.player = None;
            return;
        }
        let index = player.index(now);
        let time = index.and_then(|i| player.tour.stops.get(i)).and_then(|s| s.time.clone());
        self.nav.show(player.pose(now));
        if index != self.tour_index {
            self.tour_index = index;
            if let Some(time) = time {
                self.timebar.show_time(&time);
            }
        }
    }

    /// Шкала времени и управление туром сверху вида, подпись остановки
    /// снизу.
    fn overlays(&mut self, ctx: &egui::Context, rect: egui::Rect, now: f64) {
        let show_bar = self.timebar.open && self.timebar.extent.is_some();
        if !show_bar && self.player.is_none() {
            return;
        }
        let lang = self.lang;
        let mut act: Option<TourAct> = None;
        // Шкала времени под панелью значков от её левого края, панель тура
        // под шкалой, как у модуля (`layers.py`, `tour.py`).
        let left = self.toolbar_rect.map_or(rect.left() + 8.0, |r| r.left());
        let top = self.toolbar_rect.map_or(rect.top() + 8.0, |r| r.bottom() + 8.0);
        egui::Area::new(egui::Id::unique("planetx-top")).order(egui::Order::Foreground).pivot(egui::Align2::LEFT_TOP).fixed_pos(egui::pos2(left, top)).show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if show_bar {
                crate::theme::plate().show(ui, |ui| {
                    ui.visuals_mut().override_text_color = Some(crate::theme::pal().plate_text);
                    self.timebar.ui(ui, lang, now);
                });
            }
            if let Some(player) = &self.player {
                crate::theme::plate().show(ui, |ui| {
                    ui.visuals_mut().override_text_color = Some(crate::theme::pal().plate_text);
                    ui.horizontal(|ui| {
                        use crate::timeline::{Icon, icon_button, icon_toggle};
                        ui.spacing_mut().item_spacing.x = 3.0;
                        let duration = player.tour.duration.max(1e-9);
                        let mut share = (player.time(now) / duration) as f32;
                        ui.spacing_mut().slider_width = 220.0;
                        if ui
                            .add(egui::Slider::new(&mut share, 0.0..=1.0).show_value(false))
                            .on_hover_text(tr(lang, "How much of the tour has passed. The slider rewinds the tour. The camera stands at this point at once, the tour goes on from it."))
                            .changed()
                        {
                            act = Some(TourAct::Seek(f64::from(share)));
                        }
                        ui.label(format!("{} / {}", scene::tour::clock(player.time(now)), scene::tour::clock(player.tour.duration)));
                        if icon_button(ui, Icon::ToStart).on_hover_text(tr(lang, "Previous stop")).clicked() {
                            act = Some(TourAct::Prev);
                        }
                        let (icon, tip) = if player.paused() { (Icon::Play, tr(lang, "Continue")) } else { (Icon::Pause, tr(lang, "Pause")) };
                        if icon_button(ui, icon).on_hover_text(tip).clicked() {
                            act = Some(TourAct::Toggle);
                        }
                        if icon_button(ui, Icon::ToEnd).on_hover_text(tr(lang, "Next stop")).clicked() {
                            act = Some(TourAct::Next);
                        }
                        if icon_toggle(ui, Icon::Loop, player.looped).on_hover_text(tr(lang, "Tour in a loop. After the last stop the tour starts from the first. Pause or the cross stops it.")).clicked() {
                            act = Some(TourAct::Loop);
                        }
                        if icon_toggle(ui, Icon::Captions, player.captions).on_hover_text(tr(lang, "Stop captions on the frame. At a stop the name and the description of the placemark stand large at the bottom of the frame, in flight there is no caption.")).clicked() {
                            act = Some(TourAct::Captions);
                        }
                        if let Some((n, count)) = player.stop_number(now) {
                            let name = player.stops.get(n).map(|s| s.name.as_str()).filter(|s| !s.is_empty()).unwrap_or(tr(lang, "Untitled"));
                            ui.label(trf(lang, "Stop {n} of {count}: {name}", &[("n", &(n + 1).to_string()), ("count", &count.to_string()), ("name", name)]));
                        }
                        ui.add_space(12.0);
                        ui.label(tr(lang, "Pause"));
                        let mut pause = player.pause;
                        if ui
                            .add(egui::DragValue::new(&mut pause).range(0.0..=60.0).fixed_decimals(1).speed(0.1).suffix(tr(lang, " s")))
                            .on_hover_text(tr(lang, "How many seconds the camera stands at each stop. A new pause acts from a jump by the buttons or from a new start of the tour."))
                            .changed()
                        {
                            act = Some(TourAct::SetPause(pause));
                        }
                        if icon_button(ui, Icon::Stop).on_hover_text(tr(lang, "End tour")).clicked() {
                            act = Some(TourAct::Stop);
                        }
                    });
                    // Описание остановки под кнопками, когда подписей на
                    // кадре нет.
                    if !player.captions
                        && let Some((n, _)) = player.stop_number(now)
                        && let Some(stop) = player.stops.get(n)
                        && !stop.description.trim().is_empty()
                    {
                        ui.set_max_width(480.0f32.max(ui.min_rect().width()));
                        ui.add(egui::Label::new(caption_text(&stop.description)).wrap());
                    }
                });
            }
        });
        // Клавиши тура, как у модуля: вправо и влево - остановки, пробел -
        // пауза, Esc - конец. С Ctrl, Shift и Alt клавиши не для тура.
        if self.player.is_some() && !ctx.memory(|m| m.focused().is_some()) {
            ctx.input(|i| {
                if i.modifiers.any() {
                    return;
                }
                if i.key_pressed(egui::Key::ArrowRight) {
                    act = Some(TourAct::Next);
                } else if i.key_pressed(egui::Key::ArrowLeft) {
                    act = Some(TourAct::Prev);
                } else if i.key_pressed(egui::Key::Space) {
                    act = Some(TourAct::Toggle);
                } else if i.key_pressed(egui::Key::Escape) {
                    act = Some(TourAct::Stop);
                }
            });
        }
        if let (Some(a), Some(p)) = (act, &mut self.player) {
            match a {
                TourAct::Toggle => p.toggle(now),
                TourAct::Stop => self.player = None,
                TourAct::Prev | TourAct::Next => {
                    if let Some((n, _)) = p.stop_number(now) {
                        let k = if a == TourAct::Prev { n.saturating_sub(1) } else { n + 1 };
                        let pose = self.nav.pose.clone();
                        p.play_from(k, &pose, now);
                        self.tour_index = None;
                    }
                }
                TourAct::Seek(share) => p.seek(share, now),
                TourAct::Loop => p.looped = !p.looped,
                TourAct::Captions => p.captions = !p.captions,
                TourAct::SetPause(v) => p.pause = v,
            }
        }
        self.stop_caption(ctx, rect, now);
    }

    /// Подпись остановки внизу кадра, как `StopCaption` модуля: плашка
    /// с названием крупно и описанием под ним, по середине, 44 пикселя
    /// над нижним краем.
    fn stop_caption(&self, ctx: &egui::Context, rect: egui::Rect, now: f64) {
        let Some(player) = self.player.as_ref().filter(|p| p.captions) else {
            return;
        };
        let Some((i, alpha)) = player.tour.caption_at(player.time(now)) else {
            return;
        };
        let Some(stop) = player.tour.stops.get(i) else {
            return;
        };
        let a = alpha as f32;
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("planetx-caption")));
        let width = (rect.width() * 0.6).max(360.0).min(rect.width() - 16.0);
        let pad = 14.0;
        let title = painter.layout(
            stop.name.clone(),
            egui::FontId::new(28.0, egui::FontFamily::Name(crate::fonts::SEMIBOLD.into())),
            egui::Color32::WHITE.gamma_multiply(a),
            width - 2.0 * pad,
        );
        let text = caption_text(&stop.description);
        let mut job = egui::text::LayoutJob::simple(text, egui::FontId::proportional(17.0), egui::Color32::from_rgb(225, 232, 240).gamma_multiply(a), width - 2.0 * pad);
        job.halign = egui::Align::Center;
        let desc = painter.layout_job(job);
        let gap = if desc.size().y > 0.0 { 6.0 } else { 0.0 };
        let height = title.size().y + gap + desc.size().y + 2.0 * pad;
        let box_rect = egui::Rect::from_min_size(egui::pos2(rect.center().x - width / 2.0, rect.bottom() - 44.0 - height), egui::vec2(width, height));
        painter.rect_filled(box_rect, 8.0, egui::Color32::from_rgba_unmultiplied(15, 25, 40, (175.0 * a) as u8));
        painter.galley(egui::pos2(box_rect.center().x - title.size().x / 2.0, box_rect.top() + pad), title.clone(), egui::Color32::WHITE);
        // У текста с выравниванием по середине опорная точка - середина строк.
        painter.galley(egui::pos2(box_rect.center().x, box_rect.top() + pad + title.size().y + gap), desc, egui::Color32::WHITE);
    }
    /// Точки меток - в надписи.
    fn marks_to_labels(&mut self) {
        let points: Vec<(String, f64, f64, [u8; 4])> =
            self.marks.points().into_iter().map(|p| (p.name, p.lat, p.lon, p.icon)).collect();
        self.labels.set_marks(&points);
    }

    /// Левая панель меток: название сцены, дерево с флажками. Двойной
    /// щелчок - перелёт к метке или папке.
    fn marks_panel(&mut self, ui: &mut egui::Ui, now: f64) {
        // Действия, которые просили карточка и окна прошлого кадра.
        let mut actions = std::mem::take(&mut self.queued);
        let lang = self.lang;
        let title = tr(lang, "Places");
        let hint = tr(
            lang,
            "Open or drop a .planetx scene, KML, KMZ, GeoPackage layers, a GeoTIFF elevation raster or a LAS, LAZ, COPC point cloud into the window.",
        );
        let layers_title = tr(lang, "Layers");
        // Во весь экран панель прячется, как у модуля.
        let full = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
        if !self.panel_open || full {
            self.tree_actions(ui, actions, now);
            return;
        }
        // Сообщение видно MESSAGE_TIME секунд.
        let now_message = self.open_message.clone();
        match (&now_message, &self.message_seen) {
            (Some(text), Some((seen, at))) if text == seen => {
                if now - at > MESSAGE_TIME {
                    self.open_message = None;
                    self.message_seen = None;
                } else {
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(0.5));
                }
            }
            (Some(text), _) => self.message_seen = Some((text.clone(), now)),
            (None, _) => self.message_seen = None,
        }
        let status = self.status_text();
        egui::Panel::left("marks").resizable(true).default_size(300.0).show(ui, |ui| {
            ui.add_space(6.0);
            self.search_row(ui, now);
            ui.add_space(6.0);
            // Строка состояния внизу панели на две строки, как у модуля:
            // камера и точка под курсором.
            let status_height = 40.0;
            // Под раздел «Слои» - его высота в прошлом кадре.
            let layers_height = self.layers_height;
            if crate::theme::section(ui, "marks", title) {
                // Дереву - заданная границей высота, иначе место над
                // «Слоями». Разделам остаётся не меньше SECTION_MIN.
                let free = ui.available_height() - status_height - 34.0;
                let chosen = self.settings.places_height;
                let room = if chosen > 0.0 { chosen } else { free - layers_height };
                let room = room.min(free - SECTION_MIN).max(SECTION_MIN);
                let root = self.marks.root(marks::root_name(lang));
                let empty = root.children.is_empty();
                let shown = egui::ScrollArea::vertical()
                    .max_height(room)
                    .auto_shrink([false, chosen <= 0.0])
                    .show(ui, |ui| {
                        crate::marks::root_ui(ui, root, lang, &mut self.tree_state, &mut actions);
                    })
                    .inner_rect
                    .height();
                if empty {
                    ui.label(egui::RichText::new(hint).color(crate::theme::pal().muted));
                }
                self.tour_row(ui, &mut actions);
                self.splitter(ui, shown);
            }
            let top = ui.cursor().top();
            if crate::theme::section(ui, "layers", layers_title) {
                let mut out = Vec::new();
                let mut menu = self.layers_menu.take();
                let state = self.layers_state();
                let room = (ui.available_height() - status_height - 8.0).max(SECTION_MIN);
                egui::ScrollArea::vertical().id_salt("layers").max_height(room).auto_shrink([false, true]).show(ui, |ui| {
                    crate::layers::panel(ui, lang, &state, &mut menu, &mut out);
                });
                self.layers_menu = menu;
                self.apply_layers(out);
            }
            self.layers_height = ui.cursor().top() - top;
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(status).color(crate::theme::pal().muted));
            });
        });
        self.tree_actions(ui, actions, now);
    }

    /// Граница между «Моими метками» и «Слоями»: тянется мышью, как
    /// разделитель панели модуля. Двойной щелчок возвращает высоту
    /// по умолчанию. room - высота дерева в этом кадре.
    fn splitter(&mut self, ui: &mut egui::Ui, room: f32) {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), egui::Sense::click_and_drag());
        let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
        let active = response.hovered() || response.dragged();
        let color = if active { crate::theme::pal().accent } else { crate::theme::pal().muted.gamma_multiply(0.4) };
        ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(if active { 2.0 } else { 1.0 }, color));
        if response.dragged() {
            self.settings.places_height = (room + response.drag_delta().y).clamp(SECTION_MIN, crate::settings::PLACES_MAX);
        }
        if response.drag_stopped() {
            self.settings_dirty = true;
        }
        if response.double_clicked() {
            self.settings.places_height = 0.0;
            self.settings_dirty = true;
        }
    }

    /// Строка под деревом «Моих меток», как у модуля: ползунок
    /// непрозрачности выделенного и кнопка ▶ тура.
    fn tour_row(&mut self, ui: &mut egui::Ui, actions: &mut Vec<crate::marks::Action>) {
        let lang = self.lang;
        let selected = self.tree_state.selected.clone();
        // Выделение сменилось: цвета выделенного и содержимого папок
        // снимаются, ползунок встаёт на их непрозрачность.
        if self.opacity.as_ref().map(|o| &o.0) != selected.as_ref() {
            self.opacity = selected.as_ref().map(|path| {
                let items = self.marks.colors_of(path);
                let base = scene::opacity::selection_opacity(&items.iter().map(|(_, c)| c.clone()).collect::<Vec<_>>());
                (path.clone(), items, base, base)
            });
        }
        let tour = self.tour_key(selected.as_deref());
        ui.horizontal(|ui| {
            let usable = self.opacity.as_ref().is_some_and(|o| !o.1.is_empty());
            let mut value = self.opacity.as_ref().filter(|_| usable).map_or(100.0, |o| (o.3 * 100.0).round());
            ui.spacing_mut().slider_width = (ui.available_width() - 40.0).max(40.0);
            let slider = ui
                .add_enabled(usable, egui::Slider::new(&mut value, 0.0..=100.0).show_value(false))
                .on_hover_text(tr(lang, "Opacity of the selected placemarks, paths, polygons, pictures and folder contents. To the left - more transparent. The value is kept in My Places and in KML."));
            if slider.changed()
                && let Some((_, items, base, current)) = &mut self.opacity
            {
                *current = value / 100.0;
                let colors: Vec<(Vec<usize>, Vec<Option<scene::Rgba>>)> =
                    items.iter().map(|(path, c)| (path.clone(), scene::opacity::scaled(c, *base, *current))).collect();
                self.marks.set_colors(&colors);
                self.marks_changed();
            }
            let tip = match &tour {
                Some(p) if p.is_empty() => tr(lang, "Tour over the checked places"),
                Some(p) if matches!(self.marks.tree.as_ref().and_then(|t| marks::item_at(t, p)), Some(scene::kml::Item::Folder(_))) => {
                    tr(lang, "Tour over the checked places of the folder")
                }
                Some(_) => tr(lang, "Tour along the selected path"),
                None => tr(lang, "Tour: select the My Places folder or a path in it"),
            };
            if ui.add_enabled(tour.is_some(), egui::Button::new("▶")).on_hover_text(tip).on_disabled_hover_text(tip).clicked()
                && let Some(path) = tour
            {
                actions.push(crate::marks::Action::Tour(path));
            }
        });
    }

    /// Что облетит кнопка тура, `_tour_key` модуля: [] - все «Мои метки»,
    /// путь папки - её метки, путь линии - вдоль неё. У метки тур идёт по
    /// её папке. None - выделения нет.
    fn tour_key(&self, selected: Option<&[usize]>) -> Option<Vec<usize>> {
        let path = selected?;
        if path.is_empty() {
            return Some(Vec::new());
        }
        let tree = self.marks.tree.as_ref()?;
        match marks::item_at(tree, path)? {
            scene::kml::Item::Folder(_) => Some(path.to_vec()),
            scene::kml::Item::Place(p) if p.kind == scene::features::Kind::Line => Some(path.to_vec()),
            _ => self.tour_key(Some(&path[..path.len() - 1])),
        }
    }

    /// Клавиши и действия дерева «Моих меток».
    fn tree_actions(&mut self, ui: &mut egui::Ui, mut actions: Vec<crate::marks::Action>, now: f64) {
        // Delete удаляет выделенную строку, F2 переименовывает, как
        // в списке модуля. Пока пишется текст, клавиши - для текста.
        let typing = ui.ctx().memory(|m| m.focused().is_some());
        if !typing && let Some(path) = self.tree_state.selected.clone() {
            if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                actions.push(crate::marks::Action::Remove(path));
            } else if ui.input(|i| i.key_pressed(egui::Key::F2)) {
                actions.push(crate::marks::Action::StartRename(path));
            } else if ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy))) {
                actions.push(crate::marks::Action::Copy(path));
            }
        }
        // Ctrl+V: текст KML из буфера обмена - в выделенную папку, как
        // `paste_places` модуля.
        if !typing {
            let pasted = ui.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Paste(text) => Some(text.clone()),
                    _ => None,
                })
            });
            if let Some(text) = pasted {
                self.paste_kml(&text);
            }
        }
        for action in actions {
            match action {
                crate::marks::Action::Toggled(path) => {
                    self.marks.toggled(&path);
                    let window = self.timebar.range;
                    let open = self.timebar.open;
                    self.time_extent();
                    if self.timebar.extent.is_some() {
                        self.timebar.open = open;
                        self.timebar.range = window;
                    }
                    self.marks_to_labels();
                }
                crate::marks::Action::Fly(path) => {
                    self.player = None;
                    let camera = &self.nav.camera;
                    if let Some(pose) = self.marks.pose_of(&path, camera.fov_y, camera.aspect()) {
                        crate::input::fly(&mut self.nav, pose, now);
                    }
                    if let Some(time) = self.marks.time_of(&path) {
                        self.timebar.show_time(&time);
                    }
                }
                crate::marks::Action::Tour(path) => {
                    let camera = &self.nav.camera;
                    if let Some((name, stops)) = self.marks.tour_of(&path, camera.fov_y, camera.aspect()) {
                        self.player = Player::new(&name, &self.nav.pose, stops, camera.fov_y, now);
                        self.tour_index = None;
                        self.nav.stop();
                    }
                }
                // Удаление - после вопроса, как `_confirm_remove` модуля.
                crate::marks::Action::Remove(path) => {
                    let item = self.marks.tree.as_ref().and_then(|t| marks::item_at(t, &path));
                    self.confirm = match item {
                        Some(scene::kml::Item::Folder(f)) => Some((
                            tr(self.lang, "Delete folder").to_string(),
                            trf(self.lang, "Delete folder “{name}” with all its contents?", &[("name", &f.name)]),
                            Confirmed::Remove(path),
                        )),
                        Some(scene::kml::Item::Place(scene::kml::Place { name, .. }) | scene::kml::Item::Overlay(scene::kml::Overlay { name, .. })) => Some((
                            tr(self.lang, "Delete placemark").to_string(),
                            trf(self.lang, "Delete \"{name}\" from My Places?", &[("name", name)]),
                            Confirmed::Remove(path),
                        )),
                        None => None,
                    };
                }
                crate::marks::Action::Cut(path) => {
                    self.copy_marks(ui.ctx(), &path);
                    if self.marks.remove(&path) {
                        self.tree_state.selected = None;
                        self.player = None;
                        self.marks_changed();
                    }
                }
                crate::marks::Action::Paste(folder) => {
                    if let Some(text) = self.clip_kml.clone() {
                        self.paste_kml_into(&folder, &text);
                    }
                }
                crate::marks::Action::Sort(path) => {
                    if self.marks.sort(&path) {
                        self.tree_state.selected = None;
                        self.marks_changed();
                    }
                }
                crate::marks::Action::ViewSnapshot(path) => {
                    let p = &self.nav.pose;
                    if self.marks.set_view(&path, scene::lookat::make(p.lat, p.lon, p.distance, p.heading, p.tilt)) {
                        self.marks_changed();
                    }
                }
                crate::marks::Action::Add(folder, what) => {
                    use crate::marks::AddWhat;
                    self.tree_state.selected = Some(folder.clone());
                    match what {
                        AddWhat::Folder => self.queued.push(crate::marks::Action::NewFolder(folder)),
                        AddWhat::Point => self.set_tool(crate::tool::Tool::place(crate::ruler::Mode::Point)),
                        AddWhat::Line => self.set_tool(crate::tool::Tool::place(crate::ruler::Mode::Path)),
                        AddWhat::Polygon => self.set_tool(crate::tool::Tool::place(crate::ruler::Mode::Polygon)),
                        AddWhat::RecordedTour => {
                            if self.recording.is_none() {
                                self.toggle_record(now);
                            }
                        }
                        AddWhat::Ground => self.pick_for(ui.ctx(), Pick::New(scene::overlays::Kind::Ground, folder)),
                        AddWhat::Photo => self.pick_for(ui.ctx(), Pick::New(scene::overlays::Kind::Photo, folder)),
                        AddWhat::Screen => self.pick_for(ui.ctx(), Pick::New(scene::overlays::Kind::Screen, folder)),
                        AddWhat::Model => self.pick_for(ui.ctx(), Pick::New(scene::overlays::Kind::Model, folder)),
                        AddWhat::Link => {
                            self.link_dialog = Some(crate::netlinks::LinkDialog { path: None, parent: folder, name: tr(self.lang, "Network link").to_string(), link: String::new(), refresh: 0.0 });
                        }
                    }
                }
                crate::marks::Action::OverlayProperties(path) => self.open_overlay_props(path, now),
                crate::marks::Action::LinkProperties(path) => {
                    if let Some(f) = self.marks.folder(&path) {
                        self.link_dialog = Some(crate::netlinks::LinkDialog { path: Some(path.clone()), parent: Vec::new(), name: f.name.clone(), link: f.link.clone(), refresh: f.refresh });
                    }
                }
                crate::marks::Action::ReloadLink(path) => self.netlinks.reload(&path),
                crate::marks::Action::NewFolderAfter(path) => {
                    let parent = path[..path.len().saturating_sub(1)].to_vec();
                    let index = path.last().map_or(0, |i| i + 1);
                    let name = scene::placetree::numbered_name(tr(self.lang, "New folder"), &self.marks.names());
                    let at = self.marks.insert_at(&parent, index, scene::kml::Item::Folder(scene::kml::Folder::new(&name)), marks::root_name(self.lang));
                    self.tree_state.selected = Some(at.clone());
                    self.tree_state.renaming = Some((at, name));
                    self.marks_changed();
                }
                crate::marks::Action::FolderProperties(path) => {
                    self.close_side();
                    if let Some(scene::kml::Item::Folder(f)) = self.marks.tree.as_ref().and_then(|t| marks::item_at(t, &path)) {
                        self.folder_props = Some(crate::folderprops::FolderProps::new(path, f));
                    }
                }
                crate::marks::Action::Clear => {
                    let count = self.marks.names().len();
                    self.confirm = Some((
                        tr(self.lang, "Clear My Places…").trim_end_matches('…').to_string(),
                        trf(self.lang, "Delete all placemarks and folders of My Places, placemarks in all {count}? Deletion cannot be undone.", &[("count", &count.to_string())]),
                        Confirmed::Clear,
                    ));
                }
                crate::marks::Action::Open => self.pick_file(ui.ctx(), now),
                crate::marks::Action::Properties(path) => {
                    self.close_side();
                    if let Some(place) = self.marks.place(&path) {
                        self.tool = Some(crate::tool::Tool::edit(path, place));
                    }
                }
                crate::marks::Action::Profile(path) => {
                    if let Some(place) = self.marks.place(&path) {
                        self.profile = Some(crate::profile::ProfileWindow { title: place.name.clone(), points: place.points.clone(), hovered: None, ruler: false });
                    }
                }
                crate::marks::Action::Section(path) => {
                    if let Some(place) = self.marks.place(&path) {
                        let (title, points) = (place.name.clone(), place.points.clone());
                        match &mut self.section {
                            Some(s) => s.set_source(title, points),
                            None => self.section = Some(crate::section::SectionWindow::new(title, points, self.crust_cache().as_deref())),
                        }
                    }
                }
                crate::marks::Action::Catchment(path) => {
                    if let Some(place) = self.marks.place(&path) {
                        let polygon = place.kind == scene::features::Kind::Polygon;
                        let name = if place.name.is_empty() { tr(self.lang, "Untitled").to_string() } else { place.name.clone() };
                        let points = place.points.clone();
                        let radius = self.nav.camera.ellipsoid.a;
                        let max_level = terrain_source(&self.planet).map_or(terrain::MAX_LEVEL, |s| s.max_level);
                        self.hydro.catchment_of(points, polygon, name, radius, max_level, now);
                    }
                }
                crate::marks::Action::FlowLines(path) => {
                    if let Some(place) = self.marks.place(&path) {
                        let polygon = place.kind == scene::features::Kind::Polygon;
                        let name = if place.name.is_empty() { tr(self.lang, "Untitled").to_string() } else { place.name.clone() };
                        let points = place.points.clone();
                        let radius = self.nav.camera.ellipsoid.a;
                        self.hydro.flow_from(points, polygon, name, radius);
                    }
                }
                crate::marks::Action::Copy(path) => {
                    self.copy_marks(ui.ctx(), &path);
                }
                crate::marks::Action::SaveView(path) => {
                    let name = scene::placetree::numbered_name(tr(self.lang, "View"), &self.marks.names());
                    self.add_view_place(&path, &name);
                }
                crate::marks::Action::NewFolder(path) => {
                    // «Новая папка 2», сразу на переименование, как у модуля.
                    let name = scene::placetree::numbered_name(tr(self.lang, "New folder"), &self.marks.names());
                    let at = self.marks.insert(&path, scene::kml::Item::Folder(scene::kml::Folder::new(&name)), marks::root_name(self.lang));
                    self.tree_state.selected = Some(at.clone());
                    self.tree_state.renaming = Some((at, name));
                    self.marks_changed();
                }
                crate::marks::Action::StartRename(path) => {
                    let name = self.marks.tree.as_ref().and_then(|t| marks::item_at(t, &path)).map(|item| match item {
                        scene::kml::Item::Folder(f) => f.name.clone(),
                        scene::kml::Item::Place(p) => p.name.clone(),
                        scene::kml::Item::Overlay(o) => o.name.clone(),
                    });
                    self.tree_state.renaming = name.map(|n| (path, n));
                }
                crate::marks::Action::Rename(path, name) => {
                    self.tree_state.renaming = None;
                    if self.marks.rename(&path, &name) {
                        self.marks_changed();
                    }
                }
                crate::marks::Action::CancelRename => self.tree_state.renaming = None,
                crate::marks::Action::SaveKml(path) => self.save_marks(&path, crate::export::Kind::Kml),
                crate::marks::Action::SaveScene(path) => self.save_marks(&path, crate::export::Kind::Scene),
                crate::marks::Action::Move(from, to, into) => {
                    if self.marks.move_to(&from, &to, into) {
                        self.tree_state.selected = None;
                        self.player = None;
                        self.marks_changed();
                    }
                }
            }
        }
    }

    /// Сохранить строку «Моих меток» по пути: KML или KMZ, или сцену
    /// с текущей камерой. Итог - в строке сообщения.
    fn save_marks(&mut self, path: &[usize], kind: crate::export::Kind) {
        let Some(tree) = self.marks.export_tree(path) else {
            return;
        };
        self.open_message = match crate::export::save(kind, &tree, &self.nav.pose) {
            Ok(Some(to)) => Some(trf(self.lang, "Saved: {to}", &[("to", &to)])),
            Ok(None) => None,
            Err(e) => Some(trf(self.lang, "Not saved: {e}", &[("e", &e)])),
        };
    }

    /// Выбор файла сцены. В программе - диалог системы, на странице -
    /// выбор файла браузером, байты приходят в ящик брошенных файлов.
    fn pick_file(&mut self, ctx: &egui::Context, now: f64) {
        const KINDS: [&str; 9] = ["planetx", "kml", "kmz", "gpkg", "tif", "tiff", "las", "laz", "csv"];
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            if let Some(path) = rfd::FileDialog::new().add_filter("PlanetX, KML, KMZ, GeoPackage, GeoTIFF, LAS, LAZ, CSV", &KINDS).pick_file() {
                let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
                match std::fs::read(&path) {
                    Ok(bytes) => {
                    self.open_dir = path.parent().map(std::path::Path::to_path_buf);
                    // Ошибку открытия показывает панель.
                    let _ = self.open_file(&bytes, &name, Some(now));
                }
                    Err(e) => self.open_message = Some(format!("{name}: {e}")),
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = now;
            let (slot, ctx) = (self.dropped_box.clone(), ctx.clone());
            wasm_bindgen_futures::spawn_local(async move {
                if let Some(file) = rfd::AsyncFileDialog::new().add_filter("PlanetX, KML, KMZ, GeoPackage, GeoTIFF, LAS, LAZ, CSV", &KINDS).pick_file().await {
                    let bytes = file.read().await;
                    if let Ok(mut s) = slot.lock() {
                        s.push((Ok(bytes), file.file_name()));
                    }
                }
                ctx.request_repaint();
            });
        }
    }

    /// Выбор картинки или модели для наложения, как окна выбора файла
    /// `add_overlay`, `add_model` и «Обзор…» модуля. У модели в программе
    /// пакет собирается из файла и файлов рядом с ним, на странице -
    /// из всех выбранных файлов.
    fn pick_for(&mut self, ctx: &egui::Context, pick: Pick) {
        let model = match &pick {
            Pick::New(kind, _) => *kind == scene::overlays::Kind::Model,
            Pick::Browse => self.overlay_props.as_ref().is_some_and(|p| p.edit.kind() == scene::overlays::Kind::Model),
            Pick::TrackModel(_) => true,
        };
        let lang = self.lang;
        let (title, kinds): (&str, &[&str]) = if model {
            (tr(lang, "3D models (*.dae *.gltf *.glb *.obj *.kmz)"), &["dae", "gltf", "glb", "obj", "kmz"])
        } else {
            (tr(lang, "Images (*.png *.jpg *.jpeg *.gif)"), &["png", "jpg", "jpeg", "gif"])
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            let Some(path) = rfd::FileDialog::new().add_filter(title, kinds).pick_file() else {
                return;
            };
            let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let file = PickedFile { name, path: Some(path.to_string_lossy().into_owned()), bytes };
                    if let Ok(mut s) = self.picked_box.lock() {
                        s.push((pick, vec![file]));
                    }
                }
                Err(e) => self.open_message = Some(format!("{name}: {e}")),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let (slot, ctx) = (self.picked_box.clone(), ctx.clone());
            let title = title.to_string();
            let kinds: Vec<String> = kinds.iter().map(|k| k.to_string()).collect();
            wasm_bindgen_futures::spawn_local(async move {
                let dialog = rfd::AsyncFileDialog::new().add_filter(&title, &kinds);
                // У модели - файл модели и его картинки сразу.
                let files = if model { dialog.pick_files().await.unwrap_or_default() } else { dialog.pick_file().await.into_iter().collect() };
                let mut read = Vec::new();
                for file in files {
                    read.push(PickedFile { name: file.file_name(), path: None, bytes: file.read().await });
                }
                if !read.is_empty()
                    && let Ok(mut s) = slot.lock()
                {
                    s.push((pick, read));
                }
                ctx.request_repaint();
            });
        }
    }

    /// Пакет модели из выбранных файлов: главный - первый файл модели,
    /// остальные - по ссылкам из него. В программе файлы рядом читаются
    /// с диска. Отдаёт имя главного файла, путь на диске и пакет.
    fn model_package(files: &[PickedFile]) -> Option<(String, Option<String>, Vec<u8>)> {
        let main = files.iter().find(|f| scene::models::EXTENSIONS.iter().any(|e| f.name.to_lowercase().ends_with(e)))?;
        let folder = main.path.as_deref().and_then(|p| std::path::Path::new(p).parent().map(std::path::Path::to_path_buf));
        let fetch = |reference: &str| -> Option<Vec<u8>> {
            let short = reference.rsplit(['/', '\\']).next().unwrap_or(reference);
            if let Some(f) = files.iter().find(|f| f.name == reference || f.name == short) {
                return Some(f.bytes.clone());
            }
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(dir) = &folder {
                return std::fs::read(dir.join(reference.replace('\\', "/"))).ok();
            }
            let _ = &folder;
            None
        };
        let package = scene::models::collect(&main.name, &main.bytes, &fetch, &std::collections::BTreeMap::new()).ok()?;
        Some((main.name.clone(), main.path.clone(), package))
    }

    /// Выбранные файлы: новое наложение и его окно свойств или новая
    /// картинка открытого окна.
    fn picked(&mut self, now: f64) {
        let arrived: Vec<(Pick, Vec<PickedFile>)> = self.picked_box.lock().map(|mut s| s.drain(..).collect()).unwrap_or_default();
        for (pick, files) in arrived {
            let Some(first) = files.first() else {
                continue;
            };
            let (first_name, first_bytes, first_path) = (first.name.clone(), first.bytes.clone(), first.path.clone());
            let model = files.iter().any(|f| scene::models::EXTENSIONS.iter().any(|e| f.name.to_lowercase().ends_with(e)));
            // KMZ модели открывается как файл меток: модель в нём уже стоит.
            if first_name.to_lowercase().ends_with(".kmz") {
                let _ = self.open_file(&first_bytes, &first_name, Some(now));
                continue;
            }
            let stem = first_name.rsplit_once('.').map_or(first_name.as_str(), |(s, _)| s).to_string();
            match pick {
                Pick::New(kind, folder) => {
                    let overlay = if model || kind == scene::overlays::Kind::Model {
                        let Some((name, path, package)) = Self::model_package(&files) else {
                            self.open_message = Some(trf(self.lang, "The model cannot be read: {name}", &[("name", &first_name)]));
                            continue;
                        };
                        let mut o = crate::overlayprops::new_overlay(scene::overlays::Kind::Model, &stem, &self.nav, 1.0);
                        // Модель - ссылкой на файл, как Link у Google Earth:
                        // правка файла видна на глобусе. На странице пути нет,
                        // модель - в файле меток.
                        match path {
                            Some(p) => {
                                self.linked_models.insert(p.clone(), (file_stamp_of(&p), now, Some(Arc::new(package))));
                                o.href = p;
                            }
                            None => {
                                o.href = name;
                                o.image = Some(package);
                            }
                        }
                        o
                    } else {
                        let dims = image::ImageReader::new(std::io::Cursor::new(&first_bytes)).with_guessed_format().ok().and_then(|r| r.into_dimensions().ok());
                        let Some((w, h)) = dims else {
                            self.open_message = Some(trf(self.lang, "The image cannot be read: {name}", &[("name", &first_name)]));
                            continue;
                        };
                        let mut o = crate::overlayprops::new_overlay(kind, &stem, &self.nav, f64::from(w) / f64::from(h.max(1)));
                        o.href = first_name.clone();
                        o.image = Some(first_bytes.clone());
                        o
                    };
                    let at = self.marks.insert(&folder, scene::kml::Item::Overlay(overlay), marks::root_name(self.lang));
                    self.tree_state.selected = Some(at.clone());
                    self.marks_changed();
                    self.open_overlay_props(at, now);
                }
                Pick::Browse => {
                    let package = if model { Self::model_package(&files) } else { Some((first_name.clone(), first_path, first_bytes)) };
                    match (package, &mut self.overlay_props) {
                        (Some((name, path, bytes)), Some(props)) => props.set_file(bytes, &name, path),
                        (None, _) => self.open_message = Some(trf(self.lang, "The model cannot be read: {name}", &[("name", &first_name)])),
                        _ => {}
                    }
                }
                Pick::TrackModel(n) => match Self::model_package(&files) {
                    Some((name, _, package)) => self.tracks.set_model(n, crate::tracks::Model { name, package: Arc::new(package) }),
                    None => self.open_message = Some(trf(self.lang, "The model cannot be read: {name}", &[("name", &first_name)])),
                },
            }
        }
    }

    /// Окно свойств наложения по пути. У модели по ссылке пакет читается
    /// для рамки на глобусе.
    fn open_overlay_props(&mut self, path: Vec<usize>, now: f64) {
        self.close_side();
        let Some(overlay) = self.marks.overlay(&path) else {
            return;
        };
        let mut props = crate::overlayprops::OverlayProps::new(path, overlay);
        if props.bytes.is_none()
            && let Some(package) = self.linked_package(&props.edit.href.clone(), now)
        {
            props.set_model_bytes(package.to_vec());
        }
        self.overlay_props = Some(props);
    }

    /// Пакет модели по ссылке на файл в программе или None. Пакет
    /// помнится до смены времени правки файла.
    fn linked_package(&mut self, href: &str, now: f64) -> Option<Arc<Vec<u8>>> {
        if href.is_empty() || crate::pictures::is_remote(href) || cfg!(target_arch = "wasm32") {
            return None;
        }
        let path = crate::netlinks::local_path(href)?;
        if let Some((_, _, Some(p))) = self.linked_models.get(href) {
            return Some(p.clone());
        }
        let bytes = read_local(&path)?;
        let name = std::path::Path::new(&path).file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let file = PickedFile { name, path: Some(path.clone()), bytes };
        let package = Self::model_package(std::slice::from_ref(&file)).map(|(_, _, p)| Arc::new(p));
        self.linked_models.insert(href.to_string(), (file_stamp_of(&path), now, package.clone()));
        package
    }

    /// Наложения кадра для моделей: у модели по ссылке на файл пакет
    /// читается с диска, через промежуток обновления файл проверяется
    /// и читается заново, если изменился, как у модуля.
    fn model_overlays(&mut self, now: f64) -> Vec<scene::kml::Overlay> {
        let mut list: Vec<scene::kml::Overlay> = self.marks.overlays().to_vec();
        list.extend(self.tracks.overlays());
        for o in list.iter_mut().filter(|o| o.kind() == scene::overlays::Kind::Model && o.image.as_ref().is_none_or(Vec::is_empty)) {
            if let Some(p) = self.linked_package(&o.href, now) {
                o.image = Some(p.to_vec());
            }
        }
        list
    }

    /// Модели по ссылке с обновлением: через промежуток файл
    /// проверяется, изменённый читается заново и модели собираются
    /// заново.
    fn check_linked_models(&mut self, now: f64) {
        let mut stale = Vec::new();
        for o in self.marks.overlays().iter().filter(|o| o.kind() == scene::overlays::Kind::Model && o.overlay.refresh > 0.0) {
            let step = crate::netlinks::interval(o.overlay.refresh);
            if let Some(entry) = self.linked_models.get_mut(&o.href)
                && now - entry.1 >= step
            {
                entry.1 = now;
                if crate::netlinks::local_path(&o.href).and_then(|p| file_stamp_of(&p)) != entry.0 {
                    stale.push(o.href.clone());
                }
            }
        }
        if !stale.is_empty() {
            for href in stale {
                self.linked_models.remove(&href);
            }
            self.models.clear();
            self.marks.refresh();
        }
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

    /// Окна наложения и сетевой ссылки, как `_open_overlay_properties`
    /// и `add_link` модуля.
    fn overlay_windows(&mut self, ctx: &egui::Context) {
        let lang = self.lang;
        if let Some(props) = &mut self.overlay_props {
            let outcome = self.side.overlay.take().unwrap_or(crate::overlayprops::Outcome::Open);
            // Правки видны на глобусе сразу.
            if std::mem::take(&mut props.changed) {
                let (path, edit) = (props.path.clone(), props.edit.clone());
                self.marks.set_overlay(&path, edit);
                self.marks_changed();
            }
            match outcome {
                crate::overlayprops::Outcome::Open => {}
                crate::overlayprops::Outcome::Browse => self.pick_for(ctx, Pick::Browse),
                crate::overlayprops::Outcome::Ok => {
                    if let Some(props) = self.overlay_props.take() {
                        self.marks.set_overlay(&props.path, props.edit);
                        self.marks_changed();
                    }
                }
                crate::overlayprops::Outcome::Cancel => {
                    if let Some(props) = self.overlay_props.take() {
                        self.marks.set_overlay(&props.path, props.original);
                        self.marks_changed();
                    }
                }
            }
        }
        if let Some(dialog) = &mut self.link_dialog {
            match dialog.window(ctx, lang) {
                None => {}
                Some(false) => self.link_dialog = None,
                Some(true) => {
                    let Some(d) = self.link_dialog.take() else {
                        return;
                    };
                    let (name, link) = (d.name.trim().to_string(), d.link.trim().to_string());
                    match d.path {
                        Some(path) => {
                            let refresh = d.refresh;
                            if self.marks.edit_folder(&path, |f| {
                                if !name.is_empty() {
                                    f.name = name;
                                }
                                f.link = link;
                                f.refresh = refresh;
                            }) {
                                self.netlinks.reload(&path);
                                self.marks_changed();
                            }
                        }
                        None if !link.is_empty() => {
                            let mut folder = scene::kml::Folder::new(if name.is_empty() { &link } else { &name });
                            folder.link = link;
                            folder.refresh = d.refresh;
                            let at = self.marks.insert(&d.parent, scene::kml::Item::Folder(folder), marks::root_name(lang));
                            self.tree_state.selected = Some(at);
                            self.marks_changed();
                        }
                        None => {}
                    }
                }
            }
        }
    }

    /// Шаг полёта, `FlightSim._tick` модуля: клавиши, физика, камера,
    /// модель. Касание земли - надпись, через END_SHOW секунд режим
    /// снимается.
    fn step_flight(&mut self, ctx: &egui::Context, now: f64) {
        let Some(flight) = &mut self.flight else {
            return;
        };
        let (escape, view, hud) = ctx.input(|i| (i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::V), i.key_pressed(egui::Key::H)));
        if hud {
            flight.hud = !flight.hud;
        }
        if view {
            let tilt = self.nav.pose.tilt;
            let back = !flight.cockpit;
            flight.set_cockpit(back, tilt);
            if !back {
                let p = &flight.plane;
                self.nav.air = false;
                self.nav.show(Pose::new(p.lat, p.lon, crate::flight::CHASE_REACH, p.heading, flight.chase_tilt()));
            }
        }
        let store = self.heights.read();
        let scale = store.scale();
        let true_ground = |lat: f64, lon: f64| if scale != 0.0 { (store.height_at(lat, lon) / scale).max(0.0) } else { 0.0 };
        let ground = true_ground(flight.plane.lat, flight.plane.lon);
        let going = !escape && flight.step(now, crate::flight::controls(ctx), ground);
        let screen_ground = |lat: f64, lon: f64| store.height_at(lat, lon).max(0.0);
        if going {
            let alt = flight.plane.alt * scale;
            match flight.camera(alt, &screen_ground, self.nav.pose.tilt) {
                crate::flight::Camera::Chase(la, lo, distance, look, tilt) => self.nav.show(Pose::new(la, lo, distance, look, tilt)),
                crate::flight::Camera::Cockpit { lat, lon, h, distance, heading, tilt, roll } => {
                    self.nav.stop();
                    let mut pose = Pose::new(lat, lon, distance, heading, tilt).with_h(h).with_terrain(self.nav.terrain.clone());
                    pose.roll = roll;
                    self.nav.set_pose(pose);
                    self.nav.air = true;
                }
            }
            if let Some(globe) = &mut self.globe {
                if flight.fresh {
                    flight.fresh = false;
                    globe.set_flight_model(Some(&flight.mesh()));
                }
                let center = self.nav.camera.ellipsoid.geodetic_to_ecef(flight.plane.lat, flight.plane.lon, alt);
                globe.place_flight(render::ModelPlace { center, orient: flight.orient(), sun: crate::models::sun_at(flight.plane.lat, flight.plane.lon), shown: !flight.cockpit });
            }
            ctx.request_repaint();
        }
        drop(store);
        if !going {
            self.flight = None;
            self.nav.air = false;
            if let Some(globe) = &mut self.globe {
                globe.set_flight_model(None);
            }
        }
    }

    /// Шаг живых строк, `LiveManager` модуля: опрос служб, точки
    /// и подписи издалека, модели вблизи, камера следом.
    fn step_live(&mut self, now: f64) {
        if self.live.is_empty() {
            self.live_objects.clear();
            return;
        }
        let pose = self.nav.pose.clone();
        let view = (pose.lat, pose.lon, self.view_width());
        let mut rebuild = false;
        let mut all: Vec<(crate::live::Kind, crate::live::Object)> = Vec::new();
        {
            let loaders = &self.loaders;
            let request = |limit: u64| (loaders.request)(limit);
            for live in self.live.values_mut() {
                live.step(now, view, &request);
                rebuild |= live.dirty;
                let kind = live.kind;
                all.extend(live.objects(now).into_iter().map(|o| (kind, o)));
            }
        }
        let store = self.heights.read();
        let scale = store.scale();
        let ground = |lat: f64, lon: f64| store.height_at(lat, lon);
        let e = self.nav.camera.ellipsoid;
        let level = |o: &crate::live::Object| if o.alt.is_finite() { o.alt * scale } else { ground(o.lat, o.lon) };
        // Камера следом: объект в середине вида, азимут по курсу.
        if let Some((kind, key)) = self.live_follow.clone() {
            match all.iter().find(|(k, o)| *k == kind && o.key == key) {
                Some((_, o)) => {
                    let heading = if o.heading.is_finite() { o.heading } else { pose.heading };
                    let reach = self.live.get(&kind).map_or(crate::live::START_REACH, |l| l.reach);
                    let (la, lo, look, distance) = data::movers::chase_on(o.lat, o.lon, heading, level(o), &ground, reach, pose.tilt);
                    self.nav.show(Pose::new(la, lo, distance, look, pose.tilt));
                }
                None => self.live_follow = None,
            }
        }
        // Модели вблизи и объект под камерой следом.
        let mut keys: Vec<(crate::live::Kind, String)> = Vec::new();
        for live in self.live.values() {
            let objects: Vec<crate::live::Object> = all.iter().filter(|(k, _)| *k == live.kind).map(|(_, o)| o.clone()).collect();
            let followed = self.live_follow.as_ref().filter(|(k, _)| *k == live.kind).map(|(_, key)| key.as_str());
            keys.extend(crate::live::model_keys(live.kind, &objects, view, followed).into_iter().map(|k| (live.kind, k)));
        }
        if let Some(globe) = &mut self.globe {
            if keys != self.live_keys {
                let mut meshes = Vec::new();
                for (kind, key) in &keys {
                    if let (Some(live), Some((_, o))) = (self.live.get_mut(kind), all.iter().find(|(k, o)| k == kind && o.key == *key)) {
                        meshes.extend(live.meshes(&[o]));
                    }
                }
                globe.set_live_models(&meshes);
                self.live_keys = keys.clone();
            }
            for (i, (kind, key)) in keys.iter().enumerate() {
                if let Some((_, o)) = all.iter().find(|(k, o)| k == kind && o.key == *key) {
                    let center = e.geodetic_to_ecef(o.lat, o.lon, level(o));
                    globe.place_live(i, render::ModelPlace { center, orient: crate::live::orient(o), sun: crate::models::sun_at(o.lat, o.lon), shown: true });
                }
            }
        }
        // Точки и подписи издалека: не те, что видны моделью крупно.
        if rebuild {
            let eye = self.nav.camera.eye;
            let mut points: Vec<DVec3> = Vec::new();
            let mut colors = Vec::new();
            let mut labels = Vec::new();
            for live in self.live.values_mut() {
                let objects: Vec<crate::live::Object> = all.iter().filter(|(k, _)| *k == live.kind).map(|(_, o)| o.clone()).collect();
                let followed = self.live_follow.as_ref().filter(|(k, _)| *k == live.kind).map(|(_, key)| key.clone());
                let near = crate::live::near(live.kind, view.2);
                let skip = |o: &crate::live::Object| {
                    followed.as_deref() == Some(o.key.as_str()) || (near && (e.geodetic_to_ecef(o.lat, o.lon, level(o)) - eye).length() <= crate::live::SEEN * o.size.1)
                };
                for (n, o) in crate::live::nearest(&objects, view, &skip).into_iter().enumerate() {
                    let p = e.geodetic_to_ecef(o.lat, o.lon, level(o));
                    points.push(p);
                    colors.push(crate::live::rgba(o.color));
                    if n < LIVE_LABELS {
                        labels.push((o.label.clone(), p));
                    }
                }
                live.built(now);
            }
            self.live_points.clear();
            if !points.is_empty() {
                let center = points.iter().fold(DVec3::ZERO, |a, p| a + *p) / points.len() as f64;
                self.live_points.push(render::PointsMesh {
                    center,
                    offsets: points.iter().map(|p| (*p - center).as_vec3().to_array()).collect(),
                    colors,
                    size: crate::live::POINT_SIZE,
                    sizes: Vec::new(),
                    dot: Some(render::Dot { alpha: 1.0, lift: crate::dots::LIFT, space: false }),
                });
            }
            self.live_labels = labels;
            self.dots.dirty = true;
        }
        drop(store);
        self.live_objects = all;
    }

    /// Шаг маршрута: готовый путь ложится линией в выделенную папку
    /// «Моих меток», окно летит к нему, как `_done` модуля.
    fn step_route(&mut self, now: f64) {
        let loaders = &self.loaders;
        let request = || (loaders.request)(crate::route::SERVICE_BYTES);
        let Some((route, summary)) = self.route.step(self.lang, now, &request) else {
            return;
        };
        let lang = self.lang;
        let name = scene::placetree::numbered_name(tr(lang, "Route"), &self.marks.names());
        let mut place = scene::kml::Place::new(&name, scene::features::Kind::Line, route.points);
        place.color = crate::route::ROUTE_COLOR;
        place.width = crate::route::ROUTE_WIDTH;
        place.description = summary.clone();
        let folder = self.tree_state.selected.clone().filter(|p| self.marks.folder(p).is_some()).unwrap_or_default();
        let at = self.marks.insert(&folder, scene::kml::Item::Place(place), marks::root_name(lang));
        self.tree_state.selected = Some(at.clone());
        self.marks_changed();
        self.queued.push(crate::marks::Action::Fly(at));
        self.route.show(trf(lang, "{name}: {summary}", &[("name", &name), ("summary", &summary)]), true);
    }

    /// Шаг сетевых ссылок: пришедший документ заменяет содержимое папки,
    /// как `NetLinks._arrived` модуля.
    fn step_links(&mut self, now: f64) {
        let loaders = &self.loaders;
        let request = || (loaders.request)(crate::netlinks::MAX_BYTES);
        let read = |path: &str| -> Result<Vec<u8>, String> { read_local(path).ok_or_else(|| format!("{path}?")) };
        let arrived = self.netlinks.step(self.marks.tree.as_ref(), now, &request, &read, &file_stamp_of);
        let mut changed = false;
        for (path, link, got) in arrived {
            let Some(folder) = self.marks.folder(&path) else {
                continue;
            };
            if folder.link.trim() != link {
                continue;
            }
            let name = folder.name.clone();
            match got {
                Ok(tree) => {
                    self.netlinks.errors.remove(&path);
                    changed |= self.marks.edit_folder(&path, |f| f.children = tree.children);
                }
                Err(e) => {
                    self.open_message = Some(crate::netlinks::error_text(self.lang, &name, &e));
                    self.netlinks.errors.insert(path, e);
                }
            }
        }
        if changed {
            self.marks_changed();
        }
    }

    /// Файлы, брошенные в окно. В программе файл читается сразу, на
    /// странице браузер читает его асинхронно, байты приходят в ящик
    /// [`LiteApp::dropped_box`] и открываются в следующем кадре.
    fn dropped(&mut self, ctx: &egui::Context, now: f64) {
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        for file in files {
            let name = file.path().file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
            #[cfg(not(target_arch = "wasm32"))]
            match file.bytes() {
                Ok(bytes) => {
                    // Ошибку открытия показывает панель.
                    let _ = self.open_file(&bytes, &name, Some(now));
                }
                Err(e) => self.open_message = Some(format!("{name}: {e}")),
            }
            #[cfg(target_arch = "wasm32")]
            {
                let (slot, ctx) = (self.dropped_box.clone(), ctx.clone());
                wasm_bindgen_futures::spawn_local(async move {
                    let read = file.bytes_async().await;
                    if let Ok(mut s) = slot.lock() {
                        s.push((read, name));
                    }
                    ctx.request_repaint();
                });
            }
        }
        let arrived: Vec<(Result<Vec<u8>, String>, String)> =
            self.dropped_box.lock().map(|mut s| s.drain(..).collect()).unwrap_or_default();
        for (read, name) in arrived {
            match read {
                Ok(bytes) => {
                    // Ошибку открытия показывает панель.
                    let _ = self.open_file(&bytes, &name, Some(now));
                }
                Err(e) => self.open_message = Some(format!("{name}: {e}")),
            }
        }
    }
    /// Снимок окна вместе с левой панелью, а не только кадр глобуса.
    /// Показать при запуске снимки события по ключу «источник/id».
    pub fn with_event(mut self, key: &str) -> Self {
        self.events.open(&*self.loaders.request);
        self.pending_event = Some(key.to_string());
        self
    }

    pub fn with_shot_ui(mut self) -> Self {
        self.shot_ui = true;
        self
    }

    /// Начать с позы pose вместо начального вида.
    pub fn with_pose(mut self, pose: Pose) -> Self {
        self.nav.show(pose);
        self.keep_pose = true;
        self
    }

    /// Когда место загрузится, снять окно, отдать снимок в sink
    /// и закрыться.
    pub fn with_shot(mut self, sink: ShotSink) -> Self {
        self.shot = Some(Shot { sink: Some(sink), done: Arc::new(AtomicBool::new(false)), idle: 0 });
        self
    }

    /// Шаг снимка окна. busy - загрузка или движение идёт.
    fn step_shot(&mut self, ctx: &egui::Context, busy: bool) {
        self.idle_frames = if busy { 0 } else { self.idle_frames.saturating_add(1) };
        let Some(shot) = &mut self.shot else {
            return;
        };
        if shot.done.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        shot.idle = if busy { 0 } else { shot.idle + 1 };        if shot.idle >= SHOT_IDLE_FRAMES
            && let Some(sink) = shot.sink.take()
        {
            let done = shot.done.clone();
            ctx.request_screenshot(move |image| {
                sink(&image);
                done.store(true, Ordering::SeqCst);
            });
        }
        ctx.request_repaint();
    }

    /// Загрузчик основы: встроенный или готовой подосновы.
    fn base_loader(&self, basemap: Basemap) -> Option<&dyn TileLoader> {
        self.loaders.basemap(basemap).or_else(|| self.ready_loader.as_ref().filter(|(b, _)| *b == basemap).map(|(_, l)| l.as_ref()))
    }

    /// Сменить основу: снимки прежней забываются.
    fn set_basemap(&mut self, basemap: Basemap) {
        if basemap == self.basemap {
            return;
        }
        self.basemap = basemap;
        self.pending.clear();
        // Подробная мозаика тела COG - свой загрузчик уровней после тайлов.
        let deep = match basemap {
            Basemap::Body(key) => scene::planets::planet_by_key(key).imagery.and_then(|i| i.deep.map(|d| (d, u32::from(i.max_level)))),
            _ => None,
        };
        let loaders = &self.loaders;
        self.cog_base = deep.map(|(d, from)| crate::cogtiles::CogTiles::new(d.url, d.clon, from + 1, u32::from(d.max_level), &|limit| (loaders.request)(limit)));
        if matches!(basemap, Basemap::Ready(_) | Basemap::Own(_) | Basemap::Paleo(_) | Basemap::Body(_)) && self.ready_loader.as_ref().is_none_or(|(b, _)| *b != basemap) {
            self.ready_loader = Some((basemap, (self.loaders.make)(basemap.source())));
        }
        // Карта палеогеографии - не выбор основы, в настройки не идёт.
        if !matches!(basemap, Basemap::Paleo(_) | Basemap::Body(_)) {
            self.settings.basemap = basemap.key().to_string();
            self.settings_dirty = true;
        }
        self.sync_maps();
        if let Some(globe) = &mut self.globe {
            globe.clear_textures();
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
                && let Some(b) = &mut self.buildings
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
                *state = data::gibs::pick_day(&ready, None).map_or(ThemeDay::Failed, ThemeDay::Day);
                days_changed = true;
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
        if self.dots.dirty || self.geo.lines_dirty {
            self.geo.lines_dirty = false;
            self.labels.set_plates(&self.dots.plate_names());
            let mut sets = self.dots.line_sets();
            sets.extend(self.geo.line_set());
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
                all.push(render::PointsMesh {
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
                all.push(render::PointsMesh {
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
            vec![self.basemap.attribution(self.lang), credits::TERRAIN, labels::ATTRIBUTION]
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
        parts.extend(self.live.keys().map(|k| k.attribution()));
        if self.satellites.is_some() {
            parts.push(data::satellites::ATTRIBUTION);
        }
        if self.mining.is_some() {
            parts.push(data::mining::POLYGONS_ATTRIBUTION);
        }
        if self.currents.is_some() {
            parts.push(data::flow::CURRENTS_ATTRIBUTION);
        }
        if self.overture.is_some() {
            parts.push(crate::overture::ATTRIBUTION);
        }
        if let Some(layer) = self.events.layer() {
            parts.push(layer.credit);
        }
        parts.extend(self.lidar_credits.iter().copied());
        if self.rumsey.is_some() {
            parts.push(data::rumsey::ATTRIBUTION);
        }
        if !self.wiki_labels.is_empty() {
            parts.push(data::wikipedia::ATTRIBUTION);
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

    /// Строка поиска места с кнопкой «Поиск», как у модуля, и найденные
    /// места под ней.
    fn search_row(&mut self, ui: &mut egui::Ui, now: f64) {
        let button = tr(self.lang, "Search");
        let mut clicked = false;
        let mut make = false;
        let mut talk = false;
        let mut setup = false;
        let lang = self.lang;
        let edit = ui
            .horizontal(|ui| {
                let width = ui.available_width() - 100.0;
                // Поиск места - земной, на другом теле строка - только
                // для помощника. В небе строка ищет объекты неба, как
                // у модуля.
                let edit = ui.add_enabled(
                    self.planet.earth || self.sky.is_some() || self.settings.ai.search,
                    egui::TextEdit::singleline(&mut self.search_text)
                        .id(egui::Id::unique("planetx-search"))
                        .hint_text(tr(self.lang, "Voyages of Columbus"))
                        .desired_width(width.max(60.0)),
                )
                .on_hover_text(tr(self.lang, "Place, coordinates or action. A place name or coordinates, for example Perm or 58.0105, 56.2294, give a flight. A layer or a window, for example «ruler» or «grid», is found by its name and turned on from the hint. Ctrl+K from anywhere in the window puts the cursor into this line."));
                if ui.button(button).on_hover_text(tr(self.lang, "Find what is typed in the line. A place - a flight to it, several found places - a list below.")).clicked() {
                    clicked = true;
                }
                // Кнопка помощника, `Panel.make` модуля: нажатие создаёт
                // метки по описанию, правая кнопка - разговор и настройки.
                let ai = ui.button("✦").on_hover_text(tr(lang, "Create placemarks by the description in the line with the assistant, Ctrl+Enter. The right button - the conversation and the assistant settings."));
                make = ai.clicked();
                ai.context_menu(|ui| {
                    if ui.button(tr(lang, "Conversation with the assistant…")).clicked() {
                        talk = true;
                        ui.close();
                    }
                    if ui.button(tr(lang, "Assistant settings…")).clicked() {
                        setup = true;
                        ui.close();
                    }
                });
                edit
            })
            .inner;
        if talk {
            self.ai.open = true;
        }
        if setup {
            self.ai.settings_open = true;
        }
        make |= edit.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter));
        if make {
            let text = self.search_text.clone();
            let context = self.ai_context();
            let loaders = &self.loaders;
            let request = |limit: u64| (loaders.request)(limit);
            if self.ai.generate(lang, &text, &self.settings.ai, &context, &request) {
                self.ai_answer_from = Some(self.ai.lines.len().saturating_sub(1));
            }
            return;
        }
        // Ctrl+K - в строку поиска, как у модуля.
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K)) {
            edit.request_focus();
        }
        // Подсказки при вводе: метки, действия, прежние запросы
        // (`scene::searchbar`, `ui/commands.py` модуля). Nominatim при
        // вводе не спрашивается, только по Enter.
        let items = scene::searchbar::drop_repeats(&scene::searchbar::suggestions(
            &self.search_text,
            &self.place_entries(),
            &self.settings.history,
            scene::searchbar::LIMIT,
            &crate::commands::actions(self.lang, &self.command_state()),
        ));
        let mut chosen: Option<scene::searchbar::Suggestion> = None;
        // Список подсказок встроен в панель под строкой, как у модуля:
        // виден, пока в нём есть строки и строка поиска в фокусе.
        if self.suggesting && !items.is_empty() {
            let lang = self.lang;
            egui::Frame::new().fill(crate::theme::pal().field).stroke(egui::Stroke::new(1.0, crate::theme::pal().border)).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(FOUND_HEIGHT).id_salt("hints").show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for item in &items {
                        if ui.selectable_label(false, hint_text(lang, item)).clicked() {
                            chosen = Some(item.clone());
                        }
                    }
                });
            });
        }
        self.suggesting = edit.has_focus();
        let entered = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        // Просьба словами и запрос вне Земли - помощнику, если он
        // подключён и флажок настроек стоит, `is_request` модуля.
        let ai_ready = self.settings.ai.search && self.settings.ai.ready();
        if (entered || clicked) && ai_ready && (assistant::is_request(&self.search_text) || (!self.planet.earth && self.sky.is_none())) {
            self.ask_from_search(&self.search_text.clone());
            return;
        }
        // Место не нашлось - запрос помощнику, `fly` модуля.
        if let crate::search::Status::NotFound(q) = &self.search.status
            && self.ai_fallback.as_deref() == Some(q.as_str())
        {
            self.ai_fallback = None;
            if ai_ready {
                self.ask_from_search(&self.search_text.clone());
            }
        }
        // Ответ помощника под строкой, `set_answer` модуля.
        if let Some(from) = self.ai_answer_from {
            let answer = self.ai.answer_since(from);
            let mut close = false;
            let mut write = false;
            egui::Frame::new().fill(crate::theme::pal().field).stroke(egui::Stroke::new(1.0, crate::theme::pal().border)).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().max_height(FOUND_HEIGHT * 1.5).id_salt("ai-answer").show(ui, |ui| {
                    if answer.is_empty() {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.weak(tr(lang, "Waiting for the answer…"));
                        });
                    } else {
                        ui.label(&answer);
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if let Some(text) = self.ai.proposal_text() {
                        ui.label(text);
                        write = ui.link(tr(lang, "Write")).clicked();
                    }
                    if ui.link(tr(lang, "Conversation…")).clicked() {
                        self.ai.open = true;
                    }
                    close = ui.link(tr(lang, "Close")).clicked();
                });
            });
            if write {
                self.ai.accept_proposal();
            }
            if close {
                self.ai_answer_from = None;
            }
        }
        // Поиск в небе: каталог неба, иначе служба имён Sesame.
        if (entered || clicked)
            && let Some(sky) = &mut self.sky
        {
            let loaders = &self.loaders;
            sky.search(&self.search_text, &|limit| (loaders.request)(limit));
            return;
        }
        // Enter с точным совпадением названия метки или действия -
        // подсказка, иначе запрос Nominatim, правило службы.
        if entered && chosen.is_none() {
            chosen = scene::searchbar::exact(&self.search_text, &items).into_iter().find(|s| s.kind != scene::searchbar::Kind::History);
        }
        if let Some(item) = chosen {
            self.run_suggestion(ui.ctx(), &item, now);
            return;
        }
        let submitted = clicked || entered;
        if submitted {
            self.settings.history = scene::searchbar::remember(&self.settings.history, &self.search_text, scene::searchbar::HISTORY);
            self.settings_dirty = true;
        }
        // Сообщения поиска - в строку состояния, как у модуля.
        if let Some(message) = search_status(self.lang, &self.search.status)
            && self.open_message.as_deref() != Some(message.as_str())
            && self.message_seen.as_ref().is_none_or(|(m, _)| *m != message)
        {
            self.open_message = Some(message);
        }
        // Найденные места - список под строкой, когда их больше одного.
        let mut picked = None;
        if self.search.found.len() > 1 {
            egui::Frame::new().fill(crate::theme::pal().field).stroke(egui::Stroke::new(1.0, crate::theme::pal().border)).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
                egui::ScrollArea::vertical().max_height(FOUND_HEIGHT).id_salt("found").show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for (n, place) in self.search.found.iter().enumerate() {
                        if ui.selectable_label(false, data::geocode::place_text(place)).clicked() {
                            picked = Some(n);
                        }
                    }
                });
            });
        }
        if submitted {
            self.ai_fallback = Some(data::geocode::normalize(&self.search_text));
            if let Some(place) = self.search.submit(&self.search_text) {
                self.fly_to_found(&place, now);
            }
        }
        if let Some(place) = picked.and_then(|n| self.search.found.get(n).cloned()) {
            self.fly_to_found(&place, now);
        }
        // Строка маршрута под поиском, `set_answer` модуля.
        match self.route.ui(ui, self.lang) {
            Some(crate::route::Link::Mode(mode)) => {
                self.settings.route = mode.name().to_string();
                self.settings_dirty = true;
                self.route.set_mode(self.lang, mode, now);
            }
            Some(crate::route::Link::Fix) => ui.ctx().open_url(egui::OpenUrl::new_tab(data::routing::FIX_MAP)),
            Some(crate::route::Link::Clear) => self.route.clear(),
            None => {}
        }
    }

    /// Метки «Моих меток» для подсказок: ключ - путь через точку,
    /// название, первая точка.
    fn place_entries(&self) -> Vec<(String, String, f64, f64)> {
        fn walk(folder: &scene::kml::Folder, path: &mut Vec<usize>, out: &mut Vec<(String, String, f64, f64)>) {
            for (i, child) in folder.children.iter().enumerate() {
                path.push(i);
                match child {
                    scene::kml::Item::Folder(f) => walk(f, path, out),
                    scene::kml::Item::Place(p) if !p.name.trim().is_empty() => {
                        let key = path.iter().map(usize::to_string).collect::<Vec<_>>().join(".");
                        let (lat, lon) = p.points.first().copied().unwrap_or((0.0, 0.0));
                        out.push((key, p.name.clone(), lat, lon));
                    }
                    _ => {}
                }
                path.pop();
            }
        }
        let mut out = Vec::new();
        if let Some(tree) = &self.marks.tree {
            walk(tree, &mut Vec::new(), &mut out);
        }
        out
    }

    /// Что включено - для приписок действий строки «Поиск».
    fn command_state(&self) -> crate::commands::Now {
        crate::commands::Now {
            extras: self.extras_on.clone(),
            recording: self.recording.is_some(),
            time: self.timebar.extent.is_some().then_some(self.timebar.open),
            identify: self.identify,
        }
    }

    /// Окна и органы управления на экране: не самопроверка и не снимок
    /// без интерфейса.
    fn shows_ui(&self) -> bool {
        self.check.is_none() && (self.shot.is_none() || self.shot_ui)
    }

    /// Выполнить подсказку строки «Поиск»: перелёт к метке, действие или
    /// прежний запрос.
    fn run_suggestion(&mut self, ctx: &egui::Context, item: &scene::searchbar::Suggestion, now: f64) {
        match item.kind {
            scene::searchbar::Kind::Place => {
                let path: Vec<usize> = item.key.split('.').filter_map(|p| p.parse().ok()).collect();
                self.tree_state.selected = Some(path.clone());
                self.queued.push(crate::marks::Action::Fly(path));
                self.search_text = item.text.clone();
            }
            scene::searchbar::Kind::History => {
                self.search_text = item.text.clone();
                if let Some(place) = self.search.submit(&self.search_text) {
                    self.fly_to_found(&place, now);
                }
            }
            scene::searchbar::Kind::Action => {
                if let Some(command) = crate::commands::Command::from_key(&item.key) {
                    self.search_text.clear();
                    self.run_command(ctx, command, now);
                }
            }
        }
    }

    /// Действие программы из строки «Поиск».
    fn run_command(&mut self, ctx: &egui::Context, command: crate::commands::Command, now: f64) {
        use crate::commands::Command;
        let before = self.settings.clone();
        match command {
            Command::Layer(e) => {
                let on = self.extras_on.contains(&e);
                self.set_extra(e, !on);
            }
            Command::Catalog => self.catalog.open = true,
            Command::Ruler => self.set_tool(crate::tool::Tool::ruler()),
            Command::Place => self.set_tool(crate::tool::Tool::place(crate::ruler::Mode::Point)),
            Command::SaveView => self.name_dialog = Some((NameFor::View, scene::placetree::numbered_name(tr(self.lang, "View"), &self.marks.names()))),
            Command::Snapshot => self.snapshot = Some(Snapshot::Hide(SnapshotTo::File)),
            Command::CopyView => self.snapshot = Some(Snapshot::Hide(SnapshotTo::Clipboard)),
            Command::Record => self.toggle_record(now),
            Command::Time => self.timebar.open = !self.timebar.open && self.timebar.extent.is_some(),
            Command::Identify => self.identify = !self.identify,
            Command::Properties => self.settings_open = true,
            Command::Sources => self.credits.sources_open = true,
            Command::About => self.credits.about_request = true,
            Command::Events => self.events.open(&*self.loaders.request),
            Command::Tracks => self.tracks.open = true,
            Command::Assistant => self.ai.open = true,
            Command::MyLocation => self.gps.start(ctx),
            Command::SceneSave => {
                let folder = self.tree_state.selected.clone().unwrap_or_default();
                self.save_marks(&folder, crate::export::Kind::Scene);
            }
            Command::SceneOpen => self.pick_file(ctx, now),
            Command::Share => self.share(ctx),
            Command::Fullscreen => {
                let on = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                self.set_fullscreen(ctx, !on);
            }
            Command::Demo(s, d) => {
                if let Some(demo) = crate::demos::MENU.get(s).and_then(|sec| sec.demos.get(d)) {
                    self.open_demo(demo, now);
                }
            }
        }
        if self.settings != before {
            self.settings_dirty = true;
            if self.settings.dark != before.dark {
                crate::theme::apply(ctx, self.settings.dark);
            }
        }
    }

    fn finish_check(&mut self, ctx: &egui::Context) {
        let Some(check) = &self.check else {
            return;
        };
        if !check.done() {
            return;
        }
        let source = |name: &'static str, limit: usize, loader: &dyn TileLoader| {
            let s = loader.stats();
            SourceStats {
                name,
                limit,
                peak: s.requests_peak.load(Ordering::SeqCst),
                total: s.requests_total.load(Ordering::SeqCst),
                disk: s.disk_hits.load(Ordering::SeqCst),
            }
        };
        let sources = [
            source("osm", net::OSM.max_requests, self.loaders.osm.as_ref()),
            source("esri", net::ESRI.max_requests, self.loaders.esri.as_ref()),
            source("terrarium", net::TERRARIUM.max_requests, self.loaders.terrain.as_ref()),
            source("openfreemap", net::OPENFREEMAP.max_requests, self.loaders.places.as_ref()),
        ];
        let (report, ok) = check.report(&sources, (self.nav.camera.width, self.nav.camera.height), &self.search.sent);
        (self.report)(&report, ok);
        self.check = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
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
        if !self.places_dirty {
            return;
        }
        let Some(root) = self.marks.tree.as_ref() else {
            return;
        };
        match places::to_text(root) {
            // На странице предел localStorage около 5 МБ.
            Ok(text) if cfg!(target_arch = "wasm32") && text.len() > PAGE_PLACES_LIMIT => {
                self.open_message = Some(tr(self.lang, "My Places exceed 4 MB, the page cannot save them").to_string());
            }
            Ok(text) => storage.set_string(places::KEY, text),
            Err(e) => self.open_message = Some(format!("{}: {e}", marks::root_name(self.lang))),
        }
        self.places_dirty = false;
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
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
            || self.wiki.as_ref().is_some_and(crate::wiki::Wiki::busy)
            || self.mining.as_ref().is_some_and(crate::mining::Mining::busy)
            || self.parcel_ask.is_some()
            || self.currents.as_ref().is_some_and(crate::currents::Currents::busy)
            || self.rumsey.as_ref().is_some_and(crate::rumsey::Rumsey::busy)
            || self.sentinel.as_ref().is_some_and(crate::sentinel::Sentinel::busy)
            || self.buildings.as_ref().is_some_and(|b| b.busy(self.heights.read().version()))
            || self.overture.as_ref().is_some_and(crate::overture::Overture::busy)
            || self.gibs_catalog.is_some()
            || self.events.busy()
            || self.lidar.as_ref().is_some_and(crate::lidarnet::Lidar::busy)
            || self.relief_calc.as_ref().is_some_and(crate::reliefcalc::ReliefCalc::busy)
            || self.hydro.busy()
            || self.osm_mines.as_ref().is_some_and(|o| o.busy(now, &self.overpass_mirrors))
            || self.osm_infra.as_ref().is_some_and(|o| o.busy(now, &self.overpass_mirrors))
            || self.satellites.as_ref().is_some_and(crate::satellites::Satellites::busy)
            || self.viewshed.as_ref().is_some_and(crate::pointcalc::Calc::busy)
            || self.insolation.as_ref().is_some_and(crate::pointcalc::Calc::busy)
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
                || self.overlay_props.as_mut().is_some_and(|p| p.input(&ctx, &globe_response, rect, ppp, &self.nav));
            // Камера следом: колесо - расстояние до объекта, Esc отпускает,
            // `ChaseControl` модуля.
            if let Some((kind, _)) = self.live_follow.clone()
                && globe_response.hovered()
            {
                let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0
                    && let Some(live) = self.live.get_mut(&kind)
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
                        && let Some(s) = &mut self.satellites
                    {
                        s.select(Some(number));
                    }
                    // Первый абзац ближней статьи у щелчка.
                    let first = self.objects.as_ref().and_then(|o| o.groups.iter().find(|(g, _)| g == tr(self.lang, "Wikipedia"))).and_then(|(_, f)| f.first()).map(|(t, _)| t.clone());
                    if let (Some(title), Some(wiki)) = (first, &mut self.wiki) {
                        let loaders = &self.loaders;
                        wiki.ask_summary(&title, &|limit: u64| (loaders.request)(limit));
                    }
                }
            }
            // Щелчок по метке без инструмента - её карточка, как у модуля.
            if self.tool.is_none()
                && !self.identify
                && !padded
                && globe_response.clicked()
                && let (Some(pos), Some(tree)) = (globe_response.interact_pointer_pos(), self.marks.tree.as_ref())
            {
                let (px, py) = (f64::from(pos.x - rect.min.x) * ppp, f64::from(pos.y - rect.min.y) * ppp);
                if let Some(path) = crate::card::pick(tree, &self.nav, px, py, crate::card::PICK_PIXELS * ppp) {
                    self.tree_state.selected = Some(path.clone());
                    self.card = Some(crate::card::Card { path });
                }
            }
            // Щелчок по статье Википедии без инструмента - её ссылка и первый
            // абзац в окне «Объекты», как `open_article_card` модуля.
            if self.tool.is_none()
                && !self.identify
                && !padded
                && self.card.is_none()
                && globe_response.clicked()
                && let Some(pos) = globe_response.interact_pointer_pos()
            {
                let (px, py) = (f64::from(pos.x - rect.min.x) * ppp, f64::from(pos.y - rect.min.y) * ppp);
                let radius = crate::wiki::PICK_PIXELS * ppp;
                let hit = self.wiki.as_ref().and(
                    self.wiki_labels
                        .iter()
                        .filter_map(|(title, p)| {
                            let (q, front) = self.nav.camera.project(*p);
                            let gap = (q.x - px).hypot(q.y - py);
                            (front && gap <= radius).then_some((gap, title.clone(), *p))
                        })
                        .min_by(|a, b| a.0.total_cmp(&b.0)),
                );
                if let Some((_, title, p)) = hit {
                    let (lat, lon, _) = self.nav.camera.ellipsoid.ecef_to_geodetic(p);
                    let (q, _) = self.nav.camera.project(p);
                    self.objects = Some(self.make_objects(lat, lon, Some((q.x, q.y, ppp)), None));
                    if let Some(wiki) = &mut self.wiki {
                        let loaders = &self.loaders;
                        wiki.ask_summary(&title, &|limit: u64| (loaders.request)(limit));
                    }
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
        self.step_wiki(now);
        self.step_satellites(now);
        self.step_overpass(now);
        self.step_parcels();
        self.step_currents(&ctx, now);
        self.step_lidar(now);
        self.step_events(now);
        self.step_rumsey(&ctx, now);
        self.step_sentinel(&ctx, now);
        self.step_tracks(&ctx);
        self.step_ai(&ctx, now);
        self.step_gps(&ctx, now);
        #[cfg(not(target_arch = "wasm32"))]
        self.step_mcp(&ctx, now);
        self.paleo_bar(&ctx, now);
        self.step_cutaway();
        self.check_linked_models(now);
        for (bytes, name) in std::mem::take(&mut self.pending_open) {
            let _ = self.open_file(&bytes, &name, None);
        }
        self.dropped(&ctx, now);
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
        let moving = self.nav.step(now) || self.player.is_some() || self.timebar.playing();
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
        if tracks_models {
            self.overlays_built = None;
        }
        let model_list = if self.overlays_built != Some(self.marks.overlays_version()) { self.model_overlays(now) } else { Vec::new() };

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
        // Тайлы дорог маршрута - тем же загрузчиком, раньше основы.
        self.route_keys = self.route.wanted().into_iter().collect();
        self.route_only = self.route_keys.iter().copied().filter(|k| !vector_tiles.iter().any(|(v, _)| v == k)).collect();
        vector_tiles.extend(self.route_only.iter().map(|k| (*k, lod::INFINITE_PRIORITY)));
        // Тайлы зданий у глаза - тем же загрузчиком, `render/buildings.py`
        // модуля.
        if let Some(b) = &mut self.buildings {
            let cam = &self.nav.camera;
            let (lat, lon, _) = cam.ellipsoid.ecef_to_geodetic(cam.eye);
            let forward = -cam.rotation.z_axis;
            let wanted = b.want(cam.eye.to_array(), forward.to_array(), store.height_at(lat, lon));
            if let Some(overture) = &mut self.overture {
                // Здания Overture - своим загрузчиком архива PMTiles.
                self.building_keys.clear();
                self.building_only.clear();
                let keys: Vec<TileKey> = wanted.iter().map(|(k, _)| *k).collect();
                for (key, bytes) in overture.step(&keys, now, &*self.loaders.request) {
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
        // Снимки события «после» и «до» шторки: тайлы кадра не глубже
        // уровня снимков, своей парой текстур тайла.
        if let Some(layer) = self.events.layer() {
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
        globe.set_split(self.events.swipe().map(|f| f * self.globe_width_px));
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
        if fresh.is_some() || water_stale || geo_stale || tracks_stale {
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
            globe.set_features(&all);
        }
        if (fresh.is_some() || tracks_models) && self.overlays_built != Some(self.marks.overlays_version()) {
            self.overlays_built = Some(self.marks.overlays_version());
            globe.set_pictures(&self.marks.picture_meshes(&store));
            let built = self.models.meshes(&model_list, &store, now);
            globe.set_models(&built.meshes);
            self.movers = built.movers;
            self.animated = built.animated;
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
            let under_field = if self.settings.maps.iter().any(|m| m == "emag") { WATER_UNDER_FIELD } else { 1.0 };
            [WATER_COLOR[0], WATER_COLOR[1], WATER_COLOR[2], WATER_COLOR[3] * under_field]
        });
        let count_holes = self.check.as_ref().is_some_and(Check::count_holes);
        if self.check.as_mut().is_some_and(Check::take_grab_request) {
            globe.grab();
        }
        let stats =
            globe.render(&self.nav.camera, &selection, Some(&store), FrameOptions { count_holes, show_holes: false, air: self.settings.air && self.planet.air.is_some(), ratio: ppp as f32, sun, water, tint: self.planet.air, sky: false, sea_level });
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
        let hiding = matches!(self.snapshot, Some(Snapshot::Hide(_)));
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
        let wiki_from = self.live_labels.len() + self.track_labels.len();
        for (n, (text, p)) in self.live_labels.iter().chain(&self.track_labels).chain(&self.wiki_labels).enumerate() {
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
            // Статья Википедии - точка у подписи, как пункт подписей модуля.
            if n >= wiki_from {
                let [r, g, b] = crate::wiki::COLOR;
                painter.circle(at - egui::vec2(6.0, -6.0), crate::wiki::POINT_SIZE / 2.0, egui::Color32::from_rgb(r, g, b), egui::Stroke::new(1.0, egui::Color32::WHITE));
            }
            painter.text(at + egui::vec2(1.0, 1.0), egui::Align2::LEFT_BOTTOM, text, font.clone(), egui::Color32::from_black_alpha(200));
            painter.text(at, egui::Align2::LEFT_BOTTOM, text, font, egui::Color32::WHITE);
        }
        // Рамки карт окна «Карты Рамсея», выбранная - ярче.
        if let Some(r) = &self.rumsey {
            let (boxes, selected) = r.footprints();
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
        // Струйки течений поверх глобуса.
        if let Some(c) = &self.currents {
            c.paint(&painter, rect, ppp, &self.nav.camera);
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
        if let Some(sats) = &mut self.satellites {
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
        // Точка пути под курсором графика профиля, как у модуля.
        if let Some((lat, lon)) = self.profile.as_ref().and_then(|p| p.hovered) {
            let h = self.nav.pose.terrain.as_ref().map_or(0.0, |t| t(lat, lon));
            let (px, front) = self.nav.camera.project(self.nav.camera.ellipsoid.geodetic_to_ecef(lat, lon, h));
            if front {
                let at = rect.min + egui::vec2((px.x / ppp) as f32, (px.y / ppp) as f32);
                painter.circle(at, 5.0, egui::Color32::from_rgb(220, 60, 40), egui::Stroke::new(1.5, egui::Color32::WHITE));
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
            (crate::maps::Kind::Theme(_), Some(ThemeDay::Day(day))) => {
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
        match crate::legend::show(&ctx, self.lang, rect, self.credits.rect, crate::legend::Shown { maps: &maps, theme, dots: &dot_layers, contours }) {
            Some(crate::legend::Closed::Map(key)) => {
                if let Some(n) = crate::maps::index(key) {
                    self.set_extra(crate::layers::Extra::Map(n), false);
                }
            }
            Some(crate::legend::Closed::Dots(layer)) => self.set_extra(crate::layers::Extra::Dots(layer), false),
            None => {}
        }
        self.request_snapshot(&ctx, rect);
        if !hiding {
            self.overlays(&ctx, rect, now);
        }
        if !hiding && self.shows_ui() {
            self.toolbar(&ctx, rect, now);
            self.settings_window(&ctx);
            self.tool_window(&ctx);
            self.card_window(&ctx);
            self.profile_window(&ctx);
            self.section_window(&ctx);
            self.name_window(&ctx);
            self.dialogs(&ctx, now);
            self.overlay_windows(&ctx);
            let mut out = Vec::new();
            let mut catalog = std::mem::take(&mut self.catalog);
            catalog.window(&ctx, self.lang, &self.layers_state(), &mut out);
            self.catalog = catalog;
            self.apply_layers(out);
        }

        if let Some(check) = &mut self.check {
            check.after_frame(&stats, &self.nav, &self.heights.read());
            check.note_labels(&self.labels.drawn, rect);
        }
        self.finish_check(&ctx);
        self.step_shot(&ctx, moving || loading || lines_pending);
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
        crate::layers::Extra::Dots(crate::dots::Layer::Fires) => Some(0),
        crate::layers::Extra::Buildings | crate::layers::Extra::Overture => Some(9),
        _ => None,
    }
}
