// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

// Картинки-наложения: на поверхности и фото с камерой.
//
// Вершина - смещение от центра картинки в f32, mv - вид из глаза
// на центр, сдвиг «центр - глаз» в нём посчитан в f64. Картинка
// на поверхности подтягивается к глазу на долю расстояния, как свои
// объекты: сетка картинки грубее сетки тайла. Картинка хранится
// в значениях экрана, как её прислал сервер, цвет KML её окрашивает.

struct PictureDraw {
    mv: mat4x4<f32>,
    projection: mat4x4<f32>,
    // Цвет KML в значениях экрана, rgba 0..1.
    tint: vec4<f32>,
    // x - подтяжка к глазу.
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> picture: PictureDraw;
@group(1) @binding(0) var picture_texture: texture_2d<f32>;
@group(1) @binding(1) var picture_sampler: sampler;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn picture_vs(@location(0) offset: vec3<f32>, @location(1) uv: vec2<f32>) -> Out {
    var eye = picture.mv * vec4<f32>(offset, 1.0);
    eye = vec4<f32>(eye.xyz * (1.0 - picture.params.x), eye.w);
    var out: Out;
    out.position = picture.projection * eye;
    out.uv = uv;
    return out;
}

@fragment
fn picture_fs(in: Out) -> @location(0) vec4<f32> {
    return textureSample(picture_texture, picture_sampler, in.uv) * picture.tint;
}
