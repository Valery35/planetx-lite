// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Наборы многоугольников в картинке наложения тайла - контуры горных
//! разработок, как слой QGIS с заливкой и обводом в стопке наложения
//! модуля, и многоугольник под точкой, как `_identify_mining` модуля.
//!
//! Кольца многоугольника заливаются по правилу чётности: дыры и части
//! мультимногоугольника - кольцами одного объекта. Многоугольник мельче
//! пикселя тайла рисуется квадратом цвета обвода, иначе мелкие контуры
//! исчезали бы на видах издалека.

use std::collections::HashMap;

/// Многоугольник: кольца (широта, долгота), рамка (юг, запад, север,
/// восток).
#[derive(Debug, Clone, PartialEq)]
pub struct Area {
    pub rings: Vec<Vec<(f64, f64)>>,
    pub bbox: (f64, f64, f64, f64),
}

impl Area {
    pub fn new(rings: Vec<Vec<(f64, f64)>>) -> Area {
        let (mut s, mut w, mut n, mut e) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for (lat, lon) in rings.iter().flatten() {
            s = s.min(*lat);
            n = n.max(*lat);
            w = w.min(*lon);
            e = e.max(*lon);
        }
        Area { rings, bbox: (s, w, n, e) }
    }

    /// Точка внутри по правилу чётности по всем кольцам.
    pub fn contains(&self, lat: f64, lon: f64) -> bool {
        let (s, w, n, e) = self.bbox;
        if lat < s || lat > n || lon < w || lon > e {
            return false;
        }
        let mut inside = false;
        for ring in &self.rings {
            let count = ring.len();
            for i in 0..count {
                let (Some(a), Some(b)) = (ring.get(i), ring.get((i + count - 1) % count)) else {
                    continue;
                };
                if (a.0 > lat) != (b.0 > lat) && lon < (b.1 - a.1) * (lat - a.0) / (b.0 - a.0) + a.1 {
                    inside = !inside;
                }
            }
        }
        inside
    }
}

/// Сторона клетки индекса, градусы.
const CELL: f64 = 1.0;

/// Набор многоугольников с индексом по клеткам в градус.
#[derive(Debug, Clone, Default)]
pub struct Areas {
    pub items: Vec<Area>,
    grid: HashMap<(i32, i32), Vec<usize>>,
}

fn cell(v: f64) -> i32 {
    (v / CELL).floor() as i32
}

impl Areas {
    pub fn new(items: Vec<Area>) -> Areas {
        let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (n, a) in items.iter().enumerate() {
            let (s, w, nn, e) = a.bbox;
            if !(s.is_finite() && w.is_finite() && nn.is_finite() && e.is_finite()) {
                continue;
            }
            for y in cell(s)..=cell(nn) {
                for x in cell(w)..=cell(e) {
                    grid.entry((y, x)).or_default().push(n);
                }
            }
        }
        Areas { items, grid }
    }

