// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

// Свои объекты: линии, заливка многоугольников, стены до земли.
//
// Вершина - смещение от центра объекта в f32, mv - вид из глаза на центр,
// сдвиг «центр - глаз» в нём посчитан в f64. Точка подтягивается к глазу
// на долю pull расстояния, как в модуле: сетка тайла собрана по высотам
// грубее, чем высоты объекта, и без подтяжки линия тонет в склоне.
//
// В wgpu нет геометрического шейдера модуля. Отрезок линии - экземпляр
// из шести вершин, шейдер вершин раздвигает его на толщину в пикселях
// кадра. Концы продлены на полтолщины, так стыки соседних отрезков
// закрыты.

struct FeatureDraw {
    mv: mat4x4<f32>,
    projection: mat4x4<f32>,
    // Цвет в значениях экрана, rgba 0..1.
    color: vec4<f32>,
    // x - подтяжка к глазу, y - толщина линии в пикселях, zw - кадр.
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> feature: FeatureDraw;

fn to_clip(offset: vec3<f32>) -> vec4<f32> {
    var eye = feature.mv * vec4<f32>(offset, 1.0);
    let pull = 1.0 - feature.params.x;
    eye = vec4<f32>(eye.xyz * pull, eye.w);
    return feature.projection * eye;
}

@vertex
fn fill_vs(@location(0) offset: vec3<f32>) -> @builtin(position) vec4<f32> {
    return to_clip(offset);
}

@vertex
fn line_vs(
    @builtin(vertex_index) index: u32,
    @location(0) a: vec3<f32>,
    @location(1) b: vec3<f32>,
) -> @builtin(position) vec4<f32> {
    let ca = to_clip(a);
    let cb = to_clip(b);
    // Отрезок за ближней плоскостью пропускается.
    if ca.w <= 0.0 || cb.w <= 0.0 {
        return vec4<f32>(2.0, 2.0, 2.0, 1.0);
    }
    let half_size = 0.5 * feature.params.zw;
    let sa = ca.xy / ca.w * half_size;
    let sb = cb.xy / cb.w * half_size;
    var d = sb - sa;
    let len = length(d);
    d = select(vec2<f32>(1.0, 0.0), d / len, len > 1e-6);
    let width = 0.5 * feature.params.y;
    let n = vec2<f32>(-d.y, d.x) * width;
    let e = d * width;
    // Углы: конец 0 или 1, сторона -1 или 1.
    var ends = array<f32, 6>(0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    var sides = array<f32, 6>(-1.0, 1.0, -1.0, -1.0, 1.0, 1.0);
    let end = ends[index % 6u];
    let side = sides[index % 6u];
    let p = select(ca, cb, end > 0.5);
    let s = select(sa - e, sb + e, end > 0.5) + n * side;
    return vec4<f32>(s / half_size * p.w, p.z, p.w);
}

@fragment
fn feature_fs() -> @location(0) vec4<f32> {
    return feature.color;
}
