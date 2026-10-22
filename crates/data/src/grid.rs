// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Свой растр: GeoTIFF или COG из байтов файла, через terrio.
//!
//! Растр - сетка значений первого канала в f32 по строкам сверху вниз,
//! «нет данных» - NaN. Привязка - как у GDAL: `[x0, шаг x, поворот,
//! y0, поворот, шаг y]`, координаты - в системе растра. Пересчёт
//! в широту и долготу WGS84 - движок систем координат terrio, он сверен
//! с GDAL (doc/PLAN.md, «Проверка terrio»).
//!
//! Файл считается враждебным: растр больше [`MAX_PIXELS`] точек
//! не берётся.

use terrio::crs::{SpatialRef, Transformer};

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
}

impl std::fmt::Debug for Grid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Grid").field("width", &self.width).field("height", &self.height).field("epsg", &self.epsg).finish()
    }
}

/// Транспонер в WGS84 (долгота, широта) из системы src.
fn wgs84_from(src: &SpatialRef) -> Option<Transformer> {
    if src.is_unknown() {
        return None;
    }
    let dst = SpatialRef::from_epsg(4326).ok()?;
    Transformer::new(src, &dst).ok()
}

impl Grid {
    /// Растр из байтов файла. name - имя файла, по нему terrio узнаёт
    /// формат.
    pub fn read(bytes: Vec<u8>, name: &str) -> Result<Grid, String> {
        let ds = terrio::memory::open_raster_bytes(bytes, name).map_err(|e| e.to_string())?;
        let r = &ds.raster;
        let (width, height) = (r.cols, r.rows);
        if width == 0 || height == 0 || width.saturating_mul(height) > MAX_PIXELS {
            return Err(format!("растр {width} × {height} не берётся"));
        }
        let nodata = ds.nodata();
        let mut values = Vec::with_capacity(width * height);
        for row in 0..height as isize {
            for col in 0..width as isize {
                let v = r.get(0, row, col);
                let missing = !v.is_finite() || nodata.is_some_and(|n| v == n);
                values.push(if missing { f32::NAN } else { v as f32 });
            }
        }
        let crs = ds.crs();
        let epsg = crs.authority().filter(|(a, _)| a.eq_ignore_ascii_case("EPSG")).map(|(_, c)| c);
        Ok(Grid { width, height, values, transform: ds.geo_transform(), epsg, to_wgs84: wgs84_from(&crs) })
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
        let (lon, lat) = self.to_wgs84.as_ref()?.transform(x, y).ok()?;
        (lat.is_finite() && lon.is_finite()).then_some((lat, lon))
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

/// Широта и долгота точки (x, y) системы EPSG code.
pub fn lat_lon_of(code: u32, x: f64, y: f64) -> Option<(f64, f64)> {
    let src = SpatialRef::from_epsg(code).ok()?;
    let (lon, lat) = wgs84_from(&src)?.transform(x, y).ok()?;
    Some((lat, lon))
}
