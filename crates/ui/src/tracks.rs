// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Растущие треки из точечных слоёв GeoPackage, перенос `ui/track.py`
//! модуля. Расчёт пути - `data::track`.
//!
//! Трек - точечный слой с полем времени и, если объектов несколько,
//! полем объекта. У модуля слой и его трек задаёт проект QGIS, у Lite
//! слой с полем времени становится треком при открытии файла: поле
//! времени и поле объекта угадываются по названию и значениям, окно
//! «Треки» их меняет. Точки такого слоя в «Мои метки» не идут.
//!
//! Время задаёт шкала времени: к правому краю промежутка у объекта
//! растёт пройденный путь, в текущей точке стоит точка с названием
//! объекта. Шкала закрыта - треки целиком. Модель по треку - пакет
//! модели и строки пути на каждый объект, она едет тем же путём, что
//! модели gx:Track (`crate::models::Mover`), и камера идёт за ней
//! пунктом меню на глобусе. Высота точки - из числового поля, Z точек
//! Lite пока не читает. Камеры следом за первым объектом без модели,
//! как флажок окна «Трек» модуля, у Lite нет.

use std::collections::HashSet;
use std::sync::Arc;

use data::layers::{LayerData, Shape as LayerShape};
use data::track::{Item, Object, Track};
use eframe::egui;
use scene::features::{Kind, Shape};
use scene::kml::Overlay;
use scene::overlays::{self, TrackRow};

use crate::i18n::{Lang, tr, trf};

/// Цвет пройденного пути, `TRACK_COLOR` модуля.
pub const TRACK_COLOR: [u8; 4] = [255, 80, 40, 255];
/// Цвет путей демо самосвалов, `HAUL_COLOR` модуля.
pub const HAUL_COLOR: [u8; 4] = [250, 190, 30, 255];
/// Толщина пути, `TRACK_WIDTH` модуля.
pub const TRACK_WIDTH: f64 = 3.0;
/// Метров в футе, `FOOT` модуля.
pub const FOOT: f64 = 0.3048;
/// Доля точек слоя со значением времени, с которой поле годится в поле
/// времени.
pub const TIME_SHARE: f64 = 0.5;
/// Наибольшее количество объектов, с которым поле годится в поле
/// объекта.
pub const OBJECTS_MAX: usize = 100;
/// Файл модели демо самосвалов в папке демо.
pub const DEMO_MODEL: &str = "haul_truck.glb";

/// Названия полей времени, без учёта регистра.
const TIME_NAMES: [&str; 8] = ["time", "datetime", "date_time", "timestamp", "date", "when", "время", "дата"];
/// Названия полей объекта, без учёта регистра.
const OBJECT_NAMES: [&str; 12] = ["object", "truck", "vehicle", "unit", "id", "track", "callsign", "icao24", "mmsi", "name", "объект", "машина"];

/// Модель трека: имя файла и пакет модели.
#[derive(Clone)]
pub struct Model {
    pub name: String,
    pub package: Arc<Vec<u8>>,
}

/// Настройки трека слоя, как окно «Трек» модуля. Поля - номера полей
/// слоя.
#[derive(Clone)]
pub struct Settings {
    pub time: usize,
    /// None - один объект.
    pub object: Option<usize>,
    pub color: [u8; 4],
    /// Числовое поле высоты, None - по земле.
    pub height: Option<usize>,
    /// Высота над землёй, иначе над уровнем моря.
    pub relative: bool,
    /// Метров в единице высоты.
    pub unit: f64,
    pub model: Option<Model>,
    pub model_scale: f64,
    /// Поворот модели по часовой стрелке к ходу движения, градусы.
    pub model_turn: f64,
    pub shown: bool,
}

/// Слой-трек: точки слоя, настройки и трек по ним.
pub struct Layer {
    /// «файл · слой».
    pub title: String,
    pub fields: Vec<String>,
    pub numeric: Vec<bool>,
    /// Точки: широта, долгота, значения полей текстом.
    points: Vec<(f64, f64, Vec<String>)>,
    pub settings: Settings,
    pub track: Track,
}

