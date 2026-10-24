// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Кора Земли по модели CRUST1.0. Перенос `core/crust.py` модуля.
//!
//! CRUST1.0 (Laske, Masters, Ma, Pasyanos, 2013) - сетка ячеек 1° × 1°,
//! в каждой [`BOUNDS`] отметок границ в километрах, вверх положительно:
//!
//! - 0 - верх воды (на суше - рельеф);
//! - 1 - низ воды;
//! - 2 - низ льда;
//! - 3-5 - низ верхних, средних и нижних осадков;
//! - 6-8 - низ верхней, средней и нижней коры, последняя - Мохо.
//!
//! Слой нулевой толщины в ячейке - две равные отметки подряд. Модель
//! лежит на сайте UCSD, файл границ [`MEMBER`] в архиве [`FILE`] ([`URL`]).
//! Программа её не распространяет, а скачивает при первом показе. Авторы
//! просят ссылаться на сайт модели или на статью, см. [`CITATION`].
//!
//! Строки файла идут с широты 89.5° на юг, внутри строки широты - долготы
//! от -179.5° на восток. Ячейка - середина клетки, точка относится
//! к ячейке, в которую попадает.
//!
//! Континентальная или океаническая кора - по скорости продольных волн
//! в верхней коре, файл [`VP_MEMBER`] того же архива. Верхняя кора
//! медленнее [`OCEANIC_VP`] - океаническая.
//!
//! Архив tar.gz читается здесь же: gzip через flate2, tar - простой разбор
//! ustar с длинными именами GNU и pax. Архив считается враждебным:
//! распакованный поток ограничен [`ARCHIVE_LIMIT`], член - [`MEMBER_LIMIT`].

use std::io::Read;

/// Адрес архива модели.
pub const URL: &str = "https://igppweb.ucsd.edu/~gabi/crust1/crust1.0.tar.gz";
/// Имя архива в папке PlanetX профиля.
pub const FILE: &str = "crust1.0.tar.gz";
/// Член архива с отметками границ.
pub const MEMBER: &str = "crust1.bnds";
/// Член архива со скоростями продольных волн.
pub const VP_MEMBER: &str = "crust1.vp";
/// Ссылка на модель: название и адрес.
pub const CITATION: (&str, &str) = ("CRUST1.0, Laske, Masters, Ma, Pasyanos, 2013", "https://igppweb.ucsd.edu/~gabi/crust1.html");
/// Строк, столбцов и отметок в ячейке.
pub const ROWS: usize = 180;
pub const COLS: usize = 360;
pub const BOUNDS: usize = 9;
/// Слои между соседними отметками, сверху вниз.
pub const LAYERS: [&str; 8] =
    ["water", "ice", "sediments_upper", "sediments_middle", "sediments_lower", "crust_upper", "crust_middle", "crust_lower"];
/// Номер отметки Мохо.
pub const MOHO: usize = 8;
/// Номер слоя верхней коры в [`LAYERS`].
pub const UPPER_CRUST: usize = 5;
/// Скорость, км/с: верхняя кора медленнее - океаническая.
pub const OCEANIC_VP: f64 = 5.5;
/// Поверхности для выгрузки: ключ и номер отметки - кровли трёх слоёв
/// осадков, трёх слоёв кристаллической коры и Мохо.
pub const SURFACES: [(&str, usize); 7] = [
    ("sediments_upper", 2),
    ("sediments_middle", 3),
    ("sediments_lower", 4),
    ("crust_upper", 5),
    ("crust_middle", 6),
    ("crust_lower", 7),
    ("moho", MOHO),
];
/// Середина левой верхней ячейки сетки и шаг, градусы.
pub const GRID_WEST: f64 = -179.5;
pub const GRID_NORTH: f64 = 89.5;
pub const GRID_STEP: f64 = 1.0;
/// Наибольший распакованный поток архива. Настоящий архив - около 20 МБ.
pub const ARCHIVE_LIMIT: u64 = 256 << 20;
/// Наибольший член архива, который читается. Файл границ - около 5 МБ.
pub const MEMBER_LIMIT: u64 = 64 << 20;

/// Значений в файле границ.
const VALUES: usize = ROWS * COLS * BOUNDS;
/// Блок tar.
const BLOCK: usize = 512;

/// Файл модели не того вида.
#[derive(Debug, Clone, PartialEq)]
pub struct CrustError(pub String);

impl std::fmt::Display for CrustError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CrustError {}

fn fail(text: impl Into<String>) -> CrustError {
    CrustError(text.into())
}

