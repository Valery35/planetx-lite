// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Поезда Финляндии вживую по данным Digitraffic. Перенос `core/trains.py`
//! модуля.
//!
//! Положения поездов - открытый ответ Digitraffic (Fintraffic)
//! train-locations/latest: точка GPS, время и скорость каждого поезда,
//! раз в 10 с. Вид и категория поезда - запрос GraphQL
//! currentlyRunningTrains ([`types_body`], [`GRAPHQL_HEADERS`]). Условия -
//! CC BY 4.0, заголовок Digitraffic-User без личных данных, сжатие gzip
//! обязательно.
//!
//! Между обновлениями поезд едет со своей скоростью по своему курсу не
//! дольше [`EXTRAPOLATE_MAX`] секунд. Вблизи точка GPS притягивается
//! к рельсам OSM из векторной основы ([`Rails`]), и поезд едет по линии
//! рельсов, через стрелки - в ту ветку, что ближе по направлению. Пришло
//! новое положение - поезд переходит к нему за [`BLEND`] секунд, без
//! скачка.
//!
//! Модель - условный состав в цвет категории: вагонов по категории
//! ([`cars`]), каждый вагон стоит на рельсах на своём отступе назад от
//! головы ([`Fleet::place`], back). Длина вагона - вдоль оси Y модели
//! ([`train_parts`]), поэтому азимут модели - курс.
//!
//! Время - секунды UTC (clock) и секунды монотонных часов окна (now),
//! их передаёт вызывающий: в ядре часов нет.

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

use crate::routing::{to_grid, to_latlon, unit_metres};

pub const LATEST_URL: &str = "https://rata.digitraffic.fi/api/v1/train-locations/latest/";
pub const GRAPHQL_URL: &str = "https://rata.digitraffic.fi/api/v2/graphql/graphql";
/// Сведения одного поезда: {date} - дата отправления, {number} - номер.
/// Прибывшие поезда GraphQL не отдаёт, а положения GPS ещё идут.
pub const TRAIN_URL: &str = "https://rata.digitraffic.fi/api/v1/trains/{date}/{number}";
/// Поездов без вида спрашивается за опрос, не больше.
pub const TYPES_EACH: usize = 8;
pub const TYPES_QUERY: &str = "{ currentlyRunningTrains { trainNumber departureDate commuterLineid trainType { name trainCategory { name } } operator { shortCode } } }";
/// Digitraffic-User - программа, без личных данных, условие Digitraffic.
pub const USER: &str = "PlanetX";
/// Заголовки запросов Digitraffic, `HEADERS` окна модуля.
pub const HEADERS: [(&str, &str); 1] = [("Digitraffic-User", USER)];
/// Заголовки запроса GraphQL.
pub const GRAPHQL_HEADERS: [(&str, &str); 2] = [("Digitraffic-User", USER), ("Content-Type", "application/json")];
pub const ATTRIBUTION: (&str, &str) = ("Trains: Fintraffic / digitraffic.fi, CC BY 4.0", "https://www.digitraffic.fi/en/railway-traffic/");
pub const PAGE: &str = "https://www.digitraffic.fi/en/railway-traffic/";
/// Секунд между запросами положений, с таким шагом их обновляет служба.
pub const POLL: f64 = 10.0;
/// Секунд между запросами видов поездов.
pub const TYPES_POLL: f64 = 300.0;
/// Ход по своей скорости после последнего положения - не дольше, секунды.
pub const EXTRAPOLATE_MAX: f64 = 30.0;
/// Секунд перехода к новому положению.
pub const BLEND: f64 = 2.0;
/// Сдвиг между положениями, по которому берётся курс, метры.
pub const MOVED: f64 = 3.0;
/// Точка GPS притягивается к рельсам не дальше, метры.
pub const SNAP_REACH: f64 = 40.0;
/// Ветка за стрелкой годится, если её направление отличается от хода не
/// больше чем на это значение косинуса (около 60°).
pub const BRANCH_COS: f64 = 0.5;
/// Положение старше этого времени - поезд не показывается, секунды.
pub const STALE: f64 = 600.0;
/// Цвета категорий 0-1 и порядок шкалы - выбор помощника модуля.
pub const CATEGORIES: [(&str, [f64; 3]); 6] = [
    ("Long-distance", [0.82, 0.18, 0.18]),
    ("Commuter", [0.56, 0.24, 0.70]),
    ("Cargo", [0.20, 0.45, 0.78]),
    ("Locomotive", [0.96, 0.62, 0.12]),
    ("Shunting", [0.55, 0.55, 0.55]),
    ("On-track machines", [0.95, 0.85, 0.20]),
];
pub const OTHER: [f64; 3] = [0.85, 0.85, 0.85];
pub const DARK: [f64; 3] = [0.16, 0.17, 0.19];
pub const GLASS: [f64; 3] = [0.10, 0.14, 0.20];
/// Условный вагон, метры: длина, ширина, высота. Выбор помощника модуля.
pub const CAR_LENGTH: f64 = 25.0;
pub const CAR_WIDTH: f64 = 3.2;
pub const CAR_HEIGHT: f64 = 4.2;
/// Метров между началами соседних вагонов.
pub const CAR_STEP: f64 = 26.0;
/// Вагонов в составе по категории - типичная длина. Выбор помощника
/// модуля.
pub const CARS: [(&str, usize); 6] =
    [("Long-distance", 6), ("Commuter", 4), ("Cargo", 12), ("Locomotive", 1), ("Shunting", 1), ("On-track machines", 1)];
