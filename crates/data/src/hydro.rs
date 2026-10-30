// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Вода по рельефу: затопление от точки до уровня, линии стока, водосбор
//! точки и метки, бассейны квадрата, речная сеть. Перенос `core/hydro.py`
//! модуля. Ядро стока - [`crate::hydro_fill`], [`crate::topo_flow`],
//! [`crate::topo_gauge`] (из Isoliner, владелец тот же). Здесь - сетка
//! высот вокруг точки и то, что глобусу нужно поверх ядра.
//!
//! Сетка - узлы Web Mercator с равным шагом на местности в середине
//! участка. Шаг по осям Меркатора - шаг на местности, делённый на
//! косинус широты середины. Строка 0 северная, номер узла `r * cols + c`.
//!
//! Затопление - уровень перелива. Для каждого узла это наименьший
//! уровень воды, при котором вода от точки доходит до него: наименьший
//! по всем путям наибольший рельеф пути. Его даёт заполнение понижений
//! с единственным стоком в точке и без уклона. Узел затоплен водой
//! уровня H, если уровень перелива ниже H, поэтому смена уровня - одно
//! сравнение. Котловины, не связанные с точкой, остаются сухими.

use crate::contours;
use crate::hydro_fill::fill_depressions;
use crate::topo_flow;
use crate::topo_gauge::{self, Extras, Report};

/// Узлов сетки поперёк участка: шаг - диаметр / SIDE, но не мельче
/// пикселя высот. Выбор помощника модуля, утверждает автор.
pub const SIDE: usize = 600;
/// Стена вместо пустых высот у затопления: вода через неё не идёт.
pub const WALL: f64 = 1e20;
/// Проходов заполнения у уровня перелива в модуле. Очереди с приоритетом
/// (`hydro_fill`) проходы не нужны, величина оставлена ради
/// соответствия модулю.
pub const SPILL_PASSES: usize = 1000;
/// Уклон на плоских участках у заполнения для стока, м на ячейку.
pub const EPSILON: f64 = 0.001;
/// Притяжка точки водосбора к водотоку: радиус SNAP_METRES, но не
/// меньше SNAP_CELLS ячеек. Выбор помощника модуля.
pub const SNAP_CELLS: usize = 3;
pub const SNAP_METRES: f64 = 150.0;
/// Водотоки сети в отчёте водосбора - с водосбором от NET_AREA, м².
pub const NET_AREA: f64 = 1e6;
/// Узлов сетки поверхности воды по стороне.
pub const WATER_SIDE: usize = 256;
/// Короче MIN_LINE ячеек линии от метки не идут в ответ: старт у самой
/// чужой линии сливается с ней через шаг.
pub const MIN_LINE: usize = 3;
/// Квадрат расчёта для метки (shape_grid модуля): reach её полуразмеров
/// вокруг, не меньше SHAPE_LEAST и не больше SHAPE_MOST метров.
pub const SHAPE_REACH: f64 = 3.0;
pub const SHAPE_LEAST: f64 = 2000.0;
pub const SHAPE_MOST: f64 = 50000.0;
/// Цвет воды затопления на тайлах карты: от мелкой (0 м) к глубокой
/// (наибольшая глубина затопления). Цвета растра глубин модуля
/// (`ui/hydro.py`, DEPTH_COLORS).
pub const DEPTH_COLORS: [[u8; 3]; 2] = [[200, 230, 255], [0, 40, 140]];
/// Непрозрачность воды затопления на тайлах карты. Выбор помощника,
/// утверждает автор.
pub const FLOOD_OPACITY: f64 = 0.7;
/// Пикселей тайла по стороне.
pub const TILE: usize = 256;

/// Сетка высот вокруг точки lat, lon. x0, y0 - узел [0, 0] в метрах Web
/// Mercator (северо-западный), step - шаг в метрах Меркатора, cell - шаг
/// на местности в метрах, rows и cols - размер.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub lat: f64,
    pub lon: f64,
    pub x0: f64,
    pub y0: f64,
    pub step: f64,
    pub cell: f64,
    pub rows: usize,
    pub cols: usize,
}

