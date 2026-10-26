// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Полярные шапки снимками Esri: проекции и раскладка мозаики, перенос
//! `tests/test_polar.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use globe_core::polar as pl;

/// Точки GDAL 3.13 из QGIS 4.0.3, 23 октября 2026 года: код EPSG,
/// долгота, широта, x, y.
const GDAL: [(u32, f64, f64, f64, f64); 10] = [
    (5936, 0.0, 85.0511, 2274886.901, 2476118.079),
    (5936, 33.0, 88.0, 1988377.792, 2221764.931),
    (5936, -150.0, 86.0, 2000000.000, 1555731.571),
    (5936, 100.0, 89.5, 1947835.552, 2018986.306),
    (5936, 10.0, 70.0, 2766942.470, 4107157.119),
    (3031, 0.0, -85.0511, 0.000, 538031.105),
    (3031, 45.0, -88.0, 153672.731, 153672.731),
    (3031, -120.0, -86.0, -376529.864, -217389.618),
    (3031, 170.0, -89.5, 9433.707, -53501.210),
    (3031, 10.0, -70.0, 381069.927, 2161154.949),
];

#[test]
fn matches_gdal() {
    for (code, lon, lat, x, y) in GDAL {
        let (px, py) = pl::project(lat, lon, code == 5936);
        assert!((px - x).abs() < 0.01, "{code} {px} {x}");
        assert!((py - y).abs() < 0.01, "{code} {py} {y}");
    }
}

#[test]
fn cap_inside_mosaic() {
    for north in [true, false] {
        let lay = pl::Layout::new(north, pl::LEVEL);
        let tiles = lay.tiles();
        assert_eq!(tiles.len(), 36);
        let lat = if north { pl::MAX_LAT } else { -pl::MAX_LAT };
        for lon in -180..180 {
            let (u, v) = lay.uv(lat, f64::from(lon));
            assert!(u > 0.0 && u < 1.0 && v > 0.0 && v < 1.0, "{north} {lon} {u} {v}");
        }
        let (pu, _) = lay.uv(if north { 90.0 } else { -90.0 }, 0.0);
        assert!((pu - 0.5).abs() < 0.1);
        assert!(tiles[0].2.contains("/tile/8/"));
        assert!(tiles[0].2.contains(if north { "Arctic" } else { "Antarctic" }));
    }
}

#[test]
fn mosaic_places_tiles() {
    let lay = pl::Layout::new(false, pl::LEVEL);
    let mut tile = vec![0u8; pl::SIDE * pl::SIDE * 4];
    for px in tile.as_chunks_mut::<4>().0 {
        px[0] = 200;
    }
    let (w, h, out) = lay.mosaic(&[(lay.row0 + 1, lay.col0 + 2, tile), (lay.row0, lay.col0, vec![1, 2, 3])]);
    assert_eq!((w, h), (6 * 256, 6 * 256));
    assert_eq!(out[((256 + 5) * w + 512 + 5) * 4], 200);
    assert_eq!(out[(5 * w + 5) * 4], pl::EMPTY);
}

#[test]
fn rim_color_matches_base() {
    let lay = pl::Layout::new(true, pl::LEVEL);
    let (w, h, mut image) = lay.mosaic(&[]);
    let fill = |image: &mut Vec<u8>, c: [u8; 3]| {
        for px in image.as_chunks_mut::<4>().0 {
            px[..3].copy_from_slice(&c);
        }
    };
    fill(&mut image, [10, 40, 100]);
    let have = pl::rim_color(&lay, w, h, &image);
    assert_eq!(have.len(), pl::SECTORS);
    assert_eq!(have[3].map(f64::round), [10.0, 40.0, 100.0]);
    let mut out = image.clone();
    pl::matched(&mut out, w, h, &lay, &have, &[[15.0, 60.0, 150.0]; 8]);
    assert_eq!(&out[(5 * w + 5) * 4..(5 * w + 5) * 4 + 3], &[15, 60, 150]);
    // Множитель и сдвиг ограничены: тёмный край не делает снимок белым.
    let mut out = image.clone();
    pl::matched(&mut out, w, h, &lay, &have, &[[255.0; 3]; 8]);
    assert_eq!(f64::from(out[(5 * w + 5) * 4 + 2]), 200.0 + pl::SHIFT_MAX);
    // Почти нулевой красный канал Arctic Imagery поднимается сдвигом.
    fill(&mut image, [0, 40, 90]);
    let have = pl::rim_color(&lay, w, h, &image);
    pl::matched(&mut image, w, h, &lay, &have, &[[25.0, 80.0, 100.0]; 8]);
    assert_eq!(&image[(5 * w + 5) * 4..(5 * w + 5) * 4 + 3], &[25, 80, 100]);
}

/// Сектор 0 (долготы -180..-135) светлее вдвое, прочие как есть: точка
/// края на -157.5° вдвое светлее, на 22.5° без изменений.
#[test]
fn sector_gain_follows_longitude() {
    for north in [true, false] {
        let lay = pl::Layout::new(north, pl::LEVEL);
        let (w, h, mut image) = lay.mosaic(&[]);
        for px in image.as_chunks_mut::<4>().0 {
            px[..3].copy_from_slice(&[100, 100, 100]);
        }
        let mut want = [[100.0; 3]; 8];
        want[0] = [200.0; 3];
        pl::matched(&mut image, w, h, &lay, &[[100.0; 3]; 8], &want);
        let lat = if north { pl::RIM_LAT } else { -pl::RIM_LAT };
        let at = |lon: f64| {
            let (u, v) = lay.uv(lat, lon);
            image[(((v * h as f64) as usize) * w + (u * w as f64) as usize) * 4]
        };
        assert!(at(-157.5) > 185, "{north}");
        assert_eq!(at(22.5), 100, "{north}");
    }
}

#[test]
fn needed_poles() {
    assert_eq!(pl::needed(10.0, 2.5e7), [true, false]);
    assert_eq!(pl::needed(70.0, 1.0e5), [true]);
    assert_eq!(pl::needed(-80.0, 1.0e5), [false]);
    assert!(pl::needed(40.0, 1.0e5).is_empty());
}
