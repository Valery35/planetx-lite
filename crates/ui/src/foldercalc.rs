// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окна папки «Моих меток»: «Выдавливание» и «Батиметрия по промерам»,
//! перенос `ui/extrude.py` и `ui/sounding.py` модуля. У модуля источник -
//! слой проекта QGIS, у Lite слои GeoPackage и KML лежат папками меток,
//! поля - ExtendedData.
//!
//! Выдавливание многоугольника и линии - высота метки и флаг extrude, их
//! рисует [`crate::marks::mesh`], настройка живёт в метках и файлах KML.
//! Точка становится восьмиугольником столбика, `extrude.column` модуля.
//!
//! Батиметрия: точки папки переводятся в зону UTM середины, сетку строит
//! `data::sounding`, файл глубин открывается своим рельефом дном водоёма.

use data::sounding;
use eframe::egui;
use scene::features::Kind;
use scene::kml::{Folder, Item, Place};

use crate::i18n::{Lang, tr, trf};

/// Сторон столбика точки.
const SIDES: usize = 8;
/// Радиус столбика - доля поперечника папки, в пределах метров.
const COLUMN_SHARE: f64 = 1.0 / 150.0;
const COLUMN_RANGE: (f64, f64) = (2.0, 2000.0);
/// Радиус столбика папки из одной точки, м.
const COLUMN_LONE: f64 = 10.0;
/// Строк грида за кадр.
const ROWS: usize = 32;

/// Метки папки с вложенными, для чтения.
fn places(folder: &Folder) -> Vec<&Place> {
    let mut out = Vec::new();
    for item in &folder.children {
        match item {
            Item::Place(p) => out.push(p),
            Item::Folder(f) => out.extend(places(f)),
            _ => {}
        }
    }
    out
}

fn value_of(place: &Place, field: &str) -> Option<f64> {
    place.data.as_ref()?.iter().find(|(k, _)| k == field).and_then(|(_, v)| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite())
}

/// Числовые поля меток папки: у которых есть хоть одно число.
pub fn numeric_fields(folder: &Folder) -> Vec<String> {
    let list = places(folder);
    let mut out: Vec<String> = Vec::new();
    for p in &list {
        for (k, _) in p.data.iter().flatten() {
            if !out.contains(k) && value_of(p, k).is_some() {
                out.push(k.clone());
            }
        }
    }
    out
}

/// Высота объекта в метрах, None - пусто, не число или не выше нуля.
pub fn feature_height(value: Option<f64>, factor: f64) -> Option<f64> {
    value.map(|v| v * factor).filter(|h| h.is_finite() && *h > 0.0)
}

/// Контур столбика точки: широты и долготы, `column` модуля.
pub fn column(lat: f64, lon: f64, radius_m: f64, radius: f64) -> Vec<(f64, f64)> {
    let dlat = (radius_m / radius).to_degrees();
    let dlon = dlat / lat.to_radians().cos().max(1e-6);
    (0..SIDES)
        .map(|i| {
            let angle = i as f64 * std::f64::consts::TAU / SIDES as f64;
            (lat + dlat * angle.sin(), lon + dlon * angle.cos())
        })
        .collect()
}

/// Радиус столбика по поперечнику точек папки, метры.
pub fn column_radius(points: &[(f64, f64)]) -> f64 {
    let (lat0, lat1) = points.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.0), b.max(p.0)));
    let (lon0, lon1) = points.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.1), b.max(p.1)));
    let height = (lat1 - lat0) * data::site::METERS_PER_DEGREE;
    let width = (lon1 - lon0) * data::site::METERS_PER_DEGREE * ((lat0 + lat1) / 2.0).to_radians().cos();
    let across = height.hypot(width);
    if points.len() < 2 || across <= 0.0 { COLUMN_LONE } else { (across * COLUMN_SHARE).clamp(COLUMN_RANGE.0, COLUMN_RANGE.1) }
}

/// Метка на высоту height: точка становится столбиком.
fn raise(place: &mut Place, height: f64, column_m: f64, radius: f64) {
    if place.kind == Kind::Point
        && let Some(&(lat, lon)) = place.points.first()
    {
        place.kind = Kind::Polygon;
        place.points = column(lat, lon, column_m, radius);
        if place.fill.is_none() {
            place.fill = Some(place.color);
        }
    }
    place.height = height;
    place.extrude = true;
    place.mode = "relativeToGround".to_string();
}

/// Выдавить метки папки по полю. Возвращает количество выдавленных.
pub fn extrude_folder(folder: &mut Folder, field: &str, factor: f64, radius: f64) -> usize {
    let points: Vec<(f64, f64)> = places(folder).iter().filter(|p| p.kind == Kind::Point).filter_map(|p| p.points.first().copied()).collect();
    let column_m = column_radius(&points);
    extrude_items(folder, field, factor, column_m, radius)
}

