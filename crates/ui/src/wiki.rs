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
