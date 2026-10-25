// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Снимки чрезвычайных событий Vantor и Planet: список событий, снимки
//! события, тайлы карты и окно «Снимки событий». Перенос `ui/events.py`
//! и `ui/globe/disasters.py` модуля, ядро - `data::events`.
//!
//! Каталоги STAC приходят разовыми запросами, документов события
//! в работе не больше [`PARALLEL`], всего не больше [`MAX_DOCS`]. Тайл
//! карты на день собирается из снимков не позже дня, новые сверху:
//! заголовок COG снимка читается один раз, тайлы снимка просятся
//! запросами Range и раскодируются в главном потоке, не больше
//! [`DECODES`] за кадр. Снимки перебираются по одному, пока тайл
//! не закрыт целиком, - так у события из сотни полос просятся только
//! верхние.
//!
//! Снимки лежат на снимке подложки под линиями основы, своей парой
//! текстур тайла (`render::Globe::set_event`). Если у события есть снимки
//! до него, сразу включается шторка, как у модуля: слева от линии - последний
//! день до события, справа - выбранный день. Линия тянется мышью.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use data::cog::CogHeader;
use data::events::{self, Event, Item};
use eframe::egui;

use crate::cogread::HeaderRead;
use crate::i18n::{Lang, tr, trf};

/// Документов каталога в работе сразу, `PARALLEL` модуля.
const PARALLEL: usize = 6;
/// Документов одного события не больше, `MAX_DOCS` модуля.
const MAX_DOCS: usize = 1500;
/// Предел ответа документа каталога, байт.
const DOC_BYTES: u64 = 4 << 20;
/// Предел ответа тайла снимка, байт.
const PART_BYTES: u64 = 16 << 20;
/// Открытых снимков в памяти, `COGS_KEPT` модуля.
const COGS_KEPT: usize = 96;
/// Раскодированных тайлов снимков в памяти, байт.
const CACHE_BYTES: usize = 128 << 20;
/// Запросов тайлов снимков сразу.
const IN_FLIGHT: usize = 12;
/// Тайлов снимков раскодировать за кадр.
const DECODES: usize = 3;
/// Тайлов карты в сборке сразу.
const JOBS: usize = 16;
/// Сторона тайла карты.
const SIDE: usize = 256;
/// Узлов сетки пересчёта координат по стороне тайла.
const GRID: usize = 17;
/// Наибольшая сторона тайла снимка.
const PART_SIDE: u32 = 4096;
const FALSE_NORTHING: f64 = 10_000_000.0;
const RADIUS: f64 = 6_378_137.0;

/// Снимки события на глобусе: уровень тайлов, подпись.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layer {
    pub level: u32,
    pub credit: (&'static str, &'static str),
}

/// Что попросило окно.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Перелёт к рамке события: запад, юг, восток, север.
    Fly([f64; 4]),
    /// Снимки стороны сменились: другой день, другое событие или сняты.
    /// 0 - «после», 1 - «до» шторки.
    Changed(usize),
}

/// Обход каталогов одного события.
struct Crawl {
    event: Event,
    queue: VecDeque<(String, String, usize)>,
    seen: HashSet<String>,
    asks: Vec<(String, String, usize, Box<dyn net::Request>)>,
    items: Vec<Item>,
    count: usize,
    error: String,
}

/// Снимки события на глобусе.
pub struct Shown {
    pub event: Event,
    pub items: Vec<Item>,
    pub days: Vec<String>,
    pub day: String,
    /// День снимков слева от шторки, None - шторки нет.
    pub before: Option<String>,
    pub layer: Layer,
}

/// Окно «Снимки событий» и снимки на глобусе.
#[derive(Default)]
pub struct Events {
    pub open: bool,
    roots: Vec<(&'static str, String, Box<dyn net::Request>)>,
    docs: Vec<(&'static str, String, String, Box<dyn net::Request>)>,
    loaded: bool,
    pub events: Vec<Event>,
    errors: Vec<String>,
    chosen: Option<String>,
    status: String,
    counts: HashMap<String, usize>,
    crawl: Option<Crawl>,
    pub shown: Option<Shown>,
    tiles: Option<Painter>,
    /// Тайлы снимков «до» слева от шторки.
    before_tiles: Option<Painter>,
    /// Линия шторки - доля ширины вида от левого края.
    pub split: f32,
}

/// Ответ запроса.
type Answer = Result<Vec<u8>, String>;
/// Тайл снимка в загрузке: тайл, куски файла, их запросы и ответы.
type PartAsk = (Part, Vec<std::ops::Range<u64>>, Vec<(u64, Box<dyn net::Request>)>, Vec<Option<Vec<u8>>>);

fn json(answer: Answer) -> Result<serde_json::Value, String> {
    answer.and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()))
}

