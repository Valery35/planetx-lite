// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Слои сцен I3S: описание слоя, страницы узлов, дети без дыр, сетка
//! Draco в ECEF.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use globe_core::DVec3;
use globe_core::ellipsoid::Ellipsoid;
use scene::i3s::{self, Index};
use scene::tiles3d::{Tree, View};

/// Слой как у сервиса Esri: Draco во втором буфере, текстура JPEG.
const LAYER: &str = r#"{"name":"Bldgs","layerType":"3DObject","spatialReference":{"wkid":4326},
 "nodePages":{"nodesPerPage":2,"lodSelectionMetricType":"maxScreenThresholdSQ"},
 "textureSetDefinitions":[{"formats":[{"name":"0","format":"jpg"},{"name":"0_0_1","format":"dds"}]}],
 "materialDefinitions":[{"pbrMetallicRoughness":{"baseColorTexture":{"textureSetDefinitionId":0}}},{"pbrMetallicRoughness":{"baseColorFactor":[1,0,0,1]}}],
 "geometryDefinitions":[{"geometryBuffers":[{"offset":8,"position":{"type":"Float32","component":3}},{"compressedAttributes":{"encoding":"draco","attributes":["position"]}}]}]}"#;

/// Страница 0: корень без сетки с детьми 1 и 2, узел 1 с сеткой.
const PAGE0: &str = r#"{"nodes":[
 {"index":0,"obb":{"center":[37.6,55.75,150],"halfSize":[500,500,100],"quaternion":[0,0,0,1]},"children":[1,2]},
 {"index":1,"parentIndex":0,"lodThreshold":10000,"obb":{"center":[37.6,55.75,150],"halfSize":[50,50,10],"quaternion":[0,0,0,1]},
  "mesh":{"material":{"definition":0,"resource":7},"geometry":{"definition":0,"resource":7}}}]}"#;
/// Страница 1: узел 2 с красным материалом без текстуры.
const PAGE1: &str = r#"{"nodes":[
 {"index":2,"parentIndex":0,"lodThreshold":10000,"obb":{"center":[37.61,55.75,150],"halfSize":[50,50,10],"quaternion":[0,0,0,1]},
  "mesh":{"material":{"definition":1,"resource":8},"geometry":{"definition":0,"resource":8}}}]}"#;

fn view_near(e: &Ellipsoid) -> (DVec3, f64) {
    (e.geodetic_to_ecef(55.75, 37.6, 2000.0), 1000.0)
}

