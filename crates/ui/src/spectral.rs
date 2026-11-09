// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Минералы по спектру» подменю «Анализ участка», перенос
//! `ui/spectral.py` модуля. Расчёт - `data::spectra`.
//!
//! Просьба автора от 2 ноября 2026 года. Источник - многоканальный растр,
//! открытый в программе, каналы узнаются по подписям. Щелчок по глобусу
//! показывает спектр пикселя и ближайшие эталоны библиотеки USGS,
//! отмеченные эталоны - классы участка по углу между спектрами. Участок,
//! сетка и показ - как у классификации ([`crate::aefclass`]), «OK» кладёт
//! в папку расчёта картинки классов и угла и пишет их файлами GeoTIFF
//! в зоне UTM. Окно есть и у страницы.
//!
//! Модуль 1.31 - своя выборка снимков ASTER участка ([`crate::asterscenes`]):
//! кнопка «Загрузить» открывает все каналы снимка растром программы и
//! ставит его в поле «Растр». В поле - только растры с [`FEW_BANDS`]
//! каналами и больше, группа эталонов по умолчанию - «Минералы».

use std::sync::Arc;

use data::aef::{self, Mosaic, ZoneIndex};
use data::rasterclass::{self, Problem, Source, Warp};
use data::spectra::{self, Classify, Entry};
use eframe::egui;
use globe_core::navigation::{Navigator, ground_under};

use crate::aefcalc::utm_epsg;
use crate::aefclass::{COLORS, Layer, PIXELS, Raster, problem_text, ring_mask};
use crate::analysis::{self, Button, Saved, SiteEdit};
use crate::asterscenes::AsterScenes;
use crate::i18n::{Lang, tr, trf};

/// Участок по умолчанию и наибольший, километры, как у классификации по
/// растру.
const SIDE: f64 = 3.0;
const MOST: f64 = 1000.0;
const OPACITY: f64 = 0.6;
/// Ближайших эталонов под графиком, `NEAREST` модуля.
const NEAREST: usize = 5;
/// Цвета линий графика: ближайшие эталоны - выбор помощника модуля.
const CURVE_COLORS: [[u8; 3]; 5] = [[255, 170, 60], [90, 200, 255], [140, 230, 120], [240, 110, 200], [230, 220, 90]];
/// Шкала угла до эталона, градусы: малый - ярче, `ANGLE_STOPS` модуля.
const ANGLE_STOPS: [(f64, [u8; 3]); 4] = [(0.0, [40, 30, 120]), (5.0, [60, 160, 200]), (10.0, [240, 220, 80]), (20.0, [200, 60, 40])];
/// Время расчёта за кадр, секунды.
const BUDGET: f64 = 0.012;
/// Строк растра и клеток классов за один шаг внутри бюджета кадра.
const WARP_ROWS: usize = 4;
const RUN_CELLS: usize = 4096;
/// Строк списка эталонов не больше, как у модуля.
const MOST_ROWS: usize = 2000;
/// Датчики списка «Каналы» по порядку окна модуля.
const SENSORS: [&str; 3] = ["aster", "sentinel2", "landsat8"];
/// Каналов меньше - предупреждение, спектр слишком груб, `FEW_BANDS`
/// модуля 1.30.1. Выбор помощника модуля, утверждает автор.
pub const FEW_BANDS: usize = 6;
/// Строк списка снимков ASTER на виду.
const SCENE_ROWS: f32 = 96.0;

/// Число с тремя значащими цифрами без лишних нулей, как «{:.3g}» модуля.
pub fn short(value: f64) -> String {
    if !value.is_finite() || value == 0.0 {
        return format!("{value}");
    }
    let digits = (2 - value.abs().log10().floor() as i32).max(0) as usize;
    let text = format!("{value:.digits$}");
    if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.').to_string() } else { text }
}

/// Растр годен для спектра: каналов в файле не меньше [`FEW_BANDS`],
/// `fits` модуля 1.31. Сочетания трёх каналов и рельеф в список не идут.
/// Подписи каналов здесь не читаются: растр с шестью каналами и малым
/// количеством узнанных получает предупреждение в строке каналов.
fn fits(bytes: &[u8]) -> bool {
    rasterclass::feature_bands(bytes).is_none_or(|n| n >= FEW_BANDS)
}

/// Сетка растра на участке: ключ (растр, рамка, пиксель).
type GridKey = (String, String, Option<usize>);
/// Каналы растра, номера эталонов, у которых есть значения датчика, и их
/// значения на каналах.
type Refs = (Vec<usize>, Vec<usize>, Vec<Vec<f64>>);

/// График: длины волн каналов, спектр пикселя и ближайшие эталоны в его
/// масштабе (номер эталона, угол, кривая).
struct Chart {
    waves: Vec<f64>,
    pixel: Vec<f64>,
    near: Vec<(usize, f64, Vec<f64>)>,
}

/// Шаг расчёта.
enum Stage {
    Warp(Box<Warp>),
    Classes { grid: Arc<Mosaic<f32>>, run: Box<Classify> },
}

/// Расчёт: ключ сетки, указатель зоны, эталоны-классы и шаг.
struct Job {
    key: GridKey,
    index: Arc<ZoneIndex>,
    ids: Vec<usize>,
    stage: Stage,
}

/// Готовый расчёт: сетка, углы и эталоны-классы.
struct Done {
    index: Arc<ZoneIndex>,
    grid: Arc<Mosaic<f32>>,
    angles: Vec<f32>,
    ids: Vec<usize>,
    label: String,
}

