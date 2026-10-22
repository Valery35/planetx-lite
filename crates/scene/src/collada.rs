// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Чтение моделей Collada (.dae), как в Model KML Google Earth. Перенос
//! `core/collada.py` модуля без анимаций.
//!
//! Разбор идёт через [`crate::xml`], объявления сущностей DTD
//! запрещены, как у KML.
//!
//! Из файла берутся сетки треугольников (triangles, polylist, polygons -
//! многоугольники режутся веером), положения, нормали и первый набор
//! текстурных координат, материалы profile_COMMON - цвет diffuse или
//! картинка через sampler2D и surface, прозрачность transparency.
//! Узлы сцены - matrix, translate, rotate, scale в порядке записи,
//! instance_geometry, instance_node и вложенные узлы. Линии (lines,
//! linestrips) пропускаются. Единица - unit meter, ось вверх - up_axis.
//!
//! Итог - части [`Part`]: координаты в метрах, ось Z вверх. Модель
//! Google Earth лежит так, что X смотрит на восток, Y - на север.
//! Текстурная координата v переворачивается: в Collada начало внизу
//! картинки, в Lite - вверху, как у glTF.
//!
//! Враждебный файл ограничен: вложенность узлов не больше
//! [`MAX_DEPTH`], обходов узлов не больше [`MAX_NODES`], вершин не
//! больше [`MAX_VERTICES`].

use std::collections::HashMap;
use std::rc::Rc;

use globe_core::{DMat3, DMat4, DVec2, DVec3};

use crate::models::{GRAY, MAX_VERTICES, ModelError, Part, error, from_y_up, rows};
use crate::util::unquote;
use crate::xml::{self, Node};

/// Многоугольник длиннее пропускается: защита от файла с ошибкой
/// в счёте вершин.
pub const MAX_POLYGON: usize = 4096;
/// Наибольшая вложенность узлов сцены.
pub const MAX_DEPTH: usize = 64;
/// Наибольшее количество обходов узлов: узел, вставленный в сцену
/// много раз через instance_node, не раздувает обход.
pub const MAX_NODES: usize = 1_000_000;

/// Ссылка вида #id без решётки.
fn reference(text: &str) -> &str {
    let text = text.trim();
    text.strip_prefix('#').unwrap_or(text)
}

/// Числа текста узла.
fn numbers(node: Option<&Node>) -> Result<Vec<f64>, ModelError> {
    let Some(node) = node else {
        return Ok(Vec::new());
    };
    node.text.split_whitespace().map(|w| w.parse::<f64>().map_err(|_| error(format!("bad numbers in <{}>", node.tag)))).collect()
}

/// Целые числа текста узла.
fn integers(node: Option<&Node>) -> Result<Vec<i64>, ModelError> {
    let Some(node) = node else {
        return Ok(Vec::new());
    };
    node.text.split_whitespace().map(|w| w.parse::<i64>().map_err(|_| error(format!("bad numbers in <{}>", node.tag)))).collect()
}

/// Источник: значения подряд и шаг строки.
struct Source {
    values: Vec<f64>,
    stride: usize,
}

impl Source {
    /// Строка number, первые width значений, или ошибка за концом.
    fn row(&self, number: i64, width: usize) -> Result<&[f64], ModelError> {
        let start = usize::try_from(number).ok().and_then(|n| n.checked_mul(self.stride));
        start
            .filter(|_| width <= self.stride)
            .and_then(|s| self.values.get(s..s + width))
            .ok_or_else(|| error("index out of range"))
    }
}

/// Разобранный файл: узлы по id, единица, ось, память материалов
/// и источников.
struct Doc<'a> {
    ids: HashMap<&'a str, &'a Node>,
    unit: f64,
    y_up: bool,
    materials: HashMap<String, (Option<String>, [f64; 4])>,
    sources: HashMap<String, Rc<Source>>,
    visits: usize,
    vertices: usize,
}

impl<'a> Doc<'a> {
    fn new(root: &'a Node) -> Self {
        let mut ids = HashMap::new();
        for node in root.iter() {
            if let Some(id) = node.get("id").filter(|i| !i.is_empty()) {
                ids.insert(id, node);
            }
        }
        let mut unit = 1.0;
        let mut y_up = false;
        if let Some(asset) = root.child("asset") {
            if let Some(found) = asset.child("unit") {
                let text = found.get("meter").filter(|t| !t.is_empty()).unwrap_or("1");
                unit = text.trim().parse::<f64>().ok().filter(|u| u.is_finite()).unwrap_or(1.0);
            }
            y_up = asset.child("up_axis").is_some_and(|a| a.text.trim().eq_ignore_ascii_case("Y_UP"));
        }
        Self { ids, unit, y_up, materials: HashMap::new(), sources: HashMap::new(), visits: 0, vertices: 0 }
    }

