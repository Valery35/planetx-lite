// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Спутники по элементам CelesTrak: группы, расчёт и показ. Перенос
//! `core/satellites.py` модуля.
//!
//! Положение - модель SGP4 ([`crate::sgp4`]), поворот TEME в систему Земли
//! по звёздному времени. SGP4 считается в моменты сетки шагом [`SAMPLE`]
//! модельного времени, между ними положение берётся кубическим сплайном
//! Эрмита по положениям и скоростям концов. Ошибка сплайна на шаге 60 с
//! у низкой орбиты - метры.
//!
//! Правила CelesTrak (celestrak.org/usage-policy.php, 6 октября 2026
//! года) - группа скачивается не чаще одного обновления, данные
//! обновляются раз в 2 часа, после ответа с ошибкой запросы прекращаются.
//! Группа просится не чаще раза в [`REFETCH`].
//!
//! Эллипсоид - WGS84 из [`crate::subsurface`].

use std::collections::HashSet;
use std::sync::Arc;

use crate::sgp4::{self, Cache, Element, Integrator, Orbits, Sat, State};
use crate::subsurface::{A, B, ecef_to_geodetic};

/// Адрес CSV группы CelesTrak, `{group}` - ключ группы.
pub const FEED: &str = "https://celestrak.org/NORAD/elements/gp.php?GROUP={group}&FORMAT=csv";
/// Подпись источника: название и адрес.
pub const ATTRIBUTION: (&str, &str) = ("CelesTrak", "https://celestrak.org");
/// Не чаще этого группа просится заново, с.
pub const REFETCH: f64 = 2.0 * 3600.0;
/// Секунд модельного времени между расчётами SGP4.
pub const SAMPLE: f64 = 60.0;
/// Вращение Земли, рад/с.
pub const OMEGA: f64 = 7.292115e-5;
/// Спутников за проход цикла событий, около 4 мс.
pub const INIT_BUDGET: usize = 150;
/// Дальше этого от эпохи элементов спутник не показывается, сутки:
/// точность SGP4 падает до километров в сутки, а резонансы
/// геостационарных и навигационных спутников интегрируются шагами
/// по 12 часов от эпохи.
pub const MAX_AGE: f64 = 30.0;
/// Код «элементы старше [`MAX_AGE`]» рядом с кодами ошибок SGP4.
pub const STALE: u8 = 7;

/// Группы CelesTrak строки «Спутники»: ключ запроса и цвет точки.
/// Названия и подсказки - в интерфейсе, здесь только данные.
pub const GROUPS: [(&str, [u8; 3]); 9] = [
    ("stations", [255, 255, 255]),
    ("visual", [255, 224, 102]),
    ("gnss", [102, 204, 255]),
    ("weather", [153, 255, 153]),
    ("resource", [255, 170, 102]),
    ("science", [204, 153, 255]),
    ("geo", [255, 102, 153]),
    ("starlink", [170, 170, 170]),
    ("oneweb", [128, 160, 200]),
];
/// Группы, включённые по умолчанию.
pub const DEFAULT_GROUPS: [&str; 1] = ["stations"];
/// Размер точки в логических пикселях у больших групп и станций.
pub const SIZES: [(&str, f32); 3] = [("stations", 8.0), ("starlink", 3.0), ("oneweb", 3.0)];
/// Размер точки остальных групп.
pub const SIZE: f32 = 5.0;

/// Точек на виток орбиты и на след.
pub const ORBIT_POINTS: usize = 180;
/// Орбита и след пересчитываются, когда момент сдвинулся на столько
/// секунд: за 30 с низкий спутник проходит 230 км, полоса витка - сотни.
pub const ORBIT_REFRESH: f64 = 30.0;

type V3 = [f64; 3];

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

/// Адрес CSV группы CelesTrak.
pub fn feed(group: &str) -> String {
    FEED.replace("{group}", group)
}

/// Цвет точки группы, белый у неизвестной.
pub fn color(group: &str) -> [u8; 3] {
    GROUPS.iter().find(|(k, _)| *k == group).map_or([255, 255, 255], |(_, c)| *c)
}

/// Размер точки группы.
pub fn size(group: &str) -> f32 {
    SIZES.iter().find(|(k, _)| *k == group).map_or(SIZE, |(_, s)| *s)
}

/// Юлианская дата по секундам UTC от 1970 года.
pub fn jd_now(seconds: f64) -> f64 {
    sgp4::unix_jd(seconds)
}

/// Положение и скорость TEME (км, км/с) в систему Земли, метры и м/с.
/// Скорость - относительно вращающейся Земли.
pub fn to_ecef(r: V3, v: V3, jd: f64) -> (V3, V3) {
    let p = scale(sgp4::teme_to_ecef(r, jd), 1000.0);
    let mut w = scale(sgp4::teme_to_ecef(v, jd), 1000.0);
    w[0] += OMEGA * p[1];
    w[1] -= OMEGA * p[0];
    (p, w)
}

