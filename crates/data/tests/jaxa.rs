// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Наборы JAXA Earth: даты, адреса, пересчёт чисел файла, сетка мира.

use data::jaxa::{self, LAND_TEMPERATURE, PRECIPITATION, SETS};
use data::sentinel::Raw;

/// Начало `collection.json` набора с датами вразнобой и чужими ссылками.
const COLLECTION: &str = r#"{"type":"Collection","links":[
 {"rel":"root","href":"./collection.json"},
 {"rel":"license","href":"https://earth.jaxa.jp/policy/"},
 {"rel":"child","href":"./2026-10/catalog.json"},
 {"rel":"child","href":"./2018-01/catalog.json"},
 {"rel":"child","href":"./2026-09/catalog.json"},
 {"rel":"child","href":"./../evil/catalog.json"},
 {"rel":"child","href":"./2026-13/catalog.json"}]}"#;

#[test]
fn dates_are_sorted_and_checked() {
    let dates = jaxa::parse_dates(COLLECTION);
    assert_eq!(dates, ["2018-01", "2026-09", "2026-10"]);
    assert!(jaxa::parse_dates("not json").is_empty());
    assert_eq!(jaxa::date_seconds("2026-10"), jaxa::date_seconds("2026-10-01"));
    assert_eq!(jaxa::date_seconds("2026"), jaxa::date_seconds("2026-01-01"));
    assert_eq!(jaxa::date_seconds("26-10"), None);
}

#[test]
fn date_follows_the_moment() {
    let dates = jaxa::parse_dates(COLLECTION);
    assert_eq!(jaxa::pick_date(&dates, None), Some("2026-10"));
    let mid_september = jaxa::date_seconds("2026-09-15");
    assert_eq!(jaxa::pick_date(&dates, mid_september), Some("2026-09"));
    // Раньше первой даты - первая.
    assert_eq!(jaxa::pick_date(&dates, jaxa::date_seconds("2001-05")), Some("2018-01"));
    assert_eq!(jaxa::pick_date(&[], None), None);
}

#[test]
fn half_urls_follow_the_bucket_layout() {
    let east = jaxa::half_url(&LAND_TEMPERATURE, "2026-10", true);
    assert_eq!(east, "https://s3.ap-northeast-1.wasabisys.com/je-pds/cog/v1/JAXA.G-Portal_GCOM-C.SGLI_standard.L3-LST.daytime.v3_global_monthly/2026-10/0/E000.00-E180.00/E000.00-S90.00-E180.00-N90.00-LST.tiff");
    let west = jaxa::half_url(&PRECIPITATION, "2026-10", false);
    assert!(west.ends_with("/2026-10/0/W180.00-E000.00/W180.00-S90.00-E000.00-N90.00-PRECIP.tiff"), "{west}");
}

#[test]
fn file_numbers_become_values() {
    // 15000 × 0.02 К = 300 К = 26.85 °C.
    assert!((jaxa::value(&LAND_TEMPERATURE, 15000.0) - 26.85).abs() < 1e-9);
    assert!(jaxa::value(&LAND_TEMPERATURE, 65535.0).is_nan());
    assert!(jaxa::value(&PRECIPITATION, -999.9).is_nan());
    assert!(jaxa::value(&PRECIPITATION, -99.0).is_nan());
    assert!((jaxa::value(&PRECIPITATION, 0.5) - 0.5).abs() < 1e-12);
}

#[test]
fn sets_have_unique_keys_and_ramps() {
    let mut keys: Vec<&str> = SETS.iter().map(|s| s.key).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), SETS.len());
    for set in SETS {
        assert_eq!(set.key, set.field.key);
        assert!(set.field.ramp.windows(2).all(|w| w[0].0 < w[1].0), "{}", set.key);
        assert!(set.id.ends_with("_monthly") && set.slope > 0.0, "{}", set.key);
    }
}

#[test]
fn halves_make_one_world() {
    // Полушария 2 × 2 пикселя по 90°: запад - 10000, восток - 15000 и пропуск.
    let west = Raw::from_vec(2, 2, vec![10000.0; 4]).unwrap_or_else(|| Raw::filled(0, 0, 0.0));
    let east = Raw::from_vec(2, 2, vec![15000.0, 15000.0, 65535.0, 15000.0]).unwrap_or_else(|| Raw::filled(0, 0, 0.0));
    let grid = jaxa::world_grid(&LAND_TEMPERATURE, &west, &east, 0.0);
    let Some(grid) = grid else {
        unreachable!("сетка не собралась");
    };
    assert_eq!((grid.rows, grid.cols, grid.step), (2, 4, 90.0));
    assert_eq!((grid.west, grid.north), (-135.0, 45.0));
    // Ряд 0: запад, запад, восток, восток.
    assert!((f64::from(grid.values[0]) - (200.0 - 273.15)).abs() < 1e-3);
    assert!((f64::from(grid.values[2]) - 26.85).abs() < 1e-3);
    assert!(grid.values[6].is_nan());
    // Точка у Москвы - в восточном полушарии.
    let v = data::weather::sample(&grid, 55.0, 37.0);
    assert!(v.is_nan() || v > 0.0, "{v}");
    // Полушария разного шага не сливаются.
    let wide = Raw::filled(2, 4, 0.0);
    assert!(jaxa::world_grid(&LAND_TEMPERATURE, &west, &wide, 0.0).is_none());
}

