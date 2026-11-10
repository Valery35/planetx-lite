// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Карта рельефа тайла высот для отмывки в каждом пикселе.
//!
//! Картинка RGBA SIZE × SIZE, пиксель на пиксель высот, строки с севера:
//!
//! - R, G - восточная и северная доли единичной нормали склона в местных
//!   осях восток, север, верх, `0.5 + 0.5·n`. Верхняя доля шейдер
//!   восстанавливает из двух первых.
//! - B - открытость неба: 1 - ровное место и гребень, меньше - впадина,
//!   склон под стеной. Это затенение впадин (ambient occlusion) по
//!   горизонтам в восьми направлениях.
//! - A - 255.
//!
//! Нормаль и горизонты считаются с вертикальным масштабом, поэтому карта
//! пересчитывается при его смене. Пиксель Mercator квадратный, его
//! сторона в метрах - длина круга широты, делённая на число пикселей.
//!
//! За краем тайла высоты берутся у соседей того же уровня. Без соседа
//! склон у края считается в одну сторону, а горизонт обрывается на краю:
//! край такого тайла светлее, пока сосед не пришёл.

use crate::{HeightTile, SIZE};

/// Направления горизонтов затенения впадин, восемь по кругу.
const DIRECTIONS: [(i64, i64); 8] = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];
/// Шаги поиска горизонта в клетках прореженной сетки. Клетка - два
/// пикселя высот, дальний шаг - 32 пикселя, восьмая часть тайла.
const STEPS: [i64; 5] = [1, 2, 4, 8, 16];
/// Прореживание сетки затенения впадин: оно меняется медленнее склона,
/// а счёт в 4 раза короче.
const THIN: usize = 2;

/// Тайл и его соседи: `[строка][столбец]`, строка 0 - север, столбец 0 -
/// запад, в середине сам тайл.
pub type Around<'a> = [[Option<&'a HeightTile>; 3]; 3];

/// Высота без масштаба в доле мира Mercator (u, v) по более грубым
/// тайлам, когда соседа того же уровня нет.
pub type Fallback<'a> = &'a dyn Fn(f64, f64) -> Option<f64>;

/// Высоты тайла с соседями по номеру пикселя от угла тайла, в том числе
/// за его краем на один тайл.
struct Heights<'a> {
    around: &'a Around<'a>,
    tile: &'a HeightTile,
    fallback: Option<Fallback<'a>>,
}

impl Heights<'_> {
    fn get(&self, row: i64, col: i64) -> Option<f64> {
        let side = SIZE as i64;
        let (ty, tx) = (row.div_euclid(side), col.div_euclid(side));
        if !(-1..=1).contains(&ty) || !(-1..=1).contains(&tx) {
            return None;
        }
        if let Some(tile) = self.around[(ty + 1) as usize][(tx + 1) as usize] {
            return Some(tile.at(row.rem_euclid(side) as usize, col.rem_euclid(side) as usize));
        }
        // Соседа нет - высота грубого тайла в центре того же пикселя.
        let fallback = self.fallback?;
        let n = f64::from(1u32 << self.tile.z.min(30));
        let u = (f64::from(self.tile.x) + (col as f64 + 0.5) / SIZE as f64) / n;
        let v = (f64::from(self.tile.y) + (row as f64 + 0.5) / SIZE as f64) / n;
        fallback(u, v)
    }
}

/// Ключи тайла (z, x, y) и его соседей в порядке [`Around`]. Долгота
/// замыкается, за полюсом соседа нет.
pub fn around_keys(z: u32, x: u32, y: u32) -> [[Option<(u32, u32, u32)>; 3]; 3] {
    let n = 1i64 << z.min(30);
    let mut out = [[None; 3]; 3];
    for (dy, row) in out.iter_mut().enumerate() {
        for (dx, cell) in row.iter_mut().enumerate() {
            let yy = i64::from(y) + dy as i64 - 1;
            let xx = (i64::from(x) + dx as i64 - 1).rem_euclid(n);
            if (0..n).contains(&yy) {
                *cell = Some((z, xx as u32, yy as u32));
            }
        }
    }
    out
}

/// Сторона пикселя тайла в метрах в строке row, radius - радиус тела.
fn pixel_metres(tile: &HeightTile, row: f64, radius: f64) -> f64 {
    let n = f64::from(1u32 << tile.z.min(30));
    let v = (f64::from(tile.y) + (row + 0.5) / SIZE as f64) / n;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * v)).sinh().atan();
    (2.0 * std::f64::consts::PI * radius * lat.cos() / (SIZE as f64 * n)).max(1e-6)
}

