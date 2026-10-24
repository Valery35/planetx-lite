// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Маршрут по дорогам из векторных тайлов OpenFreeMap. Перенос
//! `core/routing.py` модуля.
//!
//! Дороги - слой transportation схемы OpenMapTiles в тайлах уровня
//! [`LEVEL`], только там есть грунтовки и тропы. Подпись «OpenFreeMap
//! © OpenMapTiles Data from OpenStreetMap» обязательна.
//!
//! Координаты - целые единицы мировой сетки уровня [`LEVEL`] (тайл ×
//! [`EXTENT`] + точка в тайле). У тайлов одного уровня сетка общая,
//! вершины соседних тайлов совпадают точно, поэтому узел графа - просто
//! одинаковая точка. Генератор тайлов режет линии по краю тайла с запасом
//! и упрощает их, часть примыканий остаётся без общей вершины. Конец
//! линии, который лежит ближе [`SNAP`] метров к отрезку другой линии того
//! же уровня (layer, мост или тоннель), встаёт в этот отрезок новой
//! вершиной. Пересечения отрезков без общей вершины узлом не считаются -
//! так мост не становится перекрёстком.
//!
//! Путь ищет A* по времени в пути. Скорость - по классу дороги,
//! у пешехода одна. Поиск частями - [`Search::step`], его гоняет окно
//! в цикле событий, страница WebAssembly потоков не имеет.
//!
//! Дальние маршруты строит сервис OSRM ([`service_url`],
//! [`parse_service`]). По умолчанию - сервер FOSSGIS
//! routing.openstreetmap.de: не больше запроса в секунду, User-Agent
//! программы, подпись OpenStreetMap и ссылка «сообщить об ошибке».
//! Расчёт по тайлам остаётся запасным без сети, до [`MAX_DISTANCE`].

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet};

use crate::mvt::{self, Geometry, Value};

/// Уровень тайлов дорог.
pub const LEVEL: u32 = 14;
/// Размер тайла в единицах сетки.
pub const EXTENT: i64 = 4096;
/// Единиц сетки на окружность мира.
pub const WORLD: f64 = (1u64 << LEVEL) as f64 * EXTENT as f64;
/// Допуск, с которым конец линии встаёт в чужой отрезок, метры.
pub const SNAP: f64 = 3.0;
/// Клетка решётки поиска концов у чужих отрезков, единиц сетки. Отрезок
/// записан во все клетки своей рамки. Новых вершин по длинным отрезкам
/// нет: вершина на ровной сетке совпала бы у двух пересекающихся дорог
/// и сделала бы из моста перекрёсток.
pub const CELL: i64 = 64;
/// Отрезок с рамкой больше стольких клеток в решётку не пишется,
/// он проверяется у каждого конца. Так враждебный тайл с отрезком через
/// полмира не занимает память решёткой.
const LONG_CELLS: i64 = 1 << 16;
/// До точки дороги от точки маршрута не дальше, метры.
pub const REACH: f64 = 1000.0;
/// Предел длины маршрута по прямой, метры.
pub const MAX_DISTANCE: f64 = 50000.0;
/// Полуширина коридора тайлов вокруг прямой - доля длины, но не меньше
/// [`CORRIDOR_MIN`] метров. Без пути коридор расширяется вдвое, до
/// [`WIDEN`] раз. Тайлов не больше [`MAX_TILES`].
pub const CORRIDOR_SHARE: f64 = 0.15;
pub const CORRIDOR_MIN: f64 = 1500.0;
pub const WIDEN: u32 = 2;
pub const MAX_TILES: usize = 600;
/// Скорости машины по классу дороги, км/ч. Величины помощника модуля,
/// утверждает автор.
pub const CAR_SPEED: [(&str, f64); 9] = [
    ("motorway", 90.0),
    ("trunk", 80.0),
    ("primary", 60.0),
    ("secondary", 50.0),
    ("tertiary", 40.0),
    ("minor", 30.0),
    ("service", 15.0),
    ("track", 15.0),
    ("busway", 0.0),
];
/// Скорость пешехода и велосипеда, км/ч.
pub const FOOT_SPEED: f64 = 5.0;
pub const BIKE_SPEED: f64 = 15.0;
/// Классы, по которым пешеход не идёт.
pub const FOOT_NO: [&str; 6] = ["motorway", "rail", "transit", "ferry", "raceway", "bus_guideway"];
/// Классы, по которым велосипед не едет: те же и trunk.
pub const BIKE_NO: [&str; 7] = ["motorway", "rail", "transit", "ferry", "raceway", "bus_guideway", "trunk"];
/// Конец маршрута ложится на дорогу связной сети: от отрезка вперёд
/// и назад достижимо не меньше CONNECTED узлов. Иначе точка у Кремля
/// садилась на его внутренние дороги, 37 узлов без выезда, и маршрут
/// не находился. Кандидатов не больше CANDIDATES ближайших.
pub const CONNECTED: usize = 300;
pub const CANDIDATES: usize = 200;
/// Сервис маршрутов OSRM: шаблон адреса с {mode} - профиль (car, bike,
/// foot) и {coords} - «долгота,широта;долгота,широта».
pub const ROUTER_URL: &str = "https://routing.openstreetmap.de/routed-{mode}/route/v1/driving/{coords}";
pub const ROUTER_TERMS: &str = "https://www.fossgis.de/arbeitsgruppen/osm-server/nutzungsbedingungen/";
pub const FIX_MAP: &str = "https://www.openstreetmap.org/fixthemap";
/// Секунд между запросами к сервису, условие FOSSGIS.
pub const SERVICE_GAP: f64 = 1.0;
/// Узлов A* за шаг поиска в цикле событий.
pub const STEP_POPS: usize = 3000;
/// Оценка A* слегка занижена: масштаб Меркатора меняется с широтой.
pub const HEURISTIC: f64 = 0.97;

