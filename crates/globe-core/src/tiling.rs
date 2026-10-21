// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сетка тайлов Web Mercator и сетка вершин тайла.
//!
//! Номер тайла (z, x, y) устроен как у OSM. Уровень z делит мир на 2^z
//! столбцов и 2^z строк. Столбец x идёт от долготы -180 на восток,
//! строка y от северной границы на юг. Сетка кончается на широте
//! [`MAX_LAT`].
//!
//! Вершины тайла лежат равномерно в координатах Mercator, поэтому
//! картинка тайла ложится на них без пересчёта. Вершины хранятся в f32
//! как смещение от центра тайла, центр хранится в f64. Мировые
//! координаты в видеокарту не попадают.
//!
//! Перенос `planetx/core/tiling.py` модуля без рельефа. Высоты, отмывка,
//! нормали рельефа и проход воды переносятся вместе с `terrain`
//! в фазе 1. Уровни глубже [`MAX_LEVEL`] прижимаются к нему.

use glam::{DVec3, Vec2, Vec3};

use crate::ellipsoid::Ellipsoid;

/// Граница сетки по широте, `atan(sinh(π))` в градусах. Число равно
/// `lat_of_row(0.0)` до бита, это стережёт тест `max_lat`.
pub const MAX_LAT: f64 = 85.051_128_779_806_6;
pub const MAX_LEVEL: u32 = 19;
pub const MIN_SKIRT: f64 = 2.0;
/// Подстилка - плоские тайлы уровня 2 глубже поверхности на столько
/// метров. Это больше шага буфера глубины у дальней плоскости, в худшем
/// случае 1.1 км.
pub const UNDERLAY_DEPTH: f64 = 3000.0;
/// Счётчик дыр не смотрит на полосу такой ширины у края диска. Луч
/// по касательной к диску проходит под хордами тайлов, пока в кадре
/// у края только уровень 2: треугольник проседает на `cell_sag(2)`,
/// подстилка ещё на [`UNDERLAY_DEPTH`].
pub const HOLE_MARGIN: f64 = 20000.0;

fn level(z: u32) -> u32 {
    z.min(MAX_LEVEL)
}

/// Количество тайлов по стороне мира на уровне z.
fn tiles_per_side(z: u32) -> u32 {
    1 << level(z)
}

/// Количество отрезков на сторону тайла уровня z.
///
/// Мелкие уровни получают больше отрезков, чтобы край глобуса оставался
/// круглым. Отрезок уровня 0 занимает 5.6° долготы.
pub fn segments(z: u32) -> u32 {
    (64u32 >> level(z)).max(16)
}

/// Широта в градусах по доле высоты мира t, t = 0 на северной границе.
pub fn lat_of_row(t: f64) -> f64 {
    (std::f64::consts::PI * (1.0 - 2.0 * t)).sinh().atan().to_degrees()
}

/// Номер тайла уровня z, в котором лежит точка.
///
/// Широта за пределами [`MAX_LAT`] прижимается к краю сетки, долгота 180
/// попадает в последний столбец. NaN даёт тайл в углу сетки.
pub fn lonlat_to_tile(lat: f64, lon: f64, z: u32) -> (u32, u32) {
    let n = f64::from(tiles_per_side(z));
    let last = tiles_per_side(z) - 1;
    let lat = lat.clamp(-MAX_LAT, MAX_LAT);
    let merc = lat.to_radians().tan().asinh();
    let x = ((lon + 180.0) / 360.0 * n).floor();
    let y = ((1.0 - merc / std::f64::consts::PI) / 2.0 * n).floor();
    // Приведение f64 к u32 насыщается, NaN даёт 0.
    ((x.max(0.0) as u32).min(last), (y.max(0.0) as u32).min(last))
}

/// Границы тайла в градусах.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

pub fn tile_bounds(z: u32, x: u32, y: u32) -> Bounds {
    let n = f64::from(tiles_per_side(z));
    let (x, y) = (f64::from(x), f64::from(y));
    Bounds {
        west: x / n * 360.0 - 180.0,
        south: lat_of_row((y + 1.0) / n),
        east: (x + 1.0) / n * 360.0 - 180.0,
        north: lat_of_row(y / n),
    }
}

