// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Основа тела из глобальной мозаики Cloud Optimized GeoTIFF по сети:
//! тайлы Web Mercator тела из окон файла, без своих копий. Так Луна берёт
//! мозаику LROC WAC 100 м прямо из хранилища astrogeo-ard USGS.
//!
//! Мозаика - в простой цилиндрической проекции тела и покрывает 360°
//! по долготе и 180° по широте: долготы и широты пересчитываются в систему
//! файла линейно по его привязке. Заголовок читается один раз
//! ([`crate::cogread::HeaderRead`]), на каждый тайл - окно уровня обзора
//! с шагом не крупнее пикселя тайла ([`crate::cogread::BandRead`]), потом
//! строки окна пересчитываются на широты Mercator. Значения канала - серые
//! пиксели 0-255.

use std::collections::HashMap;
use std::sync::Arc;

use data::cog::CogHeader;
use globe_core::lod::TileKey;

use crate::cogread::{BandRead, HeaderRead};

/// Сторона тайла.
const SIZE: usize = 256;
/// Окон в чтении одновременно.
const ACTIVE: usize = 6;

/// Пришедший тайл: снимок или пропуск.
pub enum Tile {
    Image(net::Image),
    Missing,
}

/// Охват файла в его системе: запад, север, шаг на градус по x и y,
/// долгота и широта западного и северного края.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Frame {
    west: f64,
    north: f64,
    per_lon: f64,
    per_lat: f64,
    lon0: f64,
    lat0: f64,
}

impl Frame {
    /// Привязка глобального файла: ширина - 360°, высота - 180°. clon -
    /// центральный меридиан проекции файла.
    fn of(header: &CogHeader, clon: f64) -> Option<Frame> {
        let gt = header.geotransform?;
        let level = header.levels.first()?;
        let (w, h) = (f64::from(level.width), f64::from(level.height));
        let per_lon = w * gt[1] / 360.0;
        let per_lat = h * -gt[5] / 180.0;
        if !(per_lon.is_finite() && per_lat.is_finite()) || per_lon <= 0.0 || per_lat <= 0.0 {
            return None;
        }
        Some(Frame { west: gt[0], north: gt[3], per_lon, per_lat, lon0: clon + gt[0] / per_lon, lat0: gt[3] / per_lat })
    }

    /// Градусы от западного края файла до долготы lon, от 0 до 360.
    fn east_of_edge(&self, lon: f64) -> f64 {
        (lon - self.lon0).rem_euclid(360.0)
    }

    /// Отрезок градусов от западного края файла под долготами тайла
    /// west..east. Тайл через шов файла берёт всю его ширину.
    fn span(&self, west: f64, east: f64) -> (f64, f64) {
        let from = self.east_of_edge(west);
        let to = from + (east - west);
        if to > 360.0 + 1e-9 { (0.0, 360.0) } else { (from, to) }
    }

    fn x(&self, degrees: f64) -> f64 {
        self.west + degrees * self.per_lon
    }

    fn y(&self, lat: f64) -> f64 {
        self.north - (self.lat0 - lat) * self.per_lat
    }
}

/// Широта края ряда тайлов: доля t от северного края мира.
fn mercator_lat(t: f64) -> f64 {
    (std::f64::consts::PI * (1.0 - 2.0 * t)).sinh().atan().to_degrees()
}

/// Рамка тайла: запад, юг, восток, север в градусах.
pub fn tile_bounds(key: TileKey) -> [f64; 4] {
    let (z, x, y) = key;
    let n = f64::from(1u32 << z.min(30));
    let west = f64::from(x) / n * 360.0 - 180.0;
    let east = f64::from(x + 1) / n * 360.0 - 180.0;
    [west, mercator_lat(f64::from(y + 1) / n), east, mercator_lat(f64::from(y) / n)]
}

