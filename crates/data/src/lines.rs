// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Линии векторной основы OpenFreeMap в картинку тайла глобуса, перенос
//! стиля `net/overlay.py` модуля (`LINE_STYLES`, `RAIL_LINES`).
//!
//! Модуль рисует слой векторных тайлов QGIS в картинку 256 × 256 на
//! каждый тайл глобуса и кладёт её поверх снимка. Здесь то же: векторный
//! тайл того же уровня, но не глубже [`MAX_LEVEL`], раскладывается на
//! фигуры стилей, фигуры рисуются в картинку тайла. Толщины - в пикселях
//! картинки, цвета и наименьшие уровни - как у модуля, сверены им
//! с Google Earth Pro 26 сентября 2026 года.

use std::collections::{HashMap, HashSet};

use crate::mvt::{self, DecodeError, Geometry, Value};

/// Сторона картинки тайла, пикселей, `TILE_SIZE` модуля.
pub const TILE_SIZE: u32 = 256;
/// Самый подробный уровень векторных тайлов OpenFreeMap.
pub const MAX_LEVEL: u32 = 14;
/// Железные дороги есть в векторных тайлах только с уровня 8.
pub const RAIL_ZMIN: u32 = 8;
/// С этого уровня тайла глобуса рисуются железные дороги: тайл уровня 7
/// берёт четыре векторных тайла уровня 8. Решение автора модуля
/// от 27 сентября 2026 года.
pub const RAIL_FROM: u32 = 7;

/// Группа векторной основы, её включают по отдельности, как в модуле.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Group {
    Borders,
    Rivers,
    Water,
    Roads,
    Railways,
    Parks,
    Airports,
}

/// Линия стиля: цвет RGBA, толщина в пикселях, пунктир (штрих, пробел).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pen {
    pub color: [u8; 4],
    pub width: f32,
    pub dash: Option<[f32; 2]>,
}

const fn pen(color: [u8; 4], width: f32) -> Pen {
    Pen { color, width, dash: None }
}

type Attrs = HashMap<String, Value>;

/// Стиль: группа, слой схемы OpenMapTiles, условие, наименьший уровень
/// тайла глобуса, многоугольник ли, заливка и линии.
pub struct Style {
    pub group: Group,
    pub layer: &'static str,
    pub test: fn(&Attrs) -> bool,
    pub zmin: u32,
    pub polygon: bool,
    pub fill: Option<[u8; 4]>,
    pub pens: &'static [Pen],
}

fn class(attrs: &Attrs) -> &str {
    attrs.get("class").and_then(Value::as_str).unwrap_or("")
}

fn number(attrs: &Attrs, key: &str) -> f64 {
    match attrs.get(key) {
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(v) => v.as_f64().unwrap_or(0.0),
        None => 0.0,
    }
}

/// Вода, как слой «Водоёмы» Google Earth, `BLUE` модуля.
const BLUE: [u8; 3] = [70, 130, 230];

const fn blue(alpha: u8) -> [u8; 4] {
    [BLUE[0], BLUE[1], BLUE[2], alpha]
}

