// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Вода и суша ниже уровня моря по векторной основе, перенос
//! `core/watermask.py` модуля.
//!
//! Сухие впадины ниже нуля - карьеры, Мёртвое море, Прикаспий,
//! польдеры Нидерландов - по высотам от моря не отличить. Вода здесь -
//! многоугольники слоя water тайлов OpenMapTiles, суша - точка вне них.
//! Проход воды глобуса берёт маску тайла ([`mask_image`]): белое - вода,
//! чёрное - суша ниже нуля, воды там нет.
//!
//! Отличие от модуля: тайл глобуса грубее [`LEVEL`] берёт свой тайл воды
//! того же уровня, а не 4^(12 - z) тайлов уровня 12. У модуля маски
//! начинались с уровня 10, а с 2500 км тайлы глобуса уровней 4-6:
//! Прикаспийская низменность и берега Финляндии заливались водой. Слой
//! water OpenMapTiles есть на всех уровнях.
//!
//! Тайл считается враждебным: битый тайл - [`DecodeError`], нечисла
//! и точки за краем тайла не ломают маску.

use std::collections::{HashMap, HashSet};

use crate::mvt::{self, DecodeError, Geometry};

/// Тайл: уровень, столбец, строка.
pub type TileKey = (u32, u32, u32);
/// Кольцо многоугольника: точки в долях тайла 0-1, y растёт к югу.
pub type Ring = Vec<[f64; 2]>;
/// Объект воды: кольца, внутри - по правилу чёт-нечет.
pub type Feature = Vec<Ring>;

/// Слой воды тайлов основы.
pub const LAYER: &str = "water";
/// Уровень тайлов воды под подробными тайлами глобуса, около 10 км
/// на сторону у 55° с. ш., как у модуля.
pub const LEVEL: u32 = 12;
/// Тайлы глобуса грубее не получают маски воды: тайл воды уровня 1
/// велик, а с такой высоты впадины суши не видны. Выбор помощника.
pub const MASK_FROM: u32 = 2;
/// Сторона картинки маски в пикселях. У модуля 64 при тайлах от уровня
/// 10. Тайл уровня 5 на экране занимает 300-500 пикселей, и при 64
/// берег шёл ступенями. Выбор помощника.
pub const MASK_SIDE: usize = 128;

/// Тайл уровня level под точкой и место в нём долями 0-1: (ключ, x, y),
/// y растёт к югу.
pub fn tile_of(lat: f64, lon: f64, level: u32) -> (TileKey, f64, f64) {
    let level = level.min(30);
    let n = f64::from(1u32 << level);
    let lat = if lat.is_finite() { lat.clamp(-85.0511, 85.0511) } else { 0.0 };
    let lon = if lon.is_finite() { lon } else { 0.0 };
    let fx = (lon + 180.0) / 360.0 * n;
    let phi = lat.to_radians();
    let fy = (1.0 - (phi.tan() + 1.0 / phi.cos()).ln() / std::f64::consts::PI) / 2.0 * n;
    let top = n - 1.0;
    let x = fx.floor().clamp(0.0, top);
    let y = fy.floor().clamp(0.0, top);
    ((level, x as u32, y as u32), fx - x, fy - y)
}

/// Многоугольники слоя water тайла: объекты, объект - кольца в долях
/// тайла 0-1.
pub fn decode_water(data: &[u8]) -> Result<Vec<Feature>, DecodeError> {
    let names = HashSet::from([LAYER]);
    // Атрибуты воды маске не нужны.
    let wanted: HashSet<String> = HashSet::new();
    let mut layers = mvt::decode_shapes(data, &names, Some(&wanted))?;
    let Some(layer) = layers.remove(LAYER) else {
        return Ok(Vec::new());
    };
    let extent = f64::from(layer.extent.max(1));
    Ok(layer
        .features
        .into_iter()
        .filter(|f| f.geometry == Geometry::Polygon)
        .map(|f| f.parts.into_iter().map(|part| part.into_iter().map(|(x, y)| [x as f64 / extent, y as f64 / extent]).collect()).collect::<Feature>())
        .filter(|rings: &Feature| !rings.is_empty())
        .collect())
}

/// Точка (x, y) внутри объекта из колец rings по правилу чёт-нечет:
/// двор внутри контура не считается.
pub fn inside(rings: &[Ring], x: f64, y: f64) -> bool {
    let mut odd = false;
    for ring in rings {
        let n = ring.len();
        for i in 0..n {
            let [x0, y0] = ring[i];
            let [x1, y1] = ring[(i + 1) % n];
            if (y0 > y) != (y1 > y) {
                let at = x0 + (y - y0) * (x1 - x0) / (y1 - y0);
                if x < at {
                    odd = !odd;
                }
            }
        }
    }
    odd
}

/// Часть тайла: ключ тайла воды, окно (x0, y0, сторона) части в долях
/// тайла воды и окно той же части в долях тайла глобуса.
pub type Part = (TileKey, [f64; 3], [f64; 3]);

/// Части тайла глобуса key по тайлам воды. Тайл не грубее [`LEVEL`] -
/// доля предка уровня LEVEL, грубее - свой тайл воды того же уровня.
/// Тайл грубее [`MASK_FROM`] - пустой список, маски у него нет.
pub fn mask_parts(key: TileKey) -> Vec<Part> {
    let (z, x, y) = key;
    if z < MASK_FROM {
        return Vec::new();
    }
    if z <= LEVEL {
        return vec![(key, [0.0, 0.0, 1.0], [0.0, 0.0, 1.0])];
    }
    let shift = (z - LEVEL).min(31);
    let n = 1u32 << shift;
    let (wx, wy) = (x >> shift, y >> shift);
    let nf = f64::from(n);
    vec![((LEVEL, wx, wy), [f64::from(x - wx * n) / nf, f64::from(y - wy * n) / nf, 1.0 / nf], [0.0, 0.0, 1.0])]
}

