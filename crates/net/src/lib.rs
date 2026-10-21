// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Тайлы из сети и с диска, слой L2.
//!
//! Очередь [`Queue`] держит тайлы, которые просит кадр, в порядке
//! приоритета из `globe_core::lod`. Тайл, который кадр больше не просит,
//! из очереди снимается. Начатый запрос не обрывается, его тайл ляжет
//! в кэш.
//!
//! Правила OSM действуют и для Lite, как в модуле: заголовок
//! `User-Agent` программы с адресом хранилища, к `tile.openstreetmap.org`
//! не больше двух запросов сразу, тайл кэшируется на диске и свежим
//! считается 7 дней, подпись источника стоит на экране.
//!
//! Загрузчик в программе - [`native::Loader`], рабочие потоки. Главный
//! поток он не блокирует. Страница получит свой загрузчик на fetch.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use globe_core::lod::TileKey;
use web_time::{Duration, Instant};

#[cfg(not(target_arch = "wasm32"))]
pub mod native;
pub mod placeholder;
#[cfg(target_arch = "wasm32")]
pub mod web;

/// Пришедший тайл.
#[derive(Debug)]
pub struct Loaded {
    pub key: TileKey,
    pub image: Result<Image, String>,
    /// Тайла у источника нет: ответ 404 или заглушка вместо снимка.
    /// Вместо него рисуется увеличенный предок, повторно он не просится.
    pub missing: bool,
}

/// Счётчики загрузчика.
#[derive(Debug, Default)]
pub struct Stats {
    /// Запросов к серверу сейчас.
    pub requests_now: AtomicUsize,
    /// Наибольшее количество одновременных запросов за всё время.
    pub requests_peak: AtomicUsize,
    /// Запросов к серверу всего.
    pub requests_total: AtomicUsize,
    /// Тайлов из дискового кэша всего.
    pub disk_hits: AtomicUsize,
}

impl Stats {
    /// Запрос к серверу начался.
    pub fn request_started(&self) {
        let now = self.requests_now.fetch_add(1, Ordering::SeqCst) + 1;
        self.requests_peak.fetch_max(now, Ordering::SeqCst);
        self.requests_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Запрос к серверу закончился.
    pub fn request_finished(&self) {
        self.requests_now.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Загрузчик тайлов - общий для программы и страницы.
pub trait TileLoader {
    /// Тайлы, которые просит кадр, с приоритетом.
    fn want(&self, wanted: &[(TileKey, f64)]);
    /// Пришедшие с прошлого вызова тайлы.
    fn take(&self) -> Vec<Loaded>;
    /// Загрузка идёт или ждёт.
    fn busy(&self) -> bool;
    fn stats(&self) -> &Stats;
}

/// Источник тайлов.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    /// Имя папки кэша.
    pub name: &'static str,
    /// Адрес с местами {z}, {x}, {y}.
    pub url: &'static str,
    /// Наибольшее количество одновременных запросов к серверу.
    pub max_requests: usize,
    /// Уровень самых мелких тайлов источника.
    pub max_level: u32,
    /// Расширение файла в кэше.
    pub ext: &'static str,
    /// Источник отвечает заглушкой там, где снимка нет.
    pub placeholders: bool,
}

/// Подложка OpenStreetMap.
pub const OSM: Source = Source {
    name: "osm",
    url: "https://tile.openstreetmap.org/{z}/{x}/{y}.png",
    max_requests: 2,
    max_level: 19,
    ext: "png",
    placeholders: false,
};

/// Космоснимки Esri World Imagery, как в модуле. Порядок y и x в адресе
/// обратный.
pub const ESRI: Source = Source {
    name: "esri",
    url: "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}",
    max_requests: 4,
    max_level: 19,
    ext: "jpg",
    placeholders: true,
};

/// Высоты рельефа Mapzen Terrain Tiles в записи Terrarium, как в модуле.
pub const TERRARIUM: Source = Source {
    name: "terrarium",
    url: "https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png",
    max_requests: 4,
    max_level: 15,
    ext: "png",
    placeholders: false,
};

/// Сколько тайл в кэше считается свежим.
pub const FRESH: Duration = Duration::from_secs(7 * 24 * 3600);
/// Через сколько повторить запрос тайла, который не пришёл.
pub const RETRY: Duration = Duration::from_secs(10);
/// Сторона тайла в пикселях.
pub const TILE_SIZE: u32 = 256;

/// Заголовок `User-Agent` - название, версия программы и адрес
/// хранилища. Имени и почты автора нет, решение автора от 21 октября
/// 2026 года.
pub fn user_agent() -> String {
    format!("PlanetX-Lite/{} (+https://github.com/Valery35/planetx-lite)", env!("CARGO_PKG_VERSION"))
}

/// Адрес тайла key у источника.
pub fn tile_url(source: &Source, key: TileKey) -> String {
    let (z, x, y) = key;
    source.url.replace("{z}", &z.to_string()).replace("{x}", &x.to_string()).replace("{y}", &y.to_string())
}

/// Файл тайла key в кэше с корнем root.
pub fn cache_path(root: &Path, source: &Source, key: TileKey) -> PathBuf {
    let (z, x, y) = key;
    root.join(source.name).join(z.to_string()).join(x.to_string()).join(format!("{y}.{}", source.ext))
}

/// Номер тайла допустим: уровень источника и номера в пределах уровня.
/// Тайл вне сетки не запрашивается.
pub fn valid_key(source: &Source, key: TileKey) -> bool {
    let (z, x, y) = key;
    z <= source.max_level && z < 32 && x < (1u32 << z) && y < (1u32 << z)
}

/// Картинка тайла, RGBA по 8 бит, строки сверху вниз.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Раскодировать картинку тайла, PNG или JPEG. Входные байты считаются
/// враждебными: ошибка декодера и размер не 256 × 256 дают ошибку,
/// а не сбой.
pub fn decode_tile(bytes: &[u8]) -> Result<Image, String> {
    let format = image::guess_format(bytes).map_err(|e| e.to_string())?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg) {
        return Err(format!("формат {format:?} не PNG и не JPEG"));
    }
    let img = image::load_from_memory_with_format(bytes, format).map_err(|e| e.to_string())?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    if (width, height) != (TILE_SIZE, TILE_SIZE) {
        return Err(format!("тайл {width} × {height}, ждали {TILE_SIZE} × {TILE_SIZE}"));
    }
    Ok(Image { width, height, rgba: rgba.into_raw() })
}

/// Итог загрузки тайла: картинка, «тайла нет» или ошибка.
pub enum Fetched {
    Image(Image),
    Missing,
    Failed(String),
}

impl Fetched {
    /// Раскодировать ответ сервера. Заглушка у источника с заглушками -
    /// «тайла нет».
    pub fn from_bytes(source: &Source, bytes: &[u8]) -> Self {
        match decode_tile(bytes) {
            Ok(img) if source.placeholders && placeholder::is_placeholder(&img.rgba) => Fetched::Missing,
            Ok(img) => Fetched::Image(img),
            Err(e) => Fetched::Failed(e),
        }
    }

