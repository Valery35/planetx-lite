// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Чтение окон Cloud Optimized GeoTIFF без GDAL и без сети. Замена
//! чтения каналов через `/vsicurl/` в `ui/sentinel.py` модуля.
//!
//! Модуль сам ничего не скачивает. Интерфейс просит у сети куски файла
//! запросами HTTP Range и отдаёт их сюда:
//!
//! 1. Начало файла [`PREFIX_LEN`] байт кладётся в [`Chunks`],
//!    [`CogHeader::parse_chunks`] разбирает заголовок. Ответ
//!    [`Parsed::NeedMore`] - список кусков, которых ещё нет: IFD или
//!    массивы TileOffsets и TileByteCounts за пределами начала файла.
//!    Куски докладываются в тот же [`Chunks`], разбор повторяется.
//! 2. [`level_for_resolution`] выбирает уровень обзора по нужному шагу,
//!    [`bbox_window`] переводит рамку в системе файла в окно пикселей.
//! 3. [`window_plan`] даёт тайлы, покрывающие окно, с диапазонами байт.
//! 4. [`read_window`] распаковывает тайлы, снимает предиктор и собирает
//!    окно в [`Grid<f64>`] сырых значений. Значение «нет данных» остаётся
//!    как есть, его разбирает [`crate::sentinel::reflectance`].
//!
//! Поддерживаются классический TIFF и BigTIFF, оба порядка байт, тайлы,
//! целые 8-64 бита со знаком и без, float32 и float64, сжатие none,
//! deflate, LZW и zstd, предикторы 1, 2 и 3, расположение каналов
//! chunky и separate. Файл с полосами вместо тайлов - ошибка
//! [`CogError::Stripped`]. Входной файл считается враждебным: любые
//! длины проверяются, ошибка - `Err`, а не паника.

use std::fmt;
use std::io::Read;
use std::ops::Range;

use crate::sentinel::Grid;

/// Длина начала файла для первого запроса. GDAL читает столько же.
pub const PREFIX_LEN: u64 = 16384;
/// Сколько байт просить под IFD, количество записей которого неизвестно.
const IFD_GUESS: u64 = 4096;
/// Наибольшее количество IFD в файле, дальше цепочка не читается.
const MAX_IFDS: usize = 64;
/// Наибольшее количество записей в одном IFD.
const MAX_ENTRIES: u64 = 4096;
/// Наибольшая длина значения одного тега, байт.
const MAX_VALUE_BYTES: u64 = 64 << 20;
/// Наибольшая длина распакованного тайла, байт.
const MAX_TILE_BYTES: u64 = 64 << 20;
/// Наибольшее количество пикселей окна.
const MAX_WINDOW_PIXELS: u64 = 1 << 26;
/// Куски заголовка ближе этого промежутка просятся одним запросом.
const MERGE_GAP: u64 = 4096;
/// Доля пикселя, в пределах которой край окна прилипает к целому.
const SNAP: f64 = 1e-6;
/// Относительный допуск сравнения шагов уровней.
const RES_TOLERANCE: f64 = 1e-9;

// Теги TIFF и GeoTIFF.
const NEW_SUBFILE_TYPE: u16 = 254;
const IMAGE_WIDTH: u16 = 256;
const IMAGE_LENGTH: u16 = 257;
const BITS_PER_SAMPLE: u16 = 258;
const COMPRESSION: u16 = 259;
const STRIP_OFFSETS: u16 = 273;
const SAMPLES_PER_PIXEL: u16 = 277;
const PLANAR_CONFIGURATION: u16 = 284;
const PREDICTOR: u16 = 317;
const TILE_WIDTH: u16 = 322;
const TILE_LENGTH: u16 = 323;
const TILE_OFFSETS: u16 = 324;
const TILE_BYTE_COUNTS: u16 = 325;
const SAMPLE_FORMAT: u16 = 339;
const MODEL_PIXEL_SCALE: u16 = 33550;
const MODEL_TIEPOINT: u16 = 33922;
const MODEL_TRANSFORMATION: u16 = 34264;
const GEO_KEY_DIRECTORY: u16 = 34735;
const GDAL_METADATA: u16 = 42112;
const GDAL_NODATA: u16 = 42113;

/// Теги, значения которых нужны у каждого уровня.
const LEVEL_TAGS: [u16; 13] = [
    NEW_SUBFILE_TYPE,
    IMAGE_WIDTH,
    IMAGE_LENGTH,
    BITS_PER_SAMPLE,
    COMPRESSION,
    SAMPLES_PER_PIXEL,
    PLANAR_CONFIGURATION,
    PREDICTOR,
    TILE_WIDTH,
    TILE_LENGTH,
    TILE_OFFSETS,
    TILE_BYTE_COUNTS,
    SAMPLE_FORMAT,
];

/// Теги привязки и метаданных, нужны только у полного разрешения.
const GEO_TAGS: [u16; 6] = [
    MODEL_PIXEL_SCALE,
    MODEL_TIEPOINT,
    MODEL_TRANSFORMATION,
    GEO_KEY_DIRECTORY,
    GDAL_METADATA,
    GDAL_NODATA,
];

/// Ошибка чтения COG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CogError {
    /// Начало файла - не TIFF и не BigTIFF.
    NotTiff,
    /// Файл короче, чем требуют его же смещения.
    Truncated,
    /// Нарушено устройство файла, в скобках - что именно.
    Corrupt(&'static str),
    /// Снимок разбит на полосы, а не на тайлы.
    Stripped,
    /// Возможность TIFF, которой здесь нет, в скобках - какая.
    Unsupported(&'static str),
    /// Нет уровня с таким номером или нет полного разрешения.
    NoLevel,
    /// Окно пустое или выходит за снимок, или нет такого канала.
    Window,
    /// Размер больше допустимого.
    TooLarge,
    /// Вызывающий не дал байты тайла с этим номером.
    MissingTile(usize),
    /// Тайл с этим номером не распаковывается.
    Decode(usize),
}

impl fmt::Display for CogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotTiff => write!(f, "COG не читается: не TIFF"),
            Self::Truncated => write!(f, "COG не читается: файл обрезан"),
            Self::Corrupt(what) => write!(f, "COG не читается: {what}"),
            Self::Stripped => write!(f, "COG не читается: полосы вместо тайлов"),
            Self::Unsupported(what) => write!(f, "COG не читается: не поддержано - {what}"),
            Self::NoLevel => write!(f, "COG не читается: нет уровня"),
            Self::Window => write!(f, "COG не читается: окно вне снимка"),
            Self::TooLarge => write!(f, "COG не читается: размер больше допустимого"),
            Self::MissingTile(i) => write!(f, "COG не читается: нет тайла {i}"),
            Self::Decode(i) => write!(f, "COG не читается: тайл {i} не распакован"),
        }
    }
}