pub const CARS_OTHER: usize = 3;

/// Большая полуось WGS84, метры: радиус шара хода и расстояний.
const A: f64 = 6378137.0;
/// Шагов через стрелки за один ход не больше.
const WALK_STEPS: usize = 32;

/// Ключ поезда: номер и дата отправления.
pub type TrainKey = (i64, String);
/// Точка или вектор мировой сетки `routing`.
pub type GridPoint = (f64, f64);

/// Часть условной модели одного цвета: треугольники вершинами подряд,
/// метры, и цвет RGBA 0-1. Нормалей, текстуры и картинки нет, как
/// у `models.Part` модуля из этих функций.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Part {
    pub positions: Vec<[f64; 3]>,
    pub rgba: [f64; 4],
}

/// Вагонов в составе категории.
pub fn cars(category: &str) -> usize {
    CARS.iter().find(|(name, _)| *name == category).map_or(CARS_OTHER, |c| c.1)
}

/// Цвет категории 0-1, неизвестной - [`OTHER`].
pub fn category_color(category: &str) -> [f64; 3] {
    CATEGORIES.iter().find(|(name, _)| *name == category).map_or(OTHER, |c| c.1)
}

/// Дней от 1970-01-01 до даты григорианского календаря.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Время ISO 8601 вида 2026-10-18T17:31:33.000Z в секунды UTC от
/// 1970 года или None. Знаки после 19-го, кроме дробной части секунд,
/// не читаются, как у модуля.
pub fn epoch(text: &str) -> Option<f64> {
    let head = text.as_bytes().get(..19)?;
    let field = |from: usize, to: usize| -> Option<i64> {
        let part = head.get(from..to)?;
        part.iter().all(u8::is_ascii_digit).then(|| part.iter().fold(0, |v, d| v * 10 + i64::from(d - b'0')))
    };
    let marks = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')];
    if marks.iter().any(|&(at, mark)| head.get(at) != Some(&mark)) {
        return None;
    }
    let (year, month, day) = (field(0, 4)?, field(5, 7)?, field(8, 10)?);
    let (hour, minute, second) = (field(11, 13)?, field(14, 16)?, field(17, 19)?);
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let days_in = usize::try_from(month - 1).ok().and_then(|m| month_days.get(m)).copied()?;
    if year < 1 || !(1..=days_in).contains(&day) || hour > 23 || minute > 59 || second > 61 {
        return None;
    }
    let mut seconds = (days_from_civil(year, month, day) * 86400 + hour * 3600 + minute * 60 + second) as f64;
    if let Some(rest) = text.get(19..).and_then(|r| r.strip_prefix('.')) {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() {
            seconds += format!("0.{digits}").parse::<f64>().unwrap_or(0.0);
        }
    }
    Some(seconds)
}

