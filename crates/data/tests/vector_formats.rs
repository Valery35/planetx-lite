// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Векторные форматы слоёв: GeoJSON, Shapefile из нескольких файлов,
//! GPX, TopoJSON, GML, MapInfo, FlatGeobuf, DXF. Образцы пишет terrio
//! из одного GeoJSON, чтение идёт через [`data::layers::read`].

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Read;

use data::layers::{self, Shape};

const POLYGONS: &str = r#"{"type":"FeatureCollection","features":[
{"type":"Feature","properties":{"name":"Пруд","depth":3.5},"geometry":{"type":"Polygon","coordinates":[[[56.0,58.0],[56.1,58.0],[56.1,58.1],[56.0,58.1],[56.0,58.0]],[[56.02,58.02],[56.04,58.02],[56.04,58.04],[56.02,58.02]]]}},
{"type":"Feature","properties":{"name":"Луг","depth":0.5},"geometry":{"type":"Polygon","coordinates":[[[56.2,58.0],[56.3,58.0],[56.3,58.1],[56.2,58.0]]]}}
]}"#;

const POINTS: &str = r#"{"type":"FeatureCollection","features":[
{"type":"Feature","properties":{"name":"Скважина 1"},"geometry":{"type":"Point","coordinates":[56.25,58.01]}},
{"type":"Feature","properties":{"name":"Скважина 2"},"geometry":{"type":"Point","coordinates":[56.26,58.02]}}
]}"#;

/// Слои terrio из текста GeoJSON.
fn source(text: &str) -> Vec<terrio::Layer> {
    terrio::memory::open_vector_bytes(text.as_bytes().to_vec(), "source.geojson", &Default::default()).unwrap().layers
}

/// Файл формата name из слоёв GeoJSON text.
fn written(text: &str, name: &str) -> Vec<u8> {
    terrio::memory::write_vector_bytes(&source(text), name, &Default::default()).unwrap()
}

fn close(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
}

/// Файлы архива ZIP: имя и байты.
fn members(zip: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).unwrap();
        let name = f.name().unwrap().to_string();
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes).unwrap();
        out.push((name, bytes));
    }
    out
}

#[test]
fn file_kinds() {
    for name in ["a.gpkg", "B.SHP", "c.geojson", "d.gpx", "e.topojson", "f.gml", "g.mif", "h.tab", "i.fgb", "j.dxf", "roads.shp.zip"] {
        assert!(layers::is_vector(name), "{name}");
    }
    for name in ["a.kml", "b.kmz", "c.csv", "d.tif", "e.planetx", "f.dbf"] {
        assert!(!layers::is_vector(name), "{name}");
    }
    assert!(layers::is_sidecar("roads.DBF") && layers::is_sidecar("roads.prj") && layers::is_sidecar("roads.mid"));
    assert!(layers::needs_sidecars("roads.shp") && !layers::needs_sidecars("roads.shp.zip"));
    assert_eq!(layers::stem(r"C:\data\roads.shp"), "roads");
}

#[test]
fn geojson_polygons_with_holes_and_fields() {
    let l = layers::read(POLYGONS.as_bytes().to_vec(), "ponds.geojson").unwrap();
    assert_eq!(l.len(), 1);
    assert_eq!(l[0].fields, ["name", "depth"]);
    assert_eq!(l[0].numeric, [false, true]);
    let pond = &l[0].items[0];
    assert_eq!(pond.shape, Shape::Polygon);
    // Внешнее кольцо и дыра - две части, широта первой.
    assert_eq!(pond.parts.len(), 2);
    assert!(close(pond.parts[0][0], (58.0, 56.0)));
    assert_eq!(pond.values[0], "Пруд");
}