/// Наибольший прогиб хорды отрезка уровня z под поверхностью, метры.
///
/// Считается по экватору, где отрезок самый длинный.
pub fn segment_sag(e: &Ellipsoid, z: u32) -> f64 {
    let angle = std::f64::consts::TAU / f64::from(tiles_per_side(z) * segments(z));
    e.a * (1.0 - (angle / 2.0).cos())
}

/// Наибольший прогиб треугольников ячейки уровня z, метры.
///
/// Глубже всего проседает середина диагонали ячейки, она в √2 раз
/// длиннее отрезка стороны. Считается по экватору. У уровней 0-2
/// около 15.4 км, у уровня 3 около 3.9 км.
pub fn cell_sag(e: &Ellipsoid, z: u32) -> f64 {
    let angle = std::f64::consts::SQRT_2 * std::f64::consts::TAU
        / f64::from(tiles_per_side(z) * segments(z));
    e.a * (1.0 - (angle / 2.0).cos())
}

/// Глубина юбки тайла уровня z, метры.
///
/// Юбка закрывает щель у соседа уровнем грубее. Его хорда проседает
/// под поверхность на `segment_sag(z - 1)`, юбка берётся вдвое глубже.
pub fn skirt_depth(e: &Ellipsoid, z: u32) -> f64 {
    MIN_SKIRT.max(2.0 * segment_sag(e, z.saturating_sub(1)))
}

/// Номера краевых вершин сетки по кругу.
///
/// Порядок - северный край с запада на восток, восточный с севера
/// на юг, южный с востока на запад, западный с юга на север. Снаружи
/// Земли этот обход идёт по часовой стрелке.
pub fn ring(seg: u32) -> Vec<u32> {
    let side = seg + 1;
    let north = 0..seg;
    let east = (0..seg).map(|j| j * side + seg);
    let south = (0..seg).map(|i| seg * side + seg - i);
    let west = (0..seg).map(|j| (seg - j) * side);
    north.chain(east).chain(south).chain(west).collect()
}

/// Треугольники сетки и юбки, обход против часовой стрелки снаружи.
///
/// Буфер общий для всех тайлов с тем же количеством отрезков. Номера
/// вершин помещаются в u16 при seg до 64.
pub fn index_buffer(seg: u32) -> Vec<u16> {
    let seg = seg.clamp(1, 64);
    let side = seg + 1;
    let mut out: Vec<u32> = Vec::with_capacity((6 * seg * seg + 24 * seg) as usize);
    for j in 0..seg {
        for i in 0..seg {
            let tl = j * side + i;
            let tr = tl + 1;
            let bl = tl + side;
            let br = bl + 1;
            out.extend([tl, bl, tr, tr, bl, br]);
        }
    }
    let top = ring(seg);
    let count = top.len() as u32;
    let low0 = side * side;
    for k in 0..count {
        let next = (k + 1) % count;
        let (t, tn) = (top[k as usize], top[next as usize]);
        let (l, ln) = (low0 + k, low0 + next);
        out.extend([t, tn, l, tn, ln, l]);
    }
    out.into_iter().map(|i| i as u16).collect()
}

/// Широта и долгота узлов сетки тайла по строкам, (seg+1)² пар.
///
/// Строка - широта с севера на юг, столбец - долгота с запада на восток.
/// Узлы считаются от общего целого номера узла на уровне. Поэтому общий
/// край соседних тайлов получает одни и те же числа до бита, в том числе
/// на линии смены дат. border добавляет полосу узлов вокруг тайла,
/// она нужна для нормалей на краю.
pub fn grid_latlon(z: u32, x: u32, y: u32, border: u32) -> Vec<(f64, f64)> {
    let seg = i64::from(segments(z));
    let total = i64::from(tiles_per_side(z)) * seg;
    let border = i64::from(border);
    let idx = -border..=seg + border;
    let lons: Vec<f64> = idx
        .clone()
        .map(|k| (i64::from(x) * seg + k).rem_euclid(total) as f64 / total as f64 * 360.0 - 180.0)
        .collect();
    let lats: Vec<f64> = idx.map(|k| lat_of_row((i64::from(y) * seg + k) as f64 / total as f64)).collect();
    lats.iter().flat_map(|lat| lons.iter().map(move |lon| (*lat, *lon))).collect()
}

/// Узлы сетки тайла на эллипсоиде в ECEF, f64, по строкам.
pub fn grid_ecef(e: &Ellipsoid, z: u32, x: u32, y: u32) -> Vec<DVec3> {
    grid_latlon(z, x, y, 0).into_iter().map(|(lat, lon)| e.geodetic_to_ecef(lat, lon, 0.0)).collect()
}