/// Время значения в секундах от 1970 года или None. Число - секунды,
/// текст - дата и время ISO 8601 (`scene::when::parse`).
pub fn seconds(text: &str, numeric: bool) -> Option<f64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if numeric {
        return t.parse::<f64>().ok().filter(|v| v.is_finite());
    }
    scene::when::parse(t)
}

/// Доля точек, у которых значение поля n - время.
fn time_share(points: &[(f64, f64, Vec<String>)], n: usize, numeric: bool) -> f64 {
    if points.is_empty() {
        return 0.0;
    }
    let good = points.iter().filter(|(_, _, v)| v.get(n).and_then(|t| seconds(t, numeric)).is_some()).count();
    good as f64 / points.len() as f64
}

/// Поле времени слоя: сначала по названию, потом текстовое поле
/// с датами у [`TIME_SHARE`] точек. Числовое поле годится только
/// по названию.
pub fn time_field(fields: &[String], numeric: &[bool], points: &[(f64, f64, Vec<String>)]) -> Option<usize> {
    let is_numeric = |n: usize| numeric.get(n).copied().unwrap_or(false);
    let named = (0..fields.len()).filter(|n| TIME_NAMES.contains(&fields[*n].to_lowercase().as_str()));
    let texts = (0..fields.len()).filter(|n| !is_numeric(*n));
    named.chain(texts).find(|n| time_share(points, *n, is_numeric(*n)) >= TIME_SHARE)
}

/// Поле объекта: от 2 до [`OBJECTS_MAX`] разных значений и не больше
/// половины количества точек. Сначала по названию, потом текстовое
/// поле. None - один объект.
pub fn object_field(fields: &[String], numeric: &[bool], points: &[(f64, f64, Vec<String>)], time: usize) -> Option<usize> {
    let fits = |n: usize| {
        let mut seen: HashSet<&str> = HashSet::new();
        for (_, _, v) in points {
            if let Some(value) = v.get(n) {
                seen.insert(value.as_str());
                if seen.len() > OBJECTS_MAX {
                    return false;
                }
            }
        }
        seen.len() >= 2 && seen.len() * 2 <= points.len()
    };
    let others = || (0..fields.len()).filter(move |n| *n != time);
    let named = others().filter(|n| OBJECT_NAMES.contains(&fields[*n].to_lowercase().as_str()));
    let texts = others().filter(|n| !numeric.get(*n).copied().unwrap_or(false));
    named.chain(texts).find(|n| fits(*n))
}

impl Layer {
    /// Слой-трек из точечного слоя или None, если поля времени нет.
    pub fn from_layer(layer: &LayerData, file: &str) -> Option<Self> {
        if layer.items.is_empty() || layer.items.iter().any(|i| i.shape != LayerShape::Point) {
            return None;
        }
        let points: Vec<(f64, f64, Vec<String>)> = layer
            .items
            .iter()
            .flat_map(|i| i.parts.iter().filter_map(|p| p.first()).map(|(lat, lon)| (*lat, *lon, i.values.clone())))
            .collect();
        let time = time_field(&layer.fields, &layer.numeric, &points)?;
        let object = object_field(&layer.fields, &layer.numeric, &points, time);
        let settings = Settings {
            time,
            object,
            color: TRACK_COLOR,
            height: None,
            relative: false,
            unit: 1.0,
            model: None,
            model_scale: 1.0,
            model_turn: 0.0,
            shown: true,
        };
        let mut out = Self {
            title: format!("{file} · {}", layer.name),
            fields: layer.fields.clone(),
            numeric: layer.numeric.clone(),
            points,
            settings,
            track: Track::new(Vec::new()),
        };
        out.build();
        Some(out)
    }

