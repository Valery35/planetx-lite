// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Солнце: направление в ECEF на момент времени и свет. Перенос
//! `core/sun.py` модуля.
//!
//! Положение солнца считается по формулам малой точности Астрономического
//! альманаха (раздел C) - ошибка склонения и прямого восхождения около
//! 0.01° в годы 1950-2050. Земля повёрнута на гринвичское среднее
//! звёздное время. Нутация и рефракция не учитываются.
//!
//! Свет рельефа при включённом солнце - ламбертов по нормали
//! к поверхности с рассеянным светом [`AMBIENT`] днём и [`NIGHT`] ночью.
//! Переход дня в ночь идёт по высоте солнца от [`twilight`] до
//! [`daylight`]. Яркость делится на яркость ровной местности при солнце
//! на высоте 45°.

/// 1 января 2000 года, 12:00 UT, в секундах Unix.
pub const UNIX_J2000: f64 = 946_728_000.0;
/// Рассеянный свет днём.
pub const AMBIENT: f64 = 0.35;
/// Рассеянный свет ночью.
pub const NIGHT: f64 = 0.07;

/// Синус высоты солнца, ниже которого ночь: -12°.
pub fn twilight() -> f64 {
    (-12.0f64).to_radians().sin()
}

/// Синус высоты солнца, выше которого день: 6°.
pub fn daylight() -> f64 {
    6.0f64.to_radians().sin()
}

/// Яркость ровной местности при солнце на высоте 45°.
pub fn flat() -> f64 {
    AMBIENT + (1.0 - AMBIENT) * 45.0f64.to_radians().sin()
}

/// Гринвичское среднее звёздное время в радианах, от 0 до 2π.
pub fn gmst(unix_time: f64) -> f64 {
    let days = (unix_time - UNIX_J2000) / 86400.0;
    (280.460_618_37 + 360.985_647_366_29 * days).rem_euclid(360.0).to_radians()
}

/// Прямое восхождение и склонение солнца в радианах.
pub fn equatorial(unix_time: f64) -> (f64, f64) {
    let n = (unix_time - UNIX_J2000) / 86400.0;
    let mean_lon = (280.460 + 0.985_647_4 * n).rem_euclid(360.0).to_radians();
    let anomaly = (357.528 + 0.985_600_3 * n).rem_euclid(360.0).to_radians();
    let ecl_lon = mean_lon + 1.915f64.to_radians() * anomaly.sin() + 0.020f64.to_radians() * (2.0 * anomaly).sin();
    let obliquity = (23.439 - 0.000_000_4 * n).to_radians();
    let ra = (obliquity.cos() * ecl_lon.sin()).atan2(ecl_lon.cos());
    let dec = (obliquity.sin() * ecl_lon.sin()).asin();
    (ra, dec)
}

/// Точка, где солнце в зените: широта и долгота в градусах, долгота
/// от -180 до 180.
pub fn subsolar(unix_time: f64) -> (f64, f64) {
    let (ra, dec) = equatorial(unix_time);
    let lon = (ra - gmst(unix_time)).to_degrees();
    (dec.to_degrees(), (lon + 180.0).rem_euclid(360.0) - 180.0)
}

/// Единичный вектор на солнце в ECEF. Солнце далеко, направление одно
/// для всей Земли.
pub fn direction(unix_time: f64) -> [f64; 3] {
    let (lat, lon) = subsolar(unix_time);
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    [la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin()]
}

/// Высота солнца над горизонтом в точке, градусы.
pub fn elevation(lat: f64, lon: f64, unix_time: f64) -> f64 {
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    let up = [la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin()];
    let sun = direction(unix_time);
    let dot: f64 = up.iter().zip(sun).map(|(a, b)| a * b).sum();
    dot.clamp(-1.0, 1.0).asin().to_degrees()
}

pub fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Множитель яркости поверхности, у которой косинус угла между
/// нормалью и направлением на солнце равен cos_sun. Ровная местность
/// при солнце на высоте 45° даёт 1. Та же формула стоит в шейдере.
pub fn brightness(cos_sun: f64) -> f64 {
    let day = smoothstep(twilight(), daylight(), cos_sun);
    let ambient = NIGHT + (AMBIENT - NIGHT) * day;
    (ambient + (1.0 - AMBIENT) * cos_sun.max(0.0)) / flat()
}
