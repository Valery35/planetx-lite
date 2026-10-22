// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! 3D-модели: картинки Collada, место по KML, пакет файлов. Перенос
//! части `tests/test_models.py` модуля. Тесты чтения сеток Collada,
//! glTF и OBJ - фаза 3. Где тест модуля читал сетку ради картинки
//! модели, здесь картинка ищется в пакете тем же [`models::resolve`].

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::io::{Cursor, Write};

use globe_core::DVec3;
use globe_core::camera::enu;
use globe_core::ellipsoid::Ellipsoid;
use scene::models;
use scene::overlays::{self, Kind};
use scene::{kml, scene as sc};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n0000000000000000";

// Как пишет SketchUp: дюймы, Z вверх, узел со сдвигом, картинка через
// surface и sampler2D.
const DAE: &str = r##"<?xml version="1.0" encoding="utf-8"?>
<COLLADA xmlns="http://www.collada.org/2005/11/COLLADASchema"
 version="1.4.1">
<asset><unit meter="0.0254" name="inch"/><up_axis>Z_UP</up_axis></asset>
<library_images><image id="img0"><init_from>suzd1/texture0.jpg</init_from>
</image></library_images>
<library_effects>
<effect id="fx0"><profile_COMMON>
<newparam sid="surf"><surface type="2D"><init_from>img0</init_from>
</surface></newparam>
<newparam sid="samp"><sampler2D><source>surf</source></sampler2D>
</newparam>
<technique sid="COMMON"><lambert><diffuse>
<texture texture="samp" texcoord="UVSET0"/></diffuse></lambert>
</technique></profile_COMMON></effect>
<effect id="fx1"><profile_COMMON><technique sid="COMMON"><lambert>
<diffuse><color>1 0 0 1</color></diffuse>
<transparency><float>0.5</float></transparency>
</lambert></technique></profile_COMMON></effect>
</library_effects>
<library_materials>
<material id="m0"><instance_effect url="#fx0"/></material>
<material id="m1"><instance_effect url="#fx1"/></material>
</library_materials>
<library_geometries><geometry id="g"><mesh>
<source id="pos"><float_array id="pa" count="15">
0 0 0 100 0 0 100 100 0 0 100 0 0 0 100</float_array>
<technique_common><accessor source="#pa" count="5" stride="3">
<param name="X" type="float"/><param name="Y" type="float"/>
<param name="Z" type="float"/></accessor></technique_common></source>
<source id="nrm"><float_array id="na" count="3">0 0 1</float_array>
<technique_common><accessor source="#na" count="1" stride="3"/>
</technique_common></source>
<source id="uv"><float_array id="ua" count="8">0 0 1 0 1 1 0 1
</float_array><technique_common><accessor source="#ua" count="4"
 stride="2"/></technique_common></source>
<vertices id="v"><input semantic="POSITION" source="#pos"/></vertices>
<polylist count="1" material="Mat0">
<input semantic="VERTEX" source="#v" offset="0"/>
<input semantic="NORMAL" source="#nrm" offset="1"/>
<input semantic="TEXCOORD" source="#uv" offset="2" set="0"/>
<vcount>4</vcount><p>0 0 0 1 0 1 2 0 2 3 0 3</p></polylist>
<triangles count="1" material="Mat1">
<input semantic="VERTEX" source="#v" offset="0"/>
<p>0 1 4</p></triangles>
<lines count="1"><input semantic="VERTEX" source="#v" offset="0"/>
<p>0 1</p></lines>
</mesh></geometry></library_geometries>
<library_nodes><node id="ln"><instance_geometry url="#g">
<bind_material><technique_common>
<instance_material symbol="Mat0" target="#m0"/>
<instance_material symbol="Mat1" target="#m1"/>
</technique_common></bind_material></instance_geometry></node>
</library_nodes>
<library_visual_scenes><visual_scene id="s">
<node><matrix>1 0 0 1000 0 1 0 0 0 0 1 0 0 0 0 1</matrix>
<instance_node url="#ln"/></node>
</visual_scene></library_visual_scenes>
<scene><instance_visual_scene url="#s"/></scene>
</COLLADA>
"##;

