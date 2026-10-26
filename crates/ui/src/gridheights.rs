// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Высоты узлов квадрата у точки для окон «Уклон и экспозиция»,
//! «Затопление» и «Сток воды», `GridHeights` из `ui/hydro.py` модуля.
//!
//! Сетка - `data::hydro::Grid`: узлы Web Mercator с равным шагом
//! на местности. «Построить» сначала просит тайлы высот под квадратом
//! и ждёт их не дольше [`crate::pointcalc::WAIT`] секунд, потом снимает
//! высоты узлов частями в бюджете кадра: у страницы WebAssembly потоков
//! нет. Высоты - настоящие, без масштаба рельефа, с дном моря, как
//! у видимости и инсоляции.

use data::hydro::Grid;
use globe_core::lod::TileKey;

use crate::i18n::{Lang, tr, trf};

/// Узлов за проход снятия высот.
const CHUNK: usize = 20000;
/// Время снятия высот за кадр, секунды.
const BUDGET: f64 = 0.008;

/// Ход снятия высот.
pub enum Step {
    /// Ещё идёт: текст для строки окна.
    Going(String),
    /// Высоты всех узлов по строкам, NaN - высоты нет.
    Done(Vec<f64>),
    /// У тела нет высот.
    Failed(String),
}

/// Снятие высот узлов сетки.
pub struct GridHeights {
    pub grid: Grid,
    /// Радиус тела, метры.
    pub radius: f64,
    keys: Vec<TileKey>,
    started: f64,
    /// Ждали тайлы дольше срока: расчёт идёт по менее подробным высотам.
    pub partial: bool,
    points: Vec<(f64, f64)>,
    got: Vec<f64>,
    waiting: bool,
}

