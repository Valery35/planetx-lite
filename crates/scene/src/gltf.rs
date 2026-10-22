// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Чтение моделей glTF 2.0: .gltf с файлами рядом и .glb одним файлом.
//! Перенос `core/gltf.py` модуля без анимаций.
//!
//! Берутся сетки треугольников (mode 4, полосы и веера mode 5 и 6
//! разворачиваются), POSITION, NORMAL, TEXCOORD_0, номера вершин,
//! материал pbrMetallicRoughness - baseColorFactor и картинка
//! baseColorTexture. Узлы сцены - matrix или translation, rotation,
//! scale. Сжатие сеток KHR_draco_mesh_compression и EXT_meshopt не
//! читается - такой файл даёт ошибку с именем расширения.
//!
//! Ось Y glTF смотрит вверх, итог переводится в ось Z вверх
//! ([`from_y_up`]): передняя сторона модели +Z смотрит на юг.
//! Картинки частей называются «gltf:номер картинки».
//!
//! Враждебный файл ограничен: вложенность узлов не больше
//! [`MAX_DEPTH`], обходов узлов не больше [`MAX_NODES`], вершин не
//! больше [`MAX_VERTICES`], accessor не выходит за свой bufferView.

use std::borrow::Cow;
use std::collections::HashMap;

use globe_core::{DMat3, DMat4, DVec2, DVec3};
use serde_json::{Map, Value};

use crate::models::{GLB_MAGIC, GRAY, MAX_VERTICES, ModelError, Part, Pictures, error, from_y_up, rows, split_glb};

/// Расширения сжатия, которые не читаются.
pub const UNSUPPORTED: [&str; 3] = ["KHR_draco_mesh_compression", "EXT_meshopt_compression", "KHR_mesh_quantization"];
/// Наибольшая вложенность узлов сцены.
pub const MAX_DEPTH: usize = 64;
/// Наибольшее количество обходов узлов: узел, который много раз стоит
/// в children, не раздувает обход.
pub const MAX_NODES: usize = 1_000_000;

/// Файл рядом по адресу: байты или None.
pub type Fetch<'f> = &'f dyn Fn(&str) -> Option<Vec<u8>>;

/// Тип чисел accessor.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Component {
    I8,
    U8,
    I16,
    U16,
    U32,
    F32,
}

impl Component {
    fn of(code: Option<&Value>) -> Option<Self> {
        Some(match code?.as_u64()? {
            5120 => Self::I8,
            5121 => Self::U8,
            5122 => Self::I16,
            5123 => Self::U16,
            5125 => Self::U32,
            5126 => Self::F32,
            _ => return None,
        })
    }

    fn size(self) -> usize {
        match self {
            Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::U32 | Self::F32 => 4,
        }
    }

    /// Наибольшее значение целого типа для normalized.
    fn top(self) -> f64 {
        match self {
            Self::I8 => 127.0,
            Self::U8 => 255.0,
            Self::I16 => 32767.0,
            Self::U16 => 65535.0,
            Self::U32 => 4_294_967_295.0,
            Self::F32 => 1.0,
        }
    }

    /// Значение из байтов, их ровно size.
    fn value(self, b: &[u8]) -> f64 {
        let at = |i: usize| b.get(i).copied().unwrap_or(0);
        match self {
            Self::I8 => f64::from(i8::from_le_bytes([at(0)])),
            Self::U8 => f64::from(at(0)),
            Self::I16 => f64::from(i16::from_le_bytes([at(0), at(1)])),
            Self::U16 => f64::from(u16::from_le_bytes([at(0), at(1)])),
            Self::U32 => f64::from(u32::from_le_bytes([at(0), at(1), at(2), at(3)])),
            Self::F32 => f64::from(f32::from_le_bytes([at(0), at(1), at(2), at(3)])),
        }
    }
}

/// Значения accessor: count строк по size чисел подряд.
struct Accessor {
    size: usize,
    values: Vec<f64>,
}

impl Accessor {
    fn count(&self) -> usize {
        self.values.len() / self.size.max(1)
    }

