// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Вид метки, как «Вид» метки Google Earth, перенос `core/lookat.py`
//! модуля.
//!
//! Вид - пять чисел: широта и долгота точки взгляда, расстояние до неё
//! («диапазон» Google Earth), азимут («курс») и наклон («угол обзора»).
//! Точка взгляда может не совпадать с меткой. По виду идут перелёт
//! к метке и остановка тура, в KML он уходит элементом LookAt. Прежние
//! виды из трёх чисел - расстояние, азимут, наклон - смотрят на саму
//! метку.
//!
//! Слои вида - строки раздела «Слои» модуля (ключ - включена ли), их
//! включают и выключают перелёт к метке и остановка тура. Ключи,
//! начатые знаком [`STATE`], - состояние вида: сектор разреза, земля
//! над подземной моделью, непрозрачности строк.
//!
//! Camera KML - глаз в точке с высотой, курсом и наклоном - переводится
//! в те же пять чисел ([`from_camera`]). Точка взгляда - встреча луча
//! взгляда с эллипсоидом. Крен Camera не переносится.

use std::collections::BTreeMap;

use globe_core::DVec3;
use globe_core::ellipsoid::Ellipsoid;
use serde_json::{Map, Value};

pub const MAX_TILT: f64 = 85.0;
/// Метка «Сохранить вид» стоит в точке своего вида. Доля градуса,
/// до которой точки считаются одной.
pub const SAME_POINT: f64 = 1e-7;
/// Наклоны, которые пробует [`from_camera`], если луч Camera проходит
/// мимо Земли: близкий к горизонту, [`MAX_TILT`], отвесный.
pub const CAMERA_TILTS: [f64; 3] = [89.0, MAX_TILT, 0.0];

/// Вид: широта, долгота, расстояние, азимут, наклон.
pub type View = [f64; 5];

/// Вид с азимутом 0-360 и наклоном 0-[`MAX_TILT`]. None, если
/// расстояние не больше нуля или число не конечно.
pub fn make(lat: f64, lon: f64, distance: f64, heading: f64, tilt: f64) -> Option<View> {
    let all = [lat, lon, distance, heading, tilt];
    if !all.iter().all(|v| v.is_finite()) || distance <= 0.0 {
        return None;
    }
    Some([lat, lon, distance, heading.rem_euclid(360.0), tilt.clamp(0.0, MAX_TILT)])
}

/// Восток, север и верх в точке эллипсоида, ECEF.
fn axes(e: &Ellipsoid, lat: f64, lon: f64) -> (DVec3, DVec3, DVec3) {
    let up = e.surface_normal(lat, lon);
    let lo = lon.to_radians();
    let east = DVec3::new(-lo.sin(), lo.cos(), 0.0);
    (east, up.cross(east), up)
}

/// Вид по Camera KML: глаз над точкой (lat, lon) на высоте altitude
/// метров над эллипсоидом, курс heading и наклон tilt от отвеса. Луч
/// мимо Земли - наклон уменьшается по [`CAMERA_TILTS`]. None, если вид
/// не получился.
pub fn from_camera(e: &Ellipsoid, lat: f64, lon: f64, altitude: f64, heading: f64, tilt: f64) -> Option<View> {
    if ![lat, lon, altitude, heading, tilt].iter().all(|v| v.is_finite()) {
        return None;
    }
    let eye = e.geodetic_to_ecef(lat, lon, altitude.max(1.0));
    let (east, north, up) = axes(e, lat, lon);
    let h = heading.to_radians();
    let (ray, reach) = std::iter::once(tilt.clamp(0.0, 89.0)).chain(CAMERA_TILTS).find_map(|t| {
        let r = t.to_radians();
        let ray = (east * h.sin() + north * h.cos()) * r.sin() - up * r.cos();
        e.ray_intersect(eye, ray, 0.0).filter(|d| *d > 0.0).map(|d| (ray, d))
    })?;
    let spot = eye + ray * reach;
    let (lat2, lon2, _) = e.ecef_to_geodetic(spot);
    let (east2, north2, up2) = axes(e, lat2, lon2);
    let length = ray.length();
    let slope = (-ray.dot(up2) / length).clamp(-1.0, 1.0).acos().to_degrees();
    let course = ray.dot(east2).atan2(ray.dot(north2)).to_degrees();
    make(lat2, lon2, reach * length, course, slope)
}

/// Вид из поля файла меток. Три числа - прежний вид, его точка взгляда -
/// anchor (широта, долгота метки). None при пустом или неверном поле.
pub fn parse(text: &str, anchor: Option<(f64, f64)>) -> Option<View> {
    let values: Vec<f64> = text
        .split(',')
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .ok()?;
    match (values.as_slice(), anchor) {
        ([a, b, c, d, f], _) => make(*a, *b, *c, *d, *f),
        ([c, d, f], Some((lat, lon))) => make(lat, lon, *c, *d, *f),
        _ => None,
    }
}

