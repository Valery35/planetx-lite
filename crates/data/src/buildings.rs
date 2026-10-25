// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! 3D-здания из слоя building векторных тайлов OpenFreeMap. Перенос
//! `core/buildings.py` модуля.
//!
//! Слой building схемы OpenMapTiles несёт контуры зданий и высоты
//! render_height и render_min_height в метрах. Высота в тайлах - высота
//! OSM, иначе этажи × 3.66 м, иначе 5 м. Отдельные здания есть с уровня
//! [`LEVEL`], на уровне 13 они слиты в кварталы без высот. Тайл
//! разбирает [`crate::mvt`].
//!
//! Работа идёт в два шага. [`footprints`] - разбор тайла, обрезка по краю
//! тайла и нарезка крыш, от рельефа не зависит, идёт в рабочем потоке.
//! [`mesh`] - вершины по высотам рельефа, её можно повторить, когда пришли
//! точнее высоты.
//!
//! Генератор тайлов сливает здания одной высоты в мультиполигон
//! и оставляет им запас за краем тайла. Кольца обрезаются по краю, стена
//! по краю тайла не ставится, остаток здания лежит в соседнем тайле. Низ
//! здания стоит на наименьшей высоте рельефа под его контуром в этом
//! тайле, крыша плоская. У здания на стыке тайлов на склоне половины
//! могут разойтись по высоте на перепад рельефа.
//!
//! Крыши режутся на треугольники в координатах тайла. Выпуклый контур
//! без дворов - веером. Прочие - отсечением ушей, двор соединяется
//! с контуром мостом.
//!
//! Вершина сетки - [`Vertex`] подземного режима, это `VERTEX` модуля.
//! Эллипсоид - WGS84 из [`crate::subsurface`].

use std::collections::{HashMap, HashSet};

use crate::mvt::{self, DecodeError, Geometry, Value};
pub use crate::subsurface::Vertex;
use crate::subsurface::{A, ecef_to_geodetic, geodetic_to_ecef, surface_normal};

/// Слой зданий в тайле.
pub const LAYER: &str = "building";
/// Уровень тайлов с отдельными зданиями.
pub const LEVEL: u32 = 14;
/// Цвет здания без поля colour. Величину назначил помощник модуля
/// 29 сентября 2026 года, её утверждает автор.
pub const DEFAULT_COLOR: [u8; 3] = [226, 222, 214];
/// Дальность показа от глаза, метры. Величина помощника модуля, автор
/// утвердил её 29 сентября 2026 года.
pub const RANGE: f64 = 6000.0;
/// Бюджет тайлов в видеокарте.
pub const MAX_TILES: usize = 32;
/// Бюджет вершин в видеокарте. Вершина - 20 байт, 1.5 млн вершин - около
/// 30 МБ.
pub const MAX_VERTICES: usize = 1_500_000;
/// Метка пустого тайла у загрузчика модуля: зданий нет. Здесь пустой тайл -
/// `Ok(None)` у [`footprints`], ошибка разбора - `Err`.
pub const EMPTY: &str = "empty";

/// Высота этажа и здания без высоты и этажей, метры, - как у генератора
/// тайлов OpenMapTiles. По ним считаются здания Overture Maps.
pub const FLOOR: f64 = 3.66;
pub const DEFAULT_HEIGHT: f64 = 5.0;

/// Атрибуты слоя, которые нужны зданиям: схемы OpenMapTiles и Overture.
const WANTED: [&str; 10] = ["render_height", "render_min_height", "hide_3d", "colour", "is_underground", "facade_color", "height", "min_height", "num_floors", "min_floor"];
/// Наибольшая широта сетки Web Mercator, градусы.
const MAX_LAT: f64 = 85.051_128_779_806_59;
/// Наибольшее количество тайлов-кандидатов у [`near_tiles`].
const MAX_KEYS: usize = 1 << 20;

/// Номер тайла: уровень, столбец, строка.
pub type TileKey = (u32, u32, u32);
type P = [f64; 2];
type V3 = [f64; 3];

/// Здание из тайла.
///
/// rings - кольца в единицах тайла без повтора первой точки. Первое -
/// контур с положительной площадью, дальше дворы с отрицательной.
/// height - высота крыши над землёй, base - высота низа, метры. color -
/// RGB или None.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Building {
    pub rings: Vec<Vec<P>>,
    pub height: f64,
    pub base: f64,
    pub color: Option<[u8; 3]>,
}

