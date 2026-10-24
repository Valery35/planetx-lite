// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Разрез Земли: вырезанный сектор и оболочки на его гранях. Перенос
//! `core/cutaway.py` модуля.
//!
//! Из глобуса вынимается сектор - по умолчанию четверть полушария, 90°
//! по долготе от экватора до полюса. Его грани - дуга по широте и две
//! дуги по меридианам - раскрашены оболочками Земли по радиусам модели
//! PREM (Dziewonski, Anderson, 1981, Physics of the Earth and Planetary
//! Interiors 25, 297-356).
//!
//! Радиусы PREM даны для шара [`PREM_RADIUS`]. Край грани лежит
//! на эллипсоиде WGS84, поэтому радиус границы оболочки - доля радиуса
//! эллипсоида в том же направлении. Так грань сходится с поверхностью без
//! щели, а сжатие Земли, 21 км, ложится на все оболочки поровну.
//!
//! Поверхность в секторе отбрасывает шейдер тайла по нормали вершины,
//! числа для него - [`uniform`].
//!
//! С моделью коры CRUST1.0 ([`crate::crust`]) кора на гранях - её слои
//! по ячейкам 1°. Верхняя мантия тогда начинается от Мохо этой точки.
//! Отметка слоя в км переходит в радиус так же, как граница оболочки:
//! доля (PREM_RADIUS + отметка) / PREM_RADIUS радиуса эллипсоида. Точка
//! грани ищется в модели по направлению из центра, разница
//! геоцентрической и геодезической широты, до 0.19°, меньше ячейки.
//!
//! С томографией ([`crate::tomography`]) мантия раскрашена по dVs,
//! с плитами Slab2 ([`crate::slabs`]) на гранях - их полосы
//! ([`slab_bands`]).

use crate::crust::{self, Crust};
use crate::slabs::{self, Slab};
use crate::subsurface::{self, Mesh, Part};
use crate::tomography::{self, Tomo};

type V3 = [f64; 3];

/// Оболочка PREM: ключ, радиус нижней и верхней границы в км, цвет.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shell {
    pub key: &'static str,
    pub low: f64,
    pub high: f64,
    pub color: [u8; 3],
}

/// Радиус шара модели PREM, км.
pub const PREM_RADIUS: f64 = 6371.0;
/// Оболочки снизу вверх. Границы - Мохо 24.4 км, 400 и 670 км, ядро
/// 2891 км, внутреннее ядро 5149.5 км глубины. Цвета - выбор помощника
/// модуля, утверждает автор.
pub const SHELLS: [Shell; 6] = [
    Shell { key: "inner_core", low: 0.0, high: 1221.5, color: [255, 240, 170] },
    Shell { key: "outer_core", low: 1221.5, high: 3480.0, color: [245, 195, 70] },
    Shell { key: "lower_mantle", low: 3480.0, high: 5701.0, color: [225, 135, 55] },
    Shell { key: "transition_zone", low: 5701.0, high: 5971.0, color: [205, 100, 45] },
    Shell { key: "upper_mantle", low: 5971.0, high: 6346.6, color: [180, 75, 40] },
    Shell { key: "crust", low: 6346.6, high: PREM_RADIUS, color: [120, 95, 75] },
];
/// Оболочки мантии: с томографией вместо них - сетка по dVs.
pub const MANTLE: [&str; 3] = ["upper_mantle", "transition_zone", "lower_mantle"];
/// Половина ширины сектора по долготе, градусы.
pub const HALF: f64 = 45.0;
/// Шаг дуги грани, градусы.
pub const ARC_STEP: f64 = 1.5;
/// Шаг дуги с моделью коры: две точки на ячейку.
pub const CRUST_STEP: f64 = 0.5;
/// Слои CRUST1.0 сверху вниз, как [`crust::LAYERS`], и цвет. Выбор
/// помощника модуля, утверждает автор.
pub const CRUST_COLORS: [(&str, [u8; 3]); 8] = [
    ("water", [40, 95, 170]),
    ("ice", [225, 235, 245]),
    ("sediments_upper", [235, 210, 145]),
    ("sediments_middle", [215, 185, 120]),
    ("sediments_lower", [195, 160, 100]),
    ("crust_upper", [170, 150, 135]),
    ("crust_middle", [145, 125, 115]),
    ("crust_lower", [120, 100, 95]),
];
/// Слои океанической коры ([`Crust::oceanic`]) - свой цвет,
/// у [`CRUST_COLORS`] остаётся континентальная.
pub const OCEAN_COLORS: [(&str, [u8; 3]); 3] =
    [("crust_upper", [95, 115, 110]), ("crust_middle", [75, 95, 92]), ("crust_lower", [60, 78, 78])];
