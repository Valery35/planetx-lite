// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Классификация» - пункт подменю «Анализ участка» меню на
//! глобусе, перенос `ui/aefclass.py` модуля. Расчёт - `data::aef`
//! (`classify`), векторы участка - блоки файлов по сети
//! ([`crate::aefread`]).
//!
//! Просьба автора от 28 октября 2026 года по образцу «Классификации»
//! Google Earth. Пользователь заводит классы с цветом и названием, ставит
//! щелчком по глобусу контрольные точки выбранного класса, щелчок по
//! точке убирает её. «Построить» берёт векторы AlphaEarth участка на
//! выбранный год и относит каждую клетку к классу ближайших контрольных
//! точек. Векторы участка помнятся: новые точки дают новые классы без
//! новой загрузки. Окно - на общем каркасе анализа участка
//! ([`crate::analysis`]), «OK» кладёт в папку расчёта картинку классов
//! и контрольные точки и пишет классы файлом GeoTIFF в зоне UTM файлов.
//!
//! Источник клеток - AlphaEarth или растр, открытый в программе, просьба
//! Василия Швалёва от 31 октября 2026 года. Каналы растра - признаки
//! клетки, расчёт - `data::rasterclass`. Растр пересчитывается на сетку
//! участка и классы считаются частями за кадры.

use std::sync::Arc;

use data::aef::{self, Mosaic, Part, Piece, ZoneIndex};
use data::rasterclass::{self, Problem, Run, Source, Warp};
use eframe::egui;
use globe_core::DVec3;
use globe_core::lod::TileKey;
use globe_core::navigation::{Navigator, ground_under};
use scene::features::Kind;
use scene::kml::{Item, Place};

use crate::aefcalc::{Indexes, Request, utm_epsg};
use crate::aefread::BlockRead;
use crate::analysis::{self, Button, Saved, SiteEdit};
use crate::i18n::{Lang, tr, trf};

/// Участок по умолчанию и наибольший, километры, `CLASS_SIDE` модуля.
const SIDE: f64 = 3.0;
const MOST: f64 = 10.0;
/// Наибольший участок растра, километры, `RASTER_SIDE` модуля. Большую
/// сетку отсекает `rasterclass::too_big`.
const RASTER_MOST: f64 = 1000.0;
/// Цвета новых классов по порядку - выбор помощника модуля.
const COLORS: [[u8; 3]; 8] = [[60, 160, 70], [40, 110, 220], [240, 190, 40], [200, 60, 50], [150, 90, 200], [120, 120, 120], [240, 130, 40], [60, 200, 200]];
const OPACITY: f64 = 0.6;
/// Время распаковки каналов за кадр, секунды.
const BUDGET: f64 = 0.012;
/// Логических пикселей вокруг контрольной точки, в них она убирается.
const POINT_PIXELS: f64 = 8.0;
/// Строк растра и клеток классов за один шаг внутри бюджета кадра.
const WARP_ROWS: usize = 4;
const RUN_CELLS: usize = 4096;

/// Класс: название, цвет, контрольные точки.
#[derive(Debug, Clone, PartialEq)]
pub struct Class {
    pub name: String,
    pub color: [u8; 3],
    pub points: Vec<(f64, f64)>,
}

/// Открытый растр: имя и байты файла.
pub type Raster = (String, Arc<Vec<u8>>);

/// Сетка растра на участке: ключ (растр, рамка, уровень) и сетка.
type OwnGrid = ((String, String, Option<usize>), Arc<ZoneIndex>, Arc<Mosaic<f32>>);

/// Шаг расчёта по растру.
enum OwnStage {
    /// Файл ещё не разобран: кадр со строкой «Читается…» уже показан.
    Read(Arc<Vec<u8>>),
    Warp(Box<(Source, Warp)>),
    Classes { grid: Arc<Mosaic<f32>>, run: Box<Run>, outside: usize, share: f64 },
}

/// Расчёт по растру.
struct OwnJob {
    key: (String, String, Option<usize>),
    index: Arc<ZoneIndex>,
    bbox: (f64, f64, f64, f64),
    side: f64,
    stage: OwnStage,
}

/// Векторы участка: ключ (рамка, год, уровень) и мозаика.
type Vectors = ((String, i32, usize), Arc<ZoneIndex>, Arc<Mosaic<i8>>);

