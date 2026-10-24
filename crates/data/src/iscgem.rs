// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Исторические землетрясения - каталог ISC-GEM с 1904 года. Перенос
//! `core/iscgem.py` модуля.
//!
//! ISC-GEM Global Instrumental Earthquake Catalogue (International
//! Seismological Centre, GEM Foundation, CC BY-SA 3.0) - 74 153 сильных
//! события 1904-2024 годов на 17 октября 2026 года, 13 МБ CSV,
//! с пересчитанными местами и моментными магнитудами. Сайт ISC отдаёт
//! файл после анкеты, поэтому каталог берётся у ComCat USGS
//! (catalog=iscgem) запросами по промежуткам лет - не больше 20 тысяч
//! событий на запрос. Каталог собирается один раз в файл профиля
//! ([`save`], [`load`]), записи - [`crate::quakes::Quake`]. Файл - архив
//! .npz того же вида, что у модуля, файлы переносятся между ними.

use crate::gibs::Day;
use crate::npz;
use crate::quakes::Quake;

/// Запрос ComCat: поля `{start}` и `{end}` - даты ISO.
pub const QUERY: &str = "https://earthquake.usgs.gov/fdsnws/event/1/query?format=csv&catalog=iscgem&orderby=time-asc&starttime={start}&endtime={end}";
/// Страница события: поле `{id}`.
pub const EVENT: &str = "https://earthquake.usgs.gov/earthquakes/eventpage/{id}";
pub const ATTRIBUTION: (&str, &str) = ("Historical earthquakes: ISC-GEM, ISC and GEM Foundation, CC BY-SA 3.0", "https://doi.org/10.31905/d808b825");
pub const FIRST_YEAR: i32 = 1900;
/// До 1960 года событий около 8.6 тысячи, дальше - до 1.3 тысячи в год.
pub const SPLIT_YEAR: i32 = 1960;
pub const STEP_YEARS: i32 = 5;
/// Каталог обновляется раз в год, файл старше - заново.
pub const REFETCH_DAYS: u32 = 180;
/// Имя файла каталога в профиле.
pub const FILE: &str = "iscgem.npz";

/// Промежутки запросов (начало, конец) датами ISO до last_year
/// включительно: один до SPLIT_YEAR, дальше по STEP_YEARS лет.
pub fn periods(last_year: i32) -> Vec<(String, String)> {
    let mut out = vec![(FIRST_YEAR, SPLIT_YEAR)];
    let mut year = SPLIT_YEAR;
    while year <= last_year {
        out.push((year, (year + STEP_YEARS).min(last_year.saturating_add(1))));
        year += STEP_YEARS;
    }
    out.into_iter().map(|(a, b)| (format!("{a}-01-01"), format!("{b}-01-01"))).collect()
}

/// Адрес запроса событий от start до end.
pub fn query_url(start: &str, end: &str) -> String {
    QUERY.replace("{start}", start).replace("{end}", end)
}

/// Целое из десятичных цифр ASCII, от min_len до max_len знаков.
fn digits(text: &str, min_len: usize, max_len: usize) -> Option<u32> {
    let ok = (min_len..=max_len).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit());
    ok.then(|| text.parse().ok()).flatten()
}

/// Время ComCat 1906-01-02T04:26:27.250Z в секунды UTC или None, доли
/// секунды от 1 до 6 знаков или без них.
fn seconds(text: &str) -> Option<f64> {
    let (date, clock) = text.trim().strip_suffix('Z')?.split_once('T')?;
    let mut parts = date.split('-');
    let (year, month, day) = (digits(parts.next()?, 4, 4)?, digits(parts.next()?, 1, 2)?, digits(parts.next()?, 1, 2)?);
    if parts.next().is_some() {
        return None;
    }
    let day = Day::parse(&format!("{year:04}-{month:02}-{day:02}"))?;
    let (whole, fraction) = match clock.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (clock, None),
    };
    let mut parts = whole.split(':');
    let (h, m, s) = (digits(parts.next()?, 1, 2)?, digits(parts.next()?, 1, 2)?, digits(parts.next()?, 1, 2)?);
    if parts.next().is_some() || h > 23 || m > 59 || s > 59 {
        return None;
    }
    let part = match fraction {
        Some(f) => f64::from(digits(f, 1, 6)?) / 10f64.powi(f.len() as i32),
        None => 0.0,
    };
    Some(day.seconds() + f64::from(h * 3600 + m * 60 + s) + part)
}

