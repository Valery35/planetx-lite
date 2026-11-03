// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Заросшие поля, перенос `tests/test_fallow.py` модуля 1.36.

use data::fallow::{self, Season};
use data::objecttasks::Kind;

/// Сочетание «SWIR» B12 B8A B4.
const ROLES: [Option<&str>; 3] = [Some("swir2"), Some("nir"), Some("red")];

/// Снимок из 100 клеток с одинаковыми значениями.
fn scene(red: f64, nir: f64, swir: f64) -> Vec<Vec<f64>> {
    vec![vec![swir; 100], vec![nir; 100], vec![red; 100]]
}

fn first(cells: Option<Vec<f64>>) -> f64 {
    cells.and_then(|c| c.first().copied()).unwrap_or(-1.0)
}

#[test]
fn ploughed_soil_bare_thatch_and_grass_not() {
    // Числа снимков у Шадейки: пашня, сухая ветошь, всходы.
    assert_eq!(first(fallow::bare_cells(&scene(0.12, 0.17, 0.2), &ROLES)), 1.0);
    assert_eq!(first(fallow::bare_cells(&scene(0.09, 0.20, 0.2), &ROLES)), 0.0);
    assert_eq!(first(fallow::bare_cells(&scene(0.04, 0.35, 0.2), &ROLES)), 0.0);
}

#[test]
fn water_and_snow_unknown() {
    assert!(first(fallow::bare_cells(&scene(0.03, 0.03, 0.2), &ROLES)).is_nan());
    assert!(first(fallow::bare_cells(&scene(0.8, 0.8, 0.05), &ROLES)).is_nan());
}

#[test]
fn needs_red_and_nir() {
    assert!(fallow::bare_cells(&scene(0.1, 0.2, 0.2), &[Some("swir1"), Some("nir"), Some("blue")]).is_none());
}

#[test]
fn forest_dark_in_swir_meadow_light() {
    // Числа Шадейки: лес 0.03-0.08, луг 0.10-0.20 по SWIR 2,2 мкм. Первые
    // 100 клеток - лес, следующие 100 - луг.
    let join = |a: Vec<Vec<f64>>, b: Vec<Vec<f64>>| a.into_iter().zip(b).map(|(x, y)| x.into_iter().chain(y).collect()).collect::<Vec<Vec<f64>>>();
    let one = join(scene(0.04, 0.30, 0.05), scene(0.06, 0.30, 0.14));
    let two = join(scene(0.05, 0.28, 0.07), scene(0.05, 0.32, 0.12));
    let trees = fallow::season_woody(&[(&one, &ROLES), (&two, &ROLES)]).unwrap_or_default();
    assert_eq!(trees.len(), 200);
    assert!(trees[..100].iter().all(|v| *v == 1.0));
    assert!(trees[100..].iter().all(|v| *v == 0.0));
}

#[test]
fn snow_does_not_count() {
    // Тёмный в SWIR снег - не лес.
    let snowy = scene(0.8, 0.8, 0.02);
    let trees = fallow::season_woody(&[(&snowy, &ROLES)]).unwrap_or_default();
    assert!(!trees.is_empty() && trees.iter().all(|v| v.is_nan()));
}

#[test]
fn no_swir_gives_none() {
    let bands = scene(0.1, 0.3, 0.2)[1..].to_vec();
    assert!(fallow::season_woody(&[(&bands, &[Some("nir"), Some("red")])]).is_none());
}

/// Четыре поля по 10 клеток: 0 - пашня каждый год, 1 - брошено после
/// 2023 года, 2 - не пахалось, 3 - деревня всегда голая.
fn fields() -> (Vec<i32>, Vec<(i32, Vec<f64>)>) {
    let parts: Vec<i32> = (0..40).map(|k| k / 10).collect();
    let scenes = [2023, 2024, 2025, 2026]
        .into_iter()
        .map(|year| {
            let cells = (0..40)
                .map(|k| match k / 10 {
                    0 | 3 => 1.0,
                    1 if year == 2023 => 1.0,
                    _ => 0.0,
                })
                .collect();
            (year, cells)
        })
        .collect();
    (parts, scenes)
}

#[test]
fn last_year_and_kinds() {
    let (parts, scenes) = fields();
    let years = fallow::tilled_years(&parts, &scenes, 4);
    let woody = [0.0, 0.0, 0.05, 0.0];
    let summer = [Kind::Vegetation, Kind::Vegetation, Kind::Vegetation, Kind::Soil];
    let (kinds, last) = fallow::field_kinds(&summer, &years, Some(&woody), 2026);
    assert_eq!(last, vec![2026, 2023, 0, 2026]);
    assert_eq!(kinds, vec![Kind::Tilled, Kind::FallowOld, Kind::FallowOld, Kind::Soil]);
}

#[test]
fn recent_abandonment_and_trees() {
    let (parts, mut scenes) = fields();
    // Поле 1 пахали и в 2025 году.
    for k in 10..20 {
        scenes[2].1[k] = 1.0;
    }
    let years = fallow::tilled_years(&parts, &scenes, 4);
    let mut woody = [0.0, 0.0, 0.4, 0.0];
    let (kinds, _) = fallow::field_kinds(&[Kind::Vegetation; 4], &years, Some(&woody), 2026);
    assert_eq!(kinds[1], Kind::FallowNew);
    assert_eq!(kinds[2], Kind::Overgrown);
    woody[2] = 0.9;
    let (kinds, _) = fallow::field_kinds(&[Kind::Vegetation; 4], &years, Some(&woody), 2026);
    assert_eq!(kinds[2], Kind::Forest);
}

#[test]
fn scene_without_data_on_field_is_not_counted() {
    let (parts, mut scenes) = fields();
    // 2026 год над пашней - снег.
    for k in 0..10 {
        scenes[3].1[k] = f64::NAN;
    }
    let years = fallow::tilled_years(&parts, &scenes, 4);
    assert!(!years.years[0].iter().any(|(y, _)| *y == 2026));
    assert_eq!(years.seen[0], 3);
}

#[test]
fn water_stays_water() {
    let (parts, scenes) = fields();
    let years = fallow::tilled_years(&parts, &scenes, 4);
    let (kinds, _) = fallow::field_kinds(&[Kind::Water; 4], &years, None, 2026);
    assert_eq!(kinds, vec![Kind::Water; 4]);
}

#[test]
fn best_scene_per_year_and_season() {
    let found: Vec<(String, f64, usize)> =
        [("2025-05-07", 0.0), ("2025-05-29", 1.0), ("2025-04-20", 0.0), ("2025-09-01", 0.0), ("2025-09-19", 1.0), ("2025-05-24", 5.0), ("2024-10-14", 0.0), ("2024-07-01", 0.0)].iter().enumerate().map(|(n, (w, c))| (w.to_string(), *c, n)).collect();
    let out = fallow::pick(&found);
    let best = |key: (i32, Season)| out.iter().find(|(k, _)| *k == key).and_then(|(_, ids)| ids.first()).map(|id| found[*id].0.clone());
    assert_eq!(best((2025, Season::Spring)).as_deref(), Some("2025-05-29"));
    assert_eq!(best((2025, Season::Autumn)).as_deref(), Some("2025-09-19"));
    assert_eq!(best((2024, Season::Autumn)).as_deref(), Some("2024-10-14"));
    assert_eq!(best((2024, Season::Spring)), None);
}
