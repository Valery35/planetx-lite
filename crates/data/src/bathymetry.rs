// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Своя батиметрия: растр глубин водоёма в рельеф глобуса и вода на
//! уровне водоёма. Перенос `core/bathymetry.py` модуля.
//!
//! Растр глубин с уровнем воды или растр отметок дна, полупрозрачная
//! вода на уровне водоёма. Растр врезается в высоты глобуса как свой
//! рельеф, глубины перед этим пересчитываются в отметки дна (to_bed).
//! Вода - отдельная сетка на уровне водоёма над узлами, где дно ниже
//! уровня (water_part). Глубина в растре бывает со знаком плюс или минус
//! вниз, берётся её модуль.

/// Что в растре.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Глубины под поверхностью воды.
    Depth,
    /// Отметки дна над уровнем моря.
    Bed,
}

/// Цвет воды - как у моря в шейдере тайла, альфа 0.5. Выбор помощника
/// модуля, утверждает автор.
pub const WATER_COLOR: [u8; 4] = [23, 77, 140, 128];
/// Узел у берега с дном выше уровня на столько ещё под водой.
pub const SHORE: f64 = 0.05;
/// Узлов сетки воды по стороне, не больше.
pub const WATER_SIDE: usize = 256;

/// Отметка дна из значения растра: у глубин - уровень минус модуль
/// глубины, у отметок - само значение. NaN остаётся NaN.
pub fn to_bed(value: f64, kind: Kind, level: f64) -> f64 {
    match kind {
        Kind::Depth => level - value.abs(),
        Kind::Bed => value,
    }
}

/// Широта и долгота точки EPSG:3857, градусы.
pub fn mercator_latlon(x: f64, y: f64) -> (f64, f64) {
    let radius = 6378137.0;
    let lon = (x / radius).to_degrees();
    let lat = (2.0 * (y / radius).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees();
    (lat, lon)
}

/// Треугольники сетки узлов rows × cols по признаку valid: тройки
/// номеров узлов. Ячейка с четырьмя узлами - два треугольника, с тремя -
/// один, так край идёт косой линией, а не ступенями. Порядок - как
/// у `grid_triangles` модуля.
pub fn grid_triangles(valid: &[bool], rows: usize, cols: usize) -> Vec<[u32; 3]> {
    let mut groups: [Vec<[u32; 3]>; 6] = Default::default();
    for r in 0..rows.saturating_sub(1) {
        for c in 0..cols.saturating_sub(1) {
            let (a, b) = (r * cols + c, r * cols + c + 1);
            let (cc, d) = ((r + 1) * cols + c, (r + 1) * cols + c + 1);
            let (va, vb, vc, vd) = (valid[a], valid[b], valid[cc], valid[d]);
            let n = [va, vb, vc, vd].iter().filter(|v| **v).count();
            let t = |x: usize, y: usize, z: usize| [x as u32, y as u32, z as u32];
            if n == 4 {
                groups[0].push(t(a, b, cc));
                groups[1].push(t(b, d, cc));
            } else if n == 3 {
                if !vd {
                    groups[2].push(t(a, b, cc));
                } else if !va {
                    groups[3].push(t(b, d, cc));
                } else if !vb {
                    groups[4].push(t(a, d, cc));
                } else {
                    groups[5].push(t(a, b, d));
                }
            }
        }
    }
    groups.concat()
}

/// Поверхность воды над узлами сетки rows × cols, где дно ниже уровня
/// level: номера узлов треугольников. None, если воды нет.
pub fn water_part(bed: &[f64], rows: usize, cols: usize, level: f64) -> Option<Vec<[u32; 3]>> {
    let wet: Vec<bool> = bed.iter().map(|b| b.is_finite() && *b < level + SHORE).collect();
    if !wet.iter().any(|w| *w) {
        return None;
    }
    let tris = grid_triangles(&wet, rows, cols);
    (!tris.is_empty()).then_some(tris)
}

/// Рамка растра в EPSG:3857 и уровень водоёма.
pub type Lake = ((f64, f64, f64, f64), f64);

/// Уровень воды в точках профиля высот или NaN, где воды нет: в рамке
/// растра, где дно ниже уровня. Из двух водоёмов над точкой берётся
/// верхний.
pub fn profile_water(points: &[(f64, f64)], heights: &[f64], lakes: &[Lake]) -> Vec<f64> {
    let radius = 6378137.0;
    points
        .iter()
        .zip(heights)
        .map(|((lat, lon), h)| {
            let x = radius * lon.to_radians();
            let y = radius * (std::f64::consts::FRAC_PI_4 + lat.to_radians() / 2.0).tan().ln();
            let mut out = f64::NAN;
            for ((west, south, east, north), level) in lakes {
                if x >= *west && x <= *east && y >= *south && y <= *north && *h < level + SHORE {
                    out = out.max(*level);
                    if out.is_nan() {
                        out = *level;
                    }
                }
            }
            out
        })
        .collect()
}
