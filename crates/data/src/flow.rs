// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Струйки ветра и течений - частицы по полю скоростей. Перенос
//! `core/flow.py` модуля и чтения поля течений из `ui/flow.py`.
//!
//! Частицы рождаются в рамке вида, плывут по полю U, V (м/с, восток
//! и север) и тянут за собой след из [`TRAIL`] прежних положений.
//! Отмерив свой век или уйдя из рамки и с данных (суша у течений),
//! частица рождается заново. Скорость на экране не настоящая - поле
//! проходит за секунду показа долю [`SHARE`] ширины вида при типичной
//! скорости, иначе течение в 0.5 м/с стояло бы на месте, а ветер вблизи
//! улетал бы.
//!
//! Массивы numpy модуля здесь - `Vec` по строкам (порядок C). Сетка поля
//! [`VectorGrid`] - строки с севера на юг, в строке долготы с запада
//! на восток, значение узла (строка r, столбец c) - `u[r * cols + c]`.
//! Следы [`Particles`] - частица за частицей, в следе голова первая,
//! положение j частицы i - `lats[i * TRAIL + j]`.
//!
//! Случайность - свой генератор xorshift [`Rng`] с зерном, ряд чисел
//! не совпадает с numpy модуля, но повторяется при том же зерне.
//!
//! Поле течений - выборка ERDDAP `nesdisSSH1day` в netCDF-3, адрес
//! [`currents_url`], разбор [`read_currents`]. Величины - выбор
//! помощника модуля, утверждены автором.

use std::sync::Arc;

use crate::netcdf::{NcError, NcFile, error};

/// Радиус шара Земли, м.
pub const RADIUS: f64 = 6_371_000.0;
/// Положений в следе частицы.
pub const TRAIL: usize = 14;
/// Век частицы, шагов, случайный в пределах [AGE.0, AGE.1).
pub const AGE: (i64, i64) = (40, 100);
/// Доля ширины вида за секунду при типичной скорости.
pub const SHARE: f64 = 0.12;
/// Цвета заливки скорости течений: скорость м/с и цвет RGB. С 12 октября
/// 2026 года, просьба автора, под струйками течений лежит заливка
/// скорости, струйки белые, как у ветра.
pub const CURRENT_STOPS: [(f64, [f64; 3]); 7] = [
    (0.05, [20.0, 40.0, 110.0]),
    (0.2, [30.0, 120.0, 200.0]),
    (0.4, [40.0, 200.0, 170.0]),
    (0.7, [150.0, 230.0, 70.0]),
    (1.0, [250.0, 220.0, 40.0]),
    (1.5, [250.0, 130.0, 30.0]),
    (2.5, [230.0, 40.0, 70.0]),
];
/// Непрозрачность заливки по скорости, 0..255: медленная вода почти
/// прозрачна, снимок океана под ней виден.
pub const FILL_ALPHA: [(f64, f64); 6] = [(0.0, 0.0), (0.08, 40.0), (0.2, 120.0), (0.5, 190.0), (1.0, 220.0), (2.5, 230.0)];
/// Предельный уровень тайлов заливки, пиксель 0.09°.
pub const FILL_LEVEL: u32 = 4;
/// Доля клетки поля, которую типичная скорость проходит за шаг показа,
/// не больше. Замечание автора от 12 октября 2026 года: издалека
/// частица течений за шаг перескакивала клетку 0.5°, след петлял.
pub const CELL_SHARE: f64 = 0.3;
/// Первый день ряда течений NOAA CoastWatch.
pub const CURRENTS_FIRST: &str = "2017-02-13";

// Величины показа из `ui/flow.py` модуля.

/// Миллисекунд между шагами частиц.
pub const TICK_MS: u32 = 40;
/// Количество частиц ветра.
pub const WIND_COUNT: usize = 5000;
/// Количество частиц течений.
pub const CURRENTS_COUNT: usize = 8000;
/// Типичная скорость ветра, м/с.
pub const WIND_TYPICAL: f64 = 10.0;
/// Типичная скорость течений, м/с.
pub const CURRENTS_TYPICAL: f64 = 0.3;
/// Доля угла до горизонта у круга рождения частиц.
pub const HORIZON_SHARE: f64 = 0.95;
/// Высота глаза, м, выше которой рамка частиц - видимый диск.
pub const DISK_FROM: f64 = 3.0e6;
/// Во сколько раз сменилась ширина вида - частицы рождаются заново.
pub const RESET_ZOOM: f64 = 2.0;
/// Полей течений в памяти.
pub const DAYS_KEPT: usize = 6;
/// Миллисекунд после движения бегунка до запроса дня.
pub const DAY_PAUSE_MS: u32 = 400;

