// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Скважины: устья, интервалы, инклинометрия, ствол. Перенос
//! `core/drillholes.py` модуля.
//!
//! - Устье (collar) - строка с полями номера скважины, отметки устья
//!   и глубины забоя по стволу.
//! - Интервал (interval) - номер скважины, глубины по стволу from и to
//!   (вниз положительные), код пласта или литологии. Прочие поля
//!   сохраняются.
//! - Инклинометрия (survey) - номер скважины, глубина по стволу, азимут,
//!   угол наклона. Без инклинометрии скважина вертикальная.
//!
//! Строка таблицы - словарь «имя поля - значение» ([`Row`]). Имена полей
//! ищутся по синонимам без учёта регистра, [`FIELDS`].
//!
//! Ствол - метод минимальной кривизны: между станциями ствол идёт
//! по дуге окружности, приращения умножаются на коэффициент
//! rf = 2 / β · tg(β / 2), β - угол между направлениями станций. Дуга
//! делится на части не больше [`ARC_STEP`], так ломаная ствола не срезает
//! дугу. Выше первой станции ствол идёт по её направлению, ниже
//! последней - по её направлению до забоя.
//!
//! Угол наклона приводится к зениту, 0 - вниз отвесно. Поле zenith, inc,
//! inclination - уже зенит. Поле dip - угол от горизонта: отрицательные
//! значения (-90 - вниз отвесно) дают зенит 90 + dip, положительные
//! с медианой больше 45 - зенит 90 - dip, иначе значения считаются
//! зенитом. Правило взято у Isoliner (to_zenith).

use std::collections::{BTreeMap, HashMap};
use std::f64::consts::PI;

/// Роль поля таблицы.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    HoleId,
    Z,
    Eoh,
    From,
    To,
    Code,
    Md,
    Azimuth,
    Angle,
}

/// Роль поля - имена-синонимы в порядке предпочтения.
pub const FIELDS: [(Role, &[&str]); 9] = [
    (Role::HoleId, &["hole_id", "bhid", "dhid", "well", "holeid", "hole"]),
    (Role::Z, &["z", "elev", "elevation", "rl", "collar_z"]),
    (Role::Eoh, &["eoh", "td", "max_depth", "total_depth", "depth"]),
    (Role::From, &["from", "from_m", "depth_from"]),
    (Role::To, &["to", "to_m", "depth_to"]),
    (Role::Code, &["code", "litho", "lith", "geol", "seam", "class"]),
    (Role::Md, &["md", "depth", "at"]),
    (Role::Azimuth, &["azimuth", "azi", "az", "brg", "bearing"]),
    (Role::Angle, &["zenith", "inc", "inclination", "dip"]),
];
/// Имена поля угла, у которых значение - уже зенит.
pub const ZENITH_NAMES: [&str; 3] = ["zenith", "inc", "inclination"];
/// Наибольший угол части дуги ствола, радианы (3°).
pub const ARC_STEP: f64 = 3.0 * PI / 180.0;

/// Значение поля таблицы.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Float(f64),
    Text(String),
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Int(v)
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Int(i64::from(v))
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Text(v.to_string())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Text(v)
    }
}

/// Строка таблицы: имя поля - значение.
pub type Row = BTreeMap<String, Value>;
/// Сводка пропусков: причина - количество.
pub type Skipped = BTreeMap<&'static str, usize>;

/// Скважина. x, y - положение устья, как пришло из слоя, z - отметка
/// устья, eoh - глубина забоя по стволу. axis - узлы ствола: глубина
/// по стволу, смещение на восток, на север от устья в метрах, отметка.
#[derive(Debug, Clone, PartialEq)]
pub struct Hole {
    pub hole_id: String,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub z: f64,
    pub eoh: f64,
    pub axis: Vec<[f64; 4]>,
    pub intervals: Vec<Interval>,
}

/// Интервал скважины: глубины по стволу, код и прочие поля строки.
#[derive(Debug, Clone, PartialEq)]
pub struct Interval {
    pub start: f64,
    pub end: f64,
    pub code: String,
    pub extra: Row,
}

/// Имя поля роли role среди names или None. Без учёта регистра, синонимы -
/// [`FIELDS`] по порядку. Из имён, равных без учёта регистра, берётся
/// последнее.
pub fn find_field<'a>(names: impl IntoIterator<Item = &'a str>, role: Role) -> Option<&'a str> {
    let mut lower: HashMap<String, &'a str> = HashMap::new();
    for name in names {
        lower.insert(name.to_lowercase(), name);
    }
    let words = FIELDS.iter().find(|(r, _)| *r == role).map(|(_, w)| *w).unwrap_or_default();
    words.iter().find_map(|w| lower.get(*w).copied())
}

