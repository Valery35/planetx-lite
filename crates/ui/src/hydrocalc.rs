// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Вода по рельефу: окна «Затопление», «Сток воды» и «Уровень моря» -
//! подменю «Анализ участка» меню на глобусе, перенос `ui/hydro.py` модуля.
//! Расчёт - `data::hydro`, высоты квадрата - [`crate::gridheights`].
//!
//! Затопление - слой над картами: залитые узлы с цветом по глубине,
//! уровень меняется без пересчёта. У модуля это поверхность воды сеткой
//! в воде водоёмов вида. Линия стока, водосбор, бассейны и речная сеть
//! ложатся в «Мои метки» - у модуля бассейны и речная сеть идут слоями
//! GeoPackage в проект QGIS. Уровень моря по всему глобусу - проход воды
//! тайлов на высоте [`Hydro::sea_level`].

use std::sync::Arc;

use data::hydro::{self, Grid, LineEnd};
use eframe::egui;
use globe_core::lod::TileKey;

use crate::analysis::{self, Button, Saved, SiteEdit};
use crate::gridheights::{GridHeights, Step};
use crate::i18n::{Lang, tr, trf};
use crate::reliefcalc::{escape, kml_color, number};

/// Участки окон по умолчанию и наибольший, километры, `FLOOD_SIDE`,
/// `FLOW_SIDE` и `MOST` модуля.
const FLOOD_SIDE: f64 = 10.0;
const FLOW_SIDE: f64 = 20.0;
const MOST: f64 = 100.0;

/// Число отчёта водосбора, «-» без значения.
fn num(value: Option<f64>) -> String {
    value.map_or("-".to_string(), number)
}

/// Подъём воды ползунком над высотой точки и подъём по умолчанию, м,
/// `RISE` и `DEFAULT_RISE` модуля.
const RISE: f64 = 50.0;
const DEFAULT_RISE: f64 = 2.0;
/// Уровень моря окна «Уровень моря», м, `SEA_LOW` и `SEA_HIGH` модуля.
pub const SEA_LOW: f64 = -130.0;
pub const SEA_HIGH: f64 = 100.0;
/// Водосбор метки у края участка считается на участке в GROW раз шире,
/// пока полусторона не дойдёт до SHAPE_MOST метров.
const GROW: f64 = 2.5;
const SHAPE_MOST: f64 = 50000.0;
/// Выполаживание по умолчанию: уклон 2° на 50 м. Линий от метки -
/// не больше LINES_MOST самых длинных.
const FLAT_SLOPE: f64 = 2.0;
const FLAT_LENGTH: f64 = 50.0;
const LINES_MOST: usize = 300;
const LINE_COLOR: [u8; 4] = [30, 110, 255, 255];
const BASIN_COLOR: [u8; 4] = [20, 90, 220, 255];
const BASIN_FILL: [u8; 4] = [60, 140, 255, 60];

/// Что строится по высотам квадрата.
#[derive(Debug, Clone, PartialEq)]
enum Task {
    Flood,
    Line,
    Basin,
    Rivers,
    Basins,
    /// Линии стока от узлов метки: точки, многоугольник ли.
    ShapeLines(Vec<(f64, f64)>, bool, String),
    /// Водосбор метки: точки, многоугольник ли, название, полусторона.
    ShapeBasin(Vec<(f64, f64)>, bool, String, f64),
}

/// Участок для расчёта: середина, полусторона рамки, вершины.
type SiteJob = (f64, f64, f64, Vec<(f64, f64)>);

/// Метка окна «Сток воды»: точки, многоугольник ли, название.
type FlowItem = (Vec<(f64, f64)>, bool, String);

/// Затопление: сетка, высоты, уровни перелива, уровень, итог.
struct FloodState {
    grid: Grid,
    z: Arc<Vec<f64>>,
    levels: Vec<f64>,
    point: (f64, f64),
    base: f64,
    level: f64,
    partial: bool,
    flood: Option<hydro::Flood>,
}

/// Слой воды затопления для картинок тайлов.
#[derive(Clone)]
pub struct Water {
    grid: Grid,
    z: Arc<Vec<f64>>,
    flood: Arc<hydro::Flood>,
    level: f64,
    radius: f64,
}

/// Окна гидрологии и вода затопления.
#[derive(Default)]
pub struct Hydro {
    pub flood_open: bool,
    pub flow_open: bool,
    pub sea_open: bool,
    flood_point: (f64, f64),
    flood_place: String,
    flood_status: String,
    flood: Option<FloodState>,
    /// Участок окна «Затопление».
    pub flood_site: Option<SiteEdit>,
    flow_point: (f64, f64),
    flow_place: String,
    /// Метка окна «Сток воды»: точки, многоугольник ли, название.
    flow_item: Option<FlowItem>,
    /// Участок окна «Сток воды».
    pub flow_site: Option<SiteEdit>,
    /// Сессия окна «Сток воды» и её участок: расчёты ложатся в одну папку.
    flow_session: Option<(u64, Vec<(f64, f64)>)>,
    /// Участок идущего расчёта, пусто - без участка.
    job_ring: Vec<(f64, f64)>,
    net_km2: f64,
    order: u32,
    snap_m: f64,
    on_stream: bool,
    on_flat: bool,
    flat_slope: f64,
    flat_len: f64,
    merge: bool,
    flow_status: String,
    /// Уровень моря над нынешним, м.
    pub sea_level: f64,
    job: Option<(GridHeights, Task)>,
    radius: f64,
    water: Option<Water>,
    /// Слой сменился: картинки тайлов рисуются заново.
    pub changed: bool,
    /// Расчёты для «Моих меток», забирает окно глобуса.
    pub saved: Vec<Saved>,
    /// Текст для строки сообщений окна глобуса: расчёт по метке без окна.
    pub message: Option<String>,
}

fn line_kml(name: &str, text: &str, points: &[(f64, f64)], color: [u8; 4], width: f64) -> String {
    let coords: Vec<String> = points.iter().map(|(la, lo)| format!("{lo:.7},{la:.7},0")).collect();
    format!(
        "<Placemark><name>{}</name><description>{}</description><Style><LineStyle><color>{}</color><width>{width}</width></LineStyle></Style><LineString><tessellate>1</tessellate><coordinates>{}</coordinates></LineString></Placemark>",
        escape(name),
        escape(text),
        kml_color(color),
        coords.join(" ")
    )
}

