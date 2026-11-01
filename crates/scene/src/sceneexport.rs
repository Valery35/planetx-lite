// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сцена вида в glTF 2.0 и 3D Tiles 1.1, перенос `core/sceneexport.py`
//! модуля. Объекты вида - куски [`Item`] со своим центром в ECEF
//! и смещениями float32, как в видеокарте, поэтому точность на месте
//! не теряется.
//!
//! GLB - один файл, метры от начала в точке lat, lon, ось Y вверх, как
//! требует glTF, X на восток, Z на юг. Другие программы получают сцену
//! на плоскости у начала, место записано в `asset.extras`.
//!
//! 3D Tiles - `tileset.json` и файл GLB на кусок. Кусок стоит в ECEF: у тайла
//! сдвиг в центр, вершины - ECEF в осях glTF (x, z, -y), обратный поворот
//! делает читатель 3D Tiles по `gltfUpAxis` Y. Корень без содержимого,
//! детей рисуют все сразу (refine ADD).
//!
//! У куска цвет вершин или картинка PNG с uv. Материал без блеска,
//! двусторонний: крыши и стены сеток вида обращены как придётся.

use std::io::Write;

use globe_core::DVec3;
use globe_core::ellipsoid::Ellipsoid;
use serde_json::{Value, json};

pub const GENERATOR: &str = "PlanetX Lite";
pub const GLB_MAGIC: u32 = 0x4654_6C67;
pub const JSON_CHUNK: u32 = 0x4E4F_534A;
pub const BIN_CHUNK: u32 = 0x004E_4942;
pub const FLOAT: u32 = 5126;
pub const UBYTE: u32 = 5121;
pub const UINT: u32 = 5125;
const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_BUFFER: u32 = 34963;
/// Геометрическая ошибка кусков, м.
const TILE_ERROR: f64 = 16.0;

/// Кусок сцены. center - центр в ECEF, positions - смещения, indices -
/// номера вершин или None для списка треугольников вершинами подряд,
/// picture - байты PNG.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Item {
    pub center: [f64; 3],
    pub positions: Vec<[f32; 3]>,
    pub normals: Option<Vec<[f32; 3]>>,
    pub colors: Option<Vec<[u8; 4]>>,
    pub indices: Option<Vec<u32>>,
    pub uvs: Option<Vec<[f32; 2]>>,
    pub picture: Option<Vec<u8>>,
    pub name: String,
}

/// Файл набора 3D Tiles: имя и байты GLB.
pub type TileFile = (String, Vec<u8>);

/// Кусок документа в осях glTF.
struct Part<'a> {
    positions: Vec<[f32; 3]>,
    normals: Option<Vec<[f32; 3]>>,
    item: &'a Item,
}

