// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Слои сцен I3S 1.7 и новее - сервисы ArcGIS `SceneServer`, стандарт
//! OGC. Решение автора от 9 ноября 2026 года, взято из GeoLibre 3.7.0.
//! Слой ложится в то же дерево [`Tree`], что и 3D Tiles, выбор тайлов
//! и подгрузка общие.
//!
//! Слой - JSON `layers/N`: страницы узлов (`nodePages`), определения
//! геометрии, материалов и наборов текстур. Узлы приходят страницами
//! `nodepages/K` по `nodesPerPage` узлов, номер узла сквозной. Узел -
//! ориентированный объём (`obb`, центр - долгота, широта, высота), порог
//! уровня `lodThreshold`, дети и сетка: геометрия `nodes/R/geometries/B`
//! и текстура `nodes/R/textures/T`. Уровни переключаются: дети заменяют
//! узел, как REPLACE у 3D Tiles.
//!
//! Страницы просятся по мере надобности. Узел дерева получает детей,
//! когда пришли страницы всех его детей: до того рисуется он сам, дыр
//! нет. Порог `maxScreenThresholdSQ` - квадрат экранного размера узла
//! в пикселях. Ошибка тайла дерева подобрана так, что тайл уточняется
//! при том же размере на экране.
//!
//! Геометрия - буфер Draco (`compressedAttributes`) или несжатый буфер
//! «атрибут за атрибутом». В слоях на WGS84 вершины - смещения от центра
//! узла, x и y в градусах долготы и широты (у Draco - числа, умноженные
//! на `i3s-scale_x` и `i3s-scale_y` метаданных атрибута), z в метрах. Высоты слоя -
//! над геоидом, как высоты рельефа глобуса. Слои в проекции, текстуры
//! только DDS или KTX2 и Building Scene Layer пока не читаются.

use std::collections::{HashMap, HashSet};

use globe_core::DVec3;
use globe_core::ellipsoid::Ellipsoid;
use serde_json::Value;

use crate::models::{ModelError, Part, Pictures, error};
use crate::tiles3d::{Piece, TileNode, Tree, View};

/// Наибольшее количество узлов слоя в памяти.
pub const MAX_NODES: usize = 500_000;
/// Наибольшее количество вершин сетки узла.
pub const MAX_VERTICES: usize = 3_000_000;
/// Цвет сетки без материала.
const WHITE: [f64; 4] = [0.92, 0.92, 0.92, 1.0];

/// Определение геометрии: номер буфера и сжат ли он Draco.
#[derive(Debug, Clone, PartialEq)]
struct Geometry {
    buffer: usize,
    draco: bool,
    /// У несжатого буфера: есть ли нормали и текстурные координаты.
    normals: bool,
    uv: bool,
}

/// Определение материала: имя текстуры JPEG или PNG и цвет.
#[derive(Debug, Clone, PartialEq)]
struct Material {
    texture: Option<String>,
    rgba: [f64; 4],
}

/// Описание слоя.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub name: String,
    /// Тип слоя: 3DObject, IntegratedMesh, Building.
    pub kind: String,
    /// Охват слоя: запад, юг, восток, север в градусах, если он указан.
    pub extent: Option<[f64; 4]>,
    per_page: usize,
    root: usize,
    /// Порог - квадрат размера (`maxScreenThresholdSQ`) или размер.
    squared: bool,
    geometries: Vec<Geometry>,
    materials: Vec<Material>,
}

/// Сетка узла: определение и ресурс геометрии, материал и его ресурс.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Mesh {
    geometry: usize,
    resource: usize,
    material: Option<usize>,
    texture_resource: usize,
}

/// Узел страницы: центр (долгота, широта, высота), радиус, порог, дети,
/// сетка.
#[derive(Debug, Clone, PartialEq)]
struct Node {
    center: [f64; 3],
    radius: f64,
    threshold: Option<f64>,
    children: Vec<usize>,
    mesh: Option<Mesh>,
}

/// Это ли описание слоя I3S: JSON со страницами узлов или `store`.
pub fn is_layer(data: &[u8]) -> bool {
    serde_json::from_slice::<Value>(data).is_ok_and(|v| v.get("nodePages").is_some() || (v.get("store").is_some() && v.get("layerType").is_some()))
}

/// Это ли описание сервиса `SceneServer`: JSON со списком слоёв.
pub fn service_layers(data: &[u8]) -> Vec<usize> {
    let Ok(v) = serde_json::from_slice::<Value>(data) else {
        return Vec::new();
    };
    v.get("layers").and_then(Value::as_array).into_iter().flatten().filter_map(|l| l.get("id")?.as_u64().and_then(|i| usize::try_from(i).ok())).collect()
}

