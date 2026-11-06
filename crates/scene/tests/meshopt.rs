// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Распаковка EXT_meshopt_compression: пример из спецификации, круговая
//! проверка простыми кодировщиками по той же спецификации, фильтры,
//! враждебные потоки и модель glTF со сжатыми bufferView.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use scene::meshopt::{Filter, Mode, decode};
use serde_json::json;

fn zigzag8(v: u8) -> u8 {
    if v & 0x80 != 0 { !(v << 1) } else { v << 1 }
}

fn zigzag32(d: i32) -> u32 {
    ((d << 1) ^ (d >> 31)) as u32
}

fn varint(out: &mut Vec<u8>, mut v: u32) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

/// Кодировщик ATTRIBUTES по спецификации: все группы - байтами (bits 3).
fn encode_attributes(data: &[u8], stride: usize) -> Vec<u8> {
    let count = data.len() / stride;
    let mut out = vec![0xa0];
    let block_max = ((8192 / stride) & !15).min(256);
    let mut last = vec![0u8; stride];
    let mut done = 0;
    while done < count {
        let elements = (count - done).min(block_max);
        let groups = elements.div_ceil(16);
        for (k, prev) in last.iter_mut().enumerate() {
            out.extend(std::iter::repeat_n(0xffu8, groups.div_ceil(4)));
            let mut p = *prev;
            for j in 0..groups * 16 {
                if j < elements {
                    let v = data[(done + j) * stride + k];
                    out.push(zigzag8(v.wrapping_sub(p)));
                    p = v;
                } else {
                    out.push(0);
                }
            }
            *prev = p;
        }
        done += elements;
    }
    // Хвост: основа - нулевой элемент, дополненный до 32 байтов.
    out.extend(std::iter::repeat_n(0u8, stride.max(32)));
    out
}

/// Кодировщик TRIANGLES по спецификации: каждый треугольник - код 0xff
/// и три номера явно.
fn encode_triangles(indices: &[u32]) -> Vec<u8> {
    let mut out = vec![0xe1];
    out.extend(std::iter::repeat_n(0xffu8, indices.len() / 3));
    let mut last = 0u32;
    for tri in indices.chunks(3) {
        out.push(0xff);
        for &i in tri {
            varint(&mut out, zigzag32(i.wrapping_sub(last) as i32));
            last = i;
        }
    }
    out.extend([0u8; 16]);
    out
}

/// Кодировщик INDICES: дельты от основы 0.
fn encode_indices(indices: &[u32]) -> Vec<u8> {
    let mut out = vec![0xd1];
    let mut last = 0u32;
    for &i in indices {
        varint(&mut out, zigzag32(i.wrapping_sub(last) as i32) << 1);
        last = i;
    }
    out.extend([0u8; 4]);
    out
}

#[test]
fn spec_example_group_of_four_bit_deltas() {
    // 16 элементов по 4 байта: байт 0 - группа 4-битных дельт примера
    // спецификации, байты 1-3 - нули.
    let mut s = vec![0xa0, 0b10];
    s.extend([0x17, 0x5f, 0xf0, 0xbc, 0x77, 0xa9, 0x21, 0x00, 0x34, 0xb5]);
    s.extend([0, 0, 0]);
    s.extend([0u8; 32]);
    let out = decode(&s, 16, 4, Mode::Attributes, Filter::None).unwrap();
    let deltas: [i32; 16] = [-1, -4, -3, 26, -91, 0, -6, 6, -4, -4, 5, -5, 1, -1, 0, 0];
    let mut value = 0i32;
    for (j, d) in deltas.iter().enumerate() {
        value = (value + d).rem_euclid(256);
        assert_eq!(i32::from(out[j * 4]), value, "элемент {j}");
        assert_eq!(&out[j * 4 + 1..j * 4 + 4], &[0, 0, 0]);
    }
}

