// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Изменения местности и классификация по эмбеддингам AlphaEarth
//! Foundations, перенос `core/aef.py` модуля без сети.
//!
//! Просьба автора от 28 октября 2026 года по образцу «Обнаружить
//! изменение» и «Классификации» Google Earth. Набор AlphaEarth Foundations
//! Satellite Embedding (Google, Google DeepMind, CC BY 4.0) - на каждый
//! год 2017-2025 у клетки 10 м вектор из 64 чисел, сводка снимков и радара
//! за год. Степень изменения между двумя годами - несходство векторов,
//! 1 - косинус угла между ними: у неизменённой местности около 0.05,
//! у стройки, вырубки, нового карьера - 0.5 и выше.
//!
//! Файлы - COG в хранилище Google Cloud без учётной записи: по зонам UTM
//! квадраты 81 920 м (8192 пикселя), 64 канала Int8 плоскостями, сжатие
//! ZSTD, строки с юга на север. Значение v - вектор sign(v) · (v / 127.5)²,
//! -128 - нет данных. Указатель файлов собран по зонам в
//! planetx-terrain/aef: квадраты зоны с именами файлов по годам.
//!
//! У Lite - только подробный участок окна: блоки обзора файла
//! 1024 × 1024 пикселя, по два года на блок. Обзора по всему виду, как
//! у модуля, у Lite нет: он просил бы сотни заголовков файлов по сети.
//! Каналы блока читает интерфейс ([`crate::cog`]), степень изменения
//! копится по каналу ([`ChangeSum`]), 64 плоскости в памяти не держатся.

use std::collections::BTreeMap;

/// Годы набора.
pub const YEARS: [i32; 9] = [2017, 2018, 2019, 2020, 2021, 2022, 2023, 2024, 2025];
/// Начало адреса файлов.
pub const BASE: &str = "https://storage.googleapis.com/alphaearth_foundations/satellite_embedding/v1/annual/";
/// Начало адреса указателей зон, `INDEX_BASE` модуля.
pub const INDEX_BASE: &str = "https://raw.githubusercontent.com/Valery35/planetx-terrain/main/";
/// Подпись по условиям набора (CC BY 4.0), её просит Google.
pub const CREDIT: &str = "The AlphaEarth Foundations Satellite Embedding dataset is produced by Google and Google DeepMind.";
pub const ATTRIBUTION: &str = "AlphaEarth Foundations, Google, Google DeepMind";
/// Сторона квадрата файла, м.
pub const CELL: f64 = 81920.0;
/// Пикселей полного разрешения на сторону файла.
pub const FILE_SIDE: u32 = 8192;
/// Пикселей тайла файла на всех обзорах.
pub const TILE: u32 = 1024;
/// Метров в пикселе полного разрешения.
pub const PIXEL: f64 = 10.0;
/// Каналов вектора.
pub const BANDS: u16 = 64;
pub const NODATA: i8 = -128;
/// Нет данных в байтах степени изменения.
pub const EMPTY: u8 = 255;
/// Наибольшая сторона подробного участка, м.
pub const DETAIL_SIDE: f64 = 20000.0;
/// Подробность участка: обзор и пиксель, м.
pub const DETAIL_LEVELS: [(usize, f64); 4] = [(0, 10.0), (1, 20.0), (2, 40.0), (3, 80.0)];
/// Сжатый блок 1024 × 1024 всех 64 каналов за год, МБ, по замеру
/// модуля 28 октября 2026 года у файла зоны 10N.
pub const BLOCK_MB: [f64; 4] = [42.0, 45.0, 50.0, 52.0];
/// Пиксель «Авто»: клеток участка от AUTO_FEW до AUTO_CELLS.
pub const AUTO_CELLS: f64 = 800.0 * 800.0;
pub const AUTO_FEW: f64 = 120.0 * 120.0;
/// Классификация: ближайших контрольных точек голосуют, наименьшее
/// количество точек класса и классов, как у Google Earth.
pub const KNN: usize = 5;
pub const MIN_POINTS: usize = 3;
pub const MIN_CLASSES: usize = 2;
/// Шкала - от чёрного к жёлтому, как у Google Earth.
pub const STOPS: [(f64, [u8; 3]); 4] = [(0.0, [0, 0, 0]), (0.35, [70, 60, 10]), (0.65, [190, 170, 20]), (1.0, [255, 245, 30])];
pub const OPACITY: f64 = 0.8;

