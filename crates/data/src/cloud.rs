// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Облако точек LAS, LAZ или COPC из байтов файла. Свой разбор без
//! terrio, разжатие LAZ - крейт `laz`.
//!
//! Точки пересчитываются в широту и долготу WGS84 ([`crate::proj`]),
//! высота - в метры (в системе облака футы - умножение на длину фута).
//! Цвет - каналы red, green, blue файла, без них - по классу точки, как
//! раскраска классов ASPRS. Облако COPC читается с уровней дерева, на
//! которых точек не больше заданного количества, прочие - каждая n-я
//! точка.
//!
//! Облако COPC по сети читает [`SparseCloud`] частями файла - вызывающий
//! сам запрашивает недостающие куски (запросы HTTP Range) и повторяет
//! чтение, главный поток не ждёт сети, в браузере тоже.

use std::collections::{BTreeMap, HashSet};
use std::io::Cursor;
use std::sync::{Mutex, PoisonError};

use laz::record::{LayeredPointRecordDecompressor, RecordDecompressor};
use laz::{LasZipDecompressor, LazVlr};

use crate::proj::{self, Crs};

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

/// Облако из байтов файла name, не больше max_points точек.
pub fn read(bytes: Vec<u8>, name: &str, max_points: u64) -> Result<Cloud, String> {
    let len = bytes.len() as u64;
    let mut file = Store::new(len);
    file.insert(0, &bytes);
    drop(bytes);
    let max = usize::try_from(max_points).unwrap_or(usize::MAX);
    match read_from(&file, max, false) {
        Ok(Some(cloud)) => Ok(cloud),
        Ok(None) => Err(format!("облако {name}: файл обрезан")),
        Err(e) => Err(format!("облако {name}: {e}")),
    }
}

// ---------------------------------------------------------------------
// Файл частями.

/// Байты файла, полученные кусками. Чтение отсутствующего куска
/// запоминает его для запроса.
#[derive(Debug, Default)]
struct Store {
    len: u64,
    /// Куски по началу, не пересекаются и не соприкасаются.
    parts: BTreeMap<u64, Vec<u8>>,
    /// Куски, которых не хватило при последнем чтении.
    wanted: Mutex<Vec<(u64, u64)>>,
    fetched: u64,
    inserts: u64,
}

impl Store {
    fn new(len: u64) -> Self {
        Self { len, ..Default::default() }
    }

    /// Вставка куска, слияние с соседними: пересекающиеся и смежные куски
    /// становятся одним.
    fn insert(&mut self, start: u64, bytes: &[u8]) {
        self.inserts += 1;
        self.fetched += bytes.len() as u64;
        if bytes.is_empty() {
            return;
        }
        let end = start.saturating_add(bytes.len() as u64);
        let touching: Vec<u64> = self.parts.range(..=end).filter(|(s, p)| **s + p.len() as u64 >= start).map(|(s, _)| *s).collect();
        let mut from = start;
        let mut to = end;
        for s in &touching {
            if let Some(p) = self.parts.get(s) {
                from = from.min(*s);
                to = to.max(*s + p.len() as u64);
            }
        }
        let Ok(size) = usize::try_from(to - from) else {
            return;
        };
        let mut data = vec![0u8; size];
        for s in touching {
            if let Some(p) = self.parts.remove(&s) {
                let at = usize::try_from(s - from).unwrap_or(0);
                data[at..at + p.len()].copy_from_slice(&p);
            }
        }
        let at = usize::try_from(start - from).unwrap_or(0);
        data[at..at + bytes.len()].copy_from_slice(bytes);
        self.parts.insert(from, data);
    }

    /// Кусок [start, start + len): Ok(Some) - есть, Ok(None) - его нет,
    /// он запомнен, Err - кусок за концом файла.
    fn get(&self, start: u64, len: u64) -> Result<Option<&[u8]>, String> {
        let end = start.checked_add(len).filter(|e| *e <= self.len).ok_or("файл обрезан")?;
        if let Some((&s, part)) = self.parts.range(..=start).next_back()
            && s + part.len() as u64 >= end
        {
            let a = usize::try_from(start - s).map_err(|e| e.to_string())?;
            let b = usize::try_from(end - s).map_err(|e| e.to_string())?;
            return Ok(part.get(a..b));
        }
        self.wanted.lock().unwrap_or_else(PoisonError::into_inner).push((start, len));
        Ok(None)
    }