/// Конечное число из числа или строки JSON. Логическое значение - не
/// число.
pub(crate) fn number(value: Option<&Value>) -> Option<f64> {
    let v = match value? {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse().ok()?,
        _ => return None,
    };
    v.is_finite().then_some(v)
}

/// Целое из числа или строки JSON, дробь отбрасывается.
pub(crate) fn integer(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().filter(|v| v.is_finite()).map(|v| v.trunc() as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Строка JSON без пробелов по краям, иначе пусто.
pub(crate) fn string(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).map(str::trim).unwrap_or("").to_string()
}

/// Значение JSON строкой, как `str()` модуля: нет значения - пусто,
/// null - «None».
fn py_str(value: Option<&Value>) -> String {
    match value {
        None => String::new(),
        Some(Value::Null) => "None".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => if *b { "True" } else { "False" }.to_string(),
        Some(other) => other.to_string(),
    }
}

/// Ключ поезда записи: номер и дата отправления.
fn train_key(item: &Value) -> Option<TrainKey> {
    Some((integer(item.get("trainNumber"))?, py_str(item.get("departureDate"))))
}

/// Положение поезда: ключ, время в секундах UTC, точка, скорость в м/с.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub key: TrainKey,
    pub t: f64,
    pub lat: f64,
    pub lon: f64,
    pub speed: f64,
}

/// Положения из ответа train-locations/latest байтами JSON. Записи без
/// точки или времени пропускаются, ответ не JSON - пустой список.
pub fn parse_latest(data: &[u8]) -> Vec<Sample> {
    serde_json::from_slice::<Value>(data).map(|json| parse_latest_json(&json)).unwrap_or_default()
}

/// Положения из уже разобранного ответа, как [`parse_latest`].
pub fn parse_latest_json(data: &Value) -> Vec<Sample> {
    let Some(items) = data.as_array() else { return Vec::new() };
    items.iter().filter_map(latest).collect()
}

fn latest(item: &Value) -> Option<Sample> {
    let coords = item.pointer("/location/coordinates")?.as_array()?;
    let t = epoch(item.get("timestamp")?.as_str()?)?;
    let key = train_key(item)?;
    let lon = number(coords.first())?;
    let lat = number(coords.get(1))?;
    let speed = match item.get("speed") {
        None | Some(Value::Null) => 0.0,
        speed => number(speed)?,
    };
    (-90.0..=90.0).contains(&lat).then(|| Sample { key, t, lat, lon, speed: (speed / 3.6).max(0.0) })
}

/// Сведения о поезде: вид (IC, S, HL…), категория, линия пригородного
/// поезда, оператор.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Info {
    pub kind: String,
    pub category: String,
    pub line: String,
    pub operator: String,
}

/// Строка JSON как есть, иначе пусто.
fn raw(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).unwrap_or("").to_string()
}

/// Тело запроса GraphQL видов поездов, как `json.dumps` модуля.
pub fn types_body() -> String {
    format!("{{\"query\": {}}}", Value::String(TYPES_QUERY.to_string()))
}

/// Сведения из ответа GraphQL currentlyRunningTrains байтами JSON.
pub fn parse_types(data: &[u8]) -> BTreeMap<TrainKey, Info> {
    serde_json::from_slice::<Value>(data).map(|json| parse_types_json(&json)).unwrap_or_default()
}

/// Сведения из уже разобранного ответа GraphQL, как [`parse_types`].
pub fn parse_types_json(data: &Value) -> BTreeMap<TrainKey, Info> {
    let mut out = BTreeMap::new();
    let Some(trains) = data.pointer("/data/currentlyRunningTrains").and_then(Value::as_array) else {
        return out;
    };
    for item in trains {
        let Some(key) = train_key(item) else { continue };
        let kind = item.get("trainType");
        let info = Info {
            kind: raw(kind.and_then(|k| k.get("name"))),
            category: raw(kind.and_then(|k| k.pointer("/trainCategory/name"))),
            line: raw(item.get("commuterLineid")),
            operator: raw(item.pointer("/operator/shortCode")),
        };
        out.insert(key, info);
    }
    out
}

