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

/// Содержимое тайла: картинка или байты векторного тайла.
#[derive(Debug)]
pub enum Content {
    Image(Image),
    Bytes(Vec<u8>),
}

/// Пришедший тайл.
#[derive(Debug)]
pub struct Loaded {
    pub key: TileKey,
    pub content: Result<Content, String>,
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

/// Разовый запрос, например к геокодеру. Ответ приходит позже, главный
/// поток его не ждёт.
pub trait Request {
    /// Начать запрос. Прежний запрос, если он идёт, продолжается, его
    /// ответ тоже придёт.
    fn start(&self, url: &str);
    /// Начать запрос со своими заголовками и телом: тело есть - POST,
    /// нет - GET. Ответ приходит так же, как у [`Request::start`].
    /// Без своей реализации заголовки и тело не уходят.
    fn send(&self, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>) {
        let _ = (headers, body);
        self.start(url);
    }
    /// Ответ с кодом ошибки HTTP приходит ошибкой «HTTP код: тело», а не
    /// одним кодом. Нужен помощнику ИИ: причину отказа сервис пишет
    /// в теле ответа. Без своей реализации ничего не меняет.
    fn keep_error_bodies(&mut self) {}
    /// Пришедший ответ: байты или ошибка.
    fn take(&self) -> Option<Result<Vec<u8>, String>>;
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
    /// Тайл - байты, а не картинка: векторный тайл.
    pub raw: bool,
    /// Адрес TileJSON. Шаблон адреса тайлов берётся из него, url тогда
    /// не нужен.
    pub tilejson: Option<&'static str>,
    /// Охват районной карты: вне его тайлы берутся у подложки вокруг.
    pub region: Option<Region>,
}

/// Охват районной карты, `RegionalXyz` модуля: в рамке и не мельче
/// уровня min_level - свои тайлы, вокруг - тайлы base. Иначе мир вокруг
/// страны был бы белым.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    /// Запад, юг, восток, север, сотые доли градуса.
    pub bbox: [i32; 4],
    /// Уровень, с которого берётся сама карта.
    pub min_level: u32,
    /// Шаблон адреса подложки вокруг.
    pub base: &'static str,
}

impl Region {
    /// Рамка в метрах EPSG:3857: запад, юг, восток, север.
    fn mercator(&self) -> [f64; 4] {
        let deg = |v: i32| f64::from(v) / 100.0;
        let x = |lon: f64| MERCATOR_HALF * lon / 180.0;
        let y = |lat: f64| {
            let lat = lat.clamp(-85.0511, 85.0511);
            MERCATOR_HALF / std::f64::consts::PI * (std::f64::consts::FRAC_PI_4 + lat.to_radians() / 2.0).tan().ln()
        };
        let [w, s, e, n] = self.bbox.map(deg);
        [x(w), y(s), x(e), y(n)]
    }

    /// Тайл key берётся у самой карты.
    pub fn inside(&self, key: TileKey) -> bool {
        if key.0 < self.min_level {
            return false;
        }
        let [w, s, e, n] = tile_bbox(key);
        let b = self.mercator();
        w < b[2] && e > b[0] && s < b[3] && n > b[1]
    }

    /// Тайл уровня min_level в середине охвата - превью каталога.
    pub fn preview(&self) -> TileKey {
        let z = self.min_level.min(30);
        let b = self.mercator();
        let side = 2.0 * MERCATOR_HALF / f64::from(1u32 << z);
        let x = (((b[0] + b[2]) / 2.0 + MERCATOR_HALF) / side).floor().max(0.0) as u32;
        let y = ((MERCATOR_HALF - (b[1] + b[3]) / 2.0) / side).floor().max(0.0) as u32;
        (z, x, y)
    }
}

/// Подложка OpenStreetMap.
pub const OSM: Source = Source {
    name: "osm",
    url: "https://tile.openstreetmap.org/{z}/{x}/{y}.png",
    max_requests: 2,
    max_level: 19,
    ext: "png",
    placeholders: false,
    raw: false,
    tilejson: None,
    region: None,
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
    raw: false,
    tilejson: None,
    region: None,
};

/// Высоты рельефа Mapzen Terrain Tiles в записи Terrarium, как в модуле.
pub const TERRARIUM: Source = Source {
    name: "terrarium",
    url: "https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png",
    max_requests: 4,
    max_level: 15,
    ext: "png",
    placeholders: false,
    raw: false,
    tilejson: None,
    region: None,
};

