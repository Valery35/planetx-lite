// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

// Облака точек. Точка - экземпляр из шести вершин, квадрат размером
// params.x пикселей кадра вокруг её места на экране. Смещение точки -
// от центра облака в f32, mv - вид из глаза на центр, сдвиг «центр -
// глаз» в нём посчитан в f64.

struct PointsDraw {
    mv: mat4x4<f32>,
    projection: mat4x4<f32>,
    // x - пикселей кадра на логический пиксель, y - сдвиг глубины
    // к глазу, zw - кадр.
    params: vec4<f32>,
    // x - 1: круг с тёмным ободком.
    style: vec4<f32>,
};

@group(0) @binding(0) var<uniform> cloud: PointsDraw;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    // Место в квадрате точки от -1 до 1.
    @location(1) local: vec2<f32>,
};

@vertex
fn points_vs(
    @builtin(vertex_index) index: u32,
    @location(0) offset: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) size: f32,
) -> Out {
    var out: Out;
    let clip = cloud.projection * (cloud.mv * vec4<f32>(offset, 1.0));
    var xs = array<f32, 6>(-1.0, 1.0, 1.0, -1.0, 1.0, -1.0);
    var ys = array<f32, 6>(-1.0, -1.0, 1.0, -1.0, 1.0, 1.0);
    let half = 0.5 * size * cloud.params.x;
    let corner = vec2<f32>(xs[index % 6u], ys[index % 6u]) * half / (0.5 * cloud.params.zw);
    out.position = vec4<f32>(clip.xy + corner * clip.w, clip.z - cloud.params.y * clip.w, clip.w);
    out.color = color;
    out.local = vec2<f32>(xs[index % 6u], ys[index % 6u]);
    return out;
}

@fragment
fn points_fs(in: Out) -> @location(0) vec4<f32> {
    if cloud.style.x > 0.5 {
        // Кружок с тёмным ободком: точка видна и на светлом снимке.
        let d = length(in.local);
        if d > 1.0 {
            discard;
        }
        let rgb = select(in.color.rgb, in.color.rgb * 0.35, d > 0.75);
        return vec4<f32>(rgb, in.color.a);
    }
    return in.color;
}
