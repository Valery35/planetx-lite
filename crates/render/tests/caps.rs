// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Цвет полярных шапок по краю тайла 0/0/0, как в модуле.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use render::cap_colors;

/// Тайл 4 × 4: верх - тёмная вода с одной белой точкой подписи, низ -
/// лёд с одной тёмной точкой.
#[test]
fn caps_take_edge_median() {
    let (w, h) = (4usize, 4usize);
    let mut rgba = vec![0u8; w * h * 4];
    let mut put = |x: usize, y: usize, c: [u8; 3]| {
        let i = (y * w + x) * 4;
        rgba[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
    };
    for y in 0..h {
        for x in 0..w {
            put(x, y, if y < 2 { [10, 30, 70] } else { [240, 245, 250] });
        }
    }
    put(1, 0, [255, 255, 255]);
    put(2, 3, [20, 20, 20]);
    let [north, south] = cap_colors(w as u32, h as u32, &rgba).unwrap();
    assert_eq!(north, [10, 30, 70]);
    assert_eq!(south, [240, 245, 250]);
}

/// Край тайла - среднее двух крайних строк по столбцам.
#[test]
fn edge_row_averages_two_rows() {
    let (w, h) = (3usize, 4usize);
    let mut rgba = vec![255u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            rgba[i] = (x * 100) as u8;
            rgba[i + 1] = [10, 20, 30, 40][y];
        }
    }
    let north = render::edge_row(w as u32, h as u32, &rgba, true).unwrap();
    assert_eq!(north, vec![[0, 15, 255], [100, 15, 255], [200, 15, 255]]);
    let south = render::edge_row(w as u32, h as u32, &rgba, false).unwrap();
    assert_eq!(south[1], [100, 35, 255]);
}

/// Снимок шапки берёт край тайлов на экране поверх края тайла 0/0/0:
/// у Esri тайл 4 у 85° ю. ш. темнее тайла 0, шапка из тайла 0 выходила
/// светлым кругом.
#[test]
fn strip_takes_drawn_tiles_over_tile_zero() {
    let w = render::CAP_STRIP;
    let fallback = vec![[245, 248, 254]; 256];
    let dark = vec![[239, 242, 253]; 256];
    // Тайл 4/3/15 - долготы от -112.5° до -90°, столбцы 192-255.
    let piece = render::EdgePiece { key: (4, 3, 15), window: [0.0, 0.0, 1.0, 0.0], row: &dark };
    let strip = render::cap_strip(&fallback, &[piece]);
    assert_eq!(strip.len(), w * 2 * 4);
    let at = |c: usize| [strip[c * 4], strip[c * 4 + 1], strip[c * 4 + 2]];
    assert_eq!(at(224), [239, 242, 253]);
    assert_eq!(at(600), [245, 248, 254]);
    // Нижняя строка ровная - медиана верхней.
    assert_eq!(&strip[w * 4..w * 4 + 3], &[245, 248, 254]);
    let only = render::cap_strip(&fallback, &[]);
    assert_eq!([only[896], only[897], only[898]], [245, 248, 254]);
}

#[test]
fn wrong_size_is_none() {
    assert!(cap_colors(4, 4, &[0; 10]).is_none());
    assert!(cap_colors(0, 4, &[]).is_none());
    assert!(cap_colors(4, 1, &[0; 16]).is_none());
}
