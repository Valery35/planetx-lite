// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Минералы EMIT» подменю «Анализ участка», перенос `ui/emit.py`
//! модуля. Расчёт - `data::emit`.
//!
//! Просьба автора от 2 ноября 2026 года. Сцены участка - каталог NASA
//! CMR, файл сцены - netCDF-4 по ключу Earthdata пользователя,
//! скачивается один раз в папку `emit` папки данных. Полоса съёмки
//! переводится на сетку участка в UTM по таблице GLT ближайшим соседом.
//! Классы - минералы выбранной группы, показ - на общем слое вида окон
//! анализа, «OK» кладёт картинки минералов и глубины полосы в «Мои
//! метки» и пишет их файлами GeoTIFF. Хранилище LP DAAC не отдаёт файлы
//! страницам в браузере, у страницы окна нет.
//!
//! Ключ Earthdata вводит пользователь в окне, он хранится в настройках
//! программы, как ключи помощника ИИ. Запрос файла идёт с заголовком
//! Authorization, переадресация в хранилище его не несёт. Вместо файла
//! пришла страница входа - ключ не принят.

use std::sync::Arc;

use data::aef::{self, ZoneIndex};
use data::emit::{self, Scene};
use eframe::egui;
use globe_core::navigation::Navigator;

use crate::aefcalc::{Request, utm_epsg};
use crate::aefclass::Layer;
use crate::analysis::{self, Button, Saved, SiteEdit};
use crate::i18n::{Lang, tr, trf};

/// Участок по умолчанию и наибольший, километры, `SIDE` и `MOST_SIDE`
/// модуля.
const SIDE: f64 = 10.0;
const MOST: f64 = 200.0;
const OPACITY: f64 = 0.7;
/// Облачность до, проценты, и группа по умолчанию - как у модуля.
const CLOUD: f64 = 30.0;
const GROUP: u32 = 2;
/// Сторона картинки сцены в окне, точки экрана, `PREVIEW` модуля.
const PREVIEW: f32 = 220.0;
/// Предел ответа каталога и картинки, файла сцены, байты.
const ANSWER: u64 = 16 << 20;
const FILE: u64 = 1 << 30;
/// Шкала глубины полосы поглощения: малая - светлее, `DEPTH_STOPS`
/// модуля.
const DEPTH_STOPS: [(f64, [u8; 3]); 4] = [(0.0, [255, 255, 204]), (0.1, [253, 141, 60]), (0.25, [189, 0, 38]), (0.5, [60, 0, 30])];
/// Страница ключей Earthdata.
pub const TOKENS: &str = "https://urs.earthdata.nasa.gov/profile";

/// Запрос каталога одной коллекции и его ответ: сцены или ошибка.
type Ask = (Box<dyn net::Request>, Option<Result<Vec<Scene>, String>>);

/// Шаг окна.
enum Stage {
    /// Поиск сцен: ответы коллекций по порядку [`emit::COLLECTIONS`].
    Search(Vec<Ask>),
    /// Загрузка файла выбранной сцены.
    File { scene: usize, ask: Box<dyn net::Request> },
    /// Файл есть: чтение и расчёт следующим кадром, строка «Минералы
    /// сцены…» успевает показаться.
    Compute { scene: usize, bytes: Arc<Vec<u8>>, shown: bool },
}

/// Готовый расчёт.
struct Done {
    index: Arc<ZoneIndex>,
    grid: emit::SiteGrid,
    depth: Vec<f32>,
    names: Vec<String>,
    label: String,
}

/// Окно «Минералы EMIT».
pub struct Emit {
    pub open: bool,
    place: String,
    pub site: SiteEdit,
    start: String,
    end: String,
    cloud: f64,
    group: u32,
    scenes: Vec<Scene>,
    selected: Option<usize>,
    preview: Option<(usize, egui::TextureHandle)>,
    preview_ask: Option<(usize, Box<dyn net::Request>)>,
    stage: Option<Stage>,
    status: String,
    /// Ключ Earthdata из настроек, его ставит окно глобуса.
    pub key: String,
    /// Новый ключ из поля окна: окно глобуса пишет его в настройки.
    pub key_new: Option<String>,
    key_input: String,
    /// Папка данных программы для файлов сцен.
    pub dir: Option<std::path::PathBuf>,
    pub layer: Option<Arc<Layer>>,
    done: Option<Done>,
    pub changed: bool,
    pub saved: Option<Saved>,
    /// Поиск нужно начать: окно открыто или участок изменён.
    search_wanted: bool,
}

