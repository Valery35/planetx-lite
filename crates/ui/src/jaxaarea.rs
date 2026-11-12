// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Данные JAXA» подменю «Анализ участка»: наборы JAXA Earth по
//! участку за месяцы. Перенос `ui/jaxa.py` модуля по слову автора от
//! 12 ноября 2026 года «Да, переноси», расчёт - `data::jaxa`.
//!
//! Участок - квадрат у точки меню. «Построить» читает файлы COG уровня 1
//! под участком ([`crate::cogread::BandRead`]), не больше
//! [`MONTH_WORKERS`] месяцев сразу. График - медиана участка по месяцам
//! и отрезок от 10-го до 90-го процентиля, как у «Ряда по датам». Щелчок
//! по точке выбирает месяц. Растр месяца и карта тренда (наклон
//! Тейла-Сена за год, `data::trend`) ложатся слоями-картинками «Моих
//! слоёв», у модуля - растрами проекта QGIS. Таблица ряда пишется в CSV.

use eframe::egui;

use data::jaxa::{self, AreaGrid, Part, Set};
use data::sentinel::Grid;
use data::series::{self, Point, Row};
use data::trend;

use crate::cogread::BandRead;
use crate::i18n::{Lang, tr, trf};
use crate::sentinel::Built;

/// Участок по умолчанию и наибольший, км, `SIDE` и `MOST` модуля.
pub const SIDE: f64 = 50.0;
pub const MOST: f64 = 1000.0;
/// Месяцев ряда по умолчанию - последние.
pub const MONTHS: usize = 36;
/// Месяцев, читаемых сразу.
pub const MONTH_WORKERS: usize = 6;
/// Наибольший ответ описания набора, байт.
const ANSWER_BYTES: u64 = 4 << 20;
/// Пикселей тренда за кадр.
const TREND_STEP: usize = 4096;
/// Высота под графиком сверх строки, которую оставляет `series::chart`:
/// флажок и строка состояния, точки экрана.
const BELOW_CHART: f32 = 60.0;
/// Наименьшая полоса графика, точки экрана: график в ней не ниже 220.
const CHART_LEAST: f32 = 280.0;

/// Название набора окна, `set_titles` модуля.
pub fn set_title(lang: Lang, set: &Set) -> String {
    let name = match set.key {
        "jaxa_lst" => "Land surface temperature by day, SGLI",
        "jaxa_sst" => "Sea surface temperature by day, SGLI",
        "jaxa_ndvi" => "Vegetation index NDVI, SGLI",
        "jaxa_chla" => "Chlorophyll-a in water, SGLI",
        "jaxa_precip" => "Precipitation GSMaP, mean rate",
        "jaxa_smc" => "Soil moisture, AMSR2",
        _ => "Aerosol optical depth, MODIS",
    };
    tr(lang, name).to_string()
}

/// Значение набора с единицей, `value_label` модуля.
pub fn value_text(lang: Lang, set: &Set, value: f64) -> String {
    match set.key {
        "jaxa_lst" | "jaxa_sst" => format!("{value:.1}\u{a0}°C"),
        "jaxa_chla" => format!("{value:.2}\u{a0}{}", tr(lang, "mg/m³")),
        "jaxa_precip" => format!("{value:.2}\u{a0}{}", tr(lang, "mm/h")),
        "jaxa_smc" => format!("{value:.1}\u{a0}%"),
        _ => format!("{value:.3}"),
    }
}

/// Месяц ряда: «ГГГГ-ММ», сутки середины, сводка участка (None - значений
/// меньше доли [`series::MIN_CLEAR`]), сетка значений с NaN вне участка.
struct Month {
    date: String,
    day: f64,
    point: Option<Point>,
    values: Vec<f64>,
}

/// Чтение месяца: части сетки по файлам квадратов и их чтения.
struct MonthRead {
    date: String,
    parts: Vec<(Part, BandRead)>,
}

/// Ряд: набор, сетка, маска участка, месяцы.
struct Built0 {
    set: &'static Set,
    grid: AreaGrid,
    mask: Vec<bool>,
    waiting: Vec<String>,
    reading: Vec<MonthRead>,
    months: Vec<Month>,
    total: usize,
}

/// Расчёт карты тренда.
struct TrendRun {
    job: trend::Job,
    first: String,
    last: String,
    count: usize,
}