/// Отметки границ (ROWS × COLS × BOUNDS по строкам) в км из текста
/// crust1.bnds. Байты не ASCII считаются ошибкой числа.
pub fn parse_bounds(text: &[u8]) -> Result<Vec<f32>, CrustError> {
    let text = String::from_utf8_lossy(text);
    let mut values = Vec::with_capacity(VALUES);
    for word in text.split_ascii_whitespace() {
        if values.len() == VALUES {
            return Err(fail("too many values"));
        }
        let value: f64 = word.parse().map_err(|_| fail(format!("could not convert string to float: '{word}'")))?;
        values.push(value as f32);
    }
    if values.len() != VALUES {
        return Err(fail(format!("{} values", values.len())));
    }
    Ok(values)
}

/// Число в восьмеричной записи поля заголовка tar или в base-256 GNU.
fn octal(field: &[u8]) -> Option<u64> {
    if field.first().is_some_and(|b| b & 0x80 != 0) {
        let mut value: u64 = u64::from(field[0] & 0x7f);
        for b in &field[1..] {
            value = value.checked_mul(256)?.checked_add(u64::from(*b))?;
        }
        return Some(value);
    }
    let text = std::str::from_utf8(field).ok()?;
    let text = text.trim_matches(|c: char| c == '\0' || c == ' ');
    if text.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(text, 8).ok()
}

