// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Подземный режим: сетки стволов, горизонтов и разрезов. Перенос
//! `core/subsurface.py` модуля.
//!
//! Всё подземное - треугольники с нормалью и цветом в вершине, тот же
//! формат вершины, что у 3D-зданий модуля (`core/buildings.VERTEX`),
//! здесь [`Vertex`]. Точки приходят широтой, долготой и высотой на экране:
//! настоящая отметка, умноженная на масштаб рельефа (решение автора модуля
//! от 2 октября 2026 года - масштаб общий с рельефом). Куски сетки
//! ([`Part`]) - в ECEF в f64, сетка для видеокарты ([`Mesh`]) - центр
//! в ECEF в f64 и вершины смещениями от него в f32, как у тайлов.
//!
//! - [`tube`] - трубка вдоль ломаной ствола, сечение [`SIDES`]-угольник;
//! - [`grid_surface`] - сетка горизонта по узлам растра, ячейка с пустым
//!   углом пропускается;
//! - [`wall`] - вертикальная стенка между верхней и нижней линией, стенка
//!   разреза между двумя поверхностями;
//! - [`inside`] - точки внутри многоугольника выреза.
//!
//! Эллипсоид - WGS84. Крейт data не зависит от globe-core, поэтому
//! пересчёт в ECEF здесь свой, те же формулы.

use std::collections::{HashMap, HashSet};

/// Граней трубки ствола.
pub const SIDES: usize = 8;
/// Насколько темнее стенка блока у низа, выбор помощника модуля.
pub const BLOCK_FADE: f64 = 0.45;
/// Окончания имён гридов кровли пласта.
pub const TOPS: [&str; 4] = ["top", "roof", "кровля", "кр"];
/// Окончания имён гридов подошвы пласта.
pub const BOTTOMS: [&str; 5] = ["bottom", "bot", "base", "подошва", "под"];
/// Ключ своего цвета породы между пластами на разрезе модели.
pub const HOST: &str = "@host";
/// Линия кровли по умолчанию - цвет пласта, темнее в эту долю.
pub const LINE_SHADE: f64 = 0.55;
/// Клеток на сторону коробки фона.
pub const BACKDROP_STEPS: usize = 8;
/// Проб рельефа на клетку коробки по стороне.
pub const BACKDROP_DENSE: usize = 8;

/// Большая полуось WGS84, метры.
pub const A: f64 = 6378137.0;
/// Сжатие WGS84.
pub const F: f64 = 1.0 / 298.257223563;
/// Малая полуось WGS84, метры.
pub const B: f64 = A * (1.0 - F);
/// Квадрат первого эксцентриситета.
const E2: f64 = F * (2.0 - F);
/// Квадрат второго эксцентриситета.
const EP2: f64 = E2 / (1.0 - E2);

/// Цвет RGBA.
pub type Rgba = [u8; 4];
type V3 = [f64; 3];

/// Кусок сетки в ECEF: точки, нормали, цвета в вершинах и треугольники
/// номерами вершин куска.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Part {
    pub positions: Vec<[f64; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[u8; 4]>,
    pub indices: Vec<[u32; 3]>,
}

/// Вершина сетки для видеокарты, `VERTEX` модуля: смещение от центра,
/// нормаль в i8 с масштабом 127 (четвёртый байт 0), цвет RGBA. 20 байт.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [i8; 4],
    pub color: [u8; 4],
}

/// Сетка для видеокарты: центр в ECEF, вершины, треугольники тройками
/// номеров подряд.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mesh {
    pub center: [f64; 3],
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

/// Вершина стенки с картинкой, `IMAGE_VERTEX` модуля: смещение от центра
/// и место в картинке.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ImageVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
}

/// Стенка с картинкой разреза: центр в ECEF, вершины, треугольники тройками
/// номеров подряд.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImageWall {
    pub center: [f64; 3],
    pub vertices: Vec<ImageVertex>,
    pub indices: Vec<u32>,
}

/// Признак значения в узлах сетки rows × cols, по строкам. Узел вне valid
/// считается пустым.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mask {
    pub rows: usize,
    pub cols: usize,
    pub valid: Vec<bool>,
}

impl Mask {
    /// Сетка, у которой все узлы value.
    pub fn full(rows: usize, cols: usize, value: bool) -> Mask {
        Mask { rows, cols, valid: vec![value; rows.saturating_mul(cols)] }
    }

    /// Есть ли значение в узле (row, col).
    pub fn at(&self, row: usize, col: usize) -> bool {
        row < self.rows && col < self.cols && self.valid.get(row * self.cols + col).copied().unwrap_or(false)
    }

    /// Количество узлов, если номера узлов помещаются в u32.
    fn nodes(&self) -> Option<usize> {
        self.rows.checked_mul(self.cols).filter(|n| *n <= u32::MAX as usize)
    }
}

/// Растр отметок: узел (0, 0) в точке (x0, y0) системы координат растра,
/// шаг dx по столбцам, dy по строкам (у растров с севера на юг
/// отрицательный), z - rows × cols по строкам, пусто - NaN.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Grid {
    pub x0: f64,
    pub y0: f64,
    pub dx: f64,
    pub dy: f64,
    pub rows: usize,
    pub cols: usize,
    pub z: Vec<f64>,
}

impl Grid {
    fn node(&self, row: usize, col: usize) -> f64 {
        self.z.get(row * self.cols + col).copied().unwrap_or(f64::NAN)
    }

    /// Отметка в точке (x, y) системы растра, билинейно. Вне растра и у
    /// пустых узлов - NaN.
    pub fn sample(&self, x: f64, y: f64) -> f64 {
        let (rows, cols) = (self.rows, self.cols);
        if rows == 0 || cols == 0 {
            return f64::NAN;
        }
        let fx = (x - self.x0) / self.dx;
        let fy = (y - self.y0) / self.dy;
        let ok = fx >= 0.0 && fx <= (cols - 1) as f64 && fy >= 0.0 && fy <= (rows - 1) as f64;
        if !ok {
            return f64::NAN;
        }
        let j = (fx as usize).min(cols.saturating_sub(2));
        let i = (fy as usize).min(rows.saturating_sub(2));
        let (ax, ay) = (fx - j as f64, fy - i as f64);
        let j1 = (j + 1).min(cols - 1);
        let i1 = (i + 1).min(rows - 1);
        // Узел с нулевым весом не участвует: пустой сосед на краю ячейки
        // не делает точку пустой.
        let mut total = 0.0;
        for (value, weight) in [
            (self.node(i, j), (1.0 - ax) * (1.0 - ay)),
            (self.node(i, j1), ax * (1.0 - ay)),
            (self.node(i1, j), (1.0 - ax) * ay),
            (self.node(i1, j1), ax * ay),
        ] {
            if weight > 0.0 {
                total += value * weight;
            }
        }
        total
    }
}

