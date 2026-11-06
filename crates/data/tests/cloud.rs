// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Облако COPC частями файла: [`data::cloud::SparseCloud`] даёт те же
//! точки, что чтение файла целиком, с длиной файла и без неё.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use data::cloud::{self, Cloud, SparseCloud};
use terrio::crs::SpatialRef;
use terrio::pointcloud::{self, Dimension, PointWriteOptions, Values};

/// Облако COPC в памяти: n точек в UTM 31N у Гренобля, с цветом
/// и классом.
fn copc(n: usize) -> Vec<u8> {
    let mut x = Vec::with_capacity(n);
    let mut y = Vec::with_capacity(n);
    let mut z = Vec::with_capacity(n);
    let mut class = Vec::with_capacity(n);
    let mut red = Vec::with_capacity(n);
    let mut green = Vec::with_capacity(n);
    let mut blue = Vec::with_capacity(n);
    // Простой генератор: точки плотно и неравномерно по квадрату 1 км.
    let mut s = 12345u64;
    let mut next = || {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (s >> 11) as f64 / (1u64 << 53) as f64
    };
    for _ in 0..n {
        let (a, b) = (next(), next());
        x.push(714_000.0 + 1000.0 * a);
        y.push(5_008_000.0 + 1000.0 * b);
        z.push(210.0 + 30.0 * (a * 7.0).sin() + 5.0 * next());
        class.push(if next() < 0.5 { 2u8 } else { 5 });
        red.push((a * 65535.0) as u16);
        green.push((b * 65535.0) as u16);
        blue.push(30000u16);
    }
    let dims = vec![
        Dimension::new("x", Values::F64(x)),
        Dimension::new("y", Values::F64(y)),
        Dimension::new("z", Values::F64(z)),
        Dimension::new("classification", Values::U8(class)),
        Dimension::new("red", Values::U16(red)),
        Dimension::new("green", Values::U16(green)),
        Dimension::new("blue", Values::U16(blue)),
    ];
    let cloud = pointcloud::from_dimensions(dims, SpatialRef::from_epsg(32631).unwrap());
    pointcloud::write_points_bytes(&cloud, "t.copc.laz", &PointWriteOptions::default()).unwrap()
}

/// Ответ сервера на Range: байты куска, у конца файла - сколько есть.
fn range(file: &[u8], start: u64, len: u64) -> &[u8] {
    let a = (start as usize).min(file.len());
    let b = (start.saturating_add(len) as usize).min(file.len());
    &file[a..b]
}

/// Чтение до конца, ответы - из file. Возвращает облако и количество
/// кругов запросов.
fn drive(sparse: &mut SparseCloud, file: &[u8]) -> (Cloud, usize) {
    let mut rounds = 0;
    loop {
        if let Some(c) = sparse.read().unwrap() {
            return (c, rounds);
        }
        let missing = sparse.missing();
        assert!(!missing.is_empty());
        rounds += 1;
        for (start, len) in missing {
            sparse.insert(start, range(file, start, len));
        }
    }
}

#[test]
fn sparse_copc_reads_like_the_whole_file() {
    let file = copc(300_000);
    let max = 60_000;
    let direct = cloud::read(file.clone(), "t.copc.laz", max as u64).unwrap();
    assert_eq!(direct.total, 300_000);
    assert!(direct.points.len() <= max && direct.points.len() > 1000, "{}", direct.points.len());
    assert_eq!(direct.epsg, Some(32631));
    let p = direct.points[0];
    assert!((45.1..45.3).contains(&p[0]) && (5.6..5.9).contains(&p[1]), "{p:?}");

    for len in [Some(file.len() as u64), None] {
        let mut sparse = SparseCloud::new("t.copc.laz", len, max);
        let (got, rounds) = drive(&mut sparse, &file);
        assert_eq!(got, direct, "длина {len:?}");
        assert!(rounds <= 8, "{rounds} кругов");
        // Прочитаны верхние уровни, не весь файл.
        let (bytes, _) = sparse.fetched();
        eprintln!("длина {len:?}: {} точек за {rounds} кругов, {bytes} байт из {}", got.points.len(), file.len());
        assert!(bytes < file.len() as u64, "{bytes} из {}", file.len());
    }
}

