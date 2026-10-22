// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! 3D-модели сцены на глобусе, как Model в KML.
//!
//! Модель - пакет файлов в наложении вида Model (`scene::models::read`):
//! части и картинки. Место - камера наложения (широта, долгота, высота,
//! азимут, наклон, крен), масштаб по осям и altitudeMode
//! (`scene::models::frame`, `anchor`). Модель с путём gx:Track стоит в его
//! первой точке, движения по пути пока нет. Разобранные пакеты помнятся
//! по ключу, картинки уменьшаются до [`MAX_SIDE`] по стороне. Свет -
//! солнце с юга на высоте около 50°, рассеянный - доля в render.

use std::collections::HashMap;
use std::sync::Arc;

use globe_core::DVec3;
use globe_core::camera::enu;
use render::{ModelMesh, ModelPart};
use scene::kml::Overlay;
use scene::models::{self, Part};
use scene::overlays::Kind;
use terrain::HeightStore;

/// Наибольшая сторона картинки модели в видеокарте, пикселей.
pub const MAX_SIDE: u32 = 2048;

/// Разобранная модель: части и картинки (ключ, ширина, высота, RGBA).
pub struct Parsed {
    pub parts: Vec<Part>,
    pub images: Vec<(String, u32, u32, Vec<u8>)>,
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
    let (parts, pictures) = models::read(package).map_err(|e| e.0)?;
    let images = pictures.iter().filter_map(|(key, bytes)| decode(bytes).map(|(w, h, rgba)| (key.clone(), w, h, rgba))).collect();
    Ok(Parsed { parts, images })
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

/// Модели в видеокарту по разобранной модели и месту.
pub fn mesh(parsed: &Parsed, o: &Overlay, heights: &HeightStore) -> Option<ModelMesh> {
    let ov = &o.overlay;
    let [mut lat, mut lon, mut alt, heading, tilt, roll] = ov.camera?;
    if let Some(first) = ov.track.as_ref().and_then(|t| t.first()) {
        (lat, lon, alt) = (first.lat, first.lon, first.alt);
    }
    let ground = heights.height_at(lat, lon);
    let center = models::anchor(lat, lon, &ov.mode, alt, ground, heights.scale());
    let matrix = models::frame(lat, lon, heading, tilt, roll, ov.scale);
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
            }
        })
        .collect();
    Some(ModelMesh { center, parts, images: parsed.images.clone(), sun: sun_at(lat, lon) })
}

/// Разобранные модели сцены.
#[derive(Default)]
pub struct Models {
    parsed: HashMap<String, Arc<Parsed>>,
    /// Ключи, которые не разобрались, и причина.
    pub failed: HashMap<String, String>,
}

impl Models {
    /// Забыть разобранные модели: открыта другая сцена.
    pub fn clear(&mut self) {
        self.parsed.clear();
        self.failed.clear();
    }

    /// Модели видимых наложений для видеокарты.
    pub fn meshes(&mut self, overlays: &[Overlay], heights: &HeightStore) -> Vec<ModelMesh> {
        let mut out = Vec::new();
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
                            self.parsed.insert(key, p.clone());
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
                out.push(m);
            }
        }
        out
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
    fn turbines_scene_models_become_meshes() {
        let Ok(bytes) = std::fs::read(r"C:\Dev\planetx\planetx\demo\turbines.planetx") else {
            eprintln!("демо модуля нет, проверка пропущена");
            return;
        };
        let opened = crate::marks::open(&bytes, "turbines.planetx").unwrap();
        let overlays: Vec<_> = opened.tree.overlays().into_iter().cloned().collect();
        assert_eq!(overlays.len(), 3);
        let mut models = Models::default();
        let meshes = models.meshes(&overlays, &HeightStore::new());
        assert!(models.failed.is_empty(), "{:?}", models.failed);
        assert_eq!(meshes.len(), 3);
        // Ветряк: 2 части, 132 и 144 вершины, как у модуля.
        let counts: Vec<usize> = meshes[0].parts.iter().map(|p| p.positions.len()).collect();
        assert_eq!(counts, [132, 144]);
        // Ветряк стоит: самая высокая точка заметно выше точки модели.
        let up = meshes[0].center.normalize();
        let top = meshes[0].parts.iter().flat_map(|p| &p.positions).map(|p| globe_core::DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2])).dot(up)).fold(f64::MIN, f64::max);
        assert!(top > 30.0, "{top}");
    }
}