// --- Участок, перенос `tests/test_jaxa.py` модуля ------------------------

#[test]
fn quad_urls_as_files_of_the_set() {
    let ndvi = &jaxa::NDVI;
    assert_eq!(jaxa::quad_url(ndvi, "2026-07", 0.0, 0.0), format!("{}{}/2026-07/1/E000.00-E090.00/E000.00-N00.00-E090.00-N90.00-NDVI.tiff", jaxa::ROOT, ndvi.id));
    // Имя файла сверено с каталогом набора 9 ноября 2026 года.
    assert_eq!(jaxa::quad_url(ndvi, "2026-07", -180.0, -90.0), format!("{}{}/2026-07/1/W180.00-W090.00/W180.00-S90.00-W090.00-N00.00-NDVI.tiff", jaxa::ROOT, ndvi.id));
    assert!(jaxa::quad_url(ndvi, "2026-07", -90.0, 0.0).contains("/W090.00-E000.00/W090.00-N00.00-E000.00-N90.00-"));
}

#[test]
fn quads_under_a_box() {
    assert_eq!(jaxa::quads([37.0, 55.0, 38.0, 56.0]), [(0.0, 0.0)]);
    let mut four = jaxa::quads([-1.0, -1.0, 1.0, 1.0]);
    four.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    assert_eq!(four, [(-90.0, -90.0), (-90.0, 0.0), (0.0, -90.0), (0.0, 0.0)]);
    let mut line = jaxa::quads([179.0, 60.0, -179.0, 61.0]);
    line.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    assert_eq!(line, [(-180.0, 0.0), (90.0, 0.0)]);
    assert_eq!(jaxa::quads([0.0, 10.0, 10.0, 20.0]), [(0.0, 0.0)]);
}

#[test]
fn area_grid_on_pixel_centres() {
    let grid = jaxa::grid_for([37.01, 55.01, 37.09, 55.04], 0.025);
    assert!((grid.west - 37.0125).abs() < 1e-9 && (grid.north - 55.0375).abs() < 1e-9, "{grid:?}");
    assert_eq!((grid.rows, grid.cols), (2, 4));
    assert_eq!((jaxa::grid_for([37.001, 55.001, 37.002, 55.002], 0.025).rows, jaxa::grid_for([37.001, 55.001, 37.002, 55.002], 0.025).cols), (1, 1));
    // Участок без пикселей внутри берёт пиксель своей середины.
    let small = jaxa::grid_for([37.0, 55.0, 37.075, 55.075], 0.025);
    let mask = small.mask(&|_, _| false);
    assert_eq!(mask.iter().filter(|m| **m).count(), 1);
    assert!(mask[small.cols + 1]);
}

#[test]
fn parts_split_at_greenwich_and_fill_values() {
    let set = &jaxa::NDVI;
    let grid = jaxa::grid_for([-0.05, 10.0, 0.05, 10.05], 0.025);
    let parts = grid.parts();
    assert_eq!(parts.len(), 2, "{parts:?}");
    let mut values = vec![f64::NAN; grid.rows * grid.cols];
    for part in &parts {
        // Число файла 12000 - NDVI 0.2.
        let raw = Raw::filled(part.rows, part.cols, 12000.0);
        assert!(jaxa::place(set, &grid, part, &raw, &mut values));
        assert!((part.bbox[2] - part.bbox[0] - part.cols as f64 * 0.025).abs() < 1e-9);
    }
    assert!(values.iter().all(|v| (v - 0.2).abs() < 1e-9), "{values:?}");
    // Не той формы - часть пустая.
    assert!(!jaxa::place(set, &grid, &parts[0], &Raw::filled(1, 1, 0.0), &mut values));
}

#[test]
fn area_steps_and_months() {
    assert_eq!(jaxa::area_step(&jaxa::NDVI), 0.025);
    assert_eq!(jaxa::area_step(&jaxa::AEROSOL), 0.05);
    assert_eq!(jaxa::area_step(&PRECIPITATION), 0.1);
    assert_eq!(jaxa::month_day("1970-01"), Some(14.5));
    assert_eq!(jaxa::month_day("1970-00"), None);
    let dates: Vec<String> = ["2025-11", "2025-12", "2026-01", "2026-02"].map(String::from).to_vec();
    assert_eq!(jaxa::between(&dates, "2025-12", "2026-01"), ["2025-12", "2026-01"]);
    assert_eq!(jaxa::same_month(&["2025-07", "2025-08", "2026-07"], "2026-07"), ["2025-07", "2026-07"]);
}

/// Условия данных JAXA: подпись называет исследование и JAXA, набор
/// с данными MODIS - ещё и NASA. Прежде у всех стояло «JAXA Earth».
#[test]
fn every_set_credits_its_research_and_jaxa() {
    for set in SETS {
        assert!(set.credit.contains("(JAXA") && !set.credit.starts_with("JAXA"), "{}", set.credit);
    }
    assert!(jaxa::AEROSOL.credit.contains("JAXA/NASA"));
    assert!(PRECIPITATION.credit.contains("GSMaP"));
    assert!(LAND_TEMPERATURE.credit.contains("SGLI"));
}
