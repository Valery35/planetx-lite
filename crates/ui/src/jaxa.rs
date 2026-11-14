// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Наборы JAXA Earth строками каталога, решение автора от 9 ноября
//! 2026 года. Наборы, адреса и пересчёт - `data::jaxa`.
//!
//! Строка - слой над картами, как поле погоды ([`crate::weather`]).
//! Сначала просится описание набора с датами. Месяц берётся из правого
//! бегунка шкалы времени, без шкалы - последний. Оба полушария уровня 0
//! читаются окнами COG ([`crate::cogread::BandRead`]) на сетку 0.1°.
//! Сетки дат помнятся в памяти, не больше [`MAX_GRIDS`].

use std::sync::Arc;

use data::jaxa::{self, Set};
use data::weather::{self, Grid};

use crate::cogread::BandRead;
use crate::i18n::{Lang, tr, trf};
use crate::layers::Category;

/// Наибольший ответ описания набора, байт.
const ANSWER_BYTES: u64 = 4 << 20;
/// Сеток дат в памяти. Сетка мира - около 26 МБ.
const MAX_GRIDS: usize = 3;

/// Строка каталога: набор, название и подсказка (ключи перевода),
/// раздел каталога.
pub struct Row {
    pub set: &'static Set,
    pub name: &'static str,
    pub tip: &'static str,
    pub category: Category,
}

/// Строки JAXA каталога, по одной на набор [`jaxa::SETS`].
pub const ROWS: [Row; 7] = [
    Row { set: &jaxa::LAND_TEMPERATURE, name: "Land surface temperature, JAXA", tip: "Daytime land surface temperature by the GCOM-C SGLI satellite of JAXA, monthly mean, grid 0.1°. The month is set by the time slider. Without the slider the latest month is shown.", category: Category::Weather },
    Row { set: &jaxa::SEA_TEMPERATURE, name: "Sea surface temperature, JAXA", tip: "Daytime sea surface temperature by the GCOM-C SGLI satellite of JAXA, monthly mean, grid 0.1°. The month is set by the time slider.", category: Category::Water },
    Row { set: &jaxa::NDVI, name: "Vegetation index NDVI, JAXA", tip: "Normalized difference vegetation index by the GCOM-C SGLI satellite of JAXA, monthly mean, grid 0.1°. Dense vegetation is dark green. The month is set by the time slider.", category: Category::Life },
    Row { set: &jaxa::CHLOROPHYLL, name: "Chlorophyll-a in the sea, JAXA", tip: "Chlorophyll-a concentration in the sea water by the GCOM-C SGLI satellite of JAXA, mg/m³, monthly mean. The scale goes by powers of ten. The month is set by the time slider.", category: Category::Water },
    Row { set: &jaxa::PRECIPITATION, name: "Precipitation, JAXA GSMaP", tip: "Mean precipitation rate of the month by the GSMaP satellite mosaic of JAXA, mm/h. The series starts in March 2000. The month is set by the time slider.", category: Category::Weather },
    Row { set: &jaxa::SOIL_MOISTURE, name: "Soil moisture, JAXA", tip: "Soil moisture of the top layer by the GCOM-W AMSR2 satellite of JAXA, percent of the volume, monthly mean. The series starts in July 2012. The month is set by the time slider.", category: Category::Soil },
    Row { set: &jaxa::AEROSOL, name: "Aerosol optical depth, JAXA", tip: "Aerosol optical depth at 500 nm by MODIS of the Terra and Aqua satellites, JAXA JASMES processing, monthly mean. Smoke, dust and haze raise the depth. The month is set by the time slider.", category: Category::Gases },
];

/// Место строк JAXA: поле на глобусе одно, новое снимает прежнее.
pub const SLOT: u8 = 11;

/// Чтение двух полушарий одной даты.
struct Halves {
    date: String,
    west: BandRead,
    east: BandRead,
}

/// Включённая строка JAXA.
struct Shown {
    row: usize,
    set: &'static Set,
    /// Запрос описания набора с датами.
    asking: Option<Box<dyn net::Request>>,
    dates: Vec<String>,
    reading: Option<Halves>,
    grids: Vec<(String, Arc<Grid>)>,
    shown: Option<(String, Arc<Grid>)>,
    changed: bool,
    failed: bool,
}

