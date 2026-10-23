// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно: глобус с подложкой и рельефом, навигация мышью и пальцами.

use std::collections::HashMap;
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
/// Пол высот Земли: дно моря прижато к уровню моря, подложка рисует воду.
pub const EARTH_FLOOR: Option<f32> = Some(0.0);

/// Куда отдать отчёт самопроверки: текст и итог, true - пройдена.
pub type Report = Box<dyn FnMut(&str, bool)>;

/// Куда отдать снимок окна.
pub type ShotSink = Box<dyn FnOnce(&egui::ColorImage) + Send>;

/// Кадров подряд без загрузки и движения перед снимком окна.
pub const SHOT_IDLE_FRAMES: u32 = 30;
/// Наибольшая строка «Моих меток» на странице, байт: localStorage
/// браузера держит около 5 МБ на сайт.
pub const PAGE_PLACES_LIMIT: usize = 4 << 20;
/// Скорость шкалы в демо самосвалов, `HAUL_RATE` модуля.
pub const HAUL_RATE: f64 = 10.0;
/// За столько секунд до конца рейса момент возвращается к началу,
/// `HAUL_END` модуля: момент упирается в конец шкалы, и показ там встаёт.
pub const HAUL_END: f64 = 1.0;
/// Окно уже этого, логические пиксели, - узкое: панель при запуске
/// скрыта.
pub const NARROW: f32 = 700.0;

/// Снимок окна после загрузки места.
struct Shot {
    sink: Option<ShotSink>,
    done: Arc<AtomicBool>,
    idle: u32,
}

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
}

impl Loaders {
    /// Загрузчик основы.
    pub fn basemap(&self, basemap: Basemap) -> &dyn TileLoader {
        match basemap {
            Basemap::Esri => self.esri.as_ref(),
            Basemap::Osm => self.osm.as_ref(),
        }
    }
}

