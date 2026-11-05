// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Тайлы воды векторной основы для маски прохода воды, перенос
//! `ui/watermask.py` и `GlobeView._water_mask` модуля.
//!
//! Тайлам кадра с рельефом ниже нуля нужны тайлы основы OpenFreeMap
//! (`data::watermask::mask_parts`). Они идут тем же загрузчиком, что
//! подписи и линии основы, и тем же кэшем. Пришедший тайл разбирается
//! один раз, слой water. Картинки масок рисуются в бюджете кадра,
//! не больше [`MASKS_PER_FRAME`] за кадр, и ложатся в видеокарту
//! (`render::Globe::set_water_mask`). Пока своей маски нет, проход воды
//! берёт долю маски предка, без масок - воду по высотам, как прежде.

use std::collections::HashSet;
use std::sync::Arc;

use data::watermask::{self as wm, WaterMask};
use globe_core::lod::TileKey;

/// Новых масок за кадр не больше стольких, `MASKS_PER_FRAME` модуля.
pub const MASKS_PER_FRAME: usize = 4;
/// Разобранных тайлов воды в памяти не больше стольких, `KEPT` модуля.
const KEPT: usize = 256;

/// Тайлы воды основы и маски тайлов глобуса.
#[derive(Default)]
pub struct WaterTiles {
    water: WaterMask,
    /// Тайлы воды, которых нет у источника или которые не разобрались:
    /// маски под ними нет, вода по высотам.
    failed: HashSet<TileKey>,
}

impl WaterTiles {
    /// Тайлы воды для масок тайлов кадра draw. low - у тайла рельеф ниже
    /// нуля, только им маска и нужна.
    pub fn wanted(&self, draw: &[TileKey], low: &dyn Fn(TileKey) -> bool, has_mask: &dyn Fn(TileKey) -> bool) -> Vec<(TileKey, f64)> {
        let mut out: Vec<(TileKey, f64)> = Vec::new();
        for key in draw {
            if has_mask(*key) || !low(*key) {
                continue;
            }
            for (water, _, _) in wm::mask_parts(*key) {
                if !self.water.contains(water) && !self.failed.contains(&water) && !out.iter().any(|(k, _)| *k == water) {
                    out.push((water, 50.0 - f64::from(water.0)));
                }
            }
        }
        out
    }

    /// Пришедший тайл основы: байты или None - тайла нет у источника.
    pub fn add(&mut self, key: TileKey, bytes: Option<&Arc<Vec<u8>>>) {
        match bytes.map(|b| wm::decode_water(b)) {
            Some(Ok(features)) => {
                self.failed.remove(&key);
                self.water.add(key, features);
            }
            _ => {
                self.failed.insert(key);
            }
        }
    }

    /// Нарисовать маски тайлов кадра draw, у которых их нет, не больше
    /// [`MASKS_PER_FRAME`] и не дольше budget секунд, и положить их
    /// в видеокарту. Возвращает true, если масок осталось не нарисовано.
    pub fn step(&mut self, draw: &[TileKey], low: &dyn Fn(TileKey) -> bool, budget: f64, globe: &mut render::Globe) -> bool {
        let start = web_time::Instant::now();
        let mut made = 0;
        let mut more = false;
        for key in draw {
            if globe.has_water_mask(*key) || !low(*key) {
                continue;
            }
            let Some(rings) = wm::mask_rings(*key, self.water.tiles()) else {
                continue;
            };
            if made >= MASKS_PER_FRAME || start.elapsed().as_secs_f64() > budget {
                more = true;
                break;
            }
            let image = wm::mask_image(&rings, wm::MASK_SIDE);
            globe.set_water_mask(*key, wm::MASK_SIDE as u32, &image);
            made += 1;
        }
        if self.water.len() > KEPT {
            let keep: HashSet<TileKey> = draw.iter().flat_map(|k| wm::mask_parts(*k).into_iter().map(|(w, _, _)| w)).collect();
            self.water.retain(&keep);
        }
        more
    }
}