/// Адрес сведений одного поезда.
pub fn train_url(key: &TrainKey) -> String {
    TRAIN_URL.replace("{date}", &key.1).replace("{number}", &key.0.to_string())
}

/// Сведения из ответа trains/{дата}/{номер} байтами JSON.
pub fn parse_train(data: &[u8]) -> BTreeMap<TrainKey, Info> {
    serde_json::from_slice::<Value>(data).map(|json| parse_train_json(&json)).unwrap_or_default()
}

/// Сведения из уже разобранного ответа одного поезда, как [`parse_train`].
pub fn parse_train_json(data: &Value) -> BTreeMap<TrainKey, Info> {
    let mut out = BTreeMap::new();
    for item in data.as_array().into_iter().flatten() {
        let Some(key) = train_key(item) else { continue };
        let info = Info {
            kind: raw(item.get("trainType")),
            category: raw(item.get("trainCategory")),
            line: raw(item.get("commuterLineID")),
            operator: raw(item.get("operatorShortCode")),
        };
        out.insert(key, info);
    }
    out
}

/// Подпись поезда: «IC 9», у пригородного «R 9123».
pub fn label(key: &TrainKey, info: Option<&Info>) -> String {
    let number = key.0;
    match info {
        Some(info) if !info.line.is_empty() => format!("{} {number}", info.line),
        Some(info) if !info.kind.is_empty() => format!("{} {number}", info.kind),
        _ => number.to_string(),
    }
}

/// Точка на шаре на расстоянии metres по азимуту heading, градусы.
pub fn destination(lat: f64, lon: f64, heading: f64, metres: f64) -> (f64, f64) {
    let d = metres / A;
    let (p1, l1, h) = (lat.to_radians(), lon.to_radians(), heading.to_radians());
    let p2 = (p1.sin() * d.cos() + p1.cos() * d.sin() * h.cos()).asin();
    let l2 = l1 + (h.sin() * d.sin() * p1.cos()).atan2(d.cos() - p1.sin() * p2.sin());
    (p2.to_degrees(), (l2.to_degrees() + 540.0).rem_euclid(360.0) - 180.0)
}

/// Расстояние по шару радиуса большой полуоси, метры, `_metres` модуля.
pub fn metres(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dl = (lon2 - lon1).to_radians();
    let a = ((p2 - p1) / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * A * a.sqrt().min(1.0).asin()
}

/// Начальный азимут дуги большого круга, градусы от севера, `bearing`
/// из `core/tour.py` модуля.
pub fn bearing(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dl = (lon2 - lon1).to_radians();
    let x = dl.sin() * p2.cos();
    let y = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dl.cos();
    x.atan2(y).to_degrees().rem_euclid(360.0)
}

// Рельсы.

/// Место на рельсах: линия, расстояние от её начала в единицах сетки,
/// направление хода по линии ±1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RailPos {
    pub line: usize,
    pub s: f64,
    pub direction: i8,
}

/// Линии рельсов в единицах мировой сетки `routing`. Стрелки - общие
/// вершины линий: у соседних тайлов уровня 14 вершины совпадают точно.
#[derive(Debug, Clone, Default)]
pub struct Rails {
    lines: Vec<Vec<GridPoint>>,
    /// Длина от начала линии до каждой вершины.
    lengths: Vec<Vec<f64>>,
    /// Вершина, округлённая до целых единиц, - линии и номера вершин.
    vertices: HashMap<(i64, i64), Vec<(usize, usize)>>,
}

/// Округление половин к чётному, как `round` модуля.
fn round_key(p: GridPoint) -> (i64, i64) {
    (p.0.round_ties_even() as i64, p.1.round_ties_even() as i64)
}

fn sub(a: GridPoint, b: GridPoint) -> GridPoint {
    (a.0 - b.0, a.1 - b.1)
}

