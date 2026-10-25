// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Облако точек LAS, LAZ или COPC из байтов файла, через terrio.
//!
//! Точки пересчитываются в широту и долготу WGS84, высота - в метры
//! (в системе облака футы - умножение на длину фута). Цвет - каналы
//! red, green, blue файла, без них - по классу точки, как раскраска
//! классов ASPRS. Облако COPC читается с уровней дерева, на которых
//! точек не больше заданного количества, прочие - каждая n-я точка.
//!
//! Облако COPC по сети читает [`SparseCloud`] частями файла - вызывающий
//! сам запрашивает недостающие куски (запросы HTTP Range) и повторяет
//! чтение, главный поток не ждёт сети, в браузере тоже.

use terrio::crs::{SpatialRef, Transformer};
use std::sync::{Mutex, PoisonError};

use terrio::pointcloud::{self, PointCloudReader, PointReadOptions};
use terrio::sparse::SparseFile;

/// Облако в памяти.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cloud {
    /// Точки: широта, долгота, высота в метрах.
    pub points: Vec<[f64; 3]>,
    /// Цвета точек RGB.
    pub colors: Vec<[u8; 3]>,
    /// Точек в файле.
    pub total: u64,
    /// Код EPSG горизонтальной системы, если он известен.
    pub epsg: Option<u32>,
}

/// Цвет класса ASPRS: земля, растительность, здания, вода и прочее.
pub fn class_color(class: u8) -> [u8; 3] {
    match class {
        2 => [166, 122, 76],
        3 => [140, 200, 90],
        4 => [80, 170, 60],
        5 => [30, 120, 40],
        6 => [220, 90, 70],
        9 => [70, 130, 220],
        17 => [200, 200, 200],
        _ => [190, 190, 190],
    }
}

/// Метров в единице высоты системы crs: фут - 0.3048, американский
/// фут - 1200/3937, прочее - 1.
fn vertical_unit(crs: &SpatialRef) -> f64 {
    let wkt = crs.to_wkt().unwrap_or_default().to_lowercase();
    if wkt.contains("us survey foot") || wkt.contains("foot_us") {
        1200.0 / 3937.0
    } else if wkt.contains("foot") || wkt.contains("feet") {
        0.3048
    } else {
        1.0
    }
}

/// Облако из байтов файла name, не больше max_points точек.
pub fn read(bytes: Vec<u8>, name: &str, max_points: u64) -> Result<Cloud, String> {
    let mut reader = pointcloud::open_points_bytes(bytes, name).map_err(|e| e.to_string())?;
    from_reader(&mut reader, max_points)
}

/// Облако из открытого файла, не больше max_points точек. У COPC
/// читаются верхние уровни дерева, у прочих - каждая n-я точка.
fn from_reader(reader: &mut PointCloudReader, max_points: u64) -> Result<Cloud, String> {
    let total = reader.header().point_count;
    let options = PointReadOptions { max_points: Some(max_points), ..Default::default() };
    let pc = reader.read(&options).map_err(|e| e.to_string())?;
    let crs = pc.crs.clone();
    if crs.is_unknown() {
        return Err("у облака нет системы координат".into());
    }
    let wgs = SpatialRef::from_epsg(4326).map_err(|e| e.to_string())?;
    let to_wgs = Transformer::new(&crs, &wgs).map_err(|e| e.to_string())?;
    let unit = vertical_unit(&crs);
    let (Some(xs), Some(ys), Some(zs)) = (pc.coords("x"), pc.coords("y"), pc.coords("z")) else {
        return Err("в облаке нет координат".into());
    };
    // Каналы цвета и класса - целые, читаются числами.
    let red = pc.dim("red").map(|d| d.to_f64());
    let green = pc.dim("green").map(|d| d.to_f64());
    let blue = pc.dim("blue").map(|d| d.to_f64());
    let class_values = pc.dim("classification").map(|d| d.to_f64());
    let rgb = match (&red, &green, &blue) {
        (Some(r), Some(g), Some(b)) => Some((r.as_slice(), g.as_slice(), b.as_slice())),
        _ => None,
    };
    // Каналы цвета LAS - 16 бит, у некоторых файлов - 8.
    let scale = rgb.map_or(1.0, |(r, g, b)| {
        let top = r.iter().chain(g).chain(b).fold(0.0f64, |m, v| m.max(*v));
        if top > 255.0 { 257.0 } else { 1.0 }
    });
    let classes = class_values.as_deref();
    let mut points = Vec::with_capacity(xs.len());
    let mut colors = Vec::with_capacity(xs.len());
    for i in 0..xs.len() {
        let Ok((lon, lat)) = to_wgs.transform(xs[i], ys[i]) else {
            continue;
        };
        if !(lat.is_finite() && lon.is_finite() && zs[i].is_finite()) {
            continue;
        }
        points.push([lat, lon, zs[i] * unit]);
        let color = match rgb {
            Some((r, g, b)) => [r[i], g[i], b[i]].map(|v| (v / scale).clamp(0.0, 255.0) as u8),
            None => class_color(classes.and_then(|c| c.get(i)).map_or(0, |v| *v as u8)),
        };
        colors.push(color);
    }
    let epsg = crs.authority().filter(|(a, _)| a.eq_ignore_ascii_case("EPSG")).map(|(_, c)| c);
    Ok(Cloud { points, colors, total, epsg })
}

