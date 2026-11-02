// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Минеральные карты EMIT: каталог, сетка GLT, классы минералов, чтение
//! файла. Перенос `tests/test_emit.py` модуля, файл netCDF-4 - как
//! синтетический файл шагов `emit_*` модуля.

use data::aef::{self, ZoneIndex};
use data::emit::{self, Glt};
use netcdf_writer::{NcAttrValue, NcFileBuilder, NcWriteFormat, NcWriteOptions};
use serde_json::{Value, json};

fn item(ur: &str, start: &str, version: &str) -> Value {
    let base = format!("https://data.lpdaac.earthdatacloud.nasa.gov/lp-prod-protected/EMITL2BMIN.{version}/{ur}/");
    let public = base.replace("protected", "public");
    json!({"umm": {
        "GranuleUR": ur,
        "TemporalExtent": {"RangeDateTime": {"BeginningDateTime": start, "EndingDateTime": start}},
        "CloudCover": 10,
        "SpatialExtent": {"HorizontalSpatialDomain": {"Geometry": {"GPolygons": [{"Boundary": {"Points": [
            {"Longitude": -116.0, "Latitude": 37.0}, {"Longitude": -117.5, "Latitude": 37.0},
            {"Longitude": -117.5, "Latitude": 38.0}, {"Longitude": -116.0, "Latitude": 38.0},
            {"Longitude": -116.0, "Latitude": 37.0}]}}]}}},
        "RelatedUrls": [
            {"URL": format!("https://opendap.earthdata.nasa.gov/x/{ur}"), "Type": "USE SERVICE API"},
            {"URL": format!("{base}{ur}.nc"), "Type": "GET DATA"},
            {"URL": format!("{base}{}.nc", ur.replace("_MIN_", "_MINUNCERT_")), "Type": "GET DATA"},
            {"URL": format!("{public}{ur}.png"), "Type": "GET RELATED VISUALIZATION"},
            {"URL": format!("s3://lp-prod-protected/x/{ur}.nc"), "Type": "GET DATA VIA DIRECT ACCESS"}]}})
}

#[test]
fn url() {
    let ring = [(37.4, -117.3), (37.4, -117.1), (37.6, -117.2)];
    let url = emit::cmr_url(&ring, "2023-01-01", "2024-12-31", 30.0, emit::COLLECTIONS[0].0);
    assert!(url.contains("collection_concept_id=C4079846859-LPCLOUD"));
    assert!(url.contains("bounding_box=-117.300000%2C37.400000%2C-117.100000%2C37.600000"));
    assert!(url.contains("cloud_cover=0%2C30"));
}

#[test]
fn parse_takes_min_file_not_uncertainty() {
    let scenes = emit::parse_cmr(&json!({"items": [item("EMIT_L2B_MIN_002_20261005T164917", "2026-10-05T16:49:17Z", "002")]}), "002");
    assert_eq!(scenes.len(), 1);
    let s = &scenes[0];
    assert!(s.url.ends_with("/EMIT_L2B_MIN_002_20261005T164917.nc"));
    assert!(s.preview.ends_with(".png"));
    assert_eq!(s.when, "2026-10-05 16:49:17");
    assert_eq!(s.polygon.len(), 4);
    assert_eq!(s.version, "002");
}

#[test]
fn merge_prefers_first_version() {
    let new = emit::parse_cmr(&json!({"items": [item("EMIT_L2B_MIN_002_20230801T100000", "2023-08-01T10:00:00Z", "002")]}), "002");
    let old = emit::parse_cmr(
        &json!({"items": [item("EMIT_L2B_MIN_001_20230801T100000_2321308_005", "2023-08-01T10:00:00Z", "001"), item("EMIT_L2B_MIN_001_20240901T100000_2424508_006", "2024-09-01T10:00:00Z", "001")]}),
        "001",
    );
    let merged = emit::merge(vec![new, old]);
    assert_eq!(merged.iter().map(|s| s.version.as_str()).collect::<Vec<_>>(), ["001", "002"]);
    assert_eq!(merged[1].when, "2023-08-01 10:00:00");
}

#[test]
fn coverage() {
    let polygon = [(37.0, -116.0), (37.0, -117.5), (38.0, -117.5), (38.0, -116.0)];
    let inner = [(37.4, -117.3), (37.4, -117.1), (37.6, -117.1), (37.6, -117.3)];
    assert!((emit::coverage(&polygon, &inner) - 1.0).abs() < 1e-12);
    let half = [(37.9, -117.0), (37.9, -116.5), (38.1, -116.5), (38.1, -117.0)];
    let share = emit::coverage(&polygon, &half);
    assert!(share > 0.3 && share < 0.7, "{share}");
    let away = [(10.0, 10.0), (10.0, 11.0), (11.0, 11.0), (11.0, 10.0)];
    assert_eq!(emit::coverage(&polygon, &away), 0.0);
}

