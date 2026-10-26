// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Гидрокоррекция. Перенос `tests/test_hydro_fill.py` модуля
//! (из Isoliner). Случайный рельеф - свой генератор вместо NumPy,
//! проверки те же.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use data::hydro_fill::{Fill, fill_depressions};

/// Равномерные числа [0, 1) - xorshift64*.
fn uniform(seed: u64, n: usize) -> Vec<f64> {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            s ^= s >> 12;
            s ^= s << 25;
            s ^= s >> 27;
            (s.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
        })
        .collect()
}

/// Нет ячейки, которая строго ниже всех восьми соседей.
fn has_no_interior_pit(z: &[f64], rows: usize, cols: usize, mask: Option<&[bool]>) -> bool {
    let off = |i: usize| mask.is_some_and(|m| m[i]);
    for r in 1..rows - 1 {
        for c in 1..cols - 1 {
            if off(r * cols + c) {
                continue;
            }
            let mut low = f64::INFINITY;
            let mut any = false;
            for dr in [-1i64, 0, 1] {
                for dc in [-1i64, 0, 1] {
                    if dr == 0 && dc == 0 {
                        continue;
                    }
                    let j = ((r as i64 + dr) as usize) * cols + (c as i64 + dc) as usize;
                    if !off(j) {
                        low = low.min(z[j]);
                        any = true;
                    }
                }
            }
            if any && z[r * cols + c] < low - 1e-12 {
                return false;
            }
        }
    }
    true
}

fn fill(z: &[f64], rows: usize, cols: usize, eps: f64) -> Fill {
    fill_depressions(z, rows, cols, None, eps, None).unwrap()
}

#[test]
fn single_pit_filled_eps_zero() {
    // Epsilon 0: поднимаются только настоящие ямы, ровно до слива.
    let mut z = vec![10.0; 49];
    z[3 * 7 + 3] = 5.0;
    let f = fill(&z, 7, 7, 0.0);
    assert_eq!(f.raised, 1);
    assert_eq!(f.filled[3 * 7 + 3], 10.0);
    assert!((f.max_raise - 5.0).abs() < 1e-12);
}

#[test]
fn single_pit_with_epsilon_raises_flats_too() {
    // Epsilon больше нуля дополнительно строит уклон на плоском плато.
    let mut z = vec![10.0; 49];
    z[3 * 7 + 3] = 5.0;
    let eps = 0.01;
    let f = fill(&z, 7, 7, eps);
    // Все внутренние ячейки.
    assert_eq!(f.raised, 25);
    assert!(f.filled[3 * 7 + 3] >= 10.0);
    assert!(has_no_interior_pit(&f.filled, 7, 7, None));
    // Подъём плато ограничен eps на шаг до стока, сама яма не в счёт.
    for r in 1..6 {
        for c in 1..6 {
            if (r, c) != (3, 3) {
                assert!(f.filled[r * 7 + c] - z[r * 7 + c] <= 3.0 * eps + 1e-12);
            }
        }
    }
}

#[test]
fn epsilon_gradient_in_filled_lake() {
    // Котловина 3x3 в плато: центр выше края котловины минимум на eps.
    let mut z = vec![20.0; 81];
    for r in 3..6 {
        for c in 3..6 {
            z[r * 9 + c] = 2.0;
        }
    }
    let eps = 0.05;
    let f = fill(&z, 9, 9, eps);
    assert!(f.raised >= 9);
    assert!(has_no_interior_pit(&f.filled, 9, 9, None));
    assert!(f.filled[4 * 9 + 4] >= f.filled[3 * 9 + 3] + eps - 1e-12);
}

#[test]
fn flat_gets_drainage() {
    // Идеальная плоскость: внутренние ячейки поднимаются на eps-уклон.
    let z = vec![100.0; 36];
    let f = fill(&z, 6, 6, 0.001);
    assert!(f.raised > 0);
    assert!(has_no_interior_pit(&f.filled, 6, 6, None));
    // Граница не тронута.
    assert!((0..6).all(|c| f.filled[c] == 100.0));
    assert!((0..6).all(|r| f.filled[r * 6 + 5] == 100.0));
}

#[test]
fn slope_untouched() {
    // Монотонный склон без ям меняться не должен.
    let z: Vec<f64> = (0..64).map(|i| (i / 8) as f64 * 2.0 + (i % 8) as f64 + 50.0).collect();
    let f = fill(&z, 8, 8, 0.001);
    assert_eq!(f.raised, 0);
    assert_eq!(f.max_raise, 0.0);
    assert!(f.filled.iter().zip(&z).all(|(a, b)| (a - b).abs() < 1e-9));
}

