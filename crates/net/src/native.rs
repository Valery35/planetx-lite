// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Загрузчик тайлов программы: рабочие потоки, дисковый кэш, HTTP.
//!
//! Потоков [`WORKERS`], а разрешений на запрос к серверу столько,
//! сколько допускает источник. Тайл с диска не ждёт разрешения, поэтому
//! медленный сервер не задерживает тайлы из кэша. Картинка декодируется
//! в рабочем потоке, главному потоку остаётся загрузить её в видеокарту.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use globe_core::lod::TileKey;

use crate::{
    FRESH, Fetched, Loaded, Queue, Request, Source, Stats, TileLoader, cache_path, template_url, tile_url, tilejson_template,
    user_agent, valid_key,
};

/// Рабочих потоков загрузчика.
pub const WORKERS: usize = 4;
/// Предел ожидания ответа сервера.
pub const TIMEOUT: Duration = Duration::from_secs(20);

/// Папка кэша тайлов по умолчанию: на Windows
/// `%LOCALAPPDATA%\PlanetX-Lite\tiles`, на других системах - стандартная
/// папка кэша пользователя.
pub fn default_cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("PlanetX-Lite").join("tiles"))
}

struct Shared {
    queue: Mutex<Queue>,
    work: Condvar,
    permits: Mutex<usize>,
    permit_freed: Condvar,
    stop: AtomicBool,
    stats: Stats,
    /// Шаблон адреса тайлов из TileJSON, когда он получен.
    template: Mutex<Option<String>>,
}

/// Замок, который не роняет программу, если другой поток упал с ним.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Загрузчик тайлов одного источника.
pub struct Loader {
    source: Source,
    shared: Arc<Shared>,
    results: Receiver<Loaded>,
    threads: Vec<JoinHandle<()>>,
}

impl Loader {
    /// Загрузчик источника source. cache_dir - корень дискового кэша
    /// или None. on_ready зовётся из рабочего потока после каждого тайла,
    /// например, чтобы попросить новый кадр.
    pub fn new(source: Source, cache_dir: Option<PathBuf>, on_ready: Arc<dyn Fn() + Send + Sync>) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::new()),
            work: Condvar::new(),
            permits: Mutex::new(source.max_requests.max(1)),
            permit_freed: Condvar::new(),
            stop: AtomicBool::new(false),
            stats: Stats::default(),
            template: Mutex::new(None),
        });
        let (sender, results) = channel();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .user_agent(user_agent())
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(true)
            .build()
            .into();
        let threads = (0..WORKERS)
            .filter_map(|i| {
                let worker = Worker {
                    source,
                    cache_dir: cache_dir.clone(),
                    shared: shared.clone(),
                    sender: sender.clone(),
                    agent: agent.clone(),
                    on_ready: on_ready.clone(),
                };
                std::thread::Builder::new().name(format!("tiles-{}-{i}", source.name)).spawn(move || worker.run()).ok()
            })
            .collect();
        Self { source, shared, results, threads }
    }

}

impl TileLoader for Loader {
    /// Недопустимые номера тайлов отбрасываются.
    fn want(&self, wanted: &[(TileKey, f64)]) {
        let wanted: Vec<(TileKey, f64)> =
            wanted.iter().copied().filter(|(k, _)| valid_key(&self.source, *k)).collect();
        lock(&self.shared.queue).set_wanted(&wanted);
        self.shared.work.notify_all();
    }

    fn take(&self) -> Vec<Loaded> {
        self.results.try_iter().collect()
    }

    fn busy(&self) -> bool {
        !lock(&self.shared.queue).is_idle()
    }

    fn stats(&self) -> &Stats {
        &self.shared.stats
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.work.notify_all();
        self.shared.permit_freed.notify_all();
        // Потоки не ждём: запрос к серверу может висеть до TIMEOUT.
        self.threads.clear();
    }
}

struct Worker {
    source: Source,
    cache_dir: Option<PathBuf>,
    shared: Arc<Shared>,
    sender: Sender<Loaded>,
    agent: ureq::Agent,
    on_ready: Arc<dyn Fn() + Send + Sync>,
}

impl Worker {
    fn run(self) {
        while let Some(key) = self.next_job() {
            let fetched = self.load(key);
            fetched.record(&mut lock(&self.shared.queue), key);
            if self.sender.send(fetched.into_loaded(key)).is_err() {
                return;
            }
            (self.on_ready)();
        }
    }

    fn stopped(&self) -> bool {
        self.shared.stop.load(Ordering::SeqCst)
    }

