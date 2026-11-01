// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Границы сельскохозяйственных полей Fields of the World, перенос
//! `core/fieldmap.py` модуля.
//!
//! Набор ftw/global-data на Source Cooperative - поля модели PRUE по
//! снимкам Sentinel-2 за 2024 и 2025 годы, CC BY 4.0. Это оценка модели,
//! а не кадастр. Один архив PMTiles на год, файл 2025 года - 268 ГБ.
//!
//! Тайлы архивов собраны без предела размера, замер модуля 1 ноября 2026
//! года - тайл уровня 5 в Пенджабе 396 МБ, уровня 8 - 24 МБ, уровня 10 -
//! 1.8 МБ, поэтому поля просятся с [`MIN_LEVEL`]. В архиве 2024 года уровни
//! 10-14, в архиве 2025 года 0-14. Объекты 2024 года с меткой
//! field_boundaries - полосы границ, они не рисуются.
//!
//! Оценка 0-100 - confidence_mean / [`FULL`], как в стилях набора.
//! Рекомендация набора - показывать оценки не ниже [`MIN_SCORE`].
//!
//! Модуль рисует тайл в картинку 512 пикселей. У Lite картинка карты -
//! 256 пикселей, поэтому тайлы [`PAINT_LEVEL`] рисуются из тайлов архива
//! [`MAX_LEVEL`], подробность та же. Файл считается враждебным: разбор
//! идёт через [`crate::mvt`], кольцо из меньше трёх точек отбрасывается.

use std::collections::HashSet;

use crate::mvt::{self, DecodeError, Value};

/// Годы набора.
pub const YEARS: [i32; 2] = [2025, 2024];
/// Подпись источника.
pub const ATTRIBUTION: &str = "Fields of the World, CC BY 4.0";
/// Страница условий набора.
pub const TERMS: &str = "https://source.coop/ftw/global-data";
/// Первый уровень тайлов архива.
pub const MIN_LEVEL: u32 = 10;
/// Глубший уровень тайлов архива.
pub const MAX_LEVEL: u32 = 14;
/// Глубший уровень картинок карты.
pub const PAINT_LEVEL: u32 = 15;
/// confidence_mean оценки 100.
pub const FULL: f64 = 0.578178;
/// Наименьшая показанная оценка.
pub const MIN_SCORE: f64 = 69.0;
/// Метка поля. Вторая метка набора - полосы границ.
const LABEL: &str = "field";
/// Шкала стилей набора от рекомендованного порога.
pub const STOPS: [(f64, [u8; 3]); 4] = [(MIN_SCORE, [254, 195, 121]), (80.0, [243, 250, 187]), (90.0, [207, 236, 176]), (100.0, [51, 160, 44])];
/// Деления шкалы.
pub const TICKS: [f64; 4] = [70.0, 80.0, 90.0, 100.0];
/// Сторона картинки тайла, пикселей.
const SIDE: u32 = crate::lines::TILE_SIZE;
/// Непрозрачность заливки, выбор помощника модуля, 110 из 255.
const FILL_ALPHA: u8 = 110;
/// Толщина линии, пикселей картинки 256: у модуля 1.6 на картинке 512
/// тайла того же уровня.
const LINE_WIDTH: f32 = 1.0;

/// Адрес архива года.
pub fn url(year: i32) -> &'static str {
    match year {
        2024 => "https://data.source.coop/ftw/global-data/predictions/vectors/alpha/2024_with_confidence.pmtiles",
        _ => "https://data.source.coop/ftw/global-field-boundaries/pmtiles/ftw-global-fields-2025.pmtiles",
    }
}

/// Слой векторных тайлов года.
pub fn layer(year: i32) -> &'static str {
    match year {
        2024 => "2024",
        _ => "fields",
    }
}

/// Ключ строки каталога года.
pub fn key(year: i32) -> &'static str {
    match year {
        2024 => "ftw_2024",
        _ => "ftw_2025",
    }
}

/// Год строки каталога.
pub fn year_of(key: &str) -> Option<i32> {
    YEARS.into_iter().find(|y| self::key(*y) == key)
}

/// Поле тайла: кольца долями тайла 0-1, y растёт к югу, оценка 0-100.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub rings: Vec<Vec<[f64; 2]>>,
    pub score: f64,
}

/// Оценка 0-100 по значению confidence_mean. В файлах значения строками.
pub fn score_of(value: &Value) -> Option<f64> {
    let number = match value {
        Value::Str(s) => s.trim().parse().ok(),
        other => other.as_f64(),
    }?;
    (number.is_finite()).then(|| (number / FULL * 100.0).min(100.0))
}

/// Цвет оценки RGB по шкале [`STOPS`].
pub fn color(score: f64) -> [u8; 3] {
    for pair in STOPS.windows(2) {
        let ((low, first), (high, last)) = (pair[0], pair[1]);
        if score <= high {
            let share = (score - low).max(0.0) / (high - low);
            return [0, 1, 2].map(|k| (f64::from(first[k]) + (f64::from(last[k]) - f64::from(first[k])) * share + 0.5) as u8);
        }
    }
    STOPS[STOPS.len() - 1].1
}

