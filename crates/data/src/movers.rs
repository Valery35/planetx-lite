// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Живые объекты без рельсов - суда и самолёты. Перенос `core/movers.py`
//! модуля.
//!
//! Объект - последнее положение со временем, скоростью, курсом, высотой
//! и скоростью подъёма. Между положениями объект идёт своим курсом со
//! своей скоростью не дольше extrapolate секунд, новое положение
//! принимает переходом [`BLEND`] секунд, как поезд [`crate::trains`].
//!
//! Объект с известной скоростью поворота (track_rate ADS-B или разность
//! курсов двух положений) идёт по дуге ([`arc`]), а не по прямой. Тангаж
//! модели самолёта - по подъёму и скорости, крен без данных -
//! координированный, по скорости поворота ([`Movers::attitude`]).
//!
//! Камера следом за объектом - [`chase`] и [`chase_on`].
//!
//! Время - секунды UTC (clock) и монотонные часы окна (now), их передаёт
//! вызывающий: в ядре часов нет.

use std::collections::{BTreeMap, BTreeSet};

pub use crate::trains::BLEND;
use crate::trains::{bearing, blend, destination, metres};

/// Метров в секунду в узле.
pub const KNOT: f64 = 1852.0 / 3600.0;
/// Метров в футе.
pub const FOOT: f64 = 0.3048;
/// Ускорение свободного падения, м/с².
pub const G: f64 = 9.81;
/// Скорость поворота по двум положениям не больше, °/с.
pub const TURN_MAX: f64 = 6.0;
/// Положения дальше друг от друга поворот не дают, секунды.
pub const TURN_GAP: f64 = 60.0;
/// Крен по скорости поворота не больше, градусы.
pub const ROLL_MAX: f64 = 45.0;
/// Камера следом не ближе к объекту, метры.
pub const CHASE_MIN: f64 = 50.0;
/// Проходов уточнения высоты над точкой взгляда в [`chase_on`].
pub const CHASE_PASSES: usize = 3;
/// Умолчания [`Movers`] модуля: ход по скорости и давность, секунды.
pub const EXTRAPOLATE: f64 = 60.0;
pub const STALE: f64 = 900.0;

/// Положение объекта: ключ, время (секунды UTC), точка, скорость м/с,
/// курс по земле, азимут корпуса (NaN - нет), высота м (NaN - на
/// поверхности), подъём м/с, крен (NaN - нет), скорость поворота °/с
/// (NaN - нет).
#[derive(Debug, Clone, PartialEq)]
pub struct Sample<K> {
    pub key: K,
    pub t: f64,
    pub lat: f64,
    pub lon: f64,
    pub speed: f64,
    pub course: f64,
    pub heading: f64,
    pub alt: f64,
    pub climb: f64,
    pub roll: f64,
    pub turn: f64,
}

impl<K> Sample<K> {
    /// Положение с умолчаниями модуля: скорость 0, курса, азимута,
    /// высоты и поворота нет, подъём и крен 0.
    pub fn new(key: K, t: f64, lat: f64, lon: f64) -> Self {
        Self {
            key,
            t,
            lat,
            lon,
            speed: 0.0,
            course: f64::NAN,
            heading: f64::NAN,
            alt: f64::NAN,
            climb: 0.0,
            roll: 0.0,
            turn: f64::NAN,
        }
    }
}

/// Точка на экране: широта, долгота, азимут (NaN - нет), высота (NaN -
/// на поверхности).
pub type Shown = (f64, f64, f64, f64);

/// Объект: последнее положение и переход к нему.
#[derive(Debug, Clone, PartialEq)]
pub struct Mover<K> {
    pub sample: Sample<K>,
    /// Точка на экране в начале перехода или None.
    pub blend_from: Option<Shown>,
    pub blend_at: f64,
}

/// Объекты на глобусе: положения, сведения, точка, азимут и высота на
/// момент. extrapolate - ход по своей скорости после положения не
/// дольше, секунды, stale - положение старше этого не показывается,
/// секунды.
#[derive(Debug, Clone)]
pub struct Movers<K, I> {
    pub extrapolate: f64,
    pub stale: f64,
    pub items: BTreeMap<K, Mover<K>>,
    pub info: BTreeMap<K, I>,
}

impl<K: Ord + Clone, I> Default for Movers<K, I> {
    fn default() -> Self {
        Self::new(EXTRAPOLATE, STALE)
    }
}

