// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Инсоляция - часы прямого солнечного света в сутки по рельефу. Перенос
//! `core/insolation.py` модуля, положение солнца - перенос нужной части
//! `core/sun.py` (слой data не зависит от scene).
//!
//! Высоты берутся на квадратной сетке с равным шагом в метрах вокруг
//! точки: внутренний круг радиуса radius_m и поле вокруг шириной
//! не меньше [`MIN_HORIZON`] - из него берутся тени гор. Сетка - [`GRID`]
//! узлов на сторону. Для каждого внутреннего узла заранее считается
//! горизонт - тангенс наибольшего угла на рельеф по [`AZIMUTHS`] азимутам
//! на расстоянии до ширины поля, с опусканием за кривизну d² / 2R. Узел
//! освещён в момент, если солнце выше горизонта по своему азимуту и падает
//! на поверхность с её стороны (скалярное произведение нормали
//! и направления на солнце больше нуля).
//!
//! Моменты - через [`STEP`] секунд по всем суткам UTC промежутка, если
//! суток больше [`MAX_DAYS`] - по MAX_DAYS суткам, равномерно. Итог -
//! среднее количество часов прямого света в сутки. Направление на солнце
//! одно для всей сетки, по центральной точке. Рефракция и облака
//! не учитываются.
//!
//! Расчёт идёт частями ([`Work::step`]): окно гоняет его в цикле событий,
//! у страницы WebAssembly потоков нет.
//!
//! Положение солнца - формулы малой точности Астрономического альманаха
//! (раздел C), ошибка склонения и прямого восхождения около 0.01°
//! в годы 1950-2050. Земля повёрнута на гринвичское среднее звёздное
//! время ([`crate::stars::gmst`]).

use crate::stars::{UNIX_J2000, gmst};
use crate::viewshed::{Heights, distance_bearing};

/// Секунд между положениями солнца.
pub const STEP: f64 = 600.0;
/// Суток в расчёте, остальные пропускаются равномерно.
pub const MAX_DAYS: usize = 15;
/// Направлений горизонта.
pub const AZIMUTHS: usize = 32;
/// Узлов сетки высот на сторону.
pub const GRID: usize = 800;
/// Наименьшая ширина поля для теней гор, метры.
pub const MIN_HORIZON: f64 = 5000.0;
/// Рост шага по лучу горизонта.
pub const GROWTH: f64 = 1.08;
pub const DAY: f64 = 86400.0;
pub const OPACITY: f64 = 0.6;
/// Шкала часов в долях наибольшего значения: синий - мало света,
/// красный - много.
pub const STOPS: [(f64, [u8; 3]); 6] =
    [(0.0, [49, 54, 149]), (0.2, [69, 117, 180]), (0.4, [171, 217, 233]), (0.6, [254, 224, 144]), (0.8, [244, 109, 67]), (1.0, [215, 48, 39])];
/// Большая полуось WGS84 - радиус по умолчанию.
pub const EARTH_RADIUS: f64 = 6378137.0;

// Солнце, часть `core/sun.py`.

/// Прямое восхождение и склонение солнца в радианах на момент unix_time,
/// секунды UTC.
pub fn sun_equatorial(unix_time: f64) -> (f64, f64) {
    let n = (unix_time - UNIX_J2000) / 86400.0;
    let mean_lon = (280.460 + 0.9856474 * n).rem_euclid(360.0).to_radians();
    let anomaly = (357.528 + 0.9856003 * n).rem_euclid(360.0).to_radians();
    let ecl_lon = mean_lon + 1.915f64.to_radians() * anomaly.sin() + 0.020f64.to_radians() * (2.0 * anomaly).sin();
    let obliquity = (23.439 - 0.0000004 * n).to_radians();
    let ra = (obliquity.cos() * ecl_lon.sin()).atan2(ecl_lon.cos());
    let dec = (obliquity.sin() * ecl_lon.sin()).asin();
    (ra, dec)
}

/// Единичный вектор на солнце в ECEF. Солнце далеко, направление одно
/// для всей Земли.
pub fn sun_direction(unix_time: f64) -> [f64; 3] {
    let (ra, dec) = sun_equatorial(unix_time);
    let lon = ((ra - gmst(unix_time)).to_degrees() + 180.0).rem_euclid(360.0) - 180.0;
    let (la, lo) = (dec, lon.to_radians());
    [la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin()]
}

