// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Длина, курс, площадь и окружность на эллипсоиде WGS84. У модуля их
//! считает QgsDistanceArea (`ui/measure.py`), а она - GeographicLib.
//! Здесь - перенос GeographicLib на Rust (крейт geographiclib-rs, MIT).
//! Точки - (широта, долгота) в градусах.

use geographiclib_rs::{DirectGeodesic, Geodesic, InverseGeodesic, PolygonArea, Winding};

use crate::measure::CIRCLE_POINTS;

/// Длина ломаной по геодезическим линиям, м, как measureLine.
pub fn length(points: &[(f64, f64)]) -> f64 {
    let g = Geodesic::wgs84();
    points
        .windows(2)
        .map(|w| {
            let ((a, b), (c, d)) = (w[0], w[1]);
            let s12: f64 = g.inverse(a, b, c, d);
            s12
        })
        .sum()
}

/// Курс от a к b с севера по часовой, градусы 0-360, как bearing.
pub fn bearing(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (_, azi1, _, _): (f64, f64, f64, f64) = Geodesic::wgs84().inverse(a.0, a.1, b.0, b.1);
    azi1.rem_euclid(360.0)
}

/// Площадь, м², и периметр, м, многоугольника, как measureArea
/// и measurePerimeter. Обход в любую сторону даёт ту же площадь.
pub fn area_perimeter(points: &[(f64, f64)]) -> (f64, f64) {
    if points.len() < 3 {
        return (0.0, 2.0 * length(points));
    }
    let g = Geodesic::wgs84();
    let mut polygon = PolygonArea::new(&g, Winding::CounterClockwise);
    for (lat, lon) in points {
        polygon.add_point(*lat, *lon);
    }
    // Площадь со знаком: обход по часовой стрелке без знака дал бы
    // площадь остальной Земли.
    let (perimeter, area, _) = polygon.compute(true);
    (area.abs(), perimeter)
}

/// Точка на расстоянии distance метров по азимуту azimuth градусов,
/// как computeSpheroidProject.
pub fn project(lat: f64, lon: f64, azimuth: f64, distance: f64) -> (f64, f64) {
    Geodesic::wgs84().direct(lat, lon, azimuth, distance)
}

/// Окружность радиуса radius метров по геодезическим линиям,
/// [`CIRCLE_POINTS`] точек, как `ellipsoid_ring` модуля. На сфере радиус
/// расходился с мерой эллипсоида на 0.24 % на широте 58°, 27 сентября
/// 2026 года.
pub fn ring(lat: f64, lon: f64, radius: f64) -> Vec<(f64, f64)> {
    (0..CIRCLE_POINTS).map(|i| project(lat, lon, 360.0 * i as f64 / CIRCLE_POINTS as f64, radius)).collect()
}
