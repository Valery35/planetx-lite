// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Изменения местности и классификация по эмбеддингам AlphaEarth.
//! Перенос `tests/test_aef.py` модуля.

use data::aef::{self, ChangeSum, Mosaic, Piece, ZoneIndex};

const ZONE: &str = "40N";
/// Квадрат у Перми.
const WEST: f64 = 409600.0;
const NORTH: f64 = 6471680.0;
const BANDS: usize = 4;

fn index() -> ZoneIndex {
    let doc = format!("{{\"zone\": \"{ZONE}\", \"cell\": {}, \"cells\": [[{WEST}, {NORTH}, {{\"2017\": \"a:0:0\", \"2025\": \"b:0:0\"}}]]}}", aef::CELL);
    let found = ZoneIndex::parse(doc.as_bytes());
    assert!(found.is_some());
    found.unwrap_or(ZoneIndex { zone: String::new(), number: 40, south: false, cells: Vec::new() })
}

/// Изменения - северная четверть квадрата.
fn north_block(_x: f64, y: f64) -> bool {
    y > NORTH - aef::CELL / 4.0
}

/// Блок (col, row) обзора level: векторы клеток, как у файла AlphaEarth -
/// строки с юга. second - второй год: на севере два канала со знаком
/// минус.
fn block(level: usize, col: u32, row: u32, second: bool) -> Piece<i8> {
    let scale = f64::from(1u32 << level);
    let side = aef::TILE.min(aef::FILE_SIDE >> level) as usize;
    let step = 10.0 * scale;
    let x0 = WEST + f64::from(col) * f64::from(aef::TILE) * step;
    let y0 = NORTH - aef::CELL + f64::from(row) * f64::from(aef::TILE) * step;
    let mut values = Vec::with_capacity(side * side * BANDS);
    for r in 0..side {
        let y = y0 + (r as f64 + 0.5) * step;
        for c in 0..side {
            let x = x0 + (c as f64 + 0.5) * step;
            let flip = second && north_block(x, y);
            values.extend([90, 90, if flip { -90 } else { 90 }, if flip { -90 } else { 90 }]);
        }
    }
    Piece { x0, y0, dx: step, dy: step, rows: side, cols: side, values }
}

/// Степень изменения блока: каналы двух лет по одному.
fn change_piece(level: usize, col: u32, row: u32) -> Piece<u8> {
    let (a, b) = (block(level, col, row, false), block(level, col, row, true));
    let pixels = a.rows * a.cols;
    let mut sum = ChangeSum::new(pixels);
    for band in 0..BANDS {
        let pa: Vec<i8> = a.values.iter().skip(band).step_by(BANDS).copied().collect();
        let pb: Vec<i8> = b.values.iter().skip(band).step_by(BANDS).copied().collect();
        sum.add(&pa, &pb);
    }
    Piece { x0: a.x0, y0: a.y0, dx: a.dx, dy: a.dy, rows: a.rows, cols: a.cols, values: sum.finish() }
}

fn point(northing: f64) -> (f64, f64) {
    aef::from_utm(40, false, WEST + aef::CELL / 2.0, northing)
}

fn square(lat: f64, lon: f64, side: f64) -> (f64, f64, f64, f64) {
    data::site::square(lat, lon, side)
}

#[test]
fn file_url_and_zone() {
    assert_eq!(aef::file_url(2019, "1S", "x8qq:1:0").as_deref(), Some(format!("{}2019/1S/x8qq-0000008192-0000000000.tiff", aef::BASE).as_str()));
    assert!(aef::file_url(2019, "1S", "x8qq").is_none());
    assert_eq!(aef::zone_of(56.25, 58.0), "40N");
    assert_eq!(aef::zone_of(-123.0, -45.0), "10S");
    assert_eq!(aef::zone_of(180.0, 10.0), "60N");
    assert!(aef::index_url("40N").ends_with("aef/40N.json"));
}