/// Сетка квадрата со стороной 2 radius_m с серединой lat, lon, шаг на
/// местности не мельче cell и не больше SIDE узлов поперёк, количество
/// узлов нечётное. radius - радиус тела, метры.
pub fn make_grid(lat: f64, lon: f64, radius_m: f64, cell: f64, radius: f64) -> Grid {
    let cell = cell.max(2.0 * radius_m / SIDE as f64);
    let span = (2.0 * radius_m / cell).ceil();
    let n = (if span.is_finite() && span > 0.0 { (span as usize).min(SIDE + 1) } else { 0 }) | 1;
    let scale = 1.0 / lat.to_radians().cos().max(1e-6);
    let step = cell * scale;
    let (x, y) = mercator(lat, lon, radius);
    let half = (n - 1) as f64 / 2.0;
    Grid { lat, lon, x0: x - half * step, y0: y + half * step, step, cell, rows: n, cols: n }
}

/// Широта и долгота в метрах Web Mercator.
pub fn mercator(lat: f64, lon: f64, radius: f64) -> (f64, f64) {
    let lat = lat.clamp(-85.0511, 85.0511).to_radians();
    (radius * lon.to_radians(), radius * (std::f64::consts::FRAC_PI_4 + lat / 2.0).tan().ln())
}

