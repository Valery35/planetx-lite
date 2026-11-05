// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Наборы 3D Tiles на глобусе: строки раздела «Слои» и подгрузка кусков
//! по мере приближения. Дерево и выбор тайлов - `scene::tiles3d`. Набор
//! открывается архивом ZIP (так его выгружают модуль и Lite), файлом
//! `tileset.json` - в программе с кусками из его папки - или ссылкой на
//! `tileset.json`. По ссылке куски и внешние наборы приходят запросами,
//! внешний набор подшивается к дереву, когда до него дошёл выбор.
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
//! «Моих меток»: мировые координаты в видеокарту не попадают.
//!
//! Наборы живут между запусками, как «Мои слои»: строка хранилища
//! ([`KEY`], [`Tilesets::index`]) помнит путь файла или ссылку и флажок
//! набора, при запуске набор открывается снова. Набор, открытый на
//! странице из файла, пути не имеет и не запоминается. В сцену наборы
//! не пишутся.

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

/// Запросов к набору по ссылке сразу, не больше.
pub const REQUESTS: usize = 6;
/// Предел ответа на запрос куска, байты.
const ANSWER_LIMIT: u64 = 64 << 20;

/// Откуда берутся файлы набора: архив в памяти, папка `tileset.json` или
/// ссылка - папка набора на сервере и запрос после «?», он идёт с каждым
/// файлом набора.
pub enum Source {
    Zip(ZipSet),
    Dir(std::path::PathBuf),
    Url { base: String, query: String },
}

impl Source {
    /// Набор по ссылке на его `tileset.json`.
    pub fn url(link: &str) -> Source {
        let link = link.split('#').next().unwrap_or(link);
        let (path, query) = link.split_once('?').unwrap_or((link, ""));
        let base = path.rfind('/').map_or("", |at| &path[..=at]);
        Source::Url { base: base.to_string(), query: query.to_string() }
    }

    /// Файл по пути от папки набора. По ссылке файл так не взять - None.
    pub fn get(&self, path: &str) -> Option<Vec<u8>> {
        match self {
            Source::Zip(set) => set.get(path),
            Source::Dir(dir) => std::fs::read(dir.join(scene::tiles3d::path_of(path))).ok(),
            Source::Url { .. } => None,
        }
    }

    /// Адрес файла набора по ссылке, у архива и папки - None. Запрос набора
    /// дописывается к адресу без своего запроса и к адресу со своим.
    pub fn address(&self, path: &str) -> Option<String> {
        let Source::Url { base, query } = self else {
            return None;
        };
        let mut url = if path.contains("://") { path.to_string() } else { format!("{base}{path}") };
        if !query.is_empty() && !url.contains(query.as_str()) {
            url.push(if url.contains('?') { '&' } else { '?' });
            url.push_str(query);
        }
        Some(url)
    }
}

/// Ключ списка наборов в хранилище программы.
pub const KEY: &str = "tilesets";

/// Откуда набор открыт - по этому он открывается снова при запуске.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// Файл на диске: архив ZIP или `tileset.json`.
    File(String),
    /// Ссылка на архив или `tileset.json`.
    Link(String),
}

impl Origin {
    /// Путь или адрес.
    pub fn text(&self) -> &str {
        match self {
            Origin::File(t) | Origin::Link(t) => t,
        }
    }
}

/// Список наборов прошлого запуска из строки хранилища: откуда набор
/// и виден ли он. Строки без пути и адреса пропускаются.
pub fn parse_index(text: &str) -> Vec<(Origin, bool)> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let get = |key: &str| item.get(key).and_then(serde_json::Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
            let origin = get("file").map(Origin::File).or_else(|| get("link").map(Origin::Link))?;
            Some((origin, item.get("visible").and_then(serde_json::Value::as_bool).unwrap_or(true)))
        })
        .collect()
}

/// Ссылка ли текст: адрес http или https без пробелов.
pub fn is_link(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://")) && !text.contains(char::is_whitespace) && text.len() > "https://".len()
}