    fn row(&self, index: usize) -> &[f64] {
        let start = index.saturating_mul(self.size);
        self.values.get(start..start.saturating_add(self.size)).unwrap_or_default()
    }

    fn vec3(&self, index: usize) -> DVec3 {
        match self.row(index) {
            [x, y, z, ..] => DVec3::new(*x, *y, *z),
            _ => DVec3::ZERO,
        }
    }
}

/// Целое неотрицательное число JSON или default без значения.
fn uint(value: Option<&Value>, default: usize) -> Result<usize, ModelError> {
    match value {
        None => Ok(default),
        Some(v) => v
            .as_u64()
            .or_else(|| v.as_f64().filter(|f| f.fract() == 0.0 && (0.0..9.0e15).contains(f)).map(|f| f as u64))
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| error(format!("bad number {v}"))),
    }
}

/// Числа списка JSON длины len или default без значения.
fn floats<const N: usize>(value: Option<&Value>, default: [f64; N]) -> Result<[f64; N], ModelError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let items = value.as_array().filter(|a| a.len() == N).ok_or_else(|| error(format!("bad numbers {value}")))?;
    let mut out = default;
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = item.as_f64().ok_or_else(|| error(format!("bad numbers {value}")))?;
    }
    Ok(out)
}

/// Байты base64, знаки вне алфавита пропускаются.
fn base64(text: &str) -> Result<Vec<u8>, ModelError> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut bits, mut have) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => continue,
        };
        bits = (bits << 6) | u32::from(v);
        have += 6;
        if have >= 8 {
            have -= 8;
            out.push(((bits >> have) & 0xFF) as u8);
        }
    }
    if have >= 6 {
        return Err(error("bad data uri"));
    }
    Ok(out)
}

/// Разобранный документ: JSON, двоичный кусок .glb, буферы с памятью.
struct Doc<'a> {
    json: &'a Map<String, Value>,
    blob: Option<&'a [u8]>,
    fetch: Fetch<'a>,
    /// Номер буфера - байты, None - двоичный кусок .glb.
    buffers: HashMap<usize, Option<Vec<u8>>>,
    visits: usize,
    vertices: usize,
}