    fn find(&self, text: &str) -> Option<&'a Node> {
        self.ids.get(reference(text)).copied()
    }

    /// Источник по ссылке, через vertices - его POSITION.
    fn source(&mut self, text: &str) -> Result<Rc<Source>, ModelError> {
        let mut node = self.find(text).ok_or_else(|| error(format!("source not found: {text}")))?;
        let mut hops = 0;
        while node.tag == "vertices" {
            hops += 1;
            let found = node.children_named("input").filter(|i| i.get("semantic") == Some("POSITION")).last();
            let found = found.and_then(|i| i.get("source")).ok_or_else(|| error("vertices without POSITION"))?;
            node = self.find(found).ok_or_else(|| error(format!("source not found: {found}")))?;
            if hops > MAX_DEPTH {
                return Err(error("vertices refer to each other"));
            }
        }
        let key = format!("{:p}", node);
        if let Some(found) = self.sources.get(&key) {
            return Ok(found.clone());
        }
        let mut values = numbers(node.child("float_array"))?;
        let mut stride = 1;
        if let Some(accessor) = node.child("technique_common").and_then(|c| c.child("accessor")) {
            let int = |name: &str, default: usize| {
                accessor.get(name).filter(|t| !t.is_empty()).map_or(Some(default), |t| t.trim().parse::<usize>().ok())
            };
            stride = int("stride", 1).unwrap_or(1).max(1);
            let offset = int("offset", 0).unwrap_or(0).min(values.len());
            values.drain(..offset);
        }
        let count = values.len() / stride;
        values.truncate(count * stride);
        let found = Rc::new(Source { values, stride });
        self.sources.insert(key, found.clone());
        Ok(found)
    }

    /// Путь картинки по id узла image или None.
    fn image_path(&self, ident: &str) -> Option<String> {
        let node = self.ids.get(ident).filter(|n| n.tag == "image")?;
        let init = node.child("init_from")?;
        // Collada 1.5 кладёт адрес во вложенный <ref>.
        let mut text = init.child("ref").unwrap_or(init).text.trim().to_string();
        if let Some(rest) = text.strip_prefix("file://") {
            text = rest.to_string();
            let b = text.as_bytes();
            if b.len() > 2 && b[0] == b'/' && b[2] == b':' {
                text = text.get(1..).unwrap_or_default().to_string();
            }
        }
        Some(unquote(&text.replace('\\', "/")))
    }

    /// Путь картинки по sampler2D - surface - image или прямо по id.
    fn texture_image(&self, params: &HashMap<&str, &Node>, sid: &str) -> Option<String> {
        let mut sid = sid.to_string();
        let mut seen: Vec<String> = Vec::new();
        while let Some(param) = params.get(sid.as_str()).filter(|_| !seen.contains(&sid)) {
            seen.push(sid.clone());
            if let Some(sampler) = param.child("sampler2D") {
                if let Some(source) = sampler.child("source") {
                    sid = source.text.trim().to_string();
                    continue;
                }
                return sampler.child("instance_image").and_then(|i| self.image_path(reference(i.get("url").unwrap_or_default())));
            }
            if let Some(surface) = param.child("surface") {
                return surface.child("init_from").and_then(|i| self.image_path(i.text.trim()));
            }
            return None;
        }
        self.image_path(&sid)
    }

    /// Картинка и цвет эффекта.
    fn effect(&self, effect: &Node) -> Result<(Option<String>, [f64; 4]), ModelError> {
        let Some(profile) = effect.child("profile_COMMON") else {
            return Ok((None, GRAY));
        };
        let params: HashMap<&str, &Node> = profile.children_named("newparam").map(|p| (p.get("sid").unwrap_or_default(), p)).collect();
        let shading = profile.child("technique").and_then(|t| ["lambert", "phong", "blinn", "constant"].iter().find_map(|n| t.child(n)));
        let mut image = None;
        let mut rgba = GRAY;
        if let Some(shading) = shading {
            let slot = shading.child("diffuse").or_else(|| shading.child("emission"));
            if let Some(texture) = slot.and_then(|s| s.child("texture")) {
                image = self.texture_image(&params, texture.get("texture").unwrap_or_default());
                rgba = [1.0; 4];
            } else if let Some(color) = color(slot)? {
                rgba = color;
            }
            if let Some(value) = shading.child("transparency").and_then(|a| a.child("float")) {
                let share = value.text.trim().parse::<f64>().unwrap_or(1.0);
                // SketchUp пишет непрозрачность прямо, как A_ONE. Значение
                // вне 0-1 не принимается.
                if (0.0..=1.0).contains(&share) {
                    rgba[3] *= share;
                }
            }
        }
        Ok((image.filter(|i| !i.is_empty()), rgba))
    }

    /// Картинка и цвет материала по id, с памятью.
    fn material(&mut self, ident: &str) -> Result<(Option<String>, [f64; 4]), ModelError> {
        if let Some(found) = self.materials.get(ident) {
            return Ok(found.clone());
        }
        let mut found = (None, GRAY);
        if let Some(node) = self.ids.get(ident).filter(|n| n.tag == "material") {
            let effect = node.child("instance_effect").and_then(|i| self.find(i.get("url").unwrap_or_default()));
            if let Some(effect) = effect {
                found = self.effect(effect)?;
            }
        }
        self.materials.insert(ident.to_string(), found.clone());
        Ok(found)
    }
}