/// Широта и долгота узла с номерами row, col (можно дробными).
pub fn latlon(grid: &Grid, row: f64, col: f64, radius: f64) -> (f64, f64) {
    let x = grid.x0 + col * grid.step;
    let y = grid.y0 - row * grid.step;
    ((2.0 * (y / radius).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees(), (x / radius).to_degrees())
}

/// Широты и долготы всех узлов по строкам, через stride.
pub fn nodes(grid: &Grid, radius: f64, stride: usize) -> (Vec<f64>, Vec<f64>) {
    let stride = stride.max(1);
    let (mut lats, mut lons) = (Vec::new(), Vec::new());
    for r in (0..grid.rows).step_by(stride) {
        for c in (0..grid.cols).step_by(stride) {
            let (lat, lon) = latlon(grid, r as f64, c as f64, radius);
            lats.push(lat);
            lons.push(lon);
        }
    }
    (lats, lons)
}

/// Узел (строка, столбец), ближайший к точке, в пределах сетки.
pub fn index_of(grid: &Grid, lat: f64, lon: f64, radius: f64) -> (usize, usize) {
    let (x, y) = mercator(lat, lon, radius);
    let c = ((x - grid.x0) / grid.step).round_ties_even() as i64;
    let r = ((grid.y0 - y) / grid.step).round_ties_even() as i64;
    (r.clamp(0, grid.rows.max(1) as i64 - 1) as usize, c.clamp(0, grid.cols.max(1) as i64 - 1) as usize)
}

// --- затопление ---------------------------------------------------------------

/// Уровень перелива от узла seed (строка, столбец): по узлам сетки.
/// Пустые высоты (NaN) - стена, у них и у недостижимых узлов - inf.
/// Входы не той длины и seed за сеткой - пустой вектор.
pub fn spill_levels(z: &[f64], rows: usize, cols: usize, seed: (usize, usize)) -> Vec<f64> {
    let n = rows.saturating_mul(cols);
    if z.len() != n || seed.0 >= rows || seed.1 >= cols {
        return Vec::new();
    }
    let work: Vec<f64> = z.iter().map(|v| if v.is_finite() { *v } else { WALL }).collect();
    let mut outlets = vec![false; n];
    outlets[seed.0 * cols + seed.1] = true;
    let Some(fill) = fill_depressions(&work, rows, cols, None, 0.0, Some(&outlets)) else {
        return Vec::new();
    };
    fill.filled.iter().zip(z).map(|(l, v)| if !v.is_finite() || *l >= WALL { f64::INFINITY } else { *l }).collect()
}

/// Затопление уровнем.
#[derive(Debug, Clone, PartialEq)]
pub struct Flood {
    /// Затопленные узлы.
    pub mask: Vec<bool>,
    /// Площадь, м².
    pub area: f64,
    /// Объём воды, м³.
    pub volume: f64,
    /// Наибольшая глубина, м.
    pub depth: f64,
    /// Вода дошла до края сетки и может идти дальше.
    pub edge: bool,
}

/// Узел на рамке сетки.
fn on_border(i: usize, rows: usize, cols: usize) -> bool {
    let (r, c) = (i / cols.max(1), i % cols.max(1));
    r == 0 || c == 0 || r + 1 == rows || c + 1 == cols
}

/// Маска касается рамки сетки.
fn touches_border(mask: &[bool], rows: usize, cols: usize) -> bool {
    mask.iter().enumerate().any(|(i, m)| *m && on_border(i, rows, cols))
}

/// Затопление водой уровня level по уровню перелива levels.
pub fn flood(z: &[f64], rows: usize, cols: usize, levels: &[f64], level: f64, cell: f64) -> Flood {
    let n = rows.saturating_mul(cols);
    if z.len() != n || levels.len() != n {
        return Flood { mask: vec![false; z.len()], area: 0.0, volume: 0.0, depth: 0.0, edge: false };
    }
    let mask: Vec<bool> = levels.iter().map(|l| *l < level).collect();
    let (mut count, mut sum, mut deepest) = (0usize, 0.0, f64::NEG_INFINITY);
    for i in 0..n {
        if mask[i] {
            let d = level - z[i];
            count += 1;
            sum += d;
            deepest = deepest.max(d);
        }
    }
    if count == 0 {
        return Flood { mask, area: 0.0, volume: 0.0, depth: 0.0, edge: false };
    }
    let area = cell * cell;
    let edge = touches_border(&mask, rows, cols);
    Flood { mask, area: area * count as f64, volume: area * sum, depth: deepest, edge }
}

/// Узлы поверхности воды: шаг по узлам сетки и мокрые узлы.
#[derive(Debug, Clone, PartialEq)]
pub struct WaterNodes {
    pub stride: usize,
    pub rows: usize,
    pub cols: usize,
    pub wet: Vec<bool>,
}

/// Узлы поверхности воды не больше side по стороне. Узел мокрый, если
/// затоплен любой узел его клетки - иначе узкая протока пропадала бы
/// между узлами.
pub fn water_nodes(mask: &[bool], rows: usize, cols: usize, side: usize) -> WaterNodes {
    if mask.len() != rows.saturating_mul(cols) {
        return WaterNodes { stride: 1, rows: 0, cols: 0, wet: Vec::new() };
    }
    let stride = rows.max(cols).div_ceil(side.max(1)).max(1);
    if stride == 1 {
        return WaterNodes { stride, rows, cols, wet: mask.to_vec() };
    }
    let (wr, wc) = (rows.div_ceil(stride), cols.div_ceil(stride));
    let mut wet = vec![false; wr * wc];
    for i in 0..mask.len() {
        if mask[i] {
            wet[(i / cols / stride) * wc + (i % cols) / stride] = true;
        }
    }
    WaterNodes { stride, rows: wr, cols: wc, wet }
}

// --- сток ---------------------------------------------------------------------

/// Решётка стока сетки.
#[derive(Debug, Clone, PartialEq)]
pub struct Drainage {
    /// Заполненные высоты, у пустых - NaN.
    pub filled: Vec<f64>,
    /// Номер приёмника, -1 у стока.
    pub downstream: Vec<i64>,
    /// Пустые узлы (NaN высот).
    pub empty: Vec<bool>,
}

/// Решётка стока: заполнение с EPSILON и D8. Пустые высоты (NaN) - край,
/// вода уходит в них, как в Isoliner. None - сетка меньше 2×2 или не той
/// длины.
pub fn drainage(z: &[f64], rows: usize, cols: usize) -> Option<Drainage> {
    let empty: Vec<bool> = z.iter().map(|v| !v.is_finite()).collect();
    let fill = fill_depressions(z, rows, cols, Some(&empty), EPSILON, None)?;
    let (_, downstream) = topo_flow::d8_directions(&fill.filled, rows, cols, Some(&empty));
    Some(Drainage { filled: fill.filled, downstream, empty })
}

/// Чем кончилась линия стока.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineEnd {
    /// Край сетки.
    Edge,
    /// Сток, ниже некуда.
    Pit,
    /// Дошла до водотока.
    Stream,
    /// Рельеф выположился.
    Flat,
    /// Влилась в линию, построенную раньше.
    Merge,
}

