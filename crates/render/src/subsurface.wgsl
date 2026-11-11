// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

// Подземные сетки, перенос SUBSURFACE_VERTEX и SUBSURFACE_FRAGMENT
// модуля: цвет вершины, свет с двух сторон грани, как у отмывки. Сетки
// под солнцем (здания) - свет солнца с лицевой стороны и тени карты,
// как BUILDING_VERTEX и BUILDING_FRAGMENT модуля.

struct SubDraw {
    mvp: mat4x4<f32>,
    // xyz - направление света в ECEF, w - доля рассеянного света.
    // Отрицательная w - свет солнца, доля рассеянного света -w.
    light: vec4<f32>,
    // Матрица света: вершина в координаты отсечения карты теней, нули -
    // теней нет.
    shadow_mvp: mat4x4<f32>,
    // Тексель карты, запас глубины, доля радиуса начала угасания, сдвиг
    // по нормали в метрах.
    shadow: vec4<f32>,
};

@group(0) @binding(0) var<uniform> sub: SubDraw;
@group(1) @binding(0) var shadow_map: texture_depth_2d;
@group(1) @binding(1) var shadow_sampler: sampler_comparison;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    // Доля прямого света вершины 0-1 и доля рассеянного.
    @location(1) direct: vec2<f32>,
    @location(2) light: vec4<f32>,
};

@vertex
fn sub_vs(@location(0) offset: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec4<f32>) -> Out {
    var out: Out;
    out.position = sub.mvp * vec4<f32>(offset, 1.0);
    let len = length(normal);
    let sunlit = sub.light.w < 0.0;
    let ambient = abs(sub.light.w);
    var lambert = 1.0;
    var n = vec3<f32>(0.0);
    if len > 0.0 {
        n = normal / len;
        if sunlit {
            lambert = max(dot(n, sub.light.xyz), 0.0);
        } else {
            lambert = abs(dot(n, sub.light.xyz));
        }
    }
    out.color = color;
    out.direct = vec2<f32>(lambert, ambient);
    out.light = sub.shadow_mvp * vec4<f32>(offset + n * sub.shadow.w, 1.0);
    return out;
}

// Доля прямого света точки, как `shadow_lit` моделей.
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
    let z = c.z * 0.5 + 0.5 - sub.shadow.y;
    var sum = 0.0;
    for (var i = -1; i <= 1; i += 1) {
        for (var j = -1; j <= 1; j += 1) {
            sum += textureSampleCompareLevel(shadow_map, shadow_sampler, t + vec2<f32>(f32(i), f32(j)) * sub.shadow.x, z);
        }
    }
    return mix(sum / 9.0, 1.0, smoothstep(sub.shadow.z, 1.0, r));
}

// Проход карты теней: только глубина.
@vertex
fn sub_depth_vs(@location(0) offset: vec3<f32>) -> @builtin(position) vec4<f32> {
    let c = sub.shadow_mvp * vec4<f32>(offset, 1.0);
    return vec4<f32>(c.x, c.y, c.z * 0.5 + 0.5, 1.0);
}

@fragment
fn sub_fs(in: Out) -> @location(0) vec4<f32> {
    // Выборка карты - в однородном потоке, до ветвлений.
    let lit = shadow_lit(in.light);
    let ambient = in.direct.y;
    let shade = ambient + (1.0 - ambient) * in.direct.x * lit;
    return vec4<f32>(in.color.rgb * shade, in.color.a);
}