/// Число или None для пустых, нечисловых и бесконечных значений.
fn number(value: Option<&Value>) -> Option<f64> {
    let v = match value? {
        Value::Null => return None,
        Value::Int(i) => *i as f64,
        Value::Float(f) => *f,
        Value::Text(t) => t.trim().parse::<f64>().ok()?,
    };
    v.is_finite().then_some(v)
}

/// Текст значения, как `str` в Python: у дробного числа - с точкой.
fn text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => format!("{f:?}"),
        Value::Text(t) => t.clone(),
    }
}

/// Номер скважины строкой: 12 и 12.0 из разных таблиц совпадают. Пустое
/// значение - пустая строка.
fn key(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::Float(f)) if f.fract() == 0.0 && f.abs() < 9.0e15 => format!("{}", *f as i64),
        Some(v) => text(v).trim().to_string(),
    }
}

/// Медиана чисел без NaN, NaN - если чисел нет.
fn median(values: &[f64]) -> f64 {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| !x.is_nan()).collect();
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    let mid = v.len() / 2;
    if v.len() % 2 == 1 { v[mid] } else { (v[mid - 1] + v[mid]) / 2.0 }
}

/// Углы наклона поля name в зениты, градусы. 0 - вниз отвесно.
pub fn zeniths(values: &[f64], name: &str) -> Vec<f64> {
    if ZENITH_NAMES.contains(&name.to_lowercase().as_str()) || values.is_empty() {
        return values.to_vec();
    }
    let least = values.iter().copied().filter(|x| !x.is_nan()).fold(f64::INFINITY, f64::min);
    if least < 0.0 {
        return values.iter().map(|v| 90.0 + v).collect();
    }
    if median(values) > 45.0 {
        return values.iter().map(|v| 90.0 - v).collect();
    }
    values.to_vec()
}

type V3 = [f64; 3];

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Единичное направление ствола (восток, север, вниз), радианы.
fn direction(zenith: f64, azimuth: f64) -> V3 {
    [zenith.sin() * azimuth.sin(), zenith.sin() * azimuth.cos(), zenith.cos()]
}

/// Угол между единичными направлениями.
fn angle(a: V3, b: V3) -> f64 {
    dot(a, b).clamp(-1.0, 1.0).acos()
}

/// Направление на доле share дуги от a к b.
fn slerp(a: V3, b: V3, share: f64) -> V3 {
    let angle = angle(a, b);
    if angle < 1e-9 {
        return a;
    }
    scale(add(scale(a, ((1.0 - share) * angle).sin()), scale(b, (share * angle).sin())), 1.0 / angle.sin())
}

/// Приращение (восток, север, вниз) по минимальной кривизне между
/// направлениями t1 и t2 на длине ствола length.
fn arc_step(start: V3, t1: V3, t2: V3, length: f64) -> V3 {
    let beta = angle(t1, t2);
    let rf = if beta < 1e-9 { 1.0 } else { 2.0 / beta * (beta / 2.0).tan() };
    add(start, scale(add(t1, t2), length / 2.0 * rf))
}

/// Узлы ствола: md, восток, север, отметка.
///
/// stations - (md, зенит, азимут) в градусах, в любом порядке. Станции
/// с нечисловыми значениями отбрасываются. Без станций скважина
/// вертикальная до eoh.
pub fn axis(z: f64, eoh: f64, stations: &[(f64, f64, f64)]) -> Vec<[f64; 4]> {
    let mut stations: Vec<(f64, f64, f64)> = stations
        .iter()
        .filter(|(m, zen, az)| m.is_finite() && zen.is_finite() && az.is_finite())
        .map(|(m, zen, az)| (*m, zen.to_radians(), az.to_radians()))
        .collect();
    stations.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.total_cmp(&b.2)));
    let (Some(&first), Some(&last)) = (stations.first(), stations.last()) else {
        return vec![[0.0, 0.0, 0.0, z], [eoh, 0.0, 0.0, z - eoh]];
    };
    let mut nodes: Vec<(f64, V3)> = vec![(0.0, [0.0; 3])];
    let mut position = [0.0; 3];
    let mut md = 0.0;
    if first.0 > 0.0 {
        md = first.0.min(eoh);
        position = scale(direction(first.1, first.2), md);
        nodes.push((md, position));
    }
    for pair in stations.windows(2) {
        let [(m1, z1, a1), (m2, z2, a2)] = [pair[0], pair[1]];
        if m1 >= eoh {
            break;
        }
        let m2 = m2.min(eoh);
        let (t1, t2) = (direction(z1, a1), direction(z2, a2));
        // Угол не больше π, частей не больше 60.
        let parts = ((angle(t1, t2) / ARC_STEP).ceil() as usize).max(1);
        for k in 0..parts {
            let a = slerp(t1, t2, k as f64 / parts as f64);
            let b = slerp(t1, t2, (k + 1) as f64 / parts as f64);
            position = arc_step(position, a, b, (m2 - m1) / parts as f64);
            md = m1 + (m2 - m1) * (k + 1) as f64 / parts as f64;
            nodes.push((md, position));
        }
    }
    if md < eoh {
        position = add(position, scale(direction(last.1, last.2), eoh - md));
        nodes.push((eoh, position));
    }
    // Повторные глубины - от станций на одной глубине.
    let mut out: Vec<[f64; 4]> = Vec::with_capacity(nodes.len());
    for (m, p) in nodes {
        if out.last().is_some_and(|prev| m - prev[0] <= 1e-9) {
            continue;
        }
        out.push([m, p[0], p[1], z - p[2]]);
    }
    out
}