/// Сторона пласта по имени грида.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Top,
    Bottom,
}

// Векторы.

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}

/// Единичный вектор, нулевой остаётся нулевым.
fn unit(a: V3) -> V3 {
    let n = norm(a);
    if n > 0.0 { scale(a, 1.0 / n) } else { a }
}

fn narrow(a: V3) -> [f32; 3] {
    [a[0] as f32, a[1] as f32, a[2] as f32]
}

fn wide(a: [f32; 3]) -> V3 {
    [f64::from(a[0]), f64::from(a[1]), f64::from(a[2])]
}

/// Меньшее из двух, NaN - если NaN хоть одно, как `np.minimum`.
fn nan_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.min(b) }
}

/// Помещается ли количество вершин в номера u32.
fn fits(count: usize) -> bool {
    count <= u32::MAX as usize
}

// Эллипсоид.

/// Широта, долгота в градусах и высота в метрах в ECEF.
pub fn geodetic_to_ecef(lat: f64, lon: f64, h: f64) -> [f64; 3] {
    let (sl, cl) = lat.to_radians().sin_cos();
    let (so, co) = lon.to_radians().sin_cos();
    let n = A / (1.0 - E2 * sl * sl).sqrt();
    [(n + h) * cl * co, (n + h) * cl * so, (n * (1.0 - E2) + h) * sl]
}

/// ECEF в широту, долготу в градусах и высоту в метрах. Решение
/// Хейккинена, замкнутое, без итераций, как `ellipsoid.ecef_to_geodetic`
/// модуля. Ближе 100 км к центру Земли формула неустойчива.
pub fn ecef_to_geodetic(p: [f64; 3]) -> [f64; 3] {
    let [x, y, z] = p;
    let p2 = x * x + y * y;
    let pr = p2.sqrt();
    let z2 = z * z;
    let f54 = 54.0 * B * B * z2;
    let g = p2 + (1.0 - E2) * z2 - E2 * (A * A - B * B);
    let c = E2 * E2 * f54 * p2 / (g * g * g);
    let s = (1.0 + c + (c * c + 2.0 * c).sqrt()).cbrt();
    let k = s + 1.0 + 1.0 / s;
    let pp = f54 / (3.0 * k * k * g * g);
    let q = (1.0 + 2.0 * E2 * E2 * pp).sqrt();
    // У оси Земли подкоренное около нуля, округление уводит его в минус.
    let root = (0.5 * A * A * (1.0 + 1.0 / q) - pp * (1.0 - E2) * z2 / (q * (1.0 + q)) - 0.5 * pp * p2).max(0.0);
    let r0 = -pp * E2 * pr / (1.0 + q) + root.sqrt();
    let t = pr - E2 * r0;
    let u = (t * t + z2).sqrt();
    let v = (t * t + (1.0 - E2) * z2).sqrt();
    let z0 = B * B * z / (A * v);
    let h = u * (1.0 - B * B / (A * v));
    [(z + EP2 * z0).atan2(pr).to_degrees(), y.atan2(x).to_degrees(), h]
}

/// Единичная нормаль к эллипсоиду в точке широты и долготы, градусы.
pub fn surface_normal(lat: f64, lon: f64) -> [f64; 3] {
    let (sl, cl) = lat.to_radians().sin_cos();
    let (so, co) = lon.to_radians().sin_cos();
    [cl * co, cl * so, sl]
}

/// Точки в ECEF по широтам, долготам и высотам. Лишние значения длинных
/// списков не берутся.
pub fn ecef(lats: &[f64], lons: &[f64], alts: &[f64]) -> Vec<[f64; 3]> {
    lats.iter().zip(lons).zip(alts).map(|((la, lo), h)| geodetic_to_ecef(*la, *lo, *h)).collect()
}

// Стволы.

/// Широта и долгота точки ECEF по направлению из центра - для оси сечения
/// трубки этого хватает.
fn latlon(p: V3) -> (f64, f64) {
    (p[2].atan2(p[0].hypot(p[1])).to_degrees(), p[1].atan2(p[0]).to_degrees())
}

/// Касательные ломаной: разность соседей внутри, крайний отрезок на концах.
fn tangents(points: &[V3]) -> Vec<V3> {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (a, b) = (i.saturating_sub(1), (i + 1).min(n - 1));
            sub(points[b], points[a])
        })
        .collect()
}

/// Трубка радиуса radius вдоль ломаной points в ECEF, сечение
/// sides-угольник.
///
/// Сечение переносится вдоль ломаной без поворота (параллельный перенос),
/// так грани не перекручиваются на изгибах ствола.
pub fn tube(points: &[[f64; 3]], radius: f64, color: Rgba, sides: usize) -> Option<Part> {
    let n = points.len();
    if n < 2 || sides == 0 || !n.checked_mul(sides).is_some_and(fits) {
        return None;
    }
    let tangents: Vec<V3> = tangents(points).into_iter().map(unit).collect();
    let t0 = tangents[0];
    // Начальная ось сечения - перпендикуляр к касательной.
    let (lat, lon) = latlon(points[0]);
    let mut axis = cross(t0, surface_normal(lat, lon));
    if norm(axis) < 1e-6 {
        axis = cross(t0, [1.0, 0.0, 0.0]);
    }
    axis = unit(axis);
    let mut part = Part::default();
    for (point, t) in points.iter().zip(&tangents) {
        axis = sub(axis, scale(*t, dot(axis, *t)));
        axis = scale(axis, 1.0 / norm(axis).max(1e-12));
        let other = cross(*t, axis);
        for k in 0..sides {
            let angle = k as f64 * 2.0 * std::f64::consts::PI / sides as f64;
            let ring = add(scale(axis, angle.cos()), scale(other, angle.sin()));
            part.normals.push(narrow(ring));
            part.positions.push(add(*point, scale(ring, radius)));
        }
    }
    part.colors = vec![color; n * sides];
    let s = sides as u32;
    for a in 0..(n - 1) as u32 {
        for k in 0..s {
            let (base, k1) = (a * s, (k + 1) % s);
            part.indices.push([base + k, base + k1, base + s + k]);
            part.indices.push([base + k1, base + s + k1, base + s + k]);
        }
    }
    Some(part)
}

