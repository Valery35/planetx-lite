// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Измерения на сфере, перенос части `core/measure.py` модуля: только
//! точка по азимуту и расстоянию, она нужна наложениям. Профиль высот
//! и линейка - позже.

use globe_core::ellipsoid::EARTH;

/// Точка на расстоянии distance метров по азимуту bearing, сфера
/// радиуса большой полуоси.
///
/// Для проверок и грубых построений. Точная окружность круга строится
/// на эллипсоиде окном глобуса.
pub fn destination(lat: f64, lon: f64, bearing: f64, distance: f64) -> (f64, f64) {
    let phi = lat.to_radians();
    let lam = lon.to_radians();
    let theta = bearing.to_radians();
    let delta = distance / EARTH.a;
    let sin_phi = phi.sin() * delta.cos() + phi.cos() * delta.sin() * theta.cos();
    let phi2 = sin_phi.clamp(-1.0, 1.0).asin();
    let lam2 = lam + (theta.sin() * delta.sin() * phi.cos()).atan2(delta.cos() - phi.sin() * sin_phi);
    let lon2 = (lam2.to_degrees() + 180.0).rem_euclid(360.0) - 180.0;
    (phi2.to_degrees(), lon2)
}