/// Стили по порядку отрисовки, `LINE_STYLES` модуля. Страны ярко-жёлтые,
/// области тонкие белые, вода синяя с контуром водоёмов, магистрали
/// жёлтые, остальные дороги тонкие белые.
pub const STYLES: [Style; 10] = [
    Style {
        group: Group::Borders,
        layer: "boundary",
        test: |a| number(a, "admin_level") == 2.0 && number(a, "maritime") == 0.0,
        zmin: 0,
        polygon: false,
        fill: None,
        pens: &[pen([255, 255, 0, 255], 2.0)],
    },
    // Общий отрезок границ несёт наименьший уровень: граница области,
    // которая ещё и граница федерального округа, - уровень 3.
    Style {
        group: Group::Borders,
        layer: "boundary",
        test: |a| matches!(number(a, "admin_level") as i64, 3 | 4) && number(a, "maritime") == 0.0,
        zmin: 3,
        polygon: false,
        fill: None,
        pens: &[pen([255, 255, 255, 150], 1.0)],
    },
    Style {
        group: Group::Rivers,
        layer: "waterway",
        test: |a| class(a) == "river",
        zmin: 8,
        polygon: false,
        fill: None,
        pens: &[pen(blue(200), 1.0)],
    },
    // Большая река в OpenStreetMap - полигон воды, Google Earth обводит
    // её берега.
    Style {
        group: Group::Water,
        layer: "water",
        test: |a| matches!(class(a), "lake" | "river"),
        zmin: 6,
        polygon: true,
        fill: None,
        pens: &[pen(blue(210), 1.0)],
    },
    Style {
        group: Group::Roads,
        layer: "transportation",
        test: |a| matches!(class(a), "motorway" | "trunk"),
        zmin: 6,
        polygon: false,
        fill: None,
        pens: &[pen([255, 204, 40, 235], 1.5)],
    },
    Style {
        group: Group::Roads,
        layer: "transportation",
        test: |a| class(a) == "primary",
        zmin: 8,
        polygon: false,
        fill: None,
        pens: &[pen([255, 230, 120, 210], 1.1)],
    },
    Style {
        group: Group::Roads,
        layer: "transportation",
        test: |a| matches!(class(a), "secondary" | "tertiary"),
        zmin: 10,
        polygon: false,
        fill: None,
        pens: &[pen([255, 255, 255, 140], 0.8)],
    },
    Style {
        group: Group::Roads,
        layer: "transportation",
        test: |a| matches!(class(a), "minor" | "service"),
        zmin: 13,
        polygon: false,
        fill: None,
        pens: &[pen([255, 255, 255, 110], 0.6)],
    },
    // Заповедники, национальные парки и заказники: зелёный контур
    // и едва заметная заливка, как слой «Парки» Google Earth.
    Style {
        group: Group::Parks,
        layer: "park",
        test: |a| matches!(class(a), "national_park" | "nature_reserve" | "protected_area" | "wildlife_refuge"),
        zmin: 6,
        polygon: true,
        fill: Some([90, 190, 90, 28]),
        pens: &[pen([110, 200, 110, 200], 1.0)],
    },
    Style {
        group: Group::Airports,
        layer: "aeroway",
        test: |a| class(a) == "runway",
        zmin: 10,
        polygon: false,
        fill: None,
        pens: &[pen([215, 215, 215, 230], 2.0)],
    },
];

/// Железная дорога: светлая линия и тёмный пунктир поверх, `RAIL_LINES`
/// модуля. Пути станций и подъездные пути с полем service не рисуются.
pub const RAIL: [Pen; 2] = [pen([235, 235, 235, 245], 3.6), Pen { color: [40, 40, 40, 240], width: 1.8, dash: Some([7.0, 6.0]) }];

fn is_rail(attrs: &Attrs) -> bool {
    class(attrs) == "rail" && !attrs.contains_key("service")
}

/// Толщина железной дороги падает при отдалении, `RAIL_WIDTH` модуля:
/// иначе на мелких уровнях сеть сливалась в сплошной узор.
pub fn rail_scale(z: u32) -> f32 {
    ((z as f32 - 5.0) / 4.0).clamp(0.4, 1.0)
}

/// Индекс стиля железной дороги в [`Shape::style`].
pub const RAIL_STYLE: usize = STYLES.len();

/// Фигура векторного тайла: стиль и части в долях тайла 0..1.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub style: usize,
    pub parts: Vec<Vec<[f32; 2]>>,
}

/// Фигуры векторного тайла по стилям.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tile {
    pub shapes: Vec<Shape>,
}

const WANTED: [&str; 4] = ["class", "admin_level", "maritime", "service"];

