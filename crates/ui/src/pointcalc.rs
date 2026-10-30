// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Расчёты у точки: «Видимость…» и «Инсоляция…» меню на
//! глобусе. Перенос `ui/viewshed.py`, `ui/insolation.py` и расчётной
//! части `ui/globe/tools.py` модуля.
//!
//! «Построить» сначала просит тайлы высот под кругом и ждёт их не дольше
//! [`WAIT`] секунд, потом считает по тем высотам, что есть. Инсоляция
//! снимает высоты узлов сетки и считается частями в бюджете кадра,
//! у страницы WebAssembly потоков нет. Готовый результат - слой над
//! картами: картинку тайла считает [`painter`] без сети, как
//! `ResultTiles` модуля.
//!
//! Флажка «Учитывать здания» модуля пока нет: слоя зданий у Lite нет.
//!
//! Окна - на общем каркасе анализа участка ([`crate::analysis`]):
//! «Построить», «OK» и «Отмена». Инсоляция считается по участку
//! многоугольником в круге, накрывающем его, вне многоугольника часов
//! нет. Видимость - в круге радиуса из окна, контур сохранённого расчёта
//! - этот круг, как у модуля.

use std::sync::Arc;

use eframe::egui;
use globe_core::lod::TileKey;

use data::insolation::{self, Insolation};
use data::shadows::{self, Horizon, Shadows};
use data::viewshed::{self, Viewshed};

use crate::analysis::{self, Button, ResultLayer, Saved, SiteEdit};
use crate::i18n::{Lang, tr, trf};

/// Участок инсоляции по умолчанию и наибольший, километры, `SIDE`
/// и `MOST` окна модуля.
const INSOLATION_SIDE: f64 = 10.0;
const INSOLATION_MOST: f64 = 40.0;
/// Участок теней по умолчанию и наибольший, километры, `SIDE` и `MOST`
/// окна «Тени» модуля.
const SHADOWS_SIDE: f64 = 3.0;
const SHADOWS_MOST: f64 = 40.0;
/// Сдвиг момента, после которого карта теней строится заново, секунды.
const MOMENT_STEP: f64 = 30.0;
/// Вершин круга видимости в «Моих метках», как у модуля.
const CIRCLE_POINTS: usize = 72;

/// Ожидание высот, секунды: дальше расчёт идёт по тем, что есть.
pub const WAIT: f64 = 30.0;
/// Время расчёта за кадр, секунды, `CALC_BUDGET` модуля.
pub const BUDGET: f64 = 0.004;
/// Узлов сетки инсоляции за проход, `HEIGHT_CHUNK` модуля.
pub const HEIGHT_CHUNK: usize = 20000;
/// Сторона картинки тайла.
const SIZE: usize = 256;

/// Что считается.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Viewshed,
    Insolation,
    /// Тени на момент шкалы времени, окно «Тени» модуля.
    Shadows,
}

/// Готовый слой.
#[derive(Clone)]
pub enum Layer {
    Viewshed(Arc<Viewshed>),
    Insolation(Arc<Insolation>),
    Shadows(Arc<Shadows>),
}

/// Ход расчёта.
enum Stage {
    /// Ждём тайлы высот.
    Heights,
    /// Снимаем высоты узлов сетки инсоляции.
    Nodes { points: Vec<(f64, f64)>, heights: Vec<f64> },
    /// Инсоляция считается частями.
    Work(Box<insolation::Work>),
}

struct Job {
    lat: f64,
    lon: f64,
    radius_m: f64,
    keys: Vec<TileKey>,
    started: f64,
    missing: bool,
    stage: Stage,
    /// Видимость: глаз и цель над рельефом. Инсоляция: начало первых
    /// и последних суток UTC.
    a: f64,
    b: f64,
    /// Участок инсоляции.
    ring: Option<Vec<(f64, f64)>>,
}

