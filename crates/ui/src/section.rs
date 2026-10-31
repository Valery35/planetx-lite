// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Разрез»: разрез Земли вниз вдоль пути, перенос `ui/section.py`
//! модуля (`SectionChart`, `SectionDialog`) и `_open_section`, `_section`
//! из `ui/globe/tools.py`.
//!
//! График - расстояние вдоль линии и глубина вниз: слои коры CRUST1.0,
//! оболочки PREM, плиты Slab2, очаги землетрясений из полосы вдоль
//! линии. Цвета - те же, что на гранях разреза Земли и у очагов. Расчёт -
//! [`data::section`]. Кора, зоны плит и сводка землетрясений USGS
//! просятся, если их нет. Исторические очаги ISC-GEM входят в разрез,
//! пока включена их строка, очаги берутся в окне шкалы времени, как на
//! глобусе. Стенка разреза стоит на глобусе и видна сквозь поверхность.
//! Курсор над графиком отмечает точку на глобусе. Справа от графика -
//! легенда слоёв, как `CutawayLegend` модуля.

use std::collections::{BTreeMap, HashMap};

use data::crust::{self, Crust};
use data::cutaway;
use data::quakes::{self, Quake};
use data::section::{self, Section};
use data::slabs::{self, Frame, Slab};
use eframe::egui;

use crate::cutaway::{Ask, CRUST_BYTES, INDEX_BYTES, SLAB_BYTES, SLAB_INDEX};
use crate::i18n::{Lang, tr, trf};

/// Предел ответа сводки землетрясений.
const QUAKE_BYTES: u64 = 16 << 20;
/// Очаг на графике меньше, чем на глобусе.
const QUAKE_SHRINK: f32 = 0.6;
/// Обводка исторических очагов ISC-GEM, свежие - тёмной. Выбор помощника
/// модуля.
const HISTORY_OUTLINE: egui::Color32 = egui::Color32::from_rgb(235, 235, 235);
const FRESH_OUTLINE: egui::Color32 = egui::Color32::from_rgb(40, 40, 40);
const HOVER: egui::Color32 = egui::Color32::from_rgb(220, 60, 40);
/// Высота строк итогов под графиком, логические пиксели.
const STATS_HEIGHT: f32 = 44.0;
/// Наименьшая высота графика.
const MIN_CHART: f32 = 200.0;
/// Ширина легенды справа от графика.
const LEGEND_WIDTH: f32 = 230.0;

/// Что перестраивает разрез: глубина, полоса, окно времени, кора, зоны
/// плит, количество свежих и исторических очагов.
type Key = (u64, u64, Option<(u64, u64)>, bool, usize, usize, usize);

/// Окно «Разрез» и его загрузки.
pub struct SectionWindow {
    pub title: String,
    points: Vec<(f64, f64)>,
    depth: f64,
    width: f64,
    /// Точка разреза под курсором графика: широта, долгота, км вдоль линии.
    pub hovered: Option<(f64, f64, f64)>,
    crust: Ask<Crust>,
    index: Ask<BTreeMap<String, Frame>>,
    zones: HashMap<String, Ask<Slab>>,
    /// Сводка USGS, если строки землетрясений на глобусе нет.
    recent: Ask<Vec<Quake>>,
    built: Option<Section>,
    key: Option<Key>,
    /// Что ещё загружается: кора, плиты, землетрясения.
    waiting: [bool; 3],
}

fn km_text(lang: Lang, value: f64) -> String {
    let value = (value * 10.0).round() / 10.0;
    trf(lang, "{value} km", &[("value", &format!("{value}"))]).replace(' ', "\u{a0}")
}

