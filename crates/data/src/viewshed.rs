// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Видимость из точки по рельефу. Перенос `core/viewshed.py` модуля.
//!
//! Образец - «Показать зону видимости» у метки Google Earth. Из точки
//! наблюдения расходятся лучи по азимутам с равным шагом, вдоль каждого
//! луча - точки через шаг расчёта. Высоты берутся функцией heights по
//! списку точек (широта, долгота), настоящие, без масштаба вида. Глаз
//! стоит на высоте observer над рельефом. Точка луча видна, если угол на
//! неё с поправкой на кривизну тела не ниже наибольшего угла на рельеф
//! перед ней. Цель поднята на target над рельефом. Кривизна - опускание
//! d² / (2·R'), R' = R / (1 - k), k - коэффициент рефракции: у Земли
//! [`REFRACTION`], принятый в геодезии, у тел без плотной атмосферы 0.
//!
//! Шаг и количество лучей подобраны так, чтобы клетка у края круга была
//! не крупнее шага: лучей не меньше 2πr / шаг. Лучей не больше
//! [`MAX_RAYS`], точек на луче не больше [`MAX_STEPS`].
//!
//! Картинка тайла глобуса ([`tile_rgba`]) берёт для каждого пикселя
//! ближайший луч и шаг.

/// Коэффициент рефракции Земли.
pub const REFRACTION: f64 = 0.13;
pub const MAX_RAYS: usize = 4096;
pub const MAX_STEPS: usize = 1024;
/// Шагов на радиус круга, шаг расчёта - радиус / STEPS.
pub const STEPS: usize = 500;
/// Тайлов высот под кругом.
pub const MAX_TILES: usize = 64;
/// Пикселей тайла высот на сторону.
pub const TILE: usize = 256;
pub const VISIBLE: [u8; 3] = [40, 200, 60];
pub const HIDDEN: [u8; 3] = [200, 40, 40];
/// Непрозрачность раскраски поверх снимка.
pub const OPACITY: f64 = 0.45;
/// Большая полуось WGS84 - радиус тела по умолчанию.
pub const EARTH_RADIUS: f64 = 6378137.0;
/// Самый подробный уровень тайлов высот, глубже счёт номеров не идёт.
const LEVEL_LIMIT: u32 = 30;
/// Предел широты сетки Меркатора, градусы.
const MAX_LAT: f64 = 85.05112878;

/// Точка на расстоянии distance метров по азимуту bearing (градусы) от
/// точки (lat, lon) на сфере радиуса radius. Долгота от -180 до 180.
pub fn destination(lat: f64, lon: f64, bearing: f64, distance: f64, radius: f64) -> (f64, f64) {
    let phi = lat.to_radians();
    let theta = bearing.to_radians();
    let delta = distance / radius;
    let sin_phi2 = phi.sin() * delta.cos() + phi.cos() * delta.sin() * theta.cos();
    let phi2 = sin_phi2.clamp(-1.0, 1.0).asin();
    let lam2 = lon.to_radians() + (theta.sin() * delta.sin() * phi.cos()).atan2(delta.cos() - phi.sin() * sin_phi2);
    (phi2.to_degrees(), (lam2.to_degrees() + 180.0).rem_euclid(360.0) - 180.0)
}

/// Точки по азимутам bearings на расстояниях distances: для каждого
/// азимута все расстояния подряд, широта и долгота.
pub fn destinations(lat: f64, lon: f64, bearings: &[f64], distances: &[f64], radius: f64) -> Vec<(f64, f64)> {
    bearings.iter().flat_map(|&b| distances.iter().map(move |&d| destination(lat, lon, b, d, radius))).collect()
}

/// Расстояние по дуге (м) и азимут (градусы 0-360) от точки (lat, lon)
/// к точке (lat2, lon2), сфера радиуса radius.
pub fn distance_bearing(lat: f64, lon: f64, lat2: f64, lon2: f64, radius: f64) -> (f64, f64) {
    let phi1 = lat.to_radians();
    let phi2 = lat2.to_radians();
    let dlam = lon2.to_radians() - lon.to_radians();
    let a = ((phi2 - phi1) / 2.0).sin().powi(2) + phi1.cos() * phi2.cos() * (dlam / 2.0).sin().powi(2);
    let dist = 2.0 * radius * a.clamp(0.0, 1.0).sqrt().asin();
    let y = dlam.sin() * phi2.cos();
    let x = phi1.cos() * phi2.sin() - phi1.sin() * phi2.cos() * dlam.cos();
    (dist, y.atan2(x).to_degrees().rem_euclid(360.0))
}