impl std::error::Error for CogError {}

/// Прочитанные куски файла: смещение и байты. Соседние и перекрытые
/// куски сливаются в один.
#[derive(Debug, Clone, Default)]
pub struct Chunks {
    parts: Vec<(u64, Vec<u8>)>,
    file_len: Option<u64>,
}

impl Chunks {
    /// Пустой набор.
    pub fn new() -> Self {
        Self::default()
    }

    /// Набор из начала файла.
    pub fn from_prefix(bytes: &[u8]) -> Self {
        let mut chunks = Self::new();
        chunks.add(0, bytes);
        chunks
    }

    /// Длина файла, если сеть её сообщила (Content-Range). С ней
    /// разбор отличает обрезанный файл от ещё не прочитанного куска.
    pub fn set_file_len(&mut self, len: u64) {
        self.file_len = Some(len);
    }

    /// Длина файла или None, если неизвестна.
    pub fn file_len(&self) -> Option<u64> {
        self.file_len
    }

    /// Количество байт во всех кусках.
    pub fn covered(&self) -> u64 {
        self.parts.iter().map(|p| p.1.len() as u64).sum()
    }

    /// Добавить кусок со смещения offset. true, если прочитанного
    /// стало больше. false - новых байт нет, повторный запрос того же
    /// куска бесполезен.
    pub fn add(&mut self, offset: u64, bytes: &[u8]) -> bool {
        let Some(end) = offset.checked_add(bytes.len() as u64) else {
            return false;
        };
        if bytes.is_empty() {
            return false;
        }
        let before = self.covered();
        let (mut start, mut stop) = (offset, end);
        let mut keep = Vec::with_capacity(self.parts.len() + 1);
        let mut touching = Vec::new();
        for part in std::mem::take(&mut self.parts) {
            let part_end = part.0 + part.1.len() as u64;
            if part_end < offset || part.0 > end {
                keep.push(part);
            } else {
                start = start.min(part.0);
                stop = stop.max(part_end);
                touching.push(part);
            }
        }
        let Ok(size) = usize::try_from(stop - start) else {
            keep.extend(touching);
            keep.sort_by_key(|p| p.0);
            self.parts = keep;
            return false;
        };
        let mut buf = vec![0u8; size];
        let pieces = touching
            .iter()
            .map(|(o, b)| (*o, b.as_slice()))
            .chain(std::iter::once((offset, bytes)));
        for (at, piece) in pieces {
            let from = usize::try_from(at - start).unwrap_or(usize::MAX);
            if let Some(dst) = from
                .checked_add(piece.len())
                .and_then(|to| buf.get_mut(from..to))
            {
                dst.copy_from_slice(piece);
            }
        }
        keep.push((start, buf));
        keep.sort_by_key(|p| p.0);
        self.parts = keep;
        self.covered() > before
    }

    /// Байты диапазона, если он целиком прочитан.
    pub fn get(&self, range: Range<u64>) -> Option<&[u8]> {
        if range.end < range.start {
            return None;
        }
        let at = self.parts.partition_point(|p| p.0 <= range.start);
        let (start, bytes) = self.parts.get(at.checked_sub(1)?)?;
        let from = usize::try_from(range.start - start).ok()?;
        let to = usize::try_from(range.end - start).ok()?;
        bytes.get(from..to)
    }
}

/// Слить диапазоны: отсортировать и объединить перекрытые и те, между
/// которыми не больше gap байт. Пригодно и для запросов тайлов.
pub fn merge_ranges(mut ranges: Vec<Range<u64>>, gap: u64) -> Vec<Range<u64>> {
    ranges.retain(|r| r.end > r.start);
    ranges.sort_by_key(|r| r.start);
    let mut out: Vec<Range<u64>> = Vec::with_capacity(ranges.len());
    for r in ranges {
        match out.last_mut() {
            Some(last) if r.start <= last.end.saturating_add(gap) => {
                last.end = last.end.max(r.end);
            }
            _ => out.push(r),
        }
    }
    out
}

/// Тип значения отсчёта.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    Uint,
    Int,
    Float,
}

/// Сжатие тайлов.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Deflate,
    Lzw,
    Zstd,
}

/// Предиктор TIFF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Predictor {
    /// 1 - без предиктора.
    None,
    /// 2 - разность соседних отсчётов по строке.
    Horizontal,
    /// 3 - разность байтов плавающих чисел.
    FloatingPoint,
}

/// Расположение каналов в тайлах.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Planar {
    /// 1 - каналы пикселя подряд в одном тайле.
    Chunky,
    /// 2 - у каждого канала свои тайлы.
    Separate,
}

/// Уровень: полное разрешение или обзор.
#[derive(Debug, Clone, PartialEq)]
pub struct Level {
    pub width: u32,
    pub height: u32,
    pub tile_width: u32,
    pub tile_height: u32,
    pub samples: u16,
    pub bits: u16,
    pub format: SampleFormat,
    pub compression: Compression,
    pub predictor: Predictor,
    pub planar: Planar,
    /// Смещения тайлов по строкам тайлов, у Separate - канал за каналом.
    pub tile_offsets: Vec<u64>,
    pub tile_byte_counts: Vec<u64>,
}

impl Level {
    /// Количество тайлов по ширине.
    pub fn tiles_across(&self) -> u64 {
        u64::from(self.width).div_ceil(u64::from(self.tile_width))
    }

    /// Количество тайлов по высоте.
    pub fn tiles_down(&self) -> u64 {
        u64::from(self.height).div_ceil(u64::from(self.tile_height))
    }

    /// Количество наборов тайлов: 1 у Chunky, каналы у Separate.
    fn planes(&self) -> u64 {
        match self.planar {
            Planar::Chunky => 1,
            Planar::Separate => u64::from(self.samples),
        }
    }

    /// Номер тайла (tx, ty) канала sample.
    fn tile_index(&self, tx: u64, ty: u64, sample: u16) -> Option<usize> {
        let plane = match self.planar {
            Planar::Chunky => 0,
            Planar::Separate => u64::from(sample),
        };
        let per_plane = self.tiles_across().checked_mul(self.tiles_down())?;
        let index = plane
            .checked_mul(per_plane)?
            .checked_add(ty.checked_mul(self.tiles_across())?)?
            .checked_add(tx)?;
        usize::try_from(index).ok()
    }
}

