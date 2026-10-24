// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Векторный тайл Mapbox Vector Tile 2 - сообщение Protocol Buffers.
//!
//! Раскодируются только слои из списка, остальные пропускаются по длине
//! без разбора. Тайл считается враждебным: длина за концом данных,
//! неизвестный тип поля и битый gzip дают [`DecodeError`], а не падение.

use std::collections::{HashMap, HashSet};
use std::io::Read;

/// Размер тайла в единицах слоя по умолчанию.
pub const EXTENT: u32 = 4096;

/// Тайл не разбирается как Mapbox Vector Tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError(pub String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "векторный тайл не разбирается: {}", self.0)
    }
}

impl std::error::Error for DecodeError {}

fn error(what: &str) -> DecodeError {
    DecodeError(what.to_string())
}

/// Значение атрибута.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Float(f64),
    Int(i64),
    Bool(bool),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Число, если значение числовое.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Float(v) => Some(*v),
            Self::Int(v) => Some(*v as f64),
            _ => None,
        }
    }
}

/// Тип геометрии объекта.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Geometry {
    Point,
    Line,
    /// Многоугольник: части - кольца, первое внешнее.
    Polygon,
}

/// Объект слоя. У точки - точки объекта, у линии - все вершины. parts -
/// части линии или кольца многоугольника, их раскладывает
/// [`decode_shapes`], у [`decode_layers`] они пусты.
#[derive(Debug, Clone, PartialEq)]
pub struct Feature {
    pub id: u64,
    pub geometry: Geometry,
    pub attrs: HashMap<String, Value>,
    pub points: Vec<(i64, i64)>,
    pub parts: Vec<Vec<(i64, i64)>>,
}

/// Объекты слоя и размер тайла в его единицах.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub features: Vec<Feature>,
    pub extent: u32,
}

/// Значение поля сообщения.
enum Field<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Fixed(&'a [u8]),
}

fn varint(data: &[u8], pos: &mut usize) -> Result<u64, DecodeError> {
    let mut result = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *data.get(*pos).ok_or_else(|| error("varint"))?;
        *pos += 1;
        if shift < 64 {
            result |= u64::from(byte & 0x7F) << shift;
        }
        if byte & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
        if shift > 70 {
            return Err(error("varint"));
        }
    }
}

/// Поля сообщения: номер и значение.
fn fields(data: &[u8]) -> Result<Vec<(u64, Field<'_>)>, DecodeError> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let key = varint(data, &mut pos)?;
        let (number, wire) = (key >> 3, key & 7);
        let take = |pos: &mut usize, size: usize| -> Result<&[u8], DecodeError> {
            let end = pos.checked_add(size).filter(|e| *e <= data.len()).ok_or_else(|| error("length"))?;
            let slice = &data[*pos..end];
            *pos = end;
            Ok(slice)
        };
        let value = match wire {
            0 => Field::Varint(varint(data, &mut pos)?),
            2 => {
                let size = usize::try_from(varint(data, &mut pos)?).map_err(|_| error("length"))?;
                Field::Bytes(take(&mut pos, size)?)
            }
            1 => Field::Fixed(take(&mut pos, 8)?),
            5 => Field::Fixed(take(&mut pos, 4)?),
            other => return Err(DecodeError(format!("wire type {other}"))),
        };
        out.push((number, value));
    }
    Ok(out)
}

/// Упакованные числа varint.
fn packed(data: &[u8]) -> Result<Vec<u64>, DecodeError> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        out.push(varint(data, &mut pos)?);
    }
    Ok(out)
}

fn zigzag(value: u64) -> i64 {
    ((value >> 1) as i64) ^ -((value & 1) as i64)
}

