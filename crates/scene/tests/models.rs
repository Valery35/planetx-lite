// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! 3D-модели: сетки Collada, glTF и OBJ, место по KML, пакет файлов.
//! Перенос `tests/test_models.py` модуля, кроме `vertices` - вершины
//! для видеокарты собирает интерфейс. Добавлены проверки битых
//! и враждебных файлов и чтение моделей демо модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::io::{Cursor, Write};
use std::path::PathBuf;

use globe_core::camera::enu;
use globe_core::ellipsoid::Ellipsoid;
use globe_core::{DVec2, DVec3};
use scene::models::{self, Part};
use scene::overlays::{self, Kind};
use scene::{collada, gltf, obj};
use scene::{kml, scene as sc};
use serde_json::{Value, json};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n0000000000000000";

// Как пишет SketchUp: дюймы, Z вверх, узел со сдвигом, картинка через
// surface и sampler2D.
const DAE: &str = r##"<?xml version="1.0" encoding="utf-8"?>
<COLLADA xmlns="http://www.collada.org/2005/11/COLLADASchema"
 version="1.4.1">
<asset><unit meter="0.0254" name="inch"/><up_axis>Z_UP</up_axis></asset>
<library_images><image id="img0"><init_from>suzd1/texture0.jpg</init_from>
</image></library_images>
<library_effects>
<effect id="fx0"><profile_COMMON>
<newparam sid="surf"><surface type="2D"><init_from>img0</init_from>
</surface></newparam>
<newparam sid="samp"><sampler2D><source>surf</source></sampler2D>
</newparam>
<technique sid="COMMON"><lambert><diffuse>
<texture texture="samp" texcoord="UVSET0"/></diffuse></lambert>
</technique></profile_COMMON></effect>
<effect id="fx1"><profile_COMMON><technique sid="COMMON"><lambert>
<diffuse><color>1 0 0 1</color></diffuse>
<transparency><float>0.5</float></transparency>
</lambert></technique></profile_COMMON></effect>
</library_effects>
<library_materials>
<material id="m0"><instance_effect url="#fx0"/></material>
<material id="m1"><instance_effect url="#fx1"/></material>
</library_materials>
<library_geometries><geometry id="g"><mesh>
<source id="pos"><float_array id="pa" count="15">
0 0 0 100 0 0 100 100 0 0 100 0 0 0 100</float_array>
<technique_common><accessor source="#pa" count="5" stride="3">
<param name="X" type="float"/><param name="Y" type="float"/>
<param name="Z" type="float"/></accessor></technique_common></source>
<source id="nrm"><float_array id="na" count="3">0 0 1</float_array>
<technique_common><accessor source="#na" count="1" stride="3"/>
</technique_common></source>
<source id="uv"><float_array id="ua" count="8">0 0 1 0 1 1 0 1
</float_array><technique_common><accessor source="#ua" count="4"
 stride="2"/></technique_common></source>
<vertices id="v"><input semantic="POSITION" source="#pos"/></vertices>
<polylist count="1" material="Mat0">
<input semantic="VERTEX" source="#v" offset="0"/>
<input semantic="NORMAL" source="#nrm" offset="1"/>
<input semantic="TEXCOORD" source="#uv" offset="2" set="0"/>
<vcount>4</vcount><p>0 0 0 1 0 1 2 0 2 3 0 3</p></polylist>
<triangles count="1" material="Mat1">
<input semantic="VERTEX" source="#v" offset="0"/>
<p>0 1 4</p></triangles>
<lines count="1"><input semantic="VERTEX" source="#v" offset="0"/>
<p>0 1</p></lines>
</mesh></geometry></library_geometries>
<library_nodes><node id="ln"><instance_geometry url="#g">
<bind_material><technique_common>
<instance_material symbol="Mat0" target="#m0"/>
<instance_material symbol="Mat1" target="#m1"/>
</technique_common></bind_material></instance_geometry></node>
</library_nodes>
<library_visual_scenes><visual_scene id="s">
<node><matrix>1 0 0 1000 0 1 0 0 0 0 1 0 0 0 0 1</matrix>
<instance_node url="#ln"/></node>
</visual_scene></library_visual_scenes>
<scene><instance_visual_scene url="#s"/></scene>
</COLLADA>
"##;

fn close(a: f64, b: f64, places: i32) -> bool {
    (a - b).abs() < 0.5 * 10f64.powi(-places)
}

fn near(a: DVec3, b: DVec3) -> bool {
    (a - b).abs().max_element() <= 1e-12
}