/// Заголовок COG: уровни, привязка, нет данных, масштаб и сдвиг.
#[derive(Debug, Clone, PartialEq)]
pub struct CogHeader {
    pub big_endian: bool,
    pub bigtiff: bool,
    /// 0 - полное разрешение, дальше обзоры от крупных к мелким.
    pub levels: Vec<Level>,
    /// GDAL_NODATA.
    pub nodata: Option<f64>,
    /// Привязка полного разрешения в порядке GDAL: x0, шаг x, поворот,
    /// y0, поворот, шаг y. Отсчёт от угла пикселя.
    pub geotransform: Option<[f64; 6]>,
    /// EPSG системы файла. None у своей системы (32767), например
    /// синусоидальной MODIS.
    pub epsg: Option<u32>,
    /// RasterPixelIsPoint, привязка уже сдвинута на полпикселя, как в GDAL.
    pub pixel_is_point: bool,
    /// Масштаб и сдвиг первого канала из GDAL_METADATA.
    pub scale: Option<f64>,
    pub offset: Option<f64>,
}

/// Итог разбора заголовка.
#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    /// Заголовок разобран целиком.
    Header(CogHeader),
    /// Нужны ещё эти куски файла. Конец диапазона может выходить
    /// за файл, сервер отдаёт столько, сколько есть.
    NeedMore(Vec<Range<u64>>),
}

/// Тайл окна: номер и диапазон байт файла.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileRequest {
    pub index: usize,
    pub range: Range<u64>,
}

/// Окно пикселей уровня.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub x0: u32,
    pub y0: u32,
    pub width: u32,
    pub height: u32,
}

/// Порядок байт и разрядность смещений файла.
#[derive(Debug, Clone, Copy)]
struct Fmt {
    big: bool,
    bigtiff: bool,
}

impl Fmt {
    /// Целое без знака из 1-8 байт.
    fn uint(self, b: &[u8]) -> u64 {
        let fold = |v: u64, x: &u8| (v << 8) | u64::from(*x);
        if self.big {
            b.iter().take(8).fold(0, fold)
        } else {
            b.iter().take(8).rev().fold(0, fold)
        }
    }

    fn count_size(self) -> u64 {
        if self.bigtiff { 8 } else { 2 }
    }

    fn entry_size(self) -> u64 {
        if self.bigtiff { 20 } else { 12 }
    }

    fn offset_size(self) -> u64 {
        if self.bigtiff { 8 } else { 4 }
    }
}

/// Запись IFD: тег, тип, количество, абсолютное смещение значений.
#[derive(Debug, Clone, Copy)]
struct Entry {
    tag: u16,
    typ: u16,
    count: u64,
    data: u64,
}

/// Длина одного значения типа TIFF или None у неизвестного типа.
fn type_size(typ: u16) -> Option<u64> {
    match typ {
        1 | 2 | 6 | 7 => Some(1),
        3 | 8 => Some(2),
        4 | 9 | 11 | 13 => Some(4),
        5 | 10 | 12 | 16 | 17 | 18 => Some(8),
        _ => None,
    }
}

/// Источник байт разбора с учётом недостающих кусков.
struct Src<'a> {
    chunks: &'a Chunks,
    fmt: Fmt,
    missing: Vec<Range<u64>>,
}

impl<'a> Src<'a> {
    /// Байты диапазона. Нет в кусках - диапазон в недостающие и None.
    fn bytes(&mut self, range: Range<u64>) -> Result<Option<&'a [u8]>, CogError> {
        if let Some(len) = self.chunks.file_len()
            && range.end > len
        {
            return Err(CogError::Truncated);
        }
        match self.chunks.get(range.clone()) {
            Some(b) => Ok(Some(b)),
            None => {
                self.missing.push(range);
                Ok(None)
            }
        }
    }

    /// Диапазон значений записи.
    fn entry_range(e: &Entry) -> Result<Range<u64>, CogError> {
        let size = type_size(e.typ).ok_or(CogError::Corrupt("тип тега"))?;
        let len = size
            .checked_mul(e.count)
            .filter(|n| *n <= MAX_VALUE_BYTES)
            .ok_or(CogError::TooLarge)?;
        let end = e
            .data
            .checked_add(len)
            .ok_or(CogError::Corrupt("смещение тега"))?;
        Ok(e.data..end)
    }

    /// Запросить значения записи, если их ещё нет.
    fn want(&mut self, e: &Entry) -> Result<(), CogError> {
        let range = Self::entry_range(e)?;
        self.bytes(range).map(|_| ())
    }

    /// Значения записи, к этому моменту уже прочитанные.
    fn values(&self, e: &Entry) -> Result<&'a [u8], CogError> {
        let range = Self::entry_range(e)?;
        self.chunks
            .get(range)
            .ok_or(CogError::Corrupt("значения тега"))
    }

    /// Целые значения записи.
    fn uints(&self, e: &Entry) -> Result<Vec<u64>, CogError> {
        let size = match e.typ {
            1 | 7 => 1,
            3 => 2,
            4 | 13 => 4,
            16 | 18 => 8,
            _ => return Err(CogError::Corrupt("тип целого тега")),
        };
        let b = self.values(e)?;
        Ok(b.chunks_exact(size).map(|c| self.fmt.uint(c)).collect())
    }

    /// Числа записи как f64.
    fn floats(&self, e: &Entry) -> Result<Vec<f64>, CogError> {
        let b = self.values(e)?;
        let fmt = self.fmt;
        let signed = |c: &[u8], bits: u32| {
            let shift = 64 - bits;
            (((fmt.uint(c) << shift) as i64) >> shift) as f64
        };
        let typ = e.typ;
        if matches!(typ, 2 | 13 | 18) {
            return Err(CogError::Corrupt("тип числового тега"));
        }
        let size = type_size(typ)
            .and_then(|s| usize::try_from(s).ok())
            .ok_or(CogError::Corrupt("тип числового тега"))?;
        Ok(b.chunks_exact(size)
            .map(|c| match typ {
                6 => signed(c, 8),
                8 => signed(c, 16),
                9 => signed(c, 32),
                17 => signed(c, 64),
                11 => f64::from(f32::from_bits(fmt.uint(c) as u32)),
                12 => f64::from_bits(fmt.uint(c)),
                5 | 10 => {
                    let (n, d) = c.split_at(4);
                    let (n, d) = if typ == 5 {
                        (fmt.uint(n) as f64, fmt.uint(d) as f64)
                    } else {
                        (signed(n, 32), signed(d, 32))
                    };
                    n / d
                }
                _ => fmt.uint(c) as f64,
            })
            .collect())
    }

    /// Текст записи ASCII без нулей в конце.
    fn text(&self, e: &Entry) -> Result<String, CogError> {
        let b = self.values(e)?;
        Ok(String::from_utf8_lossy(b)
            .trim_end_matches('\0')
            .trim()
            .to_string())
    }
}

