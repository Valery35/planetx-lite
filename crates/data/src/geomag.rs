// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Магнитное поле Земли: полюса по годам, модели IGRF-14 и gufm1.
//! Перенос `core/geomag.py` модуля.
//!
//! Полюса - северный и южный магнитные полюса (точки, где стрелка стоит
//! отвесно) по годам 1590-2025 из таблиц NOAA NCEI «Wandering of the
//! Geomagnetic Poles» (модели gufm1 до 1890 года и IGRF), общественное
//! достояние США. Файл модуля data/magnetic_poles.json собирает
//! tools/build_poles.py модуля. Между годами точка идёт по дуге большого
//! круга.
//!
//! Поле - Международное геомагнитное эталонное поле IGRF-14 (IAGA, 2024),
//! коэффициенты Шмидта до степени 13 на 1900-2025 годы через 5 лет
//! и вековой ход на 2025-2030 годы, файл data/igrf14coeffs.txt как есть.
//! Расчёт - по образцу igrf_utils из выпуска IGRF-14: пересчёт
//! геодезической широты в геоцентрическую, присоединённые функции
//! Лежандра, составляющие X (север), Y (восток), Z (вниз) в нТл.
//! Полюс после последнего года таблицы - точка, где горизонтальная
//! составляющая IGRF равна нулю ([`dip_pole`]).
//!
//! Lite берёт файлы данных из хранилища модуля по сети и в себе их
//! не несёт.

use crate::contours::{self, Grid};
use std::cmp::Ordering;
use std::f64::consts::PI;

/// Файлы данных модуля.
pub const POLES_URL: &str = "https://raw.githubusercontent.com/Valery35/planetx/main/planetx/data/magnetic_poles.json";
pub const IGRF_URL: &str = "https://raw.githubusercontent.com/Valery35/planetx/main/planetx/data/igrf14coeffs.txt";

/// Большая и малая полуоси эллипсоида WGS84, км.
pub const A: f64 = 6378.137;
pub const B: f64 = 6356.752314245;
/// Радиус сферы модели IGRF, км.
pub const REFERENCE: f64 = 6371.2;

/// Источник коэффициентов поля по годам: IGRF, gufm1 или их сочетание.
pub trait Source {
    /// Первый год модели.
    fn first(&self) -> f64;
    /// Последний год модели.
    fn last(&self) -> f64;
    /// Коэффициенты Шмидта на год в порядке g10, g11, h11, g20 и далее,
    /// None вне лет модели.
    fn at(&self, year: f64) -> Option<Vec<f64>>;
    /// Степень модели - для поиска полюса ([`dip_pole`]).
    fn nmax(&self) -> usize;
    /// Степень модели на год.
    fn nmax_at(&self, _year: f64) -> usize {
        self.nmax()
    }
}

/// Коэффициенты IGRF: эпохи, коэффициенты по эпохам, вековой ход.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub epochs: Vec<f64>,
    pub coeffs: Vec<Vec<f64>>,
    pub secular: Vec<f64>,
    pub nmax: usize,
}

impl Source for Model {
    fn first(&self) -> f64 {
        self.epochs.first().copied().unwrap_or(f64::NAN)
    }

    /// Конец действия модели - последняя эпоха плюс 5 лет хода.
    fn last(&self) -> f64 {
        self.epochs.last().copied().unwrap_or(f64::NAN) + 5.0
    }

    /// Коэффициенты на год, None вне 1900-2030.
    fn at(&self, year: f64) -> Option<Vec<f64>> {
        if !(self.first() <= year && year <= self.last()) {
            return None;
        }
        let end = *self.epochs.last()?;
        if year >= end {
            let dt = year - end;
            let last = self.coeffs.last()?;
            return Some(last.iter().zip(&self.secular).map(|(c, s)| c + s * dt).collect());
        }
        let i = self.epochs.iter().rposition(|e| *e <= year)?;
        let (e0, e1) = (*self.epochs.get(i)?, *self.epochs.get(i + 1)?);
        let share = (year - e0) / (e1 - e0);
        let (a, b) = (self.coeffs.get(i)?, self.coeffs.get(i + 1)?);
        Some(a.iter().zip(b).map(|(a, b)| a + (b - a) * share).collect())
    }

