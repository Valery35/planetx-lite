// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Свой растр: GeoTIFF или COG из байтов файла ([`crate::rasterfile`]).
//!
//! Растр - сетка значений первого канала в f32 по строкам сверху вниз,
//! «нет данных» - NaN. Привязка - как у GDAL: `[x0, шаг x, поворот,
//! y0, поворот, шаг y]`, координаты - в системе растра. Пересчёт
//! в широту и долготу WGS84 - [`crate::crs`].
//!
//! Файл считается враждебным: растр больше [`MAX_PIXELS`] точек
//! не берётся.

use crate::crs::{SpatialRef, Transformer};
use crate::rasterfile::RasterFile;

/// Наибольшее количество точек растра.
pub const MAX_PIXELS: usize = 64 << 20;

/// Растр в памяти.
pub struct Grid {
    pub width: usize,
    pub height: usize,
    /// Значения первого канала по строкам, NaN - нет данных.
    pub values: Vec<f32>,
    /// Привязка GDAL.
    pub transform: [f64; 6],
    /// Код EPSG системы координат, если он известен.
    pub epsg: Option<u32>,
    /// Пересчёт в WGS84, None - система неизвестна.
    to_wgs84: Option<Transformer>,
    /// Система координат растра.
    crs: SpatialRef,
}

impl std::fmt::Debug for Grid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Grid").field("width", &self.width).field("height", &self.height).field("epsg", &self.epsg).finish()
    }
}

/// Пересчёт в WGS84 (долгота, широта) из системы src.
fn wgs84_from(src: &SpatialRef) -> Option<Transformer> {
    Transformer::new(src, &SpatialRef::from_epsg(4326)?)
}

impl Grid {
    /// Растр из байтов файла. name - имя файла, по нему узнаётся формат.
    pub fn read(bytes: Vec<u8>, name: &str) -> Result<Grid, String> {
        let file = RasterFile::open(bytes, name)?;
        let (width, height, nodata) = (file.cols, file.rows, file.nodata);
        let mut values = Vec::with_capacity(width * height);
        for row in 0..height {
            for col in 0..width {
                let v = file.get(0, row, col);
                let missing = !v.is_finite() || nodata.is_some_and(|n| v == n);
                values.push(if missing { f32::NAN } else { v as f32 });
            }
        }
        let epsg = file.crs.epsg();
        let to_wgs84 = wgs84_from(&file.crs);
        Ok(Grid { width, height, values, transform: file.transform, epsg, to_wgs84, crs: file.crs })
    }

    /// Значение в точке (столбец, строка) с долями, билинейно по центрам
    /// точек. NaN - вне растра или у соседей нет данных.
    pub fn bilinear(&self, col: f64, row: f64) -> f32 {
        blend(self.width, self.height, col, row, |x, y| self.values.get(y * self.width + x).copied().unwrap_or(f32::NAN))
    }

    /// Столбец и строка растра с долями для точки (x, y) его системы.
    /// None - привязка вырождена.
    pub fn pixel_of(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        pixel_of(&self.transform, x, y)
    }

    /// Пересчёт из системы EPSG code в систему растра.
    pub fn transformer_from(&self, code: u32) -> Option<Transformer> {
        Transformer::new(&SpatialRef::from_epsg(code)?, &self.crs)
    }

    /// Пересчёт из системы растра в систему EPSG code.
    pub fn transformer_to(&self, code: u32) -> Option<Transformer> {
        Transformer::new(&self.crs, &SpatialRef::from_epsg(code)?)
    }

    /// Пересчёт из системы этого растра в систему растра other. None -
    /// системы те же или одна из них неизвестна.
    pub fn transformer_into(&self, other: &Grid) -> Option<Transformer> {
        if self.epsg.is_some() && self.epsg == other.epsg {
            return None;
        }
        Transformer::new(&self.crs, &other.crs)
    }

    /// Значение точки (столбец, строка), None - вне растра или нет данных.
    pub fn value(&self, col: usize, row: usize) -> Option<f32> {
        if col >= self.width || row >= self.height {
            return None;
        }
        self.values.get(row * self.width + col).copied().filter(|v| v.is_finite())
    }