/// Линия стока: узлы от старта вниз.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub rows: Vec<usize>,
    pub cols: Vec<usize>,
    /// Длина, м.
    pub length: f64,
    /// Падение по исходным высотам, м.
    pub drop: f64,
    pub end: LineEnd,
}

/// Линии стока от узлов seeds - номеров узлов (инструмент 2.23 Isoliner
/// «Линии стока от точек, линий и контуров»).
///
/// area - линия кончается на водотоке с водосбором от area м². flat -
/// (уклон в градусах, длина участка в метрах): линия кончается, где
/// средний уклон на этой длине ниже порога. merge - линия кончается,
/// вливаясь в построенную раньше, линии строятся от высоких узлов
/// к низким, узлы, через которые прошла линия, своих линий не дают.
#[allow(clippy::too_many_arguments)]
pub fn flow_lines(z: &[f64], rows: usize, cols: usize, seeds: &[usize], cell: f64, area: Option<f64>, flat: Option<(f64, f64)>, merge: bool) -> Vec<Line> {
    let Some(d) = drainage(z, rows, cols) else {
        return Vec::new();
    };
    let n = rows * cols;
    let mut order = seeds.to_vec();
    if merge {
        let height = |i: usize| d.filled.get(i).copied().filter(|v| !v.is_nan()).unwrap_or(f64::NEG_INFINITY);
        order.sort_by(|a, b| height(*b).total_cmp(&height(*a)));
    }
    let mut rivers = vec![false; n];
    if let Some(area) = area.filter(|a| *a != 0.0 && !a.is_nan()) {
        let acc = topo_flow::flow_accumulation(&d.downstream, rows, cols, Some(&d.empty));
        let cells = area / (cell * cell);
        for (r, a) in rivers.iter_mut().zip(&acc) {
            *r = *a >= cells;
        }
    }
    let mut owned = vec![false; n];
    let mut stop = rivers.clone();
    let mut out = Vec::new();
    for start in order {
        if start >= n || owned[start] {
            continue;
        }
        let Some(mut path) = topo_flow::trace_downhill(&d.downstream, &[start as i64], rows, cols, Some(&stop), None, None).pop().filter(|p| !p.is_empty()) else {
            continue;
        };
        let last = path[path.len() - 1];
        let mut end = if path.len() > 1 && rivers[last] {
            LineEnd::Stream
        } else if path.len() > 1 && owned[last] {
            LineEnd::Merge
        } else {
            LineEnd::Pit
        };
        // По краю сетки D8 ведёт воду вдоль рамки до угла: линия кончается
        // на первом узле края, дальше рельефа нет.
        if let Some(e) = path.iter().position(|i| on_border(*i, rows, cols))
            && (e + 1 < path.len() || end == LineEnd::Pit)
        {
            path.truncate(e + 1);
            end = LineEnd::Edge;
        }
        if let Some((degrees, length)) = flat {
            let (cut, done) = topo_flow::cut_on_flattening(&path, z, rows, cols, cell, topo_flow::slope_from_degrees(degrees), length, None);
            path = cut;
            if done {
                end = LineEnd::Flat;
            }
        }
        if merge {
            // Обрезанное выполаживанием не занято: линия ниже по склону
            // может идти через эти узлы дальше.
            for &p in &path {
                owned[p] = true;
                stop[p] = true;
            }
        }
        let m = topo_flow::path_metrics(&path, z, rows, cols, cell, None);
        out.push(Line {
            rows: path.iter().map(|i| i / cols).collect(),
            cols: path.iter().map(|i| i % cols).collect(),
            length: m.length,
            drop: if m.drop.is_finite() { m.drop } else { 0.0 },
            end,
        });
    }
    out
}

/// Линия стока от узла seed по заполненному рельефу. None - узел за
/// сеткой или сетка негодна.
pub fn flow_line(z: &[f64], rows: usize, cols: usize, seed: (usize, usize), cell: f64, area: Option<f64>, flat: Option<(f64, f64)>) -> Option<Line> {
    if seed.0 >= rows || seed.1 >= cols {
        return None;
    }
    flow_lines(z, rows, cols, &[seed.0 * cols + seed.1], cell, area, flat, false).into_iter().next()
}