/// Направление на солнце в осях восток, север, верх точки (lat, lon)
/// на моменты times.
pub fn sun_enu(lat: f64, lon: f64, times: &[f64]) -> Vec<[f64; 3]> {
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    let east = [-lo.sin(), lo.cos(), 0.0];
    let north = [-la.sin() * lo.cos(), -la.sin() * lo.sin(), la.cos()];
    let up = [la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin()];
    let dot = |a: &[f64; 3], b: &[f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    times
        .iter()
        .map(|&t| {
            let sun = sun_direction(t);
            [dot(&sun, &east), dot(&sun, &north), dot(&sun, &up)]
        })
        .collect()
}

// Сетка.

/// Количество суток промежутка от начала суток start до начала суток end
/// включительно, как у модуля: округление к чётному.
fn day_count(start: f64, end: f64) -> i64 {
    (((end - start) / DAY).round_ties_even() as i64).saturating_add(1)
}

/// Моменты расчёта и вес момента в часах на сутки. start и end - начало
/// первых и последних суток промежутка, секунды UTC. Сутки берутся все,
/// если их не больше [`MAX_DAYS`], иначе MAX_DAYS равномерно.
pub fn sample_times(start: f64, end: f64) -> (Vec<f64>, f64) {
    let days = day_count(start, end);
    let count = days.clamp(1, MAX_DAYS as i64);
    let mut picked: Vec<f64> = if count == 1 {
        vec![0.0]
    } else {
        let last = (days - 1) as f64;
        (0..count).map(|i| (i as f64 * last / (count - 1) as f64).round_ties_even()).collect()
    };
    picked.sort_by(f64::total_cmp);
    picked.dedup();
    let per_day = (DAY / STEP) as usize;
    let times = picked.iter().flat_map(|p| (0..per_day).map(move |k| start + p * DAY + k as f64 * STEP + STEP / 2.0)).collect();
    (times, STEP / 3600.0 / picked.len() as f64)
}

/// Шаг сетки, узлов поля с каждой стороны и внутренних узлов.
pub fn layout(radius_m: f64) -> (f64, usize, usize) {
    let field = radius_m.max(MIN_HORIZON);
    let cell = 2.0 * (radius_m + field) / GRID as f64;
    // Нечисло и бесконечность при приведении дают 0 или предел типа.
    let margin = (field / cell).ceil() as usize;
    (cell, margin, GRID.saturating_sub(margin.saturating_mul(2)))
}

/// Широты строк (с юга на север) и долготы столбцов сетки n×n с шагом
/// cell метров и центром в точке.
pub fn grid_axes(lat: f64, lon: f64, cell: f64, n: usize, radius: f64) -> (Vec<f64>, Vec<f64>) {
    let offsets: Vec<f64> = (0..n).map(|i| (i as f64 - (n as f64 - 1.0) / 2.0) * cell).collect();
    let lats = offsets.iter().map(|o| lat + (o / radius).to_degrees()).collect();
    let lons = offsets.iter().map(|o| lon + (o / (radius * lat.to_radians().cos())).to_degrees()).collect();
    (lats, lons)
}

/// Широты и долготы всех узлов сетки высот: [`GRID`] × GRID точек, строка
/// за строкой с юга на север, в строке с запада на восток.
pub fn grid_points(lat: f64, lon: f64, radius_m: f64, radius: f64) -> Vec<(f64, f64)> {
    let (cell, _, _) = layout(radius_m);
    let (lats, lons) = grid_axes(lat, lon, cell, GRID, radius);
    lats.iter().flat_map(|&la| lons.iter().map(move |&lo| (la, lo))).collect()
}

/// Расстояния вдоль луча горизонта: от шага сетки до ширины поля, шаг
/// растёт в [`GROWTH`] раз, но не меньше шага сетки. Шаг не больше нуля
/// и бесконечности дают пустой список.
pub fn ray_steps(cell: f64, field: f64) -> Vec<f64> {
    let mut out = Vec::new();
    if !(cell.is_finite() && cell > 0.0 && field.is_finite()) {
        return out;
    }
    let mut d = cell;
    while d <= field {
        out.push(d);
        d = (d * GROWTH).max(d + cell);
    }
    out
}

/// Сдвиги сетки для горизонта: по азимутам списки (di, dj) без повторов,
/// di - к северу, dj - к востоку, в узлах.
pub fn shifts(cell: f64, margin: usize) -> Vec<Vec<(i64, i64)>> {
    let steps = ray_steps(cell, margin as f64 * cell);
    (0..AZIMUTHS)
        .map(|k| {
            let a = 2.0 * std::f64::consts::PI * k as f64 / AZIMUTHS as f64;
            let mut seen: Vec<(i64, i64)> = Vec::new();
            for d in &steps {
                let pair = ((d * a.cos() / cell).round_ties_even() as i64, (d * a.sin() / cell).round_ties_even() as i64);
                if pair != (0, 0) && !seen.contains(&pair) {
                    seen.push(pair);
                }
            }
            seen
        })
        .collect()
}

// Итог.

/// Часы прямого света в сутки на внутренней сетке n × n: hours строка за
/// строкой с юга на север, центр сетки - точка (lat, lon).
#[derive(Debug, Clone, PartialEq)]
pub struct Insolation {
    pub lat: f64,
    pub lon: f64,
    pub radius_m: f64,
    /// Шаг сетки, метры.
    pub cell: f64,
    /// Узлов на сторону.
    pub n: usize,
    pub hours: Vec<f64>,
    /// Суток в промежутке.
    pub days: i64,
    /// Шаг сетки в градусах широты и долготы.
    pub dlat: f64,
    pub dlon: f64,
    /// Участок многоугольником (широта, долгота): вне него часов нет.
    pub ring: Option<Vec<(f64, f64)>>,
}

impl Insolation {
    /// Верх шкалы - наибольшее количество часов в круге, не меньше часа,
    /// с округлением вверх.
    pub fn top(&self) -> f64 {
        self.hours.iter().copied().filter(|v| !v.is_nan()).fold(f64::NEG_INFINITY, f64::max).ceil().max(1.0)
    }

    /// Часы в точках (широта, долгота) и признак «внутри круга»
    /// и участка. Вне них часы - нечисло.
    pub fn at(&self, points: &[(f64, f64)], radius: f64) -> Vec<(f64, bool)> {
        let n = self.n as i64;
        let middle = (self.n as f64 - 1.0) / 2.0;
        let in_ring = self.ring.as_ref().map(|r| crate::site::inside(points, r));
        points
            .iter()
            .enumerate()
            .map(|(i, &(lat, lon))| {
                let row = ((lat - self.lat) / self.dlat + middle).round_ties_even() as i64;
                let col = ((lon - self.lon) / self.dlon + middle).round_ties_even() as i64;
                let (dist, _) = distance_bearing(self.lat, self.lon, lat, lon, radius);
                let ringed = in_ring.as_ref().is_none_or(|r| r.get(i).copied().unwrap_or(false));
                let inside = dist <= self.radius_m && ringed && (0..n).contains(&row) && (0..n).contains(&col);
                let value = if inside { self.hours.get((row * n + col) as usize).copied().unwrap_or(f64::NAN) } else { f64::NAN };
                (value, inside)
            })
            .collect()
    }

    /// Часы сеткой широт и долгот для GeoTIFF, `raster` модуля: строки
    /// с севера, вне круга и участка нечисло.
    pub fn raster(&self, radius: f64) -> (crate::geotiff::LatLonGrid, Vec<f64>) {
        let half = (self.n as f64 - 1.0) / 2.0;
        let grid = crate::geotiff::LatLonGrid { north: self.lat + half * self.dlat, west: self.lon - half * self.dlon, dlat: self.dlat, dlon: self.dlon, cols: self.n, rows: self.n };
        let points: Vec<(f64, f64)> = (0..self.n).flat_map(|r| (0..self.n).map(move |c| (grid.north - r as f64 * grid.dlat, grid.west + c as f64 * grid.dlon))).collect();
        (grid, self.at(&points, radius).into_iter().map(|(v, _)| v).collect())
    }

    /// Доля шкалы 0-1 для количества часов.
    pub fn share_at(&self, value: f64) -> f64 {
        (value / self.top()).clamp(0.0, 1.0)
    }
}

/// Цвет шкалы [`STOPS`] на доле share, линейно между узлами.
fn color(share: f64) -> [f64; 3] {
    let mut out = [0.0; 3];
    for (c, slot) in out.iter_mut().enumerate() {
        *slot = interp(share, |s| f64::from(s.1[c]));
    }
    out
}

/// Линейная интерполяция по узлам шкалы, за краями - крайние значения,
/// как numpy.interp.
fn interp(x: f64, value: impl Fn(&(f64, [u8; 3])) -> f64) -> f64 {
    let (Some(first), Some(last)) = (STOPS.first(), STOPS.last()) else {
        return 0.0;
    };
    if x.is_nan() {
        return f64::NAN;
    }
    if x <= first.0 {
        return value(first);
    }
    if x >= last.0 {
        return value(last);
    }
    for w in STOPS.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if x <= b.0 {
            let t = (x - a.0) / (b.0 - a.0);
            return value(a) + t * (value(b) - value(a));
        }
    }
    value(last)
}