/// Большая полуось и малая полуось эллипсоида WGS84, метры.
const A: f64 = 6378137.0;
const B: f64 = A * (1.0 - 1.0 / 298.257223563);
/// Предел широты сетки Меркатора, градусы.
const MAX_LAT: f64 = 85.05112878;

/// Тайл: уровень, x, y.
pub type TileKey = (u32, u32, u32);
/// Точка: широта и долгота в градусах.
pub type LatLon = (f64, f64);
/// Точка мировой сетки в целых единицах.
type Point = (i64, i64);

/// Способ передвижения.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Car,
    Bike,
    Foot,
}

/// Способы по порядку, как `MODES` модуля.
pub const MODES: [Mode; 3] = [Mode::Car, Mode::Bike, Mode::Foot];

impl Mode {
    /// Имя способа - профиль сервиса OSRM.
    pub fn name(self) -> &'static str {
        match self {
            Self::Car => "car",
            Self::Bike => "bike",
            Self::Foot => "foot",
        }
    }

    /// Способ по имени или None.
    pub fn from_name(name: &str) -> Option<Self> {
        MODES.into_iter().find(|m| m.name() == name)
    }

    /// Наибольшая скорость способа, м/с, для оценки A*.
    fn top(self) -> f64 {
        let kmh = match self {
            Self::Car => CAR_SPEED.iter().map(|(_, v)| *v).fold(0.0, f64::max),
            Self::Bike => BIKE_SPEED,
            Self::Foot => FOOT_SPEED,
        };
        kmh / 3.6
    }
}

/// Линия дороги из тайла. points - вершины в единицах мировой сетки.
/// cls - класс OpenMapTiles. oneway - 1 по ходу линии, -1 против, 0 в обе
/// стороны. brunnel - «bridge», «tunnel», «ford» или None. layer -
/// уровень, 0 без него. access, foot - значения тегов или None.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Line {
    pub points: Vec<(i64, i64)>,
    pub cls: Option<String>,
    pub oneway: i8,
    pub brunnel: Option<String>,
    pub layer: i64,
    pub access: Option<String>,
    pub foot: Option<String>,
}

/// Маршрут: точки (широта, долгота), длина в метрах, время в пути
/// в секундах.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Route {
    pub points: Vec<LatLon>,
    pub length: f64,
    pub seconds: f64,
}

// Тайлы и координаты.

/// Широта и долгота в единицы мировой сетки.
pub fn to_grid(lat: f64, lon: f64) -> (f64, f64) {
    let lat = lat.clamp(-MAX_LAT, MAX_LAT);
    let x = (lon + 180.0) / 360.0 * WORLD;
    let y = (1.0 - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2.0 * WORLD;
    (x, y)
}

/// Единицы мировой сетки в широту и долготу.
pub fn to_latlon(x: f64, y: f64) -> LatLon {
    let lon = x / WORLD * 360.0 - 180.0;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * y / WORLD)).sinh().atan().to_degrees();
    (lat, lon)
}

/// Метров в единице сетки на широте lat, вдоль параллели.
pub fn unit_metres(lat: f64) -> f64 {
    2.0 * std::f64::consts::PI * A * lat.to_radians().cos() / WORLD
}

/// Длина по большому кругу шара среднего радиуса, метры. На отрезках
/// дорог в десятки метров отличие от эллипсоида - доли процента.
pub fn metres(a: LatLon, b: LatLon) -> f64 {
    let radius = (2.0 * A + B) / 3.0;
    let (p1, p2) = (a.0.to_radians(), b.0.to_radians());
    let dl = (b.1 - a.1).to_radians();
    let h = ((p2 - p1) / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * radius * h.clamp(0.0, 1.0).sqrt().asin()
}

/// Расстояние от точки p до отрезка a-b, единицы сетки.
fn point_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    project(p, a, b).1
}

/// Доля вдоль отрезка a-b ближайшей к p точки и расстояние до неё.
fn project(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let d = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * d.0 + (p.1 - a.1) * d.1) / (d.0 * d.0 + d.1 * d.1).max(1e-12)).clamp(0.0, 1.0);
    (t, (p.0 - a.0 - t * d.0).hypot(p.1 - a.1 - t * d.1))
}

