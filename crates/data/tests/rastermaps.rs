// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Карты картинками: запись Terrarium и раскраска, прозрачный цвет «нет
//! данных», значение по цвету пикселя, запрос SoilGrids. Перенос
//! `tests/test_fields.py`, `test_emag.py` и `test_tilemaps.py` модуля.

use data::rastermaps::{self, FIELDS, GLIM_CLASSES, MAPS, MapValue, Reading};

/// Тайл из одного цвета, n пикселей.
fn tile(px: [u8; 4], n: usize) -> Vec<u8> {
    px.iter().copied().cycle().take(n * 4).collect()
}

#[test]
fn scale_round_trip() {
    // Возраст с множителем 10 держит десятые доли млн лет.
    let field = rastermaps::field("seafloor_age").map(|f| f.scale).unwrap_or(0.0);
    for v in [0.0, 1.8, 132.6] {
        let back = rastermaps::decode(rastermaps::encode(Some(v), field), field).unwrap_or(f64::NAN);
        assert!((back - v).abs() < 0.051, "{v} {back}");
    }
    assert_eq!(rastermaps::encode(None, field), [0, 0, 0]);
    assert_eq!(rastermaps::decode([0, 0, 0], field), None);
}

#[test]
fn emag_values_and_colors() {
    let emag = rastermaps::field("emag");
    assert!(emag.is_some());
    let Some(emag) = emag else {
        return;
    };
    // Запись: -250.4 -> -250, 312.6 -> 313, ноль - не «нет данных».
    let back = |v: f64| rastermaps::decode(rastermaps::encode(Some(v), 1.0), 1.0);
    assert_eq!(back(-250.4), Some(-250.0));
    assert_eq!(back(312.6), Some(313.0));
    assert_eq!(back(0.0), Some(0.0));
    let low = rastermaps::color(emag, Some(-300.0));
    let high = rastermaps::color(emag, Some(300.0));
    assert!(low[2] > low[0], "слабее - синее");
    assert!(high[0] > high[2], "сильнее - краснее");
    assert_eq!(rastermaps::color(emag, None), [0, 0, 0, 0]);
    let zero = rastermaps::color(emag, Some(0.0));
    assert_eq!(zero[3], (255.0 * emag.opacity).round() as u8);
    // Альфа премножена: цвет не ярче альфы.
    for c in [low, high, zero] {
        assert!(c[..3].iter().all(|v| *v <= c[3]));
    }
}

#[test]
fn value_at_and_empty() {
    let heat = rastermaps::field("heatflow");
    assert!(heat.is_some());
    let Some(heat) = heat else {
        return;
    };
    let mut t = tile([0, 0, 0, 255], 256 * 256);
    let code = rastermaps::encode(Some(178.1), heat.scale);
    t[..3].copy_from_slice(&code);
    let v = rastermaps::value_at(heat, &t, 0, 0).unwrap_or(0.0);
    assert!((v - 178.1).abs() < 1e-3);
    assert_eq!(rastermaps::value_at(heat, &t, 1, 0), None);
    let colors = rastermaps::field_rgba(heat, &t);
    assert!(colors.is_some());
    let Some(colors) = colors else {
        return;
    };
    assert!(colors[3] > 0);
    assert_eq!(colors[7], 0);
    assert_eq!(rastermaps::field_rgba(heat, &tile([0, 0, 0, 255], 16)), None);
}

#[test]
fn every_field_is_described() {
    for f in &FIELDS {
        let stops: Vec<f64> = f.stops.iter().map(|s| s.0).collect();
        assert!(stops.windows(2).all(|w| w[0] < w[1]), "{}", f.key);
        // Код Terrarium - до 65535: предел раскраски влезает.
        assert!(stops.iter().all(|s| s.abs() * f.scale < 32767.0), "{}", f.key);
        assert!(rastermaps::field_url(f).ends_with(&format!("{}/{{z}}/{{x}}/{{y}}.png", f.folder)));
    }
}

#[test]
fn pixel_of_point() {
    assert_eq!(rastermaps::pixel_of(0.0, 0.0, 1), ((1, 1), (0, 0)));
    assert_eq!(rastermaps::pixel_of(58.0, 56.25, 5).0, (21, 9));
}

#[test]
fn every_map_has_template() {
    for m in &MAPS {
        let wms = m.url.contains("{bbox}");
        assert!(wms || m.url.contains("{z}"), "{}", m.key);
        assert!(m.clear.is_some() || !matches!(m.reading, Reading::Query(..)), "{}", m.key);
    }
}

#[test]
fn white_of_soil_map_is_clear() {
    let clay = rastermaps::tile_map("soil_clay");
    assert!(clay.is_some());
    let Some(clay) = clay else {
        return;
    };
    let mut t = tile([255, 255, 255, 255], 4);
    assert_eq!(rastermaps::map_rgba(clay, &t, 0.8), None);
    t[..3].copy_from_slice(&[201, 140, 80]);
    let out = rastermaps::map_rgba(clay, &t, 0.5);
    assert!(out.is_some());
    let Some(out) = out else {
        return;
    };
    assert_eq!(out[3], 128);
    assert_eq!(out[0], (201.0f64 * 0.5).round() as u8);
    assert_eq!(out[15], 0);
}

#[test]
fn empty_tile_is_none() {
    let water = rastermaps::tile_map("water_transitions");
    assert!(water.is_some());
    let Some(water) = water else {
        return;
    };
    assert_eq!(rastermaps::map_rgba(water, &tile([0, 0, 0, 0], 16), 0.9), None);
}

