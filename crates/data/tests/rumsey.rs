// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Карты Рамсея: указатель, карты у вида, куски IIIF, привязка. Перенос
//! `tests/test_rumsey.py` модуля.

use data::rumsey::{self, Catalogue, Piece, Warp};

const INDEX: &str = r#"{"maps": [
    [0, 37.0, 55.5, 38.0, 56.0, 1739, "Moskva plan", 8400, "50_30"],
    [1, 20.0, 40.0, 60.0, 70.0, 1745, "Russia", 5000000, "50_40"],
    [2, -180.0, -90.0, 180.0, 90.0, 1800, "World", null, "0_0"],
    [3, 2.0, 48.0, 3.0, 49.0, 1700, "Paris", 10000, "40_0"],
    [4, 37.5, 55.6, 37.8, 55.9, 1935, "Moskva centre", 2000, "50_30"]]}"#;

fn catalogue() -> Catalogue {
    let cat = Catalogue::from_text(INDEX);
    assert!(cat.is_ok(), "{cat:?}");
    cat.unwrap_or_default()
}

#[test]
fn covering_order() {
    let cat = catalogue();
    let rows = rumsey::covering(&cat, 55.75, 37.62, [37.3, 55.5, 37.9, 56.0]);
    let numbers: Vec<i64> = rows.iter().map(|r| cat.number[*r]).collect();
    // Подробные сначала, карта мира - в конце, Париж не задет.
    assert_eq!(numbers, [4, 0, 1, 2]);
}

#[test]
fn search_whole_catalogue() {
    let cat = catalogue();
    let found: Vec<i64> = rumsey::search(&cat, "moskva", rumsey::LIST_MAX).iter().map(|r| cat.number[*r]).collect();
    assert_eq!(found, [0, 4]);
    assert_eq!(rumsey::search(&cat, "Moskva 1935", rumsey::LIST_MAX), [4]);
    assert_eq!(rumsey::search(&cat, "  ", rumsey::LIST_MAX), Vec::<usize>::new());
    assert_eq!(rumsey::search(&cat, "moskva", 1), [0]);
}

#[test]
fn fly_distance_grows_with_box() {
    let small = rumsey::fly_distance(&[37.5, 55.6, 37.8, 55.9], 40.0);
    let large = rumsey::fly_distance(&[20.0, 40.0, 60.0, 70.0], 40.0);
    assert!(large > 50.0 * small);
}

#[test]
fn tile_plan() {
    let plan = rumsey::tile_plan(7984, 6160, &[1, 2, 4, 8, 16, 32], rumsey::MAX_SIDE, rumsey::TILE);
    assert!(plan.is_some());
    let Some(plan) = plan else { return };
    assert_eq!(plan.factor, 4);
    assert_eq!((plan.width, plan.height), (1996, 1540));
    assert_eq!(plan.pieces.len(), 8 * 7);
    let last = plan.pieces[plan.pieces.len() - 1];
    assert_eq!(last, Piece { x: 7168, y: 6144, w: 816, h: 16, ox: 1792, oy: 1536, ow: 204, oh: 4 });
    let url = rumsey::piece_url("https://x/iiif/abc", &plan.pieces[0]);
    assert_eq!(url, "https://x/iiif/abc/0,0,1024,1024/256,/0/default.jpg");
}

#[test]
fn small_image_full_size() {
    let plan = rumsey::tile_plan(1000, 800, &[1, 2, 4], rumsey::MAX_SIDE, rumsey::TILE);
    assert_eq!(plan.map(|p| (p.factor, p.width, p.height, p.pieces.len())), Some((1, 1000, 800, 16)));
}

#[test]
fn hostile_plan() {
    // Нули в уменьшениях пропускаются, пустой список - без уменьшения.
    let plan = rumsey::tile_plan(1000, 800, &[0], rumsey::MAX_SIDE, rumsey::TILE);
    assert_eq!(plan.map(|p| p.factor), Some(1));
    assert!(rumsey::tile_plan(u64::MAX, u64::MAX, &[1], rumsey::MAX_SIDE, rumsey::TILE).is_none());
    assert_eq!(rumsey::tile_plan(0, 0, &[], rumsey::MAX_SIDE, rumsey::TILE).map(|p| p.pieces.len()), Some(0));
}

#[test]
fn warp_options() {
    assert_eq!(rumsey::warp_options("tps", 12), Warp::Tps);
    assert_eq!(rumsey::warp_options("polynomial", 6), Warp::Polynomial(2));
    assert_eq!(rumsey::warp_options("polynomial", 10), Warp::Polynomial(3));
    assert_eq!(rumsey::warp_options("affine", 4), Warp::Polynomial(1));
    assert_eq!(rumsey::warp_options("tps", 2), Warp::Polynomial(1));
}

#[test]
fn image_url() {
    assert_eq!(rumsey::image_url("abc"), "https://images.iiifhosting.com/iiif/abc");
    assert_eq!(rumsey::image_url("https://y/z"), "https://y/z");
}

#[test]
fn year_text_and_bad_rows() {
    assert_eq!(rumsey::year_text(Some(1739)), "1739");
    assert_eq!(rumsey::year_text(Some(-99)), "100 до н. э.");
    assert_eq!(rumsey::year_text(None), "");
    let cat = Catalogue::from_text(r#"{"maps": [[1], "x", [2, 1, 2, 3, 4, 1739.0, null, null, null]]}"#).unwrap_or_default();
    assert_eq!(cat.number, [2]);
    assert_eq!(cat.year, [Some(1739)]);
    assert!(Catalogue::from_text("not json").is_err());
    assert!(Catalogue::from_text("[]").is_ok_and(|c| c.is_empty()));
}