fn near2(a: DVec3, b: DVec3, tolerance: f64) -> bool {
    (a - b).abs().max_element() <= tolerance
}

// Collada.

#[test]
fn sketchup_style_file() {
    let parts = collada::read_dae(DAE.as_bytes()).unwrap();
    assert_eq!(parts.len(), 2);
    let (square, wall) = (&parts[0], &parts[1]);
    // Четырёхугольник - два треугольника веером.
    assert_eq!(square.positions.len(), 6);
    assert_eq!(square.image.as_deref(), Some("suzd1/texture0.jpg"));
    // Дюймы в метры, узел сдвинул на 1000 дюймов по X.
    let xs: Vec<f64> = square.positions.iter().map(|p| p.x).collect();
    assert!(close(xs.iter().copied().fold(f64::INFINITY, f64::min), 25.4, 7));
    assert!(close(xs.iter().copied().fold(f64::NEG_INFINITY, f64::max), 27.94, 7));
    assert!(near2(square.normals.as_ref().unwrap()[0], DVec3::Z, 1e-7));
    // v перевёрнута: угол с v = 0 в Collada - низ картинки.
    assert!(close(square.uv.as_ref().unwrap()[0].y, 1.0, 7));
    assert_eq!(wall.image, None);
    assert_eq!(wall.rgba, [1.0, 0.0, 0.0, 0.5]);
    assert!(wall.normals.is_none());
}

#[test]
fn images_list() {
    assert_eq!(models::collada_images(DAE.as_bytes()).unwrap(), ["suzd1/texture0.jpg"]);
    assert_eq!(collada::images(DAE.as_bytes()).unwrap(), ["suzd1/texture0.jpg"]);
}

#[test]
fn y_up_turns_to_z_up() {
    let data = DAE.replace("Z_UP", "Y_UP");
    let parts = collada::read_dae(data.as_bytes()).unwrap();
    // Вершина (0, 0, 100) дюймов по оси Z Collada Y_UP - на юг.
    assert!(near2(parts[1].positions[2], DVec3::new(25.4, -2.54, 0.0), 1e-9), "{}", parts[1].positions[2]);
}

#[test]
fn entities_refused() {
    let bad = br#"<!DOCTYPE c [<!ENTITY a "aaaa">]><COLLADA>&a;</COLLADA>"#;
    assert!(collada::read_dae(bad).is_err());
}

#[test]
fn collada_bad_index_is_model_error() {
    let bad = DAE.replace("<p>0 1 4</p>", "<p>0 1 44</p>");
    assert!(collada::read_dae(bad.as_bytes()).is_err());
}