/// Загрузка векторов участка.
struct Job {
    zone: String,
    bbox: (f64, f64, f64, f64),
    year: i32,
    level: Option<usize>,
    index: Option<Arc<ZoneIndex>>,
    parts: Vec<Part>,
    next: usize,
    read: Option<(BlockRead, Vec<i8>, u16)>,
    pieces: Vec<Piece<i8>>,
}

/// Слой классов для картинок тайлов.
pub struct Layer {
    pub index: Arc<ZoneIndex>,
    pub step: f64,
    pub x0: f64,
    pub y0: f64,
    pub rows: usize,
    pub cols: usize,
    /// Номер класса клетки, -1 - нет класса.
    pub classes: Vec<i16>,
    pub colors: Vec<[u8; 3]>,
    pub opacity: f64,
}

impl Layer {
    /// Цвета точек (широта, долгота) с премноженной альфой.
    pub fn sample(&self, points: &[(f64, f64)]) -> Vec<[u8; 4]> {
        let alpha = (self.opacity * 255.0).round() as u8;
        points
            .iter()
            .map(|&(lat, lon)| {
                let (x, y) = self.index.project(lat, lon);
                let (c, r) = (((x - self.x0) / self.step).floor(), ((self.y0 - y) / self.step).floor());
                if !(c >= 0.0 && r >= 0.0 && (c as usize) < self.cols && (r as usize) < self.rows) {
                    return [0; 4];
                }
                let class = self.classes.get(r as usize * self.cols + c as usize).copied().unwrap_or(-1);
                match usize::try_from(class).ok().and_then(|k| self.colors.get(k)) {
                    Some(rgb) => {
                        let c = |v: u8| (f64::from(v) * self.opacity).round() as u8;
                        [c(rgb[0]), c(rgb[1]), c(rgb[2]), alpha]
                    }
                    None => [0; 4],
                }
            })
            .collect()
    }

    /// Рамка в градусах (юг, север, запад, восток).
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        let (w, h) = (self.step * self.cols as f64, self.step * self.rows as f64);
        let pts: Vec<(f64, f64)> = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].iter().map(|&(dx, dy)| aef::from_utm(self.index.number, self.index.south, self.x0 + dx, self.y0 - dy)).collect();
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

/// Окно «Классификация».
pub struct ClassCalc {
    pub open: bool,
    place: String,
    pub site: SiteEdit,
    year: i32,
    level: Option<usize>,
    opacity: f64,
    classes: Vec<Class>,
    /// Класс, точки которого ставит щелчок по глобусу.
    active: usize,
    status: String,
    job: Option<Job>,
    vectors: Option<Vectors>,
    pub layer: Option<Arc<Layer>>,
    pub changed: bool,
    pub saved: Option<Saved>,
    indexes: Indexes,
    /// Растры, открытые в программе, и выбранный, None - AlphaEarth.
    pub rasters: Vec<Raster>,
    source: Option<String>,
    own_job: Option<OwnJob>,
    own_grid: Option<OwnGrid>,
    /// Подпись расчёта в «Моих метках»: год или имя растра.
    label: String,
}

impl ClassCalc {
    pub fn new(lat: f64, lon: f64, place: String, rasters: Vec<Raster>, lang: Lang) -> Self {
        // Файлы AlphaEarth странице недоступны, источник - только растр.
        let source = if cfg!(target_arch = "wasm32") { rasters.first().map(|r| r.0.clone()) } else { None };
        let mut calc = Self {
            open: true,
            place,
            site: SiteEdit::new(lat, lon, SIDE, MOST),
            year: aef::YEARS[aef::YEARS.len() - 1],
            level: None,
            opacity: OPACITY,
            classes: Vec::new(),
            active: 0,
            status: String::new(),
            job: None,
            vectors: None,
            layer: None,
            changed: false,
            saved: None,
            indexes: Indexes::default(),
            rasters,
            source,
            own_job: None,
            own_grid: None,
            label: String::new(),
        };
        calc.add_class(lang);
        calc.add_class(lang);
        calc.active = 0;
        calc
    }

    fn add_class(&mut self, lang: Lang) {
        let n = self.classes.len();
        self.classes.push(Class { name: trf(lang, "Class {n}", &[("n", &(n + 1).to_string())]), color: COLORS[n % COLORS.len()], points: Vec::new() });
        self.active = n;
    }

    pub fn set_point(&mut self, lat: f64, lon: f64, place: String) {
        self.place = place;
        self.status.clear();
        self.open = true;
        self.site.move_to(lat, lon);
    }