#[test]
fn layer_is_read_and_others_refused() {
    assert!(i3s::is_layer(LAYER.as_bytes()));
    assert!(!i3s::is_layer(br#"{"asset":{},"root":{}}"#));
    let layer = i3s::layer(LAYER.as_bytes()).unwrap();
    assert_eq!((layer.name.as_str(), layer.kind.as_str()), ("Bldgs", "3DObject"));
    let building = LAYER.replace("3DObject", "Building");
    assert!(i3s::layer(building.as_bytes()).unwrap_err().0.contains("Building"));
    let projected = LAYER.replace("\"wkid\":4326", "\"wkid\":3857");
    assert!(i3s::layer(projected.as_bytes()).unwrap_err().0.contains("3857"));
    assert!(i3s::layer(b"[1,2]").is_err());
    assert_eq!(i3s::service_layers(br#"{"layers":[{"id":0},{"id":3}]}"#), [0, 3]);
}

#[test]
fn children_join_the_tree_only_all_together() {
    let e = Ellipsoid::wgs84();
    let mut index = Index::new(i3s::layer(LAYER.as_bytes()).unwrap(), e, 16.0);
    let mut tree = Tree::default();
    let (eye, pixels) = view_near(&e);
    let always = |_: DVec3, _: f64| true;
    let view = View { eye, pixels, visible: &always };
    // Пустое дерево просит страницу корня, второй раз - нет.
    assert_eq!(index.wanted(&tree, &view, 6), [0]);
    assert!(index.wanted(&tree, &view, 6).is_empty());
    index.add_page(&mut tree, 0, PAGE0.as_bytes()).unwrap();
    // Узел 2 ещё не пришёл: у корня в дереве детей нет, дыры нет.
    assert_eq!(tree.nodes.len(), 1);
    assert!(tree.nodes[0].children.is_empty() && tree.nodes[0].uri.is_none());
    assert_eq!(index.wanted(&tree, &view, 6), [1]);
    index.add_page(&mut tree, 1, PAGE1.as_bytes()).unwrap();
    assert_eq!(tree.nodes[0].children, [1, 2]);
    assert_eq!(tree.nodes[1].uri.as_deref(), Some("nodes/7/geometries/1"));
    assert_eq!(index.texture_path(1).as_deref(), Some("nodes/7/textures/0"));
    assert_eq!(index.texture_path(2), None);
    // Центр тайла - центр объёма узла в ECEF.
    assert!((tree.nodes[2].center - e.geodetic_to_ecef(55.75, 37.61, 150.0)).length() < 1e-6);
    // Порог 10000 - площадь круга поперечником 112.8 пикселя: тайл радиуса
    // r уточняется, когда его поперечник на экране больше.
    let r = tree.nodes[1].radius;
    let size = (40000.0 / std::f64::consts::PI).sqrt();
    assert!((tree.nodes[1].error - 2.0 * r * 16.0 / size).abs() < 1e-9);
    // Корень без порога уточняется всегда.
    assert!(tree.nodes[0].error.is_infinite());
}

#[test]
fn bad_pages_are_errors_not_panics() {
    let e = Ellipsoid::wgs84();
    let mut index = Index::new(i3s::layer(LAYER.as_bytes()).unwrap(), e, 16.0);
    let mut tree = Tree::default();
    assert!(index.add_page(&mut tree, 0, b"{").is_err());
    assert!(index.add_page(&mut tree, 0, br#"{"nodes":[{"index":0,"obb":{"center":[0,95,0],"halfSize":[1,1,1]}}]}"#).is_err());
    assert!(index.add_page(&mut tree, 0, br#"{"nodes":[{"index":0}]}"#).is_err());
    // Страница без корня - ошибка, дерево пусто.
    assert!(index.add_page(&mut tree, 0, PAGE1.as_bytes()).is_err());
    assert!(tree.nodes.is_empty());
}

/// Сетка Draco из треугольников: вершины points, номера faces.
fn draco_stream(points: &[[f32; 3]], faces: &[[u32; 3]]) -> Vec<u8> {
    use draco_core::{DataType, EncoderBuffer, EncoderOptions, FaceIndex, GeometryAttributeType, Mesh, MeshEncoder, PointAttribute, PointIndex};
    let mut mesh = Mesh::new();
    let mut position = PointAttribute::new();
    position.init(GeometryAttributeType::Position, 3, DataType::Float32, false, points.len());
    for (i, p) in points.iter().enumerate() {
        let bytes: Vec<u8> = p.iter().flat_map(|v| v.to_le_bytes()).collect();
        position.buffer_mut().write(i * 12, &bytes);
    }
    mesh.add_attribute(position);
    mesh.set_num_faces(faces.len());
    for (i, f) in faces.iter().enumerate() {
        mesh.set_face(FaceIndex(i as u32), f.map(PointIndex));
    }
    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh);
    let mut out = EncoderBuffer::new();
    encoder.encode(&EncoderOptions::new(), &mut out).unwrap();
    out.data().to_vec()
}

#[test]
fn draco_offsets_are_degrees_and_metres() {
    let e = Ellipsoid::wgs84();
    let mut index = Index::new(i3s::layer(LAYER.as_bytes()).unwrap(), e, 16.0);
    let mut tree = Tree::default();
    index.add_page(&mut tree, 0, PAGE0.as_bytes()).unwrap();
    index.add_page(&mut tree, 1, PAGE1.as_bytes()).unwrap();
    // Смещения: 0.001° на восток, 0.001° на север, 20 м вверх.
    let stream = draco_stream(&[[0.0, 0.0, 0.0], [0.001, 0.0, 0.0], [0.0, 0.001, 20.0]], &[[0, 1, 2]]);
    let piece = index.piece(2, &stream, None).unwrap();
    let part = &piece.parts[0];
    assert_eq!(part.positions.len(), 3);
    assert_eq!(part.rgba, [1.0, 0.0, 0.0, 1.0]);
    let want = [e.geodetic_to_ecef(55.75, 37.61, 150.0), e.geodetic_to_ecef(55.75, 37.611, 150.0), e.geodetic_to_ecef(55.751, 37.61, 170.0)];
    for (got, want) in part.positions.iter().zip(want) {
        assert!((*got - want).length() < 0.05, "{got} {want}");
    }
    // Испорченный буфер - ошибка Draco.
    assert!(index.piece(2, &stream[..stream.len() / 2], None).unwrap_err().0.contains("Draco"));
    // Тайл без сетки - ошибка.
    assert!(index.piece(0, &stream, None).is_err());
}
