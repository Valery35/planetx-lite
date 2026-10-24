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
    // Окно наложения линий векторной основы: u0, v0, сторона, признак.
    over: vec4<f32>,
    // Окно огней городов: u0, v0, сторона, признак.
    lights: vec4<f32>,
    // Проход воды: цвет rgb в значениях экрана и непрозрачность.
    water: vec4<f32>,
    // Проход воды: x - признак, y - глубина полной воды на экране.
    sea: vec4<f32>,
};

@group(0) @binding(0) var<uniform> draw: Draw;
@group(1) @binding(0) var tile_texture: texture_2d<f32>;
@group(1) @binding(1) var tile_sampler: sampler;
// Наложение линий векторной основы, как картинка overlay.py модуля
// поверх снимка: значения экрана, альфа умножена на цвет.
@group(3) @binding(0) var over_texture: texture_2d<f32>;
@group(3) @binding(1) var over_sampler: sampler;
// Огни городов NASA Black Marble, видны на ночной стороне при солнце.
@group(3) @binding(2) var lights_texture: texture_2d<f32>;
@group(3) @binding(3) var lights_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // Множитель яркости отмывки рельефа.
    @location(1) shade: f32,
    // Дымка: доля света поверхности, прошедшая к глазу, и свет воздуха.
    @location(2) through: vec3<f32>,
    @location(3) glow: vec3<f32>,
    // Нормаль рельефа в ECEF для света солнца.
    @location(4) normal: vec3<f32>,
    // Высота вершины на экране, для прохода воды.
    @location(5) height: f32,
};

// Свет солнца, перенос `brightness` из `core/sun.py` модуля: ламбертов
// по нормали, рассеянный свет днём AMBIENT и ночью NIGHT, переход
// по высоте солнца от -12° до 6°. Ровная местность при солнце на высоте
// 45° даёт 1. Ночь темнее, чем у модуля (0.07): просьба автора от
// 24 октября 2026 года «ночь потемнее».
const SUN_AMBIENT: f32 = 0.35;
const SUN_NIGHT: f32 = 0.03;
const SUN_FLAT: f32 = 0.80961941;

fn sun_brightness(cos_sun: f32) -> f32 {
    let day = smoothstep(SUN_TWILIGHT, SUN_DAYLIGHT, cos_sun);
    let ambient = SUN_NIGHT + (SUN_AMBIENT - SUN_NIGHT) * day;
    return (ambient + (1.0 - SUN_AMBIENT) * max(cos_sun, 0.0)) / SUN_FLAT;
}

// Дымка считается в вершинах, а не в пикселях, как в модуле. На кадре
// страницы 3840 × 1838 счёт в пикселях давал 30 кадров в секунду
// вместо 60, 22 октября 2026 года. Дымка меняется по кадру плавно,
// между вершинами она интерполируется.
@vertex
fn tile_vs(@location(0) offset: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) shade: f32, @location(3) normal: vec3<f32>, @location(4) sea: vec4<f32>) -> VertexOut {
    var out: VertexOut;
    // В проходе воды сетка опускается на уровень моря, как у модуля.
    var p = offset;
    if draw.sea.x > 0.5 {
        p = p - sea.xyz * sea.w;
    }
    out.position = draw.mvp * vec4<f32>(p, 1.0);
    out.height = sea.w;
    out.uv = uv;
    out.shade = shade;
    out.normal = normal;
    out.through = vec3<f32>(1.0);
    out.glow = vec3<f32>(0.0);
    let w = out.position.w;
    if w > 0.0 {
        // Луч через вершину: та же запись, что у view_dir в пикселе.
        let ndc = out.position.xy / w;
        let dir = air_frame.right.xyz * ndc.x + air_frame.up.xyz * ndc.y - air_frame.back.xyz;
        let light = air(dir, w);
        out.through = light.through;
        out.glow = light.glow;
    }
    return out;
}

@fragment
fn tile_fs(in: VertexOut) -> @location(0) vec4<f32> {
    // Выборка до ветвления: производные для фильтрации нужны во всех
    // пикселях четвёрки.
    let picture = textureSample(tile_texture, tile_sampler, draw.window.xy + in.uv * draw.window.z);
    let lines = textureSample(over_texture, over_sampler, draw.over.xy + in.uv * draw.over.z) * draw.over.w;
    let glow = textureSample(lights_texture, lights_sampler, draw.lights.xy + in.uv * draw.lights.z) * draw.lights.w;
    if draw.hole.w > 0.5 {
        let low = draw.hole.xy;
        let high = draw.hole.xy + vec2<f32>(draw.hole.z, draw.hole.z);
        if any(in.uv < low) || any(in.uv > high) {
            discard;
        }
    }
    var photo = mix(to_srgb(picture.rgb), draw.color.rgb, draw.color.a);
    var base = photo * (1.0 - lines.a) + lines.rgb;
    var shade = in.shade;
    var alpha = 1.0;
    // Вода только над рельефом ниже нуля, у берега прозрачнее.
    let water = draw.sea.x > 0.5;
    if water {
        if in.height >= 0.0 {
            discard;
        }
        base = draw.water.rgb;
        alpha = draw.water.a * clamp(-in.height / draw.sea.y, 0.25, 1.0);
        shade = 1.0;
    }
    // Со светом солнца отмывка не нужна: тени даёт само солнце.
    let n = in.normal;
    if air_frame.sun.w > 0.5 && dot(n, n) > 0.0 {
        shade = sun_brightness(dot(normalize(n), air_frame.sun.xyz));
    }
    var ground = clamp(base * shade, vec3<f32>(0.0), vec3<f32>(1.0));
    // Огни городов: ночью по высоте солнца над горизонтом, к дню гаснут,
    // как у модуля. Облаков у Lite нет.
    if air_frame.sun.w > 0.5 && draw.lights.w > 0.5 && !water && dot(n, n) > 0.0 {
        let day = smoothstep(SUN_TWILIGHT, SUN_DAYLIGHT, dot(normalize(n), air_frame.sun.xyz));
        ground = min(ground + glow.rgb * (1.0 - day), vec3<f32>(1.0));
    }
    // Дымка: воздух между глазом и поверхностью. Белая подложка под
    // дымкой остаётся белой: through + (1 - through) = 1.
    return vec4<f32>(min(ground * in.through + in.glow, vec3<f32>(1.0)), alpha);
}