// UTM, ряды Крюгера `core/coords.py` модуля.
const K0: f64 = 0.9996;
const FALSE_EASTING: f64 = 500000.0;
const FALSE_NORTHING: f64 = 10_000_000.0;
const A: f64 = 6378137.0;
const F: f64 = 1.0 / 298.257223563;

/// Постоянные рядов: n, e, A·, alpha, beta.
fn series() -> (f64, f64, f64, [f64; 6], [f64; 6]) {
    let n = F / (2.0 - F);
    let e = (F * (2.0 - F)).sqrt();
    let (n2, n3, n4, n5, n6) = (n * n, n.powi(3), n.powi(4), n.powi(5), n.powi(6));
    let ar = A / (1.0 + n) * (1.0 + n2 / 4.0 + n4 / 64.0 + n6 / 256.0);
    let alpha = [
        n / 2.0 - 2.0 * n2 / 3.0 + 5.0 * n3 / 16.0 + 41.0 * n4 / 180.0 - 127.0 * n5 / 288.0 + 7891.0 * n6 / 37800.0,
        13.0 * n2 / 48.0 - 3.0 * n3 / 5.0 + 557.0 * n4 / 1440.0 + 281.0 * n5 / 630.0 - 1983433.0 * n6 / 1935360.0,
        61.0 * n3 / 240.0 - 103.0 * n4 / 140.0 + 15061.0 * n5 / 26880.0 + 167603.0 * n6 / 181440.0,
        49561.0 * n4 / 161280.0 - 179.0 * n5 / 168.0 + 6601661.0 * n6 / 7257600.0,
        34729.0 * n5 / 80640.0 - 3418889.0 * n6 / 1995840.0,
        212378941.0 * n6 / 319334400.0,
    ];
    let beta = [
        n / 2.0 - 2.0 * n2 / 3.0 + 37.0 * n3 / 96.0 - n4 / 360.0 - 81.0 * n5 / 512.0 + 96199.0 * n6 / 604800.0,
        n2 / 48.0 + n3 / 15.0 - 437.0 * n4 / 1440.0 + 46.0 * n5 / 105.0 - 1118711.0 * n6 / 3870720.0,
        17.0 * n3 / 480.0 - 37.0 * n4 / 840.0 - 209.0 * n5 / 4480.0 + 5569.0 * n6 / 90720.0,
        4397.0 * n4 / 161280.0 - 11.0 * n5 / 504.0 - 830251.0 * n6 / 7257600.0,
        4583.0 * n5 / 161280.0 - 108847.0 * n6 / 3991680.0,
        20648693.0 * n6 / 638668800.0,
    ];
    (n, e, ar, alpha, beta)
}

/// Восток и север точки в метрах зоны zone, `utm_arrays` модуля. south -
/// северу прибавляется ложный север, как в системах EPSG 327xx.
pub fn to_utm(lat: f64, lon: f64, zone: u32, south: bool) -> (f64, f64) {
    let (_, e, ar, alpha, _) = series();
    let central = (f64::from(zone) - 1.0) * 6.0 - 180.0 + 3.0;
    let dlon = ((lon - central + 180.0).rem_euclid(360.0) - 180.0).to_radians();
    let tau = lat.to_radians().tan();
    let sigma = (e * (e * tau / 1f64.hypot(tau)).atanh()).sinh();
    let tp = tau * 1f64.hypot(sigma) - sigma * 1f64.hypot(tau);
    let xi_p = tp.atan2(dlon.cos());
    let eta_p = (dlon.sin() / tp.hypot(dlon.cos())).asinh();
    let (mut xi, mut eta) = (xi_p, eta_p);
    for (j, a) in alpha.iter().enumerate() {
        let k = 2.0 * (j as f64 + 1.0);
        xi += a * (k * xi_p).sin() * (k * eta_p).cosh();
        eta += a * (k * xi_p).cos() * (k * eta_p).sinh();
    }
    (FALSE_EASTING + K0 * ar * eta, K0 * ar * xi + if south { FALSE_NORTHING } else { 0.0 })
}

