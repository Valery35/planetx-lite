// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Слои GeoPackage из байтов файла, через terrio.
//!
//! Объект слоя - части геометрии в широте и долготе WGS84 (точка - одна
//! часть из одной точки, многоугольник - внешнее кольцо и дыры
//! отдельными частями) и значения полей текстом. Таблица без геометрии -
//! слой без частей у объектов.

use terrio::crs::{SpatialRef, Transformer};
use terrio::{FieldValue, Geometry};

/// Вид геометрии объекта.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    None,
    Point,
    Line,
    Polygon,
}

/// Объект слоя.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub shape: Shape,
    /// Части геометрии, точки (широта, долгота).
    pub parts: Vec<Vec<(f64, f64)>>,
    /// Значения полей текстом в порядке полей слоя.
    pub values: Vec<String>,
}

/// Слой.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerData {
    pub name: String,
    pub epsg: Option<u32>,
    pub fields: Vec<String>,
    pub items: Vec<Item>,
}

fn text(v: &FieldValue) -> String {
    match v {
        FieldValue::Integer(i) => i.to_string(),
        FieldValue::Float(f) => f.to_string(),
        FieldValue::Text(t) | FieldValue::Date(t) | FieldValue::DateTime(t) => t.clone(),
        FieldValue::Boolean(b) => b.to_string(),
        FieldValue::Blob(b) => format!("<{} байт>", b.len()),
        FieldValue::Null => String::new(),
    }
}

/// Части и вид геометрии в системе слоя.
fn parts(g: &Geometry, out: &mut Vec<Vec<(f64, f64)>>) -> Shape {
    macro_rules! line {
        ($c:expr) => {
            $c.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>()
        };
    }
    match g {
        Geometry::Point(p) => {
            out.push(vec![(p.x, p.y)]);
            Shape::Point
        }
        Geometry::MultiPoint(ps) => {
            out.extend(ps.iter().map(|p| vec![(p.x, p.y)]));
            Shape::Point
        }
        Geometry::LineString(c) => {
            out.push(line!(c));
            Shape::Line
        }
        Geometry::MultiLineString(ls) => {
            out.extend(ls.iter().map(|c| line!(c)));
            Shape::Line
        }
        Geometry::Polygon { exterior, interiors } => {
            out.push(line!(exterior.0));
            out.extend(interiors.iter().map(|r| line!(r.0)));
            Shape::Polygon
        }
        Geometry::MultiPolygon(ps) => {
            for (exterior, interiors) in ps {
                out.push(line!(exterior.0));
                out.extend(interiors.iter().map(|r| line!(r.0)));
            }
            Shape::Polygon
        }
        Geometry::GeometryCollection(gs) => gs.iter().map(|g| parts(g, out)).last().unwrap_or(Shape::None),
    }
}

/// Слои GeoPackage из байтов файла. Точки пересчитываются в WGS84,
/// объект, чья точка не пересчиталась, пропускается.
pub fn read(bytes: Vec<u8>, name: &str) -> Result<Vec<LayerData>, String> {
    let ds = terrio::memory::open_vector_bytes(bytes, name, &Default::default()).map_err(|e| e.to_string())?;
    let wgs = SpatialRef::from_epsg(4326).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for layer in &ds.layers {
        let crs = SpatialRef::from_layer(layer);
        let epsg = crs.authority().filter(|(a, _)| a.eq_ignore_ascii_case("EPSG")).map(|(_, c)| c);
        let to_wgs = if crs.is_unknown() { None } else { Transformer::new(&crs, &wgs).ok() };
        let mut items = Vec::with_capacity(layer.features.len());
        for f in &layer.features {
            let mut raw = Vec::new();
            let shape = f.geometry.as_ref().map_or(Shape::None, |g| parts(g, &mut raw));
            let converted: Option<Vec<Vec<(f64, f64)>>> = raw
                .into_iter()
                .map(|part| {
                    part.into_iter()
                        .map(|(x, y)| match &to_wgs {
                            Some(t) => t.transform(x, y).ok().map(|(lon, lat)| (lat, lon)),
                            None => Some((y, x)),
                        })
                        .collect()
                })
                .collect();
            let Some(parts) = converted else {
                continue;
            };
            items.push(Item { shape, parts, values: f.attributes.iter().map(text).collect() });
        }
        out.push(LayerData { name: layer.name.clone(), epsg, fields: layer.schema.fields().iter().map(|d| d.name.clone()).collect(), items });
    }
    Ok(out)
}