/// Картинка тайла: часы цветами [`STOPS`], вне круга прозрачно. RGBA
/// с премноженной альфой, пиксель на точку points.
pub fn tile_rgba(result: &Insolation, points: &[(f64, f64)], radius: f64) -> Vec<[u8; 4]> {
    let alpha = (OPACITY * 255.0).round() as u8;
    result
        .at(points, radius)
        .into_iter()
        .map(|(value, inside)| {
            if !inside {
                return [0; 4];
            }
            let rgb = color(result.share_at(value));
            let c = |v: f64| (v * OPACITY).round_ties_even() as u8;
            [c(rgb[0]), c(rgb[1]), c(rgb[2]), alpha]
        })
        .collect()
}

// Расчёт частями.

/// Состояние расчёта после шага.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    /// Расчёт не кончен, доля сделанного 0-1.
    Going(f64),
    /// Расчёт кончен, итог - [`Work::result`].
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Horizon,
    Normals,
    Sun,
    Done,
}

/// Горизонт расчёта: тангенсы горизонта азимуты × узлы, нормали узлов,
/// узлов на сторону, шаг сетки.
pub type HorizonParts = (Vec<f32>, Vec<[f64; 3]>, usize, f64);

/// Расчёт инсоляции частями, замена генератора `parts` модуля. Шаг -
/// один сдвиг сетки горизонта или одно положение солнца, 1-2 мс в
/// оптимизированной сборке.
#[derive(Debug, Clone)]
pub struct Work {
    lat: f64,
    lon: f64,
    radius_m: f64,
    radius: f64,
    cell: f64,
    margin: usize,
    inner: usize,
    days: i64,
    /// Высоты узлов сетки, строка за строкой с юга на север.
    h: Vec<f64>,
    shifts: Vec<Vec<(i64, i64)>>,
    times: Vec<f64>,
    weight: f64,
    total: usize,
    done: usize,
    /// Тангенс угла горизонта: азимуты × внутренние узлы.
    hor: Vec<f32>,
    /// Наибольший тангенс по текущему азимуту.
    best: Vec<f64>,
    azimuth: usize,
    pair: usize,
    /// Нормали поверхности внутренних узлов: восток, север, верх.
    normals: Vec<[f64; 3]>,
    /// Начало следующих суток моментов и положения солнца над горизонтом
    /// текущих суток.
    first: usize,
    suns: Vec<[f64; 3]>,
    sun: usize,
    hours: Vec<f64>,
    phase: Phase,
    result: Option<Insolation>,
    /// Только горизонт и нормали, без солнца: тени на момент
    /// ([`crate::shadows`]).
    horizon_only: bool,
}