/// Адрес выборки течений ERDDAP, `{t}` - время. Сутки, сетка 0.25°,
/// выборка через 0.5°, около 4 МБ netCDF-3.
pub const CURRENTS_URL: &str = "https://coastwatch.pfeg.noaa.gov/erddap/griddap/nesdisSSH1day.nc?\
ugos%5B({t})%5D%5B(-89.875):2:(89.875)%5D%5B(-179.875):2:(179.875)%5D,\
vgos%5B({t})%5D%5B(-89.875):2:(89.875)%5D%5B(-179.875):2:(179.875)%5D";
/// Подпись источника течений и её адрес.
pub const CURRENTS_ATTRIBUTION: (&str, &str) =
    ("Currents: NOAA CoastWatch, NESDIS SSH", "https://coastwatch.pfeg.noaa.gov/erddap/griddap/nesdisSSH1day.html");

/// Сутки течений у страницы - копия в хранилище planetx-live: у ERDDAP нет
/// заголовка CORS, браузер его ответ не читает. Копии раз в 3 часа
/// обновляет задача GitHub Actions того хранилища, с 25 октября 2026 года.
/// Хранятся последние 7 суток, None или пустая строка - последние.
pub fn currents_web_url(day: Option<&str>) -> String {
    let name = day.filter(|d| !d.is_empty()).unwrap_or("latest");
    format!("https://raw.githubusercontent.com/Valery35/planetx-live/data/currents/{name}.nc")
}

/// Адрес выборки течений: день «ГГГГ-ММ-ДД» или последний (None или
/// пустая строка).
pub fn currents_url(day: Option<&str>) -> String {
    let t = match day {
        Some(d) if !d.is_empty() => format!("{d}T00:00:00Z"),
        _ => "last".to_string(),
    };
    CURRENTS_URL.replace("{t}", &t)
}

/// Генератор xorshift64* с зерном. Зерно перемешивается splitmix64,
/// нулевое зерно годится.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        Rng(if z == 0 { 0x2545_F491_4F6C_DD1D } else { z })
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Равномерное число в [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Равномерное число в [lo, hi), как `rng.uniform` numpy.
    pub fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }

    /// Целое в [lo, hi), как `rng.integers` numpy. При hi <= lo - lo.
    pub fn integers(&mut self, lo: i64, hi: i64) -> i64 {
        let span = hi.saturating_sub(lo);
        if span <= 0 {
            return lo;
        }
        lo + (self.next_u64() % span as u64) as i64
    }
}

/// Поле скоростей u, v на сетке широт и долгот. west, north - центр
/// левого верхнего узла, step - шаг в градусах, строки с севера на юг
/// по `cols` значений, NaN - нет данных.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorGrid {
    pub u: Vec<f32>,
    pub v: Vec<f32>,
    pub rows: usize,
    pub cols: usize,
    pub west: f64,
    pub north: f64,
    pub step: f64,
}

impl VectorGrid {
    /// Поле из значений по строкам. None, если длины не равны
    /// rows * cols, строк меньше двух, столбцов нет или шаг не
    /// положителен.
    pub fn new(u: Vec<f32>, v: Vec<f32>, rows: usize, cols: usize, west: f64, north: f64, step: f64) -> Option<Self> {
        let n = rows.checked_mul(cols)?;
        if rows < 2 || cols == 0 || u.len() != n || v.len() != n || !(step > 0.0 && step.is_finite()) {
            return None;
        }
        if !(west.is_finite() && north.is_finite()) {
            return None;
        }
        Some(Self { u, v, rows, cols, west, north, step })
    }