fn polygon_kml(name: &str, text: &str, ring: &[(f64, f64)], fill: [u8; 4]) -> String {
    let mut coords: Vec<String> = ring.iter().map(|(la, lo)| format!("{lo:.7},{la:.7},0")).collect();
    if let Some(first) = coords.first().cloned() {
        coords.push(first);
    }
    format!(
        "<Placemark><name>{}</name><description>{}</description><Style><LineStyle><color>{}</color><width>2</width></LineStyle><PolyStyle><color>{}</color></PolyStyle></Style><Polygon><tessellate>1</tessellate><outerBoundaryIs><LinearRing><coordinates>{}</coordinates></LinearRing></outerBoundaryIs></Polygon></Placemark>",
        escape(name),
        escape(text),
        kml_color(BASIN_COLOR),
        kml_color(fill),
        coords.join(" ")
    )
}

fn folder_kml(name: &str, body: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><kml xmlns=\"http://www.opengis.net/kml/2.2\"><Folder><name>{}</name>{body}</Folder></kml>", escape(name))
}

fn document_kml(body: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><kml xmlns=\"http://www.opengis.net/kml/2.2\"><Document>{body}</Document></kml>")
}

/// Чем кончилась линия стока, строкой, `end_text` модуля.
fn end_text(lang: Lang, end: LineEnd) -> &'static str {
    tr(
        lang,
        match end {
            LineEnd::Edge => "The line reached the edge of the area, further the water goes beyond it.",
            LineEnd::Stream => "The line ends at a watercourse.",
            LineEnd::Flat => "The line ends where the terrain flattens.",
            LineEnd::Merge => "The line flows into another flow line.",
            LineEnd::Pit => "The line ends at the edge of the height data.",
        },
    )
}

/// Глубины узлов затопления mask при уровне level, сухие узлы - нечисло.
fn flood_depth(mask: &[bool], z: &[f64], level: f64) -> Vec<f64> {
    mask.iter().zip(z).map(|(m, z)| if *m { level - z } else { f64::NAN }).collect()
}

fn partial_text(lang: Lang) -> &'static str {
    tr(lang, "Not all heights are loaded, the computation used less detailed ones.")
}

/// Главный водоток строкой, `stream_text` модуля.
fn stream_text(lang: Lang, rep: &data::topo_gauge::Report) -> String {
    trf(lang, "Main watercourse {km} km, fall {fall} m, slope {slope} per mille.", &[("km", &num(rep.get("stream_km"))), ("fall", &num(rep.get("stream_fall_m"))), ("slope", &num(rep.get("stream_ppm")))])
}

/// Итог водосбора строками, `basin_text` модуля.
fn basin_text(lang: Lang, rep: &data::topo_gauge::Report) -> String {
    let mut lines = vec![trf(
        lang,
        "Area {area} km², heights {low}-{high} m, mean {mean} m.",
        &[("area", &num(rep.get("area_km2"))), ("low", &num(rep.get("z_min"))), ("high", &num(rep.get("z_max"))), ("mean", &num(rep.get("z_mean")))],
    )];
    lines.push(stream_text(lang, rep));
    if let Some(slope) = rep.get("sp_slope_ppm") {
        lines.push(trf(lang, "Mean slope of the hillsides {slope} per mille by SP 33-101-2003.", &[("slope", &number(slope))]));
    }
    if let Some(dens) = rep.get("net_dens") {
        lines.push(trf(lang, "Drainage density {dens} km per km², watercourses with a catchment from {area} km².", &[("dens", &number(dens)), ("area", &number(hydro::NET_AREA / 1e6))]));
    }
    lines.join(" ")
}

impl Hydro {
    pub fn new() -> Self {
        Self { net_km2: hydro::NET_AREA / 1e6, order: 1, snap_m: hydro::SNAP_METRES, flat_slope: FLAT_SLOPE, flat_len: FLAT_LENGTH, merge: true, ..Default::default() }
    }

    /// Окно «Затопление» для точки: участок - квадрат у неё.
    pub fn flood_here(&mut self, lat: f64, lon: f64, place: String) {
        self.flood_point = (lat, lon);
        self.flood_place = place;
        self.flood_status.clear();
        self.flood_open = true;
        match &mut self.flood_site {
            Some(s) => s.move_to(lat, lon),
            None => self.flood_site = Some(SiteEdit::new(lat, lon, FLOOD_SIDE, MOST)),
        }
    }

    /// Окно «Сток воды» для точки: участок - квадрат у неё.
    pub fn flow_here(&mut self, lat: f64, lon: f64, place: String) {
        self.flow_point = (lat, lon);
        self.flow_place = place;
        self.flow_item = None;
        self.flow_status.clear();
        self.flow_open = true;
        match &mut self.flow_site {
            Some(s) => s.move_to(lat, lon),
            None => self.flow_site = Some(SiteEdit::new(lat, lon, FLOW_SIDE, MOST)),
        }
    }

    /// Окно «Сток воды» для пути или многоугольника «Моих меток»,
    /// `flow_from` модуля: участок - квадрат, накрывающий метку.
    pub fn flow_from(&mut self, points: Vec<(f64, f64)>, polygon: bool, name: String, radius: f64) {
        if points.is_empty() {
            return;
        }
        let Some((lat, lon, side)) = hydro::shape_grid(&points, radius, hydro::SHAPE_REACH, hydro::SHAPE_LEAST, hydro::SHAPE_MOST) else {
            return;
        };
        self.flow_here(lat, lon, name.clone());
        if let Some(s) = &mut self.flow_site {
            s.site.side_km = (2.0 * side / 1000.0).max(s.site.side_km);
            s.site.reset_square();
        }
        self.flow_item = Some((points, polygon, name));
    }

    /// Участки открытых окон на глобусе.
    pub fn site_edits(&mut self) -> Vec<&mut SiteEdit> {
        let mut out = Vec::new();
        if self.flood_open
            && let Some(s) = &mut self.flood_site
        {
            out.push(s);
        }
        if self.flow_open
            && let Some(s) = &mut self.flow_site
        {
            out.push(s);
        }
        out
    }

    /// Участки открытых окон для рисования.
    pub fn sites(&self) -> Vec<&SiteEdit> {
        let mut out = Vec::new();
        if self.flood_open
            && let Some(s) = &self.flood_site
        {
            out.push(s);
        }
        if self.flow_open
            && let Some(s) = &self.flow_site
        {
            out.push(s);
        }
        out
    }

    /// Метки KML расчёта стока - в папку сессии окна, `_group_of` модуля:
    /// сессия служит, пока участок прежний.
    fn emit_flow(&mut self, lang: Lang, kml: &str) {
        let ring = self.flow_site.as_ref().map(|s| s.site.ring.clone()).unwrap_or_default();
        let session = match &self.flow_session {
            Some((id, r)) if *r == ring => *id,
            _ => {
                let id = analysis::new_session();
                self.flow_session = Some((id, ring.clone()));
                id
            }
        };
        self.saved.push(Saved { title: tr(lang, "Water flow").to_string(), ring, layers: Vec::new(), extra: analysis::kml_items(kml), session: Some(session) });
    }

