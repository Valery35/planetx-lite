// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Навигация: захват Земли, колесо, наклон и поворот, инерция.
//!
//! Положение камеры задаётся позой, как в Google Earth. Поза - это точка
//! взгляда на эллипсоиде (широта, долгота), расстояние до неё, азимут
//! и наклон. Камера строится из позы. Крен нулевой по построению: правая
//! ось камеры перпендикулярна нормали эллипсоида в точке взгляда.
//!
//! - Захват подбирает широту и долготу точки взгляда методом Ньютона,
//!   пока схваченная точка не встанет под курсор. Азимут, наклон
//!   и расстояние не меняются.
//! - Колесо меняет расстояние, потом точка под курсором ставится
//!   на место тем же подбором.
//! - Наклон и поворот меняют только свои величины. Точка взгляда
//!   остаётся в центре окна.
//! - Инерция продолжает захват с затухающей скоростью по широте
//!   и долготе. Поза считается от позы в момент отпускания как функция
//!   времени, поэтому путь не зависит от частоты кадров.
//!
//! Время передаётся явно, в секундах.
//!
//! Перенос `planetx/core/navigation.py` модуля. Подбор позы считает
//! на копии камеры, камера навигатора меняется только новой позой.

use std::fmt;
use std::sync::Arc;

use glam::{DMat2, DMat3, DVec2, DVec3};

use crate::camera::{Camera, MAX_TERRAIN, enu, orientation};
use crate::ellipsoid::{EARTH, Ellipsoid};
use crate::flight::Motion;

pub const MIN_ALTITUDE: f64 = 50.0;
/// Предел высоты у Земли и тел меньше неё.
pub const MAX_ALTITUDE: f64 = 5.0e7;
/// У тела крупнее Земли предел растёт с радиусом: Юпитер целиком в кадре.
pub const MAX_ALTITUDE_RADII: f64 = MAX_ALTITUDE / EARTH.a;
pub const MAX_TILT: f64 = 85.0;
pub const MAX_LAT: f64 = 89.99;
/// Постоянная затухания инерции, секунды.
pub const INERTIA_TAU: f64 = 0.35;
/// По скольким последним секундам мерится скорость.
pub const VELOCITY_WINDOW: f64 = 0.08;
/// Пауза перед отпусканием, которая гасит инерцию.
pub const STILL_BEFORE_RELEASE: f64 = 0.06;
/// Остаток пути, после которого инерция стоит.
pub const INERTIA_STOP: f64 = 0.002;
pub const ZOOM_TAU: f64 = 0.1;
/// Глаз при приближении в f раз уходит не дальше ZOOM_REACH·|1 - f|
/// расстояния до точки под курсором, но не меньше ZOOM_SLACK его: точка
/// взгляда садится на рельеф, и глаз сдвигается сверх доли.
pub const ZOOM_REACH: f64 = 2.0;
pub const ZOOM_SLACK: f64 = 0.5;
/// Точность подбора позы, пикселей.
pub const PIN_TOLERANCE: f64 = 1e-3;
pub const PIN_FAIL: f64 = 0.5;

/// Радиусы колец опроса рельефа в долях зазора.
pub const NEAREST_SHARES: [f64; 3] = [0.25, 0.5, 0.75];
pub const NEAREST_DIRECTIONS: u32 = 8;
pub const M_PER_DEGREE: f64 = 111320.0;
/// Выше этого зазора кольца не опрашиваются.
pub const FAR_CLEARANCE: f64 = 4.0 * MAX_TERRAIN;

pub const TERRAIN_STEPS: u32 = 8;
/// Шаг вдоль луча к рельефу - доля высоты над ним. Половина не
/// перешагивает склон круче 60°.
pub const MARCH_SHARE: f64 = 0.5;
/// Шаг не короче этой доли пройденного пути: у самой земли шаги не
/// мельчают без конца.
pub const MARCH_MIN: f64 = 1e-3;
/// Наибольшее количество шагов вдоль луча.
pub const MARCH_STEPS: u32 = 400;
/// Точность точки рельефа под пикселем, метры.
pub const TERRAIN_TOLERANCE: f64 = 0.1;

/// Приближений точки взгляда при повороте взгляда.
pub const LOOK_STEPS: u32 = 8;
/// Доля расстояния, при которой подбор точки взгляда закончен.
pub const LOOK_TOLERANCE: f64 = 1e-6;

/// Высота рельефа по широте и долготе.
pub type Terrain = Arc<dyn Fn(f64, f64) -> f64 + Send + Sync>;

/// Точка взгляда, расстояние, азимут и наклон в градусах.
///
/// h - высота точки взгляда над эллипсоидом. С рельефом точка взгляда
/// при сдвиге садится на рельеф. roll - крен в градусах, поворот кадра
/// вокруг оси взгляда против часовой стрелки. Его даёт только сдвиг вида
/// ([`shift`]), прочие действия его сохраняют.
#[derive(Clone)]
pub struct Pose {
    pub lat: f64,
    pub lon: f64,
    pub distance: f64,
    pub heading: f64,
    pub tilt: f64,
    pub h: f64,
    pub terrain: Option<Terrain>,
    pub roll: f64,
}

