// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Космоснимки»: сцены Sentinel-2 и Landsat 4-9 по участку
//! у точки и продукт сцены картинкой на поверхности. Перенос части
//! `ui/sentinel.py` модуля, расчёт - `data::sentinel`, чтение окон -
//! [`crate::cogread`].
//!
//! Участок - квадрат у точки со стороной в километрах. Поиск - каталоги
//! STAC: Earth Search у Sentinel-2, Planetary Computer у Landsat, файлы
//! Landsat - по ключу SAS службы ключей. Продукт - окно каналов по рамке
//! участка в системе UTM сцены с шагом `step_for`, отражение, продукт
//! `sentinel::product`, раскраска - растяжение 2-98 % у сочетаний, шкала
//! индекса у индексов. Картинка пересчитывается на широту и долготу
//! и ложится в «Мои метки», папку «Космоснимки», с днём снимка.
//!
//! Ряд по датам и карта тренда - окно [`crate::series::Series`]. Чего
//! у Lite пока нет: ASTER, MODIS и Landsat MSS, облачности над участком
//! в списке, превью, скачивания сцены и файлов GeoTIFF.

use std::collections::BTreeMap;

use eframe::egui;
use serde_json::Value;

use data::sentinel::{self as s2, Mission, Scene};

use crate::cogread::BandRead;
use crate::i18n::{Lang, tr, trf};

/// Сторона участка по умолчанию, км, `SIDE_DEFAULT` модуля.
pub const SIDE_DEFAULT: f64 = 3.0;
/// Облачность сцены в поиске, %, `SCENE_CLOUD_DEFAULT` модуля.
pub const SCENE_CLOUD_DEFAULT: f64 = 80.0;
/// Сцен в ответе за страницу.
const LIMIT: usize = 100;
/// Наибольший ответ каталога, байт.
const ANSWER_BYTES: u64 = 16 << 20;
/// Каналов, читаемых сразу, `BAND_WORKERS` модуля.
const BAND_WORKERS: usize = 4;

/// Спутники окна.
pub const MISSIONS: [Mission; 2] = [Mission::Sentinel2, Mission::Landsat];

fn mission_title(lang: Lang, m: Mission) -> &'static str {
    match m {
        Mission::Landsat => tr(lang, "Landsat 4-9 - 30 m, since 1982"),
        _ => tr(lang, "Sentinel-2 - 10 m, since 2015"),
    }
}

fn mission_credit(lang: Lang, m: Mission) -> &'static str {
    match m {
        Mission::Landsat => tr(lang, "Landsat 4-9 Collection 2 Level-2 (USGS), files and catalogue of Microsoft Planetary Computer. The layers carry the credit \"Landsat image courtesy of the U.S. Geological Survey\"."),
        _ => tr(lang, "Copernicus Sentinel-2 L2A (ESA), files and the Earth Search catalogue by Element 84 on AWS. Layers carry the Contains modified Copernicus Sentinel data notice with the image year."),
    }
}

/// Название продукта, `titles` модуля. У Landsat - без номеров каналов.
pub fn product_title(lang: Lang, name: &str, mission: Mission) -> String {
    let text = match name {
        "natural" => tr(lang, "Natural colour (B4 B3 B2)"),
        "infrared" => tr(lang, "False colour, near infrared (B8 B4 B3)"),
        "agriculture" => tr(lang, "Agriculture (B11 B8 B2)"),
        "swir" => tr(lang, "Shortwave infrared (B12 B8A B4)"),
        "geology" => tr(lang, "Geology (B12 B11 B2)"),
        "urban" => tr(lang, "Urban (B12 B11 B4)"),
        "ndvi" => tr(lang, "NDVI - vegetation"),
        "ndwi" => tr(lang, "NDWI - open water"),
        "ndmi" => tr(lang, "NDMI - vegetation moisture"),
        "nbr" => tr(lang, "NBR - burned areas"),
        "ndsi" => tr(lang, "NDSI - snow"),
        "lst" => tr(lang, "Surface temperature, °C"),
        other => return other.to_string(),
    };
    if mission == Mission::Landsat { text.split(" (").next().unwrap_or(text).to_string() } else { text.to_string() }
}

