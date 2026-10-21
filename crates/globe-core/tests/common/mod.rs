// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Общее для тестов ядра.

#![allow(dead_code)]

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
pub const PERM: (f64, f64) = (58.0105, 56.2294);
