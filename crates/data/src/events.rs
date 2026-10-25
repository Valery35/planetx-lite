// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Снимки чрезвычайных событий Vantor и Planet: каталоги STAC, снимки
//! дня, рамки и уровни тайлов. Перенос `core/events.py` модуля без
//! отрисовки тайла (`paint`), её делает интерфейс.
//!
//! Оба каталога открыты, без ключа, лицензия снимков CC BY-NC 4.0.
//!
//! - Vantor Open Data (прежде Maxar) - каталог событий, у события
//!   коллекция, у снимка - полоса WorldView одним COG в EPSG:4326
//!   и признак phase («pre», «post»).
//! - Planet Disaster Data на Source Cooperative - каталог событий,
//!   внутри pre-event и post-event, коллекции по дням и местам, сцены
//!   PlanetScope, SkySat и Pelican в UTM, до события - квадраты мозаики
//!   Planet в EPSG:3857.
//!
//! Документы каталогов считаются враждебными: числа могут прийти целыми,
//! дробными или строками, любое поле может отсутствовать. Ошибка разбора
//! - None или пустой список, а не паника.

use serde_json::Value;

use crate::gibs::{Day, Interval};

pub const VANTOR: &str = "vantor";
pub const PLANET: &str = "planet";
/// Источник и адрес корневого каталога.
pub const ROOTS: [(&str, &str); 2] = [(VANTOR, "https://vantor-opendata.s3.amazonaws.com/events/catalog.json"), (PLANET, "https://data.source.coop/planet/disasterdata/catalog.json")];
/// Лицензия снимков обоих источников.
pub const LICENSE: &str = "https://creativecommons.org/licenses/by-nc/4.0/";
/// Этап снимка: до события и после.
pub const PRE: &str = "pre";
pub const POST: &str = "post";
/// Сторона тайла глобуса, пикселей.
const SIDE: f64 = 256.0;
/// Большая полуось WGS84, м.
const RADIUS: f64 = 6378137.0;
/// Самый подробный уровень тайлов.
pub const MAX_LEVEL: u32 = 19;
/// Вложенность каталогов события, ниже обход не идёт.
pub const MAX_DEPTH: usize = 6;
/// Начало названий событий Planet, оно снимается.
const PREFIX: &str = "Planet Crisis Response";

/// Название источника для пользователя.
pub fn source_name(source: &str) -> &'static str {
    match source {
        VANTOR => "Vantor",
        PLANET => "Planet",
        _ => "",
    }
}

/// Подпись источника и адрес его страницы.
pub fn credit(source: &str) -> (&'static str, &'static str) {
    match source {
        VANTOR => ("Vantor Open Data, CC BY-NC 4.0", "https://vantor.com/company/open-data-program/"),
        PLANET => ("Planet Labs PBC, CC BY-NC 4.0", "https://www.planet.com/disasterdata/"),
        _ => ("", ""),
    }
}

/// Событие каталога: ключ (источник/id), источник, адрес документа,
/// название, описание, день ГГГГ-ММ-ДД или пустая строка, рамка
/// (запад, юг, восток, север) или None до прихода снимков.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub key: String,
    pub source: &'static str,
    pub url: String,
    pub title: String,
    pub description: String,
    pub day: String,
    pub bbox: Option<[f64; 4]>,
}

/// Снимок события: id, источник, день ГГГГ-ММ-ДД, момент в секундах
/// UTC, этап (PRE, POST или пустая строка), рамка, адрес COG, размер
/// пикселя в метрах, название спутника или мозаики.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub ident: String,
    pub source: &'static str,
    pub day: String,
    pub time: f64,
    pub phase: String,
    pub bbox: [f64; 4],
    pub href: String,
    pub gsd: f64,
    pub platform: String,
}

/// Значение JSON истинно в смысле Python: не null, не false, не ноль,
/// не пустая строка, не пустой список или объект.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// Текст значения: строка как есть, число записью, ложное - пустая
/// строка.
fn text(v: Option<&Value>) -> String {
    if !truthy(v) {
        return String::new();
    }
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(_)) => "True".to_string(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

/// Число из числа JSON или строки с числом, только конечное.
fn number(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|x| x.is_finite())
}

/// Первые четыре числа списка - рамка.
fn bbox4(v: Option<&Value>) -> Option<[f64; 4]> {
    let list = v?.as_array()?;
    if list.len() < 4 {
        return None;
    }
    Some([number(list.first())?, number(list.get(1))?, number(list.get(2))?, number(list.get(3))?])
}

/// Первые count символов строки.
fn head(s: &str, count: usize) -> String {
    s.chars().take(count).collect()
}