/// Готовый снимок: картинка PNG, рамка (запад, юг, восток, север),
/// название, день.
pub struct Built {
    pub png: Vec<u8>,
    pub bbox: [f64; 4],
    pub name: String,
    pub day: String,
    pub credit: String,
}

/// Сборка продукта.
struct Job {
    scene: Box<Scene>,
    product: String,
    xy: Vec<(f64, f64)>,
    bbox: [f64; 4],
    res: f64,
    clear: bool,
    /// Каналы: ключ и чтение. Ещё не начатые - без чтения.
    bands: Vec<(String, Option<BandRead>)>,
}

pub struct Sentinel {
    pub open: bool,
    point: (f64, f64),
    mission: Mission,
    side: f64,
    start: (i64, u32, u32),
    end: (i64, u32, u32),
    cloud: f64,
    product: String,
    clear: bool,
    search: Option<(usize, Box<dyn net::Request>)>,
    scenes: Vec<Scene>,
    selected: Option<usize>,
    token: Option<(String, f64)>,
    token_ask: Option<Box<dyn net::Request>>,
    job: Option<Job>,
    status: String,
    generation: usize,
    pub built: Option<Built>,
    /// Окно «Ряд по датам» по найденным сценам.
    pub series: Option<crate::series::Series>,
}

impl Sentinel {
    pub fn new(lat: f64, lon: f64, view_width: f64, now_unix: f64) -> Self {
        let today = crate::pointcalc::civil_from_days((now_unix / 86400.0).floor() as i64);
        let side = if view_width > 0.0 { s2::side_for_view(view_width).min(SIDE_DEFAULT.max(s2::side_for_view(view_width))) } else { SIDE_DEFAULT };
        Self {
            open: true,
            point: (lat, lon),
            mission: Mission::Sentinel2,
            side,
            start: (today.0 - 1, today.1, today.2.min(28)),
            end: today,
            cloud: SCENE_CLOUD_DEFAULT,
            product: "natural".into(),
            clear: false,
            search: None,
            scenes: Vec::new(),
            selected: None,
            token: None,
            token_ask: None,
            job: None,
            status: String::new(),
            generation: 0,
            built: None,
            series: None,
        }
    }

    /// Новая точка в открытом окне.
    pub fn show_point(&mut self, lat: f64, lon: f64) {
        self.point = (lat, lon);
        self.open = true;
        self.scenes.clear();
        self.selected = None;
    }

    /// Участок окна - рамка метки: середина и наибольший размер в метрах,
    /// `show_place` модуля.
    pub fn show_place(&mut self, lat: f64, lon: f64, size: f64) {
        self.side = (size / 1000.0).clamp(s2::SIDE_MIN, s2::SIDE_MAX);
        self.show_point(lat, lon);
    }

    pub fn busy(&self) -> bool {
        self.search.is_some() || self.job.is_some() || self.token_ask.is_some() || self.series.as_ref().is_some_and(crate::series::Series::busy)
    }

    /// Окно снимков или окно ряда открыто.
    pub fn shown(&self) -> bool {
        self.open || self.series.as_ref().is_some_and(|s| s.open)
    }

    /// Шаг ряда по датам: ключ SAS для файлов Landsat, чтение сцен,
    /// щелчок по точке - сцена в списке.
    fn step_series(&mut self, lang: Lang, now: f64, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        let token = if self.token_ok(now) { self.token.as_ref().map(|(t, _)| t.clone()) } else { None };
        let Some(series) = &mut self.series else {
            return;
        };
        if series.needs_key() && token.is_none() && self.token_ask.is_none() {
            let ask = request(1 << 20);
            ask.start(&s2::sas_url(series.mission(), series.first_href().as_deref()));
            self.token_ask = Some(ask);
        }
        series.step(lang, token.as_deref(), request);
        if let Some(id) = series.picked.take()
            && let Some(n) = self.scenes.iter().position(|s| s.id == id)
        {
            self.selected = Some(n);
        }
        if !series.open && !series.busy() && series.built.is_none() {
            self.series = None;
        }
    }

    /// Кольцо участка (широта, долгота).
    fn ring(&self) -> Vec<(f64, f64)> {
        s2::square(self.point.0, self.point.1, self.side * 1000.0)
    }