/// Буфер и списки accessor документа glTF.
#[derive(Default)]
struct Writer {
    blob: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Writer {
    fn view(&mut self, data: &[u8], target: Option<u32>) -> usize {
        let mut entry = json!({"buffer": 0, "byteOffset": self.blob.len(), "byteLength": data.len()});
        if let Some(target) = target {
            entry["target"] = json!(target);
        }
        self.views.push(entry);
        self.blob.extend_from_slice(data);
        self.blob.resize(self.blob.len().next_multiple_of(4), 0);
        self.views.len() - 1
    }

    fn accessor(&mut self, data: &[u8], count: usize, kind: &str, code: u32, target: u32) -> Value {
        let view = self.view(data, Some(target));
        json!({"bufferView": view, "componentType": code, "count": count, "type": kind})
    }

    fn push(&mut self, entry: Value) -> usize {
        self.accessors.push(entry);
        self.accessors.len() - 1
    }

    fn positions(&mut self, points: &[[f32; 3]]) -> usize {
        let bytes: Vec<u8> = points.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
        let mut entry = self.accessor(&bytes, points.len(), "VEC3", FLOAT, ARRAY_BUFFER);
        let edge = |pick: fn(f32, f32) -> f32, start: f32| -> Vec<f64> { (0..3).map(|i| f64::from(points.iter().fold(start, |a, p| pick(a, p[i])))).collect() };
        entry["min"] = json!(edge(f32::min, f32::INFINITY));
        entry["max"] = json!(edge(f32::max, f32::NEG_INFINITY));
        self.push(entry)
    }

    fn vectors(&mut self, values: &[[f32; 3]]) -> usize {
        let bytes: Vec<u8> = values.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
        let entry = self.accessor(&bytes, values.len(), "VEC3", FLOAT, ARRAY_BUFFER);
        self.push(entry)
    }

    fn colors(&mut self, values: &[[u8; 4]]) -> usize {
        let bytes: Vec<u8> = values.iter().flatten().copied().collect();
        let mut entry = self.accessor(&bytes, values.len(), "VEC4", UBYTE, ARRAY_BUFFER);
        entry["normalized"] = json!(true);
        self.push(entry)
    }

    fn uvs(&mut self, values: &[[f32; 2]]) -> usize {
        let bytes: Vec<u8> = values.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
        let entry = self.accessor(&bytes, values.len(), "VEC2", FLOAT, ARRAY_BUFFER);
        self.push(entry)
    }

    fn indices(&mut self, values: &[u32]) -> usize {
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let entry = self.accessor(&bytes, values.len(), "SCALAR", UINT, ELEMENT_BUFFER);
        self.push(entry)
    }
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn wide(p: [f32; 3]) -> [f64; 3] {
    p.map(f64::from)
}

/// Единичные векторы, нулевой остаётся нулевым.
fn unit(values: impl Iterator<Item = [f64; 3]>) -> Vec<[f32; 3]> {
    values
        .map(|v| {
            let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            let length = if length > 0.0 { length } else { 1.0 };
            v.map(|c| (c / length) as f32)
        })
        .collect()
}

/// Нормали вершин по граням, куску без своих нормалей.
fn flat_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut normals = vec![[0.0f64; 3]; positions.len()];
    for face in indices.as_chunks::<3>().0 {
        let corner = [face[0] as usize, face[1] as usize, face[2] as usize];
        let (Some(a), Some(b), Some(c)) = (positions.get(corner[0]), positions.get(corner[1]), positions.get(corner[2])) else {
            continue;
        };
        let (a, b, c) = (wide(*a), wide(*b), wide(*c));
        let side = |p: [f64; 3]| [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
        let n = cross(side(b), side(c));
        for i in corner {
            normals[i] = add(normals[i], n);
        }
    }
    unit(normals.into_iter())
}

/// Байты GLB из кусков с готовыми в осях glTF вершинами.
fn glb(parts: &[Part], extras: Option<Value>) -> Vec<u8> {
    let mut writer = Writer::default();
    let (mut primitives, mut materials, mut textures, mut images) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for part in parts {
        let count = part.positions.len();
        if count == 0 {
            continue;
        }
        let item = part.item;
        let indices: Vec<u32> = item.indices.clone().unwrap_or_else(|| (0..count as u32).collect());
        let normals = match &part.normals {
            Some(n) => unit(n.iter().map(|v| wide(*v))),
            None => flat_normals(&part.positions, &indices),
        };
        let mut attributes = json!({"POSITION": writer.positions(&part.positions), "NORMAL": writer.vectors(&normals)});
        if let Some(colors) = &item.colors {
            attributes["COLOR_0"] = json!(writer.colors(colors));
        }
        let mut material = json!({"doubleSided": true, "pbrMetallicRoughness": {"metallicFactor": 0.0, "roughnessFactor": 1.0}});
        if let (Some(picture), Some(uvs)) = (&item.picture, &item.uvs) {
            attributes["TEXCOORD_0"] = json!(writer.uvs(uvs));
            let view = writer.view(picture, None);
            images.push(json!({"bufferView": view, "mimeType": "image/png"}));
            textures.push(json!({"source": images.len() - 1}));
            material["pbrMetallicRoughness"]["baseColorTexture"] = json!({"index": textures.len() - 1});
        }
        materials.push(material);
        let indices = writer.indices(&indices);
        primitives.push(json!({"attributes": attributes, "mode": 4, "material": materials.len() - 1, "indices": indices}));
    }
    let mut document = json!({
        "asset": {"version": "2.0", "generator": GENERATOR},
        "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": primitives}], "materials": materials,
        "buffers": [{"byteLength": writer.blob.len()}],
        "bufferViews": writer.views, "accessors": writer.accessors,
    });
    if let Some(extras) = extras {
        document["asset"]["extras"] = extras;
    }
    if !images.is_empty() {
        document["images"] = json!(images);
        document["textures"] = json!(textures);
    }
    let mut text = document.to_string().into_bytes();
    text.resize(text.len().next_multiple_of(4), b' ');
    let length = 12 + 8 + text.len() + 8 + writer.blob.len();
    let mut out = Vec::with_capacity(length);
    for word in [GLB_MAGIC, 2, length as u32, text.len() as u32, JSON_CHUNK] {
        out.extend_from_slice(&word.to_le_bytes());
    }
    out.extend_from_slice(&text);
    out.extend_from_slice(&(writer.blob.len() as u32).to_le_bytes());
    out.extend_from_slice(&BIN_CHUNK.to_le_bytes());
    out.extend_from_slice(&writer.blob);
    out
}