    fn nmax(&self) -> usize {
        self.nmax
    }
}

fn number(text: &str) -> Result<f64, String> {
    text.parse::<f64>().map_err(|e| format!("{text}: {e}"))
}

/// Файл коэффициентов IGRF в [`Model`]. Порядок коэффициентов - g10,
/// g11, h11, g20, g21, h21, g22, h22 и далее.
pub fn parse_igrf(text: &str) -> Result<Model, String> {
    const NOT: &str = "not an IGRF coefficient file";
    let mut epochs: Option<Vec<f64>> = None;
    let mut rows: Vec<((i64, i64, bool), Vec<f64>)> = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        let Some(head) = parts.first() else {
            continue;
        };
        if line.starts_with('#') {
            continue;
        }
        if *head == "g/h" {
            // Последний столбец - годы векового хода, не эпоха.
            let found = parts.get(3..parts.len().saturating_sub(1)).unwrap_or_default();
            epochs = Some(found.iter().map(|p| number(p)).collect::<Result<_, _>>()?);
            continue;
        }
        if (*head != "g" && *head != "h") || epochs.is_none() {
            continue;
        }
        let (Some(n), Some(m)) = (parts.get(1), parts.get(2)) else {
            return Err(NOT.into());
        };
        let n = n.parse::<i64>().map_err(|e| format!("{n}: {e}"))?;
        let m = m.parse::<i64>().map_err(|e| format!("{m}: {e}"))?;
        let values = parts.get(3..).unwrap_or_default().iter().map(|p| number(p)).collect::<Result<Vec<_>, _>>()?;
        rows.push(((n, m, *head == "g"), values));
    }
    let Some(epochs) = epochs.filter(|e| !e.is_empty()) else {
        return Err(NOT.into());
    };
    let nmax = rows.iter().map(|((n, _, _), _)| *n).max().unwrap_or(0);
    // Степень не больше, чем дают строки файла: враждебная степень
    // иначе заняла бы память списком ключей.
    let full = usize::try_from(nmax).ok().and_then(|n| n.checked_mul(n.checked_add(2)?));
    if nmax < 1 || full.is_none_or(|f| f > rows.len()) {
        return Err(NOT.into());
    }
    let order: std::collections::HashMap<(i64, i64, bool), &Vec<f64>> = rows.iter().map(|(k, v)| (*k, v)).collect();
    let mut keys = Vec::new();
    for n in 1..=nmax {
        keys.push((n, 0, true));
        for m in 1..=n {
            keys.push((n, m, true));
            keys.push((n, m, false));
        }
    }
    let count = epochs.len();
    let value = |k: &(i64, i64, bool), e: usize| -> Result<f64, String> {
        order.get(k).and_then(|v| v.get(e)).copied().ok_or_else(|| format!("IGRF: no coefficient {k:?} for column {e}"))
    };
    let coeffs = (0..count).map(|e| keys.iter().map(|k| value(k, e)).collect::<Result<Vec<_>, _>>()).collect::<Result<Vec<_>, _>>()?;
    let secular = keys.iter().map(|k| value(k, count)).collect::<Result<Vec<_>, _>>()?;
    Ok(Model { epochs, coeffs, secular, nmax: nmax as usize })
}

// Модель gufm1 (Jackson, Jonkers, Walker 2000, doi:10.1098/rsta.2000.
// 0569) - поле 1590-1990 годов по судовым журналам и измерениям,
// коэффициенты Шмидта до степени 14, кубические B-сплайны во времени
// с узлами через 2.5 года. Файл gufm1.txt лицензии не несёт, сайт
// geomag.colorado.edu выкладывает его с разрешения автора модели,
// поэтому модуль его не распространяет, а скачивает (GUFM_URL).
// Разметка: строка заголовка, затем числа потоком - nmax, количество
// сплайнов nspl, nspl + 4 узла, nspl блоков коэффициентов в порядке
// IGRF (g10, g11, h11, g20 ...).
pub const GUFM_URL: &str = "https://geomag.colorado.edu/images/GUFM/gufm1.txt";
pub const GUFM_FIRST: f64 = 1590.0;
pub const GUFM_LAST: f64 = 1990.0;
pub const GUFM_ATTRIBUTION: (&str, &str) = ("gufm1, Jackson et al. 2000", "https://geomag.colorado.edu/node/669");
/// Порядок B-сплайнов gufm1 - кубические.
const GUFM_ORDER: usize = 4;

