// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Свой разбор GRIB2 для полей модели NOAA GFS, `doc/EDITIONS.md`:
//! у RustPlanetX нет terrio, решение автора от 6 ноября 2026 года - «в Про
//! Gdal используется, я его с собой не таскаю, остальное на внутренних
//! алгоритмах». Модуль читает поля GDAL из QGIS, у RustGIS чтения GRIB нет.
//!
//! Читается то, что кладёт GFS 0.25° (WMO FM 92 GRIB edition 2): сетка
//! широт и долгот (шаблон 3.0), данные - простая упаковка (5.0), сложная
//! (5.2) и сложная с пространственными разностями первого и второго
//! порядка (5.3), битовая маска раздела 6. Остальное - ошибка, а не
//! догадка. Порядок восстановления значений - как у g2clib NCEP
//! (`comunpack`). Единицы - как у GDAL с GRIB_NORMALIZE_UNITS: температура
//! из кельвинов в градусы Цельсия.
//!
//! Файл считается враждебным: длины разделов и групп проверяются, чтение
//! за концом данных - ошибка, число точек ограничено [`MAX_POINTS`].

/// Точек поля не больше: сетка 0.25° - 1 038 240, запас на мелкие сетки.
pub const MAX_POINTS: usize = 20_000_000;

/// Поле одного сообщения GRIB2 на сетке широт и долгот.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// Дисциплина, категория и номер параметра (таблицы 0.0, 4.1, 4.2).
    pub discipline: u8,
    pub category: u8,
    pub number: u8,
    /// Узлов по долготе и по широте.
    pub ni: usize,
    pub nj: usize,
    /// Первый узел, градусы, и шаги по долготе и широте.
    pub lat1: f64,
    pub lon1: f64,
    pub di: f64,
    pub dj: f64,
    /// Режим просмотра (таблица 3.4): бит 0x40 - строки идут с юга на север.
    pub scan: u8,
    /// Значения строками по порядку файла, NaN - нет значения.
    pub values: Vec<f32>,
}

fn err(what: &str) -> String {
    format!("GRIB2: {what}")
}

fn bytes(data: &[u8], at: usize, len: usize) -> Result<&[u8], String> {
    data.get(at..at.checked_add(len).ok_or_else(|| err("length"))?).ok_or_else(|| err("past the end"))
}

fn u8_at(data: &[u8], at: usize) -> Result<u8, String> {
    data.get(at).copied().ok_or_else(|| err("past the end"))
}

/// Целое без знака из n октетов, старший первым.
fn uint(data: &[u8], at: usize, n: usize) -> Result<u64, String> {
    Ok(bytes(data, at, n)?.iter().fold(0u64, |v, b| (v << 8) | u64::from(*b)))
}

/// Целое со знаком GRIB: старший бит - знак, остальное - модуль.
fn sint(data: &[u8], at: usize, n: usize) -> Result<i64, String> {
    let raw = uint(data, at, n)?;
    let sign = 1u64 << (8 * n - 1);
    let magnitude = (raw & (sign - 1)) as i64;
    Ok(if raw & sign != 0 { -magnitude } else { magnitude })
}

/// Чтение битов подряд, старший бит октета первым.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn take(&mut self, width: u32) -> Result<u64, String> {
        if width == 0 {
            return Ok(0);
        }
        if width > 64 {
            return Err(err("bit width"));
        }
        let mut out = 0u64;
        for _ in 0..width {
            let byte = *self.data.get(self.pos / 8).ok_or_else(|| err("packed data past the end"))?;
            let bit = (byte >> (7 - (self.pos % 8))) & 1;
            out = (out << 1) | u64::from(bit);
            self.pos += 1;
        }
        Ok(out)
    }

    /// К началу следующего октета.
    fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }
}

/// Упаковка данных раздела 5.
struct Packing {
    template: u16,
    reference: f32,
    binary: i64,
    decimal: i64,
    bits: u32,
    /// Раздел 5 целиком: у сложной упаковки параметры читаются по месту.
    section: Vec<u8>,
}

/// Сообщения файла GRIB2 по порядку. Каждое - одно поле.
pub fn decode(data: &[u8]) -> Result<Vec<Field>, String> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(start) = data.get(at..).and_then(|rest| rest.windows(4).position(|w| w == b"GRIB")).map(|p| p + at) {
        let edition = u8_at(data, start + 7)?;
        if edition != 2 {
            return Err(err("not edition 2"));
        }
        let total = usize::try_from(uint(data, start + 8, 8)?).map_err(|_| err("length"))?;
        let message = bytes(data, start, total)?;
        out.push(message_field(message)?);
        at = start + total.max(16);
    }
    if out.is_empty() {
        return Err(err("no messages"));
    }
    Ok(out)
}