/// Здания тайла без высот рельефа.
///
/// latlon - все точки колец, широта и долгота в градусах. next - номер
/// следующей точки кольца. wall - ставится ли стена на ребре от точки
/// к следующей. owner - номер здания точки. height, base - по зданиям,
/// метры. color - по зданиям. roof - треугольники крыш, номера точек.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Footprint {
    pub key: TileKey,
    pub latlon: Vec<P>,
    pub next: Vec<usize>,
    pub wall: Vec<bool>,
    pub owner: Vec<usize>,
    pub height: Vec<f64>,
    pub base: Vec<f64>,
    pub color: Vec<[u8; 3]>,
    pub roof: Vec<u32>,
}

/// Сетка зданий тайла: центр в ECEF, вершины смещениями от него,
/// треугольники тройками номеров подряд.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Mesh {
    pub key: TileKey,
    pub center: V3,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

/// Цвет «#rrggbb» или «#rgb» в тройку 0-255, иначе None.
pub fn parse_color(value: &str) -> Option<[u8; 3]> {
    let text = value.trim().trim_start_matches('#');
    if !text.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let text: String = match text.len() {
        3 => text.chars().flat_map(|c| [c, c]).collect(),
        6 => text.to_string(),
        _ => return None,
    };
    let pair = |i: usize| text.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok());
    Some([pair(0)?, pair(2)?, pair(4)?])
}

/// Истинно ли значение атрибута, как `bool` в Python.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Str(s) => !s.is_empty(),
        Value::Float(v) => *v != 0.0,
        Value::Int(v) => *v != 0,
        Value::Bool(v) => *v,
    }
}

/// Число из атрибута, как `float(value or 0.0)` модуля. Строка, которая
/// не число, - 0.
fn number(value: Option<&Value>) -> f64 {
    match value {
        Some(Value::Float(v)) => *v,
        Some(Value::Int(v)) => *v as f64,
        Some(Value::Bool(v)) => f64::from(u8::from(*v)),
        Some(Value::Str(s)) => s.trim().parse().unwrap_or(0.0),
        None => 0.0,
    }
}

/// Высота верха и низа здания, метры. Схема OpenMapTiles - render_height
/// и render_min_height. Схема Overture Maps - height, min_height, иначе
/// этажи num_floors и min_floor по [`FLOOR`], иначе [`DEFAULT_HEIGHT`],
/// как делает генератор тайлов OpenMapTiles.
pub fn heights(attrs: &HashMap<String, Value>) -> (f64, f64) {
    let given = |k: &str| attrs.get(k).filter(|v| truthy(v));
    if attrs.contains_key("render_height") {
        return (number(given("render_height")), number(given("render_min_height")));
    }
    let height = given("height").map(|v| number(Some(v))).or_else(|| given("num_floors").map(|v| number(Some(v)) * FLOOR));
    let base = given("min_height").map(|v| number(Some(v))).or_else(|| given("min_floor").map(|v| number(Some(v)) * FLOOR));
    (height.unwrap_or(DEFAULT_HEIGHT), base.unwrap_or(0.0))
}

/// Площадь кольца со знаком по формуле трапеций.
pub fn area(ring: &[P]) -> f64 {
    let n = ring.len();
    let mut forward = 0.0;
    let mut backward = 0.0;
    for (i, p) in ring.iter().enumerate() {
        let q = ring[(i + 1) % n];
        forward += p[0] * q[1];
        backward += q[0] * p[1];
    }
    0.5 * (forward - backward)
}

/// Кольцо части многоугольника MVT без повтора первой точки и его
/// площадь. Часть без ClosePath и кольцо меньше трёх точек - None.
fn ring_of(part: &[(i64, i64)]) -> Option<(Vec<P>, f64)> {
    let (first, last) = (part.first()?, part.last()?);
    if part.len() < 4 || first != last {
        return None;
    }
    let ring: Vec<P> = part[..part.len() - 1].iter().map(|&(x, y)| [x as f64, y as f64]).collect();
    let s = area(&ring);
    Some((ring, s))
}