/// Окно расчёта и его слой.
pub struct Calc {
    pub kind: Kind,
    pub open: bool,
    point: (f64, f64),
    place: String,
    observer: f64,
    target: f64,
    radius_km: f64,
    first: (i64, u32, u32),
    last: (i64, u32, u32),
    status: String,
    job: Option<Job>,
    pub layer: Option<Layer>,
    /// Слой сменился: картинки тайлов рисуются заново.
    pub changed: bool,
    /// Участок инсоляции и теней на глобусе.
    pub site: Option<SiteEdit>,
    /// Расчёт по «OK» для «Моих меток», забирает окно глобуса.
    pub saved: Option<Saved>,
    /// Горизонт участка теней: карта на момент строится по нему.
    horizon: Option<Arc<Horizon>>,
    /// Момент теней, секунды UTC: правый край шкалы времени или часы.
    moment: f64,
}

impl Calc {
    pub fn new(kind: Kind, lat: f64, lon: f64, place: String, now_unix: f64) -> Self {
        let today = civil_from_days((now_unix / insolation::DAY).floor() as i64);
        let site = match kind {
            Kind::Viewshed => None,
            Kind::Insolation => Some(SiteEdit::new(lat, lon, INSOLATION_SIDE, INSOLATION_MOST)),
            Kind::Shadows => Some(SiteEdit::new(lat, lon, SHADOWS_SIDE, SHADOWS_MOST)),
        };
        Self {
            kind,
            open: true,
            point: (lat, lon),
            place,
            observer: 2.0,
            target: 0.0,
            radius_km: 10.0,
            first: today,
            last: today,
            status: String::new(),
            job: None,
            layer: None,
            changed: false,
            site,
            saved: None,
            horizon: None,
            moment: now_unix,
        }
    }

    /// Момент теней: правый край шкалы времени или часы компьютера.
    /// Карта на момент строится заново без загрузки высот, `at` модуля.
    pub fn set_moment(&mut self, lang: Lang, moment: f64) {
        if self.kind != Kind::Shadows || !moment.is_finite() {
            return;
        }
        let moved = (moment - self.moment).abs() >= MOMENT_STEP;
        self.moment = moment;
        if moved && let Some(h) = self.horizon.clone() {
            self.show_shadows(lang, &h);
        }
    }

    /// Карта теней горизонта h на момент окна и строка итога.
    fn show_shadows(&mut self, lang: Lang, h: &Arc<Horizon>) {
        let result = h.at(self.moment);
        self.status = if result.elevation <= 0.0 {
            tr(lang, "The sun is below the horizon, the whole area is in shadow.").to_string()
        } else {
            trf(lang, "Sun on {share} of the area, the sun is {elevation}° above the horizon. The grid step is {step} m.", &[("share", &format!("{:.0}%", 100.0 * result.share())), ("elevation", &format!("{:.1}", result.elevation)), ("step", &format!("{:.0}", h.cell))])
        };
        self.layer = Some(Layer::Shadows(Arc::new(result)));
        self.changed = true;
    }

    /// Новая точка в открытом окне: участок - квадрат у неё, слой
    /// остаётся до «Построить».
    pub fn set_point(&mut self, lat: f64, lon: f64, place: String) {
        self.point = (lat, lon);
        self.place = place;
        self.status.clear();
        self.open = true;
        if let Some(s) = &mut self.site {
            s.move_to(lat, lon);
        }
    }

    /// Идёт расчёт.
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Участок окна на глобусе, пока окно открыто.
    pub fn site_edit(&mut self) -> Option<&mut SiteEdit> {
        if self.open { self.site.as_mut() } else { None }
    }

