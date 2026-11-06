// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Погода на глобусе: поля прогноза GFS и полярное сияние, перенос
//! `ui/weather.py` модуля. Расчёт и адреса - `data::weather`.
//!
//! Строка поля - слой над картами ([`crate::vector::Painter`]). Час
//! прогноза берётся из правого бегунка шкалы времени, без шкалы - из часов.
//! Поле просится так: индекс .idx файла часа, затем его участок заголовком
//! Range. Поля помнятся в памяти, не больше [`MAX_GRIDS`]. Сияние - один
//! прогноз на сейчас, шкала времени его не меняет.
//!
//! Шкала поля стоит в панели шкал, числа в узлах сетки - на глобусе.
//! Над раскраской ветра бегут белые струйки [`crate::currents::Streaks`].

use std::sync::Arc;

use data::flow;
use data::weather::{self, Field, Grid};
use eframe::egui;

use crate::currents::Streaks;
use crate::i18n::{Lang, tr, trf};

/// Наибольший ответ службы, байт.
const ANSWER_BYTES: u64 = 16 << 20;
/// Полей в памяти.
const MAX_GRIDS: usize = 12;
/// Медленнее этого, м/с, струйка ветра бледнеет, `WIND_SLOW` модуля.
const WIND_SLOW: f64 = 2.0;
/// Зерно частиц ветра.
const WIND_SEED: u64 = 20261008;
/// Размер чисел поля, логические пиксели.
const VALUE_FONT: f32 = 13.0;

/// Идущий запрос.
enum Ask {
    Index(f64, u32, Box<dyn net::Request>),
    Field(f64, u32, Box<dyn net::Request>),
    Aurora(Box<dyn net::Request>),
}

/// Строка каталога: ключ, поле, название и подсказка (ключи перевода),
/// источник.
pub struct Row {
    pub key: &'static str,
    pub field: &'static Field,
    pub name: &'static str,
    pub tip: &'static str,
    pub source: &'static str,
}

/// Строки погоды каталога.
pub const ROWS: [Row; 5] = [
    Row { key: "weather_temperature", field: &weather::TEMPERATURE, name: "Air temperature", tip: "Air temperature at a height of 2 m by the NOAA GFS model, grid step 0.25°, about 28 km. The forecast goes up to 16 days ahead, the past is the analysis of the model. The moment is set by the time slider.", source: "NOAA GFS" },
    Row { key: "weather_precipitation", field: &weather::PRECIPITATION, name: "Precipitation, GFS forecast", tip: "Precipitation rate at the moment of the slider by the NOAA GFS model, mm/h. Below 0.1 mm/h the field is transparent.", source: "NOAA GFS" },
    Row { key: "weather_wind", field: &weather::WIND, name: "Wind", tip: "Wind speed at a height of 10 m by the NOAA GFS model, m/s.", source: "NOAA GFS" },
    Row { key: "weather_clouds", field: &weather::CLOUDS, name: "Cloud cover, GFS forecast", tip: "Total cloud cover by the NOAA GFS model, the share of the sky in percent. This is a model calculation, not an image of clouds.", source: "NOAA GFS" },
    Row { key: "weather_aurora", field: &weather::AURORA, name: "Aurora", tip: "The probability of a visible aurora by the OVATION model of NOAA SWPC for the next 30-90 minutes, grid 1°. The forecast is one, for now, the time slider does not change it.", source: "NOAA SWPC" },
];

/// Строка погоды.
pub struct Weather {
    field: &'static Field,
    ask: Option<Ask>,
    /// Самый свежий выложенный выпуск.
    newest: Option<f64>,
    /// Номер пробуемого выпуска при поиске свежего.
    trying: usize,
    grids: Vec<Arc<Grid>>,
    shown: Option<Arc<Grid>>,
    /// Поле сменилось: слой рисуется заново.
    pub changed: bool,
    failed: bool,
    /// Когда пришёл прогноз сияния, секунды UTC.
    fetched: f64,
    /// Участок файла, который просится следующим: выпуск, час, диапазон.
    pending: Option<(f64, u32, (u64, Option<u64>))>,
    /// Струйки ветра, у прочих полей их нет.
    streaks: Option<Streaks>,
}