/// Широты и долготы рамки круга: (юг, север, запад, восток). Рамка через
/// 180° долготы заменяется всем поясом -180..180.
pub fn circle_box(lat: f64, lon: f64, radius_m: f64, radius: f64) -> (f64, f64, f64, f64) {
    let dlat = (radius_m / radius).to_degrees();
    let cos = (lat.abs() + dlat).min(89.9).to_radians().cos();
    let dlon = (dlat / cos.max(1e-6)).min(180.0);
    let (mut west, mut east) = (lon - dlon, lon + dlon);
    if west < -180.0 || east > 180.0 {
        (west, east) = (-180.0, 180.0);
    }
    ((lat - dlat).max(-90.0), (lat + dlat).min(90.0), west, east)
}

/// Номер тайла уровня z, в котором лежит точка, как `lonlat_to_tile`
/// модуля: широта за MAX_LAT прижимается к краю сетки, долгота 180 -
/// в последнем столбце.
fn lonlat_to_tile(lat: f64, lon: f64, z: u32) -> (i64, i64) {
    let n = 1i64 << z.min(LEVEL_LIMIT);
    let lat = lat.clamp(-MAX_LAT, MAX_LAT);
    let x = ((lon + 180.0) / 360.0 * n as f64).floor() as i64;
    let merc = lat.to_radians().tan().asinh();
    let y = ((1.0 - merc / std::f64::consts::PI) / 2.0 * n as f64).floor() as i64;
    (x.clamp(0, n - 1), y.clamp(0, n - 1))
}

/// Уровень и тайлы высот под кругом: (уровень, ключи (уровень, x, y)).
/// Уровень - тот, у которого пиксель не крупнее шага cell, но не глубже
/// max_level и не мельче, чем нужно, чтобы тайлов было не больше
/// [`MAX_TILES`].
pub fn height_tiles(lat: f64, lon: f64, radius_m: f64, cell: f64, radius: f64, max_level: u32) -> (u32, Vec<(u32, u32, u32)>) {
    let ground = 2.0 * std::f64::consts::PI * radius * lat.to_radians().cos() / TILE as f64;
    let top = max_level.min(LEVEL_LIMIT);
    let wanted = (ground.max(1e-9) / cell).log2().ceil();
    // Нечисло - самый мелкий уровень, бесконечность - предел.
    let mut level = if wanted.is_nan() { 0 } else { wanted.clamp(0.0, f64::from(top)) as u32 };
    let (south, north, west, east) = circle_box(lat, lon, radius_m, radius);
    loop {
        let (x0, y0) = lonlat_to_tile(north, west, level);
        let (x1, y1) = lonlat_to_tile(south, east.min(179.999999), level);
        let count = (x1 - x0 + 1).max(0) * (y1 - y0 + 1).max(0);
        if count <= MAX_TILES as i64 || level == 0 {
            let mut keys = Vec::new();
            for x in x0..=x1 {
                for y in y0..=y1 {
                    keys.push((level, x as u32, y as u32));
                }
            }
            return (level, keys);
        }
        level -= 1;
    }
}

/// Количество лучей и шагов и настоящий шаг для круга radius_m метров
/// при желаемом шаге cell.
pub fn layout(radius_m: f64, cell: f64) -> (usize, usize, f64) {
    let steps = (radius_m / cell).ceil().max(8.0).min(MAX_STEPS as f64) as usize;
    let step = radius_m / steps as f64;
    let rays = (2.0 * std::f64::consts::PI * radius_m / step).ceil().max(360.0).min(MAX_RAYS as f64) as usize;
    (rays, steps, step)
}

/// Высота глаза и цели и величины тела для [`compute`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    /// Глаз над рельефом, метры.
    pub observer: f64,
    /// Цель над рельефом, метры.
    pub target: f64,
    /// Коэффициент рефракции: [`REFRACTION`] у Земли, 0 у других тел.
    pub refraction: f64,
    /// Радиус тела, метры.
    pub radius: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self { observer: 2.0, target: 0.0, refraction: REFRACTION, radius: EARTH_RADIUS }
    }
}

/// Функция настоящих высот списка точек (широта, долгота), по высоте
/// на точку.
pub type Heights<'a> = &'a dyn Fn(&[(f64, f64)]) -> Vec<f64>;

/// Видимость из точки: visible - лучи × шаги подряд, луч за лучом.
#[derive(Debug, Clone, PartialEq)]
pub struct Viewshed {
    pub lat: f64,
    pub lon: f64,
    pub radius_m: f64,
    /// Шаг расчёта вдоль луча, метры.
    pub step: f64,
    pub rays: usize,
    pub steps: usize,
    pub visible: Vec<bool>,
}

impl Viewshed {
    /// Видимость шагов луча ray, пусто - луча нет.
    pub fn ray(&self, ray: usize) -> &[bool] {
        self.visible.get(ray * self.steps..(ray + 1) * self.steps).unwrap_or(&[])
    }

    /// Доля видимой площади круга. Шаг j - кольцо площадью
    /// пропорционально его радиусу.
    pub fn share(&self) -> f64 {
        let weights: f64 = (1..=self.steps).map(|j| j as f64).sum();
        let seen: f64 = self.visible.iter().enumerate().filter(|(_, v)| **v).map(|(i, _)| (i % self.steps.max(1) + 1) as f64).sum();
        let total = weights * self.rays as f64;
        if total > 0.0 { seen / total } else { 0.0 }
    }