#[test]
fn collada_self_instance_is_model_error() {
    // Узел, который вставляет сам себя: вложенность ограничена, ошибка
    // вместо переполнения стека.
    let bad = DAE.replace(r##"<instance_node url="#ln"/></node>"##, r##"<instance_node url="#ln"/></node><node id="loop"><instance_node url="#loop"/></node>"##);
    assert!(collada::read_dae(bad.as_bytes()).is_err());
}

// glTF.

/// Двоичный glTF из массивов: треугольники по номерам, картинка PNG
/// в буфере, матрица узла по столбцам.
fn glb(positions: &[[f32; 3]], indices: &[u16], uv: Option<&[[f32; 2]]>, picture: Option<&[u8]>, matrix: Option<[f64; 16]>) -> Vec<u8> {
    let mut blob: Vec<u8> = Vec::new();
    let mut views: Vec<Value> = Vec::new();
    let mut accessors: Vec<Value> = Vec::new();
    let mut add = |data: Vec<u8>, count: usize, kind: &str, ctype: u32, blob: &mut Vec<u8>| -> usize {
        while !blob.len().is_multiple_of(4) {
            blob.push(0);
        }
        views.push(json!({"buffer": 0, "byteOffset": blob.len(), "byteLength": data.len()}));
        blob.extend(data);
        accessors.push(json!({"bufferView": views.len() - 1, "componentType": ctype, "count": count, "type": kind}));
        accessors.len() - 1
    };
    let bytes = |values: &[f32]| values.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
    let mut attributes = serde_json::Map::new();
    let flat: Vec<f32> = positions.iter().flatten().copied().collect();
    attributes.insert("POSITION".into(), json!(add(bytes(&flat), positions.len(), "VEC3", 5126, &mut blob)));
    if let Some(uv) = uv {
        let flat: Vec<f32> = uv.iter().flatten().copied().collect();
        attributes.insert("TEXCOORD_0".into(), json!(add(bytes(&flat), uv.len(), "VEC2", 5126, &mut blob)));
    }
    let index_bytes: Vec<u8> = indices.iter().flat_map(|v| v.to_le_bytes()).collect();
    let indices = add(index_bytes, indices.len(), "SCALAR", 5123, &mut blob);
    let mut prim = json!({"attributes": attributes, "indices": indices});
    let mut node = json!({"mesh": 0});
    if let Some(matrix) = matrix {
        node["matrix"] = json!(matrix);
    }
    let mut doc = json!({"asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [node]});
    if let Some(picture) = picture {
        while !blob.len().is_multiple_of(4) {
            blob.push(0);
        }
        views.push(json!({"buffer": 0, "byteOffset": blob.len(), "byteLength": picture.len()}));
        blob.extend(picture);
        doc["images"] = json!([{"bufferView": views.len() - 1, "mimeType": "image/png"}]);
        doc["textures"] = json!([{"source": 0}]);
        doc["materials"] = json!([{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}, "baseColorFactor": [1, 1, 1, 1]}}]);
        prim["material"] = json!(0);
    }
    while !blob.len().is_multiple_of(4) {
        blob.push(0);
    }
    doc["meshes"] = json!([{"primitives": [prim]}]);
    doc["accessors"] = json!(accessors);
    doc["bufferViews"] = json!(views);
    doc["buffers"] = json!([{"byteLength": blob.len()}]);
    glb_of(&doc, &blob)
}

/// Байты .glb из документа JSON и двоичного куска.
fn glb_of(doc: &Value, blob: &[u8]) -> Vec<u8> {
    let mut text = doc.to_string().into_bytes();
    while !text.len().is_multiple_of(4) {
        text.push(b' ');
    }
    let total = 12 + 8 + text.len() + 8 + blob.len();
    let mut out = b"glTF".to_vec();
    out.extend(2u32.to_le_bytes());
    out.extend((total as u32).to_le_bytes());
    out.extend((text.len() as u32).to_le_bytes());
    out.extend(b"JSON");
    out.extend(&text);
    out.extend((blob.len() as u32).to_le_bytes());
    out.extend(b"BIN\0");
    out.extend(blob);
    out
}

const TRIANGLE: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

fn nothing(_: &str) -> Option<Vec<u8>> {
    None
}

#[test]
fn glb_triangle_with_picture() {
    let data = glb(&TRIANGLE, &[0, 1, 2], Some(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]), Some(PNG), None);
    let (parts, pictures) = gltf::read(&data, &nothing).unwrap();
    assert_eq!(parts.len(), 1);
    let part = &parts[0];
    assert_eq!(part.image.as_deref(), Some("gltf:0"));
    assert_eq!(pictures["gltf:0"], PNG);
    // Y вверх glTF - Z вверх модуля.
    assert!(near2(part.positions[2], DVec3::Z, 1e-7));
}

#[test]
fn node_matrix_by_columns() {
    let mut matrix = [0.0; 16];
    for (i, v) in [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 5.0, 0.0, 0.0, 1.0].into_iter().enumerate() {
        matrix[i] = v;
    }
    let data = glb(&TRIANGLE, &[0, 1, 2], None, None, Some(matrix));
    let (parts, _) = gltf::read(&data, &nothing).unwrap();
    assert!(close(parts[0].positions[0].x, 5.0, 7));
}

#[test]
fn draco_refused() {
    let text = json!({"asset": {"version": "2.0"}, "extensionsRequired": ["KHR_draco_mesh_compression"]}).to_string();
    let found = gltf::read(text.as_bytes(), &nothing);
    assert!(found.unwrap_err().0.contains("KHR_draco_mesh_compression"));
}

#[test]
fn strip_unrolls() {
    assert_eq!(gltf::triangles(5, 4, None), [0, 1, 2, 1, 3, 2]);
    assert_eq!(gltf::triangles(6, 4, None), [0, 1, 2, 0, 2, 3]);
    assert_eq!(gltf::triangles(4, 7, None), [0, 1, 2, 3, 4, 5]);
    assert!(gltf::triangles(1, 4, None).is_empty());
}