/// Окно «Данные JAXA».
pub struct JaxaArea {
    pub open: bool,
    point: (f64, f64),
    side: f64,
    set: usize,
    /// Месяцы наборов, по номеру набора.
    dates: Vec<Option<Vec<String>>>,
    asking: Option<(usize, Box<dyn net::Request>)>,
    first: String,
    last: String,
    built: Option<Built0>,
    chosen: Option<String>,
    season: bool,
    trend: Option<TrendRun>,
    status: String,
    /// Картинка месяца или карта тренда для «Моих слоёв»: картинка, папка.
    pub out: Option<(Built, bool)>,
}

impl JaxaArea {
    /// Окно у точки lat, lon.
    pub fn new(lat: f64, lon: f64, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Self {
        let mut out = Self {
            open: true,
            point: (lat, lon),
            side: SIDE,
            set: 0,
            dates: vec![None; jaxa::SETS.len()],
            asking: None,
            first: String::new(),
            last: String::new(),
            built: None,
            chosen: None,
            season: true,
            trend: None,
            status: String::new(),
            out: None,
        };
        out.load_dates(request);
        out
    }

    /// Окно снова у точки.
    pub fn open_at(&mut self, lat: f64, lon: f64) {
        self.open = true;
        self.point = (lat, lon);
    }

    fn current(&self) -> &'static Set {
        jaxa::SETS.get(self.set).copied().unwrap_or(&jaxa::NDVI)
    }

    /// Месяцы набора: из памяти или запросом описания набора.
    fn load_dates(&mut self, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        if let Some(Some(dates)) = self.dates.get(self.set) {
            let dates = dates.clone();
            self.set_period(&dates);
            return;
        }
        let ask = request(ANSWER_BYTES);
        ask.start(&jaxa::collection_url(self.current()));
        self.asking = Some((self.set, ask));
    }

    /// Период по умолчанию - последние [`MONTHS`] месяцев.
    fn set_period(&mut self, dates: &[String]) {
        self.first = dates.get(dates.len().saturating_sub(MONTHS)).cloned().unwrap_or_default();
        self.last = dates.last().cloned().unwrap_or_default();
    }

    /// Идут запросы или расчёт.
    pub fn busy(&self) -> bool {
        self.asking.is_some() || self.trend.is_some() || self.built.as_ref().is_some_and(|b| !b.waiting.is_empty() || !b.reading.is_empty())
    }

    /// Рамка участка: запад, юг, восток, север. None - участок через
    /// линию перемены дат.
    fn area_box(&self) -> Option<[f64; 4]> {
        let (lat, lon) = self.point;
        let (w, s, e, n) = data::site::square(lat, lon, self.side * 1000.0);
        (w >= -180.0 && e <= 180.0).then_some([w, s.max(-90.0), e, n.min(90.0)])
    }

    /// «Построить»: месяцы периода под участком.
    fn build(&mut self, lang: Lang) {
        let set = self.current();
        let Some(Some(dates)) = self.dates.get(self.set) else {
            self.status = tr(lang, "No months of the set.").to_string();
            return;
        };
        let (a, b) = if self.first <= self.last { (self.first.clone(), self.last.clone()) } else { (self.last.clone(), self.first.clone()) };
        let waiting = jaxa::between(dates, &a, &b);
        if waiting.is_empty() {
            self.status = tr(lang, "No months of the set.").to_string();
            return;
        }
        let Some(bbox) = self.area_box() else {
            self.status = tr(lang, "The area crosses the date line, move the point.").to_string();
            return;
        };
        let grid = jaxa::grid_for(bbox, jaxa::area_step(set));
        let [w, s, e, n] = bbox;
        let mask = grid.mask(&|lat, lon| lat >= s && lat <= n && lon >= w && lon <= e);
        self.trend = None;
        self.chosen = None;
        self.status = trf(lang, "Reading the months: {done} of {total}…", &[("done", "0"), ("total", &waiting.len().to_string())]);
        self.built = Some(Built0 { set, grid, mask, total: waiting.len(), waiting, reading: Vec::new(), months: Vec::new() });
    }