/// Фигуры стилей групп groups из векторного тайла: разбираются только их
/// слои, как модуль рисует только стили включённых групп. Слой дорог
/// тайла уровня 14 велик, а по умолчанию включены одни границы. Тайл
/// считается враждебным.
pub fn decode(data: &[u8], groups: &HashSet<Group>) -> Result<Tile, DecodeError> {
    let mut names: HashSet<&str> = STYLES.iter().filter(|s| groups.contains(&s.group)).map(|s| s.layer).collect();
    if groups.contains(&Group::Railways) {
        names.insert("transportation");
    }
    if names.is_empty() {
        return Ok(Tile::default());
    }
    let wanted: HashSet<String> = WANTED.iter().map(|s| s.to_string()).collect();
    let layers = mvt::decode_shapes(data, &names, Some(&wanted))?;
    let mut shapes = Vec::new();
    for (name, layer) in &layers {
        let extent = layer.extent.max(1) as f32;
        for f in &layer.features {
            if f.geometry == Geometry::Point || f.parts.is_empty() {
                continue;
            }
            let polygon = f.geometry == Geometry::Polygon;
            let style = STYLES.iter().position(|s| s.layer == name && s.polygon == polygon && groups.contains(&s.group) && (s.test)(&f.attrs));
            let style = match style {
                Some(s) => s,
                None if !polygon && name == "transportation" && groups.contains(&Group::Railways) && is_rail(&f.attrs) => RAIL_STYLE,
                None => continue,
            };
            let parts = f.parts.iter().map(|part| part.iter().map(|(x, y)| [*x as f32 / extent, *y as f32 / extent]).collect()).collect();
            shapes.push(Shape { style, parts });
        }
    }
    // Порядок отрисовки - порядок стилей, как у слоя модуля.
    shapes.sort_by_key(|s| s.style);
    Ok(Tile { shapes })
}

/// Тайл глобуса, тайлы - ключи (z, x, y).
pub type Key = (u32, u32, u32);

/// Векторный тайл для тайла глобуса: он сам или предок уровня
/// [`MAX_LEVEL`].
pub fn vector_key(key: Key) -> Key {
    let (z, x, y) = key;
    let level = z.min(MAX_LEVEL);
    let shift = z - level;
    (level, x >> shift, y >> shift)
}

/// Векторные тайлы, нужные картинке тайла глобуса при группах groups:
/// свой векторный тайл и у уровня [`RAIL_FROM`] с железными дорогами -
/// четыре тайла уровня [`RAIL_ZMIN`].
pub fn sources(key: Key, groups: &HashSet<Group>) -> Vec<Key> {
    let mut out = vec![vector_key(key)];
    let (z, x, y) = key;
    if groups.contains(&Group::Railways) && (RAIL_FROM..RAIL_ZMIN).contains(&z) {
        let shift = RAIL_ZMIN - z;
        for dy in 0..1u32 << shift {
            for dx in 0..1u32 << shift {
                out.push((RAIL_ZMIN, (x << shift) + dx, (y << shift) + dy));
            }
        }
    }
    out
}