/// Окно, пересчитанное на тайл Mercator: ряды окна идут по широте
/// линейно от north до south, ряды тайла - по Mercator, столбцы окна -
/// градусы span от западного края файла.
fn to_tile(raw: &data::sentinel::Raw, bounds: [f64; 4], key: TileKey, frame: &Frame, span: (f64, f64)) -> Option<net::Image> {
    let (rows, cols) = raw.shape();
    if rows == 0 || cols == 0 {
        return None;
    }
    let [west, south, east, north] = bounds;
    let n = f64::from(1u32 << key.0.min(30));
    let mut rgba = Vec::with_capacity(SIZE * SIZE * 4);
    for j in 0..SIZE {
        let lat = mercator_lat((f64::from(key.2) + (j as f64 + 0.5) / SIZE as f64) / n);
        let fy = ((north - lat) / (north - south) * rows as f64 - 0.5).clamp(0.0, rows as f64 - 1.0);
        let (r0, ty) = (fy.floor() as usize, fy - fy.floor());
        let r1 = (r0 + 1).min(rows - 1);
        for i in 0..SIZE {
            let lon = west + (i as f64 + 0.5) / SIZE as f64 * (east - west);
            let mut d = frame.east_of_edge(lon);
            if d < span.0 {
                d += 360.0;
            }
            let fx = ((d - span.0) / (span.1 - span.0) * cols as f64 - 0.5).clamp(0.0, cols as f64 - 1.0);
            let (c0, tx) = (fx.floor() as usize, fx - fx.floor());
            let c1 = (c0 + 1).min(cols - 1);
            let v = |r: usize, c: usize| raw.get(r, c).unwrap_or(0.0);
            let top = v(r0, c0) * (1.0 - tx) + v(r0, c1) * tx;
            let bottom = v(r1, c0) * (1.0 - tx) + v(r1, c1) * tx;
            let g = (top * (1.0 - ty) + bottom * ty).round().clamp(0.0, 255.0) as u8;
            rgba.extend([g, g, g, 255]);
        }
    }
    Some(net::Image { width: SIZE as u32, height: SIZE as u32, rgba })
}

/// Основа из мозаики COG.
pub struct CogTiles {
    href: String,
    header: HeaderRead,
    /// Тайлы, которые просит кадр, по приоритету, больше - раньше.
    wanted: Vec<(TileKey, f64)>,
    /// Чтения окон и их отрезки градусов от края файла.
    reads: HashMap<TileKey, (BandRead, (f64, f64))>,
    /// Центральный меридиан проекции файла.
    clon: f64,
    done: Vec<(TileKey, Tile)>,
    /// Тайлы, которые уже пришли или пропущены, второй раз не просятся.
    seen: std::collections::HashSet<TileKey>,
}