fn rgb(c: [u8; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(c[0], c[1], c[2])
}

impl SectionWindow {
    /// Окно для пути points. crust - архив CRUST1.0 из кэша программы.
    pub fn new(title: String, points: Vec<(f64, f64)>, crust: Option<&[u8]>) -> Self {
        let crust = match crust.map(crust::from_archive) {
            Some(Ok(c)) => Ask::Ready(c),
            // CRUST1.0 у страницы нет: кора - слоем PREM.
            _ if cfg!(target_arch = "wasm32") => Ask::Failed(crate::layers::DESKTOP_ONLY.to_string()),
            _ => Ask::Idle,
        };
        Self { title, points, depth: section::DEPTH, width: section::WIDTH, hovered: None, crust, index: Ask::Idle, zones: HashMap::new(), recent: Ask::Idle, built: None, key: None, waiting: [false; 3] }
    }

    /// Другой путь в открытом окне.
    pub fn set_source(&mut self, title: String, points: Vec<(f64, f64)>) {
        self.title = title;
        self.points = points;
        self.key = None;
        self.hovered = None;
    }

    pub fn busy(&self) -> bool {
        self.waiting.iter().any(|w| *w)
    }

    /// Шаг кадра: загрузки и сборка разреза. recent - очаги строки
    /// землетрясений, history - очаги ISC-GEM включённой строки, span -
    /// окно шкалы времени. Возвращает новую стенку для глобуса (Some(None) -
    /// убрать) и байты пришедшего архива коры для кэша.
    pub fn step(&mut self, request: &dyn Fn(u64) -> Box<dyn net::Request>, recent: Option<&[Quake]>, history: Option<&[Quake]>, span: Option<(f64, f64)>) -> (Option<Option<render::SubMesh>>, Option<Vec<u8>>) {
        let mut saved = None;
        if matches!(self.crust, Ask::Idle) {
            let ask = request(CRUST_BYTES);
            ask.start(crust::URL);
            self.crust = Ask::Asking(ask);
        }
        if let Ask::Asking(ask) = &self.crust
            && let Some(answer) = ask.take()
        {
            self.crust = match answer.map_err(|e| e.to_string()).and_then(|b| crust::from_archive(&b).map(|c| (c, b)).map_err(|e| e.to_string())) {
                Ok((c, bytes)) => {
                    saved = Some(bytes);
                    Ask::Ready(c)
                }
                Err(e) => Ask::Failed(e),
            };
        }
        if matches!(self.index, Ask::Idle) {
            let ask = request(INDEX_BYTES);
            ask.start(SLAB_INDEX);
            self.index = Ask::Asking(ask);
        }
        if let Ask::Asking(ask) = &self.index
            && let Some(answer) = ask.take()
        {
            self.index = match answer.and_then(|b| slabs::read_index(&b).map_err(|e| e.0)) {
                Ok(i) => Ask::Ready(i),
                Err(e) => Ask::Failed(e),
            };
        }
        for (code, zone) in self.zones.iter_mut() {
            if let Ask::Asking(ask) = zone
                && let Some(answer) = ask.take()
            {
                *zone = match answer {
                    Ok(b) => match slabs::load(&b, code) {
                        Ok(s) => Ask::Ready(s),
                        Err(e) => Ask::Failed(e.0),
                    },
                    Err(e) => Ask::Failed(e),
                };
            }
        }
        // Сводка USGS - только если строки землетрясений на глобусе нет.
        if recent.is_none() && matches!(self.recent, Ask::Idle) {
            let ask = request(QUAKE_BYTES);
            ask.start(quakes::FEED);
            self.recent = Ask::Asking(ask);
        }
        if let Ask::Asking(ask) = &self.recent
            && let Some(answer) = ask.take()
        {
            self.recent = match answer {
                Ok(b) => Ask::Ready(quakes::parse(&b)),
                Err(e) => Ask::Failed(e),
            };
        }
        let recent: &[Quake] = match (recent, &self.recent) {
            (Some(q), _) => q,
            (None, Ask::Ready(q)) => q,
            _ => &[],
        };
        let history = history.unwrap_or(&[]);
        // Зоны плит, которые задевает линия.
        let codes = match (&self.index, section::along(&self.points, section::SAMPLES)) {
            (Ask::Ready(index), Some((_, lats, lons))) => slabs::touched(index, &lats, &lons),
            _ => Vec::new(),
        };
        for code in &codes {
            self.zones.entry(code.clone()).or_insert_with(|| {
                let ask = request(SLAB_BYTES);
                ask.start(&slabs::url_of(code, None));
                Ask::Asking(ask)
            });
        }
        let zones: Vec<&Slab> = codes
            .iter()
            .filter_map(|c| match self.zones.get(c) {
                Some(Ask::Ready(s)) => Some(s),
                _ => None,
            })
            .collect();
        self.waiting = [
            matches!(self.crust, Ask::Asking(_)),
            matches!(self.index, Ask::Asking(_)) || codes.iter().any(|c| matches!(self.zones.get(c), Some(Ask::Asking(_)))),
            matches!(self.recent, Ask::Asking(_)) && recent.is_empty(),
        ];
        let crust = match &self.crust {
            Ask::Ready(c) => Some(c),
            _ => None,
        };
        let key = (self.depth.to_bits(), self.width.to_bits(), span.map(|(a, b)| (a.to_bits(), b.to_bits())), crust.is_some(), zones.len(), recent.len(), history.len());
        if self.key == Some(key) {
            return (None, saved);
        }
        self.key = Some(key);
        let recent = section::in_time(recent, span);
        let history = section::in_time(history, span);
        self.built = section::build(&self.points, crust, &zones, &recent, self.depth, self.width, section::SAMPLES, &history);
        let wall = self.built.as_ref().and_then(|s| section::wall_mesh(s, section::WALL_ALPHA));
        let wall = crate::cutaway::to_sub(wall, false).map(|m| render::SubMesh { xray: true, ..m });
        (Some(wall), saved)
    }

    /// Окно. false в open - закрыли.
    pub fn window(&mut self, ctx: &egui::Context, lang: Lang, open: &mut bool) {
        let title = if self.title.is_empty() { tr(lang, "Section").to_string() } else { trf(lang, "Section: {name}", &[("name", &self.title)]) };
        self.hovered = None;
        egui::Window::new(title)
            .id(egui::Id::unique("planetx-section"))
            .open(open)
            .collapsible(false)
            .default_size([760.0, 400.0])
            .resizable(true)
            .min_height(MIN_CHART + STATS_HEIGHT + 50.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr(lang, "Depth"));
                    egui::ComboBox::from_id_salt("section-depth")
                        .selected_text(km_text(lang, self.depth))
                        .show_ui(ui, |ui| {
                            for value in section::DEPTHS {
                                ui.selectable_value(&mut self.depth, value, km_text(lang, value));
                            }
                        })
                        .response
                        .on_hover_text(tr(lang, "How deep the section goes. 700 km is the bottom of the transition zone, there are almost no earthquakes deeper. 2891 km is the core boundary, 6371 km is the centre of the Earth."));
                    ui.add_space(12.0);
                    ui.label(tr(lang, "Foci band"));
                    ui.add(egui::DragValue::new(&mut self.width).range(10.0..=1000.0).speed(1.0).fixed_decimals(0).custom_formatter(|v, _| km_text(lang, v)))
                        .on_hover_text(tr(lang, "Width of the band along the line from which earthquake foci are moved onto the section. A wider band gives more foci, but distant foci do not lie where the line is."));
                });
                let Some(s) = self.built.as_ref() else {
                    ui.label(tr(lang, "A path of at least two points is needed."));
                    return;
                };
                let missing = matches!(self.crust, Ask::Failed(_));
                let hovered = ui
                    .horizontal_top(|ui| {
                        let size = egui::vec2((ui.available_width() - LEGEND_WIDTH - 8.0).max(240.0), ui.available_height());
                        let i = ui.allocate_ui(size, |ui| chart(ui, lang, s)).inner;
                        ui.vertical(|ui| legend(ui, lang, s, missing));
                        i
                    })
                    .inner;
                if let Some(i) = hovered {
                    self.hovered = Some((s.lats[i], s.lons[i], s.distance[i]));
                }
                let length = s.distance.last().copied().unwrap_or(0.0);
                let mut text = trf(
                    lang,
                    "Length {length}, depth {depth}. Earthquake foci in the band {count}.",
                    &[("length", &crate::profile::distance_text(lang, length * 1000.0)), ("depth", &km_text(lang, s.depth)), ("count", &s.quakes.len().to_string())],
                );
                if !s.history.is_empty() {
                    text.push(' ');
                    text.push_str(&trf(lang, "Historical ISC-GEM foci {count}.", &[("count", &s.history.len().to_string())]));
                }
                let what: Vec<&str> = ["CRUST1.0 crust", "Slab2 slabs", "earthquakes"].iter().zip(self.waiting).filter(|(_, w)| *w).map(|(t, _)| tr(lang, t)).collect();
                if !what.is_empty() {
                    text.push(' ');
                    text.push_str(&trf(lang, "Loading: {what}.", &[("what", &what.join(", "))]));
                }
                ui.label(text);
            });
    }
}