/// Окно «Минералы по спектру».
pub struct Spectral {
    pub open: bool,
    place: String,
    pub site: SiteEdit,
    /// Растры, открытые в программе, и выбранный.
    pub rasters: Vec<Raster>,
    source: Option<String>,
    /// Разобранный растр: имя и он сам. Разбор идёт кадром позже выбора,
    /// чтобы строка «Читается» успела показаться.
    opened: Option<(String, Arc<Source>)>,
    /// Строка «Читается» показана, разбор - этим кадром.
    reading: bool,
    /// Датчик вручную, None - «Авто».
    sensor: Option<usize>,
    /// Датчик и пары (канал растра, канал датчика), None - не узнаны.
    pairs: Option<spectra::Pairs>,
    level: Option<usize>,
    group: Option<&'static str>,
    search: String,
    /// Эталоны-классы по порядку выбора.
    chosen: Vec<usize>,
    limit: f64,
    chart: Option<Chart>,
    /// Точка последнего щелчка.
    spot: Option<(f64, f64)>,
    status: String,
    job: Option<Job>,
    grid: Option<(GridKey, Arc<ZoneIndex>, Arc<Mosaic<f32>>)>,
    pub layer: Option<Arc<Layer>>,
    done: Option<Done>,
    pub changed: bool,
    pub saved: Option<Saved>,
    /// Поиск и загрузка снимков ASTER, найденные снимки с долей участка
    /// и выбранный, поиск при открытии уже был.
    aster: AsterScenes,
    scenes: Vec<(data::sentinel::Scene, f64)>,
    scene: Option<usize>,
    asked: bool,
    /// Загруженный снимок: окно глобуса открывает его растром программы.
    pub loaded: Option<Raster>,
}

impl Spectral {
    pub fn new(lat: f64, lon: f64, place: String, rasters: Vec<Raster>) -> Self {
        let source = rasters.iter().rev().find(|r| fits(&r.1)).map(|r| r.0.clone());
        Self {
            open: true,
            place,
            site: SiteEdit::new(lat, lon, SIDE, MOST),
            rasters,
            source,
            opened: None,
            reading: false,
            sensor: None,
            pairs: None,
            level: None,
            group: Some("minerals"),
            search: String::new(),
            chosen: Vec::new(),
            limit: spectra::LIMIT,
            chart: None,
            spot: None,
            status: String::new(),
            job: None,
            grid: None,
            layer: None,
            done: None,
            changed: false,
            saved: None,
            aster: AsterScenes::default(),
            scenes: Vec::new(),
            scene: None,
            asked: false,
            loaded: None,
        }
    }

    pub fn set_point(&mut self, lat: f64, lon: f64, place: String) {
        self.place = place;
        self.status.clear();
        self.open = true;
        self.site.move_to(lat, lon);
    }

    pub fn busy(&self) -> bool {
        self.job.is_some() || self.aster.busy()
    }

    pub fn site_edit(&mut self) -> Option<&mut SiteEdit> {
        if self.open { Some(&mut self.site) } else { None }
    }

    /// Снять результат с глобуса, окно остаётся, `drop` общего слоя вида.
    pub fn drop_result(&mut self) {
        self.done = None;
        if self.layer.take().is_some() {
            self.changed = true;
        }
    }

    fn clear(&mut self) {
        self.job = None;
        self.aster.stop();
        self.drop_result();
        self.status.clear();
    }

    /// Разобранный выбранный растр.
    fn source(&self) -> Option<&Arc<Source>> {
        self.opened.as_ref().filter(|o| Some(&o.0) == self.source.as_ref()).map(|o| &o.1)
    }

    /// Строка «Каналы»: датчик и количество совпавших каналов.
    fn bands_text(&self, lang: Lang) -> String {
        let lib = spectra::library();
        let Some(source) = self.source() else {
            return if self.source.is_none() { tr(lang, "No image with six bands or more is open - find and load an ASTER image of the area.").to_string() } else { String::new() };
        };
        match &self.pairs {
            Some((sensor, pairs)) => {
                let text = trf(lang, "{sensor}: bands {n} of {total}", &[("sensor", lib.sensors.get(*sensor).map_or("", |s| s.name.as_str())), ("n", &pairs.len().to_string()), ("total", &source.bands().to_string())]);
                if pairs.len() < FEW_BANDS {
                    format!("{text}. {}", tr(lang, "Few bands, different minerals give the same spectrum - load an ASTER image from the list above."))
                } else {
                    text
                }
            }
            None => tr(lang, "The bands are not recognised by their labels, choose the sensor.").to_string(),
        }
    }

    /// Датчик и пары каналов по подписям растра.
    fn match_bands(&mut self) {
        self.pairs = self.source().and_then(|s| spectra::pairs_for(spectra::library(), &s.labels(), self.sensor));
        self.chart = None;
    }

    /// Найти подходящие снимки ASTER участка.
    fn find_scenes(&mut self, lang: Lang, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        if let Some(problem) = self.site.problem(lang) {
            self.status = problem;
            return;
        }
        self.scenes.clear();
        self.scene = None;
        self.aster.search(lang, &self.site.site.ring, request);
    }