    pub fn busy(&self) -> bool {
        self.job.is_some() || self.own_job.is_some()
    }

    pub fn site_edit(&mut self) -> Option<&mut SiteEdit> {
        if self.open { Some(&mut self.site) } else { None }
    }

    fn clear(&mut self) {
        self.job = None;
        self.own_job = None;
        if self.layer.take().is_some() {
            self.changed = true;
        }
        self.status.clear();
    }

    /// Ключ векторов участка.
    fn key(&self, level: usize) -> (String, i32, usize) {
        let (w, s, e, n) = self.site.site.bbox();
        (format!("{w:.6},{s:.6},{e:.6},{n:.6}"), self.year, level)
    }

    fn build(&mut self, lang: Lang) {
        let ready = self.classes.iter().filter(|c| !c.points.is_empty()).count() >= aef::MIN_CLASSES && self.classes.iter().filter(|c| !c.points.is_empty()).all(|c| c.points.len() >= aef::MIN_POINTS);
        if !ready {
            self.status = tr(lang, "At least two classes of three points each are needed.").to_string();
            return;
        }
        if let Some(problem) = self.site.problem(lang) {
            self.status = problem;
            return;
        }
        let (lat, lon) = self.site.site.centre();
        if let Some(name) = self.source.clone() {
            self.build_raster(lang, name, lat, lon);
            return;
        }
        // Векторы того же участка, года и пикселя уже есть: только классы.
        if let Some((key, _, _)) = &self.vectors
            && *key == self.key(self.level.unwrap_or(key.2))
        {
            self.classify(lang);
            return;
        }
        self.job = Some(Job { zone: aef::zone_of(lon, lat), bbox: self.site.site.bbox(), year: self.year, level: self.level, index: None, parts: Vec::new(), next: 0, read: None, pieces: Vec::new() });
    }

    /// Классы по каналам открытого растра. Сетка того же участка и пикселя
    /// уже есть - только классы.
    fn build_raster(&mut self, lang: Lang, name: String, lat: f64, lon: f64) {
        let Some(bytes) = self.rasters.iter().find(|r| r.0 == name).map(|r| r.1.clone()) else {
            self.status = tr(lang, "The raster is no longer open.").to_string();
            return;
        };
        let Some(index) = ZoneIndex::bare(&aef::zone_of(lon, lat)) else {
            return;
        };
        let (w, s, e, n) = self.site.site.bbox();
        let key = (name, format!("{w:.6},{s:.6},{e:.6},{n:.6}"), self.level);
        self.job = None;
        // Пиксель задан: сетка известна до чтения файла. У «Авто» её
        // ограничивает сам пиксель.
        if let Some((_, step)) = self.level.and_then(|l| aef::DETAIL_LEVELS.get(l)) {
            let (_, _, cols, rows) = aef::grid_frame(&index, self.site.site.bbox(), *step);
            if rasterclass::too_big(rows, cols, 1) {
                self.own_problem(lang, Problem::Big);
                return;
            }
        }
        if let Some((_, index, grid)) = self.own_grid.clone().filter(|g| g.0 == key) {
            self.start_classes(lang, key, index, grid);
            return;
        }
        let (bbox, side) = (self.site.site.bbox(), self.site.site.size());
        self.own_job = Some(OwnJob { key, index: Arc::new(index), bbox, side, stage: OwnStage::Read(bytes) });
        self.status = tr(lang, "Reading the raster of the area…").to_string();
    }

    /// Точки на сетке растра и начало расчёта классов. Отказы - как
    /// `_build_raster` модуля: вне охвата, нет точек на клетках с данными,
    /// каналы без разброса.
    fn start_classes(&mut self, lang: Lang, key: (String, String, Option<usize>), index: Arc<ZoneIndex>, grid: Arc<Mosaic<f32>>) {
        let keep = ring_mask(&index, &grid, &self.site.site.ring);
        let share = rasterclass::covered(&grid.values, grid.depth, &keep);
        if share <= 0.0 {
            self.own_problem(lang, Problem::Outside);
            return;
        }
        let rows: Vec<&Class> = self.classes.iter().filter(|c| !c.points.is_empty()).collect();
        let points: Vec<(i32, (f64, f64))> = rows.iter().enumerate().flat_map(|(n, c)| c.points.iter().map(move |p| (n as i32, *p))).collect();
        let (samples, labels, outside) = rasterclass::samples(&grid, &index, &points);
        let mut seen = labels.clone();
        seen.sort_unstable();
        seen.dedup();
        if seen.len() < aef::MIN_CLASSES {
            self.own_problem(lang, Problem::Points);
            return;
        }
        let run = Box::new(Run::new(&grid.values, grid.depth, &samples, &labels, Some(&keep), rasterclass::KNN));
        let (bbox, side) = (self.site.site.bbox(), self.site.site.size());
        self.status = tr(lang, "Computing the classes…").to_string();
        self.own_job = Some(OwnJob { key, index, bbox, side, stage: OwnStage::Classes { grid, run, outside, share } });
    }