/// Хранилище высот, общее для кадра и навигатора.
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
        if store.tile(height_key).is_none() {
            let p = want.get(key).copied().unwrap_or(1.0);
            let slot = wanted.entry(height_key).or_insert(p);
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

/// Целое с пробелом между тысячами: 20 000.
fn group_thousands(n: i64) -> String {
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
fn demo_items(ui: &mut egui::Ui, lang: Lang, clicked: &mut Option<&'static crate::demos::Demo>) {
    for section in &crate::demos::MENU {
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

/// Что нажали на панели инструментов.
#[derive(Default)]
struct ToolClicks {
    open: bool,
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
        if (settings.scale - 1.0).abs() > f64::EPSILON {
            heights.write().set_scale(settings.scale);
        }
        let mut nav = Navigator::new(camera, pose);
        let source = heights.clone();
        nav.set_terrain(Some(Arc::new(move |lat, lon| source.read().height_at(lat, lon))));
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
            settings,
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
        app
    }

    /// Панель инструментов в левом верхнем углу глобуса, как у модуля:
    /// панель, «Открыть», «Демо», «Поделиться», «Настройки», во весь
    /// экран, «О программе».
    fn toolbar(&mut self, ctx: &egui::Context, view: egui::Rect, now: f64) {
        use crate::theme::{Tool, tool_button};
        let lang = self.lang;
        let mut clicks = ToolClicks::default();
        let mut panel = self.panel_open;
        let mut settings = self.settings_open;
        let mut about = false;
        let mut full = false;
        let mut share = false;
        let mut new_tool: Option<crate::tool::Tool> = None;
        // Уровень панели - как у окон: открытое окно встаёт поверх неё.
        // На уровне Foreground панель закрывала окно «Источники данных»,
        // замечание автора от 23 октября 2026 года.
        let area = egui::Area::new(egui::Id::unique("planetx-toolbar"))
            .order(egui::Order::Middle)
            .fixed_pos(view.left_top() + egui::vec2(8.0, 8.0))
            .show(ctx, |ui| {
                crate::theme::plate().inner_margin(egui::Margin::same(3)).show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
                    ui.horizontal(|ui| {
                        if tool_button(ui, Tool::Sidebar, panel, tr(lang, "Show or hide the panel")).clicked() {
                            panel = !panel;
                        }
                        if tool_button(ui, Tool::Open, false, tr(lang, "Open a scene, KML, KMZ or a data file")).clicked() {
                            clicks.open = true;
                        }
                        let demo = tool_button(ui, Tool::Demo, false, tr(lang, "Demo scenes"));
                        egui::Popup::menu(&demo).show(|ui| demo_items(ui, lang, &mut clicks.demo));
                        if tool_button(ui, Tool::Share, false, tr(lang, "Share a link to this view")).clicked() {
                            share = true;
                        }
                        let placing = matches!(self.tool.as_ref().map(|t| &t.purpose), Some(crate::tool::Purpose::Place));
                        // Вид объекта - вкладки окна «Новая метка», как у модуля.
                        if tool_button(ui, Tool::Placemark, placing, tr(lang, "New placemark, path, polygon or circle")).clicked() {
                            new_tool = Some(if placing { crate::tool::Tool::closed() } else { crate::tool::Tool::place(crate::ruler::Mode::Point) });
                        }
                        let measuring = matches!(self.tool.as_ref().map(|t| &t.purpose), Some(crate::tool::Purpose::Ruler));
                        if tool_button(ui, Tool::Ruler, measuring, tr(lang, "Ruler: distance, area, ground length")).clicked() {
                            new_tool = if measuring { Some(crate::tool::Tool::closed()) } else { Some(crate::tool::Tool::ruler()) };
                        }
                        if tool_button(ui, Tool::Settings, settings, tr(lang, "Settings")).clicked() {
                            settings = !settings;
                        }
                        if tool_button(ui, Tool::Fullscreen, false, tr(lang, "Full screen")).clicked() {
                            full = true;
                        }
                        if tool_button(ui, Tool::Info, false, tr(lang, "About")).clicked() {
                            about = true;
                        }
                    });
                });
            });
        self.toolbar_rect = Some(area.response.rect);
        self.panel_open = panel;
        self.settings_open = settings;
        self.credits.about_request |= about;
        if full {
            let on = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!on));
        }
        if share {
            self.share(ctx);
        }
        if let Some(tool) = new_tool {
            // Повторное нажатие «Линейки» закрывает её.
            self.tool = tool.open.then_some(tool);
        }
        if clicks.open {
            self.pick_file(ctx, now);
        }
        if let Some(demo) = clicks.demo {
            self.open_demo(demo, now);
        }
    }

    /// Окно инструмента и его итог: новая метка ложится в выделенную
    /// папку «Моих меток» или в корень, правка заменяет метку.
    fn tool_window(&mut self, ctx: &egui::Context) {
        let Some(tool) = &mut self.tool else {
            return;
        };
        let names = self.marks.names();
        // Длина по рельефу - по настоящим высотам, без масштаба рельефа.
        let source = self.heights.clone();
        let heights_at = move |points: &[(f64, f64)]| -> Vec<f64> {
            let store = source.read();
            let k = store.scale().max(1e-9);
            points.iter().map(|(lat, lon)| store.height_at(*lat, *lon) / k).collect()
        };
        let ready = !self.loaders.terrain.busy();
        let action = tool.window(ctx, self.lang, &names, Some((&heights_at, ready)));
        if !tool.open {
            self.tool = None;
        }
        let root = marks::root_name(self.lang);
        match action {
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
        if !crate::settings::window(ctx, self.lang, &mut open, &mut self.settings) {
            self.settings_open = open;
            return;
        }
        self.settings_open = open;
        self.settings_dirty = true;
        let s = self.settings.clone();
        if s.dark != before.dark {
            crate::theme::apply(ctx, s.dark);
        }
        if (s.scale - before.scale).abs() > f64::EPSILON {
            self.heights.write().set_scale(s.scale);
        }
        if s.language != before.language || s.labels != before.labels {
            let old_root = marks::root_name(self.lang).to_string();
            self.lang = s.lang();
            self.labels = Labels::new(self.lang, &s.labels);
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
        let e = Ellipsoid::wgs84();
        let xyz: Vec<DVec3> = cloud.points.iter().map(|p| e.geodetic_to_ecef(p[0], p[1], p[2])).collect();
        let (center, offsets) = scene::features::centered(&xyz);
        self.clouds.push(render::PointsMesh { center, offsets, colors: cloud.colors.clone(), size: CLOUD_POINT_SIZE });
        if let Some(globe) = &mut self.globe {
            globe.set_points(&self.clouds);
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
        Ok(pose)
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
            self.relief.add(name, dem, &mut store);
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
        self.timebar.set_extent(self.marks.extent());
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
        let mut stop = false;
        let mut toggle = false;
        egui::Area::new(egui::Id::unique("planetx-top"))
            .order(egui::Order::Foreground)
            // Под панелью инструментов, как у модуля: поверх неё шкала
            // закрывала кнопки.
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(egui::pos2(rect.center().x, self.toolbar_rect.map_or(rect.top() + 12.0, |r| r.bottom() + 6.0)))
            .show(ctx, |ui| {
                crate::theme::plate()
                    .show(ui, |ui| {
                        ui.visuals_mut().override_text_color = Some(egui::Color32::WHITE);
                        if let Some(player) = &self.player {
                            ui.horizontal(|ui| {
                                use crate::timeline::{Icon, icon_button};
                                if icon_button(ui, if player.paused() { Icon::Play } else { Icon::Pause }).clicked() {
                                    toggle = true;
                                }
                                if icon_button(ui, Icon::Stop).clicked() {
                                    stop = true;
                                }
                                ui.label(format!(
                                    "{}  {} / {}",
                                    player.name,
                                    scene::tour::clock(player.time(now)),
                                    scene::tour::clock(player.tour.duration)
                                ));
                            });
                        }
                        if show_bar {
                            self.timebar.ui(ui, self.lang, now);
                        }
                    });
            });
        if toggle && let Some(p) = &mut self.player {
            p.toggle(now);
        }
        if stop {
            self.player = None;
        }
        if let Some((name, alpha)) = self.player.as_ref().and_then(|p| p.caption(now)) {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("planetx-caption")));
            let color = egui::Color32::WHITE.gamma_multiply(alpha as f32);
            let galley = painter.layout_no_wrap(name.to_string(), crate::fonts::globe(22.0), color);
            let at = egui::pos2(rect.center().x - galley.size().x / 2.0, rect.bottom() - 90.0);
            painter.galley_with_override_text_color(at + egui::vec2(1.0, 1.0), galley.clone(), egui::Color32::BLACK.gamma_multiply(alpha as f32));
            painter.galley_with_override_text_color(at, galley, color);
        }
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
        let mut actions = Vec::new();
        let lang = self.lang;
        let title = tr(lang, "Places");
        let hint = tr(
            lang,
            "Open or drop a .planetx scene, KML, KMZ, GeoPackage layers, a GeoTIFF elevation raster or a LAS, LAZ, COPC point cloud into the window.",
        );
        let time_label = tr(lang, "Time");
        let layers_title = tr(lang, "Layers");
        let status = self.status_text();
        if !self.panel_open {
            self.tree_actions(ui, actions, now);
            return;
        }
        egui::Panel::left("marks").resizable(true).default_size(300.0).show(ui, |ui| {
            ui.add_space(6.0);
            self.search_row(ui, now);
            ui.add_space(6.0);
            // Строка состояния внизу панели, как у модуля.
            let status_height = 22.0;
            // Под раздел «Слои»: заголовок и строка основы.
            let layers_height = 64.0;
            if crate::theme::section(ui, "marks", title) {
                if self.timebar.extent.is_some() {
                    ui.toggle_value(&mut self.timebar.open, time_label);
                }
                if let Some(message) = &self.open_message {
                    ui.label(message);
                }
                let room = (ui.available_height() - layers_height - status_height).max(60.0);
                let root = self.marks.root(marks::root_name(lang));
                let empty = root.children.is_empty();
                egui::ScrollArea::vertical().max_height(room).auto_shrink([false, true]).show(ui, |ui| {
                    crate::marks::root_ui(ui, root, lang, &mut self.tree_state, &mut actions);
                });
                if empty {
                    ui.label(egui::RichText::new(hint).color(crate::theme::pal().muted));
                }
                ui.add_space(6.0);
            }
            if crate::theme::section(ui, "layers", layers_title) {
                self.basemap_choice(ui);
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(status).color(crate::theme::pal().muted));
            });
        });
        self.tree_actions(ui, actions, now);
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
            }
        }
        for action in actions {
            match action {
                crate::marks::Action::Toggled(path) => {
                    self.marks.toggled(&path);
                    let window = self.timebar.range;
                    let open = self.timebar.open;
                    self.timebar.set_extent(self.marks.extent());
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
                crate::marks::Action::Remove(path) => {
                    if self.marks.remove(&path) {
                        self.tree_state.selected = None;
                        self.player = None;
                        self.marks_changed();
                    }
                }
                crate::marks::Action::Clear => {
                    self.tree_state.selected = None;
                    self.marks.clear();
                    self.shared.clear();
                    self.player = None;
                    self.open_message = None;
                    self.marks_changed();
                }
                crate::marks::Action::Open => self.pick_file(ui.ctx(), now),
                crate::marks::Action::Properties(path) => {
                    if let Some(place) = self.marks.place(&path) {
                        self.tool = Some(crate::tool::Tool::edit(path, place));
                    }
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
        let Some(shot) = &mut self.shot else {
            return;
        };
        if shot.done.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        shot.idle = if busy { 0 } else { shot.idle + 1 };
        if shot.idle >= SHOT_IDLE_FRAMES
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

    /// Сменить основу: снимки прежней забываются.
    fn set_basemap(&mut self, basemap: Basemap) {
        if basemap == self.basemap {
            return;
        }
        self.basemap = basemap;
        if let Some(globe) = &mut self.globe {
            globe.clear_textures();
        }
    }

    /// Снимки и высоты пришедших тайлов - в видеокарту и в хранилище.
    fn upload(&mut self) {
        for basemap in Basemap::ALL {
            // Тайлы прежней основы, пришедшие после смены, не нужны.
            let arrived = self.loaders.basemap(basemap).take();
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
                    globe.upload(loaded.key, image.width, image.height, &image.rgba);
                }
            }
        }
        for loaded in self.loaders.places.take() {
            self.labels.add(loaded);
        }
        let arrived = self.loaders.terrain.take();
        if arrived.is_empty() && !self.relief.busy() {
            return;
        }
        {
            let mut store = self.heights.write();
            for loaded in arrived {
                let (z, x, y) = loaded.key;
                if let Ok(net::Content::Image(img)) = loaded.content
                    && let Some(tile) = terrain::make_tile(z, x, y, &img.rgba, EARTH_FLOOR, Encoding::Terrarium)
                {
                    // Свой рельеф врезается в тайл до хранилища.
                    store.add(self.relief.patch(tile));
                }
            }
            self.relief.build(&mut store);
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
        let parts = [self.basemap.attribution(self.lang), credits::TERRAIN, labels::ATTRIBUTION];
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
        match loading {
            0 => trf(self.lang, "Altitude {height}", &[("height", &height)]),
            n => trf(self.lang, "Altitude {height} · loading {n}", &[("height", &height), ("n", &n.to_string())]),
        }
    }

    /// Строка поиска места с кнопкой «Поиск», как у модуля, и найденные
    /// места под ней.
    fn search_row(&mut self, ui: &mut egui::Ui, now: f64) {
        let button = tr(self.lang, "Search");
        let mut clicked = false;
        let edit = ui
            .horizontal(|ui| {
                let width = ui.available_width() - 70.0;
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.search_text)
                        .hint_text(tr(self.lang, "Find a place, Enter"))
                        .desired_width(width.max(60.0)),
                );
                if ui.button(button).clicked() {
                    clicked = true;
                }
                edit
            })
            .inner;
        // Запрос только по Enter или кнопке, правило Nominatim.
        let submitted = clicked || (edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
        if let Some(message) = search_status(self.lang, &self.search.status) {
            ui.label(message);
        }
        let mut picked = None;
        if self.search.found.len() > 1 {
            for (n, place) in self.search.found.iter().enumerate() {
                if ui.link(data::geocode::place_text(place)).clicked() {
                    picked = Some(n);
                }
            }
        }
        if submitted && let Some(place) = self.search.submit(&self.search_text) {
            self.fly_to_found(&place, now);
        }
        if let Some(place) = picked.and_then(|n| self.search.found.get(n).cloned()) {
            self.fly_to_found(&place, now);
        }
    }

    /// Выбор основы - раздел «Слои» панели.
    fn basemap_choice(&mut self, ui: &mut egui::Ui) {
        let mut chosen = self.basemap;
        ui.horizontal(|ui| {
            ui.label(tr(self.lang, "Basemap"));
            egui::ComboBox::from_id_salt("basemap").selected_text(chosen.name(self.lang)).show_ui(ui, |ui| {
                for b in Basemap::ALL {
                    ui.selectable_value(&mut chosen, b, b.name(self.lang));
                }
            });
        });
        self.set_basemap(chosen);
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

        let loading = self.loaders.basemap(self.basemap).busy()
            || self.loaders.terrain.busy()
            || self.loaders.places.busy()
            || self.labels.pending
            || self.search.busy()
            || self.pictures.busy()
            || self.fetching.is_some()
            || self.relief.busy();
        if let Some(check) = &mut self.check {
            let parts = [
                ("basemap", self.loaders.basemap(self.basemap).busy()),
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
            // Вершину инструмента тянет мышь - Землю она не двигает.
            let busy = self.tool.as_mut().is_some_and(|t| t.input(&ctx, &globe_response, rect, ppp, &self.nav));
            if !busy {
                self.input.handle(&ctx, rect, &mut self.nav, now, ppp);
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
        for (bytes, name) in std::mem::take(&mut self.pending_open) {
            let _ = self.open_file(&bytes, &name, None);
        }
        self.dropped(&ctx, now);
        self.step_tour(now);
        self.step_haul(now);
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
        wanted.extend(selection.want.iter().copied());
        for basemap in Basemap::ALL {
            let loader = self.loaders.basemap(basemap);
            loader.want(if basemap == self.basemap { &wanted } else { &[] });
        }
        // Высоты глубже уровня 15 у своего рельефа собираются здесь.
        let needs = self.relief.route(height_needs(&store, &selection), &store);
        self.loaders.terrain.want(&needs);
        self.loaders.places.want(&self.labels.wanted(&selection.draw));

        for key in std::mem::take(&mut self.pictures.fresh) {
            if let Some(image) = self.pictures.image(&key) {
                globe.set_picture_image(&key, image.width, image.height, &image.rgba);
            }
        }
        if let Some(meshes) = self.marks.meshes(&store, now) {
            globe.set_features(&meshes);
            if self.overlays_built != Some(self.marks.overlays_version()) {
                self.overlays_built = Some(self.marks.overlays_version());
                globe.set_pictures(&self.marks.picture_meshes(&store));
                let built = self.models.meshes(self.marks.overlays(), &store, now);
                globe.set_models(&built.meshes);
                self.movers = built.movers;
                self.animated = built.animated;
            }
        }
        let window = self.timebar.window();
        for mover in &self.movers {
            globe.place_model(mover.index, mover.place(window, &store));
        }
        for model in &self.animated {
            globe.animate_model(model.index, model.deltas(now));
        }
        let recreated = globe.resize(width, height);
        let count_holes = self.check.as_ref().is_some_and(Check::count_holes);
        if self.check.as_mut().is_some_and(Check::take_grab_request) {
            globe.grab();
        }
        let stats =
            globe.render(&self.nav.camera, &selection, Some(&store), FrameOptions { count_holes, show_holes: false, air: true, ratio: ppp as f32 });
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
        let mut blocked = self.screen_overlays(&ctx, &painter, rect);
        blocked.extend(self.credits.rect);
        blocked.extend(self.toolbar_rect);
        self.labels.draw(&painter, rect, ppp as f32, &blocked, &self.nav.camera, &selection.draw, &self.heights.read());
        if let Some(tool) = &self.tool {
            tool.paint(&painter, rect, ppp, &self.nav, self.lang);
        }
        self.attribution(&ctx, rect);
        self.overlays(&ctx, rect, now);
        if self.check.is_none() && (self.shot.is_none() || self.shot_ui) {
            self.toolbar(&ctx, rect, now);
            self.settings_window(&ctx);
            self.tool_window(&ctx);
        }

        if let Some(check) = &mut self.check {
            check.after_frame(&stats, &self.nav, &self.heights.read());
            check.note_labels(&self.labels.drawn, rect);
        }
        self.finish_check(&ctx);
        self.step_shot(&ctx, moving || loading);
        if moving || animating || loading || self.labels.pending || self.check.is_some() {
            ctx.request_repaint();
        }
    }
}
