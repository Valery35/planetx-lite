// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Перелёт по методу van Wijk и Nuij.
//!
//! Статья «Smooth and efficient zooming and panning», 2003. Путь
//! строится в плоскости (u, w). Здесь u - расстояние по дуге большого
//! круга между точками взгляда, w - ширина видимой полосы. Ширина
//! пропорциональна расстоянию камеры до точки взгляда,
//! w = 2·d·tan(fov/2).
//!
//! Метрика пути - ds² = (ρ²·du² + dw²/ρ²) / w². Оптимальный путь в ней
//! задаётся формулами статьи, длина пути S. Камера проходит путь
//! с постоянной скоростью в этой метрике, время перелёта
//! пропорционально S.
//!
//! Величины r0 и r1 статьи записаны через asinh. Исходная запись
//! ln(-b + sqrt(b² + 1)) теряет точность при большом b, то есть при
//! перелёте на большое расстояние у самой земли.
//!
//! Перенос `planetx/core/flight.py` модуля. Цель перелёта задаётся
//! позой, перелёт и вращение вокруг точки - варианты [`Motion`].

use crate::ellipsoid::Ellipsoid;
use crate::navigation::{Pose, max_altitude};

pub const RHO: f64 = std::f64::consts::SQRT_2;
/// Единиц длины пути S в секунду.
pub const SPEED: f64 = 2.0;
pub const MIN_TIME: f64 = 1.0;
pub const MAX_TIME: f64 = 8.0;
/// Метров, ближе этого точки взгляда считаются одной.
pub const PURE_ZOOM: f64 = 1.0;
/// Градусов азимута в секунду при вращении, оборот за минуту.
pub const SPIN_RATE: f64 = 6.0;
/// Наклон при вращении, если вид был отвесным.
pub const SPIN_TILT: f64 = 45.0;
/// Запас вокруг охвата при перелёте к слою.
pub const FIT_MARGIN: f64 = 1.2;
/// Метров, ближе к точечному слою не подлетаем.
pub const MIN_FIT_DISTANCE: f64 = 300.0;
pub const M_PER_DEGREE: f64 = 111320.0;

/// Путь van Wijk-Nuij от (0, w0) до (u1, w1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Path {
    pub u1: f64,
    pub w0: f64,
    pub w1: f64,
    /// Точки взгляда совпадают, меняется только ширина.
    pub zoom_only: bool,
    /// Длина пути S.
    pub length: f64,
    r0: f64,
    k: f64,
}

impl Path {
    pub fn new(u1: f64, w0: f64, w1: f64) -> Self {
        if u1 < PURE_ZOOM {
            let k = if w1 >= w0 { 1.0 } else { -1.0 };
            let length = (w1 / w0).ln().abs() / RHO;
            return Self { u1, w0, w1, zoom_only: true, length, r0: 0.0, k };
        }
        let rho2 = RHO * RHO;
        let rho4 = rho2 * rho2;
        let b0 = (w1 * w1 - w0 * w0 + rho4 * u1 * u1) / (2.0 * w0 * rho2 * u1);
        let b1 = (w1 * w1 - w0 * w0 - rho4 * u1 * u1) / (2.0 * w1 * rho2 * u1);
        // ln(-b + sqrt(b² + 1)) = -asinh(b)
        let r0 = -b0.asinh();
        let r1 = -b1.asinh();
        Self { u1, w0, w1, zoom_only: false, length: (r1 - r0) / RHO, r0, k: 0.0 }
    }

    /// Точка (u, w) на пути, s от 0 до length.
    pub fn at(&self, s: f64) -> (f64, f64) {
        if self.zoom_only {
            return (0.0, self.w0 * (self.k * RHO * s).exp());
        }
        let r0 = self.r0;
        let c = self.w0 / (RHO * RHO);
        let u = c * r0.cosh() * (RHO * s + r0).tanh() - c * r0.sinh();
        let w = self.w0 * r0.cosh() / (RHO * s + r0).cosh();
        (u, w)
    }
}

fn unit(lat: f64, lon: f64) -> [f64; 3] {
    let (sl, cl) = lat.to_radians().sin_cos();
    let (so, co) = lon.to_radians().sin_cos();
    [cl * co, cl * so, sl]
}

