// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Эллипсоид тела и геоцентрические координаты ECEF.
//!
//! ECEF - прямоугольная система с началом в центре тела. Ось X идёт
//! к пересечению экватора с нулевым меридианом, ось Z к северному
//! полюсу. Единица - метр, все расчёты в f64.
//!
//! Широта и долгота - в градусах, высота - над эллипсоидом, в метрах.
//!
//! Перенос `planetx/core/ellipsoid.py` модуля. В модуле тело одно
//! на весь модуль и меняется `set_body`. В Lite тело - значение
//! [`Ellipsoid`], его передают тем, кто считает. Общего изменяемого
//! состояния нет, поэтому копий размеров прежнего тела не бывает.
//!
//! Марс и Луна - сферы, как у тайлов снимков OpenPlanetaryMap
//! в планетоцентрических широтах. Радиус Марса 3396.19 км и Луны
//! 1737.4 км - по рекомендациям IAU 2015. Радиусы прочих тел - сферы
//! глобальных мозаик USGS, у Юпитера - сжатый эллипсоид IAU.

use glam::DVec3;

/// Тело - ключ, большая полуось в метрах и сжатие.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Body {
    pub key: &'static str,
    pub a: f64,
    pub f: f64,
}

const fn sphere(key: &'static str, a: f64) -> Body {
    Body { key, a, f: 0.0 }
}

pub const EARTH: Body = Body { key: "earth", a: 6378137.0, f: 1.0 / 298.257223563 };
pub const MARS: Body = sphere("mars", 3396190.0);
pub const MOON: Body = sphere("moon", 1737400.0);
// Остальные тела - сферы средних радиусов, как у мозаик USGS, кроме
// сжатого Юпитера: экваториальный и полярный радиусы IAU 2015.
pub const MERCURY: Body = sphere("mercury", 2439400.0);
pub const VENUS: Body = sphere("venus", 6051800.0);
pub const JUPITER: Body = Body { key: "jupiter", a: 71492000.0, f: 1.0 - 66854000.0 / 71492000.0 };
pub const IO: Body = sphere("io", 1821490.0);
pub const EUROPA: Body = sphere("europa", 1560800.0);
pub const GANYMEDE: Body = sphere("ganymede", 2631200.0);
pub const CALLISTO: Body = sphere("callisto", 2410300.0);
pub const MIMAS: Body = sphere("mimas", 198200.0);
pub const ENCELADUS: Body = sphere("enceladus", 252000.0);
pub const TETHYS: Body = sphere("tethys", 536300.0);
pub const DIONE: Body = sphere("dione", 563000.0);
pub const RHEA: Body = sphere("rhea", 764100.0);
pub const TITAN: Body = sphere("titan", 2575000.0);
pub const IAPETUS: Body = sphere("iapetus", 736000.0);
pub const CERES: Body = sphere("ceres", 470000.0);
pub const VESTA: Body = sphere("vesta", 255000.0);

pub const BODIES: [Body; 19] = [
    EARTH, MARS, MOON, MERCURY, VENUS, JUPITER, IO, EUROPA, GANYMEDE, CALLISTO, MIMAS, ENCELADUS,
    TETHYS, DIONE, RHEA, TITAN, IAPETUS, CERES, VESTA,
];

/// Тело по ключу, неизвестный ключ - Земля.
pub fn body_by_key(key: &str) -> &'static Body {
    BODIES.iter().find(|b| b.key == key).unwrap_or(&BODIES[0])
}

/// Эллипсоид тела - полуоси и эксцентриситеты.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ellipsoid {
    pub body: Body,
    /// Большая полуось, м.
    pub a: f64,
    /// Сжатие.
    pub f: f64,
    /// Малая полуось, м.
    pub b: f64,
    /// Квадрат первого эксцентриситета.
    pub e2: f64,
    /// Квадрат второго эксцентриситета.
    pub ep2: f64,
}

impl Ellipsoid {
    pub fn new(body: Body) -> Self {
        let a = body.a;
        let f = body.f;
        let e2 = f * (2.0 - f);
        Self { body, a, f, b: a * (1.0 - f), e2, ep2: e2 / (1.0 - e2) }
    }

    /// Земля, WGS84.
    pub fn wgs84() -> Self {
        Self::new(EARTH)
    }