/// Бассейн квадрата.
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub mask: Vec<bool>,
    /// Площадь, м².
    pub area: f64,
}

/// Бассейны квадрата от устьев - узлов, где вода уходит из квадрата
/// с водосбором от area м² (инструмент 2.07 Isoliner «Бассейны» без
/// точек). Узлы с меньших устьев в ответ не идут. Крупные первыми.
pub fn outlet_basins(z: &[f64], rows: usize, cols: usize, cell: f64, area: f64) -> Vec<Region> {
    let Some(d) = drainage(z, rows, cols) else {
        return Vec::new();
    };
    let acc = topo_flow::flow_accumulation(&d.downstream, rows, cols, Some(&d.empty));
    let threshold = (area / (cell * cell)).max(1.0);
    let labels = topo_flow::basins(&d.downstream, rows, cols, None, Some(&d.empty), Some(&acc), Some(threshold));
    let top = labels.iter().copied().max().unwrap_or(0).max(0) as usize;
    let mut counts = vec![0usize; top + 1];
    for l in &labels {
        if *l > 0 {
            counts[*l as usize] += 1;
        }
    }
    let mut out: Vec<Region> = (1..=top)
        .filter(|l| counts[*l] > 0)
        .map(|l| Region { mask: labels.iter().map(|v| *v as usize == l && *v > 0).collect(), area: counts[l] as f64 * cell * cell })
        .collect();
    out.sort_by(|a, b| b.area.total_cmp(&a.area));
    out
}

/// Водосбор точки.
#[derive(Debug, Clone, PartialEq)]
pub struct Basin {
    /// Узлы водосбора.
    pub mask: Vec<bool>,
    /// Створ после притяжки к водотоку.
    pub seed: (usize, usize),
    /// Отчёт по ключам topo_gauge::REPORT_KEYS.
    pub report: Report,
    /// Главный водоток от створа к истоку.
    pub stream_rows: Vec<usize>,
    pub stream_cols: Vec<usize>,
    /// Водосбор дошёл до края сетки и неполон.
    pub edge: bool,
}

/// Водосбор узла seed: створ притягивается к узлу наибольшей аккумуляции
/// в snap ячейках, как в инструменте «Бассейны» Isoliner. Без snap -
/// SNAP_METRES, не меньше SNAP_CELLS.
pub fn watershed(z: &[f64], rows: usize, cols: usize, seed: (usize, usize), cell: f64, snap: Option<usize>) -> Option<Basin> {
    let snap = snap.unwrap_or_else(|| {
        let cells = (SNAP_METRES / cell).round_ties_even();
        SNAP_CELLS.max(if cells.is_finite() && cells > 0.0 { cells.min(1e9) as usize } else { 0 })
    });
    let d = drainage(z, rows, cols)?;
    let acc = topo_flow::flow_accumulation(&d.downstream, rows, cols, Some(&d.empty));
    let wide = |v: usize| i64::try_from(v).unwrap_or(i64::MAX);
    let (r, c) = topo_gauge::snap_to_max_acc(&acc, rows, cols, wide(seed.0), wide(seed.1), wide(snap));
    let start = r * cols + c;
    let extras = Extras { slope: None, nodata: Some(&d.empty), iso_interval: contours::grid_step(z, 12.0), net_threshold: NET_AREA / (cell * cell) };
    let report = topo_gauge::gauge_report(&d.filled, &d.downstream, &acc, rows, cols, start, cell, &extras);
    let mask: Vec<bool> = topo_gauge::basin_mask(&d.downstream, rows, cols, start).iter().zip(&d.empty).map(|(m, e)| *m && !*e).collect();
    let (_, path) = topo_gauge::trace_main_stream(&d.downstream, &acc, rows, cols, start, cell);
    let edge = touches_border(&mask, rows, cols);
    Some(Basin { mask, seed: (r, c), report, stream_rows: path.iter().map(|i| i / cols).collect(), stream_cols: path.iter().map(|i| i % cols).collect(), edge })
}

