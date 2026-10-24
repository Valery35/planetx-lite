// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Форматы координат, как в Google Earth: десятичные градусы,
//! градусы-минуты-секунды, UTM и MGRS. Запись и разбор строки.
//!
//! Перенос `core/coords.py` модуля. UTM - поперечная проекция Меркатора
//! на WGS84 рядами Крюгера шестого порядка (C. F. F. Karney, Transverse
//! Mercator with an accuracy of a few nanometers, 2011). Ошибка меньше
//! миллиметра в зоне. Зоны Норвегии и Шпицбергена - по правилам UTM.
//! MGRS строится поверх UTM. Полярные области UPS выше 84° и ниже 80°
//! не поддерживаются.
//!
//! Регулярные выражения модуля разобраны вручную, без крейта regex.
//! Деление и остаток - как у Python, с округлением вниз.

use globe_core::ellipsoid::EARTH;
use globe_core::flight::parse_latlon;

/// Формат записи координат.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Десятичные градусы.
    Decimal,
    /// Градусы, минуты и секунды.
    Dms,
    /// UTM - зона, полоса, восток и север в метрах.
    Utm,
    /// MGRS - зона, полоса, квадрат 100 км и смещения в нём.
    Mgrs,
}

/// Все форматы в порядке модуля.
pub const FORMATS: [Format; 4] = [Format::Decimal, Format::Dms, Format::Utm, Format::Mgrs];

impl Format {
    /// Ключ формата в настройках, как в модуле.
    pub fn key(self) -> &'static str {
        match self {
            Format::Decimal => "decimal",
            Format::Dms => "dms",
            Format::Utm => "utm",
            Format::Mgrs => "mgrs",
        }
    }

    /// Формат по ключу или None для чужого ключа.
    pub fn from_key(key: &str) -> Option<Format> {
        FORMATS.into_iter().find(|f| f.key() == key)
    }
}

/// Масштаб на осевом меридиане.
pub const K0: f64 = 0.9996;
/// Ложный восток, м.
pub const FALSE_EASTING: f64 = 500000.0;
/// Ложный север южного полушария, м.
pub const FALSE_NORTHING: f64 = 10000000.0;
/// Полосы широты по 8°, X - 12°.
pub const BANDS: &str = "CDEFGHJKLMNPQRSTUVWX";
/// Пределы широты UTM.
pub const UTM_LIMITS: (f64, f64) = (-80.0, 84.0);
/// Буквы столбцов 100 км по востоку для трёх наборов зон.
pub const COLUMNS: [&str; 3] = ["ABCDEFGH", "JKLMNPQR", "STUVWXYZ"];
/// Буквы строк 100 км по северу, 20 букв.
pub const ROWS: &str = "ABCDEFGHJKLMNPQRSTUV";
/// Подписи полушарий по умолчанию - север, юг, восток, запад.
pub const HEMISPHERES: [&str; 4] = ["N", "S", "E", "W"];
/// Знаков после точки у десятичных градусов по умолчанию.
pub const DIGITS: usize = 5;
/// Знаков после точки у секунд по умолчанию.
pub const DMS_DIGITS: usize = 1;
/// Цифр смещения MGRS по умолчанию, точность 1 м.
pub const MGRS_DIGITS: u32 = 5;

/// Величины эллипсоида и коэффициенты рядов Крюгера.
struct Series {
    e: f64,
    ar: f64,
    alpha: [f64; 6],
    beta: [f64; 6],
}