/// gufm1: узлы, коэффициенты сплайнов (nspl строк по nmax(nmax + 2)).
#[derive(Debug, Clone, PartialEq)]
pub struct Gufm {
    pub nmax: usize,
    pub knots: Vec<f64>,
    pub splines: Vec<Vec<f64>>,
}

impl Source for Gufm {
    fn first(&self) -> f64 {
        GUFM_FIRST
    }

    fn last(&self) -> f64 {
        GUFM_LAST
    }

    /// Коэффициенты на год, None вне 1590-1990.
    fn at(&self, year: f64) -> Option<Vec<f64>> {
        if !(self.first() <= year && year <= self.last()) {
            return None;
        }
        let size = self.splines.first().map_or(0, Vec::len);
        let mut out = vec![0.0; size];
        for (b, row) in bspline(year, &self.knots, GUFM_ORDER).iter().zip(&self.splines) {
            for (o, c) in out.iter_mut().zip(row) {
                *o += b * c;
            }
        }
        Some(out)
    }

    fn nmax(&self) -> usize {
        self.nmax
    }
}

/// Текст gufm1.txt в [`Gufm`].
pub fn parse_gufm(text: &str) -> Result<Gufm, String> {
    const NOT: &str = "not a gufm1 file";
    let mut lines = text.lines();
    if lines.next().is_none() {
        return Err(NOT.into());
    }
    let rest: Vec<&str> = lines.collect();
    if rest.is_empty() {
        return Err(NOT.into());
    }
    let data = rest.iter().flat_map(|l| l.split_whitespace()).map(|p| number(p).map_err(|e| format!("{NOT}: {e}"))).collect::<Result<Vec<f64>, _>>()?;
    let (Some(&nmax), Some(&nspl)) = (data.first(), data.get(1)) else {
        return Err(NOT.into());
    };
    if !(nmax >= 1.0 && nspl >= 0.0 && nmax.is_finite() && nspl.is_finite()) {
        return Err(NOT.into());
    }
    let (nmax, nspl) = (nmax as usize, nspl as usize);
    let size = nmax.checked_mul(nmax + 2);
    let need = size.and_then(|s| nspl.checked_mul(s)).and_then(|s| s.checked_add(nspl)).and_then(|s| s.checked_add(6));
    let (Some(size), Some(need)) = (size, need) else {
        return Err(NOT.into());
    };
    if data.len() != need {
        return Err(format!("gufm1: {} numbers", data.len()));
    }
    let knots = data.get(2..2 + nspl + 4).unwrap_or_default().to_vec();
    let splines = data.get(2 + nspl + 4..).unwrap_or_default().chunks(size.max(1)).map(<[f64]>::to_vec).collect();
    Ok(Gufm { nmax, knots, splines })
}

/// Значения всех B-сплайнов порядка order в точке t, Кокс - де Бур.
pub fn bspline(t: f64, knots: &[f64], order: usize) -> Vec<f64> {
    let len = knots.len();
    let Some(&end) = knots.last() else {
        return Vec::new();
    };
    let mut b: Vec<f64> = knots.windows(2).map(|w| if w[0] <= t && t < w[1] { 1.0 } else { 0.0 }).collect();
    if t == end {
        // Правый конец - последний сплайн, иначе сумма на конце нулевая.
        if let Some(k) = knots[..len - 1].iter().rposition(|k| *k < end) {
            b[k] = 1.0;
        }
    }
    for k in 2..=order {
        let Some(n) = len.checked_sub(k) else {
            return Vec::new();
        };
        let mut next = Vec::with_capacity(n);
        for i in 0..n {
            let left = knots[i + k - 1] - knots[i];
            let right = knots[i + k] - knots[i + 1];
            let a = if left > 0.0 { (t - knots[i]) / left } else { 0.0 };
            let c = if right > 0.0 { (knots[i + k] - t) / right } else { 0.0 };
            next.push(a * b[i] + c * b[i + 1]);
        }
        b = next;
    }
    b.truncate(len.saturating_sub(order));
    b
}