impl Rails {
    /// Рельсы из линий вершин. Линии короче двух вершин и вершины
    /// с бесконечными координатами не берутся.
    pub fn new(lines: Vec<Vec<GridPoint>>) -> Self {
        let mut rails = Self::default();
        for points in lines {
            let points: Vec<GridPoint> = points.into_iter().filter(|p| p.0.is_finite() && p.1.is_finite()).collect();
            if points.len() < 2 {
                continue;
            }
            let n = rails.lines.len();
            let mut lengths = Vec::with_capacity(points.len());
            let mut total = 0.0;
            for (k, &p) in points.iter().enumerate() {
                if let Some(&prev) = k.checked_sub(1).and_then(|j| points.get(j)) {
                    let d = sub(p, prev);
                    total += d.0.hypot(d.1);
                }
                lengths.push(total);
                rails.vertices.entry(round_key(p)).or_default().push((n, k));
            }
            rails.lines.push(points);
            rails.lengths.push(lengths);
        }
        rails
    }

    /// Количество линий.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Вершины линии или None.
    pub fn line(&self, line: usize) -> Option<&[GridPoint]> {
        self.lines.get(line).map(Vec::as_slice)
    }

    /// Длина линии в единицах сетки или None.
    pub fn length(&self, line: usize) -> Option<f64> {
        self.lengths.get(line)?.last().copied()
    }

    /// Ближайшее место на рельсах к точке сетки (x, y) не дальше reach
    /// единиц: (номер линии, s) или None.
    pub fn snap(&self, x: f64, y: f64, reach: f64) -> Option<(usize, f64)> {
        let p = (x, y);
        let mut best: Option<(f64, usize, usize, f64)> = None;
        for (n, points) in self.lines.iter().enumerate() {
            for (seg, pair) in points.windows(2).enumerate() {
                let [a, b] = [pair[0], pair[1]];
                let ab = sub(b, a);
                let length2 = (ab.0 * ab.0 + ab.1 * ab.1).max(1e-12);
                let ap = sub(p, a);
                let t = ((ap.0 * ab.0 + ap.1 * ab.1) / length2).clamp(0.0, 1.0);
                let gap = (a.0 + t * ab.0 - p.0).hypot(a.1 + t * ab.1 - p.1);
                if best.is_none_or(|b| gap < b.0) {
                    best = Some((gap, n, seg, t));
                }
            }
        }
        let (gap, line, seg, t) = best?;
        if gap.is_nan() || gap > reach {
            return None;
        }
        let lengths = self.lengths.get(line)?;
        let (s0, s1) = (*lengths.get(seg)?, *lengths.get(seg + 1)?);
        Some((line, s0 + t * (s1 - s0)))
    }

    /// Точка сетки и единичная касательная по ходу линии в месте s или
    /// None, если такой линии нет.
    pub fn point(&self, line: usize, s: f64) -> Option<(GridPoint, GridPoint)> {
        let points = self.lines.get(line)?;
        let lengths = self.lengths.get(line)?;
        let total = *lengths.last()?;
        let s = s.max(0.0).min(total);
        let k = lengths.partition_point(|&v| v <= s).saturating_sub(1).min(points.len().saturating_sub(2));
        let (a, b) = (*points.get(k)?, *points.get(k + 1)?);
        let span = lengths.get(k + 1)? - lengths.get(k)?;
        let f = if span > 0.0 { (s - lengths.get(k)?) / span } else { 0.0 };
        let d = sub(b, a);
        let norm = d.0.hypot(d.1);
        let tangent = if norm > 0.0 { (d.0 / norm, d.1 / norm) } else { (0.0, -1.0) };
        Some(((a.0 + f * d.0, a.1 + f * d.1), tangent))
    }

    /// Новое место после units единиц сетки хода от pos. В конце линии
    /// ход идёт на линию с общей вершиной, направление которой ближе
    /// к ходу. Нет такой - поезд стоит в конце. Место на чужой линии не
    /// меняется.
    pub fn walk(&self, pos: RailPos, units: f64) -> RailPos {
        let RailPos { mut line, mut s, mut direction } = pos;
        let mut left = units.max(0.0);
        for _ in 0..WALK_STEPS {
            let Some(length) = self.length(line) else { return RailPos { line, s, direction } };
            let target = s + f64::from(direction) * left;
            if (0.0..=length).contains(&target) {
                return RailPos { line, s: target, direction };
            }
            let end = if direction > 0 { length } else { 0.0 };
            left -= (end - s).abs();
            let Some((_, tangent)) = self.point(line, end) else { return RailPos { line, s, direction } };
            let sign = f64::from(direction);
            match self.branch(line, end, (tangent.0 * sign, tangent.1 * sign)) {
                Some(next) => (line, s, direction) = next,
                None => return RailPos { line, s: end, direction },
            }
        }
        RailPos { line, s, direction }
    }