impl fmt::Debug for Pose {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pose")
            .field("lat", &self.lat)
            .field("lon", &self.lon)
            .field("distance", &self.distance)
            .field("heading", &self.heading)
            .field("tilt", &self.tilt)
            .field("h", &self.h)
            .field("terrain", &self.terrain.is_some())
            .field("roll", &self.roll)
            .finish()
    }
}

impl Pose {
    /// Поза без рельефа и крена, точка взгляда на эллипсоиде.
    pub fn new(lat: f64, lon: f64, distance: f64, heading: f64, tilt: f64) -> Self {
        Self { lat, lon, distance, heading, tilt, h: 0.0, terrain: None, roll: 0.0 }
    }

    pub fn with_h(mut self, h: f64) -> Self {
        self.h = h;
        self
    }

    pub fn with_terrain(mut self, terrain: Option<Terrain>) -> Self {
        self.terrain = terrain;
        self
    }

    /// Глаз в ECEF и оси камеры.
    pub fn eye_rotation(&self, e: &Ellipsoid) -> (DVec3, DMat3) {
        let mut rot = orientation(self.lat, self.lon, self.heading, self.tilt);
        if self.roll != 0.0 {
            rot *= DMat3::from_rotation_z(self.roll.to_radians());
        }
        (e.geodetic_to_ecef(self.lat, self.lon, self.h) + rot.z_axis * self.distance, rot)
    }

    pub fn apply(&self, camera: &mut Camera) {
        (camera.eye, camera.rotation) = self.eye_rotation(&camera.ellipsoid);
    }

    /// Та же поза с точкой взгляда в (lat, lon). Широта прижимается
    /// к [`MAX_LAT`], долгота приводится к -180..180, с рельефом точка
    /// садится на него.
    pub fn moved_to(&self, lat: f64, lon: f64) -> Pose {
        let mut out = self.clone();
        out.lat = lat.clamp(-MAX_LAT, MAX_LAT);
        out.lon = (lon + 180.0).rem_euclid(360.0) - 180.0;
        if let Some(t) = &out.terrain {
            out.h = t(out.lat, out.lon);
        }
        out
    }
}

/// Крен, синус угла правой оси с нормалью в точке взгляда.
pub fn roll(camera: &Camera, pose: &Pose) -> f64 {
    camera.rotation.x_axis.dot(camera.ellipsoid.surface_normal(pose.lat, pose.lon))
}

pub fn altitude(e: &Ellipsoid, eye: DVec3) -> f64 {
    e.ecef_to_geodetic(eye).2
}

/// Высота глаза над рельефом под ним, без рельефа - над эллипсоидом.
pub fn clearance(e: &Ellipsoid, eye: DVec3, terrain: Option<&Terrain>) -> f64 {
    let (lat, lon, h) = e.ecef_to_geodetic(eye);
    h - terrain.map_or(0.0, |t| t(lat, lon))
}

/// Оценка расстояния от глаза до ближайшей точки рельефа.
///
/// Под глазом рельеф на расстоянии зазора c. Точка рельефа дальше c
/// по горизонтали дальше c и по прямой, поэтому ближе может быть только
/// рельеф внутри круга радиуса c. Он опрашивается на трёх кольцах
/// по [`NEAREST_DIRECTIONS`] точек. Между точками опроса склон может
/// подойти ближе, запас на это даёт доля ближней плоскости
/// в `camera::clip_range`.
pub fn nearest_terrain(e: &Ellipsoid, eye: DVec3, terrain: Option<&Terrain>) -> f64 {
    let (lat, lon, h) = e.ecef_to_geodetic(eye);
    let Some(terrain) = terrain else {
        return h;
    };
    let c = h - terrain(lat, lon);
    if !c.is_finite() {
        // NaN в высоте или рельефе ушёл бы в радиусы колец опроса,
        // в модуле 17 октября 2026 года это роняло кадр.
        return if h.is_finite() { h } else { 0.0 };
    }
    if c <= 0.0 {
        return 0.0;
    }
    if c > FAR_CLEARANCE {
        // Рельеф не поднимается над дном больше MAX_TERRAIN. Опрос
        // из космоса ничего не меняет.
        return c - MAX_TERRAIN;
    }
    let mut best = c;
    let per_lon = M_PER_DEGREE * lat.to_radians().cos().max(1e-6);
    for share in NEAREST_SHARES {
        let r = share * c;
        for i in 0..NEAREST_DIRECTIONS {
            let a = std::f64::consts::TAU * f64::from(i) / f64::from(NEAREST_DIRECTIONS);
            let up = h - terrain(lat + r * a.cos() / M_PER_DEGREE, lon + r * a.sin() / per_lon);
            if up > 0.0 {
                best = best.min((r * r + up * up).sqrt());
            } else {
                // Склон выше глаза. По прямой между опросами он проходит
                // высоту глаза на такой доле пути.
                best = best.min(r * c / (c - up));
            }
        }
    }
    best
}