    /// Трек заново по настройкам, `read_track` модуля. Точки без времени
    /// пропускаются.
    pub fn build(&mut self) {
        let s = &self.settings;
        let time_numeric = self.numeric.get(s.time).copied().unwrap_or(false);
        let name = self.title.clone();
        let items: Vec<Item> = self
            .points
            .iter()
            .filter_map(|(lat, lon, values)| {
                let t = seconds(values.get(s.time)?, time_numeric)?;
                let key = s.object.and_then(|o| values.get(o)).filter(|v| !v.is_empty()).cloned().unwrap_or_else(|| name.clone());
                let item = Item::new(key, t, *lat, *lon);
                let alt = s.height.and_then(|h| values.get(h)).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite());
                Some(match alt {
                    Some(a) => item.with_alt(a * s.unit),
                    None => item,
                })
            })
            .collect();
        self.track = Track::new(items);
    }

    /// Количество точек слоя.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Точки слоя (широта, долгота).
    pub fn places(&self) -> Vec<(f64, f64)> {
        self.points.iter().map(|(lat, lon, _)| (*lat, *lon)).collect()
    }
}

/// Объект трека на момент: название, место, высота в метрах (NaN - по
/// земле), высота над землёй, цвет.
#[derive(Debug, Clone, PartialEq)]
pub struct Dot {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub alt: f64,
    pub relative: bool,
    pub color: [u8; 4],
}

/// Строки пути модели объекта: азимут - на следующую несовпадающую
/// точку плюс поворот модели, у последней - прежний. Высота - над
/// землёй или над уровнем моря по настройке, без высоты - 0.
pub fn rows_of(object: &Object, turn: f64) -> Vec<TrackRow> {
    let n = object.times.len().min(object.lats.len()).min(object.lons.len());
    let mut rows = Vec::with_capacity(n);
    let mut heading = 0.0;
    for i in 0..n {
        let (lat, lon) = (object.lats[i], object.lons[i]);
        if let Some(j) = (i + 1..n).find(|j| object.lats[*j] != lat || object.lons[*j] != lon) {
            heading = data::trains::bearing(lat, lon, object.lats[j], object.lons[j]);
        }
        let alt = object.alts.get(i).copied().filter(|a| a.is_finite()).unwrap_or(0.0);
        rows.push(TrackRow { t: object.times[i], lat, lon, alt, heading: Some((heading + turn).rem_euclid(360.0)), tilt: 0.0, roll: 0.0 });
    }
    rows
}

/// Треки всех слоёв, окно «Треки» и состояние на момент шкалы.
#[derive(Default)]
pub struct Tracks {
    pub layers: Vec<Layer>,
    pub open: bool,
    /// Момент, на который посчитаны пути и точки: Some(None) - треки
    /// целиком, None - ещё не посчитаны.
    built: Option<Option<f64>>,
    shapes: Vec<Shape>,
    dots: Vec<Dot>,
    /// Пути сменились, сетки надо собрать заново.
    pub dirty: bool,
    /// Модели или пути моделей сменились.
    pub models_dirty: bool,
    /// Модель демо самосвалов, ждёт слой демо.
    pub demo_model: Option<Model>,
    /// Окно просит выбрать файл модели слоя с этим номером.
    pub pick_model: Option<usize>,
}

impl Tracks {
    /// Слои-треки файла из его слоёв и номера оставшихся слоёв.
    pub fn split(layers: &[LayerData], file: &str) -> (Vec<Layer>, Vec<usize>) {
        let mut found = Vec::new();
        let mut rest = Vec::new();
        for (n, layer) in layers.iter().enumerate() {
            match Layer::from_layer(layer, file) {
                Some(t) => found.push(t),
                None => rest.push(n),
            }
        }
        (found, rest)
    }

    /// Добавить слои-треки. Файл демо самосвалов получает модель
    /// и цвет демо, высота - над землёй, как `_demo_trucks` модуля.
    pub fn add(&mut self, mut layers: Vec<Layer>, file: &str) {
        if file.starts_with("haul_")
            && let Some(model) = self.demo_model.take()
        {
            for l in &mut layers {
                l.settings.model = Some(model.clone());
                l.settings.color = HAUL_COLOR;
                l.settings.relative = true;
            }
        }
        // Тот же слой второй раз заменяет прежний.
        self.layers.retain(|old| !layers.iter().any(|l| l.title == old.title));
        self.layers.extend(layers);
        self.changed();
    }