/// Поля слоя title тайла не ниже [`MIN_SCORE`].
pub fn decode(data: &[u8], title: &str) -> Result<Vec<Field>, DecodeError> {
    let wanted = HashSet::from(["label".to_string(), "confidence_mean".to_string()]);
    let mut layers = mvt::decode_shapes(data, &HashSet::from([title]), Some(&wanted))?;
    let Some(layer) = layers.remove(title) else {
        return Ok(Vec::new());
    };
    let extent = f64::from(layer.extent.max(1));
    let mut out = Vec::new();
    for feature in layer.features {
        if feature.geometry != mvt::Geometry::Polygon || feature.attrs.get("label").and_then(Value::as_str) != Some(LABEL) {
            continue;
        }
        let Some(score) = feature.attrs.get("confidence_mean").and_then(score_of).filter(|s| *s >= MIN_SCORE) else {
            continue;
        };
        let rings: Vec<Vec<[f64; 2]>> = feature.parts.iter().filter(|p| p.len() >= 3).map(|p| p.iter().map(|&(x, y)| [x as f64 / extent, y as f64 / extent]).collect()).collect();
        if !rings.is_empty() {
            out.push(Field { rings, score });
        }
    }
    Ok(out)
}

/// Тайл архива картинки key: предок на уровне [`MAX_LEVEL`] или сам тайл.
pub fn source_of(key: (u32, u32, u32)) -> (u32, u32, u32) {
    let depth = key.0.saturating_sub(MAX_LEVEL).min(31);
    (key.0 - depth, key.1 >> depth, key.2 >> depth)
}

/// Картинка RGBA 256 × 256 с премноженной альфой тайла key по полям тайла
/// архива source. None - полей в тайле нет.
pub fn paint(fields: &[Field], source: (u32, u32, u32), key: (u32, u32, u32)) -> Option<Vec<u8>> {
    let depth = key.0.saturating_sub(source.0).min(31);
    let cells = f64::from(1u32 << depth);
    let (cx, cy) = (f64::from(key.1 & ((1 << depth) - 1)), f64::from(key.2 & ((1 << depth) - 1)));
    let mut pixmap = tiny_skia::Pixmap::new(SIDE, SIDE)?;
    let mut any = false;
    let stroke = tiny_skia::Stroke { width: LINE_WIDTH, ..tiny_skia::Stroke::default() };
    for field in fields {
        let mut path = tiny_skia::PathBuilder::new();
        for ring in &field.rings {
            for (n, p) in ring.iter().enumerate() {
                let (x, y) = (((p[0] * cells - cx) * f64::from(SIDE)) as f32, ((p[1] * cells - cy) * f64::from(SIDE)) as f32);
                if n == 0 {
                    path.move_to(x, y);
                } else {
                    path.line_to(x, y);
                }
            }
            path.close();
        }
        let Some(path) = path.finish() else {
            continue;
        };
        let b = path.bounds();
        let side = SIDE as f32;
        if b.right() < 0.0 || b.bottom() < 0.0 || b.left() > side || b.top() > side {
            continue;
        }
        let [r, g, bl] = color(field.score);
        let mut paint = tiny_skia::Paint { anti_alias: true, ..tiny_skia::Paint::default() };
        paint.set_color_rgba8(r, g, bl, FILL_ALPHA);
        pixmap.fill_path(&path, &paint, tiny_skia::FillRule::EvenOdd, tiny_skia::Transform::identity(), None);
        paint.set_color_rgba8(r, g, bl, 255);
        pixmap.stroke_path(&path, &paint, &stroke, tiny_skia::Transform::identity(), None);
        any = true;
    }
    any.then(|| pixmap.take())
}

/// Тайл уровня level под точкой и её место в нём долями тайла. Пиксель
/// тайла здесь груб: на уровне 10 он больше поля.
pub fn tile_point(lat: f64, lon: f64, level: u32) -> ((u32, u32), (f64, f64)) {
    let cells = f64::from(1u32 << level.min(30));
    let phi = lat.clamp(-85.0511, 85.0511).to_radians();
    let x = (lon + 180.0).rem_euclid(360.0) / 360.0 * cells;
    let y = (1.0 - phi.tan().asinh() / std::f64::consts::PI) / 2.0 * cells;
    ((x.floor() as u32, y.floor() as u32), (x.fract(), y.fract()))
}

/// Оценка поля в точке (x, y) долями тайла или None, правило чёт-нечет
/// по всем кольцам, как `watermask.inside` модуля.
pub fn score_at(fields: &[Field], x: f64, y: f64) -> Option<f64> {
    fields.iter().find(|f| f.rings.iter().map(|ring| crossings(ring, x, y)).sum::<usize>() % 2 == 1).map(|f| f.score)
}

/// Пересечений кольца лучом от точки на восток.
fn crossings(ring: &[[f64; 2]], x: f64, y: f64) -> usize {
    let mut count = 0;
    for (n, a) in ring.iter().enumerate() {
        let b = ring[(n + 1) % ring.len()];
        if (a[1] > y) != (b[1] > y) && x < a[0] + (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) {
            count += 1;
        }
    }
    count
}
