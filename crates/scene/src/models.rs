// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! 3D-модели на глобусе, как Model в KML Google Earth. Перенос части
//! `core/models.py` модуля - то, что нужно KML: место модели и пакет
//! её файлов. Чтение сеток Collada, glTF и OBJ - фаза 3.
//!
//! Модель - файл Collada (.dae), glTF (.gltf, .glb) или OBJ (.obj)
//! с картинками. Файлы модели хранятся одним пакетом - архивом ZIP
//! с главным файлом, картинками и записью planetx.json (главный файл
//! и замены путей ResourceMap KML). Пакет собирается по ссылкам
//! главного файла: картинки Collada, буферы и картинки glTF, файлы MTL
//! и их картинки у OBJ.
//!
//! Место - как Location, Orientation и Scale KML. Точка (широта,
//! долгота, высота), азимут heading по часовой стрелке от севера вокруг
//! оси вверх, наклон tilt вокруг оси X и крен roll вокруг оси Y против
//! часовой стрелки, масштаб по трём осям. Ось X модели смотрит на
//! восток, Y - на север, Z - вверх. Высота - по altitudeMode:
//! clampToGround - на рельефе, relativeToGround - над рельефом,
//! absolute - над уровнем моря.

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use globe_core::camera::enu;
use globe_core::ellipsoid::Ellipsoid;
use globe_core::{DMat3, DVec3};
use serde_json::{Map, Value};

use crate::util::{Archive, basename, dirname, normpath, unquote, write_zip};
use crate::xml;

pub const MODES: [&str; 3] = ["clampToGround", "relativeToGround", "absolute"];
pub const EXTENSIONS: [&str; 4] = [".dae", ".gltf", ".glb", ".obj"];
pub const PACKAGE_NOTE: &str = "planetx.json";
pub const GLB_MAGIC: &[u8; 4] = b"glTF";

/// Файл модели не читается.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelError(pub String);

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ModelError {}

fn error(text: impl fmt::Display) -> ModelError {
    ModelError(text.to_string())
}

/// Матрица по строкам.
fn rows(r0: [f64; 3], r1: [f64; 3], r2: [f64; 3]) -> DMat3 {
    DMat3::from_cols(DVec3::from(r0), DVec3::from(r1), DVec3::from(r2)).transpose()
}

/// Поворот модели в местных осях восток, север, вверх, как Orientation
/// KML.
pub fn rotation(heading: f64, tilt: f64, roll: f64) -> DMat3 {
    let (sh, ch) = heading.to_radians().sin_cos();
    let (st, ct) = tilt.to_radians().sin_cos();
    let (sr, cr) = roll.to_radians().sin_cos();
    // Азимут - по часовой стрелке, если смотреть сверху.
    let rz = rows([ch, sh, 0.0], [-sh, ch, 0.0], [0.0, 0.0, 1.0]);
    let rx = rows([1.0, 0.0, 0.0], [0.0, ct, -st], [0.0, st, ct]);
    let ry = rows([cr, 0.0, sr], [0.0, 1.0, 0.0], [-sr, 0.0, cr]);
    rz * rx * ry
}

/// Матрица 3×3 из координат модели в смещение ECEF от точки модели.
pub fn frame(lat: f64, lon: f64, heading: f64, tilt: f64, roll: f64, scale: [f64; 3]) -> DMat3 {
    let [east, north, up] = enu(lat, lon);
    DMat3::from_cols(east, north, up) * rotation(heading, tilt, roll) * DMat3::from_diagonal(DVec3::from(scale))
}

/// Поворот 3×3 вершин, посчитанных с матрицей base ([`frame`]), в место
/// lat, lon с азимутом heading: модель трека или поезд едет, вершины
/// в видеокарте остаются прежними.
pub fn moved(base: &DMat3, lat: f64, lon: f64, heading: f64, tilt: f64, roll: f64, scale: [f64; 3]) -> DMat3 {
    frame(lat, lon, heading, tilt, roll, scale) * base.inverse()
}

/// Точка модели в ECEF. ground - высота рельефа под точкой на экране,
/// scale - масштаб рельефа вида. Высота над уровнем моря absolute
/// растягивается вместе с рельефом, подъём над рельефом
/// relativeToGround - нет.
pub fn anchor(lat: f64, lon: f64, mode: &str, alt: f64, ground: f64, scale: f64) -> DVec3 {
    let height = match mode {
        "absolute" => alt * scale,
        "relativeToGround" => ground + alt,
        _ => ground,
    };
    Ellipsoid::wgs84().geodetic_to_ecef(lat, lon, height)
}