#[test]
fn shapefile_from_its_files() {
    let zip = written(POLYGONS, "ponds.shp.zip");
    let files = members(&zip);
    let (main_name, main) = files.iter().find(|(n, _)| n.ends_with(".shp")).unwrap().clone();
    // Архив .shp.zip открывается сразу.
    let whole = layers::read(zip.clone(), "ponds.shp.zip").unwrap();
    assert_eq!(whole[0].items.len(), 2);
    // Главный файл с соседними - те же поля и места.
    let around: Vec<(String, Vec<u8>)> = files.iter().filter(|(n, _)| n != &main_name).cloned().collect();
    let sidecars = layers::sidecars_of(&main_name, &around);
    assert!(sidecars.iter().any(|(n, _)| n.ends_with(".dbf")));
    let (bytes, name) = layers::bundle(&main, &main_name, &sidecars).unwrap();
    let l = layers::read(bytes, &name).unwrap();
    assert_eq!(l[0].fields, ["name", "depth"]);
    assert_eq!(l[0].items[0].values[0], "Пруд");
    assert!(close(l[0].items[0].parts[0][0], (58.0, 56.0)) || close(l[0].items[0].parts[0][0], (58.1, 56.0)));
    // Без .dbf полей нет, места те же.
    let (bytes, name) = layers::bundle(&main, &main_name, &[]).unwrap();
    let bare = layers::read(bytes, &name).unwrap();
    assert!(bare[0].fields.is_empty());
    assert_eq!(bare[0].items.len(), 2);
}

#[test]
fn sidecars_match_by_name_without_case() {
    let files = vec![("ROADS.DBF".to_string(), vec![1]), ("roads.prj".to_string(), vec![2]), ("rivers.dbf".to_string(), vec![3]), ("roads.txt".to_string(), vec![4])];
    let found: Vec<&str> = layers::sidecars_of("roads.shp", &files).iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(found, ["ROADS.DBF", "roads.prj"]);
}

#[test]
fn other_formats_round_trip() {
    for name in ["ponds.fgb", "ponds.topojson", "ponds.gml", "ponds.dxf"] {
        // GML terrio пишет архивом вместе со схемой .xsd.
        let mut bytes = written(POLYGONS, name);
        if bytes.starts_with(b"PK") {
            bytes = members(&bytes).into_iter().find(|(n, _)| n.ends_with(name.rsplit('.').next().unwrap_or_default())).unwrap().1;
        }
        let l = layers::read(bytes, name).unwrap_or_else(|e| panic!("{name}: {e}"));
        let items: Vec<_> = l.iter().flat_map(|x| x.items.iter()).filter(|i| i.shape == Shape::Polygon).collect();
        assert_eq!(items.len(), 2, "{name}");
        let first = items[0].parts[0][0];
        assert!((57.9..58.2).contains(&first.0) && (55.9..56.4).contains(&first.1), "{name}: {first:?}");
    }
    let gpx = layers::read(written(POINTS, "wells.gpx"), "wells.gpx").unwrap();
    let points: Vec<_> = gpx.iter().flat_map(|x| x.items.iter()).filter(|i| i.shape == Shape::Point).collect();
    assert_eq!(points.len(), 2);
    assert!(close(points[0].parts[0][0], (58.01, 56.25)));
}

#[test]
fn mapinfo_from_its_files() {
    // terrio пишет пару .mif и .mid архивом.
    let files = members(&written(POINTS, "wells.mif"));
    let (_, mif) = files.iter().find(|(n, _)| n.ends_with(".mif")).unwrap();
    let around: Vec<(String, Vec<u8>)> = files.iter().filter(|(n, _)| n.ends_with(".mid")).cloned().collect();
    let sidecars = layers::sidecars_of("wells.mif", &around);
    assert_eq!(sidecars.len(), 1);
    let (bytes, name) = layers::bundle(mif, "wells.mif", &sidecars).unwrap();
    let l = layers::read(bytes, &name).unwrap();
    assert_eq!(l[0].items.len(), 2);
    assert_eq!(l[0].items[0].values[0], "Скважина 1");
}

#[test]
fn drawing_in_metres_without_crs_is_refused() {
    let metres = r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{},"geometry":{"type":"LineString","coordinates":[[493200.0,6590150.0],[493300.0,6590250.0]]}}]}"#;
    let layers_src = terrio::memory::open_vector_bytes(metres.as_bytes().to_vec(), "m.geojson", &Default::default()).unwrap().layers;
    let dxf = terrio::memory::write_vector_bytes(&layers_src, "plan.dxf", &Default::default()).unwrap();
    assert_eq!(layers::read(dxf, "plan.dxf").unwrap_err(), layers::NO_CRS);
}
