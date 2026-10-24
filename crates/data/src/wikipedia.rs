// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Википедия рядом: статьи с координатами у точки взгляда. Перенос
//! `core/wikipedia.py` модуля.
//!
//! Статьи даёт служба geosearch API MediaWiki (list=geosearch) без ключа,
//! рамкой gsbbox. Рамку больше около 300 км² служба не принимает (ответ
//! toobig), поэтому запрос идёт по тайлам Web Mercator уровня [`ZOOM`] -
//! у экватора около 9.8 км, севернее меньше. Ответ - до [`LIMIT`] статей
//! тайла. Слой только земной, на Луне и Марсе служба статей не нашла.
//!
//! Первый абзац статьи даёт REST API страниц (page/summary). Тексты
//! Википедии - CC BY-SA 4.0, подпись - [`ATTRIBUTION`]. Слой виден только
//! вблизи, ниже [`MAX_ALTITUDE`].

use serde_json::Value;

use crate::mining::{tile_bounds, tile_of};

/// Уровень тайлов запроса.
pub const ZOOM: u32 = 12;
/// Тайлов вокруг точки взгляда, 5 × 5.
pub const VIEW_TILES: usize = 25;
/// Выше глаза статьи не просятся, метры.
pub const MAX_ALTITUDE: f64 = 40000.0;
/// Статей в ответе службы на тайл, её предел.
pub const LIMIT: u32 = 500;
/// Подписей на глобусе не больше, важные и ближние первыми.
pub const MAX_MARKS: usize = 2000;
/// Секунд между запросами к службе, по одному запросу сразу.
pub const GAP: f64 = 0.25;
pub const ATTRIBUTION: (&str, &str) = ("Wikipedia, CC BY-SA 4.0", "https://creativecommons.org/licenses/by-sa/4.0/");
/// Шаблоны адресов: {lang} - раздел, {title} - заголовок статьи.
pub const API: &str = "https://{lang}.wikipedia.org/w/api.php";
pub const SUMMARY: &str = "https://{lang}.wikipedia.org/api/rest_v1/page/summary/{title}";
pub const PAGE: &str = "https://{lang}.wikipedia.org/wiki/{title}";

/// Важность статьи по виду объекта (поле type разметки координат
/// Википедии): страны и области прежде городов, города прежде
/// достопримечательностей. Меньше - важнее. Выбор помощника модуля.
pub const TYPE_RANK: [(&str, u32); 16] = [
    ("country", 1),
    ("adm1st", 2),
    ("adm2nd", 3),
    ("city", 3),
    ("adm3rd", 4),
    ("mountain", 4),
    ("river", 4),
    ("waterbody", 4),
    ("isle", 4),
    ("glacier", 4),
    ("airport", 5),
    ("landmark", 5),
    ("railwaystation", 6),
    ("edu", 6),
    ("event", 6),
    ("forest", 6),
];
/// Тайлов от середины по каждой оси в [`view_tiles`] не больше.
const MAX_REACH: i64 = 50;

/// Важность статьи другого вида.
pub const OTHER_RANK: u32 = 7;

/// Статья с координатами: номер страницы, заголовок, широта и долгота
/// в градусах, важность (меньше - важнее).
#[derive(Debug, Clone, PartialEq)]
pub struct Article {
    pub pageid: i64,
    pub title: String,
    pub lat: f64,
    pub lon: f64,
    pub rank: u32,
}

/// Раздел Википедии по коду языка интерфейса: две или три латинские
/// буквы, иначе английский.
pub fn language(code: &str) -> String {
    let code = code.to_lowercase();
    if (2..=3).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_lowercase()) { code } else { "en".to_string() }
}

/// Тайлы уровня [`ZOOM`] вокруг точки взгляда, ближние первыми, не
/// больше limit. Выше [`MAX_ALTITUDE`] - ни одного.
pub fn view_tiles(lat: f64, lon: f64, altitude: f64, limit: usize) -> Vec<(u32, u32)> {
    if altitude.is_nan() || altitude > MAX_ALTITUDE {
        return Vec::new();
    }
    // Сторона квадрата не больше 2 · MAX_REACH + 1 тайлов при любом limit.
    let reach = (((limit as f64).sqrt() - 1.0) / 2.0).ceil().clamp(0.0, MAX_REACH as f64) as i64;
    let (cx, cy) = tile_of(lat, lon, ZOOM);
    let (cx, cy) = (i64::from(cx), i64::from(cy));
    let n = 1i64 << ZOOM;
    let mut tiles: Vec<(i64, i64)> = Vec::new();
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            if (0..n).contains(&(cy + dy)) {
                tiles.push(((cx + dx).rem_euclid(n), cy + dy));
            }
        }
    }
    // Расстояние - по номеру после замыкания долготы, как у модуля.
    tiles.sort_by_key(|t| (t.0 - cx).pow(2) + (t.1 - cy).pow(2));
    tiles.into_iter().take(limit).map(|(x, y)| (x as u32, y as u32)).collect()
}

/// Адрес запроса статей в рамке тайла (x, y) уровня [`ZOOM`].
pub fn geosearch_url(lang: &str, tile: (u32, u32)) -> String {
    let (south, west, north, east) = tile_bounds(tile.0, tile.1, ZOOM);
    format!(
        "{}?action=query&list=geosearch&format=json&gsbbox={north:.6}|{west:.6}|{south:.6}|{east:.6}&gslimit={LIMIT}&gsprop=type|dim",
        API.replace("{lang}", &language(lang))
    )
}

