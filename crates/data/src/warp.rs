// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Привязка изображения по опорным точкам без GDAL. Замена
//! `gdal.Translate` с опорными точками и `gdal.Warp` в EPSG:4326
//! из `ui/rumsey.py` модуля.
//!
//! Преобразования повторяют GDAL:
//! - полином степени 1, 2 или 3 - метод наименьших квадратов, как
//!   GCPTransformer (`gcp_polynomial.cpp`), члены в том же порядке
//!   1, x, y, x², xy, y², x³, x²y, xy², y³;
//! - тонкая пластина - как TPS-преобразование GDAL
//!   (`thinplatespline.cpp`), ядро r² ln r² и аффинная часть.
//!
//! Прямое направление (пиксели в долготу и широту) и обратное
//! подбираются по точкам отдельно, как в GDAL, обратное не получается
//! численным обращением прямого. Координаты перед подбором переносятся
//! к среднему и делятся на размах. Решение от этого не меняется,
//! меняется только обусловленность системы.
//!
//! Выходная сетка - как у `gdal.Warp` без заданных размеров:
//! рамка и шаг по `GDALSuggestedWarpOutput2`, координаты источника
//! по строке - приближённым преобразованием с ошибкой не больше
//! [`APPROX_ERROR`] пикселя, как у gdalwarp по умолчанию, отсчёт -
//! билинейный с учётом альфы источника.

use crate::rumsey::Warp;

/// Опорная точка: x, y в пикселях источника, долгота, широта, градусы.
/// Порядок как у [`crate::rumsey::scaled_points`].
pub type Gcp = (f64, f64, f64, f64);

/// Наибольшая ошибка приближённого преобразования по строке, пиксели
/// источника. У gdalwarp по умолчанию 0.125.
pub const APPROX_ERROR: f64 = 0.125;
/// Шагов по каждой стороне источника при поиске рамки, как
/// в `GDALSuggestedWarpOutput2`.
const EDGE_STEPS: usize = 20;
/// Порог вырожденности: отношение собственных чисел разброса точек.
const FLAT: f64 = 1e-12;
/// Порог вырожденности системы относительно наибольшего ведущего
/// элемента.
const SINGULAR: f64 = 1e-10;
/// Плотность ниже порога - пиксель пустой, как в GDALWarpKernel.
const DENSITY_MIN: f64 = 1e-5;

/// Перенос к среднему и деление на размах.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Norm {
    mx: f64,
    my: f64,
    s: f64,
}

impl Norm {
    /// Нормировка по точкам. Ошибка - точки совпадают или лежат на одной
    /// прямой.
    fn of(points: &[(f64, f64)]) -> Result<Self, String> {
        let n = points.len() as f64;
        let mx = points.iter().map(|p| p.0).sum::<f64>() / n;
        let my = points.iter().map(|p| p.1).sum::<f64>() / n;
        let s = points.iter().fold(0.0_f64, |m, p| m.max((p.0 - mx).abs()).max((p.1 - my).abs()));
        if !(s > 0.0 && s.is_finite()) {
            return Err("опорные точки совпадают".to_string());
        }
        let (mut cxx, mut cyy, mut cxy) = (0.0, 0.0, 0.0);
        for p in points {
            let (dx, dy) = ((p.0 - mx) / s, (p.1 - my) / s);
            cxx += dx * dx;
            cyy += dy * dy;
            cxy += dx * dy;
        }
        // Собственные числа матрицы разброса 2 × 2: меньшее через
        // определитель, без вычитания близких чисел.
        let half = (cxx + cyy) / 2.0;
        let big = half + (half * half - (cxx * cyy - cxy * cxy)).max(0.0).sqrt();
        let small = (cxx * cyy - cxy * cxy) / big;
        if !(big > 0.0 && small > FLAT * big) {
            return Err("опорные точки лежат на одной прямой".to_string());
        }
        Ok(Self { mx, my, s })
    }

    fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        ((x - self.mx) / self.s, (y - self.my) / self.s)
    }
}

