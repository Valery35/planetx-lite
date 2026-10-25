// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Загрузка объектов OpenStreetMap через Overpass по тайлам вида.
//! Перенос `ui/overpass.py` модуля.
//!
//! Общий код строк «Шахты и карьеры OSM» и «ЛЭП и трубопроводы OSM»:
//! тайлы уровня `mining::ZOOM` у точки взгляда ниже `mining::MAX_ALTITUDE`,
//! недостающие тайлы - одним запросом POST по общей рамке. Сервер -
//! из `mining::OSM_SERVERS`, после отказа - следующий, не чаще
//! `mining::GAP` к одному серверу. После отказа тайлы возвращаются
//! в очередь. Выключение строки посреди запроса - не отказ.

use std::collections::HashMap;

use data::mining;

/// Наибольший ответ, байт.
const ANSWER_BYTES: u64 = 64 << 20;
/// Пауза после отказа, секунды.
pub const RETRY: f64 = 30.0;

type Tile = (u32, u32);

/// Серверы Overpass, общие для всех строк, `MIRRORS` модуля: отказ
/// сервера в одной строке уводит от него и другие.
#[derive(Debug, Clone)]
pub struct Mirrors {
    server: usize,
    /// Время последнего запроса к каждому серверу.
    sent: [f64; 3],
    paused_until: f64,
}

impl Default for Mirrors {
    fn default() -> Self {
        Self { server: 0, sent: [f64::NEG_INFINITY; 3], paused_until: f64::NEG_INFINITY }
    }
}

impl Mirrors {
    /// Сервер, к которому можно идти сейчас.
    fn pick(&self, now: f64) -> Option<&'static str> {
        let gap = self.sent.get(self.server).copied().unwrap_or(f64::NEG_INFINITY);
        (now >= self.paused_until && now - gap >= mining::GAP).then(|| mining::OSM_SERVERS.get(self.server).copied()).flatten()
    }

    fn sent(&mut self, now: f64) {
        if let Some(slot) = self.sent.get_mut(self.server) {
            *slot = now;
        }
    }

    /// Отказ: следующий сервер после паузы.
    fn failed(&mut self, now: f64) {
        self.server = (self.server + 1) % mining::OSM_SERVERS.len();
        self.paused_until = now + RETRY;
    }

    /// Пауза после отказа.
    pub fn paused(&self, now: f64) -> bool {
        now < self.paused_until
    }
}

pub struct Overpass<T> {
    query: fn(mining::Bounds) -> String,
    parse: fn(&[u8]) -> Vec<T>,
    items: HashMap<Tile, Vec<T>>,
    queue: Vec<Tile>,
    ask: Option<(Vec<Tile>, Box<dyn net::Request>)>,
    /// Отказ в этом шаге: сообщение пользователю.
    pub failed: bool,
    /// Пришли новые объекты.
    pub dirty: bool,
}

impl<T: Clone> Overpass<T> {
    pub fn new(query: fn(mining::Bounds) -> String, parse: fn(&[u8]) -> Vec<T>) -> Self {
        Self { query, parse, items: HashMap::new(), queue: Vec::new(), ask: None, failed: false, dirty: false }
    }

    /// Идёт запрос или ждут тайлы без паузы после отказа.
    pub fn busy(&self, now: f64, mirrors: &Mirrors) -> bool {
        self.ask.is_some() || (!self.queue.is_empty() && !mirrors.paused(now))
    }

    /// Все объекты.
    pub fn all(&self) -> Vec<T> {
        self.items.values().flatten().cloned().collect()
    }

    /// Шаг: тайлы у точки взгляда, ответ, следующий запрос.
    pub fn step(&mut self, now: f64, view: (f64, f64, f64), mirrors: &mut Mirrors, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        self.failed = false;
        let (lat, lon, altitude) = view;
        self.queue = mining::view_tiles(lat, lon, altitude, mining::ZOOM, mining::MAX_TILES).into_iter().filter(|t| !self.items.contains_key(t)).collect();
        if let Some((tiles, ask)) = &self.ask
            && let Some(answer) = ask.take()
        {
            let tiles = tiles.clone();
            self.ask = None;
            match answer {
                Ok(data) => {
                    let found = (self.parse)(&data);
                    for (n, tile) in tiles.iter().enumerate() {
                        // Объекты общей рамки - у первого тайла, прочие
                        // помечены загруженными.
                        self.items.insert(*tile, if n == 0 { found.clone() } else { Vec::new() });
                    }
                    self.queue.retain(|t| !tiles.contains(t));
                    self.dirty = true;
                }
                Err(_) => {
                    self.failed = true;
                    mirrors.failed(now);
                }
            }
        }
        if self.ask.is_some() || self.queue.is_empty() {
            return;
        }
        let Some(server) = mirrors.pick(now) else {
            return;
        };
        let Some(bounds) = mining::union(&self.queue.iter().map(|t| mining::tile_bounds(t.0, t.1, mining::ZOOM)).collect::<Vec<_>>()) else {
            return;
        };
        mirrors.sent(now);
        let ask = request(ANSWER_BYTES);
        let body = data::infra::form_body(&(self.query)(bounds));
        ask.send(server, &[("Content-Type", data::infra::CONTENT_TYPE)], Some(body.as_bytes()));
        self.ask = Some((self.queue.clone(), ask));
    }
}
