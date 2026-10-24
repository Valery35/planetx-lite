// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Рельеф, слой L1: высоты из тайлов Terrarium.
//!
//! Тайл Terrarium - картинка 256 × 256 в сетке Web Mercator, уровни
//! 0-15. Высота пикселя в метрах равна R·256 + G + B/256 - 32768.
//! Пиксель покрывает квадрат, его значение относится к центру квадрата.
//!
//! Отрицательные высоты - это дно моря и впадины суши. Подложка лежит
//! на уровне моря, поэтому у Земли высоты ниже нуля прижимаются к нулю.
//! Впадины суши вроде Мёртвого моря при этом тоже уходят на ноль.
//!
//! Перенос `planetx/core/terrain.py` модуля. Сетка тайла на рельефе -
//! [`mesh`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use globe_core::lod::TileKey;

pub mod inset;
pub mod mesh;

pub const MAX_LEVEL: u32 = 15;
pub const SIZE: usize = 256;
/// Уровень высот для тайла подложки уровня z. 256 пикселей высот на 16
/// отрезков сетки дают 4 пикселя на отрезок уже при z - 2, поэтому
/// высоты берутся на два уровня грубее. Загрузок высот так в 16 раз
/// меньше, чем тайлов подложки.
pub const LEVEL_OFFSET: u32 = 2;
/// Самый подробный уровень высот внутри врезки своего рельефа: подложка
/// доходит до уровня 19, высоты берутся на [`LEVEL_OFFSET`] грубее.
pub const DEEP_LEVEL: u32 = 19 - LEVEL_OFFSET;
/// Уровень высот сетки, собранной без рельефа при выключенном рельефе.
/// Он глубже любого тайла высот, поэтому новые высоты такую сетку
/// не пересобирают.
pub const FLAT_LEVEL: u32 = DEEP_LEVEL + 1;
/// Граница сетки Mercator по широте, градусы.
const MERCATOR_LAT: f64 = 85.051_128_78;

/// Запись высоты в цвете пикселя.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// Mapzen Terrarium: R·256 + G + B/256 - 32768.
    Terrarium,
    /// Mapbox Terrain-RGB: -10000 + (R·65536 + G·256 + B)·0.1.
    Mapbox,
}

/// Пиксели высот (по channels байт на пиксель, первые три - RGB)
/// в высоты, метры.
///
/// floor - нижний предел высот. У Земли 0: подложка рисует воду
/// на уровне моря, дно под ней не нужно. У Марса и Луны None - впадины
/// вроде равнины Эллада глубже нуля на километры и остаются.
pub fn decode(rgba: &[u8], channels: usize, floor: Option<f32>, encoding: Encoding) -> Vec<f32> {
    if channels < 3 {
        return Vec::new();
    }
    rgba.chunks_exact(channels)
        .map(|px| {
            let (r, g, b) = (f32::from(px[0]), f32::from(px[1]), f32::from(px[2]));
            let h = match encoding {
                Encoding::Terrarium => r * 256.0 + g + b / 256.0 - 32768.0,
                Encoding::Mapbox => (r * 65536.0 + g * 256.0 + b) * 0.1 - 10000.0,
            };
            floor.map_or(h, |f| h.max(f))
        })
        .collect()
}

/// Тайл высот: SIZE × SIZE высот по строкам с севера, метры, наименьшая
/// и наибольшая высота.
#[derive(Debug, Clone, PartialEq)]
pub struct HeightTile {
    pub z: u32,
    pub x: u32,
    pub y: u32,
    pub heights: Vec<f32>,
    pub low: f32,
    pub high: f32,
}

impl HeightTile {
    /// Тайл из высот. None - высот не SIZE × SIZE.
    pub fn new(z: u32, x: u32, y: u32, heights: Vec<f32>) -> Option<Self> {
        if heights.len() != SIZE * SIZE {
            return None;
        }
        let low = heights.iter().copied().fold(f32::INFINITY, f32::min);
        let high = heights.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        Some(Self { z, x, y, heights, low, high })
    }

