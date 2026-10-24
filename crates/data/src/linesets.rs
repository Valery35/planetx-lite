// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Наборы линий слоёв «Недр» и «Энергетики» - разломы, границы плит,
//! кабели - в картинке наложения тайла, как слои в памяти QGIS в стопке
//! наложения модуля (`net.overlay`), и ближайшая к точке линия, как
//! `core/pick.py` модуля.

/// Линия набора: точки (широта, долгота), цвет RGBA, толщина в пикселях
/// картинки тайла, рамка (юг, запад, север, восток).
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub points: Vec<(f64, f64)>,
    pub color: [u8; 4],
    pub width: f32,
    pub bbox: (f64, f64, f64, f64),
}

impl Line {
    pub fn new(points: Vec<(f64, f64)>, color: [u8; 4], width: f32) -> Line {
        let (mut s, mut w, mut n, mut e) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for (lat, lon) in &points {
            s = s.min(*lat);
            n = n.max(*lat);
            w = w.min(*lon);
            e = e.max(*lon);
        }
        Line { points, color, width, bbox: (s, w, n, e) }
    }
}

/// Средний радиус Земли для расстояний у точки, метры.
const RADIUS: f64 = 6371008.8;

/// Расстояние от точки (lat, lon) до ломаной, метры: в касательной
/// плоскости у точки, как `pick.distance` модуля.
pub fn distance(points: &[(f64, f64)], lat: f64, lon: f64) -> f64 {
    let local = |(la, lo): (f64, f64)| {
        let dlon = (lo - lon + 180.0).rem_euclid(360.0) - 180.0;
        (dlon.to_radians() * RADIUS * lat.to_radians().cos(), (la - lat).to_radians() * RADIUS)
    };
    let xy: Vec<(f64, f64)> = points.iter().copied().map(local).collect();
    if xy.len() == 1 {
        return xy[0].0.hypot(xy[0].1);
    }
    let mut best = f64::INFINITY;
    for pair in xy.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let d = (b.0 - a.0, b.1 - a.1);
        let length2 = d.0 * d.0 + d.1 * d.1;
        let t = if length2 > 0.0 { ((-a.0 * d.0 - a.1 * d.1) / length2).clamp(0.0, 1.0) } else { 0.0 };
        best = best.min((a.0 + t * d.0).hypot(a.1 + t * d.1));
    }
    best
}

/// Номер ближайшей к точке линии ближе metres или None. Отбор по рамкам
/// с запасом допуска, потом расстояние до отрезков, как `nearest`
/// разломов и кабелей модуля.
pub fn nearest(lines: &[Line], lat: f64, lon: f64, metres: f64) -> Option<usize> {
    let reach = (metres / 6371000.0).to_degrees();
    let reach_lon = reach / lat.to_radians().cos().max(0.01);
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| {
            let (s, w, n, e) = l.bbox;
            s - reach <= lat && lat <= n + reach && w - reach_lon <= lon && lon <= e + reach_lon
        })
        .map(|(i, l)| (distance(&l.points, lat, lon), i))
        .filter(|(d, _)| *d <= metres)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, i)| i)
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

/// Линии наборов на картинке тайла key, RGBA с премноженной альфой.
/// Шаг через линию перемены дат разрывает линию. Пустая картинка - None.
pub fn draw(key: (u32, u32, u32), sets: &[&[Line]]) -> Option<Vec<u8>> {
    let (z, x, y) = key;
    let (s, w, n, e) = tile_box(z, x, y);
    // Запас - доля стороны тайла: толстая линия у края видна.
    let (dlat, dlon) = ((n - s) * 0.05, (e - w) * 0.05);
    let mut canvas = tiny_skia::Pixmap::new(SIZE, SIZE)?;
    let mut any = false;
    for lines in sets {
        for line in lines.iter() {
            let (ls, lw, ln, le) = line.bbox;
            if ls > n + dlat || ln < s - dlat || lw > e + dlon || le < w - dlon {
                continue;
            }
            let mut path = tiny_skia::PathBuilder::new();
            let mut last: Option<f64> = None;
            for (lat, lon) in &line.points {
                let (px, py) = pixel(z, x, y, *lat, *lon);
                match last {
                    Some(prev) if (lon - prev).abs() <= 180.0 => path.line_to(px, py),
                    _ => path.move_to(px, py),
                }
                last = Some(*lon);
            }
            let Some(path) = path.finish() else {
                continue;
            };
            let mut paint = tiny_skia::Paint::default();
            let [r, g, b, a] = line.color;
            paint.set_color_rgba8(r, g, b, a);
            paint.anti_alias = true;
            let stroke = tiny_skia::Stroke { width: line.width, line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
            canvas.stroke_path(&path, &paint, &stroke, tiny_skia::Transform::identity(), None);
            any = true;
        }
    }
    if !any {
        return None;
    }
    let data = canvas.take();
    data.as_chunks::<4>().0.iter().any(|p| p[3] > 0).then_some(data)
}