/// Поле по годам: gufm1 до первого года IGRF, дальше IGRF-14.
#[derive(Debug, Clone, PartialEq)]
pub struct Combined {
    pub igrf: Model,
    pub gufm: Option<Gufm>,
}

impl Combined {
    /// Модель на год: IGRF, gufm1 или None.
    pub fn part(&self, year: f64) -> Option<&dyn Source> {
        if year >= self.igrf.first() {
            return if year <= self.igrf.last() { Some(&self.igrf) } else { None };
        }
        match &self.gufm {
            Some(g) if g.first() <= year => Some(g),
            _ => None,
        }
    }
}

impl Source for Combined {
    fn first(&self) -> f64 {
        self.gufm.as_ref().map_or_else(|| self.igrf.first(), Source::first)
    }

    fn last(&self) -> f64 {
        self.igrf.last()
    }

    fn at(&self, year: f64) -> Option<Vec<f64>> {
        self.part(year)?.at(year)
    }

    /// Степень IGRF - для поиска полюса после таблицы ([`dip_pole`]).
    fn nmax(&self) -> usize {
        self.igrf.nmax
    }

    fn nmax_at(&self, year: f64) -> usize {
        self.part(year).map_or(self.igrf.nmax, Source::nmax)
    }
}

/// Присоединённые функции Лежандра Шмидта P[n][m] и их производные
/// по кошироте в верхнем треугольнике P[m][n + 1], igrf_utils.
fn legendre(nmax: usize, theta: f64) -> Vec<Vec<f64>> {
    let nmax = nmax.max(1);
    let cos_t = theta.cos();
    let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
    let mut p = vec![vec![0.0; nmax + 2]; nmax + 1];
    p[0][0] = 1.0;
    p[1][1] = sin_t;
    let root: Vec<f64> = (0..=2 * nmax * nmax).map(|i| (i as f64).sqrt()).collect();
    for m in 0..nmax {
        let tmp = root[m + m + 1] * p[m][m];
        p[m + 1][m] = cos_t * tmp;
        if m > 0 {
            p[m + 1][m + 1] = sin_t * tmp / root[m + m + 2];
        }
        for n in m + 2..=nmax {
            let d = n * n - m * m;
            let e = n + n - 1;
            p[n][m] = (e as f64 * cos_t * p[n - 1][m] - root[d - e] * p[n - 2][m]) / root[d];
        }
    }
    p[0][2] = -p[1][1];
    p[1][2] = p[1][0];
    for n in 2..=nmax {
        let nf = n as f64;
        p[0][n + 1] = -((nf * nf + nf) / 2.0).sqrt() * p[n][1];
        p[1][n + 1] = ((2.0 * (nf * nf + nf)).sqrt() * p[n][0] - (nf * nf + nf - 2.0).sqrt() * p[n][2]) / 2.0;
        for m in 2..n {
            let mf = m as f64;
            p[m][n + 1] = 0.5 * (((nf + mf) * (nf - mf + 1.0)).sqrt() * p[n][m - 1] - ((nf + mf + 1.0) * (nf - mf)).sqrt() * p[n][m + 1]);
        }
        p[n][n + 1] = (2.0 * nf).sqrt() * p[n][n - 1] / 2.0;
    }
    p
}