/// Коэффициенты рядов Крюгера: alpha - вперёд, beta - обратно.
fn series() -> Series {
    let (a, f) = (EARTH.a, EARTH.f);
    let n = f / (2.0 - f);
    let e = (f * (2.0 - f)).sqrt();
    let (n2, n3, n4, n5, n6) = (n.powi(2), n.powi(3), n.powi(4), n.powi(5), n.powi(6));
    let ar = a / (1.0 + n) * (1.0 + n2 / 4.0 + n4 / 64.0 + n6 / 256.0);
    let alpha = [
        n / 2.0 - 2.0 * n2 / 3.0 + 5.0 * n3 / 16.0 + 41.0 * n4 / 180.0 - 127.0 * n5 / 288.0
            + 7891.0 * n6 / 37800.0,
        13.0 * n2 / 48.0 - 3.0 * n3 / 5.0 + 557.0 * n4 / 1440.0 + 281.0 * n5 / 630.0
            - 1983433.0 * n6 / 1935360.0,
        61.0 * n3 / 240.0 - 103.0 * n4 / 140.0 + 15061.0 * n5 / 26880.0 + 167603.0 * n6 / 181440.0,
        49561.0 * n4 / 161280.0 - 179.0 * n5 / 168.0 + 6601661.0 * n6 / 7257600.0,
        34729.0 * n5 / 80640.0 - 3418889.0 * n6 / 1995840.0,
        212378941.0 * n6 / 319334400.0,
    ];
    let beta = [
        n / 2.0 - 2.0 * n2 / 3.0 + 37.0 * n3 / 96.0 - n4 / 360.0 - 81.0 * n5 / 512.0
            + 96199.0 * n6 / 604800.0,
        n2 / 48.0 + n3 / 15.0 - 437.0 * n4 / 1440.0 + 46.0 * n5 / 105.0 - 1118711.0 * n6 / 3870720.0,
        17.0 * n3 / 480.0 - 37.0 * n4 / 840.0 - 209.0 * n5 / 4480.0 + 5569.0 * n6 / 90720.0,
        4397.0 * n4 / 161280.0 - 11.0 * n5 / 504.0 - 830251.0 * n6 / 7257600.0,
        4583.0 * n5 / 161280.0 - 108847.0 * n6 / 3991680.0,
        20648693.0 * n6 / 638668800.0,
    ];
    Series { e, ar, alpha, beta }
}

/// Целая часть деления и остаток, как `divmod` у Python для float.
fn py_divmod(x: f64, y: f64) -> (f64, f64) {
    let mut rest = x % y;
    let mut div = (x - rest) / y;
    if rest != 0.0 {
        if (y < 0.0) != (rest < 0.0) {
            rest += y;
            div -= 1.0;
        }
    } else {
        rest = 0.0_f64.copysign(y);
    }
    let floor = if div != 0.0 {
        let mut floor = div.floor();
        if div - floor > 0.5 {
            floor += 1.0;
        }
        floor
    } else {
        0.0_f64.copysign(x / y)
    };
    (floor, rest)
}

fn py_floordiv(x: f64, y: f64) -> f64 {
    py_divmod(x, y).0
}

fn py_mod(x: f64, y: f64) -> f64 {
    py_divmod(x, y).1
}

/// Буква набора по номеру.
fn letter_at(set: &str, index: usize) -> Option<char> {
    set.as_bytes().get(index).map(|&b| char::from(b))
}

/// Осевой меридиан зоны в градусах.
fn central_meridian(zone: u32) -> f64 {
    (f64::from(zone) - 1.0) * 6.0 - 180.0 + 3.0
}

/// Набор букв столбцов для зоны.
fn columns_of(zone: u32) -> Option<&'static str> {
    let index = usize::try_from((i64::from(zone) - 1).rem_euclid(3)).ok()?;
    COLUMNS.get(index).copied()
}

/// Сдвиг букв строк у чётных зон.
fn row_shift(zone: u32) -> i64 {
    if zone.is_multiple_of(2) { 5 } else { 0 }
}

/// Номер зоны UTM с исключениями Норвегии и Шпицбергена.
pub fn utm_zone(lat: f64, lon: f64) -> u32 {
    let lon = py_mod(lon + 180.0, 360.0) - 180.0;
    let zone = (py_floordiv(lon + 180.0, 6.0) as i64 + 1).clamp(1, 60);
    if (56.0..64.0).contains(&lat) && (3.0..12.0).contains(&lon) {
        return 32;
    }
    if (72.0..=84.0).contains(&lat) && (0.0..42.0).contains(&lon) {
        if lon < 9.0 {
            return 31;
        }
        if lon < 21.0 {
            return 33;
        }
        if lon < 33.0 {
            return 35;
        }
        return 37;
    }
    u32::try_from(zone).unwrap_or(1)
}

/// Буква полосы широты или None вне UTM.
pub fn band(lat: f64) -> Option<char> {
    if !(UTM_LIMITS.0 <= lat && lat <= UTM_LIMITS.1) {
        return None;
    }
    let index = (py_floordiv(lat + 80.0, 8.0) as usize).min(BANDS.len() - 1);
    letter_at(BANDS, index)
}