    /// Шаг: месяцы набора, чтение месяцев, тренд частями.
    pub fn step(&mut self, lang: Lang, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        if let Some((n, ask)) = &self.asking
            && let Some(answer) = ask.take()
        {
            let n = *n;
            self.asking = None;
            let dates = answer.as_ref().ok().map(|b| jaxa::parse_dates(&String::from_utf8_lossy(b))).unwrap_or_default();
            if dates.is_empty() {
                self.status = trf(lang, "The JAXA catalog did not answer: {error}", &[("error", answer.err().as_deref().unwrap_or("-"))]);
            } else {
                self.status = trf(lang, "Months in the set: {count}, from {first} to {last}.", &[("count", &dates.len().to_string()), ("first", &dates[0]), ("last", &dates[dates.len() - 1])]);
                if n == self.set {
                    self.set_period(&dates);
                }
                if let Some(slot) = self.dates.get_mut(n) {
                    *slot = Some(dates);
                }
            }
        }
        self.step_months(lang, request);
        self.step_trend(lang);
    }

    fn step_months(&mut self, lang: Lang, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        let Some(b) = &mut self.built else {
            return;
        };
        while b.reading.len() < MONTH_WORKERS && !b.waiting.is_empty() {
            let date = b.waiting.remove(0);
            let parts: Vec<(Part, BandRead)> = b
                .grid
                .parts()
                .into_iter()
                .map(|p| {
                    let read = BandRead::new(&jaxa::quad_url(b.set, &date, p.quad.0, p.quad.1), p.bbox, b.grid.step, true, request);
                    (p, read)
                })
                .collect();
            b.reading.push(MonthRead { date, parts });
        }
        let mut changed = false;
        let mut done = Vec::new();
        for (i, m) in b.reading.iter_mut().enumerate() {
            for (_, read) in &mut m.parts {
                read.step(request);
            }
            if m.parts.iter().all(|(_, r)| r.result().is_some() || r.error().is_some()) {
                done.push(i);
            }
        }
        for i in done.into_iter().rev() {
            let m = b.reading.remove(i);
            let mut values = vec![f64::NAN; b.grid.rows * b.grid.cols];
            // Файла квадрата нет - его часть пустая, как у модуля.
            for (part, read) in &m.parts {
                if let Some(raw) = read.result() {
                    jaxa::place(b.set, &b.grid, part, raw, &mut values);
                }
            }
            for (v, inside) in values.iter_mut().zip(&b.mask) {
                if !inside {
                    *v = f64::NAN;
                }
            }
            let shape = (b.grid.rows, b.grid.cols);
            let point = match (Grid::from_vec(shape.0, shape.1, values.iter().map(|v| *v as f32).collect()), Grid::from_vec(shape.0, shape.1, b.mask.clone())) {
                (Some(v), Some(area)) => series::point(&v, &area, &Grid::filled(shape.0, shape.1, true)),
                _ => None,
            };
            let day = jaxa::month_day(&m.date).unwrap_or(0.0);
            b.months.push(Month { date: m.date, day, point, values });
            changed = true;
        }
        if !changed {
            return;
        }
        b.months.sort_by(|x, y| x.date.cmp(&y.date));
        if !b.waiting.is_empty() || !b.reading.is_empty() {
            self.status = trf(lang, "Reading the months: {done} of {total}…", &[("done", &b.months.len().to_string()), ("total", &b.total.to_string())]);
            return;
        }
        let shown: Vec<&Month> = b.months.iter().filter(|m| m.point.is_some()).collect();
        self.status = match shown.last().and_then(|m| Some((m, m.point?))) {
            None => trf(lang, "No values of the set in the area for {count} months: the area is outside the set, for example land for a sea set.", &[("count", &b.months.len().to_string())]),
            Some((last, p)) => trf(
                lang,
                "Months with a value {shown} of {count}. The last {date} - median {median}.",
                &[("shown", &shown.len().to_string()), ("count", &b.months.len().to_string()), ("date", &last.date), ("median", &value_text(lang, b.set, p.median))],
            ),
        };
    }

    /// Месяц для растра: выбранный щелчком или последний со значением.
    fn chosen_month(&self) -> Option<&Month> {
        let b = self.built.as_ref()?;
        let shown: Vec<&Month> = b.months.iter().filter(|m| m.point.is_some()).collect();
        shown.iter().find(|m| Some(&m.date) == self.chosen.as_ref()).or(shown.last()).copied()
    }