impl Work {
    /// Расчёт инсоляции в круге radius_m метров вокруг точки (lat, lon)
    /// за сутки UTC от start до end включительно (начала суток, секунды
    /// UTC). heights - высоты узлов [`grid_points`] в том же порядке.
    /// radius - радиус тела. None - высот не GRID × GRID, круг пуст или
    /// величины не числа.
    pub fn new(lat: f64, lon: f64, radius_m: f64, heights: &[f64], start: f64, end: f64, radius: f64) -> Option<Self> {
        let finite = [lat, lon, radius_m, start, end, radius].iter().all(|v| v.is_finite());
        if !finite || radius <= 0.0 || radius_m <= 0.0 || heights.len() != GRID * GRID {
            return None;
        }
        let (cell, margin, inner) = layout(radius_m);
        if inner == 0 || margin == 0 {
            return None;
        }
        let shifts = shifts(cell, margin);
        let (times, weight) = sample_times(start, end);
        let total = shifts.iter().map(Vec::len).sum::<usize>() + times.len();
        Some(Self {
            lat,
            lon,
            radius_m,
            radius,
            cell,
            margin,
            inner,
            days: day_count(start, end),
            h: heights.to_vec(),
            shifts,
            times,
            weight,
            total,
            done: 0,
            hor: vec![f32::NEG_INFINITY; AZIMUTHS * inner * inner],
            best: vec![f64::NEG_INFINITY; inner * inner],
            azimuth: 0,
            pair: 0,
            normals: Vec::new(),
            first: 0,
            suns: Vec::new(),
            sun: 0,
            hours: vec![0.0; inner * inner],
            phase: Phase::Horizon,
            result: None,
            horizon_only: false,
        })
    }