    /// Снимки ASTER: поиск при открытии без подходящего растра, ответы,
    /// готовый снимок - в поле «Растр».
    fn step_aster(&mut self, lang: Lang, now: f64, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        if !self.asked && self.open && !self.site.site.drawing {
            self.asked = true;
            if self.source.is_none() && !self.aster.busy() {
                self.find_scenes(lang, request);
            }
        }
        self.aster.step(lang, now, request);
        if let Some(text) = self.aster.status.take() {
            self.status = text;
        }
        if let Some(list) = self.aster.found.take() {
            self.scene = (!list.is_empty()).then_some(0);
            self.scenes = list;
        }
        if let Some((name, bytes)) = self.aster.loaded.take() {
            let raster: Raster = (name.clone(), Arc::new(bytes));
            self.rasters.retain(|r| r.0 != name);
            self.rasters.push(raster.clone());
            self.loaded = Some(raster);
            self.source = Some(name);
            self.job = None;
            self.drop_result();
        }
    }

    /// Шаг окна: снимки ASTER, разбор выбранного растра и расчёт.
    pub fn step(&mut self, lang: Lang, now: f64, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        self.step_aster(lang, now, request);
        if let Some(name) = self.source.clone()
            && self.opened.as_ref().is_none_or(|o| o.0 != name)
        {
            if !self.reading {
                self.reading = true;
                if self.status.is_empty() {
                    self.status = tr(lang, "Reading the raster…").to_string();
                }
                return;
            }
            self.reading = false;
            let Some(bytes) = self.rasters.iter().find(|r| r.0 == name).map(|r| r.1.clone()) else {
                self.source = None;
                return;
            };
            match Source::read(&bytes, &name) {
                Ok(source) => {
                    self.opened = Some((name, Arc::new(source)));
                    if self.status == tr(lang, "Reading the raster…") {
                        self.status.clear();
                    }
                }
                Err(problem) => {
                    self.opened = None;
                    self.status = problem_text(lang, problem).to_string();
                    self.source = None;
                }
            }
            self.grid = None;
            self.match_bands();
            return;
        }
        let started = web_time::Instant::now();
        let Some(job) = &mut self.job else {
            return;
        };
        match &mut job.stage {
            Stage::Warp(warp) => {
                let Some(source) = self.opened.as_ref().map(|o| o.1.clone()) else {
                    self.job = None;
                    return;
                };
                let mut done = false;
                while !done && started.elapsed().as_secs_f64() < BUDGET {
                    done = source.warp_rows(warp, WARP_ROWS);
                }
                if !done {
                    self.status = trf(lang, "Reading the raster of the area: {done} %.", &[("done", &format!("{:.0}", 100.0 * warp.done() as f64 / warp.grid.rows.max(1) as f64))]);
                    return;
                }
                let Some(Job { key, index, ids, stage: Stage::Warp(warp) }) = self.job.take() else {
                    return;
                };
                let grid = Arc::new(warp.grid);
                self.grid = Some((key.clone(), index.clone(), grid.clone()));
                self.start_classes(lang, key, index, grid, ids);
            }
            Stage::Classes { grid, run } => {
                let mut done = false;
                while !done && started.elapsed().as_secs_f64() < BUDGET {
                    done = run.step(&grid.values, RUN_CELLS);
                }
                if !done {
                    self.status = trf(lang, "Computing the angles between spectra: {done} %.", &[("done", &format!("{:.0}", 100.0 * run.done() as f64 / run.total().max(1) as f64))]);
                    return;
                }
                self.built(lang);
            }
        }
    }

    /// Каналы растра и эталоны-классы на каналах датчика.
    fn refs(&self, ids: &[usize]) -> Option<Refs> {
        let (sensor, pairs) = self.pairs.as_ref()?;
        let bands: Vec<usize> = pairs.iter().map(|p| p.0).collect();
        let columns: Vec<usize> = pairs.iter().map(|p| p.1).collect();
        let (kept, refs) = spectra::library().values(*sensor, ids, &columns);
        Some((bands, kept, refs))
    }

    /// Спектр пикселя под щелчком и ближайшие эталоны, `click` модуля.
    fn click(&mut self, lang: Lang, lat: f64, lon: f64) {
        self.spot = Some((lat, lon));
        let Some(source) = self.source().cloned() else {
            self.status = self.bands_text(lang);
            return;
        };
        let Some((sensor, pairs)) = self.pairs.clone() else {
            self.status = self.bands_text(lang);
            return;
        };
        let values = source.values_at(lat, lon);
        let spectrum: Option<Vec<f64>> = values.and_then(|v| pairs.iter().map(|p| v.get(p.0).copied().filter(|x| x.is_finite())).collect());
        let Some(spectrum) = spectrum else {
            self.status = tr(lang, "The point is outside the raster or has no data.").to_string();
            return;
        };
        let lib = spectra::library();
        let all: Vec<usize> = lib.spectra.iter().map(|e| e.id).collect();
        let Some((_, ids, refs)) = self.refs(&all) else {
            return;
        };
        let near: Vec<(usize, f64, Vec<f64>)> = spectra::nearest(&spectrum, &refs, NEAREST).into_iter().map(|(n, angle)| (ids[n], angle, spectra::scaled(&refs[n], &spectrum))).collect();
        let waves = pairs.iter().map(|p| lib.sensors.get(sensor).and_then(|s| s.um.get(p.1)).copied().unwrap_or(0.0)).collect();
        let nearest = near.first().and_then(|(id, angle, _)| Some((lib.entry(*id)?.name.clone(), format!("{angle:.1}"))));
        let (name, angle) = nearest.unwrap_or_else(|| ("-".to_string(), "-".to_string()));
        self.status = trf(lang, "Spectrum of the point {lat}, {lon}. The nearest reference is {name}, {angle}°.", &[("lat", &format!("{lat:.5}")), ("lon", &format!("{lon:.5}")), ("name", &name), ("angle", &angle)]);
        self.chart = Some(Chart { waves, pixel: spectrum, near });
    }