    pub fn key(&self) -> TileKey {
        (self.z, self.x, self.y)
    }

    fn at(&self, row: usize, col: usize) -> f64 {
        f64::from(self.heights.get(row * SIZE + col).copied().unwrap_or(0.0))
    }
}

/// Тайл высот из картинки RGBA SIZE × SIZE. None - картинка не того
/// размера.
pub fn make_tile(z: u32, x: u32, y: u32, rgba: &[u8], floor: Option<f32>, encoding: Encoding) -> Option<HeightTile> {
    if rgba.len() != SIZE * SIZE * 4 {
        return None;
    }
    HeightTile::new(z, x, y, decode(rgba, 4, floor, encoding))
}

/// Уровень высот для тайла подложки уровня z, не глубже top.
pub fn height_level(z: u32, top: u32) -> u32 {
    z.saturating_sub(LEVEL_OFFSET).min(top)
}

/// Предок тайла key на уровне level, level не глубже key.
pub fn ancestor(key: TileKey, level: u32) -> TileKey {
    let (z, x, y) = key;
    let shift = z.saturating_sub(level).min(31);
    (level.min(z), x >> shift, y >> shift)
}

/// Тайл высот из картинки RGBA и глубины моря того же тайла: высоты
/// ниже floor обрезаются до floor, как у [`make_tile`], глубины - высоты
/// без обрезки. Глубин нет (None), если ниже floor ничего нет.
pub fn make_tile_depths(z: u32, x: u32, y: u32, rgba: &[u8], floor: f32, encoding: Encoding) -> Option<(HeightTile, Option<Vec<i16>>)> {
    if rgba.len() != SIZE * SIZE * 4 {
        return None;
    }
    let raw = decode(rgba, 4, None, encoding);
    let depths = raw.iter().any(|h| *h < floor).then(|| raw.iter().map(|h| h.round().clamp(-32768.0, 32767.0) as i16).collect());
    let tile = HeightTile::new(z, x, y, raw.into_iter().map(|h| h.max(floor)).collect())?;
    Some((tile, depths))
}

/// Высота в точке с долями мира u (запад - восток) и v (север - юг).
///
/// Выборка билинейная по центрам пикселей, за краем тайла берётся
/// крайний пиксель.
pub fn sample(tile: &HeightTile, u: f64, v: f64) -> f64 {
    sample_grid((tile.z, tile.x, tile.y), &|row, col| tile.at(row, col), u, v)
}

/// Билинейная выборка сетки SIZE × SIZE тайла key, значение узла - at.
fn sample_grid(key: TileKey, at: &dyn Fn(usize, usize) -> f64, u: f64, v: f64) -> f64 {
    let (z, x, y) = key;
    let n = f64::from(1u32 << z.min(31));
    let size = SIZE as f64;
    let px = ((u * n - f64::from(x)) * size - 0.5).clamp(0.0, size - 1.0);
    let py = ((v * n - f64::from(y)) * size - 0.5).clamp(0.0, size - 1.0);
    // NaN доли прижимаются к краю, приведение к целому насыщается.
    let x0 = (px.floor() as usize).min(SIZE - 2);
    let y0 = (py.floor() as usize).min(SIZE - 2);
    let fx = px - x0 as f64;
    let fy = py - y0 as f64;
    let top = at(y0, x0) * (1.0 - fx) + at(y0, x0 + 1) * fx;
    let bottom = at(y0 + 1, x0) * (1.0 - fx) + at(y0 + 1, x0 + 1) * fx;
    top * (1.0 - fy) + bottom * fy
}

