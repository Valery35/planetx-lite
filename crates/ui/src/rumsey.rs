// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Карты Рамсея» и сборка привязанной карты. Перенос
//! `ui/rumsey.py` модуля, данные - `data::rumsey`, привязка -
//! `data::warp`.
//!
//! Пункт «Исторические карты Рамсея здесь…» меню на глобусе открывает
//! список карт у вида. Каталог берётся из planetx-terrain через зеркала.
//! Выбранная карта собирается из кусков IIIF не больше
//! `rumsey::MAX_SIDE` пикселей, поле вне обрезки становится прозрачным,
//! привязка - по опорным точкам способом Рамсея. Итог - картинка
//! на поверхности в «Моих метках», папка «Карты Рамсея», с годом карты:
//! год ставит карту на шкалу времени. У программы картинка лежит файлом
//! в папке данных (`rumsey/<номер>.png`), у страницы - в файле меток.

use std::collections::HashMap;

use eframe::egui;
use serde_json::Value;

use data::rumsey::{self, Bbox, Catalogue};

use crate::i18n::{Lang, tr, trf};

/// Кусков IIIF сразу, `PARALLEL` модуля.
pub const PARALLEL: usize = 4;
/// Наибольший ответ каталога и куска, байт.
const ANSWER_BYTES: u64 = 64 << 20;

/// Готовая карта: картинка PNG, рамка, номер, год, название.
pub struct Built {
    pub png: Vec<u8>,
    pub bbox: Bbox,
    pub number: i64,
    pub year: Option<i64>,
    pub title: String,
}

/// Сборка карты по шагам.
enum Job {
    Cell(usize, Box<dyn net::Request>),
    Info(usize, Box<dyn net::Request>, Value),
    Pieces {
        row: usize,
        record: Value,
        service: String,
        plan: rumsey::TilePlan,
        canvas: Vec<u8>,
        next: usize,
        asks: Vec<(usize, Box<dyn net::Request>)>,
    },
}

pub struct Rumsey {
    pub open: bool,
    catalogue: Option<Catalogue>,
    index: Option<(usize, Box<dyn net::Request>)>,
    cells: HashMap<String, Value>,
    view: (f64, f64, Bbox),
    filter: String,
    everywhere: bool,
    /// Столбец сортировки и порядок.
    sort: (usize, bool),
    rows: Vec<usize>,
    selected: Option<usize>,
    status: String,
    job: Option<Job>,
    /// Карта собрана: забирает окно глобуса.
    pub built: Option<Built>,
    /// Перелёт к рамке выбранной карты.
    pub fly: Option<Bbox>,
}

/// Адрес файла в хранилище planetx-terrain по зеркалу n.
fn mirror_url(n: usize, path: &str) -> Option<String> {
    data::slabs::TERRAIN_REPOS.get(n).map(|base| format!("{base}{path}"))
}