    /// Запомненные куски без уже полученных, соседние слиты.
    fn missing(&self) -> Vec<(u64, u64)> {
        let mut wanted = std::mem::take(&mut *self.wanted.lock().unwrap_or_else(PoisonError::into_inner));
        wanted.sort_unstable();
        let mut out: Vec<(u64, u64)> = Vec::new();
        for (s, l) in wanted {
            match out.last_mut() {
                Some((ps, pl)) if *ps + *pl >= s => *pl = (*pl).max(s + l - *ps),
                _ => out.push((s, l)),
            }
        }
        out
    }
}

/// Нехватка байтов: чтение повторится, когда куски придут.
const NEED: &str = "\u{0}need";

/// Кусок файла или ошибка NEED.
fn need(file: &Store, start: u64, len: u64) -> Result<&[u8], String> {
    file.get(start, len)?.ok_or_else(|| NEED.to_string())
}

// ---------------------------------------------------------------------
// Разбор LAS.

fn u16_at(b: &[u8], i: usize) -> u16 {
    b.get(i..i + 2).and_then(|s| s.try_into().ok()).map_or(0, u16::from_le_bytes)
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    b.get(i..i + 4).and_then(|s| s.try_into().ok()).map_or(0, u32::from_le_bytes)
}

fn i32_at(b: &[u8], i: usize) -> i32 {
    b.get(i..i + 4).and_then(|s| s.try_into().ok()).map_or(0, i32::from_le_bytes)
}

fn u64_at(b: &[u8], i: usize) -> u64 {
    b.get(i..i + 8).and_then(|s| s.try_into().ok()).map_or(0, u64::from_le_bytes)
}

fn f64_at(b: &[u8], i: usize) -> f64 {
    b.get(i..i + 8).and_then(|s| s.try_into().ok()).map_or(f64::NAN, f64::from_le_bytes)
}

/// Заголовок LAS.
#[derive(Debug, Clone)]
struct Header {
    header_size: u64,
    point_offset: u64,
    vlr_count: u32,
    format: u8,
    compressed: bool,
    record_len: usize,
    scale: [f64; 3],
    offset: [f64; 3],
    total: u64,
    evlr_start: u64,
    evlr_count: u32,
}

const LAS_HEADER: u64 = 375;

fn header(b: &[u8]) -> Result<Header, String> {
    if b.get(0..4) != Some(b"LASF") {
        return Err("не файл LAS".into());
    }
    let (major, minor) = (b.get(24).copied().unwrap_or(0), b.get(25).copied().unwrap_or(0));
    if major != 1 || minor > 4 {
        return Err(format!("версия LAS {major}.{minor} не поддерживается"));
    }
    let raw_format = b.get(104).copied().unwrap_or(0);
    let format = raw_format & 0x3F;
    if format > 10 {
        return Err(format!("формат точек {format} не поддерживается"));
    }
    let record_len = usize::from(u16_at(b, 105));
    if record_len < min_record(format) {
        return Err("длина записи точки меньше формата".into());
    }
    let legacy = u64::from(u32_at(b, 107));
    let total = if minor >= 4 && u64_at(b, 247) > 0 { u64_at(b, 247) } else { legacy };
    let scale = [f64_at(b, 131), f64_at(b, 139), f64_at(b, 147)];
    let offset = [f64_at(b, 155), f64_at(b, 163), f64_at(b, 171)];
    if scale.iter().chain(&offset).any(|v| !v.is_finite()) || scale.contains(&0.0) {
        return Err("неверный масштаб координат".into());
    }
    let (evlr_start, evlr_count) = if minor >= 4 { (u64_at(b, 235), u32_at(b, 243)) } else { (0, 0) };
    Ok(Header {
        header_size: u64::from(u16_at(b, 94)),
        point_offset: u64::from(u32_at(b, 96)),
        vlr_count: u32_at(b, 100),
        format,
        compressed: raw_format & 0x80 != 0 || raw_format & 0x40 != 0,
        record_len,
        scale,
        offset,
        total,
        evlr_start,
        evlr_count,
    })
}