/// Пределы по x точек строки y, лежащих не дальше r от отрезка a-b, или
/// None. Окрестность отрезка выпукла, поэтому пределы - объединение
/// пределов кругов у концов и полосы вдоль отрезка.
fn row_span(y: f64, a: (f64, f64), b: (f64, f64), r: f64) -> Option<(f64, f64)> {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in [a, b] {
        let h = r * r - (y - p.1) * (y - p.1);
        if h >= 0.0 {
            lo = lo.min(p.0 - h.sqrt());
            hi = hi.max(p.0 + h.sqrt());
        }
    }
    let d = (b.0 - a.0, b.1 - a.1);
    let len = d.0.hypot(d.1);
    if len > 0.0 {
        let u = (d.0 / len, d.1 / len);
        // Проекция на отрезок от 0 до len, удаление от прямой не больше r:
        // оба условия линейны по x.
        let mut span = (f64::NEG_INFINITY, f64::INFINITY);
        for (coef, rest, min, max) in [(u.0, (y - a.1) * u.1, 0.0, len), (u.1, -(y - a.1) * u.0, -r, r)] {
            if coef.abs() < 1e-12 {
                if rest < min || rest > max {
                    span = (f64::INFINITY, f64::NEG_INFINITY);
                }
                continue;
            }
            let (p, q) = ((min - rest) / coef + a.0, (max - rest) / coef + a.0);
            span = (span.0.max(p.min(q)), span.1.min(p.max(q)));
        }
        if span.0 <= span.1 {
            lo = lo.min(span.0);
            hi = hi.max(span.1);
        }
    }
    (lo <= hi).then_some((lo, hi))
}

/// Ключи тайлов уровня [`LEVEL`], центр которых ближе half метров
/// к отрезку от точки a до точки b, по сетке Меркатора. Тайлы, задетые
/// самими точками, входят всегда. Ключи сортированы.
pub fn corridor(a: LatLon, b: LatLon, half: f64) -> Vec<TileKey> {
    let (ax, ay) = to_grid(a.0, a.1);
    let (bx, by) = to_grid(b.0, b.1);
    let e = EXTENT as f64;
    let n = 1i64 << LEVEL;
    let per = unit_metres(0.5 * (a.0 + b.0));
    let reach = half / per + e;
    let tile = |v: f64| (v / e).floor() as i64;
    // Шире мира по x коридор не ищется: у полюса и при огромном half
    // перебор не кончился бы.
    let x0 = tile(ax.min(bx) - reach).max(tile(ax.min(bx)) - n);
    let x1 = tile(ax.max(bx) + reach).min(tile(ax.max(bx)) + n);
    let y0 = tile(ay.min(by) - reach).max(0);
    let y1 = tile(ay.max(by) + reach).min(n - 1);
    let limit = half / per + e * 0.75;
    let key = |x: i64, y: i64| (LEVEL, u32::try_from(x.rem_euclid(n)).unwrap_or(0), u32::try_from(y.clamp(0, n - 1)).unwrap_or(0));
    let mut keys = BTreeSet::new();
    for y in y0..=y1 {
        // Перебор только по полосе строки у отрезка, с запасом в тайл
        // на округление, проверка - по расстоянию, как у модуля.
        let Some((lo, hi)) = row_span((y as f64 + 0.5) * e, (ax, ay), (bx, by), limit) else {
            continue;
        };
        let from = x0.max((lo / e - 0.5).floor() as i64 - 1);
        let to = x1.min((hi / e - 0.5).ceil() as i64 + 1);
        for x in from..=to {
            let c = ((x as f64 + 0.5) * e, (y as f64 + 0.5) * e);
            if point_segment(c, (ax, ay), (bx, by)) <= limit {
                keys.insert(key(x, y));
            }
        }
    }
    for (x, y) in [(ax, ay), (bx, by)] {
        keys.insert(key(tile(x), tile(y)));
    }
    keys.into_iter().collect()
}

/// Имена нужных атрибутов слоя transportation.
const WANTED: [&str; 6] = ["class", "oneway", "brunnel", "layer", "access", "foot"];

fn text(attrs: &HashMap<String, Value>, key: &str) -> Option<String> {
    attrs.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Числовое значение атрибута, логическое - 0 или 1, строка - разбором.
fn number(attrs: &HashMap<String, Value>, key: &str) -> f64 {
    match attrs.get(key) {
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Str(s)) => s.trim().parse::<f64>().unwrap_or(0.0),
        Some(v) => v.as_f64().unwrap_or(0.0),
        None => 0.0,
    }
}

/// Линии дорог тайла key. Тайл менее подробного уровня или с другим
/// extent пересчитывается в сетку уровня [`LEVEL`]. Тайл считается
/// враждебным.
pub fn decode(key: TileKey, data: &[u8]) -> Result<Vec<Line>, String> {
    let (z, tx, ty) = key;
    if z > LEVEL {
        return Err(format!("уровень тайла {z} подробнее {LEVEL}"));
    }
    let names = HashSet::from(["transportation"]);
    let wanted: HashSet<String> = WANTED.iter().map(|s| s.to_string()).collect();
    let mut layers = mvt::decode_shapes(data, &names, Some(&wanted)).map_err(|e| e.to_string())?;
    let Some(layer) = layers.remove("transportation") else {
        return Ok(Vec::new());
    };
    if layer.extent == 0 {
        return Err("extent 0".to_string());
    }
    let extent = f64::from(layer.extent);
    let scale = EXTENT as f64 * f64::from(1u32 << (LEVEL - z)) / extent;
    let (ox, oy) = (f64::from(tx) * extent, f64::from(ty) * extent);
    let mut out = Vec::new();
    for f in &layer.features {
        if f.geometry != Geometry::Line {
            continue;
        }
        let way = number(&f.attrs, "oneway");
        let oneway = if way == 1.0 {
            1
        } else if way == -1.0 {
            -1
        } else {
            0
        };
        for part in f.parts.iter().filter(|p| p.len() > 1) {
            // Округление numpy rint - к чётному.
            let points = part
                .iter()
                .map(|(x, y)| (((*x as f64 + ox) * scale).round_ties_even() as i64, ((*y as f64 + oy) * scale).round_ties_even() as i64))
                .collect();
            out.push(Line {
                points,
                cls: text(&f.attrs, "class"),
                oneway,
                brunnel: text(&f.attrs, "brunnel"),
                layer: number(&f.attrs, "layer").trunc() as i64,
                access: text(&f.attrs, "access"),
                foot: text(&f.attrs, "foot"),
            });
        }
    }
    Ok(out)
}

