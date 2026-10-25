// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Снимки неба HiPS при приближении, перенос `SkySurveys`
//! (`ui/skysurvey.py`) и выбора тайлов `HipsSky.select` (`render/hips.py`)
//! модуля. Слои - [`data::hips::LAYERS`]: ближе поля зрения около 50°
//! проявляется карта неба NASA SVS, ближе 6° - снимки DSS.
//!
//! Порядок тайлов - по пикселю экрана ([`data::hips::order_for`]),
//! ячейки - задевающие конус взгляда. Пока тайла нет, на его месте
//! готовый предок не мельче нижнего порядка слоя, у слоя с Allsky.jpg -
//! кусок этой картинки порядка 3. Тайлов в видеокарте не больше KEEP
//! на слой, вытесняются давно не нужные. Просится не больше PARALLEL
//! тайлов сразу, отказ повторяется через RETRY секунд.
//!
//! Нет пока: проявления пришедших тайлов по времени.

use std::collections::{HashMap, HashSet};

use data::hips::{self, Layer};
use data::skyview::SkyView;

/// Тайлов в загрузке одновременно, как у модуля.
const PARALLEL: usize = 6;
/// Секунд до повторной просьбы тайла после отказа.
const RETRY: f64 = 60.0;
/// Тайлов слоя в видеокарте.
const KEEP: usize = 160;
/// Тайлов вида не больше, `WANT_LIMIT` модуля.
const WANT_LIMIT: usize = 120;
/// Предел ответа тайла и Allsky.jpg.
const ANSWER_BYTES: u64 = 8 << 20;
/// Сдвиг номера слоя у кусков Allsky.jpg в ключах видеокарты.
const ALLSKY_LAYER: u8 = 100;

/// Что просится: тайл слоя или Allsky.jpg слоя.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Want {
    Tile(u8, u32, u64),
    Allsky(u8),
}

/// Снимки неба.
#[derive(Default)]
pub struct Surveys {
    loading: HashMap<Want, Box<dyn net::Request>>,
    failed: HashMap<Want, f64>,
    /// Allsky.jpg слоя разрезан и лежит в видеокарте.
    allsky: HashSet<u8>,
    /// Тайлы слоёв в видеокарте по давности использования.
    used: HashMap<(u8, u32, u64), u64>,
    frame: u64,
    /// Проявление слоёв в последнем кадре.
    pub shares: Vec<f64>,
}

/// Вершины тайла для видеокарты.
fn mesh(order: u32, ipix: u64) -> (Vec<render::HipsVertex>, Vec<u32>) {
    let (dirs, uv) = hips::tile_mesh(order, ipix, hips::CELL_STEPS);
    let vertices = dirs.iter().zip(&uv).map(|(d, t)| render::HipsVertex { dir: d.map(|v| v as f32), uv: [t[0] as f32, t[1] as f32] }).collect();
    (vertices, hips::mesh_indices(hips::CELL_STEPS))
}

/// Кусок картинки RGBA: x, y, сторона.
fn crop(rgba: &[u8], width: u32, x: u32, y: u32, side: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((side * side * 4) as usize);
    for r in 0..side {
        let start = (((y + r) * width + x) * 4) as usize;
        out.extend_from_slice(rgba.get(start..start + (side * 4) as usize).unwrap_or(&[]));
    }
    out
}

impl Surveys {
    pub fn busy(&self) -> bool {
        !self.loading.is_empty()
    }

    /// Проявление слоя по полю зрения, 0 без снимков.
    pub fn share(&self, n: usize) -> f64 {
        self.shares.get(n).copied().unwrap_or(0.0)
    }

    /// Шаг кадра неба: ответы в видеокарту, выбор тайлов, новые запросы.
    /// on - флажок «Снимки неба при приближении», frame - соотношение
    /// сторон и высота кадра.
    pub fn step(&mut self, globe: &mut render::Globe, view: &SkyView, frame: (f64, f64), on: bool, now: f64, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        let (aspect, height) = frame;
        self.frame += 1;
        self.shares = hips::LAYERS.iter().map(|l| if on { hips::share(l, view.fov) } else { 0.0 }).collect();
        self.arrive(globe, now);
        let mut order: Vec<((u8, u32, u64), f32)> = Vec::new();
        let mut want: Vec<(Want, String)> = Vec::new();
        let t = (view.fov.to_radians() / 2.0).tan();
        let reach = (t * (1.0 + aspect * aspect).sqrt()).atan();
        let axis = view.forward();
        for (n, layer) in hips::LAYERS.iter().enumerate() {
            let share = self.share(n);
            if share <= 0.0 {
                continue;
            }
            let id = n as u8;
            let base = layer.base();
            if layer.allsky && !self.allsky.contains(&id) {
                want.push((Want::Allsky(id), hips::allsky_url(&base)));
            }
            let (draw, need) = self.select(globe, layer, id, axis, reach, view.radians_per_pixel(height));
            order.extend(draw.into_iter().map(|k| (k, share as f32)));
            want.extend(need.into_iter().map(|(o, p)| (Want::Tile(id, o, p), hips::tile_url(&base, o, p))));
        }
        globe.set_hips_order(order);
        // Запросы вне нужных бросаются, Allsky дожидается.
        let wanted: HashSet<Want> = want.iter().map(|(w, _)| *w).collect();
        self.loading.retain(|w, _| matches!(w, Want::Allsky(_)) || wanted.contains(w));
        self.failed.retain(|_, t| now - *t < RETRY);
        for (w, url) in want {
            if self.loading.len() >= PARALLEL {
                break;
            }
            if self.loading.contains_key(&w) || self.failed.contains_key(&w) {
                continue;
            }
            let r = request(ANSWER_BYTES);
            r.start(&url);
            self.loading.insert(w, r);
        }
        self.evict(globe);
    }