    /// Видимость в точках (широта, долгота): (видно, внутри круга).
    pub fn at(&self, points: &[(f64, f64)], radius: f64) -> Vec<(bool, bool)> {
        let (rays, steps) = (self.rays.max(1) as i64, self.steps.max(1) as i64);
        points
            .iter()
            .map(|&(lat, lon)| {
                let (dist, az) = distance_bearing(self.lat, self.lon, lat, lon, radius);
                let ray = ((az / 360.0 * rays as f64).round_ties_even() as i64).rem_euclid(rays);
                let index = (dist / self.step).round_ties_even() as i64 - 1;
                let inside = dist <= self.radius_m;
                // Клетка самой точки наблюдения.
                let near = index < 0;
                let index = index.clamp(0, steps - 1);
                let seen = self.visible.get((ray * steps + index) as usize).copied().unwrap_or(false) || near;
                (seen && inside, inside)
            })
            .collect()
    }
}

/// Видимость из точки (lat, lon) в круге radius_m метров с желаемым
/// шагом cell. heights - настоящие высоты списка точек (широта,
/// долгота), по высоте на точку. None - радиус круга или тела не
/// положителен, или высот вернулось не столько, сколько точек.
pub fn compute(lat: f64, lon: f64, radius_m: f64, cell: f64, heights: Heights, params: &Params) -> Option<Viewshed> {
    let radius = params.radius;
    if !(radius_m.is_finite() && radius_m > 0.0 && radius.is_finite() && radius > 0.0) {
        return None;
    }
    let (rays, steps, step) = layout(radius_m, cell);
    let bearings: Vec<f64> = (0..rays).map(|i| i as f64 * 360.0 / rays as f64).collect();
    let dist: Vec<f64> = (1..=steps).map(|j| j as f64 * step).collect();
    let points = destinations(lat, lon, &bearings, &dist, radius);
    let ground = heights(&points);
    if ground.len() != points.len() {
        return None;
    }
    let eye = *heights(&[(lat, lon)]).first()? + params.observer;
    let bent = 2.0 * radius / (1.0 - params.refraction);
    let mut visible = Vec::with_capacity(points.len());
    for ray in ground.chunks(steps) {
        // Наибольший угол на рельеф перед точкой, у первой точки впереди
        // ничего нет.
        let mut before = f64::NEG_INFINITY;
        for (h, d) in ray.iter().zip(&dist) {
            let drop = d * d / bent;
            let slope_ground = (h - drop - eye) / d;
            let slope_target = (h + params.target - drop - eye) / d;
            visible.push(slope_target >= before);
            before = before.max(slope_ground);
        }
    }
    Some(Viewshed { lat, lon, radius_m, step, rays, steps, visible })
}

/// Видимость сеткой широт и долгот для GeoTIFF, `raster` модуля: байты -
/// 1 видно, 0 скрыто, 255 вне круга, строки с севера. Шаг - шаг расчёта.
pub fn raster(result: &Viewshed, radius: f64) -> (crate::geotiff::LatLonGrid, Vec<u8>) {
    let (south, north, west, east) = circle_box(result.lat, result.lon, result.radius_m, radius);
    let dlat = (result.step / radius).to_degrees();
    let dlon = dlat / result.lat.to_radians().cos().max(0.01);
    let rows = ((north - south) / dlat).ceil().max(1.0) as usize;
    let cols = ((east - west) / dlon).ceil().max(1.0) as usize;
    let grid = crate::geotiff::LatLonGrid { north: north - 0.5 * dlat, west: west + 0.5 * dlon, dlat, dlon, cols, rows };
    let points: Vec<(f64, f64)> = (0..rows).flat_map(|r| (0..cols).map(move |c| (grid.north - r as f64 * dlat, grid.west + c as f64 * dlon))).collect();
    let values = result.at(&points, radius).into_iter().map(|(seen, inside)| if inside { u8::from(seen) } else { crate::geotiff::BYTE_NODATA }).collect();
    (grid, values)
}

/// Картинка тайла: видимое зелёное, скрытое красное, вне круга
/// прозрачно. RGBA с премноженной альфой, пиксель на точку points.
pub fn tile_rgba(viewshed: &Viewshed, points: &[(f64, f64)], radius: f64) -> Vec<[u8; 4]> {
    let alpha = (OPACITY * 255.0).round() as u8;
    let paint = |rgb: [u8; 3]| {
        let c = |v: u8| (f64::from(v) * OPACITY).round_ties_even() as u8;
        [c(rgb[0]), c(rgb[1]), c(rgb[2]), alpha]
    };
    viewshed
        .at(points, radius)
        .into_iter()
        .map(|(seen, inside)| {
            if seen {
                paint(VISIBLE)
            } else if inside {
                paint(HIDDEN)
            } else {
                [0; 4]
            }
        })
        .collect()
}