    fn build(&mut self, lang: Lang, now: f64) {
        let (keys, a, b, lat, lon, radius_m, ring) = match self.kind {
            Kind::Viewshed => {
                let (lat, lon) = self.point;
                let radius_m = self.radius_km * 1000.0;
                let cell = radius_m / viewshed::STEPS as f64;
                (viewshed::height_tiles(lat, lon, radius_m, cell, viewshed::EARTH_RADIUS, terrain::MAX_LEVEL).1, self.observer, self.target, lat, lon, radius_m, None)
            }
            Kind::Insolation | Kind::Shadows => {
                let Some(site) = &self.site else {
                    return;
                };
                if let Some(problem) = site.problem(lang) {
                    self.status = problem;
                    return;
                }
                // Круг, накрывающий участок, как у модуля.
                let (lat, lon) = site.site.centre();
                let radius_m = site.site.reach().max(100.0);
                let (cell, margin, _) = insolation::layout(radius_m);
                let keys = viewshed::height_tiles(lat, lon, margin as f64 * cell + radius_m, cell, viewshed::EARTH_RADIUS, terrain::MAX_LEVEL).1;
                (keys, day_start(self.first), day_start(self.last), lat, lon, radius_m, Some(site.site.ring.clone()))
            }
        };
        self.job = Some(Job { lat, lon, radius_m, keys, started: now, missing: false, stage: Stage::Heights, a, b, ring });
    }

    /// Готовый слой - расчётом для «Моих меток», `_viewshed_done`
    /// и `_insolation_done` модуля.
    fn save(&mut self, lang: Lang) {
        let Some(layer) = &self.layer else {
            return;
        };
        let stamp = clock_now();
        let (title, ring, result) = match layer {
            Layer::Viewshed(v) => {
                let (grid, values) = viewshed::raster(v, viewshed::EARTH_RADIUS);
                let rgba = values
                    .iter()
                    .map(|v| match *v {
                        1 => paint(viewshed::VISIBLE, viewshed::OPACITY),
                        0 => paint(viewshed::HIDDEN, viewshed::OPACITY),
                        _ => [0; 4],
                    })
                    .collect();
                let tiff = data::geotiff::write_byte(&grid, &values).unwrap_or_default();
                let name = trf(lang, "Viewshed {time}", &[("time", &analysis::clock(stamp))]);
                (tr(lang, "Viewshed"), circle(v.lat, v.lon, v.radius_m), ResultLayer { name, grid, rgba, tiff, stem: "viewshed".into(), summary: String::new() })
            }
            Layer::Insolation(r) => {
                let (grid, hours) = r.raster(insolation::EARTH_RADIUS);
                let top = r.top();
                let rgba = hours.iter().map(|h| if h.is_finite() { paint(stop_color((h / top).clamp(0.0, 1.0)), insolation::OPACITY) } else { [0; 4] }).collect();
                let tiff = data::geotiff::write_float(&grid, &hours).unwrap_or_default();
                let name = trf(lang, "Insolation {time}, h per day", &[("time", &analysis::clock(stamp))]);
                let summary = analysis::stats_text(lang, tr(lang, "Sun hours"), &hours, tr(lang, " h"));
                (tr(lang, "Insolation"), r.ring.clone().unwrap_or_default(), ResultLayer { name, grid, rgba, tiff, stem: "insolation".into(), summary })
            }
            Layer::Shadows(s) => {
                let (grid, values) = s.raster();
                let rgba = values
                    .iter()
                    .map(|v| match *v {
                        shadows::SUN => shadows::SUN_RGBA,
                        shadows::SHADE => shadows::SHADE_RGBA,
                        _ => [0; 4],
                    })
                    .collect();
                let tiff = data::geotiff::write_byte(&grid, &values).unwrap_or_default();
                let name = trf(lang, "Shadows {moment} UTC", &[("moment", &moment_text(s.moment))]);
                (tr(lang, "Shadows"), s.horizon.ring.clone().unwrap_or_default(), ResultLayer { name, grid, rgba, tiff, stem: "shadows".into(), summary: String::new() })
            }
        };
        self.saved = Some(Saved { title: title.to_string(), ring, layers: vec![result], extra: Vec::new(), session: None });
    }

    /// Убрать слой и расчёт.
    fn clear(&mut self) {
        self.job = None;
        self.horizon = None;
        if self.layer.take().is_some() {
            self.changed = true;
        }
        self.status.clear();
    }