/// События по дате, новые первыми, при одной дате - по названию.
fn order(events: &mut [Event]) {
    events.sort_by(|a, b| (b.day.as_str(), b.title.as_str()).cmp(&(a.day.as_str(), a.title.as_str())));
}

impl Events {
    /// Открыть окно: список событий загружается при первом открытии.
    pub fn open(&mut self, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        self.open = true;
        if self.loaded || !self.roots.is_empty() || !self.docs.is_empty() {
            return;
        }
        self.errors.clear();
        for (source, url) in events::ROOTS {
            let ask = request(DOC_BYTES);
            ask.start(url);
            self.roots.push((source, url.to_string(), ask));
        }
    }

    pub fn busy(&self) -> bool {
        !self.roots.is_empty() || !self.docs.is_empty() || self.crawl.is_some() || self.tiles.as_ref().is_some_and(Painter::busy) || self.before_tiles.as_ref().is_some_and(Painter::busy)
    }

    /// Слой карты на глобусе.
    pub fn layer(&self) -> Option<Layer> {
        self.shown.as_ref().map(|s| s.layer)
    }

    /// Шаг: ответы каталогов, обход события. Возвращает просьбы окна.
    pub fn step(&mut self, lang: Lang, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Vec<Action> {
        let mut out = Vec::new();
        let roots: Vec<(&'static str, String, Answer)> = self.roots.iter().filter_map(|(s, u, a)| a.take().map(|r| (*s, u.clone(), r))).collect();
        for (source, url, answer) in roots {
            self.roots.retain(|(_, u, _)| *u != url);
            match json(answer) {
                Ok(doc) => {
                    for (href, title) in events::parse_events(&doc, &url) {
                        let ask = request(DOC_BYTES);
                        ask.start(&href);
                        self.docs.push((source, href, title, ask));
                    }
                }
                Err(e) => self.errors.push(e),
            }
        }
        let docs: Vec<(&'static str, String, String, Answer)> = self.docs.iter().filter_map(|(s, u, t, a)| a.take().map(|r| (*s, u.clone(), t.clone(), r))).collect();
        for (source, url, title, answer) in docs {
            self.docs.retain(|(_, u, _, _)| *u != url);
            match json(answer) {
                Ok(doc) => {
                    if let Some(mut event) = events::parse_event(&doc, &url, source) {
                        let ident = event.key.split_once('/').map_or("", |(_, i)| i).replace(['-', '_'], " ");
                        if !title.is_empty() && event.title == ident {
                            event.title = title;
                        }
                        self.events.retain(|e| e.key != event.key);
                        self.events.push(event);
                    }
                }
                Err(e) => self.errors.push(e),
            }
            if self.roots.is_empty() && self.docs.is_empty() {
                self.loaded = true;
                order(&mut self.events);
            }
        }
        self.step_crawl(lang, request, &mut out);
        out
    }

    fn step_crawl(&mut self, lang: Lang, request: &dyn Fn(u64) -> Box<dyn net::Request>, out: &mut Vec<Action>) {
        let Some(crawl) = &mut self.crawl else {
            return;
        };
        let arrived: Vec<(String, String, usize, Answer)> = crawl.asks.iter().filter_map(|(u, p, d, a)| a.take().map(|r| (u.clone(), p.clone(), *d, r))).collect();
        for (url, phase, depth, answer) in arrived {
            crawl.asks.retain(|(u, _, _, _)| *u != url);
            crawl.count += 1;
            match json(answer) {
                Err(e) => crawl.error = e,
                Ok(doc) if doc.get("type").and_then(|t| t.as_str()) == Some("Feature") => {
                    if let Some(item) = events::parse_item(&doc, &url, crawl.event.source, &phase) {
                        crawl.items.push(item);
                    }
                }
                Ok(doc) if depth < events::MAX_DEPTH => {
                    for (_, href) in events::children(&doc, &url) {
                        if crawl.seen.insert(href.clone()) {
                            let p = match events::phase_of(&href) {
                                "" => phase.clone(),
                                p => p.to_string(),
                            };
                            crawl.queue.push_back((href, p, depth + 1));
                        }
                    }
                }
                Ok(_) => {}
            }
            self.status = trf(lang, "Loading the images of the event: catalog documents {count}", &[("count", &crawl.count.to_string())]);
        }
        while crawl.asks.len() < PARALLEL && crawl.count + crawl.asks.len() < MAX_DOCS {
            let Some((url, phase, depth)) = crawl.queue.pop_front() else {
                break;
            };
            let ask = request(DOC_BYTES);
            ask.start(&url);
            crawl.asks.push((url, phase, depth, ask));
        }
        if !crawl.asks.is_empty() {
            return;
        }
        let Some(crawl) = self.crawl.take() else {
            return;
        };
        if crawl.items.is_empty() {
            self.status = format!("{} {}", tr(lang, "The event has no images the globe can show."), crawl.error).trim().to_string();
            return;
        }
        let days = events::days(&crawl.items);
        self.counts.insert(crawl.event.key.clone(), crawl.items.len());
        self.status = trf(lang, "Images: {count}, days of survey: {days}. The buttons below choose the day.", &[("count", &crawl.items.len().to_string()), ("days", &days.len().to_string())]);
        if crawl.event.bbox.is_none()
            && let Some(b) = events::bbox_of(&crawl.items)
        {
            out.push(Action::Fly(b));
        }
        let day = events::post_day(&crawl.items).or_else(|| days.last().cloned()).unwrap_or_default();
        let layer = Layer { level: events::max_level(&crawl.items), credit: events::credit(crawl.event.source) };
        // Есть снимки до события - шторка сразу, слева последний день до него.
        let before = events::pre_day(&crawl.items).filter(|b| *b < day);
        self.shown = Some(Shown { event: crawl.event, items: crawl.items, days, day: String::new(), before: None, layer });
        self.set_day(&day, out);
        self.split = 0.5;
        self.set_before(before, out);
    }

    /// Показать снимки дня day.
    fn set_day(&mut self, day: &str, out: &mut Vec<Action>) {
        let Some(shown) = &mut self.shown else {
            return;
        };
        if shown.day == day {
            return;
        }
        shown.day = day.to_string();
        self.tiles = Some(Painter::new(events::shown(&shown.items, day).into_iter().cloned().collect()));
        out.push(Action::Changed(0));
    }

    /// Шторка: слева снимки дня day, None - шторки нет.
    fn set_before(&mut self, day: Option<String>, out: &mut Vec<Action>) {
        let Some(shown) = &mut self.shown else {
            return;
        };
        if shown.before == day {
            return;
        }
        self.before_tiles = day.as_deref().map(|d| Painter::new(events::shown(&shown.items, d).into_iter().cloned().collect()));
        shown.before = day;
        out.push(Action::Changed(1));
    }

    /// Линия шторки, доля ширины вида, или None - шторки нет.
    pub fn swipe(&self) -> Option<f32> {
        self.shown.as_ref().and_then(|s| s.before.as_ref()).map(|_| self.split)
    }

    /// Показать событие по ключу из списка. Ложь - такого события нет.
    pub fn show_key(&mut self, lang: Lang, key: &str, out: &mut Vec<Action>) -> bool {
        let Some(event) = self.events.iter().find(|e| e.key == key).cloned() else {
            return false;
        };
        self.chosen = Some(event.key.clone());
        self.show_event(lang, event, out);
        true
    }

    fn show_event(&mut self, lang: Lang, event: Event, out: &mut Vec<Action>) {
        self.clear(out);
        self.status = trf(lang, "Loading the images of the event «{title}»…", &[("title", &event.title)]);
        if let Some(b) = event.bbox {
            out.push(Action::Fly(b));
        }
        let start = (event.url.clone(), events::phase_of(&event.url).to_string(), 0);
        self.crawl = Some(Crawl { seen: HashSet::from([event.url.clone()]), event, queue: VecDeque::from([start]), asks: Vec::new(), items: Vec::new(), count: 0, error: String::new() });
    }

    fn clear(&mut self, out: &mut Vec<Action>) {
        self.crawl = None;
        self.tiles = None;
        self.before_tiles = None;
        if self.shown.take().is_some() {
            out.push(Action::Changed(0));
            out.push(Action::Changed(1));
        }
    }

    /// Окно. Просьбы - в out.
    pub fn window(&mut self, ctx: &egui::Context, lang: Lang, out: &mut Vec<Action>) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        let mut show = None;
        let mut clear = false;
        let mut day = None;
        let mut before: Option<Option<String>> = None;
        egui::Window::new(tr(lang, "Event imagery")).open(&mut open).default_size([620.0, 520.0]).show(ctx, |ui| {
            ui.label(tr(lang, "Images of earthquakes, floods, fires and hurricanes from the open catalogs of Vantor and Planet. The license of the images is CC BY-NC 4.0 - non-commercial use only."));
            ui.separator();
            egui::ScrollArea::vertical().max_height(220.0).id_salt("events-list").show(ui, |ui| {
                egui::Grid::new("events-grid").num_columns(3).striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
                    ui.strong(tr(lang, "Event"));
                    ui.strong(tr(lang, "Date"));
                    ui.strong(tr(lang, "Source"));
                    ui.end_row();
                    for e in &self.events {
                        let selected = self.chosen.as_deref() == Some(e.key.as_str());
                        let r = ui.selectable_label(selected, &e.title);
                        if r.clicked() {
                            self.chosen = Some(e.key.clone());
                        }
                        if r.double_clicked() {
                            show = Some(e.clone());
                        }
                        ui.label(&e.day);
                        ui.label(events::source_name(e.source));
                        ui.end_row();
                    }
                });
            });
            ui.separator();
            let chosen = self.events.iter().find(|e| self.chosen.as_deref() == Some(e.key.as_str()));
            if let Some(e) = chosen {
                ui.strong(&e.title);
                ui.label([e.day.as_str(), events::source_name(e.source)].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · "));
                if let Some(n) = self.counts.get(&e.key) {
                    ui.label(trf(lang, "Images: {count}", &[("count", &n.to_string())]));
                }
                if !e.description.is_empty() {
                    egui::ScrollArea::vertical().max_height(100.0).id_salt("events-about").show(ui, |ui| {
                        ui.label(&e.description);
                    });
                }
            }
            let status = if !self.status.is_empty() {
                self.status.clone()
            } else if !self.roots.is_empty() || !self.docs.is_empty() {
                tr(lang, "Loading the list of events…").to_string()
            } else if self.events.is_empty() && self.loaded {
                trf(lang, "The list of events did not load: {error}", &[("error", &self.errors.iter().take(2).cloned().collect::<Vec<_>>().join("; "))])
            } else {
                trf(lang, "Events: {count}", &[("count", &self.events.len().to_string())])
            };
            ui.label(egui::RichText::new(status).weak());
            if let Some(s) = &self.shown {
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    let at = s.days.iter().position(|d| *d == s.day);
                    if ui.add_enabled(at.is_some_and(|n| n > 0), egui::Button::new("‹")).on_hover_text(tr(lang, "The previous day of images")).clicked() {
                        day = at.and_then(|n| s.days.get(n.wrapping_sub(1))).cloned();
                    }
                    ui.strong(&s.day);
                    if ui.add_enabled(at.is_some_and(|n| n + 1 < s.days.len()), egui::Button::new("›")).on_hover_text(tr(lang, "The next day of images")).clicked() {
                        day = at.and_then(|n| s.days.get(n + 1)).cloned();
                    }
                    if let Some(pre) = events::pre_day(&s.items)
                        && ui.button(tr(lang, "Before the event")).on_hover_text(tr(lang, "The last day of images before the event.")).clicked()
                    {
                        day = Some(pre);
                    }
                    if let Some(post) = events::post_day(&s.items)
                        && ui.button(tr(lang, "After the event")).on_hover_text(tr(lang, "The last day of images of the event.")).clicked()
                    {
                        day = Some(post);
                    }
                });
                // Шторка: слева - снимки другого дня, обычно последнего до события.
                ui.horizontal_wrapped(|ui| {
                    let mut on = s.before.is_some();
                    let earlier: Vec<&String> = s.days.iter().filter(|d| **d < s.day).collect();
                    let r = ui.add_enabled(!earlier.is_empty(), egui::Checkbox::new(&mut on, tr(lang, "Swipe ⇆"))).on_hover_text(tr(lang, "Compare two days: left of the line on the globe - the images of an earlier day, right - of the chosen day. The line is dragged with the mouse."));
                    if r.clicked() {
                        before = Some(on.then(|| events::pre_day(&s.items).filter(|p| *p < s.day).or_else(|| earlier.last().map(|d| d.to_string()))).flatten());
                    }
                    if let Some(left) = &s.before {
                        let at = earlier.iter().position(|d| *d == left);
                        ui.label(tr(lang, "Left:"));
                        if ui.add_enabled(at.is_some_and(|n| n > 0), egui::Button::new("‹")).clicked() {
                            before = Some(at.and_then(|n| earlier.get(n.wrapping_sub(1))).map(|d| d.to_string()));
                        }
                        ui.strong(left);
                        if ui.add_enabled(at.is_some_and(|n| n + 1 < earlier.len()), egui::Button::new("›")).clicked() {
                            before = Some(at.and_then(|n| earlier.get(n + 1)).map(|d| d.to_string()));
                        }
                    }
                });
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(chosen.is_some(), egui::Button::new(tr(lang, "Show on the globe"))).clicked() {
                    show = chosen.cloned();
                }
                if ui.add_enabled(self.shown.is_some() || self.crawl.is_some(), egui::Button::new(tr(lang, "Remove from the globe"))).clicked() {
                    clear = true;
                }
            });
        });
        self.open = open;
        if let Some(e) = show {
            self.show_event(lang, e, out);
        }
        if clear {
            self.clear(out);
            self.status.clear();
        }
        if let Some(d) = day {
            self.set_day(&d, out);
            // Левый день шторки не позже правого.
            if let Some(s) = &self.shown
                && s.before.as_ref().is_some_and(|b| *b >= s.day)
            {
                let earlier = s.days.iter().rfind(|x| **x < s.day).cloned();
                self.set_before(earlier, out);
            }
        }
        if let Some(b) = before {
            if b.is_some() && self.split <= 0.0 {
                self.split = 0.5;
            }
            self.set_before(b, out);
        }
    }

    /// Тайлы снимков стороны side (0 - «после», 1 - «до»): кадр просит
    /// keys, пришедшие - (тайл, RGBA или None).
    pub fn tiles(&mut self, side: usize, keys: &[globe_core::lod::TileKey], request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Vec<(globe_core::lod::TileKey, Option<Vec<u8>>)> {
        let painter = if side == 0 { &mut self.tiles } else { &mut self.before_tiles };
        match painter {
            Some(p) => p.step(keys, request),
            None => Vec::new(),
        }
    }
}