/// Файлы пакета по порядку: путь и байты.
pub type Files = Vec<(String, Vec<u8>)>;

/// Байты файла path из files.
pub fn file<'a>(files: &'a Files, path: &str) -> Option<&'a [u8]> {
    files.iter().find(|(n, _)| n == path).map(|(_, d)| d.as_slice())
}

fn put(files: &mut Files, path: String, data: Vec<u8>) {
    match files.iter_mut().find(|(n, _)| *n == path) {
        Some(slot) => slot.1 = data,
        None => files.push((path, data)),
    }
}

/// Путь в пакете: косые черты прямые, без «./» в начале и «..» внутри.
pub fn clean(path: &str) -> String {
    let mut path = path.replace('\\', "/");
    while let Some(rest) = path.strip_prefix("./") {
        path = rest.to_string();
    }
    if path.is_empty() { path } else { normpath(&path).trim_start_matches('/').to_string() }
}

fn has_extension(name: &str) -> bool {
    let lower = name.to_lowercase();
    EXTENSIONS.iter().any(|e| lower.ends_with(e))
}

/// Пакет модели: архив ZIP. main - путь главного файла, files - пути
/// и байты, aliases - путь в модели - путь в пакете.
pub fn pack(main: &str, files: &Files, aliases: &BTreeMap<String, String>) -> Result<Vec<u8>, ModelError> {
    let mut note = Map::new();
    note.insert("main".into(), Value::from(clean(main)));
    if !aliases.is_empty() {
        let map: Map<String, Value> = aliases.iter().map(|(k, v)| (k.clone(), Value::from(clean(v)))).collect();
        note.insert("aliases".into(), Value::Object(map));
    }
    let note = Value::Object(note).to_string();
    let cleaned: Vec<(String, &[u8])> = files.iter().map(|(p, d)| (clean(p), d.as_slice())).collect();
    let entries = std::iter::once((PACKAGE_NOTE, note.as_bytes())).chain(cleaned.iter().map(|(p, d)| (p.as_str(), *d)));
    write_zip(entries).map_err(error)
}

/// Распакованный пакет модели.
#[derive(Debug, Clone, PartialEq)]
pub struct Package {
    /// Путь главного файла.
    pub main: String,
    pub files: Files,
    /// Замены путей: путь в модели - путь в пакете.
    pub aliases: BTreeMap<String, String>,
}

/// Главный файл, файлы и замены из пакета.
pub fn unpack(data: &[u8]) -> Result<Package, ModelError> {
    let mut archive = Archive::open(data).map_err(error)?;
    let names: Vec<String> = archive.names().to_vec();
    let mut note = Value::Null;
    if names.iter().any(|n| n == PACKAGE_NOTE) {
        note = serde_json::from_slice(&archive.read(PACKAGE_NOTE).map_err(error)?).unwrap_or(Value::Null);
    }
    let mut files = Files::new();
    for name in names.iter().filter(|n| *n != PACKAGE_NOTE && !n.ends_with('/')) {
        let bytes = archive.read(name).map_err(error)?;
        put(&mut files, name.clone(), bytes);
    }
    let main = note
        .get("main")
        .and_then(Value::as_str)
        .filter(|m| file(&files, m).is_some())
        .map(String::from)
        .or_else(|| files.iter().map(|(n, _)| n).find(|n| has_extension(n)).cloned())
        .ok_or_else(|| error("no model file in package"))?;
    let aliases = note
        .get("aliases")
        .and_then(Value::as_object)
        .map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect())
        .unwrap_or_default();
    Ok(Package { main, files, aliases })
}

fn join(base: &str, path: &str) -> String {
    if path.starts_with('/') || base.is_empty() {
        path.to_string()
    } else if base.ends_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}