    /// Пришедший тайл для главного потока.
    pub fn into_loaded(self, key: TileKey) -> Loaded {
        match self {
            Fetched::Image(img) => Loaded { key, image: Ok(img), missing: false },
            Fetched::Missing => Loaded { key, image: Err("тайла нет у источника".into()), missing: true },
            Fetched::Failed(e) => Loaded { key, image: Err(e), missing: false },
        }
    }

    /// Загрузка закончена: картинка или «тайла нет», повтор не нужен.
    pub fn settled(&self) -> bool {
        !matches!(self, Fetched::Failed(_))
    }

    /// Отметить итог в очереди.
    pub fn record(&self, queue: &mut Queue, key: TileKey) {
        match self {
            Fetched::Missing => queue.done_missing(key),
            other => queue.done(key, other.settled()),
        }
    }
}

/// Очередь тайлов с приоритетом.
#[derive(Debug, Default)]
pub struct Queue {
    /// Тайлы, которые просит кадр, по убыванию приоритета.
    wanted: Vec<(TileKey, f64)>,
    in_flight: HashSet<TileKey>,
    failed: HashMap<TileKey, Instant>,
    /// Тайлы, которых у источника нет. Они не просятся снова.
    missing: HashSet<TileKey>,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Заменить просьбу кадра. Тайлы, которых в ней нет, снимаются
    /// с очереди. Начатые запросы остаются.
    pub fn set_wanted(&mut self, wanted: &[(TileKey, f64)]) {
        let mut seen = HashSet::new();
        self.wanted = wanted
            .iter()
            .copied()
            .filter(|(k, p)| !p.is_nan() && !self.missing.contains(k) && seen.insert(*k))
            .collect();
        // Сортировка устойчивая: при равном приоритете раньше тот, кого
        // кадр попросил раньше, - предок.
        self.wanted.sort_by(|a, b| b.1.total_cmp(&a.1));
    }

    /// Следующий тайл для загрузки или None. Тайл помечается начатым.
    pub fn take_next(&mut self) -> Option<TileKey> {
        let now = Instant::now();
        self.failed.retain(|_, at| now.duration_since(*at) < RETRY);
        let pos = self.wanted.iter().position(|(k, _)| !self.in_flight.contains(k) && !self.failed.contains_key(k))?;
        let (key, _) = self.wanted.remove(pos);
        self.in_flight.insert(key);
        Some(key)
    }

    /// Загрузка тайла закончилась. Неудача откладывает повтор на
    /// [`RETRY`].
    pub fn done(&mut self, key: TileKey, ok: bool) {
        self.in_flight.remove(&key);
        if !ok {
            self.failed.insert(key, Instant::now());
        }
    }

    /// Тайла нет у источника: загрузка закончена, снова он не просится.
    pub fn done_missing(&mut self, key: TileKey) {
        self.in_flight.remove(&key);
        self.missing.insert(key);
    }

    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    /// Ждущих тайлов, без начатых и отложенных после неудачи.
    pub fn pending(&self) -> usize {
        self.wanted.iter().filter(|(k, _)| !self.in_flight.contains(k) && !self.failed.contains_key(k)).count()
    }

    pub fn is_idle(&self) -> bool {
        self.in_flight.is_empty() && self.pending() == 0
    }
}