    /// Отказ снимает прежние классы: «OK» сохранил бы их с новым участком.
    fn own_problem(&mut self, lang: Lang, problem: Problem) {
        self.own_job = None;
        if self.layer.take().is_some() {
            self.changed = true;
        }
        self.status = problem_text(lang, problem).to_string();
    }

    /// Шаг расчёта по растру.
    fn step_own(&mut self, lang: Lang) {
        let started = web_time::Instant::now();
        let Some(job) = &mut self.own_job else {
            return;
        };
        let index = job.index.clone();
        match &mut job.stage {
            OwnStage::Read(bytes) => {
                let (centre, side, bbox, level) = (self.site.site.centre(), job.side, job.bbox, job.key.2);
                let opened = Source::read(bytes, &job.key.0).and_then(|source| {
                    let pixel = match level.and_then(|l| aef::DETAIL_LEVELS.get(l)) {
                        Some(level) => level.1,
                        None => rasterclass::auto_pixel(source.pixel(&index, centre.0, centre.1), side),
                    };
                    let warp = source.warp(&index, bbox, pixel)?;
                    Ok((source, warp))
                });
                match opened {
                    Ok(pair) => job.stage = OwnStage::Warp(Box::new(pair)),
                    Err(problem) => self.own_problem(lang, problem),
                }
            }
            OwnStage::Warp(pair) => {
                let (source, warp) = &mut **pair;
                let mut done = false;
                while !done && started.elapsed().as_secs_f64() < BUDGET {
                    done = source.warp_rows(warp, WARP_ROWS);
                }
                if !done {
                    return;
                }
                let Some(OwnJob { key, index, stage: OwnStage::Warp(pair), .. }) = self.own_job.take() else {
                    return;
                };
                let grid = Arc::new(pair.1.grid);
                self.own_grid = Some((key.clone(), index.clone(), grid.clone()));
                self.start_classes(lang, key, index, grid);
            }
            OwnStage::Classes { grid, run, .. } => {
                let mut done = false;
                while !done && started.elapsed().as_secs_f64() < BUDGET {
                    done = run.step(&grid.values, RUN_CELLS);
                }
                if done {
                    self.own_built(lang);
                }
            }
        }
    }

    /// Классы по растру готовы, `_built` модуля.
    fn own_built(&mut self, lang: Lang) {
        let Some(OwnJob { key, index, stage: OwnStage::Classes { grid, run, outside, share }, .. }) = self.own_job.take() else {
            return;
        };
        let (classes, _) = run.finish();
        if !classes.iter().any(|c| *c >= 0) {
            self.status = problem_text(lang, Problem::Flat).to_string();
            return;
        }
        let rows: Vec<&Class> = self.classes.iter().filter(|c| !c.points.is_empty()).collect();
        let parts = class_shares(&rows, &classes);
        let pixel = format!("{}", (grid.step * 100.0).round() / 100.0);
        let mut text = trf(lang, "Classes of the raster «{name}»: {parts}. Pixel {pixel} m.", &[("name", &key.0), ("parts", &parts), ("pixel", &pixel)]);
        if share < rasterclass::COVERED {
            text = format!("{text} {}", trf(lang, "The raster covers {share} % of the area.", &[("share", &format!("{:.0}", 100.0 * share))]));
        }
        if outside > 0 {
            text = format!("{text} {}", trf(lang, "Points without raster data: {n}.", &[("n", &outside.to_string())]));
        }
        self.status = format!("{text} {}", analysis::built_text(lang, crate::pointcalc::clock_now()));
        self.layer = Some(Arc::new(Layer { index, step: grid.step, x0: grid.x0, y0: grid.y0, rows: grid.rows, cols: grid.cols, classes, colors: rows.iter().map(|c| c.color).collect(), opacity: self.opacity }));
        self.label = key.0;
        self.changed = true;
    }