/// Векторные тайлы OpenFreeMap, как в модуле. Адрес тайлов меняется
/// с каждой сборкой планеты, поэтому он берётся из TileJSON. Запросов
/// сразу - как у прочих источников модуля, не OSM.
pub const OPENFREEMAP: Source = Source {
    name: "openfreemap",
    url: "",
    max_requests: 4,
    max_level: 14,
    ext: "pbf",
    placeholders: false,
    raw: true,
    tilejson: Some("https://tiles.openfreemap.org/planet"),
    region: None,
};

/// Сколько тайл в кэше считается свежим.
pub const FRESH: Duration = Duration::from_secs(7 * 24 * 3600);
/// Через сколько повторить запрос тайла, который не пришёл. Каждая
/// следующая неудача того же тайла удваивает паузу до [`MAX_RETRY`].
/// С постоянной паузой два медленных тайла по очереди держали загрузчик
/// занятым без конца: сервер высот отвечал по 13-20 с и обрывался
/// по пределу, 23 октября 2026 года.
pub const RETRY: Duration = Duration::from_secs(10);
pub const MAX_RETRY: Duration = Duration::from_secs(320);

/// Пауза до повтора после failures неудач подряд, failures от 1.
pub fn retry_delay(failures: u32) -> Duration {
    let doubled = RETRY.saturating_mul(1u32 << failures.saturating_sub(1).min(16));
    doubled.min(MAX_RETRY)
}
/// Сторона тайла в пикселях.
pub const TILE_SIZE: u32 = 256;

/// Заголовок `User-Agent` - название, версия программы и адрес
/// хранилища. Имени и почты автора нет, решение автора от 21 октября
/// 2026 года.
pub fn user_agent() -> String {
    format!("PlanetX-Lite/{} (+https://github.com/Valery35/planetx-lite)", env!("CARGO_PKG_VERSION"))
}

/// Узел со своими правилами запроса: заголовок Referer и корневой
/// сертификат, которому верят только в запросах к этому узлу, как
/// `fetch_bytes` модуля с `headers` и `CA_FILE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostRule {
    pub host: &'static str,
    pub referer: &'static str,
    /// Корневой сертификат PEM.
    pub root: &'static [u8],
}

/// Хранилища S3 на отсутствующий ключ отвечают 403, а не 404: так
/// приходят пропуски снимков Марса и Луны OpenPlanetaryMap, как
/// `missing=(403, 404)` источника модуля.
pub const FORBIDDEN_IS_MISSING: [&str; 2] = ["https://s3.amazonaws.com/", "https://s3-eu-west-1.amazonaws.com/"];

/// Ответ 403 по адресу url значит, что тайла нет.
pub fn forbidden_is_missing(url: &str) -> bool {
    FORBIDDEN_IS_MISSING.iter().any(|h| url.starts_with(h))
}

/// Правила узлов. НСПД Росреестра (`data::cadastre`): без Referer карты
/// ответ 403, сертификат выдан «Russian Trusted Sub CA» Минцифры,
/// корневого «Russian Trusted Root CA» в системах по умолчанию нет. Его
/// отпечаток SHA-1 - `data::cadastre::CA_SHA1`. У страницы правила
/// не действуют: Referer и сертификаты там задаёт браузер.
pub const HOST_RULES: [HostRule; 1] = [HostRule { host: "nspd.gov.ru", referer: "https://nspd.gov.ru/map", root: include_bytes!("../certs/russian_trusted_root_ca.pem") }];

/// Правило узла адреса url или None.
pub fn host_rule(url: &str) -> Option<&'static HostRule> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#', ':']).next().unwrap_or("");
    HOST_RULES.iter().find(|r| host.eq_ignore_ascii_case(r.host) || host.to_ascii_lowercase().ends_with(&format!(".{}", r.host)))
}

/// Адрес тайла key у источника.
pub fn tile_url(source: &Source, key: TileKey) -> String {
    match source.region {
        Some(r) if !r.inside(key) => template_url(r.base, key),
        _ => template_url(source.url, key),
    }
}

