// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Чтение наборов 3D Tiles 1.0 и 1.1: `tileset.json` и куски GLB или B3DM.
//! Модуль PlanetX с 1.26 и сам Lite выгружают вид в 3D Tiles
//! ([`crate::sceneexport`]) - этот читатель замыкает связку: выгрузка
//! открывается снова, как и открытые городские наборы с кусками GLB
//! и B3DM.
//!
//! Тайл - узел дерева [`Tree`]: матрица transform (4 × 4 по столбцам,
//! копится от корня), geometricError, refine ADD или REPLACE, content.uri
//! и сфера вокруг boundingVolume в ECEF. Набор с внешними наборами
//! (content - другой `tileset.json`) читается вглубь.
//!
//! Куски грузятся по мере приближения: [`Tree::select`] на кадр выбирает
//! тайлы по экранной ошибке - geometricError в пикселях на расстоянии
//! от глаза до сферы тайла. Ошибка выше порога - тайл уточняется. REPLACE
//! уступает место детям, только когда их куски готовы: на месте
//! неготовых детей рисуется родитель, дыр нет. ADD рисует детей вместе
//! с тайлом. [`read`] берёт один срез без камеры - тайлы с наибольшей
//! ошибкой делятся первыми, пока кусков не больше [`MAX_CONTENTS`].
//!
//! Кусок GLB - glTF с осью Y вверх: [`crate::gltf::read`] уже переводит
//! её в Z вверх, как требует 3D Tiles. B3DM - заголовок 28 байт, таблицы
//! признаков и пакетов, за ними GLB. Сдвиг куска - RTC_CENTER таблицы
//! признаков или расширение CESIUM_RTC glTF. Итог - части в ECEF, метры.
//!
//! Враждебный набор ограничен: глубина дерева не больше [`MAX_DEPTH`],
//! тайлов в обходе не больше [`MAX_TILES`], кусков в срезе не больше
//! [`MAX_CONTENTS`]. Сжатые сетки Draco и meshopt не читаются - кусок
//! с ними пропускается, его имя - в списке непрочитанных.

use std::collections::HashMap;

use globe_core::ellipsoid::Ellipsoid;
use globe_core::{DMat3, DMat4, DVec3, DVec4};
use serde_json::Value;

use crate::gltf;
use crate::models::{GLB_MAGIC, ModelError, Part, Pictures, error, split_glb};

/// Наибольшая глубина дерева тайлов вместе с внешними наборами.
pub const MAX_DEPTH: usize = 32;
/// Наибольшее количество тайлов в обходе дерева.
pub const MAX_TILES: usize = 200_000;
/// Наибольшее количество кусков в срезе - столько файлов читается.
pub const MAX_CONTENTS: usize = 256;

/// Начало файла B3DM.
const B3DM_MAGIC: &[u8; 4] = b"b3dm";

/// Файл набора по пути от корня набора: байты или None.
pub type Fetch<'f> = &'f dyn Fn(&str) -> Option<Vec<u8>>;

/// Кусок набора на глобусе: части в ECEF и картинки частей.
#[derive(Debug, Clone, Default)]
pub struct Piece {
    pub name: String,
    pub parts: Vec<Part>,
    pub pictures: Pictures,
}

/// Прочитанный набор: куски и пути кусков, которые не прочлись.
#[derive(Debug, Clone, Default)]
pub struct Tileset {
    pub pieces: Vec<Piece>,
    pub skipped: Vec<String>,
}

/// Тайл дерева: путь куска от корня набора или None, накопленная
/// матрица, ошибка в метрах, способ уточнения, дети и сфера вокруг
/// объёма тайла в ECEF.
#[derive(Debug, Clone)]
pub struct TileNode {
    pub uri: Option<String>,
    pub transform: DMat4,
    pub error: f64,
    pub add: bool,
    pub children: Vec<usize>,
    pub center: DVec3,
    pub radius: f64,
}

/// Дерево набора: тайлы по номерам, номер корня, ось Z вверх у кусков
/// старых наборов 1.0 и внешние наборы, которые не прочлись.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    pub nodes: Vec<TileNode>,
    pub root: usize,
    pub z_up: bool,
    pub skipped: Vec<String>,
}

