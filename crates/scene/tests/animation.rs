// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Анимация узлов 3D-моделей glTF и Collada, перенос
//! `tests/test_animation.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use globe_core::{DMat3, DMat4, DVec3, DVec4};
use scene::animation::{self, Mode};
use scene::{collada, gltf, models};
use serde_json::{Value, json};

fn near(a: DVec3, b: DVec3, tolerance: f64) -> bool {
    (a - b).length() < tolerance
}

fn moved(delta: &DMat4, point: DVec3) -> DVec3 {
    delta.transform_point3(point)
}

fn nothing(_: &str) -> Option<Vec<u8>> {
    None
}

/// .glb с треугольником у узла сетки и одним каналом анимации, как
/// `channel_doc` модуля. nodes - узлы JSON, канал - path узла node
/// по ключам times и values вида kind.
fn channel_doc(nodes: Value, path: &str, node: usize, times: &[f32], values: &[f32], kind: &str) -> Vec<u8> {
    let mut blob: Vec<u8> = Vec::new();
    let mut views: Vec<Value> = Vec::new();
    let mut accessors: Vec<Value> = Vec::new();
    let mut add = |data: Vec<u8>, count: usize, kind: &str, ctype: u32| {
        while !blob.len().is_multiple_of(4) {
            blob.push(0);
        }
        views.push(json!({"buffer": 0, "byteOffset": blob.len(), "byteLength": data.len()}));
        blob.extend(data);
        accessors.push(json!({"bufferView": views.len() - 1, "componentType": ctype, "count": count, "type": kind}));
        accessors.len() - 1
    };
    let floats = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let position = add(floats(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]), 3, "VEC3", 5126);
    let indices = add([0u16, 1, 2].iter().flat_map(|v| v.to_le_bytes()).collect(), 3, "SCALAR", 5123);
    let width = if kind == "VEC4" { 4 } else { 3 };
    let input = add(floats(times), times.len(), "SCALAR", 5126);
    let output = add(floats(values), values.len() / width, kind, 5126);
    while !blob.len().is_multiple_of(4) {
        blob.push(0);
    }
    let doc = json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": nodes,
        "meshes": [{"primitives": [{"attributes": {"POSITION": position}, "indices": indices}]}],
        "animations": [{
            "samplers": [{"input": input, "output": output}],
            "channels": [{"sampler": 0, "target": {"node": node, "path": path}}]
        }],
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{"byteLength": blob.len()}],
    });
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
    out.extend(&blob);
    out
}

#[test]
fn linear_step_and_ends() {
    let times = [0.0, 2.0];
    let values = vec![vec![0.0], vec![10.0]];
    assert!((animation::sample(&times, &values, 0.5, Mode::Linear).unwrap()[0] - 2.5).abs() < 1e-12);
    assert_eq!(animation::sample(&times, &values, 0.5, Mode::Step).unwrap()[0], 0.0);
    assert_eq!(animation::sample(&times, &values, -1.0, Mode::Linear).unwrap()[0], 0.0);
    assert_eq!(animation::sample(&times, &values, 5.0, Mode::Linear).unwrap()[0], 10.0);
}

#[test]
fn cubic_spline_with_flat_tangents() {
    // Касательная входа, значение, касательная выхода.
    let values = vec![vec![0.0], vec![0.0], vec![0.0], vec![0.0], vec![10.0], vec![0.0]];
    let mid = animation::sample(&[0.0, 1.0], &values, 0.5, Mode::Cubic).unwrap();
    assert!((mid[0] - 5.0).abs() < 1e-12);
    let quarter = animation::sample(&[0.0, 1.0], &values, 0.25, Mode::Cubic).unwrap();
    assert!(quarter[0] < 2.5);
}

#[test]
fn slerp_half_turn() {
    let half = 0.5_f64.sqrt();
    let q = animation::sample_rotation(&[0.0, 1.0], &[vec![0.0, 0.0, 0.0, 1.0], vec![0.0, half, 0.0, half]], 0.5, Mode::Linear).unwrap();
    let angle = 2.0 * q.w.acos().to_degrees();
    assert!((angle - 45.0).abs() < 1e-4, "{angle}");
}

#[test]
fn parent_translation_moves_child_mesh() {
    let nodes = json!([{"children": [1]}, {"mesh": 0, "translation": [1, 0, 0]}]);
    let data = channel_doc(nodes, "translation", 0, &[0.0, 2.0], &[0.0, 0.0, 0.0, 0.0, 10.0, 0.0], "VEC3");
    let (parts, _, moving) = gltf::read_animated(&data, &nothing).unwrap();
    let moving = moving.unwrap();
    assert_eq!(parts.iter().map(|p| p.track).collect::<Vec<_>>(), [Some(0)]);
    assert!((moving.duration - 2.0).abs() < 1e-9);
    let rest = parts[0].positions[0];
    assert!(near(rest, DVec3::X, 1e-9));
    // Y glTF вверх - Z модуля вверх.
    let delta = moving.deltas(1.0)[&0];
    assert!(near(moved(&delta, rest), DVec3::new(1.0, 0.0, 5.0), 1e-6));
    // Время идёт по кругу длительности.
    assert!(moving.deltas(3.0)[&0].abs_diff_eq(delta, 1e-9));
}

