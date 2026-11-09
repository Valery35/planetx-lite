// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Запись результата анализа участка файлом GeoTIFF, замена
//! `write_lat_lon` (`ui/results.py`) и `write_grid` (`ui/relief.py`)
//! модуля без GDAL.
//!
//! Сетка - в широтах и долготах EPSG:4326 ([`LatLonGrid`]) или в метрах
//! Web Mercator EPSG:3857 ([`MercatorGrid`], сетки `hydro::Grid`
//! уклона, затопления и стока), строки с севера на юг. Узел сетки -
//! центр пикселя, как у модуля: левый верхний угол растра на полшага
//! к северо-западу от первого узла. Значения - float32 с [`NODATA`] или
//! байты 0-254 с 255 - нет данных. Сжатие deflate. Привязка - теги
//! GeoTIFF ModelPixelScale, ModelTiepoint и GeoKeyDirectory, значение
//! «нет данных» - тег GDAL_NODATA, его читают GDAL, QGIS и модуль.

use std::io::{Cursor, Write};

use flate2::write::ZlibEncoder;
use tiff::encoder::{Compression, DeflateLevel, TiffEncoder, colortype};
use tiff::tags::Tag;

/// Нет данных у растра float32, как `NODATA` модуля.
pub const NODATA: f64 = -9999.0;
/// Нет данных у растра байтов.
pub const BYTE_NODATA: u8 = 255;

/// Тег GDAL_NODATA.
const GDAL_NODATA: u16 = 42113;
/// Ключи GeoKeyDirectory: версия 1.1.0, три ключа - модель
/// географическая, пиксель - площадь, система EPSG:4326.
const LATLON_KEYS: [u16; 16] = [1, 1, 0, 3, 1024, 0, 1, 2, 1025, 0, 1, 1, 2048, 0, 1, 4326];
/// Ключи модели в проекции с кодом EPSG: 3857 у Web Mercator, 326xx
/// и 327xx у зон UTM.
const fn projected_keys(epsg: u16) -> [u16; 16] {
    [1, 1, 0, 3, 1024, 0, 1, 1, 1025, 0, 1, 1, 3072, 0, 1, epsg]
}
/// Строк в полосе файла.
const ROWS_PER_STRIP: u32 = 64;

/// Сетка растра в градусах: широта и долгота первого узла (северо-запад),
/// шаги, столбцов и строк.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatLonGrid {
    pub north: f64,
    pub west: f64,
    pub dlat: f64,
    pub dlon: f64,
    pub cols: usize,
    pub rows: usize,
}

/// Сетка растра в метрах Web Mercator: x и y первого узла
/// (северо-запад), шаг, столбцов и строк.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MercatorGrid {
    pub x0: f64,
    pub y0: f64,
    pub step: f64,
    pub cols: usize,
    pub rows: usize,
}

/// Привязка растра: размер, шаги, угол и ключи системы.
struct Frame {
    cols: usize,
    rows: usize,
    dx: f64,
    dy: f64,
    left: f64,
    top: f64,
    keys: [u16; 16],
}

impl Frame {
    /// Годится ли привязка: величины - конечные числа, шаги больше нуля,
    /// размер не пустой и помещается в TIFF.
    fn valid(&self) -> bool {
        let finite = [self.dx, self.dy, self.left, self.top].iter().all(|v| v.is_finite());
        finite && self.dx > 0.0 && self.dy > 0.0 && self.cols > 0 && self.rows > 0 && u32::try_from(self.cols).is_ok() && u32::try_from(self.rows).is_ok()
    }

    fn size(&self) -> usize {
        self.cols.saturating_mul(self.rows)
    }
}

impl From<&LatLonGrid> for Frame {
    fn from(g: &LatLonGrid) -> Self {
        Frame { cols: g.cols, rows: g.rows, dx: g.dlon, dy: g.dlat, left: g.west - g.dlon / 2.0, top: g.north + g.dlat / 2.0, keys: LATLON_KEYS }
    }
}

impl Frame {
    /// Привязка сетки в метрах проекции с кодом EPSG.
    fn projected(g: &MercatorGrid, epsg: u16) -> Self {
        Frame { cols: g.cols, rows: g.rows, dx: g.step, dy: g.step, left: g.x0 - g.step / 2.0, top: g.y0 + g.step / 2.0, keys: projected_keys(epsg) }
    }
}

/// Растр float32 в градусах: values - строка за строкой с севера,
/// нечисло пишется как [`NODATA`]. None - сетка не годится или значений
/// не столько, сколько узлов.
pub fn write_float(grid: &LatLonGrid, values: &[f64]) -> Option<Vec<u8>> {
    floats(&Frame::from(grid), values)
}