impl Shown {
    fn new(row: usize, set: &'static Set, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Self {
        let ask = request(ANSWER_BYTES);
        ask.start(&jaxa::collection_url(set));
        Self { row, set, asking: Some(ask), dates: Vec::new(), reading: None, grids: Vec::new(), shown: None, changed: false, failed: false }
    }

    fn fail(&mut self, lang: Lang) -> Option<String> {
        self.failed = true;
        self.reading = None;
        Some(tr(lang, "The JAXA data did not load, check the network and switch the row on again.").to_string())
    }

    fn show(&mut self, date: &str, grid: Arc<Grid>) {
        if self.shown.as_ref().is_none_or(|(_, g)| !Arc::ptr_eq(g, &grid)) {
            self.shown = Some((date.to_string(), grid));
            self.changed = true;
        }
    }

    /// Шаг: описание набора, чтение полушарий, выбор даты по моменту.
    fn advance(&mut self, lang: Lang, moment: Option<f64>, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Option<String> {
        if self.failed {
            return None;
        }
        if let Some(ask) = self.asking.take() {
            match ask.take() {
                None => {
                    self.asking = Some(ask);
                    return None;
                }
                Some(answer) => {
                    self.dates = answer.ok().map(|b| jaxa::parse_dates(&String::from_utf8_lossy(&b))).unwrap_or_default();
                    if self.dates.is_empty() {
                        return self.fail(lang);
                    }
                }
            }
        }
        let mut message = None;
        if let Some(h) = &mut self.reading {
            h.west.step(request);
            h.east.step(request);
            if h.west.error().is_some() || h.east.error().is_some() {
                return self.fail(lang);
            }
            if let (Some(w), Some(e)) = (h.west.result(), h.east.result()) {
                let date = h.date.clone();
                let grid = jaxa::date_seconds(&date).and_then(|s| jaxa::world_grid(self.set, w, e, s));
                self.reading = None;
                let Some(grid) = grid else {
                    return self.fail(lang);
                };
                let grid = Arc::new(grid);
                self.grids.push((date.clone(), grid.clone()));
                if self.grids.len() > MAX_GRIDS {
                    self.grids.remove(0);
                }
                self.show(&date, grid);
                message = Some(trf(lang, "JAXA Earth: {date}.", &[("date", &date)]));
            }
        }
        let Some(date) = jaxa::pick_date(&self.dates, moment).map(str::to_string) else {
            return message;
        };
        if let Some((_, grid)) = self.grids.iter().find(|(d, _)| *d == date).cloned() {
            self.show(&date, grid);
            return message;
        }
        // Бегунок ушёл на другой месяц - прежнее чтение бросается.
        if self.reading.as_ref().is_none_or(|h| h.date != date) {
            let read = |east: bool| BandRead::new(&jaxa::half_url(self.set, &date, east), if east { [0.0, -90.0, 180.0, 90.0] } else { [-180.0, -90.0, 0.0, 90.0] }, jaxa::STEP, true, request);
            self.reading = Some(Halves { west: read(false), east: read(true), date });
        }
        message
    }
}

impl crate::registry::Layer for Shown {
    fn step(&mut self, cx: &crate::registry::Ctx<'_>) -> crate::registry::Stepped {
        let message = self.advance(cx.lang, cx.moment, cx.request);
        let tick = self.busy().then_some(100);
        crate::registry::Stepped { message, painters: std::mem::take(&mut self.changed), tick, lines: false }
    }

    fn painter(&self) -> Option<crate::vector::Painter> {
        let (_, grid) = self.shown.clone()?;
        let set = self.set;
        Some(Arc::new(move |key| weather::tile_rgba(&set.field, &grid, key)))
    }

    fn busy(&self) -> bool {
        !self.failed && (self.asking.is_some() || self.reading.is_some())
    }

    fn attribution(&self) -> Option<(&'static str, &'static str)> {
        Some((self.set.credit, jaxa::SITE))
    }

