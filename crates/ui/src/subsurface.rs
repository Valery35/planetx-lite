// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Подземный режим, перенос `ui/subsurface.py` модуля: модель, сетки,
//! маска поверхности, пол камеры, окно.
//!
//! Данные - таблицы GeoPackage, как у модуля: collar, interval, survey,
//! beds (код, порядок, цвет, растр кровли), sections (линии разрезов),
//! cut (многоугольник выреза), tunnels (линии тоннелей). Роль таблицы -
//! по её имени, иначе по полям, `detect_role` модуля. Растры кровель
//! названы в поле surface таблицы beds. В программе они читаются из
//! папки файла GeoPackage, на странице и у демо приходят следом
//! отдельными файлами ([`Subsurface::wants`]).
//!
//! Сетки собирает `data::subsurface`. Высота на экране - отметка,
//! умноженная на масштаб рельефа, без рельефа - глубина под
//! поверхностью. Сетки собираются заново, когда пришли высоты рельефа
//! под моделью или сменился масштаб.
//!
//! Нет пока: изолиний на кровлях, картинок разрезов (таблица images),
//! раскраски растров и прозрачности слоёв QGIS, окна «Объекты» по
//! стволам, подписей устьев, слоёв с Z, шаблона GeoPackage.

use std::collections::HashMap;

use data::drillholes::{self, Hole, Row, Value};
use data::layers::{LayerData, Shape};
use data::subsurface::{self as core, Mask, Part, Rgba};
use eframe::egui;

use crate::i18n::{Lang, tr, trf};

/// Таблицы подземного режима, `ROLES` модуля без картинок разрезов.
pub const ROLES: [&str; 7] = ["collar", "interval", "survey", "beds", "sections", "cut", "tunnels"];
/// Узлов растра кровли на сторону, больше - прорежение.
pub const MAX_NODES: usize = 300;
/// Радиус ствола - доля поперечника модели, не меньше MIN_TUBE метров.
pub const TUBE_SHARE: f64 = 1.0 / 400.0;
pub const MIN_TUBE: f64 = 2.0;
/// Точек стенки разреза на линию, не больше.
pub const SECTION_POINTS: f64 = 400.0;
/// Метров ниже самой глубокой кровли, если скважин нет.
pub const BOTTOM_MARGIN: f64 = 20.0;
pub const EMPTY_COLOR: [u8; 3] = [150, 150, 150];
/// Цвета пластов без таблицы пластов, сверху вниз, как у модуля.
pub const BED_COLORS: [[u8; 3]; 8] = [[214, 168, 96], [122, 172, 112], [96, 146, 196], [186, 116, 150], [204, 128, 86], [118, 178, 184], [160, 150, 200], [190, 180, 110]];
pub const WALL_SHADE: f64 = 0.8;
pub const HOST_COLOR: [u8; 3] = [176, 168, 156];
pub const FLOOR_COLOR: [u8; 3] = [74, 68, 62];
/// Метров, на сколько верх стенки блока выше рельефа.
pub const BLOCK_RIM: f64 = 15.0;
/// Клеток контура кровель на сторону, не больше.
pub const FOOTPRINT_SIDE: usize = 2048;
/// Тоннели: диаметр, цвет и шаг точек оси по умолчанию, как у модуля.
pub const TUNNEL_DIAMETER: f64 = 3.7;
pub const TUNNEL_COLOR: [u8; 3] = [90, 160, 220];
pub const TUNNEL_STEP: f64 = 10.0;
/// Наибольшая сторона картинки маски поверхности.
pub const MASK_SIDE: usize = 1024;
/// Запас рамки маски и пола камеры за рамкой модели, доля.
pub const MASK_MARGIN: f64 = 0.05;
pub const FLOOR_MARGIN: f64 = 0.2;
/// Секунд после открытия, пока сетки собираются заново с приходом высот.
pub const SETTLE: f64 = 30.0;
/// Секунд между пересборками при приходе высот.
pub const REBUILD_GAP: f64 = 1.5;
/// Непрозрачность поверхности по умолчанию, `DEFAULTS` модуля.
pub const OPACITY: f32 = 0.6;
/// Метров в градусе широты для шага сгущения линий в градусах.
const DEGREE: f64 = 111_000.0;

/// Кровля пласта: узлы растра в широтах и долготах, отметки и растр
/// для выборки в любой точке.
pub struct Horizon {
    pub code: String,
    pub rows: usize,
    pub cols: usize,
    pub lats: Vec<f64>,
    pub lons: Vec<f64>,
    /// Отметки узлов, NaN - пусто.
    pub z: Vec<f64>,
    grid: data::grid::Grid,
}

impl Horizon {
    /// Кровля из растра GeoTIFF.
    pub fn read(code: &str, bytes: Vec<u8>, name: &str) -> Result<Horizon, String> {
        let grid = data::grid::Grid::read(bytes, name)?;
        let step = grid.width.max(grid.height).div_ceil(MAX_NODES).max(1);
        let cols = (grid.width / step).max(2);
        let rows = (grid.height / step).max(2);
        let (sx, sy) = (grid.width as f64 / cols as f64, grid.height as f64 / rows as f64);
        let mut lats = Vec::with_capacity(rows * cols);
        let mut lons = Vec::with_capacity(rows * cols);
        let mut z = Vec::with_capacity(rows * cols);
        for r in 0..rows {
            for c in 0..cols {
                let (pc, pr) = ((c as f64 + 0.5) * sx, (r as f64 + 0.5) * sy);
                let (x, y) = grid.at(pc, pr);
                let (lat, lon) = grid.lat_lon(x, y).ok_or("no coordinate system")?;
                lats.push(lat);
                lons.push(lon);
                z.push(f64::from(grid.bilinear(pc, pr)));
            }
        }
        Ok(Horizon { code: code.to_string(), rows, cols, lats, lons, z, grid })
    }

    /// Отметки кровли в точках, вне растра - NaN.
    pub fn at(&self, lats: &[f64], lons: &[f64]) -> Vec<f64> {
        let Some(t) = self.grid.transformer_from(4326) else {
            return vec![f64::NAN; lats.len().min(lons.len())];
        };
        lats.iter()
            .zip(lons)
            .map(|(lat, lon)| {
                let Ok((x, y)) = t.transform(*lon, *lat) else {
                    return f64::NAN;
                };
                self.grid.pixel_of(x, y).map_or(f64::NAN, |(c, r)| f64::from(self.grid.bilinear(c, r)))
            })
            .collect()
    }