/// Широта и долгота точки UTM, `from_utm` модуля.
pub fn from_utm(zone: u32, south: bool, easting: f64, northing: f64) -> (f64, f64) {
    let (_, e, ar, _, beta) = series();
    let xi = (northing - if south { FALSE_NORTHING } else { 0.0 }) / (K0 * ar);
    let eta = (easting - FALSE_EASTING) / (K0 * ar);
    let (mut xi_p, mut eta_p) = (xi, eta);
    for (j, b) in beta.iter().enumerate() {
        let k = 2.0 * (j as f64 + 1.0);
        xi_p -= b * (k * xi).sin() * (k * eta).cosh();
        eta_p -= b * (k * xi).cos() * (k * eta).sinh();
    }
    let tp = xi_p.sin() / eta_p.sinh().hypot(xi_p.cos());
    let dlon = eta_p.sinh().atan2(xi_p.cos());
    let e2 = e * e;
    let mut tau = tp;
    for _ in 0..10 {
        let sigma = (e * (e * tau / 1f64.hypot(tau)).atanh()).sinh();
        let tpi = tau * 1f64.hypot(sigma) - sigma * 1f64.hypot(tau);
        let delta = (tp - tpi) / 1f64.hypot(tpi) * (1.0 + (1.0 - e2) * tau * tau) / ((1.0 - e2) * 1f64.hypot(tau));
        tau += delta;
        if delta.abs() < 1e-14 {
            break;
        }
    }
    let central = (f64::from(zone) - 1.0) * 6.0 - 180.0 + 3.0;
    (tau.atan().to_degrees(), (central + dlon.to_degrees() + 180.0).rem_euclid(360.0) - 180.0)
}

/// Значение квантованного вектора, `LUT` модуля: sign(v) · (v / 127.5)²,
/// [`NODATA`] - нечисло.
pub fn dequantize(v: i8) -> f32 {
    if v == NODATA {
        return f32::NAN;
    }
    let x = f32::from(v) / 127.5;
    (x * x).copysign(f32::from(v))
}

/// Адрес файла года: name - «образ:строка:столбец» указателя, строка
/// и столбец в долях 8192 пикселей образа Earth Engine.
pub fn file_url(year: i32, zone: &str, name: &str) -> Option<String> {
    let mut parts = name.split(':');
    let (image, row, col) = (parts.next()?, parts.next()?.parse::<u64>().ok()?, parts.next()?.parse::<u64>().ok()?);
    Some(format!("{BASE}{year}/{zone}/{image}-{:010}-{:010}.tiff", row * u64::from(FILE_SIDE), col * u64::from(FILE_SIDE)))
}

/// Адрес указателя зоны.
pub fn index_url(zone: &str) -> String {
    format!("{INDEX_BASE}aef/{zone}.json")
}

/// Зона указателя для точки: номер 1-60 по долготе и полушарие.
pub fn zone_of(lon: f64, lat: f64) -> String {
    let number = ((lon + 180.0) / 6.0).floor().clamp(0.0, 59.0) as u32 + 1;
    format!("{number}{}", if lat >= 0.0 { "N" } else { "S" })
}

/// Квадраты одной зоны UTM: западный и северный край, имена файлов по
/// годам.
#[derive(Debug, Clone, PartialEq)]
pub struct ZoneIndex {
    pub zone: String,
    pub number: u32,
    pub south: bool,
    pub cells: Vec<(f64, f64, BTreeMap<i32, String>)>,
}