    /// Месяцы для тренда: все со значением или того же месяца года.
    fn trend_months(&self) -> Vec<&Month> {
        let Some(b) = &self.built else {
            return Vec::new();
        };
        let shown: Vec<&Month> = b.months.iter().filter(|m| m.point.is_some()).collect();
        match self.chosen_month().filter(|_| self.season) {
            Some(c) => shown.into_iter().filter(|m| m.date.get(5..) == c.date.get(5..)).collect(),
            None => shown,
        }
    }

    /// Растр выбранного месяца - картинкой в «Мои слои».
    fn month_picture(&mut self, lang: Lang) {
        let (Some(b), Some(m)) = (&self.built, self.chosen_month()) else {
            return;
        };
        let rgba: Vec<u8> = m.values.iter().flat_map(|v| data::weather::colorize(&b.set.field, *v)).collect();
        let Some(png) = png(&rgba, b.grid.cols, b.grid.rows) else {
            return;
        };
        let median = m.point.map_or_else(String::new, |p| value_text(lang, b.set, p.median));
        let name = trf(lang, "{name}, {date}", &[("name", &set_title(lang, b.set)), ("date", &m.date)]);
        let credit = format!("{} {}", trf(lang, "Median of the area {median}.", &[("median", &median)]), credit(lang, b.set));
        let date = m.date.clone();
        self.out = Some((Built { png, bbox: b.grid.bounds(), name, day: date, credit }, false));
        self.status = tr(lang, "The month raster is in My layers.").to_string();
    }

    /// Начать карту тренда, `make_trend` модуля.
    fn start_trend(&mut self, lang: Lang) {
        let chosen = self.trend_months();
        if chosen.len() < trend::MIN_SCENES {
            self.status = trf(
                lang,
                "The trend needs at least {count} months with a value, the series has {have}. Widen the period or clear the flag «One month of the year».",
                &[("count", &trend::MIN_SCENES.to_string()), ("have", &chosen.len().to_string())],
            );
            return;
        }
        let stack: Vec<Vec<f64>> = chosen.iter().map(|m| m.values.clone()).collect();
        let days: Vec<f64> = chosen.iter().map(|m| m.day).collect();
        let (first, last, count) = (chosen[0].date.clone(), chosen[chosen.len() - 1].date.clone(), chosen.len());
        let Some(job) = trend::Job::new(stack, &days, trend::MIN_SCENES) else {
            return;
        };
        self.status = trf(lang, "Computing the trend map from {count} months…", &[("count", &count.to_string())]);
        self.trend = Some(TrendRun { job, first, last, count });
    }

    fn step_trend(&mut self, lang: Lang) {
        let Some(run) = &mut self.trend else {
            return;
        };
        if !run.job.step(TREND_STEP) {
            return;
        }
        let (Some(run), Some(b)) = (self.trend.take(), &self.built) else {
            return;
        };
        let result = run.job.finish();
        let shown: Vec<usize> = (0..result.slope.len()).filter(|i| result.slope[*i].is_finite()).collect();
        if shown.is_empty() {
            self.status = tr(lang, "There is no trend in any pixel.").to_string();
            return;
        }
        let (low, high) = trend::limits(&result.slope, trend::SHARE);
        let warm = b.set.field.units == "°C";
        let rgba: Vec<u8> = result
            .slope
            .iter()
            .flat_map(|s| if s.is_finite() {
                let c = crate::series::trend_colour(*s, low, high, warm);
                [c[0], c[1], c[2], 255]
            } else {
                [0; 4]
            })
            .collect();
        let Some(png) = png(&rgba, b.grid.cols, b.grid.rows) else {
            return;
        };
        let significant = shown.iter().filter(|i| result.p.get(**i).is_some_and(|p| *p < trend::SIGNIFICANT)).count();
        let mut slopes: Vec<f64> = shown.iter().map(|i| result.slope[*i]).collect();
        slopes.sort_by(f64::total_cmp);
        let median = slopes.get(slopes.len() / 2).map_or_else(|| "-".to_string(), |m| value_text(lang, b.set, *m));
        let note = trf(
            lang,
            "Pixels with a trend {total}, significant (p < {p}) {share} %, median slope {median} per year.",
            &[("total", &shown.len().to_string()), ("p", &trend::SIGNIFICANT.to_string()), ("share", &format!("{:.0}", 100.0 * significant as f64 / shown.len() as f64)), ("median", &median)],
        );
        let name = trf(lang, "Trend «{name}» {first} - {last}, per year", &[("name", &set_title(lang, b.set)), ("first", &run.first), ("last", &run.last)]);
        let scale = trf(
            lang,
            "Theil-Sen slope per year from {count} months, the scale from {low} to {high} per year.",
            &[("count", &run.count.to_string()), ("low", &value_text(lang, b.set, low)), ("high", &value_text(lang, b.set, high))],
        );
        self.out = Some((Built { png, bbox: b.grid.bounds(), name, day: run.last.clone(), credit: format!("{scale} {note} {}", credit(lang, b.set)) }, true));
        self.status = format!("{} {note}", tr(lang, "The trend map is in My layers."));
    }

