// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! 3D-путь и 3D-многоугольник: попадание луча в здание и меры
//! в пространстве, перенос `core/measure3d.py` модуля.
//!
//! Точка 3D-линейки ставится туда, куда смотрит луч из глаза: на крышу или
//! стену здания, если луч встречает его раньше рельефа. Путь меряется
//! прямыми отрезками между точками в ECEF. Площадь многоугольника - длина
//! векторной площади по Ньюэллу, у плоского многоугольника это его
//! настоящая площадь при любом наклоне. Наклон плоскости - угол между её
//! нормалью и вертикалью в середине многоугольника.

use globe_core::DVec3;
use globe_core::ellipsoid::Ellipsoid;

/// Доля, ниже которой луч считается параллельным треугольнику.
const EPSILON: f64 = 1e-9;

/// Треугольник вершинами.
pub type Triangle = [DVec3; 3];

/// Ближнее попадание луча в треугольники: параметр t точки origin + t ·
/// direction или None. Метод Мёллера-Трумбора, стороны треугольника
/// не важны.
pub fn ray_triangles(origin: DVec3, direction: DVec3, triangles: &[Triangle]) -> Option<f64> {
    let mut best: Option<f64> = None;
    for [a, b, c] in triangles {
        let (e1, e2) = (*b - *a, *c - *a);
        let p = direction.cross(e2);
        let det = e1.dot(p);
        if det.abs() <= EPSILON * (e1.length() * e2.length() * direction.length()).max(1e-300) {
            continue;
        }
        let inv = 1.0 / det;
        let s = origin - *a;
        let u = s.dot(p) * inv;
        let q = s.cross(e1);
        let v = q.dot(direction) * inv;
        let t = q.dot(e2) * inv;
        if u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t > 0.0 && best.is_none_or(|b| t < b) {
            best = Some(t);
        }
    }
    best
}

/// Задевает ли луч шар, быстрый отбор тайлов зданий.
pub fn ray_sphere(origin: DVec3, direction: DVec3, center: DVec3, radius: f64) -> bool {
    let d = direction.normalize_or_zero();
    let m = origin - center;
    let b = m.dot(d);
    let c = m.dot(m) - radius * radius;
    c <= 0.0 || (b <= 0.0 && b * b - c >= 0.0)
}

/// Длина ломаной в ECEF прямыми отрезками, метры.
pub fn path_length(points: &[DVec3]) -> f64 {
    points.windows(2).map(|p| (p[1] - p[0]).length()).sum()
}

/// Векторная площадь замкнутого многоугольника по Ньюэллу. Направление -
/// нормаль, длина - площадь плоского многоугольника.
pub fn vector_area(points: &[DVec3]) -> DVec3 {
    if points.len() < 3 {
        return DVec3::ZERO;
    }
    let middle = points.iter().fold(DVec3::ZERO, |a, p| a + *p) / points.len() as f64;
    let mut sum = DVec3::ZERO;
    for (i, p) in points.iter().enumerate() {
        let next = points[(i + 1) % points.len()];
        sum += (*p - middle).cross(next - middle);
    }
    sum * 0.5
}

/// Периметр, площадь и наклон 3D-многоугольника. Наклон - угол плоскости
/// к горизонту в градусах, 0 - ровная крыша, 90 - стена. Для двух точек
/// и меньше площадь и наклон нулевые.
pub fn polygon(e: &Ellipsoid, points: &[DVec3]) -> (f64, f64, f64) {
    if points.len() < 2 {
        return (0.0, 0.0, 0.0);
    }
    let perimeter = if points.len() >= 3 { path_length(points) + (points[0] - points[points.len() - 1]).length() } else { 2.0 * path_length(points) };
    let vector = vector_area(points);
    let area = vector.length();
    if area == 0.0 {
        return (perimeter, 0.0, 0.0);
    }
    let middle = points.iter().fold(DVec3::ZERO, |a, p| a + *p) / points.len() as f64;
    let (lat, lon, _) = e.ecef_to_geodetic(middle);
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    let up = DVec3::new(la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin());
    let tilt = (vector.dot(up).abs() / area).min(1.0).acos().to_degrees();
    (perimeter, area, tilt)
}