/// Наименьшая длина записи формата точек.
fn min_record(format: u8) -> usize {
    match format {
        0 => 20,
        1 => 28,
        2 => 26,
        3 => 34,
        4 => 57,
        5 => 63,
        6 => 30,
        7 => 36,
        8 => 38,
        9 => 59,
        _ => 67,
    }
}

/// Смещение класса и каналов цвета в записи формата.
fn layout(format: u8) -> (usize, u8, Option<usize>) {
    // Класс: у форматов 0-5 - младшие 5 бит байта 15, у 6-10 - байт 16.
    let rgb = match format {
        2 => Some(20),
        3 | 5 => Some(28),
        7 | 8 | 10 => Some(30),
        _ => None,
    };
    if format >= 6 { (16, 0xFF, rgb) } else { (15, 0x1F, rgb) }
}

/// Записи VLR или EVLR: (user id, record id, данные).
fn records(b: &[u8], count: u32, extended: bool) -> Vec<(String, u16, &[u8])> {
    let head = if extended { 60 } else { 54 };
    let mut out = Vec::new();
    let mut i = 0usize;
    for _ in 0..count {
        let Some(h) = b.get(i..i + head) else {
            break;
        };
        let user = String::from_utf8_lossy(&h[2..18]).trim_end_matches('\0').to_string();
        let id = u16_at(h, 18);
        let len = if extended { usize::try_from(u64_at(h, 20)).unwrap_or(usize::MAX) } else { usize::from(u16_at(h, 20)) };
        let Some(data) = b.get(i + head..(i + head).saturating_add(len)) else {
            break;
        };
        out.push((user, id, data));
        i += head + len;
    }
    out
}

/// Система координат по записям: WKT (LASF_Projection 2112) или ключи
/// GeoTIFF (34735).
fn crs_of(vlrs: &[(String, u16, &[u8])]) -> Result<Crs, String> {
    if let Some((_, _, wkt)) = vlrs.iter().find(|(u, id, _)| u == "LASF_Projection" && *id == 2112) {
        return Crs::from_wkt(&String::from_utf8_lossy(wkt));
    }
    if let Some((_, _, keys)) = vlrs.iter().find(|(u, id, _)| u == "LASF_Projection" && *id == 34735) {
        let n = usize::from(u16_at(keys, 6));
        let mut code = None;
        let mut vertical = None;
        for k in 0..n {
            let e = 8 + k * 8;
            let (id, location, value) = (u16_at(keys, e), u16_at(keys, e + 2), u16_at(keys, e + 6));
            if location != 0 {
                continue;
            }
            match id {
                3072 | 2048 if value != 0 && value != 32767 => code = code.or(Some(u32::from(value))),
                4099 => vertical = Some(value),
                _ => {}
            }
        }
        let code = code.ok_or("у облака нет системы координат")?;
        let mut crs = proj::from_epsg(code).ok_or_else(|| format!("система EPSG:{code} не поддерживается"))?;
        crs.vertical_unit = match vertical {
            Some(9002) => 0.3048,
            Some(9003) => 1200.0 / 3937.0,
            _ => crs.vertical_unit,
        };
        return Ok(crs);
    }
    Err("у облака нет системы координат".into())
}

/// Сырые записи точек в облако: каждая step-я запись.
struct Collect<'a> {
    h: &'a Header,
    crs: &'a Crs,
    points: Vec<[f64; 3]>,
    /// Каналы цвета (16 бит) или класс.
    raw: Vec<([u16; 3], u8)>,
}