    /// Только горизонт и нормали узлов, без положений солнца, как
    /// `shadows.parts` модуля. Величины - как у [`Work::new`].
    pub fn horizon(lat: f64, lon: f64, radius_m: f64, heights: &[f64], radius: f64) -> Option<Self> {
        let mut work = Self::new(lat, lon, radius_m, heights, 0.0, 0.0, radius)?;
        work.times.clear();
        work.total = work.shifts.iter().map(Vec::len).sum::<usize>() + 1;
        work.horizon_only = true;
        Some(work)
    }

    /// Горизонт кончен: (тангенс горизонта азимуты × узлы, нормали узлов,
    /// узлов на сторону, шаг сетки). None - расчёт не кончен или это не
    /// расчёт горизонта.
    pub fn into_horizon(self) -> Option<HorizonParts> {
        (self.horizon_only && self.phase == Phase::Done).then_some((self.hor, self.normals, self.inner, self.cell))
    }

    fn share(&self) -> f64 {
        if self.total == 0 { 1.0 } else { self.done as f64 / self.total as f64 }
    }

    /// Шаг расчёта.
    pub fn step(&mut self) -> Step {
        loop {
            match self.phase {
                Phase::Horizon => {
                    let Some(pairs) = self.shifts.get(self.azimuth) else {
                        self.phase = Phase::Normals;
                        continue;
                    };
                    if let Some(&(di, dj)) = pairs.get(self.pair) {
                        self.horizon_pair(di, dj);
                        self.pair += 1;
                        self.done += 1;
                        return Step::Going(self.share());
                    }
                    let size = self.inner * self.inner;
                    if let Some(slot) = self.hor.get_mut(self.azimuth * size..(self.azimuth + 1) * size) {
                        for (out, best) in slot.iter_mut().zip(&self.best) {
                            *out = *best as f32;
                        }
                    }
                    self.best.fill(f64::NEG_INFINITY);
                    self.azimuth += 1;
                    self.pair = 0;
                }
                Phase::Normals => {
                    self.normals = self.surface_normals();
                    if self.horizon_only {
                        self.done += 1;
                        self.phase = Phase::Done;
                        return Step::Done;
                    }
                    self.phase = Phase::Sun;
                    return Step::Going(self.share());
                }
                Phase::Sun => {
                    if let Some(&sun) = self.suns.get(self.sun) {
                        self.sun += 1;
                        self.light(sun);
                        return Step::Going(self.share());
                    }
                    let per_day = (DAY / STEP) as usize;
                    if self.first < self.times.len() {
                        let block = self.times.get(self.first..(self.first + per_day).min(self.times.len())).unwrap_or(&[]);
                        let suns = sun_enu(self.lat, self.lon, block);
                        self.done += suns.len();
                        self.suns = suns.into_iter().filter(|s| s[2] > 0.0).collect();
                        self.sun = 0;
                        self.first += per_day;
                        continue;
                    }
                    let hours = std::mem::take(&mut self.hours);
                    let (lat, cell, radius) = (self.lat, self.cell, self.radius);
                    self.result = Some(Insolation {
                        lat,
                        lon: self.lon,
                        radius_m: self.radius_m,
                        cell,
                        n: self.inner,
                        hours,
                        days: self.days,
                        dlat: (cell / radius).to_degrees(),
                        dlon: (cell / (radius * lat.to_radians().cos())).to_degrees(),
                        ring: None,
                    });
                    self.phase = Phase::Done;
                    return Step::Done;
                }
                Phase::Done => return Step::Done,
            }
        }
    }