    /// Участок окна для расчёта: середина, полусторона и вершины, или
    /// текст, почему считать нельзя.
    fn site_job(site: Option<&SiteEdit>, lang: Lang) -> Result<SiteJob, String> {
        let Some(s) = site else {
            return Err(tr(lang, "Close the area by a click on its first vertex.").to_string());
        };
        if let Some(problem) = s.problem(lang) {
            return Err(problem);
        }
        let (lat, lon) = s.site.centre();
        Ok((lat, lon, s.site.size() / 2.0, s.site.ring.clone()))
    }

    /// Водосбор метки, `catchment_of` модуля: итог - многоугольник
    /// в «Моих метках», текст - в строку сообщений.
    pub fn catchment_of(&mut self, points: Vec<(f64, f64)>, polygon: bool, name: String, radius: f64, max_level: u32, now: f64) {
        if points.is_empty() {
            return;
        }
        let Some((lat, lon, side)) = hydro::shape_grid(&points, radius, hydro::SHAPE_REACH, hydro::SHAPE_LEAST, hydro::SHAPE_MOST) else {
            return;
        };
        self.radius = radius;
        self.job = Some((GridHeights::new(lat, lon, side, radius, max_level, now), Task::ShapeBasin(points, polygon, name, side)));
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Слой воды затопления для картинок тайлов.
    pub fn painters(&self) -> Vec<(&'static str, crate::vector::Painter)> {
        match &self.water {
            Some(w) => {
                let w = w.clone();
                vec![("flood", Arc::new(move |key: TileKey| water_tile(&w, key)) as crate::vector::Painter)]
            }
            None => Vec::new(),
        }
    }

    /// Расчёт по участку окна: полусторона рамки и вершины участка, вне
    /// него высот нет.
    #[allow(clippy::too_many_arguments)]
    fn start(&mut self, task: Task, lat: f64, lon: f64, radius_m: f64, ring: Vec<(f64, f64)>, radius: f64, max_level: u32, now: f64) {
        self.radius = radius;
        self.job_ring = ring;
        self.job = Some((GridHeights::new(lat, lon, radius_m, radius, max_level, now), task));
    }

    fn set_status(&mut self, task: &Task, text: String) {
        match task {
            Task::Flood => self.flood_status = text,
            Task::ShapeBasin(..) => self.message = Some(text),
            _ => self.flow_status = text,
        }
    }

    /// Шаг: окна, высоты, расчёт. Возвращает тайлы высот, которых ждёт
    /// расчёт.
    #[allow(clippy::too_many_arguments)]
    pub fn step(&mut self, ctx: &egui::Context, lang: Lang, now: f64, radius: f64, max_level: u32, has: &dyn Fn(TileKey) -> bool, heights: data::viewshed::Heights) -> Vec<TileKey> {
        self.flood_window(ctx, lang, now, radius, max_level);
        self.flow_window(ctx, lang, now, radius, max_level);
        self.sea_window(ctx, lang);
        let Some((job, task)) = &mut self.job else {
            return Vec::new();
        };
        let wanted = job.wanted(has);
        let task = task.clone();
        match job.step(lang, now, has, heights) {
            Step::Going(text) => {
                if !matches!(task, Task::ShapeBasin(..)) {
                    self.set_status(&task, text);
                }
                ctx.request_repaint();
            }
            Step::Failed(text) => {
                self.job = None;
                self.set_status(&task, text);
            }
            Step::Done(z) => {
                let Some((job, _)) = self.job.take() else {
                    return wanted;
                };
                let (grid, partial) = (job.grid, job.partial);
                self.finish(lang, task, grid, z, partial, max_level, now);
            }
        }
        wanted
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(&mut self, lang: Lang, task: Task, grid: Grid, mut z: Vec<f64>, partial: bool, max_level: u32, now: f64) {
        let radius = self.radius;
        data::site::mask_grid(&mut z, &grid, &std::mem::take(&mut self.job_ring), radius);
        let built = analysis::built_text(lang, crate::pointcalc::clock_now());
        let tail = |text: String| if partial { format!("{text} {} {built}", partial_text(lang)) } else { format!("{text} {built}") };
        let to_points = |rows: &[f64], cols: &[f64]| -> Vec<(f64, f64)> { rows.iter().zip(cols).map(|(r, c)| crate::gridheights::node(&grid, *r, *c, radius)).collect() };
        let ring_points = |ring: &[[f64; 2]]| -> Vec<(f64, f64)> { ring.iter().map(|p| crate::gridheights::node(&grid, p[0], p[1], radius)).collect() };
        match task {
            Task::Flood => {
                let seed = hydro::index_of(&grid, self.flood_point.0, self.flood_point.1, radius);
                let base = z.get(seed.0 * grid.cols + seed.1).copied().unwrap_or(f64::NAN);
                if !base.is_finite() {
                    self.flood_status = tr(lang, "The point is outside the area or has no heights.").to_string();
                    return;
                }
                let levels = hydro::spill_levels(&z, grid.rows, grid.cols, seed);
                let level = match &self.flood {
                    Some(old) if old.point == self.flood_point => old.level,
                    _ => base + DEFAULT_RISE,
                };
                self.flood = Some(FloodState { grid, z: Arc::new(z), levels, point: self.flood_point, base, level, partial, flood: None });
                self.set_level(lang, level);
            }
            Task::Line => {
                let seed = hydro::index_of(&grid, self.flow_point.0, self.flow_point.1, radius);
                if !z.get(seed.0 * grid.cols + seed.1).is_some_and(|h| h.is_finite()) {
                    self.flow_status = tr(lang, "There are no heights at the point.").to_string();
                    return;
                }
                let (area, flat) = self.stops();
                let Some(line) = hydro::flow_line(&z, grid.rows, grid.cols, seed, grid.cell, area, flat).filter(|l| l.rows.len() >= 2) else {
                    self.flow_status = tr(lang, "Water from the point flows nowhere.").to_string();
                    return;
                };
                let points = to_points(&line.rows.iter().map(|v| *v as f64).collect::<Vec<_>>(), &line.cols.iter().map(|v| *v as f64).collect::<Vec<_>>());
                let name = tr(lang, "Flow line").to_string();
                let text = tail(format!("{} {}", trf(lang, "Length {km} km, drop {drop} m.", &[("km", &number(line.length / 1000.0)), ("drop", &number(line.drop))]), end_text(lang, line.end)));
                self.emit_flow(lang, &document_kml(&line_kml(&name, &text, &points, LINE_COLOR, 3.0)));
                self.flow_status = trf(lang, "{name}: {text}", &[("name", &name), ("text", &text)]);
            }
            Task::ShapeLines(points, polygon, name) => {
                let (seeds, _) = hydro::shape_seeds(&grid, if polygon { hydro::Shape::Polygon } else { hydro::Shape::Path }, &points, radius);
                let seeds: Vec<usize> = seeds.into_iter().filter(|s| z.get(*s).is_some_and(|h| h.is_finite())).collect();
                if seeds.is_empty() {
                    self.flow_status = tr(lang, "The placemark is beyond the edge of the height data.").to_string();
                    return;
                }
                let (area, flat) = self.stops();
                let mut lines: Vec<hydro::Line> = hydro::flow_lines(&z, grid.rows, grid.cols, &seeds, grid.cell, area, flat, self.merge).into_iter().filter(|l| l.rows.len() >= hydro::MIN_LINE).collect();
                if lines.is_empty() {
                    self.flow_status = tr(lang, "Water from the placemark flows nowhere.").to_string();
                    return;
                }
                let dropped = lines.len().saturating_sub(LINES_MOST);
                lines.sort_by(|a, b| b.length.total_cmp(&a.length));
                lines.truncate(LINES_MOST);
                let mut body = String::new();
                for (n, line) in lines.iter().enumerate() {
                    let pts = to_points(&line.rows.iter().map(|v| *v as f64).collect::<Vec<_>>(), &line.cols.iter().map(|v| *v as f64).collect::<Vec<_>>());
                    let text = format!("{} {}", trf(lang, "Length {km} km, drop {drop} m.", &[("km", &number(line.length / 1000.0)), ("drop", &number(line.drop))]), end_text(lang, line.end));
                    body.push_str(&line_kml(&trf(lang, "Flow line {n}", &[("n", &(n + 1).to_string())]), &text, &pts, LINE_COLOR, 2.0));
                }
                let title = trf(lang, "Flow lines «{name}»", &[("name", &name)]);
                self.emit_flow(lang, &folder_kml(&title, &body));
                let total: f64 = lines.iter().map(|l| l.length).sum();
                let mut text = trf(lang, "{name}: lines {count}, total length {km} km.", &[("name", &title), ("count", &lines.len().to_string()), ("km", &number(total / 1000.0))]);
                if dropped > 0 {
                    text = format!("{text} {}", trf(lang, "The shortest lines not included {count}.", &[("count", &dropped.to_string())]));
                }
                self.flow_status = tail(text);
            }
            Task::Basin => {
                let seed = hydro::index_of(&grid, self.flow_point.0, self.flow_point.1, radius);
                if !z.get(seed.0 * grid.cols + seed.1).is_some_and(|h| h.is_finite()) {
                    self.flow_status = tr(lang, "There are no heights at the point.").to_string();
                    return;
                }
                let snap = (self.snap_m / grid.cell).round().max(0.0) as usize;
                let Some(basin) = hydro::watershed(&z, grid.rows, grid.cols, seed, grid.cell, Some(snap)) else {
                    self.flow_status = tr(lang, "Water does not flow to the point: there is no catchment.").to_string();
                    return;
                };
                let rings = hydro::outline(&basin.mask, grid.rows, grid.cols, 0.5);
                let Some(ring) = rings.first().filter(|_| basin.mask.iter().filter(|m| **m).count() >= 3) else {
                    self.flow_status = tr(lang, "Water does not flow to the point: there is no catchment.").to_string();
                    return;
                };
                let name = tr(lang, "Catchment").to_string();
                let mut text = basin_text(lang, &basin.report);
                let moved = (basin.seed.0 as f64 - seed.0 as f64).hypot(basin.seed.1 as f64 - seed.1 as f64) * grid.cell;
                if moved > 0.0 {
                    text = format!("{text} {}", trf(lang, "The outlet is moved to the channel {m} m from the point.", &[("m", &number(moved))]));
                }
                if basin.edge {
                    text = format!("{text} {}", tr(lang, "The catchment reaches the edge of the area, its part beyond the edge is not counted. A larger area is needed."));
                }
                text = tail(text);
                let mut body = polygon_kml(&name, &text, &ring_points(ring), BASIN_FILL);
                if basin.stream_rows.len() > 1 {
                    let rows: Vec<f64> = basin.stream_rows.iter().rev().map(|v| *v as f64).collect();
                    let cols: Vec<f64> = basin.stream_cols.iter().rev().map(|v| *v as f64).collect();
                    body.push_str(&line_kml(tr(lang, "Main watercourse"), &stream_text(lang, &basin.report), &to_points(&rows, &cols), LINE_COLOR, 3.0));
                }
                self.emit_flow(lang, &folder_kml(&name, &body));
                self.flow_status = trf(lang, "{name}: {text}", &[("name", &name), ("text", &text)]);
            }
            Task::Basins => {
                let regions = hydro::outlet_basins(&z, grid.rows, grid.cols, grid.cell, self.net_km2 * 1e6);
                let mut body = String::new();
                let (mut count, mut covered) = (0usize, 0.0);
                for region in &regions {
                    let rings = hydro::outline(&region.mask, grid.rows, grid.cols, 0.5);
                    let Some(ring) = rings.first() else {
                        continue;
                    };
                    count += 1;
                    covered += region.area / 1e6;
                    // Бассейны разными цветами по номеру, `style_basins` модуля.
                    let hue = (count * 47 % 360) as f64;
                    let [r, g, b] = hsl(hue, 0.7, 0.55);
                    let label = trf(lang, "Basin {n}", &[("n", &count.to_string())]);
                    body.push_str(&polygon_kml(&label, &trf(lang, "Area {area} km².", &[("area", &number(region.area / 1e6))]), &ring_points(ring), [r, g, b, 115]));
                }
                if count == 0 {
                    self.flow_status = tr(lang, "There are no watercourses with such a catchment at the edge of the area.").to_string();
                    return;
                }
                self.emit_flow(lang, &folder_kml(tr(lang, "Basins"), &body));
                self.flow_status = tail(trf(lang, "Basins written to «My places»: {count}, they cover {area} km² of the area.", &[("count", &count.to_string()), ("area", &number(covered))]));
            }
            Task::Rivers => {
                let streams = hydro::rivers(&z, grid.rows, grid.cols, grid.cell, self.net_km2 * 1e6, self.order);
                if streams.is_empty() {
                    self.flow_status = tr(lang, "There are no watercourses with such a catchment and order in the area.").to_string();
                    return;
                }
                let mut body = String::new();
                for s in &streams {
                    let pts = to_points(&s.rows.iter().map(|v| *v as f64).collect::<Vec<_>>(), &s.cols.iter().map(|v| *v as f64).collect::<Vec<_>>());
                    let text = trf(lang, "Order {order}, catchment {area} km².", &[("order", &s.order.to_string()), ("area", &number(s.area / 1e6))]);
                    // Толщина растёт с порядком, `style_rivers` модуля.
                    body.push_str(&line_kml(&trf(lang, "Watercourse of order {order}", &[("order", &s.order.to_string())]), &text, &pts, LINE_COLOR, 0.6 + 0.8 * f64::from(s.order)));
                }
                self.emit_flow(lang, &folder_kml(tr(lang, "River network"), &body));
                let top = streams.iter().map(|s| s.order).max().unwrap_or(1);
                self.flow_status = tail(trf(lang, "Watercourses written to «My places»: {count}, the largest order {order}.", &[("count", &streams.len().to_string()), ("order", &top.to_string())]));
            }
            Task::ShapeBasin(points, polygon, name, side) => {
                let (seeds, length) = hydro::shape_seeds(&grid, if polygon { hydro::Shape::Polygon } else { hydro::Shape::Path }, &points, radius);
                if seeds.is_empty() {
                    self.message = Some(tr(lang, "The placemark is beyond the edge of the height data.").to_string());
                    return;
                }
                let Some(result) = hydro::shape_catchment(&z, grid.rows, grid.cols, &seeds, grid.cell, Some(length)) else {
                    self.message = Some(tr(lang, "Water does not flow to the placemark: there is no catchment.").to_string());
                    return;
                };
                if result.edge && side < SHAPE_MOST * 0.99 {
                    let (lat, lon) = (grid.lat, grid.lon);
                    let wider = (side * GROW).min(SHAPE_MOST);
                    self.job = Some((GridHeights::new(lat, lon, wider, radius, max_level, now), Task::ShapeBasin(points, polygon, name, wider)));
                    return;
                }
                let rings = hydro::outline(&result.mask, grid.rows, grid.cols, 0.5);
                let Some(ring) = rings.first() else {
                    self.message = Some(tr(lang, "Water does not flow to the placemark: there is no catchment.").to_string());
                    return;
                };
                let rep = &result.report;
                let mut text = trf(
                    lang,
                    "Area {area} km², heights {low}-{high} m, mean {mean} m.",
                    &[("area", &num(rep.get("area_km2"))), ("low", &num(rep.get("z_min"))), ("high", &num(rep.get("z_max"))), ("mean", &num(rep.get("z_mean")))],
                );
                if result.edge {
                    text = format!("{text} {}", tr(lang, "The catchment reaches the edge of the area, its part beyond the edge is not counted. A larger area is needed."));
                }
                text = tail(text);
                let title = trf(lang, "Catchment «{name}»", &[("name", &name)]);
                let square = data::site::square_ring(grid.lat, grid.lon, 2.0 * side);
                self.saved.push(Saved { title: tr(lang, "Catchment of a placemark").to_string(), ring: square, layers: Vec::new(), extra: analysis::kml_items(&document_kml(&polygon_kml(&title, &text, &ring_points(ring), BASIN_FILL))), session: None });
                self.message = Some(trf(lang, "{name}: {text}", &[("name", &title), ("text", &text)]));
            }
        }
    }

    /// Остановки линии стока: площадь водотока и (уклон, длина).
    fn stops(&self) -> (Option<f64>, Option<(f64, f64)>) {
        (self.on_stream.then_some(self.net_km2 * 1e6), self.on_flat.then_some((self.flat_slope, self.flat_len)))
    }

    /// Вода уровня level: слой на глобусе и итог в окне, `set_level` модуля.
    fn set_level(&mut self, lang: Lang, level: f64) {
        let radius = self.radius;
        let Some(state) = &mut self.flood else {
            return;
        };
        state.level = level;
        let result = hydro::flood(&state.z, state.grid.rows, state.grid.cols, &state.levels, level, state.grid.cell);
        let any = result.mask.iter().any(|m| *m);
        let mut text = if any {
            let mut t = trf(lang, "Flooded {area} km², water volume {volume} million m³, the largest depth {depth} m.", &[("area", &number(result.area / 1e6)), ("volume", &number(result.volume / 1e6)), ("depth", &number(result.depth))]);
            let depth = flood_depth(&result.mask, &state.z, level);
            t = format!("{t} {}", analysis::stats_text(lang, tr(lang, "Depth"), &depth, tr(lang, " m")));
            if result.edge {
                t = format!("{t} {}", tr(lang, "The water reached the edge of the area and can go further. A larger area is needed."));
            }
            t
        } else {
            tr(lang, "The level is not above the point, there is no water.").to_string()
        };
        text = format!("{text} {}", trf(lang, "The grid step is {step} m.", &[("step", &number(state.grid.cell))]));
        if state.partial {
            text = format!("{text} {}", partial_text(lang));
        }
        self.flood_status = text;
        self.water = any.then(|| Water { grid: state.grid, z: state.z.clone(), flood: Arc::new(result.clone()), level, radius });
        state.flood = Some(result);
        self.changed = true;
    }

    fn clear_flood(&mut self) {
        self.flood = None;
        if self.job.as_ref().is_some_and(|(_, t)| *t == Task::Flood) {
            self.job = None;
        }
        if self.water.take().is_some() {
            self.changed = true;
        }
        self.flood_status.clear();
    }

    fn flood_window(&mut self, ctx: &egui::Context, lang: Lang, now: f64, radius: f64, max_level: u32) {
        if !self.flood_open {
            return;
        }
        if let Some(text) = self.flood_site.as_mut().and_then(|s| s.shared_note(lang)) {
            self.flood_status = text.to_string();
        }
        let mut open = true;
        let mut pressed = None;
        let mut level = self.flood.as_ref().map(|s| s.level);
        let base = self.flood.as_ref().map(|s| s.base);
        egui::Window::new(tr(lang, "Flooding")).id(egui::Id::unique("planetx-flood")).open(&mut open).collapsible(false).resizable(false).min_width(380.0).show(ctx, |ui| {
            egui::Grid::new("planetx-flood-form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Point"));
                ui.label(&self.flood_place);
                ui.end_row();
                ui.label(tr(lang, "Point height"));
                ui.label(base.map_or("-".to_string(), |b| trf(lang, "{h} m", &[("h", &format!("{b:.1}"))])));
                ui.end_row();
                if let Some(site) = &mut self.flood_site {
                    ui.label(tr(lang, "Area|site"));
                    site.row(ui, lang);
                    ui.end_row();
                }
                ui.label(tr(lang, "Water level"));
                ui.add_enabled_ui(level.is_some(), |ui| {
                    if let Some(v) = &mut level {
                        ui.add(egui::DragValue::new(v).range(-11000.0..=9000.0).speed(0.25).fixed_decimals(1).suffix(tr(lang, " m")))
                            .on_hover_text(tr(lang, "Height of the water surface above the sea level. The water floods everything below the level that it reaches from the point. Hollows beyond a higher ridge stay dry."));
                    } else {
                        ui.label("-");
                    }
                });
                ui.end_row();
                ui.label(tr(lang, "Rise"));
                ui.add_enabled_ui(level.is_some(), |ui| {
                    if let (Some(v), Some(b)) = (&mut level, base) {
                        let mut rise = (*v - b).clamp(0.0, RISE);
                        if ui.add(egui::Slider::new(&mut rise, 0.0..=RISE).show_value(false)).on_hover_text(trf(lang, "Rise of the water above the point height, up to {rise} m.", &[("rise", &number(RISE))])).changed() {
                            *v = b + rise;
                        }
                    }
                });
                ui.end_row();
            });
            ui.add_space(4.0);
            let hint = self.flood_site.as_ref().and_then(|s| s.hint(lang));
            ui.add(egui::Label::new(hint.unwrap_or(&self.flood_status)).wrap());
            ui.add_space(4.0);
            pressed = analysis::buttons(ui, lang, self.water.is_some() && self.job.is_none(), tr(lang, "Water from the point up to the given level on the area - a layer on the globe, the level changes without a new computation."));
        });
        if let Some(v) = level
            && self.flood.as_ref().is_some_and(|s| (s.level - v).abs() > 1e-9)
        {
            self.set_level(lang, v);
        }
        match pressed {
            Some(Button::Build) => match Self::site_job(self.flood_site.as_ref(), lang) {
                Ok((lat, lon, half, ring)) => self.start(Task::Flood, lat, lon, half, ring, radius, max_level, now),
                Err(text) => self.flood_status = text,
            },
            Some(Button::Ok) => {
                self.save_flood(lang);
                self.clear_flood();
                self.flood_open = false;
            }
            Some(Button::Cancel) => {
                self.clear_flood();
                self.flood_open = false;
            }
            None => {}
        }
        // Крестик - как «Отмена».
        if !open {
            self.clear_flood();
            self.flood_open = false;
        }
        if let Some(site) = &mut self.flood_site
            && std::mem::take(&mut site.changed)
            && self.flood.is_some()
            && self.job.is_none()
        {
            self.flood_status = tr(lang, "The area has changed. Press «Build» to compute it again.").to_string();
        }
    }

    /// Затопление - расчётом для «Моих меток», `export_flood` модуля:
    /// растр глубин GeoTIFF EPSG:3857, вода картинкой и контур
    /// многоугольниками.
    fn save_flood(&mut self, lang: Lang) {
        let radius = self.radius;
        let Some(state) = &self.flood else {
            return;
        };
        let Some(result) = &state.flood else {
            return;
        };
        if !result.mask.iter().any(|m| *m) {
            return;
        }
        let level = state.level;
        let depth = flood_depth(&result.mask, &state.z, level);
        let tiff = data::geotiff::write_float_mercator(&data::site::mercator_of(&state.grid), &depth).unwrap_or_default();
        let (north, west) = hydro::latlon(&state.grid, -0.5, -0.5, radius);
        let (south, east) = hydro::latlon(&state.grid, state.grid.rows as f64 - 0.5, state.grid.cols as f64 - 0.5, radius);
        let (grid, flood, z) = (state.grid, result.clone(), state.z.clone());
        let sample = move |points: &[(f64, f64)]| hydro::flood_points_rgba(&grid, &flood, &z, level, points, radius);
        let name = trf(lang, "Flooding {level} m - depth", &[("level", &format!("{level:.1}"))]);
        let mut layer = analysis::picture(name, "flood", (south, north, west, east), state.grid.cell, &sample, tiff);
        layer.summary = analysis::stats_text(lang, tr(lang, "Depth"), &depth, tr(lang, " m"));
        let rings = hydro::outline(&result.mask, state.grid.rows, state.grid.cols, 0.5);
        let text = trf(lang, "Level {level} m, flooded {area} km², water volume {volume} million m³, the largest depth {depth} m.", &[("level", &format!("{level:.1}")), ("area", &number(result.area / 1e6)), ("volume", &number(result.volume / 1e6)), ("depth", &number(result.depth))]);
        let mut body = String::new();
        for ring in &rings {
            let pts: Vec<(f64, f64)> = ring.iter().map(|p| crate::gridheights::node(&state.grid, p[0], p[1], radius)).collect();
            body.push_str(&polygon_kml(&trf(lang, "Flooding {level} m - outline", &[("level", &format!("{level:.1}"))]), &text, &pts, [60, 140, 255, 0]));
        }
        let extra = if body.is_empty() { Vec::new() } else { analysis::kml_items(&document_kml(&body)) };
        let ring = self.flood_site.as_ref().map(|s| s.site.ring.clone()).unwrap_or_default();
        self.saved.push(Saved { title: tr(lang, "Flooding").to_string(), ring, layers: vec![layer], extra, session: None });
    }
    fn flow_window(&mut self, ctx: &egui::Context, lang: Lang, now: f64, radius: f64, max_level: u32) {
        if !self.flow_open {
            return;
        }
        if let Some(text) = self.flow_site.as_mut().and_then(|s| s.shared_note(lang)) {
            self.flow_status = text.to_string();
        }
        let mut open = true;
        let mut task = None;
        let item = self.flow_item.is_some();
        egui::Window::new(tr(lang, "Water flow")).id(egui::Id::unique("planetx-flow")).open(&mut open).collapsible(false).resizable(false).min_width(400.0).show(ctx, |ui| {
            egui::Grid::new("planetx-flow-form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, if item { "Placemark" } else { "Point" }));
                ui.label(&self.flow_place);
                ui.end_row();
                if let Some(site) = &mut self.flow_site {
                    ui.label(tr(lang, "Area|site"));
                    site.row(ui, lang);
                    ui.end_row();
                }
                ui.label(tr(lang, "Watercourses from"));
                ui.add(egui::DragValue::new(&mut self.net_km2).range(0.1..=1000.0).speed(0.1).fixed_decimals(1).suffix(tr(lang, " km²")))
                    .on_hover_text(tr(lang, "The river network - watercourses with a catchment of at least this area. Smaller - a denser network with brooks and ravines."));
                ui.end_row();
                ui.label(tr(lang, "Order from"));
                ui.add(egui::DragValue::new(&mut self.order).range(1..=10))
                    .on_hover_text(tr(lang, "The least Strahler order of a watercourse in the river network. A source is order 1, after two watercourses of one order merge - one more. Larger - only large rivers, the orders are counted over the whole network."));
                ui.end_row();
                ui.label(tr(lang, "Channel search radius"));
                ui.add(egui::DragValue::new(&mut self.snap_m).range(0.0..=2000.0).speed(5.0).fixed_decimals(0).suffix(tr(lang, " m")))
                    .on_hover_text(tr(lang, "Within this radius from the point the channel is searched - the place with the largest catchment, the catchment is built from it. A click near a river so gives the catchment of the river. 0 - the catchment exactly from the point."));
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut self.on_stream, tr(lang, "Stop at a watercourse")).on_hover_text(tr(lang, "The flow line ends on reaching a watercourse with a catchment from the area of the field «Watercourses from». Without the check the line goes to the edge of the area."));
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut self.on_flat, tr(lang, "Stop on flattening")).on_hover_text(tr(lang, "The flow line ends where the mean slope over the section length is below the threshold - at the foot of a slope, on a terrace, on a flood plain. A short shelf on a steep slope does not stop the line."));
                ui.end_row();
                ui.label(tr(lang, "Flattening slope"));
                ui.add_enabled(self.on_flat, egui::DragValue::new(&mut self.flat_slope).range(0.1..=45.0).speed(0.1).fixed_decimals(1).suffix("°")).on_hover_text(tr(lang, "The slope below which the terrain is counted as flattened."));
                ui.end_row();
                ui.label(tr(lang, "Section length"));
                ui.add_enabled(self.on_flat, egui::DragValue::new(&mut self.flat_len).range(10.0..=5000.0).speed(1.0).fixed_decimals(0).suffix(tr(lang, " m"))).on_hover_text(tr(lang, "The length of the section over which the mean slope is measured. Larger - only a long flattening stops the line."));
                ui.end_row();
                if item {
                    ui.label("");
                    ui.checkbox(&mut self.merge, tr(lang, "Stop on merging")).on_hover_text(tr(lang, "Lines from the placemark end flowing into a line built earlier, and the common channel goes as one line. Without the check each line goes to the end."));
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            let hint = self.flow_site.as_ref().and_then(|s| s.hint(lang));
            ui.add(egui::Label::new(hint.unwrap_or(&self.flow_status)).wrap());
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button(tr(lang, "Flow line")).on_hover_text(tr(lang, "The path of the water from the point down the terrain - a line in «My places». Hollows are filled up to the overflow, so the line passes through lakes and closed basins. For a path or polygon - lines from all its nodes, in a folder.")).clicked() {
                    task = Some(match &self.flow_item {
                        Some((p, poly, n)) => Task::ShapeLines(p.clone(), *poly, n.clone()),
                        None => Task::Line,
                    });
                }
                if ui.add_enabled(!item, egui::Button::new(tr(lang, "Catchment"))).on_hover_text(tr(lang, "The area from which the water flows to the point and the main watercourse - a folder in «My places» with the area, slopes and the watercourse length. The catchment is built from the channel near the point.")).clicked() {
                    task = Some(Task::Basin);
                }
                if ui.button(tr(lang, "River network")).on_hover_text(tr(lang, "Watercourses of the area with the Strahler order - lines in «My places», the line width grows with the order.")).clicked() {
                    task = Some(Task::Rivers);
                }
                if ui.button(tr(lang, "Basins")).on_hover_text(tr(lang, "The area divided into basins of outlets - places where watercourses with a catchment from the area of the field «Watercourses from» leave the area. Polygons in «My places».")).clicked() {
                    task = Some(Task::Basins);
                }
            });
        });
        if let Some(t) = task {
            match Self::site_job(self.flow_site.as_ref(), lang) {
                Ok((lat, lon, half, ring)) => self.start(t, lat, lon, half, ring, radius, max_level, now),
                Err(text) => self.flow_status = text,
            }
        }
        // Окно закрыто: следующий расчёт - новая папка, `session.reset`.
        if !open {
            self.flow_open = false;
            self.flow_session = None;
        }
    }

    fn sea_window(&mut self, ctx: &egui::Context, lang: Lang) {
        if !self.sea_open {
            return;
        }
        let mut open = true;
        let before = self.sea_level;
        egui::Window::new(tr(lang, "Sea level")).id(egui::Id::unique("planetx-sea-level")).open(&mut open).collapsible(false).resizable(false).min_width(340.0).show(ctx, |ui| {
            let tip = tr(lang, "The sea level above the present one. The water floods everything below the level, below zero it opens the shelf. About -120 m - the last glacial maximum, about +70 m - the melting of all glaciers.");
            ui.horizontal(|ui| {
                ui.label(tr(lang, "Level"));
                ui.add(egui::DragValue::new(&mut self.sea_level).range(SEA_LOW..=SEA_HIGH).speed(0.5).fixed_decimals(1).suffix(tr(lang, " m"))).on_hover_text(tip);
            });
            ui.add(egui::Slider::new(&mut self.sea_level, SEA_LOW..=SEA_HIGH).show_value(false)).on_hover_text(tip);
            let text = if self.sea_level == 0.0 {
                tr(lang, "The present sea level.")
            } else {
                tr(lang, "The water floods everything below the level over the whole globe. The connection with the sea is not checked, so lowlands inside continents are flooded too. The water is seen with the terrain on.")
            };
            ui.add(egui::Label::new(text).wrap());
            if ui.button(tr(lang, "Present")).on_hover_text(tr(lang, "Return the present sea level.")).clicked() {
                self.sea_level = 0.0;
            }
        });
        if self.sea_level != before {
            ctx.request_repaint();
        }
        if !open {
            self.sea_open = false;
        }
    }
}