#[test]
fn sparse_copc_takes_the_whole_file_from_a_server_without_range() {
    let file = copc(20_000);
    let direct = cloud::read(file.clone(), "t.copc.laz", 1_000_000).unwrap();
    let mut sparse = SparseCloud::new("t.copc.laz", None, 1_000_000);
    let mut rounds = 0;
    let got = loop {
        if let Some(c) = sparse.read().unwrap() {
            break c;
        }
        rounds += 1;
        assert!(rounds < 10);
        for (start, _) in sparse.missing() {
            sparse.insert(start, &file);
        }
    };
    assert_eq!(got, direct);
}

/// Испорченный файл - ошибка или облако, но не падение: случайные байты
/// в заголовке, записях, иерархии и сжатых точках, обрезка. Только
/// в сборке release: у крейта `laz` в декодере есть `debug_assert`,
/// в программе он не собирается.
#[test]
#[cfg_attr(debug_assertions, ignore)]
fn damaged_files_do_not_panic() {
    let good = copc(3000);
    let mut s = 987654321u64;
    let mut next = |n: usize| {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((s >> 33) as usize) % n.max(1)
    };
    let mut errors = 0;
    for round in 0..3000 {
        let mut bad = good.clone();
        if round % 10 == 0 {
            bad.truncate(next(good.len()));
        } else {
            for _ in 0..1 + next(4) {
                // Чаще - начало файла: заголовок и записи VLR.
                let at = if round % 2 == 0 { next(1200.min(bad.len())) } else { next(bad.len()) };
                bad[at] = (next(256)) as u8;
            }
        }
        let name = if round % 3 == 0 { "x.laz" } else { "x.copc.laz" };
        errors += usize::from(cloud::read(bad.clone(), name, 1000).is_err());
        let mut sparse = SparseCloud::new(name, if round % 2 == 0 { Some(bad.len() as u64) } else { None }, 1000);
        while let Ok(None) = sparse.read() {
            let missing = sparse.missing();
            if missing.is_empty() {
                break;
            }
            for (start, len) in missing {
                sparse.insert(start, range(&bad, start, len));
            }
        }
    }
    eprintln!("ошибок {errors} из 3000");
}

#[test]
fn sparse_hostile_bytes_are_an_error() {
    // Не LAS.
    let junk = vec![7u8; 100_000];
    let mut sparse = SparseCloud::new("x.copc.laz", Some(junk.len() as u64), 1000);
    let mut result = sparse.read();
    while let Ok(None) = result {
        for (start, len) in sparse.missing() {
            sparse.insert(start, range(&junk, start, len));
        }
        result = sparse.read();
    }
    assert!(result.is_err());
    // LAZ, но не COPC: частями не читается.
    let mut laz = copc(1000);
    laz[377..381].copy_from_slice(b"xxxx");
    let mut sparse = SparseCloud::new("x.laz", Some(laz.len() as u64), 1000);
    let mut result = sparse.read();
    let mut n = 0;
    while let Ok(None) = result {
        n += 1;
        assert!(n < 70);
        for (start, len) in sparse.missing() {
            sparse.insert(start, range(&laz, start, len));
        }
        result = sparse.read();
    }
    assert!(result.is_err());
    // Сервер всё время отвечает пусто: ошибка после предела кругов.
    let mut sparse = SparseCloud::new("x.copc.laz", None, 1000);
    let mut result = sparse.read();
    while let Ok(None) = result {
        for (start, _) in sparse.missing() {
            sparse.insert(start, &[]);
        }
        result = sparse.read();
    }
    assert!(result.is_err());
}