/// Выделение коры издалека. Глубины до STRETCH_DEPTH растягиваются
/// функцией [`stretch`]: у поверхности в gain раз, на STRETCH_DEPTH - без
/// растяжения, ниже всё настоящее. gain растёт с расстоянием до точки
/// взгляда ступенями [`GAIN_STEPS`], одна единица на [`GAIN_DISTANCE`]
/// метров. Величины - выбор помощника модуля, утверждает автор.
pub const STRETCH_DEPTH: f64 = 400.0;
pub const GAIN_STEPS: [f64; 7] = [1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0];
pub const GAIN_DISTANCE: f64 = 2.0e6;
/// Цвет плиты Slab2 на гранях.
pub const SLAB_COLOR: [u8; 3] = [55, 85, 125];
/// Шаг дуги полосы плиты - шаг сетки файлов зон Slab2. Ячейка полосы
/// нужна с плитой на обоих концах, крупный шаг съедал бы край зоны и узкие
/// зоны целиком.
pub const SLAB_STEP: f64 = 0.1;
/// Ширина сектора по долготе и по широте - не меньше MIN_SPAN, по долготе
/// не больше MAX_SPAN, градусы.
pub const MIN_SPAN: f64 = 5.0;
pub const MAX_SPAN: f64 = 180.0;
/// Наибольшее количество точек дуги грани: шаг из враждебной сцены
/// не занимает память.
pub const ARC_LIMIT: usize = 1 << 16;

/// Высота поверхности на экране в метрах по геодезическим широтам
/// и долготам точек, см. [`boundary`].
pub type Lift<'a> = &'a dyn Fn(&[f64], &[f64]) -> Vec<f64>;

/// Сектор: западная и восточная долгота, южная и северная широта,
/// градусы. Восток отсчитывается от запада к востоку, через линию перемены
/// дат тоже. Широта 0 - плоскость экватора, широта ±90 - полюс, прочие
/// широты - конус к центру Земли.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wedge {
    pub west: f64,
    pub east: f64,
    pub south: f64,
    pub north: f64,
}

/// Остаток от деления на положительное m со знаком делителя, как `%`
/// в Python.
fn modulo(x: f64, m: f64) -> f64 {
    x.rem_euclid(m)
}

/// Большее из двух, как `max` в Python: при NaN - первое.
fn py_max(a: f64, b: f64) -> f64 {
    if b > a { b } else { a }
}

/// Меньшее из двух, как `min` в Python: при NaN - первое.
fn py_min(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

/// Меньшее из двух, NaN - если NaN хоть одно, как `np.minimum`.
fn nan_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.min(b) }
}

/// Сектор с границами в допустимых пределах: долготы от -180 до 180,
/// ширина по долготе MIN_SPAN-MAX_SPAN, широты по порядку и не уже
/// MIN_SPAN.
pub fn make_wedge(west: f64, east: f64, south: f64, north: f64) -> Wedge {
    let west = modulo(west + 180.0, 360.0) - 180.0;
    let span = py_min(py_max(modulo(east - west, 360.0), MIN_SPAN), MAX_SPAN);
    let east = modulo(west + span + 180.0, 360.0) - 180.0;
    let south = py_min(py_max(south, -90.0), 90.0 - MIN_SPAN);
    let north = py_min(py_max(north, south + MIN_SPAN), 90.0);
    Wedge { west, east, south, north }
}

/// Ширина сектора по долготе, градусы.
pub fn span(wedge: &Wedge) -> f64 {
    let s = modulo(wedge.east - wedge.west, 360.0);
    if s == 0.0 { 360.0 } else { s }
}

/// Средняя долгота сектора.
pub fn middle(wedge: &Wedge) -> f64 {
    modulo(wedge.west + span(wedge) / 2.0 + 180.0, 360.0) - 180.0
}

/// Сектор под точкой взгляда: её долгота посередине, ширина 2 · half, от
/// экватора до полюса её полушария. half модуля по умолчанию - [`HALF`].
pub fn wedge_at(lat: f64, lon: f64, half: f64) -> Wedge {
    if lat >= 0.0 {
        make_wedge(lon - half, lon + half, 0.0, 90.0)
    } else {
        make_wedge(lon - half, lon + half, -90.0, 0.0)
    }
}