/// Здания тайла и extent слоя.
///
/// Здания с hide_3d пропускаются, это части, которые OSM описывает ещё
/// и целым зданием. Кольцо нулевой площади пропускается.
pub fn decode(data: &[u8]) -> Result<(Vec<Building>, u32), DecodeError> {
    let names = HashSet::from([LAYER]);
    let wanted: HashSet<String> = WANTED.iter().map(|w| w.to_string()).collect();
    let mut layers = mvt::decode_shapes(data, &names, Some(&wanted))?;
    let Some(layer) = layers.remove(LAYER) else {
        return Ok((Vec::new(), mvt::EXTENT));
    };
    let mut out = Vec::new();
    for feature in &layer.features {
        if feature.geometry != Geometry::Polygon {
            continue;
        }
        let attrs: &HashMap<String, Value> = &feature.attrs;
        if attrs.get("hide_3d").is_some_and(truthy) || attrs.get("is_underground").is_some_and(truthy) {
            continue;
        }
        let (height, base) = heights(attrs);
        if !(height.is_finite() && base.is_finite()) || height <= base {
            continue;
        }
        let color = attrs.get("colour").filter(|v| truthy(v)).or_else(|| attrs.get("facade_color")).and_then(Value::as_str).and_then(parse_color);
        let mut rings: Vec<Vec<P>> = Vec::new();
        for (ring, s) in feature.parts.iter().filter_map(|p| ring_of(p)) {
            if s > 0.0 {
                if !rings.is_empty() {
                    out.push(Building { rings: std::mem::take(&mut rings), height, base, color });
                }
                rings = vec![ring];
            } else if s < 0.0 && !rings.is_empty() {
                rings.push(ring);
            }
        }
        if !rings.is_empty() {
            out.push(Building { rings, height, base, color });
        }
    }
    Ok((out, layer.extent))
}

/// Отсечение кольца полуплоскостью, алгоритм Сазерленда-Ходжмана.
fn clip_side(points: Vec<P>, axis: usize, limit: f64, below: bool) -> Vec<P> {
    let inside: Vec<bool> = points.iter().map(|p| if below { p[axis] <= limit } else { p[axis] >= limit }).collect();
    if inside.iter().all(|v| *v) {
        return points;
    }
    if !inside.iter().any(|v| *v) {
        return Vec::new();
    }
    let n = points.len();
    let mut out = Vec::with_capacity(n + 2);
    for i in 0..n {
        let j = (i + n - 1) % n;
        let (p, q) = (points[j], points[i]);
        if inside[i] != inside[j] {
            let t = (limit - p[axis]) / (q[axis] - p[axis]);
            let mut cut = [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t];
            cut[axis] = limit;
            out.push(cut);
        }
        if inside[i] {
            out.push(q);
        }
    }
    out
}

/// Кольцо, обрезанное по квадрату тайла [0, extent], или None.
pub fn clip(ring: &[P], extent: f64) -> Option<Vec<P>> {
    let inside = ring.iter().all(|p| p[0] >= 0.0 && p[1] >= 0.0 && p[0] <= extent && p[1] <= extent);
    if inside {
        return Some(ring.to_vec());
    }
    let mut points = ring.to_vec();
    for (axis, limit, below) in [(0, 0.0, false), (0, extent, true), (1, 0.0, false), (1, extent, true)] {
        points = clip_side(points, axis, limit, below);
        if points.len() < 3 {
            return None;
        }
    }
    // Точки на углу тайла могут повториться подряд.
    let n = points.len();
    let keep: Vec<P> = (0..n).filter(|&i| points[i] != points[(i + n - 1) % n]).map(|i| points[i]).collect();
    if keep.len() < 3 || area(&keep) == 0.0 {
        return None;
    }
    Some(keep)
}