fn extrude_items(folder: &mut Folder, field: &str, factor: f64, column_m: f64, radius: f64) -> usize {
    let mut count = 0;
    for item in &mut folder.children {
        match item {
            Item::Place(p) => {
                if let Some(height) = feature_height(value_of(p, field), factor) {
                    raise(p, height, column_m, radius);
                    count += 1;
                }
            }
            Item::Folder(f) => count += extrude_items(f, field, factor, column_m, radius),
            _ => {}
        }
    }
    count
}

/// Окно «Выдавливание» папки path.
pub struct ExtrudeWindow {
    pub path: Vec<usize>,
    name: String,
    fields: Vec<String>,
    field: String,
    factor: f64,
}

impl ExtrudeWindow {
    pub fn new(path: Vec<usize>, folder: &Folder) -> Self {
        let fields = numeric_fields(folder);
        let field = fields.first().cloned().unwrap_or_default();
        Self { path, name: folder.name.clone(), fields, field, factor: 1.0 }
    }

    /// Окно: Some(Some(поле, множитель)) - «OK», Some(None) - закрыто.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang) -> Option<Option<(String, f64)>> {
        let mut open = true;
        let mut answer = None;
        egui::Window::new(trf(lang, "Extrusion - {name}", &[("name", &self.name)])).id(egui::Id::unique("planetx-extrude")).open(&mut open).collapsible(false).resizable(false).min_width(360.0).show(ctx, |ui| {
            ui.add(egui::Label::new(tr(lang, "Objects of the folder rise above the terrain to the height from a field: a polygon as a prism, a point as a column, a line as a wall.")).wrap());
            ui.add_space(4.0);
            egui::Grid::new("planetx-extrude-form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Height from a field"));
                egui::ComboBox::from_id_salt("planetx-extrude-field").selected_text(self.field.as_str()).show_ui(ui, |ui| {
                    for f in &self.fields {
                        ui.selectable_value(&mut self.field, f.clone(), f.as_str());
                    }
                }).response.on_hover_text(tr(lang, "A numeric field of the height of an object. An object with an empty, zero or negative value is not extruded."));
                ui.end_row();
                ui.label(tr(lang, "Factor, m"));
                ui.add(egui::DragValue::new(&mut self.factor).range(0.001..=100000.0).speed(0.01).fixed_decimals(3)).on_hover_text(tr(lang, "The height in metres is the field value times the factor. For a field of storeys the factor is about 3."));
                ui.end_row();
            });
            ui.add_space(4.0);
            if self.fields.is_empty() {
                ui.label(tr(lang, "The folder has no numeric fields."));
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(!self.fields.is_empty(), egui::Button::new(tr(lang, "OK"))).clicked() {
                    answer = Some(Some((self.field.clone(), self.factor)));
                }
                if ui.button(tr(lang, "Cancel")).clicked() {
                    answer = Some(None);
                }
            });
        });
        if open { answer } else { Some(None) }
    }
}

/// Что брать глубиной.
const Z_FIELD: &str = "";

/// Точки папки (широта, долгота, глубина) по полю или Z.
pub fn folder_soundings(folder: &Folder, field: &str) -> Vec<(f64, f64, f64)> {
    let mut out = Vec::new();
    for p in places(folder) {
        if p.kind != Kind::Point {
            continue;
        }
        let Some(&(lat, lon)) = p.points.first() else {
            continue;
        };
        let depth = if field == Z_FIELD { p.alts.as_ref().and_then(|a| a.first().copied()).or(Some(p.height).filter(|h| *h != 0.0)) } else { value_of(p, field) };
        if let Some(depth) = depth {
            out.push((lat, lon, depth));
        }
    }
    out
}

/// Промеры в метрах зоны UTM.
pub struct Metric {
    pub xs: Vec<f64>,
    pub ys: Vec<f64>,
    pub zs: Vec<f64>,
    pub epsg: u16,
}

/// Промеры в метрах зоны UTM середины. None - точек нет или зона
/// не считается.
pub fn to_metric(points: &[(f64, f64, f64)]) -> Option<Metric> {
    let middle: Vec<(f64, f64)> = points.first().map(|p| (p.0, p.1)).into_iter().collect();
    let epsg = u16::try_from(data::sentinel::modis::utm_epsg(&middle)?).ok()?;
    let lats: Vec<f64> = points.iter().map(|p| p.0).collect();
    let lons: Vec<f64> = points.iter().map(|p| p.1).collect();
    let (xs, ys) = data::grid::to_projected(u32::from(epsg), &lats, &lons)?;
    let zs: Vec<f64> = points.iter().map(|p| p.2).collect();
    let (xs, ys, zs) = sounding::clean(&xs, &ys, &zs);
    Some(Metric { xs, ys, zs, epsg })
}