    fn has_data(&self) -> bool {
        self.z.iter().any(|v| v.is_finite())
    }

    fn mean(&self) -> f64 {
        let ok: Vec<f64> = self.z.iter().copied().filter(|v| v.is_finite()).collect();
        if ok.is_empty() { f64::NEG_INFINITY } else { ok.iter().sum::<f64>() / ok.len() as f64 }
    }

    fn same_grid(&self, other: &Horizon) -> bool {
        let close = |a: &[f64], b: &[f64]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1e-9 * x.abs().max(1.0));
        self.rows == other.rows && self.cols == other.cols && close(&self.lats, &other.lats) && close(&self.lons, &other.lons)
    }
}

/// Тоннель: название, точки оси (широта, долгота), глубина в начале
/// и в конце, диаметр, цвет.
pub struct Tunnel {
    pub name: String,
    pub points: Vec<[f64; 2]>,
    pub depths: (f64, f64),
    pub diameter: f64,
    pub color: [u8; 3],
}

/// Подземные данные: скважины, кровли, разрезы, вырез, пласты.
#[derive(Default)]
pub struct Model {
    /// Скважина, широта и долгота устья.
    pub holes: Vec<(Hole, f64, f64)>,
    /// Кровли сверху вниз.
    pub horizons: Vec<Horizon>,
    /// Коды пластов сверху вниз.
    pub beds: Vec<String>,
    pub palette: HashMap<String, String>,
    pub sections: Vec<Vec<[f64; 2]>>,
    /// Кольца выреза, сгущённые для стенок.
    pub rings: Vec<Vec<[f64; 2]>>,
    /// Те же кольца по вершинам, для маски.
    pub mask_rings: Vec<Vec<[f64; 2]>>,
    pub tunnels: Vec<Tunnel>,
    /// Растры кровель из таблицы пластов: код и имя файла.
    pub surfaces: Vec<(String, String)>,
    /// Не найденные растры.
    pub missing: Vec<String>,
    pub skipped: usize,
}

/// Значения строки таблицы по полям.
fn row_of(layer: &LayerData, values: &[String]) -> Row {
    layer
        .fields
        .iter()
        .zip(values)
        .map(|(f, v)| {
            let value = if v.is_empty() {
                Value::Null
            } else if let Ok(n) = v.trim().parse::<f64>() {
                Value::Float(n)
            } else {
                Value::Text(v.clone())
            };
            (f.clone(), value)
        })
        .collect()
}

fn field<'a>(layer: &LayerData, values: &'a [String], name: &str) -> Option<&'a str> {
    layer.fields.iter().position(|f| f.eq_ignore_ascii_case(name)).and_then(|i| values.get(i)).map(String::as_str)
}

fn number(layer: &LayerData, values: &[String], name: &str, default: f64) -> f64 {
    field(layer, values, name).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite()).unwrap_or(default)
}

/// Цвет «#rrggbb» или default.
fn hex_color(text: &str, default: [u8; 3]) -> [u8; 3] {
    let t = text.trim().trim_start_matches('#');
    let part = |i: usize| t.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok());
    match (t.len(), part(0), part(2), part(4)) {
        (6, Some(r), Some(g), Some(b)) => [r, g, b],
        _ => default,
    }
}

fn with_alpha(c: [u8; 3], alpha: u8) -> Rgba {
    [c[0], c[1], c[2], alpha]
}

/// Роль таблицы подземного режима, `detect_role` модуля: по имени,
/// иначе по полям.
pub fn detect_role(layer: &LayerData) -> Option<&'static str> {
    let name = layer.name.trim().to_lowercase();
    if let Some(role) = ROLES.iter().find(|r| **r == name) {
        return Some(role);
    }
    let names = || layer.fields.iter().map(String::as_str);
    let has = |roles: &[drillholes::Role]| roles.iter().all(|r| drillholes::find_field(names(), *r).is_some());
    let lower: Vec<String> = layer.fields.iter().map(|f| f.to_lowercase()).collect();
    let any = |n: &str| lower.iter().any(|f| f == n);
    let shape = layer.items.first().map_or(Shape::None, |i| i.shape);
    use drillholes::Role::*;
    match shape {
        Shape::Point if has(&[HoleId, Eoh]) => Some("collar"),
        Shape::None if has(&[HoleId, From, To]) => Some("interval"),
        Shape::None if has(&[HoleId, Md, Azimuth]) => Some("survey"),
        Shape::None if has(&[Code]) && (any("ord") || any("color")) => Some("beds"),
        Shape::Line if any("depth") && any("diameter") => Some("tunnels"),
        _ => None,
    }
}

/// Есть ли в слоях файла подземные данные: устья, пласты или тоннели.
pub fn is_subsurface(layers: &[LayerData]) -> bool {
    layers.iter().filter_map(detect_role).any(|r| matches!(r, "collar" | "beds" | "tunnels"))
}

/// Сгущённые части линий или кольца слоя, шаг в метрах, None - по
/// вершинам. Кольцо - без повтора первой точки.
fn shapes(layer: &LayerData, ring: bool, step: Option<f64>) -> Vec<(usize, Vec<[f64; 2]>)> {
    let mut out = Vec::new();
    for (i, item) in layer.items.iter().enumerate() {
        let parts: Vec<&Vec<(f64, f64)>> = if ring { item.parts.iter().take(1).collect() } else { item.parts.iter().collect() };
        for part in parts {
            let pts: Vec<[f64; 2]> = part.iter().map(|(lat, lon)| [*lat, *lon]).collect();
            if pts.len() < 2 {
                continue;
            }
            let mut dense = step.map_or_else(|| pts.clone(), |s| core::densify(&pts, s / DEGREE));
            if ring && dense.len() > 1 && dense.first() == dense.last() {
                dense.pop();
            }
            out.push((i, dense));
        }
    }
    out
}