/// Вид для выбора тайлов: глаз в ECEF, пикселей экрана на радиан
/// по вертикали и проверка, видна ли сфера (центр, радиус).
pub struct View<'v> {
    pub eye: DVec3,
    pub pixels: f64,
    pub visible: &'v dyn Fn(DVec3, f64) -> bool,
}

/// Выбор на кадр: show - тайлы с готовыми кусками для рисования, load -
/// тайлы, куски которых нужны, с экранной ошибкой, по её убыванию.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Selection {
    pub show: Vec<usize>,
    pub load: Vec<(usize, f64)>,
}

/// Набор ли это 3D Tiles: JSON с `asset` и `root`.
pub fn is_tileset(data: &[u8]) -> bool {
    let start = data.iter().position(|b| !b.is_ascii_whitespace());
    if start.is_none_or(|s| data[s] != b'{') {
        return false;
    }
    serde_json::from_slice::<Value>(data).is_ok_and(|v| v.get("asset").is_some() && v.get("root").is_some())
}

/// Матрица transform тайла, 16 чисел по столбцам. Нет поля - единичная.
fn matrix(tile: &Value) -> DMat4 {
    let Some(values) = tile.get("transform").and_then(Value::as_array).filter(|a| a.len() == 16) else {
        return DMat4::IDENTITY;
    };
    let mut cols = [0.0; 16];
    for (c, v) in cols.iter_mut().zip(values) {
        *c = v.as_f64().unwrap_or(0.0);
    }
    if cols.iter().any(|c| !c.is_finite()) {
        return DMat4::IDENTITY;
    }
    DMat4::from_cols_array(&cols)
}

/// Сфера вокруг boundingVolume тайла в ECEF: box и sphere - в осях
/// тайла через transform, region - радианы и высоты на WGS84. Нет объёма
/// или он испорчен - None.
fn sphere(tile: &Value, transform: DMat4) -> Option<(DVec3, f64)> {
    let volume = tile.get("boundingVolume")?;
    let numbers = |key: &str, len: usize| -> Option<Vec<f64>> {
        let values: Vec<f64> = volume.get(key)?.as_array()?.iter().map(Value::as_f64).collect::<Option<_>>()?;
        (values.len() == len && values.iter().all(|v| v.is_finite())).then_some(values)
    };
    let point = |v: DVec3| (transform * DVec4::from((v, 1.0))).truncate();
    let out = if let Some(b) = numbers("box", 12) {
        let c = DVec3::new(b[0], b[1], b[2]);
        let axes = [DVec3::new(b[3], b[4], b[5]), DVec3::new(b[6], b[7], b[8]), DVec3::new(b[9], b[10], b[11])];
        let center = point(c);
        let radius = (0..8)
            .map(|s: u32| {
                let sign = |k: u32| if s >> k & 1 == 1 { 1.0 } else { -1.0 };
                point(c + axes[0] * sign(0) + axes[1] * sign(1) + axes[2] * sign(2)).distance(center)
            })
            .fold(0.0, f64::max);
        (center, radius)
    } else if let Some(s) = numbers("sphere", 4) {
        let linear = DMat3::from_mat4(transform);
        let scale = [linear.x_axis, linear.y_axis, linear.z_axis].iter().map(|a| a.length()).fold(0.0, f64::max);
        (point(DVec3::new(s[0], s[1], s[2])), s[3].abs() * scale)
    } else {
        let r = numbers("region", 6)?;
        // Сетка 3 × 3 на нижней и верхней высоте. Дуга между точками
        // сетки выпирает - радиус с запасом в десятую долю.
        let e = Ellipsoid::wgs84();
        let east = if r[2] < r[0] { r[2] + std::f64::consts::TAU } else { r[2] };
        let mut points = Vec::with_capacity(18);
        for i in 0..3 {
            for j in 0..3 {
                for h in [r[4], r[5]] {
                    let lon = r[0] + (east - r[0]) * f64::from(i) / 2.0;
                    let lat = r[1] + (r[3] - r[1]) * f64::from(j) / 2.0;
                    points.push(e.geodetic_to_ecef(lat.to_degrees(), lon.to_degrees(), h));
                }
            }
        }
        let center = points.iter().fold(DVec3::ZERO, |a, p| a + *p) / points.len() as f64;
        (center, points.iter().map(|p| p.distance(center)).fold(0.0, f64::max) * 1.1)
    };
    (out.0.is_finite() && out.1.is_finite()).then_some(out)
}

