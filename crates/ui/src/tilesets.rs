// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Наборы 3D Tiles на глобусе: строки раздела «Слои» и сетки для
//! видеокарты. Чтение - `scene::tiles3d`. Набор открывается архивом ZIP
//! (так его выгружают модуль и Lite) или файлом `tileset.json`, в программе -
//! с кусками из его папки.
//!
//! Кусок набора - модель с центром в ECEF и смещениями float32, как модели
//! «Моих меток»: мировые координаты в видеокарту не попадают. Сетки
//! ложатся в конец списка моделей, номера моделей меток не сдвигаются.
//! Наборы живут до закрытия программы, в сцену не пишутся.

use eframe::egui;
use globe_core::DVec3;
use globe_core::ellipsoid::Ellipsoid;
use render::{ModelMesh, ModelPart};
use scene::tiles3d::Tileset;

use crate::i18n::{Lang, tr, trf};

/// Набор на глобусе.
pub struct Set {
    pub id: u64,
    pub title: String,
    pub visible: bool,
    pub meshes: Vec<ModelMesh>,
    /// Центры кусков (широта, долгота) - для перелёта к набору.
    pub spots: Vec<(f64, f64)>,
    /// Куски, которые не прочлись.
    pub skipped: usize,
}

/// Что попросило меню строки набора.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Visible(bool),
    Zoom,
    Remove,
}

/// Наборы 3D Tiles. dirty - сетки моделей надо собрать заново.
#[derive(Default)]
pub struct Tilesets {
    pub list: Vec<Set>,
    next: u64,
    pub dirty: bool,
}

/// Сетка куска для видеокарты: центр - середина вершин куска в ECEF,
/// вершины - смещения от него.
fn mesh(parts: &[scene::models::Part], pictures: &scene::models::Pictures, ellipsoid: &Ellipsoid) -> Option<(ModelMesh, (f64, f64))> {
    let count = parts.iter().map(|p| p.positions.len()).sum::<usize>();
    if count == 0 {
        return None;
    }
    let sum = parts.iter().flat_map(|p| p.positions.iter()).fold(DVec3::ZERO, |a, v| a + *v);
    let center = sum / count as f64;
    let (lat, lon, _) = ellipsoid.ecef_to_geodetic(center);
    let f = |v: DVec3| [v.x as f32, v.y as f32, v.z as f32];
    let model_parts = parts
        .iter()
        .map(|p| {
            let normals = p.normals.clone().unwrap_or_else(|| scene::models::flat_normals(&p.positions));
            ModelPart {
                positions: p.positions.iter().map(|v| f(*v - center)).collect(),
                normals: normals.iter().map(|n| f(n.normalize_or_zero())).collect(),
                uv: p.uv.as_ref().map_or_else(Vec::new, |uv| uv.iter().map(|t| [t.x as f32, t.y as f32]).collect()),
                image: p.image.clone(),
                rgba: p.rgba.map(|c| c as f32),
                track: None,
            }
        })
        .collect();
    let images = pictures.iter().filter_map(|(key, bytes)| crate::models::decode(bytes).map(|(w, h, rgba)| (key.clone(), w, h, rgba))).collect();
    Some((ModelMesh { center, parts: model_parts, images, sun: crate::models::sun_at(lat, lon), orient: globe_core::DMat3::IDENTITY }, (lat, lon)))
}

impl Tilesets {
    /// Набор под названием title сверху списка. Возвращает его номер,
    /// None - в наборе нет ни одного прочитанного куска.
    pub fn add(&mut self, set: Tileset, title: &str, ellipsoid: &Ellipsoid) -> Option<u64> {
        let mut meshes = Vec::new();
        let mut spots = Vec::new();
        for piece in &set.pieces {
            if let Some((m, spot)) = mesh(&piece.parts, &piece.pictures, ellipsoid) {
                meshes.push(m);
                spots.push(spot);
            }
        }
        if meshes.is_empty() {
            return None;
        }
        self.next += 1;
        self.list.insert(0, Set { id: self.next, title: title.to_string(), visible: true, meshes, spots, skipped: set.skipped.len() });
        self.dirty = true;
        Some(self.next)
    }

