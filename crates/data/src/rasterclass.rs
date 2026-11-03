// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Классификация участка по своему растру: каналы растра - признаки
//! клетки. Перенос `core/rasterclass.py` модуля и чтения
//! `ui.aefclass.read_raster`, там его делает `gdal.Warp`.
//!
//! Контрольные точки те же, что у классификации по эмбеддингам AlphaEarth
//! ([`crate::aef`]), мера другая. Векторы AlphaEarth - единичные, у них
//! работает косинус. Каналы своего растра бывают в разных единицах:
//! косинус на одном канале равен 1 у всех клеток, а при высоте в метрах
//! и отражении в долях единицы он определяется одной высотой. Поэтому
//! каждый канал приводится к среднему 0 и разбросу 1 по клеткам участка,
//! близость - евклидово расстояние, голос точки - обратное расстояние,
//! сумма голосов класса делится на количество его точек. Канал без
//! разброса на участке признаком не считается.
//!
//! Файл считается враждебным: количество точек растра ограничено, сетка
//! участка проверяется до выделения памяти, разбор тегов идёт с проверкой
//! границ.

use terrio::RasterDataset;
use terrio::crs::{SpatialRef, Transformer};

use crate::aef::{self, Mosaic, ZoneIndex};
use crate::grid;

/// Контрольных точек голосуют за клетку, как у AlphaEarth.
pub const KNN: usize = aef::KNN;
/// Клеток участка при пикселе «Авто».
pub const AUTO_CELLS: f64 = 800.0 * 800.0;
/// Добавка к расстоянию: точка на самой клетке даёт конечный голос.
const NEAR: f32 = 1e-3;
/// Клеток сетки участка.
const MAX_CELLS: u128 = 4000 * 4000;
/// Байт сетки float32 в памяти.
const MAX_BYTES: u128 = 1 << 30;
/// Пиксель «Авто», метры, если растр его не называет.
const PIXEL_DEFAULT: f64 = 10.0;
/// Шаг покрытия для подписи: ниже этой доли итог называет покрытие.
pub const COVERED: f64 = 0.995;

/// Отказ расчёта, `problem_text` модуля.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// Файл не читается.
    Unreadable,
    /// У растра нет системы координат.
    NoCrs,
    /// Сетка участка не помещается в память.
    Big,
    /// Участок вне охвата растра.
    Outside,
    /// Меньше двух классов с точками на клетках с данными.
    Points,
    /// Каналы на участке не меняются.
    Flat,
    /// Весь участок вышел одним объектом.
    Whole,
    /// Сетка участка слишком велика для деления на объекты.
    Pieces,
    /// В контурах образцов нет объектов участка.
    Samples,
    /// Тайлы подложки не получены.
    Tiles,
}

/// Пиксель «Авто», метры: пиксель растра native, если на участке со
/// стороной side метров клеток не больше [`AUTO_CELLS`], иначе крупнее.
pub fn auto_pixel(native: f64, side: f64) -> f64 {
    native.max(side / AUTO_CELLS.sqrt())
}

/// Сетка участка больше [`MAX_CELLS`] клеток или float32 не помещается
/// в [`MAX_BYTES`] байт. Защита вместо предела размера участка: большой
/// участок с мелким пикселем не вешает программу.
pub fn too_big(rows: usize, cols: usize, bands: usize) -> bool {
    let cells = (rows as u128).saturating_mul(cols as u128);
    cells > MAX_CELLS || cells.saturating_mul(bands as u128).saturating_mul(4) > MAX_BYTES
}

/// Есть все каналы клетки.
pub fn usable(cell: &[f32]) -> bool {
    cell.iter().all(|v| v.is_finite())
}

