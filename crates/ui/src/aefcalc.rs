// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Изменения местности» - пункт подменю «Анализ участка» меню на
//! глобусе, перенос `ui/aef.py` модуля. Расчёт - `data::aef`, блоки
//! файлов по сети - [`crate::aefread`].
//!
//! Окно - на общем каркасе анализа участка ([`crate::analysis`]): участок
//! многоугольником, «Построить», «OK» и «Отмена». «Построить» берёт
//! указатель зоны участка, блоки обзора выбранного пикселя за два года
//! и копит степень изменения по каналу. Пока идёт расчёт, окно называет
//! объём загрузки: блок за год - 40-50 МБ при любом пикселе. Промежуток
//! степени меняет слой без пересчёта. «Места изменений» находит места
//! самых сильных изменений, «OK» кладёт в папку расчёта картинку, места
//! и пишет степень файлом GeoTIFF в зоне UTM файлов.
//!
//! Обзора по всему виду, как у модуля, у Lite нет: слой - только участок.

use std::collections::HashMap;
use std::sync::Arc;

use data::aef::{self, ChangeSum, Mosaic, Part, Piece, ZoneIndex};
use eframe::egui;
use globe_core::lod::TileKey;
use scene::features::Kind;
use scene::kml::{Item, Place};

use crate::aefread::BlockRead;
use crate::analysis::{self, Button, Saved, SiteEdit};
use crate::i18n::{Lang, tr, trf};

/// Участок по умолчанию, километры, как у окна модуля.
const SIDE: f64 = 3.0;
/// Мест изменений и их порог, `SPOTS` и `hot_spots` модуля.
const SPOTS: usize = 10;
const SPOT_SPACING: usize = 10;
const SPOT_LOW: f64 = 0.3;
const SPOT_COLOR: [u8; 4] = [255, 210, 30, 255];
/// Время распаковки каналов за кадр, секунды.
const BUDGET: f64 = 0.012;
/// Наибольший ответ указателя зоны, байт.
const INDEX_BYTES: u64 = 16 << 20;

/// Запрос куска по сети.
pub type Request<'a> = &'a dyn Fn(u64) -> Box<dyn net::Request>;

/// Указатели зон: пришедшие и идущие запросы.
#[derive(Default)]
pub struct Indexes {
    found: HashMap<String, Option<Arc<ZoneIndex>>>,
    asks: Vec<(String, Box<dyn net::Request>)>,
}

impl Indexes {
    /// Указатель зоны: Some(None) - указателя нет, None - ещё идёт,
    /// запрос начинается при первом спросе.
    pub fn get(&mut self, zone: &str, request: Request) -> Option<Option<Arc<ZoneIndex>>> {
        if let Some(found) = self.found.get(zone) {
            return Some(found.clone());
        }
        if !self.asks.iter().any(|(z, _)| z == zone) {
            let ask = request(INDEX_BYTES);
            ask.start(&aef::index_url(zone));
            self.asks.push((zone.to_string(), ask));
        }
        None
    }

    /// Пришедшие ответы.
    pub fn step(&mut self) {
        let found = &mut self.found;
        self.asks.retain(|(zone, ask)| match ask.take() {
            None => true,
            Some(answer) => {
                found.insert(zone.clone(), answer.ok().and_then(|b| ZoneIndex::parse(&b)).map(Arc::new));
                false
            }
        });
    }
}

/// Блок двух лет: чтения и степень, копящаяся по каналу.
struct Pair {
    reads: [BlockRead; 2],
    sum: Option<ChangeSum>,
    band: u16,
}

/// Расчёт участка: зона, блоки, готовые куски.
struct Job {
    zone: String,
    bbox: (f64, f64, f64, f64),
    ring: Vec<(f64, f64)>,
    level: Option<usize>,
    years: (i32, i32),
    index: Option<Arc<ZoneIndex>>,
    parts: Vec<Part>,
    next: usize,
    pair: Option<Pair>,
    pieces: Vec<Piece<u8>>,
}