fn tau_prime(tau: f64, e: f64) -> f64 {
    let sigma = (e * (e * tau / 1.0_f64.hypot(tau)).atanh()).sinh();
    tau * 1.0_f64.hypot(sigma) - sigma * 1.0_f64.hypot(tau)
}

/// Зона, полоса, восток и север в метрах или None вне UTM.
///
/// Зона None или 0 - зона по точке. Долгота не число - None,
/// в модуле здесь исключение.
pub fn to_utm(lat: f64, lon: f64, zone: Option<u32>) -> Option<(u32, char, f64, f64)> {
    let letter = band(lat)?;
    if !lon.is_finite() {
        return None;
    }
    let zone = match zone {
        Some(zone) if zone != 0 => zone,
        _ => utm_zone(lat, lon),
    };
    let s = series();
    let central = central_meridian(zone);
    let dlon = (py_mod(lon - central + 180.0, 360.0) - 180.0).to_radians();
    let tau = lat.to_radians().tan();
    let tp = tau_prime(tau, s.e);
    let xi_p = tp.atan2(dlon.cos());
    let eta_p = (dlon.sin() / tp.hypot(dlon.cos())).asinh();
    let (mut xi, mut eta) = (xi_p, eta_p);
    for (j, a) in (1..).zip(s.alpha) {
        let k = 2.0 * f64::from(j);
        xi += a * (k * xi_p).sin() * (k * eta_p).cosh();
        eta += a * (k * xi_p).cos() * (k * eta_p).sinh();
    }
    let easting = FALSE_EASTING + K0 * s.ar * eta;
    let mut northing = K0 * s.ar * xi;
    if lat < 0.0 {
        northing += FALSE_NORTHING;
    }
    Some((zone, letter, easting, northing))
}

/// Широта и долгота из UTM. south - южное полушарие.
pub fn from_utm(zone: u32, south: bool, easting: f64, northing: f64) -> (f64, f64) {
    let s = series();
    let xi = (northing - if south { FALSE_NORTHING } else { 0.0 }) / (K0 * s.ar);
    let eta = (easting - FALSE_EASTING) / (K0 * s.ar);
    let (mut xi_p, mut eta_p) = (xi, eta);
    for (j, b) in (1..).zip(s.beta) {
        let k = 2.0 * f64::from(j);
        xi_p -= b * (k * xi).sin() * (k * eta).cosh();
        eta_p -= b * (k * xi).cos() * (k * eta).sinh();
    }
    let tp = xi_p.sin() / eta_p.sinh().hypot(xi_p.cos());
    let dlon = eta_p.sinh().atan2(xi_p.cos());
    // Широта из conformal по Ньютону, как у Karney.
    let mut tau = tp;
    let e2 = s.e * s.e;
    for _ in 0..10 {
        let tpi = tau_prime(tau, s.e);
        let delta = (tp - tpi) / 1.0_f64.hypot(tpi) * (1.0 + (1.0 - e2) * tau * tau)
            / ((1.0 - e2) * 1.0_f64.hypot(tau));
        tau += delta;
        if delta.abs() < 1e-14 {
            break;
        }
    }
    let central = central_meridian(zone);
    let lon = py_mod(central + dlon.to_degrees() + 180.0, 360.0) - 180.0;
    (tau.atan().to_degrees(), lon)
}

/// Строка MGRS с точностью 10^(5-digits) м или None вне UTM.
///
/// digits больше 5 - None, в модуле здесь исключение.
pub fn to_mgrs(lat: f64, lon: f64, digits: u32) -> Option<String> {
    if digits > 5 {
        return None;
    }
    let (zone, letter, easting, northing) = to_utm(lat, lon, None)?;
    let column_index = py_floordiv(easting, 100000.0) as i64 - 1;
    let column = letter_at(columns_of(zone)?, usize::try_from(column_index).ok()?)?;
    let row_index = (py_floordiv(northing, 100000.0) as i64 + row_shift(zone)).rem_euclid(20);
    let row = letter_at(ROWS, usize::try_from(row_index).ok()?)?;
    let scale = 10_i64.pow(5 - digits);
    let east = (py_mod(easting, 100000.0) as i64).div_euclid(scale);
    let north = (py_mod(northing, 100000.0) as i64).div_euclid(scale);
    let width = digits as usize;
    Some(format!("{zone}{letter} {column}{row} {east:0width$} {north:0width$}"))
}

fn band_bottom(index: usize) -> f64 {
    -80.0 + 8.0 * index as f64
}