    /// Номера многоугольников, чья рамка пересекает рамку (юг, запад,
    /// север, восток), без повторов. Рамка во весь мир - все.
    pub fn within(&self, s: f64, w: f64, n: f64, e: f64) -> Vec<usize> {
        let cells = (f64::from(cell(n) - cell(s) + 1)) * (f64::from(cell(e) - cell(w) + 1));
        let overlap = |a: &Area| a.bbox.0 <= n && a.bbox.2 >= s && a.bbox.1 <= e && a.bbox.3 >= w;
        if cells > self.grid.len() as f64 {
            return (0..self.items.len()).filter(|i| self.items.get(*i).is_some_and(overlap)).collect();
        }
        let mut out: Vec<usize> = Vec::new();
        for y in cell(s)..=cell(n) {
            for x in cell(w)..=cell(e) {
                if let Some(list) = self.grid.get(&(y, x)) {
                    out.extend(list.iter().copied().filter(|i| self.items.get(*i).is_some_and(overlap)));
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Многоугольники под точкой.
    pub fn at(&self, lat: f64, lon: f64) -> Vec<usize> {
        self.within(lat, lon, lat, lon).into_iter().filter(|i| self.items.get(*i).is_some_and(|a| a.contains(lat, lon))).collect()
    }
}

/// Сторона картинки тайла.
pub const SIZE: u32 = 256;

/// Место точки на картинке тайла (z, x, y) в пикселях.
fn pixel(z: u32, x: u32, y: u32, lat: f64, lon: f64) -> (f32, f32) {
    let world = f64::from(SIZE) * 2f64.powi(z.min(30) as i32);
    let lat = lat.clamp(-85.0511, 85.0511).to_radians();
    let gx = (lon + 180.0) / 360.0 * world;
    let gy = (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * world;
    ((gx - f64::from(x) * f64::from(SIZE)) as f32, (gy - f64::from(y) * f64::from(SIZE)) as f32)
}

/// Рамка тайла (юг, запад, север, восток), градусы.
fn tile_box(z: u32, x: u32, y: u32) -> (f64, f64, f64, f64) {
    let n = 2f64.powi(z.min(30) as i32);
    let lon = |x: f64| x / n * 360.0 - 180.0;
    let lat = |y: f64| (std::f64::consts::PI * (1.0 - 2.0 * y / n)).sinh().atan().to_degrees();
    (lat(f64::from(y) + 1.0), lon(f64::from(x)), lat(f64::from(y)), lon(f64::from(x) + 1.0))
}

/// Многоугольники на картинке тайла key: заливка fill и обвод line
/// толщиной width пикселей, RGBA с премноженной альфой. Пустая
/// картинка - None.
pub fn draw(key: (u32, u32, u32), areas: &Areas, fill: [u8; 4], line: [u8; 4], width: f32) -> Option<Vec<u8>> {
    let (z, x, y) = key;
    let (s, w, n, e) = tile_box(z, x, y);
    let (dlat, dlon) = ((n - s) * 0.02, (e - w) * 0.02);
    let found = areas.within(s - dlat, w - dlon, n + dlat, e + dlon);
    if found.is_empty() {
        return None;
    }
    let mut canvas = tiny_skia::Pixmap::new(SIZE, SIZE)?;
    let mut fill_paint = tiny_skia::Paint::default();
    fill_paint.set_color_rgba8(fill[0], fill[1], fill[2], fill[3]);
    fill_paint.anti_alias = true;
    let mut line_paint = tiny_skia::Paint::default();
    line_paint.set_color_rgba8(line[0], line[1], line[2], line[3]);
    line_paint.anti_alias = true;
    let stroke = tiny_skia::Stroke { width, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
    for i in found {
        let Some(area) = areas.items.get(i) else {
            continue;
        };
        let (a, b) = (pixel(z, x, y, area.bbox.2, area.bbox.1), pixel(z, x, y, area.bbox.0, area.bbox.3));
        if (b.0 - a.0).abs() < 1.5 && (b.1 - a.1).abs() < 1.5 {
            // Мельче пикселя - квадрат цвета обвода.
            let c = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
            if let Some(rect) = tiny_skia::Rect::from_xywh(c.0 - 0.75, c.1 - 0.75, 1.5, 1.5) {
                canvas.fill_rect(rect, &line_paint, tiny_skia::Transform::identity(), None);
            }
            continue;
        }
        let mut path = tiny_skia::PathBuilder::new();
        for ring in &area.rings {
            for (k, (lat, lon)) in ring.iter().enumerate() {
                let (px, py) = pixel(z, x, y, *lat, *lon);
                if k == 0 {
                    path.move_to(px, py);
                } else {
                    path.line_to(px, py);
                }
            }
            path.close();
        }
        let Some(path) = path.finish() else {
            continue;
        };
        canvas.fill_path(&path, &fill_paint, tiny_skia::FillRule::EvenOdd, tiny_skia::Transform::identity(), None);
        canvas.stroke_path(&path, &line_paint, &stroke, tiny_skia::Transform::identity(), None);
    }
    let data = canvas.take();
    data.as_chunks::<4>().0.iter().any(|p| p[3] > 0).then_some(data)
}
