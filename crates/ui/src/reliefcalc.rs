// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Уклон и экспозиция» - пункт подменю «Анализ рельефа» меню
//! на глобусе, перенос `ui/relief.py` модуля. Расчёт - `data::relief`
//! по высотам квадрата ([`crate::gridheights`]).
//!
//! Квадрат на глобусе - слой над картами с раскраской строк «Уклон»
//! и «Экспозиция»: строки на это время снимаются, как у модуля.
//! Вершины и ямы ложатся папкой точек в «Мои метки». Растров в проект
//! QGIS у Lite нет.

use std::sync::Arc;

use data::relief::{self, Mode, Relief};
use eframe::egui;
use globe_core::lod::TileKey;

use crate::gridheights::{GridHeights, Step};
use crate::i18n::{Lang, tr, trf};

/// Цвета вершин и ям в «Моих метках», `PEAK_COLOR` и `PIT_COLOR` модуля.
const PEAK_COLOR: [u8; 4] = [200, 40, 40, 255];
const PIT_COLOR: [u8; 4] = [40, 90, 220, 255];

/// Окно «Уклон и экспозиция» и его квадрат на глобусе.
pub struct ReliefCalc {
    pub open: bool,
    point: (f64, f64),
    place: String,
    radius_km: f64,
    mode: Mode,
    peak_radius: f64,
    peak_drop: f64,
    status: String,
    job: Option<GridHeights>,
    /// Расчёт и высоты его сетки.
    state: Option<(Arc<Relief>, Arc<Vec<f64>>)>,
    /// Радиус тела расчёта, метры.
    radius: f64,
    /// Слой сменился: картинки тайлов рисуются заново.
    pub changed: bool,
    /// Вершины и ямы в KML для «Моих меток», забирает окно глобуса.
    pub kml: Option<String>,
    /// Квадрат построен: строки «Уклон» и «Экспозиция» снимаются.
    pub built: bool,
}

/// Число для текста окна: целое у больших, десятые у малых.
pub fn number(value: f64) -> String {
    if !value.is_finite() {
        return "-".into();
    }
    if value.abs() >= 100.0 { format!("{value:.0}") } else if value.abs() >= 10.0 { format!("{value:.1}") } else { format!("{value:.2}") }
}

/// Сторона света по `relief::SIDES`.
fn side_name(lang: Lang, side: &str) -> &'static str {
    tr(
        lang,
        match side {
            "N" => "north",
            "NE" => "north-east",
            "E" => "east",
            "SE" => "south-east",
            "S" => "south",
            "SW" => "south-west",
            "W" => "west",
            _ => "north-west",
        },
    )
}

/// Сводка уклона и экспозиции строками окна, `summary_text` модуля.
fn summary_text(lang: Lang, rep: &relief::Summary) -> String {
    let mut lines = vec![trf(lang, "Slope from {low}° to {high}°, median {median}°.", &[("low", &number(rep.low)), ("high", &number(rep.high)), ("median", &number(rep.median))])];
    let mut bounds: Vec<f64> = rep.classes.iter().map(|c| c.0).collect();
    bounds.push(90.0);
    let parts: Vec<String> = rep
        .classes
        .iter()
        .enumerate()
        .filter(|(_, c)| c.1 > 0.0)
        .map(|(i, c)| trf(lang, "{low}-{high}° - {area} km²", &[("low", &format!("{}", c.0)), ("high", &format!("{}", bounds.get(i + 1).copied().unwrap_or(90.0))), ("area", &number(c.1 / 1e6))]))
        .collect();
    lines.push(trf(lang, "Areas by slope: {parts}.", &[("parts", &parts.join(", "))]));
    let mut order: Vec<usize> = (0..relief::SIDES.len()).collect();
    order.sort_by(|a, b| rep.sides.get(*b).unwrap_or(&0.0).total_cmp(rep.sides.get(*a).unwrap_or(&0.0)));
    let sides: Vec<String> = order
        .iter()
        .take(3)
        .filter(|i| rep.sides.get(**i).is_some_and(|s| *s > 0.0))
        .map(|i| trf(lang, "{side} {share}", &[("side", side_name(lang, relief::SIDES[*i])), ("share", &format!("{:.0}%", 100.0 * rep.sides[*i]))]))
        .collect();
    if !sides.is_empty() {
        lines.push(trf(lang, "Slopes face most often {sides}.", &[("sides", &sides.join(", "))]));
    }
    if rep.flat >= 0.005 {
        lines.push(trf(lang, "Flat ground {share}.", &[("share", &format!("{:.0}%", 100.0 * rep.flat))]));
    }
    lines.join(" ")
}