/// Доля клеток участка inside, где есть все каналы растра.
pub fn covered(grid: &[f32], depth: usize, inside: &[bool]) -> f64 {
    let total = inside.iter().filter(|v| **v).count();
    let good = grid.chunks(depth.max(1)).zip(inside).filter(|(cell, keep)| **keep && usable(cell)).count();
    if total > 0 { good as f64 / total as f64 } else { 0.0 }
}

/// Среднее и разброс каналов по клеткам участка со всеми каналами. Канал
/// без разброса - разброс 0, признаком он не будет.
pub fn scales(grid: &[f32], depth: usize, inside: Option<&[bool]>) -> (Vec<f64>, Vec<f64>) {
    let mut mean = vec![0.0; depth];
    let mut square = vec![0.0; depth];
    let mut count = 0usize;
    for (i, cell) in grid.chunks(depth.max(1)).enumerate() {
        if !usable(cell) || inside.is_some_and(|m| !m.get(i).copied().unwrap_or(false)) {
            continue;
        }
        count += 1;
        for ((m, s), v) in mean.iter_mut().zip(&mut square).zip(cell) {
            // Формула Уэлфорда: квадраты больших отметок не теряют разброс.
            let v = f64::from(*v);
            let shift = v - *m;
            *m += shift / count as f64;
            *s += shift * (v - *m);
        }
    }
    if count == 0 {
        return (vec![0.0; depth], vec![0.0; depth]);
    }
    let spread = square
        .iter()
        .zip(&mean)
        .map(|(s, m)| {
            let value = (s / count as f64).sqrt();
            // Разброс из шума округления float32 - не разброс.
            if value <= 1e-6 * m.abs().max(1e-30) { 0.0 } else { value }
        })
        .collect();
    (mean, spread)
}

/// Расчёт классов по частям: окно считает его за кадры.
pub struct Run {
    /// Приведённые значения точек подряд, каналы - только признаки.
    train: Vec<f32>,
    labels: Vec<usize>,
    /// Номера классов и количество точек в каждом.
    classes: Vec<i32>,
    sizes: Vec<f32>,
    used: Vec<usize>,
    mean: Vec<f64>,
    spread: Vec<f64>,
    depth: usize,
    k: usize,
    /// Клетки расчёта и сколько уже посчитано.
    cells: Vec<usize>,
    next: usize,
    out: Vec<i16>,
    distance: Vec<f32>,
}

impl Run {
    /// grid - значения клеток по depth подряд, NaN без данных, samples -
    /// значения точек, labels - номера классов точек, inside - клетки
    /// участка. Точка без данных не голосует.
    pub fn new(grid: &[f32], depth: usize, samples: &[Vec<f32>], labels: &[i32], inside: Option<&[bool]>, k: usize) -> Self {
        let total = grid.len() / depth.max(1);
        let (mean, spread) = scales(grid, depth, inside);
        let used: Vec<usize> = (0..depth).filter(|b| spread.get(*b).is_some_and(|s| *s > 0.0)).collect();
        let good: Vec<(&Vec<f32>, i32)> = samples.iter().zip(labels).filter(|(s, _)| s.len() == depth && usable(s)).map(|(s, l)| (s, *l)).collect();
        let mut classes: Vec<i32> = good.iter().map(|g| g.1).collect();
        classes.sort_unstable();
        classes.dedup();
        let index = |l: i32| classes.binary_search(&l).unwrap_or(0);
        let mut sizes = vec![0.0; classes.len()];
        for (_, l) in &good {
            if let Some(size) = sizes.get_mut(index(*l)) {
                *size += 1.0;
            }
        }
        let train: Vec<f32> = good.iter().flat_map(|(s, _)| normal(s, &used, &mean, &spread)).collect();
        let picked: Vec<usize> = good.iter().map(|g| index(g.1)).collect();
        // Без точек или без признаков клеток расчёта нет.
        let cells = if picked.is_empty() || used.is_empty() {
            Vec::new()
        } else {
            (0..total).filter(|i| inside.is_none_or(|m| m.get(*i).copied().unwrap_or(false)) && grid.get(i * depth..(i + 1) * depth).is_some_and(usable)).collect()
        };
        Self { k: k.clamp(1, picked.len().max(1)), train, labels: picked, classes, sizes, used, mean, spread, depth, cells, next: 0, out: vec![-1; total], distance: vec![f32::NAN; total] }
    }