// Горизонты.

/// Треугольники сетки узлов по признаку mask, номерами узлов. Ячейка
/// с четырьмя узлами - два треугольника, с тремя - один, так край растра
/// идёт косой линией, а не ступенями. Порядок - как у модуля: сначала
/// полные ячейки, потом ячейки с тремя узлами.
pub fn grid_triangles(mask: &Mask) -> Vec<[u32; 3]> {
    let (rows, cols) = (mask.rows, mask.cols);
    if rows < 2 || cols < 2 || mask.nodes().is_none() {
        return Vec::new();
    }
    let cells = || (0..rows - 1).flat_map(move |i| (0..cols - 1).map(move |j| (i, j)));
    let corners = |i: usize, j: usize| {
        let a = (i * cols + j) as u32;
        let (b, c) = (a + 1, a + cols as u32);
        (a, b, c, c + 1, [mask.at(i, j), mask.at(i, j + 1), mask.at(i + 1, j), mask.at(i + 1, j + 1)])
    };
    let mut out = Vec::new();
    // Вид треугольника: какие узлы ячейки нужны, какой узел пуст.
    for kind in 0..6 {
        for (i, j) in cells() {
            let (a, b, c, d, v) = corners(i, j);
            let count = v.iter().filter(|x| **x).count();
            let tri = match kind {
                0 if count == 4 => [a, b, c],
                1 if count == 4 => [b, d, c],
                2 if count == 3 && !v[3] => [a, b, c],
                3 if count == 3 && !v[0] => [b, d, c],
                4 if count == 3 && !v[1] => [a, d, c],
                5 if count == 3 && !v[2] => [a, b, d],
                _ => continue,
            };
            out.push(tri);
        }
    }
    out
}

/// Край треугольников сетки mask ([`grid_triangles`]): рёбра, которые
/// принадлежат одному треугольнику. Направление ребра - обход
/// треугольника, у всех треугольников он один, поэтому нормали стенок
/// смотрят в одну сторону от данных.
pub fn boundary_edges(mask: &Mask) -> Vec<[u32; 2]> {
    let tris = grid_triangles(mask);
    let edges: Vec<[u32; 2]> =
        [(0, 1), (1, 2), (2, 0)].iter().flat_map(|&(p, q)| tris.iter().map(move |t| [t[p], t[q]])).collect();
    let mut seen: HashMap<(u32, u32), (usize, usize)> = HashMap::new();
    for (i, e) in edges.iter().enumerate() {
        let entry = seen.entry((e[0].min(e[1]), e[0].max(e[1]))).or_insert((i, 0));
        entry.1 += 1;
    }
    let mut first: Vec<usize> = seen.values().filter(|(_, count)| *count == 1).map(|(i, _)| *i).collect();
    first.sort_unstable();
    first.into_iter().map(|i| edges[i]).collect()
}

/// Четырёхугольники между top и bottom по рёбрам edges. shade - множители
/// цвета узлов верха и низа. Рёбра с узлами вне top и bottom
/// пропускаются.
fn edge_quads(top: &[V3], bottom: &[V3], edges: &[[u32; 2]], color: Rgba, shade: Option<(&[f64], &[f64])>) -> Option<Part> {
    let edges: Vec<[usize; 2]> = edges
        .iter()
        .map(|e| [e[0] as usize, e[1] as usize])
        .filter(|e| e.iter().all(|i| *i < top.len() && *i < bottom.len()))
        .collect();
    let n = edges.len();
    if n == 0 || !n.checked_mul(4).is_some_and(fits) {
        return None;
    }
    let mut part = Part::default();
    let corners = [(top, 0), (top, 1), (bottom, 0), (bottom, 1)];
    for (grid, end) in corners {
        part.positions.extend(edges.iter().map(|e| grid[e[end]]));
    }
    let side: Vec<[f32; 3]> = edges.iter().map(|e| narrow(unit(cross(sub(top[e[1]], top[e[0]]), unit(top[e[0]]))))).collect();
    for _ in 0..4 {
        part.normals.extend_from_slice(&side);
    }
    for (layer, end) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
        for e in &edges {
            let mut c = color;
            if let Some((upper, lower)) = shade {
                let k = if layer == 0 { upper } else { lower }.get(e[end]).copied().unwrap_or(1.0);
                for channel in c.iter_mut().take(3) {
                    *channel = (f64::from(*channel) * k) as u8;
                }
            }
            part.colors.push(c);
        }
    }
    let m = n as u32;
    part.indices.extend((0..m).map(|i| [i, m + i, 2 * m + i]));
    part.indices.extend((0..m).map(|i| [m + i, 3 * m + i, 2 * m + i]));
    Some(part)
}

/// Боковые стенки пласта между поверхностями top и bottom в ECEF на одной
/// сетке узлов: по краю треугольников mask. Так пласт выглядит плитой
/// с торцами по краю данных и по краю выреза. skip - рёбра без стенки,
/// в любом порядке концов: там стоит стенка блока.
pub fn grid_walls(top: &[[f64; 3]], bottom: &[[f64; 3]], mask: &Mask, color: Rgba, skip: &[[u32; 2]]) -> Option<Part> {
    let skip: HashSet<(u32, u32)> = skip.iter().map(|e| (e[0].min(e[1]), e[0].max(e[1]))).collect();
    let edges: Vec<[u32; 2]> =
        boundary_edges(mask).into_iter().filter(|e| !skip.contains(&(e[0].min(e[1]), e[0].max(e[1])))).collect();
    edge_quads(top, bottom, &edges, color, None)
}