    fn legend(&self, lang: Lang) -> Option<(String, data::gibs::Colormap)> {
        let (date, _) = self.shown.as_ref()?;
        let map = weather::legend(&self.set.field)?;
        let name = tr(lang, ROWS.get(self.row)?.name);
        Some((trf(lang, "{name}, {units} · {day}", &[("name", name), ("units", tr(lang, self.set.field.units)), ("day", date)]), map))
    }
}

/// Строка JAXA номер n в [`ROWS`] - строка реестра слоёв каталога.
pub struct Entry(pub usize);

/// Строки JAXA для [`crate::registry::ALL`].
pub static ENTRIES: [Entry; 7] = [Entry(0), Entry(1), Entry(2), Entry(3), Entry(4), Entry(5), Entry(6)];

impl Entry {
    fn row(&self) -> Option<&'static Row> {
        ROWS.get(self.0)
    }
}

impl crate::registry::Entry for Entry {
    fn key(&self) -> &'static str {
        self.row().map_or("", |r| r.set.key)
    }

    fn name(&self) -> &'static str {
        self.row().map_or("", |r| r.name)
    }

    fn tip(&self) -> &'static str {
        self.row().map_or("", |r| r.tip)
    }

    fn category(&self) -> Category {
        self.row().map_or(Category::Weather, |r| r.category)
    }

    fn source(&self) -> &'static str {
        "JAXA Earth"
    }

    fn words(&self) -> &'static [&'static str] {
        &["jaxa", "джакса", "спутник", "месяц", "satellite", "monthly"]
    }

    fn legend(&self) -> Option<crate::thumbs::Recipe> {
        self.row().and_then(|r| weather::legend(&r.set.field)).map(|m| crate::thumbs::Recipe::Ramp(m.colors))
    }

    fn slot(&self) -> Option<u8> {
        Some(SLOT)
    }

    fn make(&self, cx: &crate::registry::Make<'_>) -> (Box<dyn crate::registry::Layer>, Option<String>) {
        let set = self.row().map_or(&jaxa::LAND_TEMPERATURE, |r| r.set);
        (Box::new(Shown::new(self.0, set, cx.request)), None)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::registry::Layer;

    /// Живая проверка: даты и сетка мира каждого набора с сервера JAXA.
    /// cargo test -p ui live_jaxa -- --ignored --nocapture
    #[test]
    #[ignore]
    #[cfg(not(target_arch = "wasm32"))]
    fn live_jaxa() {
        let request = |limit: u64| -> Box<dyn net::Request> { Box::new(net::native::Requests::with_limit(std::sync::Arc::new(|| {}), limit)) };
        for (n, row) in ROWS.iter().enumerate() {
            let start = std::time::Instant::now();
            let mut shown = Shown::new(n, row.set, &request);
            while shown.shown.is_none() && !shown.failed && start.elapsed().as_secs() < 240 {
                shown.advance(Lang::EN, None, &request);
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let (date, grid) = shown.shown.clone().unwrap_or_else(|| panic!("{}: не загрузилось", row.set.key));
            let finite: Vec<f32> = grid.values.iter().copied().filter(|v| v.is_finite()).collect();
            let (lo, hi) = finite.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
            eprintln!("{:>5.1} s {} {date} {}x{} step {} values {:.0}% {lo:.3}..{hi:.3}, dates {}", start.elapsed().as_secs_f64(), row.set.key, grid.cols, grid.rows, grid.step, 100.0 * finite.len() as f64 / grid.values.len() as f64, shown.dates.len());
            let points = [(-20.0, -10.0), (35.0, 18.0), (0.0, -25.0), (26.0, 52.0), (20.0, 38.0), (55.0, 37.0)];
            eprintln!("    {:?}", points.map(|(la, lo)| (la, lo, data::weather::sample(&grid, la, lo) as f32)));
            assert!(finite.len() * 20 > grid.values.len(), "{}", row.set.key);
            assert!(shown.painter().is_some_and(|p| p((2, 2, 1)).is_some()), "{}", row.set.key);
        }
    }
}