/// Части адреса: схема с двоеточием, «//» и узел, путь, запрос с «?».
fn split_url(url: &str) -> (&str, &str, &str) {
    let (rest, query) = match url.find('?') {
        Some(at) => url.split_at(at),
        None => (url, ""),
    };
    let (prefix, path) = match rest.find("://") {
        Some(at) => {
            let after = at + 3;
            let tail = rest.get(after..).unwrap_or("");
            let end = tail.find('/').map_or(rest.len(), |p| after + p);
            (rest.get(..end).unwrap_or(rest), rest.get(end..).unwrap_or(""))
        }
        None => ("", rest),
    };
    (prefix, path, query)
}

/// Схема адреса href, если она есть: буквы, цифры, «+», «-», «.» до
/// двоеточия, первая - буква.
fn has_scheme(href: &str) -> bool {
    let Some(at) = href.find(':') else {
        return false;
    };
    let scheme = href.get(..at).unwrap_or("");
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Путь без сегментов «.» и «..», как remove_dot_segments RFC 3986.
/// Лишние «..» выше корня отбрасываются, как в urljoin Python.
fn remove_dots(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').collect();
    let last = segments.len().saturating_sub(1);
    let mut out: Vec<&str> = Vec::with_capacity(segments.len());
    for (i, seg) in segments.iter().enumerate() {
        match *seg {
            "." => {
                if i == last {
                    out.push("");
                }
            }
            ".." => {
                if out.len() > 1 || out.first().is_some_and(|s| !s.is_empty()) {
                    out.pop();
                }
                if i == last {
                    out.push("");
                }
            }
            s => out.push(s),
        }
    }
    let joined = out.join("/");
    if path.starts_with('/') && !joined.starts_with('/') { format!("/{joined}") } else { joined }
}

/// Адрес href относительно base, как urljoin Python для абсолютных
/// адресов, «//узел», «/путь», «?запрос», «./x», «../x» и «x».
/// Часть после «#» отбрасывается.
pub fn join_url(base: &str, href: &str) -> String {
    let cut = |s: &str| s.split('#').next().unwrap_or("").to_string();
    let (base, href) = (cut(base), cut(href));
    if href.is_empty() {
        return base;
    }
    if has_scheme(&href) {
        let (prefix, path, query) = split_url(&href);
        if prefix.is_empty() {
            return href;
        }
        return format!("{prefix}{}{query}", remove_dots(path));
    }
    let (prefix, path, _) = split_url(&base);
    if let Some(rest) = href.strip_prefix("//") {
        let scheme = prefix.find("://").and_then(|at| prefix.get(..at)).unwrap_or("");
        let full = format!("{scheme}://{rest}");
        let (p, hp, hq) = split_url(&full);
        return format!("{p}{}{hq}", remove_dots(hp));
    }
    if href.starts_with('?') {
        return format!("{prefix}{path}{href}");
    }
    let (href_path, href_query) = match href.find('?') {
        Some(at) => href.split_at(at),
        None => (href.as_str(), ""),
    };
    let merged = if href_path.starts_with('/') {
        href_path.to_string()
    } else {
        let dir = path.rfind('/').and_then(|at| path.get(..=at)).unwrap_or("");
        let dir = if dir.is_empty() && !prefix.is_empty() { "/" } else { dir };
        format!("{dir}{href_path}")
    };
    format!("{prefix}{}{href_query}", remove_dots(&merged))
}

/// Ссылки документа STAC: (rel, абсолютный адрес, название).
pub fn links(doc: &Value, base: &str) -> Vec<(String, String, String)> {
    let Some(list) = doc.get("links").and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|link| {
            let href = link.get("href").and_then(Value::as_str).filter(|h| !h.is_empty())?;
            let rel = link.get("rel").and_then(Value::as_str).unwrap_or("").to_string();
            let title = link.get("title").and_then(Value::as_str).unwrap_or("").to_string();
            Some((rel, join_url(base, href), title))
        })
        .collect()
}

/// Адреса дочерних каталогов и снимков документа: (rel, адрес).
pub fn children(doc: &Value, base: &str) -> Vec<(String, String)> {
    links(doc, base).into_iter().filter(|(rel, _, _)| rel == "child" || rel == "item").map(|(rel, href, _)| (rel, href)).collect()
}

/// Заготовки событий из корневого каталога: (адрес, название из
/// ссылки). Описание и день приходят с документом события.
pub fn parse_events(doc: &Value, base: &str) -> Vec<(String, String)> {
    links(doc, base).into_iter().filter(|(rel, _, _)| rel == "child").map(|(_, href, title)| (href, title)).collect()
}