impl Model {
    /// Модель по слоям файла. side - байты файла рядом по имени.
    pub fn load(layers: &[LayerData], side: &dyn Fn(&str) -> Option<Vec<u8>>) -> Model {
        let mut model = Model::default();
        let role = |name: &str| layers.iter().find(|l| detect_role(l) == Some(name));
        if let Some(beds) = role("beds") {
            let mut rows: Vec<&data::layers::Item> = beds.items.iter().collect();
            let ord = |item: &data::layers::Item| field(beds, &item.values, "ord").and_then(|v| v.trim().parse::<f64>().ok());
            rows.sort_by(|a, b| (ord(a).is_none(), ord(a).unwrap_or(0.0)).partial_cmp(&(ord(b).is_none(), ord(b).unwrap_or(0.0))).unwrap_or(std::cmp::Ordering::Equal));
            let code_field = drillholes::find_field(beds.fields.iter().map(String::as_str), drillholes::Role::Code).unwrap_or("code");
            for item in rows {
                let code = field(beds, &item.values, code_field).unwrap_or("").to_string();
                if let Some(color) = field(beds, &item.values, "color").filter(|c| !c.is_empty()) {
                    model.palette.insert(code.clone(), color.to_string());
                }
                if let Some(surface) = field(beds, &item.values, "surface").filter(|s| !s.trim().is_empty()) {
                    let file = surface.rsplit(['/', '\\']).next().unwrap_or(surface).to_string();
                    model.surfaces.push((code.clone(), file));
                }
                model.beds.push(code);
            }
        }
        for (code, file) in model.surfaces.clone() {
            match side(&file).map(|b| Horizon::read(&code, b, &file)) {
                Some(Ok(h)) => model.horizons.push(h),
                _ => model.missing.push(file),
            }
        }
        // Кровли - по порядку пластов, неизвестные - по средней отметке.
        let order: HashMap<String, usize> = model.beds.iter().enumerate().map(|(i, c)| (c.clone(), i)).collect();
        model.horizons.sort_by(|a, b| {
            let rank = |h: &Horizon| order.get(&h.code).map_or(-h.mean(), |i| *i as f64);
            rank(a).partial_cmp(&rank(b)).unwrap_or(std::cmp::Ordering::Equal)
        });
        // Кровли без цвета в таблице - цвет пласта по порядку сверху.
        let mut keys: Vec<String> = Vec::new();
        for h in &model.horizons {
            let key = core::bed_key(&h.code).0;
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        for h in &model.horizons {
            if !model.palette.contains_key(&h.code) {
                let key = core::bed_key(&h.code).0;
                let color = model.palette.get(&key).cloned().unwrap_or_else(|| {
                    let i = keys.iter().position(|k| *k == key).unwrap_or(0) % BED_COLORS.len();
                    let c = BED_COLORS[i];
                    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
                });
                model.palette.insert(h.code.clone(), color);
            }
        }
        if let Some(collar) = role("collar") {
            let mut collars = Vec::new();
            let mut at: HashMap<String, (f64, f64)> = HashMap::new();
            for item in &collar.items {
                let Some(&(lat, lon)) = item.parts.first().and_then(|p| p.first()) else {
                    model.skipped += 1;
                    continue;
                };
                let mut row = row_of(collar, &item.values);
                row.insert("x".into(), Value::Float(lon));
                row.insert("y".into(), Value::Float(lat));
                at.insert(format!("{lon:.12} {lat:.12}"), (lat, lon));
                collars.push(row);
            }
            let rows = |name: &str| role(name).map(|l| l.items.iter().map(|i| row_of(l, &i.values)).collect::<Vec<Row>>()).unwrap_or_default();
            let (holes, skipped) = drillholes::assemble(&collars, &rows("interval"), &rows("survey"));
            model.skipped += skipped.values().sum::<usize>();
            for hole in holes {
                let key = format!("{:.12} {:.12}", hole.x.unwrap_or(f64::NAN), hole.y.unwrap_or(f64::NAN));
                if let Some((lat, lon)) = at.get(&key).copied() {
                    model.holes.push((hole, lat, lon));
                }
            }
        }
        let step = (model.extent() / SECTION_POINTS).max(5.0);
        if let Some(l) = role("sections") {
            model.sections = shapes(l, false, Some(step)).into_iter().map(|(_, p)| p).collect();
        }
        if let Some(l) = role("cut") {
            model.rings = shapes(l, true, Some(step)).into_iter().map(|(_, p)| p).collect();
            model.mask_rings = shapes(l, true, None).into_iter().map(|(_, p)| p).collect();
        }
        if let Some(l) = role("tunnels") {
            for (i, pts) in shapes(l, false, Some(TUNNEL_STEP)) {
                let Some(item) = l.items.get(i) else {
                    continue;
                };
                let v = &item.values;
                let start = number(l, v, "depth", 0.0);
                model.tunnels.push(Tunnel {
                    name: field(l, v, "name").unwrap_or("").to_string(),
                    points: pts,
                    depths: (start, number(l, v, "depth_end", start)),
                    diameter: number(l, v, "diameter", TUNNEL_DIAMETER),
                    color: hex_color(field(l, v, "color").unwrap_or(""), TUNNEL_COLOR),
                });
            }
        }
        model
    }

    pub fn color(&self, code: &str) -> [u8; 3] {
        if code.is_empty() {
            return EMPTY_COLOR;
        }
        drillholes::code_color(code, Some(&self.palette))
    }

    /// Рамка данных: юг, север, запад, восток. grids - с кровлями.
    pub fn bounds(&self, grids: bool) -> Option<[f64; 4]> {
        let mut b = [f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY];
        let mut add = |lat: f64, lon: f64| {
            if lat.is_finite() && lon.is_finite() {
                b = [b[0].min(lat), b[1].max(lat), b[2].min(lon), b[3].max(lon)];
            }
        };
        for (_, lat, lon) in &self.holes {
            add(*lat, *lon);
        }
        if grids {
            for h in self.horizons.iter() {
                for (lat, lon) in h.lats.iter().zip(&h.lons) {
                    add(*lat, *lon);
                }
            }
        }
        for line in self.sections.iter().chain(&self.rings).chain(self.tunnels.iter().map(|t| &t.points)) {
            for p in line {
                add(p[0], p[1]);
            }
        }
        (b[0] <= b[1] && b[2] <= b[3]).then_some(b)
    }

    /// Поперечник рамки, метры.
    pub fn extent(&self) -> f64 {
        let Some([s, n, w, e]) = self.bounds(true) else {
            return 0.0;
        };
        let dy = (n - s).to_radians() * core::A;
        let dx = (e - w).to_radians() * core::A * ((n + s) / 2.0).to_radians().cos();
        dx.hypot(dy)
    }

    /// Отметка низа модели: забой самой глубокой скважины или ниже самой
    /// глубокой кровли на BOTTOM_MARGIN.
    pub fn bottom(&self) -> f64 {
        let mut lows: Vec<f64> = self.holes.iter().filter_map(|(h, _, _)| h.axis.iter().map(|n| n[3]).reduce(f64::min)).collect();
        let roofs = self.horizons.iter().flat_map(|h| h.z.iter().copied()).filter(|v| v.is_finite()).reduce(f64::min);
        if let Some(r) = roofs {
            lows.push(r - BOTTOM_MARGIN);
        }
        lows.into_iter().filter(|v| v.is_finite()).reduce(f64::min).unwrap_or(0.0)
    }

    /// Контур кровель: рамка и клетки с узлами данных, `footprint`
    /// модуля. None - без кровель.
    pub fn footprint(&self) -> Option<([f64; 4], usize, usize, Vec<bool>)> {
        let grids: Vec<&Horizon> = self.horizons.iter().filter(|h| h.has_data()).collect();
        if grids.is_empty() {
            return None;
        }
        let b = self.bounds(true)?;
        let [s, n, w, e] = b;
        if n <= s || e <= w {
            return None;
        }
        let span = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max) - v.iter().copied().fold(f64::INFINITY, f64::min);
        let dlat = grids.iter().map(|h| span(&h.lats) / (h.rows.max(2) - 1) as f64).fold(f64::INFINITY, f64::min);
        let dlon = grids.iter().map(|h| span(&h.lons) / (h.cols.max(2) - 1) as f64).fold(f64::INFINITY, f64::min);
        if !(dlat > 0.0 && dlon > 0.0) {
            return None;
        }
        let ny = (((n - s) / dlat) as usize + 1).clamp(2, FOOTPRINT_SIDE);
        let nx = (((e - w) / dlon) as usize + 1).clamp(2, FOOTPRINT_SIDE);
        let mut cells = vec![false; ny * nx];
        for h in grids {
            for ((lat, lon), z) in h.lats.iter().zip(&h.lons).zip(&h.z) {
                if !z.is_finite() {
                    continue;
                }
                let i = (((lat - s) / (n - s) * (ny - 1) as f64).round().max(0.0) as usize).min(ny - 1);
                let j = (((lon - w) / (e - w) * (nx - 1) as f64).round().max(0.0) as usize).min(nx - 1);
                cells[i * nx + j] = true;
            }
        }
        let cells = erode(&erode(&dilate(&cells, ny, nx), ny, nx), ny, nx);
        Some((b, ny, nx, cells))
    }
}