/// Имя папки набора по ссылке на его `tileset.json` - название набора.
pub fn link_folder(link: &str) -> Option<String> {
    let path = scene::tiles3d::path_of(link.split('#').next().unwrap_or(link));
    let path = path.split_once("://").map_or(path, |(_, rest)| rest);
    let mut parts = path.split('/').filter(|p| !p.is_empty());
    let _file = parts.next_back();
    parts.next_back().map(str::to_string)
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
    /// Запросы по ссылке, которые идут: тайл и запрос.
    pending: Vec<(usize, Box<dyn net::Request>)>,
    /// Края набора (широта, долгота) - для перелёта к нему.
    pub spots: Vec<(f64, f64)>,
    /// Откуда набор открыт. None - из памяти, при запуске не открывается.
    pub origin: Option<Origin>,
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

    /// Разобрать байты файла тайла n: внешний набор подшивается к дереву,
    /// кусок ложится в видеокарту. Не прочёлся - тайл в «не прочлись».
    fn receive(&mut self, n: usize, bytes: &[u8], globe: &mut Globe, ellipsoid: &Ellipsoid, frame: u64) {
        if self.tree.is_external(n) {
            if self.tree.graft(n, bytes).is_err() {
                self.failed.insert(n);
            }
            return;
        }
        let source = &self.source;
        let piece = self.tree.piece(n, bytes, &|p| source.get(p));
        match piece.ok().and_then(|p| mesh(&p.parts, &p.pictures, ellipsoid)) {
            Some(m) => {
                globe.add_tile(key(self.id, n), &m);
                self.loaded.insert(n, Loaded { bytes: mesh_bytes(&m), mesh: m, used: frame });
            }
            None => {
                self.failed.insert(n);
            }
        }
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
    /// Список изменился - записать его в хранилище.
    pub save_dirty: bool,
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
    pub fn add(&mut self, tree: Tree, source: Source, title: &str, origin: Option<Origin>, ellipsoid: &Ellipsoid) -> Option<u64> {
        if !tree.nodes.iter().any(|n| n.uri.is_some()) {
            return None;
        }
        // Тот же файл или ссылка второй раз - прежний набор уходит.
        if let Some(o) = &origin {
            self.list.retain(|s| s.origin.as_ref() != Some(o));
        }
        self.save_dirty = true;
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
                pending: Vec::new(),
                spots,
                origin,
            },
        );
        Some(self.next)
    }

    /// Идут ли запросы кусков по ссылке.
    pub fn busy(&self) -> bool {
        self.list.iter().any(|s| s.visible && !s.pending.is_empty())
    }

    pub fn get(&self, id: u64) -> Option<&Set> {
        self.list.iter().find(|s| s.id == id)
    }

    /// Применить действие меню к набору id. Куски убранного набора
    /// уходят из видеокарты в [`Tilesets::update`].
    pub fn apply(&mut self, id: u64, act: Act) {
        if act != Act::Zoom {
            self.save_dirty = true;
        }
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
    /// Набор по ссылке заказывает куски запросами request, не больше
    /// [`REQUESTS`] сразу, и разбирает пришедшие ответы. Возвращает true,
    /// пока нужные куски не все прочитаны: нужен ещё кадр.
    pub fn update(&mut self, globe: &mut Globe, camera: &Camera, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> bool {
        self.frame += 1;
        let frame = self.frame;
        let start = web_time::Instant::now();
        let mut read = 0usize;
        let ellipsoid = &camera.ellipsoid;
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
            // Пришедшие ответы на запросы по ссылке.
            let mut i = 0;
            while i < set.pending.len() {
                if read > 0 && start.elapsed() > LOAD_TIME {
                    busy = true;
                    break;
                }
                let Some(answer) = set.pending[i].1.take() else {
                    i += 1;
                    continue;
                };
                let (n, _) = set.pending.swap_remove(i);
                read += 1;
                match answer {
                    Ok(bytes) => set.receive(n, &bytes, globe, ellipsoid, frame),
                    Err(_) => {
                        set.failed.insert(n);
                    }
                }
            }
            let selection = set.tree.select(&view, THRESHOLD, &|n| set.loaded.contains_key(&n) || set.failed.contains(&n));
            for &(n, _) in &selection.load {
                if let Source::Url { .. } = set.source {
                    if set.pending.iter().any(|(p, _)| *p == n) {
                        continue;
                    }
                    if set.pending.len() >= REQUESTS {
                        break;
                    }
                    match set.tree.nodes.get(n).and_then(|node| node.uri.as_deref()).and_then(|u| set.source.address(u)) {
                        Some(address) => {
                            let r = request(ANSWER_LIMIT);
                            r.start(&address);
                            set.pending.push((n, r));
                        }
                        None => {
                            set.failed.insert(n);
                        }
                    }
                    continue;
                }
                if read > 0 && start.elapsed() > LOAD_TIME {
                    busy = true;
                    break;
                }
                read += 1;
                let uri = set.tree.nodes.get(n).and_then(|node| node.uri.clone()).unwrap_or_default();
                match set.source.get(&uri) {
                    Some(bytes) => set.receive(n, &bytes, globe, ellipsoid, frame),
                    None => {
                        set.failed.insert(n);
                    }
                }
            }
            busy |= !set.pending.is_empty();
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

    /// Список наборов для хранилища, снизу вверх: при запуске они
    /// открываются по порядку и встают как были. Набор из памяти в список
    /// не входит.
    pub fn index(&self) -> String {
        let items: Vec<serde_json::Value> = self
            .list
            .iter()
            .rev()
            .filter_map(|s| {
                let (key, text) = match s.origin.as_ref()? {
                    Origin::File(t) => ("file", t),
                    Origin::Link(t) => ("link", t),
                };
                Some(serde_json::json!({ key: text, "visible": s.visible }))
            })
            .collect();
        serde_json::Value::Array(items).to_string()
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
        let id = sets.add(tree, Source::Zip(zip), "город", None, &e).unwrap();
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
        assert!(sets.add(empty, Source::Dir(std::path::PathBuf::new()), "пусто", None, &e).is_none());
    }

    #[test]
    fn sets_are_remembered_by_path_and_link() {
        let e = Ellipsoid::wgs84();
        let tree = || Tree::open_lazy(br#"{"asset":{"version":"1.0"},"root":{"geometricError":1,"content":{"uri":"a.glb"}}}"#).unwrap();
        let mut sets = Tilesets::default();
        let file = Origin::File("C:/data/city.zip".into());
        let link = Origin::Link("https://h.org/t/tileset.json".into());
        sets.add(tree(), Source::Dir(std::path::PathBuf::new()), "город", Some(file.clone()), &e).unwrap();
        let id = sets.add(tree(), Source::url(link.text()), "t", Some(link.clone()), &e).unwrap();
        sets.add(tree(), Source::Dir(std::path::PathBuf::new()), "память", None, &e).unwrap();
        sets.apply(id, Act::Visible(false));
        assert!(sets.save_dirty);
        // Снизу вверх, без набора из памяти, флажок сохраняется.
        assert_eq!(parse_index(&sets.index()), vec![(file.clone(), true), (link.clone(), false)]);
        // Тот же файл второй раз - один набор.
        sets.add(tree(), Source::Dir(std::path::PathBuf::new()), "город", Some(file.clone()), &e).unwrap();
        assert_eq!(sets.list.iter().filter(|s| s.origin.as_ref() == Some(&file)).count(), 1);
        // Испорченная строка - пустой список.
        assert!(parse_index("{").is_empty() && parse_index(r#"[{"file":""},{"x":1}]"#).is_empty());
    }

    #[test]
    fn link_addresses_carry_the_query_of_the_set() {
        let s = Source::url("https://h.org/city/tileset.json?key=abc#top");
        assert_eq!(s.address("tiles/0.b3dm").as_deref(), Some("https://h.org/city/tiles/0.b3dm?key=abc"));
        assert_eq!(s.address("tiles/0.b3dm?v=1").as_deref(), Some("https://h.org/city/tiles/0.b3dm?v=1&key=abc"));
        assert_eq!(s.address("https://cdn.org/a.glb?key=abc").as_deref(), Some("https://cdn.org/a.glb?key=abc"));
        assert!(s.get("tiles/0.b3dm").is_none());
        let plain = Source::url("https://h.org/tileset.json");
        assert_eq!(plain.address("a.glb").as_deref(), Some("https://h.org/a.glb"));
        assert!(Source::Dir(std::path::PathBuf::new()).address("a.glb").is_none());
        assert!(is_link(" https://h.org/tileset.json".trim()) && is_link("HTTP://h.org/a"));
        assert!(!is_link("https://") && !is_link("Москва") && !is_link("https://h.org/a b"));
        assert_eq!(link_folder("https://h.org/city/tileset.json?key=1").as_deref(), Some("city"));
        assert_eq!(link_folder("https://h.org/tileset.json").as_deref(), Some("h.org"));
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