/// Заголовок снимка: читается, готов или не читается.
enum Head {
    Reading(HeaderRead),
    Ready(Arc<CogHeader>),
    Failed,
}

/// Тайл снимка: уровень, столбец, строка.
type Part = (String, usize, u32, u32);

/// Сборка тайла карты: следующий снимок по порядку и накопленная
/// картинка с премноженной альфой.
struct Job {
    next: usize,
    color: Vec<f32>,
    alpha: Vec<f32>,
}

/// Тайлы карты на день из снимков, новые сверху.
struct Painter {
    items: Vec<Item>,
    heads: HashMap<String, Head>,
    head_order: VecDeque<String>,
    parts: HashMap<Part, Option<Arc<Vec<u8>>>>,
    part_order: VecDeque<Part>,
    part_bytes: usize,
    asks: Vec<PartAsk>,
    raw: VecDeque<(Part, Vec<Vec<u8>>)>,
    jobs: HashMap<globe_core::lod::TileKey, Job>,
}

/// Долготы и широты точек тайла: строки сверху вниз, центры пикселей.
fn grid_point(key: globe_core::lod::TileKey, gx: f64, gy: f64) -> (f64, f64) {
    let n = f64::from(1u32 << key.0.min(30));
    let lon = (f64::from(key.1) + gx) / n * 360.0 - 180.0;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * (f64::from(key.2) + gy) / n)).sinh().atan().to_degrees();
    (lon, lat)
}

