// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Тени на момент - карта «солнце или тень» участка по рельефу. Перенос
//! `core/shadows.py` модуля, задача автора от 28 октября 2026 года - окно
//! «Тени» подменю «Анализ участка», тени на момент шкалы времени.
//!
//! Сетка и горизонт - те же, что у инсоляции ([`crate::insolation`]):
//! квадратная сетка высот с полем для теней гор, тангенс угла горизонта
//! внутренних узлов по [`insolation::AZIMUTHS`] азимутам. Горизонт
//! считается один раз ([`Horizon`]), карта на момент - сравнение высоты
//! солнца с горизонтом по его азимуту, между соседними азимутами -
//! линейно. Так движение шкалы времени даёт новую карту без новой
//! загрузки высот.
//!
//! Узел в тени, если солнце ниже горизонта по своему азимуту или падает
//! на поверхность с обратной стороны. Солнце ниже горизонта места - тень
//! везде. Направление на солнце одно для всей сетки, по середине.
//! Рефракция не учитывается. Крыш зданий у Lite в расчёте нет.

use std::sync::Arc;

use crate::insolation::{self, Work};
use crate::viewshed::{Heights, distance_bearing};

/// Значения растра: солнце, тень, вне участка.
pub const SUN: u8 = 1;
pub const SHADE: u8 = 0;
pub const OUTSIDE: u8 = 255;
/// Цвета карты: тень - тёмная синеватая, солнце - лёгкая тёплая дымка.
pub const SHADE_RGBA: [u8; 4] = [18, 22, 46, 150];
pub const SUN_RGBA: [u8; 4] = [255, 214, 120, 30];

/// Горизонт и нормали внутренней сетки участка: n × n узлов, строки
/// с юга на север, центр сетки - точка (lat, lon).
#[derive(Debug, Clone, PartialEq)]
pub struct Horizon {
    pub lat: f64,
    pub lon: f64,
    pub radius_m: f64,
    /// Шаг сетки, метры.
    pub cell: f64,
    pub n: usize,
    /// Тангенс угла горизонта: азимуты × узлы.
    pub hor: Vec<f32>,
    /// Нормали узлов: восток, север, верх.
    pub normals: Vec<[f64; 3]>,
    /// Радиус тела.
    pub radius: f64,
    /// Участок многоугольником (широта, долгота): вне него карты нет.
    pub ring: Option<Vec<(f64, f64)>>,
    pub dlat: f64,
    pub dlon: f64,
}

impl Horizon {
    /// Горизонт из конченного расчёта work ([`Work::horizon`]).
    pub fn from_work(work: Work, lat: f64, lon: f64, radius_m: f64, radius: f64, ring: Option<Vec<(f64, f64)>>) -> Option<Self> {
        let (hor, normals, n, cell) = work.into_horizon()?;
        Some(Self { lat, lon, radius_m, cell, n, hor, normals, radius, ring, dlat: (cell / radius).to_degrees(), dlon: (cell / (radius * lat.to_radians().cos())).to_degrees() })
    }

    /// Карта на момент moment, секунды UTC.
    pub fn at(self: &Arc<Self>, moment: f64) -> Shadows {
        let sun = insolation::sun_enu(self.lat, self.lon, &[moment]).first().copied().unwrap_or([0.0, 0.0, -1.0]);
        let elevation = sun[2].atan2(sun[0].hypot(sun[1])).to_degrees();
        Shadows { horizon: self.clone(), moment, lit: lit_map(&self.hor, &self.normals, sun), elevation }
    }
}

/// Освещённые узлы при направлении на солнце sun (восток, север, верх).
/// hor - тангенсы горизонта азимуты × узлы, между азимутами - линейно.
pub fn lit_map(hor: &[f32], normals: &[[f64; 3]], sun: [f64; 3]) -> Vec<bool> {
    let size = normals.len();
    let [e, n, u] = sun;
    if u <= 0.0 || size == 0 {
        return vec![false; size];
    }
    let count = hor.len() / size;
    if count == 0 {
        return vec![false; size];
    }
    let level = e.hypot(n);
    let tan_el = if level > 0.0 { u / level } else { f64::INFINITY };
    let k = (e.atan2(n) / std::f64::consts::TAU * count as f64).rem_euclid(count as f64);
    let k0 = (k.floor() as usize) % count;
    let k1 = (k0 + 1) % count;
    let f = k - k.floor();
    let (Some(a), Some(b)) = (hor.get(k0 * size..(k0 + 1) * size), hor.get(k1 * size..(k1 + 1) * size)) else {
        return vec![false; size];
    };
    normals
        .iter()
        .enumerate()
        .map(|(i, nrm)| {
            let (ha, hb) = (a.get(i).copied().unwrap_or(f32::INFINITY), b.get(i).copied().unwrap_or(f32::INFINITY));
            let line = f64::from(ha) * (1.0 - f) + f64::from(hb) * f;
            line < tan_el && nrm[0] * e + nrm[1] * n + nrm[2] * u > 0.0
        })
        .collect()
}