/// Сегодня «ГГГГ-ММ-ДД» UTC.
fn today() -> String {
    let (y, m, d) = crate::pointcalc::civil_from_days((crate::pointcalc::clock_now() / 86400.0).floor() as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Дата «ГГГГ-ММ-ДД» годна.
fn date_ok(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    matches!(parts.as_slice(), [y, m, d] if y.len() == 4 && m.len() == 2 && d.len() == 2 && [y, m, d].iter().all(|p| p.chars().all(|c| c.is_ascii_digit())))
}

impl Emit {
    pub fn new(lat: f64, lon: f64, place: String) -> Self {
        Self {
            open: true,
            place,
            site: SiteEdit::new(lat, lon, SIDE, MOST),
            start: emit::FIRST_DAY.to_string(),
            end: today(),
            cloud: CLOUD,
            group: GROUP,
            scenes: Vec::new(),
            selected: None,
            preview: None,
            preview_ask: None,
            stage: None,
            status: String::new(),
            key: String::new(),
            key_new: None,
            key_input: String::new(),
            dir: None,
            layer: None,
            done: None,
            changed: false,
            saved: None,
            search_wanted: true,
        }
    }

    pub fn set_point(&mut self, lat: f64, lon: f64, place: String) {
        self.place = place;
        self.status.clear();
        self.open = true;
        self.site.move_to(lat, lon);
        self.search_wanted = true;
    }

    pub fn busy(&self) -> bool {
        self.stage.is_some() || self.preview_ask.is_some()
    }

    pub fn site_edit(&mut self) -> Option<&mut SiteEdit> {
        if self.open { Some(&mut self.site) } else { None }
    }

    /// Снять результат с глобуса, окно остаётся.
    pub fn drop_result(&mut self) {
        self.done = None;
        if self.layer.take().is_some() {
            self.changed = true;
        }
    }

    fn clear(&mut self) {
        self.stage = None;
        self.drop_result();
        self.status.clear();
    }

    /// Поиск сцен над участком в двух коллекциях, `search` модуля.
    fn search(&mut self, lang: Lang, request: Request) {
        if let Some(problem) = self.site.problem(lang) {
            self.status = problem;
            return;
        }
        if !date_ok(&self.start) || !date_ok(&self.end) {
            self.status = tr(lang, "Dates are written as YYYY-MM-DD.").to_string();
            return;
        }
        let ring = self.site.site.ring.clone();
        let asks = emit::COLLECTIONS
            .iter()
            .map(|(collection, _)| {
                let ask = request(ANSWER);
                ask.start(&emit::cmr_url(&ring, &self.start, &self.end, self.cloud, collection));
                (ask, None)
            })
            .collect();
        self.scenes.clear();
        self.selected = None;
        self.preview = None;
        self.stage = Some(Stage::Search(asks));
        self.status = tr(lang, "Searching for EMIT scenes…").to_string();
    }

    /// Доли участка под сценами, сцены вне участка уходят.
    fn shares(&mut self) {
        let ring = self.site.site.ring.clone();
        for s in &mut self.scenes {
            s.share = emit::coverage(&s.polygon, &ring);
        }
        self.scenes.retain(|s| s.share > 0.0);
        self.selected = (!self.scenes.is_empty()).then_some(0);
        self.preview = None;
    }

    /// Путь файла сцены в папке данных.
    fn path_of(&self, scene: &Scene) -> Option<std::path::PathBuf> {
        let name = scene.url.rsplit('/').next().filter(|n| !n.is_empty() && !n.contains(['\\', ':']))?;
        Some(self.dir.as_ref()?.join("emit").join(name))
    }

    /// «Построить», `build` модуля: файл есть - расчёт, иначе загрузка
    /// по ключу.
    fn build(&mut self, lang: Lang, request: Request) {
        if let Some(problem) = self.site.problem(lang) {
            self.status = problem;
            return;
        }
        let Some((n, scene)) = self.selected.and_then(|n| Some((n, self.scenes.get(n)?.clone()))) else {
            self.status = tr(lang, "Choose a scene in the list.").to_string();
            return;
        };
        if let Some(bytes) = self.path_of(&scene).and_then(|p| std::fs::read(p).ok()).filter(|b| !b.is_empty()) {
            self.stage = Some(Stage::Compute { scene: n, bytes: Arc::new(bytes), shown: false });
            return;
        }
        if self.key.trim().is_empty() {
            self.status = tr(lang, "Without an Earthdata key NASA files are not loaded.").to_string();
            return;
        }
        let ask = request(FILE);
        let header = format!("Bearer {}", self.key.trim());
        ask.send(&scene.url, &[("Authorization", header.as_str())], None);
        self.stage = Some(Stage::File { scene: n, ask });
        self.status = trf(lang, "Loading the file of the scene {when}…", &[("when", scene.when.get(..16).unwrap_or(&scene.when))]);
    }

    /// Шаг окна: ответы каталога, картинка сцены, файл, расчёт.
    pub fn step(&mut self, ctx: &egui::Context, lang: Lang, request: Request) {
        if self.search_wanted && self.open && self.stage.is_none() && !self.site.site.drawing {
            self.search_wanted = false;
            self.search(lang, request);
        }
        if let Some((n, ask)) = &self.preview_ask
            && let Some(answer) = ask.take()
        {
            let n = *n;
            self.preview_ask = None;
            if let Some(rgba) = answer.ok().and_then(|b| image::load_from_memory(&b).ok()).map(|i| i.to_rgba8()) {
                let size = [rgba.width() as usize, rgba.height() as usize];
                self.preview = Some((n, ctx.load_texture("planetx-emit", egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()), egui::TextureOptions::LINEAR)));
            }
        }
        if let Some(n) = self.selected
            && self.preview.as_ref().is_none_or(|p| p.0 != n)
            && self.preview_ask.as_ref().is_none_or(|p| p.0 != n)
            && let Some(url) = self.scenes.get(n).map(|s| s.preview.clone()).filter(|u| !u.is_empty())
        {
            let ask = request(ANSWER);
            ask.start(&url);
            self.preview_ask = Some((n, ask));
        }
        match self.stage.take() {
            Some(Stage::Search(mut asks)) => {
                for (ask, slot) in &mut asks {
                    if slot.is_none()
                        && let Some(answer) = ask.take()
                    {
                        let parsed = answer.and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).map_err(|e| e.to_string()));
                        *slot = Some(parsed.map(|v| emit::parse_cmr(&v, "")));
                    }
                }
                if asks.iter().any(|(_, s)| s.is_none()) {
                    self.stage = Some(Stage::Search(asks));
                    return;
                }
                let mut error = None;
                let mut lists = Vec::new();
                for ((_, slot), (_, version)) in asks.into_iter().zip(emit::COLLECTIONS) {
                    match slot {
                        Some(Ok(scenes)) => lists.push(scenes.into_iter().map(|s| Scene { version: version.to_string(), ..s }).collect()),
                        Some(Err(e)) => error = error.or(Some(e)),
                        None => {}
                    }
                }
                self.scenes = emit::merge(lists);
                self.shares();
                self.status = match (self.scenes.len(), error) {
                    (0, Some(e)) => trf(lang, "The NASA catalogue did not answer: {error}", &[("error", &e)]),
                    (0, None) => tr(lang, "There are no EMIT scenes over the area. EMIT images arid regions - deserts, semi-deserts, steppes.").to_string(),
                    (n, _) => trf(lang, "Scenes over the area: {n}.", &[("n", &n.to_string())]),
                };
            }
            Some(Stage::File { scene, ask }) => match ask.take() {
                None => self.stage = Some(Stage::File { scene, ask }),
                Some(Err(e)) => self.status = trf(lang, "The NASA file was not received: {error}", &[("error", &e)]),
                Some(Ok(bytes)) if !bytes.starts_with(b"\x89HDF") => {
                    // Вместо файла - страница входа: ключ устарел или неверен.
                    self.key.clear();
                    self.key_new = Some(String::new());
                    self.status = tr(lang, "The Earthdata key was not accepted - it is outdated or wrong. Enter a new one.").to_string();
                }
                Some(Ok(bytes)) => {
                    if let Some(path) = self.scenes.get(scene).and_then(|s| self.path_of(s)) {
                        let saved = path.parent().map(std::fs::create_dir_all).transpose().and_then(|_| {
                            let part = path.with_extension("nc.part");
                            std::fs::write(&part, &bytes)?;
                            std::fs::rename(&part, &path)
                        });
                        if let Err(e) = saved {
                            self.status = trf(lang, "The file is not saved: {error}", &[("error", &e.to_string())]);
                        }
                    }
                    self.stage = Some(Stage::Compute { scene, bytes: Arc::new(bytes), shown: false });
                }
            },
            Some(Stage::Compute { scene, bytes, shown }) => {
                if !shown {
                    let when = self.scenes.get(scene).map_or(String::new(), |s| s.when.chars().take(16).collect());
                    self.status = trf(lang, "Minerals of the scene {when}…", &[("when", &when)]);
                    self.stage = Some(Stage::Compute { scene, bytes, shown: true });
                    return;
                }
                self.compute(lang, scene, &bytes);
            }
            None => {}
        }
        if self.busy() {
            ctx.request_repaint();
        }
    }

    /// Минералы сцены на участке, `_compute` и `_built` модуля.
    fn compute(&mut self, lang: Lang, scene: usize, bytes: &[u8]) {
        let Some(scene) = self.scenes.get(scene).cloned() else {
            return;
        };
        let minerals = match emit::read_file(bytes, self.group) {
            Ok(m) => m,
            Err(_) => {
                self.status = tr(lang, "The file of the scene is not read.").to_string();
                return;
            }
        };
        let (lat, lon) = self.site.site.centre();
        let Some(index) = ZoneIndex::bare(&aef::zone_of(lon, lat)) else {
            return;
        };
        let grid = emit::to_site(&minerals, &index, self.site.site.bbox());
        let mosaic = aef::Mosaic { x0: grid.x0, y0: grid.y0, step: grid.step, rows: grid.rows, cols: grid.cols, depth: 1, values: vec![0u8; grid.rows * grid.cols] };
        let inside = crate::aefclass::ring_mask(&index, &mosaic, &self.site.site.ring);
        let cells = inside.iter().zip(&grid.seen).filter(|(i, s)| **i && **s).count();
        if cells == 0 {
            self.drop_result();
            self.status = tr(lang, "The area is outside the scene.").to_string();
            return;
        }
        let (kinds, kept) = emit::classes(&grid.ids, &grid.depth, Some(&inside));
        let built = analysis::built_text(lang, crate::pointcalc::clock_now());
        if kept.is_empty() {
            self.drop_result();
            self.status = format!("{} {built}", tr(lang, "No minerals of this group were found in the area."));
            return;
        }
        let names: Vec<String> = kept.iter().map(|m| if *m < 0 { tr(lang, "Others").to_string() } else { minerals.names.iter().find(|n| n.0 == *m).map_or_else(|| m.to_string(), |n| n.1.clone()) }).collect();
        let parts: Vec<String> = emit::shares(&kinds, kept.len()).iter().take(6).map(|(n, share)| format!("{} {:.0}%", names[*n], 100.0 * share)).collect();
        let found = kinds.iter().filter(|k| **k >= 0).count();
        self.status = format!(
            "{} {built}",
            trf(lang, "Minerals: {list}. Cells with a mineral {share} % of the area, {area} km².", &[("list", &parts.join(", ")), ("share", &format!("{:.0}", 100.0 * found as f64 / cells as f64)), ("area", &format!("{:.1}", emit::area_km2(found)))])
        );
        let depth = grid.depth.iter().zip(&inside).map(|(d, i)| if *i { *d } else { f32::NAN }).collect();
        let colors = kept.iter().map(|m| emit::color(*m)).collect();
        let index = Arc::new(index);
        self.layer = Some(Arc::new(Layer { index: index.clone(), step: grid.step, x0: grid.x0, y0: grid.y0, rows: grid.rows, cols: grid.cols, classes: kinds, colors, opacity: OPACITY, objects: None }));
        let label = trf(lang, "{when}, group {group}", &[("when", scene.when.get(..10).unwrap_or(&scene.when)), ("group", &self.group.to_string())]);
        self.done = Some(Done { index, grid, depth, names, label });
        self.changed = true;
    }

    /// Минералы и глубина полосы - расчётом для «Моих меток», `export`
    /// модуля.
    fn save(&mut self, lang: Lang) {
        let (Some(layer), Some(done)) = (&self.layer, &self.done) else {
            return;
        };
        let g = &done.grid;
        let mercator = data::geotiff::MercatorGrid { x0: g.x0 + g.step / 2.0, y0: g.y0 - g.step / 2.0, step: g.step, cols: g.cols, rows: g.rows };
        let epsg = utm_epsg(&done.index);
        let bytes: Vec<u8> = layer.classes.iter().map(|c| u8::try_from(*c).unwrap_or(data::geotiff::BYTE_NODATA)).collect();
        let classes_tiff = data::geotiff::write_byte_projected(&mercator, epsg, &bytes).unwrap_or_default();
        let depth: Vec<f64> = done.depth.iter().map(|d| if d.is_finite() { f64::from(*d) } else { data::geotiff::NODATA }).collect();
        let depth_tiff = data::geotiff::write_float_projected(&mercator, epsg, &depth).unwrap_or_default();
        let shown = layer.clone();
        let sample = move |points: &[(f64, f64)]| shown.sample(points);
        let mut minerals = analysis::picture(trf(lang, "EMIT minerals {name}", &[("name", &done.label)]), "emit_minerals", layer.bbox(), layer.step, &sample, classes_tiff);
        let values: Vec<String> = done.names.iter().enumerate().map(|(n, name)| format!("{n} - {name}")).collect();
        minerals.summary = format!("{} {}", trf(lang, "Values of the GeoTIFF file: {list}.", &[("list", &values.join(", "))]), emit::CREDIT);
        let (index, grid, values) = (done.index.clone(), done.grid.clone(), done.depth.clone());
        let sample_depth = move |points: &[(f64, f64)]| -> Vec<[u8; 4]> {
            points
                .iter()
                .map(|&(lat, lon)| {
                    let (x, y) = index.project(lat, lon);
                    let (c, r) = (((x - grid.x0) / grid.step).floor(), ((grid.y0 - y) / grid.step).floor());
                    let v = if c >= 0.0 && r >= 0.0 && (c as usize) < grid.cols && (r as usize) < grid.rows { values.get(r as usize * grid.cols + c as usize).copied().unwrap_or(f32::NAN) } else { f32::NAN };
                    if v.is_finite() {
                        let [r, g, b] = ramp(f64::from(v));
                        [r, g, b, 255]
                    } else {
                        [0; 4]
                    }
                })
                .collect()
        };
        let mut depth_layer = analysis::picture(tr(lang, "EMIT absorption band depth").to_string(), "emit_depth", layer.bbox(), layer.step, &sample_depth, depth_tiff);
        let finite: Vec<f64> = done.depth.iter().filter(|d| d.is_finite()).map(|d| f64::from(*d)).collect();
        depth_layer.summary = format!("{} {}", analysis::stats_text(lang, tr(lang, "Band depth"), &finite, ""), emit::CREDIT).trim().to_string();
        self.saved = Some(Saved { title: tr(lang, "EMIT minerals").to_string(), ring: self.site.site.ring.clone(), layers: vec![minerals, depth_layer], extra: Vec::new(), session: None });
    }

    /// Мышь над глобусом: только участок.
    pub fn input(&mut self, ctx: &egui::Context, response: &egui::Response, rect: egui::Rect, ppp: f64, nav: &Navigator) -> bool {
        self.open && self.site.input(ctx, response, rect, ppp, nav)
    }

    /// Участок поверх глобуса.
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect, ppp: f64, nav: &Navigator) {
        if self.open {
            self.site.paint(painter, rect, ppp, nav);
        }
    }

    /// Окно.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang, request: Request) {
        if !self.open {
            return;
        }
        if let Some(text) = self.site.shared_note(lang) {
            self.status = text;
        }
        let mut open = true;
        let mut pressed = None;
        let mut find = false;
        let group = self.group;
        egui::Window::new(tr(lang, "EMIT minerals")).id(egui::Id::unique("planetx-emit")).open(&mut open).collapsible(false).resizable(false).min_width(420.0).show(ctx, |ui| {
            ui.add(egui::Label::new(tr(lang, "NASA mineral maps from images of the EMIT spectrometer on the ISS, pixel 60 m, arid regions since 2022. NASA gives the scene files by an Earthdata key.")).wrap());
            egui::Grid::new("planetx-emit-form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Point"));
                ui.label(&self.place);
                ui.end_row();
                ui.label(tr(lang, "Area|site"));
                self.site.row(ui, lang);
                ui.end_row();
                ui.label(tr(lang, "Dates"));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.start).desired_width(90.0));
                    ui.label("-");
                    ui.add(egui::TextEdit::singleline(&mut self.end).desired_width(90.0));
                    find = ui.button(tr(lang, "Find scenes")).clicked();
                });
                ui.end_row();
                ui.label(tr(lang, "Cloud cover up to"));
                ui.add(egui::DragValue::new(&mut self.cloud).range(0.0..=100.0).speed(1.0).suffix(" %"))
                    .on_hover_text(tr(lang, "The largest cloud cover of a scene by the NASA catalogue. Clouds hide the minerals, the cells under them stay without a class."));
                ui.end_row();
                ui.label(tr(lang, "Scenes"));
                egui::ScrollArea::vertical().id_salt("planetx-emit-scenes").max_height(120.0).auto_shrink([false, true]).show(ui, |ui| {
                    for (n, s) in self.scenes.iter().enumerate() {
                        let text = trf(lang, "{when}, cloud cover {cloud} %, area {share} %", &[("when", s.when.get(..16).unwrap_or(&s.when)), ("cloud", &format!("{:.0}", s.cloud)), ("share", &format!("{:.0}", 100.0 * s.share))]);
                        if ui.selectable_label(self.selected == Some(n), text).on_hover_text(&s.id).clicked() {
                            self.selected = Some(n);
                        }
                    }
                });
                ui.end_row();
                if let Some((n, texture)) = &self.preview
                    && self.selected == Some(*n)
                {
                    ui.label("");
                    let size = texture.size_vec2();
                    let scale = (PREVIEW / size.x.max(size.y).max(1.0)).min(1.0);
                    ui.add(egui::Image::new(texture).fit_to_exact_size(size * scale));
                    ui.end_row();
                }
                ui.label(tr(lang, "Group"));
                egui::ComboBox::from_id_salt("planetx-emit-group")
                    .selected_text(group_name(lang, self.group))
                    .show_ui(ui, |ui| {
                        for g in emit::GROUPS {
                            ui.selectable_value(&mut self.group, g, group_name(lang, g));
                        }
                    })
                    .response
                    .on_hover_text(tr(lang, "Group 1 - iron absorption bands in the visible and near infrared, group 2 - Al-OH, Mg-OH and carbonate bands in the shortwave infrared. A cell may have minerals of both groups."));
                ui.end_row();
                if self.key.trim().is_empty() {
                    ui.label(tr(lang, "Earthdata key"));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.key_input).password(true).desired_width(200.0));
                        if ui.button(tr(lang, "Save")).clicked() && !self.key_input.trim().is_empty() {
                            self.key = self.key_input.trim().to_string();
                            self.key_new = Some(self.key.clone());
                            self.key_input.clear();
                        }
                        ui.hyperlink_to(tr(lang, "Get a key"), TOKENS);
                    });
                    ui.end_row();
                }
                ui.label(tr(lang, "Opacity"));
                let mut opacity = self.layer.as_ref().map_or(OPACITY, |l| l.opacity);
                if ui.add(egui::Slider::new(&mut opacity, 0.0..=1.0).show_value(false)).changed()
                    && let Some(old) = &self.layer
                {
                    self.layer = Some(Arc::new(Layer { index: old.index.clone(), step: old.step, x0: old.x0, y0: old.y0, rows: old.rows, cols: old.cols, classes: old.classes.clone(), colors: old.colors.clone(), opacity, objects: None }));
                    self.changed = true;
                }
                ui.end_row();
            });
            if let Some(done) = &self.done {
                ui.label(tr(lang, "Legend"));
                egui::ScrollArea::vertical().id_salt("planetx-emit-legend").max_height(110.0).show(ui, |ui| {
                    for (n, name) in done.names.iter().enumerate() {
                        let [r, g, b] = self.layer.as_ref().and_then(|l| l.colors.get(n)).copied().unwrap_or([128, 128, 128]);
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 2.0, egui::Color32::from_rgb(r, g, b));
                            ui.label(name);
                        });
                    }
                });
            }
            ui.add_space(4.0);
            ui.add(egui::Label::new(self.site.hint(lang).unwrap_or(&self.status)).wrap());
            ui.add(egui::Label::new(egui::RichText::new(emit::CREDIT).small()).wrap());
            ui.add_space(4.0);
            pressed = analysis::buttons(ui, lang, self.done.is_some() && !self.busy(), tr(lang, "Loads the file of the chosen scene (once) and colours the area by the minerals of the chosen group."));
        });
        if find {
            self.search(lang, request);
        }
        if self.group != group {
            self.drop_result();
        }
        match pressed {
            Some(Button::Build) => self.build(lang, request),
            Some(Button::Ok) => {
                self.save(lang);
                self.clear();
                self.open = false;
            }
            Some(Button::Cancel) => {
                self.clear();
                self.open = false;
            }
            None => {}
        }
        if !open {
            self.clear();
            self.open = false;
        }
        if std::mem::take(&mut self.site.changed) && !self.site.site.drawing {
            self.shares_changed(lang);
        }
    }

    /// Участок изменился: доли сцен заново, прежний расчёт - по прежнему
    /// участку.
    fn shares_changed(&mut self, lang: Lang) {
        if self.scenes.is_empty() {
            self.search_wanted = true;
        } else {
            self.shares();
        }
        if self.layer.is_some() && !self.busy() {
            self.status = tr(lang, "The area has changed. Press «Build» to compute it again.").to_string();
        }
    }
}

