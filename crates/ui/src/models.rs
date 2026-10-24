// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! 3D-модели сцены на глобусе, как Model в KML.
//!
//! Модель - пакет файлов в наложении вида Model (`scene::models::read`):
//! части и картинки. Место - камера наложения (широта, долгота, высота,
//! азимут, наклон, крен), масштаб по осям и altitudeMode
//! (`scene::models::frame`, `anchor`). Модель с путём gx:Track едет по
//! нему, как в модуле (`ui/globe/pictures.py`, `_track_model`): вершины
//! считаются один раз у точки 0, 0, место и поворот на момент шкалы
//! времени - `scene::track::model_pose` и `scene::models::moved`. Момент -
//! правый край промежутка шкалы, закрытая шкала - конец пути, вне
//! времени пути модели нет. Анимация узлов glTF и Collada
//! (`scene::animation`) идёт по часам окна от первого показа модели
//! по кругу, шкала времени её не ведёт, как у модуля. Разобранные пакеты помнятся
//! по ключу, картинки уменьшаются до [`MAX_SIDE`] по стороне. Свет -
//! солнце с юга на высоте около 50°, рассеянный - доля в render.

use std::collections::HashMap;
use std::sync::Arc;

use globe_core::camera::enu;
use globe_core::{DMat3, DMat4, DVec3};
use render::{ModelMesh, ModelPart, ModelPlace};
use scene::animation::{self, Animation};
use scene::kml::Overlay;
use scene::models::{self, Part};
use scene::overlays::{Kind, TrackRow};
use scene::track::model_pose;
use terrain::HeightStore;

/// Наибольшая сторона картинки модели в видеокарте, пикселей.
pub const MAX_SIDE: u32 = 2048;

/// Разобранная модель: части, картинки (ключ, ширина, высота, RGBA)
/// и анимация узлов или None.
pub struct Parsed {
    pub parts: Vec<Part>,
    pub images: Vec<(String, u32, u32, Vec<u8>)>,
    pub animation: Option<Animation>,
}

/// Картинка модели в RGBA, не больше [`MAX_SIDE`] по стороне.
fn decode(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    if w > MAX_SIDE || h > MAX_SIDE {
        let k = f64::from(MAX_SIDE) / f64::from(w.max(h));
        let (nw, nh) = (((f64::from(w) * k) as u32).max(1), ((f64::from(h) * k) as u32).max(1));
        img = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle);
    }
    let (w, h) = img.dimensions();
    Some((w, h, img.into_raw()))
}

/// Разобрать пакет модели. Нечитаемая картинка - часть без картинки.
pub fn parse(package: &[u8]) -> Result<Parsed, String> {
    let (parts, pictures, animation) = models::read_animated(package).map_err(|e| e.0)?;
    let images = pictures.iter().filter_map(|(key, bytes)| decode(bytes).map(|(w, h, rgba)| (key.clone(), w, h, rgba))).collect();
    Ok(Parsed { parts, images, animation })
}

/// Ключ разобранной модели: адрес пакета и его длина.
pub fn key_of(o: &Overlay) -> String {
    format!("{}#{}", o.href, o.image.as_ref().map_or(0, Vec::len))
}

/// Направление на солнце в ECEF у точки (lat, lon): с юга, на высоте
/// около 50°, в южном полушарии - с севера.
pub fn sun_at(lat: f64, lon: f64) -> [f32; 3] {
    let [east, north, up] = enu(lat, lon);
    let toward = if lat >= 0.0 { -north } else { north };
    let d = (up * 1.2 + toward * 0.9 + east * 0.3).normalize_or_zero();
    [d.x as f32, d.y as f32, d.z as f32]
}

/// Модель на пути gx:Track: номер в наборе видеокарты, строки пути,
/// altitudeMode, масштаб и матрица, с которой посчитаны вершины.
#[derive(Debug, Clone)]
pub struct Mover {
    pub index: usize,
    /// Название модели для меню на глобусе.
    pub name: String,
    pub rows: Vec<TrackRow>,
    pub mode: String,
    pub scale: [f64; 3],
    pub base: DMat3,
}

