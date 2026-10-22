// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Region KML и скрытие дальше расстояния. Перенос
//! `tests/test_region.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use globe_core::DVec3;
use globe_core::ellipsoid::Ellipsoid;
use scene::region as rg;

fn eye(lat: f64, lon: f64, height: f64) -> DVec3 {
    Ellipsoid::wgs84().geodetic_to_ecef(lat, lon, height)
}

fn near(a: f64, b: f64, delta: f64) -> bool {
    (a - b).abs() <= delta
}

#[test]
fn distance_field_hides_beyond_km() {
    // Метка в Перми, скрывать дальше 50 км: видна с 40 км, не видна
    // с 60 км, при любом окне глобуса.
    let place = [(58.0105, 56.2294)];
    let region = rg::for_distance(&place, 50.0, None);
    assert!(near(rg::distance_km(Some(&region)).unwrap(), 50.0, 5e-4));
    assert!(rg::active(Some(&region), eye(58.0105, 56.2294, 40000.0)));
    assert!(!rg::active(Some(&region), eye(58.0105, 56.2294, 60000.0)));
    // Рамка точки - около BOX_SIDE метров.
    assert!(near(rg::side(&region), rg::BOX_SIDE, 5.0));
}

#[test]
fn max_lod_hides_close() {
    // maxLodPixels: вблизи рамка больше предела - метка скрыта.
    let region = rg::Region::new(58.1, 57.9, 56.4, 56.0, 0.0, 128.0);
    assert!(!rg::active(Some(&region), eye(58.0, 56.2, 10000.0)));
    assert!(rg::active(Some(&region), eye(58.0, 56.2, 1_000_000.0)));
    assert!(rg::active(None, eye(0.0, 0.0, 1.0)));
}

#[test]
fn edit_keeps_box_and_max_lod() {
    let old = rg::Region::new(58.1, 57.9, 56.4, 56.0, 300.0, 4000.0);
    let new = rg::for_distance(&[(0.0, 0.0)], 20.0, Some(&old));
    assert_eq!(new.rect(), old.rect());
    assert_eq!(new.max_lod, 4000.0);
    assert!(near(rg::distance_km(Some(&new)).unwrap(), 20.0, 5e-4));
}

#[test]
fn box_over_dateline() {
    let region = rg::Region::new(10.0, 0.0, -179.0, 179.0, 0.0, -1.0);
    let (width, _) = rg::span(&region);
    assert!(near(width, 2.0 * rg::M_PER_DEGREE * 0.9962, 500.0));
    assert!(near(rg::center(&region).1.rem_euclid(360.0), 180.0, 1e-7));
}

#[test]
fn box_of_points_over_dateline() {
    // Рамка пути через линию перемены дат - узкая, восток меньше
    // запада, середина у 180°, а не на другой стороне Земли.
    let (north, south, east, west) = rg::bounding_box(&[(-16.5, -174.25), (-27.4, -178.14), (-35.1, 179.33)]);
    assert!(near(west, 179.33, 5e-7));
    assert!(near(east, -174.25, 5e-7));
    let (_, lon) = rg::center(&rg::Region::new(north, south, east, west, 0.0, -1.0));
    assert!(lon.abs() > 177.0);
}

#[test]
fn text_round_trip_and_kml() {
    let region = rg::for_distance(&[(58.0, 56.0), (58.5, 56.5)], 100.0, None);
    assert_eq!(rg::parse(&rg::text(Some(&region))), Some(region));
    assert_eq!(rg::parse(""), None);
    assert_eq!(rg::parse("[1, 2]"), None);
    assert_eq!(rg::parse("не json"), None);
    assert!(rg::kml(Some(&region)).contains("<minLodPixels>"));
    assert_eq!(rg::kml(None), "");
    assert_eq!(rg::distance_km(Some(&rg::Region::new(1.0, 0.0, 1.0, 0.0, 0.0, -1.0))), None);
}