/// Числа для шейдера тайла: направление средней долготы (x, y), косинус
/// половины ширины, синусы южной и северной широты.
pub fn uniform(wedge: &Wedge) -> [f64; 5] {
    let (sin, cos) = middle(wedge).to_radians().sin_cos();
    [cos, sin, (span(wedge) / 2.0).to_radians().cos(), wedge.south.to_radians().sin(), wedge.north.to_radians().sin()]
}

/// Точка в вынутом секторе.
pub fn inside(wedge: &Wedge, lat: f64, lon: f64) -> bool {
    let off = modulo(lon - wedge.west, 360.0);
    lat > wedge.south && lat < wedge.north && off <= span(wedge)
}

/// Угловые точки сектора на поверхности: (широта, долгота) в порядке
/// юго-запад, юго-восток, северо-запад, северо-восток. У полюса две точки
/// совпадают.
pub fn corners(wedge: &Wedge) -> [(f64, f64); 4] {
    [(wedge.south, wedge.west), (wedge.south, wedge.east), (wedge.north, wedge.west), (wedge.north, wedge.east)]
}

/// Сектор, у которого угол index (как в [`corners`]) встал в точку.
pub fn move_corner(wedge: &Wedge, index: usize, lat: f64, lon: f64) -> Wedge {
    let Wedge { mut west, mut east, mut south, mut north } = *wedge;
    let western = index == 0 || index == 2;
    if western {
        west = lon;
    } else {
        east = lon;
    }
    if index == 0 || index == 1 {
        south = lat;
    } else {
        north = lat;
    }
    if modulo(east - west, 360.0) > MAX_SPAN {
        // Угол перетащили за противоположную сторону - сектор
        // не выворачивается, ширина остаётся наименьшей.
        if western {
            west = east - MIN_SPAN;
        } else {
            east = west + MIN_SPAN;
        }
    }
    make_wedge(west, east, py_min(south, north), py_max(south, north))
}

/// Широта и долгота точки дуги по направлению из центра.
fn latlon(d: &V3) -> (f64, f64) {
    (d[2].clamp(-1.0, 1.0).asin().to_degrees(), d[1].atan2(d[0]).to_degrees())
}

fn latlons(directions: &[V3]) -> (Vec<f64>, Vec<f64>) {
    directions.iter().map(latlon).unzip()
}

/// Радиус края грани по единичным направлениям из центра: эллипсоид или,
/// с lift, поверхность над ним.
///
/// lift(lats, lons) - высота поверхности на экране в метрах
/// по геодезическим широтам и долготам точек, например геоид в его
/// масштабе. Все оболочки растягиваются в ту же долю, край грани ложится
/// на поверхность. Высота NaN или её нет - точка на эллипсоиде.
pub fn boundary(directions: &[[f64; 3]], lift: Option<Lift>) -> Vec<f64> {
    let (a, b) = (subsurface::A, subsurface::B);
    let radius: Vec<f64> = directions.iter().map(|d| 1.0 / ((d[0] * d[0] + d[1] * d[1]) / (a * a) + d[2] * d[2] / (b * b)).sqrt()).collect();
    let Some(lift) = lift else {
        return radius;
    };
    let (lats, lons) = latlons(directions);
    // Широта направления из центра - геоцентрическая, высоты лежат
    // по геодезической.
    let ratio = (b / a) * (b / a);
    let geodetic: Vec<f64> = lats.iter().map(|lat| lat.to_radians().tan().atan2(ratio).to_degrees()).collect();
    let heights = lift(&geodetic, &lons);
    radius
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let h = heights.get(i).copied().filter(|h| h.is_finite()).unwrap_or(0.0);
            py_max(r + h, r * 0.5)
        })
        .collect()
}

/// Растяжение коры для расстояния до точки взгляда, ступенью
/// [`GAIN_STEPS`]: наибольшая ступень не больше distance / GAIN_DISTANCE.
pub fn gain_for(distance: f64) -> f64 {
    let raw = distance / GAIN_DISTANCE;
    GAIN_STEPS.iter().copied().filter(|g| *g <= raw).fold(GAIN_STEPS[0], f64::max)
}