    /// «Построить», `build` модуля.
    fn build(&mut self, lang: Lang) {
        let Some(name) = self.source.clone() else {
            self.status = tr(lang, "No suitable raster is open.").to_string();
            return;
        };
        let Some(source) = self.source().cloned() else {
            return;
        };
        if self.pairs.is_none() {
            self.status = self.bands_text(lang);
            return;
        }
        let Some((_, ids, _)) = self.refs(&self.chosen.clone()).filter(|r| !r.1.is_empty()) else {
            self.status = tr(lang, "Mark at least one reference.").to_string();
            return;
        };
        if let Some(problem) = self.site.problem(lang) {
            self.status = problem;
            return;
        }
        let (lat, lon) = self.site.site.centre();
        let Some(index) = ZoneIndex::bare(&aef::zone_of(lon, lat)) else {
            return;
        };
        let bbox = self.site.site.bbox();
        let (w, s, e, n) = bbox;
        let key = (name, format!("{w:.6},{s:.6},{e:.6},{n:.6}"), self.level);
        if let Some((_, index, grid)) = self.grid.clone().filter(|g| g.0 == key) {
            self.start_classes(lang, key, index, grid, ids);
            return;
        }
        let side = self.site.site.size();
        let pixel = match self.level.and_then(|l| PIXELS.get(l)) {
            Some(pixel) => *pixel,
            None => rasterclass::auto_pixel(source.pixel(&index, lat, lon), side),
        };
        match source.warp(&index, bbox, pixel) {
            Ok(warp) => {
                self.job = Some(Job { key, index: Arc::new(index), ids, stage: Stage::Warp(Box::new(warp)) });
                self.status = tr(lang, "Reading the raster of the area…").to_string();
            }
            Err(problem) => self.problem(lang, problem),
        }
    }

    /// Отказ снимает прежние классы: «OK» сохранил бы их с новым участком.
    fn problem(&mut self, lang: Lang, problem: Problem) {
        self.job = None;
        self.drop_result();
        self.status = problem_text(lang, problem).to_string();
    }

    /// Начало расчёта углов по готовой сетке участка.
    fn start_classes(&mut self, lang: Lang, key: GridKey, index: Arc<ZoneIndex>, grid: Arc<Mosaic<f32>>, ids: Vec<usize>) {
        let Some((bands, ids, refs)) = self.refs(&ids) else {
            return;
        };
        let inside = ring_mask(&index, &grid, &self.site.site.ring);
        let run = Box::new(Classify::new(grid.rows * grid.cols, grid.depth, bands, refs, self.limit, Some(inside)));
        self.status = tr(lang, "Computing the angles between spectra…").to_string();
        self.job = Some(Job { key, index, ids, stage: Stage::Classes { grid, run } });
    }

    /// Классы готовы, `_built` модуля.
    fn built(&mut self, lang: Lang) {
        let Some(Job { key, index, ids, stage: Stage::Classes { grid, run } }) = self.job.take() else {
            return;
        };
        let cells = run.angles.iter().filter(|a| a.is_finite()).count();
        if cells == 0 {
            self.problem(lang, Problem::Outside);
            return;
        }
        let lib = spectra::library();
        let names: Vec<String> = ids.iter().map(|id| lib.entry(*id).map_or_else(String::new, |e| e.name.clone())).collect();
        let mut counts = vec![0usize; ids.len()];
        for k in &run.kinds {
            if let Some(c) = usize::try_from(*k).ok().and_then(|k| counts.get_mut(k)) {
                *c += 1;
            }
        }
        let mut order: Vec<usize> = (0..ids.len()).filter(|n| counts[*n] > 0).collect();
        order.sort_by_key(|n| std::cmp::Reverse(counts[*n]));
        let parts: Vec<String> = order.iter().take(6).map(|n| format!("{} {:.0}%", names[*n], 100.0 * counts[*n] as f64 / cells as f64)).collect();
        let none = 100.0 * (cells - counts.iter().sum::<usize>()) as f64 / cells as f64;
        let list = if parts.is_empty() { "-".to_string() } else { parts.join(", ") };
        self.status = format!("{} {}", trf(lang, "Classes: {list}. Without a class {none} %.", &[("list", &list), ("none", &format!("{none:.0}"))]), analysis::built_text(lang, crate::pointcalc::clock_now()));
        let colors = (0..ids.len()).map(|n| COLORS[n % COLORS.len()]).collect();
        let layer = Layer { index: index.clone(), step: grid.step, x0: grid.x0, y0: grid.y0, rows: grid.rows, cols: grid.cols, classes: run.kinds.clone(), colors, names, opacity: OPACITY, objects: None };
        let any = counts.iter().any(|c| *c > 0);
        self.layer = Some(Arc::new(layer));
        self.done = any.then(|| Done { index, grid, angles: run.angles.clone(), ids, label: key.0 });
        self.changed = true;
    }