/// Клетки с соседом-клеткой в окне 3×3.
fn dilate(cells: &[bool], ny: usize, nx: usize) -> Vec<bool> {
    window(cells, ny, nx, false)
}

/// Клетки, у которых всё окно 3×3 - клетки.
fn erode(cells: &[bool], ny: usize, nx: usize) -> Vec<bool> {
    window(cells, ny, nx, true)
}

fn window(cells: &[bool], ny: usize, nx: usize, all: bool) -> Vec<bool> {
    let at = |i: isize, j: isize| i >= 0 && j >= 0 && (i as usize) < ny && (j as usize) < nx && cells[i as usize * nx + j as usize];
    (0..ny * nx)
        .map(|k| {
            let (i, j) = ((k / nx) as isize, (k % nx) as isize);
            let mut near = (-1..=1).flat_map(|di| (-1..=1).map(move |dj| (di, dj))).map(|(di, dj)| at(i + di, j + dj));
            if all { near.all(|v| v) } else { near.any(|v| v) }
        })
        .collect()
}

/// Смещения в метрах от устья в широту и долготу.
fn to_latlon(lat0: f64, lon0: f64, east: f64, north: f64) -> (f64, f64) {
    (lat0 + (north / core::A).to_degrees(), lon0 + (east / (core::A * lat0.to_radians().cos())).to_degrees())
}

/// Настоящие отметки рельефа в точках (широта, долгота).
pub type Ground<'a> = dyn Fn(&[(f64, f64)]) -> Vec<f64> + 'a;

/// Кровля пласта для торцов плиты: номер, точки, узлы с данными.
type Side = (usize, Vec<[f64; 3]>, Vec<bool>);