/// Стенки блока по краю сетки mask: между поверхностями surfaces в ECEF
/// сверху вниз - пласты цветов colors. Край блока закрывает вид сквозь
/// прозрачную землю мимо гридов. Цвет темнеет с глубиной до доли
/// 1 - [`BLOCK_FADE`] у низа блока.
pub fn block_walls(surfaces: &[Vec<[f64; 3]>], mask: &Mask, colors: &[Rgba]) -> Vec<Part> {
    let edges = boundary_edges(mask);
    let (Some(first), Some(last)) = (surfaces.first(), surfaces.last()) else {
        return Vec::new();
    };
    if edges.is_empty() {
        return Vec::new();
    }
    let depth = |s: &Vec<V3>| -> Vec<f64> {
        (0..s.len()).map(|i| first.get(i).map_or(f64::NAN, |f| norm(sub(s[i], *f)))).collect()
    };
    let deepest = depth(last).into_iter().fold(1e-6, f64::max);
    let shades: Vec<Vec<f64>> =
        surfaces.iter().map(|s| depth(s).into_iter().map(|d| 1.0 - BLOCK_FADE * d / deepest).collect()).collect();
    surfaces
        .windows(2)
        .zip(colors)
        .zip(shades.windows(2))
        .filter_map(|((pair, color), shade)| {
            edge_quads(&pair[0], &pair[1], &edges, *color, Some((&shade[0], &shade[1])))
        })
        .collect()
}

/// Изолинии на поверхности - полосы ширины width метров по отрезкам
/// p0-p1 в ECEF, поднятые на lift метров от центра Земли, чтобы не мерцать
/// с поверхностью. Нормаль - вверх.
pub fn ribbons(p0: &[[f64; 3]], p1: &[[f64; 3]], width: f64, color: Rgba, lift: f64) -> Option<Part> {
    let n = p0.len().min(p1.len());
    if n == 0 || !n.checked_mul(4).is_some_and(fits) {
        return None;
    }
    let ups: Vec<V3> = p0[..n].iter().map(|p| unit(*p)).collect();
    let sides: Vec<V3> =
        (0..n).map(|i| scale(unit(cross(sub(p1[i], p0[i]), ups[i])), width / 2.0)).collect();
    let a: Vec<V3> = (0..n).map(|i| add(p0[i], scale(ups[i], lift))).collect();
    let b: Vec<V3> = (0..n).map(|i| add(p1[i], scale(ups[i], lift))).collect();
    let mut part = Part::default();
    part.positions.extend((0..n).map(|i| sub(a[i], sides[i])));
    part.positions.extend((0..n).map(|i| add(a[i], sides[i])));
    part.positions.extend((0..n).map(|i| sub(b[i], sides[i])));
    part.positions.extend((0..n).map(|i| add(b[i], sides[i])));
    for _ in 0..4 {
        part.normals.extend(ups.iter().map(|u| narrow(*u)));
    }
    part.colors = vec![color; 4 * n];
    let m = n as u32;
    part.indices.extend((0..m).map(|i| [i, m + i, 2 * m + i]));
    part.indices.extend((0..m).map(|i| [m + i, 3 * m + i, 2 * m + i]));
    Some(part)
}

/// Сетка поверхности по узлам points в ECEF, rows × cols mask по строкам.
///
/// colors - один цвет на все узлы или цвет каждого узла. Ячейка с пустым
/// углом пропускается. Нормали - по разностям соседей, наружу от центра
/// Земли.
pub fn grid_surface(points: &[[f64; 3]], mask: &Mask, colors: &[Rgba]) -> Option<Part> {
    let (rows, cols) = (mask.rows, mask.cols);
    let count = mask.nodes()?;
    if rows < 2 || cols < 2 || points.len() != count {
        return None;
    }
    let colors = match colors.len() {
        1 => vec![colors[0]; count],
        n if n == count => colors.to_vec(),
        _ => return None,
    };
    let at = |i: usize, j: usize| points[i * cols + j];
    // Разность соседей, как `np.gradient`: внутри - средняя, на краю -
    // односторонняя.
    let gradient = |lo: V3, hi: V3, inner: bool| if inner { scale(sub(hi, lo), 0.5) } else { sub(hi, lo) };
    let mut normals = Vec::with_capacity(count);
    for i in 0..rows {
        for j in 0..cols {
            let (i0, i1) = (i.saturating_sub(1), (i + 1).min(rows - 1));
            let (j0, j1) = (j.saturating_sub(1), (j + 1).min(cols - 1));
            let d_row = gradient(at(i0, j), at(i1, j), i0 + 2 == i1);
            let d_col = gradient(at(i, j0), at(i, j1), j0 + 2 == j1);
            let mut normal = unit(cross(d_col, d_row));
            if dot(normal, at(i, j)) < 0.0 {
                normal = scale(normal, -1.0);
            }
            normals.push(narrow(normal));
        }
    }
    let indices = grid_triangles(mask);
    if indices.is_empty() {
        return None;
    }
    Some(Part { positions: points.to_vec(), normals, colors, indices })
}

/// Вертикальная стенка вдоль линии точек (lats, lons) от высот tops
/// до bottoms. Где верх не выше низа, стенки нет, где поверхности
/// пересеклись, низ поднимается до верха. Нормаль - горизонтальная, поперёк
/// линии.
pub fn wall(lats: &[f64], lons: &[f64], tops: &[f64], bottoms: &[f64], color: Rgba) -> Option<Part> {
    let n = lats.len().min(lons.len()).min(tops.len()).min(bottoms.len());
    if n < 2 || !n.checked_mul(2).is_some_and(fits) {
        return None;
    }
    let (lats, lons, tops, bottoms) = (&lats[..n], &lons[..n], &tops[..n], &bottoms[..n]);
    let top = ecef(lats, lons, tops);
    let side: Vec<[f32; 3]> =
        tangents(&top).into_iter().zip(&top).map(|(along, t)| narrow(unit(cross(along, unit(*t))))).collect();
    // Ячейка есть, если стенка не нулевая хотя бы на одном краю.
    let keep: Vec<u32> = (0..n - 1).filter(|&i| tops[i] > bottoms[i] || tops[i + 1] > bottoms[i + 1]).map(|i| i as u32).collect();
    if keep.is_empty() {
        return None;
    }
    let m = n as u32;
    let mut indices: Vec<[u32; 3]> = keep.iter().map(|&i| [i, i + 1, m + i]).collect();
    indices.extend(keep.iter().map(|&i| [i + 1, m + i + 1, m + i]));
    let clipped: Vec<f64> = bottoms.iter().zip(tops).map(|(b, t)| nan_min(*b, *t)).collect();
    let mut positions = top;
    positions.extend(ecef(lats, lons, &clipped));
    let mut normals = side.clone();
    normals.extend(side);
    Some(Part { positions, normals, colors: vec![color; 2 * n], indices })
}