/// Разность высот по оси через пиксель: с обеих сторон, без соседа - в одну.
fn slope(here: f64, before: Option<f64>, after: Option<f64>) -> f64 {
    match (before, after) {
        (Some(a), Some(b)) => (b - a) / 2.0,
        (Some(a), None) => here - a,
        (None, Some(b)) => b - here,
        (None, None) => 0.0,
    }
}

/// База склона, метры. Высоты SRTM целые, на пологом склоне это
/// террасы по метру: по соседним пикселям подробного тайла они выходят
/// полосами, береговая ступенька воды - зубцами. Склон на базе в полторы
/// клетки SRTM их сглаживает, крупные формы остаются.
const SLOPE_BASE: f64 = 50.0;
/// Наибольший шаг базы склона, пикселей.
const SLOPE_STEP_MAX: i64 = 12;

/// Единичная нормаль склона в пикселе (восток, север, верх). Разности
/// через step пикселей, усреднённые по трём строкам или столбцам с тем же
/// шагом, как у фильтра Собеля.
fn normal(h: &Heights<'_>, row: i64, col: i64, metres: f64, scale: f64) -> [f64; 3] {
    let step = ((SLOPE_BASE / metres).round() as i64).clamp(1, SLOPE_STEP_MAX);
    let mut east = 0.0;
    let mut north = 0.0;
    for (offset, weight) in [(-step, 1.0), (0, 2.0), (step, 1.0)] {
        let here = h.get(row + offset, col).unwrap_or(0.0);
        east += weight * slope(here, h.get(row + offset, col - step), h.get(row + offset, col + step));
        let here = h.get(row, col + offset).unwrap_or(0.0);
        // Строки идут с севера: север - меньшая строка.
        north += weight * slope(here, h.get(row + step, col + offset), h.get(row - step, col + offset));
    }
    let east = east / 4.0 * scale / (step as f64 * metres);
    let north = north / 4.0 * scale / (step as f64 * metres);
    let (x, y) = (-east, -north);
    let length = (x * x + y * y + 1.0).sqrt();
    [x / length, y / length, 1.0 / length]
}

/// Открытость неба прореженной сетки: side × side клеток по THIN
/// пикселей, у клетки средняя высота её пикселей. Горизонт ищется и в
/// клетках соседей на дальний шаг за краем.
fn openness(tile: &HeightTile, h: &Heights<'_>, radius: f64, scale: f64) -> (usize, Vec<f64>) {
    let side = SIZE / THIN;
    let mut open = vec![1.0; side * side];
    if scale <= 0.0 {
        return (side, open);
    }
    let margin = STEPS[STEPS.len() - 1];
    let wide = side as i64 + 2 * margin;
    let cells: Vec<Option<f64>> = (0..wide * wide)
        .map(|i| {
            let (row, col) = ((i / wide - margin) * THIN as i64, (i % wide - margin) * THIN as i64);
            let mut sum = 0.0;
            for dr in 0..THIN as i64 {
                for dc in 0..THIN as i64 {
                    sum += h.get(row + dr, col + dc)?;
                }
            }
            Some(sum * scale / (THIN * THIN) as f64)
        })
        .collect();
    let cell = |r: i64, c: i64| -> Option<f64> {
        if r < -margin || c < -margin || r >= side as i64 + margin || c >= side as i64 + margin {
            return None;
        }
        cells[((r + margin) * wide + c + margin) as usize]
    };
    for row in 0..side as i64 {
        let metres = pixel_metres(tile, (row as usize * THIN) as f64 + 0.5, radius) * THIN as f64;
        for col in 0..side as i64 {
            let Some(here) = cell(row, col) else {
                continue;
            };
            let mut closed = 0.0;
            for (dx, dy) in DIRECTIONS {
                let along = ((dx * dx + dy * dy) as f64).sqrt() * metres;
                let mut steepest: f64 = 0.0;
                for step in STEPS {
                    let Some(there) = cell(row + dy * step, col + dx * step) else {
                        break;
                    };
                    steepest = steepest.max((there - here) / (along * step as f64));
                }
                // Синус угла горизонта: стена в 90° закрывает направление
                // целиком, ровное место не закрывает.
                closed += steepest / (1.0 + steepest * steepest).sqrt();
            }
            open[row as usize * side + col as usize] = 1.0 - closed / DIRECTIONS.len() as f64;
        }
    }
    (side, open)
}