    /// Шаг расчёта. has - есть ли тайл высот, heights - настоящие высоты
    /// точек. Возвращает тайлы высот, которых ждёт расчёт.
    pub fn step(&mut self, lang: Lang, now: f64, has: &dyn Fn(TileKey) -> bool, heights: viewshed::Heights) -> Vec<TileKey> {
        let Some(job) = &mut self.job else {
            return Vec::new();
        };
        let started = web_time::Instant::now();
        match &mut job.stage {
            Stage::Heights => {
                let missing: Vec<TileKey> = job.keys.iter().copied().filter(|k| !has(*k)).collect();
                if !missing.is_empty() && now - job.started < WAIT {
                    let done = (job.keys.len() - missing.len()).to_string();
                    self.status = trf(lang, "Loading heights: {done} of {total}", &[("done", &done), ("total", &job.keys.len().to_string())]);
                    return missing;
                }
                job.missing = !missing.is_empty();
                match self.kind {
                    Kind::Viewshed => {
                        let params = viewshed::Params { observer: job.a, target: job.b, ..viewshed::Params::default() };
                        let cell = job.radius_m / viewshed::STEPS as f64;
                        let result = viewshed::compute(job.lat, job.lon, job.radius_m, cell, heights, &params);
                        let missing = job.missing;
                        self.job = None;
                        match result {
                            Some(r) => {
                                let share = format!("{:.0}%", 100.0 * r.share());
                                let step = format!("{:.0}", r.step);
                                self.status = trf(lang, "{share} of the circle area is visible. The computation step is {step} m.", &[("share", &share), ("step", &step)]);
                                if missing {
                                    self.status = format!("{} {}", self.status, tr(lang, "Not all heights are loaded, the computation used less detailed ones."));
                                }
                                self.status = format!("{} {}", self.status, analysis::built_text(lang, clock_now()));
                                self.layer = Some(Layer::Viewshed(Arc::new(r)));
                            }
                            None => {
                                self.status = tr(lang, "This body has no heights.").to_string();
                                self.layer = None;
                            }
                        }
                        self.changed = true;
                    }
                    Kind::Insolation | Kind::Shadows => {
                        let points = insolation::grid_points(job.lat, job.lon, job.radius_m, insolation::EARTH_RADIUS);
                        job.stage = Stage::Nodes { heights: Vec::with_capacity(points.len()), points };
                    }
                }
            }
            Stage::Nodes { points, heights: got } => {
                while got.len() < points.len() && started.elapsed().as_secs_f64() < BUDGET * 2.0 {
                    let end = (got.len() + HEIGHT_CHUNK).min(points.len());
                    let part = points.get(got.len()..end).unwrap_or(&[]);
                    got.extend(heights(part));
                    if got.len() < end {
                        // Высот вернулось меньше точек: расчёт невозможен.
                        self.job = None;
                        self.status = tr(lang, "This body has no heights.").to_string();
                        return Vec::new();
                    }
                }
                if got.len() < points.len() {
                    let share = format!("{:.0}%", 100.0 * got.len() as f64 / points.len().max(1) as f64);
                    self.status = trf(lang, "Heights of the grid nodes: {share}", &[("share", &share)]);
                    return Vec::new();
                }
                let work = match self.kind {
                    Kind::Shadows => insolation::Work::horizon(job.lat, job.lon, job.radius_m, got, insolation::EARTH_RADIUS),
                    _ => insolation::Work::new(job.lat, job.lon, job.radius_m, got, job.a, job.b, insolation::EARTH_RADIUS),
                };
                match work {
                    Some(w) => job.stage = Stage::Work(Box::new(w)),
                    None => {
                        self.job = None;
                        self.status = tr(lang, "This body has no heights.").to_string();
                    }
                }
            }
            Stage::Work(work) => {
                let mut share = 0.0;
                let mut done = false;
                while started.elapsed().as_secs_f64() < BUDGET {
                    match work.step() {
                        insolation::Step::Going(s) => share = s,
                        insolation::Step::Done => {
                            done = true;
                            break;
                        }
                    }
                }
                if !done {
                    let text = if self.kind == Kind::Shadows { "Computing the horizon: {share}" } else { "Computing insolation: {share}" };
                    self.status = trf(lang, text, &[("share", &format!("{:.0}%", 100.0 * share))]);
                    return Vec::new();
                }
                let missing = job.missing;
                let ring = job.ring.take();
                if self.kind == Kind::Shadows {
                    let (lat, lon, radius_m) = (job.lat, job.lon, job.radius_m);
                    let horizon = self.job.take().and_then(|j| match j.stage {
                        Stage::Work(w) => Horizon::from_work(*w, lat, lon, radius_m, insolation::EARTH_RADIUS, ring),
                        _ => None,
                    });
                    if let Some(h) = horizon.map(Arc::new) {
                        self.horizon = Some(h.clone());
                        self.show_shadows(lang, &h);
                        if missing {
                            self.status = format!("{} {}", self.status, tr(lang, "Not all heights are loaded, the computation used less detailed ones."));
                        }
                        self.status = format!("{} {}", self.status, analysis::built_text(lang, clock_now()));
                    }
                    return Vec::new();
                }
                let result = self.job.take().and_then(|j| match j.stage {
                    Stage::Work(w) => w.into_result(),
                    _ => None,
                });
                if let Some(mut r) = result {
                    r.ring = ring;
                    let low = r.hours.iter().copied().fold(f64::INFINITY, f64::min);
                    let high = r.hours.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    self.status = trf(
                        lang,
                        "Over {days} days direct sun shines from {low} to {high} h per day. The grid step is {step} m.",
                        &[("days", &r.days.to_string()), ("low", &format!("{low:.1}")), ("high", &format!("{high:.1}")), ("step", &format!("{:.0}", r.cell))],
                    );
                    let summary = analysis::stats_text(lang, tr(lang, "Sun hours"), &r.hours, tr(lang, " h"));
                    if !summary.is_empty() {
                        self.status = format!("{} {summary}", self.status);
                    }
                    if missing {
                        self.status = format!("{} {}", self.status, tr(lang, "Not all heights are loaded, the computation used less detailed ones."));
                    }
                    self.status = format!("{} {}", self.status, analysis::built_text(lang, clock_now()));
                    self.layer = Some(Layer::Insolation(Arc::new(r)));
                    self.changed = true;
                }
            }
        }
        Vec::new()
    }

