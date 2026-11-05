// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Чтение 3D Tiles: своя выгрузка ложится обратно на своё место в ECEF,
//! кусок B3DM со сдвигом RTC_CENTER, срез дерева REPLACE и ADD.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;

use globe_core::ellipsoid::Ellipsoid;
use scene::sceneexport as se;
use scene::tiles3d;

/// Треугольник: вершина up метров над точкой и две на земле.
fn wall(lat: f64, lon: f64, up: f64) -> se::Item {
    let center = Ellipsoid::wgs84().geodetic_to_ecef(lat, lon, 0.0);
    let axes = se::enu_axes(lat, lon);
    let points = [[0.0, 0.0, up], [5.0, 0.0, 0.0], [0.0, 5.0, 0.0]];
    let positions = points.iter().map(|p: &[f64; 3]| [0, 1, 2].map(|k| (p[0] * axes[0][k] + p[1] * axes[1][k] + p[2] * axes[2][k]) as f32)).collect();
    se::Item { center: center.to_array(), positions, colors: Some(vec![[255, 0, 0, 255]; 3]), indices: Some(vec![0, 1, 2]), ..Default::default() }
}

/// Вершины куска в ECEF как у исходного куска выгрузки.
fn same_place(piece: &tiles3d::Piece, item: &se::Item) {
    let got: Vec<[f64; 3]> = piece.parts.iter().flat_map(|p| p.positions.iter().map(|v| v.to_array())).collect();
    assert_eq!(got.len(), item.positions.len());
    for want in &item.positions {
        let w: Vec<f64> = (0..3).map(|k| f64::from(want[k]) + item.center[k]).collect();
        let near = got.iter().any(|g| (0..3).all(|k| (g[k] - w[k]).abs() < 0.01));
        assert!(near, "{w:?} не найдена в {got:?}");
    }
}

#[test]
fn own_export_opens_back_from_zip() {
    let items = [wall(58.0, 56.0, 10.0), wall(58.01, 56.0, 20.0)];
    let zip = se::tiles_zip(&items).expect("архив");
    assert!(tiles3d::zip_has_tileset(&zip));
    let set = tiles3d::read_zip(&zip).expect("набор");
    assert!(set.skipped.is_empty(), "{:?}", set.skipped);
    assert_eq!(set.pieces.len(), 2);
    for (piece, item) in set.pieces.iter().zip(&items) {
        same_place(piece, item);
    }
}

#[test]
fn folder_of_files_and_detection() {
    let item = wall(-33.9, 18.4, 5.0);
    let (text, files) = se::tileset(std::slice::from_ref(&item)).expect("набор");
    assert!(tiles3d::is_tileset(text.as_bytes()));
    assert!(!tiles3d::is_tileset(br#"{"type": "FeatureCollection", "features": []}"#));
    let disk: HashMap<String, Vec<u8>> = files.into_iter().collect();
    let set = tiles3d::read(text.as_bytes(), &|p| disk.get(p).cloned()).expect("набор");
    same_place(&set.pieces[0], &item);
    // Пропавший кусок - в списке непрочитанных, набор открывается.
    let set = tiles3d::read(text.as_bytes(), &|_| None).expect("набор");
    assert!(set.pieces.is_empty() && set.skipped.len() == 1);
}

/// Кусок B3DM вокруг GLB: таблица признаков с RTC_CENTER.
fn b3dm(glb: &[u8], rtc: [f64; 3]) -> Vec<u8> {
    let mut table = format!(r#"{{"BATCH_LENGTH":0,"RTC_CENTER":[{},{},{}]}}"#, rtc[0], rtc[1], rtc[2]).into_bytes();
    while (28 + table.len()) % 8 != 0 {
        table.push(b' ');
    }
    let total = 28 + table.len() + glb.len();
    let mut out = b"b3dm".to_vec();
    for w in [1u32, total as u32, table.len() as u32, 0, 0, 0] {
        out.extend(w.to_le_bytes());
    }
    out.extend(&table);
    out.extend(glb);
    out
}

#[test]
fn b3dm_with_rtc_center_and_replace_cut() {
    // Кусок выгрузки без сдвига тайла: вершины - смещения от центра,
    // центр - RTC_CENTER куска B3DM.
    let item = wall(58.0, 56.0, 10.0);
    let (text, files) = se::tileset(std::slice::from_ref(&item)).expect("набор");
    let set: serde_json::Value = serde_json::from_str(&text).unwrap();
    let shift: Vec<f64> = (12..15).map(|i| set["root"]["children"][0]["transform"][i].as_f64().unwrap()).collect();
    let fine = b3dm(&files[0].1, [shift[0], shift[1], shift[2]]);
    // Грубый кусок - тот же GLB у нуля: REPLACE заменяет его детьми.
    let coarse = files[0].1.clone();
    let tileset = r#"{"asset":{"version":"1.0"},"geometricError":100,"root":{"geometricError":50,"refine":"REPLACE","content":{"uri":"coarse.glb"},"children":[{"geometricError":0,"content":{"uri":"tiles/fine.b3dm"}}]}}"#;
    let disk: HashMap<String, Vec<u8>> = [("coarse.glb".to_string(), coarse), ("tiles/fine.b3dm".to_string(), fine)].into_iter().collect();
    let read = tiles3d::read(tileset.as_bytes(), &|p| disk.get(p).cloned()).expect("набор");
    assert_eq!(read.pieces.len(), 1, "{:?}", read.pieces.iter().map(|p| &p.name).collect::<Vec<_>>());
    assert_eq!(read.pieces[0].name, "tiles/fine.b3dm");
    same_place(&read.pieces[0], &item);
    // ADD - грубый кусок остаётся вместе с детьми.
    let added = tileset.replace("REPLACE", "ADD");
    let read = tiles3d::read(added.as_bytes(), &|p| disk.get(p).cloned()).expect("набор");
    assert_eq!(read.pieces.len(), 2);
}

#[test]
fn hostile_sets_do_not_break() {
    assert!(tiles3d::read(b"{}", &|_| None).is_err());
    assert!(tiles3d::read(b"not json", &|_| None).is_err());
    // Внешний набор сам на себя - глубина ограничена.
    let selfish = br#"{"asset":{"version":"1.0"},"root":{"geometricError":1,"content":{"uri":"tileset.json"}}}"#;
    let read = tiles3d::read(selfish, &|_| Some(selfish.to_vec())).expect("набор");
    assert!(read.pieces.is_empty());
    // Короткий B3DM - ошибка куска, не паника.
    assert!(tiles3d::content(b"b3dm\x01\x00\x00\x00", &|_| None).is_err());
}