/// Номер тайла Bing (quadkey): цифры 0-3 от уровня 1 до z, `quadkey`
/// модуля.
pub fn quadkey(key: TileKey) -> String {
    let (z, x, y) = key;
    (1..=z.min(31))
        .rev()
        .map(|level| {
            let mask = 1u32 << (level - 1);
            char::from(b'0' + u8::from(x & mask != 0) + 2 * u8::from(y & mask != 0))
        })
        .collect()
}

/// Место ряда тайла эллиптического Меркатора EPSG:3395 в шаблоне адреса,
/// снимки Яндекса. Тайл глобуса собирается из одного-двух тайлов
/// источника, [`elliptic_tile`].
pub const ELLIPTIC_ROW: &str = "{ey}";

/// Эксцентриситет эллипсоида WGS84 - эллиптический Меркатор EPSG:3395.
pub const ECCENTRICITY: f64 = 0.0818191908426215;

/// Строки тайла Web Mercator (z, y) в тайлах эллиптического Меркатора
/// того же уровня, `elliptic_rows` модуля: первый и последний тайл
/// и положения середин строк в картинке, сложенной из тайлов первый -
/// последний, в пикселях от её верха. Столбцы у обеих проекций одни.
pub fn elliptic_rows(z: u32, y: u32, size: usize) -> (u32, u32, Vec<f64>) {
    let count = f64::from(1u32 << z.min(30));
    let rows: Vec<f64> = (0..size)
        .map(|j| {
            let v = (f64::from(y) + (j as f64 + 0.5) / size as f64) / count;
            let s = (std::f64::consts::PI * (1.0 - 2.0 * v)).sinh().atan().sin();
            let north = s.atanh() - ECCENTRICITY * (ECCENTRICITY * s).atanh();
            (0.5 - north / (2.0 * std::f64::consts::PI)) * count
        })
        .collect();
    let clamp = |r: f64| r.floor().clamp(0.0, count - 1.0) as u32;
    let first = clamp(rows.iter().copied().fold(f64::INFINITY, f64::min));
    let last = clamp(rows.iter().copied().fold(f64::NEG_INFINITY, f64::max));
    let at = rows.iter().map(|r| (r - f64::from(first)) * size as f64 - 0.5).collect();
    (first, last, at)
}

/// Тайл Web Mercator (z, y) из картинок тайлов EPSG:3395 одного столбца
/// сверху вниз, `elliptic_tile` модуля: строки - линейным смешением
/// соседних строк сложенной картинки. None - картинки не одной ширины.
pub fn elliptic_tile(z: u32, y: u32, images: &[Image]) -> Option<Image> {
    let width = images.first()?.width;
    if images.iter().any(|i| i.width != width || i.rgba.len() != (i.width * i.height * 4) as usize) {
        return None;
    }
    let stack: Vec<u8> = images.iter().flat_map(|i| i.rgba.iter().copied()).collect();
    let rows = stack.len() / (width as usize * 4);
    let (_, _, at) = elliptic_rows(z, y, width as usize);
    let line = width as usize * 4;
    let mut out = Vec::with_capacity(at.len() * line);
    for a in at {
        let a = a.clamp(0.0, rows.saturating_sub(1) as f64);
        let low = a.floor() as usize;
        let high = (low + 1).min(rows.saturating_sub(1));
        let share = a - low as f64;
        let (l, h) = (stack.get(low * line..(low + 1) * line)?, stack.get(high * line..(high + 1) * line)?);
        out.extend(l.iter().zip(h).map(|(p, q)| (f64::from(*p) * (1.0 - share) + f64::from(*q) * share + 0.5).clamp(0.0, 255.0) as u8));
    }
    Some(Image { width, height: width, rgba: out })
}

/// Байты ответа сервера, Err(None) - ответ 404, тайла нет.
pub type TileBytes = Result<Vec<u8>, Option<String>>;

