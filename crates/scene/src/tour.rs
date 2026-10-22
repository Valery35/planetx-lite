// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Тур: перелёты по остановкам с паузой на каждой, как в Google Earth.
//! Перенос `core/tour.py` модуля.
//!
//! Тур - цепочка перелётов van Wijk-Nuij ([`globe_core::flight`]).
//! Первый идёт от позы камеры к первой остановке, каждый следующий -
//! от остановки к остановке. После прилёта камера стоит паузу. Поза -
//! функция времени тура, а не часов на стене. Навигатор проигрывает тур
//! как обычный перелёт, пауза и продолжение сдвигают начало отсчёта.
//!
//! Остановка-путь, как у Google Earth: камера подлетает к началу линии
//! и едет вдоль неё с постоянной скоростью, азимут следует направлению
//! пути, наклон постоянный. Точки и многоугольники - остановки
//! по центру.
//!
//! Записанный тур - позы камеры по времени, как запись тура Google
//! Earth, с перелётами с подъёмом и изменениями меток по ходу
//! (gx:AnimatedUpdate KML).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use globe_core::ellipsoid::{EARTH, Ellipsoid};
use globe_core::flight::Flight;
use globe_core::navigation::Pose;
use serde_json::{Map, Value};

use crate::features::{MAX_POINTS, STEP, densify};
use crate::lookat::{Layers, number_text};
use crate::when::Time;

/// Секунд на остановке по умолчанию.
pub const PAUSE: f64 = 3.0;
/// Пределы множителя скорости перелёта к остановке.
pub const SPEED_RANGE: (f64, f64) = (0.25, 4.0);
/// Подпись остановки на кадре проявляется и гаснет за столько секунд.
pub const CAPTION_FADE: f64 = 0.6;
/// Наклон камеры на пути.
pub const PATH_TILT: f64 = 60.0;
/// Длина пути / расстояние камеры. Так земля бежит по экрану с одной
/// скоростью на пути любой длины, проезд пути длиннее 2.4 км идёт 32 с.
pub const PATH_VIEW: f64 = 8.0;
pub const PATH_MIN_DISTANCE: f64 = 300.0;
pub const PATH_MAX_DISTANCE: f64 = 20000.0;
/// Расстояний камеры в секунду.
pub const PATH_SPEED: f64 = 0.25;
/// Азимут берётся по хорде пути длиной в расстояние камеры, повороты
/// линии не дёргают камеру.
pub const PATH_LOOK: f64 = 0.5;
/// Опорных точек пути записанного облёта в окне свойств.
pub const KEY_POINTS: usize = 20;
/// Пределы множителя высоты облёта в окне свойств.
pub const HEIGHT_FACTORS: (f64, f64) = (0.25, 4.0);
/// Поле зрения перелёта по умолчанию, градусы.
pub const FOV_Y: f64 = 45.0;
/// Свойства меток, которые меняет gx:AnimatedUpdate: видимость сразу,
/// числа, цвета и вершины - плавно за время изменения.
pub const UPDATE_KEYS: [&str; 6] = ["visible", "color", "fill", "icon_scale", "width", "points"];

/// Поза записанного тура: время, широта, долгота, расстояние, азимут,
/// наклон.
pub type Sample = [f64; 6];

/// Изменение меток записанного тура: начало t, длительность d, id меток
/// KML и новые значения свойств [`UPDATE_KEYS`].
#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub t: f64,
    pub d: f64,
    pub ids: Vec<String>,
    pub set: Map<String, Value>,
}

/// Начальный азимут дуги большого круга, градусы от севера.
pub fn bearing(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dl = (lon2 - lon1).to_radians();
    let x = dl.sin() * p2.cos();
    let y = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dl.cos();
    x.atan2(y).to_degrees().rem_euclid(360.0)
}

/// Проезд вдоль линии.
#[derive(Debug, Clone)]
pub struct PathMotion {
    /// Сгущённые точки пути.
    pub points: Vec<(f64, f64)>,
    /// Расстояние от начала до каждой точки, метры.
    pub s: Vec<f64>,
    pub length: f64,
    /// Половина хорды для азимута, метры.
    pub look: f64,
}

