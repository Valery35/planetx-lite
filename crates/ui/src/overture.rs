// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! 3D-здания Overture Maps: тайлы MVT из одного архива PMTiles в сети,
//! перенос `ui/overture.py` модуля. Архив - `data::pmtiles`, разбор
//! тайла - тот же, что у зданий векторной основы (`data::buildings`).
//!
//! Сначала просится каталог STAC Overture, его поле latest - выпуск.
//! Без ответа каталога берётся [`RELEASE`]. Потом - начало архива
//! с заголовком и корневым каталогом. Тайл просится запросом Range,
//! листовой каталог - тоже, один раз на всех. Тайлов в работе
//! не больше [`IN_FLIGHT`], отказ повторяется через [`RETRY`] секунд.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use data::buildings::TileKey;
use data::pmtiles::{self, Archive, Locate};

/// Каталог STAC Overture, поле latest - последний выпуск.
pub const STAC: &str = "https://stac.overturemaps.org/catalog.json";
/// Архив зданий выпуска.
pub const PMTILES: &str = "https://tiles.overturemaps.org/{release}/buildings.pmtiles";
/// Выпуск без ответа каталога.
pub const RELEASE: &str = "2026-10-21.0";
/// Подпись источника и ссылка.
pub const ATTRIBUTION: (&str, &str) = (
    "Buildings: Overture Maps Foundation, © OpenStreetMap contributors (ODbL), Microsoft, Google Open Buildings, Esri Community Maps (CC BY 4.0)",
    "https://docs.overturemaps.org/attribution/",
);
/// Тайлов в работе сразу, `IN_FLIGHT` модуля.
const IN_FLIGHT: usize = 8;
/// Секунд до повторной просьбы после отказа.
const RETRY: f64 = 30.0;
/// Предел ответа куска архива, байт.
const ANSWER_BYTES: u64 = 16 << 20;

/// Адрес архива по ответу каталога STAC или выпуска по умолчанию.
pub fn release_url(catalog: Option<&serde_json::Value>) -> String {
    let release = catalog.and_then(|c| c.get("latest")).and_then(|v| v.as_str()).filter(|r| !r.is_empty() && !r.contains('/') && !r.contains('?')).unwrap_or(RELEASE);
    PMTILES.replace("{release}", release)
}

