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

use web_time::{Duration, Instant};

use crate::{
    Fetched, Loaded, Queue, RETRY, Request, Source, Stats, TileLoader, template_url, tile_url, tilejson_template, valid_key,
};

/// Ответ сервера до разбора: байты, Err(None) - тайла нет, Err(Some) -
/// ошибка.
type Answer = (TileKey, Result<Vec<u8>, Option<String>>);

/// Время разбора пришедших тайлов за один вызов `take`, то есть за кадр.
/// Разбор JPEG идёт в главном потоке страницы. Тайлы из кэша браузера
/// приходят без паузы, и разбор подряд останавливал кадры на секунду
/// в туре у Бриенца, 23 октября 2026 года.
/// Загрузчиков три - основа, рельеф, пункты, вместе до 12 мс.
pub const DECODE_BUDGET: Duration = Duration::from_millis(4);

struct State {
    queue: Queue,
    /// Пришедшие и ещё не разобранные ответы.
    answers: VecDeque<Answer>,
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
            answers: VecDeque::new(),
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
            let answer = fetch(&url).await;
            stats.request_finished();
            {
                // Разбор - в take, по бюджету кадра. До разбора тайл
                // в очереди считается начатым.
                let mut s = state.borrow_mut();
                s.in_flight = s.in_flight.saturating_sub(1);
                s.answers.push_back((key, answer));
            }
            on_ready();
            pump(source, &state, &stats, &on_ready);
        });
    }
}

/// Предел запроса страницы, миллисекунды: ответ и тело целиком.
pub const TIMEOUT_MS: u32 = 20_000;
/// Предел запроса с большим ответом, как `LONG_TIMEOUT` программы: ответ
/// Overpass с ЛЭП у вида шёл дольше 20 с, запрос рвался и повторялся по
/// кругу, 25 октября 2026 года.
pub const LONG_TIMEOUT_MS: u32 = 300_000;
/// Ответ больше этого - запрос с долгим пределом, как `LARGE_ANSWER`.
pub const LARGE_ANSWER: u64 = 4 << 20;

/// Байты ответа сервера. Err(None) - ответ 404, тайла нет.
async fn fetch(url: &str) -> Result<Vec<u8>, Option<String>> {
    fetch_with(url, &[], None, TIMEOUT_MS, false).await
}