/// Сетки по именам для видеокарты.
pub type Meshes = Vec<(&'static str, Option<render::SubMesh>)>;

fn to_sub(mesh: Option<core::Mesh>, cull_front: bool) -> Option<render::SubMesh> {
    let mesh = mesh?;
    let vertices = mesh
        .vertices
        .iter()
        .map(|v| render::SubVertex {
            offset: v.position,
            normal: [f32::from(v.normal[0]) / 127.0, f32::from(v.normal[1]) / 127.0, f32::from(v.normal[2]) / 127.0],
            color: v.color,
        })
        .collect();
    Some(render::SubMesh { center: globe_core::DVec3::from_array(mesh.center), vertices, indices: mesh.indices, cull_front, bias: false })
}

/// Сетки модели, `build` модуля. ground - настоящие отметки рельефа
/// в точках, cut - вырез блока включён.
pub fn build(model: &Model, scale: f64, ground: &Ground<'_>, cut: bool) -> (Meshes, usize) {
    let ground_at = |lats: &[f64], lons: &[f64]| -> Vec<f64> {
        let pts: Vec<(f64, f64)> = lats.iter().zip(lons).map(|(a, b)| (*a, *b)).collect();
        ground(&pts)
    };
    let radius = MIN_TUBE.max(model.extent() * TUBE_SHARE);
    let mut wells: Vec<Part> = Vec::new();
    for (hole, lat0, lon0) in &model.holes {
        let g0 = ground(&[(*lat0, *lon0)]).first().copied().unwrap_or(0.0);
        let pieces: Vec<(f64, f64, Rgba)> = if hole.intervals.is_empty() {
            vec![(0.0, hole.eoh, with_alpha(EMPTY_COLOR, 255))]
        } else {
            hole.intervals.iter().map(|i| (i.start, i.end, with_alpha(model.color(&i.code), 255))).collect()
        };
        for (start, end, color) in pieces {
            let pts = drillholes::piece(&hole.axis, start, end);
            let xyz: Vec<[f64; 3]> = pts
                .iter()
                .map(|p| {
                    let (lat, lon) = to_latlon(*lat0, *lon0, p[0], p[1]);
                    core::geodetic_to_ecef(lat, lon, core::display(p[2], scale, g0))
                })
                .collect();
            wells.extend(core::tube(&xyz, radius, color, core::SIDES));
        }
    }
    for t in &model.tunnels {
        let (lats, lons): (Vec<f64>, Vec<f64>) = t.points.iter().map(|p| (p[0], p[1])).unzip();
        let g = ground_at(&lats, &lons);
        let share = core::along(&lats, &lons);
        let xyz: Vec<[f64; 3]> = (0..lats.len().min(g.len()).min(share.len()))
            .map(|i| {
                let z = core::tunnel_z(g[i], t.depths.0, t.depths.1, share[i]);
                core::geodetic_to_ecef(lats[i], lons[i], core::display(z, scale, g[i]))
            })
            .collect();
        wells.extend(core::tube(&xyz, t.diameter / 2.0, with_alpha(t.color, 255), core::SIDES));
    }
    let bottom = model.bottom();
    let ring_out = |lats: &[f64], lons: &[f64], valid: &mut [bool]| {
        if cut {
            for ring in &model.mask_rings {
                for (v, hit) in valid.iter_mut().zip(core::inside(lats, lons, ring)) {
                    *v &= !hit;
                }
            }
        }
    };
    let mut surfaces: Vec<Part> = Vec::new();
    // Пласт - кровля и подошва на одной сетке: торцы плиты.
    let mut sides: HashMap<String, [Option<Side>; 2]> = HashMap::new();
    for (k, h) in model.horizons.iter().enumerate() {
        let mut valid: Vec<bool> = h.z.iter().map(|v| v.is_finite()).collect();
        ring_out(&h.lats, &h.lons, &mut valid);
        let g = if scale == 0.0 { ground_at(&h.lats, &h.lons) } else { vec![0.0; h.z.len()] };
        let points: Vec<[f64; 3]> = (0..h.z.len())
            .map(|i| {
                let z = if h.z[i].is_finite() { h.z[i] } else { 0.0 };
                core::geodetic_to_ecef(h.lats[i], h.lons[i], core::display(z, scale, g.get(i).copied().unwrap_or(0.0)))
            })
            .collect();
        let mask = Mask { rows: h.rows, cols: h.cols, valid: valid.clone() };
        surfaces.extend(core::grid_surface(&points, &mask, &[with_alpha(model.color(&h.code), 255)]));
        let (key, side) = core::bed_key(&h.code);
        if let Some(side) = side {
            let slot = usize::from(side == core::Side::Bottom);
            sides.entry(key).or_default()[slot] = Some((k, points, valid));
        }
    }
    // Блок под кровлями: стенки по краю данных и дно.
    let mut block: Vec<Part> = Vec::new();
    let mut edge: Option<(usize, Vec<[u32; 2]>)> = None;
    let grids: Vec<usize> = (0..model.horizons.len()).filter(|i| model.horizons[*i].has_data()).collect();
    if let Some(base) = grids.iter().copied().max_by_key(|i| model.horizons[*i].z.iter().filter(|v| v.is_finite()).count()) {
        let b = &model.horizons[base];
        let mut valid = vec![false; b.z.len()];
        let mut rows: Vec<Vec<f64>> = Vec::new();
        for h in &model.horizons {
            if h.same_grid(b) {
                for (v, z) in valid.iter_mut().zip(&h.z) {
                    *v |= z.is_finite();
                }
                rows.push(h.z.clone());
            } else {
                rows.push(h.at(&b.lats, &b.lons));
            }
        }
        ring_out(&b.lats, &b.lons, &mut valid);
        let g = ground_at(&b.lats, &b.lons);
        let mut all = vec![g.clone()];
        all.extend(rows);
        all.push(vec![bottom; b.z.len()]);
        let stacked = core::stack(&all);
        let mut shown: Vec<Vec<f64>> = stacked.iter().map(|s| s.iter().zip(&g).map(|(z, g)| core::display(*z, scale, *g)).collect()).collect();
        if let Some(top) = shown.first_mut() {
            for v in top.iter_mut() {
                *v += BLOCK_RIM * scale.max(1.0);
            }
        }
        let points: Vec<Vec<[f64; 3]>> = shown.iter().map(|s| (0..s.len()).map(|i| core::geodetic_to_ecef(b.lats[i], b.lons[i], s[i])).collect()).collect();
        let above = model.beds.first().cloned().unwrap_or_default();
        let mut colors = vec![with_alpha(if above.is_empty() { HOST_COLOR } else { model.color(&above) }, 255)];
        for h in &model.horizons {
            let bottom_side = core::bed_key(&h.code).1 == Some(core::Side::Bottom);
            colors.push(with_alpha(if bottom_side { HOST_COLOR } else { model.color(&h.code) }, 255));
        }
        let mask = Mask { rows: b.rows, cols: b.cols, valid };
        block.extend(core::block_walls(&points, &mask, &colors));
        if let Some(floor) = points.last() {
            surfaces.extend(core::grid_surface(floor, &mask, &[with_alpha(FLOOR_COLOR, 255)]));
        }
        edge = Some((base, core::boundary_edges(&mask)));
    }
    for pair in sides.values() {
        let [Some((top, up, v1)), Some((low, down, v2))] = pair else {
            continue;
        };
        let (t, l) = (&model.horizons[*top], &model.horizons[*low]);
        if !t.same_grid(l) {
            continue;
        }
        let shade = model.color(&t.code).map(|c| (f64::from(c) * WALL_SHADE) as u8);
        let valid: Vec<bool> = v1.iter().zip(v2).map(|(a, b)| *a && *b).collect();
        let skip = match &edge {
            Some((base, edges)) if model.horizons[*base].same_grid(t) => edges.clone(),
            _ => Vec::new(),
        };
        surfaces.extend(core::grid_walls(up, down, &Mask { rows: t.rows, cols: t.cols, valid }, with_alpha(shade, 255), &skip));
    }
    // Разрезы и край выреза - стенки пластов между поверхностями.
    let mut lines: Vec<Vec<[f64; 2]>> = model.sections.clone();
    if cut {
        for r in &model.rings {
            let mut closed = r.clone();
            if let Some(first) = r.first() {
                closed.push(*first);
            }
            lines.push(closed);
        }
    }
    let top = model.beds.first().cloned().unwrap_or_default();
    let mut colors = vec![with_alpha(model.color(&top), 255)];
    colors.extend(model.horizons.iter().map(|h| with_alpha(model.color(&h.code), 255)));
    let mut walls: Vec<Part> = Vec::new();
    for line in &lines {
        let (lats, lons): (Vec<f64>, Vec<f64>) = line.iter().map(|p| (p[0], p[1])).unzip();
        let g = ground_at(&lats, &lons);
        let mut all = vec![g.clone()];
        all.extend(model.horizons.iter().map(|h| h.at(&lats, &lons)));
        all.push(vec![bottom; lats.len()]);
        let shown: Vec<Vec<f64>> = core::stack(&all).iter().map(|s| s.iter().zip(&g).map(|(z, g)| core::display(*z, scale, *g)).collect()).collect();
        walls.extend(core::fence(&lats, &lons, &shown, &colors).into_iter().flatten());
    }
    if cut {
        for ring in &model.rings {
            let (lats, lons): (Vec<f64>, Vec<f64>) = ring.iter().map(|p| (p[0], p[1])).unzip();
            let g = ground_at(&lats, &lons);
            let mean = if g.is_empty() { 0.0 } else { g.iter().sum::<f64>() / g.len() as f64 };
            if let Some(color) = colors.last() {
                walls.extend(core::floor_part(ring, core::display(bottom, scale, mean), *color));
            }
        }
    }
    let meshes: Meshes = vec![
        ("wells", to_sub(core::merge(&wells), false)),
        ("horizons", to_sub(core::merge(&surfaces), false)),
        ("block", to_sub(core::merge(&block), true)),
        ("sections", to_sub(core::merge(&walls), false)),
    ];
    let vertices = meshes.iter().filter_map(|(_, m)| m.as_ref()).map(|m| m.vertices.len()).sum();
    (meshes, vertices)
}

/// Доли мира Web Mercator точки: u - запад-восток, v - север-юг.
fn share(lat: f64, lon: f64) -> (f64, f64) {
    terrain::mercator_share(lat, lon)
}

/// Широта по доле v мира Web Mercator.
fn lat_of(v: f64) -> f64 {
    (std::f64::consts::PI * (1.0 - 2.0 * v)).sinh().atan().to_degrees()
}

/// Маска поверхности, `cut_rgba` модуля: рамка в долях мира (запад,
/// север, восток, юг), ширина, высота, RGBA. Красный - поверхность
/// прозрачна (контур кровель и рамка прочих данных), альфа - вырез.
pub fn mask(model: &Model, cut: bool) -> Option<([f64; 4], u32, u32, Vec<u8>)> {
    let [s, n, w, e] = model.bounds(true)?;
    let (dl, dw) = ((n - s).max(1e-6) * MASK_MARGIN, (e - w).max(1e-6) * MASK_MARGIN);
    let (u0, v0) = share(n + dl, w - dw);
    let (u1, v1) = share(s - dl, e + dw);
    if !(u1 > u0 && v1 > v0) {
        return None;
    }
    let aspect = (u1 - u0) / (v1 - v0);
    let (width, height) = if aspect >= 1.0 { (MASK_SIDE, ((MASK_SIDE as f64 / aspect) as usize).max(4)) } else { (((MASK_SIDE as f64 * aspect) as usize).max(4), MASK_SIDE) };
    let footprint = model.footprint();
    let others = model.bounds(false);
    let mut rgba = vec![0u8; width * height * 4];
    let (mut lats, mut lons) = (Vec::with_capacity(width * height), Vec::with_capacity(width * height));
    for r in 0..height {
        let lat = lat_of(v0 + (r as f64 + 0.5) / height as f64 * (v1 - v0));
        for c in 0..width {
            lats.push(lat);
            lons.push((u0 + (c as f64 + 0.5) / width as f64 * (u1 - u0)) * 360.0 - 180.0);
        }
    }
    let in_box = |b: &[f64; 4], lat: f64, lon: f64| lat >= b[0] && lat <= b[1] && lon >= b[2] && lon <= b[3];
    let mut hits = vec![false; lats.len()];
    if cut {
        for ring in &model.mask_rings {
            for (h, x) in hits.iter_mut().zip(core::inside(&lats, &lons, ring)) {
                *h |= x;
            }
        }
    }
    for k in 0..lats.len() {
        let (r, c) = (k / width, k % width);
        // Край картинки пуст: за рамкой берётся крайняя точка.
        if r == 0 || c == 0 || r + 1 == height || c + 1 == width {
            continue;
        }
        let (lat, lon) = (lats[k], lons[k]);
        let region = match &footprint {
            None => in_box(&[s, n, w, e], lat, lon),
            Some((b, ny, nx, cells)) => {
                let inside = in_box(b, lat, lon) && {
                    let i = (((lat - b[0]) / (b[1] - b[0]) * (*ny - 1) as f64).round().max(0.0) as usize).min(ny - 1);
                    let j = (((lon - b[2]) / (b[3] - b[2]) * (*nx - 1) as f64).round().max(0.0) as usize).min(nx - 1);
                    cells[i * nx + j]
                };
                inside || others.is_some_and(|o| in_box(&o, lat, lon))
            }
        };
        if region {
            rgba[k * 4] = 255;
        }
        if hits[k] {
            rgba[k * 4 + 3] = 255;
        }
    }
    Some(([u0, v0, u1, v1], width as u32, height as u32, rgba))
}

/// Пол камеры под землёй, `_floor` модуля: рамка модели с запасом и
/// отметка низа модели.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Floor {
    pub bounds: [f64; 4],
    pub bottom: f64,
}

