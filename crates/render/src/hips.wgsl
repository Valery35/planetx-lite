// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

// Тайлы снимков неба HiPS, перенос `render/hips.py` модуля. Вершина -
// направление J2000 с w = 0, тайл стоит на бесконечности, глубина 1.
// Слой проявляется непрозрачностью поверх Млечного пути и звёзд.

struct HipsDraw {
    // Проекция и поворот камеры неба.
    mvp: mat4x4<f32>,
    // x - непрозрачность слоя.
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> hips: HipsDraw;
@group(1) @binding(0) var tile: texture_2d<f32>;
@group(1) @binding(1) var tile_sampler: sampler;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn hips_vs(@location(0) dir: vec3<f32>, @location(1) uv: vec2<f32>) -> Out {
    var out: Out;
    let clip = hips.mvp * vec4<f32>(dir, 0.0);
    out.position = vec4<f32>(clip.xy, clip.w, clip.w);
    out.uv = uv;
    return out;
}

@fragment
fn hips_fs(in: Out) -> @location(0) vec4<f32> {
    let c = textureSample(tile, tile_sampler, in.uv);
    return vec4<f32>(c.rgb, hips.params.x);
}