#[test]
fn utm_round_trip() {
    let (x, y) = aef::to_utm(58.0, 56.25, 40, false);
    let (lat, lon) = aef::from_utm(40, false, x, y);
    assert!((lat - 58.0).abs() < 1e-9 && (lon - 56.25).abs() < 1e-9);
    // Пермь в зоне 40: восток около 455 км, север около 6430 км.
    assert!((x - 455_000.0).abs() < 5000.0 && (y - 6_430_000.0).abs() < 5000.0, "{x} {y}");
}

#[test]
fn change_of_vectors() {
    let mut sum = ChangeSum::new(3);
    for _ in 0..BANDS {
        sum.add(&[90, 90, -128], &[90, -90, 90]);
    }
    let out = sum.finish();
    assert!(aef::from_byte(out[0]) < 0.01);
    // Противоположные векторы - несходство 2, шкала обрезает до 1.
    assert!((aef::from_byte(out[1]) - 1.0).abs() < 1e-9);
    assert_eq!(out[2], aef::EMPTY);
}

#[test]
fn dequantize_like_google() {
    for v in [-127i8, -40, 0, 1, 127] {
        let x = f64::from(v) / 127.5;
        assert!((f64::from(aef::dequantize(v)) - x * x * f64::from(v).signum()).abs() < 1e-6);
    }
    assert!(aef::dequantize(aef::NODATA).is_nan());
}

#[test]
fn bytes_round_trip_and_colors() {
    for v in [0.0, 0.5, 1.0] {
        assert!((aef::from_byte(aef::to_byte(v)) - v).abs() < 0.003);
    }
    assert!(aef::from_byte(aef::to_byte(f64::NAN)).is_nan());
    assert!(aef::color(0.1, 0.3, 1.0).is_none());
    assert!(aef::color(0.5, 0.3, 1.0).is_some());
    assert!(aef::color(f64::NAN, 0.0, 1.0).is_none());
    assert_eq!(aef::color(1.0, 0.0, 1.0), Some([255, 245, 30]));
}

#[test]
fn mosaic_puts_change_north_and_spots() {
    // Изменение в северной четверти квадрата ложится на север: строки
    // файлов идут с юга, у переворота изменение оказалось бы на юге.
    let index = index();
    let (lat, lon) = point(NORTH - aef::CELL / 4.0);
    let bbox = square(lat, lon, 4000.0);
    let parts = aef::detail_parts(&index, bbox, 2);
    assert!(!parts.is_empty());
    let pieces: Vec<Piece<u8>> = parts.iter().map(|(_, c, r)| change_piece(2, *c, *r)).collect();
    let m: Option<Mosaic<u8>> = aef::mosaic(&pieces, &index, bbox, 1, aef::EMPTY);
    assert!(m.is_some());
    let Some(m) = m else { return };
    assert_eq!(m.step, 40.0);
    let edge = NORTH - aef::CELL / 4.0;
    let (mut north, mut south) = (Vec::new(), Vec::new());
    for r in 0..m.rows {
        let y = m.y0 - (r as f64 + 0.5) * m.step;
        for c in 0..m.cols {
            let v = m.at(r, c).and_then(|v| v.first()).map_or(f64::NAN, |b| aef::from_byte(*b));
            if y > edge + 100.0 {
                north.push(v);
            } else if y < edge - 100.0 {
                south.push(v);
            }
        }
    }
    let median = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(f64::NAN)
    };
    assert!(median(&mut north) > 0.4);
    assert!(median(&mut south) < 0.05);
    let values: Vec<f64> = m.values.iter().map(|b| aef::from_byte(*b)).collect();
    let spots = aef::hot_spots(&values, m.rows, m.cols, 3, 5, 0.3);
    assert!(!spots.is_empty());
    assert!(m.y0 - (spots[0].0 as f64 + 0.5) * m.step > edge);
}

#[test]
fn detail_parts_south_up() {
    let index = index();
    let (lat, lon) = point(NORTH - 5000.0);
    let parts = aef::detail_parts(&index, square(lat, lon, 1000.0), 0);
    // Северный край квадрата - последняя строка блоков.
    assert!(!parts.is_empty());
    assert!(parts.iter().all(|p| p.2 == 7));
    assert_eq!(aef::detail_megabytes(parts.len(), 0), 2.0 * parts.len() as f64 * aef::BLOCK_MB[0]);
}