    /// Убрать все треки: открыта другая сцена.
    pub fn clear(&mut self) {
        if !self.layers.is_empty() {
            self.layers.clear();
            self.changed();
        }
    }

    /// Пути и модели надо собрать заново.
    fn changed(&mut self) {
        self.built = None;
        self.models_dirty = true;
    }

    /// Охват времени видимых треков в секундах или None.
    pub fn span(&self) -> Option<(f64, f64)> {
        let shown = self.layers.iter().filter(|l| l.settings.shown);
        let lo = shown.clone().filter_map(|l| l.track.start).reduce(f64::min)?;
        let hi = shown.filter_map(|l| l.track.end).reduce(f64::max)?;
        Some((lo, hi))
    }

    /// Пути и точки на момент moment, None - треки целиком. Тот же
    /// момент второй раз ничего не считает, `set_time` модуля.
    pub fn step(&mut self, moment: Option<f64>) {
        if self.built == Some(moment) {
            return;
        }
        self.built = Some(moment);
        self.dirty = true;
        self.shapes.clear();
        self.dots.clear();
        for layer in self.layers.iter().filter(|l| l.settings.shown) {
            let s = &layer.settings;
            for p in layer.track.at(moment) {
                // Над уровнем моря путь лежит в воздухе на своих высотах,
                // над землёй - на рельефе, как `_build` модуля.
                let flying = !s.relative && p.alts.as_ref().is_some_and(|a| a.iter().all(|v| v.is_finite()));
                if p.path.len() >= 2 {
                    let mut line = Shape::new(Kind::Line, p.path.clone());
                    line.color = s.color;
                    line.width = TRACK_WIDTH;
                    line.name = p.key.clone();
                    if flying {
                        line.alts = p.alts.clone();
                        line.mode = "absolute".into();
                    }
                    self.shapes.push(line);
                }
                self.dots.push(Dot { name: p.key, lat: p.lat, lon: p.lon, alt: p.alt, relative: s.relative, color: s.color });
            }
        }
    }