/// Количество членов полинома степени order.
fn term_count(order: u32) -> usize {
    match order {
        1 => 3,
        2 => 6,
        _ => 10,
    }
}

/// Члены полинома в порядке GDAL: 1, x, y, x², xy, y², x³, x²y, xy², y³.
fn terms(x: f64, y: f64) -> [f64; 10] {
    [1.0, x, y, x * x, x * y, y * y, x * x * x, x * x * y, x * y * y, y * y * y]
}

/// Полином одного направления: из (x, y) в (u, v).
#[derive(Debug, Clone, PartialEq)]
pub struct Polynomial {
    order: u32,
    norm: Norm,
    cu: Vec<f64>,
    cv: Vec<f64>,
}

impl Polynomial {
    /// Полином степени order (1, 2 или 3) по парам точек from - to
    /// методом наименьших квадратов. Точек нужно не меньше количества
    /// членов: 3, 6 и 10. Ошибка - мало точек, точки на одной прямой,
    /// система вырождена.
    pub fn fit(from: &[(f64, f64)], to: &[(f64, f64)], order: u32) -> Result<Self, String> {
        if !(1..=3).contains(&order) {
            return Err(format!("степень полинома {order}, нужна 1, 2 или 3"));
        }
        let m = term_count(order);
        let n = from.len();
        if n != to.len() {
            return Err("разное количество точек".to_string());
        }
        if n < m {
            return Err(format!("мало опорных точек: {n}, для степени {order} нужно не меньше {m}"));
        }
        let norm = Norm::of(from)?;
        let mut a = vec![0.0; n * m];
        let mut b = vec![0.0; n * 2];
        for (i, (p, q)) in from.iter().zip(to).enumerate() {
            let (x, y) = norm.apply(p.0, p.1);
            let t = terms(x, y);
            if let Some(row) = a.get_mut(i * m..(i + 1) * m) {
                row.copy_from_slice(&t[..m]);
            }
            if let Some(rhs) = b.get_mut(i * 2..i * 2 + 2) {
                rhs.copy_from_slice(&[q.0, q.1]);
            }
        }
        let (cu, cv) = least_squares(&mut a, n, m, &mut b)?;
        Ok(Self { order, norm, cu, cv })
    }

    /// Степень полинома.
    pub fn order(&self) -> u32 {
        self.order
    }

    /// Значение в точке (x, y).
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let (x, y) = self.norm.apply(x, y);
        let t = terms(x, y);
        let u = self.cu.iter().zip(&t).map(|(c, t)| c * t).sum();
        let v = self.cv.iter().zip(&t).map(|(c, t)| c * t).sum();
        (u, v)
    }
}

/// Наименьшие квадраты отражениями Хаусхолдера. a - матрица n × m
/// по строкам, b - n × 2. Ошибка - столбцы зависимы.
fn least_squares(a: &mut [f64], n: usize, m: usize, b: &mut [f64]) -> Result<(Vec<f64>, Vec<f64>), String> {
    let singular = || "система опорных точек вырождена".to_string();
    let mut diag = vec![0.0; m];
    let mut v = vec![0.0; n];
    for k in 0..m {
        let norm = (k..n).map(|i| a[i * m + k] * a[i * m + k]).sum::<f64>().sqrt();
        if !(norm > 0.0 && norm.is_finite()) {
            return Err(singular());
        }
        let akk = a[k * m + k];
        let alpha = if akk > 0.0 { -norm } else { norm };
        for i in k..n {
            v[i] = a[i * m + k];
        }
        v[k] -= alpha;
        let vv: f64 = (k..n).map(|i| v[i] * v[i]).sum();
        if vv > 0.0 {
            for j in k..m {
                let dot: f64 = (k..n).map(|i| v[i] * a[i * m + j]).sum();
                let f = 2.0 * dot / vv;
                for i in k..n {
                    a[i * m + j] -= f * v[i];
                }
            }
            for c in 0..2 {
                let dot: f64 = (k..n).map(|i| v[i] * b[i * 2 + c]).sum();
                let f = 2.0 * dot / vv;
                for i in k..n {
                    b[i * 2 + c] -= f * v[i];
                }
            }
        }
        diag[k] = alpha;
    }
    let top = diag.iter().fold(0.0_f64, |t, d| t.max(d.abs()));
    if diag.iter().any(|d| d.abs() <= SINGULAR * top) {
        return Err(singular());
    }
    let mut out = [vec![0.0; m], vec![0.0; m]];
    for (c, x) in out.iter_mut().enumerate() {
        for k in (0..m).rev() {
            let s: f64 = (k + 1..m).map(|j| a[k * m + j] * x[j]).sum();
            x[k] = (b[k * 2 + c] - s) / diag[k];
        }
    }
    let [u, v] = out;
    if u.iter().chain(&v).any(|c| !c.is_finite()) {
        return Err(singular());
    }
    Ok((u, v))
}

