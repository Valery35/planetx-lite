// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Расчёты у точки: «Видимость отсюда…» и «Инсоляция здесь…» меню на
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

use std::sync::Arc;

use eframe::egui;
use globe_core::lod::TileKey;

use data::insolation::{self, Insolation};
use data::viewshed::{self, Viewshed};

use crate::i18n::{Lang, tr, trf};

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
}

/// Готовый слой.
#[derive(Clone)]
pub enum Layer {
    Viewshed(Arc<Viewshed>),
    Insolation(Arc<Insolation>),
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
}

impl Calc {
    pub fn new(kind: Kind, lat: f64, lon: f64, place: String, now_unix: f64) -> Self {
        let today = civil_from_days((now_unix / insolation::DAY).floor() as i64);
        let radius_km = if kind == Kind::Viewshed { 10.0 } else { 5.0 };
        Self { kind, open: true, point: (lat, lon), place, observer: 2.0, target: 0.0, radius_km, first: today, last: today, status: String::new(), job: None, layer: None, changed: false }
    }

    /// Новая точка в открытом окне, слой остаётся до «Построить».
    pub fn set_point(&mut self, lat: f64, lon: f64, place: String) {
        self.point = (lat, lon);
        self.place = place;
        self.status.clear();
        self.open = true;
    }

    /// Идёт расчёт.
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn build(&mut self, now: f64) {
        let (lat, lon) = self.point;
        let radius_m = self.radius_km * 1000.0;
        let (keys, a, b) = match self.kind {
            Kind::Viewshed => {
                let cell = radius_m / viewshed::STEPS as f64;
                (viewshed::height_tiles(lat, lon, radius_m, cell, viewshed::EARTH_RADIUS, terrain::MAX_LEVEL).1, self.observer, self.target)
            }
            Kind::Insolation => {
                let (cell, margin, _) = insolation::layout(radius_m);
                let keys = viewshed::height_tiles(lat, lon, margin as f64 * cell + radius_m, cell, viewshed::EARTH_RADIUS, terrain::MAX_LEVEL).1;
                (keys, day_start(self.first), day_start(self.last))
            }
        };
        self.job = Some(Job { lat, lon, radius_m, keys, started: now, missing: false, stage: Stage::Heights, a, b });
    }

    /// Убрать слой и расчёт.
    fn clear(&mut self) {
        self.job = None;
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
                                self.layer = Some(Layer::Viewshed(Arc::new(r)));
                            }
                            None => {
                                self.status = tr(lang, "This body has no heights.").to_string();
                                self.layer = None;
                            }
                        }
                        self.changed = true;
                    }
                    Kind::Insolation => {
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
                match insolation::Work::new(job.lat, job.lon, job.radius_m, got, job.a, job.b, insolation::EARTH_RADIUS) {
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
                    self.status = trf(lang, "Computing insolation: {share}", &[("share", &format!("{:.0}%", 100.0 * share))]);
                    return Vec::new();
                }
                let missing = job.missing;
                let result = self.job.take().and_then(|j| match j.stage {
                    Stage::Work(w) => w.into_result(),
                    _ => None,
                });
                if let Some(r) = result {
                    let low = r.hours.iter().copied().fold(f64::INFINITY, f64::min);
                    let high = r.hours.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    self.status = trf(
                        lang,
                        "Over {days} days direct sun shines from {low} to {high} h per day. The grid step is {step} m.",
                        &[("days", &r.days.to_string()), ("low", &format!("{low:.1}")), ("high", &format!("{high:.1}")), ("step", &format!("{:.0}", r.cell))],
                    );
                    if missing {
                        self.status = format!("{} {}", self.status, tr(lang, "Not all heights are loaded, the computation used less detailed ones."));
                    }
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
        let mut open = true;
        let mut build = false;
        let mut clear = false;
        let title = match self.kind {
            Kind::Viewshed => tr(lang, "Viewshed from a point"),
            Kind::Insolation => tr(lang, "Insolation"),
        };
        egui::Window::new(title).id(egui::Id::unique(("planetx-calc", self.kind == Kind::Viewshed))).open(&mut open).collapsible(false).resizable(false).min_width(360.0).show(ctx, |ui| {
            egui::Grid::new(("planetx-calc-form", self.kind == Kind::Viewshed)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
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
                        ui.label(tr(lang, "Radius"));
                        ui.add(egui::DragValue::new(&mut self.radius_km).range(0.2..=30.0).speed(0.1).fixed_decimals(1).suffix(tr(lang, " km")))
                            .on_hover_text(tr(lang, "Radius of the computed circle. A larger radius gives a coarser grid. Mountain shadows are taken from a distance of at least the radius and at least 5 km."));
                        ui.end_row();
                    }
                }
            });
            ui.add_space(4.0);
            ui.add(egui::Label::new(&self.status).wrap());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button(tr(lang, "Build")).clicked() {
                    build = true;
                }
                let tip = match self.kind {
                    Kind::Viewshed => tr(lang, "Remove the viewshed layer from the globe."),
                    Kind::Insolation => tr(lang, "Remove the insolation layer from the globe."),
                };
                if ui.add_enabled(self.layer.is_some() || self.job.is_some(), egui::Button::new(tr(lang, "Remove"))).on_hover_text(tip).clicked() {
                    clear = true;
                }
            });
        });
        if build {
            self.build(now);
        }
        if clear {
            self.clear();
        }
        if !open {
            self.open = false;
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

/// Широты строк и долготы столбцов центров пикселей тайла.
fn tile_axes(key: TileKey) -> (Vec<f64>, Vec<f64>) {
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
    fn painter_skips_far_tiles() {
        let v = Viewshed { lat: 59.0, lon: 30.0, radius_m: 1000.0, step: 2.0, rays: 4, steps: 2, visible: vec![true; 8] };
        let p = painter(&Layer::Viewshed(Arc::new(v)));
        // Тайл уровня 2 у Австралии далеко от круга.
        assert!(p((2, 3, 2)).is_none());
    }
}