    /// Широта, долгота и высота в ECEF.
    pub fn geodetic_to_ecef(&self, lat: f64, lon: f64, h: f64) -> DVec3 {
        let (sin_lat, cos_lat) = lat.to_radians().sin_cos();
        let (sin_lon, cos_lon) = lon.to_radians().sin_cos();
        let n = self.a / (1.0 - self.e2 * sin_lat * sin_lat).sqrt();
        DVec3::new(
            (n + h) * cos_lat * cos_lon,
            (n + h) * cos_lat * sin_lon,
            (n * (1.0 - self.e2) + h) * sin_lat,
        )
    }

    /// ECEF в широту, долготу и высоту.
    ///
    /// Решение Хейккинена, замкнутое, без итераций. Точность - доли
    /// миллиметра от центра Земли на удалении больше 100 км до десятков
    /// тысяч километров над поверхностью. Ближе к центру формула
    /// неустойчива, глобусу эти точки не нужны.
    pub fn ecef_to_geodetic(&self, p: DVec3) -> (f64, f64, f64) {
        let (a, b, e2) = (self.a, self.b, self.e2);
        let p2 = p.x * p.x + p.y * p.y;
        let r = p2.sqrt();
        let z2 = p.z * p.z;
        let f54 = 54.0 * b * b * z2;
        let g = p2 + (1.0 - e2) * z2 - e2 * (a * a - b * b);
        let c = e2 * e2 * f54 * p2 / (g * g * g);
        let s = (1.0 + c + (c * c + 2.0 * c).sqrt()).cbrt();
        let k = s + 1.0 + 1.0 / s;
        let pp = f54 / (3.0 * k * k * g * g);
        let q = (1.0 + 2.0 * e2 * e2 * pp).sqrt();
        // У оси Земли подкоренное - разность величин порядка A², настоящее
        // значение около нуля. Округление уводило его в минус, и над
        // полюсом с 10 000-40 000 км выходил NaN, модуль, 6 октября 2026 года.
        let under = 0.5 * a * a * (1.0 + 1.0 / q)
            - pp * (1.0 - e2) * z2 / (q * (1.0 + q))
            - 0.5 * pp * p2;
        let r0 = -pp * e2 * r / (1.0 + q) + under.max(0.0).sqrt();
        let t = r - e2 * r0;
        let u = (t * t + z2).sqrt();
        let v = (t * t + (1.0 - e2) * z2).sqrt();
        let z0 = b * b * p.z / (a * v);
        let h = u * (1.0 - b * b / (a * v));
        let lat = (p.z + self.ep2 * z0).atan2(r).to_degrees();
        let lon = p.y.atan2(p.x).to_degrees();
        (lat, lon, h)
    }

    /// Единичная нормаль к эллипсоиду.
    pub fn surface_normal(&self, lat: f64, lon: f64) -> DVec3 {
        let (sin_lat, cos_lat) = lat.to_radians().sin_cos();
        let (sin_lon, cos_lon) = lon.to_radians().sin_cos();
        DVec3::new(cos_lat * cos_lon, cos_lat * sin_lon, sin_lat)
    }

    /// Расстояние вдоль луча до первой встречи с эллипсоидом.
    ///
    /// Эллипсоид раздут на высоту `h` по обеим полуосям. Для рельефа это
    /// приближение, для h = 0 - точный эллипсоид. Направление нормировать
    /// не нужно, расстояние считается в его единицах длины. Возвращает
    /// `None`, если луч проходит мимо или эллипсоид позади. Начало луча
    /// внутри эллипсоида даёт точку выхода.
    pub fn ray_intersect(&self, origin: DVec3, direction: DVec3, h: f64) -> Option<f64> {
        // Сжатие пространства превращает эллипсоид в единичную сферу.
        let scale = DVec3::new(1.0 / (self.a + h), 1.0 / (self.a + h), 1.0 / (self.b + h));
        let os = origin * scale;
        let ds = direction * scale;
        let qa = ds.dot(ds);
        let qb = 2.0 * os.dot(ds);
        let qc = os.dot(os) - 1.0;
        let disc = qb * qb - 4.0 * qa * qc;
        if qa == 0.0 || disc < 0.0 || disc.is_nan() {
            return None;
        }
        // Устойчивая запись корней без вычитания близких величин.
        let qq = -0.5 * (qb + disc.sqrt().copysign(qb));
        let first = qq / qa;
        let second = if qq != 0.0 { qc / qq } else { first };
        let (near, far) = if first <= second { (first, second) } else { (second, first) };
        [near, far].into_iter().find(|t| *t >= 0.0)
    }
}