/// Середина и полусторона квадрата расчёта для метки: (широта, долгота,
/// полусторона, м). Квадрат накрывает метку и reach её полуразмеров
/// вокруг, не меньше least и не больше most метров. points - (широта,
/// долгота). None - точек нет.
pub fn shape_grid(points: &[(f64, f64)], radius: f64, reach: f64, least: f64, most: f64) -> Option<(f64, f64, f64)> {
    if points.is_empty() {
        return None;
    }
    let count = points.len() as f64;
    let lat0 = points.iter().map(|p| p.0.to_radians()).sum::<f64>() / count;
    let lon0 = points.iter().map(|p| p.1.to_radians()).sum::<f64>() / count;
    let mut half = 0.0f64;
    for (lat, lon) in points {
        half = half.max(((lat.to_radians() - lat0) * radius).abs());
        half = half.max(((lon.to_radians() - lon0) * radius * lat0.cos()).abs());
    }
    let side = (half * reach).max(least).min(most);
    Some((lat0.to_degrees(), lon0.to_degrees(), side))
}

/// Вид метки.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shape {
    Path,
    Polygon,
    Point,
}

/// Узлы сетки под меткой: у пути - вдоль линии, у многоугольника -
/// внутренность и контур, у точки - узел точки (инструмент 2.16 Isoliner
/// «Водосбор линий и контуров»). Ответ - (номера узлов, длина метки, м).
/// points - (широта, долгота).
pub fn shape_seeds(grid: &Grid, kind: Shape, points: &[(f64, f64)], radius: f64) -> (Vec<usize>, f64) {
    // Координаты узлов для topo_gauge: x - столбец, y - минус строка,
    // левый верхний угол клетки узла [0, 0] - (-0.5, 0.5).
    let xy: Vec<(f64, f64)> = points
        .iter()
        .map(|(lat, lon)| {
            let (x, y) = mercator(*lat, *lon, radius);
            ((x - grid.x0) / grid.step, -(grid.y0 - y) / grid.step)
        })
        .collect();
    if kind == Shape::Polygon && xy.len() >= 3 {
        let seeds = topo_gauge::cells_in_polygon(std::slice::from_ref(&xy), -0.5, 0.5, 1.0, grid.rows, grid.cols);
        let mut ring = xy.clone();
        ring.push(xy[0]);
        (seeds, topo_gauge::polyline_length(&ring) * grid.cell)
    } else {
        let seeds = topo_gauge::cells_along_polyline(&xy, -0.5, 0.5, 1.0, grid.rows, grid.cols, topo_gauge::STEP);
        (seeds, topo_gauge::polyline_length(&xy) * grid.cell)
    }
}

/// Водосбор метки.
#[derive(Debug, Clone, PartialEq)]
pub struct Catchment {
    pub mask: Vec<bool>,
    /// Отчёт по ключам topo_gauge::DITCH_KEYS.
    pub report: Report,
    /// Водосбор дошёл до края сетки.
    pub edge: bool,
}

/// Водосбор узлов seeds - всё, откуда вода приходит в метку. length -
/// длина метки, м, в отчёт.
pub fn shape_catchment(z: &[f64], rows: usize, cols: usize, seeds: &[usize], cell: f64, length: Option<f64>) -> Option<Catchment> {
    let d = drainage(z, rows, cols)?;
    let extras = Extras { nodata: Some(&d.empty), ..Extras::default() };
    let report = topo_gauge::ditch_report(&d.filled, &d.downstream, rows, cols, seeds, cell, length, &extras);
    let mask: Vec<bool> = topo_gauge::catchment_mask(&d.downstream, rows, cols, seeds).iter().zip(&d.empty).map(|(m, e)| *m && !*e).collect();
    let edge = touches_border(&mask, rows, cols);
    Some(Catchment { mask, report, edge })
}

/// Звено речной сети: узлы вниз по течению, порядок по Стралеру
/// и площадь водосбора в замыкании звена, м².
#[derive(Debug, Clone, PartialEq)]
pub struct Stream {
    pub rows: Vec<usize>,
    pub cols: Vec<usize>,
    pub order: u32,
    pub area: f64,
}