/// Система, в которую считает [`to_crs`].
fn known_crs(epsg: Option<u32>) -> bool {
    matches!(epsg, None | Some(4326 | 3857 | 32601..=32660 | 32701..=32760))
}

/// Точка в систему файла: EPSG:4326, 3857, UTM WGS84 326xx и 327xx.
fn to_crs(epsg: Option<u32>, lon: f64, lat: f64) -> Option<(f64, f64)> {
    match epsg {
        None | Some(4326) => Some((lon, lat)),
        Some(3857) => {
            let phi = lat.clamp(-85.0511, 85.0511).to_radians();
            Some((RADIUS * lon.to_radians(), RADIUS * (std::f64::consts::FRAC_PI_4 + phi / 2.0).tan().ln()))
        }
        Some(code @ (32601..=32660 | 32701..=32760)) => {
            let (zone, south) = (code % 100, code >= 32701);
            let (_, _, e, n) = scene::coords::to_utm(lat, lon, Some(zone))?;
            let n = n - if lat < 0.0 { FALSE_NORTHING } else { 0.0 } + if south { FALSE_NORTHING } else { 0.0 };
            Some((e, n))
        }
        _ => None,
    }
}

impl Painter {
    fn new(items: Vec<Item>) -> Self {
        Self { items, heads: HashMap::new(), head_order: VecDeque::new(), parts: HashMap::new(), part_order: VecDeque::new(), part_bytes: 0, asks: Vec::new(), raw: VecDeque::new(), jobs: HashMap::new() }
    }