    /// [`Rails::walk`] с ходом обоих знаков: меньше нуля - назад от pos,
    /// по направлению pos. `move` модуля.
    pub fn advance(&self, pos: RailPos, units: f64) -> RailPos {
        if units >= 0.0 {
            return self.walk(pos, units);
        }
        let back = self.walk(RailPos { direction: -pos.direction, ..pos }, -units);
        RailPos { direction: -back.direction, ..back }
    }

    /// Продолжение хода в конце линии: (линия, s, направление) или None.
    fn branch(&self, line: usize, s: f64, heading: GridPoint) -> Option<(usize, f64, i8)> {
        let (here, _) = self.point(line, s)?;
        let mut best = None;
        let mut best_cos = BRANCH_COS;
        for &(other, k) in self.vertices.get(&round_key(here))? {
            if other == line {
                continue;
            }
            let (Some(points), Some(lengths)) = (self.lines.get(other), self.lengths.get(other)) else { continue };
            let (Some(&at), Some(&s_other)) = (points.get(k), lengths.get(k)) else { continue };
            for (direction, neighbour) in [(1i8, k.checked_add(1)), (-1i8, k.checked_sub(1))] {
                let Some(&next) = neighbour.and_then(|j| points.get(j)) else { continue };
                let step = sub(next, at);
                let norm = step.0.hypot(step.1);
                if norm <= 0.0 {
                    continue;
                }
                let cos = (step.0 * heading.0 + step.1 * heading.1) / norm;
                if cos > best_cos {
                    best = Some((other, s_other, direction));
                    best_cos = cos;
                }
            }
        }
        best
    }
}

/// Азимут по касательной в единицах сетки: x - восток, y - юг.
pub fn grid_heading(tangent: GridPoint) -> f64 {
    tangent.0.atan2(-tangent.1).to_degrees().rem_euclid(360.0)
}

// Поезда.

/// Поезд: последнее положение, курс (NaN - ещё неизвестен), место на
/// рельсах и переход.
#[derive(Debug, Clone)]
pub struct Train {
    pub key: TrainKey,
    pub sample: Sample,
    pub heading: f64,
    /// Место последнего положения на рельсах и версия рельсов, по которым
    /// оно найдено.
    pub rail: Option<RailPos>,
    pub rail_version: Option<u64>,
    /// (широта, долгота, азимут) на экране в начале перехода или None.
    pub blend_from: Option<(f64, f64, f64)>,
    pub blend_at: f64,
}

impl Train {
    pub fn new(sample: Sample) -> Self {
        Self { key: sample.key.clone(), sample, heading: f64::NAN, rail: None, rail_version: None, blend_from: None, blend_at: 0.0 }
    }

    /// Новое положение. shown - (широта, долгота, азимут) на экране
    /// в момент прихода, от него идёт переход. Положение не новее
    /// прежнего не берётся.
    pub fn update(&mut self, sample: Sample, shown: (f64, f64, f64), now: f64) {
        if sample.t <= self.sample.t {
            return;
        }
        let old = &self.sample;
        if metres(old.lat, old.lon, sample.lat, sample.lon) >= MOVED {
            self.heading = bearing(old.lat, old.lon, sample.lat, sample.lon);
        }
        self.sample = sample;
        self.rail = None;
        self.rail_version = None;
        self.blend_from = Some(shown);
        self.blend_at = now;
    }
}

/// Поезда на глобусе: положения, сведения, рельсы, точка и курс на
/// момент.
#[derive(Debug, Clone, Default)]
pub struct Fleet {
    pub trains: BTreeMap<TrainKey, Train>,
    pub info: BTreeMap<TrainKey, Info>,
    pub rails: Option<Rails>,
    pub rails_version: u64,
}

