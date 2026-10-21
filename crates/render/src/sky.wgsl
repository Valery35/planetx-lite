// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

// Небо и гало - пиксели, где не нарисован ни один тайл. Перед этим
// текстом стоит air.wgsl. Полноэкранный треугольник на глубине 1.0,
// как у счётчика дыр. Луч кончается на эллипсоиде, если попадает в него.

// Мягкое насыщение вместо обрезки: при обрезке зелёный упирался в 1
// раньше синего, и в середине неба шла бирюзовая полоса.
const SKY_EXPOSURE: f32 = 3.5;

@vertex
fn sky_vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(index & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 1.0, 1.0);
}

@fragment
fn sky_fs(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let dir = view_dir(pixel.xy);
    let d = dir * air_frame.axes.xyz;
    let qa = dot(d, d);
    let qb = dot(air_frame.eye.xyz, d);
    let disc = qb * qb - qa * air_frame.back.w;
    var t_end = 1.0e30;
    if disc > 0.0 && qb < 0.0 {
        t_end = (-qb - sqrt(disc)) / qa;
    }
    let light = air(dir, t_end);
    return vec4<f32>(1.0 - exp(-SKY_EXPOSURE * light.glow), 1.0);
}
