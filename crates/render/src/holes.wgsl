// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

// Счётчик дыр. Полноэкранный треугольник на глубине 1.0. Проверка
// глубины пропускает его только там, где тайлов нет. Пиксели, луч через
// которые проходит мимо эллипсоида, сжатого на HOLE_MARGIN,
// отбрасываются. Прошедшие пиксели считает запрос перекрытия.

struct Rays {
    // Глаз в сжатом пространстве, где эллипсоид - единичная сфера.
    eye: vec4<f32>,
    // Оси камеры в том же пространстве: вправо·tx, вверх·ty, назад.
    right: vec4<f32>,
    up: vec4<f32>,
    back: vec4<f32>,
    // Ширина и высота кадра в пикселях, qc = |eye|² - 1, посчитанное в f64.
    frame: vec4<f32>,
};

@group(0) @binding(0) var<uniform> rays: Rays;

@vertex
fn hole_vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(index & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 1.0, 1.0);
}

@fragment
fn hole_fs(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let nx = 2.0 * pixel.x / rays.frame.x - 1.0;
    let ny = 1.0 - 2.0 * pixel.y / rays.frame.y;
    let d = rays.right.xyz * nx + rays.up.xyz * ny - rays.back.xyz;
    let qa = dot(d, d);
    let qb = 2.0 * dot(d, rays.eye.xyz);
    let disc = qb * qb - 4.0 * qa * rays.frame.z;
    // Снаружи эллипсоида оба корня одного знака, перед камерой они
    // положительны, когда qb < 0.
    if disc < 0.0 || qb >= 0.0 {
        discard;
    }
    return vec4<f32>(1.0, 0.0, 1.0, 1.0);
}