    /// Итог, когда расчёт кончен.
    pub fn result(&self) -> Option<&Insolation> {
        self.result.as_ref()
    }

    /// Итог, когда расчёт кончен, без копии.
    pub fn into_result(self) -> Option<Insolation> {
        self.result
    }

    /// Горизонт внутренних узлов по сдвигу (di, dj).
    fn horizon_pair(&mut self, di: i64, dj: i64) {
        let (m, inner) = (self.margin as i64, self.inner);
        let dist = (di as f64).hypot(dj as f64) * self.cell;
        let drop = dist * dist / (2.0 * self.radius);
        let start = |row: i64, col: i64| -> Option<usize> {
            let (row, col) = (usize::try_from(row).ok()?, usize::try_from(col).ok()?);
            (row < GRID && col + inner <= GRID).then_some(row * GRID + col)
        };
        for i in 0..inner {
            let r = m + i as i64;
            let (Some(b), Some(t)) = (start(r, m), start(r + di, m + dj)) else {
                continue;
            };
            let (Some(base), Some(target), Some(best)) = (self.h.get(b..b + inner), self.h.get(t..t + inner), self.best.get_mut(i * inner..(i + 1) * inner)) else {
                continue;
            };
            for ((out, h0), h1) in best.iter_mut().zip(base).zip(target) {
                let v = (h1 - drop - h0) / dist;
                if v > *out {
                    *out = v;
                }
            }
        }
    }

    /// Нормали внутренних узлов по центральным разностям, как
    /// numpy.gradient внутри сетки.
    fn surface_normals(&self) -> Vec<[f64; 3]> {
        let (m, inner, cell) = (self.margin, self.inner, self.cell);
        let at = |r: usize, c: usize| self.h.get(r * GRID + c).copied().unwrap_or(0.0);
        let mut out = Vec::with_capacity(inner * inner);
        for i in 0..inner {
            for j in 0..inner {
                let (r, c) = (m + i, m + j);
                let d_north = (at(r + 1, c) - at(r - 1, c)) / (2.0 * cell);
                let d_east = (at(r, c + 1) - at(r, c - 1)) / (2.0 * cell);
                let norm = (d_east * d_east + d_north * d_north + 1.0).sqrt();
                out.push([-d_east / norm, -d_north / norm, 1.0 / norm]);
            }
        }
        out
    }

    /// Свет одного положения солнца (восток, север, верх) над горизонтом.
    fn light(&mut self, sun: [f64; 3]) {
        let [e, n, u] = sun;
        let level = e.hypot(n);
        let tan_el = if level > 0.0 { u / level } else { f64::INFINITY };
        let k = ((e.atan2(n) / (2.0 * std::f64::consts::PI) * AZIMUTHS as f64).round_ties_even() as i64).rem_euclid(AZIMUTHS as i64) as usize;
        let size = self.inner * self.inner;
        let Some(hor) = self.hor.get(k * size..(k + 1) * size) else {
            return;
        };
        for ((hours, h), normal) in self.hours.iter_mut().zip(hor).zip(&self.normals) {
            if f64::from(*h) < tan_el && normal[0] * e + normal[1] * n + normal[2] * u > 0.0 {
                *hours += self.weight;
            }
        }
    }
}

/// Инсоляция в круге radius_m метров вокруг точки за сутки UTC от start
/// до end включительно (начала суток, секунды UTC) целиком. heights -
/// настоящие высоты списка точек (широта, долгота). radius - радиус тела.
pub fn compute(lat: f64, lon: f64, radius_m: f64, heights: Heights, start: f64, end: f64, radius: f64) -> Option<Insolation> {
    let values = heights(&grid_points(lat, lon, radius_m, radius));
    let mut work = Work::new(lat, lon, radius_m, &values, start, end, radius)?;
    while work.step() != Step::Done {}
    work.into_result()
}
