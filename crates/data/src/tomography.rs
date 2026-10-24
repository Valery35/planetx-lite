// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сейсмическая томография мантии на гранях разреза Земли. Перенос
//! `core/tomography.py` модуля.
//!
//! Модель SEISGLOB2 (Durand и соавторы 2017, GJI 211, 1628,
//! doi:10.1093/gji/ggx405) - отклонение скорости поперечных волн dVs от
//! PREM в процентах, сетка 1°, 57 глубин от 50 до 2850 км через 50 км.
//! Медленная мантия (красная) - горячее вещество, быстрая (синяя) -
//! холодные погрузившиеся плиты. Модель лежит в каталоге EarthScope EMC,
//! лицензии у неё нет, только просьба ссылаться, поэтому программа её
//! не распространяет: файл netCDF ([`URL`]) скачивается при включении.
//! Разбор netCDF - дело вызывающего, сюда приходят массивы ([`make`]).
//! Итог хранится файлом профиля ([`save`], [`load`]) - архив .npz модуля,
//! dVs в float16.

use crate::npz;

/// Адрес файла netCDF модели.
pub const URL: &str = "https://data.earthscope.org/archive/seismology/products/emc/netcdf/SEISGLOB2_percent.r0.0-n4c.nc";
/// Подпись источника: текст и адрес.
pub const ATTRIBUTION: (&str, &str) =
    ("Tomography: SEISGLOB2, Durand et al. 2017, EarthScope EMC", "https://doi.org/10.17611/DP/EMCSEISGLOB2");
/// Граница ядра и мантии, км.
pub const CMB: f64 = 2891.0;
/// Раскраска dVs, %: медленнее PREM - красная, быстрее - синяя. Пределы -
/// выбор помощника модуля, утверждает автор.
pub const STOPS: [(f64, [u8; 3]); 5] = [
    (-2.0, [180, 25, 25]),
    (-0.8, [240, 130, 80]),
    (0.0, [245, 242, 235]),
    (0.8, [120, 165, 230]),
    (2.0, [25, 55, 165]),
];
/// Значения по модулю больше - пропуск данных файла.
const FILL: f32 = 1.0e4;

/// Модель: depths (K) км по возрастанию, lats (R) по возрастанию, lons
/// (C) от 0 с равным шагом, dvs K × R × C по строкам, %, NaN - нет данных.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tomo {
    pub depths: Vec<f64>,
    pub lats: Vec<f64>,
    pub lons: Vec<f64>,
    pub dvs: Vec<f32>,
}

impl Tomo {
    fn at(&self, k: usize, r: usize, c: usize) -> f64 {
        let (rows, cols) = (self.lats.len(), self.lons.len());
        self.dvs.get((k * rows + r) * cols + c).map_or(f64::NAN, |v| f64::from(*v))
    }
}

/// Tomo из массивов файла: dvs - K × R × C по строкам, широты в любом
/// направлении, долготы в любом виде, приводятся к 0-360 и сортируются.
/// Значения по модулю больше 1e4 - пропуск, NaN. По каждой оси нужно не
/// меньше двух узлов.
pub fn make(depths: &[f64], lats: &[f64], lons: &[f64], dvs: &[f32]) -> Result<Tomo, String> {
    let (k, r, c) = (depths.len(), lats.len(), lons.len());
    let count = k.checked_mul(r).and_then(|n| n.checked_mul(c));
    if count != Some(dvs.len()) {
        return Err(format!("tomography grid {} for {k} {r} {c}", dvs.len()));
    }
    if k < 2 || r < 2 || c < 2 {
        return Err(format!("tomography grid {k} {r} {c} is too small"));
    }
    let mut values: Vec<f32> = dvs.iter().map(|v| if v.abs() > FILL { f32::NAN } else { *v }).collect();
    let mut lats = lats.to_vec();
    if lats[0] > lats[r - 1] {
        lats.reverse();
        for layer in values.chunks_exact_mut(r * c) {
            for i in 0..r / 2 {
                for j in 0..c {
                    layer.swap(i * c + j, (r - 1 - i) * c + j);
                }
            }
        }
    }
    let wrapped: Vec<f64> = lons.iter().map(|v| v.rem_euclid(360.0)).collect();
    let mut order: Vec<usize> = (0..c).collect();
    order.sort_by(|a, b| wrapped[*a].total_cmp(&wrapped[*b]));
    let lons = order.iter().map(|i| wrapped[*i]).collect();
    let dvs = values.chunks_exact(c).flat_map(|row| order.iter().map(|i| row[*i])).collect();
    Ok(Tomo { depths: depths.to_vec(), lats, lons, dvs })
}

/// Предел, NaN проходит, как `np.clip`.
fn clip(x: f64, low: f64, high: f64) -> f64 {
    if x < low {
        low
    } else if x > high {
        high
    } else {
        x
    }
}

