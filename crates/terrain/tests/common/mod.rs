// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Общее для тестов рельефа.

#![allow(dead_code)]

use terrain::{Encoding, HeightTile, SIZE};

/// Генератор SplitMix64 - повторяемые случайные числа без зависимостей.
pub struct Rng(pub u64);

impl Rng {
    pub fn next_f64(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next_f64()
    }

    /// Целое от 0 до n - 1.
    pub fn below(&mut self, n: u32) -> u32 {
        ((self.next_f64() * f64::from(n)) as u32).min(n.saturating_sub(1))
    }
}

/// Пермь, широта и долгота.
pub const PERM_LIKE: (f64, f64) = (58.0105, 56.2294);

/// Высоты в картинку Terrarium RGBA, обратное decode.
pub fn encode(heights: &[f64]) -> Vec<u8> {
    heights
        .iter()
        .flat_map(|h| {
            let value = h + 32768.0;
            let r = (value / 256.0).floor();
            let g = (value - r * 256.0).floor();
            let b = ((value - r * 256.0 - g) * 256.0).round().min(255.0);
            [r as u8, g as u8, b as u8, 255]
        })
        .collect()
}

/// Полный тайл: первые значения - values, остальные пиксели - fill.
pub fn full(values: &[f64], fill: f64) -> Vec<f64> {
    let mut out = vec![fill; SIZE * SIZE];
    out[..values.len()].copy_from_slice(values);
    out
}

pub fn tile_of(z: u32, x: u32, y: u32, heights: &[f64], floor: Option<f32>) -> HeightTile {
    terrain::make_tile(z, x, y, &encode(heights), floor, Encoding::Terrarium).unwrap()
}

/// Тайл, высота которого растёт на восток на 10 м за пиксель.
pub fn ramp_tile(z: u32, x: u32, y: u32) -> HeightTile {
    let heights: Vec<f64> = (0..SIZE * SIZE).map(|i| (i % SIZE) as f64 * 10.0).collect();
    tile_of(z, x, y, &heights, Some(0.0))
}

/// Тайл ровной высоты.
pub fn flat_tile(z: u32, x: u32, y: u32, h: f64) -> HeightTile {
    tile_of(z, x, y, &vec![h; SIZE * SIZE], Some(0.0))
}

/// Тайл, высота которого - функция доли мира u, v в центрах пикселей.
pub fn height_tile(z: u32, x: u32, y: u32, f: impl Fn(f64, f64) -> f64) -> HeightTile {
    let n = f64::from(1u32 << z);
    let heights: Vec<f32> = (0..SIZE * SIZE)
        .map(|i| {
            let (row, col) = ((i / SIZE) as f64, (i % SIZE) as f64);
            let u = (f64::from(x) + (col + 0.5) / SIZE as f64) / n;
            let v = (f64::from(y) + (row + 0.5) / SIZE as f64) / n;
            f(u, v) as f32
        })
        .collect();
    HeightTile::new(z, x, y, heights).unwrap()
}

/// Случайный тайл высот 0..top.
pub fn random_tile(z: u32, x: u32, y: u32, top: f64, seed: u64) -> HeightTile {
    let mut rng = Rng(seed);
    let heights: Vec<f64> = (0..SIZE * SIZE).map(|_| rng.uniform(0.0, top)).collect();
    tile_of(z, x, y, &heights, Some(0.0))
}