/// Ядро тонкой пластины GDAL: r² ln r², ноль в узле.
fn kernel(r2: f64) -> f64 {
    if r2 > 0.0 { r2 * r2.ln() } else { 0.0 }
}

/// Тонкая пластина одного направления: из (x, y) в (u, v), проходит
/// через все точки.
#[derive(Debug, Clone, PartialEq)]
pub struct Tps {
    norm: Norm,
    nodes: Vec<(f64, f64)>,
    /// Веса узлов и три коэффициента аффинной части 1, x, y.
    wu: Vec<f64>,
    wv: Vec<f64>,
}

impl Tps {
    /// Тонкая пластина по парам точек from - to. Точек нужно не меньше
    /// трёх. Ошибка - мало точек, точки на одной прямой, совпадающие
    /// точки.
    pub fn fit(from: &[(f64, f64)], to: &[(f64, f64)]) -> Result<Self, String> {
        let n = from.len();
        if n != to.len() {
            return Err("разное количество точек".to_string());
        }
        if n < 3 {
            return Err(format!("мало опорных точек: {n}, для тонкой пластины нужно не меньше 3"));
        }
        let norm = Norm::of(from)?;
        let nodes: Vec<(f64, f64)> = from.iter().map(|p| norm.apply(p.0, p.1)).collect();
        let size = n + 3;
        let mut a = vec![0.0; size * size];
        let mut b = vec![0.0; size * 2];
        for (i, p) in nodes.iter().enumerate() {
            for (j, q) in nodes.iter().enumerate() {
                let (dx, dy) = (p.0 - q.0, p.1 - q.1);
                a[i * size + j] = kernel(dx * dx + dy * dy);
            }
            for (k, t) in [1.0, p.0, p.1].into_iter().enumerate() {
                a[i * size + n + k] = t;
                a[(n + k) * size + i] = t;
            }
        }
        for (i, q) in to.iter().enumerate() {
            b[i * 2] = q.0;
            b[i * 2 + 1] = q.1;
        }
        let (wu, wv) = gauss(&mut a, size, &mut b)?;
        Ok(Self { norm, nodes, wu, wv })
    }

    /// Значение в точке (x, y).
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let (x, y) = self.norm.apply(x, y);
        let n = self.nodes.len();
        let (mut u, mut v) = (0.0, 0.0);
        for ((p, wu), wv) in self.nodes.iter().zip(&self.wu).zip(&self.wv) {
            let (dx, dy) = (x - p.0, y - p.1);
            let k = kernel(dx * dx + dy * dy);
            u += wu * k;
            v += wv * k;
        }
        let affine = |w: &[f64]| match w.get(n..n + 3) {
            Some(c) => c[0] + c[1] * x + c[2] * y,
            None => f64::NAN,
        };
        (u + affine(&self.wu), v + affine(&self.wv))
    }
}