    /// Наименьшее, наибольшее и среднее по точкам с данными.
    pub fn stats(&self) -> Option<(f64, f64, f64)> {
        let mut n = 0usize;
        let (mut lo, mut hi, mut sum) = (f64::INFINITY, f64::NEG_INFINITY, 0.0);
        for v in self.values.iter().filter(|v| v.is_finite()) {
            let v = f64::from(*v);
            lo = lo.min(v);
            hi = hi.max(v);
            sum += v;
            n += 1;
        }
        (n > 0).then(|| (lo, hi, sum / n as f64))
    }

    /// Точка растра в его системе: доли столбца и строки от левого
    /// верхнего угла, 0.5 - середина точки.
    pub fn at(&self, col: f64, row: f64) -> (f64, f64) {
        let t = &self.transform;
        (t[0] + col * t[1] + row * t[2], t[3] + col * t[4] + row * t[5])
    }

    /// Широта и долгота WGS84 точки (x, y) системы растра.
    pub fn lat_lon(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let (lon, lat) = self.to_wgs84.as_ref()?.transform(x, y)?;
        Some((lat, lon))
    }

    /// Широта и долгота углов растра: левый нижний, правый нижний,
    /// правый верхний, левый верхний, как углы gx:LatLonQuad.
    pub fn corners(&self) -> Option<[(f64, f64); 4]> {
        let (w, h) = (self.width as f64, self.height as f64);
        let pts = [(0.0, h), (w, h), (w, 0.0), (0.0, 0.0)].map(|(c, r)| self.at(c, r));
        let mut out = [(0.0, 0.0); 4];
        for (slot, (x, y)) in out.iter_mut().zip(pts) {
            *slot = self.lat_lon(x, y)?;
        }
        Some(out)
    }
}

/// Широты и долготы в метрах проекции EPSG code: x и y. None - система
/// неизвестна.
pub fn to_projected(code: u32, lats: &[f64], lons: &[f64]) -> Option<(Vec<f64>, Vec<f64>)> {
    let t = crate::crs::between(4326, code)?;
    let (mut xs, mut ys) = (Vec::with_capacity(lats.len()), Vec::with_capacity(lats.len()));
    for (lat, lon) in lats.iter().zip(lons) {
        let (x, y) = t.transform(*lon, *lat)?;
        xs.push(x);
        ys.push(y);
    }
    Some((xs, ys))
}

/// Смесь четырёх соседей растра width × height в точке (столбец, строка)
/// с долями, билинейно по центрам точек. at - значение точки. NaN - вне
/// растра или у соседей нет данных.
pub fn blend(width: usize, height: usize, col: f64, row: f64, at: impl Fn(usize, usize) -> f32) -> f32 {
    let (px, py) = (col - 0.5, row - 0.5);
    if width == 0 || height == 0 || !(px > -0.5 && py > -0.5 && px < width as f64 - 0.5 && py < height as f64 - 0.5) {
        return f32::NAN;
    }
    let x0 = px.floor().clamp(0.0, (width.saturating_sub(2)) as f64) as usize;
    let y0 = py.floor().clamp(0.0, (height.saturating_sub(2)) as f64) as usize;
    let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
    let fx = (px - x0 as f64).clamp(0.0, 1.0) as f32;
    let fy = (py - y0 as f64).clamp(0.0, 1.0) as f32;
    let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
    let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
    top * (1.0 - fy) + bottom * fy
}

/// Столбец и строка растра с долями для точки (x, y) его системы по
/// привязке t. None - привязка вырождена.
pub fn pixel_of(t: &[f64; 6], x: f64, y: f64) -> Option<(f64, f64)> {
    let det = t[1] * t[5] - t[2] * t[4];
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let (dx, dy) = (x - t[0], y - t[3]);
    Some(((dx * t[5] - dy * t[2]) / det, (dy * t[1] - dx * t[4]) / det))
}

/// Широта и долгота точки (x, y) системы EPSG code.
pub fn lat_lon_of(code: u32, x: f64, y: f64) -> Option<(f64, f64)> {
    let (lon, lat) = crate::crs::between(code, 4326)?.transform(x, y)?;
    Some((lat, lon))
}