impl analysis::SiteTool for Emit {
    fn id(&self) -> &'static str {
        "emit"
    }
    fn shown(&self) -> bool {
        self.layer.is_some()
    }
    fn layer(&self) -> Option<Arc<Layer>> {
        self.layer.clone()
    }
    fn painter(&self) -> Option<crate::vector::Painter> {
        self.layer.clone().map(crate::aefclass::painter)
    }
    fn drop_result(&mut self) {
        Emit::drop_result(self);
    }
}

/// Название группы минералов.
fn group_name(lang: Lang, group: u32) -> &'static str {
    if group == 1 { tr(lang, "Group 1 - iron minerals, 0.4-1.3 µm") } else { tr(lang, "Group 2 - clays, mica, carbonates, sulphates, 2.0-2.5 µm") }
}

/// Цвет глубины полосы по шкале [`DEPTH_STOPS`].
fn ramp(depth: f64) -> [u8; 3] {
    let mut prev = DEPTH_STOPS[0];
    for stop in DEPTH_STOPS {
        if depth <= stop.0 {
            let span = stop.0 - prev.0;
            let t = if span > 0.0 { ((depth - prev.0) / span).clamp(0.0, 1.0) } else { 0.0 };
            let mix = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as u8;
            return [mix(prev.1[0], stop.1[0]), mix(prev.1[1], stop.1[1]), mix(prev.1[2], stop.1[2])];
        }
        prev = stop;
    }
    prev.1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_ramp() {
        assert!(date_ok("2022-08-09"));
        assert!(!date_ok("2022-8-9"));
        assert!(!date_ok("2022-08-0x"));
        assert!(date_ok(&today()));
        assert_eq!(ramp(0.0), [255, 255, 204]);
        assert_eq!(ramp(1.0), [60, 0, 30]);
    }
}
