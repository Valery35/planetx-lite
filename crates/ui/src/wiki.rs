// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! «Википедия рядом»: статьи с координатами у точки взгляда. Перенос
//! `ui/wikipedia.py` и `ui/globe/wiki.py` модуля, ядро -
//! `data::wikipedia`.
//!
//! Тайлы вокруг точки взгляда просятся по одному, не чаще
//! `wikipedia::GAP`. После отказа службы тайл возвращается в очередь через
//! [`RETRY`]. Смена языка интерфейса сбрасывает пришедшие тайлы. Статьи -
//! точки с подписями, важные и ближние первыми. Щелчок по статье даёт
//! в окне «Объекты» ссылку и первый абзац статьи.

use std::collections::{HashMap, VecDeque};

use data::wikipedia::{self, Article};
use globe_core::DVec3;
use globe_core::camera::Camera;

use crate::i18n::{Lang, tr};
use crate::identify::Feature;
use crate::layers::Category;
use crate::registry;
use crate::thumbs::Recipe;

/// Пауза после отказа службы, секунды.
pub const RETRY: f64 = 30.0;
/// Тайлов в памяти, давние уходят первыми.
pub const KEPT_TILES: usize = 400;
/// Наибольший ответ, байт.
const ANSWER_BYTES: u64 = 8 << 20;
/// Подписей статей на экране сразу.
pub const LABELS: usize = 120;
/// Цвет точки статьи, MAP_COLORS["wiki"] модуля.
pub const COLOR: [u8; 3] = [45, 110, 205];
/// Сторона точки статьи, пиксели.
pub const POINT_SIZE: f32 = 7.0;
/// Радиус выбора статьи щелчком, пиксели.
pub const PICK_PIXELS: f64 = 12.0;

type Tile = (u32, u32);
/// Первый абзац: название статьи, запрос, ответ.
type Summary = (String, Option<Box<dyn net::Request>>, Option<String>);

pub struct Wiki {
    lang: String,
    tiles: HashMap<Tile, Vec<Article>>,
    /// Порядок прихода тайлов для вытеснения.
    arrived: VecDeque<Tile>,
    queue: Vec<Tile>,
    ask: Option<(Tile, Box<dyn net::Request>)>,
    sent: f64,
    paused_until: f64,
    /// Отказ службы в этом шаге: сообщение пользователю.
    pub failed: bool,
    /// Набор статей сменился.
    pub dirty: bool,
    /// Первый абзац статьи: название, запрос, ответ.
    summary: Option<Summary>,
}

impl Default for Wiki {
    fn default() -> Self {
        Self { lang: "en".into(), tiles: HashMap::new(), arrived: VecDeque::new(), queue: Vec::new(), ask: None, sent: f64::NEG_INFINITY, paused_until: f64::NEG_INFINITY, failed: false, dirty: false, summary: None }
    }
}

impl Wiki {
    /// Язык раздела.
    pub fn lang(&self) -> &str {
        &self.lang
    }

    /// Идёт запрос или ждут тайлы, без паузы после отказа.
    pub fn busy(&self) -> bool {
        self.ask.is_some() || (!self.queue.is_empty() && self.paused_until < self.sent)
    }

