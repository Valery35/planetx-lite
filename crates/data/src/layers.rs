// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Векторные слои из байтов файла, через terrio: GeoPackage, Shapefile,
//! GeoJSON, GPX, TopoJSON, GML, MapInfo, FlatGeobuf, DXF и DWG.
//!
//! Объект слоя - части геометрии в широте и долготе WGS84 (точка - одна
//! часть из одной точки, многоугольник - внешнее кольцо и дыры
//! отдельными частями) и значения полей текстом. Таблица без геометрии -
//! слой без частей у объектов.
//!
//! Шейпфайл и MapInfo - несколько файлов. Из памяти terrio читает их
//! архивом ZIP, [`bundle`] собирает архив из главного файла и соседних.
//! Слой без системы координат читается в градусах. Координаты вне
//! градусов у такого слоя - ошибка [`NO_CRS`]: чертёж DXF в метрах лёг
//! бы у нулевой точки.

use std::io::Write;

use terrio::crs::{SpatialRef, Transformer};
use terrio::{FieldValue, Geometry};

/// Расширения векторных файлов, которые открываются слоями.
pub const EXTENSIONS: [&str; 14] = ["gpkg", "shp", "geojson", "json", "geojsonl", "gpx", "topojson", "gml", "mif", "tab", "fgb", "dxf", "dwg", "zip"];

/// Расширения соседних файлов шейпфайла и MapInfo.
pub const SIDECARS: [&str; 9] = ["dbf", "shx", "prj", "cpg", "qpj", "mid", "dat", "map", "id"];

/// Главные файлы, которым нужны соседние.
pub const WITH_SIDECARS: [&str; 3] = ["shp", "mif", "tab"];

/// Ошибка слоя без системы координат с координатами не в градусах.
pub const NO_CRS: &str = "The layer has no coordinate system, and its coordinates are not degrees";

/// Расширение имени файла строчными буквами, пустое - без расширения.
pub fn extension(name: &str) -> String {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    file.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// Имя файла без расширения.
pub fn stem(name: &str) -> &str {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    file.rsplit_once('.').map_or(file, |(s, _)| s)
}

/// Файл открывается векторным слоем.
pub fn is_vector(name: &str) -> bool {
    EXTENSIONS.contains(&extension(name).as_str())
}

/// Файл - соседний у шейпфайла или MapInfo.
pub fn is_sidecar(name: &str) -> bool {
    SIDECARS.contains(&extension(name).as_str())
}

/// Главному файлу нужны соседние.
pub fn needs_sidecars(name: &str) -> bool {
    WITH_SIDECARS.contains(&extension(name).as_str())
}

/// Соседние файлы главного name из списка files: то же имя без
/// расширения, без учёта регистра, расширение из [`SIDECARS`].
pub fn sidecars_of<'a>(name: &str, files: &'a [(String, Vec<u8>)]) -> Vec<&'a (String, Vec<u8>)> {
    let base = stem(name).to_lowercase();
    files.iter().filter(|(n, _)| is_sidecar(n) && stem(n).to_lowercase() == base).collect()
}

/// Архив ZIP из главного файла и соседних для чтения из памяти: байты
/// и имя архива. Расширения в архиве строчные, как их ищет terrio.
pub fn bundle(main: &[u8], name: &str, sidecars: &[&(String, Vec<u8>)]) -> Result<(Vec<u8>, String), String> {
    let base = stem(name);
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (ext, bytes) in std::iter::once((extension(name), main)).chain(sidecars.iter().map(|(n, b)| (extension(n), b.as_slice()))) {
        zip.start_file(format!("{base}.{ext}"), options).map_err(|e| e.to_string())?;
        zip.write_all(bytes).map_err(|e| e.to_string())?;
    }
    let cursor = zip.finish().map_err(|e| e.to_string())?;
    Ok((cursor.into_inner(), format!("{base}.zip")))
}