/// Оси восток, север, вверх в точке, строками.
pub fn enu_axes(lat: f64, lon: f64) -> [[f64; 3]; 3] {
    let (phi, lam) = (lat.to_radians(), lon.to_radians());
    [
        [-lam.sin(), lam.cos(), 0.0],
        [-phi.sin() * lam.cos(), -phi.sin() * lam.sin(), phi.cos()],
        [phi.cos() * lam.cos(), phi.cos() * lam.sin(), phi.sin()],
    ]
}

/// Вектор ECEF в осях восток, север, вверх.
fn turned(axes: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    axes.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2])
}

/// Один файл GLB: метры от точки lat, lon, height тела e, Y вверх.
pub fn scene_glb(e: &Ellipsoid, items: &[Item], lat: f64, lon: f64, height: f64) -> Vec<u8> {
    let origin = e.geodetic_to_ecef(lat, lon, height);
    let axes = enu_axes(lat, lon);
    let parts: Vec<Part> = items
        .iter()
        .map(|item| {
            let center = DVec3::from_array(item.center) - origin;
            let shift = turned(&axes, center.to_array());
            let positions = item
                .positions
                .iter()
                .map(|p| {
                    let local = add(turned(&axes, wide(*p)), shift);
                    // Восток, вверх, на юг - правая тройка glTF.
                    [local[0] as f32, local[2] as f32, -local[1] as f32]
                })
                .collect();
            let normals = item.normals.as_ref().map(|n| {
                n.iter()
                    .map(|v| {
                        let t = turned(&axes, wide(*v));
                        [t[0] as f32, t[2] as f32, -t[1] as f32]
                    })
                    .collect()
            });
            Part { positions, normals, item }
        })
        .collect();
    glb(&parts, Some(json!({"origin": {"lat": lat, "lon": lon, "height": height}})))
}

/// ECEF в оси glTF: читатель 3D Tiles вернёт (x, -z, y).
fn gltf_axes(values: &[[f32; 3]]) -> Vec<[f32; 3]> {
    values.iter().map(|v| [v[0], v[2], -v[1]]).collect()
}

/// 3D Tiles: текст `tileset.json` и файлы (имя, байты GLB), None - кусков
/// с вершинами нет.
pub fn tileset(items: &[Item]) -> Option<(String, Vec<TileFile>)> {
    let (mut files, mut children, mut spheres) = (Vec::new(), Vec::new(), Vec::new());
    for (number, item) in items.iter().enumerate() {
        if item.positions.is_empty() {
            continue;
        }
        let name = format!("tile{}.glb", number + 1);
        let part = Part { positions: gltf_axes(&item.positions), normals: item.normals.as_ref().map(|n| gltf_axes(n)), item };
        files.push((name.clone(), glb(&[part], None)));
        let radius = item.positions.iter().fold(0.0f64, |a, p| a.max(DVec3::from_array(wide(*p)).length())) + 1.0;
        let [x, y, z] = item.center;
        children.push(json!({
            "transform": [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, x, y, z, 1.0],
            "boundingVolume": {"sphere": [0.0, 0.0, 0.0, radius]},
            "geometricError": 0.0, "content": {"uri": name},
        }));
        spheres.push((DVec3::from_array(item.center), radius));
    }
    if children.is_empty() {
        return None;
    }
    let middle = spheres.iter().fold(DVec3::ZERO, |a, (c, _)| a + *c) / spheres.len() as f64;
    let reach = spheres.iter().fold(0.0f64, |a, (c, r)| a.max((*c - middle).length() + r));
    let root = json!({
        "boundingVolume": {"sphere": [middle.x, middle.y, middle.z, reach]},
        "geometricError": TILE_ERROR, "refine": "ADD", "children": children,
    });
    let text = json!({"asset": {"version": "1.1", "gltfUpAxis": "Y", "generator": GENERATOR}, "geometricError": TILE_ERROR, "root": root});
    Some((serde_json::to_string_pretty(&text).unwrap_or_default(), files))
}

/// Набор 3D Tiles одним архивом ZIP: у модуля - папка файлов. Программа
/// и страница отдают один файл, папку страница записать не может. None -
/// кусков с вершинами нет или архив не собрался.
pub fn tiles_zip(items: &[Item]) -> Option<Vec<u8>> {
    let (text, files) = tileset(items)?;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    writer.start_file("tileset.json", options).ok()?;
    writer.write_all(text.as_bytes()).ok()?;
    for (name, data) in &files {
        writer.start_file(name.as_str(), options).ok()?;
        writer.write_all(data).ok()?;
    }
    Some(writer.finish().ok()?.into_inner())
}