fn count(v: Option<&Value>) -> Option<usize> {
    v?.as_u64().and_then(|n| usize::try_from(n).ok())
}

/// Описание слоя из JSON `layers/N`.
pub fn layer(data: &[u8]) -> Result<Layer, ModelError> {
    let v: Value = serde_json::from_slice(data).map_err(|_| error("bad I3S layer JSON"))?;
    let kind = v.get("layerType").and_then(Value::as_str).unwrap_or_default().to_string();
    if kind.eq_ignore_ascii_case("building") {
        return Err(error("I3S Building Scene Layer is not supported yet"));
    }
    let wkid = v.get("spatialReference").and_then(|s| s.get("wkid")).and_then(Value::as_u64).unwrap_or(4326);
    if wkid != 4326 && wkid != 4979 {
        return Err(error(format!("I3S layer in a projected system (wkid {wkid}) is not supported yet")));
    }
    let pages = v.get("nodePages").ok_or_else(|| error("I3S layer without node pages, version 1.7 or newer is needed"))?;
    let per_page = count(pages.get("nodesPerPage")).filter(|n| (1..=100_000).contains(n)).ok_or_else(|| error("bad nodesPerPage"))?;
    let root = count(pages.get("rootIndex")).unwrap_or(0);
    let squared = pages.get("lodSelectionMetricType").and_then(Value::as_str).is_none_or(|m| m == "maxScreenThresholdSQ");
    let list = |key: &str| v.get(key).and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    let geometries = list("geometryDefinitions")
        .iter()
        .map(|g| {
            let buffers = g.get("geometryBuffers").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
            let draco = buffers.iter().position(|b| b.get("compressedAttributes").and_then(|c| c.get("encoding")).and_then(Value::as_str) == Some("draco"));
            let first = buffers.first();
            let has = |key: &str| first.is_some_and(|b| b.get(key).is_some());
            match draco {
                Some(buffer) => Geometry { buffer, draco: true, normals: false, uv: true },
                None => Geometry { buffer: 0, draco: false, normals: has("normal"), uv: has("uv0") },
            }
        })
        .collect();
    let sets: Vec<Option<String>> = list("textureSetDefinitions")
        .iter()
        .map(|t| {
            let formats = t.get("formats").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
            formats.iter().find(|f| matches!(f.get("format").and_then(Value::as_str), Some("jpg" | "png"))).and_then(|f| f.get("name")?.as_str()).map(str::to_string)
        })
        .collect();
    let materials = list("materialDefinitions")
        .iter()
        .map(|m| {
            let pbr = m.get("pbrMetallicRoughness");
            let texture = pbr.and_then(|p| count(p.get("baseColorTexture")?.get("textureSetDefinitionId"))).and_then(|i| sets.get(i).cloned().flatten());
            let factor: Vec<f64> = pbr.and_then(|p| p.get("baseColorFactor")?.as_array()).map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
            let rgba = match factor.as_slice() {
                [r, g, b, a] => [*r, *g, *b, *a].map(|c| c.clamp(0.0, 1.0)),
                [r, g, b] => [*r, *g, *b, 1.0].map(|c| c.clamp(0.0, 1.0)),
                _ => WHITE,
            };
            Material { texture, rgba }
        })
        .collect();
    let extent = v.get("store").and_then(|s| s.get("extent")).and_then(Value::as_array).and_then(|a| {
        let e: Vec<f64> = a.iter().filter_map(Value::as_f64).collect();
        let e = <[f64; 4]>::try_from(e).ok()?;
        (e.iter().all(|x| x.is_finite()) && e[1].abs() <= 90.0 && e[3].abs() <= 90.0 && e[0] <= e[2] && e[1] <= e[3]).then_some(e)
    });
    Ok(Layer { name: v.get("name").and_then(Value::as_str).unwrap_or_default().to_string(), kind, extent, per_page, root, squared, geometries, materials })
}