/// Записи IFD со смещения offset и смещение следующего IFD. None -
/// кусок ещё не прочитан, он уже в недостающих.
fn read_ifd(src: &mut Src<'_>, offset: u64) -> Result<Option<(Vec<Entry>, u64)>, CogError> {
    let fmt = src.fmt;
    let cs = fmt.count_size();
    let count_end = offset
        .checked_add(cs)
        .ok_or(CogError::Corrupt("смещение IFD"))?;
    if let Some(len) = src.chunks.file_len()
        && count_end > len
    {
        return Err(CogError::Truncated);
    }
    let Some(cb) = src.chunks.get(offset..count_end) else {
        let mut end = offset.saturating_add(IFD_GUESS);
        if let Some(len) = src.chunks.file_len() {
            end = end.min(len);
        }
        src.missing.push(offset..end);
        return Ok(None);
    };
    let n = fmt.uint(cb);
    if n == 0 || n > MAX_ENTRIES {
        return Err(CogError::Corrupt("количество записей IFD"));
    }
    let es = fmt.entry_size();
    let os = fmt.offset_size();
    let block_end = n
        .checked_mul(es)
        .and_then(|v| v.checked_add(os))
        .and_then(|v| v.checked_add(count_end))
        .ok_or(CogError::Corrupt("длина IFD"))?;
    let Some(block) = src.bytes(count_end..block_end)? else {
        return Ok(None);
    };
    let es_us = usize::try_from(es).map_err(|_| CogError::TooLarge)?;
    let os_us = usize::try_from(os).map_err(|_| CogError::TooLarge)?;
    let mut entries = Vec::new();
    let mut at = count_end;
    for raw in block
        .chunks_exact(es_us)
        .take(usize::try_from(n).unwrap_or(0))
    {
        let tag = fmt.uint(raw.get(0..2).unwrap_or(&[])) as u16;
        let typ = fmt.uint(raw.get(2..4).unwrap_or(&[])) as u16;
        let (count, field_at) = if fmt.bigtiff {
            (fmt.uint(raw.get(4..12).unwrap_or(&[])), 12)
        } else {
            (fmt.uint(raw.get(4..8).unwrap_or(&[])), 8)
        };
        let field = raw.get(field_at..).unwrap_or(&[]);
        if let Some(size) = type_size(typ) {
            let inline = size.checked_mul(count).is_some_and(|len| len <= os);
            let data = if inline {
                at + field_at as u64
            } else {
                fmt.uint(field)
            };
            entries.push(Entry {
                tag,
                typ,
                count,
                data,
            });
        }
        at += es;
    }
    let next = fmt.uint(
        block
            .get(block.len().saturating_sub(os_us)..)
            .unwrap_or(&[]),
    );
    Ok(Some((entries, next)))
}

fn find(entries: &[Entry], tag: u16) -> Option<&Entry> {
    entries.iter().find(|e| e.tag == tag)
}

/// NewSubfileType записи или 0.
fn subfile_type(src: &Src<'_>, entries: &[Entry]) -> Result<u64, CogError> {
    match find(entries, NEW_SUBFILE_TYPE) {
        Some(e) => Ok(src.uints(e)?.first().copied().unwrap_or(0)),
        None => Ok(0),
    }
}

/// Маска GDAL (бит 4 NewSubfileType), такие IFD пропускаются.
const SUBFILE_MASK: u64 = 4;
/// Обзор (бит 1 NewSubfileType).
const SUBFILE_REDUCED: u64 = 1;

/// Первое целое тега или None.
fn first_uint(src: &Src<'_>, entries: &[Entry], tag: u16) -> Result<Option<u64>, CogError> {
    match find(entries, tag) {
        Some(e) => Ok(src.uints(e)?.first().copied()),
        None => Ok(None),
    }
}

/// Уровень из записей IFD, все значения уже прочитаны.
fn build_level(src: &Src<'_>, entries: &[Entry]) -> Result<Level, CogError> {
    let dim = |tag, what| -> Result<u32, CogError> {
        let v = first_uint(src, entries, tag)?.ok_or(CogError::Corrupt(what))?;
        u32::try_from(v)
            .ok()
            .filter(|v| *v > 0)
            .ok_or(CogError::Corrupt(what))
    };
    let width = dim(IMAGE_WIDTH, "ширина")?;
    let height = dim(IMAGE_LENGTH, "высота")?;
    if find(entries, TILE_WIDTH).is_none() {
        return if find(entries, STRIP_OFFSETS).is_some() {
            Err(CogError::Stripped)
        } else {
            Err(CogError::Corrupt("нет TileWidth"))
        };
    }
    let tile_width = dim(TILE_WIDTH, "ширина тайла")?;
    let tile_height = dim(TILE_LENGTH, "высота тайла")?;
    let samples = first_uint(src, entries, SAMPLES_PER_PIXEL)?.unwrap_or(1);
    let samples = u16::try_from(samples)
        .ok()
        .filter(|v| *v > 0)
        .ok_or(CogError::Corrupt("SamplesPerPixel"))?;
    let bits_list = match find(entries, BITS_PER_SAMPLE) {
        Some(e) => src.uints(e)?,
        None => vec![1],
    };
    let bits = bits_list.first().copied().unwrap_or(1);
    if bits_list.iter().any(|b| *b != bits) {
        return Err(CogError::Unsupported("разная разрядность каналов"));
    }
    let format = match first_uint(src, entries, SAMPLE_FORMAT)?.unwrap_or(1) {
        1 => SampleFormat::Uint,
        2 => SampleFormat::Int,
        3 => SampleFormat::Float,
        _ => return Err(CogError::Unsupported("SampleFormat")),
    };
    let bits_ok = match format {
        SampleFormat::Float => matches!(bits, 32 | 64),
        _ => matches!(bits, 8 | 16 | 32 | 64),
    };
    if !bits_ok {
        return Err(CogError::Unsupported("BitsPerSample"));
    }
    let compression = match first_uint(src, entries, COMPRESSION)?.unwrap_or(1) {
        1 => Compression::None,
        8 | 32946 => Compression::Deflate,
        5 => Compression::Lzw,
        50000 => Compression::Zstd,
        _ => return Err(CogError::Unsupported("сжатие")),
    };
    let predictor = match first_uint(src, entries, PREDICTOR)?.unwrap_or(1) {
        1 => Predictor::None,
        2 if format != SampleFormat::Float => Predictor::Horizontal,
        3 if format == SampleFormat::Float => Predictor::FloatingPoint,
        _ => return Err(CogError::Unsupported("предиктор")),
    };
    let planar = match first_uint(src, entries, PLANAR_CONFIGURATION)?.unwrap_or(1) {
        1 => Planar::Chunky,
        2 => Planar::Separate,
        _ => return Err(CogError::Corrupt("PlanarConfiguration")),
    };
    let mut level = Level {
        width,
        height,
        tile_width,
        tile_height,
        samples,
        bits: bits as u16,
        format,
        compression,
        predictor,
        planar,
        tile_offsets: Vec::new(),
        tile_byte_counts: Vec::new(),
    };
    let expected = level
        .tiles_across()
        .checked_mul(level.tiles_down())
        .and_then(|v| v.checked_mul(level.planes()))
        .and_then(|v| usize::try_from(v).ok())
        .ok_or(CogError::TooLarge)?;
    let list = |tag, what| -> Result<Vec<u64>, CogError> {
        let e = find(entries, tag).ok_or(CogError::Corrupt(what))?;
        let mut v = src.uints(e)?;
        if v.len() < expected {
            return Err(CogError::Corrupt(what));
        }
        v.truncate(expected);
        Ok(v)
    };
    level.tile_offsets = list(TILE_OFFSETS, "TileOffsets")?;
    level.tile_byte_counts = list(TILE_BYTE_COUNTS, "TileByteCounts")?;
    Ok(level)
}

