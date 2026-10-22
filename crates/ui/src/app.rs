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
use crate::i18n::{Lang, Text, search_status, text};
use crate::input::Input;
use crate::labels::{self, Labels};
use crate::marks::{self, Marks};
use crate::timeline::TimeBar;
use crate::tours::Player;
use crate::search::{Search, place_pose};

/// Сколько снимков тайлов держать в видеокарте, сверх нужных кадру
/// старые вытесняются. Снимок 256 × 256 занимает 256 КБ.
pub const TEXTURE_BUDGET: usize = 800;
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
    pending_open: Option<(Vec<u8>, String)>,
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
}

/// Ящик прочитанных брошенных файлов.
type DroppedBox = Arc<std::sync::Mutex<Vec<(Result<Vec<u8>, String>, String)>>>;

impl LiteApp {
    /// scenario - самопроверка или None, её отчёт уходит в report.
    pub fn new(cc: &eframe::CreationContext<'_>, loaders: Loaders, scenario: Option<Scenario>, report: Report) -> Self {
        let render_state = cc.wgpu_render_state.clone();
        let globe = render_state.as_ref().map(|rs| Globe::new(&rs.device, &rs.queue));
        let camera = Camera::new(Ellipsoid::wgs84(), DVec3::ZERO, DMat3::IDENTITY);
        let pose = match scenario {
            Some(s) => s.start_pose(),
            None => Pose::new(START.0, START.1, START.2, 0.0, 0.0),
        };
        let heights = Heights::default();
        let mut nav = Navigator::new(camera, pose);
        let source = heights.clone();
        nav.set_terrain(Some(Arc::new(move |lat, lon| source.read().height_at(lat, lon))));
        let lang = Lang::from_system();
        Self {
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
            labels: Labels::new(lang),
            search: Search::new(Some(match lang {
                Lang::Ru => "ru",
                Lang::En => "en",
            })),
            search_text: String::new(),
            credits: Credits::default(),
            marks: Marks::default(),
            pending_open: None,
            open_message: None,
            dropped_box: DroppedBox::default(),
            shot_ui: false,
            player: None,
            tour_index: None,
            timebar: TimeBar::default(),
        }
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
        self.pending_open = Some((bytes, name.to_string()));
        self
    }