/// Число, как repr в Python: целое с «.0».
pub fn number_text(v: f64) -> String {
    format!("{v:?}")
}

/// Вид в поле файла меток, пустая строка без вида.
pub fn text(view: Option<&View>) -> String {
    view.map_or(String::new(), |v| v.iter().map(|x| number_text(*x)).collect::<Vec<_>>().join(","))
}

/// Метка «Сохранить вид»: точка, которая стоит в точке своего вида.
pub fn is_view_mark(kind: &str, points: &[(f64, f64)], view: Option<&View>) -> bool {
    match (kind, points.first(), view) {
        ("point", Some((lat, lon)), Some(v)) => (lat - v[0]).abs() < SAME_POINT && (lon - v[1]).abs() < SAME_POINT,
        _ => false,
    }
}

/// Ключ состояния вида среди строк «Слоёв» начинается этим знаком.
pub const STATE: &str = "@";
/// Сектор разреза Земли: запад, восток, юг, север, градусы.
pub const WEDGE: &str = "@wedge";
/// Земля над подземной моделью: opacity 0..1, cut, under.
pub const GROUND: &str = "@ground";
/// Непрозрачности строк: ключ строки - 0..1.
pub const OPACITY: &str = "@opacity";

/// Слои вида: ключ строки - включена ли, ключи состояния - значение.
pub type Layers = BTreeMap<String, Value>;

fn plain_number(v: &Value) -> Option<f64> {
    if v.is_boolean() { None } else { v.as_f64() }
}

/// Значение ключа состояния, годное для записи, иначе None.
fn state_value(key: &str, value: &Value) -> Option<Value> {
    match key {
        WEDGE => {
            let items = value.as_array().filter(|a| a.len() == 4)?;
            let numbers: Option<Vec<f64>> = items.iter().map(plain_number).collect();
            Some(Value::from(numbers?))
        }
        GROUND => {
            let o = value.as_object()?;
            let mut out = Map::new();
            if let Some(v) = o.get("opacity").and_then(plain_number) {
                out.insert("opacity".into(), Value::from(v.clamp(0.0, 1.0)));
            }
            for name in ["cut", "under"] {
                match o.get(name) {
                    Some(Value::Bool(b)) => {
                        out.insert(name.into(), Value::Bool(*b));
                    }
                    Some(v) if v.as_i64().is_some() => {
                        out.insert(name.into(), Value::Bool(v.as_i64() != Some(0)));
                    }
                    _ => {}
                }
            }
            (!out.is_empty()).then_some(Value::Object(out))
        }
        OPACITY => {
            let o = value.as_object()?;
            let out: Map<String, Value> = o
                .iter()
                .filter_map(|(k, v)| plain_number(v).map(|n| (k.clone(), Value::from(n.clamp(0.0, 1.0)))))
                .collect();
            (!out.is_empty()).then_some(Value::Object(out))
        }
        _ => None,
    }
}

fn switch(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        v => v.as_i64().map(|i| i != 0),
    }
}

/// Слои вида в поле файла меток - JSON {ключ: 0 или 1} и ключи
/// состояния вида, пустая строка без слоёв.
pub fn layers_text(layers: Option<&Layers>) -> String {
    let mut out = Map::new();
    for (key, value) in layers.into_iter().flatten() {
        if key.starts_with(STATE) {
            if let Some(v) = state_value(key, value) {
                out.insert(key.clone(), v);
            }
        } else {
            let on = switch(value).unwrap_or(!value.is_null());
            out.insert(key.clone(), Value::from(i64::from(on)));
        }
    }
    if out.is_empty() { String::new() } else { Value::Object(out).to_string() }
}

/// Слои вида из поля файла меток или KML, None при пустом или неверном
/// поле.
pub fn layers_parse(text: &str) -> Option<Layers> {
    let data: Value = serde_json::from_str(text).ok()?;
    let mut out = Layers::new();
    for (key, value) in data.as_object()? {
        if key.starts_with(STATE) {
            if let Some(v) = state_value(key, value) {
                out.insert(key.clone(), v);
            }
        } else if let Some(on) = switch(value) {
            out.insert(key.clone(), Value::Bool(on));
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Только строки «Слоёв» {ключ: включена}, без состояния вида.
pub fn switches(layers: Option<&Layers>) -> BTreeMap<String, bool> {
    layers
        .into_iter()
        .flatten()
        .filter(|(k, _)| !k.starts_with(STATE))
        .filter_map(|(k, v)| switch(v).map(|on| (k.clone(), on)))
        .collect()
}
