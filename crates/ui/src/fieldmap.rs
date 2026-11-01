// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Строки «Границы полей» каталога, перенос `ui/fieldmap.py` модуля. Ядро -
//! `data::fieldmap`, чтение архива частями файла - [`crate::overture`].
//!
//! Тайл уровня 10 весит до 1.8 МБ и несёт тысячи полей, поэтому разбор
//! и картинка идут в рабочем потоке, а у страницы - по одной картинке
//! за кадр. Разобранные тайлы помнятся для опроса щелчком, без сети.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use data::fieldmap::{self, Field};
use globe_core::lod::TileKey;

use crate::i18n::{Lang, tr, trf};
use crate::identify::Feature;
use crate::overture::Overture;

/// Разобранных тайлов для опроса щелчком, `TILES_KEPT` модуля.
const TILES_KEPT: usize = 64;
/// Картинок в работе сразу, `WORKERS` модуля.
const WORKERS: usize = 4;

/// Работа над картинкой key: тайл архива source готов или ещё в байтах.
struct Job {
    key: TileKey,
    source: TileKey,
    title: &'static str,
    fields: Option<Arc<Vec<Field>>>,
    bytes: Option<Arc<Vec<u8>>>,
}

/// Готовая картинка и тайл архива, если он разобран этой работой.
struct Done {
    key: TileKey,
    source: TileKey,
    fields: Option<Arc<Vec<Field>>>,
    rgba: Option<Vec<u8>>,
}

impl Job {
    fn run(self) -> Done {
        let fields = match (&self.fields, &self.bytes) {
            (Some(f), _) => f.clone(),
            (None, Some(b)) => Arc::new(fieldmap::decode(b, self.title).unwrap_or_default()),
            (None, None) => Arc::new(Vec::new()),
        };
        let rgba = fieldmap::paint(&fields, self.source, self.key);
        Done { key: self.key, source: self.source, fields: self.fields.is_none().then_some(fields), rgba }
    }
}

/// Загрузчик картинок полей одного года.
pub struct FieldTiles {
    pub year: i32,
    reader: Overture,
    /// Пришедшие байты тайлов архива: ждут разбора.
    raw: HashMap<TileKey, Arc<Vec<u8>>>,
    /// Разобранные тайлы архива, старые уходят первыми.
    found: HashMap<TileKey, Arc<Vec<Field>>>,
    order: VecDeque<TileKey>,
    /// Картинки и тайлы архива в работе.
    running: HashSet<TileKey>,
    parsing: HashSet<TileKey>,
    /// Кадр ждёт картинки: без новых кадров следующие тайлы не попросятся.
    waiting: bool,
    send: Sender<Done>,
    take: Receiver<Done>,
    /// Работы страницы: потоков у неё нет.
    #[cfg(target_arch = "wasm32")]
    queue: VecDeque<Job>,
}

