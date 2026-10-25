// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Солнце, Луна и планеты на небе Земли: прямое восхождение и склонение
//! J2000 на момент времени, перенос `core/ephemeris.py` модуля.
//!
//! Планеты - кеплеровы элементы орбит JPL «Approximate Positions of the
//! Planets», таблица 1, для 1800-2050 годов, E. M. Standish. Земля в них -
//! барицентр Земли и Луны, ошибка от этого для планет меньше угловой
//! секунды. Поправка на время света - одна итерация. Луна - формулы малой
//! точности Астрономического альманаха, до 0.3°, долгота переводится
//! с эклиптики даты на J2000 общей прецессией. Звёзды каталога тоже
//! в J2000 ([`crate::stars`]), поэтому планеты и звёзды стоят в одной
//! системе.

/// 1 января 2000 года, 12:00, секунды Unix.
pub const UNIX_J2000: f64 = 946728000.0;
/// Наклон эклиптики J2000, градусы.
pub const OBLIQUITY_DEG: f64 = 23.43928;
/// Время света на одну астрономическую единицу, сутки.
pub const LIGHT_DAYS_PER_AU: f64 = 0.0057755183;
/// Общая прецессия по долготе, градусов в век.
pub const PRECESSION: f64 = 1.3969713;
/// Астрономическая единица и экваториальный радиус Земли, км.
pub const AU_KM: f64 = 149597870.7;
pub const EARTH_RADIUS_KM: f64 = 6378.137;

type Elements = ([f64; 6], [f64; 6]);

/// a, e, I, L, долгота перигелия, долгота узла и их изменение за век.
/// Углы в градусах, a в астрономических единицах.
const ELEMENTS: [(&str, Elements); 8] = [
    ("mercury", ([0.38709927, 0.20563593, 7.00497902, 252.25032350, 77.45779628, 48.33076593], [0.00000037, 0.00001906, -0.00594749, 149472.67411175, 0.16047689, -0.12534081])),
    ("venus", ([0.72333566, 0.00677672, 3.39467605, 181.97909950, 131.60246718, 76.67984255], [0.00000390, -0.00004107, -0.00078890, 58517.81538729, 0.00268329, -0.27769418])),
    ("earth", ([1.00000261, 0.01671123, -0.00001531, 100.46457166, 102.93768193, 0.0], [0.00000562, -0.00004392, -0.01294668, 35999.37244981, 0.32327364, 0.0])),
    ("mars", ([1.52371034, 0.09339410, 1.84969142, -4.55343205, -23.94362959, 49.55953891], [0.00001847, 0.00007882, -0.00813131, 19140.30268499, 0.44441088, -0.29257343])),
    ("jupiter", ([5.20288700, 0.04838624, 1.30439695, 34.39644051, 14.72847983, 100.47390909], [-0.00011607, -0.00013253, -0.00183714, 3034.74612775, 0.21252668, 0.20469106])),
    ("saturn", ([9.53667594, 0.05386179, 2.48599187, 49.95424423, 92.59887831, 113.66242448], [-0.00125060, -0.00050991, 0.00193609, 1222.49362201, -0.41897216, -0.28867794])),
    ("uranus", ([19.18916464, 0.04725744, 0.77263783, 313.23810451, 170.95427630, 74.01692503], [-0.00196176, -0.00004397, -0.00242939, 428.48202785, 0.40805281, 0.04240589])),
    ("neptune", ([30.06992276, 0.00859048, 1.77004347, -55.12002969, 44.96476227, 131.78422574], [0.00026291, 0.00005105, 0.00035372, 218.45945325, -0.32241464, -0.00508664])),
];

pub const PLANETS: [&str; 7] = ["mercury", "venus", "mars", "jupiter", "saturn", "uranus", "neptune"];
pub const BODIES: [&str; 9] = ["sun", "moon", "mercury", "venus", "mars", "jupiter", "saturn", "uranus", "neptune"];

/// Юлианские века от J2000. Разница TT и UT, около минуты, не учитывается.
pub fn centuries(unix_time: f64) -> f64 {
    (unix_time - UNIX_J2000) / 86400.0 / 36525.0
}

fn ecliptic_to_equator(v: [f64; 3]) -> [f64; 3] {
    let (s, c) = OBLIQUITY_DEG.to_radians().sin_cos();
    [v[0], c * v[1] - s * v[2], s * v[1] + c * v[2]]
}

