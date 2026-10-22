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
    // x - размер точки в пикселях, zw - кадр.
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> cloud: PointsDraw;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn points_vs(
    @builtin(vertex_index) index: u32,
    @location(0) offset: vec3<f32>,
    @location(1) color: vec4<f32>,
) -> Out {
    var out: Out;
    let clip = cloud.projection * (cloud.mv * vec4<f32>(offset, 1.0));
    var xs = array<f32, 6>(-1.0, 1.0, 1.0, -1.0, 1.0, -1.0);
    var ys = array<f32, 6>(-1.0, -1.0, 1.0, -1.0, 1.0, 1.0);
    let half = 0.5 * cloud.params.x;
    let corner = vec2<f32>(xs[index % 6u], ys[index % 6u]) * half / (0.5 * cloud.params.zw);
    out.position = vec4<f32>(clip.xy + corner * clip.w, clip.z, clip.w);
    out.color = color;
    return out;
}

@fragment
fn points_fs(in: Out) -> @location(0) vec4<f32> {
    return in.color;
}