/// Узлы страницы page: номер узла и узел. Враждебная страница - ошибка.
fn page(data: &[u8], first: usize) -> Result<Vec<(usize, Node)>, ModelError> {
    let v: Value = serde_json::from_slice(data).map_err(|_| error("bad I3S node page"))?;
    let nodes = v.get("nodes").and_then(Value::as_array).ok_or_else(|| error("I3S node page without nodes"))?;
    let mut out = Vec::with_capacity(nodes.len());
    for (k, n) in nodes.iter().enumerate() {
        let index = count(n.get("index")).unwrap_or(first + k);
        let floats = |key: &str, at: &Value| -> Vec<f64> { at.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default() };
        let (center, radius) = if let Some(obb) = n.get("obb") {
            let c = floats("center", obb);
            let h = floats("halfSize", obb);
            match (c.as_slice(), h.as_slice()) {
                ([x, y, z], [a, b, d]) => ([*x, *y, *z], (a * a + b * b + d * d).sqrt()),
                _ => return Err(error("bad I3S obb")),
            }
        } else if let Some(mbs) = n.get("mbs").and_then(Value::as_array) {
            match mbs.iter().filter_map(Value::as_f64).collect::<Vec<_>>().as_slice() {
                [x, y, z, r] => ([*x, *y, *z], *r),
                _ => return Err(error("bad I3S mbs")),
            }
        } else {
            return Err(error("I3S node without a bounding volume"));
        };
        if !(center.iter().all(|c| c.is_finite()) && radius.is_finite() && radius >= 0.0 && center[1].abs() <= 90.0) {
            return Err(error("bad I3S node volume"));
        }
        let children = n.get("children").and_then(Value::as_array).map(|a| a.iter().filter_map(|c| count(Some(c))).collect()).unwrap_or_default();
        let mesh = n.get("mesh").and_then(|m| {
            let g = m.get("geometry")?;
            let material = m.get("material");
            Some(Mesh {
                geometry: count(g.get("definition")).unwrap_or(0),
                resource: count(g.get("resource"))?,
                material: material.and_then(|x| count(x.get("definition"))),
                texture_resource: material.and_then(|x| count(x.get("resource"))).unwrap_or(0),
            })
        });
        let threshold = n.get("lodThreshold").and_then(Value::as_f64).filter(|t| t.is_finite() && *t > 0.0);
        out.push((index, Node { center, radius, threshold, children, mesh }));
    }
    Ok(out)
}

/// Слой на глобусе: узлы пришедших страниц и их места в дереве.
pub struct Index {
    pub layer: Layer,
    ellipsoid: Ellipsoid,
    /// Порог экранной ошибки дерева, пиксели.
    threshold: f64,
    nodes: HashMap<usize, Node>,
    /// Номер узла I3S тайла дерева.
    of_tile: Vec<usize>,
    /// Узлы I3S, у которых дети уже в дереве.
    expanded: HashSet<usize>,
    /// Страницы, которые попрошены или пришли.
    asked: HashSet<usize>,
}

impl Index {
    /// Слой layer, дерево уточняет тайлы выше threshold пикселей.
    pub fn new(layer: Layer, ellipsoid: Ellipsoid, threshold: f64) -> Self {
        Self { layer, ellipsoid, threshold, nodes: HashMap::new(), of_tile: Vec::new(), expanded: HashSet::new(), asked: HashSet::new() }
    }

    /// Адрес страницы page от папки слоя.
    pub fn page_path(page: usize) -> String {
        format!("nodepages/{page}")
    }

    /// Страница узла index.
    fn page_of(&self, index: usize) -> usize {
        index / self.layer.per_page
    }

    /// Страницы, которые нужны дереву на этот вид: страница корня, пока
    /// дерево пусто, потом страницы детей тайлов, которые надо уточнить.
    /// Страниц не больше limit, попрошенные второй раз не просятся.
    pub fn wanted(&mut self, tree: &Tree, view: &View<'_>, limit: usize) -> Vec<usize> {
        let mut out = Vec::new();
        if tree.nodes.is_empty() {
            out.push(self.page_of(self.layer.root));
        } else {
            for (t, node) in tree.nodes.iter().enumerate() {
                let Some(&i) = self.of_tile.get(t) else {
                    continue;
                };
                if self.expanded.contains(&i) || !(view.visible)(node.center, node.radius) || tree.sse(t, view) <= self.threshold {
                    continue;
                }
                let Some(n) = self.nodes.get(&i) else {
                    continue;
                };
                out.extend(n.children.iter().filter(|c| !self.nodes.contains_key(c)).map(|c| c / self.layer.per_page));
            }
        }
        out.sort_unstable();
        out.dedup();
        out.retain(|p| !self.asked.contains(p));
        out.truncate(limit);
        self.asked.extend(out.iter().copied());
        out
    }