fn value(data: &[u8]) -> Result<Option<Value>, DecodeError> {
    for (number, field) in fields(data)? {
        let v = match (number, field) {
            (1, Field::Bytes(b)) => Value::Str(String::from_utf8_lossy(b).into_owned()),
            (2, Field::Fixed(b)) => {
                let raw: [u8; 4] = b.try_into().map_err(|_| error("float"))?;
                Value::Float(f64::from(f32::from_le_bytes(raw)))
            }
            (3, Field::Fixed(b)) => {
                let raw: [u8; 8] = b.try_into().map_err(|_| error("double"))?;
                Value::Float(f64::from_le_bytes(raw))
            }
            (4, Field::Varint(v)) => Value::Int(v as i64),
            (5, Field::Varint(v)) => Value::Int(i64::try_from(v).unwrap_or(i64::MAX)),
            (6, Field::Varint(v)) => Value::Int(zigzag(v)),
            (7, Field::Varint(v)) => Value::Bool(v != 0),
            _ => continue,
        };
        return Ok(Some(v));
    }
    Ok(None)
}

/// Точки геометрии в единицах слоя. every=false - только начала частей,
/// это точки объекта-точки. every=true - все вершины, для линии.
fn points(geometry: &[u64], every: bool) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    let (mut x, mut y) = (0i64, 0i64);
    let mut i = 0;
    while i < geometry.len() {
        let command = geometry[i];
        i += 1;
        let (kind, count) = (command & 7, command >> 3);
        if kind == 1 || kind == 2 {
            for _ in 0..count {
                let (Some(dx), Some(dy)) = (geometry.get(i), geometry.get(i + 1)) else {
                    return out;
                };
                x = x.wrapping_add(zigzag(*dx));
                y = y.wrapping_add(zigzag(*dy));
                i += 2;
                if kind == 1 || every {
                    out.push((x, y));
                }
            }
        } else if kind != 7 {
            return out;
        }
    }
    out
}

/// Части геометрии: каждая команда MoveTo начинает новую часть,
/// ClosePath замыкает кольцо первой вершиной.
fn split_parts(geometry: &[u64]) -> Vec<Vec<(i64, i64)>> {
    let mut out: Vec<Vec<(i64, i64)>> = Vec::new();
    let (mut x, mut y) = (0i64, 0i64);
    let mut i = 0;
    while i < geometry.len() {
        let command = geometry[i];
        i += 1;
        let (kind, count) = (command & 7, command >> 3);
        match kind {
            1 | 2 => {
                for _ in 0..count {
                    let (Some(dx), Some(dy)) = (geometry.get(i), geometry.get(i + 1)) else {
                        return out;
                    };
                    x = x.wrapping_add(zigzag(*dx));
                    y = y.wrapping_add(zigzag(*dy));
                    i += 2;
                    if kind == 1 {
                        out.push(vec![(x, y)]);
                    } else if let Some(part) = out.last_mut() {
                        part.push((x, y));
                    }
                }
            }
            7 => {
                if let Some(part) = out.last_mut()
                    && let Some(&first) = part.first()
                {
                    part.push(first);
                }
            }
            _ => return out,
        }
    }
    out
}

/// Сжатый gzip тайл распаковывается.
fn unpack(data: &[u8]) -> Result<std::borrow::Cow<'_, [u8]>, DecodeError> {
    if data.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(data).read_to_end(&mut out).map_err(|e| DecodeError(e.to_string()))?;
        Ok(out.into())
    } else {
        Ok(data.into())
    }
}

/// Объекты слоёв names за один проход по тайлу. wanted - имена нужных
/// атрибутов, None - все. Полигоны пропускаются.
pub fn decode_layers(
    data: &[u8],
    names: &HashSet<&str>,
    wanted: Option<&HashSet<String>>,
) -> Result<HashMap<String, Layer>, DecodeError> {
    decode(data, names, wanted, false)
}

/// Как [`decode_layers`], но с многоугольниками и частями геометрии
/// у линий и многоугольников - для отрисовки линий векторной основы.
pub fn decode_shapes(
    data: &[u8],
    names: &HashSet<&str>,
    wanted: Option<&HashSet<String>>,
) -> Result<HashMap<String, Layer>, DecodeError> {
    decode(data, names, wanted, true)
}