/// События из ответа CSV ComCat. Строка без координат, глубины или
/// магнитуды пропускается. Неизвестная глубина считается нулевой. Ответ
/// считается враждебным.
pub fn parse(data: &[u8]) -> Vec<Quake> {
    let table = crate::csv::Table::parse(&String::from_utf8_lossy(data));
    let column = |name: &str| table.column(name);
    let (lat, lon, mag, depth) = (column("latitude"), column("longitude"), column("mag"), column("depth"));
    let (id, time, place) = (column("id"), column("time"), column("place"));
    table
        .rows
        .iter()
        .filter_map(|row| {
            let cell = |c: Option<usize>| c.and_then(|c| row.get(c));
            let text = |c: Option<usize>| cell(c).map(|t| t.trim().to_string()).unwrap_or_default();
            let (lat, lon, mag) = (crate::csv::number(cell(lat))?, crate::csv::number(cell(lon))?, crate::csv::number(cell(mag))?);
            let depth = crate::csv::number(cell(depth)).unwrap_or(0.0);
            let ident = text(id);
            let url = if ident.is_empty() { String::new() } else { EVENT.replace("{id}", &ident) };
            Some(Quake { lat, lon, depth, mag, time: cell(time).and_then(|t| seconds(t)), place: text(place), url })
        })
        .collect()
}

/// Каталог в байты файла .npz без объектов Python, как `save` модуля:
/// числа массивом `numbers` (N, 5), места и адреса одной строкой UTF-8
/// `text` с разделителями.
pub fn save(events: &[Quake]) -> Result<Vec<u8>, String> {
    let numbers: Vec<f64> = events.iter().flat_map(|q| [q.lat, q.lon, q.depth, q.mag, q.time.unwrap_or(f64::NAN)]).collect();
    let text: Vec<String> = events.iter().map(|q| format!("{}\t{}", q.place.replace('\n', " "), q.url)).collect();
    let text = text.join("\n");
    npz::write_npz(&[("numbers", npz::npy_f64(&[events.len(), 5], &numbers)), ("text", npz::npy("|u1", &[text.len()], text.as_bytes()))])
}

/// Каталог из байтов файла [`save`]. Файл считается враждебным: размеры
/// массивов сверяются, текст - только UTF-8.
pub fn load(data: &[u8]) -> Result<Vec<Quake>, String> {
    let arrays = npz::read_npz(data, &["numbers", "text"])?;
    let [numbers, text] = arrays.as_slice() else {
        return Err("bad iscgem file".into());
    };
    if numbers.shape.len() != 2 || numbers.shape[1] != 5 {
        return Err("bad iscgem numbers".into());
    }
    let bytes: Vec<u8> = text.values.iter().map(|v| *v as u8).collect();
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    let lines: Vec<&str> = if text.is_empty() { Vec::new() } else { text.split('\n').collect() };
    Ok(numbers
        .values
        .as_chunks::<5>()
        .0
        .iter()
        .zip(lines)
        .map(|(row, line)| {
            let (place, url) = line.split_once('\t').unwrap_or((line, ""));
            let time = (!row[4].is_nan()).then_some(row[4]);
            Quake { lat: row[0], lon: row[1], depth: row[2], mag: row[3], time, place: place.to_string(), url: url.to_string() }
        })
        .collect())
}