/// Фокусное расстояние камеры в пикселях.
pub fn focal(camera: &Camera) -> f64 {
    f64::from(camera.height) / (2.0 * (camera.fov_y.to_radians() / 2.0).tan())
}

/// Пиксель точки при положении eye, rot. None, если точка позади.
pub fn pixel_at(camera: &Camera, eye: DVec3, rot: &DMat3, point: DVec3) -> Option<DVec2> {
    let cam = rot.transpose() * (point - eye);
    if cam.z >= 0.0 || cam.z.is_nan() {
        return None;
    }
    let f = focal(camera);
    Some(DVec2::new(
        f64::from(camera.width) / 2.0 + f * cam.x / -cam.z,
        f64::from(camera.height) / 2.0 - f * cam.y / -cam.z,
    ))
}

/// Точка поверхности под пикселем или None.
///
/// Без рельефа - точка эллипсоида. С рельефом ищется первое пересечение
/// луча с рельефом: шаги от глаза вдоль луча, каждый - доля
/// [`MARCH_SHARE`] высоты над рельефом, до смены знака, затем уточнение
/// делением отрезка. Прежде корень искался методом секущих от эллипсоида
/// и над горой ложился за неё, приближение к склону уносило камеру
/// к этой дальней точке (замечание автора 23 октября 2026 года).
/// Если шагов не хватило, остаётся метод секущих.
pub fn ground_under(camera: &Camera, px: f64, py: f64, terrain: Option<&Terrain>) -> Option<DVec3> {
    let e = &camera.ellipsoid;
    let (origin, d) = camera.ray(px, py);
    let t_surface = e.ray_intersect(origin, d, 0.0);
    let Some(terrain) = terrain else {
        return t_surface.map(|t| origin + d * t);
    };
    let gap = |t: f64| {
        let (lat, lon, h) = e.ecef_to_geodetic(origin + d * t);
        h - terrain(lat, lon)
    };
    // Луч выше горизонта эллипсоида тоже встречает горы.
    if let Some(t) = first_crossing(e, origin, d, &gap) {
        return Some(origin + d * t);
    }
    let t_surface = t_surface?;
    Some(origin + d * secant_crossing(e, origin, d, t_surface, terrain, &gap))
}

/// Первая смена знака gap вдоль луча и её уточнение. None - глаз под
/// рельефом, луч ушёл из слоя рельефа или шагов не хватило.
fn first_crossing(e: &Ellipsoid, origin: DVec3, d: DVec3, gap: &dyn Fn(f64) -> f64) -> Option<f64> {
    // Рельеф не выше MAX_TERRAIN: из космоса шаги идут от этой высоты.
    let mut ta = if altitude(e, origin) > MAX_TERRAIN { e.ray_intersect(origin, d, MAX_TERRAIN)? } else { 0.0 };
    // Конец - глубже самого глубокого дна или выход из слоя рельефа.
    let t_end = match e.ray_intersect(origin, d, -2.0 * MAX_TERRAIN) {
        Some(t) => t,
        None => {
            let inside = origin + d * (ta + MARCH_MIN * ta.max(1.0));
            ta + MARCH_MIN * ta.max(1.0) + e.ray_intersect(inside, d, MAX_TERRAIN)?
        }
    };
    let mut fa = gap(ta);
    if fa.is_nan() || fa <= 0.0 {
        return None;
    }
    for _ in 0..MARCH_STEPS {
        let tb = ta + (MARCH_SHARE * fa).max(MARCH_MIN * ta.max(1.0));
        let fb = gap(tb);
        if !fb.is_finite() {
            return None;
        }
        if fb <= 0.0 {
            return Some(refine(gap, (ta, fa), (tb, fb)));
        }
        if tb > t_end {
            return None;
        }
        (ta, fa) = (tb, fb);
    }
    None
}

/// Корень gap между a (над рельефом) и b (под ним): ложное положение
/// через шаг с делением пополам, корень не уходит из отрезка.
fn refine(gap: &dyn Fn(f64) -> f64, a: (f64, f64), b: (f64, f64)) -> f64 {
    let ((mut ta, mut fa), (mut tb, mut fb)) = (a, b);
    for i in 0..60 {
        let tm = if i % 2 == 0 && fa != fb { ta - fa * (tb - ta) / (fb - fa) } else { 0.5 * (ta + tb) };
        let tm = if tm > ta && tm < tb { tm } else { 0.5 * (ta + tb) };
        let fm = gap(tm);
        if fm.abs() < TERRAIN_TOLERANCE || !fm.is_finite() {
            return tm;
        }
        if fm > 0.0 {
            (ta, fa) = (tm, fm);
        } else {
            (tb, fb) = (tm, fm);
        }
    }
    0.5 * (ta + tb)
}