    /// Шаг загрузки векторов.
    pub fn step(&mut self, lang: Lang, request: Request) {
        self.step_own(lang);
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
            if job.read.is_none() {
                let Some(&(cell, col, row)) = job.parts.get(job.next) else {
                    break;
                };
                match index.name(cell, job.year).and_then(|n| aef::file_url(job.year, &index.zone, n)) {
                    Some(url) => job.read = Some((BlockRead::new(&url, level, col, row, request), Vec::new(), 0)),
                    None => job.next += 1,
                }
                continue;
            }
            let Some((read, values, band)) = &mut job.read else {
                break;
            };
            if read.block().is_none() {
                read.step(request);
                if let Some(e) = read.error() {
                    self.status = trf(lang, "The files are not read: {e}", &[("e", e)]);
                    self.job = None;
                    return;
                }
                if read.block().is_none() {
                    break;
                }
            }
            let Some(block) = read.block() else {
                break;
            };
            let depth = usize::from(aef::BANDS);
            if values.is_empty() {
                *values = vec![aef::NODATA; block.pixels() * depth];
            }
            if *band < block.samples().min(aef::BANDS) {
                if let Some(plane) = block.band(*band) {
                    for (i, v) in plane.into_iter().enumerate() {
                        if let Some(slot) = values.get_mut(i * depth + usize::from(*band)) {
                            *slot = v;
                        }
                    }
                }
                *band += 1;
                continue;
            }
            if let Some(piece) = block.piece(std::mem::take(values)) {
                job.pieces.push(piece);
            }
            job.read = None;
            job.next += 1;
        }
        if job.next < job.parts.len() {
            self.status = trf(lang, "Loading the vectors of the area: block {n} of {total}.", &[("n", &(job.next + 1).to_string()), ("total", &job.parts.len().to_string())]);
            return;
        }
        let Some(job) = self.job.take() else {
            return;
        };
        let Some(mosaic) = aef::mosaic(&job.pieces, &index, job.bbox, usize::from(aef::BANDS), aef::NODATA) else {
            self.status = tr(lang, "There are no data of the data set in the area for this year.").to_string();
            return;
        };
        let key = self.key(level);
        self.vectors = Some((key, index, Arc::new(mosaic)));
        self.classify(lang);
    }

    /// Классы участка по векторам и точкам, `_built` модуля.
    fn classify(&mut self, lang: Lang) {
        let Some((_, index, grid)) = &self.vectors else {
            return;
        };
        let rows: Vec<&Class> = self.classes.iter().filter(|c| !c.points.is_empty()).collect();
        let (mut samples, mut labels, mut outside) = (Vec::new(), Vec::new(), 0);
        for (label, class) in rows.iter().enumerate() {
            for &(lat, lon) in &class.points {
                let (x, y) = index.project(lat, lon);
                match grid.cell_of(x, y).and_then(|(r, c)| grid.at(r, c)) {
                    Some(v) => {
                        samples.push(v.to_vec());
                        labels.push(label as i32);
                    }
                    None => outside += 1,
                }
            }
        }
        if samples.is_empty() {
            self.status = tr(lang, "All points are outside the area.").to_string();
            return;
        }
        let found = aef::classify(&grid.values, grid.depth, &samples, &labels, aef::KNN);
        let mut classes: Vec<i16> = found.iter().map(|c| c.map_or(-1, |v| v.0 as i16)).collect();
        // Вне многоугольника участка классов нет.
        for (v, keep) in classes.iter_mut().zip(ring_mask(index, grid, &self.site.site.ring)) {
            if !keep {
                *v = -1;
            }
        }
        let parts = class_shares(&rows, &classes);
        let mut text = trf(lang, "Classes of the year {year}: {parts}.", &[("year", &self.year.to_string()), ("parts", &parts)]);
        if outside > 0 {
            text = format!("{text} {}", trf(lang, "Points outside the area: {n}.", &[("n", &outside.to_string())]));
        }
        self.status = format!("{text} {}", analysis::built_text(lang, crate::pointcalc::clock_now()));
        self.label = self.year.to_string();
        self.layer = Some(Arc::new(Layer { index: index.clone(), step: grid.step, x0: grid.x0, y0: grid.y0, rows: grid.rows, cols: grid.cols, classes, colors: rows.iter().map(|c| c.color).collect(), opacity: self.opacity }));
        self.changed = true;
    }

    /// Классы - расчётом для «Моих меток»: GeoTIFF номеров классов в зоне
    /// UTM, картинка, контрольные точки папками классов.
    fn save(&mut self, lang: Lang) {
        let Some(layer) = &self.layer else {
            return;
        };
        let grid = data::geotiff::MercatorGrid { x0: layer.x0 + layer.step / 2.0, y0: layer.y0 - layer.step / 2.0, step: layer.step, cols: layer.cols, rows: layer.rows };
        let bytes: Vec<u8> = layer.classes.iter().map(|c| u8::try_from(*c).unwrap_or(data::geotiff::BYTE_NODATA)).collect();
        let tiff = data::geotiff::write_byte_projected(&grid, utm_epsg(&layer.index), &bytes).unwrap_or_default();
        let shown = layer.clone();
        let sample = move |points: &[(f64, f64)]| shown.sample(points);
        let name = trf(lang, "Classification {year}", &[("year", &self.label)]);
        let picture = analysis::picture(name, "aef_class", layer.bbox(), layer.step, &sample, tiff);
        let mut extra = Vec::new();
        for (n, class) in self.classes.iter().filter(|c| !c.points.is_empty()).enumerate() {
            let mut folder = scene::kml::Folder::new(&trf(lang, "{n} - {name}", &[("n", &n.to_string()), ("name", &class.name)]));
            folder.description = trf(lang, "Value {n} in the GeoTIFF file.", &[("n", &n.to_string())]);
            for &(lat, lon) in &class.points {
                let mut place = Place::new(&class.name, Kind::Point, vec![(lat, lon)]);
                place.color = [class.color[0], class.color[1], class.color[2], 255];
                folder.children.push(Item::Place(place));
            }
            extra.push(Item::Folder(folder));
        }
        self.saved = Some(Saved { title: tr(lang, "Classification").to_string(), ring: self.site.site.ring.clone(), layers: vec![picture], extra, session: None });
    }

    /// Мышь над глобусом: точки выбранного класса щелчком, щелчок по
    /// точке убирает её. Участок - главнее. true - мышь занята.
    pub fn input(&mut self, ctx: &egui::Context, response: &egui::Response, rect: egui::Rect, ppp: f64, nav: &Navigator) -> bool {
        if !self.open {
            return false;
        }
        if self.site.input(ctx, response, rect, ppp, nav) {
            return true;
        }
        if self.site.site.drawing {
            return false;
        }
        let to_px = |p: egui::Pos2| (f64::from(p.x - rect.min.x) * ppp, f64::from(p.y - rect.min.y) * ppp);
        let pos = ctx.input(|i| i.pointer.interact_pos());
        let on_globe = pos.is_some_and(|p| rect.contains(p) && ctx.layer_id_at(p).is_none_or(|l| l == response.layer_id));
        if !(response.clicked() && on_globe) {
            return false;
        }
        let Some(p) = pos else {
            return false;
        };
        let (px, py) = to_px(p);
        // Щелчок по точке убирает её, `point_under` модуля.
        let radius = POINT_PIXELS * ppp;
        let mut hit = None;
        for (k, class) in self.classes.iter().enumerate() {
            for (i, pt) in class.points.iter().enumerate() {
                let (pix, visible) = pixel(nav, *pt);
                if visible && (pix[0] - px).hypot(pix[1] - py) <= radius {
                    hit = Some((k, i));
                }
            }
        }
        if let Some((k, i)) = hit {
            if let Some(class) = self.classes.get_mut(k) {
                class.points.remove(i);
            }
            return true;
        }
        let Some(ground) = ground_under(&nav.camera, px, py, nav.pose.terrain.as_ref()) else {
            return false;
        };
        let (lat, lon, _) = nav.camera.ellipsoid.ecef_to_geodetic(ground);
        if let Some(class) = self.classes.get_mut(self.active) {
            class.points.push((lat, lon));
        }
        true
    }

    /// Контрольные точки цветами классов поверх глобуса.
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect, ppp: f64, nav: &Navigator) {
        if !self.open {
            return;
        }
        self.site.paint(painter, rect, ppp, nav);
        let to_screen = |p: [f64; 2]| rect.min + egui::vec2((p[0] / ppp) as f32, (p[1] / ppp) as f32);
        for class in &self.classes {
            let fill = egui::Color32::from_rgb(class.color[0], class.color[1], class.color[2]);
            for pt in &class.points {
                let (p, visible) = pixel(nav, *pt);
                if visible {
                    painter.circle(to_screen(p), 5.0, fill, egui::Stroke::new(1.5, egui::Color32::WHITE));
                }
            }
        }
    }

    /// Окно.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang) {
        if !self.open {
            return;
        }
        if let Some(text) = self.site.shared_note(lang) {
            self.status = text.to_string();
        }
        self.site.site.most_km = if self.source.is_some() { RASTER_MOST } else { MOST };
        let mut open = true;
        let mut pressed = None;
        let opacity = self.opacity;
        let mut remove = None;
        let mut add = false;
        egui::Window::new(tr(lang, "Classification")).id(egui::Id::unique("planetx-aef-class")).open(&mut open).collapsible(false).resizable(false).min_width(400.0).show(ctx, |ui| {
            egui::Grid::new("planetx-aef-class-form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Point"));
                ui.label(&self.place);
                ui.end_row();
                ui.label(tr(lang, "Source"));
                egui::ComboBox::from_id_salt("planetx-aef-class-source")
                    .selected_text(self.source.as_deref().unwrap_or("AlphaEarth"))
                    .show_ui(ui, |ui| {
                        if !cfg!(target_arch = "wasm32") {
                            ui.selectable_value(&mut self.source, None, "AlphaEarth");
                        }
                        for (name, _) in &self.rasters {
                            ui.selectable_value(&mut self.source, Some(name.clone()), name);
                        }
                    })
                    .response
                    .on_hover_text(tr(lang, "What the cells are divided by. AlphaEarth vectors are loaded from the network. Each band of a raster opened in the program is a feature, the network is not needed."));
                ui.end_row();
                if self.source.is_none() {
                    ui.label(tr(lang, "Year"));
                    egui::ComboBox::from_id_salt("planetx-aef-class-year")
                        .selected_text(self.year.to_string())
                        .show_ui(ui, |ui| {
                            for y in aef::YEARS {
                                ui.selectable_value(&mut self.year, y, y.to_string());
                            }
                        })
                        .response
                        .on_hover_text(tr(lang, "The year by whose vectors the cells are divided into classes. The points are put by the image of the globe, it may be of another year."));
                    ui.end_row();
                }
                ui.label(tr(lang, "Area|site"));
                self.site.row(ui, lang);
                ui.end_row();
                ui.label(tr(lang, "Pixel"));
                let text = match self.level {
                    None => tr(lang, "Auto").to_string(),
                    Some(l) => trf(lang, "{pixel} m", &[("pixel", &format!("{}", aef::DETAIL_LEVELS.get(l).map_or(10.0, |v| v.1)))]),
                };
                egui::ComboBox::from_id_salt("planetx-aef-class-level")
                    .selected_text(text)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.level, None, tr(lang, "Auto"));
                        for (level, pixel) in aef::DETAIL_LEVELS {
                            ui.selectable_value(&mut self.level, Some(level), trf(lang, "{pixel} m", &[("pixel", &format!("{pixel}"))]));
                        }
                    })
                    .response
                    .on_hover_text(if self.source.is_some() {
                        tr(lang, "The pixel of the classification. «Auto» takes the pixel of the raster if the area has no more than 640 thousand cells.")
                    } else {
                        tr(lang, "The pixel of the classification. A block of the file for a year is 40-50 MB at any pixel. «Auto» takes the pixel with the least download at which the area has from 14 to 640 thousand cells.")
                    });
                ui.end_row();
                ui.label(tr(lang, "Opacity"));
                ui.add(egui::Slider::new(&mut self.opacity, 0.0..=1.0).show_value(false)).on_hover_text(tr(lang, "Opacity of the classes on the globe."));
                ui.end_row();
            });
            ui.add_space(4.0);
            ui.label(tr(lang, "Classes"));
            for (k, class) in self.classes.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.active, k, "").on_hover_text(tr(lang, "A click on the globe puts a point of this class."));
                    ui.color_edit_button_srgb(&mut class.color);
                    ui.add(egui::TextEdit::singleline(&mut class.name).desired_width(140.0));
                    ui.label(trf(lang, "points {n}", &[("n", &class.points.len().to_string())]));
                    if ui.small_button(tr(lang, "Remove")).on_hover_text(tr(lang, "Remove the class and its points.")).clicked() {
                        remove = Some(k);
                    }
                });
            }
            if ui.button(tr(lang, "Add a class")).clicked() {
                add = true;
            }
            ui.add(egui::Label::new(tr(lang, "Click the globe to put a point of the chosen class, a click on a point removes it. At least two classes of three points each are needed.")).wrap());
            ui.add_space(4.0);
            ui.add(egui::Label::new(self.site.hint(lang).unwrap_or(&self.status)).wrap());
            if self.source.is_none() {
                ui.add(egui::Label::new(egui::RichText::new(format!("{} {}", tr(lang, "Data - AlphaEarth Foundations Satellite Embedding, CC BY 4.0."), aef::CREDIT)).small()).wrap());
            }
            ui.add_space(4.0);
            let tip = if self.source.is_some() { tr(lang, "Each cell of the area goes to the class of the nearest control points by the raster bands.") } else { tr(lang, "Each cell of the area goes to the class of the nearest control points by AlphaEarth vectors.") };
            pressed = analysis::buttons(ui, lang, self.layer.is_some() && !self.busy(), tip);
        });
        if let Some(k) = remove
            && k < self.classes.len()
        {
            self.classes.remove(k);
            self.active = self.active.min(self.classes.len().saturating_sub(1));
        }
        if add {
            self.add_class(lang);
        }
        if (self.opacity - opacity).abs() > 1e-6
            && let Some(old) = &self.layer
        {
            self.layer = Some(Arc::new(Layer { index: old.index.clone(), step: old.step, x0: old.x0, y0: old.y0, rows: old.rows, cols: old.cols, classes: old.classes.clone(), colors: old.colors.clone(), opacity: self.opacity }));
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
        if !open {
            self.clear();
            self.open = false;
        }
        if std::mem::take(&mut self.site.changed) && self.layer.is_some() && !self.busy() {
            self.status = tr(lang, "The area has changed. Press «Build» to compute it again.").to_string();
        }
        if self.busy() {
            ctx.request_repaint();
        }
    }
}

