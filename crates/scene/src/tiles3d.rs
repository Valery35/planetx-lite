// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Чтение наборов 3D Tiles 1.0 и 1.1: `tileset.json` и куски GLB или B3DM.
//! Модуль PlanetX с 1.26 и сам Lite выгружают вид в 3D Tiles
//! ([`crate::sceneexport`]) - этот читатель замыкает связку: выгрузка
//! открывается снова, как и открытые городские наборы с кусками GLB
//! и B3DM.
//!
//! Тайл - узел дерева: матрица transform (4 × 4 по столбцам, копится от
//! корня), geometricError, refine ADD или REPLACE и content.uri. Набор
//! с внешними наборами (content - другой `tileset.json`) читается вглубь.
//! Подгрузки по мере приближения нет: из дерева берётся один срез -
//! тайлы с наибольшей ошибкой делятся первыми, пока кусков не больше
//! [`MAX_CONTENTS`]. REPLACE заменяет тайл детьми, ADD добавляет детей
//! к нему.
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

use globe_core::{DMat4, DVec3, DVec4};
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

/// Тайл дерева после обхода: путь куска от корня набора или None,
/// накопленная матрица, ошибка, способ уточнения и дети.
#[derive(Debug, Clone)]
struct Node {
    uri: Option<String>,
    transform: DMat4,
    error: f64,
    add: bool,
    children: Vec<usize>,
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
/// читается через fetch и встаёт вместо куска. parent - матрица и refine
/// родителя, dir - папка набора.
struct Walker<'f> {
    nodes: Vec<Node>,
    fetch: Fetch<'f>,
    skipped: Vec<String>,
}

impl Walker<'_> {
    fn walk(&mut self, tile: &Value, parent: DMat4, parent_add: bool, dir: &str, depth: usize) -> Option<usize> {
        if depth > MAX_DEPTH || self.nodes.len() >= MAX_TILES || !tile.is_object() {
            return None;
        }
        let transform = parent * matrix(tile);
        let error = tile.get("geometricError").and_then(Value::as_f64).filter(|e| e.is_finite()).unwrap_or(0.0).max(0.0);
        let add = match tile.get("refine").and_then(Value::as_str) {
            Some(r) => r.eq_ignore_ascii_case("ADD"),
            None => parent_add,
        };
        let content = tile.get("content").or_else(|| tile.get("contents").and_then(|c| c.get(0)));
        let uri = content.and_then(|c| c.get("uri").or_else(|| c.get("url"))).and_then(Value::as_str).map(|u| join(dir, u));
        let at = self.nodes.len();
        self.nodes.push(Node { uri: None, transform, error, add, children: Vec::new() });
        let mut children = Vec::new();
        match uri {
            // Внешний набор - ребёнок с его корнем.
            Some(u) if u.to_ascii_lowercase().ends_with(".json") => match (self.fetch)(&u).and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
                Some(sub) => {
                    if let Some(root) = sub.get("root") {
                        children.extend(self.walk(root, transform, add, folder(&u), depth + 1));
                    }
                }
                None => self.skipped.push(u),
            },
            other => self.nodes[at].uri = other,
        }
        for child in tile.get("children").and_then(Value::as_array).into_iter().flatten() {
            children.extend(self.walk(child, transform, add, dir, depth + 1));
        }
        self.nodes[at].children = children;
        Some(at)
    }
}

/// Срез дерева: номера узлов с кусками, не больше [`MAX_CONTENTS`]. Узел
/// с наибольшей ошибкой делится первым. REPLACE - узел уступает место
/// детям, ADD - дети ложатся вместе с ним. Узел без куска и без детей
/// в срез не входит.
fn cut(nodes: &[Node], root: usize) -> Vec<usize> {
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
    let json: Value = serde_json::from_slice(root).map_err(|_| error("bad tileset JSON"))?;
    let tile = json.get("root").ok_or_else(|| error("tileset has no root"))?;
    // gltfUpAxis Z у старых наборов 1.0: ось glTF уже Z вверх, перевод
    // gltf::read надо отменить поворотом назад.
    let z_up = json.get("asset").and_then(|a| a.get("gltfUpAxis")).and_then(Value::as_str).is_some_and(|a| a.eq_ignore_ascii_case("Z"));
    let mut walker = Walker { nodes: Vec::new(), fetch, skipped: Vec::new() };
    let Some(top) = walker.walk(tile, DMat4::IDENTITY, false, "", 0) else {
        return Err(error("empty tileset"));
    };
    let mut out = Tileset { pieces: Vec::new(), skipped: std::mem::take(&mut walker.skipped) };
    let mut cache: HashMap<String, (Vec<Part>, Pictures)> = HashMap::new();
    for n in cut(&walker.nodes, top) {
        let node = &walker.nodes[n];
        let Some(uri) = &node.uri else {
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
        let Some((parts, pictures)) = cache.get(uri) else {
            continue;
        };
        let m = node.transform;
        let linear = globe_core::DMat3::from_mat4(m);
        let normal = if linear.determinant().abs() > 1e-12 { linear.inverse().transpose() } else { linear };
        let undo = |v: DVec3| if z_up { DVec3::new(v.x, v.z, -v.y) } else { v };
        let placed: Vec<Part> = parts
            .iter()
            .map(|p| Part {
                positions: p.positions.iter().map(|v| (m * DVec4::from((undo(*v), 1.0))).truncate()).collect(),
                normals: p.normals.as_ref().map(|ns| ns.iter().map(|v| (normal * undo(*v)).normalize_or_zero()).collect()),
                ..p.clone()
            })
            .collect();
        out.pieces.push(Piece { name: uri.clone(), parts: placed, pictures: pictures.clone() });
    }
    Ok(out)
}

/// Набор из архива ZIP: `tileset.json` в корне архива или в его папке,
/// пути кусков - от папки набора.
pub fn read_zip(data: &[u8]) -> Result<Tileset, ModelError> {
    let mut archive = crate::util::Archive::open(data).map_err(|e| error(&e))?;
    let names = archive.names().to_vec();
    let main = names.iter().filter(|n| n.to_ascii_lowercase().ends_with("tileset.json")).min_by_key(|n| n.matches('/').count()).cloned().ok_or_else(|| error("no tileset.json in archive"))?;
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    for n in &names {
        if let Ok(bytes) = archive.read(n) {
            files.insert(n.replace('\\', "/"), bytes);
        }
    }
    let dir = folder(&main).to_string();
    let root = files.get(&main).cloned().ok_or_else(|| error("no tileset.json in archive"))?;
    let fetch = |path: &str| files.get(&join(&dir, path)).cloned();
    read(&root, &fetch)
}

/// Архив ZIP с набором 3D Tiles.
pub fn zip_has_tileset(data: &[u8]) -> bool {
    data.starts_with(b"PK") && crate::util::Archive::open(data).is_ok_and(|a| a.names().iter().any(|n| n.to_ascii_lowercase().ends_with("tileset.json")))
}
