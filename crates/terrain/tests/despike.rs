// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Одиночные выбросы тайлов Terrarium: у Перми пиксели на +2600
//! и -2600 м среди высот около 100 м поднимали пики сетки.

#![allow(clippy::unwrap_used)]

use terrain::{SIZE, despike};

#[test]
fn single_spikes_go_peaks_and_slopes_stay() {
    // Пологий склон в целых метрах, как SRTM, с выбросами вверх, вниз
    // и у края тайла.
    let mut h: Vec<f32> = (0..SIZE * SIZE).map(|i| 100.0 + ((i % SIZE) as f32 * 0.3).floor()).collect();
    let slope = h.clone();
    h[100 * SIZE + 100] = 2594.0;
    h[150 * SIZE + 40] = -1371.0;
    h[SIZE + 255] = 860.0;
    despike(&mut h);
    for i in [100 * SIZE + 100, 150 * SIZE + 40, SIZE + 255] {
        assert!((h[i] - slope[i]).abs() <= 1.0, "{i}: {} vs {}", h[i], slope[i]);
    }
    // Склон без выбросов не меняется.
    let changed = h.iter().zip(&slope).filter(|(a, b)| a != b).count();
    assert_eq!(changed, 0);
    // Вершина конуса крутизной в пиксель на пиксель остаётся.
    let mut cone: Vec<f32> = (0..SIZE * SIZE)
        .map(|i| {
            let (r, c) = ((i / SIZE) as f32 - 128.0, (i % SIZE) as f32 - 128.0);
            3000.0 - 30.0 * (r * r + c * c).sqrt()
        })
        .collect();
    let top = cone[128 * SIZE + 128];
    despike(&mut cone);
    assert_eq!(cone[128 * SIZE + 128], top);
}
