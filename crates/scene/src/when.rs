// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Время меток, как TimeStamp и TimeSpan в KML Google Earth, перенос
//! `core/when.py` модуля.
//!
//! Время метки - пара строк (начало, конец) в записи KML, dateTime
//! XML Schema: «2026», «2026-09», «2026-09-30», «2026-09-30T12:00:00Z»,
//! с поясом «+05:00» или без. Момент - начало и конец одинаковые.
//! Пустая строка - открытый край промежутка. Строки хранятся как есть,
//! так файл Google Earth проходит туда и обратно без потерь. Для шкалы
//! времени строка переводится в секунды от 1970 года UTC. Время без
//! пояса считается UTC.
//!
//! Годы до нашей эры пишутся со знаком, как в XML Schema 1.0: «-0264» -
//! 264 год до н. э., нулевого года нет. Календарь - пролептический
//! григорианский, счёт дней свой.

/// Время метки: начало и конец в записи KML.
pub type Time = (String, String);

/// Дней от 1 января 1970 года до даты. Год астрономический: 0 - это
/// 1 год до н. э.
fn days(year: i64, month: i64, day: i64) -> i64 {
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Дата и время UTC: год, месяц, день, час, минута, секунда. Год
/// астрономический: 0 - это 1 год до н. э., -263 - 264 год до н. э.
pub fn civil(seconds: f64) -> (i64, i64, i64, i64, i64, i64) {
    let total = seconds.floor() as i64;
    let (days, rest) = (total.div_euclid(86_400), total.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day, rest / 3600, rest % 3600 / 60, rest % 60)
}

/// Цифры с позиции pos, ровно n штук, или None.
fn digits(s: &[u8], pos: &mut usize, n: usize) -> Option<i64> {
    let part = s.get(*pos..*pos + n)?;
    if !part.iter().all(u8::is_ascii_digit) {
        return None;
    }
    *pos += n;
    part.iter().try_fold(0i64, |acc, d| acc.checked_mul(10)?.checked_add(i64::from(d - b'0')))
}

/// Секунды от 1970 года UTC или None для пустой и неверной строки.
/// Неполная дата - начало своего года, месяца или дня. Год бывает
/// длиннее четырёх цифр, «-28000» - глубокое время.
pub fn parse(text: &str) -> Option<f64> {
    let s = text.trim().as_bytes();
    if s.is_empty() {
        return None;
    }
    let mut pos = 0;
    let negative = s.first() == Some(&b'-');
    if negative {
        pos += 1;
    }
    let start = pos;
    while s.get(pos).is_some_and(u8::is_ascii_digit) {
        pos += 1;
    }
    let len = pos - start;
    if !(4..=12).contains(&len) {
        return None;
    }
    let mut year: i64 = std::str::from_utf8(&s[start..pos]).ok()?.parse().ok()?;
    if negative {
        // «-0264» - 264 год до н. э., астрономический год -263.
        year = -year + 1;
    }
    let (mut month, mut day, mut hour, mut minute, mut second) = (1, 1, 0, 0, 0);
    let mut fraction = 0.0;
    let mut zone = 0i64;
    if pos < s.len() {
        if s[pos] != b'-' {
            return None;
        }
        pos += 1;
        month = digits(s, &mut pos, 2)?;
        if pos < s.len() {
            if s[pos] != b'-' {
                return None;
            }
            pos += 1;
            day = digits(s, &mut pos, 2)?;
            if pos < s.len() {
                if s[pos] != b'T' && s[pos] != b' ' {
                    return None;
                }
                pos += 1;
                hour = digits(s, &mut pos, 2)?;
                if s.get(pos) != Some(&b':') {
                    return None;
                }
                pos += 1;
                minute = digits(s, &mut pos, 2)?;
                if s.get(pos) == Some(&b':') {
                    pos += 1;
                    second = digits(s, &mut pos, 2)?;
                    if s.get(pos) == Some(&b'.') {
                        pos += 1;
                        let from = pos;
                        while s.get(pos).is_some_and(u8::is_ascii_digit) {
                            pos += 1;
                        }
                        if pos == from {
                            return None;
                        }
                        fraction = format!("0.{}", std::str::from_utf8(&s[from..pos]).ok()?).parse().ok()?;
                    }
                }
                while s.get(pos) == Some(&b' ') {
                    pos += 1;
                }
                match s.get(pos) {
                    None => {}
                    Some(b'Z') => pos += 1,
                    Some(&c) if c == b'+' || c == b'-' => {
                        let sign = if c == b'-' { -1 } else { 1 };
                        pos += 1;
                        let h = digits(s, &mut pos, 2)?;
                        if s.get(pos) == Some(&b':') {
                            pos += 1;
                        }
                        let m = digits(s, &mut pos, 2)?;
                        zone = sign * (h * 3600 + m * 60);
                    }
                    Some(_) => return None,
                }
            }
        }
    }
    if pos != s.len() {
        return None;
    }
    if !((1..=12).contains(&month) && (1..=31).contains(&day) && hour <= 24 && minute <= 59 && second <= 61) {
        return None;
    }
    let seconds = days(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - zone;
    Some(seconds as f64 + fraction)
}

/// Строка KML «2026-09-30T12:00:00Z» из секунд.
pub fn text(seconds: f64) -> String {
    let (year, month, day, hour, minute, second) = civil(seconds);
    let era = if year > 0 { format!("{year:04}") } else { format!("-{:04}", 1 - year) };
    format!("{era}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Время-момент: пара одинаковых строк.
pub fn stamp(when: &str) -> Time {
    (when.to_string(), when.to_string())
}

pub fn is_stamp(time: Option<&Time>) -> bool {
    time.is_some_and(|t| t.0 == t.1 && !t.0.is_empty())
}

/// Промежуток в секундах (от, до). None - время не задано, открытый
/// край - бесконечность.
pub fn interval(time: Option<&Time>) -> Option<(f64, f64)> {
    let (a, b) = time?;
    if a.is_empty() && b.is_empty() {
        return None;
    }
    let (lo, hi) = (parse(a), parse(b));
    if lo.is_none() && hi.is_none() {
        return None;
    }
    Some((lo.unwrap_or(f64::NEG_INFINITY), hi.unwrap_or(f64::INFINITY)))
}

/// Видна ли метка со временем time в окне шкалы (lo, hi). Метка без
/// времени видна всегда, как в Google Earth.
pub fn visible(time: Option<&Time>, lo: f64, hi: f64) -> bool {
    interval(time).is_none_or(|(a, b)| a <= hi && b >= lo)
}

/// Доля промежутка time, прошедшая к моменту moment, от 0 до 1. Без
/// конечного начала и конца - 1: расти нечему.
pub fn share(time: Option<&Time>, moment: f64) -> f64 {
    match interval(time) {
        Some((a, b)) if a.is_finite() && b.is_finite() && b > a => ((moment - a) / (b - a)).clamp(0.0, 1.0),
        _ => 1.0,
    }
}

/// Охват шкалы по временам меток: самое раннее и самое позднее или
/// None, если ни у кого нет конечного времени.
pub fn extent<'a>(times: impl IntoIterator<Item = Option<&'a Time>>) -> Option<(f64, f64)> {
    let values: Vec<f64> = times
        .into_iter()
        .filter_map(interval)
        .flat_map(|(a, b)| [a, b])
        .filter(|v| v.is_finite())
        .collect();
    let lo = values.iter().copied().reduce(f64::min)?;
    let hi = values.iter().copied().reduce(f64::max)?;
    Some((lo, hi))
}

/// Запись для поля файла: «начало/конец», момент - одна дата.
pub fn pack(time: Option<&Time>) -> String {
    match time {
        Some((a, b)) if !(a.is_empty() && b.is_empty()) => {
            if a == b {
                a.clone()
            } else {
                format!("{a}/{b}")
            }
        }
        _ => String::new(),
    }
}

/// Обратно из поля файла: пара строк или None.
pub fn unpack(value: &str) -> Option<Time> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    Some(match value.split_once('/') {
        Some((a, b)) => (a.trim().to_string(), b.trim().to_string()),
        None => (value.to_string(), value.to_string()),
    })
}
