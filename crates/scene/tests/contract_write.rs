// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Расхождения Lite с контрактом сцены `planetx-scene`, сверка
//! 6 ноября 2026 года: время сцены и свои картинки значков в KMZ.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use scene::features::Kind;
use scene::kml::{self, Folder, Item, Place};
use scene::scene::{Places, Scene, read_scene, write_scene};
use serde_json::json;

/// Промежуток шкалы времени: новый вид и прежний вид контроллера QGIS.
#[test]
fn scene_time_both_forms() {
    let mut s = Scene::new(&[55.0, 37.0, 1000.0]);
    assert_eq!(s.span(), None);
    s.set_span(Some((0.0, 86_400.0)));
    let back = Scene::from_dict(&s.to_dict()).unwrap();
    assert_eq!(back.span(), Some((0.0, 86_400.0)));
    assert_eq!(back.to_dict()["time"], json!({"start": "1970-01-01T00:00:00Z", "end": "1970-01-02T00:00:00Z"}));
    // Прежний вид: анимация - промежуток кадра 2 по часу.
    let old = json!({"format": 1, "camera": {"lat": 0.0, "lon": 0.0, "distance": 1.0},
        "time": {"mode": 1, "start": "1970-01-01T00:00:00Z", "end": "1970-01-02T00:00:00Z", "frame": 2, "step": 3600.0}});
    assert_eq!(Scene::from_dict(&old).unwrap().span(), Some((7200.0, 10_800.0)));
    // Выключенный контроллер - без времени.
    let off = json!({"format": 1, "camera": {"lat": 0.0, "lon": 0.0, "distance": 1.0},
        "time": {"mode": 0, "start": "1970-01-01T00:00:00Z", "end": "1970-01-02T00:00:00Z"}});
    assert_eq!(Scene::from_dict(&off).unwrap().span(), None);
}

/// Тело, вид, время и слои хозяина переживают запись сцены.
#[test]
fn scene_keeps_body_view_time_layers() {
    let mut s = Scene::new(&[18.38, 77.58, 200_000.0, 0.0, 40.0]);
    s.body = "mars".into();
    s.view.insert("scale".into(), json!(3.0));
    // Ключ основы рядом с названием, решение 7 DECISIONS.md.
    s.view.insert("basemap".into(), json!("OpenStreetMap map"));
    s.view.insert("basemap_key".into(), json!("osm"));
    s.set_span(Some((0.0, 60.0)));
    s.layers.push(json!({"name": "dem", "provider": "gdal", "source": "dem.tif", "kind": "raster"}).as_object().unwrap().clone());
    let (back, _) = read_scene(&write_scene(&s, Places::None).unwrap()).unwrap();
    assert_eq!(back, s);
}

/// Своя картинка значка точки уходит в KMZ файлом и читается обратно.
/// Прежде Lite её терял, расхождение 4.
#[test]
fn icon_picture_survives_kmz() {
    let png = b"\x89PNG\r\n\x1a\nfake".to_vec();
    let mut tree = Folder::new("Значки");
    let mut a = Place::new("A", Kind::Point, vec![(1.0, 2.0)]);
    a.picture = Some(png.clone());
    let mut b = Place::new("B", Kind::Point, vec![(3.0, 4.0)]);
    b.picture = Some(png.clone());
    tree.children.push(Item::Place(a));
    tree.children.push(Item::Place(b));
    assert!(kml::needs_kmz(&tree));
    let back = kml::read_file(&kml::write_kmz(&tree).unwrap(), "z.kmz").unwrap();
    for item in &back.children {
        let Item::Place(p) = item else { panic!("не метка") };
        assert_eq!(p.picture.as_deref(), Some(png.as_slice()), "{}", p.name);
        assert_eq!(p.picture_href, "files/icon1.png", "одинаковые картинки - один файл");
    }
    // Значок набора картинкой не считается.
    let plain = kml::read_file(kml::write_kml(&Folder::new("x")).as_bytes(), "x.kml").unwrap();
    assert!(!kml::needs_kmz(&plain));
}
