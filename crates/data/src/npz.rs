// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Массивы NumPy: файл .npy и архив .npz, чтение и запись.
//!
//! Модуль пишет файлы зон Slab2 и файл томографии профиля через
//! `np.savez`. Здесь их разбор для [`crate::slabs`] и
//! [`crate::tomography`]. Файл считается враждебным: размер массива
//! сверяется с заголовком, распаковка члена архива ограничена
//! [`MEMBER_LIMIT`]. Порядок Fortran не читается, модуль его не пишет.

use std::io::{Cursor, Read, Write};

/// Наибольший размер распакованного члена архива .npz. Томография
/// SEISGLOB2 в float16 - около 7 МБ.
pub const MEMBER_LIMIT: u64 = 256 << 20;

/// Массив: размеры по осям и значения по строкам (порядок C) в f64.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Array {
    pub shape: Vec<usize>,
    pub values: Vec<f64>,
}

/// Тип значения массива.
#[derive(Debug, Clone, Copy)]
enum Kind {
    Float,
    Signed,
    Unsigned,
    Bool,
}

/// Строка в кавычках после ключа key заголовка.
fn quoted<'a>(header: &'a str, key: &str) -> Option<&'a str> {
    let rest = &header[header.find(key)? + key.len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let quote = rest.chars().next().filter(|c| *c == '\'' || *c == '"')?;
    let rest = &rest[1..];
    Some(&rest[..rest.find(quote)?])
}

/// Размеры массива из ключа 'shape' заголовка.
fn shape_of(header: &str) -> Option<Vec<usize>> {
    let key = "'shape'";
    let rest = &header[header.find(key)? + key.len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start().strip_prefix('(')?;
    let inner = &rest[..rest.find(')')?];
    inner.split(',').map(str::trim).filter(|s| !s.is_empty()).map(|s| s.parse::<usize>().ok()).collect()
}

/// Число половинной точности в f32.
pub fn half_to_f32(h: u16) -> f32 {
    let sign = u32::from(h >> 15) << 31;
    let exp = u32::from((h >> 10) & 0x1f);
    let frac = u32::from(h & 0x3ff);
    if exp == 0 {
        // Ноль и денормализованные: frac · 2^-24, точно в f32.
        let mag = frac as f32 * f32::from_bits(0x3380_0000);
        return if sign != 0 { -mag } else { mag };
    }
    if exp == 0x1f {
        return f32::from_bits(sign | 0x7f80_0000 | (frac << 13));
    }
    f32::from_bits(sign | ((exp + 112) << 23) | (frac << 13))
}

/// f32 в число половинной точности, округление к ближайшему чётному, как
/// `astype(np.float16)`. Больше 65504 - бесконечность.
pub fn f32_to_half(x: f32) -> u16 {
    let bits = x.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let man = bits & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    let round = |value: u32, rest: u32, half: u32| -> u32 {
        if rest > half || (rest == half && value & 1 == 1) { value + 1 } else { value }
    };
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = man | 0x80_0000;
        let shift = (14 - e) as u32;
        let value = round(m >> shift, m & ((1 << shift) - 1), 1 << (shift - 1));
        return sign | value as u16;
    }
    let value = round(((e as u32) << 10) | (man >> 13), man & 0x1fff, 0x1000);
    sign | value as u16
}