impl Collect<'_> {
    fn push(&mut self, rec: &[u8]) {
        let (class_at, class_mask, rgb) = layout(self.h.format);
        let x = f64::from(i32_at(rec, 0)) * self.h.scale[0] + self.h.offset[0];
        let y = f64::from(i32_at(rec, 4)) * self.h.scale[1] + self.h.offset[1];
        let z = f64::from(i32_at(rec, 8)) * self.h.scale[2] + self.h.offset[2];
        let Some((lat, lon)) = self.crs.to_lat_lon(x, y) else {
            return;
        };
        let z = z * self.crs.vertical_unit;
        if !z.is_finite() {
            return;
        }
        let class = rec.get(class_at).copied().unwrap_or(0) & class_mask;
        let color = rgb.map_or([0; 3], |i| [u16_at(rec, i), u16_at(rec, i + 2), u16_at(rec, i + 4)]);
        self.points.push([lat, lon, z]);
        self.raw.push((color, class));
    }

    fn finish(self, total: u64) -> Cloud {
        let has_rgb = layout(self.h.format).2.is_some();
        // Каналы цвета LAS - 16 бит, у некоторых файлов - 8.
        let top = self.raw.iter().flat_map(|(c, _)| c.iter()).copied().max().unwrap_or(0);
        let scale = if top > 255 { 257.0 } else { 1.0 };
        let colors = self
            .raw
            .iter()
            .map(|(c, class)| if has_rgb { c.map(|v| (f64::from(v) / scale).clamp(0.0, 255.0) as u8) } else { class_color(*class) })
            .collect();
        Cloud { points: self.points, colors, total, epsg: self.crs.epsg }
    }
}

/// Узел дерева COPC: уровень и ключ, место куска и количество точек.
#[derive(Debug, Clone, Copy)]
struct Node {
    key: [i32; 4],
    offset: u64,
    size: u64,
    count: i64,
}

/// Разбор страницы иерархии: узлы с точками и ссылки на страницы.
fn page(b: &[u8]) -> Vec<Node> {
    b.as_chunks::<32>()
        .0
        .iter()
        .map(|e| Node {
            key: [i32_at(e, 0), i32_at(e, 4), i32_at(e, 8), i32_at(e, 12)],
            offset: u64_at(e, 16),
            size: u64::from(u32_at(e, 24)),
            count: i64::from(i32_at(e, 28)),
        })
        .collect()
}

/// Наибольшее количество страниц иерархии - защита от петель.
const MAX_PAGES: usize = 100_000;

/// Чтение облака из байтов файла. sparse - файл приходит частями:
/// тогда читается только COPC. Ok(None) - не хватает байтов.
fn read_from(file: &Store, max_points: usize, sparse: bool) -> Result<Option<Cloud>, String> {
    match read_inner(file, max_points, sparse) {
        Ok(c) => Ok(Some(c)),
        Err(e) if e == NEED => Ok(None),
        Err(e) => Err(e),
    }
}

fn read_inner(file: &Store, max_points: usize, sparse: bool) -> Result<Cloud, String> {
    let h = header(need(file, 0, LAS_HEADER.min(file.len))?)?;
    if h.point_offset < h.header_size || h.header_size < 227 {
        return Err("неверный заголовок LAS".into());
    }
    let vlr_area = need(file, h.header_size, h.point_offset - h.header_size)?;
    let vlrs = records(vlr_area, h.vlr_count, false);
    let copc = vlrs.iter().find(|(u, id, _)| u == "copc" && *id == 1).map(|(_, _, d)| *d);
    let laz_vlr = match vlrs.iter().find(|(u, id, _)| u == "laszip encoded" && *id == 22204) {
        Some((_, _, d)) => Some(LazVlr::from_buffer(d).map_err(|e| e.to_string())?),
        None => None,
    };
    if h.compressed && laz_vlr.is_none() {
        return Err("нет записи LASzip".into());
    }
    if let Some(v) = &laz_vlr
        && v.items().is_empty()
    {
        return Err("пустая запись LASzip".into());
    }
    // EVLR - для WKT у файлов LAS 1.4, где она вынесена в конец.
    let mut all = vlrs.clone();
    if !vlrs.iter().any(|(u, _, _)| u == "LASF_Projection") && h.evlr_start >= h.point_offset {
        // Записи EVLR по одной: заголовок 60 байт, потом данные.
        let mut at = h.evlr_start;
        for _ in 0..h.evlr_count.min(1000) {
            let head = need(file, at, 60)?;
            let len = u64_at(head, 20);
            let whole = need(file, at, 60u64.saturating_add(len))?;
            all.extend(records(whole, 1, true));
            at = at.saturating_add(60).saturating_add(len);
        }
    }
    let crs = crs_of(&all)?;
    let mut out = Collect { h: &h, crs: &crs, points: Vec::new(), raw: Vec::new() };
    match (copc, &laz_vlr) {
        (Some(info), Some(vlr)) => copc_points(file, &h, info, vlr, max_points, &mut out)?,
        _ if sparse => return Err("облако по сети - только COPC".into()),
        (_, Some(vlr)) => laz_points(file, &h, vlr, max_points, &mut out)?,
        (_, None) => las_points(file, &h, max_points, &mut out)?,
    }
    Ok(out.finish(h.total))
}