fn close(a: f64, b: f64, places: i32) -> bool {
    (a - b).abs() < 0.5 * 10f64.powi(-places)
}

fn near(a: DVec3, b: DVec3) -> bool {
    (a - b).abs().max_element() <= 1e-12
}

#[test]
fn images_list() {
    assert_eq!(models::collada_images(DAE.as_bytes()).unwrap(), ["suzd1/texture0.jpg"]);
}

// Место модели.

#[test]
fn heading_turns_clockwise() {
    // Ось X модели (восток) при азимуте 90° смотрит на юг.
    let turned = models::rotation(90.0, 0.0, 0.0) * DVec3::X;
    assert!(near(turned, DVec3::new(0.0, -1.0, 0.0)), "{turned}");
    // Ось Y (север) - на восток.
    let turned = models::rotation(90.0, 0.0, 0.0) * DVec3::Y;
    assert!(near(turned, DVec3::X), "{turned}");
}

#[test]
fn frame_maps_model_axes_to_enu() {
    let (lat, lon) = (56.42, 40.44);
    let [east, north, up] = enu(lat, lon);
    let matrix = models::frame(lat, lon, 0.0, 0.0, 0.0, [2.0, 2.0, 2.0]);
    assert!(near(matrix * DVec3::Z, up * 2.0));
    assert!(near(matrix * DVec3::X, east * 2.0));
    assert!(near(matrix * DVec3::Y, north * 2.0));
}

#[test]
fn anchor_modes() {
    let e = Ellipsoid::wgs84();
    let height = |mode: &str, alt: f64| e.ecef_to_geodetic(models::anchor(10.0, 20.0, mode, alt, 150.0, 2.0)).2;
    assert!(close(height("clampToGround", 30.0), 150.0, 3));
    assert!(close(height("relativeToGround", 30.0), 180.0, 3));
    assert!(close(height("absolute", 100.0), 200.0, 3));
}

// Модель в KML и KMZ.

const MODEL_KML: &str = r#"<kml xmlns="http://www.opengis.net/kml/2.2"><Document>
<Placemark><name>Church</name><Model><altitudeMode>relativeToGround
</altitudeMode><Location><longitude>40.44</longitude><latitude>56.42
</latitude><altitude>2</altitude></Location><Orientation>
<heading>88.41885380513968</heading><tilt>0</tilt><roll>0</roll>
</Orientation><Scale><x>0.6721278848236247</x>
<y>1</y><z>1.5</z></Scale><Link><href>models/church.dae</href></Link>
<ResourceMap><Alias><targetHref>../images/t.png</targetHref>
<sourceHref>suzd1/texture0.jpg</sourceHref></Alias></ResourceMap>
</Model></Placemark></Document></kml>"#;