impl ZoneIndex {
    /// Указатель из байтов JSON файла planetx-terrain/aef. None - файл
    /// не того вида.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let doc: serde_json::Value = serde_json::from_slice(bytes).ok()?;
        let zone = doc.get("zone")?.as_str()?.to_string();
        let number: u32 = zone.get(..zone.len().checked_sub(1)?)?.parse().ok().filter(|n| (1..=60).contains(n))?;
        let south = zone.ends_with('S');
        let mut cells = Vec::new();
        for cell in doc.get("cells")?.as_array()? {
            let (Some(west), Some(north), Some(names)) = (cell.get(0).and_then(serde_json::Value::as_f64), cell.get(1).and_then(serde_json::Value::as_f64), cell.get(2).and_then(serde_json::Value::as_object)) else {
                continue;
            };
            let names = names.iter().filter_map(|(y, n)| Some((y.parse::<i32>().ok()?, n.as_str()?.to_string()))).collect();
            cells.push((west, north, names));
        }
        Some(Self { zone, number, south, cells })
    }

    /// Точка в метры UTM зоны.
    pub fn project(&self, lat: f64, lon: f64) -> (f64, f64) {
        to_utm(lat, lon, self.number, self.south)
    }

    /// Номера квадратов, задевающих рамку UTM.
    pub fn cells_in(&self, west: f64, south: f64, east: f64, north: f64) -> Vec<usize> {
        self.cells.iter().enumerate().filter(|(_, c)| c.0 < east && c.0 + CELL > west && c.1 > south && c.1 - CELL < north).map(|(i, _)| i).collect()
    }

    /// Имя файла квадрата cell за год.
    pub fn name(&self, cell: usize, year: i32) -> Option<&str> {
        self.cells.get(cell)?.2.get(&year).map(String::as_str)
    }

    /// Рамка UTM точек рамки box (запад, юг, восток, север) в градусах.
    fn utm_box(&self, bbox: (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
        let (west, south, east, north) = bbox;
        let pts = [(south, west), (south, east), (north, west), (north, east), ((south + north) / 2.0, (west + east) / 2.0), ((south + north) / 2.0, west), ((south + north) / 2.0, east)];
        let xy: Vec<(f64, f64)> = pts.iter().map(|&(la, lo)| self.project(la, lo)).collect();
        let fold = |f: fn(f64, f64) -> f64, init: f64, pick: fn(&(f64, f64)) -> f64| xy.iter().map(pick).fold(init, f);
        (fold(f64::min, f64::INFINITY, |p| p.0), fold(f64::min, f64::INFINITY, |p| p.1), fold(f64::max, f64::NEG_INFINITY, |p| p.0), fold(f64::max, f64::NEG_INFINITY, |p| p.1))
    }
}

/// Блок подробного участка: квадрат, столбец и строка блока обзора.
pub type Part = (usize, u32, u32);

/// Блоки обзора level подробного участка в зоне index, `detail_parts`
/// модуля. box - рамка участка в градусах (запад, юг, восток, север).
/// Строки файлов идут с юга, как у AlphaEarth.
pub fn detail_parts(index: &ZoneIndex, bbox: (f64, f64, f64, f64), level: usize) -> Vec<Part> {
    let (x0, y0, x1, y1) = index.utm_box(bbox);
    let block = f64::from(TILE) * PIXEL * f64::from(1u32 << level.min(13));
    let last = f64::from(((FILE_SIDE / TILE) >> level.min(13)).max(1) - 1);
    let mut out = Vec::new();
    for cell in index.cells_in(x0, y0, x1, y1) {
        let Some((cw, cn, _)) = index.cells.get(cell) else {
            continue;
        };
        let base = cn - CELL;
        let c0 = ((x0 - cw) / block).floor().max(0.0);
        let c1 = ((x1 - cw) / block).floor().min(last);
        let r0 = ((y0 - base) / block).floor().max(0.0);
        let r1 = ((y1 - base) / block).floor().min(last);
        if !(c0 <= c1 && r0 <= r1) {
            continue;
        }
        for col in c0 as u32..=c1 as u32 {
            for row in r0 as u32..=r1 as u32 {
                out.push((cell, col, row));
            }
        }
    }
    out
}

