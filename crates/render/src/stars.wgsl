// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

// Звёзды, перенос STAR_VERTEX и STAR_FRAGMENT `render/stars.py` модуля.
// Звезда - направление, а не точка: w = 0, после проекции глубина 1,
// звезда стоит на бесконечности и закрыта Землёй. Звезда - экземпляр
// из шести вершин, квадрат её размера на экране, в нём круг с мягким
// краем. Свет звезды прибавляется к небу.

struct StarsDraw {
    // Проекция, поворот камеры и поворот неба на звёздное время.
    mvp: mat4x4<f32>,
    // x - пикселей кадра на логический пиксель, y - видимость звёзд,
    // zw - кадр.
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> sky: StarsDraw;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) local: vec2<f32>,
};

@vertex
fn stars_vs(@builtin(vertex_index) index: u32, @location(0) dir: vec3<f32>, @location(1) size: f32, @location(2) color: vec4<f32>) -> Out {
    var out: Out;
    let clip = sky.mvp * vec4<f32>(dir, 0.0);
    var xs = array<f32, 6>(-1.0, 1.0, 1.0, -1.0, 1.0, -1.0);
    var ys = array<f32, 6>(-1.0, -1.0, 1.0, -1.0, 1.0, 1.0);
    let half = 0.5 * size * sky.params.x;
    let corner = vec2<f32>(xs[index % 6u], ys[index % 6u]) * half / (0.5 * sky.params.zw);
    out.position = vec4<f32>(clip.xy + corner * clip.w, clip.w, clip.w);
    out.color = vec4<f32>(color.rgb, color.a * sky.params.y);
    out.local = vec2<f32>(xs[index % 6u], ys[index % 6u]);
    return out;
}

@fragment
fn stars_fs(in: Out) -> @location(0) vec4<f32> {
    let d = length(in.local);
    return vec4<f32>(in.color.rgb, in.color.a * clamp(1.0 - d * d, 0.0, 1.0));
}