/// Текст KML без разметки.
pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Цвет KML aabbggrr.
pub fn kml_color(rgba: [u8; 4]) -> String {
    format!("{:02x}{:02x}{:02x}{:02x}", rgba[3], rgba[2], rgba[1], rgba[0])
}

impl ReliefCalc {
    pub fn new(lat: f64, lon: f64, place: String) -> Self {
        Self {
            open: true,
            point: (lat, lon),
            place,
            radius_km: 2.0,
            mode: Mode::Slope,
            peak_radius: relief::PEAK_RADIUS,
            peak_drop: relief::PEAK_DROP,
            status: String::new(),
            job: None,
            state: None,
            changed: false,
            kml: None,
            built: false,
            radius: viewshed_radius(),
        }
    }

    /// Новая точка в открытом окне, квадрат остаётся до «Построить».
    pub fn set_point(&mut self, lat: f64, lon: f64, place: String) {
        self.point = (lat, lon);
        self.place = place;
        self.status.clear();
        self.open = true;
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Квадрат на глобусе.
    pub fn layer(&self) -> Option<(Arc<Relief>, f64)> {
        self.state.as_ref().map(|s| (s.0.clone(), self.radius))
    }

    /// Убрать квадрат, `clear` модуля.
    pub fn clear(&mut self) {
        self.job = None;
        if self.state.take().is_some() {
            self.changed = true;
        }
        self.status.clear();
    }

    /// Тайлы высот, которых ждёт расчёт.
    pub fn wanted(&self, has: &dyn Fn(TileKey) -> bool) -> Vec<TileKey> {
        self.job.as_ref().map_or(Vec::new(), |j| j.wanted(has))
    }

    /// Шаг расчёта.
    pub fn step(&mut self, lang: Lang, now: f64, has: &dyn Fn(TileKey) -> bool, heights: data::viewshed::Heights) {
        let Some(job) = &mut self.job else {
            return;
        };
        match job.step(lang, now, has, heights) {
            Step::Going(text) => self.status = text,
            Step::Failed(text) => {
                self.status = text;
                self.job = None;
            }
            Step::Done(z) => {
                let (grid, radius, partial) = (job.grid, job.radius, job.partial);
                self.radius = radius;
                self.job = None;
                let result = relief::analyse(&z, &grid, radius, self.mode);
                let Some(rep) = relief::summary(&result) else {
                    self.status = tr(lang, "There are no heights in the square.").to_string();
                    return;
                };
                self.status = format!("{} {}", summary_text(lang, &rep), trf(lang, "The grid step is {step} m.", &[("step", &number(grid.cell))]));
                if partial {
                    self.status = format!("{} {}", self.status, tr(lang, "Not all heights are loaded, the computation used less detailed ones."));
                }
                self.state = Some((Arc::new(result), Arc::new(z)));
                self.changed = true;
                self.built = true;
            }
        }
    }

    /// Вершины и ямы квадрата папкой KML, `find_peaks` модуля.
    fn find_peaks(&mut self, lang: Lang) {
        let Some((result, z)) = &self.state else {
            return;
        };
        let grid = &result.grid;
        let found = relief::extremes(z, grid, self.peak_radius, self.peak_drop);
        if found.is_empty() {
            self.status = tr(lang, "There are no peaks and pits with such a drop in the square.").to_string();
            return;
        }
        let (mut peaks, mut pits) = (0, 0);
        let mut body = String::new();
        for (row, col, height, drop, kind) in found {
            let (lat, lon) = crate::gridheights::node(grid, row as f64, col as f64, self.radius);
            let (name, text, color) = match kind {
                data::topo_surface::Extreme::Peak => {
                    peaks += 1;
                    (trf(lang, "Peak {height} m", &[("height", &format!("{height:.0}"))]), trf(lang, "Height {height} m, rise {drop} m.", &[("height", &format!("{height:.0}")), ("drop", &number(drop))]), PEAK_COLOR)
                }
                data::topo_surface::Extreme::Pit => {
                    pits += 1;
                    (trf(lang, "Pit {height} m", &[("height", &format!("{height:.0}"))]), trf(lang, "Height {height} m, depth {drop} m.", &[("height", &format!("{height:.0}")), ("drop", &number(drop))]), PIT_COLOR)
                }
            };
            body.push_str(&format!(
                "<Placemark><name>{}</name><description>{}</description><Style><IconStyle><color>{}</color></IconStyle></Style><Point><coordinates>{lon:.7},{lat:.7},0</coordinates></Point></Placemark>",
                escape(&name),
                escape(&text),
                kml_color(color)
            ));
        }
        let title = trf(lang, "Peaks and pits {point}", &[("point", &format!("{:.4}, {:.4}", grid.lat, grid.lon))]);
        self.kml = Some(format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><kml xmlns=\"http://www.opengis.net/kml/2.2\"><Folder><name>{}</name>{body}</Folder></kml>", escape(&title)));
        self.status = trf(lang, "Written to «My places»: peaks {peaks} and pits {pits}.", &[("peaks", &peaks.to_string()), ("pits", &pits.to_string())]);
    }

