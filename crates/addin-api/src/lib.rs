// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! API аддинов PlanetX Lite, слой L2. Прототип фазы 3, версия ABI
//! [`ABI_VERSION`].
//!
//! Аддин - модуль WebAssembly (`wasm32-unknown-unknown`) без импортов:
//! ни файлов, ни сети, ни часов у него нет. Lite даёт ему байты
//! и забирает результат. Экспорты аддина:
//!
//! - `memory` - память модуля;
//! - `pxa_abi() -> i32` - версия ABI, сейчас [`ABI_VERSION`];
//! - `pxa_manifest() -> i32` - описание аддина (JSON: name, version,
//!   author, license, extensions) кладётся в выход, ответ - его длина;
//! - `pxa_input(len: i32) -> i32` - место под вход длиной len байт,
//!   ответ - адрес в памяти модуля, 0 - места нет;
//! - `pxa_read(len: i32) -> i32` - разобрать вход, результат - в выход,
//!   ответ - длина результата, меньше нуля - ошибка, текст ошибки
//!   в выходе;
//! - `pxa_output() -> i32` - адрес выхода в памяти модуля.
//!
//! Результат чтения - точки в формате [`encode_points`]. Данные вне
//! аддина считаются враждебными: [`decode_points`] не верит ни длинам,
//! ни количеству.

/// Версия ABI.
pub const ABI_VERSION: i32 = 1;
/// Метка начала результата с точками.
pub const POINTS_MAGIC: &[u8; 4] = b"PXP1";
/// Наибольшая длина названия точки, байт.
pub const MAX_NAME: usize = 1024;

/// Точка результата: широта, долгота, высота в метрах, название.
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub lat: f64,
    pub lon: f64,
    pub alt: f64,
    pub name: String,
}

/// Точки в байты: метка, количество u32, затем у каждой точки lat, lon,
/// alt f64 и название - длина u16 и UTF-8. Числа - little-endian.
/// Название длиннее [`MAX_NAME`] обрезается по границе символа.
pub fn encode_points(points: &[Point]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + points.len() * 32);
    out.extend_from_slice(POINTS_MAGIC);
    out.extend_from_slice(&(points.len() as u32).to_le_bytes());
    for p in points {
        for v in [p.lat, p.lon, p.alt] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        let mut end = p.name.len().min(MAX_NAME);
        while !p.name.is_char_boundary(end) {
            end -= 1;
        }
        let name = p.name.get(..end).unwrap_or("");
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
    }
    out
}

/// Точки из байтов [`encode_points`]. Ошибка - текстом.
pub fn decode_points(bytes: &[u8]) -> Result<Vec<Point>, String> {
    let mut at = 0usize;
    let mut take = |n: usize| -> Result<&[u8], String> {
        let end = at.checked_add(n).filter(|e| *e <= bytes.len()).ok_or("результат аддина обрезан")?;
        let s = bytes.get(at..end).ok_or("результат аддина обрезан")?;
        at = end;
        Ok(s)
    };
    if take(4)? != POINTS_MAGIC {
        return Err("результат аддина не точки".into());
    }
    let count = u32::from_le_bytes(take(4)?.try_into().map_err(|_| "обрезан")?) as usize;
    // Точка занимает не меньше 26 байт: количеству больше остатка не верим.
    if count > bytes.len() / 26 {
        return Err("количество точек больше результата".into());
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let mut f = [0.0f64; 3];
        for v in &mut f {
            *v = f64::from_le_bytes(take(8)?.try_into().map_err(|_| "обрезан")?);
        }
        let len = u16::from_le_bytes(take(2)?.try_into().map_err(|_| "обрезан")?) as usize;
        if len > MAX_NAME {
            return Err("название точки длиннее предела".into());
        }
        let name = String::from_utf8_lossy(take(len)?).into_owned();
        out.push(Point { lat: f[0], lon: f[1], alt: f[2], name });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{MAX_NAME, Point, decode_points, encode_points};

    #[test]
    fn points_round_trip_and_hostile_bytes() {
        let pts = vec![
            Point { lat: 58.0, lon: 56.2, alt: 120.5, name: "Пермь".into() },
            Point { lat: -1.0, lon: 2.0, alt: 0.0, name: "ё".repeat(MAX_NAME) },
        ];
        let bytes = encode_points(&pts);
        let back = decode_points(&bytes).unwrap();
        assert_eq!(back[0], pts[0]);
        assert!(back[1].name.len() <= MAX_NAME && back[1].name.chars().all(|c| c == 'ё'));
        // Каждое начало и каждый испорченный байт - без паники.
        for n in 0..bytes.len() {
            let _ = decode_points(&bytes[..n]);
            let mut bad = bytes.clone();
            bad[n] ^= 0xFF;
            let _ = decode_points(&bad);
        }
        let mut huge = b"PXP1".to_vec();
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode_points(&huge).is_err());
    }
}