#[test]
fn attributes_round_trip_over_several_blocks() {
    // 300 элементов по 12 байтов - блоки 256 и 44 элемента.
    let data: Vec<u8> = (0..300 * 12).map(|i: u32| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    let s = encode_attributes(&data, 12);
    assert_eq!(decode(&s, 300, 12, Mode::Attributes, Filter::None).unwrap(), data);
    // Лишний байт или нехватка байта - ошибка.
    let mut longer = s.clone();
    longer.insert(1, 0);
    assert!(decode(&longer, 300, 12, Mode::Attributes, Filter::None).is_err());
    assert!(decode(&s[..s.len() - 1], 300, 12, Mode::Attributes, Filter::None).is_err());
}

#[test]
fn triangles_and_indices_round_trip() {
    let tris: Vec<u32> = vec![0, 1, 2, 2, 1, 3, 70_000, 5, 3, 3, 5, 0];
    let s = encode_triangles(&tris);
    let out = decode(&s, tris.len(), 4, Mode::Triangles, Filter::None).unwrap();
    let got: Vec<u32> = out.chunks(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    assert_eq!(got, tris);
    // Короткие коды: 0xfe с aux 0 - три новых номера, 0x10 - ребро 1
    // и новый номер.
    let mut s = vec![0xe1, 0xfe, 0x10, 0x00];
    s.extend([0u8; 16]);
    let out = decode(&s, 6, 2, Mode::Triangles, Filter::None).unwrap();
    let got: Vec<u16> = out.chunks(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
    assert_eq!(got, [0, 1, 2, 2, 1, 3]);
    let list: Vec<u32> = vec![5, 3, 10, 1_000_000, 0];
    let out = decode(&encode_indices(&list), list.len(), 4, Mode::Indices, Filter::None).unwrap();
    let got: Vec<u32> = out.chunks(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    assert_eq!(got, list);
}

#[test]
fn filters_follow_the_spec() {
    // Порядок -2, мантисса 6 - 1.5.
    let mut s = encode_attributes(&(((-2i32) << 24) | 6).to_le_bytes(), 4);
    let out = decode(&s, 1, 4, Mode::Attributes, Filter::Exponential).unwrap();
    assert_eq!(f32::from_le_bytes([out[0], out[1], out[2], out[3]]), 1.5);
    // Октаэдр: (0, 0) с единицей 127 - ось Z, четвёртый байт как есть.
    s = encode_attributes(&[0, 0, 127, 9], 4);
    assert_eq!(decode(&s, 1, 4, Mode::Attributes, Filter::Octahedral).unwrap(), [0, 0, 127, 9]);
    // Угол (1, 0) октаэдра - ось X.
    s = encode_attributes(&[127, 0, 127, 0], 4);
    let out = decode(&s, 1, 4, Mode::Attributes, Filter::Octahedral).unwrap();
    assert_eq!([out[0] as i8, out[1] as i8, out[2] as i8], [127, 0, 0]);
    // Кватернион без поворота: наибольшая - w, номер 3.
    let tag: i16 = 0x7fff;
    let input: Vec<u8> = [0i16, 0, 0, tag].iter().flat_map(|v| v.to_le_bytes()).collect();
    s = encode_attributes(&input, 8);
    let out = decode(&s, 1, 8, Mode::Attributes, Filter::Quaternion).unwrap();
    let q: Vec<i16> = out.chunks(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    assert_eq!(q, [0, 0, 0, 32767]);
}

#[test]
fn hostile_streams_give_errors() {
    for mode in [Mode::Attributes, Mode::Triangles, Mode::Indices] {
        assert!(decode(&[], 3, 4, mode, Filter::None).is_err());
        assert!(decode(&[0x00; 64], 3, 4, mode, Filter::None).is_err());
        // Шум после верного заголовка не паникует.
        for seed in 0..200u32 {
            let head = match mode {
                Mode::Attributes => 0xa0,
                Mode::Triangles => 0xe1,
                Mode::Indices => 0xd1,
            };
            let len = 1 + (seed as usize * 7) % 90;
            let mut s = vec![head];
            s.extend((0..len).map(|i| (seed.wrapping_mul(31).wrapping_add(i as u32).wrapping_mul(2_654_435_761) >> 24) as u8));
            let _ = decode(&s, 3 + (seed as usize % 5) * 3, 4, mode, Filter::None);
        }
    }
    // Огромный count - ошибка до выделения памяти.
    assert!(decode(&[0xa0], usize::MAX / 2, 4, Mode::Attributes, Filter::None).is_err());
    // Поток версии 1 назван в ошибке.
    assert!(decode(&[0xa1; 40], 1, 4, Mode::Attributes, Filter::None).unwrap_err().0.contains("version 1"));
}

fn glb_of(doc: &serde_json::Value, blob: &[u8]) -> Vec<u8> {
    let mut text = doc.to_string().into_bytes();
    while !text.len().is_multiple_of(4) {
        text.push(b' ');
    }
    let mut blob = blob.to_vec();
    while !blob.len().is_multiple_of(4) {
        blob.push(0);
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
fn gltf_with_meshopt_views_and_quantized_uv() {
    // Квадрат: вершины float32, uv - u16 normalized (KHR_mesh_quantization)
    // со сдвигом и масштабом KHR_texture_transform, номера - TRIANGLES.
    let points: [[f32; 3]; 4] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]];
    let raw: Vec<u8> = points.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
    let uv: Vec<u8> = [[0u16, 0], [65535, 0], [0, 65535], [65535, 65535]].iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
    let tris: Vec<u32> = vec![0, 1, 2, 2, 1, 3];
    let packed = [encode_attributes(&raw, 12), encode_attributes(&uv, 4), encode_triangles(&tris)];
    let mut blob = Vec::new();
    let mut offsets = Vec::new();
    for p in &packed {
        while !blob.len().is_multiple_of(4) {
            blob.push(0);
        }
        offsets.push(blob.len());
        blob.extend(p);
    }
    let view = |i: usize, length: usize, stride: usize, count: usize, mode: &str| {
        json!({"buffer": 1, "byteLength": length,
            "extensions": {"EXT_meshopt_compression": {"buffer": 0, "byteOffset": offsets[i], "byteLength": packed[i].len(),
                "byteStride": stride, "count": count, "mode": mode}}})
    };
    let png = b"\x89PNG\r\n\x1a\n0000000000000000";
    let mut with_png = blob.clone();
    let png_at = with_png.len();
    with_png.extend(png);
    let doc = json!({
        "asset": {"version": "2.0"},
        "extensionsUsed": ["EXT_meshopt_compression", "KHR_mesh_quantization", "KHR_texture_transform"],
        "extensionsRequired": ["EXT_meshopt_compression", "KHR_mesh_quantization"],
        "buffers": [{"byteLength": with_png.len()}, {"byteLength": 1024, "extensions": {"EXT_meshopt_compression": {"fallback": true}}}],
        "bufferViews": [view(0, 48, 12, 4, "ATTRIBUTES"), view(1, 16, 4, 4, "ATTRIBUTES"), view(2, 24, 4, 6, "TRIANGLES"),
            {"buffer": 0, "byteOffset": png_at, "byteLength": png.len()}],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3"},
            {"bufferView": 1, "componentType": 5123, "normalized": true, "count": 4, "type": "VEC2"},
            {"bufferView": 2, "componentType": 5125, "count": 6, "type": "SCALAR"}
        ],
        "images": [{"bufferView": 3, "mimeType": "image/png"}],
        "textures": [{"source": 0}],
        "materials": [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0,
            "extensions": {"KHR_texture_transform": {"offset": [0.25, 0.5], "scale": [0.5, 0.25]}}}}}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "TEXCOORD_0": 1}, "indices": 2, "material": 0}]}],
        "nodes": [{"mesh": 0}],
        "scenes": [{"nodes": [0]}]
    });
    let (parts, pictures) = scene::gltf::read(&glb_of(&doc, &with_png), &|_| None).unwrap();
    assert_eq!(parts.len(), 1);
    let part = &parts[0];
    assert_eq!(part.positions.len(), 6);
    // Y вверх glTF - Z вверх: (1, 1, 0) - (1, 0, 1).
    assert!((part.positions[5] - globe_core::DVec3::new(1.0, 0.0, 1.0)).length() < 1e-9, "{:?}", part.positions);
    // uv (1, 1) после масштаба и сдвига - (0.75, 0.75).
    let uv = part.uv.as_ref().unwrap();
    assert!((uv[5] - globe_core::DVec2::new(0.75, 0.75)).length() < 1e-4, "{uv:?}");
    assert!((uv[0] - globe_core::DVec2::new(0.25, 0.5)).length() < 1e-9);
    assert_eq!(pictures.len(), 1);
}