/// Название оболочки PREM по ключу [`cutaway::SHELLS`].
fn shell_name(key: &str) -> &'static str {
    match key {
        "inner_core" => "Inner core",
        "outer_core" => "Outer core",
        "lower_mantle" => "Lower mantle",
        "transition_zone" => "Transition zone",
        "upper_mantle" => "Upper mantle",
        _ => "Crust",
    }
}

/// Название слоя CRUST1.0 по ключу [`data::crust::LAYERS`].
fn crust_name(key: &str) -> &'static str {
    match key {
        "water" => "Water",
        "ice" => "Ice",
        "sediments_upper" => "Upper sediments",
        "sediments_middle" => "Middle sediments",
        "sediments_lower" => "Lower sediments",
        "crust_upper" => "Upper crust",
        "crust_middle" => "Middle crust",
        _ => "Lower crust",
    }
}

/// Часть коры для строк «Континентальная кора, верхняя» и подобных.
fn part_name(key: &str) -> &'static str {
    match key {
        "crust_upper" => "upper",
        "crust_middle" => "middle",
        _ => "lower",
    }
}

/// Число глубины без лишних знаков, как `{:g}` модуля.
fn depth_number(value: f64) -> String {
    format!("{}", (value * 10.0).round() / 10.0)
}

/// Строки легенды, `CutawayLegend._rows` модуля: с моделью коры - слои
/// CRUST1.0 сверху и мантия от Мохо, oceanic - кора делится на
/// континентальную и океаническую, slabs - строка плиты Slab2.
fn legend_rows(lang: Lang, crust: bool, oceanic: bool, slabs: bool) -> Vec<(String, [u8; 3])> {
    let oceanic = crust && oceanic;
    let mut rows = Vec::new();
    if crust {
        for key in cutaway::crust_layers() {
            if oceanic && cutaway::ocean_color(key).is_some() {
                continue;
            }
            if let Some(color) = cutaway::crust_color(key) {
                rows.push((tr(lang, crust_name(key)).to_string(), color));
            }
        }
        if oceanic {
            for (title, sea) in [("Continental crust, {part}", false), ("Oceanic crust, {part}", true)] {
                for (key, ocean) in cutaway::OCEAN_COLORS {
                    let color = if sea { Some(ocean) } else { cutaway::crust_color(key) };
                    if let Some(color) = color {
                        rows.push((trf(lang, title, &[("part", tr(lang, part_name(key)))]), color));
                    }
                }
            }
        }
    }
    if slabs {
        rows.push((tr(lang, "Slab2 slab").to_string(), cutaway::SLAB_COLOR));
    }
    let top = cutaway::PREM_RADIUS;
    for shell in cutaway::SHELLS.iter().rev() {
        if shell.key == "crust" && crust {
            continue;
        }
        let name = tr(lang, shell_name(shell.key));
        let text = if shell.key == "upper_mantle" && crust {
            trf(lang, "{name}, Moho-{depth}", &[("name", name), ("depth", &depth_number(top - shell.low))])
        } else {
            format!("{name}, {}-{}", depth_number(top - shell.high), depth_number(top - shell.low))
        };
        rows.push((text, shell.color));
    }
    rows
}