/// Ключи GeoKeyDirectory: (GTModelType, GTRasterType, EPSG).
fn geokeys(keys: &[u64]) -> (Option<u64>, Option<u64>, Option<u32>) {
    let n = keys.get(3).copied().unwrap_or(0);
    let (mut model, mut raster, mut geographic, mut projected) = (None, None, None, None);
    for &[id, location, _count, value] in keys
        .get(4..)
        .unwrap_or(&[])
        .as_chunks::<4>()
        .0
        .iter()
        .take(usize::try_from(n).unwrap_or(0))
    {
        if location == 0 {
            match id {
                1024 => model = Some(value),
                1025 => raster = Some(value),
                2048 => geographic = Some(value),
                3072 => projected = Some(value),
                _ => {}
            }
        }
    }
    let code = |v: Option<u64>| {
        v.filter(|c| (1..32767).contains(c))
            .and_then(|c| u32::try_from(c).ok())
    };
    let epsg = match model {
        Some(1) => code(projected),
        Some(2) => code(geographic),
        None => code(projected).or_else(|| code(geographic)),
        _ => None,
    };
    (model, raster, epsg)
}

/// Привязка GDAL из ModelTransformation или ModelTiepoint и
/// ModelPixelScale.
fn geotransform(
    scale: Option<&[f64]>,
    tiepoint: Option<&[f64]>,
    matrix: Option<&[f64]>,
    point: bool,
) -> Option<[f64; 6]> {
    let mut gt = match (matrix, tiepoint, scale) {
        (Some(m), _, _) if m.len() >= 16 => [m[3], m[0], m[1], m[7], m[4], m[5]],
        (_, Some(t), Some(s)) if t.len() >= 6 && s.len() >= 2 => {
            let (i, j, x, y) = (t[0], t[1], t[3], t[4]);
            let (sx, sy) = (s[0], s[1]);
            [x - i * sx, sx, 0.0, y + j * sy, 0.0, -sy]
        }
        _ => return None,
    };
    if point {
        gt[0] -= 0.5 * (gt[1] + gt[2]);
        gt[3] -= 0.5 * (gt[4] + gt[5]);
    }
    gt.iter().all(|v| v.is_finite()).then_some(gt)
}

/// Значение атрибута key="..." в теге элемента XML.
fn xml_attr<'x>(tag: &'x str, key: &str) -> Option<&'x str> {
    let pattern = format!("{key}=\"");
    let mut rest = tag;
    while let Some(at) = rest.find(&pattern) {
        let before = rest.get(..at)?.chars().next_back();
        let after = rest.get(at + pattern.len()..)?;
        if before.is_none_or(char::is_whitespace) {
            return after.find('"').and_then(|end| after.get(..end));
        }
        rest = after;
    }
    None
}

/// Масштаб и сдвиг первого канала из XML GDAL_METADATA.
fn metadata_scale_offset(xml: &str) -> (Option<f64>, Option<f64>) {
    let (mut scale, mut offset) = (None, None);
    let mut rest = xml;
    while let Some(at) = rest.find("<Item") {
        let Some(item) = rest.get(at..) else { break };
        let Some(close) = item.find('>') else { break };
        let head = item.get(..close).unwrap_or("");
        let body = item.get(close + 1..).unwrap_or("");
        let text = body.find("</Item>").and_then(|end| body.get(..end));
        rest = body;
        let sample = xml_attr(head, "sample");
        if !(sample.is_none() || sample == Some("0")) {
            continue;
        }
        let role = xml_attr(head, "role")
            .or_else(|| xml_attr(head, "name"))
            .map(str::to_ascii_lowercase);
        let value = text.and_then(|t| t.trim().parse::<f64>().ok());
        match role.as_deref() {
            Some("scale") => scale = scale.or(value),
            Some("offset") => offset = offset.or(value),
            _ => {}
        }
    }
    (scale, offset)
}

impl CogHeader {
    /// Разобрать заголовок по началу файла.
    pub fn parse(prefix: &[u8]) -> Result<Parsed, CogError> {
        Self::parse_chunks(&Chunks::from_prefix(prefix))
    }