// Граф.

/// Скорость по линии, м/с, 0 - по линии нельзя.
pub fn speed(line: &Line, mode: Mode) -> f64 {
    let cls = line.cls.as_deref().unwrap_or("");
    if cls.ends_with("_construction") {
        return 0.0;
    }
    match mode {
        Mode::Foot => {
            if FOOT_NO.contains(&cls) || line.foot.as_deref() == Some("no") {
                0.0
            } else {
                FOOT_SPEED / 3.6
            }
        }
        Mode::Bike => {
            if BIKE_NO.contains(&cls) {
                0.0
            } else {
                BIKE_SPEED / 3.6
            }
        }
        Mode::Car => {
            if line.access.as_deref() == Some("no") {
                return 0.0;
            }
            CAR_SPEED.iter().find(|(c, _)| *c == cls).map_or(0.0, |(_, v)| *v) / 3.6
        }
    }
}

/// Граф дорог. Узлы - точки сетки, дуги со временем в секундах и длиной
/// в метрах сортированы по началу. Отрезки - для привязки точек
/// маршрута: концы (номера узлов), время вперёд и назад (бесконечность -
/// нельзя) и длина.
#[derive(Debug, Clone, PartialEq)]
pub struct Graph {
    nodes: Vec<(f64, f64)>,
    start: Vec<usize>,
    target: Vec<usize>,
    weight: Vec<f64>,
    length: Vec<f64>,
    /// Первая дуга узла k - first[k], у последнего узла - first[n].
    first: Vec<usize>,
    /// Дуги, сортированные по концу: начала и первая дуга узла.
    back_source: Vec<usize>,
    back_first: Vec<usize>,
    seg_a: Vec<usize>,
    seg_b: Vec<usize>,
    seg_cost: Vec<f64>,
    seg_back: Vec<f64>,
    seg_len: Vec<f64>,
}

impl Graph {
    /// Узлы в единицах мировой сетки.
    pub fn nodes(&self) -> &[(f64, f64)] {
        &self.nodes
    }

    /// Количество дуг.
    pub fn arcs(&self) -> usize {
        self.start.len()
    }

    /// Количество отрезков.
    pub fn segments(&self) -> usize {
        self.seg_a.len()
    }

    /// Дуги из узла: конец, время, длина.
    fn out_arcs(&self, node: usize) -> impl Iterator<Item = (usize, f64, f64)> + '_ {
        let lo = self.first.get(node).copied().unwrap_or(0);
        let hi = self.first.get(node + 1).copied().unwrap_or(lo);
        (lo..hi).filter_map(|i| Some((*self.target.get(i)?, *self.weight.get(i)?, *self.length.get(i)?)))
    }

    /// Начала дуг в узел.
    fn in_arcs(&self, node: usize) -> &[usize] {
        let lo = self.back_first.get(node).copied().unwrap_or(0);
        let hi = self.back_first.get(node + 1).copied().unwrap_or(lo);
        self.back_source.get(lo..hi).unwrap_or(&[])
    }

    fn node(&self, i: usize) -> (f64, f64) {
        self.nodes.get(i).copied().unwrap_or_default()
    }
}

/// Отрезок до графа: концы, скорость, направление, мост, уровень, номер
/// линии.
#[derive(Debug, Clone, Copy)]
struct Seg {
    a: Point,
    b: Point,
    speed: f64,
    way: i8,
    brunnel: u8,
    level: i64,
    line: usize,
}

fn brunnel_code(value: Option<&str>) -> u8 {
    match value {
        Some("bridge") => 1,
        Some("tunnel") => 2,
        _ => 0,
    }
}

/// Номера по возрастанию key: первая позиция каждого значения 0..=n
/// в сортированном списке, как searchsorted.
fn offsets(sorted: &[usize], n: usize) -> Vec<usize> {
    let mut first = vec![0; n + 1];
    for &k in sorted {
        if let Some(slot) = first.get_mut(k + 1) {
            *slot += 1;
        }
    }
    for k in 1..=n {
        first[k] += first[k - 1];
    }
    first
}