    /// u и v в точке, билинейно, долгота по кругу. Узел без данных
    /// делает точку пустой (NaN), за крайними строками - тоже NaN.
    pub fn sample(&self, lat: f64, lon: f64) -> (f64, f64) {
        if !(lat.is_finite() && lon.is_finite()) || self.rows < 2 || self.cols == 0 || self.step.is_nan() || self.step <= 0.0 {
            return (f64::NAN, f64::NAN);
        }
        let rows = self.rows as f64;
        let cols = self.cols as i64;
        let y = (self.north - lat) / self.step;
        if !(0.0..=rows - 1.0).contains(&y) {
            return (f64::NAN, f64::NAN);
        }
        let x = ((lon - self.west) / self.step).rem_euclid(self.cols as f64);
        let y0 = (y.floor() as usize).min(self.rows - 2);
        let xf = x.floor();
        let x0 = (xf as i64).rem_euclid(cols) as usize;
        let x1 = (x0 + 1) % self.cols;
        let fy = y - y0 as f64;
        let fx = x - xf;
        let at = |grid: &[f32], r: usize, c: usize| grid.get(r * self.cols + c).map_or(f64::NAN, |&g| f64::from(g));
        let one = |grid: &[f32]| {
            let top = at(grid, y0, x0) * (1.0 - fx) + at(grid, y0, x1) * fx;
            let bottom = at(grid, y0 + 1, x0) * (1.0 - fx) + at(grid, y0 + 1, x1) * fx;
            top * (1.0 - fy) + bottom * fy
        };
        (one(&self.u), one(&self.v))
    }
}

/// Рамка рождения частиц.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Frame {
    /// Круг на шаре: центр и угловой радиус, градусы. Издалека рамка -
    /// видимый круг Земли.
    Cap { lat: f64, lon: f64, angle: f64 },
    /// Прямоугольник широт и долгот, градусы. Запад больше востока -
    /// рамка переходит линию перемены дат.
    Rect { west: f64, south: f64, east: f64, north: f64 },
}

/// Рамка-круг на шаре: центр и угловой радиус в градусах. Прежде
/// прямоугольник широт и долгот издалека захватывал обратную сторону
/// шара, и там рождались две трети частиц, 12 октября 2026 года.
pub fn cap_box(lat: f64, lon: f64, angle: f64) -> Frame {
    Frame::Cap { lat, lon, angle }
}

/// Рамка-прямоугольник (запад, юг, восток, север).
pub fn rect_box(west: f64, south: f64, east: f64, north: f64) -> Frame {
    Frame::Rect { west, south, east, north }
}

/// Долгота в [-180, 180).
fn wrap_lon(lon: f64) -> f64 {
    (lon + 180.0).rem_euclid(360.0) - 180.0
}

/// Точки, равномерные по площади в рамке. Широты и долготы, градусы.
pub fn spawn(count: usize, frame: &Frame, rng: &mut Rng) -> (Vec<f64>, Vec<f64>) {
    match *frame {
        Frame::Cap { lat: lat0, lon: lon0, angle } => {
            let c = angle.to_radians().cos();
            let d: Vec<f64> = (0..count).map(|_| rng.uniform(c, 1.0).clamp(-1.0, 1.0).acos()).collect();
            let az: Vec<f64> = (0..count).map(|_| rng.uniform(0.0, 2.0 * std::f64::consts::PI)).collect();
            let b = lat0.to_radians();
            let mut lats = Vec::with_capacity(count);
            let mut lons = Vec::with_capacity(count);
            for (d, az) in d.into_iter().zip(az) {
                let lat = (b.sin() * d.cos() + b.cos() * d.sin() * az.cos()).clamp(-1.0, 1.0).asin();
                let lon = lon0.to_radians() + (az.sin() * d.sin() * b.cos()).atan2(d.cos() - b.sin() * lat.sin());
                lats.push(lat.to_degrees());
                lons.push(wrap_lon(lon.to_degrees()));
            }
            (lats, lons)
        }
        Frame::Rect { west, south, east, north } => {
            let width = (east - west).rem_euclid(360.0);
            let width = if width == 0.0 { 360.0 } else { width };
            let s = south.to_radians().sin();
            let n = north.to_radians().sin();
            let lats = (0..count).map(|_| rng.uniform(s, n).clamp(-1.0, 1.0).asin().to_degrees()).collect();
            let lons = (0..count).map(|_| wrap_lon(west + rng.uniform(0.0, width))).collect();
            (lats, lons)
        }
    }
}

