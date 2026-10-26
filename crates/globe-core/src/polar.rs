// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Полярные шапки снимком, перенос `core/polar.py` модуля.
//!
//! Web Mercator кончается на широте [`crate::tiling::MAX_LAT`], выше
//! глобус рисует шапку. Решение автора модуля от 23 октября 2026 года -
//! шапки снимками полярных служб Esri: Arctic Imagery (EPSG:5936,
//! стереографическая проекция с центром на долготе -150°, масштаб
//! 0.994, сдвиг 2 000 000 м) и Antarctic Imagery (EPSG:3031,
//! стандартная параллель -71°). Обе - снимки Earthstar Geographics на
//! условиях Esri, как World Imagery.
//!
//! Шапка - одна картинка на полюс: мозаика тайлов уровня [`LEVEL`] под
//! квадратом вокруг полюса ([`Layout`], [`Layout::mosaic`]), вершины
//! сетки шапки получают координаты в этой мозаике ([`Layout::uv`]).
//! Формулы - Snyder, «Map Projections - A Working Manual», 1987,
//! полярная стереографическая на эллипсоиде. Сверка с GDAL - тесты.
//! Цвет мозаики подгоняется к подложке у края шапки по секторам
//! долготы ([`matched`]): снимок Arctic Imagery темнее и синее World
//! Imagery, без подгонки шапка выделялась тёмным кругом.

use std::f64::consts::FRAC_PI_4;

const A: f64 = 6_378_137.0;
const F: f64 = 1.0 / 298.257_223_563;
/// Широта края шапки, как у [`crate::tiling::MAX_LAT`].
pub const MAX_LAT: f64 = 85.051_128_779_806_6;
/// Метров на пиксель уровня 0 обеих служб.
pub const RES0: f64 = 238_810.813_354;
/// Сторона тайла, пикселей.
pub const SIDE: usize = 256;
/// Уровень 8 - 933 м на пиксель, 36 тайлов на полюс. Шапку видно
/// издалека, у самого полюса снимок размыт - выбор помощника модуля.
pub const LEVEL: u32 = 8;
/// Запас квадрата мозаики за краем шапки.
pub const MARGIN: f64 = 1.02;
/// Адрес тайлов, {name} - служба.
pub const URL: &str = "https://server.arcgisonline.com/ArcGIS/rest/services/Polar/{name}/MapServer/tile/{z}/{y}/{x}";
pub const CREDIT: &str = "Earthstar Geographics, Esri";
/// Широта кольца у края шапки, где берётся цвет мозаики для подгонки.
pub const RIM_LAT: f64 = 84.9;
/// Пределы множителя канала при подгонке.
pub const GAIN_RANGE: (f64, f64) = (0.5, 2.0);
/// Наибольший сдвиг канала после множителя.
pub const SHIFT_MAX: f64 = 48.0;
/// Секторов долготы подгонки - как тайлов уровня 3 у края.
pub const SECTORS: usize = 8;
/// Серый пустого тайла мозаики, как заглушка снимка.
pub const EMPTY: u8 = 40;

/// Служба полюса: имя, начало сетки тайлов, центральная долгота,
/// масштаб или стандартная параллель, сдвиг.
struct Service {
    name: &'static str,
    origin: (f64, f64),
    lon0: f64,
    k0: Option<f64>,
    lat_ts: f64,
    shift: (f64, f64),
}

fn service(north: bool) -> Service {
    if north {
        Service { name: "Arctic_Imagery", origin: (-28_567_784.109_255, 32_567_784.109_255), lon0: -150.0, k0: Some(0.994), lat_ts: 0.0, shift: (2_000_000.0, 2_000_000.0) }
    } else {
        Service { name: "Antarctic_Imagery", origin: (-33_699_550.992_03, 33_699_551.017_03), lon0: 0.0, k0: None, lat_ts: -71.0, shift: (0.0, 0.0) }
    }
}

fn ecc() -> f64 {
    (F * (2.0 - F)).sqrt()
}

/// t Снайдера (15-9) для широты phi в радианах, полюс - север.
fn t_of(phi: f64) -> f64 {
    let e = ecc();
    let s = phi.sin();
    (FRAC_PI_4 - phi / 2.0).tan() / ((1.0 - e * s) / (1.0 + e * s)).powf(e / 2.0)
}

