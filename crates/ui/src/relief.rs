// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Свой рельеф: растры высот, врезанные в Terrarium, как врезка модуля
//! (`ui/inset.py`).
//!
//! Тайл Terrarium, который накрывает растр, при приходе получает высоты
//! растра (`terrain::inset::merge`). Тайлы глубже уровня 15 внутри
//! рамки растра Terrarium не отдаёт: они собираются здесь из предка
//! уровня 15 и растра, не больше [`PER_FRAME`] за кадр. Растр,
//! добавленный после загрузки высот, переписывает уже загруженные тайлы.

use std::collections::{HashMap, HashSet};

use data::dem::Dem;
use globe_core::lod::TileKey;
use terrain::inset::{self, Bounds};
use terrain::{HeightStore, HeightTile, Inset, MAX_LEVEL, SIZE, ancestor};

/// Тайлов глубже уровня 15, собираемых за кадр.
pub const PER_FRAME: usize = 4;

struct Entry {
    name: String,
    dem: Dem,
    bounds: Bounds,
    level: u32,
}

/// Врезанные растры и очередь глубоких тайлов.
#[derive(Default)]
pub struct Relief {
    entries: Vec<Entry>,
    /// Глубокие тайлы, чей предок уровня 15 готов, с приоритетом.
    pending: HashMap<TileKey, f64>,
}

impl Relief {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Названия врезанных растров.
    pub fn names(&self) -> Vec<&str> {
        self.entries.iter().map(|e| e.name.as_str()).collect()
    }

    /// Рамки врезок для [`HeightStore::deep`].
    pub fn insets(&self) -> Vec<Inset> {
        self.entries
            .iter()
            .map(|e| {
                let (u0, v0, u1, v1) = inset::shares(e.bounds);
                Inset { u0, v0, u1, v1, level: e.level }
            })
            .collect()
    }

    /// Врезать растр. Загруженные тайлы под ним переписываются.
    pub fn add(&mut self, name: &str, dem: Dem, store: &mut HeightStore) {
        let entry = Entry { name: name.to_string(), bounds: dem.bounds, level: inset::level_for(dem.pixel), dem };
        for key in store.keys() {
            if key.0 > MAX_LEVEL || !inset::overlaps(inset::tile_bounds(key.0, key.1, key.2), entry.bounds) {
                continue;
            }
            if let Some(tile) = store.tile(key).map(|t| (**t).clone())
                && let Some(patched) = patch(tile, std::slice::from_ref(&entry))
            {
                store.add(patched);
            }
        }
        self.entries.push(entry);
        store.deep = self.insets();
        self.pending.clear();
    }

    /// Убрать все растры. Высоты под ними вернутся с новыми тайлами.
    pub fn clear(&mut self, store: &mut HeightStore) {
        if self.entries.is_empty() {
            return;
        }
        self.entries.clear();
        self.pending.clear();
        store.deep.clear();
        store.clear();
    }

    /// Тайл Terrarium с врезанными растрами.
    pub fn patch(&self, tile: HeightTile) -> HeightTile {
        let key = tile.key();
        let touched: Vec<&Entry> =
            self.entries.iter().filter(|e| inset::overlaps(inset::tile_bounds(key.0, key.1, key.2), e.bounds)).collect();
        if touched.is_empty() {
            return tile;
        }
        let mut out = tile;
        for e in touched {
            if let Some(p) = patch(out.clone(), std::slice::from_ref(e)) {
                out = p;
            }
        }
        out
    }

    /// Нужды кадра в высотах: тайлы глубже уровня 15 уходят в очередь
    /// сборки, если их предок уровня 15 готов, иначе просится предок.
    /// Возвращает нужды для загрузчика Terrarium.
    pub fn route(&mut self, needs: Vec<(TileKey, f64)>, store: &HeightStore) -> Vec<(TileKey, f64)> {
        let mut out: HashMap<TileKey, f64> = HashMap::new();
        for (key, p) in needs {
            if key.0 <= MAX_LEVEL {
                let slot = out.entry(key).or_insert(p);
                *slot = slot.max(p);
                continue;
            }
            let base = ancestor(key, MAX_LEVEL);
            if store.tile(base).is_some() {
                let slot = self.pending.entry(key).or_insert(p);
                *slot = slot.max(p);
            } else {
                let slot = out.entry(base).or_insert(p);
                *slot = slot.max(p);
            }
        }
        out.into_iter().collect()
    }