impl FieldTiles {
    pub fn new(year: i32, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Self {
        let (send, take) = channel();
        Self {
            year,
            reader: Overture::archive(fieldmap::url(year), request),
            raw: HashMap::new(),
            found: HashMap::new(),
            order: VecDeque::new(),
            running: HashSet::new(),
            parsing: HashSet::new(),
            waiting: false,
            send,
            take,
            #[cfg(target_arch = "wasm32")]
            queue: VecDeque::new(),
        }
    }

    pub fn busy(&self) -> bool {
        self.waiting || self.reader.busy() || !self.running.is_empty()
    }

    /// Шаг кадра: wanted - картинки, нужные кадру. Возвращает готовые
    /// картинки RGBA 256 × 256 или None - полей в тайле нет.
    pub fn step(&mut self, wanted: &[TileKey], now: f64, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Vec<(TileKey, Option<Vec<u8>>)> {
        self.waiting = !wanted.is_empty();
        #[cfg(target_arch = "wasm32")]
        if let Some(job) = self.queue.pop_front() {
            let _ = self.send.send(job.run());
        }
        let mut out = Vec::new();
        while let Ok(done) = self.take.try_recv() {
            self.running.remove(&done.key);
            if let Some(fields) = done.fields {
                self.parsing.remove(&done.source);
                self.keep(done.source, fields);
            }
            out.push((done.key, done.rgba));
        }
        let mut sources: Vec<TileKey> = Vec::new();
        for key in wanted {
            let source = fieldmap::source_of(*key);
            if !self.found.contains_key(&source) && !self.raw.contains_key(&source) && !self.parsing.contains(&source) && !sources.contains(&source) {
                sources.push(source);
            }
        }
        for (key, bytes) in self.reader.step(&sources, now, request) {
            match bytes {
                Some(b) => {
                    self.raw.insert(key, b);
                }
                // Тайла в архиве нет: полей здесь нет.
                None => self.keep(key, Arc::new(Vec::new())),
            }
        }
        for key in wanted {
            if self.running.len() >= WORKERS {
                break;
            }
            if self.running.contains(key) {
                continue;
            }
            let source = fieldmap::source_of(*key);
            let title = fieldmap::layer(self.year);
            if let Some(fields) = self.found.get(&source) {
                self.start(Job { key: *key, source, title, fields: Some(fields.clone()), bytes: None });
            } else if let Some(bytes) = self.raw.remove(&source) {
                self.parsing.insert(source);
                self.start(Job { key: *key, source, title, fields: None, bytes: Some(bytes) });
            }
        }
        out
    }

    fn keep(&mut self, key: TileKey, fields: Arc<Vec<Field>>) {
        self.found.insert(key, fields);
        self.order.retain(|k| *k != key);
        self.order.push_back(key);
        while self.order.len() > TILES_KEPT {
            if let Some(old) = self.order.pop_front() {
                self.found.remove(&old);
            }
        }
    }

    fn start(&mut self, job: Job) {
        self.running.insert(job.key);
        #[cfg(target_arch = "wasm32")]
        self.queue.push_back(job);
        #[cfg(not(target_arch = "wasm32"))]
        {
            let send = self.send.clone();
            std::thread::spawn(move || {
                let _ = send.send(job.run());
            });
        }
    }

    /// Оценка поля в точке по разобранным тайлам, `under` модуля.
    pub fn under(&self, lat: f64, lon: f64) -> Option<f64> {
        for level in (fieldmap::MIN_LEVEL..=fieldmap::MAX_LEVEL).rev() {
            let ((x, y), (dx, dy)) = fieldmap::tile_point(lat, lon, level);
            if let Some(found) = self.found.get(&(level, x, y)) {
                return fieldmap::score_at(found, dx, dy);
            }
        }
        None
    }

    /// Группа окна «Объекты», `_identify_fieldmap` модуля.
    pub fn group(&self, lang: Lang, lat: f64, lon: f64) -> Option<(String, Vec<Feature>)> {
        let score = self.under(lat, lon)?;
        let values = vec![
            (tr(lang, "Model score").to_string(), trf(lang, "{score} of 100", &[("score", &format!("{:.0}", score))])),
            (tr(lang, "What this is").to_string(), tr(lang, "A model estimate from Sentinel-2 imagery, not a cadastral parcel.").to_string()),
        ];
        let title = trf(lang, "Field of {year}", &[("year", &self.year.to_string())]);
        Some((tr(lang, "Field boundaries").to_string(), vec![(title, values)]))
    }
}

#[cfg(test)]
mod tests {
    use super::FieldTiles;
    use std::cell::RefCell;

    /// Запрос с готовым ответом: архив на первый запрос, отказ на тайл.
    struct Stub(RefCell<Option<Result<Vec<u8>, String>>>, net::Stats);

    impl net::Request for Stub {
        fn start(&self, _: &str) {}
        fn send(&self, _: &str, _: &[(&str, &str)], _: Option<&[u8]>) {}
        fn take(&self) -> Option<Result<Vec<u8>, String>> {
            self.0.borrow_mut().take()
        }
        fn stats(&self) -> &net::Stats {
            &self.1
        }
    }

    /// Заголовок PMTiles без сжатия с одним тайлом уровня 0.
    fn head() -> Vec<u8> {
        let directory = [1u8, 0, 1, 1, 1];
        let mut out = b"PMTiles\x03".to_vec();
        for value in [127u64, directory.len() as u64, 0, 0, 0, 0, 200, 1, 1, 1, 1] {
            out.extend(value.to_le_bytes());
        }
        out.extend([1, 1, 1, 1, 0, 14]);
        out.resize(127, 0);
        out.extend(directory);
        out
    }

    #[test]
    fn waiting_tile_keeps_frames_going() {
        let answers = RefCell::new(vec![Ok(head())]);
        let request = |_: u64| -> Box<dyn net::Request> { Box::new(Stub(RefCell::new(Some(answers.borrow_mut().pop().unwrap_or(Err("отказ".to_string())))), net::Stats::default())) };
        let mut tiles = FieldTiles::new(2025, &request);
        // Тайл кадра в архиве есть, ответ на него - отказ, повтор через 30 с.
        for now in 0..3 {
            tiles.step(&[(0, 0, 0)], f64::from(now), &request);
        }
        // Запросов в работе нет, а кадр ещё ждёт картинку: кадры идут.
        assert!(tiles.busy());
        tiles.step(&[], 3.0, &request);
        assert!(!tiles.busy());
    }
}