fn message_field(m: &[u8]) -> Result<Field, String> {
    let discipline = u8_at(m, 6)?;
    let mut field = Field { discipline, category: 0, number: 0, ni: 0, nj: 0, lat1: 0.0, lon1: 0.0, di: 0.0, dj: 0.0, scan: 0, values: Vec::new() };
    let mut points = 0usize;
    let mut packing: Option<Packing> = None;
    let mut bitmap: Option<Vec<u8>> = None;
    let mut at = 16;
    while at + 4 <= m.len() {
        if bytes(m, at, 4)? == b"7777" {
            break;
        }
        let len = usize::try_from(uint(m, at, 4)?).map_err(|_| err("length"))?;
        if len < 5 {
            return Err(err("section length"));
        }
        let s = bytes(m, at, len)?;
        match s[4] {
            3 => {
                if uint(s, 12, 2)? != 0 {
                    return Err(err("grid is not latitude-longitude (template 3.0)"));
                }
                field.ni = usize::try_from(uint(s, 30, 4)?).map_err(|_| err("grid"))?;
                field.nj = usize::try_from(uint(s, 34, 4)?).map_err(|_| err("grid"))?;
                // Базовый угол 0 - миллионные доли градуса.
                let basic = uint(s, 38, 4)?;
                let micro = if basic == 0 || basic == 0xFFFF_FFFF { 1e-6 } else { 1.0 / basic as f64 };
                field.lat1 = sint(s, 46, 4)? as f64 * micro;
                field.lon1 = sint(s, 50, 4)? as f64 * micro;
                field.di = uint(s, 63, 4)? as f64 * micro;
                field.dj = uint(s, 67, 4)? as f64 * micro;
                field.scan = u8_at(s, 71)?;
            }
            4 => {
                field.category = u8_at(s, 9)?;
                field.number = u8_at(s, 10)?;
            }
            5 => {
                points = usize::try_from(uint(s, 5, 4)?).map_err(|_| err("points"))?;
                packing = Some(Packing {
                    template: uint(s, 9, 2)? as u16,
                    reference: f32::from_be_bytes(bytes(s, 11, 4)?.try_into().map_err(|_| err("reference"))?),
                    binary: sint(s, 15, 2)?,
                    decimal: sint(s, 17, 2)?,
                    bits: u32::from(u8_at(s, 19)?),
                    section: s.to_vec(),
                });
            }
            6 => match u8_at(s, 5)? {
                0 => bitmap = Some(s[6..].to_vec()),
                255 => bitmap = None,
                _ => return Err(err("predefined bitmap")),
            },
            7 => {
                let p = packing.as_ref().ok_or_else(|| err("data before section 5"))?;
                let grid = field.ni.checked_mul(field.nj).ok_or_else(|| err("grid"))?;
                if grid == 0 || grid > MAX_POINTS || points > grid {
                    return Err(err("grid size"));
                }
                // Температура: как GDAL с GRIB_NORMALIZE_UNITS - градусы
                // Цельсия, сдвиг до округления до f32.
                let offset = if (field.discipline, field.category, field.number) == (0, 0, 0) { -273.15 } else { 0.0 };
                let packed = unpack(p, &s[5..], points, offset)?;
                field.values = spread(packed, grid, bitmap.as_deref())?;
            }
            _ => {}
        }
        at += len;
    }
    if field.values.is_empty() {
        return Err(err("no data section"));
    }
    Ok(field)
}

/// Значения points точек по упаковке p из данных раздела 7, со сдвигом
/// единиц offset.
fn unpack(p: &Packing, data: &[u8], points: usize, offset: f64) -> Result<Vec<f32>, String> {
    let scale = |x: f64| -> f32 { ((f64::from(p.reference) + x * 2f64.powi(p.binary as i32)) / 10f64.powi(p.decimal as i32) + offset) as f32 };
    match p.template {
        0 => {
            let mut bits = Bits { data, pos: 0 };
            (0..points).map(|_| bits.take(p.bits).map(|x| scale(x as f64))).collect()
        }
        2 | 3 => complex(p, data, points).map(|ints| ints.into_iter().map(|x| scale(x as f64)).collect()),
        other => Err(format!("GRIB2: data template 5.{other} is not read")),
    }
}