impl Mover {
    /// Время пути на момент шкалы: правый край промежутка открытой
    /// шкалы, закрытая шкала - конец пути.
    pub fn moment(&self, window: Option<(f64, f64)>) -> f64 {
        window.map_or_else(|| self.rows.last().map_or(0.0, |r| r.t), |w| w.1)
    }

    /// Место модели на момент шкалы, вне времени пути - скрыта.
    pub fn place(&self, window: Option<(f64, f64)>, heights: &HeightStore) -> ModelPlace {
        match model_pose(&self.rows, self.moment(window)) {
            Some(p) => {
                let ground = heights.height_at(p.lat, p.lon);
                ModelPlace {
                    center: models::anchor(p.lat, p.lon, &self.mode, p.alt, ground, heights.scale()),
                    orient: models::moved(&self.base, p.lat, p.lon, p.heading, p.tilt, p.roll, self.scale),
                    sun: sun_at(p.lat, p.lon),
                    shown: true,
                }
            }
            None => ModelPlace { center: DVec3::ZERO, orient: DMat3::IDENTITY, sun: [0.0, 0.0, 1.0], shown: false },
        }
    }
}

/// Охват времени всех путей (от, до) или None, `span` модуля
/// (`ui/modeltrack.py`).
pub fn span(movers: &[Mover]) -> Option<(f64, f64)> {
    let lo = movers.iter().filter_map(|m| m.rows.first()).map(|r| r.t).reduce(f64::min)?;
    let hi = movers.iter().filter_map(|m| m.rows.last()).map(|r| r.t).reduce(f64::max)?;
    Some((lo, hi))
}

/// Матрица вершин модели: место и поворот наложения, у модели на пути -
/// точка 0, 0 без поворота.
fn matrix_of(o: &Overlay) -> Option<DMat3> {
    let ov = &o.overlay;
    let [lat, lon, _, heading, tilt, roll] = ov.camera?;
    let moving = ov.track.as_ref().is_some_and(|t| !t.is_empty());
    Some(if moving { models::frame(0.0, 0.0, 0.0, 0.0, 0.0, ov.scale) } else { models::frame(lat, lon, heading, tilt, roll, ov.scale) })
}

/// Анимированная модель: номер в наборе видеокарты, анимация, матрица,
/// с которой посчитаны вершины, и время первого показа.
#[derive(Debug, Clone)]
pub struct Animated {
    pub index: usize,
    pub animation: Animation,
    pub matrix: DMat3,
    pub started: f64,
}

impl Animated {
    /// Матрицы дорожек в осях вершин на момент now, как
    /// `animation.frame_delta` модуля. Время идёт от первого показа
    /// модели по кругу длительности анимации.
    pub fn deltas(&self, now: f64) -> HashMap<usize, DMat4> {
        self.animation
            .deltas(now - self.started)
            .into_iter()
            .map(|(track, delta)| {
                let (linear, shift) = animation::frame_delta(&self.matrix, &delta);
                let mut m = DMat4::from_mat3(linear);
                m.w_axis = shift.extend(1.0);
                (track, m)
            })
            .collect()
    }
}

/// Модели для видеокарты, модели на путях и анимированные модели.
#[derive(Default)]
pub struct Built {
    pub meshes: Vec<ModelMesh>,
    pub movers: Vec<Mover>,
    pub animated: Vec<Animated>,
}