/// Клетки сетки внутри многоугольника участка, `ring_mask` модуля.
fn ring_mask<T: Copy>(index: &ZoneIndex, grid: &Mosaic<T>, ring: &[(f64, f64)]) -> Vec<bool> {
    let mut out = Vec::with_capacity(grid.rows * grid.cols);
    for r in 0..grid.rows {
        let y = grid.y0 - (r as f64 + 0.5) * grid.step;
        let points: Vec<(f64, f64)> = (0..grid.cols).map(|c| aef::from_utm(index.number, index.south, grid.x0 + (c as f64 + 0.5) * grid.step, y)).collect();
        out.extend(data::site::inside(&points, ring));
    }
    out
}

/// Доли классов строкой итога: «Лес 60%, Вода 40%».
fn class_shares(rows: &[&Class], classes: &[i16]) -> String {
    let counts: Vec<usize> = (0..rows.len()).map(|k| classes.iter().filter(|c| **c == k as i16).count()).collect();
    let total = counts.iter().sum::<usize>().max(1);
    let parts: Vec<String> = rows.iter().zip(&counts).map(|(c, n)| format!("{} {:.0}%", c.name, 100.0 * *n as f64 / total as f64)).collect();
    parts.join(", ")
}

/// Текст строки итога для отказа расчёта по растру, `problem_text`
/// модуля.
pub fn problem_text(lang: Lang, problem: Problem) -> &'static str {
    match problem {
        Problem::Unreadable => tr(lang, "The raster is not read."),
        Problem::NoCrs => tr(lang, "The raster has no coordinate system."),
        Problem::Big => tr(lang, "The grid of the area does not fit in memory, take a larger pixel."),
        Problem::Outside => tr(lang, "The area is outside the raster."),
        Problem::Points => tr(lang, "At least two classes with points on raster cells with data are needed."),
        Problem::Flat => tr(lang, "The raster bands do not change in the area, the classes cannot be told apart."),
    }
}

/// Точка (широта, долгота) на экране, кадровые пиксели, и видна ли.
fn pixel(nav: &Navigator, (lat, lon): (f64, f64)) -> ([f64; 2], bool) {
    let h = nav.pose.terrain.as_ref().map_or(0.0, |t| t(lat, lon));
    let point: DVec3 = nav.camera.ellipsoid.geodetic_to_ecef(lat, lon, h);
    let (px, front) = nav.camera.project(point);
    let facing = (nav.camera.eye - point).dot(point.normalize_or_zero()) > 0.0;
    ([px.x, px.y], front && facing)
}