/// Широта и долгота в метрах проекции службы полюса north.
pub fn project(lat: f64, lon: f64, north: bool) -> (f64, f64) {
    let srv = service(north);
    let e = ecc();
    let sign = if north { 1.0 } else { -1.0 };
    let t = t_of((sign * lat).to_radians());
    let rho = match srv.k0 {
        Some(k0) => 2.0 * A * k0 * t / ((1.0 + e).powf(1.0 + e) * (1.0 - e).powf(1.0 - e)).sqrt(),
        None => {
            let phc = srv.lat_ts.abs().to_radians();
            let mc = phc.cos() / (1.0 - (e * phc.sin()).powi(2)).sqrt();
            A * mc * t / t_of(phc)
        }
    };
    let turn = (lon - srv.lon0).to_radians();
    (srv.shift.0 + rho * turn.sin(), srv.shift.1 - sign * rho * turn.cos())
}

/// Мозаика шапки: тайлы col0..=col1, row0..=row1 уровня level, левый
/// верхний угол (x0, y0) и стороны в метрах.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub north: bool,
    pub level: u32,
    pub col0: i64,
    pub col1: i64,
    pub row0: i64,
    pub row1: i64,
    pub x0: f64,
    pub y0: f64,
    pub width: f64,
    pub height: f64,
}

impl Layout {
    pub fn new(north: bool, level: u32) -> Self {
        let srv = service(north);
        let sign = if north { 1.0 } else { -1.0 };
        let (cx, cy) = project(sign * 90.0, 0.0, north);
        let (ex, ey) = project(sign * MAX_LAT, 0.0, north);
        let half = (ex - cx).hypot(ey - cy) * MARGIN;
        let span = RES0 / f64::from(1u32 << level.min(30)) * SIDE as f64;
        let (ox, oy) = srv.origin;
        let col0 = ((cx - half - ox) / span).floor() as i64;
        let col1 = ((cx + half - ox) / span).floor() as i64;
        let row0 = ((oy - (cy + half)) / span).floor() as i64;
        let row1 = ((oy - (cy - half)) / span).floor() as i64;
        Self {
            north,
            level,
            col0,
            col1,
            row0,
            row1,
            x0: ox + col0 as f64 * span,
            y0: oy - row0 as f64 * span,
            width: (col1 - col0 + 1) as f64 * span,
            height: (row1 - row0 + 1) as f64 * span,
        }
    }

    /// Тайлов по столбцам и строкам.
    pub fn size(&self) -> (usize, usize) {
        ((self.col1 - self.col0 + 1).max(0) as usize, (self.row1 - self.row0 + 1).max(0) as usize)
    }

    /// (ряд, столбец, адрес) тайлов мозаики.
    pub fn tiles(&self) -> Vec<(i64, i64, String)> {
        let name = service(self.north).name;
        let mut out = Vec::new();
        for row in self.row0..=self.row1 {
            for col in self.col0..=self.col1 {
                let url = URL.replace("{name}", name).replace("{z}", &self.level.to_string()).replace("{y}", &row.to_string()).replace("{x}", &col.to_string());
                out.push((row, col, url));
            }
        }
        out
    }

    /// Координаты в мозаике, v = 0 у верхнего края.
    pub fn uv(&self, lat: f64, lon: f64) -> (f64, f64) {
        let (x, y) = project(lat, lon, self.north);
        ((x - self.x0) / self.width, (self.y0 - y) / self.height)
    }

    /// Картинка RGBA из тайлов (ряд, столбец, RGBA 256 × 256). Тайла нет
    /// или он не того размера - тёмно-серый [`EMPTY`].
    pub fn mosaic(&self, tiles: &[(i64, i64, Vec<u8>)]) -> (usize, usize, Vec<u8>) {
        let (cols, rows) = self.size();
        let (w, h) = (cols * SIDE, rows * SIDE);
        let mut out = [EMPTY, EMPTY, EMPTY, 255].repeat(w * h);
        for (row, col, rgba) in tiles {
            if rgba.len() != SIDE * SIDE * 4 || *row < self.row0 || *row > self.row1 || *col < self.col0 || *col > self.col1 {
                continue;
            }
            let (r0, c0) = ((*row - self.row0) as usize * SIDE, (*col - self.col0) as usize * SIDE);
            for y in 0..SIDE {
                let dst = ((r0 + y) * w + c0) * 4;
                out[dst..dst + SIDE * 4].copy_from_slice(&rgba[y * SIDE * 4..(y + 1) * SIDE * 4]);
            }
        }
        (w, h, out)
    }