/// Оценка загрузки подробного участка, МБ: два года на блок.
pub fn detail_megabytes(parts: usize, level: usize) -> f64 {
    2.0 * parts as f64 * BLOCK_MB.get(level).copied().unwrap_or(BLOCK_MB[3])
}

/// Уровень пикселя «Авто» для участка box со стороной side метров,
/// `auto_level` модуля: из уровней с клетками участка от AUTO_FEW до
/// AUTO_CELLS - с наименьшей загрузкой, при равной - мельче.
pub fn auto_level(index: &ZoneIndex, bbox: (f64, f64, f64, f64), side: f64) -> usize {
    let mut best: Option<(f64, usize)> = None;
    for (level, pixel) in DETAIL_LEVELS {
        let cells = (side / pixel).powi(2);
        if cells > AUTO_CELLS {
            continue;
        }
        if cells < AUTO_FEW && best.is_some() {
            break;
        }
        let mb = detail_megabytes(detail_parts(index, bbox, level).len(), level);
        if best.is_none_or(|b| mb < b.0) {
            best = Some((mb, level));
        }
    }
    best.map_or(DETAIL_LEVELS[3].0, |b| b.1)
}

/// Степень изменения блока, копится по каналу: сумма произведений
/// и квадратов значений двух лет, `change` модуля по частям.
#[derive(Debug, Clone, PartialEq)]
pub struct ChangeSum {
    dot: Vec<f32>,
    na: Vec<f32>,
    nb: Vec<f32>,
    bad: Vec<bool>,
}

impl ChangeSum {
    pub fn new(pixels: usize) -> Self {
        Self { dot: vec![0.0; pixels], na: vec![0.0; pixels], nb: vec![0.0; pixels], bad: vec![false; pixels] }
    }

    /// Канал двух лет: сырые значения Int8 пикселей в одном порядке.
    pub fn add(&mut self, a: &[i8], b: &[i8]) {
        for (i, (va, vb)) in a.iter().zip(b).enumerate() {
            let (x, y) = (dequantize(*va), dequantize(*vb));
            let (Some(d), Some(na), Some(nb), Some(bad)) = (self.dot.get_mut(i), self.na.get_mut(i), self.nb.get_mut(i), self.bad.get_mut(i)) else {
                break;
            };
            if x.is_nan() || y.is_nan() {
                *bad = true;
                continue;
            }
            *d += x * y;
            *na += x * x;
            *nb += y * y;
        }
    }

    /// Степень 0-1 в байтах 0-254, [`EMPTY`] - нет данных: у вектора без
    /// данных или почти нулевой длины.
    pub fn finish(&self) -> Vec<u8> {
        self.dot
            .iter()
            .zip(&self.na)
            .zip(&self.nb)
            .zip(&self.bad)
            .map(|(((d, na), nb), bad)| {
                let norm = (na * nb).sqrt();
                if *bad || norm.is_nan() || norm <= 0.25 {
                    return EMPTY;
                }
                to_byte(f64::from(1.0 - d / norm))
            })
            .collect()
    }
}

/// Степень 0-1 в байт 0-254, нечисло - [`EMPTY`].
pub fn to_byte(v: f64) -> u8 {
    if v.is_finite() { (v.clamp(0.0, 1.0) * 254.0).round() as u8 } else { EMPTY }
}

/// Байт степени в 0-1, [`EMPTY`] - нечисло.
pub fn from_byte(v: u8) -> f64 {
    if v == EMPTY { f64::NAN } else { f64::from(v) / 254.0 }
}

/// Кусок участка в метрах UTM: угол (x0, y0) блока и шаги пикселя со
/// знаком - у файлов AlphaEarth строки идут с юга, шаг по y больше нуля.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece<T> {
    pub x0: f64,
    pub y0: f64,
    pub dx: f64,
    pub dy: f64,
    pub rows: usize,
    pub cols: usize,
    /// Значения строка за строкой, по каналу на пиксель или [`BANDS`].
    pub values: Vec<T>,
}