    /// Ряд в CSV: месяц, медиана, процентили, доля участка со значением.
    fn save_csv(&mut self, lang: Lang) {
        let Some(b) = &self.built else {
            return;
        };
        let rows: Vec<Row> = b.months.iter().map(|m| Row { when: m.date.clone(), scene: b.set.band.to_string(), value: m.point }).collect();
        let text = series::csv_text(&rows, &set_title(lang, b.set)).replacen("date,scene,", "month,band,", 1).replacen("clear_percent", "valid_percent", 1);
        match crate::export::save_bytes(&format!("jaxa_{}_series.csv", b.set.key), ("CSV", "csv"), text.as_bytes()) {
            Ok(Some(path)) => self.status = trf(lang, "The series is saved: {path}", &[("path", &path)]),
            Ok(None) => {}
            Err(e) => self.status = trf(lang, "The file was not written: {error}", &[("error", &e)]),
        }
    }

    /// Окно.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        if !self.open {
            return;
        }
        let mut open = true;
        let mut set = self.set;
        let (mut run, mut picture, mut make, mut csv) = (false, false, false, false);
        let dates = self.dates.get(self.set).cloned().flatten().unwrap_or_default();
        let reading = self.built.as_ref().is_some_and(|b| !b.waiting.is_empty() || !b.reading.is_empty());
        let have = self.chosen_month().is_some();
        let can_trend = !reading && self.trend.is_none() && self.trend_months().len() >= trend::MIN_SCENES;
        egui::Window::new(tr(lang, "JAXA data")).id(egui::Id::unique("planetx-jaxa-area")).open(&mut open).default_size([720.0, 520.0]).show(ctx, |ui| {
            ui.add(egui::Label::new(tr(lang, "JAXA Earth observations by months - median of the area, spread, month rasters and the trend map in My layers. The source is JAXA Earth, data.earth.jaxa.jp.")).wrap());
            egui::Grid::new("planetx-jaxa-form").num_columns(2).show(ui, |ui| {
                ui.label(tr(lang, "Area"));
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.side).range(1.0..=MOST).speed(1.0).suffix(format!("\u{a0}{}", tr(lang, "km"))))
                        .on_hover_text(tr(lang, "Side of the square around the point, km."));
                    ui.label(format!("{:.4}, {:.4}", self.point.0, self.point.1));
                });
                ui.end_row();
                ui.label(tr(lang, "Set"));
                egui::ComboBox::from_id_salt("planetx-jaxa-set")
                    .selected_text(set_title(lang, self.current()))
                    .show_ui(ui, |ui| {
                        for (n, s) in jaxa::SETS.iter().enumerate() {
                            ui.selectable_value(&mut set, n, set_title(lang, s));
                        }
                    })
                    .response
                    .on_hover_text(tr(lang, "Monthly JAXA Earth sets for the whole world. The pixel is about 2.8 km for the SGLI sets, 5.5 km for the aerosol, 11 km for precipitation and soil moisture."));
                ui.end_row();
                ui.label(tr(lang, "Months"));
                ui.horizontal(|ui| {
                    for (id, value) in [("planetx-jaxa-first", &mut self.first), ("planetx-jaxa-last", &mut self.last)] {
                        egui::ComboBox::from_id_salt(id).selected_text(value.clone()).show_ui(ui, |ui| {
                            for d in &dates {
                                ui.selectable_value(value, d.clone(), d);
                            }
                        });
                    }
                });
                ui.end_row();
            });
            let (points, labels): (Vec<(f64, Point)>, Vec<String>) = self.built.as_ref().map_or_else(Default::default, |b| b.months.iter().filter_map(|m| Some(((m.day, m.point?), m.date.clone()))).unzip());
            let current = self.built.as_ref().map_or(self.current(), |b| b.set);
            let hover = |i: usize| {
                let p = points[i].1;
                trf(
                    lang,
                    "{when}: median {median}, 10-90 % {low} - {high}, with a value {share} % of the area",
                    &[("when", &labels[i]), ("median", &value_text(lang, current, p.median)), ("low", &value_text(lang, current, p.low)), ("high", &value_text(lang, current, p.high)), ("share", &format!("{:.0}", p.share))],
                )
            };
            // График - в своей полосе: под ним флажок, строка состояния
            // и ряд кнопок. Прежде он брал всю высоту окна, а `chart`
            // оставляет снизу место на одну строку - окно росло каждый
            // кадр и уводило кнопки за край экрана.
            let band = egui::vec2(ui.available_width(), (ui.available_height() - BELOW_CHART).max(CHART_LEAST));
            let (near, clicked) = ui.allocate_ui(band, |ui| crate::series::chart(ui, lang, &points, &|v| value_text(lang, current, v), &hover)).inner;
            if let Some(i) = near.filter(|_| clicked) {
                self.chosen = labels.get(i).cloned();
                if let Some(p) = points.get(i) {
                    self.status = trf(lang, "Chosen month {date}: median {median}.", &[("date", &labels[i]), ("median", &value_text(lang, current, p.1.median))]);
                }
            }
            ui.checkbox(&mut self.season, tr(lang, "One month of the year")).on_hover_text(tr(lang, "The trend map only by the months of the same month of the year as the chosen one: the trend between years without the seasonal course."));
            ui.add(egui::Label::new(&self.status).wrap());
            ui.horizontal(|ui| {
                if ui.add_enabled(!reading && !dates.is_empty(), egui::Button::new(tr(lang, "Build"))).on_hover_text(tr(lang, "Reads the months of the set under the area and builds the series: the point is the median of the area, the segment - from the 10th to the 90th percentile.")).clicked() {
                    run = true;
                }
                if ui.add_enabled(have, egui::Button::new(tr(lang, "Month to My layers"))).on_hover_text(tr(lang, "The raster of the chosen month as a picture layer of My layers. The month is chosen by a click on a point of the chart, without a click - the last one with a value.")).clicked() {
                    picture = true;
                }
                let tip = trf(lang, "The trend of the set in each pixel of the area over the months of the series - the Theil-Sen slope per year, as a picture layer of My layers. A pixel without {count} months with a value gets no trend.", &[("count", &trend::MIN_SCENES.to_string())]);
                if ui.add_enabled(can_trend, egui::Button::new(tr(lang, "Trend map"))).on_hover_text(tip).clicked() {
                    make = true;
                }
                if ui.add_enabled(have, egui::Button::new(tr(lang, "CSV table…"))).on_hover_text(tr(lang, "The series into a CSV file - month, median, 10th and 90th percentiles, share of the area with a value.")).clicked() {
                    csv = true;
                }
            });
        });
        if set != self.set {
            self.set = set;
            self.load_dates(request);
        }
        if run {
            self.build(lang);
        }
        if picture {
            self.month_picture(lang);
        }
        if make {
            self.start_trend(lang);
        }
        if csv {
            self.save_csv(lang);
        }
        if !open {
            self.open = false;
            self.built = None;
            self.trend = None;
            self.asking = None;
        }
    }
}