/// Геодезическая широта, градусы, и высота, км, - радиус, км,
/// геоцентрическая коширота, рад, и косинус и синус поворота осей.
fn geocentric(lat: f64, height: f64) -> (f64, f64, f64, f64) {
    let colat = (90.0 - lat).to_radians();
    let (a2, b2) = (A * A, B * B);
    let (ct, st) = (colat.cos(), colat.sin());
    let (one, two) = (a2 * st * st, b2 * ct * ct);
    let three = one + two;
    let rho = three.sqrt();
    let radius = (height * (height + 2.0 * rho) + (a2 * one + b2 * two) / three).sqrt();
    let cd = (height + rho) / radius;
    let sd = (a2 - b2) / rho * ct * st / radius;
    let ct2 = ct * cd - st * sd;
    let st2 = st * cd + ct * sd;
    (radius, st2.atan2(ct2), cd, sd)
}

/// Составляющие X, Y, Z по строке точек одной широты lat и долгот lons.
fn row(coeffs: &[f64], nmax: usize, lat: f64, lons: &[f64], height: f64) -> Vec<(f64, f64, f64)> {
    let (radius, theta, cd, sd) = geocentric(lat, height);
    // У самого полюса деление на синус кошироты неустойчиво.
    let theta = theta.clamp(1e-9, PI - 1e-9);
    let p = legendre(nmax, theta);
    let sin_t = p[1][1];
    let ratio = REFERENCE / radius;
    let coeff = |k: usize| coeffs.get(k).copied().unwrap_or(0.0);
    let mut out = Vec::with_capacity(lons.len());
    for lon in lons {
        let phi = lon.to_radians();
        let (mut br, mut bt, mut bp) = (0.0, 0.0, 0.0);
        let mut k = 0;
        for n in 1..=nmax {
            let rn = ratio.powf((n + 2) as f64);
            let nf = n as f64;
            let g = coeff(k);
            k += 1;
            let pn = &p[n];
            br += (nf + 1.0) * pn[0] * rn * g;
            bt -= p[0][n + 1] * rn * g;
            for (m, pm) in p.iter().enumerate().take(n + 1).skip(1) {
                let (g, h) = (coeff(k), coeff(k + 1));
                k += 2;
                let mf = m as f64;
                let (sm, cm) = (mf * phi).sin_cos();
                br += (nf + 1.0) * pn[m] * rn * (g * cm + h * sm);
                bt -= pm[n + 1] * rn * (g * cm + h * sm);
                bp += mf * pn[m] / sin_t * rn * (g * sm - h * cm);
            }
        }
        let (x, y, z) = (-bt, bp, -br);
        // Из геоцентрической системы в геодезическую.
        out.push((x * cd + z * sd, y, z * cd - x * sd));
    }
    out
}

/// Составляющие поля X (север), Y (восток), Z (вниз), нТл, в точке
/// с геодезической широтой и долготой, градусы, на высоте height км.
pub fn components(coeffs: &[f64], nmax: usize, lat: f64, lon: f64, height: f64) -> (f64, f64, f64) {
    row(coeffs, nmax, lat, &[lon], height).first().copied().unwrap_or((0.0, 0.0, 0.0))
}

/// [`components`] сеткой: X, Y, Z (строки lats, столбцы lons), нТл.
pub fn components_grid(coeffs: &[f64], nmax: usize, lats: &[f64], lons: &[f64], height: f64) -> (Grid, Grid, Grid) {
    let (rows, cols) = (lats.len(), lons.len());
    let mut x = Vec::with_capacity(rows * cols);
    let mut y = Vec::with_capacity(rows * cols);
    let mut z = Vec::with_capacity(rows * cols);
    for lat in lats {
        for (a, b, c) in row(coeffs, nmax, *lat, lons, height) {
            x.push(a);
            y.push(b);
            z.push(c);
        }
    }
    (Grid { rows, cols, data: x }, Grid { rows, cols, data: y }, Grid { rows, cols, data: z })
}

/// Поле в точке на год: склонение и наклонение, градусы, полная
/// и горизонтальная напряжённость и составляющие X, Y, Z, нТл.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Field {
    pub declination: f64,
    pub inclination: f64,
    pub total: f64,
    pub horizontal: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// Поле на год или None вне лет модели.