/// Шаг прореживания: каждая n-я из count точек, чтобы вышло не больше max.
fn step_of(count: u64, max: usize) -> u64 {
    let max = (max as u64).max(1);
    count.div_ceil(max).max(1)
}

fn las_points(file: &Store, h: &Header, max: usize, out: &mut Collect) -> Result<(), String> {
    let len = h.record_len as u64;
    let count = h.total.min(file.len.saturating_sub(h.point_offset) / len);
    let data = need(file, h.point_offset, count * len)?;
    let step = step_of(count, max);
    for rec in data.chunks_exact(h.record_len).step_by(usize::try_from(step).unwrap_or(1)) {
        out.push(rec);
    }
    Ok(())
}

fn laz_points(file: &Store, h: &Header, vlr: &LazVlr, max: usize, out: &mut Collect) -> Result<(), String> {
    let whole = need(file, 0, file.len)?;
    let mut cursor = Cursor::new(whole);
    cursor.set_position(h.point_offset);
    let mut dec = LasZipDecompressor::new(cursor, vlr.clone()).map_err(|e| e.to_string())?;
    let size = usize::try_from(vlr.items_size()).map_err(|e| e.to_string())?;
    if size < h.record_len.min(min_record(h.format)) {
        return Err("запись LASzip короче точки".into());
    }
    let mut rec = vec![0u8; size];
    let step = step_of(h.total, max);
    for i in 0..h.total {
        dec.decompress_one(&mut rec).map_err(|e| e.to_string())?;
        if i.is_multiple_of(step) {
            out.push(&rec);
        }
    }
    Ok(())
}

