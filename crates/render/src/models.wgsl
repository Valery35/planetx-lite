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
    // Матрица света: точка модели в координаты отсечения карты теней,
    // нули - теней нет.
    shadow_mvp: mat4x4<f32>,
    // Тексель карты, запас глубины, доля радиуса начала угасания, сдвиг
    // по нормали в метрах.
    shadow: vec4<f32>,
};

@group(0) @binding(0) var<uniform> model: ModelDraw;
@group(1) @binding(0) var model_texture: texture_2d<f32>;
@group(1) @binding(1) var model_sampler: sampler;
@group(2) @binding(0) var shadow_map: texture_depth_2d;
@group(2) @binding(1) var shadow_sampler: sampler_comparison;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) light: vec4<f32>,
};

@vertex
fn model_vs(@location(0) offset: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>) -> Out {
    var out: Out;
    out.position = model.projection * (model.mv * vec4<f32>(offset, 1.0));
    out.normal = normal;
    out.uv = uv;
    out.light = model.shadow_mvp * vec4<f32>(offset + normal * model.shadow.w, 1.0);
    return out;
}

// Доля прямого света точки, как `shadow_lit` тайла.
fn shadow_lit(light: vec4<f32>) -> f32 {
    if light.w < 0.5 {
        return 1.0;
    }
    let c = light.xyz;
    let r = length(c.xy);
    if r >= 1.0 || c.z <= -1.0 || c.z >= 1.0 {
        return 1.0;
    }
    let t = vec2<f32>(c.x * 0.5 + 0.5, 0.5 - c.y * 0.5);
    let z = c.z * 0.5 + 0.5 - model.shadow.y;
    var sum = 0.0;
    for (var i = -1; i <= 1; i += 1) {
        for (var j = -1; j <= 1; j += 1) {
            sum += textureSampleCompareLevel(shadow_map, shadow_sampler, t + vec2<f32>(f32(i), f32(j)) * model.shadow.x, z);
        }
    }
    return mix(sum / 9.0, 1.0, smoothstep(model.shadow.z, 1.0, r));
}

// Проход карты теней: только глубина.
@vertex
fn model_depth_vs(@location(0) offset: vec3<f32>) -> @builtin(position) vec4<f32> {
    let c = model.shadow_mvp * vec4<f32>(offset, 1.0);
    return vec4<f32>(c.x, c.y, c.z * 0.5 + 0.5, 1.0);
}

@fragment
fn model_fs(in: Out, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let base = textureSample(model_texture, model_sampler, in.uv) * model.rgba;
    // Грань видна с обеих сторон, у обратной нормаль развёрнута.
    var n = normalize(in.normal);
    if !front {
        n = -n;
    }
    let sun = max(dot(n, model.light.xyz), 0.0) * shadow_lit(in.light);
    let shade = model.light.w + (1.0 - model.light.w) * sun;
    return vec4<f32>(base.rgb * shade, base.a);
}
