// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Разрез Земли: сектор, грани с оболочками PREM, растяжение коры, грани
//! с моделью коры. Перенос `tests/test_cutaway.py` модуля.

use std::cell::RefCell;
use std::collections::BTreeSet;

use data::crust::{self as cr, Crust};
use data::cutaway::{self as cw, Faces, Wedge};
use data::subsurface::{self as ss, Mesh};

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

/// Точки сетки в ECEF: смещения вершин плюс центр.
fn points(mesh: &Mesh) -> Vec<[f64; 3]> {
    mesh.vertices.iter().map(|v| [0, 1, 2].map(|k| f64::from(v.position[k]) + mesh.center[k])).collect()
}

fn norm(p: &[f64; 3]) -> f64 {
    (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt()
}

fn max(values: impl Iterator<Item = f64>) -> f64 {
    values.fold(f64::NEG_INFINITY, f64::max)
}

fn faces(wedge: &Wedge, options: Faces) -> Option<Mesh> {
    let mesh = cw::faces(wedge, &options);
    assert!(mesh.is_some());
    mesh
}

fn has(mesh: &Mesh, rgb: [u8; 3]) -> bool {
    mesh.vertices.iter().any(|v| v.color[..3] == rgb)
}

// Сектор и грани.

#[test]
fn shells_cover_radius_without_gaps() {
    assert_eq!(cw::SHELLS[0].low, 0.0);
    assert_eq!(cw::SHELLS[5].high, cw::PREM_RADIUS);
    for pair in cw::SHELLS.windows(2) {
        assert_eq!(pair[0].high, pair[1].low);
    }
    // Ядро - 3480 км, внутреннее ядро - 1221.5 км.
    let high = |key: &str| cw::SHELLS.iter().find(|s| s.key == key).map(|s| s.high);
    assert_eq!(high("outer_core"), Some(3480.0));
    assert_eq!(high("inner_core"), Some(1221.5));
}

#[test]
fn inside_wraps_longitude() {
    let wedge = cw::wedge_at(30.0, 170.0, cw::HALF);
    let got: Vec<bool> =
        [(10.0, -160.0), (10.0, 120.0), (-10.0, 170.0), (10.0, 215.0 - 360.0)].iter().map(|(la, lo)| cw::inside(&wedge, *la, *lo)).collect();
    assert_eq!(got, [true, false, false, true]);
    let south = cw::wedge_at(-5.0, 0.0, cw::HALF);
    assert_eq!((south.south, south.north), (-90.0, 0.0));
    assert!(cw::inside(&south, -80.0, 40.0));
}

#[test]
fn move_corner() {
    let wedge = cw::wedge_at(30.0, 100.0, cw::HALF);
    // Юго-западный угол - на 20° с. ш., 70° в. д.
    let moved = cw::move_corner(&wedge, 0, 20.0, 70.0);
    assert_eq!(moved, Wedge { west: 70.0, east: 145.0, south: 20.0, north: 90.0 });
    // Северо-восточный угол через линию перемены дат.
    let wide = cw::move_corner(&moved, 3, 60.0, -170.0);
    close(cw::span(&wide), 120.0, 1e-7);
    assert_eq!((wide.south, wide.north), (20.0, 60.0));
    assert!(cw::inside(&wide, 40.0, 179.0));
    assert!(!cw::inside(&wide, 65.0, 179.0));
    // Восток за западом - сектор не выворачивается.
    let thin = cw::move_corner(&wedge, 1, 30.0, 50.0);
    close(cw::span(&thin), cw::MIN_SPAN, 1e-7);
    // Угол за полюс и широты наоборот - по порядку.
    let flipped = cw::move_corner(&wedge, 2, -10.0, 55.0);
    assert_eq!((flipped.south, flipped.north), (-10.0, 0.0));
    // Углы сектора.
    assert_eq!(cw::corners(&moved), [(20.0, 70.0), (20.0, 145.0), (90.0, 70.0), (90.0, 145.0)]);
}

#[test]
fn uniform_matches_inside() {
    // Шейдер считает сектор по нормали: dot(xy, c) >= cos(half)·|xy|
    // и синус широты между синусами границ. Та же проверка, что и inside.
    let wedge = cw::make_wedge(20.0, 130.0, -25.0, 50.0);
    let [cx, cy, cos_half, sin_s, sin_n] = cw::uniform(&wedge);
    // Псевдослучайные точки: линейный конгруэнтный ряд.
    let mut seed: u64 = 3;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    for _ in 0..500 {
        let lat = -89.0 + 178.0 * next();
        let lon = -180.0 + 360.0 * next();
        let up = ss::surface_normal(lat, lon);
        let xy = up[0].hypot(up[1]);
        let shader = up[0] * cx + up[1] * cy >= cos_half * xy && up[2] > sin_s && up[2] < sin_n;
        assert_eq!(shader, cw::inside(&wedge, lat, lon), "{lat} {lon}");
    }
}

#[test]
fn face_edge_lies_on_ellipsoid() {
    let wedge = cw::wedge_at(50.0, 30.0, cw::HALF);
    let Some(mesh) = faces(&wedge, Faces::default()) else { return };
    let points = points(&mesh);
    // Внешний край - на эллипсоиде: x²/a² + y²/a² + z²/b² = 1. Точность
    // f32 вершин - около метра.
    let level: Vec<f64> = points
        .iter()
        .filter(|p| norm(p) > 0.99 * ss::B)
        .map(|p| (p[0] * p[0] + p[1] * p[1]) / (ss::A * ss::A) + p[2] * p[2] / (ss::B * ss::B))
        .collect();
    assert!(max(level.iter().map(|l| (l - 1.0).abs())) < 0.01);
    let outermost: Vec<f64> = level.iter().copied().filter(|l| *l > 0.999).collect();
    assert!(outermost.len() > 100);
    assert!(max(outermost.iter().map(|l| (l - 1.0).abs())) < 1e-6);
    // Ничего не выходит из сектора: долготы граней в его пределах. У оси
    // Земли долгота не определена.
    for p in points.iter().filter(|p| p[0].hypot(p[1]) > 1000.0) {
        let lon = p[1].atan2(p[0]).to_degrees();
        assert!((lon - 30.0).abs() <= 45.0 + 1e-3);
        assert!(p[2] >= -1.0);
    }
}

#[test]
fn core_boundary_radius() {
    let wedge = cw::wedge_at(10.0, 0.0, cw::HALF);
    let Some(mesh) = faces(&wedge, Faces { shells: &cw::SHELLS[1..2], ..Faces::default() }) else { return };
    // Граница ядра - 3480/6371 радиуса эллипсоида в направлении.
    close(max(points(&mesh).iter().map(norm)) / ss::A, 3480.0 / 6371.0, 1e-4);
}

#[test]
fn edge_follows_lifted_surface() {
    // Режим «Геоид»: край граней - на поверхности, поднятой на lift,
    // оболочки растянуты в ту же долю.
    let wedge = cw::wedge_at(10.0, 0.0, cw::HALF);
    let up = |lats: &[f64], _: &[f64]| vec![500000.0; lats.len()];
    let Some(flat) = faces(&wedge, Faces { step: Some(cw::CRUST_STEP), ..Faces::default() }) else { return };
    let Some(lifted) = faces(&wedge, Faces { lift: Some(&up), ..Faces::default() }) else { return };
    let r0 = max(points(&flat).iter().map(norm));
    let r1 = max(points(&lifted).iter().map(norm));
    close(r1 - r0, 500000.0, 50.0);
    let Some(core) = faces(&wedge, Faces { shells: &cw::SHELLS[1..2], lift: Some(&up), ..Faces::default() }) else { return };
    assert!(max(points(&core).iter().map(norm)) / ss::A > 3480.0 / 6371.0 + 0.04);
    // Высоты просятся по геодезической широте: у экватора и полюса она
    // совпадает с геоцентрической.
    let seen = RefCell::new(Vec::new());
    let record = |lats: &[f64], _: &[f64]| {
        seen.borrow_mut().extend_from_slice(lats);
        vec![0.0; lats.len()]
    };
    let _ = cw::faces(&wedge, &Faces { lift: Some(&record), ..Faces::default() });
    close(max(seen.borrow().iter().copied()), 90.0, 1e-6);
}

#[test]
fn missing_lift_heights_stay_finite() {
    // Высота NaN у точки дуги - край на эллипсоиде, а не вершина с NaN.
    let wedge = cw::wedge_at(10.0, 0.0, cw::HALF);
    let lift = |lats: &[f64], _: &[f64]| (0..lats.len()).map(|i| if i % 7 == 0 { f64::NAN } else { 300000.0 }).collect();
    let Some(mesh) = faces(&wedge, Faces { lift: Some(&lift), ..Faces::default() }) else { return };
    assert!(mesh.vertices.iter().all(|v| v.position.iter().all(|x| x.is_finite())));
}

// Выделение коры издалека.

#[test]
fn stretch_keeps_ends_and_order() {
    let depth: Vec<f64> = (0..141).map(|i| 5.0 * i as f64).collect();
    let shown: Vec<f64> = depth.iter().map(|d| cw::stretch(*d, 8.0)).collect();
    assert_eq!(shown[0], 0.0);
    close(cw::stretch(400.0, 8.0), 400.0, 1e-7);
    assert!(shown.windows(2).all(|w| w[1] > w[0]));
    for (d, s) in depth.iter().zip(&shown).filter(|(d, _)| **d >= cw::STRETCH_DEPTH) {
        close(*s, *d, 1e-8);
    }
    // У поверхности - в gain раз.
    close(cw::stretch(0.01, 8.0) / 0.01, 8.0, 0.01);
    assert_eq!(cw::stretch(10.0, 1.0), 10.0);
}

#[test]
fn gain_steps_with_distance() {
    assert_eq!(cw::gain_for(5.0e5), 1.0);
    assert_eq!(cw::gain_for(cw::GAIN_DISTANCE * 4.5), 4.0);
    assert_eq!(cw::gain_for(1.0e9), 8.0);
}

#[test]
fn stretched_core_unchanged() {
    let wedge = cw::wedge_at(10.0, 0.0, cw::HALF);
    let Some(plain) = faces(&wedge, Faces { shells: &cw::SHELLS[..2], ..Faces::default() }) else { return };
    let Some(far) = faces(&wedge, Faces { shells: &cw::SHELLS[..2], gain: 8.0, ..Faces::default() }) else { return };
    assert_eq!(plain.vertices.len(), far.vertices.len());
    for (a, b) in plain.vertices.iter().zip(&far.vertices) {
        for k in 0..3 {
            close(f64::from(a.position[k]), f64::from(b.position[k]), 1e-3);
        }
    }
}

// Грани с моделью коры: мантия до Мохо точки, слои CRUST1.0.

/// Вода 4 км, осадки 1 км, кора до Мохо 30 км везде.
fn crust_bounds() -> Vec<f32> {
    [0.0, -4.0, -4.0, -5.0, -5.0, -5.0, -15.0, -22.0, -30.0].repeat(cr::ROWS * cr::COLS)
}

fn crust_model(vp: Option<Vec<f32>>) -> Option<Crust> {
    let model = Crust::new(crust_bounds(), vp);
    assert!(model.is_ok());
    model.ok()
}

#[test]
fn layer_keys_match_model() {
    assert_eq!(cw::crust_layers(), cr::LAYERS);
}

#[test]
fn mantle_reaches_moho() {
    let Some(model) = crust_model(None) else { return };
    let wedge = cw::wedge_at(10.0, 0.0, cw::HALF);
    let Some(mantle) = faces(&wedge, Faces { shells: &cw::SHELLS[4..5], crust: Some(&model), ..Faces::default() }) else { return };
    // Только верхняя мантия и слои коры: самая высокая точка мантии
    // ищется среди вершин цвета мантии.
    let color = cw::SHELLS[4].color;
    let own = mantle.vertices.iter().zip(points(&mantle)).filter(|(v, _)| v.color[..3] == color).map(|(_, p)| norm(&p));
    close(max(own) / ss::A, (cw::PREM_RADIUS - 30.0) / cw::PREM_RADIUS, 1e-5);
}

#[test]
fn water_layer_on_top() {
    let Some(model) = crust_model(None) else { return };
    let wedge = cw::wedge_at(10.0, 0.0, cw::HALF);
    let Some(mesh) = faces(&wedge, Faces { shells: &[], crust: Some(&model), ..Faces::default() }) else { return };
    assert!(cw::crust_color("water").is_some_and(|c| has(&mesh, c)));
    // Лёд нулевой толщины не рисуется.
    assert!(cw::crust_color("ice").is_some_and(|c| !has(&mesh, c)));
}

#[test]
fn oceanic_crust_has_own_color() {
    // Западнее 0° - базальты, восточнее - континентальная кора. Сектор
    // у 0° в. д. задевает обе.
    let mut vp = vec![6.5f32; cr::ROWS * cr::COLS * cr::BOUNDS];
    for (cell, speeds) in vp.chunks_mut(cr::BOUNDS).enumerate() {
        speeds[cr::UPPER_CRUST] = if cell % cr::COLS < 180 { 5.0 } else { 6.1 };
    }
    let Some(model) = crust_model(Some(vp)) else { return };
    let wedge = cw::wedge_at(10.0, 0.0, cw::HALF);
    let Some(mesh) = faces(&wedge, Faces { shells: &[], crust: Some(&model), ..Faces::default() }) else { return };
    for (key, sea) in cw::OCEAN_COLORS {
        assert!(has(&mesh, sea), "{key}");
        assert!(cw::crust_color(key).is_some_and(|c| has(&mesh, c)), "{key}");
    }
    // Без скоростей - одна континентальная раскраска.
    let Some(model) = crust_model(None) else { return };
    let Some(plain) = faces(&wedge, Faces { shells: &[], crust: Some(&model), ..Faces::default() }) else { return };
    assert!(cw::ocean_color("crust_upper").is_some_and(|c| !has(&plain, c)));
}

// Слой коры на континентальную и океаническую части.

#[test]
fn parts_meet_without_gap_or_overlap() {
    let top: Vec<f64> = (0..8).map(f64::from).collect();
    let bottom: Vec<f64> = top.iter().map(|t| t + 10.0).collect();
    let ocean = [false, false, false, true, true, false, false, false];
    let parts = cw::split_types(&top, &bottom, Some(&ocean));
    let segments = |kind: bool| -> BTreeSet<usize> {
        let Some((_, t, _)) = parts.iter().find(|(k, _, _)| *k == kind) else { return BTreeSet::new() };
        (0..t.len() - 1).filter(|i| t[*i].is_finite() && t[i + 1].is_finite()).collect()
    };
    let (land, sea) = (segments(false), segments(true));
    assert!(land.is_disjoint(&sea));
    assert_eq!(land.union(&sea).copied().collect::<Vec<_>>(), (0..7).collect::<Vec<_>>());
    assert_eq!(sea, BTreeSet::from([2, 3, 4]));
}

#[test]
fn without_types_one_part() {
    let parts = cw::split_types(&[0.0, 1.0], &[5.0, 6.0], None);
    assert_eq!(parts.len(), 1);
    assert!(!parts[0].0);
}

#[test]
fn label_points_on_latitude_face() {
    let wedge = cw::wedge_at(10.0, 20.0, cw::HALF);
    let labels = cw::label_points(&wedge, &cw::SHELLS);
    assert_eq!(labels.len(), cw::SHELLS.len());
    let (key, lat, lon, alt) = labels[0];
    assert_eq!((key, lat), ("inner_core", 0.0));
    close(lon, 20.0, 1e-9);
    close(alt, (1221.5 / 2.0 / cw::PREM_RADIUS - 1.0) * ss::A, 1e-6);
}