/// Доли мира u, v узлов сетки тайла по строкам, с полосой border узлов
/// вокруг.
///
/// Узлы считаются от общего целого номера, как в `tiling::grid_latlon`.
/// Поэтому у соседей с одной картой высот общий край получает одни
/// и те же высоты до бита.
pub fn grid_shares(z: u32, x: u32, y: u32, segments: u32, border: u32) -> Vec<(f64, f64)> {
    let total = f64::from(1u32 << z.min(31)) * f64::from(segments);
    let border = i64::from(border);
    let idx: Vec<i64> = (-border..=i64::from(segments) + border).collect();
    let shares = |t: u32| -> Vec<f64> {
        idx.iter().map(|k| (i64::from(t) * i64::from(segments) + k) as f64 / total).collect()
    };
    let (us, vs) = (shares(x), shares(y));
    vs.iter().flat_map(|v| us.iter().map(move |u| (*u, *v))).collect()
}

/// Тайл без NaN и бесконечностей: такие пиксели получают нулевую
/// высоту, тайл из одних таких пикселей - None, он не берётся.
///
/// В модуле 17 октября 2026 года кадр падал в `nearest_terrain`: высота
/// рельефа под глазом пришла NaN.
pub fn finite_tile(tile: HeightTile) -> Option<HeightTile> {
    if tile.heights.iter().all(|h| h.is_finite()) {
        return HeightTile::new(tile.z, tile.x, tile.y, tile.heights);
    }
    if !tile.heights.iter().any(|h| h.is_finite()) {
        return None;
    }
    let heights = tile.heights.into_iter().map(|h| if h.is_finite() { h } else { 0.0 }).collect();
    HeightTile::new(tile.z, tile.x, tile.y, heights)
}

/// Доли мира u, v точки с широтой и долготой в градусах.
pub fn mercator_share(lat: f64, lon: f64) -> (f64, f64) {
    let lat = lat.clamp(-MERCATOR_LAT, MERCATOR_LAT);
    let u = ((lon + 180.0) / 360.0).rem_euclid(1.0);
    let v = (1.0 - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2.0;
    (u, v.clamp(0.0, 1.0))
}

/// Рамка врезки своего рельефа в долях мира: внутри неё высоты идут
/// глубже предельного уровня, до level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inset {
    pub u0: f64,
    pub v0: f64,
    pub u1: f64,
    pub v1: f64,
    pub level: u32,
}

/// Кэши размахов высот, за замком: размах спрашивают по общей ссылке.
#[derive(Debug, Default)]
struct Ranges {
    /// Размах по ключу тайла подложки без масштаба и количество
    /// добавленных тайлов при подсчёте.
    spans: HashMap<TileKey, ((f64, f64), usize)>,
    /// Для каких врезок и предельного уровня посчитаны spans.
    spans_for: Option<(Vec<Inset>, u32)>,
    /// Размах по ключу и тайл высот, по которому он посчитан.
    ranges: HashMap<TileKey, (Arc<HeightTile>, (f64, f64))>,
    /// Пересчётов размаха всего, для проверок.
    computed: usize,
}

/// Загруженные тайлы высот и выбор лучшего для точки или тайла.
///
/// Вертикальный масштаб рельефа ([`HeightStore::scale`]) умножает все
/// отдаваемые высоты и размахи, 0 - рельеф выключен, Земля гладкая.
#[derive(Debug)]
pub struct HeightStore {
    tiles: HashMap<TileKey, Arc<HeightTile>>,
    /// Глубины моря тайлов, где высоты обрезаны до уровня моря: глобус
    /// показывает море ровным, профиль высот - с дном, как модуль
    /// с глубинами морей.
    depths: HashMap<TileKey, Arc<Vec<i16>>>,
    ranges: Mutex<Ranges>,
    /// Ключи последних добавленных тайлов высот.
    added_log: Vec<TileKey>,
    /// Добавленных тайлов всего.
    adds: usize,
    scale: f64,
    /// Самый подробный уровень тайлов высот источника: у Terrarium 15,
    /// у высот Марса и Луны 5. Глубже тайлы не просятся.
    pub max_level: u32,
    /// Рамки врезок своего рельефа.
    pub deep: Vec<Inset>,
    version: u64,
    added: Vec<TileKey>,
    low: f64,
}

impl Default for HeightStore {
    fn default() -> Self {
        Self::new()
    }
}