    /// Ошибка тайла дерева у узла: тайл уточняется на экране того же
    /// размера, что и узел I3S по своему порогу.
    fn tile_error(&self, node: &Node) -> f64 {
        let Some(t) = node.threshold else {
            return f64::INFINITY;
        };
        // Порог SQ - площадь узла на экране, поперечник круга той же
        // площади - sqrt(4t/π).
        let size = if self.layer.squared { (4.0 * t / std::f64::consts::PI).sqrt() } else { t };
        2.0 * node.radius * self.threshold / size.max(1e-9)
    }

    fn tile(&self, node: &Node, depth: usize) -> TileNode {
        let [lon, lat, h] = node.center;
        let uri = node.mesh.and_then(|m| self.layer.geometries.get(m.geometry).map(|g| format!("nodes/{}/geometries/{}", m.resource, g.buffer)));
        TileNode {
            uri,
            transform: globe_core::DMat4::IDENTITY,
            error: self.tile_error(node),
            add: false,
            children: Vec::new(),
            center: self.ellipsoid.geodetic_to_ecef(lat, lon, h),
            radius: node.radius,
            depth,
        }
    }

    /// Принять страницу page: узлы - в память, тайлы, у которых пришли
    /// все дети, получают детей в дереве. Тайлов дерево не теряет.
    pub fn add_page(&mut self, tree: &mut Tree, page_number: usize, data: &[u8]) -> Result<(), ModelError> {
        let first = page_number.saturating_mul(self.layer.per_page);
        for (index, node) in page(data, first)? {
            if self.nodes.len() >= MAX_NODES {
                return Err(error("too many I3S nodes"));
            }
            self.nodes.insert(index, node);
        }
        if tree.nodes.is_empty() {
            let Some(root) = self.nodes.get(&self.layer.root) else {
                return Err(error("I3S root node is missing"));
            };
            tree.nodes.push(self.tile(root, 0));
            tree.root = 0;
            self.of_tile.push(self.layer.root);
        }
        // Дети ложатся под тайл, когда пришли все: иначе выбор показал бы
        // часть детей вместо узла, и в слое была бы дыра.
        let mut t = 0;
        while t < tree.nodes.len() {
            let Some(&i) = self.of_tile.get(t) else {
                break;
            };
            if !self.expanded.contains(&i)
                && let Some(node) = self.nodes.get(&i)
                && !node.children.is_empty()
                && node.children.iter().all(|c| self.nodes.contains_key(c))
                && tree.nodes.get(t).is_some_and(|n| n.depth < crate::tiles3d::MAX_DEPTH)
            {
                let depth = tree.nodes.get(t).map_or(0, |n| n.depth) + 1;
                let kids: Vec<usize> = node.children.clone();
                for c in kids {
                    if let Some(child) = self.nodes.get(&c) {
                        let tile = self.tile(child, depth);
                        tree.nodes.push(tile);
                        self.of_tile.push(c);
                        let at = tree.nodes.len() - 1;
                        if let Some(parent) = tree.nodes.get_mut(t) {
                            parent.children.push(at);
                        }
                    }
                }
                self.expanded.insert(i);
            }
            t += 1;
        }
        Ok(())
    }

    /// Путь текстуры тайла t от папки слоя, у сетки без текстуры - None.
    pub fn texture_path(&self, t: usize) -> Option<String> {
        let mesh = self.nodes.get(self.of_tile.get(t)?)?.mesh?;
        let name = self.layer.materials.get(mesh.material?)?.texture.clone()?;
        Some(format!("nodes/{}/textures/{name}", mesh.texture_resource))
    }

    /// Кусок тайла t из байтов геометрии и картинки текстуры: части
    /// в ECEF.
    pub fn piece(&self, t: usize, geometry: &[u8], texture: Option<Vec<u8>>) -> Result<Piece, ModelError> {
        let i = *self.of_tile.get(t).ok_or_else(|| error("no such tile"))?;
        let node = self.nodes.get(&i).ok_or_else(|| error("no such node"))?;
        let mesh = node.mesh.ok_or_else(|| error("node has no mesh"))?;
        let def = self.layer.geometries.get(mesh.geometry).ok_or_else(|| error("bad geometry definition"))?;
        let (positions, uv) = if def.draco { draco(geometry)? } else { plain(geometry, def.normals, def.uv)? };
        let [lon, lat, h] = node.center;
        let e = &self.ellipsoid;
        let points: Vec<DVec3> = positions.iter().map(|p| e.geodetic_to_ecef(lat + p.y, lon + p.x, h + p.z)).collect();
        let material = mesh.material.and_then(|m| self.layer.materials.get(m));
        let rgba = material.map_or(WHITE, |m| m.rgba);
        let mut pictures = Pictures::new();
        let (uv, image) = match (texture, uv) {
            (Some(bytes), Some(uv)) => {
                let key = format!("i3s:{}", mesh.texture_resource);
                pictures.insert(key.clone(), bytes);
                (Some(uv), Some(key))
            }
            _ => (None, None),
        };
        Ok(Piece { name: format!("node {i}"), parts: vec![Part::new(points, None, uv, image, rgba)], pictures })
    }
}