impl<'a> Doc<'a> {
    fn list(&self, name: &str) -> &'a [Value] {
        self.json.get(name).and_then(Value::as_array).map_or(&[], Vec::as_slice)
    }

    fn item(&self, name: &str, index: Option<&Value>) -> Result<&'a Map<String, Value>, ModelError> {
        let items = self.list(name);
        index
            .and_then(Value::as_u64)
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| items.get(i))
            .and_then(Value::as_object)
            .ok_or_else(|| error(format!("bad {name} index {}", index.unwrap_or(&Value::Null))))
    }

    /// Байты по адресу: data: base64 или файл рядом.
    fn uri(&self, uri: Option<&Value>) -> Result<Vec<u8>, ModelError> {
        let uri = uri.and_then(Value::as_str).ok_or_else(|| error("bad uri"))?;
        if let Some(rest) = uri.strip_prefix("data:") {
            let (head, body) = rest.split_once(',').unwrap_or((rest, ""));
            return if head.ends_with(";base64") { base64(body) } else { Ok(body.as_bytes().to_vec()) };
        }
        (self.fetch)(uri).ok_or_else(|| error(format!("file not found: {uri}")))
    }

    fn buffer(&mut self, index: usize) -> Result<&[u8], ModelError> {
        if !self.buffers.contains_key(&index) {
            let item = self.item("buffers", Some(&Value::from(index)))?;
            let data = if item.contains_key("uri") {
                Some(self.uri(item.get("uri"))?)
            } else if self.blob.is_some() {
                None
            } else {
                return Err(error("buffer without data"));
            };
            self.buffers.insert(index, data);
        }
        match self.buffers.get(&index) {
            Some(Some(data)) => Ok(data),
            _ => self.blob.ok_or_else(|| error("buffer without data")),
        }
    }

    /// Байты bufferView и его шаг.
    fn view(&mut self, index: Option<&Value>) -> Result<(&[u8], Option<usize>), ModelError> {
        let view = self.item("bufferViews", index)?;
        let buffer = uint(view.get("buffer"), 0)?;
        let start = uint(view.get("byteOffset"), 0)?;
        let length = uint(view.get("byteLength"), 0)?;
        let stride = match view.get("byteStride") {
            None | Some(Value::Null) => None,
            found => Some(uint(found, 0)?).filter(|s| *s > 0),
        };
        let data = self.buffer(buffer)?;
        let data = start.checked_add(length).and_then(|end| data.get(start..end)).ok_or_else(|| error("bufferView out of buffer"))?;
        Ok((data, stride))
    }

    fn accessor(&mut self, index: Option<&Value>) -> Result<Accessor, ModelError> {
        let acc = self.item("accessors", index)?;
        let kind = Component::of(acc.get("componentType"));
        let size = match acc.get("type").and_then(Value::as_str) {
            Some("SCALAR") => Some(1),
            Some("VEC2") => Some(2),
            Some("VEC3") => Some(3),
            Some("VEC4") => Some(4),
            Some("MAT4") => Some(16),
            _ => None,
        };
        let (Some(kind), Some(size)) = (kind, size) else {
            return Err(error("bad accessor type"));
        };
        let count = uint(acc.get("count"), 0)?;
        let Some(view) = acc.get("bufferView") else {
            if count > MAX_VERTICES {
                return Err(error("accessor too large"));
            }
            return Ok(Accessor { size, values: vec![0.0; count * size] });
        };
        let offset = uint(acc.get("byteOffset"), 0)?;
        let normalized = acc.get("normalized").and_then(Value::as_bool) == Some(true) && kind != Component::F32;
        let (data, stride) = self.view(Some(view))?;
        let item = kind.size() * size;
        let stride = stride.unwrap_or(item);
        if stride < item {
            return Err(error("bad byteStride"));
        }
        let need = if count == 0 {
            Some(0)
        } else {
            stride.checked_mul(count - 1).and_then(|n| n.checked_add(offset)).and_then(|n| n.checked_add(item))
        };
        if need.is_none_or(|n| n > data.len()) {
            return Err(error("accessor out of bufferView"));
        }
        let mut values = Vec::with_capacity(count * size);
        for row in 0..count {
            let at = offset + row * stride;
            for c in 0..size {
                let start = at + c * kind.size();
                let value = kind.value(data.get(start..start + kind.size()).unwrap_or_default());
                values.push(if normalized { (value / kind.top()).max(-1.0) } else { value });
            }
        }
        Ok(Accessor { size, values })
    }

    /// Номер картинки или None и RGBA 0-1 материала.
    fn material(&self, index: Option<&Value>) -> Result<(Option<&'a Value>, [f64; 4]), ModelError> {
        let Some(index) = index else {
            return Ok((None, GRAY));
        };
        let material = self.item("materials", Some(index))?;
        let pbr = material.get("pbrMetallicRoughness").and_then(Value::as_object);
        let rgba = floats(pbr.and_then(|p| p.get("baseColorFactor")), [1.0; 4])?;
        let Some(texture) = pbr.and_then(|p| p.get("baseColorTexture")) else {
            return Ok((None, rgba));
        };
        let texture = self.item("textures", texture.get("index"))?;
        Ok((texture.get("source").filter(|s| !s.is_null()), rgba))
    }

    fn picture(&mut self, index: &Value) -> Result<Vec<u8>, ModelError> {
        let image = self.item("images", Some(index))?;
        if let Some(view) = image.get("bufferView") {
            return Ok(self.view(Some(view))?.0.to_vec());
        }
        self.uri(Some(image.get("uri").unwrap_or(&Value::String(String::new()))))
    }

    fn mesh(&mut self, mesh: &'a Map<String, Value>, matrix: &DMat4, out: &mut Vec<Part>, pictures: &mut Pictures) -> Result<(), ModelError> {
        let linear = DMat3::from_mat4(*matrix);
        let det = linear.determinant();
        if det == 0.0 || !det.is_finite() {
            return Ok(());
        }
        let normal_matrix = linear.inverse().transpose();
        let shift = matrix.w_axis.truncate();
        let primitives = mesh.get("primitives").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
        for prim in primitives {
            let prim = prim.as_object().ok_or_else(|| error("bad primitive"))?;
            let Some(attributes) = prim.get("attributes").and_then(Value::as_object) else {
                continue;
            };
            let Some(position) = attributes.get("POSITION") else {
                continue;
            };
            let positions = self.accessor(Some(position))?;
            if positions.size < 3 {
                return Err(error("bad POSITION"));
            }
            let indices = match prim.get("indices") {
                Some(found) => {
                    let found = self.accessor(Some(found))?;
                    let rows = (0..found.count()).map(|i| found.row(i).first().copied().unwrap_or(-1.0));
                    let order: Option<Vec<usize>> =
                        rows.map(|v| (v >= 0.0 && v.fract() == 0.0 && v < 9.0e15).then_some(v as usize)).collect();
                    Some(order.ok_or_else(|| error("index out of range"))?)
                }
                None => None,
            };
            let mode = match prim.get("mode") {
                None => 4,
                Some(m) => m.as_u64().unwrap_or(u64::MAX),
            };
            let mut order = triangles(mode, positions.count(), indices.as_deref());
            if order.is_empty() {
                continue;
            }
            if order.iter().any(|i| *i >= positions.count()) {
                return Err(error("index out of range"));
            }
            self.vertices = self.vertices.saturating_add(order.len());
            if self.vertices > MAX_VERTICES {
                return Err(error(format!("too many vertices: more than {MAX_VERTICES}")));
            }
            if det < 0.0 {
                for tri in order.as_chunks_mut::<3>().0 {
                    tri.swap(0, 2);
                }
            }
            let points: Vec<DVec3> = order.iter().map(|i| linear * positions.vec3(*i) + shift).collect();
            let mut normals = None;
            if let Some(found) = attributes.get("NORMAL") {
                let found = self.accessor(Some(found))?;
                if found.size < 3 || order.iter().any(|i| *i >= found.count()) {
                    return Err(error("bad NORMAL"));
                }
                let turned = order.iter().map(|i| {
                    let v = normal_matrix * found.vec3(*i);
                    let length = v.length();
                    if length > 0.0 { v / length } else { v }
                });
                normals = Some(turned.collect());
            }
            let (image, rgba) = self.material(prim.get("material"))?;
            let mut uv = None;
            let mut name = None;
            if let (Some(image), Some(found)) = (image, attributes.get("TEXCOORD_0")) {
                let found = self.accessor(Some(found))?;
                if found.size < 2 || order.iter().any(|i| *i >= found.count()) {
                    return Err(error("bad TEXCOORD_0"));
                }
                uv = Some(order.iter().map(|i| DVec2::from_slice(found.row(*i))).collect());
                let key = format!("gltf:{image}");
                if !pictures.contains_key(&key) {
                    let bytes = self.picture(image)?;
                    pictures.insert(key.clone(), bytes);
                }
                name = Some(key);
            }
            out.push(Part::new(points, normals, uv, name, rgba));
        }
        Ok(())
    }

    fn walk(&mut self, index: &Value, parent: &DMat4, out: &mut Vec<Part>, pictures: &mut Pictures, depth: usize) -> Result<(), ModelError> {
        if depth > MAX_DEPTH {
            return Err(error("node nesting too deep"));
        }
        self.visits += 1;
        if self.visits > MAX_NODES {
            return Err(error("too many nodes"));
        }
        let node = self.item("nodes", Some(index))?;
        let matrix = *parent * node_matrix(node)?;
        if let Some(mesh) = node.get("mesh") {
            let mesh = self.item("meshes", Some(mesh))?;
            self.mesh(mesh, &matrix, out, pictures)?;
        }
        let children = node.get("children").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
        for child in children {
            self.walk(child, &matrix, out, pictures, depth + 1)?;
        }
        Ok(())
    }
}