/// Легенда справа от графика. missing - модель коры не пришла.
fn legend(ui: &mut egui::Ui, lang: Lang, s: &Section, missing: bool) {
    ui.set_width(LEGEND_WIDTH);
    let crust = s.bounds.is_some();
    let title = if crust { "CRUST1.0 crust, PREM shells, depth, km" } else { "PREM shells, depth, km" };
    ui.label(egui::RichText::new(tr(lang, title)).small().strong());
    if missing && !crust {
        ui.label(egui::RichText::new(tr(lang, "The CRUST1.0 crust model is not loaded, the crust is a PREM layer.")).small());
    }
    let slabs = s.slab_top.iter().any(|v| v.is_finite());
    for (text, color) in legend_rows(lang, crust, s.oceanic.is_some(), slabs) {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, rgb(color));
            ui.label(egui::RichText::new(text).small());
        });
    }
}

/// Слой между линиями top и bottom (глубины по точкам линии), кусками
/// там, где обе определены и слой не нулевой.
pub(crate) fn fill(painter: &egui::Painter, to_screen: &dyn Fn(f64, f64) -> egui::Pos2, distance: &[f64], top: &[f64], bottom: &[f64], color: [u8; 3]) {
    let n = distance.len().min(top.len()).min(bottom.len());
    let good = |i: usize| top[i].is_finite() && bottom[i].is_finite() && bottom[i] > top[i];
    let color = rgb(color);
    let mut i = 0;
    while i < n {
        if !good(i) {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && good(i) {
            i += 1;
        }
        if i - start < 2 {
            continue;
        }
        let mut mesh = egui::Mesh::default();
        for k in start..i {
            mesh.colored_vertex(to_screen(distance[k], top[k]), color);
            mesh.colored_vertex(to_screen(distance[k], bottom[k]), color);
        }
        for j in 0..(i - start - 1) as u32 {
            let k = 2 * j;
            mesh.add_triangle(k, k + 1, k + 2);
            mesh.add_triangle(k + 1, k + 3, k + 2);
        }
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// График разреза. Возвращает номер точки линии под курсором.
fn chart(ui: &mut egui::Ui, lang: Lang, s: &Section) -> Option<usize> {
    let width = ui.available_width().max(240.0);
    let height = (ui.available_height() - STATS_HEIGHT).max(MIN_CHART);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let text_color = ui.visuals().text_color();
    let font = egui::FontId::proportional(11.0);
    let plot = egui::Rect::from_min_max(rect.min + egui::vec2(60.0, 12.0), rect.max - egui::vec2(14.0, 22.0));
    let n = s.distance.len();
    let x1 = s.distance.last().copied().unwrap_or(1.0).max(1.0);
    let y1 = s.depth;
    let to_screen = |d: f64, depth: f64| egui::pos2(plot.left() + (d / x1) as f32 * plot.width(), plot.top() + (depth.clamp(0.0, y1) / y1) as f32 * plot.height());
    // Оболочки PREM: верхняя мантия от Мохо, остальные - ровные.
    let moho = section::moho(s);
    for layer in section::shells_below(s) {
        let upper = if layer.key == "upper_mantle" { moho.clone() } else { vec![layer.top; n] };
        fill(&painter, &to_screen, &s.distance, &upper, &vec![layer.bottom; n], layer.color);
    }
    // Кора: слои CRUST1.0 или слой PREM до Мохо.
    match &s.bounds {
        Some(bounds) => {
            for (k, key) in cutaway::crust_layers().iter().enumerate() {
                let top: Vec<f64> = bounds.iter().map(|b| -b[k]).collect();
                let bottom: Vec<f64> = bounds.iter().map(|b| -b[k + 1]).collect();
                let (Some(land), sea) = (cutaway::crust_color(key), cutaway::ocean_color(key)) else {
                    continue;
                };
                let Some(sea) = sea else {
                    fill(&painter, &to_screen, &s.distance, &top, &bottom, land);
                    continue;
                };
                for (ocean, top, bottom) in cutaway::split_types(&top, &bottom, s.oceanic.as_deref()) {
                    fill(&painter, &to_screen, &s.distance, &top, &bottom, if ocean { sea } else { land });
                }
            }
        }
        None => fill(&painter, &to_screen, &s.distance, &vec![0.0; n], &moho, section::crust_shell_color()),
    }
    // Плита поверх мантии и коры.
    fill(&painter, &to_screen, &s.distance, &s.slab_top, &s.slab_bottom, cutaway::SLAB_COLOR);
    // Очаги: цвет по глубине, размер по магнитуде, как на глобусе.
    // Исторические ISC-GEM - под свежими, со светлой обводкой.
    for (events, outline) in [(&s.history, HISTORY_OUTLINE), (&s.quakes, FRESH_OUTLINE)] {
        for q in events.iter().filter(|q| q.depth <= y1) {
            let radius = quakes::size(q.mag) as f32 * QUAKE_SHRINK / 2.0;
            painter.circle(to_screen(q.along, q.depth), radius, rgb(quakes::depth_color(q.depth)), egui::Stroke::new(0.8, outline));
        }
    }
    // Сетка и подписи осей.
    let grid = egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(128, 128, 128, 90));
    let step = crate::profile::nice_step(y1, 5.0);
    let mut z = 0.0;
    while z <= y1 + 1e-9 {
        let y = to_screen(0.0, z).y;
        painter.line_segment([egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)], grid);
        painter.text(egui::pos2(plot.left() - 6.0, y), egui::Align2::RIGHT_CENTER, km_text(lang, z), font.clone(), text_color);
        z += step;
    }
    let step = crate::profile::nice_step(x1, 6.0);
    let mut d = 0.0;
    while d <= x1 + 1e-9 {
        let x = to_screen(d, y1).x;
        painter.line_segment([egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())], grid);
        painter.text(egui::pos2(x, plot.bottom() + 4.0), egui::Align2::CENTER_TOP, crate::profile::axis_text(lang, d * 1000.0, step * 1000.0), font.clone(), text_color);
        d += step;
    }
    let pos = response.hover_pos().filter(|p| plot.x_range().contains(p.x))?;
    let target = f64::from((pos.x - plot.left()) / plot.width()) * x1;
    let i = s.distance.partition_point(|d| *d < target).min(n.checked_sub(1)?);
    let x = to_screen(s.distance[i], 0.0).x;
    painter.line_segment([egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())], egui::Stroke::new(1.2, HOVER));
    let mut parts = vec![crate::profile::distance_text(lang, s.distance[i] * 1000.0), trf(lang, "Moho {depth}", &[("depth", &km_text(lang, moho[i]))])];
    if s.slab_top[i].is_finite() {
        parts.push(trf(lang, "slab {top}-{bottom}", &[("top", &km_text(lang, s.slab_top[i])), ("bottom", &km_text(lang, s.slab_bottom[i]))]));
    }
    painter.text(plot.left_top() + egui::vec2(6.0, 2.0), egui::Align2::LEFT_TOP, parts.join(", "), font, text_color);
    Some(i)
}

#[cfg(test)]
mod tests {
    use super::legend_rows;
    use crate::i18n::Lang;

    #[test]
    fn legend_follows_crust_and_slabs() {
        // Без модели коры - кора PREM, с моделью - слои CRUST1.0 и мантия
        // от Мохо, как `CutawayLegend` модуля.
        let plain: Vec<String> = legend_rows(Lang::EN, false, false, false).into_iter().map(|r| r.0).collect();
        assert_eq!(plain.first().map(String::as_str), Some("Crust, 0-24.4"));
        assert_eq!(plain.last().map(String::as_str), Some("Inner core, 5149.5-6371"));
        assert_eq!(plain.len(), 6);
        let full: Vec<String> = legend_rows(Lang::EN, true, true, true).into_iter().map(|r| r.0).collect();
        assert!(full.iter().any(|r| r == "Oceanic crust, upper"));
        assert!(full.iter().any(|r| r == "Slab2 slab"));
        assert!(full.iter().any(|r| r == "Upper mantle, Moho-400"));
        assert!(!full.iter().any(|r| r.starts_with("Crust,")));
    }
}