/// Место последнего положения на рельсах или None.
fn rail_of(rails: Option<&Rails>, version: u64, train: &mut Train) -> Option<RailPos> {
    let rails = rails?;
    if train.rail_version == Some(version) {
        return train.rail;
    }
    train.rail_version = Some(version);
    train.rail = None;
    let sample = &train.sample;
    let (x, y) = to_grid(sample.lat, sample.lon);
    let (line, s) = rails.snap(x, y, SNAP_REACH / unit_metres(sample.lat))?;
    let (_, tangent) = rails.point(line, s)?;
    let mut direction = 1;
    if !train.heading.is_nan() {
        let turn = (train.heading - grid_heading(tangent) + 180.0).rem_euclid(360.0) - 180.0;
        direction = if turn.abs() <= 90.0 { 1 } else { -1 };
    }
    train.rail = Some(RailPos { line, s, direction });
    train.rail
}

/// Точка и азимут на back метров позади головы без перехода.
fn ahead(rails: Option<&Rails>, version: u64, train: &mut Train, clock: f64, back: f64) -> (f64, f64, f64) {
    let elapsed = (clock - train.sample.t).clamp(0.0, EXTRAPOLATE_MAX);
    let moving = !train.heading.is_nan();
    let metres = if moving { train.sample.speed * elapsed } else { 0.0 } - back;
    if let (Some(rail), Some(rails)) = (rail_of(rails, version, train), rails) {
        let pos = rails.advance(rail, metres / unit_metres(train.sample.lat));
        if let Some((xy, tangent)) = rails.point(pos.line, pos.s) {
            let (lat, lon) = to_latlon(xy.0, xy.1);
            let sign = f64::from(pos.direction);
            return (lat, lon, grid_heading((tangent.0 * sign, tangent.1 * sign)));
        }
    }
    let sample = &train.sample;
    if metres != 0.0 {
        let heading = if moving { train.heading } else { 0.0 };
        let (lat, lon) = destination(sample.lat, sample.lon, heading, metres);
        return (lat, lon, train.heading);
    }
    (sample.lat, sample.lon, train.heading)
}

/// Точка и азимут между a и b по доле k.
pub(crate) fn blend(a: (f64, f64, f64), b: (f64, f64, f64), k: f64) -> (f64, f64, f64) {
    let lat = a.0 + k * (b.0 - a.0);
    let dlon = (b.1 - a.1 + 180.0).rem_euclid(360.0) - 180.0;
    let lon = (a.1 + k * dlon + 180.0).rem_euclid(360.0) - 180.0;
    let heading = if a.2.is_nan() {
        b.2
    } else if b.2.is_nan() {
        a.2
    } else {
        let turn = (b.2 - a.2 + 180.0).rem_euclid(360.0) - 180.0;
        (a.2 + k * turn).rem_euclid(360.0)
    };
    (lat, lon, heading)
}

/// (широта, долгота, азимут) на экране точки поезда back метров позади
/// головы.
fn place_train(rails: Option<&Rails>, version: u64, train: &mut Train, now: f64, clock: f64, back: f64) -> (f64, f64, f64) {
    let mut here = ahead(rails, version, train, clock, back);
    if let Some(from) = train.blend_from {
        let k = (now - train.blend_at) / BLEND;
        if k >= 1.0 {
            train.blend_from = None;
        } else {
            let k = k.max(0.0);
            let head = if back == 0.0 { here } else { ahead(rails, version, train, clock, 0.0) };
            // Весь состав сдвинут на остаток пути головы от прежнего места
            // на экране.
            let moved = blend(from, head, k);
            here = (
                here.0 + moved.0 - head.0,
                here.1 + (moved.1 - head.1 + 180.0).rem_euclid(360.0) - 180.0,
                if back == 0.0 { moved.2 } else { here.2 },
            );
        }
    }
    here
}

impl Fleet {
    /// Рельсы у вида или None. Рельсы без линий - None.
    pub fn set_rails(&mut self, rails: Option<Rails>) {
        self.rails = rails.filter(|r| !r.is_empty());
        self.rails_version += 1;
    }