    /// Окно. now - секунды кадра для начала расчёта.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang, now: f64) {
        if !self.open {
            return;
        }
        if let Some(text) = self.site.as_mut().and_then(|s| s.shared_note(lang)) {
            self.status = text.to_string();
        }
        let mut open = true;
        let mut pressed = None;
        let title = match self.kind {
            Kind::Viewshed => tr(lang, "Viewshed from a point"),
            Kind::Insolation => tr(lang, "Insolation"),
            Kind::Shadows => tr(lang, "Shadows"),
        };
        egui::Window::new(title).id(egui::Id::unique(("planetx-calc", self.kind as u8))).open(&mut open).collapsible(false).resizable(false).min_width(360.0).show(ctx, |ui| {
            egui::Grid::new(("planetx-calc-form", self.kind as u8)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Point"));
                ui.label(&self.place);
                ui.end_row();
                match self.kind {
                    Kind::Viewshed => {
                        ui.label(tr(lang, "Observer height"));
                        ui.add(egui::DragValue::new(&mut self.observer).range(0.0..=10000.0).speed(0.5).fixed_decimals(1).suffix(tr(lang, " m")))
                            .on_hover_text(tr(lang, "Height of the eye or mast above the terrain at the point. A higher value sees farther and fewer places stay hidden behind the terrain."));
                        ui.end_row();
                        ui.label(tr(lang, "Target height"));
                        ui.add(egui::DragValue::new(&mut self.target).range(0.0..=10000.0).speed(0.5).fixed_decimals(1).suffix(tr(lang, " m")))
                            .on_hover_text(tr(lang, "Height above the terrain of what has to be seen, for example a mast or a building. At zero the ground itself is checked."));
                        ui.end_row();
                        ui.label(tr(lang, "Radius"));
                        ui.add(egui::DragValue::new(&mut self.radius_km).range(0.1..=100.0).speed(0.1).fixed_decimals(1).suffix(tr(lang, " km")))
                            .on_hover_text(tr(lang, "Radius of the computed circle. A larger radius gives a coarser step, the step is about one five-hundredth of the radius."));
                        ui.end_row();
                    }
                    Kind::Insolation => {
                        ui.label(tr(lang, "First day"));
                        if date_edit(ui, &mut self.first).on_hover_text(tr(lang, "The first day of the period. The result is the mean number of hours of direct sunlight per day over the period.")).changed() && self.last < self.first {
                            self.last = self.first;
                        }
                        ui.end_row();
                        ui.label(tr(lang, "Last day"));
                        if date_edit(ui, &mut self.last).on_hover_text(tr(lang, "The last day of the period, inclusive. A long period is computed over 15 evenly spaced days.")).changed() && self.first > self.last {
                            self.first = self.last;
                        }
                        ui.end_row();
                        if let Some(site) = &mut self.site {
                            ui.label(tr(lang, "Area|site"));
                            site.row(ui, lang);
                            ui.end_row();
                        }
                    }
                    Kind::Shadows => {
                        ui.label(tr(lang, "Moment"));
                        ui.label(trf(lang, "{moment} UTC", &[("moment", &moment_text(self.moment))])).on_hover_text(tr(
                            lang,
                            "The moment is the right handle of the time slider, without the slider - the computer clock. After «Build» the shadows follow the slider: move the handle or start the playback.",
                        ));
                        ui.end_row();
                        if let Some(site) = &mut self.site {
                            ui.label(tr(lang, "Area|site"));
                            site.row(ui, lang);
                            ui.end_row();
                        }
                    }
                }
            });
            ui.add_space(4.0);
            let hint = self.site.as_ref().and_then(|s| s.hint(lang));
            ui.add(egui::Label::new(hint.unwrap_or(&self.status)).wrap());
            ui.add_space(4.0);
            let build_tip = match self.kind {
                Kind::Viewshed => tr(lang, "What is seen from the point in the circle - a layer on the globe."),
                Kind::Insolation => tr(lang, "Hours of direct sun per day in the area - a layer on the globe. Mountain shadows are taken from a distance of at least half the area and at least 5 km."),
                Kind::Shadows => tr(lang, "Where it is shadow and where sun at the moment of the time slider - by the terrain, the shadows follow the slider. Mountain shadows are taken from a distance of at least half the area and at least 5 km."),
            };
            pressed = analysis::buttons(ui, lang, self.layer.is_some() && self.job.is_none(), build_tip);
        });
        match pressed {
            Some(Button::Build) => self.build(lang, now),
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
        // Крестик - как «Отмена», `reject` модуля.
        if !open {
            self.clear();
            self.open = false;
        }
        // Участок сменился: прежний слой построен не по нему.
        if let Some(site) = &mut self.site
            && std::mem::take(&mut site.changed)
            && self.job.is_none()
            && self.layer.is_some()
        {
            self.status = tr(lang, "The area has changed. Press «Build» to compute it again.").to_string();
        }
        if self.job.is_some() {
            ctx.request_repaint();
        }
    }
}