    /// Пришедшие картинки - в видеокарту.
    fn arrive(&mut self, globe: &mut render::Globe, now: f64) {
        let done: Vec<(Want, Result<Vec<u8>, String>)> = self.loading.iter().filter_map(|(w, r)| r.take().map(|a| (*w, a))).collect();
        for (w, answer) in done {
            self.loading.remove(&w);
            let image = answer.and_then(|b| image::load_from_memory(&b).map_err(|e| e.to_string()));
            let Ok(image) = image else {
                self.failed.insert(w, now);
                continue;
            };
            let rgba = image.to_rgba8();
            let (width, height) = rgba.dimensions();
            match w {
                Want::Tile(id, o, p) => {
                    let (v, i) = mesh(o, p);
                    globe.set_hips_tile((id, o, p), &v, &i, width, height, rgba.as_raw());
                    self.used.insert((id, o, p), self.frame);
                }
                Want::Allsky(id) => {
                    // Allsky.jpg: 768 тайлов порядка 3 рядами по 27.
                    let side = width / hips::ALLSKY_COLUMNS as u32;
                    if side == 0 {
                        self.failed.insert(w, now);
                        continue;
                    }
                    for p in 0..768u64 {
                        let [u0, v0, _, _] = hips::allsky_rect(p, f64::from(width), f64::from(height));
                        let (x, y) = ((u0 * f64::from(width)).round() as u32, (v0 * f64::from(height)).round() as u32);
                        if x + side > width || y + side > height {
                            continue;
                        }
                        let piece = crop(rgba.as_raw(), width, x, y, side);
                        let (v, i) = mesh(hips::ALLSKY_ORDER, p);
                        globe.set_hips_tile((id + ALLSKY_LAYER, hips::ALLSKY_ORDER, p), &v, &i, side, side, &piece);
                    }
                    self.allsky.insert(id);
                }
            }
        }
    }

    /// Тайлы кадра слоя: что рисовать, от грубых к подробным, и чего
    /// не хватает, ближние к середине раньше.
    #[allow(clippy::type_complexity)]
    fn select(&mut self, globe: &render::Globe, layer: &Layer, id: u8, axis: [f64; 3], reach: f64, per_pixel: f64) -> (Vec<(u8, u32, u64)>, Vec<(u32, u64)>) {
        let mut order = hips::order_for(per_pixel, layer.top);
        if order.is_none() && !layer.allsky {
            order = Some(layer.low);
        }
        let mut draw: Vec<(u8, u32, u64)> = Vec::new();
        let mut need: Vec<(u32, u64)> = Vec::new();
        let Some(mut order) = order.map(|o| o.max(layer.low)) else {
            draw.extend(hips::visible(hips::MIN_ORDER, axis, reach).into_iter().map(|p| (id + ALLSKY_LAYER, hips::MIN_ORDER, p)));
            return (draw, need);
        };
        let mut cells = hips::visible(order, axis, reach);
        while order > layer.low && cells.len() > WANT_LIMIT {
            order -= 1;
            cells = hips::visible(order, axis, reach);
        }
        let dot = |p: u64| {
            let c = hips::center(order, p);
            c[0] * axis[0] + c[1] * axis[1] + c[2] * axis[2]
        };
        cells.sort_by(|a, b| dot(*b).total_cmp(&dot(*a)));
        let mut seen = HashSet::new();
        for p in cells {
            if !globe.has_hips_tile((id, order, p)) {
                need.push((order, p));
            }
            // Готовый тайл или его ближайший готовый предок.
            let found = (layer.low..=order).rev().map(|level| (id, level, p >> (2 * (order - level)))).find(|k| globe.has_hips_tile(*k));
            let key = match found {
                Some(k) => k,
                None if layer.allsky => (id + ALLSKY_LAYER, hips::MIN_ORDER, p >> (2 * order.saturating_sub(hips::MIN_ORDER))),
                None => continue,
            };
            if seen.insert(key) {
                self.used.insert(key, self.frame);
                draw.push(key);
            }
        }
        // Allsky - под тайлами, грубые порядки - раньше подробных.
        draw.sort_by_key(|k| (k.0 < ALLSKY_LAYER, k.1));
        (draw, need)
    }

    /// Лишние тайлы слоя - из видеокарты, давно не нужные первыми.
    fn evict(&mut self, globe: &mut render::Globe) {
        for id in 0..hips::LAYERS.len() as u8 {
            let mut mine: Vec<((u8, u32, u64), u64)> = self.used.iter().filter(|(k, _)| k.0 == id).map(|(k, f)| (*k, *f)).collect();
            if mine.len() <= KEEP {
                continue;
            }
            mine.sort_by_key(|(_, f)| *f);
            let drop: HashSet<(u8, u32, u64)> = mine[..mine.len() - KEEP].iter().map(|(k, _)| *k).collect();
            self.used.retain(|k, _| !drop.contains(k));
            globe.retain_hips(&|k| !drop.contains(&k));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::crop;

    #[test]
    fn crop_takes_the_square() {
        // Картинка 4 × 2, кусок 2 × 2 с x = 2.
        let rgba: Vec<u8> = (0..4 * 2 * 4).map(|i| i as u8).collect();
        let piece = crop(&rgba, 4, 2, 0, 2);
        assert_eq!(&piece[..4], &[8, 9, 10, 11]);
        assert_eq!(&piece[8..12], &[24, 25, 26, 27]);
    }
}
