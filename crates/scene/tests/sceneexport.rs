// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сцена вида в glTF и 3D Tiles. Перенос `tests/test_sceneexport.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use globe_core::ellipsoid::Ellipsoid;
use scene::sceneexport as se;
use serde_json::Value;

/// Треугольник: вершина up метров над точкой и две на земле.
fn wall(lat: f64, lon: f64, up: f64) -> se::Item {
    let center = Ellipsoid::wgs84().geodetic_to_ecef(lat, lon, 0.0);
    let axes = se::enu_axes(lat, lon);
    let points = [[0.0, 0.0, up], [5.0, 0.0, 0.0], [0.0, 5.0, 0.0]];
    let positions = points.iter().map(|p: &[f64; 3]| [0, 1, 2].map(|k| (p[0] * axes[0][k] + p[1] * axes[1][k] + p[2] * axes[2][k]) as f32)).collect();
    se::Item { center: center.to_array(), positions, colors: Some(vec![[255, 0, 0, 255]; 3]), indices: Some(vec![0, 1, 2]), ..Default::default() }
}

fn word(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

/// Документ и двоичный кусок файла GLB.
fn chunks(data: &[u8]) -> (Value, &[u8]) {
    let size = word(data, 12) as usize;
    (serde_json::from_slice(&data[20..20 + size]).unwrap(), &data[20 + size + 8..])
}

/// Значения float accessor с номером number по три.
fn floats(document: &Value, blob: &[u8], number: &Value) -> Vec<[f32; 3]> {
    let entry = &document["accessors"][number.as_u64().unwrap() as usize];
    let view = &document["bufferViews"][entry["bufferView"].as_u64().unwrap() as usize];
    let offset = view["byteOffset"].as_u64().unwrap() as usize;
    (0..entry["count"].as_u64().unwrap() as usize)
        .map(|i| [0, 1, 2].map(|k| f32::from_bits(word(blob, offset + (i * 3 + k) * 4))))
        .collect()
}

#[test]
fn structure() {
    let data = se::scene_glb(&Ellipsoid::wgs84(), &[wall(58.0, 56.0, 10.0)], 58.0, 56.0, 0.0);
    assert_eq!(word(&data, 0), se::GLB_MAGIC);
    assert_eq!(word(&data, 4), 2);
    assert_eq!(word(&data, 8) as usize, data.len());
    assert_eq!(data.len() % 4, 0);
    assert_eq!(chunks(&data).0["asset"]["extras"]["origin"]["lat"], 58.0);
}

#[test]
fn up_is_y_and_east_is_x() {
    let data = se::scene_glb(&Ellipsoid::wgs84(), &[wall(58.0, 56.0, 10.0)], 58.0, 56.0, 0.0);
    let (document, blob) = chunks(&data);
    let points = floats(&document, blob, &document["meshes"][0]["primitives"][0]["attributes"]["POSITION"]);
    assert!((points[0][1] - 10.0).abs() < 0.01, "{points:?}");
    assert!((points[1][0] - 5.0).abs() < 0.01, "{points:?}");
    // Север - минус Z.
    assert!((points[2][2] + 5.0).abs() < 0.01, "{points:?}");
}

#[test]
fn normals_are_unit() {
    let data = se::scene_glb(&Ellipsoid::wgs84(), &[wall(58.0, 56.0, 10.0)], 58.0, 56.0, 0.0);
    let (document, blob) = chunks(&data);
    for n in floats(&document, blob, &document["meshes"][0]["primitives"][0]["attributes"]["NORMAL"]) {
        assert!((n.iter().map(|v| v * v).sum::<f32>() - 1.0).abs() < 1e-5, "{n:?}");
    }
}

#[test]
fn own_reader_opens_it() {
    let (parts, _) = scene::gltf::read(&se::scene_glb(&Ellipsoid::wgs84(), &[wall(58.0, 56.0, 10.0)], 58.0, 56.0, 0.0), &|_| None).expect("читается");
    assert!(!parts.is_empty());
}

#[test]
fn picture_goes_inside_file() {
    let item = se::Item { colors: None, picture: Some(b"\x89PNG0000".to_vec()), uvs: Some(vec![[0.0; 2]; 3]), ..wall(58.0, 56.0, 10.0) };
    let data = se::scene_glb(&Ellipsoid::wgs84(), &[item], 58.0, 56.0, 0.0);
    let document = chunks(&data).0;
    assert_eq!(document["images"][0]["mimeType"], "image/png");
    assert!(document["materials"][0]["pbrMetallicRoughness"].get("baseColorTexture").is_some());
}

#[test]
fn vertices_return_to_ecef() {
    let item = wall(58.0, 56.0, 10.0);
    let (text, files) = se::tileset(std::slice::from_ref(&item)).expect("набор");
    let set: Value = serde_json::from_str(&text).unwrap();
    let child = &set["root"]["children"][0];
    assert_eq!(child["content"]["uri"], files[0].0.as_str());
    let (document, blob) = chunks(&files[0].1);
    let points = floats(&document, blob, &document["meshes"][0]["primitives"][0]["attributes"]["POSITION"]);
    let shift: Vec<f64> = (12..15).map(|i| child["transform"][i].as_f64().unwrap()).collect();
    for (p, want) in points.iter().zip(&item.positions) {
        // Читатель 3D Tiles: glTF Y вверх в Z вверх - (x, -z, y).
        let back = [f64::from(p[0]), -f64::from(p[2]), f64::from(p[1])];
        for k in 0..3 {
            assert!((back[k] + shift[k] - (f64::from(want[k]) + item.center[k])).abs() < 0.01);
        }
    }
}

#[test]
fn root_holds_every_tile() {
    let items = [wall(58.0, 56.0, 10.0), wall(58.01, 56.0, 10.0)];
    let (text, files) = se::tileset(&items).expect("набор");
    assert_eq!(files.len(), 2);
    let set: Value = serde_json::from_str(&text).unwrap();
    let sphere: Vec<f64> = (0..4).map(|i| set["root"]["boundingVolume"]["sphere"][i].as_f64().unwrap()).collect();
    for item in &items {
        let d: f64 = (0..3).map(|k| (item.center[k] - sphere[k]).powi(2)).sum();
        assert!(d.sqrt() <= sphere[3]);
    }
}

#[test]
fn empty_gives_nothing() {
    assert!(se::tileset(&[]).is_none());
    assert!(se::tiles_zip(&[]).is_none());
}

#[test]
fn tiles_lie_in_one_archive() {
    let data = se::tiles_zip(&[wall(58.0, 56.0, 10.0)]).expect("архив");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data)).expect("читается");
    let mut names: Vec<String> = (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().unwrap().to_string()).collect();
    names.sort();
    assert_eq!(names, ["tile1.glb", "tileset.json"]);
    let mut text = String::new();
    std::io::Read::read_to_string(&mut archive.by_name("tileset.json").unwrap(), &mut text).unwrap();
    assert!(text.contains("tile1.glb"));
}
