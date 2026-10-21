// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

// Тайлы глобуса.
//
// Вершина тайла - смещение от центра тайла в f32. Матрица mvp считается
// на процессоре в f64 от разности «центр тайла - глаз» и только потом
// округляется до f32. Мировых координат здесь нет.

struct Draw {
    mvp: mat4x4<f32>,
    // Заплатка: u0, v0, сторона доли, признак (1 - заплатка).
    hole: vec4<f32>,
    // Ровный цвет: rgb и доля цвета вместо снимка.
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> draw: Draw;
@group(1) @binding(0) var tile_texture: texture_2d<f32>;
@group(1) @binding(1) var tile_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // Множитель яркости отмывки рельефа.
    @location(1) shade: f32,
};

@vertex
fn tile_vs(@location(0) offset: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) shade: f32) -> VertexOut {
    var out: VertexOut;
    out.position = draw.mvp * vec4<f32>(offset, 1.0);
    out.uv = uv;
    out.shade = shade;
    return out;
}

@fragment
fn tile_fs(in: VertexOut) -> @location(0) vec4<f32> {
    // Выборка до ветвления: производные для фильтрации нужны во всех
    // пикселях четвёрки.
    let picture = textureSample(tile_texture, tile_sampler, in.uv);
    if draw.hole.w > 0.5 {
        let low = draw.hole.xy;
        let high = draw.hole.xy + vec2<f32>(draw.hole.z, draw.hole.z);
        if any(in.uv < low) || any(in.uv > high) {
            discard;
        }
    }
    return vec4<f32>(mix(picture.rgb, draw.color.rgb, draw.color.a) * in.shade, 1.0);
}