/// Удвоенная площадь многоугольника со знаком, против часовой - больше 0.
fn area2(p: &[[f64; 2]]) -> f64 {
    let n = p.len();
    (0..n).map(|i| p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1]).sum()
}

/// Лежит ли точка p в треугольнике abc против часовой стрелки.
fn in_triangle(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let side = |u: [f64; 2], v: [f64; 2], w: [f64; 2]| (v[0] - u[0]) * (w[1] - u[1]) - (v[1] - u[1]) * (w[0] - u[0]);
    side(a, b, p) >= 0.0 && side(b, c, p) >= 0.0 && side(c, a, p) >= 0.0
}

/// Треугольники простого многоугольника отсечением ушей, тройки номеров
/// подряд, `features.triangulate` модуля. points - вершины на плоскости
/// без повтора первой, в любом обходе, без дыр и самопересечений.
fn triangulate(points: &[[f64; 2]]) -> Vec<u32> {
    let n = points.len();
    if n < 3 || !fits(n) {
        return Vec::new();
    }
    let mut order: Vec<usize> = if area2(points) > 0.0 { (0..n).collect() } else { (0..n).rev().collect() };
    let mut out = Vec::new();
    let mut guard = 0;
    while order.len() > 3 && guard < n.saturating_mul(n).saturating_mul(2) {
        guard += 1;
        let m = order.len();
        let mut cut = false;
        for i in 0..m {
            let (ia, ib, ic) = (order[(i + m - 1) % m], order[i], order[(i + 1) % m]);
            let (a, b, c) = (points[ia], points[ib], points[ic]);
            let turn = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if turn <= 0.0 || turn.is_nan() {
                continue; // вогнутая вершина или вырожденная
            }
            if order.iter().any(|j| ![ia, ib, ic].contains(j) && in_triangle(points[*j], a, b, c)) {
                continue;
            }
            out.extend([ia as u32, ib as u32, ic as u32]);
            order.remove(i);
            cut = true;
            break;
        }
        if !cut {
            break; // многоугольник неправильный, остаток не режется
        }
    }
    if order.len() == 3 {
        out.extend(order.iter().map(|i| *i as u32));
    }
    out
}

/// Тройки номеров подряд в треугольники.
fn triples(flat: &[u32]) -> Vec<[u32; 3]> {
    flat.as_chunks::<3>().0.to_vec()
}

