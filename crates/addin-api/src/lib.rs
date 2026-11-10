// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! API аддинов PlanetX Lite, слой L2. Версия ABI [`ABI_VERSION`],
//! Lite принимает версии от [`ABI_MIN`].
//!
//! Аддин - модуль WebAssembly (`wasm32-unknown-unknown`) без импортов:
//! ни файлов, ни сети, ни часов у него нет. Lite даёт ему байты
//! и забирает результат. Экспорты аддина:
//!
//! - `memory` - память модуля;
//! - `pxa_abi() -> i32` - версия ABI, 1 или 2;
//! - `pxa_manifest() -> i32` - описание аддина (JSON, поля - в описании
//!   хоста `addin_host::Manifest`) кладётся в выход, ответ - его длина;
//! - `pxa_input(len: i32) -> i32` - место под вход длиной len байт,
//!   ответ - адрес в памяти модуля, 0 - места нет;
//! - `pxa_read(len: i32) -> i32` - аддин данных: разобрать вход (байты
//!   файла), результат - точки [`encode_points`] в выход, ответ - длина
//!   результата, меньше нуля - ошибка, текст ошибки в выходе;
//! - `pxa_command(len: i32) -> i32` - аддин команды, с версии 2: вход -
//!   запрос [`encode_request`], результат - объекты [`encode_features`],
//!   ответ - как у `pxa_read`;
//! - `pxa_output() -> i32` - адрес выхода в памяти модуля.
//!
//! У аддина есть `pxa_read`, `pxa_command` или оба. Данные вне аддина
//! считаются враждебными: разборщики не верят ни длинам, ни количеству.

/// Версия ABI: 2 - команды над объектами.
pub const ABI_VERSION: i32 = 2;
/// Самая ранняя версия ABI, которую Lite принимает: 1 - только чтение
/// точек.
pub const ABI_MIN: i32 = 1;
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

/// Метка начала объектов.
pub const FEATURES_MAGIC: &[u8; 4] = b"PXG1";
/// Метка начала запроса команды.
pub const REQUEST_MAGIC: &[u8; 4] = b"PXQ1";
/// Наибольшая длина имени команды, байт.
pub const MAX_COMMAND: usize = 256;
/// Наибольшая длина параметров команды (JSON), байт.
pub const MAX_PARAMS: usize = 64 << 10;

/// Вид геометрии объекта.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Точки: каждая часть - одна вершина.
    Point,
    /// Линии: каждая часть - ломаная.
    Line,
    /// Полигон: первая часть - внешнее кольцо, остальные - дыры.
    /// Кольцо не повторяет первую вершину в конце.
    Polygon,
}

impl Kind {
    fn code(self) -> u8 {
        match self {
            Kind::Point => 1,
            Kind::Line => 2,
            Kind::Polygon => 3,
        }
    }

    fn from_code(c: u8) -> Option<Kind> {
        match c {
            1 => Some(Kind::Point),
            2 => Some(Kind::Line),
            3 => Some(Kind::Polygon),
            _ => None,
        }
    }
}

/// Объект: геометрия в широте, долготе (градусы WGS84) и высоте
/// (метры) и название.
#[derive(Debug, Clone, PartialEq)]
pub struct Feature {
    pub kind: Kind,
    /// Части, вершина - [широта, долгота, высота].
    pub parts: Vec<Vec<[f64; 3]>>,
    pub name: String,
}

/// Разборщик байтов, не верящий длинам.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.bytes.len()).ok_or("данные аддина обрезаны")?;
        let s = self.bytes.get(self.at..end).ok_or("данные аддина обрезаны")?;
        self.at = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(*self.take(1)?.first().ok_or("обрезан")?)
    }

    fn u16(&mut self) -> Result<usize, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().map_err(|_| "обрезан")?) as usize)
    }

    fn u32(&mut self) -> Result<usize, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| "обрезан")?) as usize)
    }

    fn f64(&mut self) -> Result<f64, String> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into().map_err(|_| "обрезан")?))
    }

    fn left(&self) -> usize {
        self.bytes.len().saturating_sub(self.at)
    }

    /// Строка длиной u16 или u32 не длиннее max.
    fn text(&mut self, long: bool, max: usize) -> Result<String, String> {
        let len = if long { self.u32()? } else { self.u16()? };
        if len > max {
            return Err("строка длиннее предела".into());
        }
        Ok(String::from_utf8_lossy(self.take(len)?).into_owned())
    }
}

/// Строка, обрезанная до max байт по границе символа.
fn cut(s: &str, max: usize) -> &str {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or("")
}

fn put_features(out: &mut Vec<u8>, features: &[Feature]) {
    out.extend_from_slice(FEATURES_MAGIC);
    out.extend_from_slice(&(features.len() as u32).to_le_bytes());
    for f in features {
        out.push(f.kind.code());
        let name = cut(&f.name, MAX_NAME);
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(f.parts.len() as u32).to_le_bytes());
        for part in &f.parts {
            out.extend_from_slice(&(part.len() as u32).to_le_bytes());
            for v in part {
                for x in v {
                    out.extend_from_slice(&x.to_le_bytes());
                }
            }
        }
    }
}

