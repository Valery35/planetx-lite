// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Ряд по датам»: величина участка по сценам окна «Космоснимки»
//! и карта тренда. Перенос `ui/series.py` модуля, расчёт -
//! `data::series` и `data::trend`.
//!
//! Сцены - найденные в окне снимков. Сводка сцены - медиана, 10-й
//! и 90-й процентили по участку без облаков и теней, график дополняется
//! по мере чтения. Щелчок по точке выбирает сцену в списке окна
//! снимков. Таблица ряда пишется в CSV.
//!
//! Сетки всех сцен - в одной системе UTM, самой частой у сцен ряда:
//! так они совпадают для карты тренда. Модуль пересчитывает окна сцен
//! другой зоны в неё, Lite такие сцены пропускает. Карта тренда -
//! наклон Тейла-Сена за год в каждом пикселе (`data::trend::Job`
//! частями по кадрам), картинкой на поверхности в «Мои метки», папка
//! «Карты тренда»: красный - убыль, синий - рост, у температуры наоборот,
//! шкала симметричная по 98-му процентилю модулей. У модуля - GeoTIFF
//! из трёх полос в проекте QGIS.

use eframe::egui;

use data::sentinel::{self as s2, Mission, Scene};
use data::series::{self, Point, Row};
use data::trend;

use crate::cogread::BandRead;
use crate::i18n::{Lang, tr, trf};
use crate::profile::nice_step;
use crate::sentinel::{Built, product_title};

/// Каналов, читаемых сразу по всем сценам ряда, `SERIES_WORKERS`
/// модуля.
pub const SERIES_WORKERS: usize = 8;
/// Пикселей тренда за кадр.
pub const TREND_STEP: usize = 4096;
/// Шкала наклона: убыль, ноль, рост, `TREND_RAMP` модуля.
pub const TREND_RAMP: [[u8; 3]; 3] = [[178, 24, 43], [247, 247, 247], [33, 102, 172]];
/// Продукты-температуры: у них шкала наклона наоборот.
const TEMPERATURES: [&str; 3] = ["lst", "lst_day", "lst_night"];
/// Пикселей до точки, щелчок по которой выбирает сцену, `PICK` модуля.
const PICK: f32 = 8.0;
/// Цвета графика, как у модуля.
const POINT: egui::Color32 = egui::Color32::from_rgb(30, 130, 80);
const BAND: egui::Color32 = egui::Color32::from_rgba_premultiplied(11, 46, 28, 90);
const HOVER: egui::Color32 = egui::Color32::from_rgb(220, 60, 40);

/// Значение продукта для подписи: температура в °C, индекс - три знака.
pub fn value_text(product: &str, value: f64) -> String {
    if TEMPERATURES.contains(&product) {
        format!("{value:.1}\u{a0}°C")
    } else if product == "snow_cover" {
        format!("{value:.0}\u{a0}%")
    } else {
        format!("{value:.3}")
    }
}

/// Цвет наклона slope на шкале от low до high через ноль. warm -
/// продукт-температура, шкала наоборот.
pub fn trend_colour(slope: f64, low: f64, high: f64, warm: bool) -> [u8; 3] {
    let [down, mid, up] = TREND_RAMP;
    let (down, up) = if warm { (up, down) } else { (down, up) };
    let mix = |a: [u8; 3], b: [u8; 3], t: f64| std::array::from_fn(|k| (f64::from(a[k]) + t.clamp(0.0, 1.0) * (f64::from(b[k]) - f64::from(a[k]))).round() as u8);
    if slope < 0.0 && low < 0.0 {
        mix(mid, down, slope / low)
    } else if slope > 0.0 && high > 0.0 {
        mix(mid, up, slope / high)
    } else {
        mid
    }
}

/// Состояние сцены ряда.
enum State {
    Waiting,
    /// Каналы: ключ и чтение.
    Reading(Vec<(String, BandRead)>),
    /// Сводка или None - участок закрыт облаками.
    Done(Option<Point>),
    /// Файлы не прочитаны или сцена другой зоны UTM.
    Failed,
}