/// Широта и долгота юго-западного угла квадрата MGRS.
pub fn from_mgrs(
    zone: u32,
    letter: char,
    column: char,
    row: char,
    east: u64,
    north: u64,
    digits: u32,
) -> Option<(f64, f64)> {
    let columns = columns_of(zone)?;
    let column_index = columns.find(column)?;
    let row_index = ROWS.find(row)?;
    let band_index = BANDS.find(letter)?;
    let scale = 10_f64.powi(5 - i32::try_from(digits).ok()?);
    let easting = (column_index as f64 + 1.0) * 100000.0 + east as f64 * scale;
    let north_100 = (row_index as i64 - row_shift(zone)).rem_euclid(20);
    let mut northing = north_100 as f64 * 100000.0 + north as f64 * scale;
    let south = letter < 'N';
    let bottom = band_bottom(band_index);
    let lowest = to_utm(bottom, central_meridian(zone), Some(zone))?.3;
    // Буквы строк повторяются каждые 2000 км. Северная координата -
    // первая с такими буквами не ниже низа полосы широты.
    while northing < lowest - 100000.0 {
        northing += 2000000.0;
    }
    Some(from_utm(zone, south, easting, northing))
}

/// Градусы, минуты и секунды абсолютного значения.
pub fn dms_parts(value: f64) -> (i64, i64, f64) {
    let value = value.abs();
    let degrees = value as i64;
    let minutes_all = (value - degrees as f64) * 60.0;
    let minutes = minutes_all as i64;
    let seconds = (minutes_all - minutes as f64) * 60.0;
    (degrees, minutes, seconds)
}

/// Одна координата в градусах, минутах и секундах с полушарием.
pub fn dms_text(value: f64, positive: &str, negative: &str, digits: usize) -> String {
    let (mut degrees, mut minutes, seconds) = dms_parts(value);
    // Округление, как round у Python - по точной десятичной записи.
    let mut seconds: f64 = format!("{seconds:.digits$}").parse().unwrap_or(seconds);
    if seconds >= 60.0 {
        seconds -= 60.0;
        minutes += 1;
    }
    if minutes >= 60 {
        minutes -= 60;
        degrees += 1;
    }
    let width = if digits > 0 { 3 + digits } else { 2 };
    let hemisphere = if value >= 0.0 { positive } else { negative };
    format!("{degrees}°{minutes:02}′{seconds:0width$.digits$}″ {hemisphere}")
}

/// Точка в выбранном формате.
///
/// hemispheres - подписи полушарий: север, юг, восток, запад. digits -
/// знаков после точки у десятичных градусов. Вне UTM строки UTM и MGRS
/// заменяются десятичными градусами.
pub fn format_point(lat: f64, lon: f64, fmt: Format, hemispheres: [&str; 4], digits: usize) -> String {
    match fmt {
        Format::Dms => {
            let [north, south, east, west] = hemispheres;
            return format!(
                "{}, {}",
                dms_text(lat, north, south, DMS_DIGITS),
                dms_text(lon, east, west, DMS_DIGITS)
            );
        }
        Format::Utm => {
            if let Some((zone, letter, easting, northing)) = to_utm(lat, lon, None) {
                return format!("{zone}{letter} {easting:.0} {northing:.0}");
            }
        }
        Format::Mgrs => {
            if let Some(text) = to_mgrs(lat, lon, MGRS_DIGITS) {
                return text;
            }
        }
        Format::Decimal => {}
    }
    format!("{lat:.digits$}, {lon:.digits$}")
}

// Разбор строки поиска.

/// Слова высоты в строке координат глобуса.
const HEIGHT_WORDS: [&str; 3] = ["высота", "elevation", "height"];
/// Буквы полушарий и их ось - широта или долгота - со знаком.
const HEMI: [(char, bool, f64); 8] = [
    ('N', true, 1.0),
    ('S', true, -1.0),
    ('E', false, 1.0),
    ('W', false, -1.0),
    ('С', true, 1.0),
    ('Ю', true, -1.0),
    ('В', false, 1.0),
    ('З', false, -1.0),
];
/// Русские сокращения полушарий и их буква.
const RU_HEMI: [(char, char, &str); 4] =
    [('с', 'ш', " С "), ('ю', 'ш', " Ю "), ('в', 'д', " В "), ('з', 'д', " З ")];