/// Глубина на экране для настоящей глубины depth, км. До STRETCH_DEPTH -
/// STRETCH_DEPTH · (1 - (1 - x)^gain), x = depth / STRETCH_DEPTH: наклон
/// у поверхности gain, на границе 1. Выше поверхности (depth < 0)
/// и глубже границы - без изменения.
pub fn stretch(depth: f64, gain: f64) -> f64 {
    if gain == 1.0 || !(depth > 0.0 && depth < STRETCH_DEPTH) {
        return depth;
    }
    let x = (depth / STRETCH_DEPTH).clamp(0.0, 1.0);
    STRETCH_DEPTH * (1.0 - (1.0 - x).powf(gain))
}

/// Доля радиуса для отметки в км, вверх положительно.
fn share(elevation: f64, gain: f64) -> f64 {
    (PREM_RADIUS - stretch(-elevation, gain)) / PREM_RADIUS
}

/// Доля радиуса для радиуса PREM в км.
fn radius_share(r: f64, gain: f64) -> f64 {
    share(r - PREM_RADIUS, gain)
}

/// Значение по точке или одно на все: срез длины 1 - одно.
fn pick<T: Copy>(values: &[T], i: usize) -> Option<T> {
    values.get(i).or(values.first()).copied()
}

fn narrow(a: V3) -> [f32; 3] {
    [a[0] as f32, a[1] as f32, a[2] as f32]
}

/// Полоса грани между долями радиуса low и high по точкам дуги edge (или
/// одним числом - срез длины 1). Ячейка нулевой толщины на обоих концах
/// пропускается. whole - ячейка нужна с толщиной на обоих концах: так
/// у плиты, точка без плиты стоит на поверхности, и ячейка от конца плиты
/// к ней шла бы клином до поверхности. color - один цвет (срез длины 1)
/// или цвет на точку дуги. Вершины - сначала low по точкам, потом high.
pub fn band(edge: &[[f64; 3]], normal: [f64; 3], low: &[f64], high: &[f64], color: &[[u8; 3]], whole: bool) -> Option<Part> {
    let n = edge.len();
    if n < 2 || u32::try_from(2 * n).is_err() {
        return None;
    }
    let lows: Vec<f64> = (0..n).map(|i| pick(low, i).unwrap_or(f64::NAN)).collect();
    let highs: Vec<f64> = (0..n).map(|i| pick(high, i).unwrap_or(f64::NAN)).collect();
    let thick = |i: usize| highs[i] > lows[i];
    let keep: Vec<usize> = (0..n - 1).filter(|i| if whole { thick(*i) && thick(i + 1) } else { thick(*i) || thick(i + 1) }).collect();
    if keep.is_empty() {
        return None;
    }
    let mut positions: Vec<V3> = edge.iter().zip(&lows).map(|(e, s)| e.map(|v| v * s)).collect();
    positions.extend(edge.iter().zip(&highs).map(|(e, s)| e.map(|v| v * s)));
    let m = n as u32;
    let mut indices: Vec<[u32; 3]> = keep.iter().map(|i| *i as u32).map(|i| [i, i + 1, m + i]).collect();
    indices.extend(keep.iter().map(|i| *i as u32).map(|i| [i + 1, m + i + 1, m + i]));
    let rgba = |i: usize| {
        let [r, g, b] = pick(color, i).unwrap_or([255; 3]);
        [r, g, b, 255]
    };
    let mut colors: Vec<[u8; 4]> = (0..n).map(rgba).collect();
    colors.extend((0..n).map(rgba));
    Some(Part { positions, normals: vec![narrow(normal); 2 * n], colors, indices })
}