    /// Пути объектов на момент для глобуса.
    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }

    /// Объекты на момент. У объекта с моделью точки нет, место
    /// показывает модель.
    pub fn dots(&self) -> Vec<&Dot> {
        let modelled: HashSet<String> =
            self.layers.iter().filter(|l| l.settings.shown && l.settings.model.is_some()).flat_map(|l| l.track.objects.iter().map(|o| o.key.clone())).collect();
        self.dots.iter().filter(|d| !modelled.contains(&d.name)).collect()
    }

    /// Названия объектов на момент для подписей, и у объектов с моделью.
    pub fn labels(&self) -> &[Dot] {
        &self.dots
    }

    /// Наложения-модели объектов треков с моделью: модель едет по
    /// строкам пути, как модель gx:Track.
    pub fn overlays(&self) -> Vec<Overlay> {
        let mut out = Vec::new();
        for layer in self.layers.iter().filter(|l| l.settings.shown) {
            let s = &layer.settings;
            let Some(model) = &s.model else {
                continue;
            };
            let mode = match (s.height.is_some(), s.relative) {
                (false, _) => "clampToGround",
                (true, true) => "relativeToGround",
                (true, false) => "absolute",
            };
            for object in &layer.track.objects {
                let rows = rows_of(object, s.model_turn);
                let Some(first) = rows.first() else {
                    continue;
                };
                let mut o = overlays::Overlay::new(overlays::Kind::Model);
                o.camera = Some([first.lat, first.lon, first.alt, 0.0, 0.0, 0.0]);
                o.scale = [s.model_scale; 3];
                o.mode = mode.into();
                o.track = Some(rows);
                let mut item = Overlay::new(&object.key, o);
                item.href = format!("track:{}", model.name);
                item.image = Some(model.package.to_vec());
                out.push(item);
            }
        }
        out
    }

    /// Окно «Треки»: настройки каждого слоя, как окно «Трек» модуля.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang) {
        if !self.open {
            return;
        }
        let mut open = true;
        let mut changed: Vec<usize> = Vec::new();
        let mut remove: Option<usize> = None;
        egui::Window::new(tr(lang, "Tracks")).open(&mut open).default_width(420.0).show(ctx, |ui| {
            if self.layers.is_empty() {
                ui.label(tr(lang, "No tracks. A track is a point layer of a GeoPackage file with a date and time field. Open such a file, and the layer becomes a track."));
                return;
            }
            egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
                for (n, layer) in self.layers.iter_mut().enumerate() {
                    let objects = layer.track.objects.len().to_string();
                    let points = layer.len().to_string();
                    egui::CollapsingHeader::new(&layer.title).id_salt(("planetx-track", n)).default_open(true).show(ui, |ui| {
                        if layer_ui(ui, lang, n, layer) {
                            changed.push(n);
                        }
                        ui.label(trf(lang, "Objects {objects}, points {points}.", &[("objects", &objects), ("points", &points)]));
                        ui.horizontal(|ui| {
                            if ui.button(tr(lang, "3D model…")).on_hover_text(tr(lang, "A glTF, Collada or OBJ model file. The model drives along the path of each object, its front looks along the motion.")).clicked() {
                                self.pick_model = Some(n);
                            }
                            if layer.settings.model.is_some() && ui.button(tr(lang, "Without the model")).clicked() {
                                layer.settings.model = None;
                                changed.push(n);
                            }
                            if ui.button(tr(lang, "Remove track")).on_hover_text(tr(lang, "The layer is no longer shown as a track.")).clicked() {
                                remove = Some(n);
                            }
                        });
                    });
                }
            });
        });
        let rebuild = !changed.is_empty();
        for n in changed {
            if let Some(layer) = self.layers.get_mut(n) {
                layer.build();
            }
        }
        if let Some(n) = remove.filter(|n| *n < self.layers.len()) {
            self.layers.remove(n);
        }
        if rebuild || remove.is_some() {
            self.changed();
        }
        if !open {
            self.open = false;
        }
    }

    /// Модель слоя n из пакета.
    pub fn set_model(&mut self, n: usize, model: Model) {
        if let Some(layer) = self.layers.get_mut(n) {
            layer.settings.model = Some(model);
            self.changed();
        }
    }
}