pub fn field(model: &(impl Source + ?Sized), lat: f64, lon: f64, year: f64, height: f64) -> Option<Field> {
    let coeffs = model.at(year)?;
    let (x, y, z) = components(&coeffs, model.nmax_at(year), lat, lon, height);
    let h = x.hypot(y);
    Some(Field { declination: y.atan2(x).to_degrees(), inclination: z.atan2(h).to_degrees(), total: (h * h + z * z).sqrt(), horizontal: h, x, y, z })
}

// Полюса.

/// Точка таблицы полюса: год, широта, долгота.
pub type Track = Vec<(f64, f64, f64)>;

/// Северный и южный магнитные полюса по возрастанию лет.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Poles {
    pub north: Track,
    pub south: Track,
}

/// data/magnetic_poles.json - {"north": [[год, широта, долгота]],
/// "south": [...]}.
pub fn parse_poles(text: &str) -> Result<Poles, String> {
    let json: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let track = |key: &str| -> Result<Track, String> {
        let rows = json.get(key).and_then(|v| v.as_array()).ok_or_else(|| format!("magnetic poles: no {key}"))?;
        let mut out = rows
            .iter()
            .map(|row| {
                let row = row.as_array()?;
                Some((row.first()?.as_f64()?, row.get(1)?.as_f64()?, row.get(2)?.as_f64()?))
            })
            .collect::<Option<Track>>()
            .ok_or_else(|| format!("magnetic poles: bad row in {key}"))?;
        let order = |a: &(f64, f64, f64), b: &(f64, f64, f64)| -> Ordering { a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.total_cmp(&b.2)) };
        out.sort_by(order);
        Ok(out)
    };
    Ok(Poles { north: track("north")?, south: track("south")? })
}

fn unit(lat: f64, lon: f64) -> [f64; 3] {
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    [la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin()]
}

fn point(v: [f64; 3]) -> (f64, f64) {
    (v[2].atan2(v[0].hypot(v[1])).to_degrees(), v[1].atan2(v[0]).to_degrees())
}

/// Точка на дуге большого круга между (широта, долгота) a и b.
fn between(a: (f64, f64), b: (f64, f64), share: f64) -> (f64, f64) {
    let (ua, ub) = (unit(a.0, a.1), unit(b.0, b.1));
    let dot: f64 = ua.iter().zip(&ub).map(|(p, q)| p * q).sum();
    let angle = dot.clamp(-1.0, 1.0).acos();
    if angle < 1e-12 {
        return a;
    }
    let sa = ((1.0 - share) * angle).sin() / angle.sin();
    let sb = (share * angle).sin() / angle.sin();
    point([sa * ua[0] + sb * ub[0], sa * ua[1] + sb * ub[1], sa * ua[2] + sb * ub[2]])
}

/// Полюс (широта, долгота) на год по таблице или None вне её лет.
pub fn pole_at(track: &[(f64, f64, f64)], year: f64) -> Option<(f64, f64)> {
    let (first, last) = (track.first()?, track.last()?);
    if !(first.0 <= year && year <= last.0) {
        return None;
    }
    for pair in track.windows(2) {
        let ((y0, la0, lo0), (y1, la1, lo1)) = (pair[0], pair[1]);
        if y1 >= year {
            let share = if y1 == y0 { 0.0 } else { (year - y0) / (y1 - y0) };
            return Some(between((la0, lo0), (la1, lo1), share));
        }
    }
    Some((last.1, last.2))
}

/// Точка в north, east км от (lat, lon) на сфере среднего радиуса.
fn offset(lat: f64, lon: f64, north: f64, east: f64) -> (f64, f64) {
    let distance = north.hypot(east) / REFERENCE;
    if distance == 0.0 {
        return (lat, lon);
    }
    let bearing = east.atan2(north);
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    let la2 = (la.sin() * distance.cos() + la.cos() * distance.sin() * bearing.cos()).asin();
    let lo2 = lo + (bearing.sin() * distance.sin() * la.cos()).atan2(distance.cos() - la.sin() * la2.sin());
    (la2.to_degrees(), (lo2.to_degrees() + 540.0).rem_euclid(360.0) - 180.0)
}

