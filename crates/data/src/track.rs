// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Растущие треки: точки объектов по времени, путь к моменту t.
//! Перенос `core/track.py` модуля.
//!
//! Трек - точки одного или нескольких объектов с временем в секундах
//! от 1970 года. К моменту t у объекта пройденный путь - его точки
//! не позже t и точка на отрезке, по которому он идёт в t. Положение
//! между точками линейное по времени. До первой точки объекта нет,
//! после последней он стоит в ней.
//!
//! У точки может быть высота в метрах - самолёты и дроны. Между точками
//! она тоже линейна по времени. Нет высоты - NaN.
//!
//! Поза модели на gx:Track (`model_pose` модуля) - `scene::track`.

use std::collections::BTreeMap;

use crate::trains::bearing;

/// Азимут по хорде пути за это время до момента t, секунд. Отдельные
/// точки GPS дрожат, азимут по последнему отрезку дёргал бы камеру.
pub const HEADING_SPAN: f64 = 30.0;

/// Точка трека: объект, время в секундах от 1970 года, широта,
/// долгота и высота в метрах или None.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub key: String,
    pub t: f64,
    pub lat: f64,
    pub lon: f64,
    pub alt: Option<f64>,
}

impl Item {
    /// Точка без высоты.
    pub fn new(key: impl Into<String>, t: f64, lat: f64, lon: f64) -> Self {
        Self {
            key: key.into(),
            t,
            lat,
            lon,
            alt: None,
        }
    }

    /// Та же точка с высотой alt, м.
    pub fn with_alt(mut self, alt: f64) -> Self {
        self.alt = Some(alt);
        self
    }
}

/// Точки одного объекта по возрастанию времени, высота без значения -
/// NaN.
#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    pub key: String,
    pub times: Vec<f64>,
    pub lats: Vec<f64>,
    pub lons: Vec<f64>,
    pub alts: Vec<f64>,
}

/// Объект в момент t: путь [(широта, долгота)] с текущей точкой
/// последней, текущая точка, азимут движения в градусах от севера
/// или NaN у стоящего, высота точки в метрах или NaN и высоты точек
/// пути или None, если высот у объекта нет.
#[derive(Debug, Clone, PartialEq)]
pub struct Position {
    pub key: String,
    pub path: Vec<(f64, f64)>,
    pub lat: f64,
    pub lon: f64,
    pub heading: f64,
    pub alt: f64,
    pub alts: Option<Vec<f64>>,
}

/// Точки объектов по времени.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Track {
    /// Объекты по порядку первой точки во входе.
    pub objects: Vec<Object>,
    /// Время первой и последней точки всех объектов, None у пустого.
    pub start: Option<f64>,
    pub end: Option<f64>,
}

impl Track {
    /// Трек из точек. Точки с временем, широтой или долготой не числом
    /// пропускаются. Точки одного объекта с одинаковым временем
    /// оставляются первой.
    pub fn new(items: impl IntoIterator<Item = Item>) -> Self {
        let mut index: BTreeMap<String, usize> = BTreeMap::new();
        let mut groups: Vec<(String, Vec<[f64; 4]>)> = Vec::new();
        for item in items {
            if !(item.t.is_finite() && item.lat.is_finite() && item.lon.is_finite()) {
                continue;
            }
            let row = [item.t, item.lat, item.lon, item.alt.unwrap_or(f64::NAN)];
            let at = match index.get(&item.key) {
                Some(&at) => at,
                None => {
                    index.insert(item.key.clone(), groups.len());
                    groups.push((item.key, Vec::new()));
                    groups.len() - 1
                }
            };
            if let Some((_, rows)) = groups.get_mut(at) {
                rows.push(row);
            }
        }
        let mut objects = Vec::with_capacity(groups.len());
        for (key, mut rows) in groups {
            // Сортировка устойчивая: из точек с одним временем первой
            // остаётся точка, что раньше во входе.
            rows.sort_by(|a, b| a[0].total_cmp(&b[0]));
            let mut object = Object {
                key,
                times: Vec::with_capacity(rows.len()),
                lats: Vec::with_capacity(rows.len()),
                lons: Vec::with_capacity(rows.len()),
                alts: Vec::with_capacity(rows.len()),
            };
            for [t, lat, lon, alt] in rows {
                if object.times.last().is_some_and(|&last| t <= last) {
                    continue;
                }
                object.times.push(t);
                object.lats.push(lat);
                object.lons.push(lon);
                object.alts.push(alt);
            }
            objects.push(object);
        }
        let start = objects
            .iter()
            .filter_map(|o| o.times.first().copied())
            .reduce(f64::min);
        let end = objects
            .iter()
            .filter_map(|o| o.times.last().copied())
            .reduce(f64::max);
        Self {
            objects,
            start,
            end,
        }
    }

    /// Объект по ключу.
    pub fn object(&self, key: &str) -> Option<&Object> {
        self.objects.iter().find(|o| o.key == key)
    }

    /// Объекты в момент t, None - весь трек, объект в конце пути.
    /// Объекта, который ещё не начал путь, в списке нет. Момент
    /// не число - список пуст.
    pub fn at(&self, t: Option<f64>) -> Vec<Position> {
        let mut out = Vec::new();
        for object in &self.objects {
            let (Some(&first), Some(&last)) = (object.times.first(), object.times.last()) else {
                continue;
            };
            let moment = t.unwrap_or(last);
            if moment.is_nan() || moment < first {
                continue;
            }
            let (i, (lat, lon)) = point(&object.times, &object.lats, &object.lons, moment);
            let mut path: Vec<(f64, f64)> = object
                .lats
                .iter()
                .zip(&object.lons)
                .take(i + 1)
                .map(|(&a, &b)| (a, b))
                .collect();
            if path.last() != Some(&(lat, lon)) {
                path.push((lat, lon));
            }
            let back = first.max(moment - HEADING_SPAN);
            let (_, (lat0, lon0)) = point(&object.times, &object.lats, &object.lons, back);
            let heading = if (lat0, lon0) != (lat, lon) {
                bearing(lat0, lon0, lat, lon)
            } else {
                f64::NAN
            };
            let (mut alt, mut alts) = (f64::NAN, None);
            if object.alts.iter().any(|h| !h.is_nan()) {
                let (_, (height, _)) = point(&object.times, &object.alts, &object.alts, moment);
                alt = height;
                let mut list: Vec<f64> = object.alts.iter().take(i + 1).copied().collect();
                if list.len() < path.len() {
                    list.push(height);
                }
                alts = Some(list);
            }
            out.push(Position {
                key: object.key.clone(),
                path,
                lat,
                lon,
                heading,
                alt,
                alts,
            });
        }
        out
    }
}

/// Положение объекта в момент t: номер последней точки не позже t
/// и точка на отрезке. Списки непустые и одной длины, t не раньше
/// первой точки.
fn point(times: &[f64], lats: &[f64], lons: &[f64], t: f64) -> (usize, (f64, f64)) {
    let last = times.len().saturating_sub(1);
    let i = times.partition_point(|&x| x <= t).saturating_sub(1);
    let at = |list: &[f64], k: usize| list.get(k).copied().unwrap_or(f64::NAN);
    if i >= last {
        return (last, (at(lats, last), at(lons, last)));
    }
    let span = at(times, i + 1) - at(times, i);
    let k = if span > 0.0 { (t - at(times, i)) / span } else { 0.0 };
    (
        i,
        (
            at(lats, i) + k * (at(lats, i + 1) - at(lats, i)),
            at(lons, i) + k * (at(lons, i + 1) - at(lons, i)),
        ),
    )
}