/// Знаки, без которых строка - не градусы-минуты-секунды.
const DMS_MARKS: &str = "°'′\"″NSEWnsewСЮВЗсювз";
/// Знаки, которые допустимы между числами и буквами полушарий.
const DMS_FILLER: &str = "°'′’\"″”,;";

/// Буква без учёта регистра. lower - строчная.
fn same_letter(c: char, lower: char) -> bool {
    let mut low = c.to_lowercase();
    low.next() == Some(lower) && low.next().is_none()
}

/// Буква слова - как `[^\W\d_]` у Python.
fn is_letter(c: Option<char>) -> bool {
    c.is_some_and(char::is_alphabetic)
}

/// Знак слова - как `\w` у Python.
fn is_word(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() || c == '_')
}

fn is_digit(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_ascii_digit())
}

fn is_space(c: Option<char>) -> bool {
    c.is_some_and(char::is_whitespace)
}

/// Конец ряда знаков с признаком от позиции start.
fn run_end(chars: &[char], start: usize, test: fn(Option<char>) -> bool) -> usize {
    let mut end = start;
    while test(chars.get(end).copied()) {
        end += 1;
    }
    end
}

fn text_of(chars: &[char]) -> String {
    chars.iter().collect()
}

/// Строка без хвоста высоты - `,?\s*(высота|elevation|height)\b.*$`.
fn strip_height(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    for start in 0..chars.len() {
        for word in HEIGHT_WORDS {
            let mut end = start;
            let mut matched = true;
            for lower in word.chars() {
                if !chars.get(end).is_some_and(|&c| same_letter(c, lower)) {
                    matched = false;
                    break;
                }
                end += 1;
            }
            if !matched || is_word(chars.get(end).copied()) {
                continue;
            }
            let mut cut = start;
            while cut > 0 && is_space(chars.get(cut - 1).copied()) {
                cut -= 1;
            }
            if cut > 0 && chars.get(cut - 1) == Some(&',') {
                cut -= 1;
            }
            return chars.get(..cut).map(text_of).unwrap_or_default();
        }
    }
    text.to_string()
}

/// Число `\d+(?:\.\d+)?` от позиции start и его конец.
fn number_at(chars: &[char], start: usize) -> Option<(f64, usize)> {
    let mut end = run_end(chars, start, is_digit);
    if end == start {
        return None;
    }
    if chars.get(end) == Some(&'.') && is_digit(chars.get(end + 1).copied()) {
        end = run_end(chars, end + 1, is_digit);
    }
    let value = text_of(chars.get(start..end)?).parse().ok()?;
    Some((value, end))
}

/// Начало UTM и MGRS - `^(\d{1,2})\s*([C-HJ-NP-X])`: зона, полоса и конец.
fn zone_and_band(chars: &[char]) -> Option<(u32, char, usize)> {
    let digits = run_end(chars, 0, is_digit);
    if !(1..=2).contains(&digits) {
        return None;
    }
    let zone = text_of(chars.get(..digits)?).parse().ok()?;
    let at = run_end(chars, digits, is_space);
    let letter = *chars.get(at)?;
    BANDS.contains(letter).then_some((zone, letter, at + 1))
}

/// Разделитель востока и севера UTM - `\s*[EeВв]?\s*[,;]?\s+`.
fn utm_separator(sep: &[char]) -> bool {
    let mut end = sep.len();
    while end > 0 && is_space(sep.get(end - 1).copied()) {
        end -= 1;
    }
    if end == sep.len() {
        return false;
    }
    if end > 0 && sep.get(end - 1).is_some_and(|&c| c == ',' || c == ';') {
        end -= 1;
    }
    while end > 0 && is_space(sep.get(end - 1).copied()) {
        end -= 1;
    }
    if end > 0 && sep.get(end - 1).is_some_and(|&c| "EeВв".contains(c)) {
        end -= 1;
    }
    sep.get(..end).is_some_and(|head| head.iter().all(|c| c.is_whitespace()))
}

