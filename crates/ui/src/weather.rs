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

use std::sync::Arc;

use data::weather::{self, Field, Grid};

use crate::i18n::{Lang, tr, trf};

/// Наибольший ответ службы, байт.
const ANSWER_BYTES: u64 = 16 << 20;
/// Полей в памяти.
const MAX_GRIDS: usize = 12;

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
}

impl Weather {
    pub fn new(field: &'static Field) -> Self {
        Self { field, ask: None, newest: None, trying: 0, grids: Vec::new(), shown: None, changed: false, failed: false, fetched: f64::NEG_INFINITY, pending: None }
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