impl PathMotion {
    /// Точка пути на расстоянии s от начала.
    pub fn point_at(&self, s: f64) -> (f64, f64) {
        let s = s.max(0.0).min(self.length);
        if self.points.len() < 2 || self.s.len() != self.points.len() {
            return self.points.first().copied().unwrap_or((0.0, 0.0));
        }
        let i = self.s.partition_point(|v| *v <= s).saturating_sub(1).min(self.s.len() - 2);
        let span = self.s[i + 1] - self.s[i];
        let k = if span > 0.0 { (s - self.s[i]) / span } else { 0.0 };
        let (a, b) = (self.points[i], self.points[i + 1]);
        (a.0 + k * (b.0 - a.0), a.1 + k * (b.1 - a.1))
    }

    pub fn heading_at(&self, s: f64) -> f64 {
        let a = self.point_at(s - self.look);
        let b = self.point_at(s + self.look);
        if a == b { 0.0 } else { bearing(a.0, a.1, b.0, b.1) }
    }
}

/// Записанное движение камеры.
#[derive(Debug, Clone)]
pub struct RecordedMotion {
    /// Позы по времени, начало времени 0.
    pub samples: Vec<Sample>,
    /// Номера поз, к которым запись летит перелётом с подъёмом.
    pub bounce: BTreeSet<usize>,
    /// Изменения меток по ходу записи, их применяет окно глобуса.
    pub updates: Vec<Update>,
    lons: Vec<f64>,
    headings: Vec<f64>,
    log_distance: Vec<f64>,
    flights: HashMap<usize, Flight>,
}

/// Движение остановки после прилёта.
#[derive(Debug, Clone)]
pub enum Motion {
    /// Камера стоит.
    Still,
    /// Камера едет вдоль линии.
    Path(PathMotion),
    /// Камера проигрывает запись.
    Recorded(RecordedMotion),
}

/// Остановка тура: название и поза в конце перелёта.
///
/// glide - секунд движения после прилёта. time - время остановки для
/// шкалы времени: время вида метки, иначе время самой метки.
/// description - описание метки, его показывает панель тура. layers -
/// слои вида метки или None. pause - своя пауза на остановке, секунд,
/// None - пауза тура. speed - множитель скорости перелёта к остановке,
/// больше - быстрее.
#[derive(Debug, Clone)]
pub struct Stop {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub distance: f64,
    pub heading: f64,
    pub tilt: f64,
    pub glide: f64,
    pub time: Option<Time>,
    pub description: String,
    pub layers: Option<Layers>,
    pub pause: Option<f64>,
    pub speed: f64,
    pub motion: Motion,
}

/// Фазы np.unwrap: скачки больше π убираются сдвигом на 2π.
fn unwrap(values: &[f64]) -> Vec<f64> {
    let pi = std::f64::consts::PI;
    let mut out = Vec::with_capacity(values.len());
    let mut correction = 0.0;
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            let dd = v - values[i - 1];
            let mut ddmod = (dd + pi).rem_euclid(2.0 * pi) - pi;
            if ddmod == -pi && dd > 0.0 {
                ddmod = pi;
            }
            if dd.abs() >= pi {
                correction += ddmod - dd;
            }
        }
        out.push(v + correction);
    }
    out
}

impl Stop {
    /// Остановка, где камера стоит.
    pub fn new(name: &str, lat: f64, lon: f64, distance: f64, heading: f64, tilt: f64) -> Self {
        Self {
            name: name.into(),
            lat,
            lon,
            distance,
            heading,
            tilt,
            glide: 0.0,
            time: None,
            description: String::new(),
            layers: None,
            pause: None,
            speed: 1.0,
            motion: Motion::Still,
        }
    }

    /// Остановка-путь: проезд вдоль линии (широта, долгота) с наклоном
    /// tilt.
    pub fn path(name: &str, points: &[(f64, f64)], tilt: f64) -> Self {
        let pts = densify(points, false, STEP, MAX_POINTS);
        let mut s = vec![0.0];
        let units: Vec<[f64; 3]> = pts
            .iter()
            .map(|p| {
                let (la, lo) = (p.0.to_radians(), p.1.to_radians());
                [la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin()]
            })
            .collect();
        for w in units.windows(2) {
            let dot = (w[0][0] * w[1][0] + w[0][1] * w[1][1] + w[0][2] * w[1][2]).clamp(-1.0, 1.0);
            let last = s.last().copied().unwrap_or(0.0);
            s.push(last + dot.acos() * EARTH.a);
        }
        let length = s.last().copied().unwrap_or(0.0);
        let distance = (length / PATH_VIEW).clamp(PATH_MIN_DISTANCE, PATH_MAX_DISTANCE);
        let glide = length / (PATH_SPEED * distance);
        let motion = PathMotion { points: pts, s, length, look: PATH_LOOK * distance };
        let (lat, lon) = motion.point_at(0.0);
        let heading = motion.heading_at(0.0);
        let mut stop = Self::new(name, lat, lon, distance, heading, tilt);
        stop.glide = glide;
        stop.motion = Motion::Path(motion);
        stop
    }