/// Точка в рамке.
pub fn inside(lat: f64, lon: f64, frame: &Frame) -> bool {
    match *frame {
        Frame::Cap { lat: lat0, lon: lon0, angle } => {
            let a = lat.to_radians();
            let b = lat0.to_radians();
            let cos = a.sin() * b.sin() + a.cos() * b.cos() * (lon - lon0).to_radians().cos();
            cos >= angle.to_radians().cos()
        }
        Frame::Rect { west, south, east, north } => {
            let keep = lat >= south && lat <= north;
            if (east - west).rem_euclid(360.0) == 0.0 {
                return keep;
            }
            if west <= east {
                return keep && lon >= west && lon <= east;
            }
            keep && (lon >= west || lon <= east)
        }
    }
}

/// Частицы со следом. lats, lons - по [`TRAIL`] положений на частицу,
/// голова первая, положение j частицы i - индекс `i * TRAIL + j`.
/// speed - скорость поля у головы, м/с.
#[derive(Debug, Clone)]
pub struct Particles {
    pub count: usize,
    rng: Rng,
    pub lats: Vec<f64>,
    pub lons: Vec<f64>,
    pub speed: Vec<f64>,
    pub age: Vec<i64>,
    pub life: Vec<i64>,
    /// След ещё не тянется.
    pub fresh: Vec<bool>,
    /// Рамка рождения, None - частицы не запущены.
    pub frame: Option<Frame>,
}

impl Particles {
    pub fn new(count: usize, seed: u64) -> Self {
        Self {
            count,
            rng: Rng::new(seed),
            lats: vec![0.0; count * TRAIL],
            lons: vec![0.0; count * TRAIL],
            speed: vec![0.0; count],
            age: vec![0; count],
            life: vec![0; count],
            fresh: vec![true; count],
            frame: None,
        }
    }

    /// Положение j следа частицы i (широта, долгота), голова - j = 0.
    pub fn point(&self, i: usize, j: usize) -> Option<(f64, f64)> {
        if j >= TRAIL {
            return None;
        }
        let k = i.checked_mul(TRAIL)?.checked_add(j)?;
        Some((*self.lats.get(k)?, *self.lons.get(k)?))
    }

    /// Все частицы заново в рамке frame.
    pub fn reset(&mut self, frame: Frame) {
        self.frame = Some(frame);
        let all: Vec<usize> = (0..self.count).collect();
        self.respawn(&all);
        // Возраст вразброс, иначе все частицы перерождались бы разом.
        for a in &mut self.age {
            *a = self.rng.integers(0, AGE.1);
        }
    }

    fn respawn(&mut self, which: &[usize]) {
        let Some(frame) = self.frame else { return };
        if which.is_empty() {
            return;
        }
        let (lats, lons) = spawn(which.len(), &frame, &mut self.rng);
        for (&i, (lat, lon)) in which.iter().zip(lats.into_iter().zip(lons)) {
            if let Some(trail) = self.lats.get_mut(i * TRAIL..(i + 1) * TRAIL) {
                trail.fill(lat);
            }
            if let Some(trail) = self.lons.get_mut(i * TRAIL..(i + 1) * TRAIL) {
                trail.fill(lon);
            }
        }
        for &i in which {
            let life = self.rng.integers(AGE.0, AGE.1);
            if let (Some(a), Some(l), Some(s), Some(f)) =
                (self.age.get_mut(i), self.life.get_mut(i), self.speed.get_mut(i), self.fresh.get_mut(i))
            {
                *a = 0;
                *l = life;
                *s = 0.0;
                *f = true;
            }
        }
    }