/// Граф из линий или None, если дорог для mode нет. Расчёт целиком,
/// без шагов: на тайлах коридора он короче кадра поиска пути.
pub fn build(lines: &[Line], mode: Mode) -> Option<Graph> {
    let mut segs = Vec::new();
    let mut number = 0;
    for ln in lines {
        let s = speed(ln, mode);
        if s <= 0.0 {
            continue;
        }
        let way = if mode == Mode::Car { ln.oneway } else { 0 };
        let brunnel = brunnel_code(ln.brunnel.as_deref());
        for w in ln.points.windows(2) {
            if w[0] != w[1] {
                segs.push(Seg { a: w[0], b: w[1], speed: s, way, brunnel, level: ln.layer, line: number });
            }
        }
        number += 1;
    }
    let segs = snap(segs);
    if segs.is_empty() {
        return None;
    }
    // Узлы - разные точки по возрастанию, как numpy.unique.
    let mut index: BTreeMap<Point, usize> = BTreeMap::new();
    for s in &segs {
        index.insert(s.a, 0);
        index.insert(s.b, 0);
    }
    for (i, v) in index.values_mut().enumerate() {
        *v = i;
    }
    let nodes: Vec<(f64, f64)> = index.keys().map(|p| (p.0 as f64, p.1 as f64)).collect();
    let n = nodes.len();
    let place = |p: &Point| index.get(p).copied().unwrap_or(0);
    let (mut seg_a, mut seg_b, mut seg_cost, mut seg_back, mut seg_len) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut forward_arcs = Vec::new();
    let mut backward_arcs = Vec::new();
    for s in &segs {
        let (u, v) = (place(&s.a), place(&s.b));
        let (pu, pv) = (nodes[u], nodes[v]);
        let length = metres(to_latlon(pu.0, pu.1), to_latlon(pv.0, pv.1));
        let cost = length / s.speed;
        let (forward, backward) = (s.way >= 0, s.way <= 0);
        if forward {
            forward_arcs.push((u, v, cost, length));
        }
        if backward {
            backward_arcs.push((v, u, cost, length));
        }
        seg_a.push(u);
        seg_b.push(v);
        seg_cost.push(if forward { cost } else { f64::INFINITY });
        seg_back.push(if backward { cost } else { f64::INFINITY });
        seg_len.push(length);
    }
    let mut arcs = forward_arcs;
    arcs.extend(backward_arcs);
    // Сортировка устойчивая, как argsort kind="stable".
    arcs.sort_by_key(|arc| arc.0);
    let start: Vec<usize> = arcs.iter().map(|a| a.0).collect();
    let target: Vec<usize> = arcs.iter().map(|a| a.1).collect();
    let mut back: Vec<(usize, usize)> = arcs.iter().map(|a| (a.1, a.0)).collect();
    back.sort_by_key(|arc| arc.0);
    let back_target: Vec<usize> = back.iter().map(|a| a.0).collect();
    Some(Graph {
        first: offsets(&start, n),
        back_first: offsets(&back_target, n),
        back_source: back.iter().map(|a| a.1).collect(),
        nodes,
        start,
        target,
        weight: arcs.iter().map(|a| a.2).collect(),
        length: arcs.iter().map(|a| a.3).collect(),
        seg_a,
        seg_b,
        seg_cost,
        seg_back,
        seg_len,
    })
}

fn as_f64(p: Point) -> (f64, f64) {
    (p.0 as f64, p.1 as f64)
}