fn angle(p: [f64; 3], q: [f64; 3]) -> f64 {
    let dot = p[0] * q[0] + p[1] * q[1] + p[2] * q[2];
    let cross = [p[1] * q[2] - p[2] * q[1], p[2] * q[0] - p[0] * q[2], p[0] * q[1] - p[1] * q[0]];
    (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt().atan2(dot)
}

/// Точка на дуге большого круга от p к q, доля share пути.
fn slerp(p: [f64; 3], q: [f64; 3], angle: f64, share: f64) -> [f64; 3] {
    if angle < 1e-15 {
        return p;
    }
    let sin = angle.sin();
    let a = ((1.0 - share) * angle).sin() / sin;
    let b = (share * angle).sin() / sin;
    [a * p[0] + b * q[0], a * p[1] + b * q[1], a * p[2] + b * q[2]]
}

/// Плавный вход и выход, производная на концах нулевая.
fn smooth(share: f64) -> f64 {
    share * share * (3.0 - 2.0 * share)
}

/// Перелёт от позы start к позе end.
///
/// Азимут и наклон плавно приходят к азимуту и наклону end. Точка
/// взгляда идёт по дуге большого круга. Широта и долгота берутся как
/// углы на сфере, поэтому концы пути совпадают с позами точно. Отличие
/// дуги такой сферы от геодезической линии эллипсоида на плавность
/// не влияет.
#[derive(Debug, Clone)]
pub struct Flight {
    pub start: Pose,
    /// Конечная поза без рельефа и крена.
    pub end: Pose,
    pub path: Path,
    /// Длительность, секунды.
    pub duration: f64,
    p0: [f64; 3],
    p1: [f64; 3],
    angle: f64,
    scale: f64,
    /// Поворот по кратчайшему пути, не больше 180°.
    turn: f64,
    max_distance: f64,
}

impl Flight {
    /// Перелёт над телом e. От end берутся точка взгляда, расстояние,
    /// азимут и наклон.
    pub fn new(e: &Ellipsoid, start: &Pose, end: Pose, fov_y: f64) -> Self {
        let end = Pose::new(end.lat, end.lon, end.distance, end.heading, end.tilt);
        let p0 = unit(start.lat, start.lon);
        let p1 = unit(end.lat, end.lon);
        let angle = angle(p0, p1);
        let scale = 2.0 * (fov_y.to_radians() / 2.0).tan();
        let path = Path::new(e.a * angle, start.distance * scale, end.distance * scale);
        // Путь с NaN длится MIN_TIME, как в модуле, а не NaN секунд.
        let seconds = path.length / SPEED;
        let duration = if seconds.is_nan() { MIN_TIME } else { seconds.clamp(MIN_TIME, MAX_TIME) };
        let turn = (end.heading - start.heading + 180.0).rem_euclid(360.0) - 180.0;
        Self {
            start: start.clone(),
            end,
            path,
            duration,
            p0,
            p1,
            angle,
            scale,
            turn,
            max_distance: max_altitude(e),
        }
    }

    /// Длина пройденного пути ко времени t, скорость постоянна.
    pub fn s_at(&self, t: f64) -> f64 {
        (t / self.duration).clamp(0.0, 1.0) * self.path.length
    }

    pub fn pose_at(&self, t: f64) -> Pose {
        if t >= self.duration {
            return self.end.clone();
        }
        let (u, w) = self.path.at(self.s_at(t));
        let share = if self.path.zoom_only { 0.0 } else { u / self.path.u1 };
        let [x, y, z] = slerp(self.p0, self.p1, self.angle, share);
        let lat = z.atan2(x.hypot(y)).to_degrees();
        let lon = y.atan2(x).to_degrees();
        let blend = smooth(t / self.duration);
        Pose::new(
            lat,
            lon,
            (w / self.scale).min(self.max_distance),
            (self.start.heading + self.turn * blend).rem_euclid(360.0),
            self.start.tilt + (self.end.tilt - self.start.tilt) * blend,
        )
    }
}

/// Вращение вокруг точки: перелёт к ней с прежним расстоянием, потом
/// азимут растёт на [`SPIN_RATE`] в секунду. Конца нет, вращение гасит
/// любое движение мыши или клавиш, как перелёт. Пункт «Вращаться
/// вокруг», как в Google Earth.
#[derive(Debug, Clone)]
pub struct Spin {
    pub flight: Flight,
}

impl Spin {
    pub fn new(e: &Ellipsoid, start: &Pose, lat: f64, lon: f64, fov_y: f64) -> Self {
        let tilt = if start.tilt > 5.0 { start.tilt } else { SPIN_TILT };
        let end = Pose::new(lat, lon, start.distance, start.heading, tilt);
        Self { flight: Flight::new(e, start, end, fov_y) }
    }

    pub fn pose_at(&self, t: f64) -> Pose {
        if t < self.flight.duration {
            return self.flight.pose_at(t);
        }
        let mut pose = self.flight.end.clone();
        pose.heading = (pose.heading + SPIN_RATE * (t - self.flight.duration)).rem_euclid(360.0);
        pose
    }
}

/// Движение камеры по времени - перелёт или вращение.
#[derive(Debug, Clone)]
pub enum Motion {
    Flight(Flight),
    Spin(Spin),
}

impl Motion {
    /// Длительность, у вращения - бесконечность.
    pub fn duration(&self) -> f64 {
        match self {
            Motion::Flight(f) => f.duration,
            Motion::Spin(_) => f64::INFINITY,
        }
    }

    pub fn pose_at(&self, t: f64) -> Pose {
        match self {
            Motion::Flight(f) => f.pose_at(t),
            Motion::Spin(s) => s.pose_at(t),
        }
    }
}

/// Наименьшая дуга долгот, на которой лежат все lons: (запад, восток),
/// восток не меньше запада и может быть больше 180°.
///
/// Охват min..max ошибался у объектов через линию перемены дат: путь
/// -174°, -178°, +179° давал охват -178..179 и середину на другой
/// стороне Земли, замечание автора от 18 октября 2026 года. Дуга - круг
/// без самого широкого промежутка между соседними долготами.
pub fn lon_span(lons: &[f64]) -> (f64, f64) {
    let mut ring: Vec<f64> = lons.iter().map(|v| wrap_lon(*v)).collect();
    ring.sort_by(f64::total_cmp);
    let (Some(first), Some(last)) = (ring.first().copied(), ring.last().copied()) else {
        return (0.0, 0.0);
    };
    let (mut gap, mut after) = (first + 360.0 - last, 0);
    for (n, pair) in ring.windows(2).enumerate() {
        let step = pair[1] - pair[0];
        if step > gap {
            (gap, after) = (step, n + 1);
        }
    }
    let west = ring[after];
    let east = if after > 0 { ring[after - 1] + 360.0 } else { last };
    (west, east)
}

/// Долгота в промежутке -180..180.
pub fn wrap_lon(lon: f64) -> f64 {
    (lon + 180.0).rem_euclid(360.0) - 180.0
}

/// Точка взгляда и расстояние, с которых охват виден целиком.
///
/// Охват в градусах WGS84. Камера смотрит отвесно в середину охвата.
/// Ширина считается по средней широте. Расстояние - такое, чтобы охват
/// с запасом [`FIT_MARGIN`] поместился и по высоте, и по ширине кадра,
/// но не меньше [`MIN_FIT_DISTANCE`]. Восток меньше запада - охват через
/// линию перемены дат, как у Region KML.
pub fn fit_view(west: f64, south: f64, east: f64, north: f64, fov_y: f64, aspect: f64) -> (f64, f64, f64) {
    let east = if east < west { east + 360.0 } else { east };
    let lat = 0.5 * (south + north);
    let lon = wrap_lon(0.5 * (west + east));
    let height = (north - south) * M_PER_DEGREE;
    let width = (east - west) * M_PER_DEGREE * lat.to_radians().cos();
    let half = (fov_y.to_radians() / 2.0).tan();
    let distance = height.max(width / aspect) * FIT_MARGIN / (2.0 * half);
    (lat, lon, distance.max(MIN_FIT_DISTANCE))
}

/// Широта и долгота из строки или None.
///
/// Понимает «58.0105, 56.2294», «58.0105 56.2294» и запись с запятой
/// в дробной части через точку с запятой: «58,0105; 56,2294».
pub fn parse_latlon(text: &str) -> Option<(f64, f64)> {
    let text = text.trim();
    let parts: Vec<String> = if text.contains(';') {
        text.split(';').map(|p| p.trim().replace(',', ".")).collect()
    } else if text.contains(',') {
        text.split(',').map(|p| p.trim().to_string()).collect()
    } else {
        text.split_whitespace().map(str::to_string).collect()
    };
    let [lat, lon] = parts.as_slice() else {
        return None;
    };
    let lat: f64 = lat.parse().ok()?;
    let lon: f64 = lon.parse().ok()?;
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then_some((lat, lon))
}
