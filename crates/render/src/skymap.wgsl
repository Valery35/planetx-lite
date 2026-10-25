// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

// Картинка неба с Млечным путём, перенос SKY_FRAGMENT `render/sky.py`
// модуля. Полноэкранный треугольник на глубине 1.0: Земля его закрывает.
// Для пикселя считается направление взгляда в экваториальной системе,
// оно даёт прямое восхождение и склонение, то есть точку картинки. Свет
// картинки прибавляется к небу, альфа кадра не меняется.

struct SkyMap {
    // Оси камеры в экваториальной системе: вправо·tan·aspect, w - ширина
    // кадра, вверх·tan, w - высота кадра, назад.
    right: vec4<f32>,
    up: vec4<f32>,
    back: vec4<f32>,
    // x - яркость, y - порог чёрного: ниже него зерно слабых звёзд.
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> map: SkyMap;
@group(0) @binding(1) var image: texture_2d<f32>;
@group(0) @binding(2) var image_sampler: sampler;

const PI: f32 = 3.14159265358979;

@vertex
fn skymap_vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(index & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 1.0, 1.0);
}

@fragment
fn skymap_fs(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let nx = 2.0 * pixel.x / map.right.w - 1.0;
    let ny = 1.0 - 2.0 * pixel.y / map.up.w;
    let d = normalize(map.right.xyz * nx + map.up.xyz * ny - map.back.xyz);
    // Уровень 0 явно: на шве 0h производные координат рвутся, и выбор
    // уровня по ним дал бы полосу.
    let uv = vec2<f32>(fract(atan2(d.y, d.x) / (2.0 * PI)), acos(clamp(d.z, -1.0, 1.0)) / PI);
    let floor_level = map.params.y;
    let c = max(textureSampleLevel(image, image_sampler, uv, 0.0).rgb - vec3<f32>(floor_level), vec3<f32>(0.0)) / (1.0 - floor_level);
    return vec4<f32>(c * map.params.x, 1.0);
}