/// Вершины треугольников подряд и текстурные координаты из буфера Draco.
fn draco(data: &[u8]) -> Result<(Vec<DVec3>, Option<Vec<globe_core::DVec2>>), ModelError> {
    use draco_core::GeometryAttributeType as Kind;
    let top = MAX_VERTICES as u64;
    let limits = draco_core::DecodeLimits::default().with_max_points(top).with_max_faces(top).with_max_decoded_bytes(top.saturating_mul(64));
    let mut buffer = draco_core::DecoderBuffer::new(data).with_limits(limits);
    let mut mesh = draco_core::Mesh::new();
    draco_core::MeshDecoder::new().decode(&mut buffer, &mut mesh).map_err(|e| error(format!("Draco: {e}")))?;
    let points = mesh.num_points();
    let position = mesh.named_attribute(Kind::Position).ok_or_else(|| error("Draco mesh without positions"))?;
    let xyz = position.read_f32s(points, 3);
    // Смещения x и y хранятся числами, градусы - они же, умноженные
    // на масштаб из метаданных атрибута, как пишет ArcGIS.
    let scale = |name: &str| {
        mesh.metadata().and_then(|m| m.attribute_metadata_by_unique_id(position.unique_id())).and_then(|a| a.metadata().get_f64(name)).filter(|s| s.is_finite() && *s != 0.0).unwrap_or(1.0)
    };
    let (sx, sy) = (scale("i3s-scale_x"), scale("i3s-scale_y"));
    let uvs = mesh.named_attribute(Kind::TexCoord).map(|a| a.read_f32s(points, 2));
    let order: Vec<usize> = mesh.faces().iter().flat_map(|f| f.iter().map(|p| p.0 as usize)).collect();
    if order.len() > MAX_VERTICES {
        return Err(error(format!("too many vertices: more than {MAX_VERTICES}")));
    }
    let at3 = |i: usize| -> Option<DVec3> { Some(DVec3::new(f64::from(*xyz.get(3 * i)?) * sx, f64::from(*xyz.get(3 * i + 1)?) * sy, f64::from(*xyz.get(3 * i + 2)?))) };
    let positions: Option<Vec<DVec3>> = order.iter().map(|i| at3(*i)).collect();
    let positions = positions.ok_or_else(|| error("Draco index out of range"))?;
    let uv = uvs.and_then(|t| order.iter().map(|i| Some(globe_core::DVec2::new(f64::from(*t.get(2 * i)?), f64::from(*t.get(2 * i + 1)?)))).collect::<Option<Vec<_>>>());
    Ok((positions, uv))
}

/// Вершины и текстурные координаты несжатого буфера: число вершин
/// и признаков (по 4 байта), затем все позиции float32 × 3, все нормали × 3,
/// если они есть, и все координаты × 2. Вершины уже идут треугольниками.
fn plain(data: &[u8], normals: bool, has_uv: bool) -> Result<(Vec<DVec3>, Option<Vec<globe_core::DVec2>>), ModelError> {
    let word = |at: usize| data.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let float = |at: usize| data.get(at..at + 4).map(|b| f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]])));
    let n = word(0).ok_or_else(|| error("short I3S geometry"))? as usize;
    if n > MAX_VERTICES || !n.is_multiple_of(3) {
        return Err(error("bad I3S vertex count"));
    }
    let start = 8usize;
    let positions: Option<Vec<DVec3>> = (0..n).map(|i| Some(DVec3::new(float(start + 12 * i)?, float(start + 12 * i + 4)?, float(start + 12 * i + 8)?))).collect();
    let positions = positions.ok_or_else(|| error("short I3S geometry"))?;
    let uv_at = start + 12 * n + if normals { 12 * n } else { 0 };
    let uv: Option<Vec<globe_core::DVec2>> = if has_uv { (0..n).map(|i| Some(globe_core::DVec2::new(float(uv_at + 8 * i)?, float(uv_at + 8 * i + 4)?))).collect() } else { None };
    Ok((positions, uv))
}