#[test]
fn ortho_takes_one_based_rows_and_columns() {
    let swath: Vec<f32> = (0..12).map(|v| v as f32).collect();
    let glt = Glt { rows: 2, cols: 3, x: vec![1, 4, 0, 2, 3, 4], y: vec![1, 3, 0, 2, -2, 1] };
    let out = emit::ortho(&swath, 3, 4, &glt, f32::NAN);
    assert_eq!(out[0], 0.0);
    assert_eq!(out[1], 11.0);
    assert!(out[2].is_nan());
    assert_eq!(out[3], 5.0);
    // Знак таблицы не учитывается.
    assert_eq!(out[4], 6.0);
    assert_eq!(out[5], 3.0);
    let ids = emit::ortho(&[5, 7], 1, 2, &Glt { rows: 1, cols: 2, x: vec![2, 0], y: vec![1, 0] }, -1);
    assert_eq!(ids, [7, -1]);
}

#[test]
fn fit_transform() {
    let (rows, cols) = (4, 5);
    let lats: Vec<f64> = (0..rows * cols).map(|i| 40.0 - 0.01 * ((i / cols) as f64 + 0.5)).collect();
    let lons: Vec<f64> = (0..rows * cols).map(|i| 10.0 + 0.02 * ((i % cols) as f64 + 0.5)).collect();
    let glt = Glt { rows, cols, x: (0..rows * cols).map(|i| (i % cols) as i64 + 1).collect(), y: (0..rows * cols).map(|i| (i / cols) as i64 + 1).collect() };
    let gt = emit::fit_transform(&glt, &lats, &lons, rows, cols).unwrap_or([0.0; 6]);
    assert!(emit::fit_ok(&gt));
    for (got, want) in [(gt[0], 10.0), (gt[1], 0.02), (gt[3], 40.0), (gt[5], -0.01)] {
        assert!((got - want).abs() < 1e-9, "{gt:?}");
    }
}

#[test]
fn classes_by_area_without_empty_cells() {
    let ids = [3, 3, 3, 9, 9, -9999, 3, 0, 7];
    let depth = [0.1, 0.2, 0.1, 0.1, 0.3, 0.2, f32::NAN, 0.0, 0.05];
    let (kinds, kept) = emit::classes(&ids, &depth, None);
    assert_eq!(kept, [3, 9, 7]);
    assert_eq!(kinds, [0, 0, 0, 1, 1, -1, -1, -1, 2]);
    let shares = emit::shares(&kinds, kept.len());
    assert_eq!(shares[0], (0, 3.0 / 6.0));
}

#[test]
fn mask() {
    let (kinds, kept) = emit::classes(&[1, 2], &[0.1, 0.1], Some(&[false, true]));
    assert_eq!(kept, [2]);
    assert_eq!(kinds, [-1, 0]);
}

#[test]
fn many_minerals_keep_rest_class() {
    let ids: Vec<i32> = (0..300).collect();
    let (kinds, kept) = emit::classes(&ids, &[1.0; 300], None);
    assert_eq!(kept.len(), emit::MOST);
    assert_eq!(kept.last(), Some(&-1));
    assert_eq!(kinds.iter().max().copied(), Some((emit::MOST - 1) as i16));
    assert_eq!(emit::color(-1), [160, 160, 160]);
}

#[test]
fn names_prefer_sample_name() {
    let files = ["lib/s07_Goethite_WS222.txt", "Hematite_GDS27", "kao.txt"].map(String::from);
    let samples = ["", "Hematite GDS27", "Kaolinite CM9"].map(String::from);
    let names = emit::names_of(&[0, 1, 0], &[1, 1, 2], &files, Some(&samples));
    assert_eq!(names[0], (1, 0, "s07_Goethite_WS222".to_string()));
    assert_eq!(names[1], (1, 1, "Hematite GDS27".to_string()));
    assert_eq!(names[2], (2, 0, "Kaolinite CM9".to_string()));
}