fn ask_range(url: &str, start: u64, length: u64, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Box<dyn net::Request> {
    let ask = request(ANSWER_BYTES);
    let range = format!("bytes={}-{}", start, (start + length).saturating_sub(1));
    ask.send(url, &[("Range", range.as_str())], None);
    ask
}

/// Ответ запроса.
type Answer = Result<Vec<u8>, String>;
/// Листовой каталог в загрузке: запрос и тайлы, которые его ждут.
type LeafAsk = (Box<dyn net::Request>, HashSet<TileKey>);

enum Open {
    Catalog(Box<dyn net::Request>),
    Head(String, Box<dyn net::Request>),
    Ready(String, Archive),
    Failed(String),
}

/// Загрузчик тайлов зданий Overture.
pub struct Overture {
    open: Open,
    /// Тайлы в загрузке.
    tiles: HashMap<TileKey, Box<dyn net::Request>>,
    /// Листовые каталоги в загрузке: начало, длина и тайлы, которые их ждут.
    leaves: HashMap<(u64, u64), LeafAsk>,
    failed: HashMap<TileKey, f64>,
}

impl Overture {
    pub fn new(request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Self {
        let ask = request(1 << 20);
        ask.start(STAC);
        Self { open: Open::Catalog(ask), tiles: HashMap::new(), leaves: HashMap::new(), failed: HashMap::new() }
    }

    pub fn busy(&self) -> bool {
        !matches!(self.open, Open::Ready(..) | Open::Failed(_)) || !self.tiles.is_empty() || !self.leaves.is_empty()
    }

    /// Архив не открылся.
    pub fn error(&self) -> Option<&str> {
        match &self.open {
            Open::Failed(e) => Some(e),
            _ => None,
        }
    }

    /// Шаг: открыть архив, принять ответы, попросить тайлы wanted
    /// по порядку. Возвращает пришедшие тайлы - байты MVT или None,
    /// тайла в архиве нет.
    pub fn step(&mut self, wanted: &[TileKey], now: f64, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Vec<(TileKey, Option<Arc<Vec<u8>>>)> {
        self.open_step(request);
        let Open::Ready(url, archive) = &mut self.open else {
            return Vec::new();
        };
        let mut out = Vec::new();
        // Листовые каталоги.
        let leaves: Vec<((u64, u64), Answer)> = self.leaves.iter().filter_map(|(k, (ask, _))| ask.take().map(|a| (*k, a))).collect();
        for (key, answer) in leaves {
            let Some((_, waiting)) = self.leaves.remove(&key) else {
                continue;
            };
            let ok = matches!(answer, Ok(bytes) if archive.add_leaf(key.0, key.1, &bytes).is_ok());
            if !ok {
                for tile in waiting {
                    self.failed.insert(tile, now);
                }
            }
        }
        // Тайлы.
        let tiles: Vec<(TileKey, Answer)> = self.tiles.iter().filter_map(|(k, ask)| ask.take().map(|a| (*k, a))).collect();
        for (key, answer) in tiles {
            self.tiles.remove(&key);
            match answer.map_err(pmtiles::PmtilesError).and_then(|b| archive.tile_bytes(&b)) {
                Ok(bytes) => out.push((key, Some(Arc::new(bytes)))),
                Err(_) => {
                    self.failed.insert(key, now);
                }
            }
        }
        // Новые просьбы: ненужные тайлы бросаются.
        let keep: HashSet<TileKey> = wanted.iter().copied().collect();
        self.tiles.retain(|k, _| keep.contains(k));
        self.failed.retain(|_, t| now - *t < RETRY);
        for key in wanted {
            if self.tiles.len() + self.leaves.len() >= IN_FLIGHT {
                break;
            }
            if self.tiles.contains_key(key) || self.failed.contains_key(key) || self.leaves.values().any(|(_, w)| w.contains(key)) {
                continue;
            }
            match archive.locate(key.0, key.1, key.2) {
                Locate::Missing => out.push((*key, None)),
                Locate::Tile(start, length) => {
                    self.tiles.insert(*key, ask_range(url, start, length, request));
                }
                Locate::Leaf(start, length) => {
                    let entry = self.leaves.entry((start, length)).or_insert_with(|| (ask_range(url, start, length, request), HashSet::new()));
                    entry.1.insert(*key);
                }
            }
        }
        out
    }

    fn open_step(&mut self, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        let open = std::mem::replace(&mut self.open, Open::Failed(String::new()));
        self.open = match open {
            Open::Catalog(ask) => match ask.take() {
                None => Open::Catalog(ask),
                Some(answer) => {
                    let catalog = answer.ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
                    let url = release_url(catalog.as_ref());
                    let head = ask_range(&url, 0, pmtiles::HEAD, request);
                    Open::Head(url, head)
                }
            },
            Open::Head(url, ask) => match ask.take() {
                None => Open::Head(url, ask),
                Some(Ok(bytes)) => match Archive::open(&bytes) {
                    Ok(archive) => Open::Ready(url, archive),
                    Err(e) => Open::Failed(e.to_string()),
                },
                Some(Err(e)) => Open::Failed(e),
            },
            other => other,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_from_catalog() {
        let catalog = serde_json::json!({"latest": "2026-11-18.0"});
        assert_eq!(release_url(Some(&catalog)), "https://tiles.overturemaps.org/2026-11-18.0/buildings.pmtiles");
        // Без ответа и с чужим значением - выпуск по умолчанию.
        assert!(release_url(None).contains(RELEASE));
        assert!(release_url(Some(&serde_json::json!({"latest": "../x"}))).contains(RELEASE));
    }
}