/// Путь uri от папки dir: «./», «../» и обратные косые разбираются,
/// адрес с протоколом и путь от корня остаются как есть.
fn join(dir: &str, uri: &str) -> String {
    let uri = uri.split(['?', '#']).next().unwrap_or(uri).replace('\\', "/");
    if uri.contains("://") || dir.is_empty() {
        return uri.trim_start_matches("./").to_string();
    }
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for p in uri.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(p),
        }
    }
    parts.join("/")
}

/// Папка пути: всё до последней косой.
fn folder(path: &str) -> &str {
    path.rfind('/').map_or("", |at| &path[..at])
}

/// Обход дерева тайлов: узлы по номерам. Внешний набор - content с .json -
/// читается через fetch и встаёт ребёнком вместо куска.
struct Walker<'f> {
    nodes: Vec<TileNode>,
    fetch: Fetch<'f>,
    skipped: Vec<String>,
}

/// Что тайл берёт от родителя: матрицу, способ уточнения и сферу -
/// у тайла без объёма она родительская.
#[derive(Clone, Copy)]
struct Parent {
    transform: DMat4,
    add: bool,
    sphere: (DVec3, f64),
}

impl Walker<'_> {
    fn walk(&mut self, tile: &Value, parent: Parent, dir: &str, depth: usize) -> Option<usize> {
        if depth > MAX_DEPTH || self.nodes.len() >= MAX_TILES || !tile.is_object() {
            return None;
        }
        let transform = parent.transform * matrix(tile);
        let error = tile.get("geometricError").and_then(Value::as_f64).filter(|e| e.is_finite()).unwrap_or(0.0).max(0.0);
        let add = match tile.get("refine").and_then(Value::as_str) {
            Some(r) => r.eq_ignore_ascii_case("ADD"),
            None => parent.add,
        };
        let (center, radius) = sphere(tile, transform).unwrap_or(parent.sphere);
        let content = tile.get("content").or_else(|| tile.get("contents").and_then(|c| c.get(0)));
        let uri = content.and_then(|c| c.get("uri").or_else(|| c.get("url"))).and_then(Value::as_str).map(|u| join(dir, u));
        let at = self.nodes.len();
        self.nodes.push(TileNode { uri: None, transform, error, add, children: Vec::new(), center, radius });
        let here = Parent { transform, add, sphere: (center, radius) };
        let mut children = Vec::new();
        match uri {
            // Внешний набор - ребёнок с его корнем.
            Some(u) if u.to_ascii_lowercase().ends_with(".json") => match (self.fetch)(&u).and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
                Some(sub) => {
                    if let Some(root) = sub.get("root") {
                        children.extend(self.walk(root, here, folder(&u), depth + 1));
                    }
                }
                None => self.skipped.push(u),
            },
            other => self.nodes[at].uri = other,
        }
        for child in tile.get("children").and_then(Value::as_array).into_iter().flatten() {
            children.extend(self.walk(child, here, dir, depth + 1));
        }
        self.nodes[at].children = children;
        Some(at)
    }
}

impl Tree {
    /// Дерево из `tileset.json` root, внешние наборы - через fetch.
    pub fn open(root: &[u8], fetch: Fetch<'_>) -> Result<Tree, ModelError> {
        let json: Value = serde_json::from_slice(root).map_err(|_| error("bad tileset JSON"))?;
        let tile = json.get("root").ok_or_else(|| error("tileset has no root"))?;
        // gltfUpAxis Z у старых наборов 1.0: ось glTF уже Z вверх, перевод
        // gltf::read надо отменить поворотом назад.
        let z_up = json.get("asset").and_then(|a| a.get("gltfUpAxis")).and_then(Value::as_str).is_some_and(|a| a.eq_ignore_ascii_case("Z"));
        let mut walker = Walker { nodes: Vec::new(), fetch, skipped: Vec::new() };
        // Корень без объёма - сфера без меры: экранная ошибка у него
        // бесконечна, он сразу уточняется.
        let top = Parent { transform: DMat4::IDENTITY, add: false, sphere: (DVec3::ZERO, f64::INFINITY) };
        let root = walker.walk(tile, top, "", 0).ok_or_else(|| error("empty tileset"))?;
        Ok(Tree { nodes: walker.nodes, root, z_up, skipped: walker.skipped })
    }