/// Концы линий, лежащие у чужого отрезка ближе [`SNAP`], - в этот
/// отрезок вершиной, отрезок делится в этой точке. Конец у вершины
/// чужого отрезка переносится в эту вершину.
fn snap(segs: Vec<Seg>) -> Vec<Seg> {
    let Some(first) = segs.first() else {
        return segs;
    };
    let tol = SNAP / unit_metres(to_latlon(first.a.0 as f64, first.a.1 as f64).0);
    // Свободные концы - точки, которые встречаются в отрезках один раз.
    // Порядок - все начала, потом все концы, как у модуля.
    let mut counts: HashMap<Point, usize> = HashMap::new();
    for s in &segs {
        *counts.entry(s.a).or_default() += 1;
        *counts.entry(s.b).or_default() += 1;
    }
    let once = |p: &Point| counts.get(p) == Some(&1);
    let mut ends: Vec<(Point, usize)> = segs.iter().enumerate().filter(|(_, s)| once(&s.a)).map(|(i, s)| (s.a, i)).collect();
    ends.extend(segs.iter().enumerate().filter(|(_, s)| once(&s.b)).map(|(i, s)| (s.b, i)));
    if ends.is_empty() {
        return segs;
    }
    // Решётка клеток CELL: отрезок записан во все клетки рамки с допуском.
    let cell = |v: f64| (v / CELL as f64).floor() as i64;
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let mut long = Vec::new();
    for (i, s) in segs.iter().enumerate() {
        let (lx, ly) = (cell(s.a.0.min(s.b.0) as f64 - tol), cell(s.a.1.min(s.b.1) as f64 - tol));
        let (hx, hy) = (cell(s.a.0.max(s.b.0) as f64 + tol), cell(s.a.1.max(s.b.1) as f64 + tol));
        let count = (hx - lx + 1).saturating_mul(hy - ly + 1);
        if !(0..=LONG_CELLS).contains(&count) {
            long.push(i);
            continue;
        }
        for cy in ly..=hy {
            for cx in lx..=hx {
                grid.entry((cx, cy)).or_default().push(i);
            }
        }
    }
    // Конец - в ближайший отрезок другой линии того же уровня. При
    // равных расстояниях - отрезок с меньшим номером.
    let mut hits: Vec<Option<(usize, f64)>> = Vec::with_capacity(ends.len());
    for &(p, own) in &ends {
        let key = (p.0.div_euclid(CELL), p.1.div_euclid(CELL));
        let mut cands: Vec<usize> = grid.get(&key).cloned().unwrap_or_default();
        if !long.is_empty() {
            cands.extend(&long);
            cands.sort_unstable();
        }
        let me = segs[own];
        let mut best: Option<(usize, f64, f64)> = None;
        for j in cands {
            let s = segs[j];
            if s.brunnel != me.brunnel || s.level != me.level || s.line == me.line {
                continue;
            }
            let (t, dist) = project(as_f64(p), as_f64(s.a), as_f64(s.b));
            if dist <= tol && best.is_none_or(|(_, _, d)| dist < d) {
                best = Some((j, t, dist));
            }
        }
        hits.push(best.map(|(j, t, _)| (j, t)));
    }
    if hits.iter().all(Option::is_none) {
        return segs;
    }
    // Ближайшая точка - вершина отрезка: конец переносится в неё. Переносы
    // по порядку концов, каждый видит прежние, как у модуля.
    let mut a: Vec<Point> = segs.iter().map(|s| s.a).collect();
    let mut b: Vec<Point> = segs.iter().map(|s| s.b).collect();
    let mut splits: Vec<(usize, f64, Point)> = Vec::new();
    for (&(p, k), hit) in ends.iter().zip(&hits) {
        let Some((j, t)) = *hit else {
            continue;
        };
        if t <= 0.0 || t >= 1.0 {
            let q = if t <= 0.0 { a[j] } else { b[j] };
            if a[k] == p {
                a[k] = q;
            } else {
                b[k] = q;
            }
        } else {
            splits.push((j, t, p));
        }
    }
    // Деление отрезков: точки по порядку вдоль отрезка.
    splits.sort_by(|x, y| x.0.cmp(&y.0).then(x.1.total_cmp(&y.1)));
    let mut split = vec![false; segs.len()];
    for s in &splits {
        split[s.0] = true;
    }
    let mut out: Vec<Seg> = segs.iter().enumerate().filter(|(i, _)| !split[*i]).map(|(i, s)| Seg { a: a[i], b: b[i], ..*s }).collect();
    for group in splits.chunk_by(|x, y| x.0 == y.0) {
        let Some(&(j, _, _)) = group.first() else {
            continue;
        };
        let mut chain = vec![a[j]];
        chain.extend(group.iter().map(|g| g.2));
        chain.push(b[j]);
        for w in chain.windows(2) {
            out.push(Seg { a: w[0], b: w[1], ..segs[j] });
        }
    }
    out.retain(|s| s.a != s.b);
    out
}

// Поиск пути.

/// Ближайшая точка дороги связной сети графа к точке (lat, lon): номер
/// отрезка, доля вдоль него, расстояние в метрах. Отрезок островка дорог
/// без выезда пропускается, если рядом есть связный. None - дорог ближе
/// [`REACH`] нет.
pub fn attach(graph: &Graph, lat: f64, lon: f64) -> Option<(usize, f64, f64)> {
    let p = to_grid(lat, lon);
    let per = unit_metres(lat);
    let mut near: Vec<(usize, f64, f64)> = graph
        .seg_a
        .iter()
        .zip(&graph.seg_b)
        .enumerate()
        .map(|(i, (sa, sb))| {
            let (t, dist) = project(p, graph.node(*sa), graph.node(*sb));
            (i, t, dist * per)
        })
        .collect();
    near.sort_by(|x, y| x.2.total_cmp(&y.2));
    near.truncate(CANDIDATES);
    near.retain(|c| c.2 <= REACH);
    let nearest = *near.first()?;
    let mut island = HashSet::new();
    for &(i, t, dist) in &near {
        let node = graph.seg_a.get(i).copied().unwrap_or(0);
        if island.contains(&node) {
            continue;
        }
        match connected(graph, node, CONNECTED) {
            None => return Some((i, t, dist)),
            Some(seen) => island.extend(seen),
        }
    }
    Some(nearest)
}

/// None, если от узла и до него достижимо не меньше limit узлов - узел
/// в связной сети. Иначе множество узлов островка.
fn connected(graph: &Graph, node: usize, limit: usize) -> Option<HashSet<usize>> {
    for forward in [true, false] {
        let mut seen = HashSet::from([node]);
        let mut frontier = vec![node];
        while !frontier.is_empty() && seen.len() < limit {
            let mut next = Vec::new();
            for u in frontier {
                let ahead: Vec<usize> = if forward { graph.out_arcs(u).map(|a| a.0).collect() } else { graph.in_arcs(u).to_vec() };
                for v in ahead {
                    if seen.insert(v) {
                        next.push(v);
                    }
                }
            }
            frontier = next;
        }
        if seen.len() < limit {
            return Some(seen);
        }
    }
    None
}