impl Rumsey {
    pub fn new(lat: f64, lon: f64, view: Bbox, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Self {
        let mut out = Self {
            open: true,
            catalogue: None,
            index: None,
            cells: HashMap::new(),
            view: (lat, lon, view),
            filter: String::new(),
            everywhere: false,
            sort: (0, true),
            rows: Vec::new(),
            selected: None,
            status: String::new(),
            job: None,
            built: None,
            fly: None,
        };
        out.ask_index(0, request);
        out
    }

    fn ask_index(&mut self, mirror: usize, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        if let Some(url) = mirror_url(mirror, rumsey::INDEX_PATH) {
            let ask = request(ANSWER_BYTES);
            ask.start(&url);
            self.index = Some((mirror, ask));
        }
    }

    /// Новый вид в открытом окне.
    pub fn show_view(&mut self, lat: f64, lon: f64, view: Bbox) {
        self.view = (lat, lon, view);
        self.open = true;
        self.refill();
    }

    /// Идёт загрузка.
    pub fn busy(&self) -> bool {
        self.index.is_some() || self.job.is_some()
    }

    /// Рамки карт списка и выбранной, для глобуса.
    pub fn footprints(&self) -> (Vec<Bbox>, Option<Bbox>) {
        let Some(cat) = &self.catalogue else {
            return (Vec::new(), None);
        };
        let boxes = self.rows.iter().take(rumsey::LIST_MAX).filter_map(|r| cat.bbox.get(*r).copied()).collect();
        (boxes, self.selected.and_then(|r| cat.bbox.get(r).copied()))
    }

    fn refill(&mut self) {
        let Some(cat) = &self.catalogue else {
            return;
        };
        let text = self.filter.trim().to_lowercase();
        let mut rows = if self.everywhere {
            rumsey::search(cat, &text, rumsey::LIST_MAX)
        } else {
            let (lat, lon, view) = self.view;
            rumsey::covering(cat, lat, lon, view)
        };
        if !self.everywhere && !text.is_empty() {
            rows.retain(|r| {
                let line = format!("{} {}", cat.year.get(*r).copied().flatten().map_or(String::new(), |y| y.to_string()), cat.title.get(*r).map_or("", String::as_str)).to_lowercase();
                line.contains(&text)
            });
        }
        let missing = i64::MAX;
        let (column, up) = self.sort;
        rows.sort_by(|a, b| {
            let order = match column {
                0 => cat.year.get(*a).copied().flatten().unwrap_or(missing).cmp(&cat.year.get(*b).copied().flatten().unwrap_or(missing)),
                1 => cat.title.get(*a).map(|t| t.to_lowercase()).cmp(&cat.title.get(*b).map(|t| t.to_lowercase())),
                _ => cat.scale.get(*a).copied().flatten().unwrap_or(f64::MAX).total_cmp(&cat.scale.get(*b).copied().flatten().unwrap_or(f64::MAX)),
            };
            if up { order } else { order.reverse() }
        });
        self.rows = rows;
    }

    /// Шаг: каталог, сведения о карте, куски, сборка и привязка.
    pub fn step(&mut self, lang: Lang, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        if let Some((mirror, ask)) = &self.index
            && let Some(answer) = ask.take()
        {
            let mirror = *mirror;
            self.index = None;
            match answer.and_then(|b| Catalogue::from_text(&String::from_utf8_lossy(&b))) {
                Ok(cat) => {
                    self.catalogue = Some(cat);
                    self.status.clear();
                    self.refill();
                }
                Err(_) if mirror_url(mirror + 1, rumsey::INDEX_PATH).is_some() => self.ask_index(mirror + 1, request),
                Err(_) => self.status = tr(lang, "The Rumsey map catalogue did not load, check the network.").to_string(),
            }
        }
        let Some(job) = self.job.take() else {
            return;
        };
        self.job = self.advance(lang, job, request);
    }

    fn fail(&mut self, lang: Lang) -> Option<Job> {
        self.status = tr(lang, "The map was not built.").to_string();
        None
    }

    fn advance(&mut self, lang: Lang, job: Job, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Option<Job> {
        match job {
            Job::Cell(row, ask) => match ask.take() {
                None => Some(Job::Cell(row, ask)),
                Some(Ok(bytes)) => {
                    let cell = self.catalogue.as_ref()?.cell.get(row)?.clone();
                    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
                        return self.fail(lang);
                    };
                    self.cells.insert(cell, value);
                    self.have_cell(lang, row, request)
                }
                Some(Err(_)) => self.fail(lang),
            },
            Job::Info(row, ask, record) => match ask.take() {
                None => Some(Job::Info(row, ask, record)),
                Some(Ok(bytes)) => {
                    let Ok(info) = serde_json::from_slice::<Value>(&bytes) else {
                        return self.fail(lang);
                    };
                    let width = info.get("width").and_then(Value::as_u64).unwrap_or(0);
                    let height = info.get("height").and_then(Value::as_u64).unwrap_or(0);
                    let mut factors: Vec<u64> = info.get("tiles").and_then(Value::as_array).into_iter().flatten().flat_map(|t| t.get("scaleFactors").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64)).collect();
                    if factors.is_empty() {
                        factors.push(1);
                    }
                    let Some(plan) = rumsey::tile_plan(width, height, &factors, rumsey::MAX_SIDE, rumsey::TILE) else {
                        return self.fail(lang);
                    };
                    let service = rumsey::image_url(record.get("i").and_then(Value::as_str).unwrap_or(""));
                    let canvas = vec![0u8; (plan.width * plan.height * 4) as usize];
                    Some(Job::Pieces { row, record, service, plan, canvas, next: 0, asks: Vec::new() })
                }
                Some(Err(_)) => self.fail(lang),
            },
            Job::Pieces { row, record, service, plan, mut canvas, mut next, mut asks } => {
                // Пришедшие куски - в холст.
                let mut failed = false;
                asks.retain(|(n, ask)| match ask.take() {
                    None => true,
                    Some(Ok(bytes)) => {
                        match (plan.pieces.get(*n), image::load_from_memory(&bytes)) {
                            (Some(piece), Ok(img)) => paste(&mut canvas, plan.width, plan.height, piece.ox, piece.oy, &img.to_rgba8()),
                            _ => failed = true,
                        }
                        false
                    }
                    Some(Err(_)) => {
                        failed = true;
                        false
                    }
                });
                if failed {
                    return self.fail(lang);
                }
                while asks.len() < PARALLEL && next < plan.pieces.len() {
                    if let Some(piece) = plan.pieces.get(next) {
                        let ask = request(ANSWER_BYTES);
                        ask.start(&rumsey::piece_url(&service, piece));
                        asks.push((next, ask));
                    }
                    next += 1;
                }
                let total = plan.pieces.len();
                if !asks.is_empty() || next < total {
                    let done = (next - asks.len()).to_string();
                    self.status = trf(lang, "Loading the map image: {done} of {total}", &[("done", &done), ("total", &total.to_string())]);
                    return Some(Job::Pieces { row, record, service, plan, canvas, next, asks });
                }
                self.assemble(lang, row, &record, &plan, canvas);
                None
            }
        }
    }