/// Удвоенная площадь треугольника abc со знаком, больше нуля - против
/// часовой стрелки.
pub fn orient(a: P, b: P, c: P) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Треугольники кольца против часовой стрелки, тройки номеров точек.
///
/// Ухо проверяется только по вогнутым вершинам, их у зданий единицы.
/// Точки, совпавшие с вершинами уха, его не закрывают, так проходят
/// мосты к дворам. Если ухо не нашлось, остаток режется веером.
pub fn ear_clip(points: &[P]) -> Vec<[usize; 3]> {
    let n = points.len();
    if n < 3 {
        return Vec::new();
    }
    let mut prev: Vec<usize> = (0..n).map(|i| if i == 0 { n - 1 } else { i - 1 }).collect();
    let mut next: Vec<usize> = (0..n).map(|i| if i + 1 == n { 0 } else { i + 1 }).collect();
    let cross = |a: usize, b: usize, c: usize| {
        let (pa, pb, pc) = (points[a], points[b], points[c]);
        (pb[0] - pa[0]) * (pc[1] - pb[1]) - (pb[1] - pa[1]) * (pc[0] - pb[0])
    };
    let mut is_reflex: Vec<bool> = (0..n).map(|i| cross(prev[i], i, next[i]) <= 0.0).collect();
    let reflex: Vec<usize> = (0..n).filter(|i| is_reflex[*i]).collect();
    let blocked = |a: usize, b: usize, c: usize, is_reflex: &[bool]| {
        let ([ax, ay], [bx, by], [cx, cy]) = (points[a], points[b], points[c]);
        let (left, right) = (ax.min(bx).min(cx), ax.max(bx).max(cx));
        let (low, high) = (ay.min(by).min(cy), ay.max(by).max(cy));
        reflex.iter().filter(|r| is_reflex[**r]).any(|&r| {
            let [px, py] = points[r];
            if px < left || px > right || py < low || py > high {
                return false;
            }
            if (px == ax && py == ay) || (px == bx && py == by) || (px == cx && py == cy) {
                return false;
            }
            (bx - ax) * (py - ay) - (by - ay) * (px - ax) >= 0.0
                && (cx - bx) * (py - by) - (cy - by) * (px - bx) >= 0.0
                && (ax - cx) * (py - cy) - (ay - cy) * (px - cx) >= 0.0
        })
    };
    let mut out = Vec::with_capacity(n - 2);
    let mut remaining = n;
    let mut i = 0;
    let mut stall = 0;
    while remaining > 3 && stall < remaining {
        let (a, c) = (prev[i], next[i]);
        if !is_reflex[i] && !blocked(a, i, c, &is_reflex) {
            out.push([a, i, c]);
            next[a] = c;
            prev[c] = a;
            remaining -= 1;
            for v in [a, c] {
                if is_reflex[v] && cross(prev[v], v, next[v]) > 0.0 {
                    is_reflex[v] = false;
                }
            }
            i = c;
            stall = 0;
        } else {
            i = next[i];
            stall += 1;
        }
    }
    let a = i;
    let mut b = next[a];
    let mut c = next[b];
    while c != a {
        out.push([a, b, c]);
        b = c;
        c = next[c];
    }
    out
}

/// Пересекает ли отрезок pq какой-нибудь из отрезков внутри или проходит
/// ли через вершину, кроме своих концов.
///
/// Мост через вершину двора ложится на его сторону, нарезка на таком
/// кольце вставала. Так было у здания в Берлине, 29 сентября 2026 года.
fn crosses(p: P, q: P, edges: &[(P, P)]) -> bool {
    let length = (q[0] - p[0]) * (q[0] - p[0]) + (q[1] - p[1]) * (q[1] - p[1]);
    edges.iter().any(|&(s, e)| {
        let d1 = orient(s, e, p);
        let d2 = orient(s, e, q);
        let d3 = orient(p, q, s);
        let d4 = orient(p, q, e);
        let along = (s[0] - p[0]) * (q[0] - p[0]) + (s[1] - p[1]) * (q[1] - p[1]);
        (d1 * d2 < 0.0 && d3 * d4 < 0.0) || (d3 == 0.0 && along > 0.0 && along < length)
    })
}

/// Стороны колец: начало и конец каждой.
fn sides(points: &[P], ring: &[usize], out: &mut Vec<(P, P)>) {
    let n = ring.len();
    for (k, &i) in ring.iter().enumerate() {
        out.push((points[i], points[ring[(k + 1) % n]]));
    }
}

/// Контур с дворами одним кольцом через мосты, номера точек.
///
/// outer - номера точек контура против часовой стрелки, holes - номера
/// точек дворов по часовой. Мост идёт от самой правой точки двора
/// к ближайшей точке кольца, если он не пересекает стороны. Двор без
/// такого моста пропускается, крыша тогда его закрывает. Номер за
/// пределами points - пустой ответ.
pub fn bridge(points: &[P], outer: &[usize], holes: &[Vec<usize>]) -> Vec<usize> {
    if outer.iter().chain(holes.iter().flatten()).any(|i| *i >= points.len()) {
        return Vec::new();
    }
    let rightmost = |hole: &[usize]| hole.iter().map(|i| points[*i][0]).fold(f64::NEG_INFINITY, f64::max);
    let mut holes: Vec<&Vec<usize>> = holes.iter().filter(|h| !h.is_empty()).collect();
    holes.sort_by(|a, b| rightmost(b).total_cmp(&rightmost(a)));
    let mut ring = outer.to_vec();
    for number in 0..holes.len() {
        let hole = holes[number];
        let mut start = 0;
        for (k, i) in hole.iter().enumerate() {
            if points[*i][0] > points[hole[start]][0] {
                start = k;
            }
        }
        let m = points[hole[start]];
        let mut edges = Vec::new();
        sides(points, &ring, &mut edges);
        for h in &holes[number..] {
            sides(points, h, &mut edges);
        }
        let distance = |i: usize| {
            let p = points[i];
            (p[0] - m[0]) * (p[0] - m[0]) + (p[1] - m[1]) * (p[1] - m[1])
        };
        let mut order: Vec<usize> = (0..ring.len()).collect();
        order.sort_by(|a, b| distance(ring[*a]).total_cmp(&distance(ring[*b])));
        for position in order {
            if !crosses(m, points[ring[position]], &edges) {
                let mut joined = Vec::with_capacity(ring.len() + hole.len() + 2);
                joined.extend_from_slice(&ring[..=position]);
                joined.extend_from_slice(&hole[start..]);
                joined.extend_from_slice(&hole[..=start]);
                joined.extend_from_slice(&ring[position..]);
                ring = joined;
                break;
            }
        }
    }
    ring
}