impl HeightStore {
    pub const RANGE_CACHE: usize = 50_000;
    /// Ключей добавленных тайлов в памяти.
    pub const LOG_SIZE: usize = 256;
    /// Не больше стольких новых тайлов сверяется с ключом.
    pub const LOG_CHECK: usize = 32;

    pub fn new() -> Self {
        Self {
            tiles: HashMap::new(),
            depths: HashMap::new(),
            ranges: Mutex::new(Ranges::default()),
            added_log: Vec::new(),
            adds: 0,
            scale: 1.0,
            max_level: MAX_LEVEL,
            deep: Vec::new(),
            version: 0,
            added: Vec::new(),
            low: 0.0,
        }
    }

    fn ranges(&self) -> MutexGuard<'_, Ranges> {
        self.ranges.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Насколько рельеф опускается ниже эллипсоида, метры, с масштабом.
    /// На эту глубину опускается подстилка.
    pub fn depth(&self) -> f64 {
        -self.low * self.scale
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// Новый вертикальный масштаб. Меняет все высоты сразу.
    pub fn set_scale(&mut self, scale: f64) {
        self.scale = if scale.is_finite() { scale } else { 0.0 };
        self.version += 1;
    }

    /// Забыть все высоты: другое тело, у него свой рельеф или нет
    /// никакого.
    pub fn clear(&mut self) {
        self.tiles.clear();
        self.depths.clear();
        *self.ranges() = Ranges::default();
        self.added.clear();
        self.low = 0.0;
        self.version += 1;
    }

    /// Добавить тайл высот. Тайл из одних NaN не берётся.
    ///
    /// Кэш размахов не сбрасывается. Размах помнит тайл высот, по
    /// которому посчитан, и пересчитывается, когда лучший тайл другой.
    pub fn add(&mut self, tile: HeightTile) {
        let Some(tile) = finite_tile(tile) else {
            return;
        };
        let key = tile.key();
        self.low = self.low.min(f64::from(tile.low));
        self.tiles.insert(key, Arc::new(tile));
        self.added_log.push(key);
        if self.added_log.len() > Self::LOG_SIZE {
            let keep = Self::LOG_SIZE / 2;
            self.added_log.drain(..self.added_log.len() - keep);
        }
        self.adds += 1;
        self.version += 1;
        self.added.push(key);
    }

    /// Ключи тайлов, добавленных с прошлого вызова.
    pub fn take_added(&mut self) -> Vec<TileKey> {
        std::mem::take(&mut self.added)
    }

    /// Растёт с каждым добавленным тайлом и со сменой масштаба. По ней
    /// выбор тайлов понимает, что размахи высот могли измениться.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Самая низкая высота загруженных тайлов без масштаба, не выше нуля.
    pub fn low(&self) -> f64 {
        self.low
    }

    pub fn tile(&self, key: TileKey) -> Option<&Arc<HeightTile>> {
        self.tiles.get(&key)
    }

    /// Ключи загруженных тайлов высот.
    pub fn keys(&self) -> Vec<TileKey> {
        self.tiles.keys().copied().collect()
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// Предельный уровень высот для тайла key: max_level, внутри рамки
    /// врезки - уровень врезки.
    pub fn cap(&self, key: TileKey) -> u32 {
        let (z, x, y) = key;
        let n = f64::from(1u32 << z.min(31));
        let (u0, v0, u1, v1) = (f64::from(x) / n, f64::from(y) / n, f64::from(x + 1) / n, f64::from(y + 1) / n);
        self.deep
            .iter()
            .filter(|d| u0 < d.u1 && d.u0 < u1 && v0 < d.v1 && d.v0 < v1)
            .map(|d| d.level)
            .fold(self.max_level, u32::max)
    }

    /// Самый точный готовый тайл высот для тайла подложки key.
    ///
    /// Идёт от уровня `height_level(z)` вверх к уровню 0. None, если
    /// не готов даже тайл уровня 0.
    pub fn best(&self, key: TileKey) -> Option<Arc<HeightTile>> {
        let level = height_level(key.0, self.cap(key));
        (0..=level).rev().find_map(|zh| self.tiles.get(&ancestor(key, zh)).cloned())
    }

    /// Тайл высот для сетки тайла подложки key и его уровень.
    ///
    /// Рельеф выключен - (None, FLAT_LEVEL). Высот нет - (None, -1).
    /// Сетка пересобирается, когда уровень больше уровня её сборки.
    pub fn for_mesh(&self, key: TileKey) -> (Option<Arc<HeightTile>>, i32) {
        if self.scale == 0.0 {
            return (None, FLAT_LEVEL as i32);
        }
        let tile = self.best(key);
        let level = tile.as_ref().map_or(-1, |t| t.z as i32);
        (tile, level)
    }

    /// Ключ тайла высот, нужного тайлу подложки key.
    pub fn wanted(&self, key: TileKey) -> TileKey {
        ancestor(key, height_level(key.0, self.cap(key)))
    }

    /// Высота без масштаба по самому точному готовому тайлу.
    fn raw_height(&self, lat: f64, lon: f64) -> f64 {
        let (u, v) = mercator_share(lat, lon);
        for zh in (0..=DEEP_LEVEL).rev() {
            let n = 1u32 << zh;
            let nf = f64::from(n);
            let key = (zh, ((u * nf) as u32).min(n - 1), ((v * nf) as u32).min(n - 1));
            if let Some(tile) = self.tiles.get(&key) {
                return sample(tile, u, v);
            }
        }
        0.0
    }

    /// Глубины моря тайла key ([`make_tile_depths`]).
    pub fn add_depths(&mut self, key: TileKey, depths: Vec<i16>) {
        if depths.len() == SIZE * SIZE {
            self.depths.insert(key, Arc::new(depths));
        }
    }

    /// Настоящие высоты в точках без масштаба, с дном моря: где высота
    /// тайла на уровне моря или ниже, а у тайла есть глубины, берётся
    /// глубина. Свой рельеф поверх моря остаётся своим.
    pub fn floor_heights_at(&self, points: &[(f64, f64)]) -> Vec<f64> {
        points
            .iter()
            .map(|(lat, lon)| {
                let (u, v) = mercator_share(*lat, *lon);
                for zh in (0..=DEEP_LEVEL).rev() {
                    let n = 1u32 << zh;
                    let nf = f64::from(n);
                    let key = (zh, ((u * nf) as u32).min(n - 1), ((v * nf) as u32).min(n - 1));
                    let Some(tile) = self.tiles.get(&key) else {
                        continue;
                    };
                    let h = sample(tile, u, v);
                    return match self.depths.get(&key) {
                        Some(d) if h <= 0.0 => sample_grid(key, &|row, col| f64::from(d.get(row * SIZE + col).copied().unwrap_or(0)), u, v).min(0.0),
                        _ => h,
                    };
                }
                0.0
            })
            .collect()
    }

    /// Высота рельефа в точке по самому точному готовому тайлу,
    /// с масштабом.
    pub fn height_at(&self, lat: f64, lon: f64) -> f64 {
        if self.scale == 0.0 {
            return 0.0;
        }
        self.raw_height(lat, lon) * self.scale
    }

    /// Высоты в наборе точек (широта, долгота). scaled = false -
    /// настоящие высоты без масштаба, они нужны и при выключенном
    /// рельефе.
    pub fn heights_at(&self, points: &[(f64, f64)], scaled: bool) -> Vec<f64> {
        if scaled && self.scale == 0.0 {
            return vec![0.0; points.len()];
        }
        let k = if scaled { self.scale } else { 1.0 };
        points.iter().map(|(lat, lon)| self.raw_height(*lat, *lon) * k).collect()
    }

    /// Годен ли размах ключа, посчитанный при seen добавленных тайлах:
    /// после него не пришёл тайл высот над ключом.
    fn span_fresh(&self, key: TileKey, seen: usize) -> bool {
        let new = self.adds.saturating_sub(seen);
        if new == 0 {
            return true;
        }
        let log = &self.added_log;
        if new > Self::LOG_CHECK || new > log.len() {
            return false;
        }
        let (z, x, y) = key;
        !log[log.len() - new..].iter().any(|&(hz, hx, hy)| {
            z.checked_sub(hz).is_some_and(|shift| shift < 32 && x >> shift == hx && y >> shift == hy)
        })
    }

    /// Наименьшая и наибольшая высота для тайла подложки key, с масштабом.
    ///
    /// Берутся из блока пикселей готового тайла высот, который тайл
    /// подложки накрывает. Блок расширен на пиксель с каждой стороны:
    /// выборка билинейная и берёт соседние пиксели. Без данных - (0, 0).
    ///
    /// Размах помнится по ключу и годен, пока над ключом не пришёл новый
    /// тайл высот. Проверка смотрит только тайлы, пришедшие после
    /// подсчёта, не больше [`HeightStore::LOG_CHECK`], иначе размах
    /// считается заново. Смена врезок или предельного уровня забывает всё.
    pub fn range_for(&self, key: TileKey) -> (f64, f64) {
        if self.scale == 0.0 {
            return (0.0, 0.0);
        }
        let stamp = (self.deep.clone(), self.max_level);
        let remembered = {
            let mut r = self.ranges();
            if r.spans_for.as_ref() != Some(&stamp) || r.spans.len() > Self::RANGE_CACHE {
                r.spans.clear();
                r.spans_for = Some(stamp);
            }
            r.spans.get(&key).copied()
        };
        let span = match remembered {
            Some((span, seen)) if self.span_fresh(key, seen) => span,
            _ => self.best(key).map_or((0.0, 0.0), |tile| self.range(key, &tile)),
        };
        self.ranges().spans.insert(key, (span, self.adds));
        (span.0 * self.scale, span.1 * self.scale)
    }

    /// Пересчётов размаха всего.
    pub fn range_computations(&self) -> usize {
        self.ranges().computed
    }

    /// Размах высот тайла подложки key по тайлу высот, без масштаба.
    fn range(&self, key: TileKey, tile: &Arc<HeightTile>) -> (f64, f64) {
        let mut r = self.ranges();
        if let Some((t, result)) = r.ranges.get(&key)
            && Arc::ptr_eq(t, tile)
        {
            return *result;
        }
        r.computed += 1;
        let (z, x, y) = key;
        let shift = z.saturating_sub(tile.z);
        let whole = (f64::from(tile.low), f64::from(tile.high));
        // Тайл высот - предок ключа, иначе берётся его размах целиком.
        let offsets = (u64::from(x).checked_sub(u64::from(tile.x) << shift.min(31)))
            .zip(u64::from(y).checked_sub(u64::from(tile.y) << shift.min(31)));
        let result = if shift == 0 || shift >= 32 {
            whole
        } else if let Some((dx, dy)) = offsets {
            let size = if shift < 8 { SIZE >> shift } else { 1 };
            let span = 1u64 << shift;
            let col = (dx * SIZE as u64 / span).min(SIZE as u64 - 1) as usize;
            let row = (dy * SIZE as u64 / span).min(SIZE as u64 - 1) as usize;
            let (c0, c1) = (col.saturating_sub(1), (col + size + 1).min(SIZE));
            let (r0, r1) = (row.saturating_sub(1), (row + size + 1).min(SIZE));
            let mut low = f64::INFINITY;
            let mut high = f64::NEG_INFINITY;
            for rr in r0..r1 {
                for cc in c0..c1 {
                    let h = tile.at(rr, cc);
                    low = low.min(h);
                    high = high.max(h);
                }
            }
            if low > high { whole } else { (low, high) }
        } else {
            whole
        };
        if r.ranges.len() > Self::RANGE_CACHE {
            r.ranges.clear();
        }
        r.ranges.insert(key, (tile.clone(), result));
        result
    }
}