/// Мантия по томографии: сетка от top (доля радиуса по точкам - Мохо)
/// до ядра, ряды на глубинах модели, цвет вершины - dVs.
fn mantle(edge: &[V3], directions: &[V3], normal: V3, top: &[f64], tomo: &Tomo, gain: f64) -> Option<Part> {
    let (lats, lons) = latlons(directions);
    let depths = tomography::rows(tomo);
    let n = edge.len();
    let first = *depths.first()?;
    let mut shares: Vec<Vec<f64>> = vec![(0..n).map(|i| top.get(i).copied().unwrap_or(f64::NAN)).collect()];
    let mut rgb = vec![tomography::colors(&tomography::sample(tomo, &lats, &lons, first))];
    for depth in &depths {
        let level = share(-depth, gain);
        let row = shares[0].iter().map(|t| nan_min(level, *t)).collect();
        shares.push(row);
        rgb.push(tomography::colors(&tomography::sample(tomo, &lats, &lons, *depth)));
    }
    let count = shares.len();
    let total = count.checked_mul(n).filter(|t| u32::try_from(*t).is_ok())?;
    let positions: Vec<V3> = shares.iter().flat_map(|s| edge.iter().zip(s).map(|(e, k)| e.map(|v| v * k))).collect();
    let colors: Vec<[u8; 4]> =
        rgb.iter().flat_map(|row| (0..n).map(|i| row.get(i).copied().unwrap_or([255; 3])).map(|[r, g, b]| [r, g, b, 255])).collect();
    let mut indices = Vec::new();
    let m = n as u32;
    for r in 0..count as u32 - 1 {
        let (a, b) = (r * m, (r + 1) * m);
        indices.extend((0..m.saturating_sub(1)).map(|i| [a + i, a + i + 1, b + i]));
        indices.extend((0..m.saturating_sub(1)).map(|i| [a + i + 1, b + i + 1, b + i]));
    }
    Some(Part { positions, normals: vec![narrow(normal); total], colors, indices })
}

/// Настройка граней [`faces`]. По умолчанию - оболочки [`SHELLS`], шаг
/// по модели, без коры, растяжения, подъёма края и томографии.
#[derive(Clone, Copy)]
pub struct Faces<'a> {
    /// Оболочки PREM на гранях.
    pub shells: &'a [Shell],
    /// Шаг дуги, градусы. None - [`ARC_STEP`], а с корой, lift или
    /// томографией - [`CRUST_STEP`].
    pub step: Option<f64>,
    /// Модель коры или None - кора оболочкой PREM.
    pub crust: Option<&'a Crust>,
    /// Растяжение коры, см. [`stretch`].
    pub gain: f64,
    /// Высота поверхности, к которой тянется край граней, см. [`boundary`].
    pub lift: Option<Lift<'a>>,
    /// Томография мантии или None - мантия оболочками PREM.
    pub tomo: Option<&'a Tomo>,
}

impl Default for Faces<'_> {
    fn default() -> Self {
        Faces { shells: &SHELLS, step: None, crust: None, gain: 1.0, lift: None, tomo: None }
    }
}

/// Нижняя граница коры PREM, км радиуса.
fn crust_low() -> f64 {
    SHELLS.iter().find(|s| s.key == "crust").map_or(PREM_RADIUS, |s| s.low)
}

/// Грань - полосы оболочек по дуге единичных направлений.
fn face(directions: &[V3], normal: V3, options: &Faces) -> Vec<Option<Part>> {
    let gain = options.gain;
    let edge: Vec<V3> = boundary(directions, options.lift).iter().zip(directions).map(|(r, d)| d.map(|v| v * r)).collect();
    let radius = |r: f64| radius_share(r, gain);
    let is_mantle = |key: &str| MANTLE.contains(&key);
    let tomo = options.tomo;
    let mut parts = Vec::new();
    let Some(crust) = options.crust else {
        for s in options.shells.iter().filter(|s| tomo.is_none() || !is_mantle(s.key)) {
            parts.push(band(&edge, normal, &[radius(s.low)], &[radius(s.high)], &[s.color], false));
        }
        if let Some(tomo) = tomo {
            parts.push(mantle(&edge, directions, normal, &vec![radius(crust_low()); edge.len()], tomo, gain));
        }
        return parts;
    };
    let (lats, lons) = latlons(directions);
    let shares: Vec<[f64; crust::BOUNDS]> = lats.iter().zip(&lons).map(|(la, lo)| crust.at(*la, *lo).map(|b| share(b, gain))).collect();
    let moho: Vec<f64> = shares.iter().map(|s| s[crust::MOHO]).collect();
    for s in options.shells {
        if s.key == "crust" || (tomo.is_some() && is_mantle(s.key)) {
            continue;
        }
        let top = if s.key == "upper_mantle" { moho.clone() } else { vec![radius(s.high)] };
        parts.push(band(&edge, normal, &[radius(s.low)], &top, &[s.color], false));
    }
    if let Some(tomo) = tomo {
        parts.push(mantle(&edge, directions, normal, &moho, tomo, gain));
    }
    let oceanic: Option<Vec<bool>> = lats.iter().zip(&lons).map(|(la, lo)| crust.oceanic(*la, *lo)).collect();
    for (k, key) in crust_layers().iter().enumerate() {
        let low: Vec<f64> = shares.iter().map(|s| s[k + 1]).collect();
        let high: Vec<f64> = shares.iter().map(|s| s[k]).collect();
        parts.push(band(&edge, normal, &low, &high, &layer_colors(key, oceanic.as_deref()), false));
    }
    parts
}