    fn next_job(&self) -> Option<TileKey> {
        let mut queue = lock(&self.shared.queue);
        loop {
            if self.stopped() {
                return None;
            }
            if let Some(key) = queue.take_next() {
                return Some(key);
            }
            // Ожидание с пределом: отложенные после неудачи тайлы
            // возвращаются в очередь сами, без нового вызова want.
            queue = self
                .shared
                .work
                .wait_timeout(queue, Duration::from_secs(1))
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
    }

    fn load(&self, key: TileKey) -> Fetched {
        let path = self.cache_dir.as_ref().map(|root| cache_path(root, &self.source, key));
        let cached = path.as_deref().and_then(read_cached);
        if let Some((bytes, fresh)) = &cached
            && *fresh
        {
            let fetched = Fetched::from_bytes(&self.source, bytes);
            if fetched.settled() {
                self.shared.stats.disk_hits.fetch_add(1, Ordering::Relaxed);
                return fetched;
            }
        }
        match self.fetch(key) {
            Ok(bytes) => {
                let fetched = Fetched::from_bytes(&self.source, &bytes);
                if fetched.settled()
                    && let Some(path) = &path
                {
                    // Заглушка тоже кэшируется: сервер не спрашивается
                    // о ней снова. Ошибка записи кэша не мешает показу.
                    let _ = write_atomic(path, &bytes);
                }
                fetched
            }
            Err(None) => Fetched::Missing,
            // Сервер недоступен - годится и устаревший тайл.
            Err(Some(e)) => match cached.map(|(bytes, _)| Fetched::from_bytes(&self.source, &bytes)) {
                Some(f) if f.settled() => f,
                _ => Fetched::Failed(e),
            },
        }
    }

    /// Байты ответа сервера. Err(None) - ответ 404, тайла нет.
    fn fetch(&self, key: TileKey) -> Result<Vec<u8>, Option<String>> {
        let url = match self.source.tilejson {
            Some(address) => template_url(&self.template(address).map_err(Some)?, key),
            None => tile_url(&self.source, key),
        };
        self.request(&url, 4 << 20)
    }

    /// Шаблон адреса тайлов. TileJSON спрашивается один раз, остальные
    /// потоки ждут его под замком.
    fn template(&self, address: &str) -> Result<String, String> {
        let mut slot = lock(&self.shared.template);
        if let Some(t) = slot.as_ref() {
            return Ok(t.clone());
        }
        let bytes = self.request(address, 1 << 20).map_err(|e| e.unwrap_or_else(|| "TileJSON: 404".to_string()))?;
        let text = String::from_utf8_lossy(&bytes);
        let template = tilejson_template(&text).ok_or_else(|| "TileJSON без адреса тайлов".to_string())?;
        *slot = Some(template.clone());
        Ok(template)
    }

    /// Запрос к серверу с разрешением источника.
    fn request(&self, url: &str, limit: u64) -> Result<Vec<u8>, Option<String>> {
        self.acquire().map_err(Some)?;
        let stats = &self.shared.stats;
        stats.request_started();
        let result = self
            .agent
            .get(url)
            .call()
            .and_then(|mut r| r.body_mut().with_config().limit(limit).read_to_vec())
            .map_err(|e| match e {
                ureq::Error::StatusCode(404) => None,
                e => Some(e.to_string()),
            });
        stats.request_finished();
        self.release();
        result
    }

    fn acquire(&self) -> Result<(), String> {
        let mut free = lock(&self.shared.permits);
        while *free == 0 {
            if self.stopped() {
                return Err("загрузчик остановлен".into());
            }
            free = self.shared.permit_freed.wait(free).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *free -= 1;
        Ok(())
    }

    fn release(&self) {
        *lock(&self.shared.permits) += 1;
        self.shared.permit_freed.notify_one();
    }
}

/// Байты тайла из кэша и признак свежести.
fn read_cached(path: &Path) -> Option<(Vec<u8>, bool)> {
    let bytes = fs::read(path).ok()?;
    let age = fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok());
    Some((bytes, age.is_some_and(|a| a < FRESH)))
}

/// Запись через временный файл: оборванная запись не оставляет битый тайл.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().ok_or_else(|| io::Error::other("нет папки"))?;
    fs::create_dir_all(dir)?;
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

/// Разовые запросы программы: каждый в своём потоке, с заголовком
/// `User-Agent` программы.
pub struct Requests {
    agent: ureq::Agent,
    sender: Sender<Result<Vec<u8>, String>>,
    results: Receiver<Result<Vec<u8>, String>>,
    stats: Arc<Stats>,
    on_ready: Arc<dyn Fn() + Send + Sync>,
}

impl Requests {
    /// on_ready зовётся из потока запроса, когда пришёл ответ.
    pub fn new(on_ready: Arc<dyn Fn() + Send + Sync>) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .user_agent(user_agent())
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(true)
            .build()
            .into();
        let (sender, results) = channel();
        Self { agent, sender, results, stats: Arc::new(Stats::default()), on_ready }
    }
}

impl Request for Requests {
    fn start(&self, url: &str) {
        let (agent, sender, stats, on_ready, url) =
            (self.agent.clone(), self.sender.clone(), self.stats.clone(), self.on_ready.clone(), url.to_string());
        let spawned = std::thread::Builder::new().name("request".into()).spawn(move || {
            stats.request_started();
            let result = agent
                .get(&url)
                .call()
                .and_then(|mut r| r.body_mut().with_config().limit(1 << 20).read_to_vec())
                .map_err(|e| e.to_string());
            stats.request_finished();
            let _ = sender.send(result);
            on_ready();
        });
        if let Err(e) = spawned {
            let _ = self.sender.send(Err(e.to_string()));
        }
    }

    fn take(&self) -> Option<Result<Vec<u8>, String>> {
        self.results.try_recv().ok()
    }

    fn stats(&self) -> &Stats {
        &self.stats
    }
}