    fn date(d: (i64, u32, u32)) -> String {
        format!("{:04}-{:02}-{:02}", d.0, d.1, d.2)
    }

    fn start_search(&mut self, lang: Lang, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        self.generation += 1;
        self.scenes.clear();
        self.selected = None;
        let body = s2::search_body(&s2::geojson(&self.ring()), &Self::date(self.start), &Self::date(self.end), self.cloud, LIMIT, self.mission);
        let ask = request(ANSWER_BYTES);
        let text = body.to_string();
        ask.send(s2::search_url(self.mission), &s2::SEARCH_HEADERS, Some(text.as_bytes()));
        self.search = Some((self.generation, ask));
        self.status = tr(lang, "Searching scenes…").to_string();
    }

    /// Ключ SAS Planetary Computer свежий.
    fn token_ok(&self, now: f64) -> bool {
        self.token.as_ref().is_some_and(|(_, at)| now - at < s2::SAS_LIFE as f64)
    }

    /// Начать сборку продукта выбранной сцены.
    fn start_build(&mut self, lang: Lang) {
        let Some(scene) = self.selected.and_then(|i| self.scenes.get(i)).cloned() else {
            return;
        };
        let Some(epsg) = scene.epsg else {
            self.status = tr(lang, "The scene has no UTM system.").to_string();
            return;
        };
        let Ok(xy) = s2::ring_utm(&self.ring(), epsg) else {
            self.status = tr(lang, "The area is outside the UTM zone of the scene.").to_string();
            return;
        };
        let products = [self.product.as_str()];
        let res = s2::step_for(&xy, scene.mission, &products);
        let Some(bbox) = s2::bounds(&xy, res) else {
            return;
        };
        let mut keys: Vec<String> = s2::needed(&products, scene.mission, false).into_iter().filter(|k| scene.assets.contains(k) || !s2::OPTIONAL_BANDS.contains(&k.as_str())).collect();
        if self.clear {
            keys.push(s2::cloud_asset(scene.mission).0.to_string());
        }
        self.status = tr(lang, "Reading the bands…").to_string();
        self.job = Some(Job { scene: Box::new(scene), product: self.product.clone(), xy, bbox, res, clear: self.clear, bands: keys.into_iter().map(|k| (k, None)).collect() });
    }

