// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Самолёты вживую по ADS-B сети adsb.lol. Перенос `core/aircraft.py`
//! модуля.
//!
//! Запрос - борта в круге у точки взгляда ([`point_url`], радиус в
//! морских милях не больше [`MAX_RADIUS`]), ответ в разметке readsb:
//! hex, позывной, бортовой номер, тип, высота по барометру и по GNSS
//! в футах или «ground», путевая скорость в узлах, путевой угол, курс,
//! вертикальная скорость в футах в минуту, крен, категория ADS-B,
//! давность положения. Без ключа. Лицензия выдачи у adsb.lol не названа.
//!
//! Модель - условный самолёт длиной 1 вдоль оси Y, его растягивает
//! масштаб по категории ([`unit_plane_parts`], [`plane_size`]).

use std::collections::BTreeMap;

use serde_json::Value;

use crate::movers::Sample;
pub use crate::movers::{FOOT, KNOT};
use crate::trains::{Part, box_triangles, number, opaque, string};

/// Шаблон адреса модуля: широта и долгота с 4 знаками, радиус в морских
/// милях.
pub const POINT_URL: &str = "https://api.adsb.lol/v2/point/{lat:.4f}/{lon:.4f}/{radius}";
pub const ATTRIBUTION: (&str, &str) = ("Aircraft: adsb.lol", "https://www.adsb.lol/");
/// Радиус запроса в морских милях: предел службы и наименьший.
pub const MAX_RADIUS: i64 = 250;
pub const MIN_RADIUS: i64 = 5;
/// Секунд между запросами.
pub const POLL: f64 = 10.0;
/// Секунд хода своим курсом после положения.
pub const EXTRAPOLATE: f64 = 30.0;
/// Положение старше этого не показывается, секунды.
pub const STALE: f64 = 120.0;
/// Метров в морской миле.
pub const NM: f64 = 1852.0;
/// Категории ADS-B: группа, коды, длина условной модели в метрах, цвет
/// киля 0-1. Выбор помощника модуля.
pub const GROUPS: [(&str, &[&str], f64, [f64; 3]); 6] = [
    ("light", &["A1"], 9.0, [0.30, 0.75, 0.35]),
    ("small", &["A2", "A6"], 16.0, [0.15, 0.75, 0.85]),
    ("large", &["A3", "A4"], 38.0, [0.20, 0.45, 0.90]),
    ("heavy", &["A5"], 64.0, [0.62, 0.30, 0.85]),
    ("rotor", &["A7"], 14.0, [0.96, 0.58, 0.14]),
    ("other", &[], 30.0, [0.75, 0.75, 0.75]),
];
pub const WHITE: [f64; 3] = [0.93, 0.94, 0.95];
pub const GREY: [f64; 3] = [0.70, 0.72, 0.75];

type Group = (&'static str, &'static [&'static str], f64, [f64; 3]);

fn other() -> Group {
    GROUPS[GROUPS.len() - 1]
}

/// Группа по категории ADS-B.
pub fn group(category: &str) -> &'static str {
    GROUPS.iter().find(|g| g.1.contains(&category)).map_or("other", |g| g.0)
}

/// Цвет киля группы 0-1, неизвестной - цвет «other».
pub fn group_color(key: &str) -> [f64; 3] {
    GROUPS.iter().copied().find(|g| g.0 == key).unwrap_or_else(other).3
}

/// (размах, длина, высота) модели в метрах по категории.
pub fn plane_size(category: &str) -> (f64, f64, f64) {
    let key = group(category);
    let length = GROUPS.iter().copied().find(|g| g.0 == key).unwrap_or_else(other).2;
    (length, length, length)
}

/// Адрес бортов в круге radius_m метров у точки.
pub fn point_url(lat: f64, lon: f64, radius_m: f64) -> String {
    let miles = (radius_m / NM).ceil();
    let radius = if miles.is_nan() { MIN_RADIUS } else { (miles.clamp(MIN_RADIUS as f64, MAX_RADIUS as f64)) as i64 };
    format!("https://api.adsb.lol/v2/point/{lat:.4}/{lon:.4}/{radius}")
}

/// Сведения о борте: позывной, бортовой номер, тип, категория, на земле
/// ли он, код ответчика.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Info {
    pub flight: String,
    pub reg: String,
    pub kind: String,
    pub category: String,
    pub ground: bool,
    pub squawk: String,
}