impl Floor {
    /// Высота пола в точке на экране или None вне рамки. ground -
    /// настоящая отметка рельефа, нужна без рельефа (scale 0).
    pub fn at(&self, lat: f64, lon: f64, scale: f64, ground: impl Fn() -> f64) -> Option<f64> {
        let [s, n, w, e] = self.bounds;
        if !(lat >= s && lat <= n && lon >= w && lon <= e) {
            return None;
        }
        Some(if scale != 0.0 { self.bottom * scale } else { self.bottom - ground() })
    }
}

/// Настройки окна: непрозрачность, вырез, камера под землёй.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    pub opacity: f32,
    pub cut: bool,
    pub under: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { opacity: OPACITY, cut: true, under: false }
    }
}

/// Подземный режим окна глобуса.
pub struct Subsurface {
    /// Слои файла и его имя: модель собирается заново, когда приходят
    /// растры кровель.
    layers: Vec<LayerData>,
    pub name: String,
    /// Файлы рядом: имя в нижнем регистре - байты.
    side: HashMap<String, Vec<u8>>,
    /// Папка файла в программе, растры читаются из неё.
    pub folder: Option<std::path::PathBuf>,
    pub model: Option<Model>,
    pub options: Options,
    /// Нужно собрать сетки заново.
    pub dirty: bool,
    /// Маску поставить заново.
    pub mask_dirty: bool,
    pub opened_at: f64,
    built_at: f64,
    built_version: u64,
    built_scale: f64,
    pub status: String,
    pub window_open: bool,
}