/// Плоский многоугольник по вершинам points в ECEF - кусок сетки или None.
/// Повтор первой вершины в конце отбрасывается.
///
/// Слои проекта с отметками Z, пункт «Объёмом по отметкам Z», просьба
/// автора модуля от 15 октября 2026 года: «3D-забор» разреза Isoliner3D -
/// вертикальные многоугольники пластов. Плоскость - по нормали Ньюэлла,
/// многоугольник режется отсечением ушей в проекции на ту плоскость осей,
/// к которой нормаль ближе всего.
pub fn polygon3d(points: &[[f64; 3]], color: Rgba) -> Option<Part> {
    let mut p = points;
    if let (Some(first), Some(last), true) = (p.first(), p.last(), p.len() > 1) {
        // Повтор первой - совпадение в доле размера многоугольника.
        let mut low = [f64::INFINITY; 3];
        let mut high = [f64::NEG_INFINITY; 3];
        for q in p {
            for k in 0..3 {
                low[k] = low[k].min(q[k]);
                high[k] = high[k].max(q[k]);
            }
        }
        if norm(sub(*first, *last)) <= 1e-9 * norm(sub(high, low)) {
            p = &p[..p.len() - 1];
        }
    }
    let n = p.len();
    if n < 3 {
        return None;
    }
    let mut normal = [0.0; 3];
    for i in 0..n {
        let (a, b) = (p[i], p[(i + 1) % n]);
        normal[0] += (a[1] - b[1]) * (a[2] + b[2]);
        normal[1] += (a[2] - b[2]) * (a[0] + b[0]);
        normal[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let size = norm(normal);
    if size.is_nan() || size <= 0.0 {
        return None;
    }
    let normal = scale(normal, 1.0 / size);
    let mut drop = 0;
    for k in 1..3 {
        if normal[k].abs() > normal[drop].abs() {
            drop = k;
        }
    }
    let keep: Vec<usize> = (0..3).filter(|k| *k != drop).collect();
    let mean = scale(p.iter().fold([0.0; 3], |s, q| add(s, *q)), 1.0 / n as f64);
    let flat: Vec<[f64; 2]> = p.iter().map(|q| [q[keep[0]] - mean[keep[0]], q[keep[1]] - mean[keep[1]]]).collect();
    let indices = triples(&triangulate(&flat));
    if indices.is_empty() {
        return None;
    }
    Some(Part { positions: p.to_vec(), normals: vec![narrow(normal); n], colors: vec![color; n], indices })
}

/// Куски в одну сетку для видеокарты: центр - середина рамки. Куски без
/// треугольников не берутся, треугольник с номером вне своего куска
/// отбрасывается. None - треугольников нет.
pub fn merge<'a>(parts: impl IntoIterator<Item = &'a Part>) -> Option<Mesh> {
    let parts: Vec<&Part> = parts.into_iter().filter(|p| !p.indices.is_empty()).collect();
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for p in parts.iter().flat_map(|p| &p.positions) {
        for k in 0..3 {
            low[k] = low[k].min(p[k]);
            high[k] = high[k].max(p[k]);
        }
    }
    let center = scale(add(low, high), 0.5);
    let mut mesh = Mesh { center, vertices: Vec::new(), indices: Vec::new() };
    for part in parts {
        let offset = u32::try_from(mesh.vertices.len()).ok()?;
        let count = u32::try_from(part.positions.len()).ok()?;
        offset.checked_add(count)?;
        for (i, p) in part.positions.iter().enumerate() {
            let n = part.normals.get(i).map_or([0.0; 3], |n| wide(*n));
            let byte = |v: f64| (v * 127.0).round_ties_even().clamp(-127.0, 127.0) as i8;
            mesh.vertices.push(Vertex {
                position: narrow(sub(*p, center)),
                normal: [byte(n[0]), byte(n[1]), byte(n[2]), 0],
                color: part.colors.get(i).copied().unwrap_or([255; 4]),
            });
        }
        for t in part.indices.iter().filter(|t| t.iter().all(|i| *i < count)) {
            mesh.indices.extend(t.iter().map(|i| offset + i));
        }
    }
    if mesh.indices.is_empty() {
        return None;
    }
    Some(mesh)
}

// Растры и разрезы.

/// Высота на экране для отметки z. С рельефом - z, умноженная на масштаб,
/// как у поверхности. Рельеф выключен (scale 0) - глубина под
/// поверхностью: z - ground, ground - отметка рельефа.
pub fn display(z: f64, scale: f64, ground: f64) -> f64 {
    if scale != 0.0 { z * scale } else { z - ground }
}

/// Отметки растра в точках (x, y) его системы, билинейно ([`Grid::sample`]).
pub fn sample_grid(grid: &Grid, x: &[f64], y: &[f64]) -> Vec<f64> {
    x.iter().zip(y).map(|(x, y)| grid.sample(*x, *y)).collect()
}

/// Поверхности сверху вниз без пересечений: каждая не выше предыдущей,
/// пустое значение - мощность пласта 0.
pub fn stack(surfaces: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let mut out = surfaces.to_vec();
    for i in 1..out.len() {
        let prev = out[i - 1].clone();
        for (v, p) in out[i].iter_mut().zip(prev) {
            *v = if v.is_nan() { p } else { nan_min(*v, p) };
        }
    }
    out
}

/// Пласт и сторона по имени грида: («В», Top) у «В_top», («В», Bottom)
/// у «В подошва», (имя, None) без такого окончания.
pub fn bed_key(code: &str) -> (String, Option<Side>) {
    let text = code.trim();
    let lower = text.to_lowercase();
    let sides: [(Side, &[&str]); 2] = [(Side::Top, &TOPS), (Side::Bottom, &BOTTOMS)];
    for (side, words) in sides {
        for word in words {
            for sep in ['_', ' ', '-', '.'] {
                if lower.strip_suffix(word).is_some_and(|head| head.ends_with(sep)) {
                    let keep = text.chars().count().saturating_sub(word.chars().count() + 1);
                    let head: String = text.chars().take(keep).collect();
                    return (head.trim_matches([' ', '_', '-', '.']).to_string(), Some(side));
                }
            }
        }
    }
    (text.to_string(), None)
}

/// Заливки промежутков разреза модели между поверхностями сверху вниз:
/// (ключ, цвет) на каждую поверхность codes - промежуток под ней. Первая
/// поверхность - рельеф, её код - верхний пласт таблицы или «». Под
/// рельефом без кода и под подошвой пласта ([`bed_key`]) - порода между
/// пластами host с ключом [`HOST`], иначе - цвет пласта fill_of(код)
/// с ключом кода. Просьба автора модуля от 17 октября 2026 года - пласт
/// и промежуток под его подошвой шли одним цветом.
pub fn section_fills<S: AsRef<str>, C: Clone>(codes: &[S], mut fill_of: impl FnMut(&str) -> C, host: &C) -> Vec<(String, C)> {
    codes
        .iter()
        .map(|code| {
            let code = code.as_ref();
            if code.is_empty() || bed_key(code).1 == Some(Side::Bottom) {
                (HOST.to_string(), host.clone())
            } else {
                (code.to_string(), fill_of(code))
            }
        })
        .collect()
}

/// Линия кровли по умолчанию - цвет пласта темнее в [`LINE_SHADE`].
pub fn line_color(color: [u8; 3]) -> [u8; 3] {
    color.map(|c| (f64::from(c) * LINE_SHADE) as u8)
}

/// Стенки разреза вдоль линии: между поверхностями surfaces высот
/// на экране сверху вниз - пласты цветов colors.
pub fn fence(lats: &[f64], lons: &[f64], surfaces: &[Vec<f64>], colors: &[Rgba]) -> Vec<Option<Part>> {
    surfaces.windows(2).zip(colors).map(|(pair, color)| wall(lats, lons, &pair[0], &pair[1], *color)).collect()
}

/// Точки ломаной через шаг не больше step, с вершинами. Шаг не больше
/// нуля или не число - ломаная как есть. Всего точек не больше
/// [`DENSIFY_MAX`], дальше отрезки идут без новых точек.
pub fn densify(points: &[[f64; 2]], step: f64) -> Vec<[f64; 2]> {
    let Some(first) = points.first() else {
        return Vec::new();
    };
    let mut out = vec![*first];
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let parts = ((b[0] - a[0]).hypot(b[1] - a[1]) / step).ceil();
        let room = DENSIFY_MAX.saturating_sub(out.len()) as f64;
        let count = if parts.is_finite() && parts >= 1.0 && parts <= room { parts as usize } else { 1 };
        for k in 1..=count {
            let share = k as f64 / count as f64;
            out.push([a[0] + (b[0] - a[0]) * share, a[1] + (b[1] - a[1]) * share]);
        }
    }
    out
}

/// Наибольшее количество точек [`densify`], защита от шага, мелкого
/// против длины линии.
pub const DENSIFY_MAX: usize = 1 << 20;

/// Доля длины ломаной от начала до каждой точки, от 0 до 1. Длина - по шару
/// радиуса эллипсоида, хватает для линий в километры.
pub fn along(lats: &[f64], lons: &[f64]) -> Vec<f64> {
    let n = lats.len().min(lons.len());
    if n < 2 {
        return vec![0.0; n];
    }
    let mut run = vec![0.0; n];
    for i in 1..n {
        let (la0, la1) = (lats[i - 1].to_radians(), lats[i].to_radians());
        let dy = la1 - la0;
        let dx = (lons[i] - lons[i - 1]).to_radians() * ((la1 + la0) / 2.0).cos();
        run[i] = run[i - 1] + dx.hypot(dy);
    }
    let total = run[n - 1];
    if total > 0.0 { run.iter().map(|r| r / total).collect() } else { vec![0.0; n] }
}