/// Слой степени изменения участка для картинок тайлов.
pub struct Layer {
    pub index: Arc<ZoneIndex>,
    pub mosaic: Arc<Mosaic<u8>>,
    pub low: f64,
    pub high: f64,
}

impl Layer {
    /// Цвета точек (широта, долгота) с премноженной альфой.
    pub fn sample(&self, points: &[(f64, f64)]) -> Vec<[u8; 4]> {
        let alpha = (aef::OPACITY * 255.0).round() as u8;
        points
            .iter()
            .map(|&(lat, lon)| {
                let (x, y) = self.index.project(lat, lon);
                let v = self.mosaic.cell_of(x, y).and_then(|(r, c)| self.mosaic.at(r, c)).and_then(|v| v.first()).map_or(f64::NAN, |b| aef::from_byte(*b));
                match aef::color(v, self.low, self.high) {
                    Some(rgb) => {
                        let c = |v: u8| (f64::from(v) * aef::OPACITY).round() as u8;
                        [c(rgb[0]), c(rgb[1]), c(rgb[2]), alpha]
                    }
                    None => [0; 4],
                }
            })
            .collect()
    }

    /// Рамка участка в градусах (юг, север, запад, восток).
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        let m = &self.mosaic;
        let corners = [(m.x0, m.y0), (m.x0 + m.step * m.cols as f64, m.y0), (m.x0, m.y0 - m.step * m.rows as f64), (m.x0 + m.step * m.cols as f64, m.y0 - m.step * m.rows as f64)];
        let pts: Vec<(f64, f64)> = corners.iter().map(|&(x, y)| aef::from_utm(self.index.number, self.index.south, x, y)).collect();
        let lats = pts.iter().map(|p| p.0);
        let lons = pts.iter().map(|p| p.1);
        (lats.clone().fold(f64::INFINITY, f64::min), lats.fold(f64::NEG_INFINITY, f64::max), lons.clone().fold(f64::INFINITY, f64::min), lons.fold(f64::NEG_INFINITY, f64::max))
    }
}

/// Картинки тайлов слоя.
pub fn painter(layer: Arc<Layer>) -> crate::vector::Painter {
    let (south, north, west, east) = layer.bbox();
    Arc::new(move |key: TileKey| {
        let (lats, lons) = crate::pointcalc::tile_axes(key);
        let (top, bottom) = (lats.first().copied()?, lats.last().copied()?);
        let (left, right) = (lons.first().copied()?, lons.last().copied()?);
        if bottom > north || top < south || left > east || right < west {
            return None;
        }
        let points: Vec<(f64, f64)> = lats.iter().flat_map(|&la| lons.iter().map(move |&lo| (la, lo))).collect();
        let pixels = layer.sample(&points);
        pixels.iter().any(|p| p[3] > 0).then(|| pixels.concat())
    })
}

/// Код EPSG зоны UTM указателя: 326xx на севере, 327xx на юге.
pub fn utm_epsg(index: &ZoneIndex) -> u16 {
    (if index.south { 32700 } else { 32600 }) + index.number as u16
}

/// Окно «Изменения местности».
pub struct ChangeCalc {
    pub open: bool,
    place: String,
    pub site: SiteEdit,
    before: i32,
    after: i32,
    low: f64,
    high: f64,
    /// Пиксель участка: обзор файла, None - «Авто».
    level: Option<usize>,
    status: String,
    volume: String,
    job: Option<Job>,
    pub layer: Option<Arc<Layer>>,
    ring: Vec<(f64, f64)>,
    spots: Vec<Item>,
    pub changed: bool,
    pub saved: Option<Saved>,
    indexes: Indexes,
}

impl ChangeCalc {
    pub fn new(lat: f64, lon: f64, place: String) -> Self {
        Self {
            open: true,
            place,
            site: SiteEdit::new(lat, lon, SIDE, aef::DETAIL_SIDE / 1000.0),
            before: aef::YEARS[0],
            after: aef::YEARS[aef::YEARS.len() - 1],
            low: 0.0,
            high: 1.0,
            level: None,
            status: String::new(),
            volume: String::new(),
            job: None,
            layer: None,
            ring: Vec::new(),
            spots: Vec::new(),
            changed: false,
            saved: None,
            indexes: Indexes::default(),
        }
    }

