// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сцена: запись, чтение, проверка файла. Перенос `tests/test_scene.py`
//! модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Cursor, Write};

use scene::scene::{self as sc, Places, Scene};
use serde_json::{Value, json};

fn sample() -> Scene {
    let mut scene = Scene::new(&[58.01, 56.23, 12000.0, 30.0, 55.0]);
    scene.time = json!({"mode": 1, "start": "2026-09-28T10:00:00", "end": "2026-09-28T10:20:00", "frame": 4, "step": 60.0})
        .as_object()
        .cloned();
    scene.layers = vec![
        json!({"name": "Скважины", "provider": "ogr", "source": "C:/data/wells.gpkg|layername=wells", "kind": "vector"})
            .as_object()
            .cloned()
            .unwrap(),
    ];
    scene.view = json!({"basemap": "Esri World Imagery", "relief": true, "scale": 2.0, "groups": ["borders", "places"], "language": "ru"})
        .as_object()
        .cloned()
        .unwrap();
    scene.places = "Маршрут".into();
    scene.name = "Пермь".into();
    scene
}

#[test]
fn round_trip_with_places() {
    let data = sc::write_scene(&sample(), Places::Kml("<kml/>")).unwrap();
    let (scene, kml) = sc::read_scene(&data).unwrap();
    assert_eq!(scene.camera, [58.01, 56.23, 12000.0, 30.0, 55.0]);
    assert_eq!(scene.time.as_ref().unwrap()["frame"], json!(4));
    assert_eq!(scene.layers[0]["name"], json!("Скважины"));
    assert_eq!(scene.view["groups"], json!(["borders", "places"]));
    assert_eq!((scene.places.as_str(), scene.name.as_str()), ("Маршрут", "Пермь"));
    assert_eq!(kml, b"<kml/>");
}

#[test]
fn without_places_and_time() {
    let (scene, kml) = sc::read_scene(&sc::write_scene(&Scene::new(&[0.0, 0.0, 1.0e6]), Places::None).unwrap()).unwrap();
    assert_eq!(scene.time, None);
    assert!(kml.is_empty());
}

#[test]
fn body() {
    let mut mars = Scene::new(&[18.65, -133.8, 1.2e7]);
    mars.body = "mars".into();
    let (scene, _) = sc::read_scene(&sc::write_scene(&mars, Places::None).unwrap()).unwrap();
    assert_eq!(scene.body, "mars");
    // Сцена до планет, без тела в камере, - земная.
    let mut old = sample().to_dict();
    old["camera"].as_object_mut().unwrap().remove("body");
    assert_eq!(Scene::from_dict(&old).unwrap().body, "earth");
}

#[test]
fn bad_files() {
    let pack = |payload: &str| -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer.start_file(sc::SCENE_FILE, zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(payload.as_bytes()).unwrap();
        writer.finish().unwrap().into_inner()
    };
    let mut newer = sample().to_dict();
    newer["format"] = Value::from(sc::FORMAT + 1);
    let mut far = sample().to_dict();
    far["camera"]["lat"] = Value::from(120.0);
    let cases = [
        b"not a zip".to_vec(),
        pack("{"),
        pack("[]"),
        pack(&newer.to_string()),
        pack(&far.to_string()),
        pack(r#"{"format": 1}"#),
    ];
    for data in cases {
        assert!(sc::read_scene(&data).is_err());
    }
}
