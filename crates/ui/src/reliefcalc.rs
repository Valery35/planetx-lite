// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Уклон и экспозиция» - пункт подменю «Анализ участка» меню
//! на глобусе, перенос `ui/relief.py` модуля. Расчёт - `data::relief`
//! по высотам рамки участка ([`crate::gridheights`]), вне многоугольника
//! высот нет.
//!
//! Окно - на общем каркасе анализа участка ([`crate::analysis`]):
//! участок многоугольником, «Построить», «OK» и «Отмена». Участок на
//! глобусе - слой над картами с раскраской строк «Уклон» и «Экспозиция»:
//! строки на это время снимаются, как у модуля. «OK» кладёт в папку
//! расчёта картинку показанного слоя, вершины и ямы, если их искали,
//! и пишет растры уклона и экспозиции GeoTIFF EPSG:3857, как модуль.

use std::sync::Arc;

use data::relief::{self, Mode, Relief};
use eframe::egui;
use globe_core::lod::TileKey;
use scene::features::Kind;
use scene::kml::{Item, Place};

use crate::analysis::{self, Button, ResultLayer, Saved, SiteEdit};
use crate::gridheights::{GridHeights, Step};
use crate::i18n::{Lang, tr, trf};

/// Цвета вершин и ям в «Моих метках», `PEAK_COLOR` и `PIT_COLOR` модуля.
const PEAK_COLOR: [u8; 4] = [200, 40, 40, 255];
const PIT_COLOR: [u8; 4] = [40, 90, 220, 255];
/// Участок по умолчанию и наибольший, километры, `SIDE` и `MOST` модуля.
const SIDE: f64 = 4.0;
const MOST: f64 = 100.0;

/// Расчёт, высоты его сетки и участок.
type State = (Arc<Relief>, Arc<Vec<f64>>, Vec<(f64, f64)>);

/// Окно «Уклон и экспозиция» и его участок на глобусе.
pub struct ReliefCalc {
    pub open: bool,
    place: String,
    mode: Mode,
    peak_radius: f64,
    peak_drop: f64,
    status: String,
    job: Option<(GridHeights, Vec<(f64, f64)>)>,
    /// Расчёт, высоты его сетки и участок.
    state: Option<State>,
    /// Радиус тела расчёта, метры.
    radius: f64,
    /// Слой сменился: картинки тайлов рисуются заново.
    pub changed: bool,
    /// Участок построен: строки «Уклон» и «Экспозиция» снимаются.
    pub built: bool,
    /// Участок окна на глобусе.
    pub site: SiteEdit,
    /// Найденные вершины и ямы - в папку расчёта по «OK».
    peaks: Vec<Item>,
    /// Расчёт по «OK» для «Моих меток», забирает окно глобуса.
    pub saved: Option<Saved>,
}