/// Точка, азимут и высота положения на момент clock без перехода.
fn ahead<K>(sample: &Sample<K>, clock: f64, extrapolate: f64) -> Shown {
    let elapsed = (clock - sample.t).max(0.0).min(extrapolate);
    let mut heading = if sample.heading.is_nan() { sample.course } else { sample.heading };
    let (mut lat, mut lon) = (sample.lat, sample.lon);
    if !sample.course.is_nan() && sample.speed > 0.0 && elapsed > 0.0 {
        let turn = if sample.turn.is_nan() { 0.0 } else { sample.turn };
        let (chord, run) = arc(sample.course, sample.speed, turn, elapsed);
        (lat, lon) = destination(lat, lon, chord, run);
        if !heading.is_nan() {
            heading = (heading + turn * elapsed).rem_euclid(360.0);
        }
    }
    let mut alt = sample.alt;
    if !alt.is_nan() {
        alt = (alt + sample.climb * elapsed).max(0.0);
    }
    (lat, lon, heading, alt)
}

/// Точка объекта на экране с переходом.
fn place_item<K>(item: &mut Mover<K>, now: f64, clock: f64, extrapolate: f64) -> Shown {
    let (mut lat, mut lon, mut heading, mut alt) = ahead(&item.sample, clock, extrapolate);
    if let Some(a) = item.blend_from {
        let k = (now - item.blend_at) / BLEND;
        if k >= 1.0 {
            item.blend_from = None;
        } else {
            let k = k.max(0.0);
            (lat, lon, heading) = blend((a.0, a.1, a.2), (lat, lon, heading), k);
            if !a.3.is_nan() && !alt.is_nan() {
                alt = a.3 + k * (alt - a.3);
            }
        }
    }
    (lat, lon, heading, alt)
}

/// (тангаж, крен) модели в градусах по положению: тангаж - по подъёму
/// и скорости, крен - данный или по скорости поворота, на поверхности
/// без данного крена - 0.
pub fn attitude<K>(sample: &Sample<K>) -> (f64, f64) {
    let mut pitch = 0.0;
    if !sample.alt.is_nan() && sample.speed > 1.0 {
        pitch = sample.climb.atan2(sample.speed).to_degrees();
    }
    let mut roll = sample.roll;
    if roll.is_nan() {
        let turn = if sample.turn.is_nan() { 0.0 } else { sample.turn };
        roll = if sample.alt.is_nan() { 0.0 } else { bank(turn, sample.speed) };
    }
    (pitch, roll)
}

impl<K: Ord + Clone, I> Movers<K, I> {
    pub fn new(extrapolate: f64, stale: f64) -> Self {
        Self { extrapolate, stale, items: BTreeMap::new(), info: BTreeMap::new() }
    }

    /// Количество объектов.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Новые положения. keep - ответ только части мира (рамка вида):
    /// объекты вне ответа остаются, пока не устареют.
    pub fn update(&mut self, samples: Vec<Sample<K>>, now: f64, clock: f64, keep: bool) {
        let mut seen = BTreeSet::new();
        for mut sample in samples {
            if clock - sample.t > self.stale {
                continue;
            }
            seen.insert(sample.key.clone());
            let Some(item) = self.items.get_mut(&sample.key) else {
                self.items.insert(sample.key.clone(), Mover { sample, blend_from: None, blend_at: 0.0 });
                continue;
            };
            if sample.t <= item.sample.t {
                continue;
            }
            if sample.turn.is_nan() {
                sample.turn = turn_between(&item.sample, &sample);
            }
            let shown = place_item(item, now, clock, self.extrapolate);
            item.sample = sample;
            item.blend_from = Some(shown);
            item.blend_at = now;
        }
        let stale = self.stale;
        self.items.retain(|key, item| {
            let old = clock - item.sample.t > stale;
            !old && (keep || seen.contains(key))
        });
    }

    /// (широта, долгота, азимут, высота) объекта key на экране или None.
    pub fn place(&mut self, key: &K, now: f64, clock: f64) -> Option<Shown> {
        let extrapolate = self.extrapolate;
        self.items.get_mut(key).map(|item| place_item(item, now, clock, extrapolate))
    }

    /// (тангаж, крен) модели объекта key в градусах или None, см.
    /// [`attitude`].
    pub fn attitude(&self, key: &K) -> Option<(f64, f64)> {
        self.items.get(key).map(|item| attitude(&item.sample))
    }