    pub fn set_point(&mut self, lat: f64, lon: f64, place: String) {
        self.place = place;
        self.status.clear();
        self.open = true;
        self.site.move_to(lat, lon);
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    pub fn site_edit(&mut self) -> Option<&mut SiteEdit> {
        if self.open { Some(&mut self.site) } else { None }
    }

    fn clear(&mut self) {
        self.job = None;
        self.spots.clear();
        if self.layer.take().is_some() {
            self.changed = true;
        }
        self.status.clear();
    }

    /// Уровень обзора участка: выбранный или «Авто».
    fn level_for(&self, index: &ZoneIndex) -> usize {
        self.level.unwrap_or_else(|| aef::auto_level(index, self.site.site.bbox(), self.site.site.size()))
    }

    /// Оценка загрузки под полями, `volume_changed` модуля.
    fn update_volume(&mut self, lang: Lang, request: Request) {
        let (lat, lon) = self.site.site.centre();
        let zone = aef::zone_of(lon, lat);
        self.volume = match self.indexes.get(&zone, request) {
            None => tr(lang, "Loading the index of files…").to_string(),
            Some(None) => tr(lang, "There are no files of the data set for this zone.").to_string(),
            Some(Some(index)) if self.site.site.closed() => {
                let level = self.level_for(&index);
                let parts = aef::detail_parts(&index, self.site.site.bbox(), level).len();
                let pixel = aef::DETAIL_LEVELS.get(level).map_or(10.0, |l| l.1);
                trf(lang, "Download about {mb} MB, pixel {pixel} m.", &[("mb", &format!("{:.0}", aef::detail_megabytes(parts, level))), ("pixel", &format!("{pixel}"))])
            }
            Some(Some(_)) => tr(lang, "The area is not closed.").to_string(),
        };
    }

    fn build(&mut self, lang: Lang) {
        if let Some(problem) = self.site.problem(lang) {
            self.status = problem;
            return;
        }
        if self.before == self.after {
            self.status = tr(lang, "Choose two different years.").to_string();
            return;
        }
        let (lat, lon) = self.site.site.centre();
        self.layer = None;
        self.spots.clear();
        self.changed = true;
        self.job = Some(Job {
            zone: aef::zone_of(lon, lat),
            bbox: self.site.site.bbox(),
            ring: self.site.site.ring.clone(),
            level: self.level,
            years: (self.before, self.after),
            index: None,
            parts: Vec::new(),
            next: 0,
            pair: None,
            pieces: Vec::new(),
        });
    }

    /// Шаг расчёта: указатель, блоки, каналы, мозаика.
    pub fn step(&mut self, lang: Lang, request: Request) {
        self.indexes.step();
        let Some(job) = &mut self.job else {
            return;
        };
        if job.index.is_none() {
            match self.indexes.get(&job.zone, request) {
                None => {
                    self.status = tr(lang, "Loading the index of files…").to_string();
                    return;
                }
                Some(None) => {
                    self.status = tr(lang, "There are no files of the data set for this zone.").to_string();
                    self.job = None;
                    return;
                }
                Some(Some(index)) => {
                    let level = job.level.unwrap_or_else(|| aef::auto_level(&index, job.bbox, self.site.site.size()));
                    job.level = Some(level);
                    job.parts = aef::detail_parts(&index, job.bbox, level);
                    job.index = Some(index);
                }
            }
        }
        let (Some(index), Some(level)) = (job.index.clone(), job.level) else {
            return;
        };
        let started = web_time::Instant::now();
        while started.elapsed().as_secs_f64() < BUDGET {
            if job.pair.is_none() {
                let Some(&(cell, col, row)) = job.parts.get(job.next) else {
                    break;
                };
                let names = (index.name(cell, job.years.0), index.name(cell, job.years.1));
                let urls = match names {
                    (Some(a), Some(b)) => (aef::file_url(job.years.0, &index.zone, a), aef::file_url(job.years.1, &index.zone, b)),
                    _ => (None, None),
                };
                match urls {
                    (Some(a), Some(b)) => job.pair = Some(Pair { reads: [BlockRead::new(&a, level, col, row, request), BlockRead::new(&b, level, col, row, request)], sum: None, band: 0 }),
                    _ => job.next += 1,
                }
                continue;
            }
            let Some(pair) = &mut job.pair else {
                break;
            };
            if pair.sum.is_none() {
                for read in &mut pair.reads {
                    read.step(request);
                }
                if let Some(e) = pair.reads.iter().find_map(BlockRead::error) {
                    self.status = trf(lang, "The files are not read: {e}", &[("e", e)]);
                    self.job = None;
                    return;
                }
                let [a, b] = &pair.reads;
                match (a.block(), b.block()) {
                    (Some(a), Some(_)) => pair.sum = Some(ChangeSum::new(a.pixels())),
                    _ => break,
                }
            }
            let [a, b] = &pair.reads;
            let (Some(ba), Some(bb), Some(sum)) = (a.block(), b.block(), pair.sum.as_mut()) else {
                break;
            };
            if pair.band < ba.samples().min(bb.samples()) {
                if let (Some(va), Some(vb)) = (ba.band(pair.band), bb.band(pair.band)) {
                    sum.add(&va, &vb);
                }
                pair.band += 1;
                continue;
            }
            if let Some(piece) = ba.piece(sum.finish()) {
                job.pieces.push(piece);
            }
            job.pair = None;
            job.next += 1;
        }
        if job.next < job.parts.len() {
            let band = job.pair.as_ref().map_or(0, |p| p.band);
            let loading = job.pair.as_ref().is_some_and(|p| p.sum.is_none());
            self.status = if loading {
                trf(lang, "Block {n} of {total}: loading.", &[("n", &(job.next + 1).to_string()), ("total", &job.parts.len().to_string())])
            } else {
                trf(lang, "Block {n} of {total}: channel {band} of 64.", &[("n", &(job.next + 1).to_string()), ("total", &job.parts.len().to_string()), ("band", &band.to_string())])
            };
            return;
        }
        let Some(job) = self.job.take() else {
            return;
        };
        let Some(mut mosaic) = aef::mosaic(&job.pieces, &index, job.bbox, 1, aef::EMPTY) else {
            self.status = tr(lang, "There are no data of the data set in the area for these years.").to_string();
            return;
        };
        // Вне многоугольника участка степени нет.
        for r in 0..mosaic.rows {
            let y = mosaic.y0 - (r as f64 + 0.5) * mosaic.step;
            let points: Vec<(f64, f64)> = (0..mosaic.cols).map(|c| aef::from_utm(index.number, index.south, mosaic.x0 + (c as f64 + 0.5) * mosaic.step, y)).collect();
            for (c, keep) in data::site::inside(&points, &job.ring).into_iter().enumerate() {
                if !keep && let Some(v) = mosaic.values.get_mut(r * mosaic.cols + c) {
                    *v = aef::EMPTY;
                }
            }
        }
        let values: Vec<f64> = mosaic.values.iter().map(|b| aef::from_byte(*b)).filter(|v| v.is_finite()).collect();
        let mean = if values.is_empty() { f64::NAN } else { values.iter().sum::<f64>() / values.len() as f64 };
        let strong = values.iter().filter(|v| **v >= 0.5).count() as f64 / values.len().max(1) as f64;
        self.status = format!(
            "{} {}",
            trf(lang, "Mean degree of change {mean}, strong changes (from 0.5) on {share} of the area. The pixel is {pixel} m.", &[("mean", &format!("{mean:.2}")), ("share", &format!("{:.1}%", 100.0 * strong)), ("pixel", &format!("{}", mosaic.step))]),
            analysis::built_text(lang, crate::pointcalc::clock_now())
        );
        self.ring = job.ring;
        self.layer = Some(Arc::new(Layer { index, mosaic: Arc::new(mosaic), low: self.low, high: self.high }));
        self.changed = true;
    }

    /// Места самых сильных изменений точками, `spots` модуля.
    fn find_spots(&mut self, lang: Lang) {
        let Some(layer) = &self.layer else {
            return;
        };
        let m = &layer.mosaic;
        let values: Vec<f64> = m.values.iter().map(|b| aef::from_byte(*b)).collect();
        let found = aef::hot_spots(&values, m.rows, m.cols, SPOTS, SPOT_SPACING, SPOT_LOW);
        self.spots = found
            .iter()
            .enumerate()
            .map(|(n, (r, c, v))| {
                let (lat, lon) = aef::from_utm(layer.index.number, layer.index.south, m.x0 + (*c as f64 + 0.5) * m.step, m.y0 - (*r as f64 + 0.5) * m.step);
                let mut place = Place::new(&trf(lang, "Change {n}", &[("n", &(n + 1).to_string())]), Kind::Point, vec![(lat, lon)]);
                place.color = SPOT_COLOR;
                place.description = trf(lang, "Degree of change {value}.", &[("value", &format!("{v:.2}"))]);
                Item::Place(place)
            })
            .collect();
        self.status = if self.spots.is_empty() {
            tr(lang, "There are no strong changes in the area.").to_string()
        } else {
            trf(lang, "Found places of changes: {count}. They go into the folder of the computation by «OK».", &[("count", &self.spots.len().to_string())])
        };
    }

    /// Слой - расчётом для «Моих меток»: степень GeoTIFF в зоне UTM,
    /// картинка и места изменений.
    fn save(&mut self, lang: Lang) {
        let Some(layer) = &self.layer else {
            return;
        };
        let m = &layer.mosaic;
        let grid = data::geotiff::MercatorGrid { x0: m.x0 + m.step / 2.0, y0: m.y0 - m.step / 2.0, step: m.step, cols: m.cols, rows: m.rows };
        let values: Vec<f64> = m.values.iter().map(|b| aef::from_byte(*b)).collect();
        let tiff = data::geotiff::write_float_projected(&grid, utm_epsg(&layer.index), &values).unwrap_or_default();
        let shown = layer.clone();
        let sample = move |points: &[(f64, f64)]| shown.sample(points);
        let name = trf(lang, "Changes {before}-{after}", &[("before", &self.before.to_string()), ("after", &self.after.to_string())]);
        let mut picture = analysis::picture(name, "aef_change", layer.bbox(), m.step, &sample, tiff);
        picture.summary = analysis::stats_text(lang, tr(lang, "Degree of change"), &values, "");
        self.saved = Some(Saved { title: tr(lang, "Changes of the terrain").to_string(), ring: self.ring.clone(), layers: vec![picture], extra: std::mem::take(&mut self.spots), session: None });
    }

    /// Окно.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang, request: Request) {
        if !self.open {
            return;
        }
        if let Some(text) = self.site.shared_note(lang) {
            self.status = text.to_string();
        }
        let mut open = true;
        let mut pressed = None;
        let mut spots = false;
        let (low, high) = (self.low, self.high);
        self.update_volume(lang, request);
        egui::Window::new(tr(lang, "Changes of the terrain")).id(egui::Id::unique("planetx-aef-change")).open(&mut open).collapsible(false).resizable(false).min_width(400.0).show(ctx, |ui| {
            egui::Grid::new("planetx-aef-change-form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Point"));
                ui.label(&self.place);
                ui.end_row();
                ui.label(tr(lang, "Years"));
                ui.horizontal(|ui| {
                    let tip = tr(lang, "The years of the comparison. Each 10 m cell is described for a year by 64 numbers - a summary of Sentinel and Landsat images and radar. The degree of change is how much the descriptions of the two years differ.");
                    for (id, year) in [("before", &mut self.before), ("after", &mut self.after)] {
                        egui::ComboBox::from_id_salt(("planetx-aef-year", id)).selected_text(year.to_string()).show_ui(ui, |ui| {
                            for y in aef::YEARS {
                                ui.selectable_value(year, y, y.to_string());
                            }
                        })
                        .response
                        .on_hover_text(tip);
                    }
                });
                ui.end_row();
                ui.label(tr(lang, "Degree from"));
                ui.horizontal(|ui| {
                    let tip = tr(lang, "Show cells with a degree of change in this span, the rest are transparent. Unchanged terrain has a degree about 0.05, a building site, a clearing, a new quarry - 0.5 and more.");
                    ui.add(egui::DragValue::new(&mut self.low).range(0.0..=1.0).speed(0.01).fixed_decimals(2)).on_hover_text(tip);
                    ui.label(tr(lang, "to"));
                    ui.add(egui::DragValue::new(&mut self.high).range(0.0..=1.0).speed(0.01).fixed_decimals(2)).on_hover_text(tip);
                });
                ui.end_row();
                ui.label(tr(lang, "Area|site"));
                self.site.row(ui, lang);
                ui.end_row();
                ui.label(tr(lang, "Area pixel"));
                let text = match self.level {
                    None => tr(lang, "Auto").to_string(),
                    Some(l) => trf(lang, "{pixel} m", &[("pixel", &format!("{}", aef::DETAIL_LEVELS.get(l).map_or(10.0, |v| v.1)))]),
                };
                egui::ComboBox::from_id_salt("planetx-aef-level")
                    .selected_text(text)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.level, None, tr(lang, "Auto"));
                        for (level, pixel) in aef::DETAIL_LEVELS {
                            ui.selectable_value(&mut self.level, Some(level), trf(lang, "{pixel} m", &[("pixel", &format!("{pixel}"))]));
                        }
                    })
                    .response
                    .on_hover_text(tr(lang, "The pixel of the area. The files of the data set keep blocks of 1024 × 1024 pixels, a block for a year is 40-50 MB at any pixel: 10 m cover 10 km by a block, 80 m - 82 km. «Auto» takes the pixel with the least download."));
                ui.end_row();
                ui.label("");
                ui.label(&self.volume);
                ui.end_row();
            });
            ui.add_space(4.0);
            ui.add(egui::Label::new(self.site.hint(lang).unwrap_or(&self.status)).wrap());
            ui.add(egui::Label::new(egui::RichText::new(format!("{} {}", tr(lang, "Data - AlphaEarth Foundations Satellite Embedding, CC BY 4.0."), aef::CREDIT)).small()).wrap());
            ui.add_space(4.0);
            if ui
                .add_enabled(self.layer.is_some(), egui::Button::new(tr(lang, "Places of changes")))
                .on_hover_text(tr(lang, "Places of the strongest changes of the area - points in the folder of the computation. Then they are handy to look at with the image archive and the swipe."))
                .clicked()
            {
                spots = true;
            }
            pressed = analysis::buttons(ui, lang, self.layer.is_some() && self.job.is_none(), tr(lang, "The degree of change of the area between the two years - a layer on the globe."));
        });
        if (self.low, self.high) != (low, high)
            && let Some(old) = &self.layer
        {
            self.layer = Some(Arc::new(Layer { index: old.index.clone(), mosaic: old.mosaic.clone(), low: self.low, high: self.high }));
            self.changed = true;
        }
        match pressed {
            Some(Button::Build) => self.build(lang),
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
        if spots {
            self.find_spots(lang);
        }
        if !open {
            self.clear();
            self.open = false;
        }
        if std::mem::take(&mut self.site.changed) && self.layer.is_some() && self.job.is_none() {
            self.status = tr(lang, "The area has changed. Press «Build» to compute it again.").to_string();
        }
        if self.job.is_some() {
            ctx.request_repaint();
        }
    }
}