/// Широта и долгота узла (строка, столбец) сетки.
pub fn node(grid: &Grid, row: f64, col: f64, radius: f64) -> (f64, f64) {
    let x = grid.x0 + col * grid.step;
    let y = grid.y0 - row * grid.step;
    let lat = (2.0 * (y / radius).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees();
    (lat, (x / radius).to_degrees())
}

impl GridHeights {
    /// Квадрат со стороной 2 radius_m у точки. max_level - самый
    /// подробный уровень тайлов высот тела.
    pub fn new(lat: f64, lon: f64, radius_m: f64, radius: f64, max_level: u32, now: f64) -> Self {
        // Шаг не мельче пикселя высот самого подробного уровня.
        let pixel = 2.0 * std::f64::consts::PI * radius * lat.to_radians().cos().abs().max(1e-6) / (256.0 * f64::from(1u32 << max_level.min(30)));
        let grid = data::hydro::make_grid(lat, lon, radius_m, pixel, radius);
        // Тайлы под кругом, описанным около квадрата.
        let (_, keys) = data::viewshed::height_tiles(lat, lon, radius_m * std::f64::consts::SQRT_2, grid.cell, radius, max_level);
        Self { grid, radius, keys, started: now, partial: false, points: Vec::new(), got: Vec::new(), waiting: true }
    }

    /// Тайлы высот, которых ещё ждёт расчёт.
    pub fn wanted(&self, has: &dyn Fn(TileKey) -> bool) -> Vec<TileKey> {
        if self.waiting { self.keys.iter().copied().filter(|k| !has(*k)).collect() } else { Vec::new() }
    }

    /// Шаг: ожидание тайлов, потом высоты узлов частями.
    pub fn step(&mut self, lang: Lang, now: f64, has: &dyn Fn(TileKey) -> bool, heights: data::viewshed::Heights) -> Step {
        if self.waiting {
            let missing = self.keys.iter().filter(|k| !has(**k)).count();
            if missing > 0 && now - self.started < crate::pointcalc::WAIT {
                let done = (self.keys.len() - missing).to_string();
                return Step::Going(trf(lang, "Loading heights: {done} of {total}", &[("done", &done), ("total", &self.keys.len().to_string())]));
            }
            self.partial = missing > 0;
            self.waiting = false;
            let (rows, cols) = (self.grid.rows, self.grid.cols);
            self.points = (0..rows).flat_map(|r| (0..cols).map(move |c| (r, c))).map(|(r, c)| node(&self.grid, r as f64, c as f64, self.radius)).collect();
            self.got = Vec::with_capacity(self.points.len());
        }
        let started = web_time::Instant::now();
        while self.got.len() < self.points.len() && started.elapsed().as_secs_f64() < BUDGET {
            let end = (self.got.len() + CHUNK).min(self.points.len());
            let part = self.points.get(self.got.len()..end).unwrap_or(&[]);
            let found = heights(part);
            if found.len() < part.len() {
                return Step::Failed(tr(lang, "This body has no heights.").to_string());
            }
            self.got.extend(found);
        }
        if self.got.len() < self.points.len() {
            let share = format!("{:.0}%", 100.0 * self.got.len() as f64 / self.points.len().max(1) as f64);
            return Step::Going(trf(lang, "Heights of the grid nodes: {share}", &[("share", &share)]));
        }
        Step::Done(std::mem::take(&mut self.got))
    }
}

/// Картинки тайлов квадрата: центры пикселей тайла key в номерах
/// узлов сетки (строка, столбец, дробные), строки сверху вниз.
pub fn tile_nodes(grid: &Grid, key: TileKey, radius: f64) -> Vec<(f64, f64)> {
    let (z, x, y) = key;
    let n = f64::from(1u32 << z.min(30));
    let world = 2.0 * std::f64::consts::PI * radius;
    let mut out = Vec::with_capacity(256 * 256);
    for r in 0..256 {
        let my = world * (0.5 - (f64::from(y) + (r as f64 + 0.5) / 256.0) / n);
        for c in 0..256 {
            let mx = world * ((f64::from(x) + (c as f64 + 0.5) / 256.0) / n - 0.5);
            out.push(((grid.y0 - my) / grid.step, (mx - grid.x0) / grid.step));
        }
    }
    out
}

/// Тайл key задевает квадрат сетки.
pub fn touches(grid: &Grid, key: TileKey, radius: f64) -> bool {
    let (z, x, y) = key;
    let n = f64::from(1u32 << z.min(30));
    let world = 2.0 * std::f64::consts::PI * radius;
    let (west, east) = (world * (f64::from(x) / n - 0.5), world * ((f64::from(x) + 1.0) / n - 0.5));
    let (north, south) = (world * (0.5 - f64::from(y) / n), world * (0.5 - (f64::from(y) + 1.0) / n));
    let (gx1, gy1) = (grid.x0 + grid.step * (grid.cols as f64 - 1.0), grid.y0 - grid.step * (grid.rows as f64 - 1.0));
    east >= grid.x0 && west <= gx1 && north >= gy1 && south <= grid.y0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_and_tiles_agree() {
        let r = 6378137.0;
        let grid = data::hydro::make_grid(58.0, 56.25, 2000.0, 5.0, r);
        let (lat, lon) = node(&grid, (grid.rows as f64 - 1.0) / 2.0, (grid.cols as f64 - 1.0) / 2.0, r);
        assert!((lat - 58.0).abs() < 1e-6 && (lon - 56.25).abs() < 1e-6);
        // Тайл уровня 14 с серединой квадрата задевает его, тайл мира 0/0/0 - тоже.
        let n = f64::from(1u32 << 14);
        let tx = ((56.25 + 180.0) / 360.0 * n) as u32;
        let ty = ((1.0 - (58f64.to_radians().tan() + 1.0 / 58f64.to_radians().cos()).ln() / std::f64::consts::PI) / 2.0 * n) as u32;
        assert!(touches(&grid, (14, tx, ty), r) && touches(&grid, (0, 0, 0), r) && !touches(&grid, (14, 0, 0), r));
        let pts = tile_nodes(&grid, (14, tx, ty), r);
        assert_eq!(pts.len(), 256 * 256);
    }
}
