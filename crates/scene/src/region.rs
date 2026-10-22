// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Region KML: метка или папка видна только вблизи. Перенос
//! `core/region.py` модуля.
//!
//! Region - рамка LatLonAltBox и Lod с minLodPixels и maxLodPixels.
//! Метка видна, когда размер рамки на экране, корень из её площади
//! в пикселях, не меньше minLodPixels и не больше maxLodPixels, -1 -
//! без предела. Так делает Google Earth.
//!
//! Размер рамки считается с постоянным фокусным расстоянием
//! [`REF_FOCAL`], а не по окну глобуса. Тогда поле «Скрывать дальше, км»
//! окна свойств действует на том расстоянии, которое в нём записано,
//! при любом окне. [`REF_FOCAL`] - окно высотой 1000 пикселей с полем
//! зрения 60°. Поле пишется в KML тем же Region ([`for_distance`]),
//! Google Earth читает его так же. Расстояние - от глаза до середины
//! рамки на уровне эллипсоида.

use globe_core::DVec3;
use globe_core::ellipsoid::Ellipsoid;
use globe_core::flight::{lon_span, wrap_lon};
use serde_json::Value;

use crate::util::py_round;

pub const M_PER_DEGREE: f64 = 111320.0;
/// Фокусное расстояние окна высотой 1000 пикселей с полем 60°,
/// 500 / tg 30°, около 866 пикселей.
pub const REF_FOCAL: f64 = 866.025_403_784_438_6;
/// Сторона рамки точки или очень малой фигуры, метры.
pub const BOX_SIDE: f64 = 1000.0;

/// Region KML: рамка в градусах и пределы Lod в пикселях, max_lod -1 -
/// без предела.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    pub north: f64,
    pub south: f64,
    pub east: f64,
    pub west: f64,
    pub min_lod: f64,
    pub max_lod: f64,
}

impl Region {
    pub fn new(north: f64, south: f64, east: f64, west: f64, min_lod: f64, max_lod: f64) -> Self {
        Self { north, south, east, west, min_lod, max_lod }
    }

    /// Шесть чисел по порядку полей.
    pub fn values(&self) -> [f64; 6] {
        [self.north, self.south, self.east, self.west, self.min_lod, self.max_lod]
    }

    /// Рамка без Lod: север, юг, восток, запад.
    pub fn rect(&self) -> [f64; 4] {
        [self.north, self.south, self.east, self.west]
    }
}

/// Ширина и высота рамки в метрах.
pub fn span(region: &Region) -> (f64, f64) {
    let east = if region.east >= region.west { region.east } else { region.east + 360.0 };
    let middle = ((region.north + region.south) / 2.0).to_radians();
    let width = (east - region.west) * M_PER_DEGREE * middle.cos();
    let height = (region.north - region.south) * M_PER_DEGREE;
    (width.max(0.0), height.max(0.0))
}

/// Корень из площади рамки, метры.
pub fn side(region: &Region) -> f64 {
    let (width, height) = span(region);
    (width * height).sqrt()
}

/// Середина рамки: широта, долгота.
pub fn center(region: &Region) -> (f64, f64) {
    let east = if region.east >= region.west { region.east } else { region.east + 360.0 };
    let lon = (region.west + east) / 2.0;
    ((region.north + region.south) / 2.0, (lon + 180.0).rem_euclid(360.0) - 180.0)
}

/// Размер рамки на экране в пикселях при глазе eye (ECEF, метры).
pub fn pixels(region: &Region, eye: DVec3) -> f64 {
    let (lat, lon) = center(region);
    let point = Ellipsoid::wgs84().geodetic_to_ecef(lat, lon, 0.0);
    side(region) * REF_FOCAL / eye.distance(point).max(1.0)
}

/// Видна ли метка с Region при глазе eye. Без Region - видна.
pub fn active(region: Option<&Region>, eye: DVec3) -> bool {
    let Some(region) = region else {
        return true;
    };
    let size = pixels(region, eye);
    if size < region.min_lod {
        return false;
    }
    region.max_lod < 0.0 || size <= region.max_lod
}