/// Ключи слоёв коры сверху вниз, как [`crust::LAYERS`].
pub fn crust_layers() -> [&'static str; 8] {
    CRUST_COLORS.map(|(key, _)| key)
}

/// Цвет континентального слоя коры key.
pub fn crust_color(key: &str) -> Option<[u8; 3]> {
    CRUST_COLORS.iter().find(|(k, _)| *k == key).map(|(_, c)| *c)
}

/// Цвет океанического слоя коры key, если слой бывает океаническим.
pub fn ocean_color(key: &str) -> Option<[u8; 3]> {
    OCEAN_COLORS.iter().find(|(k, _)| *k == key).map(|(_, c)| *c)
}

/// Цвет слоя коры key: один цвет (срез длины 1) или цвет на точку, если
/// слой бывает океаническим и oceanic - признаки по точкам. Неизвестный
/// ключ - пусто.
pub fn layer_colors(key: &str, oceanic: Option<&[bool]>) -> Vec<[u8; 3]> {
    let Some(land) = crust_color(key) else {
        return Vec::new();
    };
    match (oceanic, ocean_color(key)) {
        (Some(oceanic), Some(sea)) => oceanic.iter().map(|o| if *o { sea } else { land }).collect(),
        _ => vec![land],
    }
}

/// Слой коры по точкам линии на континентальную и океаническую части:
/// (океаническая ли, верх, низ) с NaN вне своей части. Края океанической
/// части расширены на точку, так между частями нет ни щели, ни перекрытия:
/// отрезок на стыке берёт океаническая.
pub fn split_types(top: &[f64], bottom: &[f64], oceanic: Option<&[bool]>) -> Vec<(bool, Vec<f64>, Vec<f64>)> {
    let Some(ocean) = oceanic else {
        return vec![(false, top.to_vec(), bottom.to_vec())];
    };
    let n = top.len();
    let sea = |i: usize| ocean.get(i).copied().unwrap_or(false);
    let wide: Vec<bool> = (0..n).map(|i| sea(i) || (i > 0 && sea(i - 1)) || sea(i + 1)).collect();
    let land: Vec<bool> = (0..n).map(|i| !sea(i)).collect();
    let mut out = Vec::new();
    for (kind, mask) in [(false, land), (true, wide)] {
        if mask.iter().any(|m| *m) {
            let cut = |values: &[f64]| (0..n).map(|i| if mask[i] { values.get(i).copied().unwrap_or(f64::NAN) } else { f64::NAN }).collect();
            out.push((kind, cut(top), cut(bottom)));
        }
    }
    out
}

/// Сетка граней сектора для видеокарты или None - треугольников нет.
///
/// Широтные грани - дуги по долготе на южной и северной широте,
/// меридиональные - дуги от южной до северной широты на крайних долготах.
pub fn faces(wedge: &Wedge, options: &Faces) -> Option<Mesh> {
    let plain = options.crust.is_none() && options.lift.is_none() && options.tomo.is_none();
    let step = options.step.unwrap_or(if plain { ARC_STEP } else { CRUST_STEP });
    let parts: Vec<Part> = arcs(wedge, step).iter().flat_map(|(d, normal)| face(d, *normal, options)).flatten().collect();
    subsurface::merge(&parts)
}

/// Количество точек дуги длины width с шагом step, не меньше двух
/// и не больше [`ARC_LIMIT`]. Нулевой шаг или NaN - [`ARC_LIMIT`].
fn arc_count(width: f64, step: f64) -> usize {
    let raw = (width / step).ceil() + 1.0;
    if raw.is_nan() || raw >= ARC_LIMIT as f64 { ARC_LIMIT } else { py_max(raw, 2.0) as usize }
}

/// Равные шаги от start до stop, count значений, конец точно, как
/// `np.linspace`.
fn linspace(start: f64, stop: f64, count: usize) -> Vec<f64> {
    let step = (stop - start) / (count.max(2) - 1) as f64;
    (0..count).map(|i| if i + 1 == count { stop } else { start + i as f64 * step }).collect()
}