/// Доля part времени cost по отрезку. Ноль пути - ноль времени и по
/// отрезку, где ехать нельзя: бесконечность × 0 дала бы NaN.
fn share(part: f64, cost: f64) -> f64 {
    if part <= 0.0 { 0.0 } else { part * cost }
}

/// Точки без повторов подряд.
fn clean(points: impl IntoIterator<Item = LatLon>) -> Vec<LatLon> {
    let mut out: Vec<LatLon> = Vec::new();
    for p in points {
        if out.last().is_none_or(|q| (p.0 - q.0).abs() > 1e-9 || (p.1 - q.1).abs() > 1e-9) {
            out.push(p);
        }
    }
    out
}

/// Узел очереди A*: оценка, время от старта, номер. Меньшая оценка -
/// раньше, при равенстве - меньшее время, потом меньший номер, как
/// у кортежей heapq модуля.
struct Entry {
    f: f64,
    total: f64,
    node: usize,
}

impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.f.total_cmp(&self.f).then(other.total.total_cmp(&self.total)).then(other.node.cmp(&self.node))
    }
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Entry {}

/// Состояние поиска после шага.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Поиск не кончен, нужен ещё шаг.
    Going,
    /// Поиск кончен: маршрут или None, если пути нет.
    Done(Option<Route>),
}

/// Поиск пути A* частями, замена генератора `route_steps` модуля.
/// Граф поиск не держит: каждый шаг получает тот же граф, что и
/// [`Search::new`], поэтому граф и поиск лежат рядом у окна.
pub struct Search {
    a: LatLon,
    b: LatLon,
    /// Отрезок и доля вдоль него у старта и финиша.
    start: (usize, f64),
    end: (usize, f64),
    /// Дуги от точки старта и к точке финиша: конец и время.
    extra: HashMap<usize, Vec<(usize, f64)>>,
    goal: (f64, f64),
    per_unit: f64,
    top: f64,
    best: Vec<f64>,
    came: Vec<usize>,
    done: Vec<bool>,
    heap: BinaryHeap<Entry>,
    /// Итог, если поиск кончен.
    result: Option<Option<Route>>,
}

impl Search {
    /// Поиск пути от точки a до точки b (широта, долгота) способом mode.
    pub fn new(graph: &Graph, a: LatLon, b: LatLon, mode: Mode) -> Self {
        let n = graph.nodes.len();
        let mut search = Self {
            a,
            b,
            start: (0, 0.0),
            end: (0, 0.0),
            extra: HashMap::new(),
            goal: to_grid(b.0, b.1),
            per_unit: unit_metres(0.5 * (a.0 + b.0)),
            top: mode.top(),
            best: vec![f64::INFINITY; n + 2],
            came: vec![usize::MAX; n + 2],
            done: vec![false; n + 2],
            heap: BinaryHeap::new(),
            result: None,
        };
        let (Some((si, st, _)), Some((ei, et, _))) = (attach(graph, a.0, a.1), attach(graph, b.0, b.1)) else {
            search.result = Some(None);
            return search;
        };
        search.start = (si, st);
        search.end = (ei, et);
        let (source, sink) = (n, n + 1);
        let get = |v: &[f64], i: usize| v.get(i).copied().unwrap_or(f64::INFINITY);
        let node = |v: &[usize], i: usize| v.get(i).copied().unwrap_or(0);
        // Дуги от точки старта к концам её отрезка и от концов отрезка
        // финиша к точке финиша, по доле пути вдоль отрезка.
        let (sa, sb) = (node(&graph.seg_a, si), node(&graph.seg_b, si));
        let (ea, eb) = (node(&graph.seg_a, ei), node(&graph.seg_b, ei));
        let (forward, back) = (get(&graph.seg_cost, si), get(&graph.seg_back, si));
        search.extra.entry(source).or_default().extend([(sb, share(1.0 - st, forward)), (sa, share(st, back))]);
        let (forward, back) = (get(&graph.seg_cost, ei), get(&graph.seg_back, ei));
        search.extra.entry(ea).or_default().push((sink, share(et, forward)));
        search.extra.entry(eb).or_default().push((sink, share(1.0 - et, back)));
        if si == ei {
            // Обе точки на одном отрезке: прямо по нему, если можно.
            let cost = if et >= st { (et - st) * get(&graph.seg_cost, si) } else { (st - et) * get(&graph.seg_back, si) };
            search.extra.entry(source).or_default().push((sink, cost));
        }
        search.best[source] = 0.0;
        search.heap.push(Entry { f: 0.0, total: 0.0, node: source });
        search
    }