/// Поля настроек слоя. true - настройки сменились.
fn layer_ui(ui: &mut egui::Ui, lang: Lang, n: usize, layer: &mut Layer) -> bool {
    let before = (layer.settings.time, layer.settings.object, layer.settings.color, layer.settings.height, layer.settings.relative, layer.settings.unit, layer.settings.model_scale, layer.settings.model_turn, layer.settings.shown);
    let s = &mut layer.settings;
    let fields = &layer.fields;
    let name_of = |i: Option<usize>, none: &str| i.and_then(|i| fields.get(i)).cloned().unwrap_or_else(|| none.to_string());
    egui::Grid::new(("planetx-track-grid", n)).num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label(tr(lang, "Time"));
        egui::ComboBox::from_id_salt(("planetx-track-time", n)).selected_text(name_of(Some(s.time), "")).show_ui(ui, |ui| {
            for (i, f) in fields.iter().enumerate() {
                ui.selectable_value(&mut s.time, i, f);
            }
        })
        .response
        .on_hover_text(tr(lang, "The field with the date and time of the point. Date and time fields, ISO 8601 text and a number of seconds since 1970 fit."));
        ui.end_row();
        let one = tr(lang, "One object");
        ui.label(tr(lang, "Object"));
        egui::ComboBox::from_id_salt(("planetx-track-object", n)).selected_text(name_of(s.object, one)).show_ui(ui, |ui| {
            ui.selectable_value(&mut s.object, None, one);
            for (i, f) in fields.iter().enumerate() {
                ui.selectable_value(&mut s.object, Some(i), f);
            }
        })
        .response
        .on_hover_text(tr(lang, "The field that tells the objects apart, for example the vehicle number. Each object has its own path and label."));
        ui.end_row();
        ui.label(tr(lang, "Colour"));
        ui.color_edit_button_srgba_unmultiplied(&mut s.color).on_hover_text(tr(lang, "The colour of the travelled path."));
        ui.end_row();
        let ground = tr(lang, "On the ground");
        ui.label(tr(lang, "Height"));
        egui::ComboBox::from_id_salt(("planetx-track-height", n)).selected_text(name_of(s.height, ground)).show_ui(ui, |ui| {
            ui.selectable_value(&mut s.height, None, ground);
            for (i, f) in fields.iter().enumerate().filter(|(i, _)| layer.numeric.get(*i).copied().unwrap_or(false)) {
                ui.selectable_value(&mut s.height, Some(i), f);
            }
        })
        .response
        .on_hover_text(tr(lang, "A numeric field with the height of the point. On the ground - the object and the model stand on the terrain."));
        ui.end_row();
        if s.height.is_some() {
            ui.label(tr(lang, "Height reference"));
            ui.horizontal(|ui| {
                ui.selectable_value(&mut s.relative, false, tr(lang, "above sea level"));
                ui.selectable_value(&mut s.relative, true, tr(lang, "above ground"));
            });
            ui.end_row();
            ui.label(tr(lang, "Height units"));
            ui.horizontal(|ui| {
                ui.selectable_value(&mut s.unit, 1.0, tr(lang, "metres"));
                ui.selectable_value(&mut s.unit, FOOT, tr(lang, "feet"));
            });
            ui.end_row();
        }
        ui.label(tr(lang, "3D model"));
        ui.label(s.model.as_ref().map_or_else(|| tr(lang, "without a model").to_string(), |m| m.name.clone()));
        ui.end_row();
        if s.model.is_some() {
            ui.label(tr(lang, "Model scale"));
            ui.add(egui::DragValue::new(&mut s.model_scale).range(0.01..=10000.0).speed(0.05).fixed_decimals(2))
                .on_hover_text(tr(lang, "The size factor of the model. With the factor 1 the model stands in its own metres."));
            ui.end_row();
            ui.label(tr(lang, "Model turn"));
            ui.add(egui::DragValue::new(&mut s.model_turn).range(-180.0..=180.0).speed(1.0).fixed_decimals(1).suffix("°"))
                .on_hover_text(tr(lang, "Clockwise turn of the model to the motion. Needed when the front of the model in the file does not look north."));
            ui.end_row();
        }
    });
    ui.checkbox(&mut layer.settings.shown, tr(lang, "Show"));
    let s = &layer.settings;
    before != (s.time, s.object, s.color, s.height, s.relative, s.unit, s.model_scale, s.model_turn, s.shown)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use data::layers::Item as LayerItem;

    fn layer(rows: &[(&str, &str, f64, f64)]) -> LayerData {
        LayerData {
            name: "trucks".into(),
            epsg: Some(4326),
            fields: vec!["truck".into(), "time".into(), "speed".into()],
            numeric: vec![false, false, true],
            items: rows
                .iter()
                .map(|(truck, time, lat, lon)| LayerItem { shape: LayerShape::Point, parts: vec![vec![(*lat, *lon)]], values: vec![truck.to_string(), time.to_string(), "10".into()] })
                .collect(),
        }
    }

    fn trucks() -> LayerData {
        layer(&[
            ("A", "2026-10-18T08:00:00Z", 58.0, 59.0),
            ("A", "2026-10-18T08:01:00Z", 58.0, 59.01),
            ("A", "2026-10-18T08:02:00Z", 58.0, 59.02),
            ("B", "2026-10-18T08:00:00Z", 58.1, 59.0),
            ("B", "2026-10-18T08:02:00Z", 58.1, 59.02),
        ])
    }

    #[test]
    fn fields_are_guessed() {
        let found = Layer::from_layer(&trucks(), "haul.gpkg").unwrap();
        assert_eq!(found.settings.time, 1);
        assert_eq!(found.settings.object, Some(0));
        assert_eq!(found.track.objects.len(), 2);
        assert_eq!(found.title, "haul.gpkg · trucks");
        // Слой без времени треком не становится.
        let mut plain = trucks();
        plain.fields[1] = "kind".into();
        for i in &mut plain.items {
            i.values[1] = "дорога".into();
        }
        assert!(Layer::from_layer(&plain, "x.gpkg").is_none());
    }

    #[test]
    fn numbers_are_seconds_and_text_is_iso() {
        assert_eq!(seconds("1792483200", true), Some(1_792_483_200.0));
        assert_eq!(seconds("1970-01-02T00:00:00Z", false), Some(86400.0));
        assert_eq!(seconds("", false), None);
        assert_eq!(seconds("груз", false), None);
    }

    #[test]
    fn paths_grow_with_the_moment() {
        let mut tracks = Tracks::default();
        tracks.add(vec![Layer::from_layer(&trucks(), "haul.gpkg").unwrap()], "haul.gpkg");
        let (lo, hi) = tracks.span().unwrap();
        assert_eq!(hi - lo, 120.0);
        tracks.step(Some(lo + 90.0));
        assert!(tracks.dirty);
        assert_eq!(tracks.dots().len(), 2);
        let a = tracks.shapes().iter().find(|s| s.name == "A").unwrap();
        // Две точки пройдены, третья - на середине отрезка.
        assert_eq!(a.points.len(), 3);
        assert!((a.points[2].1 - 59.015).abs() < 1e-9);
        tracks.dirty = false;
        tracks.step(Some(lo + 90.0));
        assert!(!tracks.dirty);
        tracks.step(None);
        assert_eq!(tracks.shapes().iter().find(|s| s.name == "A").unwrap().points.len(), 3);
    }

    #[test]
    fn model_rows_look_along_the_motion() {
        let found = Layer::from_layer(&trucks(), "haul.gpkg").unwrap();
        let rows = rows_of(&found.track.objects[0], 0.0);
        assert_eq!(rows.len(), 3);
        // На восток.
        assert!((rows[0].heading.unwrap() - 90.0).abs() < 0.1);
        assert!((rows_of(&found.track.objects[0], 180.0)[0].heading.unwrap() - 270.0).abs() < 0.1);
        let mut tracks = Tracks { demo_model: Some(Model { name: DEMO_MODEL.into(), package: Arc::new(vec![1, 2, 3]) }), ..Tracks::default() };
        tracks.add(vec![found], "haul_kachkanar.gpkg");
        let overlays = tracks.overlays();
        assert_eq!(overlays.len(), 2);
        assert_eq!(overlays[0].overlay.track.as_ref().map(Vec::len), Some(3));
        assert_eq!(tracks.layers[0].settings.color, HAUL_COLOR);
        // У объектов с моделью точки нет, подпись есть.
        tracks.step(None);
        assert!(tracks.dots().is_empty());
        assert_eq!(tracks.labels().len(), 2);
    }

    /// Слой самосвалов демо модуля: поля time и truck угадываются.
    #[test]
    fn module_haul_layer_becomes_track() {
        let path = std::path::Path::new(r"C:\Dev\planetx\planetx\demo\haul\haul_kachkanar.gpkg");
        let Ok(bytes) = std::fs::read(path) else {
            eprintln!("демо модуля нет, проверка пропущена");
            return;
        };
        let layers = data::layers::read(bytes, "haul_kachkanar.gpkg").unwrap();
        let (found, _) = Tracks::split(&layers, "haul_kachkanar.gpkg");
        let trucks = found.iter().find(|l| l.title.ends_with("trucks")).unwrap();
        assert_eq!(trucks.fields[trucks.settings.time], "time");
        assert_eq!(trucks.settings.object.map(|o| trucks.fields[o].as_str()), Some("truck"));
        assert!(trucks.track.objects.len() >= 2);
        assert!(trucks.track.start.is_some());
    }
}