    /// Сведения о карте есть: запрос info.json изображения.
    fn have_cell(&mut self, lang: Lang, row: usize, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Option<Job> {
        let cat = self.catalogue.as_ref()?;
        let number = cat.number.get(row)?.to_string();
        let record = self.cells.get(cat.cell.get(row)?)?.get(&number).cloned();
        let Some(record) = record else {
            return self.fail(lang);
        };
        let service = rumsey::image_url(record.get("i").and_then(Value::as_str).unwrap_or(""));
        self.status = tr(lang, "Loading the map image…").to_string();
        let ask = request(ANSWER_BYTES);
        ask.start(&format!("{service}/info.json"));
        Some(Job::Info(row, ask, record))
    }

    /// Обрезка, привязка, картинка PNG.
    fn assemble(&mut self, lang: Lang, row: usize, record: &Value, plan: &rumsey::TilePlan, mut canvas: Vec<u8>) {
        let factor = plan.factor as f64;
        let crop: Vec<(f64, f64)> = record.get("k").and_then(Value::as_array).into_iter().flatten().filter_map(|p| Some((p.get(0)?.as_f64()? / factor, p.get(1)?.as_f64()? / factor))).collect();
        if crop.len() >= 3 {
            cut(&mut canvas, plan.width as u32, plan.height as u32, &crop);
        }
        let points: Vec<(f64, f64, f64, f64)> = record.get("g").and_then(Value::as_array).into_iter().flatten().filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?, p.get(2)?.as_f64()?, p.get(3)?.as_f64()?))).collect();
        let points = rumsey::scaled_points(&points, factor);
        let method = record.get("m").and_then(Value::as_str).unwrap_or("");
        let warp = rumsey::warp_options(method, points.len());
        self.status = tr(lang, "Georeferencing the map…").to_string();
        let warped = match data::warp::warp_rgba(&canvas, plan.width as u32, plan.height as u32, &points, warp, rumsey::MAX_SIDE as u32) {
            Ok(w) => w,
            Err(_) => {
                self.status = tr(lang, "The map was not built.").to_string();
                return;
            }
        };
        let mut png = Vec::new();
        let encoded = image::RgbaImage::from_raw(warped.width, warped.height, warped.rgba).map(|img| img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png));
        if !matches!(encoded, Some(Ok(()))) {
            self.status = tr(lang, "The map was not built.").to_string();
            return;
        }
        let Some(cat) = &self.catalogue else {
            return;
        };
        let title = cat.title.get(row).cloned().unwrap_or_default();
        self.status = trf(lang, "The map is on the globe: {title}.", &[("title", &title)]);
        self.built = Some(Built { png, bbox: warped.bbox, number: cat.number.get(row).copied().unwrap_or(0), year: cat.year.get(row).copied().flatten(), title });
    }

    fn build(&mut self, lang: Lang, row: usize, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        if self.job.is_some() {
            return;
        }
        let Some(cat) = &self.catalogue else {
            return;
        };
        let Some(cell) = cat.cell.get(row).cloned() else {
            return;
        };
        if self.cells.contains_key(&cell) {
            self.job = self.have_cell(lang, row, request);
            return;
        }
        let Some(url) = mirror_url(0, &rumsey::CELL_PATH.replace("{cell}", &cell)) else {
            return;
        };
        self.status = tr(lang, "Loading map details…").to_string();
        let ask = request(ANSWER_BYTES);
        ask.start(&url);
        self.job = Some(Job::Cell(row, ask));
    }

    /// Окно.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        if !self.open {
            return;
        }
        let mut open = true;
        let mut build = None;
        let mut page = None;
        let mut changed = false;
        egui::Window::new(tr(lang, "Rumsey maps")).open(&mut open).default_size([620.0, 520.0]).show(ctx, |ui| {
            ui.label(trf(lang, "Maps at the view: {count}, by year. A click on a column header changes the order.", &[("count", &self.rows.len().to_string())]));
            ui.horizontal(|ui| {
                changed |= ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text(tr(lang, "Year or title")).desired_width(300.0)).changed();
                changed |= ui
                    .checkbox(&mut self.everywhere, tr(lang, "Whole catalogue"))
                    .on_hover_text(tr(lang, "Search by title and year among all maps of the collection, not only at the view. The Show on the globe button flies to the chosen map."))
                    .changed();
            });
            let Some(cat) = &self.catalogue else {
                ui.label(tr(lang, "Loading the map catalogue…"));
                return;
            };
            egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show(ui, |ui| {
                egui::Grid::new("planetx-rumsey").num_columns(3).striped(true).spacing([10.0, 3.0]).show(ui, |ui| {
                    for (n, name) in [tr(lang, "Year"), tr(lang, "Map"), tr(lang, "Scale")].into_iter().enumerate() {
                        let arrow = if self.sort.0 == n { if self.sort.1 { " ▲" } else { " ▼" } } else { "" };
                        if ui.button(format!("{name}{arrow}")).clicked() {
                            self.sort = if self.sort.0 == n { (n, !self.sort.1) } else { (n, true) };
                            changed = true;
                        }
                    }
                    ui.end_row();
                    for row in &self.rows {
                        let year = rumsey::year_text(cat.year.get(*row).copied().flatten());
                        let title = cat.title.get(*row).map_or("", String::as_str);
                        let scale = cat.scale.get(*row).copied().flatten().map_or(String::new(), |s| crate::sheets::scale_text(Some(s.round() as u32)));
                        ui.label(year);
                        let chosen = self.selected == Some(*row);
                        let r = ui.selectable_label(chosen, title);
                        if r.clicked() {
                            self.selected = Some(*row);
                        }
                        if r.double_clicked() {
                            build = Some(*row);
                        }
                        ui.label(scale);
                        ui.end_row();
                    }
                });
            });
            ui.label(&self.status);
            ui.horizontal(|ui| {
                let enabled = self.selected.is_some() && self.job.is_none();
                if ui.add_enabled(enabled, egui::Button::new(tr(lang, "On the globe"))).on_hover_text(tr(lang, "The map is built from the collection image, georeferenced and laid as a picture on the surface in My Places, folder Rumsey maps, with the year of the map. The year puts the map on the time slider.")).clicked() {
                    build = self.selected;
                }
                if ui.add_enabled(self.selected.is_some(), egui::Button::new(tr(lang, "Show on the globe"))).on_hover_text(tr(lang, "Flight to the frame of the chosen map.")).clicked() {
                    self.fly = self.selected.and_then(|r| cat.bbox.get(r).copied());
                }
                if ui.add_enabled(self.selected.is_some(), egui::Button::new(tr(lang, "Map page"))).on_hover_text(tr(lang, "The map page on the collection site in the browser.")).clicked() {
                    page = self.selected;
                }
            });
            ui.label(tr(lang, "Maps of the David Rumsey collection, georeferenced by its control points. The images are CC BY-NC-SA 3.0: non-commercial use only, with credit to the source."));
        });
        if changed {
            self.refill();
        }
        if let Some(row) = build {
            self.selected = Some(row);
            self.build(lang, row, request);
        }
        if let Some(row) = page {
            let cat = self.catalogue.as_ref();
            let record = cat.and_then(|c| Some((c.cell.get(row)?, c.number.get(row)?))).and_then(|(cell, number)| self.cells.get(cell)?.get(number.to_string()));
            let url = record.and_then(|r| r.get("u")).and_then(Value::as_str).unwrap_or("https://www.davidrumsey.com").to_string();
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
        if !open {
            self.open = false;
            self.job = None;
        }
    }
}