    /// Новые положения из ответа службы. now - монотонные часы окна для
    /// перехода, clock - секунды UTC для хода по скорости. Поездов,
    /// которых нет в ответе или чьё положение старше [`STALE`], нет.
    pub fn update(&mut self, samples: Vec<Sample>, now: f64, clock: f64) {
        let mut seen = std::collections::BTreeSet::new();
        let version = self.rails_version;
        for sample in samples {
            if clock - sample.t > STALE {
                continue;
            }
            seen.insert(sample.key.clone());
            match self.trains.get_mut(&sample.key) {
                None => {
                    self.trains.insert(sample.key.clone(), Train::new(sample));
                }
                Some(train) => {
                    let shown = place_train(self.rails.as_ref(), version, train, now, clock, 0.0);
                    train.update(sample, shown, now);
                }
            }
        }
        self.trains.retain(|key, _| seen.contains(key));
    }

    /// (широта, долгота, азимут) на экране точки поезда key back метров
    /// позади головы - вагона состава. Нет поезда - None.
    pub fn place(&mut self, key: &TrainKey, now: f64, clock: f64, back: f64) -> Option<(f64, f64, f64)> {
        let version = self.rails_version;
        let train = self.trains.get_mut(key)?;
        Some(place_train(self.rails.as_ref(), version, train, now, clock, back))
    }

    /// [(ключ, широта, долгота, азимут)] на момент, по порядку ключей.
    /// Азимут NaN - курс ещё неизвестен.
    pub fn positions(&mut self, now: f64, clock: f64) -> Vec<(TrainKey, f64, f64, f64)> {
        let version = self.rails_version;
        let rails = self.rails.as_ref();
        self.trains
            .iter_mut()
            .map(|(key, train)| {
                let (lat, lon, heading) = place_train(rails, version, train, now, clock, 0.0);
                (key.clone(), lat, lon, heading)
            })
            .collect()
    }
}

// Модель.

/// Треугольники коробки подряд, гранями наружу.
pub(crate) fn box_triangles(x: (f64, f64), y: (f64, f64), z: (f64, f64)) -> Vec<[f64; 3]> {
    let ((x0, x1), (y0, y1), (z0, z1)) = (x, y, z);
    let v = [[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0], [x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]];
    let faces = [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [2, 3, 7, 6], [1, 2, 6, 5], [3, 0, 4, 7]];
    faces.iter().flat_map(|&[a, b, c, d]| [v[a], v[b], v[c], v[a], v[c], v[d]]).collect()
}

/// Цвет RGB 0-1 с непрозрачностью 1.
pub(crate) fn opaque(color: [f64; 3]) -> [f64; 4] {
    [color[0], color[1], color[2], 1.0]
}

/// Условный вагон: тележки, кузов цвета категории, полоса окон,
/// у головного front - лобовое стекло. Перёд смотрит по оси +Y.
pub fn train_parts(color: [f64; 3], front: bool) -> Vec<Part> {
    let (hl, hw) = (CAR_LENGTH / 2.0, CAR_WIDTH / 2.0);
    let under = box_triangles((-hw + 0.3, hw - 0.3), (-hl + 1.0, hl - 1.0), (0.0, 1.2));
    let band = box_triangles((-hw - 0.02, hw + 0.02), (-hl + 1.5, hl - 2.0), (2.5, 3.4));
    let (body, glass) = if front {
        let mut body = box_triangles((-hw, hw), (-hl, hl - 1.2), (1.2, CAR_HEIGHT));
        body.extend(box_triangles((-hw, hw), (hl - 1.2, hl), (1.2, CAR_HEIGHT - 1.2)));
        (body, box_triangles((-hw + 0.2, hw - 0.2), (hl - 1.2, hl + 0.02), (CAR_HEIGHT - 1.2, CAR_HEIGHT - 0.2)))
    } else {
        (box_triangles((-hw, hw), (-hl, hl), (1.2, CAR_HEIGHT)), Vec::new())
    };
    let mut windows = band;
    windows.extend(glass);
    vec![
        Part { positions: under, rgba: opaque(DARK) },
        Part { positions: body, rgba: opaque(color) },
        Part { positions: windows, rgba: opaque(GLASS) },
    ]
}