/// Сетка подробного участка на сетке файлов зоны: левый верхний угол,
/// шаг, строки с севера.
#[derive(Debug, Clone, PartialEq)]
pub struct Mosaic<T> {
    pub x0: f64,
    pub y0: f64,
    pub step: f64,
    pub rows: usize,
    pub cols: usize,
    /// Каналов на клетку: 1 у степени, [`BANDS`] у векторов.
    pub depth: usize,
    pub values: Vec<T>,
}

impl<T: Copy> Mosaic<T> {
    /// Строка и столбец клетки с точкой UTM, None - вне сетки.
    pub fn cell_of(&self, x: f64, y: f64) -> Option<(usize, usize)> {
        let c = ((x - self.x0) / self.step).floor();
        let r = ((self.y0 - y) / self.step).floor();
        (c >= 0.0 && r >= 0.0 && (c as usize) < self.cols && (r as usize) < self.rows).then_some((r as usize, c as usize))
    }

    /// Значения клетки (строка, столбец).
    pub fn at(&self, row: usize, col: usize) -> Option<&[T]> {
        let i = (row * self.cols + col) * self.depth;
        self.values.get(i..i + self.depth)
    }
}

/// Куски на одной сетке участка box, `mosaic` и `vectors` модуля:
/// квадраты одной зоны лежат на одной сетке 10 м, куски складываются без
/// пересчёта. empty - значение «нет данных», depth - каналов на пиксель.
/// None - кусков нет или сетка не годится.
pub fn mosaic<T: Copy + PartialEq>(pieces: &[Piece<T>], index: &ZoneIndex, bbox: (f64, f64, f64, f64), depth: usize, empty: T) -> Option<Mosaic<T>> {
    let first = pieces.first()?;
    let step = first.dx.abs();
    if !(step.is_finite() && step > 0.0) || depth == 0 {
        return None;
    }
    let (xa, ya, xb, yb) = index.utm_box(bbox);
    let x0 = (xa / step).floor() * step;
    let y1 = (yb / step).ceil() * step;
    let cols = ((xb - x0) / step).ceil().max(1.0) as usize;
    let rows = ((y1 - ya) / step).ceil().max(1.0) as usize;
    if cols.checked_mul(rows)?.checked_mul(depth)? > 1 << 28 {
        return None;
    }
    let mut values = vec![empty; rows * cols * depth];
    for p in pieces {
        if p.dx == 0.0 || p.dy == 0.0 {
            continue;
        }
        for r in 0..rows {
            let cy = y1 - (r as f64 + 0.5) * step;
            let pr = ((cy - p.y0) / p.dy).floor();
            if !(pr >= 0.0 && (pr as usize) < p.rows) {
                continue;
            }
            for c in 0..cols {
                let cx = x0 + (c as f64 + 0.5) * step;
                let pc = ((cx - p.x0) / p.dx).floor();
                if !(pc >= 0.0 && (pc as usize) < p.cols) {
                    continue;
                }
                let src = (pr as usize * p.cols + pc as usize) * depth;
                let (Some(from), dst) = (p.values.get(src..src + depth), (r * cols + c) * depth) else {
                    continue;
                };
                if from.first().is_some_and(|v| *v != empty)
                    && let Some(to) = values.get_mut(dst..dst + depth)
                {
                    to.copy_from_slice(from);
                }
            }
        }
    }
    Some(Mosaic { x0, y0: y1, step, rows, cols, depth, values })
}

/// Цвет степени v шкалы [`STOPS`] или None - вне промежутка low..high
/// и без данных, `colors` модуля.
pub fn color(v: f64, low: f64, high: f64) -> Option<[u8; 3]> {
    if !v.is_finite() || v < low - 1e-6 || v > high + 1e-6 {
        return None;
    }
    let mut out = [0u8; 3];
    for (ch, slot) in out.iter_mut().enumerate() {
        let mut value = f64::from(STOPS[3].1[ch]);
        for w in STOPS.windows(2) {
            let (a, b) = (w[0], w[1]);
            if v <= b.0 {
                let t = ((v - a.0) / (b.0 - a.0)).clamp(0.0, 1.0);
                value = f64::from(a.1[ch]) + t * (f64::from(b.1[ch]) - f64::from(a.1[ch]));
                break;
            }
        }
        *slot = value.round() as u8;
    }
    Some(out)
}