/// Растр float32 в метрах Web Mercator, как [`write_float`].
pub fn write_float_mercator(grid: &MercatorGrid, values: &[f64]) -> Option<Vec<u8>> {
    floats(&Frame::projected(grid, 3857), values)
}

/// Растр float32 в метрах проекции с кодом EPSG, например зоны UTM
/// файлов AlphaEarth, как [`write_float`].
pub fn write_float_projected(grid: &MercatorGrid, epsg: u16, values: &[f64]) -> Option<Vec<u8>> {
    floats(&Frame::projected(grid, epsg), values)
}

/// Растр байтов в метрах проекции с кодом EPSG, как [`write_byte`].
pub fn write_byte_projected(grid: &MercatorGrid, epsg: u16, values: &[u8]) -> Option<Vec<u8>> {
    let frame = Frame::projected(grid, epsg);
    if !frame.valid() || values.len() != frame.size() {
        return None;
    }
    write::<colortype::Gray8>(&frame, values, &BYTE_NODATA.to_string())
}

/// Растр байтов в градусах: values - строка за строкой с севера,
/// [`BYTE_NODATA`] - нет данных. None - сетка не годится или значений
/// не столько, сколько узлов.
pub fn write_byte(grid: &LatLonGrid, values: &[u8]) -> Option<Vec<u8>> {
    let frame = Frame::from(grid);
    if !frame.valid() || values.len() != frame.size() {
        return None;
    }
    write::<colortype::Gray8>(&frame, values, &BYTE_NODATA.to_string())
}

/// Многоканальный растр float32 в метрах проекции с кодом EPSG: каналы
/// с подписями, значения каждого - строка за строкой с севера, нечисло
/// пишется как [`NODATA`]. Подписи - описания каналов в теге
/// GDAL_METADATA, их читают GDAL, QGIS и terrio. Каналы лежат
/// плоскостями (PlanarConfiguration 2), полосы сжаты deflate. Так пишет
/// «Все каналы сцены» окна «Минералы по спектру». None - сетка не годится,
/// каналов нет или значений не столько, сколько узлов.
pub fn write_bands_projected(grid: &MercatorGrid, epsg: u16, bands: &[(String, Vec<f32>)]) -> Option<Vec<u8>> {
    let frame = Frame::projected(grid, epsg);
    let count = u16::try_from(bands.len()).ok().filter(|n| *n > 0)?;
    if !frame.valid() || bands.iter().any(|b| b.1.len() != frame.size()) {
        return None;
    }
    let (cols, rows) = (frame.cols, frame.rows);
    let per = ROWS_PER_STRIP as usize;
    let mut out: Vec<u8> = vec![b'I', b'I', 42, 0, 0, 0, 0, 0];
    let mut offsets: Vec<u32> = Vec::new();
    let mut counts: Vec<u32> = Vec::new();
    for (_, values) in bands {
        for part in values.chunks(per * cols) {
            let mut encoder = ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            for v in part {
                let v = if v.is_finite() { *v } else { NODATA as f32 };
                encoder.write_all(&v.to_le_bytes()).ok()?;
            }
            let packed = encoder.finish().ok()?;
            offsets.push(u32::try_from(out.len()).ok()?);
            counts.push(u32::try_from(packed.len()).ok()?);
            out.extend_from_slice(&packed);
        }
    }
    let n = usize::from(count);
    let mut xml = String::from("<GDALMetadata>\n");
    for (i, (label, _)) in bands.iter().enumerate() {
        let text = label.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        xml.push_str(&format!("  <Item name=\"DESCRIPTION\" sample=\"{i}\" role=\"description\">{text}</Item>\n"));
    }
    xml.push_str("</GDALMetadata>");
    let long = |v: u32| v.to_le_bytes().to_vec();
    let shorts = |v: &[u16]| v.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<u8>>();
    let longs = |v: &[u32]| v.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<u8>>();
    let doubles = |v: &[f64]| v.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<u8>>();
    let ascii = |s: &str| {
        let mut b = s.as_bytes().to_vec();
        b.push(0);
        b
    };
    // Теги по возрастанию номера: номер, тип (2 ASCII, 3 SHORT, 4 LONG,
    // 12 DOUBLE), количество, значения.
    let entries: Vec<(u16, u16, usize, Vec<u8>)> = vec![
        (256, 4, 1, long(u32::try_from(cols).ok()?)),
        (257, 4, 1, long(u32::try_from(rows).ok()?)),
        (258, 3, n, shorts(&vec![32; n])),
        (259, 3, 1, shorts(&[8])),
        (262, 3, 1, shorts(&[1])),
        (273, 4, offsets.len(), longs(&offsets)),
        (277, 3, 1, shorts(&[count])),
        (278, 4, 1, long(ROWS_PER_STRIP.min(u32::try_from(rows).ok()?))),
        (279, 4, counts.len(), longs(&counts)),
        (284, 3, 1, shorts(&[2])),
        (339, 3, n, shorts(&vec![3; n])),
        (33550, 12, 3, doubles(&[frame.dx, frame.dy, 0.0])),
        (33922, 12, 6, doubles(&[0.0, 0.0, 0.0, frame.left, frame.top, 0.0])),
        (34735, 3, 16, shorts(&frame.keys)),
        (42112, 2, xml.len() + 1, ascii(&xml)),
        (GDAL_NODATA, 2, format!("{NODATA}").len() + 1, ascii(&format!("{NODATA}"))),
    ];
    if out.len() % 2 == 1 {
        out.push(0);
    }
    let ifd = out.len();
    out.get_mut(4..8)?.copy_from_slice(&u32::try_from(ifd).ok()?.to_le_bytes());
    // Значения длиннее 4 байт - после каталога, с чётных мест.
    let mut extra = ifd + 2 + entries.len() * 12 + 4;
    let mut tail: Vec<u8> = Vec::new();
    out.extend_from_slice(&u16::try_from(entries.len()).ok()?.to_le_bytes());
    for (tag, kind, number, data) in &entries {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&u32::try_from(*number).ok()?.to_le_bytes());
        if data.len() <= 4 {
            let mut inline = data.clone();
            inline.resize(4, 0);
            out.extend_from_slice(&inline);
        } else {
            out.extend_from_slice(&u32::try_from(extra).ok()?.to_le_bytes());
            tail.extend_from_slice(data);
            if tail.len() % 2 == 1 {
                tail.push(0);
            }
            extra = ifd + 2 + entries.len() * 12 + 4 + tail.len();
        }
    }
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&tail);
    Some(out)
}

