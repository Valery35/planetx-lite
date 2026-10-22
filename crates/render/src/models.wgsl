// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

// 3D-модели. Вершина - смещение от точки модели в f32, mv - вид из глаза
// на точку модели, сдвиг «точка - глаз» в нём посчитан в f64. Цвет -
// картинка части, умноженная на цвет материала, со светом: рассеянный
// и от солнца сверху. Нормаль - в осях ECEF, свет - тоже.

struct ModelDraw {
    mv: mat4x4<f32>,
    projection: mat4x4<f32>,
    // Цвет материала в значениях экрана, rgba 0..1.
    rgba: vec4<f32>,
    // xyz - направление на солнце в ECEF, w - доля рассеянного света.
    light: vec4<f32>,
};

@group(0) @binding(0) var<uniform> model: ModelDraw;
@group(1) @binding(0) var model_texture: texture_2d<f32>;
@group(1) @binding(1) var model_sampler: sampler;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn model_vs(@location(0) offset: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>) -> Out {
    var out: Out;
    out.position = model.projection * (model.mv * vec4<f32>(offset, 1.0));
    out.normal = normal;
    out.uv = uv;
    return out;
}

@fragment
fn model_fs(in: Out, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let base = textureSample(model_texture, model_sampler, in.uv) * model.rgba;
    // Грань видна с обеих сторон, у обратной нормаль развёрнута.
    var n = normalize(in.normal);
    if !front {
        n = -n;
    }
    let sun = max(dot(n, model.light.xyz), 0.0);
    let shade = model.light.w + (1.0 - model.light.w) * sun;
    return vec4<f32>(base.rgb * shade, base.a);
}