/// Кусок картинки в холст по месту (ox, oy).
fn paste(canvas: &mut [u8], width: u64, height: u64, ox: u64, oy: u64, piece: &image::RgbaImage) {
    let (pw, ph) = (u64::from(piece.width()), u64::from(piece.height()));
    for y in 0..ph.min(height.saturating_sub(oy)) {
        let src = (y * pw * 4) as usize;
        let dst = (((oy + y) * width + ox) * 4) as usize;
        let n = (pw.min(width.saturating_sub(ox)) * 4) as usize;
        if let (Some(s), Some(d)) = (piece.as_raw().get(src..src + n), canvas.get_mut(dst..dst + n)) {
            d.copy_from_slice(s);
        }
    }
}

/// Поле вне многоугольника обрезки - прозрачное.
fn cut(canvas: &mut [u8], width: u32, height: u32, crop: &[(f64, f64)]) {
    let Some(mut mask) = tiny_skia::Pixmap::new(width, height) else {
        return;
    };
    let mut path = tiny_skia::PathBuilder::new();
    for (k, (x, y)) in crop.iter().enumerate() {
        if k == 0 {
            path.move_to(*x as f32, *y as f32);
        } else {
            path.line_to(*x as f32, *y as f32);
        }
    }
    path.close();
    let Some(path) = path.finish() else {
        return;
    };
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(0, 0, 0, 255);
    paint.anti_alias = true;
    mask.fill_path(&path, &paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
    for (pixel, m) in canvas.as_chunks_mut::<4>().0.iter_mut().zip(mask.data().as_chunks::<4>().0) {
        pixel[3] = ((u16::from(pixel[3]) * u16::from(m[3])) / 255) as u8;
    }
}
