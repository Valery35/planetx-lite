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

use terrio::crs::{SpatialRef, Transformer};
use terrio::pointcloud::{self, PointReadOptions};

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