    /// Посчитано клеток.
    pub fn done(&self) -> usize {
        self.next
    }

    /// Всего клеток расчёта.
    pub fn total(&self) -> usize {
        self.cells.len()
    }

    /// Следующие count клеток. true - посчитаны все.
    pub fn step(&mut self, grid: &[f32], count: usize) -> bool {
        let features = self.used.len().max(1);
        let mut near: Vec<(f32, usize)> = Vec::with_capacity(self.labels.len());
        let mut votes = vec![0.0f32; self.classes.len()];
        let end = self.next.saturating_add(count).min(self.cells.len());
        for at in self.next..end {
            let Some(&cell) = self.cells.get(at) else {
                continue;
            };
            let Some(raw) = grid.get(cell * self.depth..(cell + 1) * self.depth) else {
                continue;
            };
            let value = normal(raw, &self.used, &self.mean, &self.spread);
            near.clear();
            for (point, label) in self.train.chunks(features).zip(&self.labels) {
                let sum: f32 = point.iter().zip(&value).map(|(a, b)| (a - b) * (a - b)).sum();
                near.push((sum.sqrt(), *label));
            }
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            votes.fill(0.0);
            for (distance, label) in near.iter().take(self.k) {
                if let Some(vote) = votes.get_mut(*label) {
                    *vote += 1.0 / (distance + NEAR);
                }
            }
            // Голос на точку класса: класс с лишними точками не перетягивает
            // клетки, когда голосуют почти все точки.
            let mut best = (0, f32::NEG_INFINITY);
            for (i, (vote, size)) in votes.iter().zip(&self.sizes).enumerate() {
                let share = vote / size;
                if share > best.1 {
                    best = (i, share);
                }
            }
            if let (Some(slot), Some(class)) = (self.out.get_mut(cell), self.classes.get(best.0)) {
                *slot = i16::try_from(*class).unwrap_or(-1);
            }
            if let (Some(slot), Some(first)) = (self.distance.get_mut(cell), near.first()) {
                *slot = first.0;
            }
        }
        self.next = end;
        self.next >= self.cells.len()
    }

    /// Классы клеток, -1 без данных и вне участка, и расстояние до лучшей
    /// точки.
    pub fn finish(self) -> (Vec<i16>, Vec<f32>) {
        (self.out, self.distance)
    }
}

/// Признаки клетки: каналы used к среднему 0 и разбросу 1. Сначала в f64:
/// большое среднее в float32 съело бы значение.
fn normal(cell: &[f32], used: &[usize], mean: &[f64], spread: &[f64]) -> Vec<f32> {
    used.iter()
        .map(|&b| {
            let (v, m, s) = (cell.get(b).copied().unwrap_or(f32::NAN), mean.get(b).copied().unwrap_or(0.0), spread.get(b).copied().unwrap_or(1.0));
            ((f64::from(v) - m) / s) as f32
        })
        .collect()
}

/// Классы клеток целиком, см. [`Run::new`] и [`Run::finish`].
pub fn classify(grid: &[f32], depth: usize, samples: &[Vec<f32>], labels: &[i32], inside: Option<&[bool]>, k: usize) -> (Vec<i16>, Vec<f32>) {
    let mut run = Run::new(grid, depth, samples, labels, inside, k);
    run.step(grid, usize::MAX);
    run.finish()
}