impl Weather {
    pub fn new(field: &'static Field) -> Self {
        let streaks = (field.key == weather::WIND.key).then(|| Streaks::new(flow::WIND_COUNT, WIND_SEED, flow::WIND_TYPICAL, None, WIND_SLOW, 0.0));
        Self { field, ask: None, newest: None, trying: 0, grids: Vec::new(), shown: None, changed: false, failed: false, fetched: f64::NEG_INFINITY, pending: None, streaks }
    }

    pub fn busy(&self) -> bool {
        self.ask.is_some() || self.pending.is_some()
    }

    pub fn is_aurora(&self) -> bool {
        self.field.key == weather::AURORA.key
    }

    pub fn grid(&self) -> Option<Arc<Grid>> {
        self.shown.clone()
    }

    /// Заголовок и цвета шкалы показанного поля. name - название строки
    /// на языке интерфейса.
    pub fn legend(&self, lang: Lang, name: &str) -> Option<(String, data::gibs::Colormap)> {
        let grid = self.grid()?;
        let map = weather::legend(self.field)?;
        let day = format!("{} UTC", weather::valid_text(&grid));
        Some((trf(lang, "{name}, {units} · {day}", &[("name", name), ("units", tr(lang, self.field.units)), ("day", &day)]), map))
    }

    /// Идут ли струйки: поле ветра пришло, кадры идут по такту частиц.
    pub fn streaming(&self) -> bool {
        self.streaks.is_some() && self.shown.is_some()
    }

    /// Шаг струек. view - точка взгляда, высота глаза и ширина вида в метрах.
    pub fn animate(&mut self, now: f64, view: (f64, f64, f64, f64)) {
        let vectors = self.shown.as_ref().and_then(|g| g.vectors.clone());
        if let (Some(streaks), Some(grid)) = (&mut self.streaks, vectors) {
            streaks.step(&grid, now, view);
        }
    }