fn kmz(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in files {
        writer.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn read() -> kml::Folder {
    let data = kmz(&[
        ("doc.kml", MODEL_KML.as_bytes()),
        ("models/church.dae", DAE.as_bytes()),
        ("images/t.png", PNG),
        ("images/unused.png", PNG),
    ]);
    kml::read_file(&data, "x").unwrap()
}

/// Картинка модели в пакете, как её нашло бы чтение сетки.
fn picture(package: &[u8]) -> Vec<u8> {
    let p = models::unpack(package).unwrap();
    let path = models::resolve(&p.files, &p.main, "suzd1/texture0.jpg", &p.aliases).unwrap();
    models::file(&p.files, &path).unwrap().to_vec()
}

#[test]
fn kmz_model_with_resource_map() {
    let tree = read();
    let item = tree.overlays()[0];
    assert_eq!(item.kind(), Kind::Model);
    assert_eq!(item.name, "Church");
    assert_eq!(item.overlay.camera.unwrap()[..4], [56.42, 40.44, 2.0, 88.41885380513968]);
    assert_eq!(item.overlay.scale, [0.6721278848236247, 1.0, 1.5]);
    assert_eq!(item.overlay.mode, "relativeToGround");
    let image = item.image.as_deref().unwrap();
    assert_eq!(picture(image), PNG);
    // В пакет идёт только картинка по ссылке модели.
    assert_eq!(models::unpack(image).unwrap().files.len(), 2);
}

#[test]
fn round_trip() {
    let tree = read();
    let again = kml::read_file(&kml::write_kmz(&tree).unwrap(), "y").unwrap();
    let again = again.overlays()[0];
    let item = tree.overlays()[0];
    // Место без потери знаков: азимут и масштаб из KMZ Google Earth
    // с 16 знаками, запись {:g} оставляла 6.
    assert_eq!(again.overlay.params(), item.overlay.params());
    assert_eq!(picture(again.image.as_deref().unwrap()), PNG);
}

#[test]
fn scene_carries_model_files() {
    // Сцена с моделью - places.kmz с файлами модели. Прежде метки сцены
    // шли простым KML, и модель теряла файлы.
    let tree = read();
    let places = kml::write_kmz(&tree).unwrap();
    let data = sc::write_scene(&sc::Scene::new(&[56.42, 40.44, 300.0]), sc::Places::Kmz(&places)).unwrap();
    let archive = zip::ZipArchive::new(Cursor::new(data.as_slice())).unwrap();
    let names: Vec<String> = archive.file_names().map(|n| n.unwrap().into_owned()).collect();
    assert!(names.iter().any(|n| n == sc::KMZ_FILE));
    assert!(!names.iter().any(|n| n == sc::PLACES_FILE));
    let (_, places) = sc::read_scene(&data).unwrap();
    let tree = kml::read_file(&places, "z").unwrap();
    assert_eq!(picture(tree.overlays()[0].image.as_deref().unwrap()), PNG);
}

#[test]
fn plain_kml_keeps_href() {
    let tree = kml::read_kml(MODEL_KML.as_bytes(), "").unwrap();
    let item = tree.overlays()[0].clone();
    assert_eq!(item.image, None);
    assert_eq!(item.href, "models/church.dae");
    let aliases: BTreeMap<String, String> = [("suzd1/texture0.jpg".to_string(), "../images/t.png".to_string())].into();
    assert_eq!(item.aliases, aliases);
    let text = kml::write_kml(&kml::Folder::with("d", vec![kml::Item::Overlay(item)]));
    assert!(text.contains("<sourceHref>suzd1/texture0.jpg</sourceHref>"));
}

#[test]
fn overlay_params_round_trip() {
    let tree = read();
    let item = tree.overlays()[0];
    let back = overlays::from_params(Kind::Model, &item.overlay.params(), None);
    assert_eq!(back.camera, item.overlay.camera);
    assert_eq!(back.scale, item.overlay.scale);
    assert_eq!(back.mode, item.overlay.mode);
}

// OBJ с материалами MTL.

const OBJ: &str = "# square with a picture and a coloured triangle
mtllib walls.mtl
v 0 0 0
v 2 0 0
v 2 3 0
v 0 3 0
vt 0 0
vt 1 0
vt 1 1
vt 0 1
vn 0 0 1
usemtl brick
f 1/1/1 2/2/1 3/3/1 4/4/1
usemtl red
f -4//1 -3//1 -2//1
l 1 2
";
const MTL: &str = "newmtl brick
Kd 1 1 1
map_Kd -s 1 1 1 tex/brick.png
newmtl red
Kd 1 0 0
d 0.5
";

#[test]
fn package_from_files() {
    let fetch = |path: &str| -> Option<Vec<u8>> {
        match path {
            "walls.mtl" => Some(MTL.as_bytes().to_vec()),
            "tex/brick.png" => Some(PNG.to_vec()),
            _ => None,
        }
    };
    let data = models::collect("walls.obj", OBJ.as_bytes(), &fetch, &BTreeMap::new()).unwrap();
    let mut packed: Vec<String> = models::unpack(&data).unwrap().files.into_iter().map(|(n, _)| n).collect();
    packed.sort();
    assert_eq!(packed, ["tex/brick.png", "walls.mtl", "walls.obj"]);
}