/// Кольца воды тайла глобуса key в его долях 0-1 по объектам. None -
/// тайлов воды под ним ещё нет или маски у него нет.
pub fn mask_rings(key: TileKey, tiles: &HashMap<TileKey, Vec<Feature>>) -> Option<Vec<Feature>> {
    let parts = mask_parts(key);
    if parts.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for (water, [sx, sy, ss], [dx, dy, ds]) in parts {
        let features = tiles.get(&water)?;
        let scale = ds / ss;
        for rings in features {
            out.push(rings.iter().map(|ring| ring.iter().map(|[x, y]| [(x - sx) * scale + dx, (y - sy) * scale + dy]).collect()).collect());
        }
    }
    Some(out)
}

/// Картинка маски воды side × side, RGBA: белые многоугольники объектов
/// (доли тайла 0-1, чёт-нечет внутри объекта, объекты складываются) на
/// чёрном, первая строка - северный край. Пиксель - вода, если его
/// центр в воде. Кольцо с нечислами пропускается целиком: без одного
/// пересечения чёт-нечет перевернул бы строку.
pub fn mask_image(features: &[Feature], side: usize) -> Vec<u8> {
    let mut wet = vec![false; side * side];
    let s = side as f64;
    let mut rows: Vec<Vec<f64>> = vec![Vec::new(); side];
    for feature in features {
        for row in rows.iter_mut() {
            row.clear();
        }
        for ring in feature {
            if ring.len() < 2 || ring.iter().any(|p| !p[0].is_finite() || !p[1].is_finite()) {
                continue;
            }
            let n = ring.len();
            for i in 0..n {
                let [x0, y0] = ring[i];
                let [x1, y1] = ring[(i + 1) % n];
                if y0 == y1 {
                    continue;
                }
                // Строки, чьи центры (r + 0.5) / s лежат в [min, max).
                let (lo, hi) = (y0.min(y1) * s - 0.5, y0.max(y1) * s - 0.5);
                let first = lo.ceil().max(0.0);
                let last = (hi.ceil() - 1.0).min(s - 1.0);
                if first > last {
                    continue;
                }
                let (first, last) = (first as usize, last as usize);
                for (r, row) in rows.iter_mut().enumerate().take(last + 1).skip(first) {
                    let yc = (r as f64 + 0.5) / s;
                    row.push(x0 + (yc - y0) * (x1 - x0) / (y1 - y0));
                }
            }
        }
        for (r, row) in rows.iter_mut().enumerate() {
            row.sort_by(f64::total_cmp);
            for pair in row.as_chunks::<2>().0 {
                // Столбцы, чьи центры лежат в [a, b).
                let first = (pair[0] * s - 0.5).ceil().max(0.0);
                let last = ((pair[1] * s - 0.5).ceil() - 1.0).min(s - 1.0);
                if first > last {
                    continue;
                }
                for c in first as usize..=last as usize {
                    wet[r * side + c] = true;
                }
            }
        }
    }
    wet.iter().flat_map(|w| if *w { [255, 255, 255, 255] } else { [0, 0, 0, 255] }).collect()
}

/// Тайлы воды основы и проверка точек на сушу.
#[derive(Debug, Default)]
pub struct WaterMask {
    tiles: HashMap<TileKey, Vec<Feature>>,
    /// Растёт с каждым новым тайлом.
    generation: u64,
}

impl WaterMask {
    pub fn add(&mut self, key: TileKey, features: Vec<Feature>) {
        self.tiles.insert(key, features);
        self.generation += 1;
    }

    pub fn clear(&mut self) {
        if !self.tiles.is_empty() {
            self.tiles.clear();
            self.generation += 1;
        }
    }

    /// Убрать тайлы, которых нет в keep.
    pub fn retain(&mut self, keep: &HashSet<TileKey>) {
        let before = self.tiles.len();
        self.tiles.retain(|k, _| keep.contains(k));
        if self.tiles.len() != before {
            self.generation += 1;
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn tiles(&self) -> &HashMap<TileKey, Vec<Feature>> {
        &self.tiles
    }

    pub fn contains(&self, key: TileKey) -> bool {
        self.tiles.contains_key(&key)
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// Ключи тайлов уровня [`LEVEL`] под точками (широта, долгота),
    /// которых ещё нет.
    pub fn missing(&self, points: &[(f64, f64)]) -> HashSet<TileKey> {
        points.iter().map(|(lat, lon)| tile_of(*lat, *lon, LEVEL).0).filter(|k| !self.tiles.contains_key(k)).collect()
    }

    /// Точки на суше: тайл уровня [`LEVEL`] есть и точка не в воде.
    pub fn land(&self, points: &[(f64, f64)]) -> Vec<bool> {
        points
            .iter()
            .map(|(lat, lon)| {
                let (key, fx, fy) = tile_of(*lat, *lon, LEVEL);
                self.tiles.get(&key).is_some_and(|features| !features.iter().any(|rings| inside(rings, fx, fy)))
            })
            .collect()
    }
}