/// Матрица узла: matrix по столбцам или translation, rotation, scale.
fn node_matrix(node: &Map<String, Value>) -> Result<DMat4, ModelError> {
    if let Some(matrix) = node.get("matrix").filter(|m| m.as_array().is_some_and(|a| a.len() == 16)) {
        // Матрица glTF записана по столбцам, как в glam.
        return Ok(DMat4::from_cols_array(&floats(Some(matrix), [0.0; 16])?));
    }
    let [tx, ty, tz] = floats(node.get("translation"), [0.0; 3])?;
    let [qx, qy, qz, qw] = floats(node.get("rotation"), [0.0, 0.0, 0.0, 1.0])?;
    let scale = floats(node.get("scale"), [1.0; 3])?;
    let rot = rows(
        [1.0 - 2.0 * (qy * qy + qz * qz), 2.0 * (qx * qy - qz * qw), 2.0 * (qx * qz + qy * qw)],
        [2.0 * (qx * qy + qz * qw), 1.0 - 2.0 * (qx * qx + qz * qz), 2.0 * (qy * qz - qx * qw)],
        [2.0 * (qx * qz - qy * qw), 2.0 * (qy * qz + qx * qw), 1.0 - 2.0 * (qx * qx + qy * qy)],
    );
    let linear = rot * DMat3::from_diagonal(DVec3::from(scale));
    let mut out = DMat4::from_mat3(linear);
    out.w_axis = DVec3::new(tx, ty, tz).extend(1.0);
    Ok(out)
}