    /// Шаг: недостающие тайлы у точки взгляда - в очередь, пришедший
    /// ответ - в тайлы, следующий запрос - по паузе.
    pub fn step(&mut self, now: f64, view: (f64, f64, f64), ui_lang: &str, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        self.failed = false;
        let lang = wikipedia::language(ui_lang);
        if lang != self.lang {
            self.ask = None;
            self.tiles.clear();
            self.arrived.clear();
            self.lang = lang;
            self.dirty = true;
        }
        let (lat, lon, altitude) = view;
        self.queue = wikipedia::view_tiles(lat, lon, altitude, wikipedia::VIEW_TILES).into_iter().filter(|t| !self.tiles.contains_key(t)).collect();
        if let Some((tile, ask)) = &self.ask
            && let Some(answer) = ask.take()
        {
            let tile = *tile;
            self.ask = None;
            match answer.and_then(|data| wikipedia::parse_geosearch(&data)) {
                Ok(found) => {
                    self.tiles.insert(tile, found);
                    self.arrived.push_back(tile);
                    while self.arrived.len() > KEPT_TILES {
                        if let Some(old) = self.arrived.pop_front() {
                            self.tiles.remove(&old);
                        }
                    }
                    self.queue.retain(|t| *t != tile);
                    self.dirty = true;
                }
                Err(_) => {
                    self.paused_until = now + RETRY;
                    self.failed = true;
                }
            }
        }
        if self.ask.is_none() && now >= self.sent + wikipedia::GAP && now >= self.paused_until
            && let Some(tile) = self.queue.first().copied()
        {
            self.sent = now;
            let ask = request(ANSWER_BYTES);
            // origin=* - запрос без учётной записи со страницы WebAssembly.
            ask.start(&format!("{}&origin=*", wikipedia::geosearch_url(&self.lang, tile)));
            self.ask = Some((tile, ask));
        }
        if let Some((_, ask, text)) = &mut self.summary
            && let Some(a) = ask.as_ref()
            && let Some(answer) = a.take()
        {
            *ask = None;
            *text = Some(answer.and_then(|d| wikipedia::parse_summary(&d)).unwrap_or_default());
            self.dirty = true;
        }
    }

    /// Статьи: важные и ближние к точке взгляда первыми.
    pub fn articles(&self, lat: f64, lon: f64) -> Vec<Article> {
        let all: Vec<Article> = self.tiles.values().flatten().cloned().collect();
        wikipedia::order(&all, lat, lon, wikipedia::MAX_MARKS)
    }

    /// Снять запросы и статьи: строка выключена.
    pub fn clear(&mut self) {
        self.ask = None;
        self.queue.clear();
        self.summary = None;
    }

    /// Попросить первый абзац статьи, если его ещё нет.
    pub fn ask_summary(&mut self, title: &str, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        if self.summary.as_ref().is_some_and(|(t, _, _)| t == title) {
            return;
        }
        let ask = request(ANSWER_BYTES);
        ask.start(&wikipedia::summary_url(&self.lang, title));
        self.summary = Some((title.to_string(), Some(ask), None));
    }

    /// Первый абзац статьи title, если пришёл.
    pub fn summary_of(&self, title: &str) -> Option<&str> {
        self.summary.as_ref().filter(|(t, _, _)| t == title).and_then(|(_, _, text)| text.as_deref())
    }
}

/// Пауза между сборками подписей без новых статей, секунды.
const REBUILD: f64 = 0.5;

/// Строка реестра «Википедия рядом».
pub struct Entry;

/// Строка «Википедия рядом» в [`crate::registry::ALL`].
pub static ENTRY: Entry = Entry;

impl registry::Entry for Entry {
    fn key(&self) -> &'static str {
        "wiki"
    }
    fn name(&self) -> &'static str {
        "Wikipedia nearby"
    }
    fn tip(&self) -> &'static str {
        "Wikipedia articles with coordinates near the look-at point, the important and the near ones first. They show below 40 km of camera height. A click on an article gives its link and first paragraph in the Objects window."
    }
    fn category(&self) -> Category {
        Category::Wiki
    }
    fn source(&self) -> &'static str {
        "Wikipedia"
    }
    fn words(&self) -> &'static [&'static str] {
        &["википедия", "энциклопедия", "статьи", "достопримечательности", "wikipedia", "encyclopedia", "articles", "landmarks"]
    }
    fn legend(&self) -> Option<Recipe> {
        data::thumbs::colors(self.key()).map(|c| Recipe::Ramp(c.to_vec()))
    }
    fn make(&self, _cx: &registry::Make<'_>) -> (Box<dyn registry::Layer>, Option<String>) {
        (Box::new(Shown { wiki: Wiki::default(), labels: Vec::new(), built: f64::NEG_INFINITY }), None)
    }
}

/// Включённая строка: статьи и их подписи на глобусе.
struct Shown {
    wiki: Wiki,
    /// Подписи статей: название и точка глобуса.
    labels: Vec<(String, DVec3)>,
    /// Время последней сборки подписей.
    built: f64,
}