/// Точка ствола (восток, север, отметка) на глубине md. За забоем ствол
/// продолжается по последнему отрезку. None - у ствола нет узлов.
pub fn point_at(nodes: &[[f64; 4]], md: f64) -> Option<[f64; 3]> {
    let first = nodes.first()?;
    if md <= first[0] || nodes.len() == 1 {
        return Some([first[1], first[2], first[3]]);
    }
    let i = nodes.partition_point(|n| n[0] < md).clamp(1, nodes.len() - 1);
    let (a, b) = (nodes[i - 1], nodes[i]);
    let share = if b[0] > a[0] { (md - a[0]) / (b[0] - a[0]) } else { 0.0 };
    Some([a[1] + (b[1] - a[1]) * share, a[2] + (b[2] - a[2]) * share, a[3] + (b[3] - a[3]) * share])
}

/// Узлы ствола между глубинами start и end, с концами.
pub fn piece(nodes: &[[f64; 4]], start: f64, end: f64) -> Vec<[f64; 3]> {
    let (Some(a), Some(b)) = (point_at(nodes, start), point_at(nodes, end)) else {
        return Vec::new();
    };
    let mut out = vec![a];
    out.extend(nodes.iter().filter(|n| n[0] > start && n[0] < end).map(|n| [n[1], n[2], n[3]]));
    out.push(b);
    out
}

/// Цвет «#rrggbb» в тройку 0-255, иначе None.
fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let t = text.trim_start_matches('#');
    if t.len() < 6 || !t.is_char_boundary(6) {
        return None;
    }
    let part = |i: usize| t.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok());
    Some([part(0)?, part(2)?, part(4)?])
}

/// Цвет кода пласта (r, g, b). palette - словарь код - цвет «#rrggbb»,
/// иначе цвет из хэша кода, один и тот же у кода всегда.
pub fn code_color(code: &str, palette: Option<&HashMap<String, String>>) -> [u8; 3] {
    if let Some(rgb) = palette.and_then(|p| p.get(code)).and_then(|t| parse_hex(t)) {
        return rgb;
    }
    // MD5 здесь - только устойчивый цвет кода, как у модуля, не защита.
    let digest = md5(code.as_bytes());
    let hue = f64::from(digest[0]) / 255.0;
    let (r, g, b) =
        hsv(hue, 0.55 + f64::from(digest[1]) / 255.0 * 0.35, 0.70 + f64::from(digest[2]) / 255.0 * 0.25);
    [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8]
}

fn hsv(h: f64, s: f64, v: f64) -> (f64, f64, f64) {
    let whole = (h * 6.0).floor();
    let f = h * 6.0 - whole;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match (whole as i64).rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

/// Дайджест MD5 (RFC 1321). Только для устойчивого цвета кода.
fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
        14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15,
        21, 6, 10, 15, 21,
    ];
    // Постоянные - целая часть 2³² · |sin(i + 1)|, таблица RFC 1321.
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501, 0x698098d8,
        0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340,
        0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87,
        0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c,
        0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039,
        0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92,
        0xffeff47d, 0x85845dd1, 0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];
    let mut state: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
    let mut message = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bits.to_le_bytes());
    for block in message.as_chunks::<64>().0 {
        let words: [u32; 16] = std::array::from_fn(|i| {
            u32::from_le_bytes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]])
        });
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let rotated = a.wrapping_add(f).wrapping_add(K[i]).wrapping_add(words[g]).rotate_left(S[i]);
            (a, d, c) = (d, c, b);
            b = b.wrapping_add(rotated);
        }
        state = [state[0].wrapping_add(a), state[1].wrapping_add(b), state[2].wrapping_add(c), state[3].wrapping_add(d)];
    }
    let mut out = [0u8; 16];
    for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(state) {
        *chunk = word.to_le_bytes();
    }
    out
}

/// Имя поля роли в строке-образце.
fn field_of(sample: Option<&Row>, role: Role) -> Option<String> {
    find_field(sample?.keys().map(String::as_str), role).map(str::to_string)
}