/// Дуги граней: единичные направления и нормаль грани. Широтные -
/// по долготе на южной и северной широте, если она не полюс: на экваторе
/// это плоскость, на прочих широтах конус к центру. Меридиональные - от
/// южной до северной широты на крайних долготах.
pub fn arcs(wedge: &Wedge, step: f64) -> Vec<(Vec<[f64; 3]>, [f64; 3])> {
    let width = span(wedge);
    let count = arc_count(width, step);
    let lons: Vec<f64> = linspace(0.0, width, count).iter().map(|v| (wedge.west + v).to_radians()).collect();
    let mut out = Vec::new();
    for lat in [wedge.south, wedge.north] {
        if lat.abs() >= 90.0 {
            continue;
        }
        let (sp, cp) = lat.to_radians().sin_cos();
        let directions = lons.iter().map(|l| [cp * l.cos(), cp * l.sin(), sp]).collect();
        // Нормаль конуса - на север в середине дуги: свет одной грани один.
        let (sl, cl) = middle(wedge).to_radians().sin_cos();
        out.push((directions, [-sp * cl, -sp * sl, cp]));
    }
    let count = arc_count(wedge.north - wedge.south, step);
    let lats: Vec<f64> = linspace(wedge.south, wedge.north, count).iter().map(|v| v.to_radians()).collect();
    for lon in [wedge.west, wedge.east] {
        let (sl, cl) = lon.to_radians().sin_cos();
        out.push((lats.iter().map(|p| [p.cos() * cl, p.cos() * sl, p.sin()]).collect(), [-sl, cl, 0.0]));
    }
    out
}

/// Широты и долготы всех точек дуг граней: по ним ищутся зоны плит,
/// которых касается разрез. Шаг модуля по умолчанию - [`CRUST_STEP`].
pub fn arc_points(wedge: &Wedge, step: f64) -> (Vec<f64>, Vec<f64>) {
    let mut lats = Vec::new();
    let mut lons = Vec::new();
    for (d, _) in arcs(wedge, step) {
        let (la, lo) = latlons(&d);
        lats.extend(la);
        lons.extend(lo);
    }
    (lats, lons)
}

/// Замена NaN нулём и бесконечностей крайними числами, как `np.nan_to_num`.
fn nan_to_num(v: f64) -> f64 {
    if v.is_nan() {
        0.0
    } else if v == f64::INFINITY {
        f64::MAX
    } else if v == f64::NEG_INFINITY {
        f64::MIN
    } else {
        v
    }
}

/// Полосы плит Slab2 на гранях или None. zones - зоны, которых касается
/// разрез. Глубины - с тем же растяжением gain, что у коры, край - как
/// у граней (lift). Шаг модуля по умолчанию - [`SLAB_STEP`].
pub fn slab_bands(wedge: &Wedge, zones: &[&Slab], gain: f64, step: f64, lift: Option<Lift>) -> Option<Mesh> {
    let mut parts = Vec::new();
    for (directions, normal) in arcs(wedge, step) {
        let (lats, lons) = latlons(&directions);
        let (top, bottom) = slabs::band(zones.iter().copied(), &lats, &lons);
        let none: Vec<bool> = top.iter().map(|t| !t.is_finite()).collect();
        let level = |values: &[f64]| -> Vec<f64> {
            values.iter().zip(&none).map(|(v, empty)| if *empty { 1.0 } else { share(-nan_to_num(*v), gain) }).collect()
        };
        let edge: Vec<V3> = boundary(&directions, lift).iter().zip(&directions).map(|(r, d)| d.map(|v| v * r)).collect();
        parts.push(band(&edge, normal, &level(&bottom), &level(&top), &[SLAB_COLOR], true));
    }
    subsurface::merge(parts.iter().flatten())
}

/// Места подписей оболочек: ключ, широта, долгота и высота над
/// эллипсоидом середины оболочки на широтной грани, по средней долготе.
/// Высота отрицательная - точка внутри Земли.
pub fn label_points(wedge: &Wedge, shells: &[Shell]) -> Vec<(&'static str, f64, f64, f64)> {
    let lat = if wedge.north > 0.0 { py_max(wedge.south, 0.0) } else { wedge.north };
    shells
        .iter()
        .map(|s| {
            let mid = (s.low + s.high) / 2.0 / PREM_RADIUS;
            (s.key, lat, middle(wedge), (mid - 1.0) * subsurface::A)
        })
        .collect()
}
