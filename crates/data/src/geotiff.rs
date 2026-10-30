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

use std::io::Cursor;

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

fn floats(frame: &Frame, values: &[f64]) -> Option<Vec<u8>> {
    if !frame.valid() || values.len() != frame.size() {
        return None;
    }
    let data: Vec<f32> = values.iter().map(|v| if v.is_finite() { *v as f32 } else { NODATA as f32 }).collect();
    write::<colortype::Gray32Float>(frame, &data, &format!("{NODATA}"))
}

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
        dir.write_tag(Tag::Unknown(GDAL_NODATA), nodata).ok()?;
        image.write_data(data).ok()?;
    }
    Some(out.into_inner())
}
