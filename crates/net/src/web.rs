// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Загрузчик тайлов страницы: fetch браузера.
//!
//! Страница однопоточная, запросы идут асинхронно, главный поток
//! их не ждёт. К серверу одновременно не больше `Source::max_requests`
//! запросов. Кэш - HTTP-кэш браузера по заголовкам сервера.
//!
//! Заголовок `User-Agent` браузер странице задать не даёт. Сервер
//! опознаёт страницу по заголовку `Referer` - адресу сайта.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use globe_core::lod::TileKey;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use crate::{Loaded, Queue, Source, Stats, TileLoader, decode_png, tile_url, valid_key};

struct State {
    queue: Queue,
    results: VecDeque<Loaded>,
    in_flight: usize,
}

/// Загрузчик тайлов одного источника для страницы.
pub struct Loader {
    source: Source,
    state: Rc<RefCell<State>>,
    stats: Rc<Stats>,
    on_ready: Rc<dyn Fn()>,
}

impl Loader {
    /// on_ready зовётся после каждого пришедшего тайла, например, чтобы
    /// попросить новый кадр.
    pub fn new(source: Source, on_ready: Rc<dyn Fn()>) -> Self {
        let state = State { queue: Queue::new(), results: VecDeque::new(), in_flight: 0 };
        Self { source, state: Rc::new(RefCell::new(state)), stats: Rc::new(Stats::default()), on_ready }
    }

    fn pump(&self) {
        pump(self.source, &self.state, &self.stats, &self.on_ready);
    }
}

/// Начать запросы, пока есть место.
fn pump(source: Source, state: &Rc<RefCell<State>>, stats: &Rc<Stats>, on_ready: &Rc<dyn Fn()>) {
    loop {
        let key = {
            let mut s = state.borrow_mut();
            if s.in_flight >= source.max_requests.max(1) {
                return;
            }
            let Some(key) = s.queue.take_next() else {
                return;
            };
            s.in_flight += 1;
            key
        };
        let (state, stats, on_ready) = (state.clone(), stats.clone(), on_ready.clone());
        wasm_bindgen_futures::spawn_local(async move {
            stats.request_started();
            let image = fetch(&tile_url(&source, key)).await.and_then(|bytes| decode_png(&bytes));
            stats.request_finished();
            {
                let mut s = state.borrow_mut();
                s.in_flight = s.in_flight.saturating_sub(1);
                s.queue.done(key, image.is_ok());
                s.results.push_back(Loaded { key, image });
            }
            on_ready();
            pump(source, &state, &stats, &on_ready);
        });
    }
}

/// Байты ответа сервера.
async fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let window = web_sys::window().ok_or("нет окна браузера")?;
    let response = JsFuture::from(window.fetch_with_str(url)).await.map_err(|e| format!("{e:?}"))?;
    let response: web_sys::Response = response.dyn_into().map_err(|_| "ответ не Response".to_string())?;
    if !response.ok() {
        return Err(format!("HTTP {}", response.status()));
    }
    let buffer = response.array_buffer().map_err(|e| format!("{e:?}"))?;
    let buffer = JsFuture::from(buffer).await.map_err(|e| format!("{e:?}"))?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

impl TileLoader for Loader {
    /// Недопустимые номера тайлов отбрасываются.
    fn want(&self, wanted: &[(TileKey, f64)]) {
        let wanted: Vec<(TileKey, f64)> =
            wanted.iter().copied().filter(|(k, _)| valid_key(&self.source, *k)).collect();
        self.state.borrow_mut().queue.set_wanted(&wanted);
        self.pump();
    }

    fn take(&self) -> Vec<Loaded> {
        self.state.borrow_mut().results.drain(..).collect()
    }

    fn busy(&self) -> bool {
        !self.state.borrow().queue.is_idle()
    }

    fn stats(&self) -> &Stats {
        &self.stats
    }
}
