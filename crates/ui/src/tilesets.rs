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

/// Ссылка на сервис или слой `SceneServer` с ответом в JSON: без `f=json`
/// служба ArcGIS отдаёт страницу HTML. Прочие ссылки - как есть.
pub fn scene_server_link(url: &str) -> String {
    if !url.contains("/SceneServer") || url.contains("f=") || url.contains("/nodepages") {
        return url.to_string();
    }
    let (path, query) = url.split_once('?').map_or((url, ""), |(p, q)| (p, q));
    let path = path.trim_end_matches('/');
    if query.is_empty() { format!("{path}?f=json") } else { format!("{path}?{query}&f=json") }
}

/// Ссылка на слой number сервиса `SceneServer` по ссылке на сервис.
pub fn scene_server_layer(service: &str, number: usize) -> String {
    let (path, query) = service.split_once('?').map_or((service, ""), |(p, q)| (p, q));
    let link = format!("{}/layers/{number}", path.trim_end_matches('/'));
    scene_server_link(&if query.is_empty() { link } else { format!("{link}?{query}") })
}

/// Источник файлов слоя I3S: папка слоя и запрос без `f=json`, ключ
/// доступа `token` идёт с каждым файлом.
pub fn scene_server_source(layer: &str) -> Source {
    let layer = layer.split('#').next().unwrap_or(layer);
    let (path, query) = layer.split_once('?').map_or((layer, ""), |(p, q)| (p, q));
    let query: Vec<&str> = query.split('&').filter(|p| !p.is_empty() && !p.starts_with("f=")).collect();
    Source::Url { base: format!("{}/", path.trim_end_matches('/')), query: query.join("&") }
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

/// Слой сцены I3S: узлы страниц, запросы страниц и текстур. Геометрия
/// узла приходит обычным запросом куска, текстура - вслед за ней.
pub struct I3s {
    index: scene::i3s::Index,
    pages: Vec<(usize, Box<dyn net::Request>)>,
    /// Тайл, байты его геометрии и запрос текстуры.
    textures: Vec<(usize, Vec<u8>, Box<dyn net::Request>)>,
    /// Страницы, которые не прочлись.
    bad_pages: usize,
}

impl I3s {
    pub fn new(index: scene::i3s::Index) -> Self {
        Self { index, pages: Vec::new(), textures: Vec::new(), bad_pages: 0 }
    }
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
    /// Слой I3S, у набора 3D Tiles - None.
    i3s: Option<I3s>,
}

impl Set {
    /// Количество кусков на экране.
    pub fn shown(&self) -> usize {
        self.shown.len()
    }

    /// Количество кусков и внешних наборов, которые не прочлись.
    pub fn skipped(&self) -> usize {
        self.failed.len() + self.tree.skipped.len() + self.i3s.as_ref().map_or(0, |l| l.bad_pages)
    }

    /// Кусок в видеокарту, не прочёлся - тайл в «не прочлись».
    fn place(&mut self, n: usize, piece: Option<scene::tiles3d::Piece>, globe: &mut Globe, ellipsoid: &Ellipsoid, frame: u64) {
        match piece.and_then(|p| mesh(&p.parts, &p.pictures, ellipsoid)) {
            Some(m) => {
                globe.add_tile(key(self.id, n), &m);
                self.loaded.insert(n, Loaded { bytes: mesh_bytes(&m), mesh: m, used: frame });
            }
            None => {
                self.failed.insert(n);
            }
        }
    }

    /// Разобрать байты файла тайла n: внешний набор подшивается к дереву,
    /// кусок ложится в видеокарту. Не прочёлся - тайл в «не прочлись».
    /// У слоя I3S это геометрия: за ней просится текстура узла.
    fn receive(&mut self, n: usize, bytes: &[u8], globe: &mut Globe, ellipsoid: &Ellipsoid, frame: u64, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        if let Some(layer) = &mut self.i3s {
            match layer.index.texture_path(n).and_then(|p| self.source.address(&p)) {
                Some(address) => {
                    let r = request(ANSWER_LIMIT);
                    r.start(&address);
                    layer.textures.push((n, bytes.to_vec(), r));
                }
                None => {
                    let piece = layer.index.piece(n, bytes, None).ok();
                    self.place(n, piece, globe, ellipsoid, frame);
                }
            }
            return;
        }
        if self.tree.is_external(n) {
            if self.tree.graft(n, bytes).is_err() {
                self.failed.insert(n);
            }
            return;
        }
        let source = &self.source;
        let piece = self.tree.piece(n, bytes, &|p| source.get(p)).ok();
        self.place(n, piece, globe, ellipsoid, frame);
    }

    /// Шаг слоя I3S: пришедшие страницы ложатся в дерево, пришедшие
    /// текстуры - в куски, нужные виду страницы просятся.
    fn step_i3s(&mut self, view: &View<'_>, globe: &mut Globe, ellipsoid: &Ellipsoid, frame: u64, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        let Some(layer) = &mut self.i3s else {
            return;
        };
        let mut i = 0;
        while i < layer.pages.len() {
            let Some(answer) = layer.pages[i].1.take() else {
                i += 1;
                continue;
            };
            let (page, _) = layer.pages.swap_remove(i);
            let added = answer.map_err(|_| ()).and_then(|bytes| layer.index.add_page(&mut self.tree, page, &bytes).map_err(|_| ()));
            if added.is_err() {
                layer.bad_pages += 1;
            }
        }
        let mut done = Vec::new();
        let mut i = 0;
        while i < layer.textures.len() {
            let Some(answer) = layer.textures[i].2.take() else {
                i += 1;
                continue;
            };
            let (n, geometry, _) = layer.textures.swap_remove(i);
            // Текстура не пришла - сетка ложится цветом материала.
            done.push((n, layer.index.piece(n, &geometry, answer.ok()).ok()));
        }
        let room = REQUESTS.saturating_sub(layer.pages.len());
        for page in layer.index.wanted(&self.tree, view, room) {
            match self.source.address(&scene::i3s::Index::page_path(page)) {
                Some(address) => {
                    let r = request(ANSWER_LIMIT);
                    r.start(&address);
                    layer.pages.push((page, r));
                }
                None => layer.bad_pages += 1,
            }
        }
        for (n, piece) in done {
            self.place(n, piece, globe, ellipsoid, frame);
        }
    }

    /// Идут ли запросы набора.
    fn waiting(&self) -> bool {
        !self.pending.is_empty() || self.i3s.as_ref().is_some_and(|l| !l.pages.is_empty() || !l.textures.is_empty())
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
                i3s: None,
            },
        );
        Some(self.next)
    }

    /// Слой сцены I3S под названием title сверху списка. Дерево пусто,
    /// узлы приходят страницами. spots - края охвата слоя. Возвращает
    /// номер набора.
    pub fn add_i3s(&mut self, layer: I3s, source: Source, title: &str, origin: Option<Origin>, spots: Vec<(f64, f64)>) -> u64 {
        if let Some(o) = &origin {
            self.list.retain(|s| s.origin.as_ref() != Some(o));
        }
        self.save_dirty = true;
        self.next += 1;
        self.list.insert(
            0,
            Set {
                id: self.next,
                title: title.to_string(),
                visible: true,
                tree: Tree::default(),
                source,
                loaded: HashMap::new(),
                failed: HashSet::new(),
                shown: HashSet::new(),
                pending: Vec::new(),
                spots,
                origin,
                i3s: Some(layer),
            },
        );
        self.next
    }

    /// Идут ли запросы кусков по ссылке.
    pub fn busy(&self) -> bool {
        self.list.iter().any(|s| s.visible && s.waiting())
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
                    Ok(bytes) => set.receive(n, &bytes, globe, ellipsoid, frame, request),
                    Err(_) => {
                        set.failed.insert(n);
                    }
                }
            }
            set.step_i3s(&view, globe, ellipsoid, frame, request);
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
                    Some(bytes) => set.receive(n, &bytes, globe, ellipsoid, frame, request),
                    None => {
                        set.failed.insert(n);
                    }
                }
            }
            busy |= set.waiting();
            set.shown = selection.show.into_iter().collect();
            // Готовые дети показанного тайла ждут соседей: вытесни их -
            // и тяжёлый слой грузится по кругу, не дойдя до детей.
            let waiting: HashSet<usize> = set.shown.iter().filter_map(|&n| set.tree.nodes.get(n)).flat_map(|node| node.children.iter().copied()).collect();
            for (&n, l) in &mut set.loaded {
                let on = set.shown.contains(&n);
                if on || waiting.contains(&n) {
                    l.used = frame;
                }
                globe.show_tile(key(set.id, n), on);
            }
        }
        self.evict(globe);
        busy || read > 0
    }

    /// Вытеснить куски сверх [`MEMORY`]: первыми - те, что дольше всего
    /// не рисовались. Куски на экране и готовые дети показанных тайлов
    /// остаются.
    fn evict(&mut self, globe: &mut Globe) {
        let frame = self.frame;
        let total: usize = self.list.iter().flat_map(|s| s.loaded.values()).map(|l| l.bytes).sum();
        if total <= MEMORY {
            return;
        }
        let mut idle: Vec<(u64, u64, usize, usize)> =
            self.list.iter().flat_map(|s| s.loaded.iter().filter(|(n, l)| !s.shown.contains(n) && l.used < frame).map(move |(&n, l)| (l.used, s.id, n, l.bytes))).collect();
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

    #[test]
    fn scene_server_links_answer_json() {
        let svc = "https://tiles.arcgis.com/tiles/x/arcgis/rest/services/SF/SceneServer";
        assert_eq!(scene_server_link(svc), format!("{svc}?f=json"));
        assert_eq!(scene_server_link(&format!("{svc}/?token=t")), format!("{svc}?token=t&f=json"));
        assert_eq!(scene_server_link(&format!("{svc}/layers/0?f=json")), format!("{svc}/layers/0?f=json"));
        assert_eq!(scene_server_link("https://h.org/tileset.json"), "https://h.org/tileset.json");
        assert_eq!(scene_server_layer(&format!("{svc}?f=json"), 2), format!("{svc}/layers/2?f=json"));
        let source = scene_server_source(&format!("{svc}/layers/0?token=t&f=json"));
        assert_eq!(source.address("nodepages/3").as_deref(), Some(format!("{svc}/layers/0/nodepages/3?token=t").as_str()));
        let plain = scene_server_source(&format!("{svc}/layers/0?f=json"));
        assert_eq!(plain.address("nodes/5/geometries/1").as_deref(), Some(format!("{svc}/layers/0/nodes/5/geometries/1").as_str()));
    }

    /// Живая проверка слоя I3S Сан-Франциско: страницы до узлов с сеткой,
    /// геометрия Draco и текстура одного узла.
    /// cargo test -p ui live_i3s -- --ignored --nocapture
    #[test]
    #[ignore]
    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
    fn live_i3s() {
        let get = |url: &str| -> Vec<u8> {
            let r = net::native::Requests::with_limit(std::sync::Arc::new(|| {}), 64 << 20);
            net::Request::start(&r, url);
            let start = std::time::Instant::now();
            loop {
                if let Some(answer) = net::Request::take(&r) {
                    return answer.unwrap_or_else(|e| panic!("{url}: {e}"));
                }
                assert!(start.elapsed().as_secs() < 60, "{url}: долго");
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        };
        let link = scene_server_link("https://tiles.arcgis.com/tiles/z2tnIkrLQ2BRzr6P/arcgis/rest/services/SanFrancisco_Bldgs/SceneServer/layers/0");
        let layer = scene::i3s::layer(&get(&link)).unwrap();
        let source = scene_server_source(&link);
        let e = Ellipsoid::wgs84();
        let mut index = scene::i3s::Index::new(layer, e, THRESHOLD);
        let mut tree = Tree::default();
        // Вид с 1.5 км над центром города.
        let eye = e.geodetic_to_ecef(37.79, -122.40, 1500.0);
        let always = |_: DVec3, _: f64| true;
        let view = View { eye, pixels: 1000.0, visible: &always };
        let mut pages = 0;
        while pages < 40 {
            let wanted = index.wanted(&tree, &view, 6);
            if wanted.is_empty() {
                break;
            }
            for p in wanted {
                index.add_page(&mut tree, p, &get(&source.address(&scene::i3s::Index::page_path(p)).unwrap())).unwrap();
                pages += 1;
            }
        }
        let with_mesh = (0..tree.nodes.len()).filter(|&n| tree.nodes[n].uri.is_some()).count();
        eprintln!("страниц {pages}, тайлов {}, с сеткой {with_mesh}", tree.nodes.len());
        let n = (0..tree.nodes.len()).find(|&n| tree.nodes[n].uri.is_some()).expect("нет узла с сеткой");
        let geometry = get(&source.address(tree.nodes[n].uri.as_deref().unwrap()).unwrap());
        let texture = index.texture_path(n).map(|p| get(&source.address(&p).unwrap()));
        let piece = index.piece(n, &geometry, texture).unwrap();
        let part = &piece.parts[0];
        let (lat, lon, h) = e.ecef_to_geodetic(part.positions[0]);
        eprintln!("узел {n}: вершин {}, текстура {:?}, первая вершина {lat:.5} {lon:.5} {h:.1}", part.positions.len(), part.image);
        assert!(part.positions.len() >= 3 && (37.6..37.9).contains(&lat) && (-122.6..-122.3).contains(&lon) && (-100.0..700.0).contains(&h));
        assert!(part.uv.is_some() == part.image.is_some());
        assert!(mesh(&piece.parts, &piece.pictures, &e).is_some_and(|m| m.images.len() == usize::from(part.image.is_some())));
    }
}