/// Название без начала «Planet Crisis Response —» (тире, дефис или
/// двоеточие, пробелы вокруг).
fn strip_prefix(title: &str) -> &str {
    let Some(rest) = title.strip_prefix(PREFIX) else {
        return title;
    };
    let rest = rest.trim_start();
    match rest.strip_prefix(['—', '-', ':']) {
        Some(tail) => tail.trim_start(),
        None => title,
    }
}

/// Первая дата ГГГГ-ММ-ДД в строке.
fn find_date(s: &str) -> Option<&str> {
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| b.get(r).is_some_and(|p| p.iter().all(u8::is_ascii_digit));
    (0..b.len().saturating_sub(9)).find(|&i| digits(i..i + 4) && b.get(i + 4) == Some(&b'-') && digits(i + 5..i + 7) && b.get(i + 7) == Some(&b'-') && digits(i + 8..i + 10)).and_then(|i| s.get(i..i + 10))
}

/// Событие из документа события (коллекция Vantor или каталог Planet)
/// или None.
pub fn parse_event(doc: &Value, url: &str, source: &'static str) -> Option<Event> {
    let kind = doc.get("type").and_then(Value::as_str)?;
    if kind != "Catalog" && kind != "Collection" {
        return None;
    }
    let mut ident = text(doc.get("id"));
    if ident.is_empty() {
        ident = url.to_string();
    }
    let mut title = strip_prefix(text(doc.get("title")).trim()).to_string();
    if title.is_empty() || title == ident {
        title = ident.replace(['-', '_'], " ");
    }
    let description = text(doc.get("description")).trim().to_string();
    let extent = doc.get("extent");
    let first = |key: &str, inner: &str| extent.and_then(|e| e.get(key)).and_then(|t| t.get(inner)).and_then(Value::as_array).and_then(|a| a.first());
    let mut day = String::new();
    if let Some(interval) = first("temporal", "interval").and_then(Value::as_array)
        && truthy(interval.first())
    {
        day = head(&text(interval.first()), 10);
    }
    if day.is_empty()
        && let Some(found) = find_date(&ident)
    {
        day = found.to_string();
    }
    let bbox = bbox4(first("spatial", "bbox"));
    Some(Event { key: format!("{source}/{ident}"), source, url: url.to_string(), title, description, day, bbox })
}

/// Этап по пути документа Planet: pre-event, post-event.
pub fn phase_of(url: &str) -> &'static str {
    if url.contains("/pre-event/") {
        PRE
    } else if url.contains("/post-event/") {
        POST
    } else {
        ""
    }
}

/// Снимок из документа Feature STAC или None, если в нём нет картинки
/// visual в COG, момента или рамки.
pub fn parse_item(doc: &Value, url: &str, source: &'static str, phase: &str) -> Option<Item> {
    if doc.get("type").and_then(Value::as_str) != Some("Feature") {
        return None;
    }
    let asset = doc.get("assets")?.get("visual").filter(|a| truthy(Some(a)))?;
    if !text(asset.get("type")).contains("tiff") {
        return None;
    }
    let href = join_url(url, &text(asset.get("href")));
    let props = doc.get("properties");
    let prop = |key: &str| props.and_then(|p| p.get(key));
    let stamp = if truthy(prop("datetime")) { text(prop("datetime")) } else { text(prop("start_datetime")) };
    let time = moment(&stamp)?;
    let bbox = bbox4(doc.get("bbox"))?;
    let gsd = ["gsd", "pan_gsd", "pl:pixel_resolution"].into_iter().map(prop).find(|v| truthy(*v)).and_then(number).unwrap_or(0.0);
    let platform = if truthy(prop("pl:mosaic")) {
        "Planet basemap".to_string()
    } else if truthy(prop("vehicle_name")) {
        text(prop("vehicle_name"))
    } else {
        text(prop("constellation"))
    };
    let mut ident = text(doc.get("id"));
    if ident.is_empty() {
        ident = href.clone();
    }
    let mut item_phase = text(prop("phase"));
    if item_phase.is_empty() {
        item_phase = phase.to_string();
    }
    Some(Item { ident, source, day: head(&stamp, 10), time, phase: item_phase, bbox, href, gsd, platform })
}