/// Расчёт карты тренда и что нужно для картинки.
struct TrendRun {
    job: trend::Job,
    first: String,
    last: String,
    count: usize,
}

pub struct Series {
    pub open: bool,
    mission: Mission,
    scenes: Vec<Scene>,
    ring: Vec<(f64, f64)>,
    area: String,
    product: String,
    epsg: Option<u32>,
    xy: Vec<(f64, f64)>,
    bbox: [f64; 4],
    res: f64,
    states: Vec<State>,
    /// Сетки значений сцен с NaN вне чистых пикселей, по строкам.
    grids: Vec<Option<Vec<f64>>>,
    rows: usize,
    cols: usize,
    status: String,
    trend: Option<TrendRun>,
    /// Готовая карта тренда для «Моих меток».
    pub built: Option<Built>,
    hover: Option<usize>,
    /// Щелчок по точке: номер сцены в окне снимков.
    pub picked: Option<String>,
    /// Сцен другой зоны UTM.
    other_zone: usize,
}

impl Series {
    /// Ряд по сценам scenes спутника mission над участком ring.
    pub fn new(mission: Mission, mut scenes: Vec<Scene>, ring: Vec<(f64, f64)>, area: String, product: Option<&str>) -> Self {
        scenes.sort_by(|a, b| a.when.cmp(&b.when));
        let products = series::products(mission);
        let product = product.filter(|p| products.contains(p)).or_else(|| products.first().copied()).unwrap_or("ndvi").to_string();
        let mut out = Self {
            open: true,
            mission,
            scenes,
            ring,
            area,
            product,
            epsg: None,
            xy: Vec::new(),
            bbox: [0.0; 4],
            res: 0.0,
            states: Vec::new(),
            grids: Vec::new(),
            rows: 0,
            cols: 0,
            status: String::new(),
            trend: None,
            built: None,
            hover: None,
            picked: None,
            other_zone: 0,
        };
        out.restart();
        out
    }

    /// Ряд заново по выбранной величине. Система сеток - самая частая
    /// зона UTM сцен.
    fn restart(&mut self) {
        self.trend = None;
        self.hover = None;
        let mut zones: Vec<(u32, usize)> = Vec::new();
        for epsg in self.scenes.iter().filter_map(|s| s.epsg) {
            match zones.iter_mut().find(|(z, _)| *z == epsg) {
                Some(z) => z.1 += 1,
                None => zones.push((epsg, 1)),
            }
        }
        self.epsg = zones.iter().max_by_key(|(_, n)| *n).map(|(z, _)| *z);
        self.states = self.scenes.iter().map(|_| State::Waiting).collect();
        self.grids = vec![None; self.scenes.len()];
        self.other_zone = 0;
        let grid = self.epsg.and_then(|epsg| {
            let xy = s2::ring_utm(&self.ring, epsg).ok()?;
            let res = series::step(&xy, self.mission);
            let bbox = s2::bounds(&xy, res)?;
            Some((xy, res, bbox))
        });
        match grid {
            Some((xy, res, bbox)) => {
                self.xy = xy;
                self.res = res;
                self.bbox = bbox;
            }
            None => {
                self.states = self.scenes.iter().map(|_| State::Failed).collect();
            }
        }
    }

    pub fn mission(&self) -> Mission {
        self.mission
    }

    /// Каналы ряда ещё читаются или считается тренд.
    pub fn busy(&self) -> bool {
        self.trend.is_some() || self.states.iter().any(|s| matches!(s, State::Waiting | State::Reading(_)))
    }

    /// Ряду нужны файлы по ключу SAS.
    pub fn needs_key(&self) -> bool {
        s2::needs_key(self.mission) && self.states.iter().any(|s| matches!(s, State::Waiting))
    }

    /// Адрес первого файла ряда для ключа SAS.
    pub fn first_href(&self) -> Option<String> {
        let scene = self.scenes.first()?;
        let key = s2::needed(&[self.product.as_str()], self.mission, false).into_iter().next()?;
        scene.assets.get(&key)?.href.first().map(str::to_string)
    }

