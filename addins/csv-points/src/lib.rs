// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Аддин-прототип PlanetX Lite: точки из CSV.
//!
//! Первая строка - заголовок. Столбцы ищутся по названию без учёта
//! регистра: широта - lat, latitude, широта; долгота - lon, lng, long,
//! longitude, долгота; высота - alt, z, height, ele, elevation, высота;
//! название - name, title, название. Разделитель - запятая, точка
//! с запятой или табуляция, по заголовку. При точке с запятой дробная
//! часть может идти через запятую. Строка без широты и долготы
//! пропускается.
//!
//! Единственное место `unsafe` - атрибут `#[unsafe(no_mangle)]`
//! у экспортов: без него функцию не экспортировать из модуля
//! WebAssembly. Сам код аддина - безопасный Rust.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::todo)]

use std::sync::{Mutex, MutexGuard};

use addin_api::{ABI_VERSION, Point, encode_points};

/// Наибольший вход, байт.
const MAX_INPUT: usize = 64 << 20;

static INPUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());
static OUTPUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());

fn lock(m: &Mutex<Vec<u8>>) -> MutexGuard<'_, Vec<u8>> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Положить в выход, ответ - длина.
fn put(bytes: Vec<u8>) -> i32 {
    let len = i32::try_from(bytes.len()).unwrap_or(i32::MAX);
    *lock(&OUTPUT) = bytes;
    len
}

#[unsafe(no_mangle)]
pub extern "C" fn pxa_abi() -> i32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn pxa_manifest() -> i32 {
    put(br#"{"name":"csv-points","version":"0.1.0","author":"OOO Inform++","license":"Apache-2.0","extensions":["csv"]}"#.to_vec())
}

#[unsafe(no_mangle)]
pub extern "C" fn pxa_input(len: i32) -> i32 {
    let Ok(n) = usize::try_from(len) else {
        return 0;
    };
    if n > MAX_INPUT {
        return 0;
    }
    let mut input = lock(&INPUT);
    *input = vec![0; n.max(1)];
    i32::try_from(input.as_ptr() as usize).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn pxa_output() -> i32 {
    i32::try_from(lock(&OUTPUT).as_ptr() as usize).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn pxa_read(len: i32) -> i32 {
    let input = lock(&INPUT);
    let n = usize::try_from(len).unwrap_or(0).min(input.len());
    let text = String::from_utf8_lossy(input.get(..n).unwrap_or(&[])).into_owned();
    drop(input);
    match parse(&text) {
        Ok(points) => put(encode_points(&points)),
        Err(e) => -put(e.into_bytes()).max(1),
    }
}

fn column(header: &[String], names: &[&str]) -> Option<usize> {
    header.iter().position(|h| names.contains(&h.as_str()))
}

fn number(text: &str, comma_decimal: bool) -> Option<f64> {
    let t = text.trim().trim_matches('"');
    let v = if comma_decimal { t.replace(',', ".").parse::<f64>().ok()? } else { t.parse::<f64>().ok()? };
    v.is_finite().then_some(v)
}

/// Точки из текста CSV.
fn parse(text: &str) -> Result<Vec<Point>, String> {
    // Метка BOM в начале файла - не часть первого столбца.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let head = lines.next().ok_or("пустой файл")?;
    let sep = [';', '\t', ','].into_iter().find(|c| head.contains(*c)).unwrap_or(',');
    let header: Vec<String> = head.split(sep).map(|h| h.trim().trim_matches('"').to_lowercase()).collect();
    let lat = column(&header, &["lat", "latitude", "широта"]).ok_or("нет столбца широты")?;
    let lon = column(&header, &["lon", "lng", "long", "longitude", "долгота"]).ok_or("нет столбца долготы")?;
    let alt = column(&header, &["alt", "z", "height", "ele", "elevation", "высота"]);
    let name = column(&header, &["name", "title", "название"]);
    let comma_decimal = sep == ';';
    let mut out = Vec::new();
    for line in lines {
        let cells: Vec<&str> = line.split(sep).collect();
        let get = |i: usize| cells.get(i).copied().unwrap_or("");
        let (Some(la), Some(lo)) = (number(get(lat), comma_decimal), number(get(lon), comma_decimal)) else {
            continue;
        };
        if !(-90.0..=90.0).contains(&la) || !(-180.0..=360.0).contains(&lo) {
            continue;
        }
        out.push(Point {
            lat: la,
            lon: lo,
            alt: alt.and_then(|i| number(get(i), comma_decimal)).unwrap_or(0.0),
            name: name.map_or_else(String::new, |i| get(i).trim().trim_matches('"').to_string()),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::parse;

    #[test]
    fn csv_with_header_and_separators() {
        let p = parse("name,lat,lon,alt\nПермь,58.01,56.23,170\nплохая,abc,1,2\n").unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!((p[0].name.as_str(), p[0].lat, p[0].alt), ("Пермь", 58.01, 170.0));
        let p = parse("Широта;Долгота;Название\n58,01;56,23;Пермь\n").unwrap();
        assert_eq!((p[0].lat, p[0].lon), (58.01, 56.23));
        assert!(parse("x,y\n1,2").is_err());
        // Файл с меткой BOM: название находится.
        let p = parse("\u{feff}name,lat,lon\nА,1,2\n").unwrap();
        assert_eq!(p[0].name, "А");
    }
}
