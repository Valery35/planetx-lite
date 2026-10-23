// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

// Воздух на луче из глаза, перенос ATMOSPHERE модуля
// (`render/shaders.py`). Плотность падает с высотой по экспоненте,
// рассеяние по каналам как у рэлеевского на уровне моря. Свет
// рассеивается одинаково во все стороны. Солнца пока нет. Луч идёт
// в координатах в долях полуосей, эллипсоид там - единичная сфера.
// Члены |eye|² - r² считаются на процессоре в f64.
//
// Интеграл плотности берётся по двум половинам отрезка от точки,
// ближайшей к центру Земли. Шаги сгущаются к ней как u², там плотность
// наибольшая.
//
// Цвета здесь - значения экрана, как в модуле: снимок переводится
// из линейного цвета в sRGB до отмывки и дымки.

struct Air {
    // Глаз в долях полуосей, w - |eye|² - SHELL², посчитано в f64.
    eye: vec4<f32>,
    // 1 / полуоси, w - большая полуось в километрах.
    axes: vec4<f32>,
    // Оси камеры в ECEF: вправо·tan·aspect, w - ширина кадра.
    right: vec4<f32>,
    // Вверх·tan, w - высота кадра.
    up: vec4<f32>,
    // Назад, w - |eye|² - 1, посчитано в f64.
    back: vec4<f32>,
    // x - 1 атмосфера включена, 0 выключена.
    flags: vec4<f32>,
    // Направление на солнце в ECEF, w - 1, когда свет солнца включён.
    sun: vec4<f32>,
};

@group(2) @binding(0) var<uniform> air_frame: Air;

// Высота однородной атмосферы вшестеро больше настоящей, иначе гало
// из космоса уже 5 пикселей. Рассеяние уменьшено в той же доле,
// отвесный столб воздуха такой же, как у настоящей атмосферы.
const SCALE_KM: f32 = 50.0;
const BETA: vec3<f32> = vec3<f32>(0.0058, 0.0135, 0.0331) * (8.0 / SCALE_KM);
// Шагов на половину отрезка.
const STEPS: i32 = 8;

// Луч через пиксель, в метрах на единицу параметра t. Составляющая
// вдоль взгляда равна 1, поэтому t совпадает с глубиной пикселя.
fn view_dir(pixel: vec2<f32>) -> vec3<f32> {
    let nx = 2.0 * pixel.x / air_frame.right.w - 1.0;
    let ny = 1.0 - 2.0 * pixel.y / air_frame.up.w;
    return air_frame.right.xyz * nx + air_frame.up.xyz * ny - air_frame.back.xyz;
}

// Интеграл плотности от tc до b в километрах воздуха уровня моря.
fn half_depth(d: vec3<f32>, tc: f32, b: f32, km_per_t: f32) -> f32 {
    let span = b - tc;
    var sum = 0.0;
    for (var i = 0; i < STEPS; i++) {
        let u = (f32(i) + 0.5) / f32(STEPS);
        let p = air_frame.eye.xyz + (tc + span * u * u) * d;
        let h = max(length(p) - 1.0, 0.0) * air_frame.axes.w;
        sum += exp(-h / SCALE_KM) * u;
    }
    return sum * 2.0 * abs(span) / f32(STEPS) * km_per_t;
}

// Свет воздуха на отрезке луча от глаза до t_end и доля света,
// прошедшая насквозь.
struct AirLight {
    glow: vec3<f32>,
    through: vec3<f32>,
};

fn air(dir: vec3<f32>, t_end: f32) -> AirLight {
    var light = AirLight(vec3<f32>(0.0), vec3<f32>(1.0));
    if air_frame.flags.x < 0.5 {
        return light;
    }
    let d = dir * air_frame.axes.xyz;
    let qa = dot(d, d);
    let qb = dot(air_frame.eye.xyz, d);
    let disc = qb * qb - qa * air_frame.eye.w;
    if disc <= 0.0 {
        return light;
    }
    let root = sqrt(disc);
    let t0 = max((-qb - root) / qa, 0.0);
    let t1 = min((-qb + root) / qa, t_end);
    if t1 <= t0 {
        return light;
    }
    let tc = clamp(-qb / qa, t0, t1);
    let km_per_t = length(dir) / 1000.0;
    let depth = half_depth(d, tc, t0, km_per_t) + half_depth(d, tc, t1, km_per_t);
    light.through = exp(-BETA * depth);
    light.glow = 1.0 - light.through;
    return light;
}

// Линейный цвет в sRGB.
fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3<f32>(0.0031308));
}