#[test]
fn nodata_acts_as_outlet() {
    // Яма примыкает к дыре nodata: вода уходит в дыру.
    let mut z = vec![30.0; 81];
    z[4 * 9 + 4] = 1.0;
    let mut mask = vec![false; 81];
    mask[4 * 9 + 5] = true;
    let f = fill_depressions(&z, 9, 9, Some(&mask), 0.0, None).unwrap();
    // Ячейка ямы соседствует с nodata, значит она сток и не поднимается.
    assert_eq!(f.filled[4 * 9 + 4], 1.0);
    assert_eq!(f.raised, 0);
    // Значение под маской не изменилось.
    assert_eq!(f.filled[4 * 9 + 5], 30.0);
}

#[test]
fn nan_treated_as_nodata() {
    let mut z = vec![10.0; 49];
    z[2 * 7 + 2] = f64::NAN;
    z[4 * 7 + 4] = 3.0;
    let f = fill(&z, 7, 7, 0.01);
    assert!(f.filled[2 * 7 + 2].is_nan());
    assert!(f.filled[4 * 7 + 4] >= 10.0);
}

#[test]
fn nested_pits() {
    // Яма в яме: обе выталкиваются до уровня стока.
    let mut z = vec![50.0; 121];
    for r in 3..8 {
        for c in 3..8 {
            z[r * 11 + c] = 20.0;
        }
    }
    z[5 * 11 + 5] = 5.0;
    let f = fill(&z, 11, 11, 0.01);
    assert!(has_no_interior_pit(&f.filled, 11, 11, None));
    assert!(f.filled[5 * 11 + 5] >= 50.0);
}

#[test]
fn valley_to_edge_not_filled() {
    // Долина с выходом на край: настоящего понижения нет.
    let mut z = vec![40.0; 7 * 9];
    for c in 0..9 {
        z[3 * 9 + c] = 10.0 - 8.0 * c as f64 / 8.0;
    }
    let f = fill(&z, 7, 9, 0.0);
    assert_eq!(f.raised, 0);
    assert!((0..9).all(|c| (f.filled[3 * 9 + c] - z[3 * 9 + c]).abs() < 1e-12));
}

#[test]
fn determinism() {
    let z: Vec<f64> = uniform(7, 30 * 40).iter().map(|v| v * 100.0).collect();
    let a = fill(&z, 30, 40, 0.005);
    let b = fill(&z, 30, 40, 0.005);
    assert_eq!(a.filled, b.filled);
    assert_eq!((a.raised, a.max_raise), (b.raised, b.max_raise));
}

#[test]
fn random_terrain_no_pits_left() {
    let z: Vec<f64> = uniform(123, 1600).iter().map(|v| v * 100.0).collect();
    let f = fill(&z, 40, 40, 0.01);
    assert!(has_no_interior_pit(&f.filled, 40, 40, None));
    // Заполнение никогда не опускает рельеф.
    assert!(f.filled.iter().zip(&z).all(|(a, b)| *a >= b - 1e-12));
}

/// Заполнение по очереди с приоритетом даёт ту же неподвижную точку,
/// что и проходы Planchon-Darboux модуля: у каждой внутренней ячейки
/// высота - наибольшее из своей и наименьшей соседней плюс eps.
#[test]
fn result_is_planchon_darboux_fixed_point() {
    let (rows, cols, eps) = (50, 50, 0.001);
    let z: Vec<f64> = uniform(11, rows * cols).iter().map(|v| v * 100.0).collect();
    let f = fill(&z, rows, cols, eps);
    for r in 1..rows - 1 {
        for c in 1..cols - 1 {
            let mut low = f64::INFINITY;
            for dr in [-1i64, 0, 1] {
                for dc in [-1i64, 0, 1] {
                    if dr != 0 || dc != 0 {
                        low = low.min(f.filled[((r as i64 + dr) as usize) * cols + (c as i64 + dc) as usize]);
                    }
                }
            }
            let i = r * cols + c;
            assert_eq!(f.filled[i], z[i].max(low + eps));
        }
    }
}

#[test]
fn bad_input() {
    assert!(fill_depressions(&[0.0; 5], 1, 5, None, 0.001, None).is_none());
    assert!(fill_depressions(&[0.0; 25], 5, 5, None, -1.0, None).is_none());
    assert!(fill_depressions(&[0.0; 24], 5, 5, None, 0.001, None).is_none());
    assert!(fill_depressions(&[0.0; 25], 5, 5, Some(&[false; 3]), 0.001, None).is_none());
}