fn norm(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Положение планеты от Солнца в экваториальной системе J2000, а. е.
/// t - юлианские века от J2000. None - тело не планета таблицы.
pub fn heliocentric(name: &str, t: f64) -> Option<[f64; 3]> {
    let (base, rate) = ELEMENTS.iter().find(|(k, _)| *k == name)?.1;
    let v: Vec<f64> = base.iter().zip(rate).map(|(b, r)| b + r * t).collect();
    let (a, e, inc, mean_long, peri, node) = (v[0], v[1], v[2], v[3], v[4], v[5]);
    let anomaly = ((mean_long - peri + 180.0).rem_euclid(360.0) - 180.0).to_radians();
    let omega = (peri - node).to_radians();
    let (inc, node) = (inc.to_radians(), node.to_radians());
    let mut ecc = anomaly + e * anomaly.sin();
    for _ in 0..30 {
        let step = (ecc - e * ecc.sin() - anomaly) / (1.0 - e * ecc.cos());
        ecc -= step;
        if step.abs() < 1e-12 {
            break;
        }
    }
    let xp = a * (ecc.cos() - e);
    let yp = a * (1.0 - e * e).sqrt() * ecc.sin();
    let (so, co) = omega.sin_cos();
    let (sn, cn) = node.sin_cos();
    let (si, ci) = inc.sin_cos();
    let x = (co * cn - so * sn * ci) * xp + (-so * cn - co * sn * ci) * yp;
    let y = (co * sn + so * cn * ci) * xp + (-so * sn + co * cn * ci) * yp;
    let z = so * si * xp + co * si * yp;
    Some(ecliptic_to_equator([x, y, z]))
}

/// Вектор от Земли к телу, а. е., с поправкой на время света.
fn geocentric(name: &str, t: f64) -> Option<[f64; 3]> {
    let earth = heliocentric("earth", t)?;
    if name == "sun" {
        // Солнце стоит в начале координат, свет идёт от него к Земле.
        return Some([-earth[0], -earth[1], -earth[2]]);
    }
    let vector = sub(heliocentric(name, t)?, earth);
    let delay = norm(vector) * LIGHT_DAYS_PER_AU / 36525.0;
    Some(sub(heliocentric(name, t - delay)?, earth))
}

/// Направление на Луну от центра Земли, J2000, и расстояние в радиусах
/// Земли.
pub fn moon(t: f64) -> ([f64; 3], f64) {
    let s = |a: f64, b: f64| (a + b * t).to_radians().sin();
    let c = |a: f64, b: f64| (a + b * t).to_radians().cos();
    let lon = 218.32 + 481267.881 * t + 6.29 * s(135.0, 477198.87) - 1.27 * s(259.3, -413335.36) + 0.66 * s(235.7, 890534.22) + 0.21 * s(269.9, 954397.74)
        - 0.19 * s(357.5, 35999.05)
        - 0.11 * s(186.5, 966404.03);
    let lat = 5.13 * s(93.3, 483202.02) + 0.28 * s(228.2, 960400.89) - 0.28 * s(318.3, 6003.15) - 0.17 * s(217.6, -407332.21);
    let parallax = 0.9508 + 0.0518 * c(135.0, 477198.87) + 0.0095 * c(259.3, -413335.36) + 0.0078 * c(235.7, 890534.22) + 0.0028 * c(269.9, 954397.74);
    // Эклиптика даты - в эклиптику J2000.
    let lon = (lon - PRECESSION * t).to_radians();
    let lat = lat.to_radians();
    let v = [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()];
    (ecliptic_to_equator(v), 1.0 / parallax.to_radians().sin())
}

/// Единичный вектор на тело в экваториальной системе J2000. None -
/// тело не из [`BODIES`].
pub fn direction(name: &str, unix_time: f64) -> Option<[f64; 3]> {
    let t = centuries(unix_time);
    if name == "moon" {
        return Some(moon(t).0);
    }
    let v = geocentric(name, t)?;
    let n = norm(v);
    (n > 0.0).then(|| [v[0] / n, v[1] / n, v[2] / n])
}

/// Прямое восхождение от 0 до 360° и склонение направления, градусы.
pub fn ra_dec_of(v: [f64; 3]) -> (f64, f64) {
    (v[1].atan2(v[0]).to_degrees().rem_euclid(360.0), v[2].clamp(-1.0, 1.0).asin().to_degrees())
}

/// Прямое восхождение от 0 до 360° и склонение тела, градусы.
pub fn ra_dec(name: &str, unix_time: f64) -> Option<(f64, f64)> {
    direction(name, unix_time).map(ra_dec_of)
}

/// Расстояние от Земли, а. е. У Луны - по параллаксу.
pub fn distance(name: &str, unix_time: f64) -> Option<f64> {
    let t = centuries(unix_time);
    if name == "moon" {
        return Some(moon(t).1 * EARTH_RADIUS_KM / AU_KM);
    }
    geocentric(name, t).map(norm)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// Секунды Unix даты UTC.
    fn unix(y: i64, m: i64, d: i64, h: i64) -> f64 {
        // Дни от 1970-01-01 по гражданскому календарю.
        let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * m + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146097 + doe - 719468;
        (days * 86400 + h * 3600) as f64
    }

    /// Угол между двумя точками неба, градусы.
    fn arc(a: (f64, f64), b: (f64, f64)) -> f64 {
        let (ra1, de1, ra2, de2) = (a.0.to_radians(), a.1.to_radians(), b.0.to_radians(), b.1.to_radians());
        let c = de1.sin() * de2.sin() + de1.cos() * de2.cos() * (ra1 - ra2).cos();
        c.clamp(-1.0, 1.0).acos().to_degrees()
    }

    /// JPL Horizons, астрометрические RA и Dec ICRF, центр Земли, градусы.
    /// Запрошены модулем 1 октября 2026 года.
    /// Дата UTC (год, месяц, день, час) и RA, Dec светил.
    type Moment = ((i64, i64, i64, i64), [(&'static str, (f64, f64)); 9]);

    const HORIZONS: [Moment; 2] = [
        (
            (2026, 10, 1, 0),
            [
                ("sun", (186.85871, -2.96284)),
                ("moon", (60.42382, 25.90867)),
                ("mercury", (207.57717, -13.21449)),
                ("venus", (213.21656, -20.84700)),
                ("mars", (123.78543, 20.85435)),
                ("jupiter", (141.83407, 15.62451)),
                ("saturn", (11.35542, 1.93264)),
                ("uranus", (63.24310, 21.00366)),
                ("neptune", (2.84049, -0.31722)),
            ],
        ),
        (
            (2012, 3, 20, 6),
            [
                ("sun", (359.87330, -0.05488)),
                ("moon", (333.07481, -5.73402)),
                ("mercury", (1.16882, 4.26439)),
                ("venus", (42.43609, 18.99746)),
                ("mars", (160.57900, 12.27538)),
                ("jupiter", (38.49222, 14.15009)),
                ("saturn", (206.90977, -8.17348)),
                ("uranus", (4.02260, 0.99033)),
                ("neptune", (333.73644, -11.45436)),
            ],
        ),
    ];

    #[test]
    fn positions() {
        // Луна - формулы малой точности. У Юпитера и Сатурна JPL называет
        // ошибку таблицы 1 до 400 и 600 секунд дуги.
        for ((y, m, d, h), bodies) in HORIZONS {
            let t = unix(y, m, d, h);
            for (name, expect) in bodies {
                let got = ra_dec(name, t).unwrap();
                let limit = match name {
                    "moon" => 0.35,
                    "jupiter" => 0.12,
                    "saturn" => 0.17,
                    _ => 0.02,
                };
                assert!(arc(got, expect) < limit, "{y}-{m}-{d} {name} {got:?}");
            }
        }
    }

    #[test]
    fn distances() {
        let t = unix(2026, 10, 1, 0);
        assert!((distance("sun", t).unwrap() - 1.0).abs() < 0.02);
        assert!((distance("moon", t).unwrap() * AU_KM - 384400.0).abs() < 25000.0);
    }

    #[test]
    fn every_body_has_a_direction() {
        for name in BODIES {
            let v = direction(name, 0.0).unwrap();
            assert!((v[0] * v[0] + v[1] * v[1] + v[2] * v[2] - 1.0).abs() < 1e-9);
        }
        assert!(direction("pluto", 0.0).is_none());
    }
}
