// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Типовые задачи выделения объектов, перенос `tests/test_objecttasks.py`
//! модуля 1.36.

use data::objecttasks::{self as ot, Kind, Scenario};

/// Средние отражения цвета в сочетании «Сельское хозяйство»: SWIR1, NIR,
/// синий.
const AGRI: [Option<&str>; 3] = [Some("swir1"), Some("nir"), Some("blue")];
const CONIFER: [f64; 3] = [0.08, 0.20, 0.02];
const BROADLEAF: [f64; 3] = [0.17, 0.38, 0.03];
const SOIL: [f64; 3] = [0.30, 0.22, 0.08];
const WATER: [f64; 3] = [0.01, 0.02, 0.04];
const SNOW: [f64; 3] = [0.10, 0.70, 0.80];

fn rows(list: &[&[f64]]) -> Vec<Vec<f64>> {
    list.iter().map(|r| r.to_vec()).collect()
}

/// Средние двух снимков рядом: каналы первого и за ними второго.
fn both(a: &[&[f64]], b: &[&[f64]]) -> Vec<Vec<f64>> {
    a.iter().zip(b).map(|(x, y)| x.iter().chain(y.iter()).copied().collect()).collect()
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn descriptions_give_roles() {
    assert_eq!(ot::channels(&strings(&["B11 swir16", "B8 nir", "B2 blue"]), "x"), Some(AGRI.to_vec()));
}

#[test]
fn file_name_of_composite() {
    assert_eq!(ot::channels(&strings(&["", "", ""]), "S2_20240701_40VDK_agriculture"), Some(AGRI.to_vec()));
    assert_eq!(ot::channels(&[], "L9_20250923_166-020_infrared_2"), Some(vec![Some("nir"), Some("red"), Some("green")]));
    assert_eq!(ot::month_of("S2_20240701_40VDK_agriculture"), Some(7));
    assert_eq!(ot::day_of("S2_20240701_40VDK_agriculture_2").as_deref(), Some("2024-07-01"));
    assert_eq!(ot::day_of("ortho"), None);
    assert_eq!(ot::role_names(&[Some("nir"), None, Some("red"), Some("nir")]), vec!["nir", "red"]);
}

#[test]
fn unknown_raster_has_no_roles() {
    assert_eq!(ot::channels(&strings(&["Band 1"]), "ortho"), None);
    assert!(!ot::usable(&[]));
    assert!(!ot::usable(&[Some("red"), Some("green"), Some("blue")]));
    assert!(ot::usable(&AGRI));
}

#[test]
fn agriculture_composite() {
    let kinds = ot::color_kinds(&rows(&[&CONIFER, &BROADLEAF, &SOIL, &WATER, &SNOW]), &AGRI);
    assert_eq!(kinds, vec![Kind::Conifer, Kind::Broadleaf, Kind::Soil, Kind::Water, Kind::Snow]);
}

#[test]
fn infrared_composite_by_ndvi() {
    // NIR, красный, зелёный: лес, пашня, вода.
    let kinds = ot::color_kinds(&rows(&[&[0.35, 0.04, 0.06], &[0.22, 0.18, 0.14], &[0.02, 0.03, 0.05]]), &[Some("nir"), Some("red"), Some("green")]);
    assert_eq!(kinds, vec![Kind::Vegetation, Kind::Soil, Kind::Water]);
}

#[test]
fn close_vegetation_is_not_split() {
    let kinds = ot::color_kinds(&rows(&[&[0.17, 0.36, 0.03], &[0.16, 0.38, 0.03]]), &AGRI);
    assert_eq!(kinds, vec![Kind::Vegetation, Kind::Vegetation]);
}

#[test]
fn scenario_merges() {
    let kinds = [Kind::Conifer, Kind::Broadleaf, Kind::Soil, Kind::Water];
    assert_eq!(ot::scenario_kinds(&kinds, Scenario::Fields), vec![Kind::Vegetation, Kind::Vegetation, Kind::Soil, Kind::Water]);
    assert_eq!(ot::scenario_kinds(&kinds, Scenario::Water), vec![Kind::Other, Kind::Other, Kind::Other, Kind::Water]);
}

#[test]
fn leaf_off_tells_conifer() {
    // Лето: хвоя и листва одной яркости ИК, осень: листва потеряла ИК,
    // хвоя нет.
    let summer: [&[f64]; 3] = [&[0.17, 0.32, 0.03], &[0.17, 0.33, 0.03], &SOIL];
    let autumn: [&[f64]; 3] = [&[0.15, 0.29, 0.03], &[0.20, 0.14, 0.04], &SOIL];
    let kinds = ot::color_kinds(&rows(&summer), &AGRI);
    assert_eq!(kinds, vec![Kind::Vegetation, Kind::Vegetation, Kind::Soil]);
    assert_eq!(ot::two_dates(&kinds, &both(&summer, &autumn), &AGRI, &AGRI), vec![Kind::Conifer, Kind::Broadleaf, Kind::Soil]);
}

#[test]
fn snow_tells_conifer() {
    // Март: хвоя тёмная на снегу, голый лиственный лес светлый, поле -
    // снег. У снега ближний ИК высокий - правило ИК дало бы хвою и листве.
    let summer: [&[f64]; 3] = [&[0.17, 0.32, 0.03], &[0.17, 0.33, 0.03], &SOIL];
    let march: [&[f64]; 3] = [&[0.10, 0.22, 0.12], &[0.18, 0.55, 0.45], &[0.08, 0.80, 0.85]];
    let centers = both(&summer, &march);
    let kinds = ot::color_kinds(&rows(&summer), &AGRI);
    assert!(ot::winter(&centers, &AGRI, &AGRI).is_some());
    assert_eq!(ot::two_dates(&kinds, &centers, &AGRI, &AGRI), vec![Kind::Conifer, Kind::Broadleaf, Kind::Soil]);
}

#[test]
fn snow_and_summer_nir_together() {
    // Числа снимков 21 июля 2025 и 17 апреля 2026 года у 59.99, 57.43.
    // Голый лиственный лес тёмный на снегу, но летом ярок в ИК - листва.
    let summer: [&[f64]; 5] = [&[0.134, 0.329, 0.027], &[0.114, 0.279, 0.026], &[0.093, 0.216, 0.026], &[0.203, 0.468, 0.035], &[0.09, 0.21, 0.03]];
    let april: [&[f64]; 5] = [&[0.082, 0.22, 0.11], &[0.068, 0.188, 0.053], &[0.056, 0.159, 0.02], &[0.056, 0.531, 0.532], &[0.06, 0.6, 0.55]];
    let kinds = ot::color_kinds(&rows(&summer), &AGRI);
    assert_eq!(ot::two_dates(&kinds, &both(&summer, &april), &AGRI, &AGRI), vec![Kind::Broadleaf, Kind::Conifer, Kind::Conifer, Kind::Broadleaf, Kind::Broadleaf]);
}

#[test]
fn no_snow_keeps_nir_rule() {
    let centers = both(&[&[0.17, 0.32, 0.03]], &[&[0.15, 0.29, 0.03]]);
    assert_eq!(ot::winter(&centers, &AGRI, &AGRI), None);
}

#[test]
fn second_without_nir_changes_nothing() {
    let kinds = [Kind::Vegetation, Kind::Soil];
    assert_eq!(ot::two_dates(&kinds, &[vec![0.0; 6], vec![0.0; 6]], &AGRI, &[Some("red"), Some("green"), Some("blue")]), kinds.to_vec());
}

/// Сетка номеров областей rows × cols, заполнение - f(строка, столбец).
fn grid(rows: usize, cols: usize, f: impl Fn(usize, usize) -> i32) -> Vec<i32> {
    (0..rows * cols).map(|k| f(k / cols, k % cols)).collect()
}

/// Слева лес (0), в нём вырубка (1), справа поле (2) среди пашни без леса.
fn forest_parts() -> Vec<i32> {
    grid(10, 20, |r, c| {
        if c >= 10 {
            2
        } else if (3..6).contains(&r) && (3..6).contains(&c) {
            1
        } else {
            0
        }
    })
}

#[test]
fn soil_in_forest_is_clearing() {
    let parts = forest_parts();
    let kinds = [Kind::Conifer, Kind::Soil];
    assert_eq!(ot::region_kinds(&parts, 10, 20, &[0, 1, 1], &kinds, Scenario::Forest)[1], Kind::Clearing);
    assert_eq!(ot::region_kinds(&parts, 10, 20, &[0, 1, 1], &kinds, Scenario::Fields)[1], Kind::Clearing);
}

#[test]
fn clearing_of_two_soil_shades() {
    // Вырубка из двух оттенков почвы (1 и 2): кусок - весь в лесу.
    let parts = grid(20, 20, |r, c| if (5..15).contains(&r) && (5..10).contains(&c) { 1 } else if (5..15).contains(&r) && (10..15).contains(&c) { 2 } else { 0 });
    assert_eq!(ot::region_kinds(&parts, 20, 20, &[0, 1, 2], &[Kind::Conifer, Kind::Soil, Kind::Soil], Scenario::Forest), vec![Kind::Conifer, Kind::Clearing, Kind::Clearing]);
}

#[test]
fn field_next_to_crops_stays_soil() {
    // Пашня (0) у посева (1) - светлой растительности, хвои нет.
    let parts = grid(10, 20, |_, c| i32::from(c >= 10));
    assert_eq!(ot::region_kinds(&parts, 10, 20, &[0, 1], &[Kind::Soil, Kind::Broadleaf], Scenario::Forest), vec![Kind::Soil, Kind::Broadleaf]);
}

#[test]
fn field_with_little_forest_border_stays_soil() {
    // Пашня 0 граничит с посевом 1 длинной стороной и с хвоей 2 - углом.
    let parts = grid(20, 20, |r, c| if c < 10 { 0 } else if r < 2 && c < 12 { 2 } else { 1 });
    assert_eq!(ot::region_kinds(&parts, 20, 20, &[0, 1, 2], &[Kind::Soil, Kind::Broadleaf, Kind::Conifer], Scenario::Forest)[0], Kind::Soil);
}

/// Кольцо 2 в клетку вокруг квадрата 1 среди 0, сетка 30 × 30.
fn ringed() -> Vec<i32> {
    grid(30, 30, |r, c| if (10..20).contains(&r) && (10..20).contains(&c) { 1 } else if (9..21).contains(&r) && (9..21).contains(&c) { 2 } else { 0 })
}

#[test]
fn thin_conifer_rim_is_not_forest() {
    // Поле (1) в кольце смешанных пикселей «хвои» (2) среди листвы (0) -
    // не рубка.
    assert_eq!(ot::region_kinds(&ringed(), 30, 30, &[0, 1, 2], &[Kind::Broadleaf, Kind::Soil, Kind::Conifer], Scenario::Forest)[1], Kind::Soil);
}

#[test]
fn inner_shares() {
    let parts = grid(10, 10, |r, _| i32::from(r == 0));
    let shares = ot::inner_shares(&parts, 10, 10);
    assert_eq!(shares[1], 0.0);
    assert!((shares[0] - 80.0 / 90.0).abs() < 1e-12);
}

#[test]
fn water_scenario_has_no_clearing() {
    assert_eq!(ot::region_kinds(&forest_parts(), 10, 20, &[0, 1, 1], &[Kind::Conifer, Kind::Soil], Scenario::Water), vec![Kind::Other; 3]);
}

#[test]
fn ring_share() {
    let shares = ot::ring_shares(&forest_parts(), 10, 20, &[Kind::Conifer, Kind::Soil, Kind::Soil], Kind::Conifer, &[1, 2], 3);
    assert_eq!(shares.len(), 2);
    assert!((shares[0].1 - 1.0).abs() < 1e-12);
    assert!((shares[1].1 - 1.0).abs() < 1e-12);
}

#[test]
fn clearing_behind_thin_rim() {
    // Вырубка (1) в кольце смешанных пикселей «листвы» (2), вокруг - хвоя.
    assert_eq!(ot::region_kinds(&ringed(), 30, 30, &[0, 1, 2], &[Kind::Conifer, Kind::Soil, Kind::Broadleaf], Scenario::Forest)[1], Kind::Clearing);
}

#[test]
fn line_masks() {
    let masks = ot::line_masks(&forest_parts(), &[Kind::Conifer, Kind::Soil, Kind::Water]);
    let get = |g: &str| masks.iter().find(|(k, _)| *k == g).map(|(_, m)| m.clone()).unwrap_or_default();
    assert!(get("roads")[4 * 20 + 4]);
    assert!(get("water")[15]);
    assert!(!get("belts").iter().any(|v| *v));
}

#[test]
fn classes_in_order() {
    let (used, numbers) = ot::classes_of(&[Kind::Water, Kind::Conifer, Kind::Water]);
    assert_eq!(used, vec![Kind::Conifer, Kind::Water]);
    assert_eq!(numbers, vec![1, 0, 1]);
}

#[test]
fn labels_from_tiff_header() {
    // Подписи каналов файла окна читаются без разбора точек.
    let grid = data::geotiff::MercatorGrid { x0: 500_000.0, y0: 6_400_000.0, step: 10.0, cols: 3, rows: 2 };
    let bands: Vec<(String, Vec<f32>)> = ["B11 swir16", "B8 nir", "B2 blue"].iter().map(|l| (l.to_string(), vec![0.1; 6])).collect();
    let bytes = data::geotiff::write_bands_projected(&grid, 32640, &bands).unwrap_or_default();
    let labels = data::rasterclass::quick_labels(&bytes);
    assert_eq!(labels, strings(&["B11 swir16", "B8 nir", "B2 blue"]));
    assert_eq!(ot::channels(&labels, "x"), Some(AGRI.to_vec()));
}