/// Тайл key источника в эллиптическом Меркаторе по шаблону с местом
/// [`ELLIPTIC_ROW`]: тайлы источника того же столбца просит get, тайл
/// глобуса собирается [`elliptic_tile`] и отдаётся картинкой PNG.
pub fn elliptic_fetch(template: &str, key: TileKey, get: &dyn Fn(&str) -> TileBytes) -> TileBytes {
    let (z, x, y) = key;
    let (first, last, _) = elliptic_rows(z, y, TILE_SIZE as usize);
    let mut images = Vec::new();
    for row in first..=last {
        let url = template_url(&template.replace(ELLIPTIC_ROW, &row.to_string()), (z, x, y));
        let bytes = get(&url)?;
        images.push(decode_tile(&bytes).map_err(Some)?);
    }
    let tile = elliptic_tile(z, y, &images).ok_or_else(|| Some("тайлы эллиптического Меркатора разной ширины".to_string()))?;
    let img = image::RgbaImage::from_raw(tile.width, tile.height, tile.rgba).ok_or_else(|| Some("тайл не собран".to_string()))?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| Some(e.to_string()))?;
    Ok(out.into_inner())
}

/// Адрес тайла key по шаблону с местами {z}, {x}, {y} или {bbox} - рамкой
/// тайла WMS в метрах EPSG:3857, как `WmsSource` модуля. {-y} - ряд снизу
/// вверх, как у TMS, {q} - номер тайла Bing ([`quadkey`]).
pub fn template_url(template: &str, key: TileKey) -> String {
    let (z, x, y) = key;
    let flipped = (1u64 << z.min(31)).saturating_sub(1).saturating_sub(u64::from(y));
    let mut out = template.replace("{z}", &z.to_string()).replace("{x}", &x.to_string()).replace("{-y}", &flipped.to_string()).replace("{y}", &y.to_string());
    if out.contains("{q}") {
        out = out.replace("{q}", &quadkey(key));
    }
    if out.contains("{bbox}") {
        let b = tile_bbox(key);
        return out.replace("{bbox}", &format!("{:.2},{:.2},{:.2},{:.2}", b[0], b[1], b[2], b[3]));
    }
    out
}

/// Половина стороны мира Web Mercator, метры.
pub const MERCATOR_HALF: f64 = std::f64::consts::PI * 6378137.0;

/// Рамка тайла key в метрах EPSG:3857: запад, юг, восток, север.
pub fn tile_bbox(key: TileKey) -> [f64; 4] {
    let (z, x, y) = key;
    let side = 2.0 * MERCATOR_HALF / 2f64.powi(z.min(30) as i32);
    let west = -MERCATOR_HALF + f64::from(x) * side;
    let north = MERCATOR_HALF - f64::from(y) * side;
    [west, north - side, west + side, north]
}

/// Первый шаблон адреса тайлов из TileJSON - строка массива "tiles".
/// Ответ сервера считается враждебным: разбор без сбоев, None - шаблона
/// нет или он не https с местами {z}, {x}, {y}. Адрес http допустим
/// только у сервера на этой машине, это сервер тестов.
pub fn tilejson_template(text: &str) -> Option<String> {
    let after = &text[text.find("\"tiles\"")? + 7..];
    let after = after.trim_start().strip_prefix(':')?.trim_start().strip_prefix('[')?.trim_start();
    let body = after.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = body.chars();
    loop {
        match chars.next()? {
            '"' => break,
            '\\' => match chars.next()? {
                '/' => out.push('/'),
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                _ => return None,
            },
            c => out.push(c),
        }
        if out.len() > 2048 {
            return None;
        }
    }
    let scheme = out.starts_with("https://") || out.starts_with("http://127.0.0.1:");
    let ok = scheme && ["{z}", "{x}", "{y}"].iter().all(|p| out.contains(p));
    ok.then_some(out)
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
    // Тайлы 512 × 512, как у Macrostrat, - вдвое мельче, средним
    // четырёх точек с премноженной альфой.
    if (width, height) == (2 * TILE_SIZE, 2 * TILE_SIZE) {
        return Ok(Image { width: TILE_SIZE, height: TILE_SIZE, rgba: halve(rgba.as_raw(), width as usize) });
    }
    if (width, height) != (TILE_SIZE, TILE_SIZE) {
        return Err(format!("тайл {width} × {height}, ждали {TILE_SIZE} × {TILE_SIZE}"));
    }
    Ok(Image { width, height, rgba: rgba.into_raw() })
}

