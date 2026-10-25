// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Строка «3D-здания»: здания из слоя building векторных тайлов
//! OpenFreeMap. Перенос `render/buildings.py` модуля, расчёт -
//! `data::buildings`.
//!
//! Тайлы уровня `buildings::LEVEL` ближе `buildings::RANGE` к глазу
//! просит загрузчик тайлов OpenFreeMap, тот же, что у подписей. Тайл
//! разбирается в рабочем потоке (`footprints`), у страницы - сразу.
//! Сетка по высотам рельефа собирается в главном потоке, не больше
//! [`UPLOADS`] за кадр. Сменились высоты - сетка тайла собирается заново,
//! до подмены рисуется прежняя. Сетки рисует подземный проход видеокарты
//! с проверкой глубины, как блоки подземного режима.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use data::buildings::{self as core, Footprint, TileKey};

/// Сеток в видеокарту за кадр, `UPLOADS` модуля.
pub const UPLOADS: usize = 1;
/// Разобранных тайлов в памяти, по давности, `FOOTPRINTS_KEPT` модуля.
pub const KEPT: usize = 96;

/// Разобранный тайл: контуры или пусто.
type Parsed = (TileKey, Option<Arc<Footprint>>);

pub struct Buildings {
    footprints: HashMap<TileKey, Option<Arc<Footprint>>>,
    order: VecDeque<TileKey>,
    parsing: HashSet<TileKey>,
    #[cfg(not(target_arch = "wasm32"))]
    done: (std::sync::mpsc::Sender<Parsed>, std::sync::mpsc::Receiver<Parsed>),
    /// Тайлы кадра, ближние первыми.
    pub wanted: Vec<TileKey>,
    /// Сетки в видеокарте: тайл и версия высот.
    shown: HashMap<TileKey, u64>,
}

impl Default for Buildings {
    fn default() -> Self {
        Self {
            footprints: HashMap::new(),
            order: VecDeque::new(),
            parsing: HashSet::new(),
            #[cfg(not(target_arch = "wasm32"))]
            done: std::sync::mpsc::channel(),
            wanted: Vec::new(),
            shown: HashMap::new(),
        }
    }
}

/// Имя сетки тайла в подземном проходе.
fn mesh_name(key: TileKey) -> String {
    format!("building:{}/{}/{}", key.0, key.1, key.2)
}

fn parse(key: TileKey, bytes: &[u8]) -> Parsed {
    (key, core::footprints(key, bytes).ok().flatten().map(Arc::new))
}

impl Buildings {
    /// Тайлы кадра: ближние к глазу в пределах бюджета вершин. eye
    /// и forward - глаз и направление взгляда в ECEF, ground - высота
    /// земли под глазом. Возвращает тайлы, которых ещё нет, с приоритетом
    /// загрузчика.
    pub fn want(&mut self, eye: [f64; 3], forward: [f64; 3], ground: f64) -> Vec<(TileKey, f64)> {
        let near = core::near_tiles(eye, forward, ground, core::RANGE, core::MAX_TILES);
        let mut total = 0;
        self.wanted.clear();
        for (key, _) in near {
            if let Some(Some(fp)) = self.footprints.get(&key) {
                total += core::vertex_count(fp);
                if total > core::MAX_VERTICES && !self.wanted.is_empty() {
                    break;
                }
            }
            self.wanted.push(key);
        }
        self.wanted.iter().enumerate().filter(|(_, k)| !self.footprints.contains_key(*k) && !self.parsing.contains(*k)).map(|(i, k)| (*k, 1.0e6 - i as f64)).collect()
    }

    /// Пришёл тайл: байты или None - тайла нет.
    pub fn add(&mut self, key: TileKey, bytes: Option<Arc<Vec<u8>>>) {
        let Some(bytes) = bytes else {
            self.keep(key, None);
            return;
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let tx = self.done.0.clone();
            if std::thread::Builder::new().name("planetx-buildings".into()).spawn(move || {
                let _ = tx.send(parse(key, &bytes));
            }).is_ok() {
                self.parsing.insert(key);
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let (key, fp) = parse(key, &bytes);
            self.keep(key, fp);
        }
    }

    fn keep(&mut self, key: TileKey, fp: Option<Arc<Footprint>>) {
        self.parsing.remove(&key);
        self.footprints.insert(key, fp);
        self.order.retain(|k| *k != key);
        self.order.push_back(key);
        while self.order.len() > KEPT {
            if let Some(old) = self.order.pop_front() {
                self.footprints.remove(&old);
            }
        }
    }

    /// Идёт разбор или сетки кадра не готовы.
    pub fn busy(&self, version: u64) -> bool {
        !self.parsing.is_empty() || self.wanted.iter().any(|k| matches!(self.footprints.get(k), Some(Some(_))) && self.shown.get(k) != Some(&version))
    }

    /// Шаг: разобранные тайлы, сетки по высотам в видеокарту, сетки вне
    /// кадра - вон. version - версия хранилища высот.
    pub fn step(&mut self, globe: &mut render::Globe, version: u64, heights: core::HeightsAt<'_>) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let arrived: Vec<Parsed> = self.done.1.try_iter().collect();
            for (key, fp) in arrived {
                self.keep(key, fp);
            }
        }
        let mut uploads = 0;
        for key in self.wanted.clone() {
            if uploads >= UPLOADS {
                break;
            }
            let Some(Some(fp)) = self.footprints.get(&key) else {
                continue;
            };
            if self.shown.get(&key) == Some(&version) {
                continue;
            }
            let mesh = core::mesh(fp, Some(heights));
            let name = mesh_name(key);
            if core::mesh_ok(&mesh) && !mesh.indices.is_empty() {
                let sub = render::SubMesh {
                    center: globe_core::DVec3::from_array(mesh.center),
                    vertices: mesh
                        .vertices
                        .iter()
                        .map(|v| render::SubVertex { offset: v.position, normal: [f32::from(v.normal[0]) / 127.0, f32::from(v.normal[1]) / 127.0, f32::from(v.normal[2]) / 127.0], color: v.color })
                        .collect(),
                    indices: mesh.indices,
                    cull_front: false,
                    bias: false,
                };
                globe.set_subsurface(&name, Some(&sub));
            } else {
                globe.set_subsurface(&name, None);
            }
            self.shown.insert(key, version);
            uploads += 1;
        }
        let keep: HashSet<TileKey> = self.wanted.iter().copied().collect();
        let gone: Vec<TileKey> = self.shown.keys().copied().filter(|k| !keep.contains(k)).collect();
        for key in gone {
            globe.set_subsurface(&mesh_name(key), None);
            self.shown.remove(&key);
        }
    }

    /// Убрать сетки из видеокарты: строка выключена.
    pub fn clear(&mut self, globe: &mut render::Globe) {
        for key in self.shown.keys() {
            globe.set_subsurface(&mesh_name(*key), None);
        }
        self.shown.clear();
        self.wanted.clear();
    }
}
