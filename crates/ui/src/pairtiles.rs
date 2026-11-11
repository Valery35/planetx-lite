// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Тайлы пары текстур «после» и «до» шторки по адресу-шаблону XYZ:
//! выпуски Wayback и дни темы NASA GIBS. Сторона 0 - справа от линии,
//! 1 - слева, как у снимков событий.

use globe_core::lod::TileKey;

/// Наибольший тайл, байт.
const TILE_BYTES: u64 = 1 << 20;
/// Запросов тайлов одной стороны сразу, не больше.
const FETCH_MOST: usize = 8;

type Requests<'a> = &'a dyn Fn(u64) -> Box<dyn net::Request>;

/// Адрес тайла по шаблону с {z}, {x}, {y}.
pub fn address(template: &str, key: TileKey) -> String {
    let (z, x, y) = key;
    template.replace("{z}", &z.to_string()).replace("{y}", &y.to_string()).replace("{x}", &x.to_string())
}

/// Запросы тайлов двух сторон.
#[derive(Default)]
pub struct PairTiles {
    pending: [Vec<(TileKey, Box<dyn net::Request>)>; 2],
}

impl PairTiles {
    /// Тайлы стороны side по шаблону template: пришедшие картинки RGBA
    /// 256 × 256 (None - тайла нет) и новые запросы для keys, не больше
    /// [`FETCH_MOST`] сразу.
    pub fn step(&mut self, side: usize, template: &str, keys: &[TileKey], request: Requests) -> Vec<(TileKey, Option<Vec<u8>>)> {
        let Some(pending) = self.pending.get_mut(side) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        pending.retain(|(key, ask)| match ask.take() {
            Some(answer) => {
                out.push((*key, answer.ok().and_then(|b| net::decode_tile(&b).ok()).map(|i| i.rgba)));
                false
            }
            None => true,
        });
        for key in keys {
            if pending.len() >= FETCH_MOST {
                break;
            }
            if pending.iter().any(|(k, _)| k == key) {
                continue;
            }
            let ask = request(TILE_BYTES);
            ask.start(&address(template, *key));
            pending.push((*key, ask));
        }
        out
    }

    /// Забыть запросы стороны: у неё другой выпуск или день.
    pub fn clear(&mut self, side: usize) {
        if let Some(p) = self.pending.get_mut(side) {
            p.clear();
        }
    }

    /// Идут ли запросы.
    pub fn busy(&self) -> bool {
        self.pending.iter().any(|p| !p.is_empty())
    }
}