/// Треугольники крыши, номера точек.
///
/// rings - списки номеров точек колец здания, первое - контур. Если
/// с дворами вышел вывернутый треугольник, крыша режется по контуру без
/// дворов и закрывает их. Вывернутый треугольник лёг бы за контур здания.
pub fn roof(points: &[P], rings: &[Vec<usize>]) -> Vec<u32> {
    let Some(outer) = rings.first() else {
        return Vec::new();
    };
    if rings.len() > 1 {
        let tri = cut(points, &bridge(points, outer, &rings[1..]));
        let inverted = tri.as_chunks::<3>().0.iter().any(|t| {
            let [a, b, c] = t.map(|i| points.get(i as usize).copied().unwrap_or([f64::NAN; 2]));
            orient(a, b, c) < 0.0
        });
        if !inverted {
            return tri;
        }
    }
    cut(points, outer)
}

/// Треугольники кольца ring номерами точек points. Номер за пределами
/// points или больше u32 - пустой ответ.
pub fn cut(points: &[P], ring: &[usize]) -> Vec<u32> {
    if ring.iter().any(|i| *i >= points.len() || u32::try_from(*i).is_err()) {
        return Vec::new();
    }
    let local: Vec<P> = ring.iter().map(|i| points[*i]).collect();
    ear_clip(&local).into_iter().flatten().map(|k| ring[k] as u32).collect()
}

/// Широта в градусах по доле высоты мира t, t = 0 на северной границе.
fn lat_of_row(t: f64) -> f64 {
    (PI_F * (1.0 - 2.0 * t)).sinh().atan().to_degrees()
}

const PI_F: f64 = std::f64::consts::PI;

/// Номер тайла уровня z, в котором лежит точка. Широта за пределами
/// сетки прижимается к краю, долгота 180 попадает в последний столбец.
fn lonlat_to_tile(lat: f64, lon: f64, z: u32) -> (i64, i64) {
    let n = 1i64 << z;
    let lat = lat.clamp(-MAX_LAT, MAX_LAT);
    let x = ((lon + 180.0) / 360.0 * n as f64).floor() as i64;
    let merc = lat.to_radians().tan().asinh();
    let y = ((1.0 - merc / PI_F) / 2.0 * n as f64).floor() as i64;
    (x.clamp(0, n - 1), y.clamp(0, n - 1))
}

/// Границы тайла в градусах: запад, юг, восток, север.
fn tile_bounds(z: u32, x: u32, y: u32) -> (f64, f64, f64, f64) {
    let n = f64::from(1u32 << z.min(30));
    let west = f64::from(x) / n * 360.0 - 180.0;
    let east = (f64::from(x) + 1.0) / n * 360.0 - 180.0;
    (west, lat_of_row((f64::from(y) + 1.0) / n), east, lat_of_row(f64::from(y) / n))
}

/// Точки в единицах тайла key в широту и долготу.
pub fn tile_latlon(key: TileKey, extent: f64, xy: &[P]) -> Vec<P> {
    let (z, x, y) = key;
    let n = f64::from(1u32 << z.min(30));
    xy.iter()
        .map(|p| {
            let lon = (f64::from(x) + p[0] / extent) / n * 360.0 - 180.0;
            [lat_of_row((f64::from(y) + p[1] / extent) / n), lon]
        })
        .collect()
}

