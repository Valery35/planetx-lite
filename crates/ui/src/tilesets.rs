// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Наборы 3D Tiles на глобусе: строки раздела «Слои» и подгрузка кусков
//! по мере приближения. Дерево и выбор тайлов - `scene::tiles3d`. Набор
//! открывается архивом ZIP (так его выгружают модуль и Lite) или файлом
//! `tileset.json`, в программе - с кусками из его папки.
//!
//! Каждый кадр [`Tilesets::update`] выбирает тайлы по экранной ошибке
//! с порогом [`THRESHOLD`] пикселей и читает нужные куски, пока не
//! истечёт [`LOAD_TIME`] кадра - хотя бы один кусок за кадр. Кусок ложится
//! в видеокарту под своим ключом и остаётся там, пока его не вытеснит
//! предел [`MEMORY`]: первыми уходят куски, которые дольше всего не
//! рисовались. Кусок, который не прочёлся, считается готовым и пустым -
//! выбор не ждёт его вечно.
//!
//! Кусок набора - модель с центром в ECEF и смещениями float32, как модели
//! «Моих меток»: мировые координаты в видеокарту не попадают. Наборы
//! живут до закрытия программы, в сцену не пишутся.

use std::collections::{HashMap, HashSet};

use eframe::egui;
use globe_core::DVec3;
use globe_core::camera::Camera;
use globe_core::ellipsoid::Ellipsoid;
use render::{Globe, ModelMesh, ModelPart};
use scene::tiles3d::{Tree, View, ZipSet};

use crate::i18n::{Lang, tr, trf};

/// Порог экранной ошибки тайла в пикселях: выше - тайл уточняется.
pub const THRESHOLD: f64 = 16.0;
/// Время кадра на чтение кусков.
pub const LOAD_TIME: web_time::Duration = web_time::Duration::from_millis(8);
/// Предел памяти кусков в видеокарте, байты.
pub const MEMORY: usize = 512 << 20;

/// Откуда берутся файлы набора: архив в памяти или папка `tileset.json`.
pub enum Source {
    Zip(ZipSet),
    Dir(std::path::PathBuf),
}

impl Source {
    /// Файл по пути от папки набора.
    pub fn get(&self, path: &str) -> Option<Vec<u8>> {
        match self {
            Source::Zip(set) => set.get(path),
            Source::Dir(dir) => std::fs::read(dir.join(path)).ok(),
        }
    }
}

/// Прочитанный кусок: сетка, её размер в байтах и номер кадра, когда
/// кусок рисовался последний раз.
struct Loaded {
    mesh: ModelMesh,
    bytes: usize,
    used: u64,
}

/// Набор на глобусе.
pub struct Set {
    pub id: u64,
    pub title: String,
    pub visible: bool,
    tree: Tree,
    source: Source,
    loaded: HashMap<usize, Loaded>,
    /// Тайлы, куски которых не прочлись.
    failed: HashSet<usize>,
    /// Тайлы, которые рисуются сейчас.
    shown: HashSet<usize>,
    /// Края набора (широта, долгота) - для перелёта к нему.
    pub spots: Vec<(f64, f64)>,
}

impl Set {
    /// Количество кусков на экране.
    pub fn shown(&self) -> usize {
        self.shown.len()
    }

    /// Количество кусков и внешних наборов, которые не прочлись.
    pub fn skipped(&self) -> usize {
        self.failed.len() + self.tree.skipped.len()
    }
}

/// Что попросило меню строки набора.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Visible(bool),
    Zoom,
    Remove,
}

/// Наборы 3D Tiles и счётчик кадров для вытеснения.
#[derive(Default)]
pub struct Tilesets {
    pub list: Vec<Set>,
    next: u64,
    frame: u64,
}

/// Ключ куска в видеокарте: номер набора и номер тайла.
fn key(set: u64, node: usize) -> u64 {
    set << 32 | node as u64
}

/// Сетка куска для видеокарты: центр - середина вершин куска в ECEF,
/// вершины - смещения от него.
fn mesh(parts: &[scene::models::Part], pictures: &scene::models::Pictures, ellipsoid: &Ellipsoid) -> Option<ModelMesh> {
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
    Some(ModelMesh { center, parts: model_parts, images, sun: crate::models::sun_at(lat, lon), orient: globe_core::DMat3::IDENTITY })
}

/// Байты сетки в видеокарте: вершины по 32 байта и картинки.
fn mesh_bytes(mesh: &ModelMesh) -> usize {
    mesh.parts.iter().map(|p| p.positions.len() * 32).sum::<usize>() + mesh.images.iter().map(|i| i.3.len()).sum::<usize>()
}