    /// [(ключ, широта, долгота, азимут, высота)] на момент, по порядку
    /// ключей.
    pub fn positions(&mut self, now: f64, clock: f64) -> Vec<(K, f64, f64, f64, f64)> {
        let extrapolate = self.extrapolate;
        self.items
            .iter_mut()
            .map(|(key, item)| {
                let (lat, lon, heading, alt) = place_item(item, now, clock, extrapolate);
                (key.clone(), lat, lon, heading, alt)
            })
            .collect()
    }
}

/// (азимут хорды, длина хорды) пути с постоянной скоростью speed м/с
/// и поворотом turn °/с за seconds секунд от курса course.
pub fn arc(course: f64, speed: f64, turn: f64, seconds: f64) -> (f64, f64) {
    let run = speed * seconds;
    let swing = (turn * seconds).to_radians();
    if swing.is_nan() || swing.abs() < 1e-6 {
        return (course, run);
    }
    let chord = 2.0 * run / swing * (swing / 2.0).sin();
    ((course + swing.to_degrees() / 2.0).rem_euclid(360.0), chord.abs())
}

/// Скорость поворота °/с по курсам двух положений или NaN.
pub fn turn_between<K>(old: &Sample<K>, new: &Sample<K>) -> f64 {
    let gap = new.t - old.t;
    if !(gap > 0.0 && gap <= TURN_GAP) || old.course.is_nan() || new.course.is_nan() || new.speed.is_nan() || new.speed <= 0.0 {
        return f64::NAN;
    }
    let delta = (new.course - old.course + 180.0).rem_euclid(360.0) - 180.0;
    (delta / gap).clamp(-TURN_MAX, TURN_MAX)
}

/// Координированный крен по скорости поворота °/с и скорости м/с.
pub fn bank(turn: f64, speed: f64) -> f64 {
    (turn.to_radians() * speed / G).atan().to_degrees().clamp(-ROLL_MAX, ROLL_MAX)
}

/// Поза камеры следом за объектом на высоте height над поверхностью:
/// (широта и долгота точки взгляда, азимут взгляда, расстояние до неё).
/// Камера в reach метрах позади объекта по курсу heading, наклон tilt
/// от отвеса, градусы. Объект лежит на луче взгляда: точка взгляда -
/// продолжение луча через объект до поверхности, впереди на
/// height · tan(tilt). Азимут берётся в самой точке взгляда - курс по
/// дуге большого круга там другой, и объект уходил бы вбок.
pub fn chase(lat: f64, lon: f64, heading: f64, height: f64, reach: f64, tilt: f64) -> Shown {
    let height = if height.is_nan() { 0.0 } else { height.max(0.0) };
    let tilt = tilt.clamp(0.0, 85.0);
    let reach = reach.max(CHASE_MIN);
    let ahead = height * tilt.to_radians().tan();
    if ahead.is_nan() || ahead < 1.0 {
        return (lat, lon, heading, reach);
    }
    let (look_lat, look_lon) = destination(lat, lon, heading, ahead);
    let look = (bearing(look_lat, look_lon, lat, lon) + 180.0).rem_euclid(360.0);
    (look_lat, look_lon, look, reach + height / tilt.to_radians().cos())
}

/// [`chase`] для объекта на высоте alt (как у ground, высоты на экране)
/// над неровной поверхностью ground(широта, долгота): высота берётся над
/// рельефом в точке взгляда, а не под объектом.
pub fn chase_on(lat: f64, lon: f64, heading: f64, alt: f64, ground: &dyn Fn(f64, f64) -> f64, reach: f64, tilt: f64) -> Shown {
    let (mut look_lat, mut look_lon) = (lat, lon);
    let mut out = (lat, lon, heading, reach.max(CHASE_MIN));
    for _ in 0..CHASE_PASSES {
        let height = alt - ground(look_lat, look_lon);
        out = chase(lat, lon, heading, if height.is_nan() { 0.0 } else { height }, reach, tilt);
        (look_lat, look_lon) = (out.0, out.1);
    }
    out
}

/// Ближе ли точка reach метров.
pub fn near(lat: f64, lon: f64, other_lat: f64, other_lon: f64, reach: f64) -> bool {
    metres(lat, lon, other_lat, other_lon) <= reach
}