/// Корень методом секущих от эллипсоида и эллипсоида, раздутого на
/// высоту рельефа в первой точке. У горизонта над горами точка может
/// лечь на соседний склон.
fn secant_crossing(e: &Ellipsoid, origin: DVec3, d: DVec3, t_surface: f64, terrain: &Terrain, gap: &dyn Fn(f64) -> f64) -> f64 {
    let mut t0 = t_surface;
    let mut f0 = gap(t0);
    let (lat, lon, _) = e.ecef_to_geodetic(origin + d * t0);
    let Some(mut t1) = e.ray_intersect(origin, d, terrain(lat, lon)) else {
        return t0;
    };
    let mut f1 = gap(t1);
    for _ in 0..TERRAIN_STEPS {
        if f1.abs() < TERRAIN_TOLERANCE || f1 == f0 {
            break;
        }
        let next = (t1 - f1 * (t1 - t0) / (f1 - f0)).max(0.0);
        (t0, f0, t1) = (t1, f1, next);
        f1 = gap(t1);
    }
    t1
}

/// Начальная поза для подбора.
///
/// Под пикселем (px, py) при позе pose лежит точка Q. Поворот вокруг
/// центра тела, который переводит Q в point, переводит и точку взгляда
/// почти туда, где point встанет под пиксель. Для сферы это точно, для
/// эллипсоида приближённо. Без такого начала подбор у края диска уходит
/// в чужую долину невязки. Если под пикселем нет тела, начало - сама
/// поза.
fn first_guess(camera: &Camera, pose: &Pose, point: DVec3, px: f64, py: f64) -> Pose {
    let e = &camera.ellipsoid;
    let mut cam = camera.clone();
    pose.apply(&mut cam);
    let Some(q) = ground_under(&cam, px, py, pose.terrain.as_ref()) else {
        return pose.clone();
    };
    let a = q.normalize_or_zero();
    let b = point.normalize_or_zero();
    let axis = a.cross(b);
    let sin = axis.length();
    if sin < 1e-15 || sin.is_nan() {
        return pose.clone();
    }
    let angle = sin.atan2(a.dot(b));
    let axis = axis / sin;
    let look = e.geodetic_to_ecef(pose.lat, pose.lon, 0.0);
    // Поворот вектора вокруг оси, формула Родрига.
    let (s, c) = angle.sin_cos();
    let turned = look * c + axis.cross(look) * s + axis * axis.dot(look) * (1.0 - c);
    let (lat, lon, _) = e.ecef_to_geodetic(turned);
    pose.moved_to(lat, lon)
}

/// Поза с той же дальностью и углами, где point стоит в (px, py).
///
/// Возвращает новую позу или None, если решения нет.
pub fn solve_pin(camera: &Camera, pose: &Pose, point: DVec3, px: f64, py: f64) -> Option<Pose> {
    let e = &camera.ellipsoid;
    let target = DVec2::new(px, py);
    let (eye, _) = pose.eye_rotation(e);
    let dist = (point - eye).length();
    // Сдвиг точки взгляда на h радиан сдвигает картинку примерно
    // на A·h·f/dist пикселей. Шаг производной - сотая доля пикселя.
    let h = (0.01 * dist / (e.a * focal(camera))).to_degrees();
    let pixel = |p: &Pose| {
        let (eye, rot) = p.eye_rotation(e);
        pixel_at(camera, eye, &rot, point)
    };
    let mut current = first_guess(camera, pose, point, px, py);
    let mut p = pixel(&current)?;
    let mut error = (p - target).length();
    for _ in 0..30 {
        if error < PIN_TOLERANCE {
            return Some(current);
        }
        let d_lat = pixel(&current.moved_to(current.lat + h, current.lon))?;
        let d_lon = pixel(&current.moved_to(current.lat, current.lon + h))?;
        let jac = DMat2::from_cols((d_lat - p) / h, (d_lon - p) / h);
        let det = jac.determinant();
        if det == 0.0 || !det.is_finite() {
            return None;
        }
        let mut delta = jac.inverse() * (target - p);
        // Шаг ограничен 20 градусами, чтобы не перепрыгнуть шар.
        let biggest = delta.abs().max_element();
        if biggest > 20.0 {
            delta *= 20.0 / biggest;
        }
        // У края диска картинка почти не двигается, и полный шаг Ньютона
        // перелетает. Шаг принимается, только если невязка убывает.
        let mut accepted = None;
        for _ in 0..20 {
            let trial = current.moved_to(current.lat + delta.x, current.lon + delta.y);
            if let Some(q) = pixel(&trial)
                && (q - target).length() < error
            {
                accepted = Some((trial, q));
                break;
            }
            delta *= 0.5;
        }
        let Some((trial, q)) = accepted else {
            break;
        };
        (current, p) = (trial, q);
        error = (p - target).length();
    }
    if error > PIN_FAIL {
        return None;
    }
    Some(current)
}

/// Наибольшая высота глаза над телом, метров.
pub fn max_altitude(e: &Ellipsoid) -> f64 {
    MAX_ALTITUDE.max(MAX_ALTITUDE_RADII * e.a)
}