    /// Классы и угол - расчётом для «Моих меток», `export` модуля.
    fn save(&mut self, lang: Lang) {
        let (Some(layer), Some(done)) = (&self.layer, &self.done) else {
            return;
        };
        let g = &done.grid;
        let mercator = data::geotiff::MercatorGrid { x0: g.x0 + g.step / 2.0, y0: g.y0 - g.step / 2.0, step: g.step, cols: g.cols, rows: g.rows };
        let epsg = utm_epsg(&done.index);
        let bytes: Vec<u8> = layer.classes.iter().map(|c| u8::try_from(*c).unwrap_or(data::geotiff::BYTE_NODATA)).collect();
        let classes_tiff = data::geotiff::write_byte_projected(&mercator, epsg, &bytes).unwrap_or_default();
        let angles: Vec<f64> = done.angles.iter().map(|a| if a.is_finite() { f64::from(*a) } else { data::geotiff::NODATA }).collect();
        let angle_tiff = data::geotiff::write_float_projected(&mercator, epsg, &angles).unwrap_or_default();
        let shown = layer.clone();
        let sample = move |points: &[(f64, f64)]| shown.sample(points);
        let lib = spectra::library();
        let values: Vec<String> = done.ids.iter().enumerate().map(|(n, id)| format!("{n} - {}", lib.entry(*id).map_or("", |e| e.name.as_str()))).collect();
        let mut classes = analysis::picture(trf(lang, "Minerals by spectrum {name}", &[("name", &done.label)]), "spectral_classes", layer.bbox(), layer.step, &sample, classes_tiff);
        classes.summary = trf(lang, "Values of the GeoTIFF file: {list}.", &[("list", &values.join(", "))]);
        classes.look = Some(layer.look(""));
        let angle_layer = AngleLayer { done_index: done.index.clone(), grid: done.grid.clone(), angles: done.angles.clone() };
        let sample_angle = move |points: &[(f64, f64)]| angle_layer.sample(points);
        let mut angle = analysis::picture(tr(lang, "Angle to the reference, °").to_string(), "spectral_angle", layer.bbox(), layer.step, &sample_angle, angle_tiff);
        let finite: Vec<f64> = done.angles.iter().filter(|a| a.is_finite()).map(|a| f64::from(*a)).collect();
        angle.summary = analysis::stats_text(lang, tr(lang, "Angle to the reference"), &finite, "°");
        angle.look = Some((analysis::Look::Ramp(ANGLE_STOPS.iter().map(|(v, [r, g, b])| (*v, [*r, *g, *b, 255])).collect()), 1.0));
        if !lib.source.is_empty() {
            angle.summary = format!("{} {}", angle.summary, lib.source).trim().to_string();
        }
        self.saved = Some(Saved { title: tr(lang, "Minerals by spectrum").to_string(), ring: self.site.site.ring.clone(), layers: vec![classes, angle], extra: Vec::new(), session: None });
    }

    /// Мышь над глобусом: участок главнее, щелчок - спектр пикселя.
    /// true - мышь занята.
    pub fn input(&mut self, ctx: &egui::Context, response: &egui::Response, rect: egui::Rect, ppp: f64, nav: &Navigator, lang: Lang) -> bool {
        if !self.open {
            return false;
        }
        if self.site.input(ctx, response, rect, ppp, nav) {
            return true;
        }
        if self.site.site.drawing {
            return false;
        }
        let pos = ctx.input(|i| i.pointer.interact_pos());
        let on_globe = pos.is_some_and(|p| rect.contains(p) && ctx.layer_id_at(p).is_none_or(|l| l == response.layer_id));
        if !(response.clicked() && on_globe) {
            return false;
        }
        let Some(p) = pos else {
            return false;
        };
        let (px, py) = (f64::from(p.x - rect.min.x) * ppp, f64::from(p.y - rect.min.y) * ppp);
        let Some(ground) = ground_under(&nav.camera, px, py, nav.pose.terrain.as_ref()) else {
            return false;
        };
        let (lat, lon, _) = nav.camera.ellipsoid.ecef_to_geodetic(ground);
        self.click(lang, lat, lon);
        true
    }