    /// Долгота и доля радиуса края шапки у точки (x, y) проекции.
    fn lon_share(&self, x: f64, y: f64) -> (f64, f64) {
        let srv = service(self.north);
        let (dx, dy) = (x - srv.shift.0, y - srv.shift.1);
        let turn = if self.north { dx.atan2(-dy) } else { dx.atan2(dy) };
        let lon = (turn.to_degrees() + srv.lon0 + 180.0).rem_euclid(360.0) - 180.0;
        let sign = if self.north { 1.0 } else { -1.0 };
        let (ex, ey) = project(sign * MAX_LAT, 0.0, self.north);
        let rim = (ex - srv.shift.0).hypot(ey - srv.shift.1);
        (lon, dx.hypot(dy) / rim)
    }
}

/// Полюса, шапку которых видно, `needed` модуля: глаз выше 2000 км
/// видит оба, ближе - тот, к которому ближе широта точки взгляда.
pub fn needed(lat: f64, altitude: f64) -> Vec<bool> {
    if altitude > 2.0e6 {
        vec![true, false]
    } else if lat > 55.0 {
        vec![true]
    } else if lat < -55.0 {
        vec![false]
    } else {
        Vec::new()
    }
}

/// Средние цвета RGB мозаики w × h на кольце [`RIM_LAT`] по секторам
/// долготы от -180°.
pub fn rim_color(layout: &Layout, w: usize, h: usize, rgba: &[u8]) -> Vec<[f64; 3]> {
    let count = 3600;
    let lat = if layout.north { RIM_LAT } else { -RIM_LAT };
    let per = count / SECTORS;
    let mut out = vec![[0.0; 3]; SECTORS];
    for (k, slot) in out.iter_mut().enumerate() {
        let mut sum = [0.0; 3];
        for i in 0..per {
            let lon = (k * per + i) as f64 * 360.0 / count as f64 - 180.0;
            let (u, v) = layout.uv(lat, lon);
            let c = ((u * w as f64) as usize).min(w.saturating_sub(1));
            let r = ((v * h as f64) as usize).min(h.saturating_sub(1));
            if let Some(px) = rgba.get((r * w + c) * 4..(r * w + c) * 4 + 3) {
                for j in 0..3 {
                    sum[j] += f64::from(px[j]);
                }
            }
        }
        *slot = sum.map(|s| s / per as f64);
    }
    out
}

/// Мозаика с цветом have, приведённым к want по секторам долготы:
/// множитель канала и сдвиг остатка. Между серединами секторов они
/// плавно переходят, к полюсу - к среднему по всем секторам.
pub fn matched(rgba: &mut [u8], w: usize, h: usize, layout: &Layout, have: &[[f64; 3]], want: &[[f64; 3]]) {
    let count = have.len().min(want.len());
    if count == 0 || rgba.len() != w * h * 4 {
        return;
    }
    let mut gains = vec![[1.0; 3]; count];
    let mut shifts = vec![[0.0; 3]; count];
    for k in 0..count {
        for j in 0..3 {
            let g = (want[k][j] / have[k][j].max(1.0)).clamp(GAIN_RANGE.0, GAIN_RANGE.1);
            gains[k][j] = g;
            shifts[k][j] = (want[k][j] - have[k][j] * g).clamp(-SHIFT_MAX, SHIFT_MAX);
        }
    }
    let mean = |v: &[[f64; 3]]| -> [f64; 3] { [0, 1, 2].map(|j| v.iter().map(|x| x[j]).sum::<f64>() / v.len() as f64) };
    let (gain_mean, shift_mean) = (mean(&gains), mean(&shifts));
    for r in 0..h {
        let y = layout.y0 - (r as f64 + 0.5) / h as f64 * layout.height;
        for c in 0..w {
            let x = layout.x0 + (c as f64 + 0.5) / w as f64 * layout.width;
            let (lon, share) = layout.lon_share(x, y);
            let pos = (lon + 180.0) / 360.0 * count as f64 - 0.5;
            let low = pos.floor();
            let t = pos - low;
            let i0 = (low as i64).rem_euclid(count as i64) as usize;
            let i1 = (i0 + 1) % count;
            let s = share.clamp(0.0, 1.0);
            let px = &mut rgba[(r * w + c) * 4..(r * w + c) * 4 + 3];
            for j in 0..3 {
                let near = |v: &[[f64; 3]]| v[i0][j] * (1.0 - t) + v[i1][j] * t;
                let g = near(&gains) * s + gain_mean[j] * (1.0 - s);
                let d = near(&shifts) * s + shift_mean[j] * (1.0 - s);
                px[j] = (f64::from(px[j]) * g + d + 0.5).clamp(0.0, 255.0) as u8;
            }
        }
    }
}