/// Открытость в пикселе тайла по прореженной сетке, билинейно.
fn open_at(side: usize, open: &[f64], row: usize, col: usize) -> f64 {
    let at = |p: usize| (p as f64 + 0.5) / THIN as f64 - 0.5;
    let (fr, fc) = (at(row).clamp(0.0, (side - 1) as f64), at(col).clamp(0.0, (side - 1) as f64));
    let (r0, c0) = (fr.floor() as usize, fc.floor() as usize);
    let (r1, c1) = ((r0 + 1).min(side - 1), (c0 + 1).min(side - 1));
    let (tr, tc) = (fr - r0 as f64, fc - c0 as f64);
    let v = |r: usize, c: usize| open.get(r * side + c).copied().unwrap_or(1.0);
    let top = v(r0, c0) * (1.0 - tc) + v(r0, c1) * tc;
    let bottom = v(r1, c0) * (1.0 - tc) + v(r1, c1) * tc;
    top * (1.0 - tr) + bottom * tr
}

/// Байт доли 0..1.
fn byte(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Карта рельефа тайла в середине around: RGBA SIZE × SIZE, см. описание
/// модуля. radius - радиус тела в метрах, scale - вертикальный масштаб
/// рельефа, 0 - карта ровного места. fallback - высоты за краем, где
/// соседа нет. Пустая картинка - тайла в середине нет.
pub fn relief_rgba(around: &Around<'_>, fallback: Option<Fallback<'_>>, radius: f64, scale: f64) -> Vec<u8> {
    let Some(tile) = around[1][1] else {
        return Vec::new();
    };
    let h = Heights { around, tile, fallback };
    let (side, open) = openness(tile, &h, radius, scale);
    let mut out = Vec::with_capacity(SIZE * SIZE * 4);
    for row in 0..SIZE {
        let metres = pixel_metres(tile, row as f64, radius);
        for col in 0..SIZE {
            let n = if scale > 0.0 { normal(&h, row as i64, col as i64, metres, scale) } else { [0.0, 0.0, 1.0] };
            out.extend_from_slice(&[byte(0.5 + 0.5 * n[0]), byte(0.5 + 0.5 * n[1]), byte(open_at(side, &open, row, col)), 255]);
        }
    }
    out
}

/// Карта рельефа одного тайла без соседей.
pub fn relief_alone(tile: &HeightTile, radius: f64, scale: f64) -> Vec<u8> {
    let mut around: Around<'_> = [[None; 3]; 3];
    around[1][1] = Some(tile);
    relief_rgba(&around, None, radius, scale)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{Around, SIZE, around_keys, pixel_metres, relief_alone, relief_rgba};
    use crate::HeightTile;

    const R: f64 = 6_378_137.0;

    /// Тайл (10, 600 + dx, 300 + dy) с высотами f(строка, столбец, метры
    /// пикселя) в сквозной нумерации от угла тайла (10, 600, 300).
    fn tile_at(dx: i64, dy: i64, f: &impl Fn(i64, i64, f64) -> f64) -> HeightTile {
        let probe = HeightTile::new(10, 600, 300, vec![0.0; SIZE * SIZE]).unwrap();
        let side = SIZE as i64;
        let heights = (0..SIZE * SIZE)
            .map(|i| {
                let (row, col) = ((i / SIZE) as i64 + dy * side, (i % SIZE) as i64 + dx * side);
                f(row, col, pixel_metres(&probe, row as f64, R)) as f32
            })
            .collect();
        HeightTile::new(10, (600 + dx) as u32, (300 + dy) as u32, heights).unwrap()
    }

    fn tile(f: impl Fn(i64, i64, f64) -> f64) -> HeightTile {
        tile_at(0, 0, &f)
    }

    fn px(rgba: &[u8], row: usize, col: usize) -> [u8; 4] {
        let i = (row * SIZE + col) * 4;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    }

    #[test]
    fn flat_ground_faces_up_and_is_open() {
        let rgba = relief_alone(&tile(|_, _, _| 120.0), R, 1.0);
        assert_eq!(rgba.len(), SIZE * SIZE * 4);
        assert_eq!(px(&rgba, 100, 100), [128, 128, 255, 255]);
        assert_eq!(px(&rgba, 0, 255), [128, 128, 255, 255]);
    }

    #[test]
    fn slope_of_45_degrees_to_the_east_and_scale() {
        // Высота растёт на восток на сторону пикселя за пиксель - склон
        // в 45°, нормаль смотрит на запад и вверх поровну, у края тоже.
        let rising = tile(|_, col, m| col as f64 * m);
        let rgba = relief_alone(&rising, R, 1.0);
        let want = (0.5 - 0.5 * std::f64::consts::FRAC_1_SQRT_2) * 255.0;
        for col in [0, 128, 255] {
            let p = px(&rgba, 128, col);
            assert!((f64::from(p[0]) - want).abs() <= 1.0, "{col}: {p:?}");
            assert_eq!(p[1], 128);
        }
        // Масштаб 0 - ровное место, вдвое - склон круче.
        assert_eq!(px(&relief_alone(&rising, R, 0.0), 128, 128), [128, 128, 255, 255]);
        assert!(px(&relief_alone(&rising, R, 2.0), 128, 128)[0] < px(&rgba, 128, 128)[0]);
        // Склон на север: высота растёт к северу, то есть к строке 0.
        let north = relief_alone(&tile(|row, _, m| (SIZE as i64 - row) as f64 * m), R, 1.0);
        let p = px(&north, 128, 128);
        assert_eq!(p[0], 128);
        assert!((f64::from(p[1]) - want).abs() <= 1.0, "{p:?}");
    }

    #[test]
    fn metre_terraces_of_a_fine_tile_are_smooth() {
        // Пологий склон 5 % в целых метрах, как SRTM, на тайле уровня 15:
        // пиксель около 3 м, терраса через 20 м. Наклон по строке почти
        // ровный, а не полосами.
        let probe = HeightTile::new(15, 19_000, 9_000, vec![0.0; SIZE * SIZE]).unwrap();
        let m = pixel_metres(&probe, 128.0, R);
        let heights = (0..SIZE * SIZE).map(|i| ((i % SIZE) as f64 * m * 0.05).floor() as f32).collect();
        let tile = HeightTile::new(15, 19_000, 9_000, heights).unwrap();
        let rgba = relief_alone(&tile, R, 1.0);
        let reds: Vec<u8> = (40..216).map(|col| px(&rgba, 128, col)[0]).collect();
        let (low, high) = (reds.iter().min().unwrap(), reds.iter().max().unwrap());
        assert!(high - low <= 4, "{low}..{high}");
    }

    #[test]
    fn pit_is_darker_than_ridge() {
        // Чаша и холм одной формы: дно чаши закрыто, вершина холма открыта.
        let bowl = |row: i64, col: i64, m: f64| {
            let (dr, dc) = (row as f64 - 128.0, col as f64 - 128.0);
            (dr * dr + dc * dc).sqrt() * m * 0.5
        };
        let pit = relief_alone(&tile(bowl), R, 1.0);
        let hill = relief_alone(&tile(|r, c, m| -bowl(r, c, m)), R, 1.0);
        let (floor, top) = (px(&pit, 128, 128)[2], px(&hill, 128, 128)[2]);
        // Прореженная сетка размывает вершину на пиксель-другой.
        assert!(floor < 160 && top > 240, "{floor} {top}");
    }

    #[test]
    fn neighbours_remove_the_seam() {
        // Долина вдоль меридиана на стыке тайлов: без соседа край тайла
        // открыт, с соседом - так же закрыт, как в глубине долины.
        let valley = |_: i64, col: i64, m: f64| (col as f64 - 256.0).abs() * m * 0.6;
        let me = tile_at(0, 0, &valley);
        let east = tile_at(1, 0, &valley);
        let alone = relief_alone(&me, R, 1.0);
        let mut around: Around<'_> = [[None; 3]; 3];
        around[1][1] = Some(&me);
        around[1][2] = Some(&east);
        let joined = relief_rgba(&around, None, R, 1.0);
        let (edge_alone, edge_joined, inside) = (px(&alone, 128, 255)[2], px(&joined, 128, 255)[2], px(&joined, 128, 250)[2]);
        assert!(edge_alone > edge_joined + 20, "{edge_alone} {edge_joined}");
        assert!(edge_joined.abs_diff(inside) <= 25, "{edge_joined} {inside}");
        // Соседа нет, но есть грубые высоты: край почти как с соседом.
        let probe = HeightTile::new(10, 600, 300, vec![0.0; SIZE * SIZE]).unwrap();
        let coarse = |u: f64, _v: f64| {
            let col = (u * 1024.0 - 600.0) * SIZE as f64 - 0.5;
            Some(valley(0, col.round() as i64, pixel_metres(&probe, 128.0, R)))
        };
        let mut lone: Around<'_> = [[None; 3]; 3];
        lone[1][1] = Some(&me);
        let filled = relief_rgba(&lone, Some(&coarse), R, 1.0);
        assert!(px(&filled, 128, 255)[2].abs_diff(edge_joined) <= 10, "{:?} {edge_joined}", px(&filled, 128, 255));
        // Соседи по долготе замыкаются, за полюсом их нет.
        let keys = around_keys(2, 0, 0);
        assert_eq!(keys[1][0], Some((2, 3, 0)));
        assert_eq!(keys[0][1], None);
    }
}