/// Кубический сплайн Эрмита между (p0, v0) и (p1, v1), шаг h секунд,
/// доля шага s от 0 до 1.
pub fn hermite(p0: V3, v0: V3, p1: V3, v1: V3, h: f64, s: f64) -> V3 {
    let s2 = s * s;
    let s3 = s2 * s;
    let (a, b, c, d) = (2.0 * s3 - 3.0 * s2 + 1.0, s3 - 2.0 * s2 + s, -2.0 * s3 + 3.0 * s2, s3 - s2);
    std::array::from_fn(|k| a * p0[k] + b * h * v0[k] + c * p1[k] + d * h * v1[k])
}

/// Закрыта ли точка Землёй для глаза eye: отрезок от глаза до точки
/// пересекает эллипсоид. Точки - метры ECEF.
pub fn hidden(eye: V3, points: &[V3]) -> Vec<bool> {
    let k = [1.0 / A, 1.0 / A, 1.0 / B];
    let e = [eye[0] * k[0], eye[1] * k[1], eye[2] * k[2]];
    let qc = dot(e, e) - 1.0;
    if qc <= 0.0 {
        return vec![false; points.len()];
    }
    points
        .iter()
        .map(|p| {
            let d = [p[0] * k[0] - e[0], p[1] * k[1] - e[1], p[2] * k[2] - e[2]];
            let qa = dot(d, d);
            let qb = 2.0 * dot(d, e);
            let disc = qb * qb - 4.0 * qa * qc;
            let s = (-qb - disc.max(0.0).sqrt()) / (2.0 * qa);
            disc > 0.0 && s > 0.0 && s < 1.0
        })
        .collect()
}

/// Высота точки над эллипсоидом, метры.
pub fn height(point: V3) -> f64 {
    ecef_to_geodetic(point)[2]
}

/// Минуты от эпохи спутника до jd, не дальше [`MAX_AGE`] суток,
/// и признак «дальше».
fn since(sat: &Sat, jd: f64) -> (f64, bool) {
    let tsince = (jd - sgp4::JD_EPOCH0 - sat.epoch) * 1440.0;
    let limit = MAX_AGE * 1440.0;
    (tsince.clamp(-limit, limit), tsince.abs() > limit)
}

/// Один спутник на юлианскую дату jd от эпохи, без запомненного
/// интегратора: состояние и признак «элементы старше [`MAX_AGE`]».
fn single(sat: &Sat, jd: f64) -> (State, bool) {
    let (tsince, stale) = since(sat, jd);
    (sgp4::propagate_one(sat, tsince, &mut Integrator::new(sat)), stale)
}

/// Положения и скорости в системе Земли в момент шага сетки и коды.
#[derive(Debug, Clone, PartialEq)]
struct Sample {
    p: Vec<V3>,
    w: Vec<V3>,
    error: Vec<u8>,
}

/// Сведения о спутнике: название, номер NORAD, группа, высота км,
/// скорость км/с в TEME, возраст элементов в сутках, код ошибки.
#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    pub name: String,
    pub number: i64,
    pub group: String,
    pub height: f64,
    pub speed: f64,
    pub age: f64,
    pub error: u8,
}

/// Спутники выбранных групп: элементы, начальные величины, положения
/// на момент.
#[derive(Debug, Clone, Default)]
pub struct Swarm {
    /// Группы по порядку добавления.
    groups: Vec<(String, Orbits)>,
    pub names: Vec<String>,
    pub numbers: Vec<i64>,
    pub group_of: Vec<String>,
    /// Начальные величины всех спутников, None - пока не собраны.
    f: Option<Vec<Sat>>,
    cache: Cache,
    /// Два последних шага сетки, по порядку расчёта.
    samples: Vec<(i64, Arc<Sample>)>,
    /// Цвета точек RGBA 0-1.
    pub colors: Vec<[f32; 4]>,
    /// Размеры точек, логические пиксели.
    pub sizes: Vec<f32>,
}

impl Swarm {
    pub fn new() -> Swarm {
        Swarm::default()
    }

    /// Элементы группы. Одинаковые номера NORAD из разных групп
    /// показываются один раз - у первой группы.
    pub fn set_group(&mut self, group: &str, elements: Vec<Element>) {
        let orbits = Orbits::new(elements);
        match self.groups.iter_mut().find(|(g, _)| g == group) {
            Some(slot) => slot.1 = orbits,
            None => self.groups.push((group.to_string(), orbits)),
        }
        self.merge();
    }