/// RGBA 0-1 из ребёнка color или None.
fn color(node: Option<&Node>) -> Result<Option<[f64; 4]>, ModelError> {
    let Some(item) = node.and_then(|n| n.child("color")) else {
        return Ok(None);
    };
    let values = numbers(Some(item))?;
    if values.len() < 3 {
        return Ok(None);
    }
    let mut rgba = [1.0; 4];
    for (slot, v) in rgba.iter_mut().zip(values) {
        *slot = v.clamp(0.0, 1.0);
    }
    Ok(Some(rgba))
}

/// Треугольники примитива: положения, нормали или None, uv или None,
/// по вершине на угол.
type Triangles = (Vec<DVec3>, Option<Vec<DVec3>>, Option<Vec<DVec2>>);

fn primitive(doc: &mut Doc<'_>, prim: &Node) -> Result<Option<Triangles>, ModelError> {
    let mut inputs: Vec<(&str, usize, i64, &str)> = Vec::new();
    for item in prim.children_named("input") {
        let int = |name: &str| item.get(name).filter(|t| !t.is_empty()).map_or(Ok(0), |t| t.trim().parse::<i64>());
        let (Ok(offset), Ok(set)) = (int("offset"), int("set")) else {
            return Err(error("bad input"));
        };
        let offset = usize::try_from(offset).ok().filter(|o| *o < MAX_POLYGON).ok_or_else(|| error("bad input"))?;
        inputs.push((item.get("semantic").unwrap_or_default(), offset, set, item.get("source").unwrap_or_default()));
    }
    let Some(width) = inputs.iter().map(|i| i.1 + 1).max() else {
        return Ok(None);
    };
    // Номера углов по треугольникам, по width номеров на угол.
    let mut corners: Vec<i64> = Vec::new();
    let mut add = |corners: &mut Vec<i64>, count: usize| -> Result<(), ModelError> {
        doc.vertices = doc.vertices.saturating_add(count);
        if doc.vertices > MAX_VERTICES {
            return Err(error(format!("too many vertices: more than {MAX_VERTICES}")));
        }
        corners.reserve(count * width);
        Ok(())
    };
    match prim.tag.as_str() {
        "triangles" => {
            let p = integers(prim.child("p"))?;
            let count = p.len() / width / 3 * 3;
            add(&mut corners, count)?;
            corners.extend_from_slice(p.get(..count * width).unwrap_or_default());
        }
        "polylist" | "polygons" => {
            let pieces: Vec<Vec<i64>> = if prim.tag == "polylist" {
                let p = integers(prim.child("p"))?;
                let mut pieces = Vec::new();
                let mut at = 0usize;
                for n in integers(prim.child("vcount"))? {
                    let n = usize::try_from(n).map_err(|_| error("bad vcount"))?;
                    let start = at.saturating_mul(width).min(p.len());
                    let end = at.saturating_add(n).saturating_mul(width).min(p.len());
                    pieces.push(p.get(start..end).unwrap_or_default().to_vec());
                    at = at.saturating_add(n);
                }
                pieces
            } else {
                prim.children_named("p").map(|p| integers(Some(p))).collect::<Result<_, _>>()?
            };
            for piece in pieces {
                let n = piece.len() / width;
                if !(3..=MAX_POLYGON).contains(&n) {
                    continue;
                }
                add(&mut corners, 3 * (n - 2))?;
                let corner = |k: usize| piece.get(k * width..(k + 1) * width).unwrap_or_default();
                for k in 1..n - 1 {
                    corners.extend_from_slice(corner(0));
                    corners.extend_from_slice(corner(k));
                    corners.extend_from_slice(corner(k + 1));
                }
            }
        }
        _ => return Ok(None),
    }
    if corners.is_empty() {
        return Ok(None);
    }
    let column = |offset: usize| corners.chunks_exact(width).map(move |row| row.get(offset).copied().unwrap_or(-1));
    let (mut positions, mut normals, mut uv) = (None, None, None);
    let mut texcoord_set: Option<i64> = None;
    for (semantic, offset, set, source) in inputs {
        match semantic {
            "VERTEX" => {
                let data = doc.source(source)?;
                let rows: Result<Vec<DVec3>, _> = column(offset).map(|i| data.row(i, 3).map(DVec3::from_slice)).collect();
                positions = Some(rows?);
            }
            "NORMAL" if normals.is_none() => {
                let data = doc.source(source)?;
                let rows: Result<Vec<DVec3>, _> = column(offset).map(|i| data.row(i, 3).map(DVec3::from_slice)).collect();
                normals = Some(rows?);
            }
            "TEXCOORD" if texcoord_set.is_none_or(|s| set < s) => {
                let data = doc.source(source)?;
                let rows: Result<Vec<DVec2>, _> =
                    column(offset).map(|i| data.row(i, 2).map(|r| DVec2::new(r[0], 1.0 - r[1]))).collect();
                uv = Some(rows?);
                texcoord_set = Some(set);
            }
            _ => {}
        }
    }
    let positions = positions.ok_or_else(|| error("primitive without VERTEX"))?;
    Ok(Some((positions, normals, uv)))
}

