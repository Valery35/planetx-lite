// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Заросшие поля по ряду весенних и осенних снимков, перенос
//! `core/fallow.py` модуля 1.36.
//!
//! Пашня весной до всходов и осенью после уборки и зяби - голая почва,
//! у залежи по весне старая трава и кустарник. Поэтому у каждого поля
//! летнего снимка (область трассировки) по снимкам весны и осени за годы
//! видно, в каком году на нём последний раз была голая почва. Голая
//! клетка - NDVI ниже [`BARE_NDVI`]. Прогон модуля 3 ноября 2026 года
//! у Шадейки (57.43, 56.80) по снимкам Sentinel-2 мая и сентября - октября
//! 2023-2026 годов: пашня весной 0.10-0.25, сухая ветошь залежи 0.35-0.5,
//! всходы и травы выше 0.6, порог 0.3 отделил пашню от ветоши на всех
//! восьми снимках. Деревья - по ряду сезонов ([`season_woody`]), летний
//! NDMI - запасной путь. Пороги - выбор помощника модуля.
//!
//! Многолетние травы на сенокосе за годы ряда тоже ни разу не голые -
//! они выходят залежью, это ограничение способа.
//!
//! Каналы снимка - значения клеток подряд по строкам, NaN - нет данных.

use crate::objecttasks::{Kind, SNOW_SWIR, SNOW_VIS, WATER_NIR};

/// Лет ряда, выбор автора модуля от 3 ноября 2026 года.
pub const YEARS: i32 = 4;
/// Месяцы весны и осени по порядку предпочтения.
pub const SPRING: [u32; 2] = [5, 4];
pub const AUTUMN: [u32; 2] = [9, 10];
/// Голая почва межсезонного снимка.
pub const BARE_NDVI: f64 = 0.3;
/// Доля голых клеток поля, с которой поле обрабатывалось.
pub const BARE_SHARE: f64 = 0.25;
/// Доля клеток поля с данными, чтобы снимок считался.
pub const KNOWN_SHARE: f64 = 0.5;
/// Голое на такой доле снимков - не поле, застройка.
pub const ALWAYS_BARE: f64 = 0.8;
/// Дерево - медиана SWIR снимков сезонов ниже.
pub const WOODY_SWIR: f64 = 0.09;
/// Дерево на летнем снимке, если у сезонов нет SWIR.
pub const WOODY_NDMI: f64 = 0.3;
/// Доля деревьев залежи, с которой она зарастает.
pub const WOODY_SHARE: f64 = 0.25;
/// Доля деревьев, с которой область - лес.
pub const FOREST_SHARE: f64 = 0.7;
/// Лет без обработки, которые ещё «недавно».
pub const RECENT: i32 = 2;

/// Виды полей по порядку от пашни к лесу.
pub const KINDS: [Kind; 5] = [Kind::Tilled, Kind::FallowNew, Kind::FallowOld, Kind::Overgrown, Kind::Forest];
/// Летние виды, из которых бывают поля.
pub const FIELD_SOURCES: [Kind; 6] = [Kind::Conifer, Kind::Broadleaf, Kind::Vegetation, Kind::Soil, Kind::Clearing, Kind::Other];

/// Каналы снимка: значения по каналам и роли каналов.
pub type Scene<'a> = (&'a [Vec<f64>], &'a [Option<&'a str>]);

fn band<'a>(bands: &'a [Vec<f64>], roles: &[Option<&str>], role: &str) -> Option<&'a Vec<f64>> {
    roles.iter().position(|r| *r == Some(role)).and_then(|n| bands.get(n))
}

/// Голые клетки межсезонного снимка: 1 - голая почва, 0 - растения,
/// NaN - нет данных, вода или снег. Нужны красный и ближний ИК.
pub fn bare_cells(bands: &[Vec<f64>], roles: &[Option<&str>]) -> Option<Vec<f64>> {
    let (red, nir) = (band(bands, roles, "red")?, band(bands, roles, "nir")?);
    let swir = band(bands, roles, "swir1").or_else(|| band(bands, roles, "swir2"));
    Some(
        red.iter()
            .zip(nir)
            .enumerate()
            .map(|(k, (r, n))| {
                let ndvi = (n - r) / (n + r);
                let snow = *r > SNOW_VIS && swir.is_none_or(|s| s.get(k).is_some_and(|s| *s < SNOW_SWIR));
                if !ndvi.is_finite() || *n < WATER_NIR || snow {
                    f64::NAN
                } else if ndvi < BARE_NDVI {
                    1.0
                } else {
                    0.0
                }
            })
            .collect(),
    )
}

