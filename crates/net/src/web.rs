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

use web_time::Instant;

use crate::{
    Fetched, Loaded, Queue, RETRY, Source, Stats, TileLoader, template_url, tile_url, tilejson_template, valid_key,
};

struct State {
    queue: Queue,
    results: VecDeque<Loaded>,
    in_flight: usize,
    /// Шаблон адреса тайлов из TileJSON, когда он получен.
    template: Option<String>,
    /// Запрос TileJSON идёт.
    tilejson_busy: bool,
    /// Когда запрос TileJSON не удался: следующий - через [`RETRY`].
    tilejson_failed: Option<Instant>,
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
        let state = State {
            queue: Queue::new(),
            results: VecDeque::new(),
            in_flight: 0,
            template: None,
            tilejson_busy: false,
            tilejson_failed: None,
        };
        Self { source, state: Rc::new(RefCell::new(state)), stats: Rc::new(Stats::default()), on_ready }
    }

    fn pump(&self) {
        pump(self.source, &self.state, &self.stats, &self.on_ready);
    }
}

/// Запросить TileJSON источника, если шаблона адреса ещё нет. true -
/// шаблон есть, тайлы можно просить.
fn ensure_template(source: Source, state: &Rc<RefCell<State>>, stats: &Rc<Stats>, on_ready: &Rc<dyn Fn()>) -> bool {
    let Some(address) = source.tilejson else {
        return true;
    };
    {
        let mut s = state.borrow_mut();
        if s.template.is_some() {
            return true;
        }
        let waiting = s.tilejson_failed.is_some_and(|at| Instant::now().duration_since(at) < RETRY);
        if s.tilejson_busy || waiting || s.queue.is_idle() {
            return false;
        }
        s.tilejson_busy = true;
    }
    let (state, stats, on_ready) = (state.clone(), stats.clone(), on_ready.clone());
    wasm_bindgen_futures::spawn_local(async move {
        stats.request_started();
        let template = fetch(address).await.ok().and_then(|b| tilejson_template(&String::from_utf8_lossy(&b)));
        stats.request_finished();
        {
            let mut s = state.borrow_mut();
            s.tilejson_busy = false;
            s.tilejson_failed = template.is_none().then(Instant::now);
            s.template = template;
        }
        on_ready();
        pump(source, &state, &stats, &on_ready);
    });
    false
}

/// Начать запросы, пока есть место.
fn pump(source: Source, state: &Rc<RefCell<State>>, stats: &Rc<Stats>, on_ready: &Rc<dyn Fn()>) {
    if !ensure_template(source, state, stats, on_ready) {
        return;
    }
    loop {
        let (key, url) = {
            let mut s = state.borrow_mut();
            if s.in_flight >= source.max_requests.max(1) {
                return;
            }
            let Some(key) = s.queue.take_next() else {
                return;
            };
            s.in_flight += 1;
            let url = match &s.template {
                Some(t) => template_url(t, key),
                None => tile_url(&source, key),
            };
            (key, url)
        };
        let (state, stats, on_ready) = (state.clone(), stats.clone(), on_ready.clone());
        wasm_bindgen_futures::spawn_local(async move {
            stats.request_started();
            let fetched = match fetch(&url).await {
                Ok(bytes) => Fetched::from_bytes(&source, &bytes),
                Err(None) => Fetched::Missing,
                Err(Some(e)) => Fetched::Failed(e),
            };
            stats.request_finished();
            {
                let mut s = state.borrow_mut();
                s.in_flight = s.in_flight.saturating_sub(1);
                fetched.record(&mut s.queue, key);
                s.results.push_back(fetched.into_loaded(key));
            }
            on_ready();
            pump(source, &state, &stats, &on_ready);
        });
    }
}

/// Байты ответа сервера. Err(None) - ответ 404, тайла нет.
async fn fetch(url: &str) -> Result<Vec<u8>, Option<String>> {
    let window = web_sys::window().ok_or_else(|| Some("нет окна браузера".to_string()))?;
    let response = JsFuture::from(window.fetch_with_str(url)).await.map_err(|e| Some(format!("{e:?}")))?;
    let response: web_sys::Response = response.dyn_into().map_err(|_| Some("ответ не Response".to_string()))?;
    if response.status() == 404 {
        return Err(None);
    }
    if !response.ok() {
        return Err(Some(format!("HTTP {}", response.status())));
    }
    let buffer = response.array_buffer().map_err(|e| Some(format!("{e:?}")))?;
    let buffer = JsFuture::from(buffer).await.map_err(|e| Some(format!("{e:?}")))?;
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
