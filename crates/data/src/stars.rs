// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Звёзды: направления в ECEF на момент времени, размер и цвет. Перенос
//! `core/stars.py` и `load` из `render/stars.py` модуля.
//!
//! Каталог - звёзды BSC5 до величины MAX_MAG, прямое восхождение
//! и склонение J2000 в радианах, видимая величина и показатель цвета B-V,
//! файл data/stars.npy модуля, Lite берёт его из хранилища модуля.
//! Небесная сфера поворачивается вокруг оси Земли на гринвичское среднее
//! звёздное время. Прецессия с 2000 года - около 0.4°, она не
//! учитывается. Звёзды гаснут, когда камера опускается в атмосферу,
//! от FADE_HIGH до FADE_LOW метров.

/// Файл каталога модуля.
pub const URL: &str = "https://raw.githubusercontent.com/Valery35/planetx/main/planetx/data/stars.npy";
pub const ATTRIBUTION: (&str, &str) = ("Stars: Yale Bright Star Catalogue", "http://tdc-www.harvard.edu/catalogs/bsc5.html");
/// Звёзды тусклее в файл не идут.
pub const MAX_MAG: f64 = 6.0;
/// Выше звёзды видны полностью, ниже FADE_LOW звёзд нет, метры.
pub const FADE_HIGH: f64 = 150000.0;
pub const FADE_LOW: f64 = 30000.0;
/// 1 января 2000 года, 12:00 UT, в секундах Unix.
pub const UNIX_J2000: f64 = 946728000.0;
/// Размер самой яркой звезды и звезды величины MAX_MAG, логические
/// пиксели.
pub const SIZE_BRIGHT: f64 = 4.5;
pub const SIZE_FAINT: f64 = 1.6;

/// Звезда для видеокарты: направление в экваториальной системе, размер
/// в логических пикселях, цвет RGB и яркость от 0 до 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Star {
    pub dir: [f64; 3],
    pub size: f64,
    pub color: [f64; 3],
    pub brightness: f64,
}

/// Гринвичское среднее звёздное время в радианах, от 0 до 2π.
pub fn gmst(unix_time: f64) -> f64 {
    let days = (unix_time - UNIX_J2000) / 86400.0;
    let degrees = 280.46061837 + 360.98564736629 * days;
    degrees.rem_euclid(360.0).to_radians()
}

/// Единичный вектор звезды в экваториальной системе.
pub fn sky_direction(ra: f64, dec: f64) -> [f64; 3] {
    [dec.cos() * ra.cos(), dec.cos() * ra.sin(), dec.sin()]
}

/// Поворот из экваториальной системы в ECEF на момент unix_time, по
/// строкам: Земля повёрнута на угол GMST, звёзды в её системе - на тот
/// же угол назад.
pub fn sky_rotation(unix_time: f64) -> [[f64; 3]; 3] {
    let angle = gmst(unix_time);
    let (s, c) = angle.sin_cos();
    [[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]]
}

/// Направление в ECEF на момент unix_time.
pub fn to_ecef(dir: [f64; 3], unix_time: f64) -> [f64; 3] {
    let m = sky_rotation(unix_time);
    [0, 1, 2].map(|r| m[r][0] * dir[0] + m[r][1] * dir[1] + m[r][2] * dir[2])
}

/// Размер точки в логических пикселях по видимой величине.
pub fn size(mag: f64) -> f64 {
    let share = ((mag + 1.5) / (MAX_MAG + 1.5)).clamp(0.0, 1.0);
    SIZE_BRIGHT + (SIZE_FAINT - SIZE_BRIGHT) * share
}

/// Яркость от 0 до 1. Разница в 5 величин - в 100 раз по свету, здесь
/// сжата, иначе тусклые звёзды не видны на экране.
pub fn brightness(mag: f64) -> f64 {
    10f64.powf(-0.1 * (mag - 1.0)).clamp(0.45, 1.0)
}

fn interp(x: f64, xs: &[f64], ys: &[f64]) -> f64 {
    if x <= xs[0] {
        return ys[0];
    }
    for i in 1..xs.len() {
        if x <= xs[i] {
            let t = (x - xs[i - 1]) / (xs[i] - xs[i - 1]);
            return ys[i - 1] + t * (ys[i] - ys[i - 1]);
        }
    }
    ys[ys.len() - 1]
}

/// Цвет RGB от 0 до 1 по показателю B-V: голубые звёзды около -0.3,
/// белые около 0.3, оранжевые и красные от 1.2. Нет значения - 0.6.
pub fn color(bv: f64) -> [f64; 3] {
    let bv = if bv.is_finite() { bv } else { 0.6 }.clamp(-0.4, 2.0);
    let stops = [-0.4, 0.0, 0.4, 0.8, 1.2, 2.0];
    [
        interp(bv, &stops, &[0.62, 0.80, 1.00, 1.00, 1.00, 1.00]),
        interp(bv, &stops, &[0.72, 0.86, 0.98, 0.93, 0.80, 0.62]),
        interp(bv, &stops, &[1.00, 1.00, 0.96, 0.80, 0.60, 0.40]),
    ]
}

/// Доля видимости звёзд от высоты камеры в метрах, от 0 до 1.
pub fn fade(altitude: f64) -> f64 {
    if altitude <= FADE_LOW {
        return 0.0;
    }
    if altitude >= FADE_HIGH {
        return 1.0;
    }
    (altitude / FADE_LOW).ln() / (FADE_HIGH / FADE_LOW).ln()
}

/// Таблица (N, 4) чисел float32 или float64 из файла NumPy .npy:
/// восхождение, склонение, величина, B-V. Файл считается враждебным.
pub fn read_npy(data: &[u8]) -> Result<Vec<[f64; 4]>, String> {
    if !data.starts_with(b"\x93NUMPY") || data.len() < 10 {
        return Err("not a .npy file".into());
    }
    let major = data[6];
    let (len, start) = if major == 1 { (usize::from(u16::from_le_bytes([data[8], data[9]])), 10) } else {
        let b = data.get(8..12).ok_or("short .npy header")?;
        (u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize, 12)
    };
    let header = std::str::from_utf8(data.get(start..start + len).ok_or("short .npy header")?).map_err(|e| e.to_string())?;
    let width = if header.contains("'<f4'") {
        4
    } else if header.contains("'<f8'") {
        8
    } else {
        return Err("unsupported .npy type".into());
    };
    if header.contains("'fortran_order': True") || !header.contains(", 4)") {
        return Err("unsupported .npy layout".into());
    }
    let body = &data[start + len..];
    let value = |chunk: &[u8]| -> f64 {
        match chunk {
            [a, b, c, d] => f64::from(f32::from_le_bytes([*a, *b, *c, *d])),
            [a, b, c, d, e, f, g, h] => f64::from_le_bytes([*a, *b, *c, *d, *e, *f, *g, *h]),
            _ => f64::NAN,
        }
    };
    Ok(body.chunks_exact(width * 4).map(|row| [0, 1, 2, 3].map(|k| value(&row[k * width..(k + 1) * width]))).collect())
}

/// Звёзды каталога из байтов stars.npy, как `load` модуля.
pub fn parse(data: &[u8]) -> Result<Vec<Star>, String> {
    Ok(read_npy(data)?
        .into_iter()
        .filter(|r| r[0].is_finite() && r[1].is_finite() && r[2].is_finite())
        .map(|[ra, dec, mag, bv]| Star { dir: sky_direction(ra, dec), size: size(mag), color: color(bv), brightness: brightness(mag) })
        .collect())
}