/// Массив из байтов файла .npy.
pub fn read_npy(data: &[u8]) -> Result<Array, String> {
    if !data.starts_with(b"\x93NUMPY") || data.len() < 10 {
        return Err("not a .npy file".into());
    }
    let (len, start): (usize, usize) = if data[6] == 1 {
        (usize::from(u16::from_le_bytes([data[8], data[9]])), 10)
    } else {
        let b = data.get(8..12).ok_or("short .npy header")?;
        (u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize, 12)
    };
    let end = start.checked_add(len).ok_or("bad .npy header")?;
    let header = std::str::from_utf8(data.get(start..end).ok_or("short .npy header")?).map_err(|e| e.to_string())?;
    if header.contains("'fortran_order': True") {
        return Err("unsupported .npy layout".into());
    }
    let descr = quoted(header, "'descr'").ok_or("no .npy type")?;
    let shape = shape_of(header).ok_or("no .npy shape")?;
    let mut chars = descr.chars();
    let big = match chars.next() {
        Some('<' | '|' | '=') => false,
        Some('>') => true,
        _ => return Err(format!("unsupported .npy type {descr}")),
    };
    let kind = match chars.next() {
        Some('f') => Kind::Float,
        Some('i') => Kind::Signed,
        Some('u') => Kind::Unsigned,
        Some('b') => Kind::Bool,
        _ => return Err(format!("unsupported .npy type {descr}")),
    };
    let width: usize = chars.as_str().parse().map_err(|_| format!("unsupported .npy type {descr}"))?;
    let supported = matches!(
        (kind, width),
        (Kind::Float, 2 | 4 | 8) | (Kind::Signed | Kind::Unsigned, 1 | 2 | 4 | 8) | (Kind::Bool, 1)
    );
    if !supported {
        return Err(format!("unsupported .npy type {descr}"));
    }
    let count = shape.iter().try_fold(1usize, |n, s| n.checked_mul(*s)).ok_or("bad .npy shape")?;
    let size = count.checked_mul(width).ok_or("bad .npy shape")?;
    let body = data.get(end..).unwrap_or_default();
    if body.len() < size {
        return Err("short .npy data".into());
    }
    let values = body[..size]
        .chunks_exact(width)
        .map(|chunk| {
            let mut b = [0u8; 8];
            if big {
                for (k, v) in chunk.iter().rev().enumerate() {
                    b[k] = *v;
                }
            } else {
                b[..width].copy_from_slice(chunk);
            }
            match (kind, width) {
                (Kind::Float, 2) => f64::from(half_to_f32(u16::from_le_bytes([b[0], b[1]]))),
                (Kind::Float, 4) => f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                (Kind::Float, _) => f64::from_le_bytes(b),
                (Kind::Signed, 1) => f64::from(b[0] as i8),
                (Kind::Signed, 2) => f64::from(i16::from_le_bytes([b[0], b[1]])),
                (Kind::Signed, 4) => f64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                (Kind::Signed, _) => i64::from_le_bytes(b) as f64,
                (Kind::Unsigned | Kind::Bool, 1) => f64::from(b[0]),
                (Kind::Unsigned, 2) => f64::from(u16::from_le_bytes([b[0], b[1]])),
                (Kind::Unsigned, 4) => f64::from(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                (Kind::Unsigned | Kind::Bool, _) => u64::from_le_bytes(b) as f64,
            }
        })
        .collect();
    Ok(Array { shape, values })
}

/// Массивы names из байтов архива .npz, члены `<имя>.npy`.
pub fn read_npz(data: &[u8], names: &[&str]) -> Result<Vec<Array>, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(data)).map_err(|e| e.to_string())?;
    names
        .iter()
        .map(|name| {
            let member = archive.by_name(&format!("{name}.npy")).map_err(|e| format!("{name}: {e}"))?;
            let mut bytes = Vec::new();
            member.take(MEMBER_LIMIT).read_to_end(&mut bytes).map_err(|e| format!("{name}: {e}"))?;
            read_npy(&bytes).map_err(|e| format!("{name}: {e}"))
        })
        .collect()
}

/// Байты файла .npy версии 1.0: тип descr, размеры shape, тело body.
fn npy(descr: &str, shape: &[usize], body: &[u8]) -> Vec<u8> {
    let dims: Vec<String> = shape.iter().map(usize::to_string).collect();
    let dims = if dims.len() == 1 { format!("{},", dims[0]) } else { dims.join(", ") };
    let mut header = format!("{{'descr': '{descr}', 'fortran_order': False, 'shape': ({dims}), }}");
    // Заголовок с переводом строки дополняется пробелами до кратного 64,
    // как у NumPy.
    while (10 + header.len() + 1) % 64 != 0 {
        header.push(' ');
    }
    header.push('\n');
    let mut out = b"\x93NUMPY\x01\x00".to_vec();
    out.extend_from_slice(&(header.len() as u16).to_le_bytes());
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(body);
    out
}

/// Файл .npy чисел f64.
pub fn npy_f64(shape: &[usize], values: &[f64]) -> Vec<u8> {
    let body: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    npy("<f8", shape, &body)
}

/// Файл .npy чисел половинной точности из f32.
pub fn npy_f16(shape: &[usize], values: &[f32]) -> Vec<u8> {
    let body: Vec<u8> = values.iter().flat_map(|v| f32_to_half(*v).to_le_bytes()).collect();
    npy("<f2", shape, &body)
}

/// Архив .npz со сжатием deflate, как `np.savez_compressed`: члены
/// `<имя>.npy`.
pub fn write_npz(members: &[(&str, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in members {
        writer.start_file(format!("{name}.npy"), options).map_err(|e| e.to_string())?;
        writer.write_all(bytes).map_err(|e| e.to_string())?;
    }
    writer.finish().map(Cursor::into_inner).map_err(|e| e.to_string())
}