    /// Разобрать заголовок по прочитанным кускам. Читает все IFD,
    /// пропускает маски, полное разрешение - первый IFD без признака
    /// обзора.
    pub fn parse_chunks(chunks: &Chunks) -> Result<Parsed, CogError> {
        let mut src = Src {
            chunks,
            fmt: Fmt {
                big: false,
                bigtiff: false,
            },
            missing: Vec::new(),
        };
        let need_prefix = |chunks: &Chunks| {
            let end = chunks.file_len().map_or(PREFIX_LEN, |l| l.min(PREFIX_LEN));
            Ok(Parsed::NeedMore(std::iter::once(0..end).collect()))
        };
        let Some(head) = src.bytes(0..8)? else {
            return need_prefix(chunks);
        };
        let big = match head.get(0..2) {
            Some(b"II") => false,
            Some(b"MM") => true,
            _ => return Err(CogError::NotTiff),
        };
        let probe = Fmt {
            big,
            bigtiff: false,
        };
        let magic = probe.uint(head.get(2..4).unwrap_or(&[]));
        let first = match magic {
            42 => probe.uint(head.get(4..8).unwrap_or(&[])),
            43 => {
                let Some(head) = src.bytes(0..16)? else {
                    return need_prefix(chunks);
                };
                let bytesize = probe.uint(head.get(4..6).unwrap_or(&[]));
                let zero = probe.uint(head.get(6..8).unwrap_or(&[]));
                if bytesize != 8 || zero != 0 {
                    return Err(CogError::Corrupt("заголовок BigTIFF"));
                }
                probe.uint(head.get(8..16).unwrap_or(&[]))
            }
            _ => return Err(CogError::NotTiff),
        };
        src.fmt = Fmt {
            big,
            bigtiff: magic == 43,
        };
        src.missing.clear();

        // Цепочка IFD, пока она прочитана.
        let mut ifds: Vec<Vec<Entry>> = Vec::new();
        let mut visited: Vec<u64> = Vec::new();
        let mut offset = first;
        while offset != 0 && ifds.len() < MAX_IFDS {
            if visited.contains(&offset) {
                return Err(CogError::Corrupt("кольцо IFD"));
            }
            visited.push(offset);
            match read_ifd(&mut src, offset)? {
                Some((entries, next)) => {
                    ifds.push(entries);
                    offset = next;
                }
                None => break,
            }
        }

        // Значения нужных тегов. Маски пропускаются, привязка - только
        // у первого уровня полного разрешения.
        let mut kinds = Vec::with_capacity(ifds.len());
        let mut full_seen = false;
        for entries in &ifds {
            // Сначала тип IFD: маске остальные значения не нужны.
            let kind = match find(entries, NEW_SUBFILE_TYPE) {
                None => 0,
                Some(e) => match chunks.get(Src::entry_range(e)?) {
                    Some(_) => subfile_type(&src, entries)?,
                    None => {
                        src.want(e)?;
                        kinds.push(None);
                        continue;
                    }
                },
            };
            kinds.push(Some(kind));
            if kind & SUBFILE_MASK != 0 {
                continue;
            }
            for e in entries.iter().filter(|e| LEVEL_TAGS.contains(&e.tag)) {
                src.want(e)?;
            }
            if kind & SUBFILE_REDUCED == 0 && !full_seen {
                full_seen = true;
                for e in entries.iter().filter(|e| GEO_TAGS.contains(&e.tag)) {
                    src.want(e)?;
                }
            }
        }
        if !src.missing.is_empty() {
            let mut ranges = std::mem::take(&mut src.missing);
            if let Some(len) = chunks.file_len() {
                for r in &mut ranges {
                    r.end = r.end.min(len);
                }
            }
            return Ok(Parsed::NeedMore(merge_ranges(ranges, MERGE_GAP)));
        }

        let mut full: Option<(Level, &Vec<Entry>)> = None;
        let mut overviews = Vec::new();
        for (entries, kind) in ifds.iter().zip(kinds) {
            let kind = kind.unwrap_or(0);
            if kind & SUBFILE_MASK != 0 {
                continue;
            }
            if kind & SUBFILE_REDUCED == 0 {
                if full.is_none() {
                    full = Some((build_level(&src, entries)?, entries));
                }
            } else {
                overviews.push(build_level(&src, entries)?);
            }
        }
        let (base, geo) = full.ok_or(CogError::NoLevel)?;
        overviews.retain(|o| o.width <= base.width && o.height <= base.height);
        overviews.sort_by(|a, b| b.width.cmp(&a.width).then(b.height.cmp(&a.height)));

        let floats = |tag| -> Result<Option<Vec<f64>>, CogError> {
            find(geo, tag).map(|e| src.floats(e)).transpose()
        };
        let keys = match find(geo, GEO_KEY_DIRECTORY) {
            Some(e) => src.uints(e)?,
            None => Vec::new(),
        };
        let (_model, raster, epsg) = geokeys(&keys);
        let pixel_is_point = raster == Some(2);
        let scale = floats(MODEL_PIXEL_SCALE)?;
        let tiepoint = floats(MODEL_TIEPOINT)?;
        let matrix = floats(MODEL_TRANSFORMATION)?;
        let gt = geotransform(
            scale.as_deref(),
            tiepoint.as_deref(),
            matrix.as_deref(),
            pixel_is_point,
        );
        let nodata = match find(geo, GDAL_NODATA) {
            Some(e) => src.text(e)?.parse::<f64>().ok(),
            None => None,
        };
        let (value_scale, value_offset) = match find(geo, GDAL_METADATA) {
            Some(e) => metadata_scale_offset(&src.text(e)?),
            None => (None, None),
        };
        let mut levels = vec![base];
        levels.extend(overviews);
        Ok(Parsed::Header(CogHeader {
            big_endian: big,
            bigtiff: magic == 43,
            levels,
            nodata,
            geotransform: gt,
            epsg,
            pixel_is_point,
            scale: value_scale,
            offset: value_offset,
        }))
    }

    /// Уровень по номеру.
    pub fn level(&self, level: usize) -> Option<&Level> {
        self.levels.get(level)
    }

    /// Привязка уровня: шаг полного разрешения, умноженный на отношение
    /// размеров, как у обзоров GDAL.
    pub fn level_transform(&self, level: usize) -> Option<[f64; 6]> {
        let gt = self.geotransform?;
        let base = self.levels.first()?;
        let lvl = self.levels.get(level)?;
        let fx = f64::from(base.width) / f64::from(lvl.width);
        let fy = f64::from(base.height) / f64::from(lvl.height);
        Some([gt[0], gt[1] * fx, gt[2] * fy, gt[3], gt[4] * fx, gt[5] * fy])
    }

    /// Шаг пикселя уровня (x, y) в единицах системы файла, по модулю.
    pub fn level_resolution(&self, level: usize) -> Option<(f64, f64)> {
        let gt = self.level_transform(level)?;
        Some((gt[1].hypot(gt[4]), gt[2].hypot(gt[5])))
    }
}

/// Номер уровня для огрубления factor относительно полного разрешения:
/// самый грубый обзор, который ещё не грубее нужного. factor не больше
/// 1 или не число - уровень 0.
pub fn level_for_factor(header: &CogHeader, factor: f64) -> usize {
    let Some(base) = header.levels.first() else {
        return 0;
    };
    if !factor.is_finite() || factor <= 1.0 {
        return 0;
    }
    let limit = factor * (1.0 + RES_TOLERANCE);
    let mut best = 0;
    for (i, lvl) in header.levels.iter().enumerate().skip(1) {
        let fx = f64::from(base.width) / f64::from(lvl.width);
        let fy = f64::from(base.height) / f64::from(lvl.height);
        if fx.max(fy) <= limit {
            best = i;
        }
    }
    best
}