/// Шагов поиска полюса и допуск горизонтальной составляющей, нТл.
const DIP_STEPS: usize = 30;
const DIP_TOLERANCE: f64 = 0.01;

/// Магнитный полюс на год по модели - точка с нулевой горизонтальной
/// составляющей, поиск Ньютоном от точки start. None вне лет модели
/// или без схождения.
pub fn dip_pole(model: &(impl Source + ?Sized), year: f64, start: (f64, f64)) -> Option<(f64, f64)> {
    let coeffs = model.at(year)?;
    let nmax = model.nmax();
    let (mut lat, mut lon) = start;
    let step = 1.0; // км, шаг конечных разностей
    for _ in 0..DIP_STEPS {
        let (x, y, _) = components(&coeffs, nmax, lat, lon, 0.0);
        if x.hypot(y) < DIP_TOLERANCE {
            return Some((lat, lon));
        }
        // Производные в местных осях исходной точки: у полюса оси
        // соседней точки повёрнуты, поэтому сдвиг меряется так же.
        let derivative = |dn: f64, de: f64| {
            let (la2, lo2) = offset(lat, lon, dn, de);
            let (x2, y2, _) = components(&coeffs, nmax, la2, lo2, 0.0);
            ((x2 - x) / step, (y2 - y) / step)
        };
        let ((a, c), (b, d)) = (derivative(step, 0.0), derivative(0.0, step));
        let det = a * d - b * c;
        if det == 0.0 {
            return None;
        }
        let mut north = -(d * x - b * y) / det;
        let mut east = -(-c * x + a * y) / det;
        // Не больше 200 км за шаг - начало поиска рядом с полюсом.
        let size = north.hypot(east);
        if size > 200.0 {
            north = north * 200.0 / size;
            east = east * 200.0 / size;
        }
        (lat, lon) = offset(lat, lon, north, east);
    }
    None
}

/// Дней от 1 января 1970 года до даты григорианского календаря.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Год с дробью по секундам UTC от 1970 года, по календарю: 1 января
/// - целый год. Средний год давал 2019.997 вместо 2020.
pub fn year_of(seconds: f64) -> f64 {
    if !seconds.is_finite() {
        return f64::NAN;
    }
    // Дни - в пределах, где счёт календаря не переполняется.
    let days = (seconds / 86400.0).floor().clamp(-1e12, 1e12) as i64;
    let mut year = 1970 + (days as f64 / 365.2425).floor() as i64;
    while days_from_civil(year, 1, 1) > days {
        year -= 1;
    }
    while days_from_civil(year + 1, 1, 1) <= days {
        year += 1;
    }
    let start = days_from_civil(year, 1, 1) as f64 * 86400.0;
    let end = days_from_civil(year + 1, 1, 1) as f64 * 86400.0;
    year as f64 + (seconds - start) / (end - start)
}

/// Полюс на год: по таблице в её годах, позже - по модели от
/// последней точки таблицы. None вне лет таблицы и модели.
pub fn pole(model: &(impl Source + ?Sized), track: &[(f64, f64, f64)], year: f64) -> Option<(f64, f64)> {
    let found = pole_at(track, year);
    let Some(last) = track.last() else {
        return found;
    };
    if found.is_some() || year < last.0 {
        return found;
    }
    dip_pole(model, year, (last.1, last.2))
}

// Линии равного склонения - просьба автора модуля от 16 октября 2026
// года после списка геофизики. Поле считается сеткой GRID_STEP градусов,
// линии - через ISOGONIC_STEP градусов, нулевая - агона. Шаги - выбор
// помощника, утверждает автор.
pub const GRID_STEP: f64 = 2.0;
pub const ISOGONIC_STEP: f64 = 10.0;
/// Широта края сетки, у полюса склонение без смысла.
pub const GRID_LAT: f64 = 88.0;
/// Наибольшее количество узлов оси сетки - сторож шага.
const MAX_AXIS: f64 = 1.0e7;