/// Края набора: центр сферы первого тайла с мерой и точки на её радиусе
/// к востоку, западу, северу и югу.
fn spots(tree: &Tree, ellipsoid: &Ellipsoid) -> Vec<(f64, f64)> {
    let Some(node) = tree.nodes.iter().find(|n| n.radius.is_finite() && n.center.length() > 0.0) else {
        return Vec::new();
    };
    let (lat, lon, _) = ellipsoid.ecef_to_geodetic(node.center);
    let [east, north, _] = globe_core::camera::enu(lat, lon);
    [DVec3::ZERO, east, -east, north, -north]
        .iter()
        .map(|axis| {
            let (lat, lon, _) = ellipsoid.ecef_to_geodetic(node.center + *axis * node.radius);
            (lat, lon)
        })
        .collect()
}

/// Видна ли сфера камере: не за спиной и не за боковыми гранями
/// пирамиды взгляда.
fn visible(camera: &Camera, center: DVec3, radius: f64) -> bool {
    if !radius.is_finite() {
        return true;
    }
    let p = camera.rotation.transpose() * (center - camera.eye);
    let depth = -p.z;
    if depth < -radius {
        return false;
    }
    let half_v = camera.fov_y.to_radians() / 2.0;
    let half_h = (half_v.tan() * camera.aspect()).atan();
    let side = |offset: f64, half: f64| offset * half.cos() - depth * half.sin() <= radius;
    side(p.y, half_v) && side(-p.y, half_v) && side(p.x, half_h) && side(-p.x, half_h)
}

impl Tilesets {
    /// Набор под названием title сверху списка. Возвращает его номер,
    /// None - в наборе нет ни одного тайла с куском.
    pub fn add(&mut self, tree: Tree, source: Source, title: &str, ellipsoid: &Ellipsoid) -> Option<u64> {
        if !tree.nodes.iter().any(|n| n.uri.is_some()) {
            return None;
        }
        self.next += 1;
        let spots = spots(&tree, ellipsoid);
        self.list.insert(
            0,
            Set {
                id: self.next,
                title: title.to_string(),
                visible: true,
                tree,
                source,
                loaded: HashMap::new(),
                failed: HashSet::new(),
                shown: HashSet::new(),
                spots,
            },
        );
        Some(self.next)
    }

    pub fn get(&self, id: u64) -> Option<&Set> {
        self.list.iter().find(|s| s.id == id)
    }

    /// Применить действие меню к набору id. Куски убранного набора
    /// уходят из видеокарты в [`Tilesets::update`].
    pub fn apply(&mut self, id: u64, act: Act) {
        match act {
            Act::Visible(on) => {
                if let Some(s) = self.list.iter_mut().find(|s| s.id == id) {
                    s.visible = on;
                }
            }
            Act::Remove => self.list.retain(|s| s.id != id),
            Act::Zoom => {}
        }
    }

    /// Выбрать тайлы на кадр, прочитать нужные куски и вытеснить лишние.
    /// Возвращает true, пока нужные куски не все прочитаны: нужен ещё кадр.
    pub fn update(&mut self, globe: &mut Globe, camera: &Camera) -> bool {
        self.frame += 1;
        let frame = self.frame;
        let start = web_time::Instant::now();
        let mut read = 0usize;
        // Куски убранных наборов.
        for k in globe.tile_keys() {
            if !self.list.iter().any(|s| s.id == k >> 32) {
                globe.remove_tile(k);
            }
        }
        let test = |c: DVec3, r: f64| visible(camera, c, r);
        let view = View { eye: camera.eye, pixels: f64::from(camera.height.max(1)) / (2.0 * (camera.fov_y.to_radians() / 2.0).tan()), visible: &test };
        let mut busy = false;
        for set in &mut self.list {
            if !set.visible {
                for &n in set.loaded.keys() {
                    globe.show_tile(key(set.id, n), false);
                }
                set.shown.clear();
                continue;
            }
            let selection = set.tree.select(&view, THRESHOLD, &|n| set.loaded.contains_key(&n) || set.failed.contains(&n));
            for &(n, _) in &selection.load {
                if read > 0 && start.elapsed() > LOAD_TIME {
                    busy = true;
                    break;
                }
                read += 1;
                let source = &set.source;
                let piece = set.tree.load(n, &|p| source.get(p));
                match piece.ok().and_then(|p| mesh(&p.parts, &p.pictures, &camera.ellipsoid)) {
                    Some(m) => {
                        globe.add_tile(key(set.id, n), &m);
                        set.loaded.insert(n, Loaded { bytes: mesh_bytes(&m), mesh: m, used: frame });
                    }
                    None => {
                        set.failed.insert(n);
                    }
                }
            }
            set.shown = selection.show.into_iter().collect();
            for (&n, l) in &mut set.loaded {
                let on = set.shown.contains(&n);
                if on {
                    l.used = frame;
                }
                globe.show_tile(key(set.id, n), on);
            }
        }
        self.evict(globe);
        busy || read > 0
    }