/// Вертикальная стенка вдоль линии для картинки разреза. tops, bottoms -
/// высоты верха и низа на экране в точках линии, одно значение - для всех
/// точек. Картинка ложится по длине линии слева направо, верх картинки -
/// вверху стенки. None - точек меньше двух или высот не столько, сколько
/// точек.
pub fn image_wall(lats: &[f64], lons: &[f64], tops: &[f64], bottoms: &[f64]) -> Option<ImageWall> {
    let n = lats.len().min(lons.len());
    if n < 2 || !n.checked_mul(2).is_some_and(fits) {
        return None;
    }
    let spread = |v: &[f64]| -> Option<Vec<f64>> {
        match v.len() {
            1 => Some(vec![v[0]; n]),
            len if len == n => Some(v.to_vec()),
            _ => None,
        }
    };
    let (lats, lons) = (&lats[..n], &lons[..n]);
    let mut points = ecef(lats, lons, &spread(tops)?);
    points.extend(ecef(lats, lons, &spread(bottoms)?));
    let center = scale(points.iter().fold([0.0; 3], |s, p| add(s, *p)), 1.0 / points.len() as f64);
    let share = along(lats, lons);
    let vertices = points
        .iter()
        .enumerate()
        .map(|(i, p)| ImageVertex {
            position: narrow(sub(*p, center)),
            uv: [share[i % n] as f32, if i < n { 0.0 } else { 1.0 }],
        })
        .collect();
    let m = n as u32;
    let indices = (0..m - 1).flat_map(|i| [i, i + m, i + 1, i + 1, i + m, i + m + 1]).collect();
    Some(ImageWall { center, vertices, indices })
}

/// Точки (lats, lons) на разрезе вдоль линии с точками (line_lats,
/// line_lons) и расстояниями distance от начала: (расстояние вдоль линии
/// до ближайшей точки линии, удаление от неё) в единицах distance
/// и метрах. Шаг точек линии меньше полосы разреза. Пустая линия - пустой
/// список.
pub fn to_line(line_lats: &[f64], line_lons: &[f64], distance: &[f64], lats: &[f64], lons: &[f64]) -> Vec<(f64, f64)> {
    let dir = |la: f64, lo: f64| {
        let (sl, cl) = la.to_radians().sin_cos();
        let (so, co) = lo.to_radians().sin_cos();
        [cl * co, cl * so, sl]
    };
    let line: Vec<V3> = line_lats.iter().zip(line_lons).map(|(la, lo)| dir(*la, *lo)).collect();
    if line.is_empty() {
        return Vec::new();
    }
    let radius = (2.0 * A + B) / 3.0;
    lats.iter()
        .zip(lons)
        .map(|(la, lo)| {
            let p = dir(*la, *lo);
            let mut best = (0, f64::NEG_INFINITY);
            for (i, q) in line.iter().enumerate() {
                let d = dot(p, *q);
                if d > best.1 {
                    best = (i, d);
                }
            }
            let gap = best.1.clamp(-1.0, 1.0).acos();
            (distance.get(best.0).copied().unwrap_or(f64::NAN), gap * radius)
        })
        .collect()
}

/// Часть объекта с Z на разрезе вдоль линии: точки part - широта,
/// долгота, отметка - переносятся на линию ([`to_line`]). Это стенка
/// «3D-забора» вдоль линии или линия с отметками. Возвращает (расстояния,
/// отметки) или None, если точка части дальше band / 2 от линии. Стенка
/// поперёк линии ложится на разрез тонкой полосой. Расстояние - до
/// ближайшей точки линии, точность - шаг её точек.
pub fn fence_part(
    part: &[[f64; 3]],
    line_lats: &[f64],
    line_lons: &[f64],
    distance: &[f64],
    band: f64,
) -> Option<(Vec<f64>, Vec<f64>)> {
    if part.len() < 2 {
        return None;
    }
    let lats: Vec<f64> = part.iter().map(|p| p[0]).collect();
    let lons: Vec<f64> = part.iter().map(|p| p[1]).collect();
    let on = to_line(line_lats, line_lons, distance, &lats, &lons);
    if on.len() != part.len() || on.iter().any(|(_, gap)| *gap > band / 2.0) {
        return None;
    }
    Some((on.iter().map(|(d, _)| *d).collect(), part.iter().map(|p| p[2]).collect()))
}

/// Ближайшая к пикселю (px, py) точка ломаной на экране: (расстояние
/// в пикселях, номер отрезка, доля t вдоль него) или None. front - точка
/// перед камерой, отрезок с концом за камерой не берётся.
pub fn nearest_on_screen(pixels: &[[f64; 2]], front: &[bool], px: f64, py: f64) -> Option<(f64, usize, f64)> {
    let ok = |i: usize| front.get(i).copied().unwrap_or(false);
    if let [p] = pixels {
        return ok(0).then(|| ((p[0] - px).hypot(p[1] - py), 0, 0.0));
    }
    let mut best: Option<(f64, usize, f64)> = None;
    for (i, pair) in pixels.windows(2).enumerate() {
        if !(ok(i) && ok(i + 1)) {
            continue;
        }
        let (a, b) = (pair[0], pair[1]);
        let d = [b[0] - a[0], b[1] - a[1]];
        let length2 = (d[0] * d[0] + d[1] * d[1]).max(1e-12);
        let t = (((px - a[0]) * d[0] + (py - a[1]) * d[1]) / length2).clamp(0.0, 1.0);
        let gap = (a[0] + t * d[0] - px).hypot(a[1] + t * d[1] - py);
        if best.is_none_or(|(g, _, _)| gap < g) {
            best = Some((gap, i, t));
        }
    }
    best
}

/// Отметка оси тоннеля: под рельефом ground на глубину, которая меняется
/// от start в начале линии до end в конце по доле длины share. Спуск
/// к порталу - линия от 0 до глубины тоннеля.
pub fn tunnel_z(ground: f64, start: f64, end: f64, share: f64) -> f64 {
    ground - (start + (end - start) * share)
}