/// Места сильных изменений, `hot_spots` модуля: до count (строка,
/// столбец, степень) по сглаженной окном 3 × 3 степени, не ближе spacing
/// клеток друг к другу и не слабее low.
pub fn hot_spots(values: &[f64], rows: usize, cols: usize, count: usize, spacing: usize, low: f64) -> Vec<(usize, usize, f64)> {
    if values.len() != rows * cols || rows == 0 || cols == 0 {
        return Vec::new();
    }
    let at = |r: i64, c: i64| -> f64 {
        let (r, c) = (r.clamp(0, rows as i64 - 1) as usize, c.clamp(0, cols as i64 - 1) as usize);
        values.get(r * cols + c).copied().filter(|v| v.is_finite()).unwrap_or(0.0)
    };
    let mut smooth: Vec<f64> = (0..rows * cols)
        .map(|i| {
            let (r, c) = ((i / cols) as i64, (i % cols) as i64);
            let mut sum = 0.0;
            for dr in -1..=1 {
                for dc in -1..=1 {
                    sum += at(r + dr, c + dc);
                }
            }
            sum / 9.0
        })
        .collect();
    let mut out = Vec::new();
    while out.len() < count {
        let Some((best, value)) = smooth.iter().copied().enumerate().max_by(|a, b| a.1.total_cmp(&b.1)) else {
            break;
        };
        if value < low {
            break;
        }
        let (row, col) = (best / cols, best % cols);
        out.push((row, col, value));
        for r in row.saturating_sub(spacing)..(row + spacing + 1).min(rows) {
            for c in col.saturating_sub(spacing)..(col + spacing + 1).min(cols) {
                if let Some(v) = smooth.get_mut(r * cols + c) {
                    *v = 0.0;
                }
            }
        }
    }
    out
}

/// Вектор Int8 в единичный, None - без данных или нулевой длины.
pub fn unit(raw: &[i8]) -> Option<Vec<f32>> {
    let values: Vec<f32> = raw.iter().map(|v| dequantize(*v)).collect();
    if values.iter().any(|v| v.is_nan()) {
        return None;
    }
    let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt();
    (norm > 0.0).then(|| values.iter().map(|v| v / norm).collect())
}

/// Классы клеток по контрольным точкам, `classify` модуля: grid -
/// векторы клеток по [`BANDS`] подряд, samples - векторы точек, labels -
/// их классы. Ближайшие k точек по косинусу голосуют сходством. Итог -
/// класс клетки (None без данных) и сходство с лучшей точкой.
pub fn classify(grid: &[i8], depth: usize, samples: &[Vec<i8>], labels: &[i32], k: usize) -> Vec<Option<(i32, f32)>> {
    let train: Vec<(Vec<f32>, i32)> = samples.iter().zip(labels).filter_map(|(s, l)| unit(s).map(|u| (u, *l))).collect();
    if depth == 0 {
        return Vec::new();
    }
    let k = k.clamp(1, train.len().max(1));
    let mut classes: Vec<i32> = train.iter().map(|t| t.1).collect();
    classes.sort_unstable();
    classes.dedup();
    grid.chunks(depth)
        .map(|cell| {
            let u = unit(cell)?;
            if train.is_empty() {
                return None;
            }
            let mut sim: Vec<(f32, i32)> = train.iter().map(|(t, l)| (t.iter().zip(&u).map(|(a, b)| a * b).sum::<f32>(), *l)).collect();
            sim.sort_by(|a, b| b.0.total_cmp(&a.0));
            let best = sim.first().map_or(f32::NAN, |s| s.0);
            let mut votes = vec![0.0f32; classes.len()];
            for (s, l) in sim.iter().take(k) {
                if let Ok(i) = classes.binary_search(l)
                    && let Some(v) = votes.get_mut(i)
                {
                    *v += s.max(0.0) + 1e-6;
                }
            }
            let (i, _) = votes.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
            Some((*classes.get(i)?, best))
        })
        .collect()
}