/// Скважины из строк таблиц. У каждой строки устья ещё ключи "x" и "y" -
/// положение из геометрии слоя. Имена полей берутся по первой строке
/// таблицы. Возвращает скважины и сводку пропусков.
pub fn assemble(collars: &[Row], intervals: &[Row], surveys: &[Row]) -> (Vec<Hole>, Skipped) {
    let mut skipped = Skipped::new();
    let mut skip = |reason: &'static str| *skipped.entry(reason).or_default() += 1;
    let sample = collars.first();
    let (f_id, f_z, f_eoh) = (field_of(sample, Role::HoleId), field_of(sample, Role::Z), field_of(sample, Role::Eoh));
    let get = |row: &'_ Row, f: &Option<String>| f.as_ref().and_then(|f| row.get(f)).cloned();
    // Станции инклинометрии по скважинам, на одной глубине - последняя.
    let mut by_hole: HashMap<String, Vec<(f64, f64, f64)>> = HashMap::new();
    if let Some(first) = surveys.first() {
        let names = [Role::HoleId, Role::Md, Role::Azimuth, Role::Angle].map(|r| field_of(Some(first), r));
        if let [Some(s_id), Some(s_md), Some(s_az), Some(s_angle)] = names {
            let rows: Vec<(String, f64, f64, f64)> = surveys
                .iter()
                .filter_map(|r| {
                    let (md, az, angle) = (number(r.get(&s_md))?, number(r.get(&s_az))?, number(r.get(&s_angle))?);
                    Some((key(r.get(&s_id)), md, az, angle))
                })
                .collect();
            let angles: Vec<f64> = rows.iter().map(|r| r.3).collect();
            for ((hole, md, az, _), zen) in rows.into_iter().zip(zeniths(&angles, &s_angle)) {
                let list = by_hole.entry(hole).or_default();
                match list.iter_mut().find(|s| s.0 == md) {
                    Some(s) => *s = (md, zen, az),
                    None => list.push((md, zen, az)),
                }
            }
        } else {
            skip("survey_fields");
        }
    }
    // Интервалы по скважинам, в порядке первого появления скважины.
    let mut parts: Vec<(String, Vec<Interval>)> = Vec::new();
    if let Some(first) = intervals.first() {
        let [i_id, i_from, i_to, i_code] =
            [Role::HoleId, Role::From, Role::To, Role::Code].map(|r| field_of(Some(first), r));
        for row in intervals {
            let hole = get(row, &i_id).map(|v| key(Some(&v))).unwrap_or_default();
            let (start, end) = (number(get(row, &i_from).as_ref()), number(get(row, &i_to).as_ref()));
            let (Some(mut start), Some(mut end)) = (start, end) else {
                skip("interval_fields");
                continue;
            };
            if i_id.is_none() || hole.is_empty() {
                skip("interval_fields");
                continue;
            }
            if end < start {
                (start, end) = (end, start);
                skip("interval_swapped");
            }
            if end == start {
                skip("interval_empty");
                continue;
            }
            let own = [&i_id, &i_from, &i_to, &i_code];
            let extra: Row = row
                .iter()
                .filter(|(k, _)| !own.iter().any(|f| f.as_deref() == Some(k.as_str())))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let code = get(row, &i_code).map(|v| text(&v)).unwrap_or_default();
            let interval = Interval { start, end, code, extra };
            match parts.iter_mut().find(|(h, _)| *h == hole) {
                Some((_, list)) => list.push(interval),
                None => parts.push((hole, vec![interval])),
            }
        }
    }
    let mut holes = Vec::new();
    for row in collars {
        let hole = get(row, &f_id).map(|v| key(Some(&v))).unwrap_or_default();
        let z = number(get(row, &f_z).as_ref());
        let Some(z) = z.filter(|_| !hole.is_empty()) else {
            skip("collar_fields");
            continue;
        };
        let mut own = parts.iter().find(|(h, _)| *h == hole).map(|(_, l)| l.clone()).unwrap_or_default();
        own.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.end.total_cmp(&b.end)));
        let deepest = own.iter().map(|p| p.end).fold(0.0, f64::max);
        let eoh = match number(get(row, &f_eoh).as_ref()) {
            Some(e) if e >= deepest => e,
            _ => deepest,
        };
        if eoh <= 0.0 {
            skip("collar_no_depth");
            continue;
        }
        let stations = by_hole.get(&hole).cloned().unwrap_or_default();
        let axis = axis(z, eoh, &stations);
        holes.push(Hole {
            hole_id: hole,
            x: number(row.get("x")),
            y: number(row.get("y")),
            z,
            eoh,
            axis,
            intervals: own,
        });
    }
    for (hole, _) in &parts {
        if !holes.iter().any(|h| h.hole_id == *hole) {
            skip("interval_no_collar");
        }
    }
    (holes, skipped)
}