/// Точка в градусах.
fn in_degrees(x: f64, y: f64) -> bool {
    (-180.0..=180.0).contains(&x) && (-90.0..=90.0).contains(&y)
}

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
    /// Отметки Z точек частей, у части без отметок пусто.
    pub heights: Vec<Vec<f64>>,
    /// Значения полей текстом в порядке полей слоя.
    pub values: Vec<String>,
}

/// Слой.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerData {
    pub name: String,
    pub epsg: Option<u32>,
    pub fields: Vec<String>,
    /// Поле числовое: все непустые значения - целые или дробные числа.
    pub numeric: Vec<bool>,
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

/// Точки частей в системе слоя с отметками Z.
type RawParts = Vec<Vec<(f64, f64, Option<f64>)>>;

/// Части и вид геометрии в системе слоя.
fn parts(g: &Geometry, out: &mut RawParts) -> Shape {
    macro_rules! line {
        ($c:expr) => {
            $c.iter().map(|p| (p.x, p.y, p.z)).collect::<Vec<_>>()
        };
    }
    match g {
        Geometry::Point(p) => {
            out.push(vec![(p.x, p.y, p.z)]);
            Shape::Point
        }
        Geometry::MultiPoint(ps) => {
            out.extend(ps.iter().map(|p| vec![(p.x, p.y, p.z)]));
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

/// Слои из байтов файла. Точки пересчитываются в WGS84, объект, чья
/// точка не пересчиталась, пропускается. Слой без системы координат
/// с точкой вне градусов - ошибка [`NO_CRS`].
pub fn read(bytes: Vec<u8>, name: &str) -> Result<Vec<LayerData>, String> {
    let ds = terrio::memory::open_vector_bytes(bytes, name, &Default::default()).map_err(|e| e.to_string())?;
    let wgs = SpatialRef::from_epsg(4326).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for layer in &ds.layers {
        let crs = SpatialRef::from_layer(layer);
        let epsg = crs.authority().filter(|(a, _)| a.eq_ignore_ascii_case("EPSG")).map(|(_, c)| c);
        let to_wgs = if crs.is_unknown() { None } else { Transformer::new(&crs, &wgs).ok() };
        let count = layer.schema.fields().len();
        let mut numeric = vec![true; count];
        let mut filled = vec![false; count];
        let mut items = Vec::with_capacity(layer.features.len());
        for f in &layer.features {
            let mut raw = Vec::new();
            for (n, v) in f.attributes.iter().enumerate().take(count) {
                match v {
                    FieldValue::Null => {}
                    FieldValue::Integer(_) | FieldValue::Float(_) => filled[n] = true,
                    _ => {
                        filled[n] = true;
                        numeric[n] = false;
                    }
                }
            }
            let shape = f.geometry.as_ref().map_or(Shape::None, |g| parts(g, &mut raw));
            let heights = raw.iter().map(|part| part.iter().map(|p| p.2).collect::<Option<Vec<f64>>>().unwrap_or_default()).collect();
            let converted: Option<Vec<Vec<(f64, f64)>>> = raw
                .into_iter()
                .map(|part| {
                    part.into_iter()
                        .map(|(x, y, _)| match &to_wgs {
                            Some(t) => t.transform(x, y).ok().map(|(lon, lat)| (lat, lon)),
                            None => Some((y, x)),
                        })
                        .collect()
                })
                .collect();
            let Some(parts) = converted else {
                continue;
            };
            if to_wgs.is_none() && parts.iter().flatten().any(|(lat, lon)| !in_degrees(*lon, *lat)) {
                return Err(NO_CRS.to_string());
            }
            items.push(Item { shape, parts, heights, values: f.attributes.iter().map(text).collect() });
        }
        let numeric = numeric.iter().zip(&filled).map(|(n, f)| *n && *f).collect();
        out.push(LayerData { name: layer.name.clone(), epsg, fields: layer.schema.fields().iter().map(|d| d.name.clone()).collect(), numeric, items });
    }
    Ok(out)
}