/// Поле даты: день, месяц, год. Возвращает ответ последнего поля
/// с отметкой изменения любого.
pub fn date_edit(ui: &mut egui::Ui, date: &mut (i64, u32, u32)) -> egui::Response {
    ui.horizontal(|ui| {
        let (mut year, mut month, mut day) = *date;
        let a = ui.add(egui::DragValue::new(&mut day).range(1..=31).custom_formatter(|v, _| format!("{v:02}")));
        ui.label(".");
        let b = ui.add(egui::DragValue::new(&mut month).range(1..=12).custom_formatter(|v, _| format!("{v:02}")));
        ui.label(".");
        let mut c = ui.add(egui::DragValue::new(&mut year).range(1950..=2050));
        day = day.min(month_days(year, month));
        if a.changed() || b.changed() || c.changed() {
            *date = (year, month, day);
            c.mark_changed();
        }
        c
    })
    .inner
}

/// Суток в месяце.
fn month_days(year: i64, month: u32) -> u32 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Номер суток от 1970-01-01 по дате, алгоритм Хиннанта.
pub fn days_from_civil(date: (i64, u32, u32)) -> i64 {
    let (y, m, d) = date;
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Дата по номеру суток от 1970-01-01.
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { yoe + era * 400 + 1 } else { yoe + era * 400 }, m, d)
}