#[test]
fn gltf_bad_files_are_model_errors() {
    let good = glb(&TRIANGLE, &[0, 1, 2], None, None, None);
    let (doc, blob) = models::split_glb(&good).unwrap();
    let doc: Value = serde_json::from_slice(doc).unwrap();
    let blob = blob.unwrap().to_vec();
    let broken = |change: &dyn Fn(&mut Value)| {
        let mut doc = doc.clone();
        change(&mut doc);
        gltf::read(&glb_of(&doc, &blob), &nothing)
    };
    // Номер вершины за концом массива.
    assert!(broken(&|d| d["accessors"][1]["byteOffset"] = json!(0)).is_ok());
    let data = glb(&TRIANGLE, &[0, 1, 7], None, None, None);
    assert!(gltf::read(&data, &nothing).is_err());
    // accessor за краем bufferView и bufferView за краем буфера.
    assert!(broken(&|d| d["accessors"][0]["count"] = json!(1000)).is_err());
    assert!(broken(&|d| d["bufferViews"][0]["byteLength"] = json!(1_000_000)).is_err());
    // Огромный accessor без bufferView - ошибка, а не гигабайты нулей.
    assert!(
        broken(&|d| {
            d["accessors"][0] = json!({"componentType": 5126, "count": 4_000_000_000u64, "type": "VEC3"});
        })
        .is_err()
    );
    // Узел, который сам себе ребёнок.
    assert!(broken(&|d| d["nodes"][0]["children"] = json!([0])).is_err());
    // Неверные номера и типы.
    assert!(broken(&|d| d["nodes"][0]["mesh"] = json!(5)).is_err());
    assert!(broken(&|d| d["accessors"][0]["componentType"] = json!(1)).is_err());
    assert!(broken(&|d| d["accessors"][0]["type"] = json!("VEC2")).is_err());
    // Файл, которого нет.
    assert!(broken(&|d| d["buffers"][0]["uri"] = json!("absent.bin")).is_err());
    assert!(gltf::read(b"[1, 2]", &nothing).is_err());
    assert!(gltf::read(b"glTF\x02\0\0\0", &nothing).is_err());
}

#[test]
fn gltf_data_uri_buffer() {
    // Файл .gltf с буфером в адресе data: base64.
    let good = glb(&TRIANGLE, &[0, 1, 2], None, None, None);
    let (doc, blob) = models::split_glb(&good).unwrap();
    let mut doc: Value = serde_json::from_slice(doc).unwrap();
    doc["buffers"][0]["uri"] = json!(format!("data:application/octet-stream;base64,{}", base64(blob.unwrap())));
    let (parts, _) = gltf::read(doc.to_string().as_bytes(), &nothing).unwrap();
    assert_eq!(parts[0].positions.len(), 3);
    // Файл буфера рядом, через fetch.
    doc["buffers"][0]["uri"] = json!("tri.bin");
    let bin = blob.unwrap().to_vec();
    let fetch = move |path: &str| (path == "tri.bin").then(|| bin.clone());
    let (parts, _) = gltf::read(doc.to_string().as_bytes(), &fetch).unwrap();
    assert!(near2(parts[0].positions[1], DVec3::X, 1e-7));
}

