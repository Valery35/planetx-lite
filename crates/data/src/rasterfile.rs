// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Растр из байтов файла, одинаково для обеих сборок.
//!
//! С признаком `import` (PlanetX Lite) растр читает terrio - GeoTIFF,
//! COG, LERC и прочие его форматы. Без него (RustPlanetX) - свой разбор
//! [`crate::cog`]: TIFF с тайлами, как его выгружает RustGIS командой
//! `share.exportRaster` с `cog`. Система координат - по коду EPSG
//! из ключей GeoTIFF ([`crate::crs`]).
//!
//! Файл считается враждебным: растр больше [`crate::grid::MAX_PIXELS`]
//! точек не берётся.

use crate::crs::SpatialRef;

/// Растр в памяти: все каналы, привязка GDAL, система координат.
pub struct RasterFile {
    pub cols: usize,
    pub rows: usize,
    pub bands: usize,
    /// Привязка GDAL: x0, шаг x, поворот, y0, поворот, шаг y.
    pub transform: [f64; 6],
    pub nodata: Option<f64>,
    pub crs: SpatialRef,
    /// Подписи каналов, пусто - подписи нет.
    labels: Vec<String>,
    #[cfg(feature = "import")]
    data: terrio::RasterDataset,
    /// Значения каналов по строкам.
    #[cfg(not(feature = "import"))]
    values: Vec<Vec<f64>>,
}

impl RasterFile {
    /// Значение канала band в точке (строка, столбец), сырое. NaN - вне
    /// растра.
    pub fn get(&self, band: usize, row: usize, col: usize) -> f64 {
        if band >= self.bands || row >= self.rows || col >= self.cols {
            return f64::NAN;
        }
        #[cfg(feature = "import")]
        {
            self.data.raster.get(band as isize, row as isize, col as isize)
        }
        #[cfg(not(feature = "import"))]
        {
            self.values.get(band).and_then(|v| v.get(row * self.cols + col)).copied().unwrap_or(f64::NAN)
        }
    }

    /// Подпись канала band: описание канала файла, пусто - подписи нет.
    pub fn label(&self, band: usize) -> String {
        self.labels.get(band).cloned().unwrap_or_default()
    }

    fn checked(cols: usize, rows: usize) -> Result<(), String> {
        if cols == 0 || rows == 0 || cols.saturating_mul(rows) > crate::grid::MAX_PIXELS {
            return Err(format!("растр {cols} × {rows} не берётся"));
        }
        Ok(())
    }
}

#[cfg(feature = "import")]
impl RasterFile {
    /// Растр из байтов файла. name - имя файла, по нему terrio узнаёт
    /// формат.
    pub fn open(bytes: Vec<u8>, name: &str) -> Result<Self, String> {
        let data = terrio::memory::open_raster_bytes(bytes, name).map_err(|e| e.to_string())?;
        let (cols, rows, bands) = (data.raster.cols, data.raster.rows, data.raster.bands);
        Self::checked(cols, rows)?;
        let labels = (0..bands).map(|b| data.band_metadata.get(b).and_then(|m| m.description.clone()).unwrap_or_default()).collect();
        let (transform, nodata, crs) = (data.geo_transform(), data.nodata(), SpatialRef::from_terrio(data.crs()));
        Ok(Self { cols, rows, bands, transform, nodata, crs, labels, data })
    }
}

#[cfg(not(feature = "import"))]
impl RasterFile {
    /// Растр из байтов файла GeoTIFF с тайлами. name не участвует: формат
    /// один.
    pub fn open(bytes: Vec<u8>, _name: &str) -> Result<Self, String> {
        use crate::cog::{CogHeader, Parsed, read_window_sample};
        let header = match CogHeader::parse(&bytes).map_err(|e| e.to_string())? {
            Parsed::Header(h) => h,
            Parsed::NeedMore(_) => return Err("растр: файл обрезан".into()),
        };
        let level = header.levels.first().ok_or("растр: нет изображения")?;
        let (cols, rows) = (level.width as usize, level.height as usize);
        Self::checked(cols, rows)?;
        let transform = header.geotransform.ok_or("у растра нет привязки")?;
        let bands = usize::from(level.samples.max(1));
        let tile = |i: usize| -> Option<&[u8]> {
            let start = usize::try_from(*level.tile_offsets.get(i)?).ok()?;
            let len = usize::try_from(*level.tile_byte_counts.get(i)?).ok()?;
            bytes.get(start..start.checked_add(len)?)
        };
        let mut values = Vec::with_capacity(bands);
        for band in 0..level.samples.max(1) {
            let grid = read_window_sample(&header, 0, 0, 0, level.width, level.height, band, &tile).map_err(|e| e.to_string())?;
            values.push(grid.data().to_vec());
        }
        let crs = header.epsg.and_then(SpatialRef::from_epsg).unwrap_or_else(SpatialRef::unknown);
        Ok(Self { cols, rows, bands, transform, nodata: header.nodata, crs, labels: Vec::new(), values })
    }
}