/// Начало суток даты, секунды UTC.
fn day_start(date: (i64, u32, u32)) -> f64 {
    days_from_civil(date) as f64 * insolation::DAY
}

/// Дата и время UTC момента unix «ДД.ММ.ГГГГ ЧЧ:ММ», `moment_text` модуля.
pub fn moment_text(unix: f64) -> String {
    let (y, m, d) = civil_from_days((unix / insolation::DAY).floor() as i64);
    format!("{d:02}.{m:02}.{y} {}", analysis::clock(unix).get(..5).unwrap_or_default())
}

/// Часы компьютера, секунды UTC.
pub fn clock_now() -> f64 {
    web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// Цвет без премножения с непрозрачностью opacity, для картинки
/// в «Моих метках».
fn paint(rgb: [u8; 3], opacity: f64) -> [u8; 4] {
    [rgb[0], rgb[1], rgb[2], (opacity * 255.0).round() as u8]
}

/// Круг radius_m метров вокруг точки - CIRCLE_POINTS вершин, контур
/// расчёта видимости, как у модуля.
fn circle(lat: f64, lon: f64, radius_m: f64) -> Vec<(f64, f64)> {
    let reach = radius_m / data::site::METERS_PER_DEGREE;
    let scale = lat.to_radians().cos().max(0.01);
    (0..CIRCLE_POINTS)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / CIRCLE_POINTS as f64;
            (lat + reach * a.cos(), lon + reach * a.sin() / scale)
        })
        .collect()
}

/// Широты строк и долготы столбцов центров пикселей тайла.
pub fn tile_axes(key: TileKey) -> (Vec<f64>, Vec<f64>) {
    let (z, x, y) = key;
    let n = f64::from(1u32 << z.min(30));
    let lats = (0..SIZE).map(|r| (std::f64::consts::PI * (1.0 - 2.0 * (f64::from(y) + (r as f64 + 0.5) / SIZE as f64) / n)).sinh().atan().to_degrees()).collect();
    let lons = (0..SIZE).map(|c| (f64::from(x) + (c as f64 + 0.5) / SIZE as f64) / n * 360.0 - 180.0).collect();
    (lats, lons)
}

/// Картинка тайлов слоя. Тайл вне рамки круга - None без расчёта.
pub fn painter(layer: &Layer) -> crate::vector::Painter {
    let layer = layer.clone();
    let (lat, lon, radius_m) = match &layer {
        Layer::Viewshed(v) => (v.lat, v.lon, v.radius_m),
        Layer::Insolation(r) => (r.lat, r.lon, r.radius_m),
        Layer::Shadows(s) => (s.horizon.lat, s.horizon.lon, s.horizon.radius_m),
    };
    let (south, north, west, east) = viewshed::circle_box(lat, lon, radius_m, viewshed::EARTH_RADIUS);
    Arc::new(move |key: TileKey| {
        let (lats, lons) = tile_axes(key);
        let (top, bottom) = (lats.first().copied()?, lats.last().copied()?);
        let (left, right) = (lons.first().copied()?, lons.last().copied()?);
        if bottom > north || top < south || left > east || right < west {
            return None;
        }
        let points: Vec<(f64, f64)> = lats.iter().flat_map(|&la| lons.iter().map(move |&lo| (la, lo))).collect();
        let pixels = match &layer {
            Layer::Viewshed(v) => viewshed::tile_rgba(v, &points, viewshed::EARTH_RADIUS),
            Layer::Insolation(r) => insolation::tile_rgba(r, &points, insolation::EARTH_RADIUS),
            Layer::Shadows(s) => shadows::tile_rgba(s, &points, insolation::EARTH_RADIUS),
        };
        pixels.iter().any(|p| p[3] > 0).then(|| pixels.concat())
    })
}