impl Info {
    /// Группа по категории.
    pub fn group(&self) -> &'static str {
        group(&self.category)
    }
}

/// Положения и {hex: сведения} из ответа adsb.lol байтами JSON. Ключ -
/// hex в нижнем регистре. Высота борта на земле - NaN, его ставит на
/// рельеф окно. Ответ не JSON - пусто.
pub fn parse(data: &[u8]) -> (Vec<Sample<String>>, BTreeMap<String, Info>) {
    serde_json::from_slice::<Value>(data).map(|json| parse_json(&json)).unwrap_or_default()
}

/// Положения и сведения из уже разобранного ответа, как [`parse`].
pub fn parse_json(data: &Value) -> (Vec<Sample<String>>, BTreeMap<String, Info>) {
    let (mut samples, mut info) = (Vec::new(), BTreeMap::new());
    let (Some(now), Some(list)) = (number(data.get("now")), data.get("ac").and_then(Value::as_array)) else {
        return (samples, info);
    };
    for ac in list {
        let key = match ac.get("hex") {
            Some(Value::String(s)) => s.trim().to_lowercase(),
            Some(Value::Number(n)) if n.as_f64() != Some(0.0) => n.to_string(),
            _ => continue,
        };
        if key.is_empty() {
            continue;
        }
        let field = |name: &str| number(ac.get(name));
        let (Some(lat), Some(lon)) = (field("lat"), field("lon")) else { continue };
        if !(-90.0..=90.0).contains(&lat) {
            continue;
        }
        let seen = field("seen_pos").or_else(|| field("seen")).unwrap_or(0.0);
        let ground = ac.get("alt_baro").and_then(Value::as_str) == Some("ground");
        let feet = field("alt_geom").or_else(|| field("alt_baro"));
        let alt = match feet {
            Some(feet) if !ground => (feet * FOOT).max(0.0),
            _ => f64::NAN,
        };
        let rate = field("geom_rate").or_else(|| field("baro_rate")).unwrap_or(0.0);
        samples.push(Sample {
            speed: field("gs").unwrap_or(0.0) * KNOT,
            course: field("track").unwrap_or(f64::NAN),
            heading: field("true_heading").unwrap_or(f64::NAN),
            alt,
            climb: if ground { 0.0 } else { rate * FOOT / 60.0 },
            roll: field("roll").unwrap_or(f64::NAN),
            turn: field("track_rate").unwrap_or(f64::NAN),
            ..Sample::new(key.clone(), now / 1000.0 - seen, lat, lon)
        });
        info.insert(
            key,
            Info {
                flight: string(ac.get("flight")),
                reg: string(ac.get("r")),
                kind: string(ac.get("t")),
                category: string(ac.get("category")),
                ground,
                squawk: string(ac.get("squawk")),
            },
        );
    }
    (samples, info)
}

/// Подпись борта: позывной, иначе бортовой номер, иначе hex.
pub fn label(key: &str, info: Option<&Info>) -> String {
    match info {
        Some(info) if !info.flight.is_empty() => info.flight.clone(),
        Some(info) if !info.reg.is_empty() => info.reg.clone(),
        _ => key.to_uppercase(),
    }
}

/// Условный самолёт длиной 1 (ось Y, нос - +Y), размахом 1: фюзеляж,
/// крыло, стабилизатор белые и серые, киль цвета группы. Точка модели -
/// середина фюзеляжа по высоте.
pub fn unit_plane_parts(color: [f64; 3]) -> Vec<Part> {
    let mut body = box_triangles((-0.05, 0.05), (-0.5, 0.42), (-0.05, 0.05));
    body.extend(box_triangles((-0.03, 0.03), (0.42, 0.5), (-0.03, 0.03)));
    let mut wings = box_triangles((-0.5, 0.5), (-0.06, 0.1), (-0.012, 0.012));
    wings.extend(box_triangles((-0.17, 0.17), (-0.5, -0.41), (0.0, 0.015)));
    let fin = box_triangles((-0.008, 0.008), (-0.5, -0.38), (0.05, 0.2));
    vec![
        Part { positions: body, rgba: opaque(WHITE) },
        Part { positions: wings, rgba: opaque(GREY) },
        Part { positions: fin, rgba: opaque(color) },
    ]
}