    pub fn drop_group(&mut self, group: &str) {
        let before = self.groups.len();
        self.groups.retain(|(g, _)| g != group);
        if self.groups.len() != before {
            self.merge();
        }
    }

    /// Оставить только группы groups.
    pub fn keep<S: AsRef<str>>(&mut self, groups: impl IntoIterator<Item = S>) {
        let wanted: HashSet<String> = groups.into_iter().map(|g| g.as_ref().to_string()).collect();
        self.groups.retain(|(g, _)| wanted.contains(g));
        self.merge();
    }

    /// Ключи групп по порядку добавления.
    pub fn group_names(&self) -> Vec<&str> {
        self.groups.iter().map(|(g, _)| g.as_str()).collect()
    }

    /// Общие массивы всех групп сбрасываются, их соберёт init_some.
    fn merge(&mut self) {
        self.names.clear();
        self.numbers.clear();
        self.group_of.clear();
        self.f = None;
        self.cache = Cache::default();
        self.samples.clear();
        self.colors.clear();
        self.sizes.clear();
    }

    /// Посчитаны ли начальные величины всех групп.
    pub fn ready(&self) -> bool {
        self.groups.iter().all(|(_, o)| o.ready)
    }

    /// Начальные величины следующих спутников, не больше budget за вызов.
    /// Возвращает true, когда посчитаны все.
    pub fn init_some(&mut self, budget: usize) -> bool {
        if let Some((_, orbits)) = self.groups.iter_mut().find(|(_, o)| !o.ready) {
            orbits.init_some(Some(budget));
            return false;
        }
        if self.f.is_none() {
            self.build();
        }
        true
    }

    fn build(&mut self) {
        let mut seen = HashSet::new();
        let mut sats = Vec::new();
        for (group, orbits) in &self.groups {
            let take: Vec<usize> = (0..orbits.elements.len().min(orbits.sats.len()))
                .filter(|i| !seen.contains(&orbits.elements[*i].number))
                .collect();
            for i in &take {
                let e = &orbits.elements[*i];
                seen.insert(e.number);
                sats.push(orbits.sats[*i]);
                self.names.push(e.name.clone());
                self.numbers.push(e.number);
                self.group_of.push(group.clone());
                let [r, g, b] = color(group);
                self.colors.push([f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, 1.0]);
                self.sizes.push(size(group));
            }
        }
        self.cache = Cache::new(&sats);
        self.f = Some(sats);
        self.samples.clear();
    }

    /// Начальные величины всех спутников по порядку, пусто - пока
    /// не собраны.
    pub fn sats(&self) -> &[Sat] {
        self.f.as_deref().unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Положения и скорости в системе Земли в момент шага step сетки
    /// [`SAMPLE`], запоминаются два последних шага.
    fn sample(&mut self, step: i64) -> Arc<Sample> {
        if let Some((_, s)) = self.samples.iter().find(|(k, _)| *k == step) {
            return s.clone();
        }
        let jd = jd_now(step as f64 * SAMPLE);
        let sats = self.f.as_deref().unwrap_or_default();
        let (tsince, stale): (Vec<f64>, Vec<bool>) = sats.iter().map(|s| since(s, jd)).unzip();
        let states = sgp4::propagate(sats, &tsince, Some(&mut self.cache));
        let mut sample = Sample { p: Vec::new(), w: Vec::new(), error: Vec::new() };
        for (state, old) in states.iter().zip(&stale) {
            let (p, w) = to_ecef(state.r, state.v, jd);
            sample.p.push(p);
            sample.w.push(w);
            sample.error.push(if *old { STALE } else { state.error });
        }
        if self.samples.len() >= 2 {
            // Остаётся ближайший к новому шагу, при равенстве - ранний.
            let mut best = 0;
            for (i, (k, _)) in self.samples.iter().enumerate() {
                if k.abs_diff(step) < self.samples[best].0.abs_diff(step) {
                    best = i;
                }
            }
            let keep = self.samples.swap_remove(best);
            self.samples.clear();
            self.samples.push(keep);
        }
        let sample = Arc::new(sample);
        self.samples.push((step, sample.clone()));
        sample
    }

    /// Положения спутников, метры ECEF, на момент seconds UTC от 1970 года
    /// и признак годного положения. Пока начальные величины не посчитаны,
    /// спутников нет.
    pub fn positions(&mut self, seconds: f64) -> (Vec<V3>, Vec<bool>) {
        if self.f.is_none() || self.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let step = (seconds / SAMPLE).floor();
        let s = seconds / SAMPLE - step;
        let step = step as i64;
        let a = self.sample(step);
        let b = self.sample(step.saturating_add(1));
        let mut points = Vec::with_capacity(a.p.len());
        let mut ok = Vec::with_capacity(a.p.len());
        for i in 0..a.p.len().min(b.p.len()) {
            let point = hermite(a.p[i], a.w[i], b.p[i], b.w[i], SAMPLE, s);
            ok.push(a.error[i] == 0 && b.error[i] == 0 && point.iter().all(|v| v.is_finite()));
            points.push(point);
        }
        (points, ok)
    }

    /// Номер спутника по номеру NORAD или None.
    pub fn index_of(&self, number: i64) -> Option<usize> {
        self.f.as_ref()?;
        self.numbers.iter().position(|n| *n == number)
    }

    /// Положение (метры ECEF) и скорость относительно Земли спутника
    /// index на момент seconds или None.
    pub fn motion(&self, index: usize, seconds: f64) -> Option<(V3, V3)> {
        let sat = self.sats().get(index)?;
        let jd = jd_now(seconds);
        let (state, stale) = single(sat, jd);
        if stale || state.error != 0 {
            return None;
        }
        Some(to_ecef(state.r, state.v, jd))
    }

    /// Сведения о спутнике index на момент seconds или None, если такого
    /// нет.
    pub fn info(&self, index: usize, seconds: f64) -> Option<Info> {
        let sat = self.sats().get(index)?;
        let jd = jd_now(seconds);
        let (state, stale) = single(sat, jd);
        let (p, _) = to_ecef(state.r, state.v, jd);
        Some(Info {
            name: self.names.get(index)?.clone(),
            number: *self.numbers.get(index)?,
            group: self.group_of.get(index)?.clone(),
            height: height(p) / 1000.0,
            speed: dot(state.v, state.v).sqrt(),
            age: jd - sgp4::JD_EPOCH0 - sat.epoch,
            error: if stale { STALE } else { state.error },
        })
    }
}

/// Виток орбиты и след спутника: широты, долготы и высоты витка, метры,
/// широты и долготы следа, градусы.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Path {
    pub lat: Vec<f64>,
    pub lon: Vec<f64>,
    pub h: Vec<f64>,
    pub tlat: Vec<f64>,
    pub tlon: Vec<f64>,
}