/// Решение системы size × size с двумя правыми частями методом Гаусса
/// с выбором ведущего элемента по столбцу. Ошибка - система вырождена.
fn gauss(a: &mut [f64], size: usize, b: &mut [f64]) -> Result<(Vec<f64>, Vec<f64>), String> {
    let singular = || "система опорных точек вырождена".to_string();
    let scale = a.iter().fold(0.0_f64, |t, x| t.max(x.abs()));
    if !(scale > 0.0 && scale.is_finite()) {
        return Err(singular());
    }
    for k in 0..size {
        let mut pivot = k;
        for i in k + 1..size {
            if a[i * size + k].abs() > a[pivot * size + k].abs() {
                pivot = i;
            }
        }
        let p = a[pivot * size + k];
        if p.is_nan() || p.abs() <= SINGULAR * scale {
            return Err(singular());
        }
        if pivot != k {
            for j in 0..size {
                a.swap(k * size + j, pivot * size + j);
            }
            b.swap(k * 2, pivot * 2);
            b.swap(k * 2 + 1, pivot * 2 + 1);
        }
        for i in k + 1..size {
            let f = a[i * size + k] / p;
            if f == 0.0 {
                continue;
            }
            for j in k..size {
                a[i * size + j] -= f * a[k * size + j];
            }
            b[i * 2] -= f * b[k * 2];
            b[i * 2 + 1] -= f * b[k * 2 + 1];
        }
    }
    let mut out = [vec![0.0; size], vec![0.0; size]];
    for (c, x) in out.iter_mut().enumerate() {
        for k in (0..size).rev() {
            let s: f64 = (k + 1..size).map(|j| a[k * size + j] * x[j]).sum();
            x[k] = (b[k * 2 + c] - s) / a[k * size + k];
        }
    }
    let [u, v] = out;
    if u.iter().chain(&v).any(|c| !c.is_finite()) {
        return Err(singular());
    }
    Ok((u, v))
}

/// Подобранное преобразование одного направления.
#[derive(Debug, Clone, PartialEq)]
pub enum Fit {
    Polynomial(Polynomial),
    Tps(Tps),
}

impl Fit {
    /// Преобразование способом warp по парам точек from - to.
    pub fn new(from: &[(f64, f64)], to: &[(f64, f64)], warp: Warp) -> Result<Self, String> {
        match warp {
            Warp::Tps => Tps::fit(from, to).map(Self::Tps),
            Warp::Polynomial(order) => Polynomial::fit(from, to, order).map(Self::Polynomial),
        }
    }

    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        match self {
            Self::Polynomial(p) => p.apply(x, y),
            Self::Tps(t) => t.apply(x, y),
        }
    }
}

/// Преобразование опорных точек в обе стороны: прямое из пикселей
/// источника в долготу и широту, обратное - из долготы и широты
/// в пиксели. Пиксели - от верхнего левого угла изображения, как
/// у опорных точек GDAL.
#[derive(Debug, Clone, PartialEq)]
pub struct GcpTransform {
    forward: Fit,
    inverse: Fit,
}

impl GcpTransform {
    /// Преобразование по опорным точкам (x, y, долгота, широта) способом
    /// warp. Ошибка - мало точек, точки не числа, точки на одной прямой,
    /// система вырождена.
    pub fn new(gcps: &[Gcp], warp: Warp) -> Result<Self, String> {
        if gcps.iter().any(|g| !(g.0.is_finite() && g.1.is_finite() && g.2.is_finite() && g.3.is_finite())) {
            return Err("опорная точка не число".to_string());
        }
        let pixels: Vec<(f64, f64)> = gcps.iter().map(|g| (g.0, g.1)).collect();
        let geo: Vec<(f64, f64)> = gcps.iter().map(|g| (g.2, g.3)).collect();
        let forward = Fit::new(&pixels, &geo, warp)?;
        let inverse = Fit::new(&geo, &pixels, warp)?;
        Ok(Self { forward, inverse })
    }

    /// Пиксель источника (x, y) - долгота и широта.
    pub fn forward(&self, x: f64, y: f64) -> (f64, f64) {
        self.forward.apply(x, y)
    }