/// Точки в касательной плоскости у середины, метры: восток, север.
fn plane(latlon: &[[f64; 2]]) -> Vec<[f64; 2]> {
    if latlon.is_empty() {
        return Vec::new();
    }
    let count = latlon.len() as f64;
    let lat0 = latlon.iter().map(|p| p[0]).sum::<f64>() / count;
    let lon0 = latlon.iter().map(|p| p[1]).sum::<f64>() / count;
    let k = lat0.to_radians().cos();
    latlon
        .iter()
        .map(|p| {
            let dlon = (p[1] - lon0 + 180.0).rem_euclid(360.0) - 180.0;
            [dlon.to_radians() * A * k, (p[0] - lat0).to_radians() * A]
        })
        .collect()
}

/// Дно выреза: многоугольник ring - широта, долгота - на высоте alt,
/// нормаль вверх.
pub fn floor_part(ring: &[[f64; 2]], alt: f64, color: Rgba) -> Option<Part> {
    let indices = triples(&triangulate(&plane(ring)));
    if indices.is_empty() {
        return None;
    }
    Some(Part {
        positions: ring.iter().map(|p| geodetic_to_ecef(p[0], p[1], alt)).collect(),
        normals: ring.iter().map(|p| narrow(surface_normal(p[0], p[1]))).collect(),
        colors: vec![color; ring.len()],
        indices,
    })
}

/// Равномерные значения от start до stop, num штук, последнее - stop,
/// как `np.linspace`.
fn linspace(start: f64, stop: f64, num: usize) -> Vec<f64> {
    if num < 2 {
        return vec![start; num];
    }
    let step = (stop - start) / (num - 1) as f64;
    (0..num).map(|i| if i + 1 == num { stop } else { start + step * i as f64 }).collect()
}

/// Коробка фона под прозрачной землёй: треугольники в ECEF. bounds - рамка
/// (юг, север, запад, восток), верх - рельеф top_of(широты, долготы)
/// в узлах сетки steps × steps, низ - высота low. Фон рисуется только
/// внутри её силуэта, поэтому небо и космос вокруг модели остаются своими.
/// Первый вариант модуля заливал весь кадр, и из космоса небо стало
/// светлым, замечание автора от 15 октября 2026 года.
///
/// Верх в узле - наибольший рельеф соседних клеток по dense пробам
/// на клетку. С рельефом только в узлах холм между ними поднимался над
/// коробкой, и сквозь прозрачную землю холма над дальней стенкой блока
/// была видна чёрная полоса. steps 0 - коробки нет, dense 0 считается 1.
pub fn backdrop_box(
    bounds: [f64; 4],
    top_of: impl FnOnce(&[f64], &[f64]) -> Vec<f64>,
    low: f64,
    steps: usize,
    dense: usize,
) -> Vec<[[f64; 3]; 3]> {
    let [south, north, west, east] = bounds;
    let dense = dense.max(1);
    let Some(side) = steps.checked_mul(dense).and_then(|s| s.checked_add(1)) else {
        return Vec::new();
    };
    if steps == 0 || side.checked_mul(side).is_none() {
        return Vec::new();
    }
    let (fine_lat, fine_lon) = (linspace(south, north, side), linspace(west, east, side));
    let mut lats = Vec::new();
    let mut lons = Vec::new();
    for la in &fine_lat {
        for lo in &fine_lon {
            lats.push(*la);
            lons.push(*lo);
        }
    }
    let fine = top_of(&lats, &lons);
    let height = |r: usize, c: usize| fine.get(r * side + c).copied().filter(|v| v.is_finite()).unwrap_or(f64::NEG_INFINITY);
    let (node_lat, node_lon) = (linspace(south, north, steps + 1), linspace(west, east, steps + 1));
    let nodes = steps + 1;
    let mut top = Vec::new();
    let mut bottom = Vec::new();
    for (i, lat) in node_lat.iter().enumerate() {
        for (j, lon) in node_lon.iter().enumerate() {
            let rows = i.saturating_sub(1) * dense..((i + 1) * dense + 1).min(side);
            let cols = j.saturating_sub(1) * dense..((j + 1) * dense + 1).min(side);
            let mut best = f64::NEG_INFINITY;
            for r in rows {
                for c in cols.clone() {
                    best = best.max(height(r, c));
                }
            }
            let h = if best.is_finite() { best } else { 0.0 };
            top.push(geodetic_to_ecef(*lat, *lon, h));
            bottom.push(geodetic_to_ecef(*lat, *lon, low));
        }
    }
    let at = |g: &[V3], i: usize, j: usize| g[i * nodes + j];
    let mut out = Vec::new();
    // Четырёхугольники a-b-c-d - по два треугольника: сначала все a-b-c,
    // потом все a-c-d, как у модуля.
    for grid in [&top, &bottom] {
        let cells: Vec<(usize, usize)> = (0..steps).flat_map(|i| (0..steps).map(move |j| (i, j))).collect();
        out.extend(cells.iter().map(|&(i, j)| [at(grid, i, j), at(grid, i + 1, j), at(grid, i + 1, j + 1)]));
        out.extend(cells.iter().map(|&(i, j)| [at(grid, i, j), at(grid, i + 1, j + 1), at(grid, i, j + 1)]));
    }
    // Стенки: юг, север, запад, восток.
    for wall in 0..4 {
        let edge = |k: usize| match wall {
            0 => (0, k),
            1 => (steps, k),
            2 => (k, 0),
            _ => (k, steps),
        };
        let t = |k: usize| {
            let (i, j) = edge(k);
            at(&top, i, j)
        };
        let b = |k: usize| {
            let (i, j) = edge(k);
            at(&bottom, i, j)
        };
        out.extend((0..steps).map(|k| [t(k), t(k + 1), b(k + 1)]));
        out.extend((0..steps).map(|k| [t(k), b(k + 1), b(k)]));
    }
    out
}

/// Признак «внутри многоугольника» для точек. ring - широта, долгота, без
/// повтора первой точки. Чётность пересечений луча.
pub fn inside(lats: &[f64], lons: &[f64], ring: &[[f64; 2]]) -> Vec<bool> {
    let n = ring.len();
    lats.iter()
        .zip(lons)
        .map(|(lat, lon)| {
            let mut result = false;
            for i in 0..n {
                let ([a, b], [c, d]) = (ring[i], ring[(i + 1) % n]);
                if (a > *lat) != (c > *lat) {
                    let x = b + (lat - a) * (d - b) / if c != a { c - a } else { 1.0 };
                    if *lon < x {
                        result = !result;
                    }
                }
            }
            result
        })
        .collect()
}