impl Shown {
    /// Статьи у пикселя, ближние первыми: расстояние в пикселях и название.
    fn near(&self, camera: &Camera, pixel: (f64, f64, f64)) -> Vec<(f64, &(String, DVec3))> {
        let (px, py, ppp) = pixel;
        let mut near: Vec<(f64, &(String, DVec3))> = self.labels.iter().filter_map(|l| registry::gap(camera, l.1, px, py, PICK_PIXELS * ppp).map(|g| (g, l))).collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        near
    }
}

impl registry::Layer for Shown {
    fn step(&mut self, cx: &registry::Ctx<'_>) -> registry::Stepped {
        let (lat, lon, altitude, _) = cx.view;
        self.wiki.step(cx.now, (lat, lon, altitude), cx.lang.code(), cx.request);
        let message = self.wiki.failed.then(|| tr(cx.lang, "Wikipedia did not answer, retry in 30 s.").to_string());
        // Набор и порядок подписей - по приходу статей и движению вида.
        if self.wiki.dirty || cx.now - self.built >= REBUILD {
            self.wiki.dirty = false;
            self.built = cx.now;
            let articles = if altitude <= wikipedia::MAX_ALTITUDE { self.wiki.articles(lat, lon) } else { Vec::new() };
            let e = cx.camera.ellipsoid;
            self.labels = articles.into_iter().take(LABELS).map(|a| {
                let p = e.geodetic_to_ecef(a.lat, a.lon, (cx.ground)(a.lat, a.lon));
                (a.title, p)
            }).collect();
        }
        registry::Stepped { message, ..Default::default() }
    }
    fn labels(&self) -> &[(String, DVec3)] {
        &self.labels
    }
    fn dot(&self) -> Option<([u8; 3], f32)> {
        Some((COLOR, POINT_SIZE))
    }
    fn busy(&self) -> bool {
        self.wiki.busy()
    }
    fn attribution(&self) -> Option<(&'static str, &'static str)> {
        (!self.labels.is_empty()).then_some(wikipedia::ATTRIBUTION)
    }
    /// Статьи у пикселя, `_identify_wiki` модуля.
    fn pick(&self, lang: Lang, camera: &Camera, pixel: (f64, f64, f64)) -> Option<(String, Vec<Feature>)> {
        let features: Vec<Feature> = self
            .near(camera, pixel)
            .iter()
            .take(10)
            .map(|(_, (title, _))| {
                let mut fields = vec![(tr(lang, "Article").to_string(), wikipedia::page_url(self.wiki.lang(), title))];
                if let Some(text) = self.wiki.summary_of(title).filter(|t| !t.is_empty()) {
                    fields.insert(0, (tr(lang, "Summary").to_string(), text.to_string()));
                }
                (title.clone(), fields)
            })
            .collect();
        (!features.is_empty()).then(|| (tr(lang, "Wikipedia").to_string(), features))
    }
    /// Щелчок по статье без инструмента, `open_article_card` модуля.
    fn hit(&self, camera: &Camera, pixel: (f64, f64, f64)) -> Option<DVec3> {
        self.near(camera, pixel).first().map(|(_, (_, p))| *p)
    }
    /// Первый абзац ближней статьи у щелчка.
    fn clicked(&mut self, lang: Lang, groups: &[(String, Vec<Feature>)], request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        let name = tr(lang, "Wikipedia");
        if let Some((title, _)) = groups.iter().find(|(g, _)| g == name).and_then(|(_, f)| f.first()) {
            self.wiki.ask_summary(title, request);
        }
    }
    /// Пришедший первый абзац - в группу «Википедия» окна «Объекты».
    fn enrich(&self, lang: Lang, groups: &mut [(String, Vec<Feature>)]) {
        let (name, summary) = (tr(lang, "Wikipedia"), tr(lang, "Summary"));
        for (_, features) in groups.iter_mut().filter(|(g, _)| g == name) {
            for (title, fields) in features.iter_mut() {
                if let Some(text) = self.wiki.summary_of(title).filter(|t| !t.is_empty())
                    && !fields.iter().any(|(k, _)| k == summary)
                {
                    fields.insert(0, (summary.to_string(), text.to_string()));
                }
            }
        }
    }
}