/// Номер уровня для нужного шага res в единицах системы файла, правило
/// выбора обзора GDAL: самый грубый обзор с шагом не крупнее res. Без
/// привязки - уровень 0.
pub fn level_for_resolution(header: &CogHeader, res: f64) -> usize {
    match header.level_resolution(0) {
        Some((rx, ry)) if rx > 0.0 && ry > 0.0 => level_for_factor(header, res / rx.max(ry)),
        _ => 0,
    }
}

/// Окно пикселей уровня, покрывающее рамку bbox [xmin, ymin, xmax, ymax]
/// в системе файла, обрезанное по снимку. None - рамка вне снимка или
/// нет привязки.
pub fn bbox_window(header: &CogHeader, level: usize, bbox: [f64; 4]) -> Option<Window> {
    let lvl = header.level(level)?;
    let g = header.level_transform(level)?;
    let det = g[1] * g[5] - g[2] * g[4];
    if !det.is_finite() || det == 0.0 || bbox.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let [xmin, ymin, xmax, ymax] = bbox;
    let corners = [(xmin, ymin), (xmin, ymax), (xmax, ymin), (xmax, ymax)];
    let (mut c0, mut c1, mut r0, mut r1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for (x, y) in corners {
        let (dx, dy) = (x - g[0], y - g[3]);
        let col = (g[5] * dx - g[2] * dy) / det;
        let row = (g[1] * dy - g[4] * dx) / det;
        c0 = c0.min(col);
        c1 = c1.max(col);
        r0 = r0.min(row);
        r1 = r1.max(row);
    }
    let lo = |v: f64, max: u32| (v + SNAP).floor().clamp(0.0, f64::from(max));
    let hi = |v: f64, max: u32| (v - SNAP).ceil().clamp(0.0, f64::from(max));
    let (x0, x1) = (lo(c0, lvl.width), hi(c1, lvl.width));
    let (y0, y1) = (lo(r0, lvl.height), hi(r1, lvl.height));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(Window {
        x0: x0 as u32,
        y0: y0 as u32,
        width: (x1 - x0) as u32,
        height: (y1 - y0) as u32,
    })
}

/// Привязка окна уровня.
pub fn window_transform(header: &CogHeader, level: usize, window: &Window) -> Option<[f64; 6]> {
    let g = header.level_transform(level)?;
    let (x, y) = (f64::from(window.x0), f64::from(window.y0));
    Some([
        g[0] + x * g[1] + y * g[2],
        g[1],
        g[2],
        g[3] + x * g[4] + y * g[5],
        g[4],
        g[5],
    ])
}

/// Уровень и проверка окна.
fn checked_window(
    header: &CogHeader,
    level: usize,
    x0: u32,
    y0: u32,
    width: u32,
    height: u32,
    sample: u16,
) -> Result<&Level, CogError> {
    let lvl = header.level(level).ok_or(CogError::NoLevel)?;
    let inside = width > 0
        && height > 0
        && u64::from(x0) + u64::from(width) <= u64::from(lvl.width)
        && u64::from(y0) + u64::from(height) <= u64::from(lvl.height)
        && sample < lvl.samples;
    if !inside {
        return Err(CogError::Window);
    }
    if u64::from(width) * u64::from(height) > MAX_WINDOW_PIXELS {
        return Err(CogError::TooLarge);
    }
    Ok(lvl)
}

/// Тайлы окна: (номер, tx, ty) по строкам тайлов.
fn window_tiles(
    lvl: &Level,
    x0: u32,
    y0: u32,
    width: u32,
    height: u32,
    sample: u16,
) -> Result<Vec<(usize, u64, u64)>, CogError> {
    let (tw, th) = (u64::from(lvl.tile_width), u64::from(lvl.tile_height));
    let (tx0, tx1) = (
        u64::from(x0) / tw,
        (u64::from(x0) + u64::from(width) - 1) / tw,
    );
    let (ty0, ty1) = (
        u64::from(y0) / th,
        (u64::from(y0) + u64::from(height) - 1) / th,
    );
    let mut out = Vec::new();
    for ty in ty0..=ty1 {
        for tx in tx0..=tx1 {
            let index = lvl
                .tile_index(tx, ty, sample)
                .filter(|i| *i < lvl.tile_offsets.len())
                .ok_or(CogError::Corrupt("номер тайла"))?;
            out.push((index, tx, ty));
        }
    }
    Ok(out)
}

/// Тайлы первого канала, покрывающие окно уровня level.
pub fn window_plan(
    header: &CogHeader,
    level: usize,
    x0: u32,
    y0: u32,
    width: u32,
    height: u32,
) -> Result<Vec<TileRequest>, CogError> {
    window_plan_sample(header, level, x0, y0, width, height, 0)
}

/// Тайлы канала sample, покрывающие окно. Пустые тайлы (длина 0) не
/// входят, их не нужно читать.
pub fn window_plan_sample(
    header: &CogHeader,
    level: usize,
    x0: u32,
    y0: u32,
    width: u32,
    height: u32,
    sample: u16,
) -> Result<Vec<TileRequest>, CogError> {
    let lvl = checked_window(header, level, x0, y0, width, height, sample)?;
    let mut out = Vec::new();
    for (index, _, _) in window_tiles(lvl, x0, y0, width, height, sample)? {
        let offset = lvl.tile_offsets.get(index).copied().unwrap_or(0);
        let count = lvl.tile_byte_counts.get(index).copied().unwrap_or(0);
        if count == 0 || offset == 0 {
            continue;
        }
        let end = offset
            .checked_add(count)
            .ok_or(CogError::Corrupt("смещение тайла"))?;
        out.push(TileRequest {
            index,
            range: offset..end,
        });
    }
    Ok(out)
}

/// Окно первого канала уровня level, сырые значения в f64. tiles - байты
/// тайла по номеру из [`window_plan`].
pub fn read_window<'a>(
    header: &CogHeader,
    level: usize,
    x0: u32,
    y0: u32,
    width: u32,
    height: u32,
    tiles: &dyn Fn(usize) -> Option<&'a [u8]>,
) -> Result<Grid<f64>, CogError> {
    read_window_sample(header, level, x0, y0, width, height, 0, tiles)
}