    /// Шаг: поиск, ключ SAS, чтение каналов, сборка.
    pub fn step(&mut self, lang: Lang, now: f64, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        if let Some((generation, ask)) = &self.search
            && let Some(answer) = ask.take()
        {
            let generation = *generation;
            self.search = None;
            if generation == self.generation {
                match answer.map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice::<Value>(&b).map_err(|e| e.to_string())) {
                    Ok(page) => {
                        let (scenes, _) = s2::parse_page(&page);
                        self.scenes = scenes.into_iter().take(s2::MAX_SCENES).collect();
                        self.status = trf(lang, "Scenes found: {count}", &[("count", &self.scenes.len().to_string())]);
                    }
                    Err(e) => self.status = trf(lang, "The catalogue did not answer: {error}", &[("error", &e)]),
                }
            }
        }
        if let Some(ask) = &self.token_ask
            && let Some(answer) = ask.take()
        {
            self.token_ask = None;
            match answer.ok().and_then(|b| s2::parse_token(&b)) {
                Some(t) => self.token = Some((t, now)),
                None => {
                    self.status = tr(lang, "Planetary Computer did not give a key for the Landsat files.").to_string();
                    self.job = None;
                }
            }
        }
        self.step_series(lang, now, request);
        let Some(mut job) = self.job.take() else {
            return;
        };
        // Файлы Landsat - по ключу SAS.
        if s2::needs_key(job.scene.mission) && !self.token_ok(now) {
            if self.token_ask.is_none() {
                let first = job.bands.first().and_then(|(k, _)| job.scene.assets.get(k)).and_then(|a| a.href.first()).map(str::to_string);
                let ask = request(1 << 20);
                ask.start(&s2::sas_url(job.scene.mission, first.as_deref()));
                self.token_ask = Some(ask);
            }
            self.job = Some(job);
            return;
        }
        let sas = self.token.as_ref().map_or(String::new(), |(t, _)| t.clone());
        let cloud_key = s2::cloud_asset(job.scene.mission).0;
        let mut running = job.bands.iter().filter(|(_, r)| r.as_ref().is_some_and(|r| r.result().is_none() && r.error().is_none())).count();
        for (key, read) in &mut job.bands {
            match read {
                Some(r) => r.step(request),
                None if running < BAND_WORKERS => {
                    let Some(asset) = job.scene.assets.get(key) else {
                        self.status = trf(lang, "The scene has no band {band}.", &[("band", key)]);
                        return;
                    };
                    let href = if s2::needs_key(job.scene.mission) { s2::sign(&asset.href, &sas) } else { asset.href.clone() };
                    let Some(href) = href.first().map(str::to_string) else {
                        return;
                    };
                    *read = Some(BandRead::new(&href, job.bbox, job.res, key == cloud_key, request));
                    running += 1;
                }
                None => {}
            }
        }
        if let Some(e) = job.bands.iter().find_map(|(k, r)| r.as_ref().and_then(BandRead::error).map(|e| format!("{k}: {e}"))) {
            self.status = trf(lang, "The bands were not read: {error}", &[("error", &e)]);
            return;
        }
        let done = job.bands.iter().filter(|(_, r)| r.as_ref().is_some_and(|r| r.result().is_some())).count();
        if done < job.bands.len() {
            self.status = trf(lang, "Reading the bands: {done} of {total}", &[("done", &done.to_string()), ("total", &job.bands.len().to_string())]);
            self.job = Some(job);
            return;
        }
        self.finish(lang, &job);
    }

    /// Каналы прочитаны: продукт, раскраска, пересчёт на широту и долготу.
    fn finish(&mut self, lang: Lang, job: &Job) {
        let scene = &job.scene;
        let cloud_key = s2::cloud_asset(scene.mission).0;
        let Some(mut inside) = s2::mask(&job.xy, job.bbox, job.res) else {
            self.status = tr(lang, "The product was not built.").to_string();
            return;
        };
        let mut bands: s2::Bands = BTreeMap::new();
        for (key, read) in &job.bands {
            let Some(raw) = read.as_ref().and_then(BandRead::result) else {
                continue;
            };
            if key == cloud_key && job.clear {
                let clear = s2::clear_mask(scene, raw);
                for (m, c) in inside.data_mut().iter_mut().zip(clear.data()) {
                    *m = *m && *c;
                }
                continue;
            }
            if let Some(asset) = scene.assets.get(key) {
                bands.insert(key.clone(), s2::reflectance(raw, asset));
            }
        }
        for values in bands.values_mut() {
            for (v, m) in values.data_mut().iter_mut().zip(inside.data()) {
                if !*m {
                    *v = f32::NAN;
                }
            }
        }
        let Some(product) = s2::product(&job.product, &bands, scene.mission) else {
            self.status = tr(lang, "The product was not built.").to_string();
            return;
        };
        let Some(rgba) = colour(&job.product, &product) else {
            self.status = tr(lang, "The area has no data in this scene.").to_string();
            return;
        };
        let (rows, cols) = inside.shape();
        let Some((png, bbox)) = to_latlon(&rgba, rows, cols, job.bbox, job.res, &self.ring(), scene.epsg.unwrap_or(32601)) else {
            self.status = tr(lang, "The product was not built.").to_string();
            return;
        };
        let day = scene.when.get(..10).unwrap_or(&scene.when).to_string();
        let name = format!("{day} {} - {}", scene.tile, product_title(lang, &job.product, scene.mission));
        self.status = trf(lang, "The image is on the globe: {name}.", &[("name", &name)]);
        self.built = Some(Built { png, bbox, name, day, credit: s2::credit(scene) });
    }

    /// Окно.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        if let Some(series) = &mut self.series {
            series.show(ctx, lang);
        }
        if !self.open {
            return;
        }
        let mut open = true;
        let mut find = false;
        let mut build = false;
        let mut series = false;
        egui::Window::new(tr(lang, "Satellite images")).open(&mut open).default_size([640.0, 560.0]).show(ctx, |ui| {
            egui::Grid::new("planetx-sentinel").num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Satellite"));
                egui::ComboBox::from_id_salt("planetx-sentinel-mission").selected_text(mission_title(lang, self.mission)).show_ui(ui, |ui| {
                    for m in MISSIONS {
                        if ui.selectable_value(&mut self.mission, m, mission_title(lang, m)).clicked() {
                            self.scenes.clear();
                            self.selected = None;
                            if !s2::products_of(m).contains(&self.product.as_str()) {
                                self.product = "natural".into();
                            }
                        }
                    }
                });
                ui.label(tr(lang, "Area side"));
                ui.add(egui::DragValue::new(&mut self.side).range(s2::SIDE_MIN..=s2::SIDE_MAX).speed(0.1).fixed_decimals(1).suffix(tr(lang, " km")));
                ui.end_row();
                ui.label(tr(lang, "Acquired from"));
                crate::pointcalc::date_edit(ui, &mut self.start).on_hover_text(tr(lang, "The first acquisition day searched."));
                ui.label(tr(lang, "to"));
                crate::pointcalc::date_edit(ui, &mut self.end).on_hover_text(tr(lang, "The last acquisition day searched."));
                ui.end_row();
                ui.label(tr(lang, "Scene cloud cover up to"));
                ui.add(egui::DragValue::new(&mut self.cloud).range(0.0..=100.0).speed(1.0).suffix(" %"));
                if ui.add_enabled(self.search.is_none(), egui::Button::new(tr(lang, "Find"))).clicked() {
                    find = true;
                }
                ui.end_row();
            });
            ui.separator();
            egui::ScrollArea::vertical().max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
                egui::Grid::new("planetx-sentinel-scenes").num_columns(3).striped(true).spacing([14.0, 3.0]).show(ui, |ui| {
                    ui.strong(tr(lang, "Date, UTC"));
                    ui.strong(tr(lang, "Scene"));
                    ui.strong(tr(lang, "Tile"));
                    ui.end_row();
                    for (n, s) in self.scenes.iter().enumerate() {
                        let when = s.when.get(..16).unwrap_or(&s.when).replace('T', " ");
                        if ui.selectable_label(self.selected == Some(n), when).clicked() {
                            self.selected = Some(n);
                        }
                        ui.label(format!("{:.0} %", s.cloud));
                        ui.label(&s.tile);
                        ui.end_row();
                    }
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(tr(lang, "Product"));
                egui::ComboBox::from_id_salt("planetx-sentinel-product").selected_text(product_title(lang, &self.product, self.mission)).show_ui(ui, |ui| {
                    for p in s2::products_of(self.mission) {
                        if matches!(p, "all_bands" | "mss_bands") {
                            continue;
                        }
                        ui.selectable_value(&mut self.product, p.to_string(), product_title(lang, p, self.mission));
                    }
                });
                ui.checkbox(&mut self.clear, tr(lang, "Clouds and shadows empty"));
            });
            ui.horizontal(|ui| {
                if ui.add_enabled(self.selected.is_some() && self.job.is_none(), egui::Button::new(tr(lang, "On the globe"))).on_hover_text(tr(lang, "The product of the chosen scene over the area as a picture on the surface in My Places, folder Satellite images, with the day of the image.")).clicked() {
                    build = true;
                }
                if ui.add_enabled(!self.scenes.is_empty() && !data::series::products(self.mission).is_empty(), egui::Button::new(tr(lang, "Series by dates…"))).on_hover_text(tr(lang, "A chart of the value over the area by the found scenes and the trend map.")).clicked() {
                    series = true;
                }
            });
            ui.label(&self.status);
            ui.add(egui::Label::new(egui::RichText::new(mission_credit(lang, self.mission)).small()).wrap());
        });
        if find {
            self.start_search(lang, request);
        }
        if build {
            self.start_build(lang);
        }
        if series {
            let area = format!("{:.4}, {:.4}, {:.1}{}", self.point.0, self.point.1, self.side, tr(lang, " km"));
            let product = data::series::products(self.mission).contains(&self.product.as_str()).then_some(self.product.as_str());
            self.series = Some(crate::series::Series::new(self.mission, self.scenes.clone(), self.ring(), area, product));
        }
        if !open {
            self.open = false;
            self.job = None;
            self.search = None;
        }
    }
}

