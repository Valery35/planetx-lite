// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Превью строк каталога «Добавить данные»: тайл подложки, шкала цветов,
//! тайл темы поверх подложки. Перенос `ramp_image`, `tile_image`, `_over`
//! из `ui/nasamaps.py` модуля.

use data::thumbs::{self as th, SIDE};

fn pixel(image: &[u8], x: usize, y: usize) -> [u8; 4] {
    let i = (y * SIDE + x) * 4;
    [image[i], image[i + 1], image[i + 2], image[i + 3]]
}

fn solid(side: usize, rgba: [u8; 4]) -> Vec<u8> {
    rgba.iter().copied().cycle().take(side * side * 4).collect()
}

fn opaque(image: &[u8]) -> bool {
    image.len() == SIDE * SIDE * 4 && image.as_chunks::<4>().0.iter().all(|p| p[3] == 255)
}

#[test]
fn fit_scales_tile_to_side() {
    let tile = solid(256, [10, 20, 30, 255]);
    let fitted = th::fit(&tile, 256, 256);
    assert!(fitted.as_ref().is_some_and(|f| f.len() == SIDE * SIDE * 4));
    let fitted = fitted.unwrap_or_default();
    assert_eq!(pixel(&fitted, 0, 0), [10, 20, 30, 255]);
    assert_eq!(pixel(&fitted, SIDE - 1, SIDE - 1), [10, 20, 30, 255]);
    // Размер не сходится с байтами или пустой - превью нет.
    assert!(th::fit(&tile, 255, 256).is_none());
    assert!(th::fit(&[], 0, 0).is_none());
}

#[test]
fn blank_is_background() {
    let image = th::over(None, None);
    assert!(opaque(&image));
    let [r, g, b] = th::BACKGROUND;
    assert_eq!(pixel(&image, 50, 50), [r, g, b, 255]);
}

#[test]
fn ramp_lies_in_bottom_band() {
    let base = th::fit(&solid(256, [0, 0, 255, 255]), 256, 256);
    let colors = [[255, 0, 0], [0, 255, 0]];
    let image = th::ramp(&colors, base.as_deref());
    assert!(opaque(&image));
    // Над полосой - подложка, в полосе - цвета по порядку слева направо.
    assert_eq!(pixel(&image, 10, 10), [0, 0, 255, 255]);
    assert_eq!(pixel(&image, 10, SIDE - 2), [255, 0, 0, 255]);
    assert_eq!(pixel(&image, SIDE - 10, SIDE - 2), [0, 255, 0, 255]);
    let band = SIDE / 4;
    assert_eq!(pixel(&image, 10, SIDE - band - 1), [0, 0, 255, 255]);
    // Без цветов и подложки - фон.
    let [r, g, b] = th::BACKGROUND;
    assert_eq!(pixel(&th::ramp(&[], None), 10, SIDE - 2), [r, g, b, 255]);
}

#[test]
fn premultiplied_top_mixes_with_base() {
    let base = th::fit(&solid(256, [0, 0, 200, 255]), 256, 256);
    // Красный с альфой 0.5, цвет уже умножен на альфу.
    let top = th::fit(&solid(256, [100, 0, 0, 128]), 256, 256);
    let image = th::over(base.as_deref(), top.as_deref());
    assert!(opaque(&image));
    let [r, _, b, _] = pixel(&image, 30, 30);
    assert!((99..=101).contains(&r), "{r}");
    assert!((98..=101).contains(&b), "{b}");
    // Без верхней картинки - подложка как есть.
    assert_eq!(pixel(&th::over(base.as_deref(), None), 30, 30), [0, 0, 200, 255]);
}

#[test]
fn colors_of_known_keys() {
    assert!(th::colors("grid").is_some_and(|c| c.len() == 2));
    assert!(th::colors("geology").is_some());
    assert!(th::colors("no such layer").is_none());
    assert_eq!(th::vector_colors(0).len(), 2);
    assert_eq!(th::vector_colors(4), th::vector_colors(1));
}