/// Значения start, start + step... меньше stop, как numpy.arange.
fn arange(start: f64, stop: f64, step: f64) -> Vec<f64> {
    if step.is_nan() || step <= 0.0 {
        return Vec::new();
    }
    let count = ((stop - start) / step).ceil();
    if !(count > 0.0 && count <= MAX_AXIS) {
        return Vec::new();
    }
    (0..count as usize).map(|i| start + i as f64 * step).collect()
}

/// Склонение, градусы, на сетке: широты, долготы, значения
/// (строки - широты с юга).
pub fn declination_grid(coeffs: &[f64], nmax: usize, step: f64) -> (Vec<f64>, Vec<f64>, Grid) {
    let lats = arange(-GRID_LAT, GRID_LAT + step / 2.0, step);
    let lons = arange(-180.0, 180.0 + step / 2.0, step);
    let (x, y, _) = components_grid(coeffs, nmax, &lats, &lons, 0.0);
    let data = x.data.iter().zip(&y.data).map(|(x, y)| y.atan2(*x).to_degrees()).collect();
    let values = Grid { rows: lats.len(), cols: lons.len(), data };
    (lats, lons, values)
}

/// Значение axis в дробном номере узла x, как numpy.interp по номерам.
fn interp(axis: &[f64], x: f64) -> f64 {
    let Some(&last) = axis.last() else {
        return f64::NAN;
    };
    let top = (axis.len() - 1) as f64;
    let x = x.max(0.0).min(top);
    let i = x.floor() as usize;
    match (axis.get(i), axis.get(i + 1)) {
        (Some(a), Some(b)) => a + (b - a) * (x - i as f64),
        (Some(a), None) => *a,
        _ => last,
    }
}

/// Линии равного склонения: список (значение, [(широта, долгота)]).
/// Ячейка, где склонение переходит через ±180°, пропускается - там
/// угол не непрерывен.
pub fn isogonic_lines(lats: &[f64], lons: &[f64], values: &Grid, step: f64) -> Vec<(f64, Vec<(f64, f64)>)> {
    let (rows, cols) = (values.rows, values.cols);
    if values.data.len() != rows * cols {
        return Vec::new();
    }
    // Ячейка с переходом через ±180° - NaN в её угле, segments её
    // пропускает. Соседние ячейки того же угла пропадают тоже, это
    // места у полюсов, где линии сходятся.
    let mut cut = values.clone();
    for r in 0..rows.saturating_sub(1) {
        for c in 0..cols.saturating_sub(1) {
            let corners = [values.at(r, c), values.at(r, c + 1), values.at(r + 1, c), values.at(r + 1, c + 1)];
            if corners.iter().any(|v| v.is_nan()) {
                continue;
            }
            let hi = corners.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let lo = corners.iter().copied().fold(f64::INFINITY, f64::min);
            if hi - lo > 180.0 {
                cut.data[r * cols + c] = f64::NAN;
            }
        }
    }
    let mut out = Vec::new();
    for level in arange(-180.0 + step, 180.0 - step / 2.0, step) {
        let found = contours::segments(&cut, level);
        for line in contours::chain(&found, 6) {
            if line.len() < 2 {
                continue;
            }
            let points = line.iter().map(|p| (interp(lats, p[0]), interp(lons, p[1]))).collect();
            out.push((level, points));
        }
    }
    out
}

/// След полюса - точки таблицы не позже года. start - год левого
/// бегунка шкалы: след начинается с полюса на этот год, а не с первой
/// точки таблицы. Замечание пользователя модуля от 14 октября 2026
/// года - левый бегунок след не обрезал.
pub fn trail(track: &[(f64, f64, f64)], year: f64, start: Option<f64>) -> Vec<(f64, f64)> {
    let Some(first) = track.first() else {
        return Vec::new();
    };
    let start = match start {
        Some(s) if s.is_nan() || s > first.0 => s,
        _ => return track.iter().filter(|p| p.0 <= year).map(|p| (p.1, p.2)).collect(),
    };
    let mut path: Vec<(f64, f64)> = track.iter().filter(|p| start < p.0 && p.0 <= year).map(|p| (p.1, p.2)).collect();
    if let Some(head) = pole_at(track, start).filter(|_| start <= year) {
        path.insert(0, head);
    }
    path
}