/// Шкала инсоляции на глобусе: часы прямого света в сутки от 0 до top.
pub fn legend(ctx: &egui::Context, lang: Lang, top: f64, at: egui::Pos2) {
    egui::Area::new(egui::Id::unique("planetx-insolation-legend")).order(egui::Order::Foreground).fixed_pos(at).interactable(false).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.label(tr(lang, "Direct sun, h per day"));
            let (rect, _) = ui.allocate_exact_size(egui::vec2(180.0, 12.0), egui::Sense::hover());
            let painter = ui.painter();
            let steps = 60;
            for i in 0..steps {
                let share = (i as f64 + 0.5) / f64::from(steps);
                let rgb = stop_color(share);
                let x0 = rect.left() + rect.width() * i as f32 / steps as f32;
                let x1 = rect.left() + rect.width() * (i + 1) as f32 / steps as f32;
                painter.rect_filled(egui::Rect::from_min_max(egui::pos2(x0, rect.top()), egui::pos2(x1 + 0.5, rect.bottom())), 0.0, egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
            }
            ui.horizontal(|ui| {
                ui.set_width(180.0);
                let text = |share: f64| format!("{}", (share * top * 10.0).round() / 10.0);
                ui.label(text(0.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(text(1.0));
                    ui.centered_and_justified(|ui| ui.label(text(0.5)));
                });
            });
        });
    });
}

/// Цвет шкалы `insolation::STOPS` на доле share.
fn stop_color(share: f64) -> [u8; 3] {
    let stops = insolation::STOPS;
    let mut out = stops.last().map_or([0; 3], |s| s.1);
    for w in stops.windows(2) {
        let (a, b) = (w[0], w[1]);
        if share <= b.0 {
            let t = ((share - a.0) / (b.0 - a.0)).clamp(0.0, 1.0);
            for (c, slot) in out.iter_mut().enumerate() {
                *slot = (f64::from(a.1[c]) + t * (f64::from(b.1[c]) - f64::from(a.1[c]))).round() as u8;
            }
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_days_round_trip() {
        assert_eq!(days_from_civil((1970, 1, 1)), 0);
        assert_eq!(days_from_civil((2026, 10, 24)), 20750);
        assert_eq!(civil_from_days(20750), (2026, 10, 24));
        assert_eq!(civil_from_days(days_from_civil((2024, 2, 29))), (2024, 2, 29));
    }

    #[test]
    fn insolation_with_area_paints_its_tile() {
        let flat = |p: &[(f64, f64)]| vec![0.0; p.len()];
        let day = days_from_civil((2026, 6, 21)) as f64 * insolation::DAY;
        let result = insolation::compute(58.0, 56.25, 1000.0, &flat, day, day, insolation::EARTH_RADIUS);
        assert!(result.is_some());
        let Some(mut r) = result else { return };
        r.ring = Some(data::site::square_ring(58.0, 56.25, 1500.0));
        let p = painter(&Layer::Insolation(Arc::new(r)));
        // Тайл уровня 14 с точкой.
        let z = 14u32;
        let n = f64::from(1u32 << z);
        let x = ((56.25 + 180.0) / 360.0 * n) as u32;
        let y = ((1.0 - 58.0f64.to_radians().tan().asinh() / std::f64::consts::PI) / 2.0 * n) as u32;
        let tile = p((z, x, y));
        assert!(tile.is_some_and(|t| t.chunks(4).any(|px| px[3] > 0)));
    }

    #[test]
    fn painter_skips_far_tiles() {
        let v = Viewshed { lat: 59.0, lon: 30.0, radius_m: 1000.0, step: 2.0, rays: 4, steps: 2, visible: vec![true; 8] };
        let p = painter(&Layer::Viewshed(Arc::new(v)));
        // Тайл уровня 2 у Австралии далеко от круга.
        assert!(p((2, 3, 2)).is_none());
    }
}