    /// Экранная ошибка тайла n в пикселях. Глаз внутри сферы - ошибка
    /// бесконечна.
    pub fn sse(&self, n: usize, view: &View<'_>) -> f64 {
        let Some(node) = self.nodes.get(n) else {
            return 0.0;
        };
        let distance = node.center.distance(view.eye) - node.radius;
        if distance.is_nan() || distance <= 0.0 {
            return f64::INFINITY;
        }
        node.error * view.pixels / distance
    }

    /// Тайлы на кадр при пороге threshold пикселей. ready - готов ли кусок
    /// тайла (прочитан или не прочтётся вовсе).
    pub fn select(&self, view: &View<'_>, threshold: f64, ready: &dyn Fn(usize) -> bool) -> Selection {
        let mut out = Selection::default();
        if !self.nodes.is_empty() {
            self.visit(self.root, view, threshold, ready, &mut out);
        }
        out.load.sort_by(|a, b| b.1.total_cmp(&a.1));
        out
    }

    fn visit(&self, n: usize, view: &View<'_>, threshold: f64, ready: &dyn Fn(usize) -> bool, out: &mut Selection) {
        let Some(node) = self.nodes.get(n) else {
            return;
        };
        if !(view.visible)(node.center, node.radius) {
            return;
        }
        let sse = self.sse(n, view);
        let has = node.uri.is_some();
        let want = |out: &mut Selection| {
            if has {
                if ready(n) { out.show.push(n) } else { out.load.push((n, sse)) }
            }
        };
        if sse <= threshold || node.children.is_empty() {
            want(out);
            return;
        }
        if node.add {
            want(out);
        } else if has && !self.covered(n, view, ready) {
            // Дети не готовы: рисуется тайл, дети грузятся. Неготовый тайл
            // грузится раньше детей - сперва грубый вид.
            if ready(n) {
                out.show.push(n);
                for &c in &node.children {
                    self.request(c, view, ready, out);
                }
            } else {
                out.load.push((n, sse));
            }
            return;
        }
        for &c in &node.children {
            self.visit(c, view, threshold, ready, out);
        }
    }

    /// Готовы ли видимые дети тайла n заменить его. Ребёнок без куска
    /// готов, когда готовы его дети.
    fn covered(&self, n: usize, view: &View<'_>, ready: &dyn Fn(usize) -> bool) -> bool {
        self.nodes.get(n).is_none_or(|node| {
            node.children.iter().all(|&c| {
                self.nodes.get(c).is_none_or(|child| {
                    !(view.visible)(child.center, child.radius) || if child.uri.is_some() { ready(c) } else { self.covered(c, view, ready) }
                })
            })
        })
    }

    /// Заказать ближайшие куски под тайлом n: его кусок или, у тайла без
    /// куска, куски детей.
    fn request(&self, n: usize, view: &View<'_>, ready: &dyn Fn(usize) -> bool, out: &mut Selection) {
        let Some(node) = self.nodes.get(n) else {
            return;
        };
        if !(view.visible)(node.center, node.radius) {
            return;
        }
        if node.uri.is_some() {
            if !ready(n) {
                out.load.push((n, self.sse(n, view)));
            }
            return;
        }
        for &c in &node.children {
            self.request(c, view, ready, out);
        }
    }