    /// Записанное движение камеры, как запись тура Google Earth.
    ///
    /// samples - позы с началом времени 0. Тур подлетает к первой позе,
    /// как к любой остановке, и проигрывает запись вместо проезда. Между
    /// позами широта и наклон идут линейно, долгота и азимут - без скачка
    /// через 180° и 360°, расстояние - по логарифму, так приближение
    /// ровное. bounce - номера поз, к которым запись летит перелётом
    /// с подъёмом (gx:flyToMode bounce KML), van Wijk-Nuij за время
    /// отрезка. updates - изменения меток по ходу записи ([`animated`]).
    pub fn recorded(name: &str, samples: &[Sample], bounce: &[usize], updates: Vec<Update>) -> Self {
        let mut data = samples.to_vec();
        data.sort_by(|a, b| a[0].total_cmp(&b[0]));
        let Some(first) = data.first().copied() else {
            return Self::new(name, 0.0, 0.0, 1.0, 0.0, 0.0);
        };
        for row in &mut data {
            row[0] -= first[0];
        }
        let lons = unwrap(&data.iter().map(|r| r[2].to_radians()).collect::<Vec<_>>()).into_iter().map(f64::to_degrees).collect();
        let headings =
            unwrap(&data.iter().map(|r| r[4].to_radians()).collect::<Vec<_>>()).into_iter().map(f64::to_degrees).collect();
        let log_distance = data.iter().map(|r| r[3].max(1.0).ln()).collect();
        let bounce: BTreeSet<usize> = bounce.iter().copied().collect();
        let e = Ellipsoid::wgs84();
        let pose = |r: &Sample| Pose::new(r[1], r[2], r[3], r[4], r[5]);
        let flights = bounce
            .iter()
            .filter(|j| **j >= 1 && **j < data.len())
            .map(|j| (*j, Flight::new(&e, &pose(&data[j - 1]), pose(&data[*j]), FOV_Y)))
            .collect();
        let glide = data.last().map_or(0.0, |r| r[0]);
        let mut stop = Self::new(name, first[1], first[2], first[3], first[4], first[5]);
        stop.glide = glide;
        stop.motion = Motion::Recorded(RecordedMotion { samples: data, bounce, updates, lons, headings, log_distance, flights });
        stop
    }

    /// Длина пути у остановки-пути, иначе 0.
    pub fn length(&self) -> f64 {
        match &self.motion {
            Motion::Path(p) => p.length,
            _ => 0.0,
        }
    }

    /// Поза через t секунд после прилёта.
    pub fn pose_at(&self, t: f64) -> Pose {
        match &self.motion {
            Motion::Still => Pose::new(self.lat, self.lon, self.distance, self.heading, self.tilt),
            Motion::Path(path) => {
                let share = if self.glide != 0.0 { (t / self.glide).clamp(0.0, 1.0) } else { 1.0 };
                let s = share * path.length;
                let (lat, lon) = path.point_at(s);
                Pose::new(lat, lon, self.distance, path.heading_at(s), self.tilt)
            }
            Motion::Recorded(rec) => self.recorded_pose(rec, t),
        }
    }

    /// Поза в конце движения.
    pub fn end_pose(&self) -> Pose {
        self.pose_at(self.glide)
    }

    fn recorded_pose(&self, rec: &RecordedMotion, t: f64) -> Pose {
        let count = rec.samples.len();
        if count == 0 {
            return Pose::new(self.lat, self.lon, self.distance, self.heading, self.tilt);
        }
        let t = t.max(0.0).min(self.glide);
        let i = rec.samples.partition_point(|r| r[0] <= t).saturating_sub(1).min(count - 1);
        let j = (i + 1).min(count - 1);
        let span = rec.samples[j][0] - rec.samples[i][0];
        let k = if span > 0.0 { (t - rec.samples[i][0]) / span } else { 0.0 };
        if j != i
            && span > 0.0
            && let Some(flight) = rec.flights.get(&j)
        {
            return flight.pose_at(k * flight.duration);
        }
        let mix = |values: &dyn Fn(usize) -> f64| values(i) + k * (values(j) - values(i));
        let lon = (mix(&|n| rec.lons[n]) + 180.0).rem_euclid(360.0) - 180.0;
        Pose::new(
            mix(&|n| rec.samples[n][1]),
            lon,
            mix(&|n| rec.log_distance[n]).exp(),
            mix(&|n| rec.headings[n]).rem_euclid(360.0),
            mix(&|n| rec.samples[n][5]),
        )
    }
}