/// Речная сеть сетки - водотоки с водосбором от area м², звенья
/// с порядком Стралера (инструмент 2.06 Isoliner). order - наименьший
/// порядок звена в ответе, порядки считаются по всей сети.
///
/// Как у модуля, к звену добавляется приёмник его последнего узла:
/// звенья на глобусе идут без разрывов. Звено topo_flow уже кончается
/// узлом слияния, поэтому соседние звенья перекрываются на узел.
pub fn rivers(z: &[f64], rows: usize, cols: usize, cell: f64, area: f64, order: u32) -> Vec<Stream> {
    let Some(d) = drainage(z, rows, cols) else {
        return Vec::new();
    };
    let acc = topo_flow::flow_accumulation(&d.downstream, rows, cols, Some(&d.empty));
    let cells = (area / (cell * cell)).max(1.0);
    let mut out = Vec::new();
    for link in topo_flow::river_network(&d.downstream, &acc, cells, rows, cols) {
        if link.order < order {
            continue;
        }
        let mut chain = link.cells;
        if let Some(&last) = chain.last()
            && let Some(&nxt) = d.downstream.get(last)
            && nxt >= 0
        {
            chain.push(nxt as usize);
        }
        if chain.len() < 2 {
            continue;
        }
        out.push(Stream { rows: chain.iter().map(|i| i / cols).collect(), cols: chain.iter().map(|i| i % cols).collect(), order: link.order, area: link.acc_out * cell * cell });
    }
    out
}

// --- контур -------------------------------------------------------------------

/// Кольца границы маски в дробных номерах узлов [строка, столбец], первое -
/// самое длинное. Граница проходит посередине между мокрым и сухим
/// узлом, вершины упрощены по Дугласу-Пекеру с допуском tolerance ячеек.
pub fn outline(mask: &[bool], rows: usize, cols: usize, tolerance: f64) -> Vec<Vec<[f64; 2]>> {
    if mask.len() != rows.saturating_mul(cols) || mask.is_empty() {
        return Vec::new();
    }
    let padded = contours::Grid::from_fn(rows + 2, cols + 2, |r, c| if r >= 1 && c >= 1 && r <= rows && c <= cols && mask[(r - 1) * cols + c - 1] { 1.0 } else { 0.0 });
    let mut rings = Vec::new();
    for line in contours::chain(&contours::segments(&padded, 0.5), 6) {
        if line.len() < 4 {
            continue;
        }
        let close = |a: f64, b: f64| (a - b).abs() <= 1e-8 + 1e-5 * b.abs();
        let (first, last) = (line[0], line[line.len() - 1]);
        let closed = close(first[0], last[0]) && close(first[1], last[1]);
        let body = if closed { &line[..line.len() - 1] } else { &line[..] };
        let shifted: Vec<[f64; 2]> = body.iter().map(|p| [p[0] - 1.0, p[1] - 1.0]).collect();
        let ring = simplify(&shifted, tolerance);
        if ring.len() >= 3 {
            rings.push(ring);
        }
    }
    rings.sort_by_key(|ring| std::cmp::Reverse(ring.len()));
    rings
}

/// Дуглас-Пекер без рекурсии: вершины points, которые остаются.
/// Расстояние - до отрезка, а не до прямой: у кольца концы соседние,
/// и точки далеко вдоль их прямой иначе выпадали бы.
pub fn simplify(points: &[[f64; 2]], tolerance: f64) -> Vec<[f64; 2]> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0, points.len() - 1)];
    while let Some((first, last)) = stack.pop() {
        if last - first < 2 {
            continue;
        }
        let (a, b) = (points[first], points[last]);
        let d = [b[0] - a[0], b[1] - a[1]];
        let norm = d[0] * d[0] + d[1] * d[1];
        let mut worst = (first + 1, f64::NEG_INFINITY);
        for (k, p) in points.iter().enumerate().take(last).skip(first + 1) {
            let t = if norm == 0.0 { 0.0 } else { (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / norm).clamp(0.0, 1.0) };
            let dist = (p[0] - (a[0] + t * d[0])).hypot(p[1] - (a[1] + t * d[1]));
            if dist > worst.1 {
                worst = (k, dist);
            }
        }
        if worst.1 > tolerance {
            keep[worst.0] = true;
            stack.push((first, worst.0));
            stack.push((worst.0, last));
        }
    }
    points.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

// --- тайлы карты --------------------------------------------------------------