/// Здания тайла key из байтов тайла: Footprint, None без зданий, ошибка -
/// тайл не разбирается.
pub fn footprints(key: TileKey, data: &[u8]) -> Result<Option<Footprint>, DecodeError> {
    let (buildings, extent) = decode(data)?;
    if buildings.is_empty() || extent == 0 {
        return Ok(None);
    }
    let extent = f64::from(extent);
    let mut parts: Vec<Vec<Vec<P>>> = Vec::new();
    let mut kept: Vec<&Building> = Vec::new();
    for building in &buildings {
        let mut rings = Vec::new();
        for (number, ring) in building.rings.iter().enumerate() {
            match clip(ring, extent) {
                Some(r) => rings.push(r),
                None if number == 0 => break,
                None => {}
            }
        }
        if !rings.is_empty() {
            parts.push(rings);
            kept.push(building);
        }
    }
    let sizes: Vec<usize> = parts.iter().flatten().map(Vec::len).collect();
    let total: usize = sizes.iter().sum();
    if kept.is_empty() || u32::try_from(total).is_err() {
        return Ok(None);
    }
    let mut starts = Vec::with_capacity(sizes.len());
    let mut xy: Vec<P> = Vec::with_capacity(total);
    let mut owner = Vec::with_capacity(total);
    let mut next = Vec::with_capacity(total);
    let mut prev = Vec::with_capacity(total);
    let ring_count: Vec<usize> = parts.iter().map(Vec::len).collect();
    for (b, rings) in parts.iter().enumerate() {
        for ring in rings {
            let start = xy.len();
            starts.push(start);
            let n = ring.len();
            for k in 0..n {
                next.push(start + (k + 1) % n);
                prev.push(start + (k + n - 1) % n);
                owner.push(b);
            }
            xy.extend_from_slice(ring);
        }
    }
    // Стена не ставится на ребре по краю тайла.
    let wall: Vec<bool> = (0..total)
        .map(|k| {
            let (p, q) = (xy[k], xy[next[k]]);
            let border = (0..2).any(|axis| [0.0, extent].iter().any(|limit| p[axis] == *limit && q[axis] == *limit));
            !border
        })
        .collect();
    // Выпуклые контуры без дворов режутся веером.
    let hypot = |a: P, b: P| (b[0] - a[0]).hypot(b[1] - a[1]);
    let concave: Vec<bool> = starts
        .iter()
        .zip(&sizes)
        .map(|(&start, &size)| {
            (start..start + size).any(|k| {
                let (a, b, c) = (xy[prev[k]], xy[k], xy[next[k]]);
                orient(a, b, c) < -1e-9 * hypot(a, b) * hypot(b, c)
            })
        })
        .collect();
    let mut first_ring = Vec::with_capacity(kept.len());
    let mut ring = 0;
    for count in &ring_count {
        first_ring.push(ring);
        ring += count;
    }
    let fan: Vec<bool> = (0..kept.len()).map(|b| ring_count[b] == 1 && !concave[first_ring[b]]).collect();
    let mut triangles: Vec<u32> = Vec::new();
    for b in (0..kept.len()).filter(|b| fan[*b]) {
        let (start, size) = (starts[first_ring[b]], sizes[first_ring[b]]);
        for k in 0..size.saturating_sub(2) {
            triangles.extend([start, start + k + 1, start + k + 2].map(|i| i as u32));
        }
    }
    for b in (0..kept.len()).filter(|b| !fan[*b]) {
        let first = first_ring[b];
        let rings: Vec<Vec<usize>> =
            (first..first + ring_count[b]).map(|r| (starts[r]..starts[r] + sizes[r]).collect()).collect();
        triangles.extend(roof(&xy, &rings));
    }
    Ok(Some(Footprint {
        key,
        latlon: tile_latlon(key, extent, &xy),
        next,
        wall,
        owner,
        height: kept.iter().map(|b| b.height).collect(),
        base: kept.iter().map(|b| b.base).collect(),
        color: kept.iter().map(|b| b.color.unwrap_or(DEFAULT_COLOR)).collect(),
        roof: triangles,
    }))
}

/// Нормали в i8 с масштабом 127, четвёртый байт 0. Нулевой вектор - нуль.
pub fn normals(vectors: &[V3]) -> Vec<[i8; 4]> {
    vectors
        .iter()
        .map(|v| {
            let mut length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            if length == 0.0 {
                length = 1.0;
            }
            let q = |a: f64| (a / length * 127.0).round_ties_even() as i8;
            [q(v[0]), q(v[1]), q(v[2]), 0]
        })
        .collect()
}