    /// Шаг dt секунд показа: голова сдвигается на скорость поля,
    /// умноженную на metres_per_mps метров за секунду на 1 м/с. Частица
    /// без данных, отжившая или ушедшая из рамки рождается заново.
    pub fn step(&mut self, field: &VectorGrid, dt: f64, metres_per_mps: f64) {
        let Some(frame) = self.frame else { return };
        let distance = metres_per_mps * dt;
        let mut gone = Vec::new();
        for i in 0..self.count {
            let base = i * TRAIL;
            let (Some(trail_lat), Some(trail_lon)) =
                (self.lats.get_mut(base..base + TRAIL), self.lons.get_mut(base..base + TRAIL))
            else {
                continue;
            };
            let (lat, lon) = (trail_lat[0], trail_lon[0]);
            let (mut u, mut v) = field.sample(lat, lon);
            let dead = !u.is_finite() || !v.is_finite();
            if dead {
                u = 0.0;
                v = 0.0;
            }
            let cos_lat = lat.to_radians().cos().max(0.05);
            let new_lat = lat + (v * distance / RADIUS).to_degrees();
            let new_lon = wrap_lon(lon + (u * distance / (RADIUS * cos_lat)).to_degrees());
            trail_lat.copy_within(0..TRAIL - 1, 1);
            trail_lon.copy_within(0..TRAIL - 1, 1);
            trail_lat[0] = new_lat.clamp(-89.9, 89.9);
            trail_lon[0] = new_lon;
            let head = (trail_lat[0], trail_lon[0]);
            if let (Some(s), Some(a), Some(l), Some(f)) =
                (self.speed.get_mut(i), self.age.get_mut(i), self.life.get(i), self.fresh.get_mut(i))
            {
                *s = u.hypot(v);
                *a += 1;
                *f &= *a < TRAIL as i64;
                if dead || *a > *l || !inside(head.0, head.1, &frame) {
                    gone.push(i);
                }
            }
        }
        self.respawn(&gone);
    }
}

/// Метров за секунду показа на 1 м/с поля: типичная скорость проходит
/// долю [`SHARE`] ширины вида.
pub fn metres_per_mps(view_width: f64, typical: f64) -> f64 {
    SHARE * view_width / typical.max(1e-6)
}

/// Метров за секунду показа на 1 м/с поля: доля [`SHARE`] ширины вида,
/// но типичная скорость за шаг dt секунд проходит не больше доли share
/// клетки поля шагом step_deg градусов. share None - без предела.
/// Предел действует издалека, вблизи остаётся доля ширины вида.
pub fn display_metres(view_width: f64, typical: f64, step_deg: f64, dt: f64, share: Option<f64>) -> f64 {
    let metres = metres_per_mps(view_width, typical);
    let Some(share) = share else { return metres };
    let cell = step_deg.to_radians() * RADIUS;
    metres.min(share * cell / (typical.max(1e-6) * dt))
}

/// Отрезки следов: пары точек [[широта, долгота]; 2], доля от головы
/// 0..1 и скорость частицы у каждого отрезка. Порядок - частица
/// за частицей, в частице от головы.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Segments {
    pub pairs: Vec<[[f64; 2]; 2]>,
    pub share: Vec<f64>,
    pub speed: Vec<f64>,
}

/// Отрезки следов. Отрезки через линию перемены дат и у новорождённых
/// частиц отбрасываются.
pub fn segments(particles: &Particles) -> Segments {
    let mut out = Segments::default();
    for i in 0..particles.count {
        let age = particles.age.get(i).copied().unwrap_or(0);
        let speed = particles.speed.get(i).copied().unwrap_or(0.0);
        let depth = age.clamp(0, TRAIL as i64 - 1) as usize;
        for k in 0..depth {
            let (Some(a), Some(b)) = (particles.point(i, k), particles.point(i, k + 1)) else {
                continue;
            };
            if (a.1 - b.1).abs() < 90.0 {
                out.pairs.push([[a.0, a.1], [b.0, b.1]]);
                out.share.push(k as f64 / (TRAIL as f64 - 1.0));
                out.speed.push(speed);
            }
        }
    }
    out
}

// Даты по григорианскому календарю, алгоритм Говарда Хиннанта.

pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub(crate) fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// День «ГГГГ-ММ-ДД» по UTC для секунд moment. Годы - от 0 до 9999,
/// за ними день края.
fn utc_day(moment: f64) -> String {
    let lo = days_from_civil(0, 1, 1);
    let hi = days_from_civil(9999, 12, 31);
    let days = (moment / 86_400.0).floor().clamp(lo as f64, hi as f64) as i64;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// День течений «ГГГГ-ММ-ДД» для момента moment (секунды UTC) или
/// последний готовый last при None. За краями ряда - первый
/// [`CURRENTS_FIRST`] или последний день.
pub fn current_day(moment: Option<f64>, last: &str) -> String {
    current_day_in(moment, last, CURRENTS_FIRST)
}

/// [`current_day`] с первым днём ряда first.
pub fn current_day_in(moment: Option<f64>, last: &str, first: &str) -> String {
    let Some(moment) = moment.filter(|m| m.is_finite()) else { return last.to_string() };
    if last.is_empty() {
        return last.to_string();
    }
    let day = utc_day(moment);
    let day = if day.as_str() < first { first.to_string() } else { day };
    if day.as_str() > last { last.to_string() } else { day }
}

/// Полдень дня «ГГГГ-ММ-ДД» в секундах UTC, None у неверной даты.
pub fn day_seconds(day: &str) -> Option<f64> {
    let b = day.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let num = |s: &str| -> Option<i64> {
        if s.bytes().all(|c| c.is_ascii_digit()) { s.parse().ok() } else { None }
    };
    let y = num(day.get(0..4)?)?;
    let m = num(day.get(5..7)?)?;
    let d = num(day.get(8..10)?)?;
    if !(1..=12).contains(&m) || d < 1 {
        return None;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days_in = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if d > days_in[(m - 1) as usize] {
        return None;
    }
    Some(days_from_civil(y, m, d) as f64 * 86_400.0 + 43_200.0)
}

/// Линейная интерполяция по узлам xs, ys, за краями - крайние значения,
/// как `np.interp`. NaN даёт NaN.
fn interp(x: f64, nodes: &[(f64, f64)]) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    let (Some(first), Some(last)) = (nodes.first(), nodes.last()) else { return f64::NAN };
    if x <= first.0 {
        return first.1;
    }
    if x >= last.0 {
        return last.1;
    }
    for w in nodes.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if x <= x1 {
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0);
        }
    }
    last.1
}

/// Цвет заливки течений RGB 0..255 по скорости, м/с.
pub fn current_color(speed: f64) -> [f64; 3] {
    let mut out = [0.0; 3];
    for (k, c) in out.iter_mut().enumerate() {
        let nodes: Vec<(f64, f64)> = CURRENT_STOPS.iter().map(|(s, rgb)| (*s, rgb[k])).collect();
        *c = interp(speed, &nodes);
    }
    out
}

/// Цвета заливки течений по скоростям, м/с.
pub fn current_colors(speeds: &[f64]) -> Vec<[f64; 3]> {
    speeds.iter().map(|&s| current_color(s)).collect()
}

/// Поле течений для картинок тайлов заливки: поле и рамка всего мира
/// (юг, север, запад, восток).
#[derive(Debug, Clone)]
pub struct FillJob {
    pub grid: Arc<VectorGrid>,
    pub bounds: (f64, f64, f64, f64),
}

pub fn fill_job(grid: Arc<VectorGrid>) -> FillJob {
    FillJob { grid, bounds: (-90.0, 90.0, -180.0, 180.0) }
}

/// Картинка тайла заливки скорости: RGBA с премноженной альфой
/// по точкам lats, lons (по строкам картинки, сколько точек - столько
/// пикселей, лишние точки длинного массива отбрасываются). Суша и места
/// без данных прозрачны.
pub fn speed_rgba(job: &FillJob, lats: &[f64], lons: &[f64]) -> Vec<[u8; 4]> {
    let alpha_nodes = FILL_ALPHA;
    lats.iter()
        .zip(lons)
        .map(|(&lat, &lon)| {
            let (u, v) = job.grid.sample(lat, lon);
            let speed = u.hypot(v);
            // np.nan_to_num: NaN - ноль, бесконечность - наибольшее число.
            let s = if speed.is_nan() { 0.0 } else { speed.min(f64::MAX) };
            let rgb = current_color(s);
            let alpha = if speed.is_finite() { interp(s, &alpha_nodes) } else { 0.0 };
            let k = alpha / 255.0;
            let px = |x: f64| x.round_ties_even().clamp(0.0, 255.0) as u8;
            [px(rgb[0] * k), px(rgb[1] * k), px(rgb[2] * k), px(alpha)]
        })
        .collect()
}

/// Поле течений из файла ERDDAP: поле, широты узлов с севера на юг,
/// долготы с запада на восток и день данных «ГГГГ-ММ-ДД» (пусто, если
/// в файле его нет).
#[derive(Debug, Clone, PartialEq)]
pub struct Currents {
    pub grid: VectorGrid,
    pub lats: Vec<f64>,
    pub lons: Vec<f64>,
    pub day: String,
}

/// Срез переменной: значения по строкам, широты и долготы узлов.
type Plane = (Vec<f64>, Vec<f64>, Vec<f64>);