/// Окно «Батиметрия по промерам» папки.
pub struct SoundingWindow {
    name: String,
    fields: Vec<String>,
    field: String,
    points: Vec<(f64, f64, f64)>,
    metric: Option<Metric>,
    folder: Folder,
    cell: f64,
    limit: f64,
    level: f64,
    status: String,
    job: Option<sounding::Build>,
    /// Готовый файл GeoTIFF глубин, имя и уровень воды.
    pub done: Option<(Vec<u8>, String, f64)>,
}

impl SoundingWindow {
    pub fn new(folder: &Folder, level: f64, lang: Lang) -> Self {
        let mut fields = numeric_fields(folder);
        if !folder_soundings(folder, Z_FIELD).is_empty() {
            fields.insert(0, Z_FIELD.to_string());
        }
        let field = fields.first().cloned().unwrap_or_default();
        let mut w = Self { name: folder.name.clone(), fields, field, points: Vec::new(), metric: None, folder: folder.clone(), cell: 1.0, limit: 10.0, level, status: String::new(), job: None, done: None };
        w.read(lang);
        w
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Точки по выбранному полю, шаг и предел по умолчанию.
    fn read(&mut self, lang: Lang) {
        self.points = folder_soundings(&self.folder, &self.field);
        self.metric = to_metric(&self.points);
        let Some(m) = self.metric.as_ref().filter(|m| m.xs.len() >= 3) else {
            self.status = tr(lang, "Fewer than three soundings with a depth.").to_string();
            self.metric = None;
            return;
        };
        self.cell = sounding::auto_cell(&m.xs, &m.ys);
        self.limit = self.cell * sounding::MAX_GAP_CELLS;
        let (low, high) = m.zs.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(*v), b.max(*v)));
        self.status = trf(lang, "Soundings {count}, depth from {low} to {high} m.", &[("count", &m.xs.len().to_string()), ("low", &format!("{low:.1}")), ("high", &format!("{high:.1}"))]);
    }

    /// Окно и шаг расчёта. Ложь - окно закрыто.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang) -> bool {
        let mut open = true;
        let mut run = false;
        let before = self.field.clone();
        let label = |f: &str| if f == Z_FIELD { tr(lang, "Point Z").to_string() } else { f.to_string() };
        egui::Window::new(tr(lang, "Bathymetry from soundings")).id(egui::Id::unique("planetx-sounding")).open(&mut open).collapsible(false).resizable(false).min_width(380.0).show(ctx, |ui| {
            ui.add(egui::Label::new(trf(lang, "The points of the folder «{name}» are depth soundings, for example echo sounder tracks. A raster of depths is built from them by triangulation, it goes into the globe terrain as the bed of the water body.", &[("name", &self.name)])).wrap());
            ui.add_space(4.0);
            egui::Grid::new("planetx-sounding-form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Depth"));
                egui::ComboBox::from_id_salt("planetx-sounding-field").selected_text(label(&self.field)).show_ui(ui, |ui| {
                    for f in &self.fields {
                        ui.selectable_value(&mut self.field, f.clone(), label(f));
                    }
                }).response.on_hover_text(tr(lang, "The field with the depth or the Z of a point. The sign of the depth does not matter, its absolute value is taken."));
                ui.end_row();
                ui.label(tr(lang, "Cell"));
                ui.add(egui::DragValue::new(&mut self.cell).range(sounding::MIN_CELL..=1000.0).speed(0.1).fixed_decimals(1).suffix(tr(lang, " m"))).on_hover_text(tr(lang, "The cell size of the depth raster. By default - by the number of points per area of the soundings. A smaller cell gives a more detailed bed and a larger file."));
                ui.end_row();
                ui.label(tr(lang, "Limit from a sounding"));
                ui.add(egui::DragValue::new(&mut self.limit).range(1.0..=100000.0).speed(1.0).fixed_decimals(0).suffix(tr(lang, " m"))).on_hover_text(tr(lang, "A cell further than this from the nearest sounding stays empty. So the raster does not flood the shore and bays without soundings. The limit is larger than the distance between tracks, otherwise strips stay between them."));
                ui.end_row();
                ui.label(tr(lang, "Water level"));
                ui.add(egui::DragValue::new(&mut self.level).range(-500.0..=9000.0).speed(0.1).fixed_decimals(2).suffix(tr(lang, " m"))).on_hover_text(tr(lang, "The height of the water surface above sea level on the day of the soundings. The bed elevation is the water level minus the depth."));
                ui.end_row();
            });
            ui.add_space(4.0);
            ui.add(egui::Label::new(&self.status).wrap());
            ui.add_space(4.0);
            run = ui.add_enabled(self.metric.is_some() && self.job.is_none(), egui::Button::new(tr(lang, "Build"))).clicked();
        });
        if self.field != before {
            self.read(lang);
        }
        if run {
            self.start(lang);
        }
        self.step(lang);
        if self.job.is_some() {
            ctx.request_repaint();
        }
        open && self.done.is_none()
    }

    fn start(&mut self, lang: Lang) {
        let Some(m) = &self.metric else {
            return;
        };
        match sounding::Build::new(&m.xs, &m.ys, &m.zs, self.cell, self.limit) {
            Some(job) => self.job = Some(job),
            None => self.status = tr(lang, "The soundings do not form a grid: they lie on one line or the grid is too large.").to_string(),
        }
    }

    fn step(&mut self, lang: Lang) {
        let Some(job) = &mut self.job else {
            return;
        };
        if !job.step(ROWS) {
            self.status = trf(lang, "Computing: {done} %.", &[("done", &format!("{:.0}", 100.0 * job.done()))]);
            return;
        }
        let (Some(job), Some(m)) = (self.job.take(), &self.metric) else {
            return;
        };
        if job.known() == 0 {
            self.status = tr(lang, "The grid has no cells with a depth, increase the limit.").to_string();
            return;
        }
        let grid = data::geotiff::MercatorGrid { x0: job.west + job.cell / 2.0, y0: job.north - job.cell / 2.0, step: job.cell, cols: job.cols, rows: job.rows };
        match data::geotiff::write_float_projected(&grid, m.epsg, &job.depths) {
            Some(tiff) => self.done = Some((tiff, format!("{}.tif", self.name), self.level)),
            None => self.status = tr(lang, "The raster is not written.").to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    const R: f64 = 6378137.0;

    fn point(lat: f64, lon: f64, field: &str, value: &str) -> Item {
        let mut p = Place::new("p", Kind::Point, vec![(lat, lon)]);
        p.data = Some(vec![(field.to_string(), value.to_string())]);
        Item::Place(p)
    }

    #[test]
    fn height_needs_a_positive_number() {
        assert_eq!(feature_height(Some(3.0), 2.0), Some(6.0));
        assert_eq!(feature_height(Some(0.0), 2.0), None);
        assert_eq!(feature_height(Some(-1.0), 2.0), None);
        assert_eq!(feature_height(None, 2.0), None);
    }

    #[test]
    fn column_has_eight_sides_of_given_radius() {
        let ring = column(0.0, 0.0, 100.0, R);
        assert_eq!(ring.len(), SIDES);
        let reach = (ring[0].1.to_radians() * R - 100.0).abs();
        assert!(reach < 1e-6, "{reach}");
    }

    #[test]
    fn extrusion_makes_columns_and_prisms() {
        let mut folder = Folder::new("слой");
        folder.children.push(point(58.0, 56.0, "h", "12"));
        folder.children.push(point(58.1, 56.1, "h", "нет"));
        let mut poly = Place::new("дом", Kind::Polygon, vec![(58.0, 56.0), (58.0, 56.1), (58.1, 56.1)]);
        poly.data = Some(vec![("h".to_string(), "5".to_string())]);
        folder.children.push(Item::Place(poly));
        assert_eq!(numeric_fields(&folder), ["h"]);
        assert_eq!(extrude_folder(&mut folder, "h", 3.0, R), 2);
        let list = places(&folder);
        assert_eq!(list[0].kind, Kind::Polygon);
        assert_eq!(list[0].points.len(), SIDES);
        assert_eq!(list[0].height, 36.0);
        assert!(list[0].extrude);
        assert_eq!(list[1].kind, Kind::Point);
        assert_eq!(list[2].height, 15.0);
    }

    #[cfg(feature = "import")]
    #[test]
    fn soundings_of_a_folder_become_a_raster() {
        let mut folder = Folder::new("галсы");
        for r in 0..20 {
            for c in 0..20 {
                folder.children.push(point(58.0 + f64::from(r) * 0.0001, 56.0 + f64::from(c) * 0.0002, "depth", &format!("{}", 1 + r + c)));
            }
        }
        let mut w = SoundingWindow::new(&folder, 100.0, Lang::EN);
        assert!(w.metric.is_some(), "{}", w.status);
        assert!(w.status.contains("Soundings 400"), "{}", w.status);
        w.start(Lang::EN);
        while w.busy() {
            w.step(Lang::EN);
        }
        let (tiff, name, level) = w.done.take().expect("файл");
        assert_eq!(name, "галсы.tif");
        assert_eq!(level, 100.0);
        let grid = data::grid::Grid::read(tiff, "a.tif").expect("читается");
        let (low, high, _) = grid.stats().expect("значения");
        assert!(low >= 0.5 && high <= 40.0, "{low} {high}");
    }
}