/// Согласованы ли длины и номера записи.
fn consistent(fp: &Footprint) -> bool {
    let n = fp.latlon.len();
    let b = fp.height.len();
    fp.next.len() == n
        && fp.wall.len() == n
        && fp.owner.len() == n
        && fp.base.len() == b
        && fp.color.len() == b
        && fp.next.iter().all(|j| *j < n)
        && fp.owner.iter().all(|o| *o < b)
}

/// Меньшее из двух, NaN - если NaN хоть одно, как `np.minimum`.
fn nan_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.min(b) }
}

fn nan_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.max(b) }
}

/// Высоты рельефа точек по широтам и долготам.
pub type HeightsAt<'a> = &'a dyn Fn(&[f64], &[f64]) -> Vec<f64>;

/// Сетка зданий по высотам рельефа.
///
/// heights_at - высоты точек по широтам и долготам, None - земля
/// на эллипсоиде. Низ здания - наименьшая высота под его точками.
/// Недостающая высота - NaN, такую сетку отвергнет [`mesh_ok`]. Запись
/// с несогласованными длинами или номерами даёт пустую сетку.
pub fn mesh(fp: &Footprint, heights_at: Option<HeightsAt<'_>>) -> Mesh {
    if !consistent(fp) {
        return Mesh { key: fp.key, ..Mesh::default() };
    }
    let lat: Vec<f64> = fp.latlon.iter().map(|p| p[0]).collect();
    let lon: Vec<f64> = fp.latlon.iter().map(|p| p[1]).collect();
    let ground: Vec<f64> = match heights_at {
        None => vec![0.0; lat.len()],
        Some(f) => {
            let got = f(&lat, &lon);
            (0..lat.len()).map(|k| got.get(k).copied().unwrap_or(f64::NAN)).collect()
        }
    };
    let mut low = vec![f64::INFINITY; fp.height.len()];
    for (o, g) in fp.owner.iter().zip(&ground) {
        low[*o] = nan_min(low[*o], *g);
    }
    let point = |k: usize, rise: f64| geodetic_to_ecef(lat[k], lon[k], low[fp.owner[k]] + rise);
    let top: Vec<V3> = (0..lat.len()).map(|k| point(k, fp.height[fp.owner[k]])).collect();
    let bottom: Vec<V3> = (0..lat.len()).map(|k| point(k, fp.base[fp.owner[k]])).collect();
    let up: Vec<V3> = (0..lat.len()).map(|k| surface_normal(lat[k], lon[k])).collect();
    let walls: Vec<usize> = (0..lat.len()).filter(|k| fp.wall[*k]).collect();
    // Кольцо контура идёт против часовой стрелки в координатах тайла, это
    // по часовой, если смотреть сверху. Наружу смотрит up × ребро.
    let outward: Vec<V3> = walls
        .iter()
        .map(|&i| {
            let (u, j) = (up[i], fp.next[i]);
            let e = [bottom[j][0] - bottom[i][0], bottom[j][1] - bottom[i][1], bottom[j][2] - bottom[i][2]];
            [u[1] * e[2] - u[2] * e[1], u[2] * e[0] - u[0] * e[2], u[0] * e[1] - u[1] * e[0]]
        })
        .collect();
    let mut positions: Vec<V3> = Vec::with_capacity(4 * walls.len() + top.len());
    for &i in &walls {
        let j = fp.next[i];
        positions.extend([bottom[i], bottom[j], top[j], top[i]]);
    }
    positions.extend_from_slice(&top);
    // Центр - середина рамки, смещения в f32.
    let mut center = [0.0; 3];
    for (a, c) in center.iter_mut().enumerate() {
        let lo = positions.iter().fold(f64::INFINITY, |m, p| nan_min(m, p[a]));
        let hi = positions.iter().fold(f64::NEG_INFINITY, |m, p| nan_max(m, p[a]));
        *c = 0.5 * (lo + hi);
    }
    let wall_normals = normals(&outward);
    let up_normals = normals(&up);
    let mut vertices = Vec::with_capacity(positions.len());
    for (k, &i) in walls.iter().enumerate() {
        let [r, g, b] = fp.color[fp.owner[i]];
        for corner in 0..4 {
            let p = positions[4 * k + corner];
            vertices.push(Vertex {
                position: [0, 1, 2].map(|a| (p[a] - center[a]) as f32),
                normal: wall_normals[k],
                color: [r, g, b, 255],
            });
        }
    }
    for (k, p) in top.iter().enumerate() {
        let [r, g, b] = fp.color[fp.owner[k]];
        vertices.push(Vertex {
            position: [0, 1, 2].map(|a| (p[a] - center[a]) as f32),
            normal: up_normals[k],
            color: [r, g, b, 255],
        });
    }
    let count = u32::try_from(walls.len()).unwrap_or(u32::MAX);
    let roof_start = count.saturating_mul(4);
    let mut indices = Vec::with_capacity(6 * walls.len() + fp.roof.len());
    for q in 0..count {
        let base = q.saturating_mul(4);
        indices.extend([0, 1, 2, 0, 2, 3].map(|d| base.saturating_add(d)));
    }
    indices.extend(fp.roof.iter().map(|i| i.saturating_add(roof_start)));
    Mesh { key: fp.key, center, vertices, indices }
}