    /// Кусок тайла n из уже прочитанных частей в осях тайла: части в ECEF
    /// по матрице тайла.
    pub fn place(&self, n: usize, parts: &[Part], pictures: &Pictures) -> Piece {
        let Some(node) = self.nodes.get(n) else {
            return Piece::default();
        };
        let m = node.transform;
        let linear = DMat3::from_mat4(m);
        let normal = if linear.determinant().abs() > 1e-12 { linear.inverse().transpose() } else { linear };
        let z_up = self.z_up;
        let undo = |v: DVec3| if z_up { DVec3::new(v.x, v.z, -v.y) } else { v };
        let parts = parts
            .iter()
            .map(|p| Part {
                positions: p.positions.iter().map(|v| (m * DVec4::from((undo(*v), 1.0))).truncate()).collect(),
                normals: p.normals.as_ref().map(|ns| ns.iter().map(|v| (normal * undo(*v)).normalize_or_zero()).collect()),
                ..p.clone()
            })
            .collect();
        Piece { name: node.uri.clone().unwrap_or_default(), parts, pictures: pictures.clone() }
    }

    /// Прочитать кусок тайла n через fetch. Тайл без куска - ошибка.
    pub fn load(&self, n: usize, fetch: Fetch<'_>) -> Result<Piece, ModelError> {
        let uri = self.nodes.get(n).and_then(|node| node.uri.as_deref()).ok_or_else(|| error("tile has no content"))?;
        let dir = folder(uri).to_string();
        let near = |name: &str| fetch(&join(&dir, name));
        let bytes = fetch(uri).ok_or_else(|| error("missing file"))?;
        let (parts, pictures) = content(&bytes, &near)?;
        Ok(self.place(n, &parts, &pictures))
    }
}

/// Срез дерева: номера узлов с кусками, не больше [`MAX_CONTENTS`]. Узел
/// с наибольшей ошибкой делится первым. REPLACE - узел уступает место
/// детям, ADD - дети ложатся вместе с ним. Узел без куска и без детей
/// в срез не входит.
fn cut(nodes: &[TileNode], root: usize) -> Vec<usize> {
    let count = |set: &[usize]| set.iter().filter(|&&n| nodes[n].uri.is_some()).count();
    // В срезе - узлы, которые рисуются, в ожидании - узлы для деления.
    let mut shown: Vec<usize> = vec![root];
    let mut open: Vec<usize> = vec![root];
    while let Some((k, &n)) = open.iter().enumerate().max_by(|a, b| nodes[*a.1].error.total_cmp(&nodes[*b.1].error)) {
        open.remove(k);
        let kids = &nodes[n].children;
        if kids.is_empty() {
            continue;
        }
        let mut next: Vec<usize> = shown.clone();
        if !nodes[n].add {
            next.retain(|&s| s != n);
        }
        next.extend(kids.iter().copied());
        if count(&next) > MAX_CONTENTS {
            continue;
        }
        shown = next;
        open.extend(kids.iter().copied());
    }
    shown.retain(|&n| nodes[n].uri.is_some());
    shown
}

/// Части куска GLB или B3DM в осях тайла, Z вверх, со сдвигом RTC.
pub fn content(data: &[u8], fetch: gltf::Fetch<'_>) -> Result<(Vec<Part>, Pictures), ModelError> {
    let (glb, rtc) = if data.starts_with(B3DM_MAGIC) { b3dm(data)? } else { (data, None) };
    if !glb.starts_with(GLB_MAGIC) {
        return Err(error("tile content is not GLB or B3DM"));
    }
    // CESIUM_RTC glTF - центр куска, как RTC_CENTER у B3DM.
    let rtc = rtc.or_else(|| {
        let (text, _) = split_glb(glb).ok()?;
        let json: Value = serde_json::from_slice(text).ok()?;
        let c = json.get("extensions")?.get("CESIUM_RTC")?.get("center")?.as_array()?;
        Some(DVec3::new(c.first()?.as_f64()?, c.get(1)?.as_f64()?, c.get(2)?.as_f64()?))
    });
    let (mut parts, pictures) = gltf::read(glb, fetch)?;
    if let Some(center) = rtc.filter(|c| c.is_finite()) {
        for p in &mut parts {
            for v in &mut p.positions {
                *v += center;
            }
        }
    }
    Ok((parts, pictures))
}