/// Длина файла неизвестной длины для чтения частями, 1 ТиБ. Конец файла
/// узнаётся по короткому ответу на запрос куска.
const UNKNOWN_LEN: u64 = 1 << 40;

/// Наибольшее количество повторов чтения. COPC читается за несколько
/// кругов - заголовок, записи EVLR, страницы иерархии, узлы дерева.
const MAX_ROUNDS: usize = 64;

/// Чтение облака COPC по сети частями файла.
///
/// Порядок работы - [`SparseCloud::read`], при `Ok(None)` - запрос
/// кусков [`SparseCloud::missing`], [`SparseCloud::insert`] каждого
/// ответа и снова `read`. Длина файла может быть неизвестна - тогда
/// короткий ответ на кусок у конца файла дополняется нулями, за концом
/// файла правильный COPC ничего не читает.
#[derive(Debug)]
pub struct SparseCloud {
    file: SparseFile,
    name: String,
    max_points: usize,
    /// Длина файла известна.
    exact: bool,
    /// Открытый файл между повторами - заголовок и записи VLR прочитаны.
    reader: Option<PointCloudReader>,
    /// Куски, отданные на запрос и ещё не вставленные.
    asked: Mutex<Vec<(u64, u64)>>,
    rounds: usize,
}

impl SparseCloud {
    /// Чтение файла name длиной len байт, None - длина неизвестна.
    /// В облаке будет около max_points точек - верхние уровни дерева.
    pub fn new(name: &str, len: Option<u64>, max_points: usize) -> Self {
        Self {
            file: SparseFile::new(len.unwrap_or(UNKNOWN_LEN)),
            name: name.to_string(),
            max_points,
            exact: len.is_some(),
            reader: None,
            asked: Mutex::new(Vec::new()),
            rounds: 0,
        }
    }

    /// Попытка чтения. `Ok(Some(cloud))` - облако готово. `Ok(None)` -
    /// не хватает байтов - их список даёт [`SparseCloud::missing`],
    /// после [`SparseCloud::insert`] чтение повторяется. `Err` - файл
    /// не читается.
    pub fn read(&mut self) -> Result<Option<Cloud>, String> {
        self.rounds += 1;
        if self.rounds > MAX_ROUNDS {
            return Err(format!("облако {} не прочитано за {MAX_ROUNDS} запросов", self.name));
        }
        if self.reader.is_none() {
            match pointcloud::open_points_sparse(self.file.clone(), &self.name) {
                Ok(r) if !self.file.has_missing() => self.reader = Some(r),
                Ok(_) => return Ok(None),
                Err(_) if self.file.has_missing() => return Ok(None),
                Err(e) => return Err(e.to_string()),
            }
        }
        let Some(reader) = self.reader.as_mut() else {
            return Ok(None);
        };
        let max = u64::try_from(self.max_points).unwrap_or(u64::MAX);
        match from_reader(reader, max) {
            Ok(_) | Err(_) if self.file.has_missing() => Ok(None),
            Ok(cloud) => Ok(Some(cloud)),
            Err(e) => Err(e),
        }
    }

    /// Куски файла для запроса сейчас - начало и длина в байтах,
    /// соседние слиты.
    pub fn missing(&self) -> Vec<(u64, u64)> {
        let ranges = self.file.missing();
        let mut asked = self.asked.lock().unwrap_or_else(PoisonError::into_inner);
        asked.extend(ranges.iter().copied());
        ranges
    }

    /// Ответ на запрос куска, начиная с байта start. Короткий ответ при
    /// неизвестной длине - конец файла. Ответ длиннее запрошенного
    /// не с начала файла - сервер не понял Range и прислал файл целиком.
    pub fn insert(&mut self, start: u64, bytes: &[u8]) {
        let len = bytes.len() as u64;
        let asked = {
            let mut asked = self.asked.lock().unwrap_or_else(PoisonError::into_inner);
            asked.iter().position(|(s, _)| *s == start).map(|i| asked.swap_remove(i).1)
        };
        match asked {
            Some(want) if len > want && start > 0 => {
                // Весь файл: длина теперь известна.
                self.file = SparseFile::new(len);
                self.file.insert(0, bytes);
                self.exact = true;
                self.reader = None;
            }
            Some(want) if len < want && !self.exact => {
                let mut padded = bytes.to_vec();
                padded.resize(usize::try_from(want).unwrap_or(bytes.len()), 0);
                self.file.insert(start, &padded);
            }
            _ => self.file.insert(start, bytes),
        }
    }

    /// Получено байтов и ответов вставлено.
    pub fn fetched(&self) -> (u64, u64) {
        self.file.fetched()
    }
}