/// Число из числа или строки JSON, конечное.
fn number(value: Option<&Value>) -> Option<f64> {
    let v = match value? {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    v.is_finite().then_some(v)
}

/// Целое из числа или строки JSON, дробь отбрасывается.
fn integer(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().filter(|v| v.is_finite()).map(|v| v.trunc() as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Значение JSON строкой для текста ошибки.
fn text(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// Статьи из ответа geosearch байтами JSON. Ответ не JSON или с ошибкой
/// службы - Err с её кодом и пояснением.
pub fn parse_geosearch(data: &[u8]) -> Result<Vec<Article>, String> {
    let json: Value = serde_json::from_slice(data).map_err(|e| e.to_string())?;
    parse_geosearch_json(&json)
}

/// Статьи из уже разобранного ответа geosearch, как [`parse_geosearch`].
pub fn parse_geosearch_json(data: &Value) -> Result<Vec<Article>, String> {
    if !data.is_object() {
        return Err("not a geosearch answer".to_string());
    }
    if let Some(error) = data.get("error") {
        return Err(format!("{}: {}", text(error.get("code")), text(error.get("info"))));
    }
    let items = data.pointer("/query/geosearch").and_then(Value::as_array);
    Ok(items.map(|items| items.iter().filter_map(article).collect()).unwrap_or_default())
}

/// Статья из записи ответа или None, если в записи нет точки, номера
/// или заголовка.
fn article(item: &Value) -> Option<Article> {
    let title = item.get("title")?.as_str()?.trim();
    let lat = number(item.get("lat"))?;
    let lon = number(item.get("lon"))?;
    let pageid = integer(item.get("pageid"))?;
    if title.is_empty() || !(-90.0..=90.0).contains(&lat) {
        return None;
    }
    let kind = item.get("type").and_then(Value::as_str);
    let rank = TYPE_RANK.iter().find(|(k, _)| Some(*k) == kind).map_or(OTHER_RANK, |(_, r)| *r);
    Some(Article { pageid, title: title.to_string(), lat, lon, rank })
}

/// Заголовок в части пути адреса: пробелы - «_», прочее вне букв, цифр
/// и «_.-~» - %XX по байтам UTF-8, как `quote(safe="")` модуля.
fn path(title: &str) -> String {
    let mut out = String::with_capacity(title.len() * 3);
    for byte in title.replace(' ', "_").bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => out.push(char::from(byte)),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Адрес первого абзаца статьи.
pub fn summary_url(lang: &str, title: &str) -> String {
    SUMMARY.replace("{lang}", &language(lang)).replace("{title}", &path(title))
}

/// Адрес статьи для браузера.
pub fn page_url(lang: &str, title: &str) -> String {
    PAGE.replace("{lang}", &language(lang)).replace("{title}", &path(title))
}

/// Пробельный знак, как `\s` и `str.strip` модуля.
fn space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Первый абзац статьи текстом из ответа page/summary байтами JSON.
/// Ответ без текста - Err с заголовком или видом ответа.
pub fn parse_summary(data: &[u8]) -> Result<String, String> {
    let json: Value = serde_json::from_slice(data).map_err(|e| e.to_string())?;
    parse_summary_json(&json)
}

/// Первый абзац из уже разобранного ответа, как [`parse_summary`].
pub fn parse_summary_json(data: &Value) -> Result<String, String> {
    if !data.is_object() {
        return Err("not a summary answer".to_string());
    }
    // Википедия снимает из абзаца ударения и пояснения в скобках, на их
    // месте остаются двойные пробелы. Переводы строк остаются.
    let extract = data.get("extract").and_then(Value::as_str).unwrap_or("");
    let mut out = String::with_capacity(extract.len());
    let mut run = String::new();
    for c in extract.chars() {
        if space(c) && c != '\n' {
            run.push(c);
            continue;
        }
        flush(&mut out, &mut run);
        out.push(c);
    }
    flush(&mut out, &mut run);
    let out = out.trim_matches(space).to_string();
    if out.is_empty() {
        let name = [data.get("title"), data.get("type")].into_iter().flatten().filter_map(Value::as_str).find(|s| !s.is_empty());
        return Err(name.unwrap_or("no extract").to_string());
    }
    Ok(out)
}

/// Пробелы подряд: два и больше - один пробел, один - как есть.
fn flush(out: &mut String, run: &mut String) {
    if run.chars().count() >= 2 {
        out.push(' ');
    } else {
        out.push_str(run);
    }
    run.clear();
}

/// Статьи без повторов в порядке подписей: важные первыми, среди равных -
/// ближние к точке взгляда. Не больше limit. Из повторов одной страницы
/// остаётся первая.
pub fn order(articles: &[Article], lat: f64, lon: f64, limit: usize) -> Vec<Article> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<Article> = articles.iter().filter(|a| seen.insert(a.pageid)).cloned().collect();
    let cos = lat.to_radians().cos().max(1e-6);
    let gap = |a: &Article| {
        let dlon = (a.lon - lon + 180.0).rem_euclid(360.0) - 180.0;
        (a.lat - lat).powi(2) + (dlon * cos).powi(2)
    };
    out.sort_by(|a, b| a.rank.cmp(&b.rank).then(gap(a).total_cmp(&gap(b))));
    out.truncate(limit);
    out
}
