// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

// Подземные сетки, перенос SUBSURFACE_VERTEX и SUBSURFACE_FRAGMENT
// модуля: цвет вершины, свет с двух сторон грани, как у отмывки.

struct SubDraw {
    mvp: mat4x4<f32>,
    // xyz - направление света в ECEF, w - доля рассеянного света.
    light: vec4<f32>,
};

@group(0) @binding(0) var<uniform> sub: SubDraw;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn sub_vs(@location(0) offset: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec4<f32>) -> Out {
    var out: Out;
    out.position = sub.mvp * vec4<f32>(offset, 1.0);
    let len = length(normal);
    var lambert = 1.0;
    if len > 0.0 {
        lambert = abs(dot(normal / len, sub.light.xyz));
    }
    let shade = sub.light.w + (1.0 - sub.light.w) * lambert;
    out.color = vec4<f32>(color.rgb * shade, color.a);
    return out;
}

@fragment
fn sub_fs(in: Out) -> @location(0) vec4<f32> {
    return in.color;
}