#[test]
fn rotation_about_up_axis() {
    let half = 0.5_f32.sqrt();
    let data = channel_doc(json!([{"mesh": 0}]), "rotation", 0, &[0.0, 1.0], &[0.0, 0.0, 0.0, 1.0, 0.0, half, 0.0, half], "VEC4");
    let (parts, _, moving) = gltf::read_animated(&data, &nothing).unwrap();
    let point = parts[0].positions[1];
    assert!(near(point, DVec3::X, 1e-9));
    let turned = moved(&moving.unwrap().deltas(0.5)[&0], point);
    // Поворот glTF вокруг Y на 45° - поворот вокруг Z модуля.
    let h = 0.5_f64.sqrt();
    assert!(near(turned, DVec3::new(h, h, 0.0), 1e-6), "{turned}");
}

#[test]
fn still_model_has_no_animation() {
    let files = vec![("m.dae".to_string(), DAE.replace("<channel source=\"#s\" target=\"n/location.X\"/>", "").into_bytes())];
    let (parts, _, moving) = models::read_animated(&models::pack("m.dae", &files, &BTreeMap::new()).unwrap()).unwrap();
    assert!(moving.is_none());
    assert!(parts.iter().all(|p| p.track.is_none()));
}

#[test]
fn merge_keeps_moving_parts_apart() {
    let still = models::Part::new(vec![DVec3::ZERO; 3], None, None, None, [1.0; 4]);
    let moving = models::Part { track: Some(0), ..models::Part::new(vec![DVec3::ONE; 3], None, None, None, [1.0; 4]) };
    let merged = models::merge(vec![still, moving]).unwrap();
    let mut tracks: Vec<bool> = merged.iter().map(|p| p.track.is_none()).collect();
    tracks.sort();
    assert_eq!(tracks, [false, true]);
}

const DAE: &str = r##"<?xml version="1.0" encoding="utf-8"?>
<COLLADA xmlns="http://www.collada.org/2005/11/COLLADASchema" version="1.4.1">
<asset><unit meter="0.5"/><up_axis>Z_UP</up_axis></asset>
<library_animations><animation>
<source id="t"><float_array id="ta" count="2">0 4</float_array>
<technique_common><accessor source="#ta" count="2" stride="1"/></technique_common></source>
<source id="x"><float_array id="xa" count="2">0 8</float_array>
<technique_common><accessor source="#xa" count="2" stride="1"/></technique_common></source>
<sampler id="s"><input semantic="INPUT" source="#t"/><input semantic="OUTPUT" source="#x"/></sampler>
<channel source="#s" target="n/location.X"/>
</animation></library_animations>
<library_geometries><geometry id="g"><mesh>
<source id="pos"><float_array id="pa" count="9">0 0 0 1 0 0 0 1 0</float_array>
<technique_common><accessor source="#pa" count="3" stride="3"/></technique_common></source>
<vertices id="v"><input semantic="POSITION" source="#pos"/></vertices>
<triangles count="1"><input semantic="VERTEX" source="#v" offset="0"/><p>0 1 2</p></triangles>
</mesh></geometry></library_geometries>
<library_visual_scenes><visual_scene id="vs">
<node id="n"><translate sid="location">0 0 0</translate><instance_geometry url="#g"/></node>
<node id="still"><instance_geometry url="#g"/></node>
</visual_scene></library_visual_scenes>
<scene><instance_visual_scene url="#vs"/></scene>
</COLLADA>
"##;

#[test]
fn keyed_translate_component_in_metres() {
    let (parts, moving) = collada::read_dae_animated(DAE.as_bytes()).unwrap();
    assert_eq!(parts.iter().map(|p| p.track).collect::<Vec<_>>(), [Some(0), None]);
    let moving = moving.unwrap();
    assert!((moving.duration - 4.0).abs() < 1e-9);
    let delta = moving.deltas(2.0)[&0];
    // 4 единицы файла по 0.5 м.
    assert!(near(delta.w_axis.truncate(), DVec3::new(2.0, 0.0, 0.0), 1e-9));
    assert!(DMat3::from_mat4(delta).abs_diff_eq(DMat3::IDENTITY, 1e-9));
}

#[test]
fn package_reads_animation() {
    let files = vec![("a.dae".to_string(), DAE.as_bytes().to_vec())];
    let (parts, _, moving) = models::read_animated(&models::pack("a.dae", &files, &BTreeMap::new()).unwrap()).unwrap();
    assert!(moving.is_some());
    let mut tracks: Vec<bool> = parts.iter().map(|p| p.track.is_none()).collect();
    tracks.sort();
    assert_eq!(tracks, [false, true]);
}

#[test]
fn frame_delta_in_ecef() {
    let frame = models::frame(57.9, 56.2, 30.0, 0.0, 0.0, [1.0; 3]);
    let mut delta = DMat4::IDENTITY;
    delta.w_axis = DVec4::new(0.0, 0.0, 3.0, 1.0);
    let (linear, shift) = animation::frame_delta(&frame, &delta);
    assert!(linear.abs_diff_eq(DMat3::IDENTITY, 1e-9));
    assert!(near(shift, frame * DVec3::new(0.0, 0.0, 3.0), 1e-9));
}