/// Кусочно-линейная функция по узлам xp, fp, за краями - крайнее
/// значение, как `np.interp`.
pub(crate) fn interp(x: f64, xp: &[f64], fp: &[f64]) -> f64 {
    let n = xp.len().min(fp.len());
    if n == 0 || x.is_nan() {
        return f64::NAN;
    }
    if x <= xp[0] {
        return fp[0];
    }
    if x >= xp[n - 1] {
        return fp[n - 1];
    }
    let j = xp[..n].partition_point(|v| *v <= x).clamp(1, n - 1) - 1;
    let span = xp[j + 1] - xp[j];
    if span == 0.0 {
        return fp[j];
    }
    fp[j] + (x - xp[j]) / span * (fp[j + 1] - fp[j])
}

/// dVs, %, в точках на глубине depth км: билинейно по широте и долготе,
/// линейно по глубине. За краями сетки - край.
pub fn sample(tomo: &Tomo, lats: &[f64], lons: &[f64], depth: f64) -> Vec<f64> {
    let (count, rows, cols) = (tomo.depths.len(), tomo.lats.len(), tomo.lons.len());
    if count < 2 || rows < 2 || cols < 2 {
        return vec![f64::NAN; lats.len().min(lons.len())];
    }
    let d = clip(depth, tomo.depths[0], tomo.depths[count - 1]);
    let k = (tomo.depths.partition_point(|v| *v < d) as i64 - 1).clamp(0, count as i64 - 2) as usize;
    let span = tomo.depths[k + 1] - tomo.depths[k];
    let w = if span <= 0.0 { 0.0 } else { (d - tomo.depths[k]) / span };
    let step = tomo.lons[1] - tomo.lons[0];
    let index: Vec<f64> = (0..rows).map(|i| i as f64).collect();
    lats.iter()
        .zip(lons)
        .map(|(lat, lon)| {
            let fc = (lon.rem_euclid(360.0) - tomo.lons[0]) / step;
            let floor = fc.floor();
            let c0 = if floor.is_finite() { (floor as i64).rem_euclid(cols as i64) as usize } else { 0 };
            let c1 = (c0 + 1) % cols;
            let dc = fc - floor;
            let fr = interp(*lat, &tomo.lats, &index);
            let r0 = if fr.is_nan() { 0 } else { (fr.floor() as i64).clamp(0, rows as i64 - 2) as usize };
            let dr = clip(fr - r0 as f64, 0.0, 1.0);
            let layer = |k: usize| {
                tomo.at(k, r0, c0) * (1.0 - dc) * (1.0 - dr)
                    + tomo.at(k, r0, c1) * dc * (1.0 - dr)
                    + tomo.at(k, r0 + 1, c0) * (1.0 - dc) * dr
                    + tomo.at(k, r0 + 1, c1) * dc * dr
            };
            layer(k) * (1.0 - w) + layer(k + 1) * w
        })
        .collect()
}

/// Цвет RGB для dVs, %, NaN - цвет нуля.
pub fn color(dvs: f64) -> [u8; 3] {
    let x = if dvs.is_nan() { 0.0 } else { dvs };
    let at = STOPS.map(|(s, _)| s);
    std::array::from_fn(|k| interp(x, &at, &STOPS.map(|(_, c)| f64::from(c[k]))).round_ties_even() as u8)
}

/// Цвета RGB для значений dVs, %.
pub fn colors(dvs: &[f64]) -> Vec<[u8; 3]> {
    dvs.iter().map(|v| color(*v)).collect()
}

/// Глубины рядов сетки грани: глубины модели внутри мантии и низ мантии.
pub fn rows(tomo: &Tomo) -> Vec<f64> {
    let mut out: Vec<f64> = tomo.depths.iter().copied().filter(|d| *d > 0.0 && *d < CMB).collect();
    out.push(CMB);
    out
}

/// Байты файла профиля: архив .npz, как `save` модуля - оси в f64,
/// dVs в float16.
pub fn save(tomo: &Tomo) -> Result<Vec<u8>, String> {
    let shape = [tomo.depths.len(), tomo.lats.len(), tomo.lons.len()];
    npz::write_npz(&[
        ("depths", npz::npy_f64(&shape[..1], &tomo.depths)),
        ("lats", npz::npy_f64(&shape[1..2], &tomo.lats)),
        ("lons", npz::npy_f64(&shape[2..], &tomo.lons)),
        ("dvs", npz::npy_f16(&shape, &tomo.dvs)),
    ])
}

/// Модель из байтов файла профиля [`save`] или модуля. Проверки - как
/// у [`make`].
pub fn load(data: &[u8]) -> Result<Tomo, String> {
    let arrays = npz::read_npz(data, &["depths", "lats", "lons", "dvs"])?;
    let [depths, lats, lons, dvs] = <[npz::Array; 4]>::try_from(arrays).map_err(|_| "tomography file".to_string())?;
    let flat = depths.shape.len() == 1 && lats.shape.len() == 1 && lons.shape.len() == 1;
    let grid = [depths.values.len(), lats.values.len(), lons.values.len()];
    if !flat || dvs.shape != grid {
        return Err(format!("tomography grid {:?} for {grid:?}", dvs.shape));
    }
    let values: Vec<f32> = dvs.values.iter().map(|v| *v as f32).collect();
    make(&depths.values, &lats.values, &lons.values, &values)
}