/// Значения переменной name на последних двух измерениях (широта,
/// долгота) с координатами. Первые измерения (время) - первый срез.
fn plane(file: &NcFile<'_>, name: &str) -> Result<Plane, NcError> {
    let var = file.variable(name).ok_or_else(|| error(format!("нет переменной {name}")))?;
    let n = var.dims.len();
    if n < 2 {
        return Err(error(format!("у {name} меньше двух измерений")));
    }
    let coord = |id: usize| -> Result<Vec<f64>, NcError> {
        let dim = file.dims.get(id).ok_or_else(|| error("номер измерения вне списка"))?;
        let values = file.read_f64(&dim.name)?;
        if values.len() != dim.len || values.iter().any(|x| !x.is_finite()) {
            return Err(error(format!("неверная координата {}", dim.name)));
        }
        Ok(values)
    };
    let lats = coord(var.dims[n - 2])?;
    let lons = coord(var.dims[n - 1])?;
    let count = lats.len().checked_mul(lons.len()).ok_or_else(|| error("сетка слишком велика"))?;
    let mut values = file.read_f64(name)?;
    if count == 0 || values.len() < count {
        return Err(error(format!("в {name} нет значений")));
    }
    values.truncate(count);
    Ok((values, lats, lons))
}

/// Шаг ряда координат, если он ровный: каждый шаг отличается от среднего
/// не больше чем на тысячную его долю. Знак шага - направление ряда.
fn regular_step(values: &[f64]) -> Option<f64> {
    let (first, last) = (*values.first()?, *values.last()?);
    if values.len() < 2 {
        return None;
    }
    let step = (last - first) / (values.len() - 1) as f64;
    if step == 0.0 || !step.is_finite() {
        return None;
    }
    let even = values.windows(2).all(|w| ((w[1] - w[0]) - step).abs() <= 1e-3 * step.abs());
    even.then_some(step)
}

/// День данных: глобальный атрибут `time_coverage_end`, без него -
/// последнее значение переменной `time` в секундах от 1970 года.
fn data_day(file: &NcFile<'_>) -> String {
    if let Some(text) = file.attribute("time_coverage_end").and_then(|a| a.text()) {
        return text.chars().take(10).collect();
    }
    let Some(var) = file.variable("time") else { return String::new() };
    let units = var.attr("units").and_then(|a| a.text()).unwrap_or("");
    if !units.starts_with("seconds since 1970-01-01") {
        return String::new();
    }
    match file.read_f64("time").ok().and_then(|t| t.last().copied()) {
        Some(t) if t.is_finite() => utc_day(t),
        _ => String::new(),
    }
}

/// Поле течений из байтов ответа ERDDAP на [`currents_url`]: netCDF-3
/// с переменными `ugos` и `vgos` (время, широта, долгота). Широты файла
/// идут с юга на север, строки переворачиваются на север - юг, как
/// при чтении GDAL в модуле. Заполнители дают NaN - суша.
pub fn read_currents(bytes: &[u8]) -> Result<Currents, NcError> {
    let file = NcFile::parse(bytes)?;
    let (mut u, mut lats, lons) = plane(&file, "ugos")?;
    let (mut v, lats_v, lons_v) = plane(&file, "vgos")?;
    if lats_v != lats || lons_v != lons {
        return Err(error("сетки ugos и vgos различны"));
    }
    let rows = lats.len();
    let cols = lons.len();
    let lon_step = regular_step(&lons).filter(|s| *s > 0.0).ok_or_else(|| error("долготы не ровным рядом"))?;
    let lat_step = regular_step(&lats).ok_or_else(|| error("широты не ровным рядом"))?;
    if (lat_step.abs() - lon_step).abs() > 1e-3 * lon_step {
        return Err(error("шаг широт и долгот различен"));
    }
    if lat_step > 0.0 {
        // С юга на север - строки наоборот.
        lats.reverse();
        for grid in [&mut u, &mut v] {
            let flipped: Vec<f64> = grid.chunks_exact(cols).rev().flatten().copied().collect();
            *grid = flipped;
        }
    }
    let west = lons[0];
    let north = lats[0];
    let to32 = |g: Vec<f64>| g.into_iter().map(|x| x as f32).collect::<Vec<f32>>();
    let grid = VectorGrid::new(to32(u), to32(v), rows, cols, west, north, lon_step)
        .ok_or_else(|| error("сетка течений меньше двух строк"))?;
    Ok(Currents { grid, lats, lons, day: data_day(&file) })
}