/// Момент в секундах UTC из начала метки ГГГГ-ММ-ДД[Tчч:мм:сс]. Без
/// времени или с неполным временем - полночь. Неверная дата - None.
pub fn moment(stamp: &str) -> Option<f64> {
    let b = stamp.as_bytes();
    let digits = |r: std::ops::Range<usize>| b.get(r).is_some_and(|p| p.iter().all(u8::is_ascii_digit));
    let date = digits(0..4) && b.get(4) == Some(&b'-') && digits(5..7) && b.get(7) == Some(&b'-') && digits(8..10);
    if !date {
        return None;
    }
    let day = Day::parse(stamp.get(..10)?).filter(|d| d.year >= 1)?;
    let mut seconds = 0.0;
    if b.get(10) == Some(&b'T') && digits(11..13) && b.get(13) == Some(&b':') && digits(14..16) && b.get(16) == Some(&b':') && digits(17..19) {
        let part = |r: std::ops::Range<usize>| stamp.get(r).and_then(|s| s.parse::<u32>().ok());
        let (h, m, s) = (part(11..13)?, part(14..16)?, part(17..19)?);
        if h > 23 || m > 59 || s > 59 {
            return None;
        }
        seconds = f64::from(h * 3600 + m * 60 + s);
    }
    Some(day.seconds() + seconds)
}

/// Рамка всех снимков или None.
pub fn bbox_of(items: &[Item]) -> Option<[f64; 4]> {
    let first = items.first()?;
    Some(items.iter().fold(first.bbox, |a, i| [a[0].min(i.bbox[0]), a[1].min(i.bbox[1]), a[2].max(i.bbox[2]), a[3].max(i.bbox[3])]))
}

/// Дни снимков по возрастанию, без повторов.
pub fn days(items: &[Item]) -> Vec<String> {
    let mut out: Vec<String> = items.iter().map(|i| i.day.clone()).collect();
    out.sort();
    out.dedup();
    out
}

/// Ряд дат темы: каждый день снимков - отдельный промежуток, как
/// у выпусков Wayback. День, который не разбирается, пропускается.
pub fn intervals(items: &[Item]) -> Vec<Interval> {
    days(items).iter().filter_map(|d| Day::parse(d)).map(|d| (d, d, 1, 'D')).collect()
}

/// Последний день снимков до события или None.
pub fn pre_day(items: &[Item]) -> Option<String> {
    items.iter().filter(|i| i.phase == PRE).map(|i| i.day.clone()).max()
}

/// Последний день снимков события: после события и без этапа, если
/// таких нет - всех.
pub fn post_day(items: &[Item]) -> Option<String> {
    items.iter().filter(|i| i.phase != PRE).map(|i| i.day.clone()).max().or_else(|| items.iter().map(|i| i.day.clone()).max())
}

/// Снимки на день: не позже дня, новые первыми - они лягут сверху.
/// При равном моменте первым идёт снимок без размера пикселя, затем
/// более подробный. Порядок равных сохраняется, как у sorted Python.
pub fn shown<'a>(items: &'a [Item], day: &str) -> Vec<&'a Item> {
    let key = |i: &Item| if i.gsd == 0.0 { 0.0 } else { -i.gsd };
    let mut out: Vec<&Item> = items.iter().filter(|i| i.day.as_str() <= day).collect();
    out.sort_by(|a, b| b.time.total_cmp(&a.time).then(key(b).total_cmp(&key(a))));
    out
}

/// Уровень тайлов, пиксель которого не мельче самого подробного
/// снимка.
pub fn max_level(items: &[Item]) -> u32 {
    let best = items.iter().map(|i| i.gsd).filter(|g| *g > 0.0).fold(f64::INFINITY, f64::min);
    let Some(bbox) = bbox_of(items).filter(|_| best.is_finite()) else {
        return MAX_LEVEL;
    };
    let lat = (bbox[1] + bbox[3]) / 2.0;
    let ground = 2.0 * std::f64::consts::PI * RADIUS * lat.to_radians().cos().max(0.05) / SIDE;
    let level = (ground / best).log2().ceil();
    // Как min(MAX_LEVEL, max(0, level)) Python: NaN - уровень 0.
    let level = if level > 0.0 { level } else { 0.0 };
    level.min(f64::from(MAX_LEVEL)) as u32
}

/// Рамка тайла XYZ (запад, юг, восток, север) в градусах.
pub fn tile_box(z: u32, x: u32, y: u32) -> [f64; 4] {
    let n = 2f64.powi(i32::try_from(z).unwrap_or(i32::MAX));
    let lat = |row: f64| (std::f64::consts::PI * (1.0 - 2.0 * row / n)).sinh().atan().to_degrees();
    let (x, y) = (f64::from(x), f64::from(y));
    [x / n * 360.0 - 180.0, lat(y + 1.0), (x + 1.0) / n * 360.0 - 180.0, lat(y)]
}

/// Рамки пересекаются или касаются.
pub fn touches(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}