fn base64(data: &[u8]) -> String {
    const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for k in 0..4 {
            if k <= chunk.len() {
                out.push(char::from(ABC[((n >> (18 - 6 * k)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

// Пакет модели.

#[test]
fn pack_read_dae_with_texture() {
    let files = vec![("models/suzd1.dae".to_string(), DAE.as_bytes().to_vec()), ("models/suzd1/texture0.jpg".to_string(), PNG.to_vec())];
    let data = models::pack("models/suzd1.dae", &files, &BTreeMap::new()).unwrap();
    let (parts, pictures) = models::read(&data).unwrap();
    let mut images: Vec<String> = parts.iter().map(|p| p.image.clone().unwrap_or_default()).collect();
    images.sort();
    assert_eq!(images, ["", "models/suzd1/texture0.jpg"]);
    assert_eq!(pictures["models/suzd1/texture0.jpg"], PNG);
}

#[test]
fn alias_and_missing_picture() {
    let files = vec![("a.dae".to_string(), DAE.as_bytes().to_vec()), ("images/t.png".to_string(), PNG.to_vec())];
    let aliases: BTreeMap<String, String> = [("suzd1/texture0.jpg".to_string(), "images/t.png".to_string())].into();
    let (_, pictures) = models::read(&models::pack("a.dae", &files, &aliases).unwrap()).unwrap();
    assert!(pictures.contains_key("images/t.png"));
    let files = vec![("a.dae".to_string(), DAE.as_bytes().to_vec())];
    let (parts, pictures) = models::read(&models::pack("a.dae", &files, &BTreeMap::new()).unwrap()).unwrap();
    // Без картинки часть остаётся цветом материала.
    assert!(pictures.is_empty());
    assert!(parts.iter().all(|p| p.image.is_none()));
}

#[test]
fn flat_normals_added() {
    let files = vec![("a.dae".to_string(), DAE.as_bytes().to_vec())];
    let (parts, _) = models::read(&models::pack("a.dae", &files, &BTreeMap::new()).unwrap()).unwrap();
    for part in &parts {
        assert_eq!(part.normals.as_ref().unwrap().len(), part.positions.len());
        assert_eq!(part.uv.as_ref().unwrap().len(), part.positions.len());
    }
    // Нормаль грани по обходу против часовой стрелки.
    let normals = models::flat_normals(&[DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::ZERO, DVec3::X, DVec3::X]);
    assert_eq!(normals, [DVec3::Z, DVec3::Z, DVec3::Z, DVec3::ZERO, DVec3::ZERO, DVec3::ZERO]);
}

#[test]
fn glb_package() {
    let files = vec![("m.glb".to_string(), glb(&TRIANGLE, &[0, 1, 2], None, None, None))];
    let (parts, _) = models::read(&models::pack("m.glb", &files, &BTreeMap::new()).unwrap()).unwrap();
    assert_eq!(parts[0].positions.len(), 3);
}

#[test]
fn vertex_limit() {
    let many = Part::new(vec![DVec3::ZERO; 9], None, None, None, models::GRAY);
    assert!(models::merge_limit(vec![many.clone()], 6).is_err());
    assert_eq!(models::merge_limit(vec![many], 9).unwrap()[0].positions.len(), 9);
}

#[test]
fn merge_groups_by_material() {
    let red = [1.0, 0.0, 0.0, 1.0];
    let tri = vec![DVec3::ZERO, DVec3::X, DVec3::Y];
    let parts = vec![
        Part::new(tri.clone(), None, None, None, red),
        Part::new(tri.clone(), None, None, None, models::GRAY),
        Part::new(tri.clone(), None, Some(vec![DVec2::ONE; 3]), Some("a.png".into()), red),
        Part::new(tri.iter().map(|p| *p * f64::NAN).collect(), None, None, None, red),
        Part::new(vec![DVec3::ZERO; 2], None, None, None, red),
        Part::new([tri.clone(), vec![DVec3::Z]].concat(), None, None, None, red),
    ];
    let merged = models::merge(parts).unwrap();
    // Красные без картинки - вместе, неполный треугольник отброшен,
    // часть с NaN и часть короче треугольника пропущены.
    assert_eq!(merged.len(), 3);
    assert_eq!(merged[0].positions.len(), 6);
    assert_eq!(merged[2].image.as_deref(), Some("a.png"));
    assert_eq!(merged[2].uv.as_ref().unwrap()[0], DVec2::ONE);
    assert_eq!(merged[0].uv.as_ref().unwrap()[0], DVec2::ZERO);
    assert!(models::merge(Vec::new()).is_err());
}

#[test]
fn y_up_turn() {
    let part = Part::new(vec![DVec3::new(1.0, 2.0, 3.0)], Some(vec![DVec3::Z]), None, None, models::GRAY);
    let turned = models::from_y_up(part);
    assert_eq!(turned.positions[0], DVec3::new(1.0, -3.0, 2.0));
    assert_eq!(turned.normals.unwrap()[0], DVec3::new(0.0, -1.0, 0.0));
}

#[test]
fn radius() {
    let part = Part::new(vec![DVec3::new(3.0, 4.0, 0.0), DVec3::ZERO, DVec3::ZERO], None, None, None, models::GRAY);
    let parts = [part];
    assert!(close(models::radius(&parts, [1.0, 1.0, 1.0]), 5.0, 7));
    assert!(close(models::radius(&parts, [2.0, 2.0, 2.0]), 10.0, 7));
    assert!(models::radius(&parts, [1.0, 1.0, 1.0]).is_finite());
    assert_eq!(models::bounds(&parts), Some([DVec3::ZERO, DVec3::new(3.0, 4.0, 0.0)]));
    assert_eq!(models::bounds(&[]), None);
}

#[test]
fn broken_files_do_not_panic() {
    // Каждое начало файла и файл с испорченным байтом читаются без
    // паники: ошибка или части.
    let data = glb(&TRIANGLE, &[0, 1, 2], Some(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]), Some(PNG), None);
    for end in 0..data.len() {
        let _ = gltf::read(&data[..end], &nothing);
    }
    for at in 0..data.len() {
        let mut bad = data.clone();
        bad[at] ^= 0x5A;
        let _ = gltf::read(&bad, &nothing);
    }
    for end in (0..DAE.len()).filter(|e| DAE.is_char_boundary(*e)) {
        let _ = collada::read_dae(&DAE.as_bytes()[..end]);
    }
    for end in (0..OBJ.len()).filter(|e| OBJ.is_char_boundary(*e)) {
        let _ = obj::read(&OBJ.as_bytes()[..end], &files);
    }
    let package = models::pack("m.glb", &vec![("m.glb".to_string(), data)], &BTreeMap::new()).unwrap();
    for end in 0..package.len() {
        let _ = models::read(&package[..end]);
    }
    assert!(models::read(b"not a zip").is_err());
}

// Место модели.

#[test]
fn heading_turns_clockwise() {
    // Ось X модели (восток) при азимуте 90° смотрит на юг.
    let turned = models::rotation(90.0, 0.0, 0.0) * DVec3::X;
    assert!(near(turned, DVec3::new(0.0, -1.0, 0.0)), "{turned}");
    // Ось Y (север) - на восток.
    let turned = models::rotation(90.0, 0.0, 0.0) * DVec3::Y;
    assert!(near(turned, DVec3::X), "{turned}");
}

#[test]
fn frame_maps_model_axes_to_enu() {
    let (lat, lon) = (56.42, 40.44);
    let [east, north, up] = enu(lat, lon);
    let matrix = models::frame(lat, lon, 0.0, 0.0, 0.0, [2.0, 2.0, 2.0]);
    assert!(near(matrix * DVec3::Z, up * 2.0));
    assert!(near(matrix * DVec3::X, east * 2.0));
    assert!(near(matrix * DVec3::Y, north * 2.0));
}

#[test]
fn anchor_modes() {
    let e = Ellipsoid::wgs84();
    let height = |mode: &str, alt: f64| e.ecef_to_geodetic(models::anchor(10.0, 20.0, mode, alt, 150.0, 2.0)).2;
    assert!(close(height("clampToGround", 30.0), 150.0, 3));
    assert!(close(height("relativeToGround", 30.0), 180.0, 3));
    assert!(close(height("absolute", 100.0), 200.0, 3));
}

// Модель в KML и KMZ.

const MODEL_KML: &str = r#"<kml xmlns="http://www.opengis.net/kml/2.2"><Document>
<Placemark><name>Church</name><Model><altitudeMode>relativeToGround
</altitudeMode><Location><longitude>40.44</longitude><latitude>56.42
</latitude><altitude>2</altitude></Location><Orientation>
<heading>88.41885380513968</heading><tilt>0</tilt><roll>0</roll>
</Orientation><Scale><x>0.6721278848236247</x>
<y>1</y><z>1.5</z></Scale><Link><href>models/church.dae</href></Link>
<ResourceMap><Alias><targetHref>../images/t.png</targetHref>
<sourceHref>suzd1/texture0.jpg</sourceHref></Alias></ResourceMap>
</Model></Placemark></Document></kml>"#;

fn kmz(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in files {
        writer.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn read() -> kml::Folder {
    let data = kmz(&[
        ("doc.kml", MODEL_KML.as_bytes()),
        ("models/church.dae", DAE.as_bytes()),
        ("images/t.png", PNG),
        ("images/unused.png", PNG),
    ]);
    kml::read_file(&data, "x").unwrap()
}

/// Картинка модели в пакете, как её нашло бы чтение сетки.
fn picture(package: &[u8]) -> Vec<u8> {
    let p = models::unpack(package).unwrap();
    let path = models::resolve(&p.files, &p.main, "suzd1/texture0.jpg", &p.aliases).unwrap();
    models::file(&p.files, &path).unwrap().to_vec()
}

/// Картинки и количество вершин модели по чтению сетки.
fn mesh(package: &[u8]) -> (Vec<Vec<u8>>, usize) {
    let (parts, pictures) = models::read(package).unwrap();
    (pictures.into_values().collect(), parts.iter().map(|p| p.positions.len()).sum())
}

#[test]
fn kmz_model_with_resource_map() {
    let tree = read();
    let item = tree.overlays()[0];
    assert_eq!(item.kind(), Kind::Model);
    assert_eq!(item.name, "Church");
    assert_eq!(item.overlay.camera.unwrap()[..4], [56.42, 40.44, 2.0, 88.41885380513968]);
    assert_eq!(item.overlay.scale, [0.6721278848236247, 1.0, 1.5]);
    assert_eq!(item.overlay.mode, "relativeToGround");
    let image = item.image.as_deref().unwrap();
    assert_eq!(picture(image), PNG);
    assert_eq!(mesh(image).0, [PNG]);
    // В пакет идёт только картинка по ссылке модели.
    assert_eq!(models::unpack(image).unwrap().files.len(), 2);
}

#[test]
fn round_trip() {
    let tree = read();
    let again = kml::read_file(&kml::write_kmz(&tree).unwrap(), "y").unwrap();
    let again = again.overlays()[0];
    let item = tree.overlays()[0];
    // Место без потери знаков: азимут и масштаб из KMZ Google Earth
    // с 16 знаками, запись {:g} оставляла 6.
    assert_eq!(again.overlay.params(), item.overlay.params());
    assert_eq!(picture(again.image.as_deref().unwrap()), PNG);
    assert_eq!(mesh(again.image.as_deref().unwrap()), (vec![PNG.to_vec()], 9));
}

#[test]
fn scene_carries_model_files() {
    // Сцена с моделью - places.kmz с файлами модели. Прежде метки сцены
    // шли простым KML, и модель теряла файлы.
    let tree = read();
    let places = kml::write_kmz(&tree).unwrap();
    let data = sc::write_scene(&sc::Scene::new(&[56.42, 40.44, 300.0]), sc::Places::Kmz(&places)).unwrap();
    let archive = zip::ZipArchive::new(Cursor::new(data.as_slice())).unwrap();
    let names: Vec<String> = archive.file_names().map(|n| n.unwrap().into_owned()).collect();
    assert!(names.iter().any(|n| n == sc::KMZ_FILE));
    assert!(!names.iter().any(|n| n == sc::PLACES_FILE));
    let (_, places) = sc::read_scene(&data).unwrap();
    let tree = kml::read_file(&places, "z").unwrap();
    assert_eq!(picture(tree.overlays()[0].image.as_deref().unwrap()), PNG);
    assert_eq!(mesh(tree.overlays()[0].image.as_deref().unwrap()), (vec![PNG.to_vec()], 9));
}

#[test]
fn plain_kml_keeps_href() {
    let tree = kml::read_kml(MODEL_KML.as_bytes(), "").unwrap();
    let item = tree.overlays()[0].clone();
    assert_eq!(item.image, None);
    assert_eq!(item.href, "models/church.dae");
    let aliases: BTreeMap<String, String> = [("suzd1/texture0.jpg".to_string(), "../images/t.png".to_string())].into();
    assert_eq!(item.aliases, aliases);
    let text = kml::write_kml(&kml::Folder::with("d", vec![kml::Item::Overlay(item)]));
    assert!(text.contains("<sourceHref>suzd1/texture0.jpg</sourceHref>"));
}

#[test]
fn overlay_params_round_trip() {
    let tree = read();
    let item = tree.overlays()[0];
    let back = overlays::from_params(Kind::Model, &item.overlay.params(), None);
    assert_eq!(back.camera, item.overlay.camera);
    assert_eq!(back.scale, item.overlay.scale);
    assert_eq!(back.mode, item.overlay.mode);
}

// OBJ с материалами MTL.

const OBJ: &str = "# square with a picture and a coloured triangle
mtllib walls.mtl
v 0 0 0
v 2 0 0
v 2 3 0
v 0 3 0
vt 0 0
vt 1 0
vt 1 1
vt 0 1
vn 0 0 1
usemtl brick
f 1/1/1 2/2/1 3/3/1 4/4/1
usemtl red
f -4//1 -3//1 -2//1
l 1 2
";
const MTL: &str = "newmtl brick
Kd 1 1 1
map_Kd -s 1 1 1 tex/brick.png
newmtl red
Kd 1 0 0
d 0.5
";

fn files(path: &str) -> Option<Vec<u8>> {
    match path {
        "walls.mtl" => Some(MTL.as_bytes().to_vec()),
        "tex/brick.png" => Some(PNG.to_vec()),
        _ => None,
    }
}

#[test]
fn faces_materials_and_y_up() {
    let parts = obj::read(OBJ.as_bytes(), &files).unwrap();
    let (brick, red) = (&parts[0], &parts[1]);
    assert_eq!(brick.positions.len(), 6);
    assert_eq!(brick.image.as_deref(), Some("tex/brick.png"));
    // Ось Y файла - вверх: вершина (2, 3, 0) встаёт на 3 м.
    assert_eq!(brick.positions[2], DVec3::new(2.0, 0.0, 3.0));
    // v перевёрнута, нормаль +Z файла смотрит на юг.
    assert!(close(brick.uv.as_ref().unwrap()[0].y, 1.0, 7));
    assert_eq!(brick.normals.as_ref().unwrap()[0], DVec3::new(0.0, -1.0, 0.0));
    assert_eq!(red.image, None);
    assert_eq!(red.rgba, [1.0, 0.0, 0.0, 0.5]);
}

#[test]
fn package_from_files() {
    let data = models::collect("walls.obj", OBJ.as_bytes(), &files, &BTreeMap::new()).unwrap();
    let mut packed: Vec<String> = models::unpack(&data).unwrap().files.into_iter().map(|(n, _)| n).collect();
    packed.sort();
    assert_eq!(packed, ["tex/brick.png", "walls.mtl", "walls.obj"]);
    assert_eq!(mesh(&data), (vec![PNG.to_vec()], 9));
}

#[test]
fn missing_mtl_gives_gray() {
    let parts = obj::read(OBJ.as_bytes(), &nothing).unwrap();
    assert!(parts.iter().all(|p| p.image.is_none() && p.rgba == models::GRAY));
}

#[test]
fn obj_bad_index_is_model_error() {
    assert!(obj::read(b"v 0 0 0\nf 1 2 3\n", &nothing).is_err());
    assert!(obj::read(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf /1 /2 /3\n", &nothing).is_err());
    assert!(obj::read(b"v 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n", &nothing).is_err());
    assert!(obj::read(b"v 0 0 x\n", &nothing).is_err());
    assert!(obj::read(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 99999999999999999999\n", &nothing).is_err());
}

// Модели демо модуля.

/// Эталон модуля: сцена, главный файл модели, вершины частей после
/// merge, картинки, рамка (наименьшие и наибольшие X, Y, Z, метры).
/// Снят `models.read` модуля в Python QGIS 4.0.3 23 октября 2026 года.
type Reference = (&'static str, &'static str, &'static [usize], usize, [f64; 6]);
const DEMO_MODELS: [Reference; 4] = [
    ("turbines", "turbine.glb", &[132, 144], 0, [-35.141, -6.0, 0.0, 35.141, 7.0, 120.0]),
    ("haul_kachkanar", "haul_truck.glb", &[72, 36, 180, 1008], 0, [-4.4, -5.6, 0.038, 4.4, 5.8, 5.6]),
    (
        "models",
        "suzd1.dae",
        &[6, 9, 54, 24, 9, 15, 6, 12, 15, 9, 3, 21, 9, 6, 6, 12, 33, 6, 30],
        18,
        [0.0, 0.0, 0.25, 30.35, 24.21, 24.36],
    ),
    ("models", "bah.dae", &[51117, 1344, 474, 519], 0, [-2.912, 0.896, -0.011, 26.93, 27.585, 25.77]),
];

#[test]
fn demo_models_read_like_module() {
    let dir = std::env::var_os("PLANETX_DEMO").map_or_else(|| PathBuf::from(r"C:\Dev\planetx\planetx\demo"), PathBuf::from);
    let mut checked = 0;
    for name in ["turbines", "haul_kachkanar", "models"] {
        let Ok(bytes) = std::fs::read(dir.join(format!("{name}.planetx"))) else {
            eprintln!("сцены {name} демо модуля нет, проверка пропущена");
            continue;
        };
        let (_, places) = sc::read_scene(&bytes).unwrap();
        let tree = kml::read_file(&places, name).unwrap();
        for item in tree.overlays().into_iter().filter(|i| i.kind() == Kind::Model) {
            let package = item.image.as_deref().unwrap();
            let main = models::unpack(package).unwrap().main;
            let (parts, pictures) = models::read(package).unwrap_or_else(|e| panic!("{name} {main}: {e}"));
            let counts: Vec<usize> = parts.iter().map(|p| p.positions.len()).collect();
            let [lo, hi] = models::bounds(&parts).unwrap();
            println!("{name} {main} частей {} вершин {:?} картинок {} рамка {lo} {hi}", parts.len(), counts, pictures.len());
            let (_, _, expected, images, frame) =
                DEMO_MODELS.iter().find(|m| m.0 == name && m.1 == main).unwrap_or_else(|| panic!("{name} {main}: нет в эталоне"));
            assert_eq!(counts, *expected, "{name} {main}");
            assert_eq!(pictures.len(), *images, "{name} {main}");
            assert_eq!(parts.iter().filter(|p| p.image.is_some()).count(), *images, "{name} {main}");
            for (got, want) in [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z].into_iter().zip(frame) {
                assert!((got - want).abs() <= 0.0006, "{name} {main}: рамка {got} против {want}");
            }
            for part in &parts {
                assert_eq!(part.normals.as_ref().unwrap().len(), part.positions.len());
                assert_eq!(part.uv.as_ref().unwrap().len(), part.positions.len());
                if let Some(image) = &part.image {
                    assert!(pictures.contains_key(image));
                }
            }
            checked += 1;
        }
    }
    println!("моделей демо проверено {checked}");
}