/// Строка UTM в верхнем регистре: зона, полоса, восток и север.
///
/// Образец модуля - `^(\d{1,2})\s*([C-HJ-NP-X])\s+(\d+(?:\.\d+)?)\s*[EeВв]?
/// \s*[,;]?\s+(\d+(?:\.\d+)?)\s*[NnСс]?$`.
fn match_utm(chars: &[char]) -> Option<(u32, char, f64, f64)> {
    let (zone, letter, after) = zone_and_band(chars)?;
    let start = run_end(chars, after, is_space);
    if start == after {
        return None;
    }
    let (east, end) = number_at(chars, start)?;
    // Север начинается с первой цифры после востока.
    let next = (end..chars.len()).find(|&i| is_digit(chars.get(i).copied()))?;
    if !utm_separator(chars.get(end..next)?) {
        return None;
    }
    let (north, end) = number_at(chars, next)?;
    let rest = chars.get(run_end(chars, end, is_space)..)?;
    let tail_ok = match rest {
        [] => true,
        [c] => "NnСс".contains(*c),
        _ => false,
    };
    tail_ok.then_some((zone, letter, east, north))
}

/// Части строки MGRS: зона, полоса, столбец, строка, цифры востока
/// и севера.
type MgrsMatch = (u32, char, char, char, String, String);

/// Строка MGRS в верхнем регистре - `^(\d{1,2})\s*([C-HJ-NP-X])\s*
/// ([A-HJ-NP-Z])\s*([A-HJ-NP-V])\s*(\d*)\s*(\d*)$`.
fn match_mgrs(chars: &[char]) -> Option<MgrsMatch> {
    let (zone, letter, after) = zone_and_band(chars)?;
    let at = run_end(chars, after, is_space);
    let column = *chars.get(at)?;
    if !COLUMNS.iter().any(|set| set.contains(column)) {
        return None;
    }
    let at = run_end(chars, at + 1, is_space);
    let row = *chars.get(at)?;
    if !ROWS.contains(row) {
        return None;
    }
    let start_e = run_end(chars, at + 1, is_space);
    let end_e = run_end(chars, start_e, is_digit);
    let start_n = run_end(chars, end_e, is_space);
    let end_n = run_end(chars, start_n, is_digit);
    if end_n != chars.len() {
        return None;
    }
    Some((zone, letter, column, row, text_of(chars.get(start_e..end_e)?), text_of(chars.get(start_n..end_n)?)))
}

fn parse_mgrs(found: MgrsMatch) -> Option<(f64, f64)> {
    let (zone, letter, column, row, mut digits_e, mut digits_n) = found;
    if digits_n.is_empty() && !digits_e.is_empty() {
        if digits_e.len() % 2 != 0 {
            return None;
        }
        digits_n = digits_e.split_off(digits_e.len() / 2);
    }
    if digits_e.len() != digits_n.len() || digits_e.len() > 5 || !(1..=60).contains(&zone) {
        return None;
    }
    let digits = u32::try_from(digits_e.len()).ok()?;
    let east = if digits > 0 { digits_e.parse().ok()? } else { 0 };
    let north = if digits > 0 { digits_n.parse().ok()? } else { 0 };
    from_mgrs(zone, letter, column, row, east, north, digits)
}