/// Медиана ряда без NaN, None - значений нет.
fn median(values: &mut Vec<f64>) -> Option<f64> {
    values.retain(|v| v.is_finite());
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let n = values.len();
    Some(if n % 2 == 1 { values[n / 2] } else { 0.5 * (values[n / 2 - 1] + values[n / 2]) })
}

/// Деревья по ряду снимков весны и осени: медиана SWIR 2,2 мкм (без него -
/// 1,6 мкм) по снимкам ниже [`WOODY_SWIR`]. Снег и нет данных - не в счёт.
/// None - ни у одного снимка нет SWIR.
///
/// Лес тёмный в SWIR круглый год от теней полога и воды в листве, трава
/// и сухая ветошь залежи весной и осенью светлые. Прогон модуля у Шадейки,
/// 8 снимков 2023-2026 годов: лес 0.03-0.08, луга и залежь 0.10-0.20, пашня
/// 0.15-0.30.
pub fn season_woody(scenes: &[Scene]) -> Option<Vec<f64>> {
    let mut rows: Vec<Vec<f64>> = Vec::new();
    for (bands, roles) in scenes {
        let Some(swir) = band(bands, roles, "swir2").or_else(|| band(bands, roles, "swir1")) else {
            continue;
        };
        let Some(bare) = bare_cells(bands, roles) else {
            continue;
        };
        rows.push(swir.iter().zip(&bare).map(|(s, b)| if b.is_finite() { *s } else { f64::NAN }).collect());
    }
    let cells = rows.first()?.len();
    Some(
        (0..cells)
            .map(|k| {
                let mut column: Vec<f64> = rows.iter().filter_map(|r| r.get(k).copied()).collect();
                match median(&mut column) {
                    Some(m) if m < WOODY_SWIR => 1.0,
                    Some(_) => 0.0,
                    None => f64::NAN,
                }
            })
            .collect(),
    )
}

/// Деревья летнего снимка по NDMI, None - нет ближнего ИК и SWIR.
/// Запасной путь, если у снимков сезонов нет SWIR.
pub fn woody_cells(bands: &[Vec<f64>], roles: &[Option<&str>]) -> Option<Vec<f64>> {
    let (nir, swir) = (band(bands, roles, "nir")?, band(bands, roles, "swir1")?);
    Some(nir.iter().zip(swir).map(|(n, s)| if (n - s) / (n + s) > WOODY_NDMI { 1.0 } else { 0.0 }).collect())
}

/// Сумма values и количество клеток с данными по областям.
fn per_region(parts: &[i32], values: &[f64], count: usize) -> (Vec<f64>, Vec<usize>) {
    let mut total = vec![0.0; count];
    let mut known = vec![0usize; count];
    for (p, v) in parts.iter().zip(values) {
        if let Ok(p) = usize::try_from(*p)
            && p < count
            && v.is_finite()
        {
            total[p] += v;
            known[p] += 1;
        }
    }
    (total, known)
}

/// Годы обработки по областям.
pub struct Years {
    /// Год и обрабатывалось ли поле в этом году, по областям.
    pub years: Vec<Vec<(i32, bool)>>,
    /// Голых снимков по областям.
    pub bare: Vec<usize>,
    /// Учтённых снимков по областям.
    pub seen: Vec<usize>,
}

/// По областям: годы обработки и количество голых и учтённых снимков.
/// scenes - (год, голые клетки [`bare_cells`]). Снимок учтён у области,
/// если данные есть на [`KNOWN_SHARE`] её клеток.
pub fn tilled_years(parts: &[i32], scenes: &[(i32, Vec<f64>)], count: usize) -> Years {
    let mut cells = vec![0usize; count];
    for p in parts {
        if let Ok(p) = usize::try_from(*p)
            && p < count
        {
            cells[p] += 1;
        }
    }
    let mut out = Years { years: vec![Vec::new(); count], bare: vec![0; count], seen: vec![0; count] };
    for (year, values) in scenes {
        let (total, known) = per_region(parts, values, count);
        for n in 0..count {
            let enough = known[n] as f64 >= KNOWN_SHARE * cells[n].max(1) as f64;
            if !enough {
                continue;
            }
            let tilled = total[n] >= BARE_SHARE * known[n].max(1) as f64;
            match out.years[n].iter_mut().find(|(y, _)| y == year) {
                Some((_, t)) => *t = *t || tilled,
                None => out.years[n].push((*year, tilled)),
            }
            out.seen[n] += 1;
            if tilled {
                out.bare[n] += 1;
            }
        }
    }
    out
}

/// Доля деревьев по областям.
pub fn woody_shares(parts: &[i32], woody: Option<&[f64]>, count: usize) -> Option<Vec<f64>> {
    let (total, known) = per_region(parts, woody?, count);
    Some(total.iter().zip(known).map(|(t, k)| t / k.max(1) as f64).collect())
}