    /// Долгота и широта - пиксель источника (x, y).
    pub fn inverse(&self, lon: f64, lat: f64) -> (f64, f64) {
        self.inverse.apply(lon, lat)
    }
}

/// Привязанное изображение: RGBA по строкам с севера, размер и рамка
/// запад, юг, восток, север в градусах. Пиксели квадратные в градусах.
#[derive(Debug, Clone, PartialEq)]
pub struct Warped {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub bbox: [f64; 4],
}

/// Изображение src (RGBA width × height) в сетку долготы и широты
/// по опорным точкам gcps способом warp. Большая сторона результата -
/// не больше max_side. Ошибка - неверный размер изображения, ошибка
/// подбора преобразования, рамка не находится.
pub fn warp_rgba(src: &[u8], width: u32, height: u32, gcps: &[Gcp], warp: Warp, max_side: u32) -> Result<Warped, String> {
    let transform = GcpTransform::new(gcps, warp)?;
    warp_with(src, width, height, &transform, max_side)
}

/// То же, что [`warp_rgba`], с готовым преобразованием.
pub fn warp_with(src: &[u8], width: u32, height: u32, transform: &GcpTransform, max_side: u32) -> Result<Warped, String> {
    let expected = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(4));
    if width == 0 || height == 0 || expected != Some(src.len()) {
        return Err(format!("размер изображения {width} × {height} не совпадает с данными"));
    }
    if max_side == 0 {
        return Err("наибольшая сторона результата - ноль".to_string());
    }
    let (w, h) = (f64::from(width), f64::from(height));
    // Точки по четырём сторонам в порядке GDALSuggestedWarpOutput2: верх,
    // низ, левая, правая. Первая точка - (0, 0), последняя - (w, h).
    let mut samples = Vec::with_capacity((EDGE_STEPS + 1) * 4);
    for k in 0..=EDGE_STEPS {
        let r = k as f64 / EDGE_STEPS as f64;
        for (x, y) in [(r * w, 0.0), (r * w, h), (0.0, r * h), (w, r * h)] {
            samples.push(transform.forward(x, y));
        }
    }
    let finite = |p: &(f64, f64)| p.0.is_finite() && p.1.is_finite();
    let (mut west, mut south, mut east, mut north) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in samples.iter().filter(|p| finite(p)) {
        west = west.min(p.0);
        east = east.max(p.0);
        south = south.min(p.1);
        north = north.max(p.1);
    }
    if !(west.is_finite() && east.is_finite() && south.is_finite() && north.is_finite()) {
        return Err("рамка привязанного изображения не находится".to_string());
    }
    // Шаг: столько же пикселей по диагонали, сколько у источника.
    // Диагональ - от образа угла (0, 0) до образа угла (w, h), при нуле -
    // диагональ рамки, как в GDAL.
    let (mut dx, mut dy) = (0.0, 0.0);
    if let (Some(a), Some(b)) = (samples.first().filter(|p| finite(p)), samples.last().filter(|p| finite(p))) {
        dx = b.0 - a.0;
        dy = b.1 - a.1;
    }
    if dx == 0.0 || dy == 0.0 {
        dx = east - west;
        dy = north - south;
    }
    let mut res = dx.hypot(dy) / w.hypot(h);
    if !(res > 0.0 && res.is_finite()) {
        return Err("шаг привязанного изображения не находится".to_string());
    }
    let side = f64::from(max_side);
    let count = |extent: f64, res: f64| (extent / res + 0.5).floor().max(1.0);
    let (mut nx, mut ny) = (count(east - west, res), count(north - south, res));
    if nx.max(ny) > side {
        res = (east - west).max(north - south) / side;
        nx = count(east - west, res).min(side);
        ny = count(north - south, res).min(side);
    }
    // Значения ограничены max_side, приведение без потерь.
    let (nx, ny) = (nx as u32, ny as u32);
    let bbox = [west, north - f64::from(ny) * res, west + f64::from(nx) * res, north];
    let row_len = nx as usize * 4;
    let mut rgba = vec![0_u8; row_len * ny as usize];
    let mut coords = vec![(0.0, 0.0); nx as usize];
    for (j, row) in rgba.chunks_exact_mut(row_len).enumerate() {
        let lat = north - (j as f64 + 0.5) * res;
        approx_span(transform, west, res, lat, 0, &mut coords);
        for (pixel, c) in row.as_chunks_mut::<4>().0.iter_mut().zip(&coords) {
            *pixel = sample(src, width, height, c.0, c.1);
        }
    }
    Ok(Warped { rgba, width: nx, height: ny, bbox })
}