    fn busy(&self) -> bool {
        !self.jobs.is_empty() || !self.asks.is_empty() || !self.raw.is_empty()
    }

    fn head(&mut self, href: &str, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Option<Option<Arc<CogHeader>>> {
        if !self.heads.contains_key(href) {
            self.heads.insert(href.to_string(), Head::Reading(HeaderRead::new(href, request)));
            self.head_order.push_back(href.to_string());
            while self.head_order.len() > COGS_KEPT {
                if let Some(old) = self.head_order.pop_front() {
                    self.heads.remove(&old);
                }
            }
        }
        match self.heads.get(href)? {
            Head::Reading(_) => None,
            Head::Ready(h) => Some(Some(h.clone())),
            Head::Failed => Some(None),
        }
    }

    fn keep_part(&mut self, part: Part, rgba: Option<Arc<Vec<u8>>>) {
        self.part_bytes += rgba.as_ref().map_or(0, |r| r.len());
        self.parts.insert(part.clone(), rgba);
        self.part_order.push_back(part);
        while self.part_bytes > CACHE_BYTES {
            let Some(old) = self.part_order.pop_front() else {
                break;
            };
            if let Some(Some(r)) = self.parts.remove(&old) {
                self.part_bytes = self.part_bytes.saturating_sub(r.len());
            }
        }
    }

    fn step(&mut self, keys: &[globe_core::lod::TileKey], request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Vec<(globe_core::lod::TileKey, Option<Vec<u8>>)> {
        // Заголовки.
        for head in self.heads.values_mut() {
            if let Head::Reading(read) = head {
                read.step(request);
                match read.result() {
                    Some(Ok(h)) if h.is_picture() => *head = Head::Ready(h.clone()),
                    Some(_) => *head = Head::Failed,
                    None => {}
                }
            }
        }
        // Тайлы снимков: ответы, раскодирование в бюджете кадра.
        let mut done = Vec::new();
        for (part, _, asks, got) in &mut self.asks {
            let mut failed = false;
            for (n, (_, ask)) in asks.iter().enumerate() {
                if got[n].is_none()
                    && let Some(answer) = ask.take()
                {
                    match answer {
                        Ok(b) => got[n] = Some(b),
                        Err(_) => failed = true,
                    }
                }
            }
            if failed || got.iter().all(Option::is_some) {
                done.push((part.clone(), failed));
            }
        }
        for (part, failed) in done {
            let Some(at) = self.asks.iter().position(|a| a.0 == part) else {
                continue;
            };
            let (part, _, _, got) = self.asks.remove(at);
            if failed {
                self.keep_part(part, None);
            } else {
                self.raw.push_back((part, got.into_iter().flatten().collect()));
            }
        }
        for _ in 0..DECODES {
            let Some((part, raws)) = self.raw.pop_front() else {
                break;
            };
            let rgba = match self.heads.get(&part.0) {
                Some(Head::Ready(h)) => {
                    let slices: Vec<&[u8]> = raws.iter().map(Vec::as_slice).collect();
                    data::cog::decode_picture(h, part.1, &slices).ok().map(Arc::new)
                }
                _ => None,
            };
            self.keep_part(part, rgba);
        }
        // Сборка тайлов карты.
        let wanted: HashSet<globe_core::lod::TileKey> = keys.iter().copied().collect();
        self.jobs.retain(|k, _| wanted.contains(k));
        for key in keys {
            if self.jobs.len() >= JOBS {
                break;
            }
            self.jobs.entry(*key).or_insert_with(|| Job { next: 0, color: vec![0.0; SIDE * SIDE * 3], alpha: vec![0.0; SIDE * SIDE] });
        }
        let mut out = Vec::new();
        let keys: Vec<globe_core::lod::TileKey> = self.jobs.keys().copied().collect();
        for key in keys {
            if let Some(finished) = self.advance(key, request) {
                self.jobs.remove(&key);
                out.push((key, finished));
            }
        }
        out
    }

    /// Шаг сборки тайла key. Some - тайл готов: RGBA или пусто.
    fn advance(&mut self, key: globe_core::lod::TileKey, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Option<Option<Vec<u8>>> {
        let tile_box = events::tile_box(key.0, key.1, key.2);
        loop {
            let next = self.jobs.get(&key)?.next;
            let Some(item) = self.items.get(next).cloned() else {
                return Some(self.finish(key));
            };
            if !events::touches(item.bbox, tile_box) {
                self.jobs.get_mut(&key)?.next += 1;
                continue;
            }
            let header = match self.head(&item.href, request)? {
                Some(h) if known_crs(h.epsg) => h,
                _ => {
                    self.jobs.get_mut(&key)?.next += 1;
                    continue;
                }
            };
            // Точки тайла в системе файла по сетке GRID × GRID.
            let mut nodes = Vec::with_capacity(GRID * GRID);
            for gy in 0..GRID {
                for gx in 0..GRID {
                    let (lon, lat) = grid_point(key, gx as f64 / (GRID - 1) as f64, gy as f64 / (GRID - 1) as f64);
                    nodes.push(to_crs(header.epsg, lon, lat));
                }
            }
            if nodes.iter().any(Option::is_none) {
                self.jobs.get_mut(&key)?.next += 1;
                continue;
            }
            let nodes: Vec<(f64, f64)> = nodes.into_iter().flatten().collect();
            let step = ((nodes[1].0 - nodes[0].0).hypot(nodes[1].1 - nodes[0].1)) * (GRID - 1) as f64 / SIDE as f64;
            let level = data::cog::level_for_resolution(&header, step);
            let (Some(lvl), Some(g)) = (header.levels.get(level), header.level_transform(level)) else {
                self.jobs.get_mut(&key)?.next += 1;
                continue;
            };
            let det = g[1] * g[5] - g[2] * g[4];
            if !det.is_finite() || det == 0.0 || lvl.tile_width == 0 || lvl.tile_height == 0 || lvl.tile_width > PART_SIDE || lvl.tile_height > PART_SIDE {
                self.jobs.get_mut(&key)?.next += 1;
                continue;
            }
            let pixel = |x: f64, y: f64| -> (f64, f64) {
                let (dx, dy) = (x - g[0], y - g[3]);
                ((g[5] * dx - g[2] * dy) / det - 0.5, (g[1] * dy - g[4] * dx) / det - 0.5)
            };
            // Тайлы снимка, которые задевает тайл карты.
            let (mut c0, mut c1, mut r0, mut r1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
            for (x, y) in &nodes {
                let (c, r) = pixel(*x, *y);
                c0 = c0.min(c);
                c1 = c1.max(c);
                r0 = r0.min(r);
                r1 = r1.max(r);
            }
            let (w, h) = (f64::from(lvl.width), f64::from(lvl.height));
            if c1 < -0.5 || r1 < -0.5 || c0 > w - 0.5 || r0 > h - 0.5 {
                self.jobs.get_mut(&key)?.next += 1;
                continue;
            }
            let (tw, th) = (f64::from(lvl.tile_width), f64::from(lvl.tile_height));
            let cols = (c0.max(0.0) / tw).floor() as u32..=((c1 + 1.0).min(w - 1.0) / tw).floor() as u32;
            let rows = (r0.max(0.0) / th).floor() as u32..=((r1 + 1.0).min(h - 1.0) / th).floor() as u32;
            let mut missing = false;
            for row in rows.clone() {
                for col in cols.clone() {
                    let part = (item.href.clone(), level, col, row);
                    if self.parts.contains_key(&part) || self.raw.iter().any(|(p, _)| *p == part) {
                        continue;
                    }
                    missing = true;
                    if self.asks.iter().any(|a| a.0 == part) || self.asks.len() >= IN_FLIGHT {
                        continue;
                    }
                    match data::cog::picture_ranges(&header, level, col, row) {
                        Some(ranges) => {
                            let asks = ranges
                                .iter()
                                .map(|r| {
                                    let ask = request(PART_BYTES);
                                    let range = format!("bytes={}-{}", r.start, r.end.saturating_sub(1));
                                    ask.send(&item.href, &[("Range", range.as_str())], None);
                                    (r.start, ask)
                                })
                                .collect();
                            let n = ranges.len();
                            self.asks.push((part, ranges, asks, vec![None; n]));
                        }
                        None => self.keep_part(part, None),
                    }
                }
            }
            if missing {
                return None;
            }
            // Все тайлы снимка здесь: снимок ложится под собранное.
            let (tw, th) = (lvl.tile_width as usize, lvl.tile_height as usize);
            let mut local: HashMap<(usize, usize), Arc<Vec<u8>>> = HashMap::new();
            for row in rows {
                for col in cols.clone() {
                    if let Some(Some(tile)) = self.parts.get(&(item.href.clone(), level, col, row)) {
                        local.insert((col as usize, row as usize), tile.clone());
                    }
                }
            }
            let at = |c: i64, r: i64| -> Option<[f32; 4]> {
                if c < 0 || r < 0 || c as f64 >= w || r as f64 >= h {
                    return None;
                }
                let (c, r) = (c as usize, r as usize);
                let tile = local.get(&(c / tw, r / th))?;
                let i = ((r % th) * tw + c % tw) * 4;
                let p = tile.get(i..i + 4)?;
                Some([f32::from(p[0]), f32::from(p[1]), f32::from(p[2]), f32::from(p[3])])
            };
            let mut color = std::mem::take(&mut self.jobs.get_mut(&key)?.color);
            let mut alpha = std::mem::take(&mut self.jobs.get_mut(&key)?.alpha);
            let mut full = true;
            for py in 0..SIDE {
                for px in 0..SIDE {
                    let o = py * SIDE + px;
                    if alpha[o] >= 254.5 {
                        continue;
                    }
                    // Точка пикселя в системе файла по сетке узлов.
                    let (u, v) = ((px as f64 + 0.5) / SIDE as f64 * (GRID - 1) as f64, (py as f64 + 0.5) / SIDE as f64 * (GRID - 1) as f64);
                    let (i, j) = ((u.floor() as usize).min(GRID - 2), (v.floor() as usize).min(GRID - 2));
                    let (tx, ty) = (u - i as f64, v - j as f64);
                    let n = |a: usize, b: usize| nodes[b * GRID + a];
                    let lerp = |a: (f64, f64), b: (f64, f64), t: f64| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                    let (x, y) = lerp(lerp(n(i, j), n(i + 1, j), tx), lerp(n(i, j + 1), n(i + 1, j + 1), tx), ty);
                    let (fx, fy) = pixel(x, y);
                    if fx <= -0.5 || fy <= -0.5 || fx >= w - 0.5 || fy >= h - 0.5 {
                        full = false;
                        continue;
                    }
                    let (fx, fy) = (fx.clamp(0.0, w - 1.0), fy.clamp(0.0, h - 1.0));
                    let (ix, iy) = (fx.floor() as i64, fy.floor() as i64);
                    let (wx, wy) = ((fx - fx.floor()) as f32, (fy - fy.floor()) as f32);
                    let mut rgb = [0.0f32; 3];
                    let mut weight = 0.0f32;
                    for (dx, dy, wt) in [(0, 0, (1.0 - wx) * (1.0 - wy)), (1, 0, wx * (1.0 - wy)), (0, 1, (1.0 - wx) * wy), (1, 1, wx * wy)] {
                        if let Some(p) = at((ix + dx).min(w as i64 - 1), (iy + dy).min(h as i64 - 1)) {
                            let wa = wt * p[3] / 255.0;
                            rgb[0] += wa * p[0];
                            rgb[1] += wa * p[1];
                            rgb[2] += wa * p[2];
                            weight += wa;
                        }
                    }
                    if weight <= 0.0 {
                        full = false;
                        continue;
                    }
                    // Нижний снимок ложится под собранные, по прозрачности.
                    let rest = 1.0 - alpha[o] / 255.0;
                    let share = weight.min(1.0) * rest;
                    for k in 0..3 {
                        color[o * 3 + k] += rgb[k] / weight * share;
                    }
                    alpha[o] += share * 255.0;
                    full &= alpha[o] >= 254.5;
                }
            }
            let job = self.jobs.get_mut(&key)?;
            job.color = color;
            job.alpha = alpha;
            job.next += 1;
            if full {
                return Some(self.finish(key));
            }
        }
    }

    /// Готовый тайл RGBA без премножения или None - снимков в нём нет.
    fn finish(&self, key: globe_core::lod::TileKey) -> Option<Vec<u8>> {
        let job = self.jobs.get(&key)?;
        if job.alpha.iter().all(|a| *a <= 0.0) {
            return None;
        }
        let mut out = Vec::with_capacity(SIDE * SIDE * 4);
        for (o, a) in job.alpha.iter().enumerate() {
            let share = (a / 255.0).max(1e-6);
            for k in 0..3 {
                out.push((job.color[o * 3 + k] / share).round().clamp(0.0, 255.0) as u8);
            }
            out.push(a.round().clamp(0.0, 255.0) as u8);
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::{known_crs, to_crs};

    #[test]
    fn crs_of_event_images() {
        assert!(known_crs(Some(32633)) && known_crs(Some(3857)) && !known_crs(Some(2154)));
        // Центральный меридиан зоны 33 - 15° в. д., восток 500 км.
        let (e, n) = to_crs(Some(32633), 15.0, 50.0).unwrap_or_default();
        assert!((e - 500_000.0).abs() < 1e-3 && (n - 5_540_000.0).abs() < 10_000.0);
        // Южное полушарие в 326xx - север отрицательный, в 327xx - с ложным севером.
        let (_, north) = to_crs(Some(32633), 15.0, -10.0).unwrap_or_default();
        let (_, south) = to_crs(Some(32733), 15.0, -10.0).unwrap_or_default();
        assert!(north < 0.0 && (south - north - 10_000_000.0).abs() < 1e-6);
        let (x, _) = to_crs(Some(3857), 180.0, 0.0).unwrap_or_default();
        assert!((x - 20_037_508.342_789_244).abs() < 1e-6);
    }
}