    /// Шаг: чтение каналов сцен, сводки, тренд частями. sas - ключ
    /// файлов Landsat, None - ключа ещё нет.
    pub fn step(&mut self, lang: Lang, sas: Option<&str>, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        self.step_trend(lang);
        if s2::needs_key(self.mission) && sas.is_none() {
            return;
        }
        let cloud_key = s2::cloud_asset(self.mission).0;
        let mut running: usize = self.states.iter().map(|s| if let State::Reading(r) = s { r.iter().filter(|(_, b)| b.result().is_none() && b.error().is_none()).count() } else { 0 }).sum();
        let mut changed = false;
        for n in 0..self.scenes.len() {
            let start = matches!(self.states[n], State::Waiting) && running < SERIES_WORKERS;
            if start {
                let scene = &self.scenes[n];
                if scene.epsg != self.epsg {
                    self.other_zone += 1;
                    self.states[n] = State::Failed;
                    changed = true;
                    continue;
                }
                let mut keys: Vec<String> = s2::needed(&[self.product.as_str()], self.mission, false).into_iter().filter(|k| scene.assets.contains(k) || !s2::OPTIONAL_BANDS.contains(&k.as_str())).collect();
                if !keys.iter().any(|k| k == cloud_key) {
                    keys.push(cloud_key.to_string());
                }
                let mut reads = Vec::new();
                for key in keys {
                    let Some(asset) = scene.assets.get(&key) else {
                        reads.clear();
                        break;
                    };
                    let hrefs = match sas {
                        Some(sas) if s2::needs_key(self.mission) => s2::sign(&asset.href, sas),
                        _ => asset.href.clone(),
                    };
                    let Some(href) = hrefs.first() else {
                        reads.clear();
                        break;
                    };
                    let read = BandRead::new(href, self.bbox, self.res, key == cloud_key, request);
                    reads.push((key, read));
                }
                running += reads.len();
                self.states[n] = if reads.is_empty() { State::Failed } else { State::Reading(reads) };
                changed = true;
                continue;
            }
            let State::Reading(reads) = &mut self.states[n] else {
                continue;
            };
            for (_, r) in reads.iter_mut() {
                r.step(request);
            }
            if reads.iter().any(|(_, r)| r.error().is_some()) {
                self.states[n] = State::Failed;
                changed = true;
            } else if reads.iter().all(|(_, r)| r.result().is_some()) {
                let (point, grid) = self.summary(n);
                self.grids[n] = grid;
                self.states[n] = State::Done(point);
                changed = true;
            }
        }
        if changed {
            self.show_status(lang);
        }
    }

    /// Сводка и сетка значений прочитанной сцены n, `series_value`
    /// модуля.
    fn summary(&mut self, n: usize) -> (Option<Point>, Option<Vec<f64>>) {
        let (Some(scene), Some(State::Reading(reads))) = (self.scenes.get(n), self.states.get(n)) else {
            return (None, None);
        };
        let cloud_key = s2::cloud_asset(scene.mission).0;
        let Some(area) = s2::mask(&self.xy, self.bbox, self.res) else {
            return (None, None);
        };
        let mut clear = None;
        let mut bands: s2::Bands = std::collections::BTreeMap::new();
        for (key, read) in reads {
            let Some(raw) = read.result() else {
                continue;
            };
            if key == cloud_key {
                clear = Some(s2::clear_mask(scene, raw));
            } else if let Some(asset) = scene.assets.get(key) {
                bands.insert(key.clone(), s2::reflectance(raw, asset));
            }
        }
        let Some(clear) = clear else {
            return (None, None);
        };
        let Some(s2::Product::Single(values)) = s2::product(&self.product, &bands, scene.mission) else {
            return (None, None);
        };
        let point = series::point(&values, &area, &clear);
        if point.is_none() {
            return (None, None);
        }
        let (rows, cols) = area.shape();
        self.rows = rows;
        self.cols = cols;
        let grid = values.data().iter().zip(area.data()).zip(clear.data()).map(|((v, a), c)| if *a && *c && v.is_finite() { f64::from(*v) } else { f64::NAN }).collect();
        (point, Some(grid))
    }