/// Значения контрольных точек на клетках с данными: (значения, классы,
/// сколько точек вне сетки или без данных). points - номер класса и
/// (широта, долгота).
pub fn samples(grid: &Mosaic<f32>, index: &ZoneIndex, points: &[(i32, (f64, f64))]) -> (Vec<Vec<f32>>, Vec<i32>, usize) {
    let (mut values, mut labels, mut outside) = (Vec::new(), Vec::new(), 0);
    for &(label, (lat, lon)) in points {
        let (x, y) = index.project(lat, lon);
        match grid.cell_of(x, y).and_then(|(r, c)| grid.at(r, c)).filter(|v| usable(v)) {
            Some(v) => {
                values.push(v.to_vec());
                labels.push(label);
            }
            None => outside += 1,
        }
    }
    (values, labels, outside)
}

/// Клеток образца с контура, `RING_CELLS` модуля 1.38: поровну по клеткам
/// внутри, голос класса делится на количество его образцов ([`Run`]).
pub const RING_CELLS: usize = 40;

/// Номера take из n по порядку поровну, первый и последний - края, как
/// `np.linspace(0, n - 1, take).astype(np.int64)` модуля.
pub fn spread_pick(n: usize, take: usize) -> Vec<usize> {
    let take = take.min(n);
    match take {
        0 => Vec::new(),
        1 => vec![0],
        _ => (0..take).map(|i| (i as f64 * (n - 1) as f64 / (take - 1) as f64) as usize).collect(),
    }
}

/// Значения образцов контура: клетки inside со всеми каналами, не больше
/// [`RING_CELLS`] поровну по ним. Пусто - в контуре нет клеток с данными.
pub fn ring_samples(grid: &Mosaic<f32>, inside: &[bool]) -> Vec<Vec<f32>> {
    let depth = grid.depth.max(1);
    let cells: Vec<&[f32]> = grid.values.chunks(depth).zip(inside).filter(|(cell, keep)| **keep && usable(cell)).map(|(cell, _)| cell).collect();
    spread_pick(cells.len(), RING_CELLS).into_iter().filter_map(|i| cells.get(i).map(|c| c.to_vec())).collect()
}

/// Свой растр: все каналы, система координат, канал прозрачности.
pub struct Source {
    data: RasterDataset,
    crs: SpatialRef,
    transform: [f64; 6],
    nodata: Option<f64>,
    /// Каналы-признаки.
    bands: Vec<usize>,
    /// Канал прозрачности: маска, а не признак.
    alpha: Option<usize>,
}

/// Пересчёт растра на сетку участка.
pub struct Warp {
    to_raster: Transformer,
    pub grid: Mosaic<f32>,
    next: usize,
}

impl Warp {
    /// Готовых строк.
    pub fn done(&self) -> usize {
        self.next
    }
}

impl Source {
    /// Растр из байтов файла. name - имя файла, по нему terrio узнаёт
    /// формат.
    pub fn read(bytes: &[u8], name: &str) -> Result<Self, Problem> {
        let data = terrio::memory::open_raster_bytes(bytes.to_vec(), name).map_err(|_| Problem::Unreadable)?;
        let r = &data.raster;
        if r.cols == 0 || r.rows == 0 || r.cols.saturating_mul(r.rows) > grid::MAX_PIXELS {
            return Err(Problem::Unreadable);
        }
        let alpha = alpha_band(bytes, r.bands);
        let bands: Vec<usize> = (0..r.bands).filter(|b| Some(*b) != alpha).collect();
        if bands.is_empty() {
            return Err(Problem::Unreadable);
        }
        let crs = data.crs();
        if crs.is_unknown() {
            return Err(Problem::NoCrs);
        }
        let (transform, nodata) = (data.geo_transform(), data.nodata());
        Ok(Self { data, crs, transform, nodata, bands, alpha })
    }

    /// Количество каналов-признаков.
    pub fn bands(&self) -> usize {
        self.bands.len()
    }