impl CogTiles {
    pub fn new(href: &str, clon: f64, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Self {
        Self { href: href.to_string(), header: HeaderRead::new(href, request), wanted: Vec::new(), reads: HashMap::new(), clon, done: Vec::new(), seen: Default::default() }
    }

    /// Тайлы, которые просит кадр. Чтения тайлов вне списка бросаются.
    pub fn want(&mut self, wanted: &[(TileKey, f64)]) {
        self.wanted = wanted.iter().copied().filter(|(k, p)| p.is_finite() && !self.seen.contains(k)).collect();
        self.wanted.sort_by(|a, b| b.1.total_cmp(&a.1));
        let keep: std::collections::HashSet<TileKey> = self.wanted.iter().map(|(k, _)| *k).collect();
        self.reads.retain(|k, _| keep.contains(k));
    }

    /// Загрузка идёт: заголовок, чтения окон или тайлы кадра, которые ещё
    /// не просились.
    pub fn busy(&self) -> bool {
        if matches!(self.header.result(), Some(Err(_))) {
            return false;
        }
        self.header.result().is_none() || !self.reads.is_empty() || self.wanted.iter().any(|(k, _)| !self.seen.contains(k))
    }

    /// Ошибка заголовка, если файл не прочитался.
    pub fn error(&self) -> Option<&str> {
        match self.header.result() {
            Some(Err(e)) => Some(e),
            _ => None,
        }
    }

    /// Шаг: ответы, новые чтения окон.
    pub fn step(&mut self, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        self.header.step(request);
        let Some(Ok(header)) = self.header.result() else {
            return;
        };
        let header: Arc<CogHeader> = header.clone();
        let Some(frame) = Frame::of(&header, self.clon) else {
            return;
        };
        for (read, _) in self.reads.values_mut() {
            read.step(request);
        }
        let finished: Vec<TileKey> = self.reads.iter().filter(|(_, (r, _))| r.result().is_some() || r.error().is_some()).map(|(k, _)| *k).collect();
        for key in finished {
            if let Some((read, span)) = self.reads.remove(&key) {
                let tile = read.result().and_then(|raw| to_tile(raw, tile_bounds(key), key, &frame, span)).map_or(Tile::Missing, Tile::Image);
                self.seen.insert(key);
                self.done.push((key, tile));
            }
        }
        let starts: Vec<TileKey> = self.wanted.iter().map(|(k, _)| *k).filter(|k| !self.reads.contains_key(k) && !self.seen.contains(k)).take(ACTIVE.saturating_sub(self.reads.len())).collect();
        for key in starts {
            let [west, south, east, north] = tile_bounds(key);
            // Шаг окна - пиксель тайла по долготе, рамка - по системе файла.
            let span = frame.span(west, east);
            let res = (span.1 - span.0) / SIZE as f64 * frame.per_lon;
            let bbox = [frame.x(span.0), frame.y(south), frame.x(span.1), frame.y(north)];
            self.reads.insert(key, (BandRead::with_header(&self.href, header.clone(), bbox, res, false, request), span));
        }
    }

    /// Пришедшие с прошлого вызова тайлы.
    pub fn take(&mut self) -> Vec<(TileKey, Tile)> {
        std::mem::take(&mut self.done)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn bounds_and_frame() {
        let [w, s, e, n] = tile_bounds((1, 0, 0));
        assert_eq!((w, e), (-180.0, 0.0));
        assert!(s.abs() < 1e-9 && (n - 85.0511).abs() < 1e-3);
        // Мозаика WAC: 244098 × 122049 пикселей по 100 м в метрах
        // простой цилиндрической проекции Луны.
        let r = 1737400.0 * std::f64::consts::PI / 180.0;
        let px = 360.0 * r / 244098.0;
        let header = CogHeader {
            big_endian: false,
            bigtiff: true,
            levels: vec![data::cog::Level {
                width: 244098,
                height: 122049,
                tile_width: 512,
                tile_height: 512,
                samples: 1,
                bits: 8,
                format: data::cog::SampleFormat::Uint,
                compression: data::cog::Compression::Lzw,
                predictor: data::cog::Predictor::None,
                planar: data::cog::Planar::Chunky,
                tile_offsets: Vec::new(),
                tile_byte_counts: Vec::new(),
            }],
            nodata: None,
            geotransform: Some([-180.0 * r, px, 0.0, 90.0 * r, 0.0, -px]),
            epsg: None,
            pixel_is_point: false,
            scale: None,
            offset: None,
        };
        // Центральный меридиан 180°: западный край файла - долгота 0.
        let f = Frame::of(&header, 180.0).unwrap();
        assert!(f.lon0.abs() < 1e-9);
        assert_eq!(f.span(23.0, 24.0), (23.0, 24.0));
        assert_eq!(f.span(-90.0, 0.0), (270.0, 360.0));
        assert_eq!(f.span(-180.0, 180.0), (0.0, 360.0));
        assert!((f.x(360.0) - (header.geotransform.unwrap()[0] + 244098.0 * px)).abs() < 1e-3);
        let g = Frame::of(&header, 0.0).unwrap();
        assert!((g.lon0 + 180.0).abs() < 1e-9 && g.span(-180.0, -90.0) == (0.0, 90.0));
    }

    #[test]
    fn window_rows_follow_mercator() {
        // Окно из двух рядов: север белый, юг чёрный. В тайле над
        // экватором белого больше, чем в линейной разбивке.
        let raw = data::sentinel::Raw::from_vec(2, 1, vec![255.0, 0.0]).unwrap();
        let key = (1, 0, 0);
        let frame = Frame { west: 0.0, north: 0.0, per_lon: 1.0, per_lat: 1.0, lon0: -180.0, lat0: 90.0 };
        let img = to_tile(&raw, tile_bounds(key), key, &frame, (0.0, 180.0)).unwrap();
        assert_eq!(img.rgba.len(), SIZE * SIZE * 4);
        let middle = img.rgba[(SIZE / 2 * SIZE) * 4];
        assert!(middle > 128, "{middle}");
    }

    /// Живая проверка: тайлы ближней стороны Луны из мозаики WAC.
    /// cargo test -p ui live_wac -- --ignored --nocapture
    #[test]
    #[ignore]
    #[cfg(not(target_arch = "wasm32"))]
    fn live_wac() {
        let request = |limit: u64| -> Box<dyn net::Request> { Box::new(net::native::Requests::with_limit(std::sync::Arc::new(|| {}), limit)) };
        let mut tiles = CogTiles::new(scene::planets::MOON_WAC, 180.0, &request);
        let keys: Vec<(TileKey, f64)> = [(2, 1, 1), (2, 2, 1), (3, 3, 3), (3, 4, 3), (4, 7, 7), (4, 8, 7)].iter().map(|k| (*k, 1.0)).collect();
        let start = std::time::Instant::now();
        let mut got = Vec::new();
        while got.len() < keys.len() && start.elapsed().as_secs() < 180 {
            tiles.want(&keys);
            tiles.step(&request);
            for (k, t) in tiles.take() {
                let what = match &t {
                    Tile::Image(i) => format!("image, mean {}", i.rgba.iter().step_by(4).map(|v| u64::from(*v)).sum::<u64>() / (i.rgba.len() as u64 / 4)),
                    Tile::Missing => "missing".to_string(),
                };
                eprintln!("{:>6.1} s {k:?} {what}", start.elapsed().as_secs_f64());
                got.push(k);
            }
            if let Some(e) = tiles.error() {
                panic!("{e}");
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(got.len(), keys.len(), "busy {}", tiles.busy());
    }
}