#[test]
fn auto_level_fits_blocks_and_cells() {
    // Середина квадрата файла - угол блоков всех уровней мельче 80 м.
    let index = index();
    let (lat, lon) = point(NORTH - aef::CELL / 2.0);
    assert_eq!(aef::auto_level(&index, square(lat, lon, 2000.0), 2000.0), 0);
    let side = 10000.0;
    let bbox = square(lat, lon, side);
    let level = aef::auto_level(&index, bbox, side);
    // У угла блоков меньше всего грузит 80 м - один блок.
    assert_eq!(level, 3);
    assert_eq!(aef::detail_parts(&index, bbox, level).len(), 1);
}

#[test]
fn classes_follow_control_points() {
    let index = index();
    let edge = NORTH - aef::CELL / 4.0;
    let (lat, lon) = aef::from_utm(40, false, WEST + aef::CELL / 2.0, edge);
    let bbox = square(lat, lon, 4000.0);
    let pieces: Vec<Piece<i8>> = aef::detail_parts(&index, bbox, 2).iter().map(|(_, c, r)| block(2, *c, *r, true)).collect();
    let grid = aef::mosaic(&pieces, &index, bbox, BANDS, aef::NODATA);
    assert!(grid.is_some());
    let Some(grid) = grid else { return };
    let (mut samples, mut labels) = (Vec::new(), Vec::new());
    for (label, shift) in [(0, -1500.0), (1, 1500.0)] {
        for dx in [-500.0, 0.0, 500.0] {
            let cell = grid.cell_of(WEST + aef::CELL / 2.0 + dx, edge + shift);
            assert!(cell.is_some());
            let Some((r, c)) = cell else { return };
            samples.push(grid.at(r, c).map(<[i8]>::to_vec).unwrap_or_default());
            labels.push(label);
        }
    }
    let classes = aef::classify(&grid.values, BANDS, &samples, &labels, aef::KNN);
    for r in 0..grid.rows {
        let y = grid.y0 - (r as f64 + 0.5) * grid.step;
        for c in 0..grid.cols {
            let got = classes.get(r * grid.cols + c).copied().flatten();
            if y > edge + 100.0 {
                assert_eq!(got.map(|g| g.0), Some(1));
            } else if y < edge - 100.0 {
                assert_eq!(got.map(|g| g.0), Some(0));
            }
            if let Some((_, score)) = got {
                assert!(score > 0.99);
            }
        }
    }
}

#[test]
fn cells_without_data_have_no_class() {
    let grid: Vec<i8> = [[-128, -128, -128, -128], [90, 90, 90, 90], [90, 90, 90, 90], [90, 90, -90, -90]].concat();
    let samples = vec![vec![90, 90, 90, 90], vec![90, 90, -90, -90]];
    let classes = aef::classify(&grid, BANDS, &samples, &[3, 7], aef::KNN);
    let got: Vec<Option<i32>> = classes.iter().map(|c| c.map(|v| v.0)).collect();
    assert_eq!(got, vec![None, Some(3), Some(3), Some(7)]);
    assert!(aef::unit(&[90, 90, 90, 90]).is_some_and(|u| (u.iter().map(|v| v * v).sum::<f32>() - 1.0).abs() < 1e-5));
    assert!(aef::unit(&[aef::NODATA, 90, 90, 90]).is_none());
}

#[test]
fn hostile_index_gives_nothing() {
    assert!(ZoneIndex::parse(b"not json").is_none());
    assert!(ZoneIndex::parse(b"{\"zone\": \"99N\", \"cells\": []}").is_none());
    assert!(ZoneIndex::parse(b"{\"zone\": \"N\", \"cells\": []}").is_none());
    let index = ZoneIndex::parse(b"{\"zone\": \"40N\", \"cells\": [[1, 2], \"x\"]}");
    assert!(index.is_some_and(|i| i.cells.is_empty()));
}