/// Байты ответа с заголовками и телом: тело есть - POST.
async fn fetch_with(url: &str, headers: &[(String, String)], body: Option<&[u8]>, timeout_ms: u32, error_bodies: bool) -> Result<Vec<u8>, Option<String>> {
    let window = web_sys::window().ok_or_else(|| Some("нет окна браузера".to_string()))?;
    // У fetch браузера своего предела нет: медленный сервер высот держал
    // запрос минутами, загрузка не кончалась, 23 октября 2026 года.
    // Предел - как у программы. Браузер без AbortSignal.timeout ждёт
    // сколько ждёт.
    let init = web_sys::RequestInit::new();
    let has_timeout = js_sys::Reflect::get(&js_sys::global(), &"AbortSignal".into())
        .is_ok_and(|s| !s.is_undefined() && js_sys::Reflect::has(&s, &"timeout".into()).unwrap_or(false));
    if has_timeout {
        init.set_signal(Some(&web_sys::AbortSignal::timeout_with_u32(timeout_ms)));
    }
    if !headers.is_empty() {
        let h = web_sys::Headers::new().map_err(|e| Some(format!("{e:?}")))?;
        for (k, v) in headers {
            h.set(k, v).map_err(|e| Some(format!("{e:?}")))?;
        }
        init.set_headers(&h);
    }
    if let Some(b) = body {
        init.set_method("POST");
        init.set_body(&js_sys::Uint8Array::from(b));
    }
    let response = JsFuture::from(window.fetch_with_str_and_init(url, &init)).await.map_err(|e| Some(format!("{e:?}")))?;
    let response: web_sys::Response = response.dyn_into().map_err(|_| Some("ответ не Response".to_string()))?;
    if !response.ok() && error_bodies {
        let text = match response.text() {
            Ok(p) => JsFuture::from(p).await.ok().and_then(|v| v.as_string()).unwrap_or_default(),
            Err(_) => String::new(),
        };
        return Err(Some(format!("HTTP {}: {text}", response.status())));
    }
    if response.status() == 404 || (response.status() == 403 && crate::forbidden_is_missing(url)) {
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

    /// Разобрать пришедшие ответы, пока не кончился [`DECODE_BUDGET`],
    /// хотя бы один. Остаток - в следующий кадр.
    fn take(&self) -> Vec<Loaded> {
        let started = Instant::now();
        let mut out = Vec::new();
        loop {
            let Some((key, answer)) = self.state.borrow_mut().answers.pop_front() else {
                break;
            };
            let fetched = match answer {
                Ok(bytes) => Fetched::from_bytes(&self.source, &bytes),
                Err(None) => Fetched::Missing,
                Err(Some(e)) => Fetched::Failed(e),
            };
            fetched.record(&mut self.state.borrow_mut().queue, key);
            out.push(fetched.into_loaded(key));
            if started.elapsed() >= DECODE_BUDGET {
                break;
            }
        }
        if !self.state.borrow().answers.is_empty() {
            (self.on_ready)();
        }
        out
    }

    fn busy(&self) -> bool {
        !self.state.borrow().queue.is_idle()
    }

    fn stats(&self) -> &Stats {
        &self.stats
    }
}

/// Пришедшие ответы разовых запросов.
type Answers = Rc<RefCell<VecDeque<Result<Vec<u8>, String>>>>;

/// Разовые запросы страницы через fetch.
pub struct Requests {
    results: Answers,
    /// Предел запроса, миллисекунды.
    timeout_ms: u32,
    stats: Rc<Stats>,
    on_ready: Rc<dyn Fn()>,
    /// Ответ с кодом ошибки HTTP - ошибка с телом ответа.
    error_bodies: bool,
}

impl Requests {
    /// on_ready зовётся, когда пришёл ответ.
    pub fn new(on_ready: Rc<dyn Fn()>) -> Self {
        Self { results: Rc::new(RefCell::new(VecDeque::new())), timeout_ms: TIMEOUT_MS, stats: Rc::new(Stats::default()), on_ready, error_bodies: false }
    }

    /// Запросы с пределом ответа limit байт: большой ответ ждётся дольше.
    pub fn with_limit(on_ready: Rc<dyn Fn()>, limit: u64) -> Self {
        let timeout_ms = if limit > LARGE_ANSWER { LONG_TIMEOUT_MS } else { TIMEOUT_MS };
        Self { timeout_ms, ..Self::new(on_ready) }
    }
}

impl Request for Requests {
    fn start(&self, url: &str) {
        self.send(url, &[], None);
    }

    fn send(&self, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>) {
        let (results, stats, on_ready, url, timeout_ms, error_bodies) =
            (self.results.clone(), self.stats.clone(), self.on_ready.clone(), url.to_string(), self.timeout_ms, self.error_bodies);
        let headers: Vec<(String, String)> = headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let body = body.map(<[u8]>::to_vec);
        wasm_bindgen_futures::spawn_local(async move {
            stats.request_started();
            let result = fetch_with(&url, &headers, body.as_deref(), timeout_ms, error_bodies).await.map_err(|e| e.unwrap_or_else(|| "HTTP 404".to_string()));
            stats.request_finished();
            results.borrow_mut().push_back(result);
            on_ready();
        });
    }

    fn keep_error_bodies(&mut self) {
        self.error_bodies = true;
    }

    fn take(&self) -> Option<Result<Vec<u8>, String>> {
        self.results.borrow_mut().pop_front()
    }

    fn stats(&self) -> &Stats {
        &self.stats
    }
}