    /// Участок и точка спектра поверх глобуса.
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect, ppp: f64, nav: &Navigator) {
        if !self.open {
            return;
        }
        self.site.paint(painter, rect, ppp, nav);
        if let Some((lat, lon)) = self.spot {
            let h = nav.pose.terrain.as_ref().map_or(0.0, |t| t(lat, lon));
            let point = nav.camera.ellipsoid.geodetic_to_ecef(lat, lon, h);
            let (px, front) = nav.camera.project(point);
            if front {
                let at = rect.min + egui::vec2((px.x / ppp) as f32, (px.y / ppp) as f32);
                painter.circle(at, 5.0, egui::Color32::WHITE, egui::Stroke::new(1.5, egui::Color32::from_gray(30)));
            }
        }
    }

    /// Строка эталона в списке: флажок с названием и прибором, у ближайших -
    /// угол и цвет кривой.
    fn entry_row(ui: &mut egui::Ui, chosen: &mut Vec<usize>, entry: &Entry, angle: Option<f64>, color: Option<[u8; 3]>) {
        let mut on = chosen.contains(&entry.id);
        let mut text = format!("{}  ·  {}", entry.name, entry.instrument);
        if let Some(a) = angle {
            text = format!("{a:.1}°  {text}");
        }
        let label = match color {
            Some([r, g, b]) => egui::RichText::new(text).color(egui::Color32::from_rgb(r, g, b)),
            None => egui::RichText::new(text),
        };
        let tip = [entry.name.as_str(), entry.instrument.as_str(), entry.kind.as_str()].iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" ");
        if ui.checkbox(&mut on, label).on_hover_text(tip).changed() {
            if on && !chosen.contains(&entry.id) {
                chosen.push(entry.id);
            } else if !on {
                chosen.retain(|id| *id != entry.id);
            }
        }
    }

    /// Окно.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        if !self.open {
            return;
        }
        if let Some(text) = self.site.shared_note(lang) {
            self.status = text.to_string();
        }
        let lib = spectra::library();
        let mut open = true;
        let mut pressed = None;
        let mut find = false;
        let mut take = false;
        let (source, sensor) = (self.source.clone(), self.sensor);
        let bands_text = self.bands_text(lang);
        let pixel_text = self.pixel_text(lang);
        egui::Window::new(tr(lang, "Minerals by spectrum")).id(egui::Id::unique("planetx-spectral")).open(&mut open).collapsible(false).resizable(false).min_width(440.0).show(ctx, |ui| {
            egui::Grid::new("planetx-spectral-form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Point"));
                ui.label(&self.place);
                ui.end_row();
                ui.label(tr(lang, "ASTER images")).on_hover_text(tr(lang, "ASTER images of 2000-2006 over the area suitable for minerals: in daytime, with SWIR bands, cloud cover up to 10 % and data on at least half of the area. On top - the least cloudy."));
                ui.horizontal_top(|ui| {
                    egui::Frame::new().stroke(egui::Stroke::new(1.0, crate::theme::pal().border)).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
                        egui::ScrollArea::vertical().id_salt("planetx-spectral-aster").max_height(SCENE_ROWS).max_width(330.0).auto_shrink([false, true]).show(ui, |ui| {
                            for (n, (s, valid)) in self.scenes.iter().enumerate() {
                                let text = trf(lang, "{when} · cloud cover {cloud} % · area {valid} % · sun {sun}°", &[("when", s.when.get(..10).unwrap_or(&s.when)), ("cloud", &format!("{:.0}", s.cloud)), ("valid", &format!("{valid:.0}")), ("sun", &format!("{:.0}", s.sun.unwrap_or(f64::NAN)))]);
                                let row = ui.selectable_label(self.scene == Some(n), text).on_hover_text(&s.id);
                                if row.clicked() {
                                    self.scene = Some(n);
                                }
                                if row.double_clicked() {
                                    self.scene = Some(n);
                                    take = true;
                                }
                            }
                        });
                    });
                    ui.vertical(|ui| {
                        find = ui.add_enabled(!self.aster.busy(), egui::Button::new(tr(lang, "Find"))).on_hover_text(tr(lang, "Find suitable ASTER images of the area.")).clicked();
                        take |= ui.add_enabled(self.scene.is_some() && !self.aster.loading(), egui::Button::new(tr(lang, "Load"))).on_hover_text(tr(lang, "All bands of the chosen image over the area as a raster of the program. The image takes the Raster field.")).clicked();
                    });
                });
                ui.end_row();
                ui.label(tr(lang, "Raster"));
                egui::ComboBox::from_id_salt("planetx-spectral-raster")
                    .selected_text(self.source.as_deref().unwrap_or("-"))
                    .show_ui(ui, |ui| {
                        for (name, bytes) in &self.rasters {
                            if fits(bytes) {
                                ui.selectable_value(&mut self.source, Some(name.clone()), name);
                            }
                        }
                    })
                    .response
                    .on_hover_text(tr(lang, "A raster in reflectance with six bands or more, for example an ASTER image from the list above. The bands are recognised by their labels, thermal bands do not take part."));
                ui.end_row();
                ui.label(tr(lang, "Bands"));
                let auto = tr(lang, "Auto");
                egui::ComboBox::from_id_salt("planetx-spectral-sensor")
                    .selected_text(self.sensor.and_then(|s| lib.sensors.get(s)).map_or(auto, |s| s.name.as_str()))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.sensor, None, auto);
                        for key in SENSORS {
                            if let Some(n) = lib.sensor(key) {
                                ui.selectable_value(&mut self.sensor, Some(n), &lib.sensors[n].name);
                            }
                        }
                    })
                    .response
                    .on_hover_text(tr(lang, "The sensor to whose bands the references are recomputed. «Auto» recognises it by the wavelengths in the band labels of the raster. If there are no labels, the sensor is chosen by hand, the bands are taken by the numbers in the labels or in order."));
                ui.end_row();
                ui.label("");
                ui.add(egui::Label::new(bands_text).wrap());
                ui.end_row();
                ui.label(tr(lang, "Area|site"));
                self.site.row(ui, lang);
                ui.end_row();
                ui.label(tr(lang, "Pixel"));
                let text = self.level.and_then(|l| PIXELS.get(l)).map_or_else(|| tr(lang, "Auto").to_string(), |p| trf(lang, "{pixel} m", &[("pixel", &format!("{p}"))]));
                egui::ComboBox::from_id_salt("planetx-spectral-level")
                    .selected_text(text)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.level, None, tr(lang, "Auto"));
                        for (level, pixel) in PIXELS.iter().enumerate() {
                            ui.selectable_value(&mut self.level, Some(level), trf(lang, "{pixel} m", &[("pixel", &format!("{pixel}"))]));
                        }
                    })
                    .response
                    .on_hover_text(tr(lang, "The pixel of the grid of the area. «Auto» takes the pixel of the raster if the area has no more than 640 thousand cells."));
                ui.end_row();
                if let Some(text) = pixel_text {
                    ui.label("");
                    ui.add(egui::Label::new(text).wrap());
                    ui.end_row();
                }
                ui.label(tr(lang, "Group"));
                egui::ComboBox::from_id_salt("planetx-spectral-group")
                    .selected_text(group_name(lang, self.group))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.group, None, group_name(lang, None));
                        for g in spectra::GROUPS {
                            ui.selectable_value(&mut self.group, Some(g), group_name(lang, Some(g)));
                        }
                    });
                ui.end_row();
                ui.label(tr(lang, "Search"));
                ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(tr(lang, "Words of the name, for example kaolinite")).desired_width(280.0))
                    .on_hover_text(tr(lang, "The names of the references are in English, as in the USGS library. All words must be in the name."));
                ui.end_row();
            });
            ui.label(tr(lang, "References"));
            let found = spectra::search(lib, &self.search, self.group);
            let shown = found.len().min(MOST_ROWS);
            let row = ui.spacing().interact_size.y;
            egui::Frame::new().stroke(egui::Stroke::new(1.0, crate::theme::pal().border)).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("planetx-spectral-list").max_height(160.0).auto_shrink([false, true]).show_rows(ui, row, shown, |ui, range| {
                    for entry in found.get(range).unwrap_or_default() {
                        Self::entry_row(ui, &mut self.chosen, entry, None, None);
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.label(trf(lang, "References marked: {n}", &[("n", &self.chosen.len().to_string())]));
                if ui.button(tr(lang, "Clear all")).on_hover_text(tr(lang, "Unmark all references.")).clicked() {
                    self.chosen.clear();
                }
            });
            egui::Grid::new("planetx-spectral-limit").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Angle up to"));
                ui.add(egui::DragValue::new(&mut self.limit).range(0.5..=45.0).speed(0.1).fixed_decimals(1).suffix(" °"))
                    .on_hover_text(tr(lang, "The largest angle between the spectra of a cell and a reference. A cell farther from all marked references stays without a class. Smaller - stricter."));
                ui.end_row();
                ui.label(tr(lang, "Opacity"));
                let mut opacity = self.layer.as_ref().map_or(OPACITY, |l| l.opacity);
                if ui.add(egui::Slider::new(&mut opacity, 0.0..=1.0).show_value(false)).on_hover_text(tr(lang, "Opacity of the classes on the globe.")).changed()
                    && let Some(old) = &self.layer
                {
                    self.layer = Some(Arc::new(Layer { index: old.index.clone(), step: old.step, x0: old.x0, y0: old.y0, rows: old.rows, cols: old.cols, classes: old.classes.clone(), colors: old.colors.clone(), names: old.names.clone(), opacity, objects: None }));
                    self.changed = true;
                }
                ui.end_row();
            });
            ui.label(tr(lang, "Spectrum"));
            chart(ui, lang, self.chart.as_ref());
            if let Some(chart) = &self.chart {
                ui.label(tr(lang, "Nearest"));
                for (n, (id, angle, _)) in chart.near.iter().enumerate() {
                    if let Some(entry) = lib.entry(*id) {
                        Self::entry_row(ui, &mut self.chosen, entry, Some(*angle), Some(CURVE_COLORS[n % CURVE_COLORS.len()]));
                    }
                }
            }
            ui.add_space(4.0);
            ui.add(egui::Label::new(tr(lang, "Click the image on the globe - the chart shows the spectrum of the pixel and the nearest references. Marked references become classes of the area.")).wrap());
            ui.add(egui::Label::new(self.site.hint(lang).unwrap_or(&self.status)).wrap());
            ui.add(egui::Label::new(egui::RichText::new(&lib.source).small()).wrap());
            ui.add_space(4.0);
            pressed = analysis::buttons(ui, lang, self.done.is_some() && !self.busy(), tr(lang, "Each cell of the area gets the marked reference with the smallest angle between the spectra."));
        });
        if find {
            self.find_scenes(lang, request);
        }
        if take && let Some((scene, _)) = self.scene.and_then(|n| self.scenes.get(n)).cloned() {
            if let Some(problem) = self.site.problem(lang) {
                self.status = problem;
            } else {
                self.aster.load(lang, &scene, &self.site.site.ring);
            }
        }
        if self.source != source {
            self.job = None;
            self.drop_result();
            self.status.clear();
        } else if self.sensor != sensor {
            self.match_bands();
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
        if self.busy() || self.opened.as_ref().map(|o| &o.0) != self.source.as_ref() {
            ctx.request_repaint();
        }
    }

    /// Строка под полем «Пиксель»: шаг и количество клеток участка.
    fn pixel_text(&self, lang: Lang) -> Option<String> {
        if self.site.problem(lang).is_some() {
            return None;
        }
        let side = self.site.site.size();
        let pixel = match self.level.and_then(|l| PIXELS.get(l)) {
            Some(p) => *p,
            None => {
                let (lat, lon) = self.site.site.centre();
                let index = ZoneIndex::bare(&aef::zone_of(lon, lat))?;
                rasterclass::auto_pixel(self.source()?.pixel(&index, lat, lon), side)
            }
        };
        let cells = (side / pixel).powi(2);
        Some(trf(lang, "Pixel {pixel} m, about {cells} thousand cells.", &[("pixel", &short(pixel)), ("cells", &format!("{:.0}", cells / 1000.0))]))
    }
}