/// Цветной растр RGBA в градусах - картинка слоя, как её видно на
/// глобусе: pixels - по четыре байта на узел (красный, зелёный, синий,
/// непрозрачность без премножения), строки с севера. Четвёртый канал
/// помечен прозрачностью (ExtraSamples 2), QGIS и GDAL берут его сами.
/// None - сетка не годится или байтов не вчетверо больше узлов.
pub fn write_rgba(grid: &LatLonGrid, pixels: &[u8]) -> Option<Vec<u8>> {
    let frame = Frame::from(grid);
    if !frame.valid() || pixels.len() != frame.size().checked_mul(4)? {
        return None;
    }
    write::<colortype::RGBA8>(&frame, pixels, "")
}

fn floats(frame: &Frame, values: &[f64]) -> Option<Vec<u8>> {
    if !frame.valid() || values.len() != frame.size() {
        return None;
    }
    let data: Vec<f32> = values.iter().map(|v| if v.is_finite() { *v as f32 } else { NODATA as f32 }).collect();
    write::<colortype::Gray32Float>(frame, &data, &format!("{NODATA}"))
}

/// Запись растра. Пустой nodata - тега GDAL_NODATA нет, у цветного
/// растра RGBA вместо него четвёртый канал прозрачности.
fn write<C: colortype::ColorType>(frame: &Frame, data: &[C::Inner], nodata: &str) -> Option<Vec<u8>>
where
    [C::Inner]: tiff::encoder::TiffValue,
{
    let (cols, rows) = (u32::try_from(frame.cols).ok()?, u32::try_from(frame.rows).ok()?);
    let mut out = Cursor::new(Vec::new());
    {
        let mut encoder = TiffEncoder::new(&mut out).ok()?.with_compression(Compression::Deflate(DeflateLevel::Balanced));
        let mut image = encoder.new_image::<C>(cols, rows).ok()?;
        image.rows_per_strip(ROWS_PER_STRIP.min(rows)).ok()?;
        let dir = image.encoder();
        dir.write_tag(Tag::ModelPixelScaleTag, &[frame.dx, frame.dy, 0.0][..]).ok()?;
        dir.write_tag(Tag::ModelTiepointTag, &[0.0, 0.0, 0.0, frame.left, frame.top, 0.0][..]).ok()?;
        dir.write_tag(Tag::GeoKeyDirectoryTag, &frame.keys[..]).ok()?;
        if nodata.is_empty() {
            // Четвёртый канал - непрозрачность без премножения.
            dir.write_tag(Tag::ExtraSamples, 2u16).ok()?;
        } else {
            dir.write_tag(Tag::Unknown(GDAL_NODATA), nodata).ok()?;
        }
        image.write_data(data).ok()?;
    }
    Some(out.into_inner())
}