/// Русские сокращения «с. ш.», «ю. ш.», «в. д.», «з. д.» - одной буквой
/// с пробелами, образец `с\.?\s*ш\.?` без учёта регистра.
fn replace_ru_hemi(chars: &[char], first: char, second: char, letter: &str) -> Vec<char> {
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if same_letter(c, first) {
            let mut j = i + 1;
            if chars.get(j) == Some(&'.') {
                j += 1;
            }
            j = run_end(chars, j, is_space);
            if chars.get(j).is_some_and(|&c| same_letter(c, second)) {
                j += 1;
                if chars.get(j) == Some(&'.') {
                    j += 1;
                }
                out.extend(letter.chars());
                i = j;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Запятая между цифрами - десятичная точка, образец `(\d),(\d)`.
fn decimal_commas(chars: &[char]) -> Vec<char> {
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if is_digit(Some(c)) && chars.get(i + 1) == Some(&',') && is_digit(chars.get(i + 2).copied()) {
            out.push(c);
            out.push('.');
            out.extend(chars.get(i + 2).copied());
            i += 3;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Знак разбора градусов - число или буква полушария.
enum Token {
    Number(f64),
    Hemi(char),
}

/// Число со знаком `[+-]?\d+(?:\.\d+)?` от позиции start.
fn signed_number_at(chars: &[char], start: usize) -> Option<(f64, usize)> {
    let digits = if chars.get(start).is_some_and(|&c| c == '+' || c == '-') { start + 1 } else { start };
    let (_, end) = number_at(chars, digits)?;
    let value = text_of(chars.get(start..end)?).parse().ok()?;
    Some((value, end))
}

/// Буква полушария в верхнем регистре или None.
fn hemi_letter(c: char) -> Option<char> {
    let mut upper = c.to_uppercase();
    let first = upper.next()?;
    (upper.next().is_none() && HEMI.iter().any(|h| h.0 == first)).then_some(first)
}

/// Числа и буквы полушарий или None, если в строке есть что-то ещё.
fn dms_tokens(chars: &[char]) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if let Some((value, end)) = signed_number_at(chars, i) {
            tokens.push(Token::Number(value));
            i = end;
            continue;
        }
        // Буква полушария стоит отдельно от слов.
        let before = i.checked_sub(1).and_then(|k| chars.get(k).copied());
        if let Some(letter) = hemi_letter(c)
            && !is_letter(before)
            && !is_letter(chars.get(i + 1).copied())
        {
            tokens.push(Token::Hemi(letter));
            i += 1;
            continue;
        }
        if !(c.is_whitespace() || DMS_FILLER.contains(c)) {
            return None;
        }
        i += 1;
    }
    Some(tokens)
}

/// Градусы, минуты и секунды с полушариями или знаком.
fn parse_dms(text: &str) -> Option<(f64, f64)> {
    if !text.chars().any(|c| DMS_MARKS.contains(c)) {
        return None;
    }
    let mut work: Vec<char> = text.chars().collect();
    for (first, second, letter) in RU_HEMI {
        work = replace_ru_hemi(&work, first, second, letter);
    }
    let work = decimal_commas(&work);
    // Кроме чисел, знаков градусов и букв полушарий в строке ничего нет,
    // иначе это название места, например «Москва».
    let tokens = dms_tokens(&work)?;
    let mut groups: Vec<(Vec<f64>, Option<char>)> = Vec::new();
    let mut numbers = Vec::new();
    for token in tokens {
        match token {
            Token::Number(value) => numbers.push(value),
            Token::Hemi(letter) => groups.push((std::mem::take(&mut numbers), Some(letter))),
        }
    }
    if !numbers.is_empty() {
        if !groups.is_empty() || numbers.len() % 2 != 0 {
            return None;
        }
        let second = numbers.split_off(numbers.len() / 2);
        groups = vec![(numbers, None), (second, None)];
    }
    if groups.len() != 2 {
        return None;
    }
    let (mut lat, mut lon) = (None, None);
    for (index, (parts, letter)) in groups.into_iter().enumerate() {
        let (&first, rest) = parts.split_first()?;
        if rest.len() > 2 || rest.iter().any(|&p| !(0.0..60.0).contains(&p)) {
            return None;
        }
        let mut sum = 0.0;
        for (k, p) in (1..).zip(rest) {
            sum += p / 60.0_f64.powi(k);
        }
        let magnitude = first.abs() + sum;
        let mut sign = if first < 0.0 { -1.0 } else { 1.0 };
        let mut is_lat = index == 0;
        if let Some(letter) = letter {
            let (_, axis, hemi_sign) = HEMI.iter().find(|h| h.0 == letter)?;
            is_lat = *axis;
            sign *= hemi_sign;
        }
        let slot = if is_lat { &mut lat } else { &mut lon };
        if slot.is_some() {
            return None;
        }
        *slot = Some(sign * magnitude);
    }
    let (lat, lon) = (lat?, lon?);
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then_some((lat, lon))
}

/// Широта и долгота из строки поиска или None.
///
/// Понимает десятичные градусы и строку координат глобуса с высотой,
/// градусы-минуты-секунды с полушариями, UTM «40V 513215 6429312»
/// и MGRS «40V EL 13215 29312».
pub fn parse_point(text: &str) -> Option<(f64, f64)> {
    let text = text.trim();
    let stripped = strip_height(text);
    let plain = stripped.trim();
    if let Some(found) = parse_latlon(plain) {
        return Some(found);
    }
    let upper: Vec<char> = plain.to_uppercase().chars().collect();
    if let Some((zone, letter, east, north)) = match_utm(&upper)
        && (1..=60).contains(&zone)
    {
        return Some(from_utm(zone, letter < 'N', east, north));
    }
    if let Some(found) = match_mgrs(&upper) {
        return parse_mgrs(found);
    }
    parse_dms(plain)
}
