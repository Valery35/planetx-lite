// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

// Тайлы глобуса. Перед этим текстом стоит air.wgsl.
//
// Вершина тайла - смещение от центра тайла в f32. Матрица mvp считается
// на процессоре в f64 от разности «центр тайла - глаз» и только потом
// округляется до f32. Мировых координат здесь нет.
//
// Кадр глобуса - текстура без sRGB, egui берёт из неё значения как
// есть. С текстурой sRGB цвет переводился дважды, и снимки выходили
// втрое темнее, 22 октября 2026 года.

struct Draw {
    mvp: mat4x4<f32>,
    // Заплатка: u0, v0, сторона доли, признак (1 - заплатка).
    hole: vec4<f32>,
    // Ровный цвет в значениях экрана: rgb и доля цвета вместо снимка.
    color: vec4<f32>,
    // Окно снимка: u0, v0, сторона. Тайла нет у источника - рисуется
    // его доля в снимке предка.
    window: vec4<f32>,
};

@group(0) @binding(0) var<uniform> draw: Draw;
@group(1) @binding(0) var tile_texture: texture_2d<f32>;
@group(1) @binding(1) var tile_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // Множитель яркости отмывки рельефа.
    @location(1) shade: f32,
    // Глубина вершины, метры вдоль взгляда.
    @location(2) depth: f32,
};

@vertex
fn tile_vs(@location(0) offset: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) shade: f32) -> VertexOut {
    var out: VertexOut;
    out.position = draw.mvp * vec4<f32>(offset, 1.0);
    out.uv = uv;
    out.shade = shade;
    out.depth = out.position.w;
    return out;
}

@fragment
fn tile_fs(in: VertexOut) -> @location(0) vec4<f32> {
    // Выборка до ветвления: производные для фильтрации нужны во всех
    // пикселях четвёрки.
    let picture = textureSample(tile_texture, tile_sampler, draw.window.xy + in.uv * draw.window.z);
    if draw.hole.w > 0.5 {
        let low = draw.hole.xy;
        let high = draw.hole.xy + vec2<f32>(draw.hole.z, draw.hole.z);
        if any(in.uv < low) || any(in.uv > high) {
            discard;
        }
    }
    let base = mix(to_srgb(picture.rgb), draw.color.rgb, draw.color.a);
    let ground = clamp(base * in.shade, vec3<f32>(0.0), vec3<f32>(1.0));
    // Дымка: воздух между глазом и поверхностью. Белая подложка под
    // дымкой остаётся белой: through + (1 - through) = 1.
    let light = air(view_dir(in.position.xy), in.depth);
    return vec4<f32>(min(ground * light.through + light.glow, vec3<f32>(1.0)), 1.0);
}