/// Рамка точек part задевает окно [u0, v0, u1, v1].
fn touches(part: &[[f32; 2]], w: [f32; 4]) -> bool {
    let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
    for p in part {
        for i in 0..2 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    lo[0] <= w[2] && hi[0] >= w[0] && lo[1] <= w[3] && hi[1] >= w[1]
}

/// Рамка отрезка a-b задевает окно.
fn segment_touches(a: [f32; 2], b: [f32; 2], w: [f32; 4]) -> bool {
    a[0].min(b[0]) <= w[2] && a[0].max(b[0]) >= w[0] && a[1].min(b[1]) <= w[3] && a[1].max(b[1]) >= w[1]
}

/// Картинка линий тайла глобуса key: RGBA с альфой, умноженной на цвет,
/// стороной [`TILE_SIZE`]. tiles - разобранные векторные тайлы по ключу.
/// None - на тайле нечего рисовать.
pub fn draw(key: Key, tiles: &dyn Fn(Key) -> Option<std::sync::Arc<Tile>>, groups: &HashSet<Group>) -> Option<Vec<u8>> {
    let (z, x, y) = key;
    let size = TILE_SIZE as f32;
    let mut pixmap = tiny_skia::Pixmap::new(TILE_SIZE, TILE_SIZE)?;
    let mut drawn = false;
    let rails = groups.contains(&Group::Railways) && z >= RAIL_FROM;
    for source in sources(key, groups) {
        let Some(tile) = tiles(source) else {
            continue;
        };
        let (sz, sx, sy) = source;
        // Доля тайла source в пиксели картинки key.
        let scale = 2f64.powi(z as i32 - sz as i32);
        let to_px = |p: [f32; 2]| -> (f32, f32) {
            let u = ((f64::from(sx) + f64::from(p[0])) * scale - f64::from(x)) * f64::from(size);
            let v = ((f64::from(sy) + f64::from(p[1])) * scale - f64::from(y)) * f64::from(size);
            (u as f32, v as f32)
        };
        let rail_only = sz != vector_key(key).0;
        // Окно картинки в долях тайла source с запасом на толщину линий.
        // Части за окном не рисуются: у тайла глубже 14-го в окно
        // попадает малая доля векторного тайла, и обход всех его линий
        // замедлял кадр до 17 в секунду.
        let side = 1.0 / scale;
        let u0 = f64::from(x) * side - f64::from(sx);
        let v0 = f64::from(y) * side - f64::from(sy);
        let margin = side * 8.0 / f64::from(size);
        let window = [(u0 - margin) as f32, (v0 - margin) as f32, (u0 + side + margin) as f32, (v0 + side + margin) as f32];
        for shape in &tile.shapes {
            let (pens, fill, polygon): (&[Pen], Option<[u8; 4]>, bool) = if shape.style == RAIL_STYLE {
                if !rails {
                    continue;
                }
                (&RAIL, None, false)
            } else {
                let Some(style) = STYLES.get(shape.style) else {
                    continue;
                };
                if rail_only || !groups.contains(&style.group) || z < style.zmin {
                    continue;
                }
                (style.pens, style.fill, style.polygon)
            };
            let mut builder = tiny_skia::PathBuilder::new();
            for part in &shape.parts {
                if polygon {
                    // Кольцо целиком, если его рамка задевает окно: заливке
                    // нужен замкнутый контур.
                    if !touches(part, window) {
                        continue;
                    }
                    let mut points = part.iter().map(|p| to_px(*p));
                    let Some((u, v)) = points.next() else {
                        continue;
                    };
                    builder.move_to(u, v);
                    for (u, v) in points {
                        builder.line_to(u, v);
                    }
                    builder.close();
                    continue;
                }
                // Линия - отрезками, которые задевают окно.
                let mut open = false;
                for pair in part.windows(2) {
                    if segment_touches(pair[0], pair[1], window) {
                        if !open {
                            let (u, v) = to_px(pair[0]);
                            builder.move_to(u, v);
                            open = true;
                        }
                        let (u, v) = to_px(pair[1]);
                        builder.line_to(u, v);
                    } else {
                        open = false;
                    }
                }
            }
            let Some(path) = builder.finish() else {
                continue;
            };
            if let Some(c) = fill {
                let mut paint = tiny_skia::Paint::default();
                paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
                paint.anti_alias = true;
                pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
            }
            for p in pens {
                let mut paint = tiny_skia::Paint::default();
                paint.set_color_rgba8(p.color[0], p.color[1], p.color[2], p.color[3]);
                paint.anti_alias = true;
                let width = if shape.style == RAIL_STYLE { p.width * rail_scale(z) } else { p.width };
                let (cap, dash) = match p.dash {
                    // Пунктир модуля - с плоскими концами.
                    Some([on, off]) => (tiny_skia::LineCap::Butt, tiny_skia::StrokeDash::new(vec![on, off], 0.0)),
                    None => (tiny_skia::LineCap::Round, None),
                };
                let stroke = tiny_skia::Stroke { width, line_cap: cap, line_join: tiny_skia::LineJoin::Round, dash, ..Default::default() };
                pixmap.stroke_path(&path, &paint, &stroke, tiny_skia::Transform::identity(), None);
            }
            drawn = true;
        }
    }
    drawn.then(|| pixmap.take())
}