/// Окно канала sample уровня level. Пустой тайл заполняется значением
/// «нет данных» или 0, как в GDAL.
#[allow(clippy::too_many_arguments)]
pub fn read_window_sample<'a>(
    header: &CogHeader,
    level: usize,
    x0: u32,
    y0: u32,
    width: u32,
    height: u32,
    sample: u16,
    tiles: &dyn Fn(usize) -> Option<&'a [u8]>,
) -> Result<Grid<f64>, CogError> {
    let lvl = checked_window(header, level, x0, y0, width, height, sample)?;
    let fmt = Fmt {
        big: header.big_endian,
        bigtiff: header.bigtiff,
    };
    let (w, h) = (width as usize, height as usize);
    let fill = header.nodata.unwrap_or(0.0);
    let mut out = vec![fill; w * h];
    let (tw, th) = (u64::from(lvl.tile_width), u64::from(lvl.tile_height));
    for (index, tx, ty) in window_tiles(lvl, x0, y0, width, height, sample)? {
        let offset = lvl.tile_offsets.get(index).copied().unwrap_or(0);
        let count = lvl.tile_byte_counts.get(index).copied().unwrap_or(0);
        let values = if count == 0 || offset == 0 {
            None
        } else {
            let bytes = tiles(index).ok_or(CogError::MissingTile(index))?;
            let data = usize::try_from(count)
                .ok()
                .and_then(|n| bytes.get(..n))
                .ok_or(CogError::Decode(index))?;
            Some(decode_tile(fmt, lvl, data, sample).map_err(|_| CogError::Decode(index))?)
        };
        // Пересечение тайла с окном в пикселях уровня.
        let (px0, py0) = (tx * tw, ty * th);
        let cx0 = px0.max(u64::from(x0));
        let cx1 = (px0 + tw).min(u64::from(x0) + u64::from(width));
        let cy0 = py0.max(u64::from(y0));
        let cy1 = (py0 + th).min(u64::from(y0) + u64::from(height));
        for y in cy0..cy1 {
            let dst_row = (y - u64::from(y0)) as usize * w;
            let src_row = (y - py0) as usize * tw as usize;
            for x in cx0..cx1 {
                let dst = dst_row + (x - u64::from(x0)) as usize;
                let value = match &values {
                    Some(v) => *v
                        .get(src_row + (x - px0) as usize)
                        .ok_or(CogError::Decode(index))?,
                    None => fill,
                };
                if let Some(cell) = out.get_mut(dst) {
                    *cell = value;
                }
            }
        }
    }
    Grid::from_vec(h, w, out).ok_or(CogError::TooLarge)
}

/// Распаковать тайл и вернуть значения канала sample по строкам тайла.
fn decode_tile(fmt: Fmt, lvl: &Level, data: &[u8], sample: u16) -> Result<Vec<f64>, ()> {
    let spt = match lvl.planar {
        Planar::Chunky => usize::from(lvl.samples),
        Planar::Separate => 1,
    };
    let channel = match lvl.planar {
        Planar::Chunky => usize::from(sample),
        Planar::Separate => 0,
    };
    let bps = usize::from(lvl.bits / 8);
    let (tw, th) = (lvl.tile_width as usize, lvl.tile_height as usize);
    let row_samples = tw.checked_mul(spt).ok_or(())?;
    let row_bytes = row_samples.checked_mul(bps).ok_or(())?;
    let total = row_bytes.checked_mul(th).ok_or(())?;
    if total as u64 > MAX_TILE_BYTES || row_bytes == 0 {
        return Err(());
    }
    let raw = decompress(lvl.compression, data, total)?;
    let bits = u32::from(lvl.bits);
    let mask = if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    let mut out = Vec::with_capacity(tw * th);
    let mut tmp = Vec::new();
    for row in raw.chunks_exact(row_bytes) {
        match lvl.predictor {
            Predictor::None | Predictor::Horizontal => {
                let mut prev = 0u64;
                for c in 0..tw {
                    let at = (c * spt + channel) * bps;
                    let v = fmt.uint(row.get(at..at + bps).ok_or(())?);
                    let v = if lvl.predictor == Predictor::Horizontal {
                        prev.wrapping_add(v) & mask
                    } else {
                        v
                    };
                    prev = v;
                    out.push(to_f64(lvl.format, bits, v));
                }
            }
            Predictor::FloatingPoint => {
                // Разности байтов с шагом spt по всей строке, затем байты
                // отсчёта собираются из bps плоскостей, старший первым.
                tmp.clear();
                tmp.extend_from_slice(row);
                for i in spt..tmp.len() {
                    tmp[i] = tmp[i].wrapping_add(tmp[i - spt]);
                }
                for c in 0..tw {
                    let k = c * spt + channel;
                    let mut v = 0u64;
                    for j in 0..bps {
                        v = (v << 8) | u64::from(*tmp.get(j * row_samples + k).ok_or(())?);
                    }
                    out.push(to_f64(lvl.format, bits, v));
                }
            }
        }
    }
    if out.len() != tw * th {
        return Err(());
    }
    Ok(out)
}

/// Отсчёт из bits младших бит v.
fn to_f64(format: SampleFormat, bits: u32, v: u64) -> f64 {
    match format {
        SampleFormat::Uint => v as f64,
        SampleFormat::Int => {
            let shift = 64 - bits;
            (((v << shift) as i64) >> shift) as f64
        }
        SampleFormat::Float => {
            if bits == 32 {
                f64::from(f32::from_bits(v as u32))
            } else {
                f64::from_bits(v)
            }
        }
    }
}

/// Распаковать данные тайла ровно в total байт.
fn decompress(compression: Compression, data: &[u8], total: usize) -> Result<Vec<u8>, ()> {
    match compression {
        Compression::None => data.get(..total).map(<[u8]>::to_vec).ok_or(()),
        Compression::Deflate => {
            let mut out = Vec::with_capacity(total);
            flate2::read::ZlibDecoder::new(data)
                .take(total as u64)
                .read_to_end(&mut out)
                .map_err(|_| ())?;
            if out.len() == total { Ok(out) } else { Err(()) }
        }
        Compression::Lzw => {
            let mut decoder =
                weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
            let mut out = vec![0u8; total];
            let (mut read, mut written) = (0usize, 0usize);
            while written < total {
                let (Some(inp), Some(dst)) = (data.get(read..), out.get_mut(written..)) else {
                    return Err(());
                };
                let r = decoder.decode_bytes(inp, dst);
                read += r.consumed_in;
                written += r.consumed_out;
                match r.status {
                    Err(_) => return Err(()),
                    Ok(weezl::LzwStatus::Done) | Ok(weezl::LzwStatus::NoProgress) => break,
                    Ok(weezl::LzwStatus::Ok) => {
                        if r.consumed_in == 0 && r.consumed_out == 0 {
                            break;
                        }
                    }
                }
            }
            if written == total { Ok(out) } else { Err(()) }
        }
        Compression::Zstd => {
            let mut out = vec![0u8; total];
            let n = ruzstd::decoding::FrameDecoder::new()
                .decode_all(data, &mut out)
                .map_err(|_| ())?;
            if n == total { Ok(out) } else { Err(()) }
        }
    }
}