impl analysis::SiteTool for Spectral {
    fn id(&self) -> &'static str {
        "spectral"
    }
    fn legend_title(&self, lang: Lang) -> Option<&'static str> {
        Some(tr(lang, "Minerals by spectrum"))
    }
    fn shown(&self) -> bool {
        self.layer.is_some()
    }
    fn layer(&self) -> Option<Arc<Layer>> {
        self.layer.clone()
    }
    fn painter(&self) -> Option<crate::vector::Painter> {
        self.layer.clone().map(crate::aefclass::painter)
    }
    fn drop_result(&mut self) {
        Spectral::drop_result(self);
    }
}

/// Цвета угла до эталона на сетке участка для картинки «Моих меток».
struct AngleLayer {
    done_index: Arc<ZoneIndex>,
    grid: Arc<Mosaic<f32>>,
    angles: Vec<f32>,
}

impl AngleLayer {
    fn sample(&self, points: &[(f64, f64)]) -> Vec<[u8; 4]> {
        let g = &self.grid;
        points
            .iter()
            .map(|&(lat, lon)| {
                let (x, y) = self.done_index.project(lat, lon);
                let angle = g.cell_of(x, y).and_then(|(r, c)| self.angles.get(r * g.cols + c)).copied().unwrap_or(f32::NAN);
                if angle.is_finite() {
                    let [r, gg, b] = ramp(f64::from(angle));
                    [r, gg, b, 255]
                } else {
                    [0; 4]
                }
            })
            .collect()
    }
}