    /// Открыть файл. fly - лететь к позе сцены, иначе встать в неё сразу.
    fn open_file(&mut self, bytes: &[u8], name: &str, fly: Option<f64>) {
        match marks::open(bytes, name) {
            Ok(opened) => {
                self.marks.set(opened.tree, &opened.name);
                self.player = None;
                self.timebar.set_extent(self.marks.extent());
                self.marks_to_labels();
                if let Some(pose) = opened.pose {
                    match fly {
                        Some(now) => crate::input::fly(&mut self.nav, pose, now),
                        None => self.nav.show(pose),
                    }
                }
                self.open_message = Some(opened.name);
            }
            Err(e) => {
                self.open_message = Some(match self.lang {
                    Lang::Ru => format!("Файл не открылся: {name}: {e}"),
                    Lang::En => format!("The file did not open: {name}: {e}"),
                });
            }
        }
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
            .fixed_pos(egui::pos2(rect.center().x - 220.0, rect.top() + 12.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_black_alpha(160))
                    .inner_margin(egui::Margin::same(6))
                    .corner_radius(4.0)
                    .show(ui, |ui| {
                        ui.visuals_mut().override_text_color = Some(egui::Color32::WHITE);
                        if let Some(player) = &self.player {
                            ui.horizontal(|ui| {
                                if ui.button(if player.paused() { "▶" } else { "⏸" }).clicked() {
                                    toggle = true;
                                }
                                if ui.button("⏹").clicked() {
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
                            self.timebar.ui(ui, now);
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
            let galley = painter.layout_no_wrap(name.to_string(), egui::FontId::proportional(22.0), color);
            let at = egui::pos2(rect.center().x - galley.size().x / 2.0, rect.bottom() - 90.0);
            painter.galley_with_override_text_color(at + egui::vec2(1.0, 1.0), galley.clone(), egui::Color32::BLACK.gamma_multiply(alpha as f32));
            painter.galley_with_override_text_color(at, galley, color);
        }
    }

    /// Точки меток - в надписи.
    fn marks_to_labels(&mut self) {
        let points: Vec<(String, f64, f64)> = self.marks.points().into_iter().map(|p| (p.name, p.lat, p.lon)).collect();
        self.labels.set_marks(&points);
    }

    /// Левая панель меток: название сцены, дерево с флажками. Двойной
    /// щелчок - перелёт к метке или папке.
    fn marks_panel(&mut self, ui: &mut egui::Ui, now: f64) {
        let mut actions = Vec::new();
        let (title, hint, open_label, time_label) = match self.lang {
            Lang::Ru => ("Метки", "Откройте или бросьте в окно сцену .planetx, KML или KMZ.", "Открыть…", "Время"),
            Lang::En => ("Places", "Open or drop a .planetx scene, KML or KMZ into the window.", "Open…", "Time"),
        };
        let mut open_clicked = false;
        egui::Panel::left("marks").resizable(true).default_size(280.0).show(ui, |ui| {
            ui.add_space(4.0);
            self.basemap_choice(ui, now);
            ui.separator();
            ui.horizontal(|ui| {
                ui.heading(title);
                if ui.button(open_label).clicked() {
                    open_clicked = true;
                }
                if self.timebar.extent.is_some() {
                    ui.toggle_value(&mut self.timebar.open, time_label);
                }
            });
            if let Some(message) = &self.open_message {
                ui.label(message);
            }
            match self.marks.tree.as_mut() {
                Some(tree) => {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        crate::marks::tree_ui(ui, tree, &mut Vec::new(), &mut actions);
                    });
                }
                None => {
                    ui.label(hint);
                }
            }
        });
        if open_clicked {
            self.pick_file(ui.ctx(), now);
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
            }
        }
    }

    /// Выбор файла сцены. В программе - диалог системы, на странице -
    /// выбор файла браузером, байты приходят в ящик брошенных файлов.
    fn pick_file(&mut self, ctx: &egui::Context, now: f64) {
        const KINDS: [&str; 3] = ["planetx", "kml", "kmz"];
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            if let Some(path) = rfd::FileDialog::new().add_filter("PlanetX, KML, KMZ", &KINDS).pick_file() {
                let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
                match std::fs::read(&path) {
                    Ok(bytes) => self.open_file(&bytes, &name, Some(now)),
                    Err(e) => self.open_message = Some(format!("{name}: {e}")),
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = now;
            let (slot, ctx) = (self.dropped_box.clone(), ctx.clone());
            wasm_bindgen_futures::spawn_local(async move {
                if let Some(file) = rfd::AsyncFileDialog::new().add_filter("PlanetX, KML, KMZ", &KINDS).pick_file().await {
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
                Ok(bytes) => self.open_file(&bytes, &name, Some(now)),
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
                Ok(bytes) => self.open_file(&bytes, &name, Some(now)),
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
        if arrived.is_empty() {
            return;
        }
        {
            let mut store = self.heights.write();
            for loaded in arrived {
                let (z, x, y) = loaded.key;
                if let Ok(net::Content::Image(img)) = loaded.content
                    && let Some(tile) = terrain::make_tile(z, x, y, &img.rgba, EARTH_FLOOR, Encoding::Terrarium)
                {
                    store.add(tile);
                }
            }
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

    /// Выбор основы и строка поиска в углу окна.
    /// Поиск места и выбор основы - верх левой панели, как в Google Earth.
    fn basemap_choice(&mut self, ui: &mut egui::Ui, now: f64) {
        let mut chosen = self.basemap;
        let edit = ui.add(
            egui::TextEdit::singleline(&mut self.search_text)
                .hint_text(text(self.lang, Text::SearchHint))
                .desired_width(f32::INFINITY),
        );
        // Запрос только по Enter, правило Nominatim.
        let submitted = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
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
        ui.horizontal(|ui| {
            ui.label(text(self.lang, Text::Basemap));
            egui::ComboBox::from_id_salt("basemap").selected_text(chosen.name(self.lang)).show_ui(ui, |ui| {
                for b in Basemap::ALL {
                    ui.selectable_value(&mut chosen, b, b.name(self.lang));
                }
            });
        });
        self.set_basemap(chosen);
        if submitted && let Some(place) = self.search.submit(&self.search_text) {
            self.fly_to_found(&place, now);
        }
        if let Some(place) = picked.and_then(|n| self.search.found.get(n).cloned()) {
            self.fly_to_found(&place, now);
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.upload();
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
            || self.search.busy();
        if let Some(check) = &mut self.check {
            let mut basemap = self.basemap;
            check.before_frame(&mut self.nav, now, loading, &mut basemap);
            self.set_basemap(basemap);
        } else {
            self.input.handle(&ctx, rect, &mut self.nav, now, ppp);
        }
        if let Some(query) = self.check.as_mut().and_then(Check::take_search) {
            self.search_text = query.clone();
            if let Some(place) = self.search.submit(&query) {
                self.fly_to_found(&place, now);
            }
        }
        if let Some(place) = self.search.step(now, self.loaders.search.as_ref()) {
            self.fly_to_found(&place, now);
        }
        if let Some((bytes, name)) = self.pending_open.take() {
            self.open_file(&bytes, &name, None);
        }
        self.dropped(&ctx, now);
        self.step_tour(now);
        self.timebar.step(now);
        if self.marks.set_window(self.timebar.window()) {
            self.marks_to_labels();
        }
        let moving = self.nav.step(now) || self.player.is_some() || self.timebar.playing();
        // Ближняя плоскость - от ближайшего рельефа.
        let eye = self.nav.camera.eye;
        self.nav.camera.nearest =
            Some(nearest_terrain(&self.nav.camera.ellipsoid, eye, self.nav.pose.terrain.as_ref()));

        let Some(globe) = &mut self.globe else {
            ui.centered_and_justified(|ui| ui.label(text(self.lang, Text::NoGpu)));
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
        self.loaders.terrain.want(&height_needs(&store, &selection));
        self.loaders.places.want(&self.labels.wanted(&selection.draw));

        if let Some(meshes) = self.marks.meshes(&store, now) {
            globe.set_features(&meshes);
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
        let blocked: Vec<egui::Rect> = self.credits.rect.into_iter().collect();
        let painter = ui.painter().with_clip_rect(rect);
        self.labels.draw(&painter, rect, ppp as f32, &blocked, &self.nav.camera, &selection.draw, &self.heights.read());
        self.attribution(&ctx, rect);
        self.overlays(&ctx, rect, now);

        if let Some(check) = &mut self.check {
            check.after_frame(&stats, &self.nav, &self.heights.read());
            check.note_labels(&self.labels.drawn, rect);
        }
        self.finish_check(&ctx);
        self.step_shot(&ctx, moving || loading);
        if moving || loading || self.labels.pending || self.check.is_some() {
            ctx.request_repaint();
        }
    }
}