    /// Собирается ли что-то.
    pub fn busy(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Собрать до [`PER_FRAME`] глубоких тайлов из очереди, важные раньше.
    pub fn build(&mut self, store: &mut HeightStore) {
        if self.pending.is_empty() {
            return;
        }
        let mut order: Vec<(TileKey, f64)> = self.pending.iter().map(|(k, p)| (*k, *p)).collect();
        order.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut done = HashSet::new();
        for (key, _) in order.into_iter().take(PER_FRAME) {
            done.insert(key);
            if store.tile(key).is_some() {
                continue;
            }
            let Some(base) = store.tile(ancestor(key, MAX_LEVEL)).cloned() else {
                continue;
            };
            let heights = inset::resample(&base, key);
            let Some(tile) = HeightTile::new(key.0, key.1, key.2, heights) else {
                continue;
            };
            store.add(self.patch(tile));
        }
        self.pending.retain(|k, _| !done.contains(k));
    }
}

/// Тайл с врезанными растрами entries. None - тайл не того размера.
fn patch(tile: HeightTile, entries: &[Entry]) -> Option<HeightTile> {
    let (z, x, y) = tile.key();
    let bounds = inset::tile_bounds(z, x, y);
    let mut heights = tile.heights;
    for e in entries {
        let (dem, weight) = e.dem.sample(bounds, SIZE);
        heights = inset::merge(&heights, &dem, &weight);
    }
    HeightTile::new(z, x, y, heights)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::Relief;
    use data::dem::Dem;
    use data::grid::Grid;
    use terrain::{HeightStore, HeightTile, MAX_LEVEL, SIZE, ancestor, inset};

    fn quarry() -> Option<Dem> {
        let bytes = std::fs::read(r"C:\Dev\planetx\planetx\demo\quarry\quarry_dem.tif").ok()?;
        Some(Dem::new(Grid::read(bytes, "quarry_dem.tif").ok()?, inset::BAND_SHARE).unwrap())
    }

    /// Ключ тайла уровня z под точкой EPSG:3857.
    fn key_at(z: u32, mx: f64, my: f64) -> (u32, u32, u32) {
        let n = f64::from(1u32 << z);
        let u = (mx + inset::HALF) / (2.0 * inset::HALF);
        let v = (inset::HALF - my) / (2.0 * inset::HALF);
        (z, (u * n) as u32, (v * n) as u32)
    }

    #[test]
    fn quarry_replaces_terrarium_and_goes_deep() {
        let Some(dem) = quarry() else {
            eprintln!("демо модуля нет, проверка пропущена");
            return;
        };
        let (w, s, e, n) = dem.bounds;
        let (cx, cy) = ((w + e) / 2.0, (s + n) / 2.0);
        let mut store = HeightStore::new();
        // Плоский Terrarium 1000 м под серединой до врезки.
        let k15 = key_at(MAX_LEVEL, cx, cy);
        store.add(HeightTile::new(k15.0, k15.1, k15.2, vec![1000.0; SIZE * SIZE]).unwrap());
        let mut relief = Relief::default();
        relief.add("quarry_dem.tif", dem, &mut store);
        // Уже загруженный тайл переписан: в середине высоты карьера,
        // 81-274 м, а не 1000.
        let t = store.tile(k15).unwrap();
        let mid = t.heights[128 * SIZE + 128];
        assert!((80.0..=275.0).contains(&mid), "{mid}");
        // Тайл уровня 19 подложки просит высоты уровня 17 - врезки.
        let draw = key_at(19, cx, cy);
        assert_eq!(store.wanted(draw).0, 17);
        let needs = relief.route(vec![(store.wanted(draw), 5.0)], &store);
        assert!(needs.is_empty(), "предок готов, грузить нечего: {needs:?}");
        relief.build(&mut store);
        let deep = store.tile(store.wanted(draw)).unwrap();
        assert_eq!(deep.z, 17);
        assert_eq!(ancestor(deep.key(), MAX_LEVEL), k15);
        assert!(deep.heights.iter().all(|h| (80.0..=275.0).contains(h)));
        assert!(!relief.busy());
    }
}