/// Матрица 4×4 узла по его преобразованиям в порядке записи.
fn transform(node: &Node) -> Result<DMat4, ModelError> {
    let mut out = DMat4::IDENTITY;
    for item in &node.children {
        if !matches!(item.tag.as_str(), "matrix" | "translate" | "rotate" | "scale") {
            continue;
        }
        let v = numbers(Some(item))?;
        let step = match (item.tag.as_str(), v.as_slice()) {
            // Матрица Collada записана по строкам.
            ("matrix", values) if values.len() == 16 => {
                let mut cols = [0.0; 16];
                cols.copy_from_slice(values);
                DMat4::from_cols_array(&cols).transpose()
            }
            ("translate", [x, y, z]) => DMat4::from_translation(DVec3::new(*x, *y, *z)),
            ("scale", [x, y, z]) => DMat4::from_scale(DVec3::new(*x, *y, *z)),
            ("rotate", [x, y, z, angle]) => DMat4::from_mat3(rotation(DVec3::new(*x, *y, *z), *angle)),
            _ => continue,
        };
        out *= step;
    }
    Ok(out)
}

/// Поворот на degrees вокруг оси axis, нулевая ось - без поворота.
fn rotation(axis: DVec3, degrees: f64) -> DMat3 {
    let length = axis.length();
    if length == 0.0 {
        return DMat3::IDENTITY;
    }
    let DVec3 { x, y, z } = axis / length;
    let (s, c) = degrees.to_radians().sin_cos();
    let t = 1.0 - c;
    rows(
        [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
    )
}

/// Символ материала - id материала из bind_material.
fn bindings(instance: &Node) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let common = instance.child("bind_material").and_then(|b| b.child("technique_common"));
    for item in common.into_iter().flat_map(|c| c.children_named("instance_material")) {
        out.insert(item.get("symbol").unwrap_or_default().to_string(), reference(item.get("target").unwrap_or_default()).to_string());
    }
    out
}