    /// Подписи каналов-признаков по порядку: описание канала файла
    /// (у GeoTIFF - тег GDAL_METADATA), пусто - подписи нет.
    pub fn labels(&self) -> Vec<String> {
        self.bands.iter().map(|b| self.data.band_metadata.get(*b).and_then(|m| m.description.clone()).unwrap_or_default()).collect()
    }

    /// Значения каналов-признаков в точке (широта, долгота): пиксель под
    /// точкой, NaN - «нет данных». None - точка вне растра или прозрачна,
    /// как `pixel_values` модуля.
    pub fn values_at(&self, lat: f64, lon: f64) -> Option<Vec<f64>> {
        let from_wgs84 = Transformer::new(&SpatialRef::from_epsg(4326).ok()?, &self.crs).ok()?;
        let (x, y) = from_wgs84.transform(lon, lat).ok()?;
        let (col, row) = grid::pixel_of(&self.transform, x, y)?;
        let r = &self.data.raster;
        if !(col >= 0.0 && row >= 0.0 && col < r.cols as f64 && row < r.rows as f64) {
            return None;
        }
        let (c, w) = (col.floor() as isize, row.floor() as isize);
        let value = |band: usize| {
            let v = r.get(band as isize, w, c);
            if !v.is_finite() || self.nodata.is_some_and(|n| v == n) { f64::NAN } else { v }
        };
        if self.alpha.map(value).is_some_and(|v| v.is_nan() || v <= 0.0) {
            return None;
        }
        Some(self.bands.iter().map(|b| value(*b)).collect())
    }

    fn transformer(&self, index: &ZoneIndex, to_raster: bool) -> Result<Transformer, Problem> {
        let utm = SpatialRef::from_epsg(utm_epsg(index)).map_err(|_| Problem::NoCrs)?;
        let made = if to_raster { Transformer::new(&utm, &self.crs) } else { Transformer::new(&self.crs, &utm) };
        made.map_err(|_| Problem::NoCrs)
    }

    /// Широта и долгота углов растра, как [`crate::grid::Grid::corners`].
    pub fn corners(&self) -> Option<[(f64, f64); 4]> {
        let to_wgs84 = Transformer::new(&self.crs, &SpatialRef::from_epsg(4326).ok()?).ok()?;
        let (w, h) = (self.data.raster.cols as f64, self.data.raster.rows as f64);
        let t = &self.transform;
        let mut out = [(0.0, 0.0); 4];
        for (slot, (c, r)) in out.iter_mut().zip([(0.0, h), (w, h), (w, 0.0), (0.0, 0.0)]) {
            let (lon, lat) = to_wgs84.transform(t[0] + c * t[1] + r * t[2], t[3] + c * t[4] + r * t[5]).ok()?;
            *slot = (lat, lon);
        }
        Some(out)
    }

    /// Пиксель растра в метрах UTM около точки (широта, долгота): шаги по
    /// столбцу и строке, пересчитанные в систему участка.
    pub fn pixel(&self, index: &ZoneIndex, lat: f64, lon: f64) -> f64 {
        let found = || -> Option<f64> {
            let (x, y) = index.project(lat, lon);
            let (sx, sy) = self.transformer(index, true).ok()?.transform(x, y).ok()?;
            let back = self.transformer(index, false).ok()?;
            let t = &self.transform;
            let at = |dx: f64, dy: f64| back.transform(sx + dx, sy + dy).ok();
            let (a, b, c) = (at(0.0, 0.0)?, at(t[1], t[4])?, at(t[2], t[5])?);
            Some(((b.0 - a.0).hypot(b.1 - a.1) + (c.0 - a.0).hypot(c.1 - a.1)) / 2.0)
        };
        found().filter(|p| p.is_finite() && *p > 0.0).unwrap_or(PIXEL_DEFAULT)
    }