/// Виды областей. summer - виды по летнему снимку (region_kinds), years -
/// [`tilled_years`], woody - доли деревьев или None, newest - самый новый
/// год ряда. Итог - (виды, последний год голой почвы или 0).
pub fn field_kinds(summer: &[Kind], years: &Years, woody: Option<&[f64]>, newest: i32) -> (Vec<Kind>, Vec<i32>) {
    let mut kinds = Vec::new();
    let mut last = Vec::new();
    for (n, kind) in summer.iter().enumerate() {
        let empty = Vec::new();
        let list = years.years.get(n).unwrap_or(&empty);
        let mut tilled: Vec<i32> = list.iter().filter(|(_, t)| *t).map(|(y, _)| *y).collect();
        tilled.sort_unstable();
        last.push(tilled.last().copied().unwrap_or(0));
        let tree = woody.map_or(if *kind == Kind::Conifer { 1.0 } else { 0.0 }, |w| w.get(n).copied().unwrap_or(0.0));
        let (bare, seen) = (years.bare.get(n).copied().unwrap_or(0), years.seen.get(n).copied().unwrap_or(0));
        kinds.push(if !FIELD_SOURCES.contains(kind) || list.is_empty() {
            *kind
        } else if *kind == Kind::Soil && seen >= 3 && bare as f64 >= ALWAYS_BARE * seen as f64 {
            // Голое и летом, и почти на всех снимках сезонов - застройка,
            // дороги, карьер. Пашня летом под посевом.
            Kind::Soil
        } else if let Some(year) = tilled.last() {
            let age = newest - year;
            if age <= 0 {
                Kind::Tilled
            } else if age <= RECENT {
                Kind::FallowNew
            } else if tree >= WOODY_SHARE {
                Kind::Overgrown
            } else {
                Kind::FallowOld
            }
        } else if tree >= FOREST_SHARE {
            Kind::Forest
        } else if tree >= WOODY_SHARE {
            Kind::Overgrown
        } else {
            Kind::FallowOld
        });
    }
    (kinds, last)
}

/// Сезон ряда.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Season {
    Spring,
    Autumn,
}

/// Лучший день сезона: конец мая - пашня ещё голая, залежь уже зелёная,
/// начало октября - урожай убран, зябь вспахана, снега ещё нет.
pub fn best_day(season: Season) -> u32 {
    match season {
        Season::Spring => 145,
        Season::Autumn => 274,
    }
}

/// Снимок для подбора: день «ГГГГ-ММ-ДД», облачность, номер снимка.
pub type Shot = (String, f64, usize);

/// Облачность в процентах, которая не различается.
pub const CLOUD_STEP: f64 = 2.0;

/// День года по дню «ГГГГ-ММ-ДД».
pub fn day_of_year(when: &str) -> u32 {
    let month: usize = when.get(5..7).and_then(|m| m.parse().ok()).unwrap_or(1);
    let day: u32 = when.get(8..10).and_then(|d| d.parse().ok()).unwrap_or(1);
    [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334].get(month.saturating_sub(1)).copied().unwrap_or(0) + day
}

/// Лучшие снимки на год и сезон: found - (день «ГГГГ-ММ-ДД», облачность,
/// номер снимка), итог - ((год, сезон), номера снимков, лучший первым:
/// меньше облаков с шагом [`CLOUD_STEP`], ближе к лучшему дню сезона).
pub fn pick(found: &[Shot]) -> Vec<((i32, Season), Vec<usize>)> {
    let mut out: Vec<((i32, Season), Vec<Shot>)> = Vec::new();
    for (when, cloud, id) in found {
        let month: u32 = when.get(5..7).and_then(|m| m.parse().ok()).unwrap_or(0);
        let year: i32 = when.get(..4).and_then(|y| y.parse().ok()).unwrap_or(0);
        for (season, months) in [(Season::Spring, SPRING), (Season::Autumn, AUTUMN)] {
            if months.contains(&month) {
                let key = (year, season);
                match out.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, rows)) => rows.push((when.clone(), *cloud, *id)),
                    None => out.push((key, vec![(when.clone(), *cloud, *id)])),
                }
            }
        }
    }
    out.sort_by_key(|(k, _)| *k);
    out.into_iter()
        .map(|((year, season), mut rows)| {
            let best = best_day(season);
            rows.sort_by(|a, b| {
                let key = |r: &Shot| ((r.1 / CLOUD_STEP).floor() as i64, day_of_year(&r.0).abs_diff(best), r.0.clone());
                key(a).cmp(&key(b))
            });
            ((year, season), rows.into_iter().map(|r| r.2).collect())
        })
        .collect()
}