    fn show_status(&mut self, lang: Lang) {
        let total = self.scenes.len();
        let done = self.states.iter().filter(|s| matches!(s, State::Done(_) | State::Failed)).count();
        let points = self.states.iter().filter(|s| matches!(s, State::Done(Some(_)))).count();
        self.status = if done < total {
            trf(lang, "Computing the series: {done} of {total}, points {count}…", &[("done", &done.to_string()), ("total", &total.to_string()), ("count", &points.to_string())])
        } else {
            let mut text = trf(
                lang,
                "Points {count} of {total}. Scenes without a point - the area is covered by clouds by more than {share} % or the files were not read.",
                &[("count", &points.to_string()), ("total", &total.to_string()), ("share", &format!("{:.0}", 100.0 - series::MIN_CLEAR))],
            );
            if self.other_zone > 0 {
                text.push(' ');
                text.push_str(&trf(lang, "Scenes of another UTM zone are skipped: {count}.", &[("count", &self.other_zone.to_string())]));
            }
            text
        };
    }

    /// Точки графика: сутки, номер сцены, сводка.
    fn points(&self) -> Vec<(f64, usize, Point)> {
        self.states
            .iter()
            .enumerate()
            .filter_map(|(n, s)| match s {
                State::Done(Some(p)) => Some((series::day_number(&self.scenes[n].when)?, n, *p)),
                _ => None,
            })
            .collect()
    }

    /// Сетки сцен для тренда: с одной формой, по времени сцены.
    fn trend_input(&self) -> (Vec<Vec<f64>>, Vec<f64>, Vec<usize>) {
        let size = self.rows * self.cols;
        let mut stack = Vec::new();
        let mut days = Vec::new();
        let mut used = Vec::new();
        for (n, grid) in self.grids.iter().enumerate() {
            let (Some(grid), Some(day)) = (grid, series::day_number(&self.scenes[n].when)) else {
                continue;
            };
            if grid.len() == size && size > 0 {
                stack.push(grid.clone());
                days.push(day);
                used.push(n);
            }
        }
        (stack, days, used)
    }

    /// Начать карту тренда, `make_trend` модуля.
    fn start_trend(&mut self, lang: Lang) {
        let (stack, days, used) = self.trend_input();
        if used.len() < trend::MIN_SCENES {
            return;
        }
        let day = |n: Option<&usize>| n.and_then(|n| self.scenes.get(*n)).map(|s| s.when.get(..10).unwrap_or(&s.when).to_string()).unwrap_or_default();
        let (first, last) = (day(used.first()), day(used.last()));
        let Some(job) = trend::Job::new(stack, &days, trend::MIN_SCENES) else {
            return;
        };
        self.status = trf(lang, "Computing the trend map from {count} scenes…", &[("count", &used.len().to_string())]);
        self.trend = Some(TrendRun { job, first, last, count: used.len() });
    }