    pub fn get(&self, id: u64) -> Option<&Set> {
        self.list.iter().find(|s| s.id == id)
    }

    /// Применить действие меню к набору id.
    pub fn apply(&mut self, id: u64, act: Act) {
        match act {
            Act::Visible(on) => {
                if let Some(s) = self.list.iter_mut().find(|s| s.id == id) {
                    s.visible = on;
                }
            }
            Act::Remove => self.list.retain(|s| s.id != id),
            Act::Zoom => return,
        }
        self.dirty = true;
    }

    /// Сетки видимых наборов.
    pub fn meshes(&self) -> Vec<ModelMesh> {
        self.list.iter().filter(|s| s.visible).flat_map(|s| s.meshes.iter().cloned()).collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use scene::sceneexport as se;

    #[test]
    fn own_export_becomes_meshes_at_its_place() {
        let e = Ellipsoid::wgs84();
        let center = e.geodetic_to_ecef(58.0, 56.25, 0.0);
        let axes = se::enu_axes(58.0, 56.25);
        let points = [[0.0, 0.0, 30.0], [20.0, 0.0, 0.0], [0.0, 20.0, 0.0]];
        let positions = points.iter().map(|p: &[f64; 3]| [0, 1, 2].map(|k| (p[0] * axes[0][k] + p[1] * axes[1][k] + p[2] * axes[2][k]) as f32)).collect();
        let item = se::Item { center: center.to_array(), positions, colors: Some(vec![[0, 200, 0, 255]; 3]), indices: Some(vec![0, 1, 2]), ..Default::default() };
        let set = scene::tiles3d::read_zip(&se::tiles_zip(&[item]).unwrap()).unwrap();
        let mut sets = Tilesets::default();
        let id = sets.add(set, "город", &e).unwrap();
        assert!(sets.dirty);
        let s = sets.get(id).unwrap();
        // Центр сетки - у точки, смещения маленькие: в видеокарту не идут
        // мировые координаты.
        assert!((s.spots[0].0 - 58.0).abs() < 1e-3 && (s.spots[0].1 - 56.25).abs() < 1e-3, "{:?}", s.spots);
        assert!(s.meshes[0].parts[0].positions.iter().all(|p| p.iter().all(|v| v.abs() < 100.0)));
        assert_eq!(sets.meshes().len(), 1);
        sets.apply(id, Act::Visible(false));
        assert!(sets.meshes().is_empty());
        sets.apply(id, Act::Remove);
        assert!(sets.list.is_empty());
        // Пустой набор не ложится.
        assert!(sets.add(scene::tiles3d::Tileset::default(), "пусто", &e).is_none());
    }
}

/// Строки наборов в разделе «Слои».
pub fn rows(ui: &mut egui::Ui, lang: Lang, sets: &Tilesets, menu_open: &mut Option<String>, out: &mut Vec<(u64, Act)>) {
    for set in &sets.list {
        let tip = if set.skipped > 0 {
            trf(lang, "3D Tiles, pieces on the globe: {count}, not read: {skipped}. The check hides the set.", &[("count", &set.meshes.len().to_string()), ("skipped", &set.skipped.to_string())])
        } else {
            trf(lang, "3D Tiles, pieces on the globe: {count}. The check hides the set.", &[("count", &set.meshes.len().to_string())])
        };
        let title = format!("3D Tiles · {}", set.title);
        let hit = crate::layers::row(ui, 4.0, Some(Some(set.visible)), false, &title, "⋯", &tip);
        if hit.check {
            out.push((set.id, Act::Visible(!set.visible)));
        }
        let key = format!("tiles:{}", set.id);
        if hit.menu || hit.text {
            *menu_open = Some(key.clone());
        }
        crate::layers::popup(ui, menu_open, &key, hit.rect, |ui| {
            if ui.button(tr(lang, "Zoom to layer")).clicked() {
                out.push((set.id, Act::Zoom));
                return true;
            }
            if ui.button(tr(lang, "Remove from the globe")).clicked() {
                out.push((set.id, Act::Remove));
                return true;
            }
            false
        });
    }
}
