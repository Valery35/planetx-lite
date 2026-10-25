// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Созвездия, имена звёзд и светила для вида неба, перенос
//! `core/skydata.py` модуля.
//!
//! Файл data/constellations.json модуля собран из данных d3-celestial
//! (BSD 3-Clause). Lite берёт его из хранилища модуля, как каталог звёзд.
//! Положения Солнца, Луны и планет считает [`crate::ephemeris`]. Всё
//! в экваториальной системе J2000.

use crate::ephemeris;

pub const URL: &str = "https://raw.githubusercontent.com/Valery35/planetx/main/planetx/data/constellations.json";
pub const ATTRIBUTION: (&str, &str) = ("Constellations: d3-celestial, Olaf Frohn", "https://github.com/ofrohn/d3-celestial");

/// Светила: цвет RGB и размер точки в логических пикселях. Размер
/// условный, он не следует блеску. Подобраны помощником модуля.
pub const BODY_STYLE: [(&str, [f32; 3], f32); 9] = [
    ("sun", [1.0, 0.93, 0.62], 16.0),
    ("moon", [0.92, 0.92, 0.88], 14.0),
    ("mercury", [0.82, 0.76, 0.70], 5.0),
    ("venus", [1.0, 0.98, 0.88], 8.0),
    ("mars", [1.0, 0.52, 0.32], 6.5),
    ("jupiter", [1.0, 0.93, 0.80], 7.5),
    ("saturn", [0.96, 0.86, 0.60], 6.5),
    ("uranus", [0.62, 0.90, 0.95], 4.5),
    ("neptune", [0.50, 0.62, 1.0], 4.5),
];

/// Подпись неба: прямое восхождение и склонение в градусах, имена
/// по-русски и по-английски, ранг (у звёзд - видимая величина).
#[derive(Debug, Clone, PartialEq)]
pub struct Named {
    pub ra: f64,
    pub dec: f64,
    pub ru: String,
    pub en: String,
    pub rank: f64,
}

/// Созвездия: линии фигур (RA, Dec в градусах), названия созвездий
/// и имена звёзд.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Constellations {
    pub lines: Vec<Vec<(f64, f64)>>,
    pub names: Vec<Named>,
    pub stars: Vec<Named>,
}

/// Светило на момент: ключ, направление, цвет, размер.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Body {
    pub key: &'static str,
    pub direction: [f64; 3],
    pub color: [f32; 3],
    pub size: f32,
}

fn named(list: Option<&serde_json::Value>) -> Vec<Named> {
    let number = |v: Option<&serde_json::Value>| v.and_then(serde_json::Value::as_f64).filter(|x| x.is_finite());
    let text = |v: Option<&serde_json::Value>| v.and_then(serde_json::Value::as_str).unwrap_or("").to_string();
    list.and_then(serde_json::Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|e| {
                    let rank = number(e.get("rank")).or_else(|| number(e.get("mag"))).unwrap_or(0.0);
                    Some(Named { ra: number(e.get("ra"))?, dec: number(e.get("dec"))?, ru: text(e.get("ru")), en: text(e.get("en")), rank })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Созвездия из байтов constellations.json.
pub fn parse(data: &[u8]) -> Result<Constellations, String> {
    let value: serde_json::Value = serde_json::from_slice(data).map_err(|e| e.to_string())?;
    let lines = value
        .get("lines")
        .and_then(serde_json::Value::as_array)
        .ok_or("no lines")?
        .iter()
        .filter_map(|line| {
            let points: Vec<(f64, f64)> = line.as_array()?.iter().filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?))).collect();
            (points.len() >= 2).then_some(points)
        })
        .collect();
    Ok(Constellations { lines, names: named(value.get("names")), stars: named(value.get("stars")) })
}

/// Единичный вектор J2000 по RA и Dec в градусах.
pub fn direction(ra: f64, dec: f64) -> [f64; 3] {
    let (r, d) = (ra.to_radians(), dec.to_radians());
    [d.cos() * r.cos(), d.cos() * r.sin(), d.sin()]
}

/// Отрезки фигур созвездий парами направлений.
pub fn segments(data: &Constellations) -> Vec<[f64; 3]> {
    let mut out = Vec::new();
    for line in &data.lines {
        for pair in line.windows(2) {
            out.push(direction(pair[0].0, pair[0].1));
            out.push(direction(pair[1].0, pair[1].1));
        }
    }
    out
}

/// Светила на момент unix_time.
pub fn bodies(unix_time: f64) -> Vec<Body> {
    BODY_STYLE
        .iter()
        .filter_map(|(key, color, size)| Some(Body { key, direction: ephemeris::direction(key, unix_time)?, color: *color, size: *size }))
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// Созвездия из рабочей копии модуля или None, если её нет.
    fn load() -> Option<Constellations> {
        let bytes = std::fs::read(r"C:\Dev\planetx\planetx\data\constellations.json").ok()?;
        Some(parse(&bytes).unwrap())
    }

    #[test]
    fn all_constellations_named() {
        let Some(data) = load() else {
            eprintln!("файла модуля нет, проверка пропущена");
            return;
        };
        // Змея из двух частей.
        assert_eq!(data.names.len(), 88 + 1);
        assert!(data.names.iter().all(|n| !n.ru.is_empty() && !n.en.is_empty() && (0.0..360.0).contains(&n.ra)));
        let seg = segments(&data);
        assert_eq!(seg.len() % 2, 0);
        assert!(seg.iter().all(|v| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2] - 1.0).abs() < 1e-9));
        // Отрезок фигуры не длиннее 60°, иначе RA перепутано со швом 0h.
        let shortest = seg.as_chunks::<2>().0.iter().map(|p| p[0][0] * p[1][0] + p[0][1] * p[1][1] + p[0][2] * p[1][2]).fold(1.0, f64::min);
        assert!(shortest > 60f64.to_radians().cos());
        let star = data.stars.iter().find(|s| s.en == "Betelgeuse").unwrap();
        assert!((star.ra - 88.79).abs() < 0.05 && (star.dec - 7.41).abs() < 0.05);
        assert_eq!(star.ru, "Бетельгейзе");
    }

    #[test]
    fn parse_sample_and_bodies() {
        let sample = r#"{"lines":[[[0,0],[10,0],[10,10]],[[5,5]]],"names":[{"ra":1,"dec":2,"ru":"А","en":"A","rank":1}],"stars":[{"ra":3,"dec":4,"en":"S"}]}"#.as_bytes();
        let data = parse(sample).unwrap();
        assert_eq!(data.lines.len(), 1);
        assert_eq!(segments(&data).len(), 4);
        assert_eq!(data.names[0].rank, 1.0);
        assert_eq!(data.stars[0].ru, "");
        assert_eq!(bodies(0.0).len(), 9);
    }
}