/// Значение на доле share пути от old к new.
fn mix_value(old: Option<&Value>, new: &Value, share: f64) -> Value {
    let Some(old) = old.filter(|_| share < 1.0) else {
        return new.clone();
    };
    let number = |v: &Value| match v {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        _ => None,
    };
    if let (Some(a), Some(b)) = (number(old), number(new)) {
        return Value::from(a + (b - a) * share);
    }
    if let (Value::Array(a), Value::Array(b)) = (old, new)
        && a.len() == b.len()
    {
        let mixed: Vec<Value> = a.iter().zip(b).map(|(x, y)| mix_value(Some(x), y, share)).collect();
        if b.iter().all(|v| v.is_i64() || v.is_u64()) {
            return Value::from(
                mixed.iter().map(|v| v.as_f64().map_or(Value::Null, |f| Value::from(f.round_ties_even() as i64))).collect::<Vec<_>>(),
            );
        }
        return Value::Array(mixed);
    }
    if share > 0.0 { new.clone() } else { old.clone() }
}

/// Свойства меток: id метки KML - ключ свойства - значение.
pub type State = BTreeMap<String, Map<String, Value>>;

/// Свойства меток на время t записи: base - свойства до изменений,
/// updates - изменения. Видимость меняется в начале изменения, прочее -
/// плавно.
pub fn animated(updates: &[Update], t: f64, base: &State) -> State {
    let mut state = base.clone();
    let mut sorted: Vec<&Update> = updates.iter().collect();
    sorted.sort_by(|a, b| a.t.total_cmp(&b.t));
    for update in sorted {
        if update.t > t {
            break;
        }
        let share = if update.d <= 0.0 { 1.0 } else { ((t - update.t) / update.d).min(1.0) };
        for target in &update.ids {
            let Some(values) = state.get_mut(target) else {
                continue;
            };
            for (key, value) in &update.set {
                if !UPDATE_KEYS.contains(&key.as_str()) {
                    continue;
                }
                let next = if key == "visible" {
                    Value::Bool(truthy(value))
                } else {
                    mix_value(values.get(key), value, share)
                };
                values.insert(key.clone(), next);
            }
        }
    }
    state
}

/// Истинность значения, как bool() в Python.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Номера опорных поз записи для правки пути: count поз поровну
/// по длине пути точки взгляда, первая и последняя всегда.
pub fn key_indices(samples: &[Sample], count: usize) -> Vec<usize> {
    let n = samples.len();
    if n <= count {
        return (0..n).collect();
    }
    if count == 0 {
        return Vec::new();
    }
    let lons = unwrap(&samples.iter().map(|r| r[2].to_radians()).collect::<Vec<_>>());
    let mut s = vec![0.0];
    for i in 1..n {
        let dlat = samples[i][1] - samples[i - 1][1];
        let dlon = (lons[i] - lons[i - 1]).to_degrees() * samples[i][1].to_radians().cos();
        let last = s[i - 1];
        s.push(last + dlat.hypot(dlon));
    }
    if s[n - 1] <= 0.0 || s[n - 1].is_nan() {
        // Камера стояла на месте и вращалась: опорные - по времени.
        s = samples.iter().map(|r| r[0] - samples[0][0]).collect();
    }
    let total = s[n - 1];
    let step = if count > 1 { total / (count - 1) as f64 } else { 0.0 };
    let mut picks: Vec<usize> = (0..count)
        .map(|k| {
            let target = if k + 1 == count { total } else { k as f64 * step };
            let right = s.partition_point(|v| *v < target).clamp(1, n - 1);
            if target - s[right - 1] <= s[right] - target { right - 1 } else { right }
        })
        .collect();
    picks[0] = 0;
    picks[count - 1] = n - 1;
    let set: BTreeSet<usize> = picks.into_iter().collect();
    set.into_iter().collect()
}