/// Модели в видеокарту по разобранной модели и месту. Модель на пути -
/// вершины у точки 0, 0, место - [`Mover::place`].
pub fn mesh(parsed: &Parsed, o: &Overlay, heights: &HeightStore) -> Option<ModelMesh> {
    let ov = &o.overlay;
    let [lat, lon, alt, ..] = ov.camera?;
    let ground = heights.height_at(lat, lon);
    let center = models::anchor(lat, lon, &ov.mode, alt, ground, heights.scale());
    let matrix = matrix_of(o)?;
    let normal_matrix = if matrix.determinant().abs() > 1e-12 { matrix.inverse().transpose() } else { matrix };
    let f = |v: DVec3| [v.x as f32, v.y as f32, v.z as f32];
    let parts = parsed
        .parts
        .iter()
        .map(|p| {
            let normals = p.normals.clone().unwrap_or_else(|| models::flat_normals(&p.positions));
            ModelPart {
                positions: p.positions.iter().map(|v| f(matrix * *v)).collect(),
                normals: normals.iter().map(|n| f((normal_matrix * *n).normalize_or_zero())).collect(),
                uv: p.uv.as_ref().map_or_else(Vec::new, |uv| uv.iter().map(|t| [t.x as f32, t.y as f32]).collect()),
                image: p.image.clone(),
                rgba: p.rgba.map(|c| c as f32),
                track: p.track,
            }
        })
        .collect();
    Some(ModelMesh { center, parts, images: parsed.images.clone(), sun: sun_at(lat, lon), orient: DMat3::IDENTITY })
}

/// Разобранные модели сцены.
#[derive(Default)]
pub struct Models {
    parsed: HashMap<String, Arc<Parsed>>,
    /// Ключи, которые не разобрались, и причина.
    pub failed: HashMap<String, String>,
    /// Время первого показа анимированной модели по ключу: анимация
    /// идёт от него, как `started` в `render/models.py` модуля.
    started: HashMap<String, f64>,
}

impl Models {
    /// Забыть разобранные модели: открыта другая сцена.
    pub fn clear(&mut self) {
        self.parsed.clear();
        self.failed.clear();
        self.started.clear();
    }