/// GLB внутри B3DM и RTC_CENTER таблицы признаков.
fn b3dm(data: &[u8]) -> Result<(&[u8], Option<DVec3>), ModelError> {
    let word = |at: usize| -> Option<usize> { data.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize) };
    let (Some(total), Some(ft_json), Some(ft_bin), Some(bt_json), Some(bt_bin)) = (word(8), word(12), word(16), word(20), word(24)) else {
        return Err(error("short B3DM header"));
    };
    let start = 28usize;
    let glb_at = [ft_json, ft_bin, bt_json, bt_bin].iter().try_fold(start, |a, n| a.checked_add(*n)).ok_or_else(|| error("bad B3DM header"))?;
    let end = total.min(data.len());
    let glb = data.get(glb_at..end).ok_or_else(|| error("bad B3DM header"))?;
    let rtc = data.get(start..start + ft_json).and_then(|t| serde_json::from_slice::<Value>(t).ok()).and_then(|v| {
        let c = v.get("RTC_CENTER")?.as_array()?;
        Some(DVec3::new(c.first()?.as_f64()?, c.get(1)?.as_f64()?, c.get(2)?.as_f64()?))
    });
    Ok((glb, rtc))
}

/// Набор из `tileset.json` root и файлов по путям от его папки. Куски
/// среза читаются через fetch, их части - в ECEF по матрицам тайлов.
pub fn read(root: &[u8], fetch: Fetch<'_>) -> Result<Tileset, ModelError> {
    let tree = Tree::open(root, fetch)?;
    let mut out = Tileset { pieces: Vec::new(), skipped: tree.skipped.clone() };
    let mut cache: HashMap<String, (Vec<Part>, Pictures)> = HashMap::new();
    for n in cut(&tree.nodes, tree.root) {
        let Some(uri) = &tree.nodes[n].uri else {
            continue;
        };
        if !cache.contains_key(uri) {
            let dir = folder(uri).to_string();
            let near = |name: &str| fetch(&join(&dir, name));
            match fetch(uri).ok_or_else(|| error("missing file")).and_then(|b| content(&b, &near)) {
                Ok(read) => {
                    cache.insert(uri.clone(), read);
                }
                Err(_) => {
                    out.skipped.push(uri.clone());
                    continue;
                }
            }
        }
        if let Some((parts, pictures)) = cache.get(uri) {
            out.pieces.push(tree.place(n, parts, pictures));
        }
    }
    Ok(out)
}

/// Файлы набора из архива ZIP: `tileset.json` в корне архива или в его
/// папке, пути - от папки набора.
#[derive(Debug, Clone, Default)]
pub struct ZipSet {
    pub root: Vec<u8>,
    dir: String,
    files: HashMap<String, Vec<u8>>,
}

impl ZipSet {
    pub fn open(data: &[u8]) -> Result<ZipSet, ModelError> {
        let mut archive = crate::util::Archive::open(data).map_err(|e| error(&e))?;
        let names = archive.names().to_vec();
        let main = names.iter().filter(|n| n.to_ascii_lowercase().ends_with("tileset.json")).min_by_key(|n| n.matches('/').count()).cloned().ok_or_else(|| error("no tileset.json in archive"))?;
        let mut files: HashMap<String, Vec<u8>> = HashMap::new();
        for n in &names {
            if let Ok(bytes) = archive.read(n) {
                files.insert(n.replace('\\', "/"), bytes);
            }
        }
        let root = files.get(&main.replace('\\', "/")).cloned().ok_or_else(|| error("no tileset.json in archive"))?;
        Ok(ZipSet { root, dir: folder(&main.replace('\\', "/")).to_string(), files })
    }

    /// Файл по пути от папки набора.
    pub fn get(&self, path: &str) -> Option<Vec<u8>> {
        self.files.get(&join(&self.dir, path)).cloned()
    }
}

/// Набор из архива ZIP, один срез.
pub fn read_zip(data: &[u8]) -> Result<Tileset, ModelError> {
    let set = ZipSet::open(data)?;
    read(&set.root, &|p| set.get(p))
}

/// Архив ZIP с набором 3D Tiles.
pub fn zip_has_tileset(data: &[u8]) -> bool {
    data.starts_with(b"PK") && crate::util::Archive::open(data).is_ok_and(|a| a.names().iter().any(|n| n.to_ascii_lowercase().ends_with("tileset.json")))
}