/// Пиксели источника для центров выходных пикселей first.. строки lat
/// приближённым преобразованием, как GDALApproxTransform: концы
/// и середина считаются точно, при ошибке середины не больше
/// [`APPROX_ERROR`] отрезок заполняется линейно, иначе делится пополам.
fn approx_span(t: &GcpTransform, west: f64, res: f64, lat: f64, first: usize, out: &mut [(f64, f64)]) {
    let n = out.len();
    let lon = |i: usize| west + ((first + i) as f64 + 0.5) * res;
    if n <= 5 {
        for (i, c) in out.iter_mut().enumerate() {
            *c = t.inverse(lon(i), lat);
        }
        return;
    }
    let last = n - 1;
    let mid = last / 2;
    let a = t.inverse(lon(0), lat);
    let b = t.inverse(lon(last), lat);
    let m = t.inverse(lon(mid), lat);
    let f = mid as f64 / last as f64;
    let error = (a.0 + (b.0 - a.0) * f - m.0).abs() + (a.1 + (b.1 - a.1) * f - m.1).abs();
    if error.is_finite() && error <= APPROX_ERROR {
        for (i, c) in out.iter_mut().enumerate() {
            let f = i as f64 / last as f64;
            *c = (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f);
        }
        return;
    }
    let (left, right) = out.split_at_mut(mid);
    approx_span(t, west, res, lat, first, left);
    approx_span(t, west, res, lat, first + mid, right);
}

/// Билинейный отсчёт RGBA в точке (x, y) пикселей источника от верхнего
/// левого угла, центр пикселя (i, j) - (i + 0.5, j + 0.5). Цвет
/// смешивается с весом альфы, соседи за краем отбрасываются, веса
/// оставшихся нормируются, как в GDALWarpKernel. Точка вне изображения
/// и неверные данные - прозрачный пиксель.
pub fn sample(src: &[u8], width: u32, height: u32, x: f64, y: f64) -> [u8; 4] {
    let (w, h) = (f64::from(width), f64::from(height));
    let size = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(4));
    if !(x >= 0.0 && x < w && y >= 0.0 && y < h) || size != Some(src.len()) {
        return [0; 4];
    }
    let (sx, sy) = (x - 0.5, y - 0.5);
    let (x0, y0) = (sx.floor(), sy.floor());
    let (fx, fy) = (sx - x0, sy - y0);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let (mut total, mut density) = (0.0, 0.0);
    let mut colour = [0.0_f64; 3];
    for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
        let yi = y0 + dy;
        if wy <= 0.0 || yi < 0 || yi >= i64::from(height) {
            continue;
        }
        for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
            let xi = x0 + dx;
            if wx <= 0.0 || xi < 0 || xi >= i64::from(width) {
                continue;
            }
            let index = (yi as usize * width as usize + xi as usize) * 4;
            let Some(p) = src.get(index..index + 4) else { continue };
            let weight = wx * wy;
            let alpha = f64::from(p[3]) / 255.0;
            total += weight;
            density += weight * alpha;
            for (c, v) in colour.iter_mut().zip(p) {
                *c += weight * alpha * f64::from(*v);
            }
        }
    }
    if total <= 0.0 || density / total < DENSITY_MIN {
        return [0; 4];
    }
    let byte = |v: f64| v.round().clamp(0.0, 255.0) as u8;
    [byte(colour[0] / density), byte(colour[1] / density), byte(colour[2] / density), byte(density / total * 255.0)]
}