/// num значений от start до stop, как `np.linspace`.
fn linspace(start: f64, stop: f64, num: usize) -> Vec<f64> {
    let step = (stop - start) / (num.saturating_sub(1).max(1)) as f64;
    let mut out: Vec<f64> = (0..num).map(|i| start + i as f64 * step).collect();
    if let Some(last) = out.last_mut()
        && num > 1
    {
        *last = stop;
    }
    out
}

/// Виток орбиты и след спутника index на момент seconds.
///
/// Орбита - положения на один период вперёд в инерциальной системе,
/// повёрнутые в систему Земли на сам момент. Так виток замкнут и стоит
/// на звёздах, Земля под ним поворачивается. След - точки под спутником
/// от половины периода назад до половины вперёд с учётом вращения Земли.
/// None, если спутник не считается.
pub fn path(sats: &[Sat], index: usize, seconds: f64) -> Option<Path> {
    let sat = sats.get(index)?;
    let period = 2.0 * std::f64::consts::PI / sat.no; // минут
    if !(period.is_finite() && period > 0.0) {
        return None;
    }
    let n = ORBIT_POINTS;
    let jd = jd_now(seconds);
    let mut out = Path::default();
    for step in linspace(0.0, period, n + 1) {
        let (state, stale) = single(sat, jd + step / 1440.0);
        if stale || state.error != 0 {
            return None;
        }
        let [lat, lon, h] = ecef_to_geodetic(scale(sgp4::teme_to_ecef(state.r, jd), 1000.0));
        out.lat.push(lat);
        out.lon.push(lon);
        out.h.push(h);
    }
    for step in linspace(-0.5 * period, 0.5 * period, n + 1) {
        let moment = jd + step / 1440.0;
        let (state, stale) = single(sat, moment);
        if stale || state.error != 0 {
            continue;
        }
        // Каждая точка следа - своим поворотом Земли.
        let [lat, lon, _] = ecef_to_geodetic(scale(sgp4::teme_to_ecef(state.r, moment), 1000.0));
        out.tlat.push(lat);
        out.tlon.push(lon);
    }
    Some(out)
}

/// Азимут движения точки над Землёй, градусы от севера.
pub fn heading(point: V3, velocity: V3) -> f64 {
    let [lat, lon, _] = ecef_to_geodetic(point);
    let (sla, cla) = lat.to_radians().sin_cos();
    let (slo, clo) = lon.to_radians().sin_cos();
    let east = [-slo, clo, 0.0];
    let north = [-sla * clo, -sla * slo, cla];
    dot(velocity, east).atan2(dot(velocity, north)).to_degrees().rem_euclid(360.0)
}