/// Глаз не ниже [`MIN_ALTITUDE`] над рельефом и не выше [`max_altitude`].
pub fn allowed(e: &Ellipsoid, pose: &Pose) -> bool {
    let (eye, _) = pose.eye_rotation(e);
    clearance(e, eye, pose.terrain.as_ref()) >= MIN_ALTITUDE && altitude(e, eye) <= max_altitude(e)
}

/// Ближайшее к distance расстояние, при котором высота глаза в рамках.
pub fn clamp_distance(e: &Ellipsoid, pose: &Pose, distance: f64) -> f64 {
    let mut trial = pose.clone();
    let mut fits = |d: f64| {
        trial.distance = d;
        allowed(e, &trial)
    };
    if fits(distance) {
        return distance;
    }
    // Высота растёт с расстоянием. Делением пополам между текущим
    // расстоянием и желаемым находится расстояние на границе.
    let (mut inside, mut outside) = (pose.distance, distance);
    for _ in 0..60 {
        let mid = 0.5 * (inside + outside);
        if fits(mid) {
            inside = mid;
        } else {
            outside = mid;
        }
    }
    inside
}

/// Поза, поднятая настолько, чтобы глаз был над рельефом с зазором.
///
/// Рельеф догружается, и точные высоты могут оказаться выше глаза.
/// Расстояние растёт, пока глаз не поднимется на [`MIN_ALTITUDE`] над
/// рельефом. Уже допустимая поза возвращается как есть.
pub fn lifted(e: &Ellipsoid, pose: Pose) -> Pose {
    if pose.terrain.is_none() || allowed(e, &pose) {
        return pose;
    }
    let mut out = pose;
    let mut distance = out.distance.max(MIN_ALTITUDE);
    for _ in 0..40 {
        out.distance = distance;
        if allowed(e, &out) {
            break;
        }
        distance *= 1.25;
    }
    out
}

/// Приблизить в factor раз к точке под курсором.
///
/// factor меньше 1 приближает. Возвращает новую позу или None, если
/// под курсором нет тела.
pub fn zoom(camera: &Camera, pose: &Pose, px: f64, py: f64, factor: f64) -> Option<Pose> {
    let e = &camera.ellipsoid;
    let point = ground_under(camera, px, py, pose.terrain.as_ref())?;
    let mut scaled = pose.clone();
    scaled.distance = clamp_distance(e, pose, pose.distance * factor);
    let out = solve_pin(camera, &scaled, point, px, py)?;
    // Приближение к точке сдвигает глаз к ней на долю |1 - f| пути.
    // Подбор, ушедший дальше, нашёл чужую позу, где точка тоже стоит под
    // курсором, - такой скачок не принимается.
    let (eye, _) = pose.eye_rotation(e);
    let (moved_eye, _) = out.eye_rotation(e);
    let f = scaled.distance / pose.distance;
    let reach = (point - eye).length();
    if (moved_eye - eye).length() > (ZOOM_REACH * (1.0 - f).abs()).max(ZOOM_SLACK) * reach {
        return None;
    }
    Some(out)
}

/// Повернуть на d_heading и наклонить на d_tilt градусов.
///
/// Наклон ограничен [`MAX_TILT`]. Наклон, при котором глаз опускается
/// ниже [`MIN_ALTITUDE`], уменьшается до допустимого.
pub fn orbit(e: &Ellipsoid, pose: &Pose, d_heading: f64, d_tilt: f64) -> Pose {
    let mut out = pose.clone();
    out.heading = (pose.heading + d_heading).rem_euclid(360.0);
    let wanted = (pose.tilt + d_tilt).clamp(0.0, MAX_TILT);
    out.tilt = wanted;
    if !allowed(e, &out) {
        let (mut low, mut high) = (pose.tilt.min(wanted), pose.tilt.max(wanted));
        for _ in 0..50 {
            out.tilt = 0.5 * (low + high);
            if allowed(e, &out) {
                low = out.tilt;
            } else {
                high = out.tilt;
            }
        }
        out.tilt = low;
    }
    out
}

/// Сдвинуть точку взгляда на forward метров по азимуту и right метров
/// вправо от него. Стрелки клавиатуры, как в Google Earth.
pub fn pan(pose: &Pose, forward: f64, right: f64) -> Pose {
    let (s, c) = pose.heading.to_radians().sin_cos();
    let north = forward * c - right * s;
    let east = forward * s + right * c;
    let cos_lat = pose.lat.to_radians().cos().max(1e-6);
    pose.moved_to(pose.lat + north / M_PER_DEGREE, pose.lon + east / (M_PER_DEGREE * cos_lat))
}