impl Subsurface {
    /// Подземный режим по слоям файла name.
    pub fn new(layers: Vec<LayerData>, name: &str, folder: Option<std::path::PathBuf>, now: f64) -> Subsurface {
        let mut s = Subsurface {
            layers,
            name: name.to_string(),
            side: HashMap::new(),
            folder,
            model: None,
            options: Options::default(),
            dirty: true,
            mask_dirty: true,
            opened_at: now,
            built_at: f64::NEG_INFINITY,
            built_version: u64::MAX,
            built_scale: f64::NAN,
            status: String::new(),
            window_open: false,
        };
        s.reload();
        s
    }

    /// Модель заново по слоям и файлам рядом.
    fn reload(&mut self) {
        let side = &self.side;
        let folder = self.folder.clone();
        let read = |name: &str| -> Option<Vec<u8>> {
            if let Some(b) = side.get(&name.to_lowercase()) {
                return Some(b.clone());
            }
            read_near(folder.as_deref(), name)
        };
        self.model = Some(Model::load(&self.layers, &read));
        self.dirty = true;
        self.mask_dirty = true;
    }

    /// Ждёт ли модель файл name (растр кровли из таблицы пластов).
    pub fn wants(&self, name: &str) -> bool {
        let lower = name.to_lowercase();
        self.model.as_ref().is_some_and(|m| m.missing.iter().any(|f| f.to_lowercase() == lower))
    }

    /// Пришёл файл рядом: модель заново.
    pub fn add_side(&mut self, name: &str, bytes: Vec<u8>) {
        self.side.insert(name.to_lowercase(), bytes);
        self.reload();
    }

    /// Пора ли собрать сетки: данные или вырез сменились, сменился
    /// масштаб, после открытия пришли новые высоты.
    pub fn due(&self, now: f64, version: u64, scale: f64) -> bool {
        self.model.is_some()
            && (self.dirty || scale != self.built_scale || (version != self.built_version && now - self.opened_at < SETTLE && now - self.built_at > REBUILD_GAP))
    }

    /// Сетки собраны.
    pub fn built(&mut self, lang: Lang, now: f64, version: u64, scale: f64, vertices: usize) {
        self.dirty = false;
        self.built_at = now;
        self.built_version = version;
        self.built_scale = scale;
        self.status = self.summary(lang, vertices);
    }

    /// Сводка модели, как `apply` модуля.
    pub fn summary(&self, lang: Lang, vertices: usize) -> String {
        let Some(m) = &self.model else {
            return String::new();
        };
        let mut text = trf(
            lang,
            "Drill holes {holes}, roofs {horizons}, sections {sections}. Vertices {vertices}.",
            &[("holes", &m.holes.len().to_string()), ("horizons", &m.horizons.len().to_string()), ("sections", &m.sections.len().to_string()), ("vertices", &vertices.to_string())],
        );
        if !m.tunnels.is_empty() {
            text.push(' ');
            text.push_str(&trf(lang, "Tunnels {count}.", &[("count", &m.tunnels.len().to_string())]));
        }
        if !m.missing.is_empty() {
            text.push(' ');
            text.push_str(&trf(lang, "Not found: {names}.", &[("names", &m.missing.join(", "))]));
        }
        if m.skipped > 0 {
            text.push(' ');
            text.push_str(&trf(lang, "Rows skipped - {count}.", &[("count", &m.skipped.to_string())]));
        }
        text
    }

    /// Вырез включён и есть кольца выреза.
    pub fn cut_on(&self) -> bool {
        self.options.cut && self.model.as_ref().is_some_and(|m| !m.rings.is_empty())
    }

    /// Пол камеры под землёй или None.
    pub fn floor(&self) -> Option<Floor> {
        if !self.options.under {
            return None;
        }
        let m = self.model.as_ref()?;
        let [s, n, w, e] = m.bounds(true)?;
        let (dl, dw) = ((n - s) * FLOOR_MARGIN, (e - w) * FLOOR_MARGIN);
        Some(Floor { bounds: [s - dl, n + dl, w - dw, e + dw], bottom: m.bottom() })
    }

    /// Точка середины модели для перелёта.
    pub fn middle(&self) -> Option<(f64, f64, f64)> {
        let m = self.model.as_ref()?;
        let [s, n, w, e] = m.bounds(true)?;
        Some(((s + n) / 2.0, (w + e) / 2.0, m.extent()))
    }