    /// Пересчёт на сетку участка bbox в метрах UTM с шагом step.
    pub fn warp(&self, index: &ZoneIndex, bbox: (f64, f64, f64, f64), step: f64) -> Result<Warp, Problem> {
        let (x0, y0, cols, rows) = aef::grid_frame(index, bbox, step);
        let depth = self.bands();
        if too_big(rows, cols, depth) {
            return Err(Problem::Big);
        }
        let to_raster = self.transformer(index, true)?;
        Ok(Warp { to_raster, grid: Mosaic { x0, y0, step, rows, cols, depth, values: vec![f32::NAN; rows * cols * depth] }, next: 0 })
    }

    /// Значение канала band в точке (столбец, строка) с долями, «нет
    /// данных» канала - NaN. В смесь пустые соседи не входят.
    fn sample(&self, band: usize, col: f64, row: f64) -> f32 {
        let r = &self.data.raster;
        let at = |x: usize, y: usize| {
            let v = r.get(band as isize, y as isize, x as isize);
            if !v.is_finite() || self.nodata.is_some_and(|n| v == n) { f32::NAN } else { v as f32 }
        };
        grid::blend(r.cols, r.rows, col, row, at)
    }

    /// Следующие count строк сетки билинейно. true - готовы все.
    /// Прозрачная клетка остаётся без данных.
    pub fn warp_rows(&self, warp: &mut Warp, count: usize) -> bool {
        let (step, x0, y0, cols, depth) = (warp.grid.step, warp.grid.x0, warp.grid.y0, warp.grid.cols, warp.grid.depth);
        let end = warp.next.saturating_add(count).min(warp.grid.rows);
        for r in warp.next..end {
            let y = y0 - (r as f64 + 0.5) * step;
            for c in 0..cols {
                let x = x0 + (c as f64 + 0.5) * step;
                let Some((col, row)) = warp.to_raster.transform(x, y).ok().and_then(|(sx, sy)| grid::pixel_of(&self.transform, sx, sy)) else {
                    continue;
                };
                let clear = self.alpha.map(|a| self.sample(a, col, row)).is_some_and(|v| v.is_nan() || v <= 0.0);
                if clear {
                    continue;
                }
                let start = (r * cols + c) * depth;
                if let Some(cell) = warp.grid.values.get_mut(start..start + depth) {
                    for (slot, band) in cell.iter_mut().zip(&self.bands) {
                        *slot = self.sample(*band, col, row);
                    }
                }
            }
        }
        warp.next = end;
        warp.next >= warp.grid.rows
    }
}

/// Код EPSG зоны UTM указателя.
fn utm_epsg(index: &ZoneIndex) -> u32 {
    (if index.south { 32700 } else { 32600 }) + index.number
}

/// Значения по 2 байта тега tag первого IFD TIFF, не больше most штук.
/// None - не TIFF, пусто - тега нет или значений больше most.
fn tag_shorts(bytes: &[u8], tag: u64, most: usize) -> Option<Vec<u64>> {
    let little = match bytes.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let number = |at: usize, size: usize| -> Option<u64> {
        let raw = bytes.get(at..at.checked_add(size)?)?;
        let fold = |n: u64, b: &u8| (n << 8) | u64::from(*b);
        Some(if little { raw.iter().rev().fold(0, fold) } else { raw.iter().fold(0, fold) })
    };
    let big = match number(2, 2)? {
        42 => false,
        43 => true,
        _ => return None,
    };
    let (word, entry, value_at) = if big { (8, 20, 12) } else { (4, 12, 8) };
    let ifd = usize::try_from(number(if big { 8 } else { 4 }, word)?).ok()?;
    let count = number(ifd, if big { 8 } else { 2 })?;
    let first = ifd.checked_add(if big { 8 } else { 2 })?;
    for n in 0..usize::try_from(count).ok()?.min(4096) {
        let at = first.checked_add(n.checked_mul(entry)?)?;
        if number(at, 2)? != tag {
            continue;
        }
        let found = usize::try_from(number(at + 4, word)?).ok()?;
        if found == 0 || found > most {
            return Some(Vec::new());
        }
        // Значения по 2 байта: в запись или по смещению.
        let values = if found * 2 <= word { at + value_at } else { usize::try_from(number(at + value_at, word)?).ok()? };
        return (0..found).map(|i| number(values.checked_add(i * 2)?, 2)).collect();
    }
    Some(Vec::new())
}