/// Повернуть взгляд на d_heading и d_tilt градусов, глаз на месте.
///
/// Взгляд по сторонам в Google Earth - Ctrl с левой кнопкой и Ctrl
/// со стрелками. Азимут и наклон позы отсчитываются в точке взгляда,
/// а она сама зависит от них. Точка ищется простой итерацией: луч
/// из глаза по новому направлению до земли, в новой точке заново
/// направление. Возвращает None, если взгляд уходит в небо или выходит
/// за наклон [`MAX_TILT`].
pub fn look(e: &Ellipsoid, pose: &Pose, d_heading: f64, d_tilt: f64) -> Option<Pose> {
    let heading = (pose.heading + d_heading).rem_euclid(360.0);
    let tilt = pose.tilt + d_tilt;
    if !(0.0..=MAX_TILT).contains(&tilt) {
        return None;
    }
    let (eye, _) = pose.eye_rotation(e);
    let (mut lat, mut lon, mut ground) = (pose.lat, pose.lon, pose.h);
    let mut distance = pose.distance;
    for _ in 0..LOOK_STEPS {
        let back = orientation(lat, lon, heading, tilt).z_axis;
        let t = e.ray_intersect(eye, -back, ground)?;
        let (new_lat, new_lon, _) = e.ecef_to_geodetic(eye - back * t);
        let done = (t - distance).abs() < LOOK_TOLERANCE * t;
        (lat, lon, distance) = (new_lat, new_lon, t);
        if let Some(terrain) = &pose.terrain {
            ground = terrain(lat, lon);
        }
        if done {
            break;
        }
    }
    let mut out = pose.moved_to(lat, lon);
    out.distance = distance;
    out.heading = heading;
    out.tilt = tilt;
    Some(out)
}

/// Сдвинуть вид параллельно экрану на dx, dy пикселей кадра.
///
/// Ctrl с левой кнопкой: часть разреза Земли у края кадра встаёт
/// в середину без поворота вокруг центра Земли. Глаз уходит в плоскости
/// экрана так, что точка на расстоянии точки взгляда идёт за курсором,
/// направление взгляда прежнее. Новая точка взгляда - пересечение того
/// же луча с поверхностью, азимут и наклон считаются в ней заново. Ось
/// вправо при сдвиге вбок уже не лежит в плоскости горизонта новой
/// точки, поэтому прежний поворот кадра держит крен позы. Возвращает
/// None, если луч уходит мимо тела, наклон больше [`MAX_TILT`] или глаз
/// выходит из допустимой высоты.
pub fn shift(camera: &Camera, pose: &Pose, dx: f64, dy: f64) -> Option<Pose> {
    let e = &camera.ellipsoid;
    let (eye, rot) = pose.eye_rotation(e);
    let metres = pose.distance / focal(camera);
    let eye = eye - rot.x_axis * (dx * metres) + rot.y_axis * (dy * metres);
    let back = rot.z_axis;
    let mut ground = pose.h;
    let mut found: Option<(f64, f64, f64)> = None;
    for _ in 0..LOOK_STEPS {
        let t = e.ray_intersect(eye, -back, ground)?;
        let (new_lat, new_lon, _) = e.ecef_to_geodetic(eye - back * t);
        let done = found.is_some_and(|(_, _, d)| (t - d).abs() < LOOK_TOLERANCE * t);
        found = Some((new_lat, new_lon, t));
        if let Some(terrain) = &pose.terrain {
            ground = terrain(new_lat, new_lon);
        }
        if done {
            break;
        }
    }
    let (lat, lon, distance) = found?;
    let [east, north, up] = enu(lat, lon);
    let tilt = back.dot(up).clamp(-1.0, 1.0).acos().to_degrees();
    if tilt > MAX_TILT || tilt.is_nan() {
        return None;
    }
    // Азимут - направление взгляда на касательной плоскости, при
    // отвесном взгляде - верх экрана. Верх экрана при наклоне не годится,
    // крен в новой точке нулевой, и прежний верх от направления отходит.
    let top = if tilt > 1e-3 { -back } else { rot.y_axis };
    let top = top - up * top.dot(up);
    let heading = top.dot(east).atan2(top.dot(north)).to_degrees().rem_euclid(360.0);
    let mut out = pose.moved_to(lat, lon);
    out.distance = distance;
    out.heading = heading;
    out.tilt = tilt;
    let plain = orientation(lat, lon, heading, tilt);
    out.roll = rot.x_axis.dot(plain.y_axis).atan2(rot.x_axis.dot(plain.x_axis)).to_degrees();
    allowed(e, &out).then_some(out)
}

/// Инерция после броска: момент и поза отпускания, скорость в градусах
/// в секунду.
#[derive(Debug, Clone)]
struct Inertia {
    start: f64,
    pose: Pose,
    v_lat: f64,
    v_lon: f64,
}

/// Плавное приближение: точка окна, остаток логарифма множителя и время
/// прошлого шага.
#[derive(Debug, Clone, Copy)]
struct Zooming {
    px: f64,
    py: f64,
    log_left: f64,
    last: f64,
}

/// Состояние навигации между событиями мыши.
pub struct Navigator {
    pub camera: Camera,
    pub pose: Pose,
    /// Схваченная точка поверхности.
    pub grab: Option<DVec3>,
    /// Рельеф из [`Navigator::set_terrain`]. Новая поза без рельефа
    /// получает его в [`Navigator::set_pose`], иначе точка взгляда
    /// стояла бы на эллипсоиде.
    pub terrain: Option<Terrain>,
    /// Точка взгляда в воздухе, не на рельефе - вид из кабины полёта.
    /// [`Navigator::keep_clear`] её не сажает, [`Navigator::show`] снимает.
    pub air: bool,
    history: Vec<(f64, f64, f64)>,
    inertia: Option<Inertia>,
    zooming: Option<Zooming>,
    /// Момент начала и само движение - перелёт или вращение.
    flight: Option<(f64, Motion)>,
}