/// Подпись данных набора set, `credit_text` модуля. Название
/// исследования и JAXA - по условиям данных JAXA.
fn credit(lang: Lang, set: &Set) -> String {
    trf(lang, "Data - {credit}, {url}.", &[("credit", set.credit), ("url", jaxa::SITE)])
}

/// Картинка PNG сетки RGBA cols × rows.
fn png(rgba: &[u8], cols: usize, rows: usize) -> Option<Vec<u8>> {
    let img = image::RgbaImage::from_raw(u32::try_from(cols).ok()?, u32::try_from(rows).ok()?, rgba.to_vec())?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn titles_and_values_of_sets() {
        assert_eq!(set_title(Lang::EN, &jaxa::NDVI), "Vegetation index NDVI, SGLI");
        assert_eq!(value_text(Lang::EN, &jaxa::LAND_TEMPERATURE, 20.04), "20.0\u{a0}°C");
        assert_eq!(value_text(Lang::EN, &jaxa::NDVI, 0.51234), "0.512");
        assert_eq!(value_text(Lang::EN, &jaxa::PRECIPITATION, 0.25), "0.25\u{a0}mm/h");
    }

    /// Запрос без ответа.
    struct Silent(net::Stats);

    impl net::Request for Silent {
        fn start(&self, _: &str) {}
        fn take(&self) -> Option<Result<Vec<u8>, String>> {
            None
        }
        fn stats(&self) -> &net::Stats {
            &self.0
        }
    }

    /// Кнопки окна уходили под нижний край экрана: график брал всю высоту
    /// окна и оставлял снизу место на одну строку, а под ним их три.
    #[test]
    fn buttons_stay_inside_the_window() {
        let request = |_: u64| Box::new(Silent(net::Stats::default())) as Box<dyn net::Request>;
        let mut w = JaxaArea::new(58.0, 56.2, &request);
        w.status = "Months in the set: 106, from 2018-01 to 2026-10.".into();
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 800.0));
        let mut shapes = Vec::new();
        // Окно росло на несколько точек каждый кадр.
        for _ in 0..60 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            let mut out = ctx.run_pass(input, |ctx| w.show(ctx, Lang::EN, &request));
            out.textures_delta.clear();
            shapes = out.shapes;
        }
        let window = ctx.memory(|m| m.area_rect(egui::Id::unique("planetx-jaxa-area"))).unwrap();
        let build = shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == "Build" => Some(t.pos + t.galley.rect.size()),
            _ => None,
        });
        let build = build.expect("кнопки Build нет в кадре");
        assert!(window.bottom() <= screen.bottom(), "окно {window:?} ниже экрана");
        assert!(window.contains(build), "Build {build:?} вне окна {window:?}");
        assert!(window.height() <= 600.0, "окно выросло до {}", window.height());
    }

    /// Живая проверка, как у модуля: участок 50 км у Перми, NDVI за
    /// 12 месяцев 2025 года, ряд и карта тренда.
    /// cargo test -p ui live_jaxa_area -- --ignored --nocapture
    #[test]
    #[ignore]
    #[cfg(not(target_arch = "wasm32"))]
    fn live_jaxa_area() {
        let request = |limit: u64| -> Box<dyn net::Request> { Box::new(net::native::Requests::with_limit(std::sync::Arc::new(|| {}), limit)) };
        let mut w = JaxaArea::new(58.0105, 56.2502, &request);
        w.set = 2;
        let start = std::time::Instant::now();
        w.load_dates(&request);
        while w.asking.is_some() {
            w.step(Lang::RU, &request);
            assert!(start.elapsed().as_secs() < 60, "каталог не ответил");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        w.first = "2025-01".into();
        w.last = "2025-12".into();
        w.season = false;
        w.build(Lang::RU);
        while w.busy() {
            w.step(Lang::RU, &request);
            assert!(start.elapsed().as_secs() < 600, "ряд не досчитан: {}", w.status);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let b = w.built.as_ref().unwrap();
        eprintln!("{:.0} с, {} | сетка {} × {}", start.elapsed().as_secs_f64(), w.status, b.grid.rows, b.grid.cols);
        for m in &b.months {
            eprintln!("{} {:?}", m.date, m.point.map(|p| (p.median, p.low, p.high, p.share)));
        }
        // Зимой под снегом значений NDVI нет: 9 месяцев со значением
        // из 12, проверено 12 ноября 2026 года.
        assert!(b.months.iter().filter(|m| m.point.is_some()).count() >= 8);
        w.start_trend(Lang::RU);
        while w.trend.is_some() {
            w.step(Lang::RU, &request);
        }
        let (built, trend) = w.out.take().unwrap();
        assert!(trend);
        eprintln!("{} | {} | PNG {} байт, рамка {:?}", built.name, built.credit, built.png.len(), built.bbox);
        w.month_picture(Lang::RU);
        let (month, _) = w.out.take().unwrap();
        std::fs::write(std::env::temp_dir().join("planetx-jaxa-month.png"), &month.png).unwrap();
        eprintln!("{} | PNG {} байт", month.name, month.png.len());
    }
}
