// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

// Линии фигур созвездий вида неба, перенос `render/constellations.py`
// модуля. Вершина - направление J2000, w = 0: линия стоит на бесконечности,
// глубина 1, как у звёзд. Свет линии прибавляется к небу.

struct LinesDraw {
    // Проекция и поворот камеры неба.
    mvp: mat4x4<f32>,
    // Цвет линий с непрозрачностью.
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> lines: LinesDraw;

@vertex
fn lines_vs(@location(0) dir: vec3<f32>) -> @builtin(position) vec4<f32> {
    let clip = lines.mvp * vec4<f32>(dir, 0.0);
    return vec4<f32>(clip.xy, clip.w, clip.w);
}

@fragment
fn lines_fs() -> @location(0) vec4<f32> {
    return lines.color;
}