impl Navigator {
    pub fn new(mut camera: Camera, pose: Pose) -> Self {
        pose.apply(&mut camera);
        Self {
            camera,
            pose,
            grab: None,
            terrain: None,
            air: false,
            history: Vec::new(),
            inertia: None,
            zooming: None,
            flight: None,
        }
    }

    fn ellipsoid(&self) -> Ellipsoid {
        self.camera.ellipsoid
    }

    pub fn set_pose(&mut self, pose: Pose) {
        let mut pose = pose;
        if pose.terrain.is_none()
            && let Some(terrain) = &self.terrain
        {
            pose.h = terrain(pose.lat, pose.lon);
            pose.terrain = Some(terrain.clone());
        }
        pose.apply(&mut self.camera);
        self.pose = pose;
    }

    /// Поставить позу кадра записи: на рельеф с зазором, как в перелёте.
    /// Движение гасится.
    pub fn show(&mut self, pose: Pose) {
        self.stop();
        self.air = false;
        let mut pose = pose.with_terrain(self.pose.terrain.clone());
        if let Some(terrain) = &pose.terrain {
            pose.h = terrain(pose.lat, pose.lon);
        }
        self.set_pose(lifted(&self.ellipsoid(), pose));
    }

    /// Подключить рельеф или снять его (None).
    pub fn set_terrain(&mut self, terrain: Option<Terrain>) {
        self.terrain = terrain.clone();
        let mut pose = self.pose.clone().with_terrain(terrain);
        pose.h = pose.terrain.as_ref().map_or(0.0, |t| t(pose.lat, pose.lon));
        self.set_pose(lifted(&self.ellipsoid(), pose));
    }

    /// Поднять глаз над рельефом, если точные высоты пришли выше.
    ///
    /// Точка взгляда тоже садится на уточнённый рельеф. Возвращает true,
    /// если поза изменилась.
    pub fn keep_clear(&mut self) -> bool {
        let e = self.ellipsoid();
        // Поза с точкой взгляда в воздухе - вид из кабины режима полёта,
        // её ставит полёт.
        let Some(terrain) = self.pose.terrain.clone() else {
            return false;
        };
        if self.air || self.pose.tilt > MAX_TILT {
            return false;
        }
        let ground = terrain(self.pose.lat, self.pose.lon);
        if (ground - self.pose.h).abs() < 0.01 && allowed(&e, &self.pose) {
            return false;
        }
        let out = self.pose.clone().with_h(ground);
        self.set_pose(lifted(&e, out));
        true
    }

    // Захват.

    pub fn press(&mut self, px: f64, py: f64, now: f64) -> bool {
        self.stop();
        self.grab = ground_under(&self.camera, px, py, self.pose.terrain.as_ref());
        self.history = vec![(now, self.pose.lat, self.pose.lon)];
        self.grab.is_some()
    }

    pub fn drag(&mut self, px: f64, py: f64, now: f64) -> bool {
        let Some(grab) = self.grab else {
            return false;
        };
        let Some(pose) = solve_pin(&self.camera, &self.pose, grab, px, py) else {
            return false;
        };
        self.history.push((now, pose.lat, pose.lon));
        self.set_pose(pose);
        let cutoff = now - 4.0 * VELOCITY_WINDOW;
        self.history.retain(|h| h.0 >= cutoff);
        true
    }

    /// Отпускание. Возвращает true, если началась инерция.
    pub fn release(&mut self, now: f64) -> bool {
        let history = std::mem::take(&mut self.history);
        let grabbed = self.grab.take();
        if grabbed.is_none() || history.len() < 2 {
            return false;
        }
        let (Some(first_entry), Some(last)) = (history.first(), history.last()) else {
            return false;
        };
        if now - last.0 > STILL_BEFORE_RELEASE {
            return false;
        }
        let recent = history.iter().filter(|h| h.0 >= now - VELOCITY_WINDOW).count();
        let first = history.len().checked_sub(recent + 1).and_then(|i| history.get(i)).unwrap_or(first_entry);
        let span = last.0 - first.0;
        if span <= 0.0 || span.is_nan() {
            return false;
        }
        let v_lat = (last.1 - first.1) / span;
        let d_lon = (last.2 - first.2 + 180.0).rem_euclid(360.0) - 180.0;
        let v_lon = d_lon / span;
        if v_lat == 0.0 && v_lon == 0.0 {
            return false;
        }
        self.inertia = Some(Inertia { start: now, pose: self.pose.clone(), v_lat, v_lon });
        true
    }

    // Колесо.