    /// Шаг поиска: не больше pops узлов из очереди. graph - граф
    /// [`Search::new`].
    pub fn step(&mut self, graph: &Graph, pops: usize) -> Step {
        if let Some(result) = &self.result {
            return Step::Done(result.clone());
        }
        let n = graph.nodes.len();
        if self.best.len() != n + 2 {
            // Другой граф: поиск по нему не имеет смысла.
            self.result = Some(None);
            return Step::Done(None);
        }
        let (source, sink) = (n, n + 1);
        let mut count = 0;
        while let Some(Entry { total: cost, node, .. }) = self.heap.pop() {
            if self.done[node] {
                continue;
            }
            self.done[node] = true;
            if node == sink {
                break;
            }
            let mut arcs: Vec<(usize, f64)> = self.extra.get(&node).cloned().unwrap_or_default();
            if node < n {
                arcs.extend(graph.out_arcs(node).map(|(v, w, _)| (v, w)));
            }
            for (next, w) in arcs {
                if !w.is_finite() {
                    continue;
                }
                let total = cost + w;
                if total < self.best[next] {
                    self.best[next] = total;
                    self.came[next] = node;
                    let gap = if next < n {
                        let p = graph.node(next);
                        (p.0 - self.goal.0).hypot(p.1 - self.goal.1) * self.per_unit
                    } else {
                        0.0
                    };
                    self.heap.push(Entry { f: total + HEURISTIC * gap / self.top, total, node: next });
                }
            }
            count += 1;
            if count >= pops {
                return Step::Going;
            }
        }
        let result = if self.done[sink] { self.finish(graph, source, sink) } else { None };
        self.result = Some(result.clone());
        Step::Done(result)
    }

    /// Маршрут по найденной цепочке узлов.
    fn finish(&self, g: &Graph, source: usize, sink: usize) -> Option<Route> {
        let mut chain = vec![sink];
        while let Some(&last) = chain.last() {
            if last == source {
                break;
            }
            // Цепочка длиннее узлов графа - петля, пути нет.
            if chain.len() > self.came.len() {
                return None;
            }
            chain.push(*self.came.get(last)?);
        }
        chain.reverse();
        let (si, st) = self.start;
        let (ei, et) = self.end;
        let along = |i: usize, t: f64| {
            let p = g.node(g.seg_a.get(i).copied().unwrap_or(0));
            let q = g.node(g.seg_b.get(i).copied().unwrap_or(0));
            (p.0 + t * (q.0 - p.0), p.1 + t * (q.1 - p.1))
        };
        let mut xy = vec![to_grid(self.a.0, self.a.1), along(si, st)];
        xy.extend(chain.iter().skip(1).take(chain.len().saturating_sub(2)).map(|i| g.node(*i)));
        xy.push(along(ei, et));
        xy.push(to_grid(self.b.0, self.b.1));
        let points = clean(xy.into_iter().map(|(x, y)| to_latlon(x, y)));
        // Подходы от точек к дороге не входят во время, входят в длину.
        let length = points.windows(2).map(|w| metres(w[0], w[1])).sum();
        Some(Route { points, length, seconds: self.best.get(sink).copied().unwrap_or(f64::INFINITY) })
    }
}

/// Путь от точки a до точки b (широта, долгота) способом mode целиком
/// или None, если пути нет.
pub fn route(graph: &Graph, a: LatLon, b: LatLon, mode: Mode) -> Option<Route> {
    let mut search = Search::new(graph, a, b, mode);
    loop {
        if let Step::Done(result) = search.step(graph, usize::MAX) {
            return result;
        }
    }
}

// Сервис маршрутов.

/// Адрес запроса маршрута от a до b (широта, долгота) к сервису OSRM по
/// шаблону template, путь целиком в GeoJSON.
pub fn service_url(template: &str, mode: Mode, a: LatLon, b: LatLon) -> String {
    let coords = format!("{:.6},{:.6};{:.6},{:.6}", a.1, a.0, b.1, b.0);
    let url = template.replace("{mode}", mode.name()).replace("{coords}", &coords);
    let glue = if url.contains('?') { '&' } else { '?' };
    format!("{url}{glue}overview=full&geometries=geojson&steps=false")
}

/// Годится ли шаблон адреса сервиса: http(s) и {coords}.
pub fn router_ok(template: &str) -> bool {
    let template = template.trim();
    (template.starts_with("http://") || template.starts_with("https://")) && template.contains("{coords}")
}

/// Ответ OSRM байтами JSON в маршрут. Ошибка - маршрута нет, текст -
/// код ответа сервиса или «no data».
pub fn parse_service(data: &[u8]) -> Result<Route, String> {
    let json: serde_json::Value = serde_json::from_slice(data).unwrap_or_default();
    parse_service_json(&json)
}

/// Ответ OSRM, уже разобранный JSON, в маршрут, как [`parse_service`].
pub fn parse_service_json(json: &serde_json::Value) -> Result<Route, String> {
    let code = json.get("code");
    if code.and_then(serde_json::Value::as_str) != Some("Ok") {
        return Err(match code {
            Some(serde_json::Value::String(s)) if !s.is_empty() => s.clone(),
            Some(serde_json::Value::Number(v)) if v.as_f64() != Some(0.0) => v.to_string(),
            _ => "no data".to_string(),
        });
    }
    let no_route = || "NoRoute".to_string();
    let best = json.get("routes").and_then(serde_json::Value::as_array).and_then(|r| r.first()).ok_or_else(no_route)?;
    let coords = best.pointer("/geometry/coordinates").and_then(serde_json::Value::as_array).cloned().unwrap_or_default();
    let points = clean(coords.iter().filter_map(|c| {
        let c = c.as_array()?;
        Some((c.get(1)?.as_f64()?, c.first()?.as_f64()?))
    }));
    if points.len() < 2 {
        return Err(no_route());
    }
    let number = |key: &str| best.get(key).and_then(serde_json::Value::as_f64).unwrap_or(0.0);
    Ok(Route { points, length: number("distance"), seconds: number("duration") })
}
