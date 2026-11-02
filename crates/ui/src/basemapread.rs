// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Тайлы подложки глобуса под участком по сети для окон анализа,
//! перенос `ui/tilemosaic.py` и `gather` из `core/tilemosaic.py`
//! модуля. Сборка мозаики - `data::tilemosaic`.
//!
//! Тайлы просятся разовыми запросами в обход загрузчика глобуса: его
//! очередь живёт кадром вида, а мозаике нужны все тайлы рамки. Запросов
//! сразу - не больше, чем разрешает источник. Тайла нет или вместо снимка
//! заглушка - берётся предок, не выше `MAX_FILL_DEPTH` уровней, как на
//! глобусе. Прозрачный тайл - ответ «здесь пусто», заглушкой он
//! не считается.

use std::collections::{BTreeSet, HashMap};

use data::tilemosaic::{self, Frame, Tiles};
use net::placeholder::{MAX_FILL_DEPTH, is_placeholder};

use crate::aefcalc::Request;

/// Предел ответа на тайл, байты.
const TILE_BYTES: u64 = 8 << 20;

/// Ключ тайла (z, x, y).
type Key = (u32, u32, u32);

/// Загрузка тайлов рамки.
pub struct BaseRead {
    source: net::Source,
    /// Тайлы рамки без картинки.
    left: Vec<Key>,
    /// Уровень предков, у которых ищутся картинки.
    depth: u32,
    /// Предки текущего уровня, ещё не запрошенные.
    queue: Vec<Key>,
    asks: Vec<(Key, Box<dyn net::Request>)>,
    /// Ответы текущего уровня: картинка RGBA или None.
    got: HashMap<Key, Option<Vec<u8>>>,
    pub tiles: Tiles,
    /// Тайлов рамки.
    pub total: usize,
}

impl BaseRead {
    pub fn new(source: net::Source, frame: Frame) -> Self {
        let left = frame.keys();
        let total = left.len();
        let mut read = Self { source, left, depth: 0, queue: Vec::new(), asks: Vec::new(), got: HashMap::new(), tiles: Tiles { frame: Some(frame), images: HashMap::new() }, total };
        read.queue = read.ancestors();
        read
    }

    /// Предки оставшихся тайлов на нынешнем уровне.
    fn ancestors(&self) -> Vec<Key> {
        let found: BTreeSet<Key> = self.left.iter().filter(|k| self.depth <= k.0).map(|k| tilemosaic::ancestor(*k, self.depth)).collect();
        found.into_iter().collect()
    }

    /// Тайлов рамки с картинкой.
    pub fn found(&self) -> usize {
        self.tiles.images.len()
    }

    /// Картинка ответа: RGBA [`tilemosaic::SIZE`] × [`tilemosaic::SIZE`]
    /// или None - не картинка или заглушка.
    fn picture(&self, bytes: &[u8]) -> Option<Vec<u8>> {
        let image = image::load_from_memory(bytes).ok()?.to_rgba8();
        let side = tilemosaic::SIZE as u32;
        let image = if image.width() == side && image.height() == side { image } else { image::imageops::resize(&image, side, side, image::imageops::FilterType::Triangle) };
        let rgba = image.into_raw();
        let empty = !rgba.chunks(4).any(|p| p[3] > 0);
        (empty || !(self.source.placeholders && is_placeholder(&rgba))).then_some(rgba)
    }

    /// Шаг загрузки. true - готово: картинки нашлись у всех тайлов или
    /// предки кончились.
    pub fn step(&mut self, request: Request) -> bool {
        // Пришедшие ответы.
        let mut i = 0;
        while i < self.asks.len() {
            match self.asks[i].1.take() {
                Some(answer) => {
                    let (key, _) = self.asks.swap_remove(i);
                    let picture = answer.ok().and_then(|b| self.picture(&b));
                    self.got.insert(key, picture);
                }
                None => i += 1,
            }
        }
        // Новые запросы в пределах источника.
        while self.asks.len() < self.source.max_requests.max(1) {
            let Some(key) = self.queue.pop() else {
                break;
            };
            let ask = request(TILE_BYTES);
            ask.start(&net::tile_url(&self.source, key));
            self.asks.push((key, ask));
        }
        if !self.asks.is_empty() || !self.queue.is_empty() {
            return false;
        }
        // Уровень готов: тайлы с картинкой предка уходят в мозаику.
        let depth = self.depth;
        let mut rest = Vec::new();
        for key in std::mem::take(&mut self.left) {
            let part = (depth <= key.0).then(|| self.got.get(&tilemosaic::ancestor(key, depth))).flatten().and_then(|p| p.as_deref()).and_then(|p| tilemosaic::crop(p, key, depth));
            match part {
                Some(rgba) => {
                    self.tiles.images.insert((key.1, key.2), rgba);
                }
                None => rest.push(key),
            }
        }
        self.left = rest;
        self.got.clear();
        self.depth += 1;
        if self.left.is_empty() || self.depth > MAX_FILL_DEPTH {
            return true;
        }
        self.queue = self.ancestors();
        self.queue.is_empty()
    }
}