/// Линейная интерполяция np.interp: x вне узлов - значение крайнего.
fn interp(x: f64, xp: &[f64], fp: &[f64]) -> f64 {
    let (Some(first), Some(last)) = (xp.first(), xp.last()) else {
        return 0.0;
    };
    if x <= *first || xp.len() < 2 {
        return fp[0];
    }
    if x >= *last {
        return fp[fp.len() - 1];
    }
    let j = xp.partition_point(|v| *v <= x).clamp(1, xp.len() - 1);
    let (x0, x1, y0, y1) = (xp[j - 1], xp[j], fp[j - 1], fp[j]);
    if x1 == x0 { y1 } else { y0 + (x - x0) * (y1 - y0) / (x1 - x0) }
}

/// Записанный тур после правки пути.
///
/// keys - номера опорных поз, points - их новые точки взгляда (широта,
/// долгота). Позы между опорными сдвигаются на сдвиг соседних опорных,
/// смешанный по номеру позы, так путь меняется плавно. factor -
/// множитель расстояния камеры, то есть высоты облёта. Время, азимут
/// и наклон прежние.
pub fn edit_samples(samples: &[Sample], keys: &[usize], points: &[(f64, f64)], factor: f64) -> Vec<Sample> {
    let mut data = samples.to_vec();
    let pairs: Vec<(usize, (f64, f64))> =
        keys.iter().zip(points).filter(|(k, _)| **k < data.len()).map(|(k, p)| (*k, *p)).collect();
    let xp: Vec<f64> = pairs.iter().map(|(k, _)| *k as f64).collect();
    let dlat: Vec<f64> = pairs.iter().map(|(k, p)| p.0 - samples[*k][1]).collect();
    let dlon: Vec<f64> = pairs.iter().map(|(k, p)| p.1 - samples[*k][2]).collect();
    for (i, row) in data.iter_mut().enumerate() {
        if !xp.is_empty() {
            row[1] += interp(i as f64, &xp, &dlat);
            row[2] += interp(i as f64, &xp, &dlon);
        }
        row[1] = row[1].clamp(-89.9, 89.9);
        row[2] = (row[2] + 180.0).rem_euclid(360.0) - 180.0;
        row[3] *= factor;
    }
    data
}

/// Прореживание записи: на стоянке остаются первая и последняя поза,
/// время между ними сохраняется.
pub fn thin(samples: &[Sample]) -> Vec<Sample> {
    let mut out: Vec<Sample> = Vec::new();
    for sample in samples {
        let pose = &sample[1..];
        let n = out.len();
        if n >= 2 && out[n - 1][1..] == *pose && out[n - 2][1..] == *pose {
            out[n - 1] = *sample;
        } else {
            out.push(*sample);
        }
    }
    out
}

/// Темп остановки: пауза или None, скорость.
pub type Pace = (Option<f64>, f64);

/// Темп остановки в поле файла меток и KML: «пауза;скорость», пауза
/// пустая - пауза тура. Пустая строка - темп тура.
pub fn pace_text(pace: Option<Pace>) -> String {
    let Some((pause, speed)) = pace else {
        return String::new();
    };
    if pause.is_none() && (speed - 1.0).abs() < 1e-9 {
        return String::new();
    }
    format!("{};{}", pause.map_or(String::new(), number_text), number_text(speed))
}

/// Темп остановки из поля или None.
pub fn pace_parse(text: Option<&str>) -> Option<Pace> {
    let (head, tail) = text.unwrap_or_default().split_once(';')?;
    let pause = if head.trim().is_empty() { None } else { Some(head.trim().parse::<f64>().ok()?) };
    let speed = if tail.trim().is_empty() { 1.0 } else { tail.trim().parse::<f64>().ok()? };
    let pause = pause.filter(|p| *p >= 0.0 || p.is_nan());
    let speed = speed.clamp(SPEED_RANGE.0, SPEED_RANGE.1);
    if pause.is_none() && speed == 1.0 {
        return None;
    }
    Some((pause, speed))
}

/// Время тура для панели: «м:сс», с часами - «ч:мм:сс».
pub fn clock(seconds: f64) -> String {
    let total = (seconds.max(0.0) + 0.5).floor() as u64;
    let (hours, rest) = (total / 3600, total % 3600);
    let (minutes, secs) = (rest / 60, rest % 60);
    if hours > 0 { format!("{hours}:{minutes:02}:{secs:02}") } else { format!("{minutes}:{secs:02}") }
}