fn walk<'a>(doc: &mut Doc<'a>, node: &'a Node, matrix: DMat4, out: &mut Vec<Part>, depth: usize) -> Result<(), ModelError> {
    if depth > MAX_DEPTH {
        return Err(error("node nesting too deep"));
    }
    doc.visits += 1;
    if doc.visits > MAX_NODES {
        return Err(error("too many nodes"));
    }
    let matrix = matrix * transform(node)?;
    for item in &node.children {
        match item.tag.as_str() {
            "instance_geometry" => {
                if let Some(geometry) = doc.find(item.get("url").unwrap_or_default()) {
                    add_geometry(doc, geometry, &matrix, &bindings(item), out)?;
                }
            }
            "instance_node" => {
                if let Some(target) = doc.find(item.get("url").unwrap_or_default()) {
                    walk(doc, target, matrix, out, depth + 1)?;
                }
            }
            "node" => walk(doc, item, matrix, out, depth + 1)?,
            _ => {}
        }
    }
    Ok(())
}

fn add_geometry(
    doc: &mut Doc<'_>,
    geometry: &Node,
    matrix: &DMat4,
    bindings: &HashMap<String, String>,
    out: &mut Vec<Part>,
) -> Result<(), ModelError> {
    let Some(mesh) = geometry.child("mesh") else {
        return Ok(());
    };
    let linear = DMat3::from_mat4(*matrix);
    let det = linear.determinant();
    if det == 0.0 || !det.is_finite() {
        return Ok(());
    }
    // Нормали - обратной транспонированной, у зеркального узла она та
    // же, что у прямого, порядок углов разворачивается ниже.
    let normal_matrix = linear.inverse().transpose();
    let shift = matrix.w_axis.truncate();
    for prim in &mesh.children {
        if !matches!(prim.tag.as_str(), "triangles" | "polylist" | "polygons") {
            continue;
        }
        let Some((positions, normals, uv)) = primitive(doc, prim)? else {
            continue;
        };
        let mut positions: Vec<DVec3> = positions.into_iter().map(|p| linear * p + shift).collect();
        let mut normals: Option<Vec<DVec3>> = normals.map(|n| {
            n.into_iter()
                .map(|v| {
                    let v = normal_matrix * v;
                    let length = v.length();
                    if length > 0.0 { v / length } else { v }
                })
                .collect()
        });
        let mut uv = uv;
        if det < 0.0 {
            for tri in positions.as_chunks_mut::<3>().0 {
                tri.swap(0, 2);
            }
            for tri in normals.iter_mut().flat_map(|n| n.as_chunks_mut::<3>().0) {
                tri.swap(0, 2);
            }
            for tri in uv.iter_mut().flat_map(|n| n.as_chunks_mut::<3>().0) {
                tri.swap(0, 2);
            }
        }
        let symbol = prim.get("material").unwrap_or_default();
        let ident = bindings.get(symbol).map_or(symbol, String::as_str);
        let (image, rgba) = doc.material(ident)?;
        let uv = if image.is_some() { uv } else { None };
        out.push(Part::new(positions, normals, uv, image, rgba));
    }
    Ok(())
}

fn parse(data: &[u8]) -> Result<Node, ModelError> {
    xml::parse(data).map_err(|e| error(e.0))
}

/// Части модели из байтов .dae: метры, ось Z вверх.
pub fn read_dae(data: &[u8]) -> Result<Vec<Part>, ModelError> {
    let root = parse(data)?;
    if root.tag != "COLLADA" {
        return Err(error("not COLLADA"));
    }
    let mut doc = Doc::new(&root);
    let instance = root.child("scene").and_then(|s| s.child("instance_visual_scene"));
    let visual = instance.and_then(|i| doc.find(i.get("url").unwrap_or_default())).or_else(|| {
        root.child("library_visual_scenes").and_then(|l| l.child("visual_scene"))
    });
    let visual = visual.ok_or_else(|| error("no visual scene"))?;
    let mut out = Vec::new();
    for node in visual.children_named("node") {
        walk(&mut doc, node, DMat4::IDENTITY, &mut out, 0)?;
    }
    for part in &mut out {
        for p in &mut part.positions {
            *p *= doc.unit;
        }
    }
    if doc.y_up {
        out = out.into_iter().map(from_y_up).collect();
    }
    Ok(out)
}

/// Пути картинок файла .dae, как записаны в library_images.
pub fn images(data: &[u8]) -> Result<Vec<String>, ModelError> {
    let root = parse(data)?;
    let doc = Doc::new(&root);
    let mut out: Vec<String> = Vec::new();
    for node in root.iter().into_iter().filter(|n| n.tag == "image") {
        let Some(path) = node.get("id").and_then(|id| doc.image_path(id)) else {
            continue;
        };
        if !path.is_empty() && !out.contains(&path) {
            out.push(path);
        }
    }
    Ok(out)
}