    /// Струйки и числа поверх глобуса. view - широта, долгота точки взгляда
    /// и ширина вида в метрах.
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect, ppp: f64, camera: &globe_core::camera::Camera, view: (f64, f64, f64)) {
        if let Some(streaks) = &self.streaks {
            streaks.paint(painter, rect, ppp, camera);
        }
        let Some(grid) = &self.shown else {
            return;
        };
        let (lat, lon, width) = view;
        let e = camera.ellipsoid;
        let font = egui::FontId::proportional(VALUE_FONT);
        for (la, lo, value) in weather::value_marks(grid, self.field, lat, lon, width) {
            let point = e.geodetic_to_ecef(la, lo, 0.0);
            let (q, front) = camera.project(point);
            let at = rect.min + egui::vec2((q.x / ppp) as f32, (q.y / ppp) as f32);
            // Сторона шара за горизонтом отбрасывается.
            if !front || point.dot(camera.eye - point) <= 0.0 || !rect.contains(at) {
                continue;
            }
            let text = weather::value_text(self.field, value);
            for shift in [egui::vec2(-1.0, 0.0), egui::vec2(1.0, 0.0), egui::vec2(0.0, -1.0), egui::vec2(0.0, 1.0)] {
                painter.text(at + shift, egui::Align2::CENTER_CENTER, &text, font.clone(), egui::Color32::from_black_alpha(200));
            }
            painter.text(at, egui::Align2::CENTER_CENTER, &text, font.clone(), egui::Color32::WHITE);
        }
    }

    /// Слой поля для картинок тайлов.
    pub fn painter(&self) -> Option<crate::vector::Painter> {
        let grid = self.grid()?;
        let field = self.field;
        Some(Arc::new(move |key| weather::tile_rgba(field, &grid, key)))
    }

    fn show(&mut self, grid: Arc<Grid>) {
        if self.shown.as_ref().is_none_or(|g| !Arc::ptr_eq(g, &grid)) {
            self.shown = Some(grid);
            self.changed = true;
        }
    }

    /// Шаг: ответы, потом запрос поля момента. utc - сейчас, moment -
    /// момент шкалы или None. Возвращает сообщение пользователю.
    pub fn step(&mut self, lang: Lang, utc: f64, moment: Option<f64>, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Option<String> {
        let mut message = self.answers(lang);
        if let Some((run, hour, (start, end))) = self.pending.take() {
            let ask = request(ANSWER_BYTES);
            let range = end.map_or(format!("bytes={start}-"), |e| format!("bytes={start}-{e}"));
            ask.send(&weather::file_url(run, hour), &[("Range", range.as_str())], None);
            self.ask = Some(Ask::Field(run, hour, ask));
            return message;
        }
        if self.field.key == weather::AURORA.key {
            if self.ask.is_none() && !self.failed && utc - self.fetched >= weather::AURORA_REFRESH {
                let ask = request(ANSWER_BYTES);
                ask.start(weather::AURORA_URL);
                self.ask = Some(Ask::Aurora(ask));
                self.fetched = utc;
            }
            return message;
        }
        if self.ask.is_some() || self.failed {
            return message;
        }
        let moment = moment.unwrap_or(utc);
        let run = match self.newest {
            Some(newest) => weather::source_run(newest, moment),
            None => match weather::candidate_runs(utc).get(self.trying) {
                Some(run) => *run,
                None => {
                    self.failed = true;
                    message = Some(tr(lang, "The forecast did not load, check the network and switch the row on again.").to_string());
                    return message;
                }
            },
        };
        let hour = weather::pick_hour(run, moment);
        if let Some(grid) = self.grids.iter().find(|g| g.run == run && g.hour == hour).cloned() {
            self.show(grid);
            return message;
        }
        let ask = request(ANSWER_BYTES);
        ask.start(&format!("{}.idx", weather::file_url(run, hour)));
        self.ask = Some(Ask::Index(run, hour, ask));
        message
    }

    /// Ответы служб.
    fn answers(&mut self, lang: Lang) -> Option<String> {
        let mut message = None;
        match self.ask.take() {
            Some(Ask::Aurora(ask)) => match ask.take() {
                None => self.ask = Some(Ask::Aurora(ask)),
                Some(answer) => match answer.ok().and_then(|b| weather::parse_aurora(&b)) {
                    Some(grid) => self.show(Arc::new(grid)),
                    None if self.shown.is_none() => {
                        self.failed = true;
                        message = Some(tr(lang, "The forecast did not load, check the network and switch the row on again.").to_string());
                    }
                    None => {}
                },
            },
            Some(Ask::Index(run, hour, ask)) => match ask.take() {
                None => self.ask = Some(Ask::Index(run, hour, ask)),
                Some(answer) => {
                    let range = answer.ok().and_then(|b| weather::byte_range(&String::from_utf8_lossy(&b), self.field, hour));
                    match range {
                        Some(range) => {
                            self.newest = Some(self.newest.map_or(run, |n| n.max(run)));
                            self.pending = Some((run, hour, range));
                        }
                        None if self.newest.is_none() => self.trying += 1,
                        None => self.failed = true,
                    }
                }
            },
            Some(Ask::Field(run, hour, ask)) => match ask.take() {
                None => self.ask = Some(Ask::Field(run, hour, ask)),
                Some(answer) => match answer.ok().and_then(|b| weather::read_grib(b, self.field, run, hour)) {
                    Some(grid) => {
                        let grid = Arc::new(grid);
                        self.grids.push(grid.clone());
                        if self.grids.len() > MAX_GRIDS {
                            self.grids.remove(0);
                        }
                        self.show(grid);
                        message = Some(trf(lang, "Forecast: run {run}, hour {hour}.", &[("run", &data::gibs::Day::from_seconds(run).text()), ("hour", &hour.to_string())]));
                    }
                    None => self.failed = true,
                },
            },
            None => {}
        }
        message
    }
}