/// Число для текста окна: целое у больших, десятые у малых.
pub fn number(value: f64) -> String {
    if value.is_finite() { data::stats::rounded(value) } else { "-".into() }
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
            place,
            mode: Mode::Slope,
            peak_radius: relief::PEAK_RADIUS,
            peak_drop: relief::PEAK_DROP,
            status: String::new(),
            job: None,
            state: None,
            changed: false,
            built: false,
            radius: viewshed_radius(),
            site: SiteEdit::new(lat, lon, SIDE, MOST),
            peaks: Vec::new(),
            saved: None,
        }
    }

    /// Новая точка в открытом окне: участок - квадрат у неё, слой
    /// остаётся до «Построить».
    pub fn set_point(&mut self, lat: f64, lon: f64, place: String) {
        self.place = place;
        self.status.clear();
        self.open = true;
        self.site.move_to(lat, lon);
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Участок окна на глобусе, пока окно открыто.
    pub fn site_edit(&mut self) -> Option<&mut SiteEdit> {
        if self.open { Some(&mut self.site) } else { None }
    }

    /// Участок на глобусе.
    pub fn layer(&self) -> Option<(Arc<Relief>, f64)> {
        self.state.as_ref().map(|s| (s.0.clone(), self.radius))
    }

    /// Убрать участок, `clear` модуля.
    pub fn clear(&mut self) {
        self.job = None;
        if self.state.take().is_some() {
            self.changed = true;
        }
        self.peaks.clear();
        self.status.clear();
    }

    /// Тайлы высот, которых ждёт расчёт.
    pub fn wanted(&self, has: &dyn Fn(TileKey) -> bool) -> Vec<TileKey> {
        self.job.as_ref().map_or(Vec::new(), |j| j.0.wanted(has))
    }

    /// Шаг расчёта.
    pub fn step(&mut self, lang: Lang, now: f64, has: &dyn Fn(TileKey) -> bool, heights: data::viewshed::Heights) {
        let Some((job, _)) = &mut self.job else {
            return;
        };
        match job.step(lang, now, has, heights) {
            Step::Going(text) => self.status = text,
            Step::Failed(text) => {
                self.status = text;
                self.job = None;
            }
            Step::Done(mut z) => {
                let (grid, radius, partial) = (job.grid, job.radius, job.partial);
                let ring = self.job.take().map(|j| j.1).unwrap_or_default();
                self.radius = radius;
                data::site::mask_grid(&mut z, &grid, &ring, radius);
                let result = relief::analyse(&z, &grid, radius, self.mode);
                let Some(rep) = relief::summary(&result) else {
                    self.status = tr(lang, "There are no heights in the area.").to_string();
                    return;
                };
                let values = analysis::stats_text(lang, tr(lang, "Slope"), &result.slope, "°");
                self.status = format!("{} {values} {}", summary_text(lang, &rep), trf(lang, "The grid step is {step} m.", &[("step", &number(grid.cell))]));
                if partial {
                    self.status = format!("{} {}", self.status, tr(lang, "Not all heights are loaded, the computation used less detailed ones."));
                }
                self.status = format!("{} {}", self.status, analysis::built_text(lang, crate::pointcalc::clock_now()));
                self.state = Some((Arc::new(result), Arc::new(z), ring));
                self.peaks.clear();
                self.changed = true;
                self.built = true;
            }
        }
    }

    /// Вершины и ямы участка точками для папки расчёта, `find_peaks`
    /// модуля.
    fn find_peaks(&mut self, lang: Lang) {
        let Some((result, z, _)) = &self.state else {
            return;
        };
        let grid = &result.grid;
        let found = relief::extremes(z, grid, self.peak_radius, self.peak_drop);
        if found.is_empty() {
            self.status = tr(lang, "There are no peaks and pits with such a drop in the area.").to_string();
            return;
        }
        let (mut peaks, mut pits) = (0, 0);
        self.peaks.clear();
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
            let mut place = Place::new(&name, Kind::Point, vec![(lat, lon)]);
            place.color = color;
            place.description = text;
            self.peaks.push(Item::Place(place));
        }
        self.status = trf(lang, "Found peaks {peaks} and pits {pits}. They go into the folder of the computation by «OK».", &[("peaks", &peaks.to_string()), ("pits", &pits.to_string())]);
    }

    /// Готовый участок - расчётом для «Моих меток», `export` модуля:
    /// растры уклона и экспозиции GeoTIFF EPSG:3857, на глобусе - то, что
    /// показывал участок.
    fn save(&mut self, lang: Lang) {
        let Some((result, _, ring)) = &self.state else {
            return;
        };
        let radius = self.radius;
        let mercator = data::site::mercator_of(&result.grid);
        let slope_tiff = data::geotiff::write_float_mercator(&mercator, &result.slope).unwrap_or_default();
        let aspect_tiff = data::geotiff::write_float_mercator(&mercator, &result.aspect).unwrap_or_default();
        let (south, north, west, east) = result.bbox;
        let shown = result.clone();
        let sample = move |points: &[(f64, f64)]| relief::points_rgba(&shown, points, radius).as_chunks::<4>().0.to_vec();
        let (name, stem) = match result.mode {
            Mode::Slope => (tr(lang, "Slope, °"), "slope"),
            Mode::Aspect => (tr(lang, "Aspect, °"), "aspect"),
        };
        let mut shown_layer = analysis::picture(name.to_string(), stem, (south, north, west, east), result.grid.cell, &sample, Vec::new());
        // Файлы обоих растров, картинка - только показанного, как у модуля.
        let (first, second) = match result.mode {
            Mode::Slope => ((slope_tiff, "slope"), (aspect_tiff, "aspect")),
            Mode::Aspect => ((aspect_tiff, "aspect"), (slope_tiff, "slope")),
        };
        shown_layer.tiff = first.0;
        shown_layer.summary = analysis::stats_text(lang, tr(lang, "Slope"), &result.slope, "°");
        let other = ResultLayer { name: String::new(), grid: shown_layer.grid, rgba: Vec::new(), tiff: second.0, stem: second.1.to_string(), summary: String::new() };
        self.saved = Some(Saved { title: tr(lang, "Slope and aspect").to_string(), ring: ring.clone(), layers: vec![shown_layer, other], extra: std::mem::take(&mut self.peaks), session: None });
    }

    /// Окно. radius - радиус тела, max_level - уровень тайлов высот.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang, now: f64, radius: f64, max_level: u32) {
        if !self.open {
            return;
        }
        if let Some(text) = self.site.shared_note(lang) {
            self.status = text.to_string();
        }
        let mut open = true;
        let mut pressed = None;
        let mut peaks = false;
        let before = self.mode;
        egui::Window::new(tr(lang, "Slope and aspect")).id(egui::Id::unique("planetx-relief")).open(&mut open).collapsible(false).resizable(false).min_width(380.0).show(ctx, |ui| {
            egui::Grid::new("planetx-relief-form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Point"));
                ui.label(&self.place);
                ui.end_row();
                ui.label(tr(lang, "Area|site"));
                self.site.row(ui, lang);
                ui.end_row();
                ui.label(tr(lang, "Show"));
                egui::ComboBox::from_id_salt("planetx-relief-mode").selected_text(tr(lang, if self.mode == Mode::Slope { "Slope" } else { "Aspect" })).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.mode, Mode::Slope, tr(lang, "Slope"));
                    ui.selectable_value(&mut self.mode, Mode::Aspect, tr(lang, "Aspect"));
                })
                .response
                .on_hover_text(tr(lang, "What the area shows on the globe - the steepness of a slope or the side of the world it faces. The rows «Slope» and «Aspect» of the section «Layers» go off meanwhile."));
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
            ui.add(egui::Label::new(self.site.hint(lang).unwrap_or(&self.status)).wrap());
            ui.add_space(4.0);
            if ui
                .add_enabled(self.state.is_some(), egui::Button::new(tr(lang, "Peaks and pits")))
                .on_hover_text(tr(lang, "Peaks and closed hollows of the area - points with the height and the drop in the folder of the computation."))
                .clicked()
            {
                peaks = true;
            }
            pressed = analysis::buttons(
                ui,
                lang,
                self.state.is_some() && self.job.is_none(),
                tr(lang, "Slope and aspect of the area by the heights, a 3 × 3 kernel - a colouring on the globe and a summary by slope classes and sides of the world."),
            );
        });
        match pressed {
            Some(Button::Build) => {
                if let Some(problem) = self.site.problem(lang) {
                    self.status = problem;
                } else {
                    let (lat, lon) = self.site.site.centre();
                    let half = self.site.site.size() / 2.0;
                    self.job = Some((GridHeights::new(lat, lon, half, radius, max_level, now), self.site.site.ring.clone()));
                }
            }
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
        if peaks {
            self.find_peaks(lang);
        }
        // Крестик - как «Отмена».
        if !open {
            self.clear();
            self.open = false;
        }
        // Показ уклона или экспозиции без пересчёта.
        if self.mode != before
            && let Some((r, z, ring)) = self.state.take()
        {
            let mut r = (*r).clone();
            r.mode = self.mode;
            self.state = Some((Arc::new(r), z, ring));
            self.changed = true;
        }
        if std::mem::take(&mut self.site.changed) && self.state.is_some() && self.job.is_none() {
            self.status = tr(lang, "The area has changed. Press «Build» to compute it again.").to_string();
        }
        if self.job.is_some() {
            ctx.request_repaint();
        }
    }
}

/// Картинки тайлов участка: раскраска уклона или экспозиции.
pub fn painter(result: Arc<Relief>, radius: f64) -> crate::vector::Painter {
    Arc::new(move |key: TileKey| data::relief::tile_rgba(&result, key, radius))
}

/// Радиус Земли по умолчанию.
fn viewshed_radius() -> f64 {
    data::viewshed::EARTH_RADIUS
}