    /// Окно. radius - радиус тела, max_level - уровень тайлов высот.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang, now: f64, radius: f64, max_level: u32) {
        if !self.open {
            return;
        }
        let mut open = true;
        let (mut build, mut clear, mut peaks) = (false, false, false);
        let before = self.mode;
        egui::Window::new(tr(lang, "Slope and aspect")).id(egui::Id::unique("planetx-relief")).open(&mut open).collapsible(false).resizable(false).min_width(380.0).show(ctx, |ui| {
            egui::Grid::new("planetx-relief-form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Point"));
                ui.label(&self.place);
                ui.end_row();
                ui.label(tr(lang, "Radius"));
                ui.add(egui::DragValue::new(&mut self.radius_km).range(0.2..=50.0).speed(0.1).fixed_decimals(1).suffix(tr(lang, " km")))
                    .on_hover_text(tr(lang, "Half the side of the computed square. Larger - a coarser grid step, it is about one six-hundredth of the side."));
                ui.end_row();
                ui.label(tr(lang, "Show"));
                egui::ComboBox::from_id_salt("planetx-relief-mode").selected_text(tr(lang, if self.mode == Mode::Slope { "Slope" } else { "Aspect" })).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.mode, Mode::Slope, tr(lang, "Slope"));
                    ui.selectable_value(&mut self.mode, Mode::Aspect, tr(lang, "Aspect"));
                })
                .response
                .on_hover_text(tr(lang, "What the square shows on the globe - the steepness of a slope or the side of the world it faces. The rows «Slope» and «Aspect» of the section «Layers» go off meanwhile."));
                ui.end_row();
                ui.label(tr(lang, "Peak radius"));
                ui.add(egui::DragValue::new(&mut self.peak_radius).range(20.0..=5000.0).speed(5.0).fixed_decimals(0).suffix(tr(lang, " m")))
                    .on_hover_text(tr(lang, "A peak is higher than everything around it within this radius, a pit is lower. Larger - only large peaks and pits remain."));
                ui.end_row();
                ui.label(tr(lang, "Drop from"));
                ui.add(egui::DragValue::new(&mut self.peak_drop).range(0.5..=2000.0).speed(0.5).fixed_decimals(1).suffix(tr(lang, " m")))
                    .on_hover_text(tr(lang, "The least rise of a peak over the lowest point around it or depth of a pit under the highest one. Smaller - more small hills and hollows."));
                ui.end_row();
            });
            ui.add_space(4.0);
            ui.add(egui::Label::new(&self.status).wrap());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button(tr(lang, "Build")).on_hover_text(tr(lang, "Slope and aspect of the square by the heights, a 3 × 3 kernel - a colouring on the globe and a summary by slope classes and sides of the world.")).clicked() {
                    build = true;
                }
                if ui.add_enabled(self.state.is_some(), egui::Button::new(tr(lang, "Peaks and pits"))).on_hover_text(tr(lang, "Peaks and closed hollows of the square - points in «My places» with the height and the drop.")).clicked() {
                    peaks = true;
                }
                if ui.add_enabled(self.state.is_some() || self.job.is_some(), egui::Button::new(tr(lang, "Remove"))).on_hover_text(tr(lang, "Remove the colouring of the square from the globe.")).clicked() {
                    clear = true;
                }
            });
        });
        if build {
            let (lat, lon) = self.point;
            self.job = Some(GridHeights::new(lat, lon, self.radius_km * 1000.0, radius, max_level, now));
        }
        if peaks {
            self.find_peaks(lang);
        }
        if clear {
            self.clear();
        }
        // Показ уклона или экспозиции без пересчёта.
        if self.mode != before
            && let Some((r, z)) = self.state.take()
        {
            let mut r = (*r).clone();
            r.mode = self.mode;
            self.state = Some((Arc::new(r), z));
            self.changed = true;
        }
        if !open {
            self.open = false;
        }
        if self.job.is_some() {
            ctx.request_repaint();
        }
    }
}

/// Картинки тайлов квадрата: раскраска уклона или экспозиции.
pub fn painter(result: Arc<Relief>, radius: f64) -> crate::vector::Painter {
    Arc::new(move |key: TileKey| data::relief::tile_rgba(&result, key, radius))
}

/// Радиус Земли по умолчанию.
fn viewshed_radius() -> f64 {
    data::viewshed::EARTH_RADIUS
}