/// Рамка точек (широта, долгота): (север, юг, восток, запад). Малая
/// рамка растягивается до [`BOX_SIDE`] метров.
pub fn bounding_box(points: &[(f64, f64)]) -> (f64, f64, f64, f64) {
    let lats = points.iter().map(|p| p.0);
    let mut north = lats.clone().fold(f64::NEG_INFINITY, f64::max);
    let mut south = lats.fold(f64::INFINITY, f64::min);
    if points.is_empty() {
        (north, south) = (0.0, 0.0);
    }
    // Наименьшая дуга долгот: у фигуры через линию перемены дат восток
    // меньше запада, как в LatLonAltBox KML.
    let lons: Vec<f64> = points.iter().map(|p| p.1).collect();
    let (mut west, mut east) = lon_span(&lons);
    let half_lat = BOX_SIDE / 2.0 / M_PER_DEGREE;
    if north - south < 2.0 * half_lat {
        let middle = (north + south) / 2.0;
        (north, south) = (middle + half_lat, middle - half_lat);
    }
    let half_lon = half_lat / ((north + south) / 2.0).to_radians().cos().max(0.01);
    if east - west < 2.0 * half_lon {
        let middle = (east + west) / 2.0;
        (east, west) = (middle + half_lon, middle - half_lon);
    }
    (north, south, wrap_lon(east), wrap_lon(west))
}

/// Region, который скрывает метку дальше km километров. Рамка - прежняя
/// old или рамка точек, maxLodPixels - прежний или -1.
pub fn for_distance(points: &[(f64, f64)], km: f64, old: Option<&Region>) -> Region {
    let ((north, south, east, west), max_lod) = match old {
        Some(o) => ((o.north, o.south, o.east, o.west), o.max_lod),
        None => (bounding_box(points), -1.0),
    };
    let probe = Region::new(north, south, east, west, 0.0, -1.0);
    let min_lod = side(&probe) * REF_FOCAL / (km * 1000.0);
    Region::new(north, south, east, west, py_round(min_lod, 6), max_lod)
}

/// Расстояние, дальше которого метка скрыта, км, или None.
pub fn distance_km(region: Option<&Region>) -> Option<f64> {
    let region = region.filter(|r| r.min_lod > 0.0)?;
    Some(side(region) * REF_FOCAL / region.min_lod / 1000.0)
}

/// Region для поля файла меток: JSON, пустая строка - нет.
pub fn text(region: Option<&Region>) -> String {
    region.map_or(String::new(), |r| Value::from(r.values().map(|v| py_round(v, 7)).to_vec()).to_string())
}

/// Число из JSON: число или строка с числом, как float() в Python.
fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Region из поля файла меток или None.
pub fn parse(value: &str) -> Option<Region> {
    if value.is_empty() {
        return None;
    }
    let data: Value = serde_json::from_str(value).ok()?;
    let items = data.as_array()?;
    let values: Vec<f64> = items.iter().map(number).collect::<Option<_>>()?;
    let [north, south, east, west, min_lod, max_lod] = values.as_slice() else {
        return None;
    };
    if north < south {
        return None;
    }
    Some(Region::new(*north, *south, *east, *west, *min_lod, *max_lod))
}

/// Region для записи в KML.
pub fn kml(region: Option<&Region>) -> String {
    let Some(r) = region else {
        return String::new();
    };
    format!(
        "<Region><LatLonAltBox><north>{:.7}</north><south>{:.7}</south><east>{:.7}</east><west>{:.7}</west>\
         </LatLonAltBox><Lod><minLodPixels>{}</minLodPixels><maxLodPixels>{}</maxLodPixels></Lod></Region>",
        r.north,
        r.south,
        r.east,
        r.west,
        crate::util::fmt_g(r.min_lod),
        crate::util::fmt_g(r.max_lod)
    )
}