/// Цвет HSL, доли 0-1, оттенок в градусах.
fn hsl(h: f64, s: f64, l: f64) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [((r + m) * 255.0).round() as u8, ((g + m) * 255.0).round() as u8, ((b + m) * 255.0).round() as u8]
}

/// Картинка тайла воды затопления: цвет по глубине, `flood_tile_rgba`.
fn water_tile(w: &Water, key: TileKey) -> Option<Vec<u8>> {
    hydro::flood_tile_rgba(&w.grid, &w.flood, &w.z, w.level, key, w.radius)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    const R: f64 = 6378137.0;
    const LAT: f64 = 58.0;
    const LON: f64 = 56.25;

    /// Метры к востоку и северу от середины.
    fn local(lat: f64, lon: f64) -> (f64, f64) {
        ((lon - LON).to_radians() * R * LAT.to_radians().cos(), (lat - LAT).to_radians() * R)
    }

    /// Прогон расчёта задачи task на рельефе height до конца.
    fn run(h: &mut Hydro, task: Task, radius_m: f64, height: fn(f64, f64) -> f64) {
        let ctx = egui::Context::default();
        h.start(task, LAT, LON, radius_m, Vec::new(), R, 14, 0.0);
        let heights = |pts: &[(f64, f64)]| -> Vec<f64> {
            pts.iter().map(|(la, lo)| {
                let (x, y) = local(*la, *lo);
                height(x, y)
            }).collect()
        };
        for _ in 0..10000 {
            if !h.busy() {
                break;
            }
            let mut out = ctx.run_pass(egui::RawInput::default(), |ctx| {
                h.step(ctx, Lang::EN, 0.0, R, 14, &|_| true, &heights);
            });
            out.textures_delta.clear();
        }
        assert!(!h.busy());
    }

    /// Чаша: высота растёт от середины, 1 м на 100 м.
    fn bowl(x: f64, y: f64) -> f64 {
        x.hypot(y) * 0.01
    }

    /// Склон к югу с долинами вдоль склона через 600 м.
    fn valleys(x: f64, y: f64) -> f64 {
        100.0 + y * 0.02 + 15.0 * (x / 600.0 * std::f64::consts::TAU).cos()
    }

    /// Метки расчёта: строки KML дошли до «Моих меток».
    fn items_of(s: &Saved) -> usize {
        s.extra.len()
    }

    #[test]
    fn flood_of_a_bowl_lies_on_tiles() {
        let mut h = Hydro::new();
        h.flood_point = (LAT, LON);
        run(&mut h, Task::Flood, 2000.0, bowl);
        let water = h.water.clone().expect("вода");
        // Уровень +2 м над дном - круг около 200 м.
        let wet = water.flood.mask.iter().filter(|m| **m).count() as f64 * water.grid.cell.powi(2);
        let disk = std::f64::consts::PI * 200.0f64.powi(2);
        assert!((wet - disk).abs() < 0.25 * disk, "{wet} {disk}");
        assert!(h.flood_status.contains("Flooded"), "{}", h.flood_status);
        let n = f64::from(1u32 << 15);
        let tx = ((LON + 180.0) / 360.0 * n) as u32;
        let ty = ((1.0 - (LAT.to_radians().tan() + 1.0 / LAT.to_radians().cos()).ln() / std::f64::consts::PI) / 2.0 * n) as u32;
        let tile = water_tile(&water, (15, tx, ty)).expect("тайл с водой");
        assert!(tile.chunks(4).any(|p| p[3] > 0));
        // Подъём меняет воду без пересчёта.
        h.set_level(Lang::EN, h.flood.as_ref().unwrap().base + 5.0);
        assert!(h.water.as_ref().unwrap().flood.area > water.flood.area);
        // «OK»: глубины файлом GeoTIFF, вода картинкой, контур метками.
        h.save_flood(Lang::EN);
        let saved = h.saved.pop().expect("расчёт");
        assert_eq!(saved.layers.len(), 1);
        assert!(!saved.layers[0].tiff.is_empty() && saved.layers[0].png().is_some());
        assert!(saved.layers[0].rgba.iter().any(|p| p[3] > 0));
        assert!(items_of(&saved) > 0);
    }

    #[test]
    fn flood_stays_inside_the_area() {
        // Участок - квадрат 600 м: вода +5 м над дном чаши (круг 500 м)
        // за его край не выходит.
        let mut h = Hydro::new();
        h.flood_point = (LAT, LON);
        let ctx = egui::Context::default();
        h.start(Task::Flood, LAT, LON, 2000.0, data::site::square_ring(LAT, LON, 600.0), R, 14, 0.0);
        let heights = |pts: &[(f64, f64)]| -> Vec<f64> { pts.iter().map(|(la, lo)| { let (x, y) = local(*la, *lo); bowl(x, y) }).collect() };
        while h.busy() {
            let mut out = ctx.run_pass(egui::RawInput::default(), |ctx| {
                h.step(ctx, Lang::EN, 0.0, R, 14, &|_| true, &heights);
            });
            out.textures_delta.clear();
        }
        let base = h.flood.as_ref().unwrap().base;
        h.set_level(Lang::EN, base + 5.0);
        let water = h.water.clone().expect("вода");
        let wet = water.flood.mask.iter().filter(|m| **m).count() as f64 * water.grid.cell.powi(2);
        assert!(wet <= 600.0 * 600.0 * 1.05, "{wet}");
    }

    #[test]
    fn flow_line_catchment_rivers_basins() {
        let mut h = Hydro::new();
        h.flow_point = (LAT + 0.005, LON + 0.001);
        run(&mut h, Task::Line, 3000.0, valleys);
        assert_eq!(h.saved.len(), 1, "{}", h.flow_status);
        assert!(matches!(h.saved[0].extra.first(), Some(scene::kml::Item::Place(p)) if p.kind == scene::features::Kind::Line));
        run(&mut h, Task::Basin, 3000.0, valleys);
        assert_eq!(h.saved.len(), 2, "{}", h.flow_status);
        assert!(h.flow_status.contains("Area"));
        run(&mut h, Task::Rivers, 3000.0, valleys);
        assert_eq!(h.saved.len(), 3, "{}", h.flow_status);
        run(&mut h, Task::Basins, 3000.0, valleys);
        assert_eq!(h.saved.len(), 4, "{}", h.flow_status);
        // Одна сессия - одна папка расчёта.
        assert!(h.saved.iter().all(|s| s.session.is_some() && s.session == h.saved[0].session && items_of(s) > 0));
    }

    #[test]
    fn placemark_catchment_and_lines() {
        let mut h = Hydro::new();
        // Путь поперёк долины ниже по склону.
        let path = vec![(LAT - 0.004, LON - 0.003), (LAT - 0.004, LON + 0.003)];
        h.catchment_of(path.clone(), false, "створ".into(), R, 14, 0.0);
        let ctx = egui::Context::default();
        let heights = |pts: &[(f64, f64)]| -> Vec<f64> { pts.iter().map(|(la, lo)| { let (x, y) = local(*la, *lo); valleys(x, y) }).collect() };
        for _ in 0..10000 {
            if !h.busy() {
                break;
            }
            let mut out = ctx.run_pass(egui::RawInput::default(), |ctx| {
                h.step(ctx, Lang::EN, 0.0, R, 14, &|_| true, &heights);
            });
            out.textures_delta.clear();
        }
        let basin = h.saved.pop().expect("водосбор метки");
        assert!(matches!(basin.extra.first(), Some(scene::kml::Item::Place(p)) if p.kind == scene::features::Kind::Polygon), "{:?}", h.message);
        assert_eq!(basin.ring.len(), 4);
        h.flow_from(path, false, "отвал".into(), R);
        let item = h.flow_item.clone().unwrap();
        let side = h.flow_site.as_ref().unwrap().site.side_km * 500.0;
        run(&mut h, Task::ShapeLines(item.0, item.1, item.2), side, valleys);
        assert_eq!(h.saved.len(), 1, "{}", h.flow_status);
        assert!(items_of(&h.saved[0]) > 0);
    }
    #[test]
    fn hsl_primaries() {
        assert_eq!(hsl(0.0, 1.0, 0.5), [255, 0, 0]);
        assert_eq!(hsl(120.0, 1.0, 0.5), [0, 255, 0]);
        assert_eq!(hsl(240.0, 1.0, 0.5), [0, 0, 255]);
    }
}