    /// Шаг тренда, готовый - картинкой.
    fn step_trend(&mut self, lang: Lang) {
        let Some(run) = &mut self.trend else {
            return;
        };
        if !run.job.step(TREND_STEP) {
            return;
        }
        let Some(run) = self.trend.take() else {
            return;
        };
        let result = run.job.finish();
        let (low, high) = trend::limits(&result.slope, trend::SHARE);
        let warm = TEMPERATURES.contains(&self.product.as_str());
        let mut rgba = Vec::with_capacity(result.slope.len() * 4);
        for s in &result.slope {
            if s.is_finite() {
                let c = trend_colour(*s, low, high, warm);
                rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
        let shown: Vec<usize> = (0..result.slope.len()).filter(|i| result.slope[*i].is_finite()).collect();
        let total = shown.len();
        let significant = shown.iter().filter(|i| result.p.get(**i).is_some_and(|p| *p < trend::SIGNIFICANT)).count();
        let mut slopes: Vec<f64> = shown.iter().map(|i| result.slope[*i]).collect();
        slopes.sort_by(f64::total_cmp);
        let median = slopes.get(slopes.len() / 2).map_or_else(|| "-".to_string(), |m| value_text(&self.product, *m));
        let epsg = self.epsg.unwrap_or(32601);
        let title = product_title(lang, &self.product, self.mission);
        let name = trf(lang, "Trend «{name}» {first} - {last}, per year", &[("name", &title), ("first", &run.first), ("last", &run.last)]);
        let Some((png, bbox)) = crate::sentinel::to_latlon(&rgba, self.rows, self.cols, self.bbox, self.res, &self.ring, epsg) else {
            self.status = tr(lang, "The trend map was not built.").to_string();
            return;
        };
        let scale = trf(
            lang,
            "Theil-Sen slope per year from {count} scenes, the scale from {low} to {high} per year. Pixels without {min} clear scenes have no trend.",
            &[("count", &run.count.to_string()), ("low", &value_text(&self.product, low)), ("high", &value_text(&self.product, high)), ("min", &trend::MIN_SCENES.to_string())],
        );
        let credit = self.scenes.first().map(s2::credit).unwrap_or_default();
        self.built = Some(Built { png, bbox, name, day: run.last.clone(), credit: format!("{scale} {credit}") });
        self.status = trf(
            lang,
            "The trend map is in My Places: pixels with a trend {total}, significant (p < {p}) {share} %, median slope {median} per year.",
            &[("total", &total.to_string()), ("p", &trend::SIGNIFICANT.to_string()), ("share", &format!("{:.0}", 100.0 * significant as f64 / total.max(1) as f64)), ("median", &median)],
        );
    }

    /// Ряд в CSV.
    fn save_csv(&mut self, lang: Lang) {
        let rows: Vec<Row> = self
            .scenes
            .iter()
            .zip(&self.states)
            .filter_map(|(s, st)| match st {
                State::Done(value) => Some(Row { when: s.when.clone(), scene: s.id.clone(), value: *value }),
                State::Failed => Some(Row { when: s.when.clone(), scene: s.id.clone(), value: None }),
                _ => None,
            })
            .collect();
        let text = series::csv_text(&rows, &product_title(lang, &self.product, self.mission));
        match crate::export::save_bytes(&format!("series_{}.csv", self.product), ("CSV", "csv"), text.as_bytes()) {
            Ok(Some(path)) => self.status = trf(lang, "The series is saved: {path}", &[("path", &path)]),
            Ok(None) => {}
            Err(e) => self.status = trf(lang, "The file was not written: {error}", &[("error", &e)]),
        }
    }

    /// Окно.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang) {
        if !self.open {
            return;
        }
        let mut open = true;
        let mut product = self.product.clone();
        let mut csv = false;
        let mut make = false;
        let ready = !self.busy() && self.trend_input().2.len() >= trend::MIN_SCENES;
        egui::Window::new(tr(lang, "Series by dates")).open(&mut open).default_size([720.0, 420.0]).show(ctx, |ui| {
            ui.label(trf(lang, "Area: {name}, scenes {count}", &[("name", &self.area), ("count", &self.scenes.len().to_string())]));
            ui.horizontal(|ui| {
                ui.label(tr(lang, "Value"));
                egui::ComboBox::from_id_salt("planetx-series-product").selected_text(product_title(lang, &product, self.mission)).show_ui(ui, |ui| {
                    for p in series::products(self.mission) {
                        ui.selectable_value(&mut product, p.to_string(), product_title(lang, p, self.mission));
                    }
                })
                .response
                .on_hover_text(tr(lang, "The value of the series. The point is the median over the area without clouds and shadows, the segment - from the 10th to the 90th percentile. A change of the value computes the series again."));
            });
            self.chart(ui, lang);
            ui.label(&self.status);
            ui.horizontal(|ui| {
                if ui.add_enabled(self.states.iter().any(|s| matches!(s, State::Done(_))), egui::Button::new(tr(lang, "CSV table…")))
                    .on_hover_text(tr(lang, "The series into a CSV file - date and scene, median, 10th and 90th percentiles, share of the area without clouds. Scenes without a value go as empty rows."))
                    .clicked()
                {
                    csv = true;
                }
                let tip = trf(
                    lang,
                    "The trend of the value in each pixel of the area over the scenes of the series - the Theil-Sen slope per year, as a picture on the surface in My Places. A pixel without {count} clear scenes gets no trend. A series of one season gives the trend between years, a series of all months mixes the season and the trend.",
                    &[("count", &trend::MIN_SCENES.to_string())],
                );
                if ui.add_enabled(ready, egui::Button::new(tr(lang, "Trend map"))).on_hover_text(tip).clicked() {
                    make = true;
                }
            });
        });
        if product != self.product {
            self.product = product;
            self.restart();
            self.status.clear();
        }
        if csv {
            self.save_csv(lang);
        }
        if make {
            self.start_trend(lang);
        }
        if !open {
            self.open = false;
            self.trend = None;
            self.states.iter_mut().filter(|s| matches!(s, State::Waiting | State::Reading(_))).for_each(|s| *s = State::Failed);
        }
    }

    /// График ряда, `SeriesChart` модуля.
    fn chart(&mut self, ui: &mut egui::Ui, lang: Lang) {
        let width = ui.available_width().max(300.0);
        let height = (ui.available_height() - 60.0).max(220.0);
        let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
        let painter = ui.painter_at(rect);
        let text = ui.visuals().text_color();
        let font = egui::FontId::proportional(11.0);
        let plot = egui::Rect::from_min_max(rect.min + egui::vec2(58.0, 26.0), rect.max - egui::vec2(14.0, 30.0));
        let points = self.points();
        if points.is_empty() {
            painter.text(plot.center(), egui::Align2::CENTER_CENTER, tr(lang, "No points of the series"), font, text);
            return;
        }
        let first = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
        let last = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
        let pad = ((last - first) * 0.03).max(3.0);
        let (x0, x1) = (first - pad, last + pad);
        let low = points.iter().map(|p| p.2.low).fold(f64::INFINITY, f64::min);
        let high = points.iter().map(|p| p.2.high).fold(f64::NEG_INFINITY, f64::max);
        let span = (high - low).max(1e-3);
        let (y0, y1) = (low - span * 0.08, high + span * 0.08);
        let to_screen = |d: f64, v: f64| egui::pos2(plot.left() + ((d - x0) / (x1 - x0)) as f32 * plot.width(), plot.bottom() - ((v - y0) / (y1 - y0)) as f32 * plot.height());
        let grid = egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(128, 128, 128, 90));
        let step = nice_step(y1 - y0, 4.0);
        let mut v = (y0 / step).ceil() * step;
        while v <= y1 {
            let y = to_screen(x0, v).y;
            painter.line_segment([egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)], grid);
            painter.text(egui::pos2(plot.left() - 6.0, y), egui::Align2::RIGHT_CENTER, value_text(&self.product, v), font.clone(), text);
            v += step;
        }
        for (day, label) in series::date_ticks(x0, x1, series::TICKS) {
            let x = to_screen(day, y0).x;
            painter.line_segment([egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())], grid);
            painter.text(egui::pos2(x, plot.bottom() + 4.0), egui::Align2::CENTER_TOP, label, font.clone(), text);
        }
        for (day, _, p) in &points {
            painter.line_segment([to_screen(*day, p.low), to_screen(*day, p.high)], egui::Stroke::new(3.0, BAND));
        }
        let line: Vec<egui::Pos2> = points.iter().map(|(d, _, p)| to_screen(*d, p.median)).collect();
        if line.len() > 1 {
            painter.add(egui::Shape::line(line.clone(), egui::Stroke::new(1.2, POINT)));
        }
        for at in &line {
            painter.circle_filled(*at, 3.0, POINT);
        }
        let near = response.hover_pos().and_then(|pos| {
            line.iter().enumerate().map(|(i, at)| (i, at.distance(pos))).filter(|(_, d)| *d <= PICK).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
        });
        self.hover = near;
        if let Some(i) = near {
            let (_, n, p) = points[i];
            painter.circle_filled(line[i], 4.5, HOVER);
            let when = self.scenes[n].when.get(..16).unwrap_or(&self.scenes[n].when).replace('T', " ");
            let label = trf(
                lang,
                "{when}: median {median}, 10-90 % {low} - {high}, without clouds {share} % of the area",
                &[("when", &when), ("median", &value_text(&self.product, p.median)), ("low", &value_text(&self.product, p.low)), ("high", &value_text(&self.product, p.high)), ("share", &format!("{:.0}", p.share))],
            );
            painter.text(egui::pos2(plot.left() + 4.0, rect.top() + 4.0), egui::Align2::LEFT_TOP, label, font, text);
            if response.clicked() {
                self.picked = Some(self.scenes[n].id.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{Series, TREND_RAMP, trend_colour, value_text};
    use crate::i18n::Lang;
    use data::sentinel::{self as s2, Mission};

    /// Живая проверка, как шаги `series_*` модуля: центр Перми, участок
    /// 2 км, Sentinel-2 за 2025 год, ряд NDVI и карта тренда.
    /// cargo test -p ui live_series -- --ignored --nocapture
    #[test]
    #[ignore]
    #[cfg(not(target_arch = "wasm32"))]
    fn live_series() {
        let request = |limit: u64| -> Box<dyn net::Request> { Box::new(net::native::Requests::with_limit(std::sync::Arc::new(|| {}), limit)) };
        let ring = s2::square(58.0105, 56.2502, 2000.0);
        let body = s2::search_body(&s2::geojson(&ring), "2025-01-01", "2025-12-31", 80.0, 100, Mission::Sentinel2);
        let ask = request(16 << 20);
        let text = body.to_string();
        ask.send(s2::search_url(Mission::Sentinel2), &s2::SEARCH_HEADERS, Some(text.as_bytes()));
        let start = std::time::Instant::now();
        let page = loop {
            if let Some(answer) = ask.take() {
                break serde_json::from_slice::<serde_json::Value>(&answer.unwrap()).unwrap();
            }
            assert!(start.elapsed().as_secs() < 60, "каталог не ответил");
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        let (scenes, _) = s2::parse_page(&page);
        eprintln!("сцен {}", scenes.len());
        let mut series = Series::new(Mission::Sentinel2, scenes, ring, "Пермь".into(), Some("ndvi"));
        while series.states.iter().any(|s| matches!(s, super::State::Waiting | super::State::Reading(_))) {
            series.step(Lang::RU, None, &request);
            assert!(start.elapsed().as_secs() < 900, "ряд не досчитан: {}", series.status);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let points = series.points();
        eprintln!("{:.0} с, {} | точек {}, сетка {} × {}", start.elapsed().as_secs_f64(), series.status, points.len(), series.rows, series.cols);
        for (day, n, p) in points.iter().take(60) {
            eprintln!("{} {day:.0} {:.3} {:.3} {:.3} {:.0} %", series.scenes[*n].when, p.median, p.low, p.high, p.share);
        }
        assert!(points.len() >= data::trend::MIN_SCENES);
        series.start_trend(Lang::RU);
        let begun = std::time::Instant::now();
        while series.trend.is_some() {
            series.step(Lang::RU, None, &request);
        }
        eprintln!("тренд {:.1} с: {}", begun.elapsed().as_secs_f64(), series.status);
        let built = series.built.take().unwrap();
        eprintln!("{} | {} | PNG {} байт, рамка {:?}", built.name, built.credit, built.png.len(), built.bbox);
        std::fs::write(std::env::temp_dir().join("planetx-trend.png"), &built.png).unwrap();
    }

    #[test]
    fn values_and_colours() {
        assert_eq!(value_text("ndvi", 0.12345), "0.123");
        assert_eq!(value_text("lst", 21.04), "21.0\u{a0}°C");
        assert_eq!(trend_colour(0.0, -1.0, 1.0, false), TREND_RAMP[1]);
        assert_eq!(trend_colour(-1.0, -1.0, 1.0, false), TREND_RAMP[0]);
        assert_eq!(trend_colour(2.0, -1.0, 1.0, false), TREND_RAMP[2]);
        // У температуры рост - красный.
        assert_eq!(trend_colour(1.0, -1.0, 1.0, true), TREND_RAMP[0]);
    }
}
