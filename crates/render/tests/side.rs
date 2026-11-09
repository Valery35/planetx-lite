// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Отсечение шторкой слоёв: объект стороны виден только по свою сторону
//! линии, без шторки - целиком.

use render::{Side, side_rect};

#[test]
fn sides_split_the_frame_at_the_line() {
    let (w, h) = (800, 600);
    assert_eq!(side_rect(Side::Left, Some(300.4), w, h), Some((0, 0, 300, h)));
    assert_eq!(side_rect(Side::Right, Some(300.4), w, h), Some((300, 0, 500, h)));
    assert_eq!(side_rect(Side::Both, Some(300.0), w, h), Some((0, 0, w, h)));
    // Без шторки сторона не важна.
    assert_eq!(side_rect(Side::Left, None, w, h), Some((0, 0, w, h)));
    // Линия у края: пустая сторона не рисуется, линия за кадром прижата.
    assert_eq!(side_rect(Side::Left, Some(0.0), w, h), None);
    assert_eq!(side_rect(Side::Right, Some(5000.0), w, h), None);
    assert_eq!(side_rect(Side::Left, Some(-20.0), w, h), None);
    assert_eq!(side_rect(Side::Left, Some(f32::NAN), w, h), Some((0, 0, w, h)));
}