    /// Модели видимых наложений для видеокарты, модели на путях
    /// и анимированные модели. now - часы окна, секунды.
    pub fn meshes(&mut self, overlays: &[Overlay], heights: &HeightStore, now: f64) -> Built {
        let mut out = Vec::new();
        let mut movers = Vec::new();
        let mut animated = Vec::new();
        for o in overlays.iter().filter(|o| o.kind() == Kind::Model) {
            let key = key_of(o);
            if self.failed.contains_key(&key) {
                continue;
            }
            let parsed = match self.parsed.get(&key) {
                Some(p) => p.clone(),
                None => {
                    let Some(package) = o.image.as_deref().filter(|b| !b.is_empty()) else {
                        self.failed.insert(key, "нет файлов модели".into());
                        continue;
                    };
                    match parse(package) {
                        Ok(p) => {
                            let p = Arc::new(p);
                            self.parsed.insert(key.clone(), p.clone());
                            p
                        }
                        Err(e) => {
                            self.failed.insert(key, e);
                            continue;
                        }
                    }
                }
            };
            if let Some(m) = mesh(&parsed, o, heights) {
                if let (Some(found), Some(matrix)) = (parsed.animation.as_ref(), matrix_of(o)) {
                    let started = *self.started.entry(key.clone()).or_insert(now);
                    animated.push(Animated { index: out.len(), animation: found.clone(), matrix, started });
                }
                if let Some(rows) = o.overlay.track.as_ref().filter(|t| !t.is_empty()) {
                    let scale = o.overlay.scale;
                    movers.push(Mover {
                        index: out.len(),
                        name: o.name.clone(),
                        rows: rows.clone(),
                        mode: o.overlay.mode.clone(),
                        scale,
                        base: models::frame(0.0, 0.0, 0.0, 0.0, 0.0, scale),
                    });
                }
                out.push(m);
            }
        }
        Built { meshes: out, movers, animated }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{Models, sun_at};
    use terrain::HeightStore;

    #[test]
    fn sun_is_above_and_from_the_south() {
        let [x, y, z] = sun_at(58.0, 56.0);
        let up = globe_core::camera::enu(58.0, 56.0)[2];
        let s = globe_core::DVec3::new(f64::from(x), f64::from(y), f64::from(z));
        assert!(s.dot(up) > 0.6);
        assert!(s.dot(globe_core::camera::enu(58.0, 56.0)[1]) < 0.0);
    }

    #[test]
    fn haul_trucks_move_along_tracks() {
        let Ok(bytes) = std::fs::read(r"C:\Dev\planetx\planetx\demo\haul_kachkanar.planetx") else {
            eprintln!("демо модуля нет, проверка пропущена");
            return;
        };
        let opened = crate::marks::open(&bytes, "haul_kachkanar.planetx").unwrap();
        let overlays: Vec<_> = opened.tree.overlays().into_iter().cloned().collect();
        let mut models = Models::default();
        let heights = HeightStore::new();
        let super::Built { meshes, movers, .. } = models.meshes(&overlays, &heights, 0.0);
        assert!(models.failed.is_empty(), "{:?}", models.failed);
        assert_eq!((meshes.len(), movers.len()), (6, 6));
        // Рейс 08:00 - 09:00 20 октября 2026 года.
        let (start, end) = super::span(&movers).unwrap();
        assert_eq!(end - start, 3600.0);
        let truck = &movers[0];
        let at = |t: f64| truck.place(Some((t, t)), &heights);
        let (a, b) = (at(start), at(start + 1254.0));
        assert!(a.shown && b.shown);
        // За 20 минут самосвал ушёл почти на километр, к точке пути
        // 08:20:54 - 59.5159948, 58.7533024.
        let want = globe_core::ellipsoid::Ellipsoid::wgs84().geodetic_to_ecef(58.7533024, 59.5159948, 0.0);
        assert!((b.center - want).length() < 1.0, "{}", (b.center - want).length());
        assert!((a.center - b.center).length() > 500.0);
        // Поворот модели - ортонормальный.
        let o = b.orient;
        assert!((o * o.transpose() - globe_core::DMat3::IDENTITY).abs_diff_eq(globe_core::DMat3::ZERO, 1e-9));
        // Вне времени рейса модели нет, закрытая шкала - конец пути.
        assert!(!at(start - 1.0).shown);
        let last = truck.place(None, &heights);
        let want = globe_core::ellipsoid::Ellipsoid::wgs84().geodetic_to_ecef(58.7518244, 59.5098495, 0.0);
        assert!(last.shown && (last.center - want).length() < 1.0);
    }

    #[test]
    fn turbines_scene_models_become_meshes() {
        let Ok(bytes) = std::fs::read(r"C:\Dev\planetx\planetx\demo\turbines.planetx") else {
            eprintln!("демо модуля нет, проверка пропущена");
            return;
        };
        let opened = crate::marks::open(&bytes, "turbines.planetx").unwrap();
        let overlays: Vec<_> = opened.tree.overlays().into_iter().cloned().collect();
        assert_eq!(overlays.len(), 3);
        let mut models = Models::default();
        let super::Built { meshes, movers, animated } = models.meshes(&overlays, &HeightStore::new(), 0.0);
        assert!(movers.is_empty());
        assert!(models.failed.is_empty(), "{:?}", models.failed);
        assert_eq!(meshes.len(), 3);
        // Ветряк: 2 части, 132 и 144 вершины, как у модуля.
        let counts: Vec<usize> = meshes[0].parts.iter().map(|p| p.positions.len()).collect();
        assert_eq!(counts, [132, 144]);
        // Ветряк стоит: самая высокая точка заметно выше точки модели.
        let up = meshes[0].center.normalize();
        let top = meshes[0].parts.iter().flat_map(|p| &p.positions).map(|p| globe_core::DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2])).dot(up)).fold(f64::MIN, f64::max);
        assert!(top > 30.0, "{top}");
        // Лопасти крутятся: у каждого ветряка анимация, одна часть на
        // дорожке, её матрица за полсекунды меняется, ось стоит.
        assert_eq!(animated.len(), 3);
        let turbine = &animated[0];
        assert!(meshes[0].parts.iter().filter(|p| p.track.is_some()).count() >= 1);
        let (a, b) = (turbine.deltas(0.0), turbine.deltas(0.5));
        let track = *a.keys().next().unwrap();
        assert!(!a[&track].abs_diff_eq(b[&track], 1e-6));
        let linear = globe_core::DMat3::from_mat4(b[&track]);
        assert!((linear * linear.transpose()).abs_diff_eq(globe_core::DMat3::IDENTITY, 1e-6));
    }
}
