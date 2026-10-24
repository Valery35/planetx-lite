// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Своя батиметрия: глубины в отметки дна, сетка воды, вода на профиле.
//! Перенос `tests/test_bathymetry.py` модуля.

use data::bathymetry::{self, Kind};

#[test]
fn depths_become_bed_either_sign() {
    let bed: Vec<f64> = [10.0, -10.0, f64::NAN, 0.0].iter().map(|v| bathymetry::to_bed(*v, Kind::Depth, 564.0)).collect();
    assert_eq!(&bed[..2], &[554.0, 554.0]);
    assert!(bed[2].is_nan());
    assert_eq!(bed[3], 564.0);
    assert_eq!(bathymetry::to_bed(308.0, Kind::Bed, 564.0), 308.0);
}

#[test]
fn water_over_wet_nodes_only() {
    // Сетка 3 × 3: левый столбец - берег выше воды, прочие - дно.
    let bed = [600.0, 500.0, 480.0, 600.0, 450.0, 470.0, 600.0, 500.0, f64::NAN];
    let tris = bathymetry::water_part(&bed, 3, 3, 564.0).unwrap_or_default();
    let used: Vec<u32> = tris.iter().flatten().copied().collect();
    assert!(!used.iter().any(|n| [0, 3, 6].contains(n)), "берег без воды");
    assert!(!used.contains(&8), "пустой узел");
    assert_eq!(tris.len(), 3);
    let dry: Vec<f64> = bed.iter().map(|b| b + 200.0).collect();
    assert!(bathymetry::water_part(&dry, 3, 3, 564.0).is_none());
}

#[test]
fn mercator_back_to_degrees() {
    let (lat, _) = bathymetry::mercator_latlon(0.0, 0.0);
    assert!(lat.abs() < 1e-9);
    let (lat, lon) = bathymetry::mercator_latlon(885725.0, 5897250.0);
    assert!((lon - 7.9566).abs() < 1e-3);
    assert!((lat - 46.75).abs() < 0.05);
}

#[test]
fn water_only_inside_frame_and_below_level() {
    // Рамка озера в EPSG:3857 около долготы 0, широты 0.
    let lake = ((-1000.0, -1000.0, 1000.0, 1000.0), 564.0);
    let points = [(0.0, 0.0), (0.0, 0.001), (0.0, 0.005), (0.0, 0.05)];
    let out = bathymetry::profile_water(&points, &[300.0, 600.0, 400.0, 300.0], &[lake]);
    assert_eq!(out[0], 564.0);
    assert!(out[1].is_nan(), "берег выше уровня");
    assert_eq!(out[2], 564.0);
    assert!(out[3].is_nan(), "вне рамки растра");
    assert!(bathymetry::profile_water(&[(0.0, 0.0)], &[100.0], &[])[0].is_nan());
}