/// Строка поля до первого нуля.
fn field_text(field: &[u8]) -> String {
    let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// Путь из расширенного заголовка pax или None.
fn pax_path(data: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(data);
    let mut rest: &str = &text;
    let mut found = None;
    while !rest.is_empty() {
        let space = rest.find(' ')?;
        let len: usize = rest[..space].parse().ok()?;
        let record = rest.get(space + 1..len)?;
        if let Some(path) = record.strip_suffix('\n').unwrap_or(record).strip_prefix("path=") {
            found = Some(path.to_string());
        }
        rest = rest.get(len..)?;
    }
    found
}

/// Читает ровно buf, false - поток кончился раньше.
fn fill(reader: &mut impl Read, buf: &mut [u8]) -> Result<bool, CrustError> {
    let mut done = 0;
    while done < buf.len() {
        match reader.read(&mut buf[done..]) {
            Ok(0) => return Ok(false),
            Ok(n) => done += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(fail(e.to_string())),
        }
    }
    Ok(true)
}

/// Члены wanted из архива tar.gz: по имени - содержимое последнего члена
/// с этим именем или None.
fn members(data: &[u8], wanted: &[&str]) -> Result<Vec<Option<Vec<u8>>>, CrustError> {
    let mut reader = flate2::read::MultiGzDecoder::new(data).take(ARCHIVE_LIMIT);
    let mut found: Vec<Option<Vec<u8>>> = vec![None; wanted.len()];
    let mut long_name: Option<String> = None;
    let mut header = [0u8; BLOCK];
    let mut seen = 0usize;
    loop {
        if !fill(&mut reader, &mut header)? {
            if seen == 0 {
                return Err(fail("empty file"));
            }
            break;
        }
        if header.iter().all(|b| *b == 0) {
            break;
        }
        let sum = octal(&header[148..156]).ok_or_else(|| fail("bad checksum"))?;
        // Сумма байтов без знака, у старых архивов - со знаком.
        let field = |i: usize| (148..156).contains(&i);
        let own: u64 = header.iter().enumerate().map(|(i, b)| if field(i) { 32 } else { u64::from(*b) }).sum();
        let signed: i64 = header.iter().enumerate().map(|(i, b)| if field(i) { 32 } else { i64::from(*b as i8) }).sum();
        if sum != own && i64::try_from(sum).ok() != Some(signed) {
            return Err(fail("bad checksum"));
        }
        seen += 1;
        let size = octal(&header[124..136]).ok_or_else(|| fail("bad size"))?;
        let kind = header[156];
        let mut name = field_text(&header[0..100]);
        // Приставка имени - только у POSIX ustar, у GNU там другие поля.
        if &header[257..263] == b"ustar\0" {
            let prefix = field_text(&header[345..500]);
            if !prefix.is_empty() {
                name = format!("{prefix}/{name}");
            }
        }
        if let Some(long) = long_name.take() {
            name = long;
        }
        let padded = size.div_ceil(BLOCK as u64) * BLOCK as u64;
        let slot = wanted.iter().position(|w| *w == name).filter(|_| matches!(kind, b'0' | 0 | b'7'));
        let keep = matches!(kind, b'L' | b'x') || slot.is_some();
        if keep {
            if size > MEMBER_LIMIT {
                return Err(fail(format!("{name}: member too large")));
            }
            let mut body = vec![0u8; padded as usize];
            if !fill(&mut reader, &mut body)? {
                return Err(fail("unexpected end of data"));
            }
            body.truncate(size as usize);
            match (kind, slot) {
                (b'L', _) => long_name = Some(field_text(&body)),
                (b'x', _) => long_name = pax_path(&body),
                (_, Some(k)) => found[k] = Some(body),
                _ => {}
            }
        } else {
            let skipped = std::io::copy(&mut (&mut reader).take(padded), &mut std::io::sink()).map_err(|e| fail(e.to_string()))?;
            if skipped < padded {
                return Err(fail("unexpected end of data"));
            }
        }
    }
    Ok(found)
}

/// Модель из байтов архива crust1.0.tar.gz. Скорости crust1.vp читаются,
/// если файл есть в архиве.
pub fn from_archive(data: &[u8]) -> Result<Crust, CrustError> {
    let mut found = members(data, &[MEMBER, VP_MEMBER])?.into_iter();
    let text = found.next().flatten().ok_or_else(|| fail(format!("\"filename '{MEMBER}' not found\"")))?;
    let speeds = found.next().flatten();
    let bounds = parse_bounds(&text)?;
    let vp = speeds.map(|s| parse_bounds(&s)).transpose()?;
    Crust::new(bounds, vp)
}

/// Строка и столбец ячейки для широты и долготы.
pub fn cell(lat: f64, lon: f64) -> (usize, usize) {
    let row = (90.0 - lat).floor();
    // np.clip и astype: NaN уходит в 0 после приведения, здесь так же.
    let row = if row.is_nan() { 0.0 } else { row.clamp(0.0, (ROWS - 1) as f64) };
    let col = (lon + 180.0).rem_euclid(360.0).floor();
    let col = if col.is_finite() { (col as i64).rem_euclid(COLS as i64) as usize } else { 0 };
    (row as usize, col)
}

/// Отметки границ CRUST1.0 и скорости продольных волн vp той же разметки,
/// км/с, или None.
#[derive(Debug, Clone, PartialEq)]
pub struct Crust {
    bounds: Vec<f32>,
    vp: Option<Vec<f32>>,
}

impl Crust {
    /// Модель из отметок и скоростей по строкам, ROWS × COLS × BOUNDS
    /// значений в каждом.
    pub fn new(bounds: Vec<f32>, vp: Option<Vec<f32>>) -> Result<Crust, CrustError> {
        if bounds.len() != VALUES || vp.as_ref().is_some_and(|v| v.len() != VALUES) {
            return Err(fail(format!("{} values", bounds.len())));
        }
        Ok(Crust { bounds, vp })
    }

    /// Отметки границ по строкам.
    pub fn bounds(&self) -> &[f32] {
        &self.bounds
    }

    /// Скорости продольных волн по строкам или None.
    pub fn vp(&self) -> Option<&[f32]> {
        self.vp.as_deref()
    }

    fn slot(lat: f64, lon: f64) -> usize {
        let (row, col) = cell(lat, lon);
        (row * COLS + col) * BOUNDS
    }

    /// Океаническая ли кора в точке, None без скоростей.
    pub fn oceanic(&self, lat: f64, lon: f64) -> Option<bool> {
        let vp = self.vp.as_ref()?;
        let speed = vp.get(Self::slot(lat, lon) + UPPER_CRUST).copied().unwrap_or(f32::NAN);
        Some(f64::from(speed) < OCEANIC_VP)
    }

    /// Отметки границ в км для точки.
    pub fn at(&self, lat: f64, lon: f64) -> [f64; BOUNDS] {
        let start = Self::slot(lat, lon);
        std::array::from_fn(|k| self.bounds.get(start + k).map_or(f64::NAN, |v| f64::from(*v)))
    }

    /// Глубина Мохо ниже уровня моря, км, положительная.
    pub fn moho(&self, lat: f64, lon: f64) -> f64 {
        -self.at(lat, lon)[MOHO]
    }

    /// Толщина коры без воды, км: от низа воды до Мохо.
    pub fn thickness(&self, lat: f64, lon: f64) -> f64 {
        let b = self.at(lat, lon);
        b[1] - b[MOHO]
    }

    /// Поверхности [`SURFACES`] на всю сетку: ключ и отметки ROWS × COLS
    /// по строкам в метрах над уровнем моря, вниз отрицательно. Строки
    /// с севера, середина первой ячейки [`GRID_WEST`], [`GRID_NORTH`]. Где
    /// слоя нет, кровля совпадает с кровлей следующего.
    pub fn surfaces(&self) -> Vec<(&'static str, Vec<f64>)> {
        SURFACES
            .iter()
            .map(|(key, k)| (*key, self.bounds.iter().skip(*k).step_by(BOUNDS).map(|v| f64::from(*v) * 1000.0).collect()))
            .collect()
    }

    /// Слои в точке: ключ, верх и низ в км, только ненулевые, сверху вниз.
    pub fn layers(&self, lat: f64, lon: f64) -> Vec<(&'static str, f64, f64)> {
        let b = self.at(lat, lon);
        LAYERS.iter().enumerate().filter(|(k, _)| b[*k] > b[k + 1]).map(|(k, key)| (*key, b[k], b[k + 1])).collect()
    }
}