/// Путь в пакете для ссылки reference из главного файла или None.
pub fn resolve(files: &Files, main: &str, reference: &str, aliases: &BTreeMap<String, String>) -> Option<String> {
    if reference.is_empty() {
        return None;
    }
    let reference = reference.replace('\\', "/");
    if let Some(target) = aliases.get(&reference).filter(|t| file(files, t).is_some()) {
        return Some(target.clone());
    }
    let base = dirname(main);
    let lower: HashMap<String, &String> = files.iter().map(|(n, _)| (n.to_lowercase(), n)).collect();
    for path in [clean(&join(base, &reference)), clean(&reference)] {
        if file(files, &path).is_some() {
            return Some(path);
        }
        if let Some(found) = lower.get(&path.to_lowercase()) {
            return Some((*found).clone());
        }
    }
    // Картинка с тем же именем в любой папке пакета.
    let name = basename(&reference).to_lowercase();
    let found: Vec<&String> = files.iter().map(|(n, _)| n).filter(|n| basename(n).to_lowercase() == name).collect();
    match found.as_slice() {
        [one] => Some((*one).clone()),
        _ => None,
    }
}

/// JSON и двоичный кусок (или None) из байтов .glb.
pub fn split_glb(data: &[u8]) -> Result<(&[u8], Option<&[u8]>), ModelError> {
    if data.len() < 20 || !data.starts_with(GLB_MAGIC) {
        return Err(error("not GLB"));
    }
    let word = |at: usize| -> usize {
        data.get(at..at + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    let limit = word(8).min(data.len());
    let mut at = 12usize;
    let (mut text, mut blob) = (None, None);
    while at.saturating_add(8) <= limit {
        let (length, kind) = (word(at), word(at + 4));
        let start = at + 8;
        let end = start.saturating_add(length).min(data.len());
        let chunk = &data[start.min(end)..end];
        if kind == 0x4E4F_534A {
            text = Some(chunk);
        } else if kind == 0x004E_4942 && blob.is_none() {
            blob = Some(chunk);
        }
        at = start.saturating_add(length);
    }
    Ok((text.ok_or_else(|| error("GLB without JSON"))?, blob))
}

/// Пути картинок файла .dae, как записаны в library_images.
pub fn collada_images(data: &[u8]) -> Result<Vec<String>, ModelError> {
    let root = xml::parse(data).map_err(error)?;
    let nodes = root.iter();
    let mut ids: HashMap<&str, &xml::Node> = HashMap::new();
    for node in &nodes {
        if let Some(id) = node.get("id").filter(|i| !i.is_empty()) {
            ids.insert(id, node);
        }
    }
    let mut out: Vec<String> = Vec::new();
    for node in nodes.iter().filter(|n| n.tag == "image") {
        let Some(found) = node.get("id").and_then(|id| ids.get(id)).filter(|n| n.tag == "image") else {
            continue;
        };
        let Some(init) = found.child("init_from") else {
            continue;
        };
        // Collada 1.5 кладёт адрес во вложенный <ref>.
        let mut text = init.child("ref").unwrap_or(init).text.trim().to_string();
        if let Some(rest) = text.strip_prefix("file://") {
            text = rest.to_string();
            let b = text.as_bytes();
            if b.len() > 2 && b[0] == b'/' && b[2] == b':' {
                text = text[1..].to_string();
            }
        }
        let path = unquote(&text.replace('\\', "/"));
        if !path.is_empty() && !out.contains(&path) {
            out.push(path);
        }
    }
    Ok(out)
}

fn lines(data: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(data).split(['\n', '\r']).map(|l| l.trim().to_string()).collect()
}

/// Остаток строки после первого слова: имя файла может быть
/// с пробелами.
fn rest(line: &str) -> &str {
    line.trim_start().split_once(char::is_whitespace).map_or("", |(_, r)| r.trim())
}

/// Имя картинки map_Kd без ключей вида -s 1 1 1.
fn texture_name(rest: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut skip = 0;
    for word in rest.split_whitespace() {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        if word.starts_with('-') && out.is_empty() {
            // -s, -o, -t берут три числа, -mm - два, прочие - одно.
            skip = match word {
                "-s" | "-o" | "-t" => 3,
                "-mm" => 2,
                _ => 1,
            };
            continue;
        }
        out.push(word);
    }
    out.join(" ").replace('\\', "/")
}

/// Картинки материалов файла MTL по порядку материалов.
fn mtl_images(data: &[u8]) -> Vec<String> {
    let mut materials: Vec<(String, Option<String>)> = Vec::new();
    let mut current: Option<(String, Option<String>)> = None;
    let close = |current: &mut Option<(String, Option<String>)>, materials: &mut Vec<(String, Option<String>)>| {
        if let Some((name, image)) = current.take() {
            match materials.iter_mut().find(|(n, _)| *n == name) {
                Some(slot) => slot.1 = image,
                None => materials.push((name, image)),
            }
        }
    };
    for line in lines(data) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let key = line.split_whitespace().next().unwrap_or_default();
        match key {
            "newmtl" => {
                close(&mut current, &mut materials);
                current = Some((rest(&line).to_string(), None));
            }
            "map_Kd" => {
                if let Some((_, image)) = current.as_mut() {
                    let name = texture_name(rest(&line));
                    *image = (!name.is_empty()).then_some(name);
                }
            }
            _ => {}
        }
    }
    close(&mut current, &mut materials);
    materials.into_iter().filter_map(|(_, image)| image).collect()
}

/// Ссылки файла OBJ: файлы MTL и их картинки.
fn obj_refs(data: &[u8], fetch: &dyn Fn(&str) -> Option<Vec<u8>>) -> Vec<String> {
    let mut libraries: Vec<String> = Vec::new();
    for line in lines(data) {
        if line.starts_with("mtllib") {
            let name = rest(&line).replace('\\', "/");
            if !name.is_empty() && !libraries.contains(&name) {
                libraries.push(name);
            }
        }
    }
    let mut out = Vec::new();
    for name in libraries {
        out.push(name.clone());
        let Some(found) = fetch(&name) else {
            continue;
        };
        for image in mtl_images(&found) {
            if !out.contains(&image) {
                out.push(image);
            }
        }
    }
    out
}

/// Ссылки главного файла модели на другие файлы: картинки Collada,
/// буферы и картинки glTF, кроме data:, файлы MTL и их картинки у OBJ.
/// fetch(путь) - байты файла рядом, нужен OBJ для чтения MTL.
pub fn refs(main: &str, data: &[u8], fetch: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Vec<String>, ModelError> {
    let lower = main.to_lowercase();
    if lower.ends_with(".dae") {
        return collada_images(data);
    }
    if lower.ends_with(".obj") {
        return Ok(obj_refs(data, fetch));
    }
    let text = if data.starts_with(GLB_MAGIC) { split_glb(data)?.0 } else { data };
    let doc: Value = serde_json::from_slice(text).map_err(|_| error("bad glTF JSON"))?;
    let mut out = Vec::new();
    for name in ["buffers", "images"] {
        for item in doc.get(name).and_then(Value::as_array).into_iter().flatten() {
            if let Some(uri) = item.get("uri").and_then(Value::as_str).filter(|u| !u.is_empty() && !u.starts_with("data:")) {
                out.push(unquote(uri));
            }
        }
    }
    Ok(out)
}

/// Пакет модели из главного файла и файлов по его ссылкам.
///
/// main - имя главного файла, data - его байты, fetch(путь) - байты
/// файла по пути от папки главного файла или None. aliases - замены
/// путей ResourceMap KML: путь в модели - путь файла. Файл по ссылке
/// с «..» ложится в папку files пакета под заменой пути. Ненайденные
/// файлы пропускаются, часть остаётся цветом материала.
pub fn collect(
    main: &str,
    data: &[u8],
    fetch: &dyn Fn(&str) -> Option<Vec<u8>>,
    aliases: &BTreeMap<String, String>,
) -> Result<Vec<u8>, ModelError> {
    let name = basename(&main.replace('\\', "/")).to_string();
    let name = if name.is_empty() { "model.dae".to_string() } else { name };
    let mut files: Files = vec![(name.clone(), data.to_vec())];
    let mut new_aliases = BTreeMap::new();
    let target = |reference: &str| aliases.get(reference).cloned().unwrap_or_else(|| reference.to_string());
    let near = |reference: &str| fetch(&target(reference));
    for (n, reference) in refs(&name, data, &near)?.into_iter().enumerate() {
        let Some(found) = fetch(&target(&reference)) else {
            continue;
        };
        let mut path = clean(&reference);
        if path.is_empty() || path.starts_with("..") || file(&files, &path).is_some() {
            let short = basename(&path);
            path = format!("files/{n}_{}", if short.is_empty() { "file" } else { short });
            new_aliases.insert(reference, path.clone());
        }
        put(&mut files, path, found);
    }
    pack(&name, &files, &new_aliases)
}