/// Номера вершин треугольников по режиму примитива: 4 - треугольники,
/// 5 - полоса, 6 - веер, прочие - ничего. indices - номера вершин или
/// None, тогда вершины идут по порядку count штук.
pub fn triangles(mode: u64, count: usize, indices: Option<&[usize]>) -> Vec<usize> {
    let order: Cow<'_, [usize]> = indices.map_or_else(|| Cow::Owned((0..count).collect()), Cow::Borrowed);
    let n = order.len();
    let mut out = Vec::new();
    match mode {
        4 => out.extend_from_slice(order.get(..n / 3 * 3).unwrap_or_default()),
        5 if n >= 3 => {
            for k in 0..n - 2 {
                let (a, b, c) = (order[k], order[k + 1], order[k + 2]);
                out.extend(if k % 2 == 0 { [a, b, c] } else { [a, c, b] });
            }
        }
        6 if n >= 3 => {
            for k in 1..n - 1 {
                out.extend([order[0], order[k], order[k + 1]]);
            }
        }
        _ => {}
    }
    out
}

/// Части с осью Z вверх и картинки «gltf:номер» из .gltf или .glb.
/// fetch(адрес) - байты файла рядом или None.
pub fn read(data: &[u8], fetch: Fetch<'_>) -> Result<(Vec<Part>, Pictures), ModelError> {
    let (text, blob) = if data.starts_with(GLB_MAGIC) { split_glb(data)? } else { (data, None) };
    let json: Value = serde_json::from_slice(text).map_err(|_| error("bad glTF JSON"))?;
    let json = json.as_object().ok_or_else(|| error("bad glTF JSON"))?;
    for list in ["extensionsRequired", "extensionsUsed"] {
        for name in json.get(list).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            if UNSUPPORTED.contains(&name) {
                return Err(error(format!("unsupported glTF extension: {name}")));
            }
        }
    }
    let mut doc = Doc { json, blob, fetch, buffers: HashMap::new(), visits: 0, vertices: 0 };
    let scenes = doc.list("scenes");
    let roots: Vec<Value> = if scenes.is_empty() {
        (0..doc.list("nodes").len()).map(Value::from).collect()
    } else {
        let index = json.get("scene").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).filter(|i| *i < scenes.len()).unwrap_or(0);
        let scene = scenes.get(index).and_then(Value::as_object).ok_or_else(|| error("bad scene"))?;
        scene.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default()
    };
    let mut out = Vec::new();
    let mut pictures = Pictures::new();
    for root in &roots {
        doc.walk(root, &DMat4::IDENTITY, &mut out, &mut pictures, 0)?;
    }
    Ok((out.into_iter().map(from_y_up).collect(), pictures))
}