#[test]
fn class_by_pixel_colour() {
    let (lith, water) = (rastermaps::tile_map("glim"), rastermaps::tile_map("water_transitions"));
    assert!(lith.is_some() && water.is_some());
    let (Some(lith), Some(water)) = (lith, water) else {
        return;
    };
    let sc = GLIM_CLASSES.iter().find(|c| c.0 == "sc").map_or([0; 3], |c| c.1);
    assert_eq!(rastermaps::value_of(lith, [sc[0], sc[1], sc[2], 255]), Some(MapValue::Class("sc")));
    assert_eq!(rastermaps::value_of(lith, [sc[0], sc[1], sc[2], 0]), None);
    assert_eq!(rastermaps::value_of(water, [208, 47, 60, 255]), Some(MapValue::Class("lost_permanent")));
    assert_eq!(rastermaps::value_of(water, [60, 60, 60, 255]), None);
}

#[test]
fn water_occurrence_is_blue_share() {
    let water = rastermaps::tile_map("water_occurrence");
    assert!(water.is_some());
    let Some(water) = water else {
        return;
    };
    assert_eq!(rastermaps::value_of(water, [2, 0, 252, 252]), Some(MapValue::Percent(99)));
    assert_eq!(rastermaps::value_of(water, [237, 0, 17, 18]), Some(MapValue::Percent(7)));
}

#[test]
fn worldcover_coarse_levels_from_terrain() {
    let cover = rastermaps::tile_map("worldcover");
    assert!(cover.is_some());
    let Some(cover) = cover else {
        return;
    };
    assert!(rastermaps::map_template(cover, 5).ends_with("worldcover/{z}/{x}/{y}.png"));
    let fine = rastermaps::map_template(cover, 6);
    assert!(fine.contains("planetarycomputer"));
    assert_eq!(rastermaps::value_of(cover, [240, 150, 255, 255]), Some(MapValue::Class("crop")));
    let lith = rastermaps::tile_map("glim");
    assert!(lith.is_some());
    let Some(lith) = lith else {
        return;
    };
    assert!(rastermaps::map_template(lith, 2).ends_with("glim/{z}/{x}/{y}.png"));
}

#[test]
fn soil_query() {
    let (clay, lith) = (rastermaps::tile_map("soil_clay"), rastermaps::tile_map("glim"));
    assert!(clay.is_some() && lith.is_some());
    let (Some(clay), Some(lith)) = (clay, lith) else {
        return;
    };
    let url = rastermaps::query_url(clay, 58.0, 56.25).unwrap_or_default();
    assert!(url.contains("BBOX=56.24400,57.99400,56.25600,58.00600"), "{url}");
    assert!(url.contains("QUERY_LAYERS=clay_0-5cm_mean"));
    // Ответ GetFeatureInfo SoilGrids у Перми 17 октября 2026 года.
    let answer = b"<html><body><b>Value: </b><span id=\"value\">250</span><br><b>Query Layer: </b><span id=\"value\">clay_0-5cm_mean</span></body></html>";
    assert_eq!(rastermaps::parse_query(clay, answer), Some(25.0));
    assert_eq!(rastermaps::parse_query(clay, b"<html></html>"), None);
    assert_eq!(rastermaps::query_url(lith, 0.0, 0.0), None);
}

#[test]
fn overlay_is_premultiplied() {
    let out = rastermaps::overlay_rgba(&tile([200, 100, 50, 128], 2));
    assert!(out.is_some());
    let Some(out) = out else {
        return;
    };
    assert_eq!(&out[..4], &[100, 50, 25, 128]);
    assert_eq!(rastermaps::overlay_rgba(&tile([200, 100, 50, 0], 2)), None);
}

#[test]
fn compose_scales_ancestor_and_keeps_top() {
    // Предок: левая половина красная, правая прозрачная.
    let mut low = vec![0u8; 256 * 256 * 4];
    for row in 0..256 {
        for col in 0..128 {
            let at = (row * 256 + col) * 4;
            low[at..at + 4].copy_from_slice(&[255, 0, 0, 255]);
        }
    }
    // Правая верхняя четверть предка - прозрачная: итог пуст по цвету.
    let right = rastermaps::compose(&[rastermaps::Piece { rgba: &low, depth: 1, cell: (1, 0), opacity: 1.0 }]).unwrap_or_default();
    assert_eq!(right[128 * 1024 + 512 + 3], 0);
    // Левая верхняя - целиком красная.
    let left = rastermaps::compose(&[rastermaps::Piece { rgba: &low, depth: 1, cell: (0, 0), opacity: 1.0 }]).unwrap_or_default();
    assert_eq!(&left[(200 * 256 + 250) * 4..(200 * 256 + 250) * 4 + 4], &[255, 0, 0, 255]);
    // Слой сверху закрывает нижний.
    let top = [0u8, 0, 255, 255].repeat(256 * 256);
    let both = rastermaps::compose(&[rastermaps::Piece { rgba: &low, depth: 0, cell: (0, 0), opacity: 1.0 }, rastermaps::Piece { rgba: &top, depth: 0, cell: (0, 0), opacity: 1.0 }]).unwrap_or_default();
    assert_eq!(&both[..4], &[0, 0, 255, 255]);
    assert_eq!(rastermaps::compose(&[]), None);
}

#[test]
fn opacity_scales_the_layer() {
    // Непрозрачность слоя меню «⋯» делит альфу картинки.
    let full = vec![255u8; 256 * 256 * 4];
    let half = rastermaps::compose(&[rastermaps::Piece { rgba: &full, depth: 0, cell: (0, 0), opacity: 0.5 }]).unwrap_or_default();
    let a = half.get(3).copied().unwrap_or(0);
    assert!((126..=129).contains(&a), "{a}");
    let none = rastermaps::compose(&[rastermaps::Piece { rgba: &full, depth: 0, cell: (0, 0), opacity: f32::NAN }]).unwrap_or_default();
    assert_eq!(none.get(3), Some(&255));
}