    /// Окно «Подземный режим», `SubsurfaceDialog` модуля. Возвращает,
    /// сменились ли настройки, и нажата ли «Убрать модель».
    pub fn window(&mut self, ctx: &egui::Context, lang: Lang) -> (bool, bool) {
        let before = self.options;
        let mut open = self.window_open;
        let mut remove = false;
        egui::Window::new(tr(lang, "Subsurface mode")).open(&mut open).collapsible(false).resizable(false).default_width(360.0).show(ctx, |ui| {
            ui.label(egui::RichText::new(&self.name).strong());
            egui::Grid::new("subsurface-options").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Surface opacity"));
                ui.add(egui::Slider::new(&mut self.options.opacity, 0.0..=1.0).show_value(true))
                    .on_hover_text(tr(lang, "Opacity of the surface. Lower - drill holes and bed roofs show through the terrain."));
                ui.end_row();
            });
            ui.checkbox(&mut self.options.cut, tr(lang, "Block cut")).on_hover_text(tr(lang, "Remove the surface and the roofs inside the cut polygon. The beds show along the cut edge, the model bottom at its floor."));
            ui.checkbox(&mut self.options.under, tr(lang, "Camera under the ground")).on_hover_text(tr(lang, "The camera goes below the terrain down to the model bottom inside its frame. Above the model the camera moves along its bottom, not along the terrain."));
            if !self.status.is_empty() {
                ui.label(egui::RichText::new(&self.status).small());
            }
            ui.horizontal(|ui| {
                if ui.button(tr(lang, "Remove the model")).clicked() {
                    remove = true;
                }
            });
        });
        self.window_open = open;
        let changed = self.options != before;
        if changed && self.options.cut != before.cut {
            self.dirty = true;
            self.mask_dirty = true;
        }
        (changed, remove)
    }
}

/// Файл name рядом с файлом GeoPackage в программе.
#[cfg(not(target_arch = "wasm32"))]
fn read_near(folder: Option<&std::path::Path>, name: &str) -> Option<Vec<u8>> {
    // Имя - только имя файла, без пути из таблицы.
    let file = std::path::Path::new(name).file_name()?;
    std::fs::read(folder?.join(file)).ok()
}

#[cfg(target_arch = "wasm32")]
fn read_near(_folder: Option<&std::path::Path>, _name: &str) -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> Option<Vec<LayerData>> {
        let bytes = std::fs::read(r"C:\Dev\planetx\planetx\demo\subsurface\perm_subsurface.gpkg").ok()?;
        data::layers::read(bytes, "perm_subsurface.gpkg").ok()
    }

    fn near(name: &str) -> Option<Vec<u8>> {
        std::fs::read(std::path::Path::new(r"C:\Dev\planetx\planetx\demo\subsurface").join(name)).ok()
    }

    #[test]
    fn roles_by_name_and_fields() {
        let layer = |name: &str, fields: &[&str], shape: Shape| LayerData {
            name: name.into(),
            epsg: None,
            fields: fields.iter().map(|f| f.to_string()).collect(),
            items: vec![data::layers::Item { shape, parts: Vec::new(), values: Vec::new() }],
        };
        assert_eq!(detect_role(&layer("Collar", &[], Shape::Point)), Some("collar"));
        assert_eq!(detect_role(&layer("holes", &["hole_id", "eoh"], Shape::Point)), Some("collar"));
        assert_eq!(detect_role(&layer("runs", &["hole_id", "from", "to"], Shape::None)), Some("interval"));
        assert_eq!(detect_role(&layer("loop", &["name", "depth", "diameter"], Shape::Line)), Some("tunnels"));
        assert_eq!(detect_role(&layer("roads", &["name"], Shape::Line)), None);
        assert!(!is_subsurface(&[layer("roads", &["name"], Shape::Line)]));
    }

    #[test]
    fn demo_model_builds() {
        let Some(layers) = demo() else {
            eprintln!("демо модуля нет, проверка пропущена");
            return;
        };
        assert!(is_subsurface(&layers));
        let model = Model::load(&layers, &near);
        assert_eq!(model.holes.len(), 24);
        assert_eq!(model.horizons.len(), 9, "{:?}", model.missing);
        assert!(model.missing.is_empty());
        assert_eq!(model.sections.len(), 2);
        assert_eq!(model.mask_rings.len(), 1);
        assert!(model.bottom() < -200.0, "{}", model.bottom());
        let flat = |pts: &[(f64, f64)]| vec![150.0; pts.len()];
        let (meshes, vertices) = build(&model, 2.0, &flat, true);
        assert!(vertices > 1000);
        for (name, mesh) in &meshes {
            assert!(mesh.is_some(), "{name}");
        }
        let made = mask(&model, true);
        assert!(made.is_some());
        let Some((b, w, h, rgba)) = made else {
            return;
        };
        assert!(b[2] > b[0] && b[3] > b[1]);
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        let cut = rgba.as_chunks::<4>().0.iter().filter(|p| p[3] == 255).count();
        let glass = rgba.as_chunks::<4>().0.iter().filter(|p| p[0] == 255).count();
        assert!(cut > 0 && glass > cut, "{cut} {glass}");
        // Край пуст.
        assert!(rgba[..(w as usize) * 4].iter().all(|v| *v == 0));
    }

    #[test]
    fn rasters_come_later() {
        let Some(layers) = demo() else {
            return;
        };
        let mut s = Subsurface::new(layers, "perm_subsurface.gpkg", None, 0.0);
        assert!(s.wants("ROOF_01.tif"));
        assert!(!s.wants("other.tif"));
        let Some(bytes) = near("roof_01.tif") else {
            return;
        };
        s.add_side("roof_01.tif", bytes);
        assert!(!s.wants("roof_01.tif"));
        assert_eq!(s.model.as_ref().map(|m| m.horizons.len()), Some(1));
    }

    #[test]
    fn floor_inside_frame_only() {
        let f = Floor { bounds: [59.0, 60.0, 56.0, 57.0], bottom: -300.0 };
        assert_eq!(f.at(59.5, 56.5, 2.0, || 100.0), Some(-600.0));
        assert_eq!(f.at(59.5, 56.5, 0.0, || 100.0), Some(-400.0));
        assert_eq!(f.at(61.0, 56.5, 2.0, || 100.0), None);
    }

    #[test]
    fn hostile_color_and_numbers() {
        assert_eq!(hex_color("#zz0000", [1, 2, 3]), [1, 2, 3]);
        assert_eq!(hex_color("#ff8000", [1, 2, 3]), [255, 128, 0]);
        assert_eq!(hex_color("#ÿÿÿ", [1, 2, 3]), [1, 2, 3]);
    }
}
