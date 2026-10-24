// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Геоид EGM2008 - «измятый глобус». Перенос `core/geoid.py` модуля.
//!
//! Высота геоида над эллипсоидом WGS84 по модели EGM2008 (NGA, 2008),
//! сетка 5' из набора GeographicLib egm2008-5.pgm. Тайлы уровней
//! 0-MAX_LEVEL в записи Terrarium (метры) режет tools/build_geoid.py
//! модуля в хранилище planetx-terrain. В режиме геоида высоты Земли
//! берутся из этих тайлов вместо рельефа и растягиваются в SCALE раз -
//! горбы и впадины геоида от -107 до +86 м становятся видны с орбиты.

pub const MAX_LEVEL: u32 = 4;
pub const TILE_URL: &str = "https://raw.githubusercontent.com/Valery35/planetx-terrain/main/geoid/{z}/{x}/{y}.png";
pub const ATTRIBUTION: (&str, &str) = ("EGM2008, NGA", "https://earth-info.nga.mil/");
/// Вертикальный масштаб режима: 100 м геоида - 1000 км на экране. Выбор
/// помощника модуля, утверждает автор.
pub const SCALE: f64 = 10000.0;
/// Ступени ползунка масштаба меню строки «Геоид», просьба пользователя
/// модуля от 14 октября 2026 года. Ступени - выбор помощника модуля.
pub const SCALES: [f64; 8] = [1000.0, 2000.0, 3000.0, 5000.0, 7000.0, 10000.0, 15000.0, 20000.0];

/// Номер ступени SCALES, ближайшей к масштабу scale. Из равных - первая.
pub fn scale_step(scale: f64) -> usize {
    let mut best = 0;
    for (i, s) in SCALES.iter().enumerate() {
        if (s - scale).abs() < (SCALES[best] - scale).abs() {
            best = i;
        }
    }
    best
}

/// Масштаб для подписи - «×10 000»: целое, тысячи через пробел.
pub fn scale_text(scale: f64) -> String {
    let whole = scale.round_ties_even() as i64;
    let digits = whole.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    format!("×{}{out}", if whole < 0 { "-" } else { "" })
}

/// Метры (NaN - нет данных) в RGB записи Terrarium:
/// высота = R * 256 + G + B / 256 - 32768.
pub fn encode(heights: &[f64]) -> Vec<[u8; 3]> {
    heights
        .iter()
        .map(|&h| {
            let code = (if h.is_nan() { 0.0 } else { h } + 32768.0).clamp(0.0, 65535.996);
            let whole = code.floor();
            let blue = ((code - whole) * 256.0).round_ties_even().min(255.0);
            [(whole / 256.0).floor() as u8, (whole % 256.0) as u8, blue as u8]
        })
        .collect()
}

/// Сетка высот: строки с севера, столбцы с востока от 0°, по строкам.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Grid {
    pub width: usize,
    pub height: usize,
    pub values: Vec<f64>,
}

impl Grid {
    /// Значение строки row и столбца col, вне сетки - NaN.
    pub fn at(&self, row: usize, col: usize) -> f64 {
        if col >= self.width {
            return f64::NAN;
        }
        row.checked_mul(self.width).and_then(|i| self.values.get(i + col)).copied().unwrap_or(f64::NAN)
    }
}

/// Сетка GeographicLib PGM (P5, 16 бит) в метры и шаг в градусах:
/// строки с севера от 90°, столбцы с востока от 0°. Смещение и шкала -
/// из комментариев `# Offset` и `# Scale`. Файл считается враждебным.
pub fn read_pgm(data: &[u8]) -> Result<(Grid, f64), String> {
    let (mut offset, mut scale) = (None, None);
    let mut pos = 0;
    let mut fields: Vec<String> = Vec::new();
    let number = |parts: &[&str]| parts.get(1).and_then(|v| v.parse::<f64>().ok()).ok_or("bad PGM comment");
    while fields.len() < 4 {
        let end = data.get(pos..).and_then(|rest| rest.iter().position(|b| *b == b'\n')).ok_or("short PGM header")? + pos;
        let line = std::str::from_utf8(&data[pos..end]).map_err(|e| e.to_string())?;
        if !line.is_ascii() {
            return Err("not an ASCII PGM header".into());
        }
        let line = line.trim();
        pos = end + 1;
        if let Some(comment) = line.strip_prefix('#') {
            let parts: Vec<&str> = comment.split_whitespace().collect();
            match parts.first() {
                Some(&"Offset") => offset = Some(number(&parts)?),
                Some(&"Scale") => scale = Some(number(&parts)?),
                _ => {}
            }
            continue;
        }
        fields.extend(line.split_whitespace().map(str::to_string));
    }
    let (Some(offset), Some(scale)) = (offset, scale) else {
        return Err("not a GeographicLib geoid PGM".into());
    };
    if fields[0] != "P5" {
        return Err("not a GeographicLib geoid PGM".into());
    }
    let size = |text: &str| text.parse::<usize>().map_err(|e| e.to_string());
    let (width, height) = (size(&fields[1])?, size(&fields[2])?);
    if width == 0 {
        return Err("empty PGM".into());
    }
    let count = width.checked_mul(height).ok_or("bad PGM size")?;
    let body = count.checked_mul(2).and_then(|n| data.get(pos..pos.checked_add(n)?)).ok_or("short PGM data")?;
    let values = body.as_chunks::<2>().0.iter().map(|b| offset + scale * f64::from(u16::from_be_bytes(*b))).collect();
    Ok((Grid { width, height, values }, 360.0 / width as f64))
}
