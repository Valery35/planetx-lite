// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Чтение моделей Collada (.dae), как в Model KML Google Earth. Перенос
//! `core/collada.py` модуля. Анимация - каналы library_animations на
//! преобразования узлов по sid ([`crate::animation`]), BEZIER идёт как
//! LINEAR.
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

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use globe_core::{DMat3, DMat4, DVec2, DVec3};

use crate::animation::{self, Animation, Mode};
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
    /// Узлы обхода: id и преобразования, номер - по адресу узла.
    nodes: Vec<(Option<String>, Vec<Step>)>,
    node_numbers: HashMap<usize, usize>,
    /// id узлов с ключами анимации.
    moving: HashSet<String>,
    /// Дорожки: узлы от корня до сетки и W⁻¹ в покое.
    chains: BTreeMap<usize, Vec<usize>>,
    rest: BTreeMap<usize, DMat4>,
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
        Self {
            ids,
            unit,
            y_up,
            materials: HashMap::new(),
            sources: HashMap::new(),
            visits: 0,
            vertices: 0,
            nodes: Vec::new(),
            node_numbers: HashMap::new(),
            moving: HashSet::new(),
            chains: BTreeMap::new(),
            rest: BTreeMap::new(),
        }
    }

    /// Номер узла в [`Doc::nodes`], узел - один раз на адрес.
    fn node_index(&mut self, node: &Node) -> Result<usize, ModelError> {
        let address = std::ptr::from_ref(node) as usize;
        if let Some(found) = self.node_numbers.get(&address) {
            return Ok(*found);
        }
        let index = self.nodes.len();
        self.nodes.push((node.get("id").filter(|i| !i.is_empty()).map(str::to_string), steps(node)?));
        self.node_numbers.insert(address, index);
        Ok(index)
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

/// Преобразование узла: тег, sid и числа.
#[derive(Debug, Clone)]
struct Step {
    tag: String,
    sid: Option<String>,
    values: Vec<f64>,
}

/// Преобразования узла в порядке записи.
fn steps(node: &Node) -> Result<Vec<Step>, ModelError> {
    let mut out = Vec::new();
    for item in &node.children {
        if matches!(item.tag.as_str(), "matrix" | "translate" | "rotate" | "scale") {
            let sid = item.get("sid").filter(|s| !s.is_empty()).map(str::to_string);
            out.push(Step { tag: item.tag.clone(), sid, values: numbers(Some(item))? });
        }
    }
    Ok(out)
}

/// Числа преобразования на момент анимации: sid и числа в покое.
type Change<'a> = dyn Fn(&str, &[f64]) -> Option<Vec<f64>> + 'a;

/// Матрица 4×4 по преобразованиям. change(sid, числа) - числа
/// преобразования на момент анимации.
fn compose(steps: &[Step], change: &Change<'_>) -> DMat4 {
    let mut out = DMat4::IDENTITY;
    for step in steps {
        let changed = step.sid.as_deref().and_then(|sid| change(sid, &step.values));
        let v = changed.as_deref().unwrap_or(&step.values);
        let step = match (step.tag.as_str(), v) {
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
    out
}

/// Матрица 4×4 узла по его преобразованиям в порядке записи.
fn transform(node: &Node) -> Result<DMat4, ModelError> {
    Ok(compose(&steps(node)?, &|_, _| None))
}

/// Номер значения в преобразовании по имени: X, Y, Z, ANGLE.
fn selector(name: &str) -> Option<usize> {
    match name.to_ascii_uppercase().as_str() {
        "X" => Some(0),
        "Y" => Some(1),
        "Z" => Some(2),
        "ANGLE" => Some(3),
        _ => None,
    }
}

/// (id узла, sid, номер значения или None) из target канала:
/// «узел/sid», «узел/sid.X», «узел/sid(1)(2)».
fn target(text: &str) -> Option<(String, String, Option<usize>)> {
    let (path, sid) = text.split_once('/')?;
    let (sid, index) = if let Some((sid, name)) = sid.split_once('.') {
        (sid, Some(selector(name)?))
    } else if let Some((sid, rest)) = sid.split_once('(') {
        let numbers: Vec<usize> = rest.replace([')', '('], " ").split_whitespace().filter_map(|v| v.parse().ok()).collect();
        let index = match numbers.as_slice() {
            [] => return None,
            [row, column, ..] => row * 4 + column,
            [one] => *one,
        };
        (sid, Some(index))
    } else {
        (sid, None)
    };
    (!path.is_empty() && !sid.is_empty()).then(|| (path.to_string(), sid.to_string(), index))
}

/// Ключи канала: номер значения или None, времена, значения, способ.
type Key = (Option<usize>, Vec<f64>, Vec<Vec<f64>>, Mode);

/// Ключи всех каналов library_animations по (id узла, sid).
fn keys(doc: &mut Doc<'_>, root: &Node) -> HashMap<(String, String), Vec<Key>> {
    let mut out: HashMap<(String, String), Vec<Key>> = HashMap::new();
    for channel in root.iter().into_iter().filter(|n| n.tag == "channel") {
        let (Some((path, sid, index)), Some(sampler)) =
            (target(channel.get("target").unwrap_or_default()), doc.find(channel.get("source").unwrap_or_default()))
        else {
            continue;
        };
        let inputs: HashMap<&str, &str> =
            sampler.children_named("input").filter_map(|i| Some((i.get("semantic")?, i.get("source")?))).collect();
        let (Some(input), Some(output)) = (inputs.get("INPUT"), inputs.get("OUTPUT")) else {
            continue;
        };
        let (Ok(times), Ok(values)) = (doc.source(input), doc.source(output)) else {
            continue;
        };
        let times: Vec<f64> = times.values.chunks(times.stride).filter_map(|r| r.first().copied()).collect();
        let values: Vec<Vec<f64>> = values.values.chunks(values.stride).map(<[f64]>::to_vec).collect();
        if times.is_empty() || values.len() != times.len() || times.iter().any(|t| !t.is_finite()) {
            continue;
        }
        let names = inputs.get("INTERPOLATION").and_then(|i| doc.find(i)).and_then(|n| n.child("Name_array"));
        let step = names.and_then(|n| n.text.split_whitespace().next()) == Some("STEP");
        out.entry((path, sid)).or_default().push((index, times, values, if step { Mode::Step } else { Mode::Linear }));
    }
    out
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

/// Обход узла. chain - номера узлов в [`Doc::nodes`] от корня до
/// родителя: часть, у которой движется узел цепочки, получает дорожку.
fn walk<'a>(doc: &mut Doc<'a>, node: &'a Node, matrix: DMat4, out: &mut Vec<Part>, depth: usize, chain: &[usize]) -> Result<(), ModelError> {
    if depth > MAX_DEPTH {
        return Err(error("node nesting too deep"));
    }
    doc.visits += 1;
    if doc.visits > MAX_NODES {
        return Err(error("too many nodes"));
    }
    let matrix = matrix * transform(node)?;
    let mut chain = chain.to_vec();
    chain.push(doc.node_index(node)?);
    for item in &node.children {
        match item.tag.as_str() {
            "instance_geometry" => {
                if let Some(geometry) = doc.find(item.get("url").unwrap_or_default()) {
                    let start = out.len();
                    add_geometry(doc, geometry, &matrix, &bindings(item), out)?;
                    let moving = chain.iter().any(|n| doc.nodes.get(*n).and_then(|d| d.0.as_ref()).is_some_and(|id| doc.moving.contains(id)));
                    if out.len() > start && moving && matrix.determinant().abs() > 0.0 {
                        let track = doc.chains.len();
                        doc.chains.insert(track, chain.clone());
                        doc.rest.insert(track, matrix.inverse());
                        for part in out.iter_mut().skip(start) {
                            part.track = Some(track);
                        }
                    }
                }
            }
            "instance_node" => {
                if let Some(target) = doc.find(item.get("url").unwrap_or_default()) {
                    walk(doc, target, matrix, out, depth + 1, &chain)?;
                }
            }
            "node" => walk(doc, item, matrix, out, depth + 1, &chain)?,
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
    read_dae_animated(data).map(|(parts, _)| parts)
}

/// То же, что [`read_dae`], и анимация или None.
pub fn read_dae_animated(data: &[u8]) -> Result<(Vec<Part>, Option<Animation>), ModelError> {
    let root = parse(data)?;
    if root.tag != "COLLADA" {
        return Err(error("not COLLADA"));
    }
    let mut doc = Doc::new(&root);
    let keys = keys(&mut doc, &root);
    doc.moving = keys.keys().map(|(id, _)| id.clone()).collect();
    let instance = root.child("scene").and_then(|s| s.child("instance_visual_scene"));
    let visual = instance.and_then(|i| doc.find(i.get("url").unwrap_or_default())).or_else(|| {
        root.child("library_visual_scenes").and_then(|l| l.child("visual_scene"))
    });
    let visual = visual.ok_or_else(|| error("no visual scene"))?;
    let mut out = Vec::new();
    for node in visual.children_named("node") {
        walk(&mut doc, node, DMat4::IDENTITY, &mut out, 0, &[])?;
    }
    for part in &mut out {
        for p in &mut part.positions {
            *p *= doc.unit;
        }
    }
    if doc.y_up {
        out = out.into_iter().map(from_y_up).collect();
    }
    let moving = animation_of(doc, keys);
    Ok((out, moving))
}

/// Анимация по ключам и дорожкам обхода или None, `_animation` модуля.
fn animation_of(doc: Doc<'_>, keys: HashMap<(String, String), Vec<Key>>) -> Option<Animation> {
    if doc.chains.is_empty() {
        return None;
    }
    let duration = keys.values().flatten().filter_map(|(_, times, _, _)| times.last().copied()).fold(0.0, f64::max);
    let convert = animation::scaled_convert(doc.unit, doc.y_up);
    let nodes = doc.nodes;
    let local = move |index: usize, t: f64| {
        let Some((id, steps)) = nodes.get(index) else {
            return DMat4::IDENTITY;
        };
        let change = |sid: &str, values: &[f64]| -> Option<Vec<f64>> {
            let items = keys.get(&(id.clone()?, sid.to_string()))?;
            let mut values = values.to_vec();
            for (index, times, keyed, mode) in items {
                let Some(found) = animation::sample(times, keyed, t, *mode) else {
                    continue;
                };
                match index {
                    None if found.len() == values.len() => values = found,
                    Some(i) if *i < values.len() => {
                        if let (Some(slot), Some(v)) = (values.get_mut(*i), found.first()) {
                            *slot = *v;
                        }
                    }
                    _ => {}
                }
            }
            Some(values)
        };
        compose(steps, &change)
    };
    Some(Animation::new(duration, Arc::new(local), doc.chains, doc.rest, convert))
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