/// Текст тега ASCII первого IFD TIFF. None - не TIFF или тега нет.
fn tag_text(bytes: &[u8], tag: u64) -> Option<String> {
    let little = match bytes.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let number = |at: usize, size: usize| -> Option<u64> {
        let raw = bytes.get(at..at.checked_add(size)?)?;
        let fold = |n: u64, b: &u8| (n << 8) | u64::from(*b);
        Some(if little { raw.iter().rev().fold(0, fold) } else { raw.iter().fold(0, fold) })
    };
    let big = match number(2, 2)? {
        42 => false,
        43 => true,
        _ => return None,
    };
    let (word, entry, value_at) = if big { (8, 20, 12) } else { (4, 12, 8) };
    let ifd = usize::try_from(number(if big { 8 } else { 4 }, word)?).ok()?;
    let count = number(ifd, if big { 8 } else { 2 })?;
    let first = ifd.checked_add(if big { 8 } else { 2 })?;
    for n in 0..usize::try_from(count).ok()?.min(4096) {
        let at = first.checked_add(n.checked_mul(entry)?)?;
        if number(at, 2)? != tag {
            continue;
        }
        let size = usize::try_from(number(at + 4, word)?).ok()?.min(1 << 20);
        let start = if size <= word { at + value_at } else { usize::try_from(number(at + value_at, word)?).ok()? };
        let raw = bytes.get(start..start.checked_add(size)?)?;
        return Some(String::from_utf8_lossy(raw).trim_end_matches('\0').to_string());
    }
    None
}

/// Подписи каналов TIFF по тегу GDAL_METADATA без чтения точек: описания
/// `<Item name="DESCRIPTION" sample="n">` по номеру канала, пусто - подписи
/// нет. Для списков окон, которые выбирают снимки по каналам.
pub fn quick_labels(bytes: &[u8]) -> Vec<String> {
    let bands = feature_bands(bytes).unwrap_or(0).min(256);
    let mut out = vec![String::new(); bands];
    let Some(xml) = tag_text(bytes, 42112) else {
        return out;
    };
    for item in xml.split("<Item").skip(1) {
        let Some((head, rest)) = item.split_once('>') else {
            continue;
        };
        if !head.contains("name=\"DESCRIPTION\"") {
            continue;
        }
        let sample = head.split("sample=\"").nth(1).and_then(|s| s.split('"').next()).and_then(|s| s.parse::<usize>().ok());
        let text = rest.split("</Item>").next().unwrap_or("").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&");
        if let Some(slot) = sample.and_then(|s| out.get_mut(s)) {
            *slot = text;
        }
    }
    out
}

/// Канал прозрачности TIFF по тегу ExtraSamples (338), как `GCI_AlphaBand`
/// у GDAL: значения 1 и 2 - прозрачность. Каналы ExtraSamples идут
/// последними. None - не TIFF или прозрачности нет.
fn alpha_band(bytes: &[u8], bands: usize) -> Option<usize> {
    let extra = tag_shorts(bytes, 338, bands)?;
    let at = extra.iter().position(|v| matches!(v, 1 | 2))?;
    Some(bands - extra.len() + at)
}

/// Количество каналов-признаков TIFF по заголовку, без чтения точек:
/// SamplesPerPixel (277) без канала прозрачности. None - не TIFF.
pub fn feature_bands(bytes: &[u8]) -> Option<usize> {
    let bands = tag_shorts(bytes, 277, 1)?.first().map_or(1, |v| *v as usize);
    Some(bands - usize::from(alpha_band(bytes, bands).is_some()))
}