/// Место строк погоды: поле на глобусе одно, новое снимает прежнее.
pub const SLOT: u8 = 10;

/// Строка погоды номер n в [`ROWS`] - строка реестра слоёв каталога
/// [`crate::registry`].
pub struct Entry(pub usize);

/// Строки погоды для [`crate::registry::ALL`], по одной на строку [`ROWS`].
pub static ENTRIES: [Entry; 5] = [Entry(0), Entry(1), Entry(2), Entry(3), Entry(4)];

impl Entry {
    fn row(&self) -> Option<&'static Row> {
        ROWS.get(self.0)
    }
}

impl crate::registry::Entry for Entry {
    fn key(&self) -> &'static str {
        self.row().map_or("", |r| r.key)
    }

    fn name(&self) -> &'static str {
        self.row().map_or("", |r| r.name)
    }

    fn tip(&self) -> &'static str {
        self.row().map_or("", |r| r.tip)
    }

    fn category(&self) -> crate::layers::Category {
        crate::layers::Category::Weather
    }

    fn source(&self) -> &'static str {
        self.row().map_or("", |r| r.source)
    }

    fn words(&self) -> &'static [&'static str] {
        &["погода", "прогноз", "температура", "ветер", "сияние", "weather", "forecast", "wind", "aurora"]
    }

    fn legend(&self) -> Option<crate::thumbs::Recipe> {
        self.row().and_then(|r| weather::legend(r.field)).map(|m| crate::thumbs::Recipe::Ramp(m.colors))
    }

    fn slot(&self) -> Option<u8> {
        Some(SLOT)
    }

    fn make(&self, _cx: &crate::registry::Make<'_>) -> (Box<dyn crate::registry::Layer>, Option<String>) {
        let field = self.row().map_or(&weather::TEMPERATURE, |r| r.field);
        (Box::new(Shown { row: self.0, weather: Weather::new(field) }), None)
    }
}

/// Включённая строка погоды: номер строки и поле.
struct Shown {
    row: usize,
    weather: Weather,
}

impl crate::registry::Layer for Shown {
    fn step(&mut self, cx: &crate::registry::Ctx<'_>) -> crate::registry::Stepped {
        let w = &mut self.weather;
        let message = w.step(cx.lang, crate::pointcalc::clock_now(), cx.moment, cx.request);
        let mut tick = w.busy().then_some(100);
        if w.streaming() {
            w.animate(cx.now, cx.view);
            // Струйки бегут - кадры идут по такту частиц.
            tick = Some(u64::from(flow::TICK_MS));
        }
        crate::registry::Stepped { message, painters: std::mem::take(&mut w.changed), tick }
    }

    fn painter(&self) -> Option<crate::vector::Painter> {
        self.weather.painter()
    }

    fn paint(&self, p: &egui::Painter, rect: egui::Rect, ppp: f64, camera: &globe_core::camera::Camera, view: (f64, f64, f64)) {
        self.weather.paint(p, rect, ppp, camera, view);
    }

    fn busy(&self) -> bool {
        self.weather.busy()
    }

    fn attribution(&self) -> Option<(&'static str, &'static str)> {
        Some((if self.weather.is_aurora() { weather::AURORA_ATTRIBUTION.0 } else { weather::ATTRIBUTION.0 }, ""))
    }

    fn legend(&self, lang: Lang) -> Option<(String, data::gibs::Colormap)> {
        self.weather.legend(lang, tr(lang, ROWS.get(self.row)?.name))
    }
}