/// Цвет угла по шкале [`ANGLE_STOPS`].
fn ramp(angle: f64) -> [u8; 3] {
    let mut prev = ANGLE_STOPS[0];
    for stop in ANGLE_STOPS {
        if angle <= stop.0 {
            let span = stop.0 - prev.0;
            let t = if span > 0.0 { ((angle - prev.0) / span).clamp(0.0, 1.0) } else { 0.0 };
            let mix = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as u8;
            return [mix(prev.1[0], stop.1[0]), mix(prev.1[1], stop.1[1]), mix(prev.1[2], stop.1[2])];
        }
        prev = stop;
    }
    prev.1
}

/// Название группы эталонов.
fn group_name(lang: Lang, group: Option<&str>) -> &'static str {
    match group {
        None => tr(lang, "All groups"),
        Some("minerals") => tr(lang, "Minerals"),
        Some("soils") => tr(lang, "Soils and mixtures"),
        Some("vegetation") => tr(lang, "Vegetation"),
        Some("coatings") => tr(lang, "Coatings"),
        Some("artificial") => tr(lang, "Artificial materials"),
        Some(_) => tr(lang, "Water, snow and liquids"),
    }
}

/// График спектра пикселя и ближайших эталонов по длинам волн,
/// `SpectrumChart` модуля.
fn chart(ui: &mut egui::Ui, lang: Lang, chart: Option<&Chart>) {
    let (response, painter) = ui.allocate_painter(egui::vec2(ui.available_width().max(300.0), 150.0), egui::Sense::hover());
    let text = ui.visuals().text_color();
    let rect = egui::Rect::from_min_max(response.rect.min + egui::vec2(34.0, 8.0), response.rect.max - egui::vec2(8.0, 18.0));
    painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, text), egui::StrokeKind::Inside);
    let font = egui::FontId::proportional(11.0);
    let Some(chart) = chart.filter(|c| c.waves.len() >= 2) else {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, tr(lang, "Click the image on the globe"), font, text);
        return;
    };
    let top = chart.near.iter().map(|n| &n.2).chain(std::iter::once(&chart.pixel)).flatten().copied().filter(|v| v.is_finite()).fold(0.0f64, f64::max);
    let top = if top > 0.0 { top } else { 1.0 };
    let (low, high) = chart.waves.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), w| (a.min(*w), b.max(*w)));
    let span = if high > low { high - low } else { 1.0 };
    let mut order: Vec<usize> = (0..chart.waves.len()).collect();
    order.sort_by(|a, b| chart.waves[*a].total_cmp(&chart.waves[*b]));
    let point = |w: f64, v: f64| egui::pos2(rect.left() + ((w - low) / span) as f32 * rect.width(), rect.bottom() - (v.max(0.0) / (top * 1.05)) as f32 * rect.height());
    let line = |values: &[f64]| -> Vec<egui::Pos2> { order.iter().filter_map(|&i| Some(point(chart.waves[i], *values.get(i)?))).collect() };
    for (n, (_, _, curve)) in chart.near.iter().enumerate() {
        let [r, g, b] = CURVE_COLORS[n % CURVE_COLORS.len()];
        painter.add(egui::Shape::line(line(curve), egui::Stroke::new(1.5, egui::Color32::from_rgb(r, g, b))));
    }
    painter.add(egui::Shape::line(line(&chart.pixel), egui::Stroke::new(2.5, text)));
    for w in [low, high] {
        let p = point(w, 0.0);
        painter.text(egui::pos2(p.x, rect.bottom() + 2.0), egui::Align2::CENTER_TOP, format!("{w:.2}"), font.clone(), text);
    }
    painter.text(egui::pos2(response.rect.left() + 2.0, rect.top()), egui::Align2::LEFT_TOP, short(top), font.clone(), text);
    painter.text(egui::pos2(rect.right(), rect.bottom() + 2.0), egui::Align2::RIGHT_TOP, tr(lang, "µm"), font, text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_keeps_three_digits() {
        assert_eq!(short(3.75), "3.75");
        assert_eq!(short(10.0), "10");
        assert_eq!(short(1.0), "1");
        assert_eq!(short(0.123456), "0.123");
        assert_eq!(short(1234.6), "1235");
    }

    #[test]
    fn ramp_follows_stops() {
        assert_eq!(ramp(0.0), [40, 30, 120]);
        assert_eq!(ramp(5.0), [60, 160, 200]);
        assert_eq!(ramp(30.0), [200, 60, 40]);
        assert_eq!(ramp(2.5), [50, 95, 160]);
    }
}