/// Чья сетка - тайла или шапки у полюса.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MeshId {
    Tile { z: u32, x: u32, y: u32 },
    PolarCap { north: bool },
}

/// Сетка вершин одного тайла.
#[derive(Debug, Clone, PartialEq)]
pub struct TileMesh {
    pub id: MeshId,
    /// Отрезков на сторону у тайла, вершин края у шапки.
    pub segments: u32,
    /// Центр в ECEF, f64.
    pub center: DVec3,
    /// Радиус описанной сферы вокруг центра.
    pub radius: f64,
    /// Смещения вершин от центра, f32. Сначала сетка по строкам,
    /// потом юбка по кругу [`ring`].
    pub positions: Vec<Vec3>,
    /// Текстурные координаты, v = 0 на северном краю.
    pub uv: Vec<Vec2>,
    pub indices: Vec<u16>,
}

fn offsets(world: &[DVec3], center: DVec3) -> (Vec<Vec3>, f64) {
    let radius = world.iter().map(|p| (*p - center).length()).fold(0.0, f64::max);
    (world.iter().map(|p| (*p - center).as_vec3()).collect(), radius)
}

/// Сетка вершин тайла на гладком эллипсоиде с юбкой.
pub fn tile_mesh(e: &Ellipsoid, z: u32, x: u32, y: u32) -> TileMesh {
    let z = level(z);
    let seg = segments(z);
    let side = seg + 1;
    let latlon = grid_latlon(z, x, y, 0);
    let up: Vec<DVec3> = latlon.iter().map(|(lat, lon)| e.surface_normal(*lat, *lon)).collect();
    let grid: Vec<DVec3> = latlon.iter().map(|(lat, lon)| e.geodetic_to_ecef(*lat, *lon, 0.0)).collect();
    let rim: Vec<usize> = ring(seg).into_iter().map(|i| i as usize).collect();
    let depth = skirt_depth(e, z);
    let mut world = grid.clone();
    world.extend(rim.iter().map(|i| grid[*i] - up[*i] * depth));

    let center = grid[((seg / 2) * side + seg / 2) as usize];
    let (positions, radius) = offsets(&world, center);

    let step = f64::from(seg);
    let mut uv: Vec<Vec2> = (0..side)
        .flat_map(|j| (0..side).map(move |i| Vec2::new((f64::from(i) / step) as f32, (f64::from(j) / step) as f32)))
        .collect();
    let skirt_uv: Vec<Vec2> = rim.iter().map(|i| uv[*i]).collect();
    uv.extend(skirt_uv);

    TileMesh { id: MeshId::Tile { z, x, y }, segments: seg, center, radius, positions, uv, indices: index_buffer(seg) }
}

/// Шапка над краем сетки у полюса, веер треугольников.
///
/// Край шапки лежит на широте [`MAX_LAT`], вершина веера на полюсе.
/// Шапка закрашивается сплошным цветом, поэтому uv всех вершин нулевые.
/// Количество вершин края прижимается к 3-65534, чтобы номера вершин
/// помещались в u16.
pub fn polar_cap_mesh(e: &Ellipsoid, north: bool, count: u32) -> TileMesh {
    let count = count.clamp(3, u32::from(u16::MAX) - 1);
    let sign = if north { 1.0 } else { -1.0 };
    let pole = e.geodetic_to_ecef(sign * 90.0, 0.0, 0.0);
    let mut world = vec![pole];
    world.extend(
        (0..count).map(|k| e.geodetic_to_ecef(sign * MAX_LAT, f64::from(k) / f64::from(count) * 360.0 - 180.0, 0.0)),
    );
    let (positions, radius) = offsets(&world, pole);

    // Долгота растёт на восток. Над северным полюсом обход против часовой
    // стрелки снаружи идёт в сторону роста долготы, над южным наоборот.
    let mut indices = Vec::with_capacity(3 * count as usize);
    for k in 0..count {
        let a = k + 1;
        let b = (k + 1) % count + 1;
        let (a, b) = if north { (a, b) } else { (b, a) };
        indices.extend([0, a as u16, b as u16]);
    }
    let uv = vec![Vec2::ZERO; world.len()];
    TileMesh { id: MeshId::PolarCap { north }, segments: count, center: pole, radius, positions, uv, indices }
}