/// Цвет шкалы на значении: линейно между узлами, за краями - крайние.
fn ramp(stops: &[(f64, [u8; 3])], v: f64) -> [u8; 3] {
    let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
        return [0; 3];
    };
    if v <= first.0 {
        return first.1;
    }
    for w in stops.windows(2) {
        let (a, b) = (w[0], w[1]);
        if v <= b.0 {
            let t = if b.0 > a.0 { (v - a.0) / (b.0 - a.0) } else { 0.0 };
            return std::array::from_fn(|k| (f64::from(a.1[k]) + t * (f64::from(b.1[k]) - f64::from(a.1[k]))).round() as u8);
        }
    }
    last.1
}

/// Продукт в RGBA строк сверху вниз: сочетание - растяжение 2-98 %
/// каждого канала, индекс и температура - шкала. NaN - прозрачно. None -
/// значений нет.
fn colour(name: &str, product: &s2::Product) -> Option<Vec<u8>> {
    match product {
        s2::Product::Stack(bands) => {
            let [r, g, b] = bands.get(..3)? else {
                return None;
            };
            let ranges: Vec<(f64, f64)> = [r, g, b].iter().map(|band| s2::stretch(band, 2.0, 98.0)).collect();
            let mut out = Vec::with_capacity(r.data().len() * 4);
            let mut any = false;
            for i in 0..r.data().len() {
                let vals = [r.data().get(i), g.data().get(i), b.data().get(i)];
                if vals.iter().any(|v| v.is_none_or(|v| !v.is_finite())) {
                    out.extend_from_slice(&[0, 0, 0, 0]);
                    continue;
                }
                any = true;
                for (v, (lo, hi)) in vals.iter().zip(&ranges) {
                    let v = f64::from(v.copied().unwrap_or(0.0));
                    out.push((((v - lo) / (hi - lo)).clamp(0.0, 1.0) * 255.0).round() as u8);
                }
                out.push(255);
            }
            any.then_some(out)
        }
        s2::Product::Single(grid) => {
            let scale = if name == "lst" { s2::TEMPERATURE } else { s2::index(name)?.scale };
            let mut out = Vec::with_capacity(grid.data().len() * 4);
            let mut any = false;
            for v in grid.data() {
                if !v.is_finite() {
                    out.extend_from_slice(&[0, 0, 0, 0]);
                    continue;
                }
                any = true;
                let v = f64::from(*v).clamp(scale.range.0, scale.range.1);
                let c = ramp(scale.stops, v);
                out.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
            any.then_some(out)
        }
    }
}

/// Картинка в UTM - на сетку широты и долготы участка: ближайший пиксель.
/// Возвращает PNG и рамку (запад, юг, восток, север).
pub(crate) fn to_latlon(rgba: &[u8], rows: usize, cols: usize, bbox: [f64; 4], res: f64, ring: &[(f64, f64)], epsg: u32) -> Option<(Vec<u8>, [f64; 4])> {
    let south = ring.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let north = ring.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let west = ring.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let east = ring.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    let (w, h) = (cols.max(1), rows.max(1));
    let mut points = Vec::with_capacity(w * h);
    for r in 0..h {
        let lat = north - (r as f64 + 0.5) / h as f64 * (north - south);
        for c in 0..w {
            points.push((lat, west + (c as f64 + 0.5) / w as f64 * (east - west)));
        }
    }
    let xy = s2::ring_utm(&points, epsg).ok()?;
    let mut out = vec![0u8; w * h * 4];
    for (n, (x, y)) in xy.iter().enumerate() {
        let c = ((x - bbox[0]) / res).floor();
        let r = ((bbox[3] - y) / res).floor();
        if c < 0.0 || r < 0.0 || c as usize >= cols || r as usize >= rows {
            continue;
        }
        let src = (r as usize * cols + c as usize) * 4;
        if let (Some(s), Some(d)) = (rgba.get(src..src + 4), out.get_mut(n * 4..n * 4 + 4)) {
            d.copy_from_slice(s);
        }
    }
    let mut png = Vec::new();
    image::RgbaImage::from_raw(w as u32, h as u32, out)?.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some((png, [west, south, east, north]))
}