/// Вершин в сетке тайла: по 4 на стену и по одной на точку крыши.
pub fn vertex_count(fp: &Footprint) -> usize {
    4 * fp.wall.iter().filter(|w| **w).count() + fp.latlon.len()
}

/// Тайлы зданий уровня [`LEVEL`] ближе reach к глазу, ближние первыми.
///
/// eye - глаз в ECEF, forward - единичное направление взгляда, ground -
/// высота земли под глазом. Расстояние - от глаза до ближайшей точки рамки
/// тайла на высоте ground. Тайл целиком за спиной глаза пропускается.
/// Возвращает не больше limit пар (ключ, расстояние). Модуль берёт ground,
/// reach и limit по умолчанию 0, [`RANGE`] и [`MAX_TILES`].
pub fn near_tiles(eye: V3, forward: V3, ground: f64, reach: f64, limit: usize) -> Vec<(TileKey, f64)> {
    let [lat, lon, height] = ecef_to_geodetic(eye);
    let above = height - ground;
    if above >= reach {
        return Vec::new();
    }
    let radius = (reach * reach - above.max(0.0).powi(2)).sqrt();
    let dlat = (radius / A).to_degrees();
    let dlon = dlat / lat.to_radians().cos().max(1e-6);
    if !(dlat.is_finite() && dlon.is_finite() && lat.is_finite() && lon.is_finite()) {
        return Vec::new();
    }
    let n = 1i64 << LEVEL;
    let (x0, y0) = lonlat_to_tile((lat + dlat).min(85.0), lon - dlon, LEVEL);
    let (mut x1, y1) = lonlat_to_tile((lat - dlat).max(-85.0), lon + dlon, LEVEL);
    if x1 < x0 {
        x1 += n; // через линию перемены дат
    }
    let columns = usize::try_from(x1 - x0 + 1).unwrap_or(0);
    let rows = usize::try_from(y1 - y0 + 1).unwrap_or(0);
    if columns.saturating_mul(rows) > MAX_KEYS {
        return Vec::new();
    }
    let mut found: Vec<(TileKey, f64)> = Vec::new();
    for x in x0..=x1 {
        for y in y0..=y1 {
            let (Ok(tx), Ok(ty)) = (u32::try_from(x.rem_euclid(n)), u32::try_from(y)) else {
                continue;
            };
            let (west, south, east, north) = tile_bounds(LEVEL, tx, ty);
            let near_lat = lat.max(south).min(north);
            let shift = (lon - west + 180.0).rem_euclid(360.0) - 180.0; // долгота от запада
            let near_lon = west + shift.max(0.0).min(east - west);
            let near = geodetic_to_ecef(near_lat, near_lon, ground);
            let distance = dist(near, eye);
            let middle = geodetic_to_ecef(0.5 * (south + north), 0.5 * (west + east), ground);
            let corner = geodetic_to_ecef(north, west, ground);
            let half = dist(corner, middle);
            let ahead = (0..3).map(|a| (middle[a] - eye[a]) * forward[a]).sum::<f64>();
            if distance < reach && ahead > -half {
                found.push(((LEVEL, tx, ty), distance));
            }
        }
    }
    found.sort_by(|a, b| a.1.total_cmp(&b.1));
    found.truncate(limit);
    found
}

fn dist(a: V3, b: V3) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Можно ли отдать сетку видеокарте: треугольники целые, номера вершин
/// в пределах массива, координаты конечные.
///
/// Номер за пределами массива вершин драйвер не проверяет, чтение
/// за буфером может испортить его память.
pub fn mesh_ok(mesh: &Mesh) -> bool {
    if !mesh.indices.len().is_multiple_of(3) {
        return false;
    }
    if mesh.indices.iter().any(|i| *i as usize >= mesh.vertices.len()) {
        return false;
    }
    mesh.vertices.iter().all(|v| v.position.iter().all(|p| p.is_finite()))
}