/// Объекты в байты: метка, количество u32, у объекта - вид u8, название
/// (длина u16 и UTF-8), количество частей u32, у части - количество
/// вершин u32 и вершины по три f64. Числа - little-endian.
pub fn encode_features(features: &[Feature]) -> Vec<u8> {
    let mut out = Vec::new();
    put_features(&mut out, features);
    out
}

fn read_features(r: &mut Reader<'_>) -> Result<Vec<Feature>, String> {
    if r.take(4)? != FEATURES_MAGIC {
        return Err("данные аддина не объекты".into());
    }
    let count = r.u32()?;
    // Объект занимает не меньше 7 байт, часть - 4, вершина - 24:
    // количеству больше остатка не верим.
    if count > r.left() / 7 {
        return Err("количество объектов больше данных".into());
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let kind = Kind::from_code(r.u8()?).ok_or("неизвестный вид геометрии")?;
        let name = r.text(false, MAX_NAME)?;
        let nparts = r.u32()?;
        if nparts > r.left() / 4 {
            return Err("количество частей больше данных".into());
        }
        let mut parts = Vec::with_capacity(nparts);
        for _ in 0..nparts {
            let n = r.u32()?;
            if n > r.left() / 24 {
                return Err("количество вершин больше данных".into());
            }
            let mut part = Vec::with_capacity(n);
            for _ in 0..n {
                let v = [r.f64()?, r.f64()?, r.f64()?];
                if !v.iter().all(|x| x.is_finite()) || !(-90.0..=90.0).contains(&v[0]) {
                    return Err("вершина вне широты или не число".into());
                }
                part.push(v);
            }
            parts.push(part);
        }
        out.push(Feature { kind, parts, name });
    }
    Ok(out)
}

/// Объекты из байтов [`encode_features`]. Ошибка - текстом.
pub fn decode_features(bytes: &[u8]) -> Result<Vec<Feature>, String> {
    let mut r = Reader { bytes, at: 0 };
    let out = read_features(&mut r)?;
    if r.left() != 0 {
        return Err("лишние байты после объектов".into());
    }
    Ok(out)
}

/// Запрос команды: метка, имя команды (длина u16 и UTF-8), параметры
/// JSON (длина u32 и UTF-8), объекты [`encode_features`].
pub fn encode_request(command: &str, params: &str, features: &[Feature]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(REQUEST_MAGIC);
    let c = cut(command, MAX_COMMAND);
    out.extend_from_slice(&(c.len() as u16).to_le_bytes());
    out.extend_from_slice(c.as_bytes());
    let p = cut(params, MAX_PARAMS);
    out.extend_from_slice(&(p.len() as u32).to_le_bytes());
    out.extend_from_slice(p.as_bytes());
    put_features(&mut out, features);
    out
}

/// Запрос из байтов [`encode_request`]: имя команды, параметры JSON,
/// объекты.
pub fn decode_request(bytes: &[u8]) -> Result<(String, String, Vec<Feature>), String> {
    let mut r = Reader { bytes, at: 0 };
    if r.take(4)? != REQUEST_MAGIC {
        return Err("вход не запрос команды".into());
    }
    let command = r.text(false, MAX_COMMAND)?;
    let params = r.text(true, MAX_PARAMS)?;
    let features = read_features(&mut r)?;
    if r.left() != 0 {
        return Err("лишние байты после запроса".into());
    }
    Ok((command, params, features))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{Feature, Kind, MAX_NAME, Point, decode_features, decode_points, decode_request, encode_features, encode_points, encode_request};

    #[test]
    fn features_and_request_round_trip_and_hostile_bytes() {
        let fs = vec![
            Feature { kind: Kind::Point, parts: vec![vec![[58.0, 56.2, 120.0]]], name: "Пермь".into() },
            Feature { kind: Kind::Line, parts: vec![vec![[1.0, 2.0, 0.0], [1.5, 2.5, 0.0]]], name: String::new() },
            Feature { kind: Kind::Polygon, parts: vec![vec![[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]]], name: "ё".repeat(MAX_NAME) },
        ];
        let bytes = encode_features(&fs);
        let back = decode_features(&bytes).unwrap();
        assert_eq!(back[..2], fs[..2]);
        assert!(back[2].name.len() <= MAX_NAME);
        let req = encode_request("buffer", r#"{"distance":100}"#, &fs[..2]);
        let (c, p, f) = decode_request(&req).unwrap();
        assert_eq!((c.as_str(), p.as_str(), f.as_slice()), ("buffer", r#"{"distance":100}"#, &fs[..2]));
        // Каждое начало и каждый испорченный байт - без паники.
        for n in 0..req.len() {
            let _ = decode_request(&req[..n]);
            let mut bad = req.clone();
            bad[n] ^= 0xFF;
            let _ = decode_request(&bad);
        }
        let mut huge = b"PXG1".to_vec();
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode_features(&huge).is_err());
        // Широта вне круга и бесконечность отклоняются.
        let bad = encode_features(&[Feature { kind: Kind::Point, parts: vec![vec![[91.0, 0.0, 0.0]]], name: String::new() }]);
        assert!(decode_features(&bad).is_err());
        let bad = encode_features(&[Feature { kind: Kind::Point, parts: vec![vec![[0.0, f64::NAN, 0.0]]], name: String::new() }]);
        assert!(decode_features(&bad).is_err());
    }

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