/// Файл сцены: полоса 4 × 6 повёрнута относительно сетки GLT 6 × 4,
/// запад - каолинит (номер 5), восток - кальцит (номер 8), как шаги
/// `emit_*` модуля. Сетка широт и долгот - у точки (37.5, -117.2), клетка
/// 0.01°.
fn scene_file(with_transform: bool) -> netcdf_writer::Result<Vec<u8>> {
    let (strip_rows, strip_cols) = (4u64, 6u64);
    let (rows, cols) = (6u64, 4u64);
    let mut b = NcFileBuilder::new();
    let down = b.add_dimension("downtrack", strip_rows)?;
    let cross = b.add_dimension("crosstrack", strip_cols)?;
    let oy = b.add_dimension("ortho_y", rows)?;
    let ox = b.add_dimension("ortho_x", cols)?;
    let minerals = b.add_dimension("minerals", 2)?;
    // Полоса повёрнута: строка полосы - столбец сетки, столбец полосы -
    // строка сетки. Запад сетки (столбцы 0-1) - каолинит.
    let mut ids = vec![0f32; (strip_rows * strip_cols) as usize];
    let mut depth = vec![0f32; ids.len()];
    let (mut gx, mut gy) = (vec![0i32; (rows * cols) as usize], vec![0i32; (rows * cols) as usize]);
    for r in 0..rows as usize {
        for c in 0..cols as usize {
            let (sr, sc) = (c, r);
            gx[r * cols as usize + c] = sc as i32 + 1;
            gy[r * cols as usize + c] = sr as i32 + 1;
            let k = sr * strip_cols as usize + sc;
            ids[k] = if c < 2 { 5.0 } else { 8.0 };
            depth[k] = 0.2;
        }
    }
    // Клетка полосы без данных и угол сетки вне полосы.
    ids[0] = -9999.0;
    gx[(rows * cols - 1) as usize] = 0;
    let v = b.add_variable::<f32>("group_2_mineral_id", &[down, cross])?;
    b.write_variable(v, &ids)?;
    let v = b.add_variable::<f32>("group_2_band_depth", &[down, cross])?;
    b.write_variable(v, &depth)?;
    let v = b.add_variable::<f32>("group_1_mineral_id", &[down, cross])?;
    b.write_variable(v, &vec![-9999.0f32; ids.len()])?;
    let v = b.add_variable::<f32>("group_1_band_depth", &[down, cross])?;
    b.write_variable(v, &vec![-9999.0f32; ids.len()])?;
    let v = b.add_variable_path::<i32>("location/glt_x", &[oy, ox])?;
    b.write_variable(v, &gx)?;
    let v = b.add_variable_path::<i32>("location/glt_y", &[oy, ox])?;
    b.write_variable(v, &gy)?;
    // Широты и долготы полосы - центры клеток сетки под ней.
    let (west, north, step) = (-117.22, 37.53, 0.01);
    let (mut lat, mut lon) = (vec![0f64; ids.len()], vec![0f64; ids.len()]);
    for r in 0..rows as usize {
        for c in 0..cols as usize {
            let k = c * strip_cols as usize + r;
            lat[k] = north - step * (r as f64 + 0.5);
            lon[k] = west + step * (c as f64 + 0.5);
        }
    }
    let v = b.add_variable_path::<f64>("location/lat", &[down, cross])?;
    b.write_variable(v, &lat)?;
    let v = b.add_variable_path::<f64>("location/lon", &[down, cross])?;
    b.write_variable(v, &lon)?;
    let v = b.add_variable_path::<i32>("mineral_metadata/index", &[minerals])?;
    b.write_variable(v, &[5, 8])?;
    let v = b.add_variable_path::<i32>("mineral_metadata/group", &[minerals])?;
    b.write_variable(v, &[2, 2])?;
    let v = b.add_string_variable_path("mineral_metadata/name", &[minerals])?;
    b.write_string_variable(v, &["lib/kaolinite_cm9.txt", "lib/calcite_ws272.txt"])?;
    if with_transform {
        b.add_attribute("geotransform", NcAttrValue::Doubles(vec![west, step, 0.0, north, 0.0, -step]))?;
    }
    b.to_vec(NcWriteOptions { format: NcWriteFormat::Nc4 }).map(|(_, bytes)| bytes)
}

#[test]
fn file_on_site_grid() {
    for with_transform in [true, false] {
        let bytes = scene_file(with_transform);
        assert!(bytes.is_ok(), "{bytes:?}");
        let bytes = bytes.unwrap_or_default();
        assert!(bytes.starts_with(b"\x89HDF"), "файл не HDF5");
        let m = emit::read_file(&bytes, 2);
        assert!(m.is_ok(), "{m:?}");
        let Ok(m) = m else {
            return;
        };
        assert_eq!((m.rows, m.cols), (6, 4));
        let t = m.transform;
        assert!((t[0] + 117.22).abs() < 1e-6 && (t[1] - 0.01).abs() < 1e-6 && (t[5] + 0.01).abs() < 1e-6, "{t:?}");
        assert_eq!(m.names, [(5, "kaolinite_cm9".to_string()), (8, "calcite_ws272".to_string())]);
        // Запад - каолинит, восток - кальцит, клетка без данных и угол вне
        // полосы - без минерала.
        assert_eq!(m.ids[0], -1);
        assert_eq!(m.ids[1], 5);
        assert_eq!(m.ids[2], 8);
        assert!(!m.covered[(6 * 4) - 1] && m.covered[1]);
        let Some(index) = ZoneIndex::bare(&aef::zone_of(-117.2, 37.5)) else {
            return;
        };
        let grid = emit::to_site(&m, &index, (-117.215, 37.48, -117.185, 37.52));
        let (kinds, kept) = emit::classes(&grid.ids, &grid.depth, None);
        assert_eq!(kept.len(), 2, "{kept:?}");
        // Запад участка - каолинит, восток - кальцит.
        let row = grid.rows / 2;
        let west = grid.ids[row * grid.cols];
        let east = grid.ids[row * grid.cols + grid.cols - 1];
        assert_eq!((west, east), (5, 8));
        assert!(kinds.iter().filter(|k| **k >= 0).count() > grid.rows * grid.cols / 2);
        // Группа 1 - минералов нет.
        let empty = emit::read_file(&bytes, 1).map(|m| m.ids.iter().all(|id| *id < 0));
        assert_eq!(empty, Ok(true));
    }
    assert!(emit::read_file(b"<html>login</html>", 2).is_err());
}