fn copc_points(file: &Store, h: &Header, info: &[u8], vlr: &LazVlr, max: usize, out: &mut Collect) -> Result<(), String> {
    if info.len() < 160 {
        return Err("короткая запись COPC".into());
    }
    let size = usize::try_from(vlr.items_size()).map_err(|e| e.to_string())?;
    if size < min_record(h.format) {
        return Err("запись LASzip короче точки".into());
    }
    // Иерархия: страницы читаются по уровням, пока нужны.
    let root = (u64_at(info, 40), u64_at(info, 48));
    let mut pages: Vec<(u64, u64, i32)> = vec![(root.0, root.1, 0)];
    let mut read_pages: HashSet<u64> = HashSet::new();
    let mut nodes: Vec<Node> = Vec::new();
    let mut level = 0i32;
    let mut chosen: Vec<Node> = Vec::new();
    let mut taken = 0u64;
    loop {
        // Страницы, чьи узлы не глубже уровня: нужны все сразу.
        let mut missing = false;
        let mut i = 0;
        while i < pages.len() {
            let (off, len, depth) = pages[i];
            i += 1;
            if depth > level || read_pages.contains(&off) {
                continue;
            }
            if read_pages.len() >= MAX_PAGES {
                return Err("слишком много страниц иерархии COPC".into());
            }
            let Some(b) = file.get(off, len)? else {
                missing = true;
                continue;
            };
            read_pages.insert(off);
            for n in page(b) {
                if n.count == -1 {
                    pages.push((n.offset, n.size, n.key[0]));
                } else if n.count > 0 {
                    nodes.push(n);
                }
            }
        }
        if missing {
            return Err(NEED.into());
        }
        let here: Vec<Node> = nodes.iter().filter(|n| n.key[0] == level).copied().collect();
        let count: u64 = here.iter().map(|n| n.count.max(0) as u64).sum();
        let deeper = nodes.iter().any(|n| n.key[0] > level) || pages.iter().any(|p| p.2 > level);
        if level > 0 && taken + count > max as u64 {
            break;
        }
        taken += count;
        chosen.extend(here);
        if !deeper || taken >= max as u64 {
            break;
        }
        level += 1;
        if level > 64 {
            break;
        }
    }
    // Куски узлов: все недостающие запрашиваются за один круг.
    let mut chunks = Vec::with_capacity(chosen.len());
    let mut missing = false;
    for n in &chosen {
        match file.get(n.offset, n.size)? {
            Some(b) => chunks.push((n, b)),
            None => missing = true,
        }
    }
    if missing {
        return Err(NEED.into());
    }
    let step = step_of(taken, max);
    let mut index = 0u64;
    let mut rec = vec![0u8; size];
    for (n, bytes) in chunks {
        let count = u64::try_from(n.count).unwrap_or(0);
        // Враждебный узел: точек больше, чем в файле.
        if count > h.total {
            return Err("в узле COPC точек больше, чем в файле".into());
        }
        let mut dec = LayeredPointRecordDecompressor::new(Cursor::new(bytes));
        dec.set_fields_from(vlr.items()).map_err(|e| e.to_string())?;
        for _ in 0..count {
            dec.decompress_next(&mut rec).map_err(|e| e.to_string())?;
            if index.is_multiple_of(step) {
                out.push(&rec);
            }
            index += 1;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// По сети.

/// Длина файла неизвестной длины для чтения частями, 1 ТиБ. Конец файла
/// узнаётся по короткому ответу на запрос куска.
const UNKNOWN_LEN: u64 = 1 << 40;

/// Наибольшее количество повторов чтения. COPC читается за несколько
/// кругов - заголовок, записи VLR, страницы иерархии, узлы дерева.
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
    file: Store,
    name: String,
    max_points: usize,
    /// Длина файла известна.
    exact: bool,
    /// Куски, отданные на запрос и ещё не вставленные.
    asked: Mutex<Vec<(u64, u64)>>,
    /// Куски, которых не хватило при последнем чтении.
    pending: Mutex<Vec<(u64, u64)>>,
    rounds: usize,
}

impl SparseCloud {
    /// Чтение файла name длиной len байт, None - длина неизвестна.
    /// В облаке будет около max_points точек - верхние уровни дерева.
    pub fn new(name: &str, len: Option<u64>, max_points: usize) -> Self {
        Self {
            file: Store::new(len.unwrap_or(UNKNOWN_LEN)),
            name: name.to_string(),
            max_points,
            exact: len.is_some(),
            asked: Mutex::new(Vec::new()),
            pending: Mutex::new(Vec::new()),
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
        let result = read_from(&self.file, self.max_points, true).map_err(|e| format!("облако {}: {e}", self.name));
        *self.pending.lock().unwrap_or_else(PoisonError::into_inner) = self.file.missing();
        result
    }

    /// Куски файла для запроса сейчас - начало и длина в байтах,
    /// соседние слиты.
    pub fn missing(&self) -> Vec<(u64, u64)> {
        let ranges = std::mem::take(&mut *self.pending.lock().unwrap_or_else(PoisonError::into_inner));
        self.asked.lock().unwrap_or_else(PoisonError::into_inner).extend(ranges.iter().copied());
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
            Some(want) if len > want => {
                // Весь файл: длина теперь известна.
                let (fetched, inserts) = (self.file.fetched, self.file.inserts);
                self.file = Store::new(len);
                self.file.fetched = fetched;
                self.file.inserts = inserts;
                self.file.insert(0, bytes);
                self.exact = true;
            }
            Some(want) if len < want && !self.exact => {
                // Конец файла: длина теперь известна.
                self.file.len = start + len;
                self.exact = true;
                self.file.insert(start, bytes);
            }
            _ => self.file.insert(start, bytes),
        }
    }

    /// Получено байтов и ответов вставлено.
    pub fn fetched(&self) -> (u64, u64) {
        (self.file.fetched, self.file.inserts)
    }
}