    /// Плавное приближение: множитель копится и отдаётся по кадрам.
    pub fn wheel(&mut self, px: f64, py: f64, factor: f64, now: f64) {
        self.inertia = None;
        self.flight = None;
        let mut log_left = factor.ln();
        if let Some(z) = self.zooming
            && z.px == px
            && z.py == py
        {
            log_left += z.log_left;
        }
        self.zooming = Some(Zooming { px, py, log_left, last: now });
    }

    // Наклон и поворот.

    /// Поворот гасит инерцию и перелёт, но не обрывает приближение.
    pub fn turn(&mut self, d_heading: f64, d_tilt: f64) {
        self.stop_inertia();
        self.flight = None;
        let pose = orbit(&self.ellipsoid(), &self.pose, d_heading, d_tilt);
        self.set_pose(pose);
    }

    /// Приближение без сглаживания: правая кнопка с перетаскиванием.
    /// Точка под (px, py) остаётся на месте.
    pub fn zoom_now(&mut self, px: f64, py: f64, factor: f64) -> bool {
        self.stop();
        let Some(pose) = zoom(&self.camera, &self.pose, px, py, factor) else {
            return false;
        };
        self.set_pose(pose);
        true
    }

    /// Взгляд по сторонам, глаз на месте.
    pub fn look_around(&mut self, d_heading: f64, d_tilt: f64) -> bool {
        self.stop();
        let Some(pose) = look(&self.ellipsoid(), &self.pose, d_heading, d_tilt) else {
            return false;
        };
        self.set_pose(pose);
        true
    }

    /// Сдвиг вида параллельно экрану, Ctrl с левой кнопкой.
    pub fn shift_by(&mut self, dx: f64, dy: f64) -> bool {
        self.stop();
        let Some(pose) = shift(&self.camera, &self.pose, dx, dy) else {
            return false;
        };
        self.set_pose(pose);
        true
    }

    /// Сдвиг точки взгляда в метрах, стрелки клавиатуры.
    pub fn pan_by(&mut self, forward: f64, right: f64) {
        self.stop();
        let pose = lifted(&self.ellipsoid(), pan(&self.pose, forward, right));
        self.set_pose(pose);
    }

    pub fn has_inertia(&self) -> bool {
        self.inertia.is_some()
    }

    // Перелёт.

    /// Начать перелёт или вращение. Всё прочее гасится.
    pub fn start_flight(&mut self, motion: Motion, now: f64) {
        self.stop();
        self.flight = Some((now, motion));
    }

    pub fn has_flight(&self) -> bool {
        self.flight.is_some()
    }

    pub fn stop_inertia(&mut self) {
        self.inertia = None;
    }

    // Кадр.

    /// Продвинуть перелёт, инерцию и плавное приближение. true - ещё идёт.
    pub fn step(&mut self, now: f64) -> bool {
        let mut busy = false;
        if let Some((start, motion)) = &self.flight {
            let elapsed = now - start;
            let done = elapsed >= motion.duration();
            let mut pose = motion.pose_at(elapsed).with_terrain(self.pose.terrain.clone());
            if let Some(terrain) = &pose.terrain {
                pose.h = terrain(pose.lat, pose.lon);
            }
            let pose = lifted(&self.ellipsoid(), pose);
            self.set_pose(pose);
            if done {
                self.flight = None;
            } else {
                busy = true;
            }
        }
        if self.inertia.is_some() {
            busy = self.step_inertia(now) || busy;
        }
        if self.zooming.is_some() {
            busy = self.step_zoom(now) || busy;
        }
        busy
    }

    fn step_inertia(&mut self, now: f64) -> bool {
        let Some(inertia) = self.inertia.clone() else {
            return false;
        };
        let mut remaining = (-(now - inertia.start) / INERTIA_TAU).exp();
        let done = remaining < INERTIA_STOP;
        if done {
            // Конец пути - его предел. Он один при любой частоте кадров,
            // скачок к нему не больше INERTIA_STOP от всего пути.
            remaining = 0.0;
        }
        let path = INERTIA_TAU * (1.0 - remaining);
        let p = &inertia.pose;
        self.set_pose(p.moved_to(p.lat + inertia.v_lat * path, p.lon + inertia.v_lon * path));
        if done {
            self.inertia = None;
            return false;
        }
        true
    }

    fn step_zoom(&mut self, now: f64) -> bool {
        let Some(z) = self.zooming else {
            return false;
        };
        let dt = (now - z.last).max(0.0);
        let mut part = z.log_left * (1.0 - (-dt / ZOOM_TAU).exp());
        if z.log_left.abs() < 1e-3 {
            part = z.log_left;
        }
        let pose = zoom(&self.camera, &self.pose, z.px, z.py, part.exp());
        let log_left = z.log_left - part;
        let moved = pose.is_some();
        if let Some(pose) = pose {
            self.set_pose(pose);
        }
        if !moved || log_left.abs() < 1e-9 {
            self.zooming = None;
            return false;
        }
        self.zooming = Some(Zooming { log_left, last: now, ..z });
        true
    }

    pub fn stop(&mut self) {
        self.inertia = None;
        self.zooming = None;
        self.flight = None;
    }
}