/// Картинка RGBA side × side вдвое мельче: среднее четырёх точек, цвет -
/// по весу альфы, чтобы прозрачные точки не темнили край.
fn halve(rgba: &[u8], side: usize) -> Vec<u8> {
    let half = side / 2;
    let mut out = Vec::with_capacity(half * half * 4);
    for r in 0..half {
        for c in 0..half {
            let mut sum = [0u32; 4];
            for (dr, dc) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                let i = ((2 * r + dr) * side + 2 * c + dc) * 4;
                let a = u32::from(rgba[i + 3]);
                for k in 0..3 {
                    sum[k] += u32::from(rgba[i + k]) * a;
                }
                sum[3] += a;
            }
            for k in 0..3 {
                out.push(sum[k].checked_div(sum[3]).unwrap_or(0) as u8);
            }
            out.push((sum[3] / 4) as u8);
        }
    }
    out
}

/// Итог загрузки тайла: картинка, байты, «тайла нет» или ошибка.
pub enum Fetched {
    Image(Image),
    Bytes(Vec<u8>),
    Missing,
    Failed(String),
}

impl Fetched {
    /// Раскодировать ответ сервера. Заглушка у источника с заглушками -
    /// «тайла нет».
    pub fn from_bytes(source: &Source, bytes: &[u8]) -> Self {
        if source.raw {
            return Fetched::Bytes(bytes.to_vec());
        }
        match decode_tile(bytes) {
            Ok(img) if source.placeholders && placeholder::is_placeholder(&img.rgba) => Fetched::Missing,
            Ok(img) => Fetched::Image(img),
            Err(e) => Fetched::Failed(e),
        }
    }

    /// Пришедший тайл для главного потока.
    pub fn into_loaded(self, key: TileKey) -> Loaded {
        match self {
            Fetched::Image(img) => Loaded { key, content: Ok(Content::Image(img)), missing: false },
            Fetched::Bytes(b) => Loaded { key, content: Ok(Content::Bytes(b)), missing: false },
            Fetched::Missing => Loaded { key, content: Err("тайла нет у источника".into()), missing: true },
            Fetched::Failed(e) => Loaded { key, content: Err(e), missing: false },
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
    /// Тайлы после неудачи: когда можно повторить и неудач подряд.
    failed: HashMap<TileKey, (Instant, u32)>,
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
        // Неудачи тайлов, которые кадру больше не нужны и уже дождались
        // повтора, забываются: таблица не растёт без конца.
        let now = Instant::now();
        self.failed.retain(|k, (at, _)| now < *at || seen.contains(k));
    }

    /// Следующий тайл для загрузки или None. Тайл помечается начатым.
    pub fn take_next(&mut self) -> Option<TileKey> {
        self.take_next_at(Instant::now())
    }

    /// [`Queue::take_next`] на момент now.
    pub fn take_next_at(&mut self, now: Instant) -> Option<TileKey> {
        let waiting = |k: &TileKey, failed: &HashMap<TileKey, (Instant, u32)>| failed.get(k).is_some_and(|(at, _)| now < *at);
        let pos = self.wanted.iter().position(|(k, _)| !self.in_flight.contains(k) && !waiting(k, &self.failed))?;
        let (key, _) = self.wanted.remove(pos);
        self.in_flight.insert(key);
        Some(key)
    }

    /// Загрузка тайла закончилась. Неудача откладывает повтор на
    /// [`retry_delay`], удача забывает прежние неудачи.
    pub fn done(&mut self, key: TileKey, ok: bool) {
        self.done_at(key, ok, Instant::now());
    }

    /// [`Queue::done`] на момент now.
    pub fn done_at(&mut self, key: TileKey, ok: bool, now: Instant) {
        self.in_flight.remove(&key);
        if ok {
            self.failed.remove(&key);
        } else {
            let failures = self.failed.get(&key).map_or(1, |(_, n)| n + 1);
            self.failed.insert(key, (now + retry_delay(failures), failures));
        }
    }

    /// Пауза до повтора тайла после неудачи на момент now, None - тайл
    /// не ждёт повтора.
    pub fn retry_in(&self, key: TileKey, now: Instant) -> Option<Duration> {
        self.failed.get(&key).map(|(at, _)| at.saturating_duration_since(now)).filter(|d| !d.is_zero())
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
        let now = Instant::now();
        self.wanted.iter().filter(|(k, _)| !self.in_flight.contains(k) && self.retry_in(*k, now).is_none()).count()
    }

    pub fn is_idle(&self) -> bool {
        self.in_flight.is_empty() && self.pending() == 0
    }
}