/// Целые значения сложной упаковки 5.2 и 5.3, `comunpack` g2clib.
fn complex(p: &Packing, data: &[u8], points: usize) -> Result<Vec<i64>, String> {
    let s = &p.section;
    if u8_at(s, 22)? != 0 {
        return Err(err("missing value management is not read"));
    }
    let groups = usize::try_from(uint(s, 31, 4)?).map_err(|_| err("groups"))?;
    if groups > points.max(1) {
        return Err(err("groups"));
    }
    let width_ref = u64::from(u8_at(s, 35)?);
    let width_bits = u32::from(u8_at(s, 36)?);
    let length_ref = uint(s, 37, 4)?;
    let length_inc = u64::from(u8_at(s, 41)?);
    let last_length = uint(s, 42, 4)?;
    let length_bits = u32::from(u8_at(s, 46)?);
    let (order, extra) = if p.template == 3 { (u8_at(s, 47)?, usize::from(u8_at(s, 48)?)) } else { (0, 0) };
    if order > 2 || (order > 0 && !(1..=8).contains(&extra)) {
        return Err(err("spatial differencing"));
    }
    // Первые значения и наименьшая разность пространственных разностей.
    let mut at = 0;
    let mut first = Vec::new();
    for _ in 0..order {
        first.push(sint(data, at, extra)?);
        at += extra;
    }
    let minimum = if order > 0 {
        let m = sint(data, at, extra)?;
        at += extra;
        m
    } else {
        0
    };
    let mut bits = Bits { data, pos: at * 8 };
    let refs: Vec<u64> = (0..groups).map(|_| bits.take(p.bits)).collect::<Result<_, _>>()?;
    bits.align();
    let widths: Vec<u64> = (0..groups).map(|_| bits.take(width_bits).map(|w| w + width_ref)).collect::<Result<_, _>>()?;
    bits.align();
    let mut lengths: Vec<u64> = (0..groups).map(|_| bits.take(length_bits).map(|l| length_ref + l * length_inc)).collect::<Result<_, _>>()?;
    bits.align();
    if let Some(last) = lengths.last_mut() {
        *last = last_length;
    }
    if lengths.iter().sum::<u64>() != points as u64 {
        return Err(err("group lengths do not add up to the points"));
    }
    let mut out = Vec::with_capacity(points);
    for ((r, w), l) in refs.iter().zip(&widths).zip(&lengths) {
        let width = u32::try_from(*w).map_err(|_| err("group width"))?;
        for _ in 0..*l {
            out.push((*r + bits.take(width)?) as i64);
        }
    }
    // Обратные пространственные разности.
    match order {
        1 if !out.is_empty() => {
            out[0] = first[0];
            for n in 1..out.len() {
                out[n] = out[n] + minimum + out[n - 1];
            }
        }
        2 if out.len() >= 2 => {
            out[0] = first[0];
            out[1] = first[1];
            for n in 2..out.len() {
                out[n] = out[n] + minimum + 2 * out[n - 1] - out[n - 2];
            }
        }
        _ => {}
    }
    Ok(out)
}

/// Значения по узлам сетки: с битовой маской - только отмеченные узлы,
/// прочие NaN.
fn spread(packed: Vec<f32>, grid: usize, bitmap: Option<&[u8]>) -> Result<Vec<f32>, String> {
    let Some(mask) = bitmap else {
        if packed.len() != grid {
            return Err(err("points do not match the grid"));
        }
        return Ok(packed);
    };
    let mut out = vec![f32::NAN; grid];
    let mut next = packed.into_iter();
    for (n, v) in out.iter_mut().enumerate() {
        let byte = *mask.get(n / 8).ok_or_else(|| err("bitmap past the end"))?;
        if byte >> (7 - (n % 8)) & 1 == 1 {
            *v = next.next().ok_or_else(|| err("bitmap has more points than the data"))?;
        }
    }
    Ok(out)
}

/// Поле строками с севера на юг, первый столбец - долгота −180°, как его
/// отдаёт GDAL: (значения, строк, столбцов, долгота и широта центра
/// левого верхнего узла, шаг). Сетка не глобальная по долготе - без сдвига.
pub fn north_up(f: &Field) -> (Vec<f32>, usize, usize, f64, f64, f64) {
    let (cols, rows) = (f.ni, f.nj);
    let mut values = f.values.clone();
    // Строки с юга на север - перевернуть.
    if f.scan & 0x40 != 0 {
        values = values.chunks(cols).rev().flatten().copied().collect();
    }
    let north = if f.scan & 0x40 != 0 { f.lat1 + f.dj * (rows as f64 - 1.0) } else { f.lat1 };
    let mut west = if f.lon1 > 180.0 { f.lon1 - 360.0 } else { f.lon1 };
    // Глобальная сетка от 0° - столбцы с −180°.
    let global = (f.di * cols as f64 - 360.0).abs() < f.di * 0.5;
    if global && west >= 0.0 {
        let shift = ((west + 180.0) / f.di).round() as usize % cols.max(1);
        if shift > 0 {
            values = values.chunks(cols).flat_map(|row| row[cols - shift..].iter().chain(&row[..cols - shift]).copied().collect::<Vec<_>>()).collect();
            west -= shift as f64 * f.di;
        }
    }
    (values, rows, cols, west, north, f.di)
}