/// Тени на момент: lit - солнце в узлах, строки с юга на север,
/// elevation - высота солнца над горизонтом места, градусы.
#[derive(Debug, Clone, PartialEq)]
pub struct Shadows {
    pub horizon: Arc<Horizon>,
    pub moment: f64,
    pub lit: Vec<bool>,
    pub elevation: f64,
}

impl Shadows {
    /// Солнце в точках (широта, долгота) и признак «внутри участка».
    pub fn at(&self, points: &[(f64, f64)], radius: f64) -> Vec<(bool, bool)> {
        let h = &self.horizon;
        let n = h.n as i64;
        let middle = (h.n as f64 - 1.0) / 2.0;
        let in_ring = h.ring.as_ref().map(|r| crate::site::inside(points, r));
        points
            .iter()
            .enumerate()
            .map(|(i, &(lat, lon))| {
                let row = ((lat - h.lat) / h.dlat + middle).round_ties_even() as i64;
                let col = ((lon - h.lon) / h.dlon + middle).round_ties_even() as i64;
                let (dist, _) = distance_bearing(h.lat, h.lon, lat, lon, radius);
                let ringed = in_ring.as_ref().is_none_or(|r| r.get(i).copied().unwrap_or(false));
                let inside = dist <= h.radius_m && ringed && (0..n).contains(&row) && (0..n).contains(&col);
                let lit = inside && self.lit.get((row * n + col) as usize).copied().unwrap_or(false);
                (lit, inside)
            })
            .collect()
    }

    /// Сетка широт и долгот для GeoTIFF, `raster` модуля: строки с севера,
    /// [`SUN`], [`SHADE`], вне участка [`OUTSIDE`].
    pub fn raster(&self) -> (crate::geotiff::LatLonGrid, Vec<u8>) {
        let h = &self.horizon;
        let half = (h.n as f64 - 1.0) / 2.0;
        let grid = crate::geotiff::LatLonGrid { north: h.lat + half * h.dlat, west: h.lon - half * h.dlon, dlat: h.dlat, dlon: h.dlon, cols: h.n, rows: h.n };
        let points: Vec<(f64, f64)> = (0..h.n).flat_map(|r| (0..h.n).map(move |c| (grid.north - r as f64 * grid.dlat, grid.west + c as f64 * grid.dlon))).collect();
        let values = self.at(&points, h.radius).into_iter().map(|(lit, inside)| if !inside { OUTSIDE } else if lit { SUN } else { SHADE }).collect();
        (grid, values)
    }

    /// Доля участка на солнце, 0-1.
    pub fn share(&self) -> f64 {
        let (_, values) = self.raster();
        let inside = values.iter().filter(|v| **v != OUTSIDE).count();
        if inside == 0 { 0.0 } else { values.iter().filter(|v| **v == SUN).count() as f64 / inside as f64 }
    }
}

/// Цвет с премноженной альфой.
fn premultiplied(rgba: [u8; 4]) -> [u8; 4] {
    let a = f64::from(rgba[3]) / 255.0;
    let c = |v: u8| (f64::from(v) * a).round() as u8;
    [c(rgba[0]), c(rgba[1]), c(rgba[2]), rgba[3]]
}

/// Картинка в точках: тень [`SHADE_RGBA`], солнце [`SUN_RGBA`], вне
/// участка прозрачно. RGBA с премноженной альфой.
pub fn tile_rgba(result: &Shadows, points: &[(f64, f64)], radius: f64) -> Vec<[u8; 4]> {
    result
        .at(points, radius)
        .into_iter()
        .map(|(lit, inside)| match (inside, lit) {
            (false, _) => [0; 4],
            (true, true) => premultiplied(SUN_RGBA),
            (true, false) => premultiplied(SHADE_RGBA),
        })
        .collect()
}

/// Горизонт участка целиком: круг radius_m вокруг точки, heights -
/// настоящие высоты точек, radius - радиус тела, ring - участок.
pub fn compute(lat: f64, lon: f64, radius_m: f64, heights: Heights, radius: f64, ring: Option<Vec<(f64, f64)>>) -> Option<Arc<Horizon>> {
    let values = heights(&insolation::grid_points(lat, lon, radius_m, radius));
    let mut work = Work::horizon(lat, lon, radius_m, &values, radius)?;
    while work.step() != insolation::Step::Done {}
    Horizon::from_work(work, lat, lon, radius_m, radius, ring).map(Arc::new)
}