/// Перелёты от позы start по остановкам stops с паузой pause.
#[derive(Debug, Clone)]
pub struct Tour {
    pub stops: Vec<Stop>,
    pub pause: f64,
    /// Время начала перелёта к каждой остановке.
    pub starts: Vec<f64>,
    /// Время прилёта и начала проезда.
    pub glides: Vec<f64>,
    /// Время, когда камера встала на остановке.
    pub arrivals: Vec<f64>,
    pub flights: Vec<Flight>,
    /// Пауза на каждой остановке.
    pub pauses: Vec<f64>,
    /// Множитель времени перелёта к остановке.
    pub speeds: Vec<f64>,
    pub duration: f64,
    pub start: Pose,
}

impl Tour {
    /// Тур с полем зрения перелёта [`FOV_Y`].
    pub fn new(start: &Pose, stops: Vec<Stop>, pause: f64) -> Self {
        Self::with_fov(start, stops, pause, FOV_Y)
    }

    pub fn with_fov(start: &Pose, stops: Vec<Stop>, pause: f64, fov_y: f64) -> Self {
        let e = Ellipsoid::wgs84();
        let pause = pause.max(0.0);
        let mut tour = Self {
            stops: Vec::new(),
            pause,
            starts: Vec::new(),
            glides: Vec::new(),
            arrivals: Vec::new(),
            flights: Vec::new(),
            pauses: Vec::new(),
            speeds: Vec::new(),
            duration: 0.0,
            start: start.clone(),
        };
        let mut t = 0.0;
        let mut pose = start.clone();
        for stop in &stops {
            let end = Pose::new(stop.lat, stop.lon, stop.distance, stop.heading, stop.tilt);
            let flight = Flight::new(&e, &pose, end, fov_y);
            let speed = if stop.speed == 0.0 || stop.speed.is_nan() { 1.0 } else { stop.speed };
            let speed = speed.clamp(SPEED_RANGE.0, SPEED_RANGE.1);
            let own = stop.pause.map_or(pause, |p| p.max(0.0));
            tour.starts.push(t);
            t += flight.duration / speed;
            tour.flights.push(flight);
            tour.speeds.push(speed);
            tour.pauses.push(own);
            tour.glides.push(t);
            t += stop.glide;
            tour.arrivals.push(t);
            t += own;
            pose = stop.end_pose();
        }
        tour.stops = stops;
        tour.duration = t;
        tour
    }

    /// Подпись остановки на кадре на время t: (номер остановки,
    /// непрозрачность 0..1) или None. Подпись видна от прилёта до начала
    /// следующего перелёта, проявляется и гаснет за [`CAPTION_FADE`]
    /// секунд, в полёте её нет.
    pub fn caption_at(&self, t: f64) -> Option<(usize, f64)> {
        let i = self.index_at(t)?;
        let begin = self.glides[i];
        let end = self.arrivals[i] + self.pauses[i];
        if !(begin <= t && t <= end) || end <= begin {
            return None;
        }
        let fade = CAPTION_FADE.min((end - begin) / 3.0);
        let alpha = if fade > 0.0 { 1f64.min((t - begin) / fade).min((end - t) / fade) } else { 1.0 };
        Some((i, alpha.max(0.0)))
    }

    /// Номер остановки, к которой летит или на которой стоит камера,
    /// None без остановок.
    pub fn index_at(&self, t: f64) -> Option<usize> {
        if self.stops.is_empty() {
            return None;
        }
        Some(self.starts.partition_point(|s| *s <= t).saturating_sub(1))
    }

    /// Время тура конца паузы остановки, на которой камера стоит
    /// в момент t, или None - камера летит, едет или тур кончился.
    pub fn pause_end(&self, t: f64) -> Option<f64> {
        let i = self.index_at(t)?;
        if t < self.arrivals[i] {
            return None;
        }
        let end = self.arrivals[i] + self.pauses[i];
        (t < end).then_some(end)
    }

    /// Стоит ли камера на остановке, а не летит и не едет.
    pub fn arrived(&self, t: f64) -> bool {
        self.index_at(t).is_some_and(|i| t >= self.arrivals[i])
    }

    pub fn pose_at(&self, t: f64) -> Pose {
        let Some(i) = self.index_at(t) else {
            return self.start.clone();
        };
        if t < self.glides[i] {
            return self.flights[i].pose_at((t - self.starts[i]) * self.speeds[i]);
        }
        self.stops[i].pose_at(t - self.glides[i])
    }
}