fn decode(
    data: &[u8],
    names: &HashSet<&str>,
    wanted: Option<&HashSet<String>>,
    shapes: bool,
) -> Result<HashMap<String, Layer>, DecodeError> {
    let data = unpack(data)?;
    let mut out = HashMap::new();
    for (number, field) in fields(&data)? {
        let (3, Field::Bytes(layer)) = (number, field) else {
            continue;
        };
        let parts = fields(layer)?;
        let title = parts.iter().find_map(|(n, f)| match (n, f) {
            (1, Field::Bytes(b)) => Some(String::from_utf8_lossy(b).into_owned()),
            _ => None,
        });
        let Some(title) = title.filter(|t| names.contains(t.as_str())) else {
            continue;
        };
        let keys: Vec<String> = parts
            .iter()
            .filter_map(|(n, f)| match (n, f) {
                (3, Field::Bytes(b)) => Some(String::from_utf8_lossy(b).into_owned()),
                _ => None,
            })
            .collect();
        // Значений в слое place тайла уровня 4 около 10 тысяч, это
        // названия стран на всех языках. Раскодируются только значения
        // нужных атрибутов.
        let raw: Vec<&[u8]> = parts
            .iter()
            .filter_map(|(n, f)| match (n, f) {
                (4, Field::Bytes(b)) => Some(*b),
                _ => None,
            })
            .collect();
        let extent = parts
            .iter()
            .find_map(|(n, f)| match (n, f) {
                (5, Field::Varint(v)) => u32::try_from(*v).ok(),
                _ => None,
            })
            .unwrap_or(EXTENT);
        let mut decoded: HashMap<usize, Option<Value>> = HashMap::new();
        let mut features = Vec::new();
        for (n, f) in &parts {
            let (2, Field::Bytes(feature)) = (n, f) else {
                continue;
            };
            let (mut id, mut tags, mut geometry, mut kind) = (0, Vec::new(), Vec::new(), 0);
            for (fn_, fv) in fields(feature)? {
                match (fn_, fv) {
                    (1, Field::Varint(v)) => id = v,
                    (2, Field::Bytes(b)) => tags = packed(b)?,
                    (3, Field::Varint(v)) => kind = v,
                    (4, Field::Bytes(b)) => geometry = packed(b)?,
                    _ => {}
                }
            }
            let geometry_kind = match kind {
                1 => Geometry::Point,
                2 => Geometry::Line,
                3 if shapes => Geometry::Polygon,
                _ => continue,
            };
            let mut attrs = HashMap::new();
            for pair in tags.as_chunks::<2>().0 {
                let (Ok(k), Ok(v)) = (usize::try_from(pair[0]), usize::try_from(pair[1])) else {
                    continue;
                };
                let (Some(key), Some(bytes)) = (keys.get(k), raw.get(v)) else {
                    continue;
                };
                if wanted.is_some_and(|w| !w.contains(key)) {
                    continue;
                }
                if let std::collections::hash_map::Entry::Vacant(slot) = decoded.entry(v) {
                    slot.insert(value(bytes)?);
                }
                if let Some(Some(value)) = decoded.get(&v) {
                    attrs.insert(key.clone(), value.clone());
                }
            }
            let every = geometry_kind == Geometry::Line;
            let split = if shapes && geometry_kind != Geometry::Point { split_parts(&geometry) } else { Vec::new() };
            features.push(Feature { id, geometry: geometry_kind, attrs, points: points(&geometry, every), parts: split });
        }
        out.insert(title, Layer { features, extent });
    }
    Ok(out)
}

/// Точечные объекты слоя name и размер тайла.
pub fn decode_layer(
    data: &[u8],
    name: &str,
    wanted: Option<&HashSet<String>>,
) -> Result<(Vec<Feature>, u32), DecodeError> {
    let names = HashSet::from([name]);
    let mut layers = decode_layers(data, &names, wanted)?;
    Ok(match layers.remove(name) {
        Some(layer) => {
            (layer.features.into_iter().filter(|f| f.geometry == Geometry::Point).collect(), layer.extent)
        }
        None => (Vec::new(), EXTENT),
    })
}