    /// Вытеснить куски сверх [`MEMORY`]: первыми - те, что дольше всего
    /// не рисовались. Куски на экране остаются.
    fn evict(&mut self, globe: &mut Globe) {
        let total: usize = self.list.iter().flat_map(|s| s.loaded.values()).map(|l| l.bytes).sum();
        if total <= MEMORY {
            return;
        }
        let mut idle: Vec<(u64, u64, usize, usize)> =
            self.list.iter().flat_map(|s| s.loaded.iter().filter(|(n, _)| !s.shown.contains(n)).map(move |(&n, l)| (l.used, s.id, n, l.bytes))).collect();
        idle.sort_unstable();
        let mut over = total - MEMORY;
        for (_, id, n, bytes) in idle {
            if over == 0 {
                break;
            }
            if let Some(s) = self.list.iter_mut().find(|s| s.id == id) {
                s.loaded.remove(&n);
            }
            globe.remove_tile(key(id, n));
            over = over.saturating_sub(bytes);
        }
    }

    /// Сетки кусков на экране у видимых наборов - для выгрузки вида.
    pub fn meshes(&self) -> Vec<ModelMesh> {
        self.list
            .iter()
            .filter(|s| s.visible)
            .flat_map(|s| s.loaded.iter().filter(|(n, _)| s.shown.contains(n)).map(|(_, l)| l.mesh.clone()))
            .collect()
    }
}

/// Строки наборов в разделе «Слои».
pub fn rows(ui: &mut egui::Ui, lang: Lang, sets: &Tilesets, menu_open: &mut Option<String>, out: &mut Vec<(u64, Act)>) {
    for set in &sets.list {
        let count = set.shown().to_string();
        let skipped = set.skipped();
        let tip = if skipped > 0 {
            trf(lang, "3D Tiles, pieces on the globe: {count}, not read: {skipped}. The check hides the set.", &[("count", &count), ("skipped", &skipped.to_string())])
        } else {
            trf(lang, "3D Tiles, pieces on the globe: {count}. The check hides the set.", &[("count", &count)])
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use scene::sceneexport as se;

    #[test]
    fn own_export_opens_as_a_set_at_its_place() {
        let e = Ellipsoid::wgs84();
        let center = e.geodetic_to_ecef(58.0, 56.25, 0.0);
        let axes = se::enu_axes(58.0, 56.25);
        let points = [[0.0, 0.0, 30.0], [20.0, 0.0, 0.0], [0.0, 20.0, 0.0]];
        let positions = points.iter().map(|p: &[f64; 3]| [0, 1, 2].map(|k| (p[0] * axes[0][k] + p[1] * axes[1][k] + p[2] * axes[2][k]) as f32)).collect();
        let item = se::Item { center: center.to_array(), positions, colors: Some(vec![[0, 200, 0, 255]; 3]), indices: Some(vec![0, 1, 2]), ..Default::default() };
        let zip = ZipSet::open(&se::tiles_zip(&[item]).unwrap()).unwrap();
        let tree = Tree::open(&zip.root, &|p| zip.get(p)).unwrap();
        let mut sets = Tilesets::default();
        let id = sets.add(tree, Source::Zip(zip), "город", &e).unwrap();
        let s = sets.get(id).unwrap();
        // Края набора - у точки выгрузки.
        assert!((s.spots[0].0 - 58.0).abs() < 1e-2 && (s.spots[0].1 - 56.25).abs() < 1e-2, "{:?}", s.spots);
        // Кусок читается, его центр - у точки, смещения маленькие: в
        // видеокарту не идут мировые координаты.
        let n = s.tree.nodes.iter().position(|n| n.uri.is_some()).unwrap();
        let piece = s.tree.load(n, &|p| s.source.get(p)).unwrap();
        let m = mesh(&piece.parts, &piece.pictures, &e).unwrap();
        assert!(m.center.distance(center) < 100.0);
        assert!(m.parts[0].positions.iter().all(|p| p.iter().all(|v| v.abs() < 100.0)));
        sets.apply(id, Act::Visible(false));
        assert!(!sets.get(id).unwrap().visible);
        sets.apply(id, Act::Remove);
        assert!(sets.list.is_empty());
        // Набор без кусков не ложится.
        let empty = Tree::open(br#"{"asset":{"version":"1.0"},"root":{"geometricError":0}}"#, &|_| None).unwrap();
        assert!(sets.add(empty, Source::Dir(std::path::PathBuf::new()), "пусто", &e).is_none());
    }

    #[test]
    fn sphere_behind_the_eye_is_not_visible() {
        let e = Ellipsoid::wgs84();
        let camera = Camera::look_at(e, 58.0, 56.25, 1000.0, 0.0, 0.0, 0.0);
        let ahead = e.geodetic_to_ecef(58.0, 56.25, 0.0);
        assert!(visible(&camera, ahead, 10.0));
        let behind = camera.eye + (camera.eye - ahead);
        assert!(!visible(&camera, behind, 10.0));
        // Сбоку далеко за краем взгляда - не видна, с большим радиусом - видна.
        let aside = camera.eye + camera.rotation.x_axis * 10_000.0 - camera.rotation.z_axis * 100.0;
        assert!(!visible(&camera, aside, 10.0));
        assert!(visible(&camera, aside, 20_000.0));
    }
}