/// Узел сетки под центром каждого пикселя тайла key (уровень, x, y) Web
/// Mercator: TILE × TILE по строкам, None - пиксель вне сетки. Ближайший
/// узел, как у `relief.sample` модуля. None - тайл не задевает сетку.
pub fn tile_cells(grid: &Grid, key: (u32, u32, u32), radius: f64) -> Option<Vec<Option<usize>>> {
    let (z, x, y) = key;
    let n = f64::from(1u32 << z.min(30));
    let pi = std::f64::consts::PI;
    let mut out = vec![None; TILE * TILE];
    let mut any = false;
    for row in 0..TILE {
        let my = radius * pi * (1.0 - 2.0 * (f64::from(y) + (row as f64 + 0.5) / TILE as f64) / n);
        let r = ((grid.y0 - my) / grid.step).round_ties_even();
        if !(r >= 0.0 && r < grid.rows as f64) {
            continue;
        }
        for col in 0..TILE {
            let mx = radius * pi * (2.0 * (f64::from(x) + (col as f64 + 0.5) / TILE as f64) / n - 1.0);
            let c = ((mx - grid.x0) / grid.step).round_ties_even();
            if c >= 0.0 && c < grid.cols as f64 {
                out[row * TILE + col] = Some(r as usize * grid.cols + c as usize);
                any = true;
            }
        }
    }
    any.then_some(out)
}

/// Картинка тайла key с водой затопления flood: RGBA с премноженной
/// альфой, TILE × TILE. Цвет - по глубине level - z от DEPTH_COLORS[0]
/// у берега к DEPTH_COLORS[1] на наибольшей глубине затопления (не
/// меньше 0.1 м, как у растра глубин модуля), непрозрачность
/// FLOOD_OPACITY, сухое прозрачно. None - тайл не задевает сетку.
pub fn flood_tile_rgba(grid: &Grid, flood: &Flood, z: &[f64], level: f64, key: (u32, u32, u32), radius: f64) -> Option<Vec<u8>> {
    let cells = tile_cells(grid, key, radius)?;
    let mut out = vec![0u8; TILE * TILE * 4];
    for (k, cell) in cells.iter().enumerate() {
        if let Some(rgba) = cell.and_then(|i| depth_color(flood, z, level, i)) {
            out[k * 4..k * 4 + 4].copy_from_slice(&rgba);
        }
    }
    Some(out)
}

/// Вода затопления в точках (широта, долгота): RGBA с премноженной
/// альфой на точку, как у [`flood_tile_rgba`], вне сетки и сухое
/// прозрачно.
pub fn flood_points_rgba(grid: &Grid, flood: &Flood, z: &[f64], level: f64, points: &[(f64, f64)], radius: f64) -> Vec<[u8; 4]> {
    points
        .iter()
        .map(|&(lat, lon)| {
            let (x, y) = mercator(lat, lon, radius);
            let c = ((x - grid.x0) / grid.step).round_ties_even();
            let r = ((grid.y0 - y) / grid.step).round_ties_even();
            if !(r >= 0.0 && c >= 0.0 && r < grid.rows as f64 && c < grid.cols as f64) {
                return [0; 4];
            }
            depth_color(flood, z, level, r as usize * grid.cols + c as usize).unwrap_or([0; 4])
        })
        .collect()
}

/// Цвет воды узла i по глубине или None - узел сухой.
fn depth_color(flood: &Flood, z: &[f64], level: f64, i: usize) -> Option<[u8; 4]> {
    if !flood.mask.get(i).copied().unwrap_or(false) {
        return None;
    }
    let top = flood.depth.max(0.1);
    let depth = level - z.get(i).copied().unwrap_or(f64::NAN);
    let t = if depth.is_finite() { (depth / top).clamp(0.0, 1.0) } else { 1.0 };
    let mut out = [0u8; 4];
    for (ch, slot) in out.iter_mut().take(3).enumerate() {
        let (a, b) = (f64::from(DEPTH_COLORS[0][ch]), f64::from(DEPTH_COLORS[1][ch]));
        *slot = ((a + (b - a) * t) * FLOOD_OPACITY).round() as u8;
    }
    out[3] = (FLOOD_OPACITY * 255.0).round() as u8;
    Some(out)
}
