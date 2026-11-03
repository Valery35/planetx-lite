// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Типовые задачи выделения объектов поверх трассировки, перенос
//! `core/objecttasks.py` модуля 1.33-1.36.
//!
//! Задача - настройки трассировки плюс правила названий цветов по спектру
//! снимка. Правила работают по средним отражениям цвета в каналах,
//! поэтому каналы снимка нужно знать: подписи каналов («B11 swir16») или
//! имя файла «S2_20240701_40VDK_agriculture» ([`channels`]). Нужен
//! ближний ИК и красный или SWIR 1,6 мкм. У подложки глобуса и растра без
//! подписей каналов правил нет.
//!
//! Пороги отражения - по типичным спектрам USGS Spectral Library и практике
//! дешифрирования Sentinel-2 и Landsat, выбор помощника модуля. Хвоя
//! от листвы отделяется не порогом, а внутри снимка: тёмная по ближнему
//! ИК часть растительности.

use crate::segments;
use crate::sentinel::COMPOSITES;

/// Роли каналов по ключам каталогов Earth Search и Planetary Computer.
pub const ROLES: [(&str, &str); 13] = [
    ("blue", "blue"),
    ("green", "green"),
    ("red", "red"),
    ("nir", "nir"),
    ("nir08", "nir"),
    ("swir16", "swir1"),
    ("swir22", "swir2"),
    ("coastal", "coastal"),
    ("rededge1", "re1"),
    ("rededge2", "re2"),
    ("rededge3", "re3"),
    ("nir09", "vapour"),
    ("lwir", "thermal"),
];

/// Вода темнее в ближнем ИК и SWIR.
pub const WATER_NIR: f64 = 0.07;
pub const WATER_SWIR: f64 = 0.05;
/// Растительность по (nir - red) / (nir + red) и по (nir - swir1) /
/// (nir + swir1), если красного нет.
pub const VEG_NDVI: f64 = 0.35;
pub const VEG_NDMI: f64 = 0.05;
/// Снег ярок в видимом и тёмен в SWIR.
pub const SNOW_VIS: f64 = 0.35;
pub const SNOW_SWIR: f64 = 0.15;
/// Нижняя доля размаха ближнего ИК растительности - хвоя.
pub const CONIFER_SHARE: f64 = 0.4;
/// Размах ниже этого - хвою не отделить.
pub const CONIFER_SPREAD: f64 = 0.05;
/// ИК меньшей даты к большей у хвои по двум снимкам.
pub const EVERGREEN: f64 = 0.75;
/// Видимая яркость хвои на снежном снимке не выше.
pub const WINTER_DARK: f64 = 0.25;
/// Значения выше - не отражение.
pub const REFLECTANCE_MAX: f64 = 1.5;
/// Доля хвои вокруг почвы, с которой это рубка.
pub const NEAR_FOREST: f64 = 0.25;
/// Клеток, ширина полосы вокруг куска почвы.
pub const RIM: usize = 3;
/// Доля внутренних клеток куска хвои, с которой это лес.
pub const THICK: f64 = 0.5;

/// Вид цвета, области или поля. Виды заросших полей - [`crate::fallow`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Water,
    Conifer,
    Broadleaf,
    Vegetation,
    Soil,
    Clearing,
    Snow,
    Other,
    Tilled,
    FallowNew,
    FallowOld,
    Overgrown,
    Forest,
}

impl Kind {
    /// Ключ вида, как у модуля.
    pub fn key(self) -> &'static str {
        match self {
            Kind::Water => "water",
            Kind::Conifer => "conifer",
            Kind::Broadleaf => "broadleaf",
            Kind::Vegetation => "vegetation",
            Kind::Soil => "soil",
            Kind::Clearing => "clearing",
            Kind::Snow => "snow",
            Kind::Other => "other",
            Kind::Tilled => "tilled",
            Kind::FallowNew => "fallow_new",
            Kind::FallowOld => "fallow_old",
            Kind::Overgrown => "overgrown",
            Kind::Forest => "forest",
        }
    }

    /// Цвет вида в слое - выбор помощника модуля.
    pub fn color(self) -> [u8; 3] {
        match self {
            Kind::Water => [40, 110, 220],
            Kind::Conifer => [20, 90, 40],
            Kind::Broadleaf => [110, 190, 80],
            Kind::Vegetation => [90, 170, 70],
            Kind::Soil => [205, 160, 110],
            Kind::Clearing => [230, 80, 160],
            Kind::Snow => [235, 240, 250],
            Kind::Other => [150, 150, 150],
            Kind::Tilled => [190, 135, 80],
            Kind::FallowNew => [240, 200, 70],
            Kind::FallowOld => [175, 205, 95],
            Kind::Overgrown => [95, 150, 60],
            Kind::Forest => [25, 90, 45],
        }
    }
}

/// Задача окна.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    Forest,
    Fields,
    Fallow,
    Water,
}

/// Задачи по порядку списка окна.
pub const SCENARIOS: [Scenario; 4] = [Scenario::Forest, Scenario::Fields, Scenario::Water, Scenario::Fallow];

/// Настройки трассировки задачи и виды, которые она различает.
pub struct Spec {
    pub colors: usize,
    pub smooth: usize,
    pub least: usize,
    pub kinds: &'static [Kind],
    /// Вид, которого в kinds нет, переходит в замену, без замены -
    /// «прочее».
    pub merge: &'static [(Kind, Kind)],
}

impl Scenario {
    pub fn key(self) -> &'static str {
        match self {
            Scenario::Forest => "forest",
            Scenario::Fields => "fields",
            Scenario::Fallow => "fallow",
            Scenario::Water => "water",
        }
    }

    pub fn spec(self) -> Spec {
        match self {
            Scenario::Forest => Spec {
                colors: 8,
                smooth: 1,
                least: 25,
                kinds: &[Kind::Conifer, Kind::Broadleaf, Kind::Clearing, Kind::Soil, Kind::Water, Kind::Snow],
                merge: &[(Kind::Vegetation, Kind::Broadleaf)],
            },
            Scenario::Fields => Spec {
                colors: 8,
                smooth: 1,
                least: 50,
                kinds: &[Kind::Vegetation, Kind::Soil, Kind::Clearing, Kind::Water, Kind::Snow],
                merge: &[(Kind::Conifer, Kind::Vegetation), (Kind::Broadleaf, Kind::Vegetation)],
            },
            // Заросшие поля ([`crate::fallow`]): летние виды - основа, вид
            // поля даёт ряд снимков весны и осени.
            Scenario::Fallow => Spec {
                colors: 8,
                smooth: 1,
                least: 50,
                kinds: &[Kind::Conifer, Kind::Broadleaf, Kind::Soil, Kind::Water, Kind::Snow],
                merge: &[(Kind::Vegetation, Kind::Broadleaf)],
            },
            Scenario::Water => Spec {
                colors: 4,
                smooth: 1,
                least: 25,
                kinds: &[Kind::Water, Kind::Other],
                merge: &[(Kind::Conifer, Kind::Other), (Kind::Broadleaf, Kind::Other), (Kind::Vegetation, Kind::Other), (Kind::Soil, Kind::Other), (Kind::Clearing, Kind::Other), (Kind::Snow, Kind::Other)],
            },
        }
    }
}

/// Порядок классов в слое.
pub const KIND_ORDER: [Kind; 8] = [Kind::Conifer, Kind::Broadleaf, Kind::Vegetation, Kind::Clearing, Kind::Soil, Kind::Water, Kind::Snow, Kind::Other];

/// Роль канала по ключу каталога.
fn role(key: &str) -> Option<&'static str> {
    ROLES.iter().find(|(k, _)| *k == key).map(|(_, r)| *r)
}

/// Имя файла без хвоста «_2» разбирается на (спутник, день ГГГГММДД,
/// сочетание): «S2_20240701_40VDK_agriculture», как `FILE` модуля.
fn file_parts(stem: &str) -> Option<(&str, &str, &str)> {
    let stem = match stem.rsplit_once('_') {
        Some((head, tail)) if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => stem,
    };
    let mut words = stem.split('_');
    let mission = words.next()?;
    let day = words.next()?;
    let last = stem.rsplit_once('_')?.1;
    let rest = stem.get(mission.len() + day.len() + 2..)?;
    let mission_ok = mission == "S2" || {
        let b = mission.as_bytes();
        match b {
            [b'L', d] => d.is_ascii_digit(),
            [b'L', m, d] => matches!(m, b'C' | b'E' | b'O' | b'T' | b'M') && d.is_ascii_digit(),
            _ => false,
        }
    };
    let day_ok = day.len() == 8 && day.bytes().all(|b| b.is_ascii_digit());
    // После дня - хотя бы одно слово и сочетание из строчных латинских.
    let last_ok = !last.is_empty() && last.bytes().all(|b| b.is_ascii_lowercase()) && rest.len() > last.len();
    (mission_ok && day_ok && last_ok).then_some((mission, day, last))
}

/// Роли каналов растра по подписям каналов (последнее слово подписи -
/// ключ каталога, «B11 swir16») или по имени файла. Итог - роли по
/// каналам или None.
pub fn channels(descriptions: &[String], file_stem: &str) -> Option<Vec<Option<&'static str>>> {
    let found: Vec<Option<&'static str>> = descriptions
        .iter()
        .map(|text| {
            let lower = text.trim().to_lowercase();
            lower.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()).collect::<Vec<_>>().into_iter().rev().find_map(role)
        })
        .collect();
    if found.iter().any(Option::is_some) {
        return Some(found);
    }
    let (_, _, product) = file_parts(file_stem)?;
    let (_, keys) = COMPOSITES.iter().find(|(name, _)| *name == product)?;
    Some(keys.iter().map(|k| role(k)).collect())
}

/// День съёмки «ГГГГ-ММ-ДД» по имени файла или None.
pub fn day_of(file_stem: &str) -> Option<String> {
    let (_, raw, _) = file_parts(file_stem)?;
    Some(format!("{}-{}-{}", &raw[..4], &raw[4..6], &raw[6..]))
}

/// Месяц съёмки по имени файла, None - не узнать.
pub fn month_of(file_stem: &str) -> Option<u32> {
    day_of(file_stem).and_then(|d| d.get(5..7)?.parse().ok())
}

/// Роли каналов, которые узнало окно, по порядку без повторов.
pub fn role_names(roles: &[Option<&'static str>]) -> Vec<&'static str> {
    let mut seen = Vec::new();
    for r in roles.iter().flatten() {
        if !seen.contains(r) {
            seen.push(*r);
        }
    }
    seen
}

/// Хватает ли каналов для правил: ближний ИК и красный или SWIR.
pub fn usable(roles: &[Option<&str>]) -> bool {
    let has = |r: &str| roles.contains(&Some(r));
    has("nir") && (has("red") || has("swir1"))
}

fn ratio(a: f64, b: f64) -> f64 {
    let total = a + b;
    if total > 1e-6 { (a - b) / total } else { 0.0 }
}

/// Номер канала роли.
fn at(roles: &[Option<&str>], role: &str) -> Option<usize> {
    roles.iter().position(|r| *r == Some(role))
}

/// Вид каждого цвета трассировки по средним каналов centers и ролям
/// каналов: вода, хвоя, листва, почва, снег. Хвоя - тёмная по ближнему
/// ИК доля растительности, если размах её ИК больше [`CONIFER_SPREAD`],
/// иначе вся растительность - [`Kind::Vegetation`].
pub fn color_kinds(centers: &[Vec<f64>], roles: &[Option<&str>]) -> Vec<Kind> {
    let get = |row: &[f64], role: &str| at(roles, role).and_then(|n| row.get(n).copied());
    let mut kinds: Vec<Kind> = centers
        .iter()
        .map(|row| {
            let nir = get(row, "nir").unwrap_or(0.0);
            let swir = get(row, "swir1");
            let red = get(row, "red");
            let visible: Vec<f64> = ["blue", "green", "red"].iter().filter_map(|r| get(row, r)).collect();
            if !visible.is_empty() && visible.iter().copied().fold(f64::INFINITY, f64::min) > SNOW_VIS && swir.is_none_or(|s| s < SNOW_SWIR) {
                Kind::Snow
            } else if nir < WATER_NIR && swir.is_none_or(|s| s < WATER_SWIR) {
                Kind::Water
            } else if red.map_or_else(|| ratio(nir, swir.unwrap_or(0.0)) > VEG_NDMI, |r| ratio(nir, r) > VEG_NDVI) {
                Kind::Vegetation
            } else {
                Kind::Soil
            }
        })
        .collect();
    let veg: Vec<usize> = kinds.iter().enumerate().filter(|(_, k)| **k == Kind::Vegetation).map(|(n, _)| n).collect();
    if veg.len() > 1
        && let Some(band) = at(roles, "nir")
    {
        let nirs: Vec<f64> = veg.iter().map(|n| centers[*n].get(band).copied().unwrap_or(0.0)).collect();
        let low = nirs.iter().copied().fold(f64::INFINITY, f64::min);
        let high = nirs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if high - low > CONIFER_SPREAD {
            let edge = low + CONIFER_SHARE * (high - low);
            for (n, value) in veg.iter().zip(nirs) {
                kinds[*n] = if value <= edge { Kind::Conifer } else { Kind::Broadleaf };
            }
        }
    }
    kinds
}

/// Видимая яркость второго снимка по цветам, если на нём лежит снег,
/// иначе None. Снег - хотя бы у одного цвета видимые каналы ярче
/// [`SNOW_VIS`], а SWIR, если он есть, темнее [`SNOW_SWIR`]. Каналы
/// второго снимка в centers идут за roles.len() каналами первого.
pub fn winter(centers: &[Vec<f64>], roles: &[Option<&str>], more: &[Option<&str>]) -> Option<Vec<f64>> {
    let first = roles.len();
    let visible: Vec<usize> = more.iter().enumerate().filter(|(_, r)| matches!(r, Some("blue" | "green" | "red"))).map(|(n, _)| first + n).collect();
    if visible.is_empty() {
        return None;
    }
    let swir = at(more, "swir1").map(|n| first + n);
    let bright: Vec<f64> = centers.iter().map(|row| visible.iter().map(|n| row.get(*n).copied().unwrap_or(0.0)).fold(f64::INFINITY, f64::min)).collect();
    let snowy = centers.iter().zip(&bright).any(|(row, b)| *b > SNOW_VIS && swir.is_none_or(|s| row.get(s).copied().unwrap_or(0.0) < SNOW_SWIR));
    snowy.then_some(bright)
}

/// Виды цветов по двум снимкам: centers - средние каналов первого снимка
/// и за ними второго, roles и more - их роли.
///
/// Второй снимок со снегом (зима - лето, просьба автора модуля от
/// 3 ноября 2026 года): хвоя - растительность, которая на снегу осталась
/// тёмной в видимых каналах (темнее [`WINTER_DARK`]) и летом не светлая
/// в ближнем ИК (вид первого снимка - хвоя или растительность без
/// разделения). Остальная растительность - листва и травы. Голый
/// лиственный лес на снегу бывает тёмным от стволов и теней - его отличает
/// яркий летний ИК, тёмный луг или болото летом похожи на хвою - их
/// отличает снег.
///
/// Без снега: растительность, у которой ближний ИК на двух датах почти не
/// меняется (меньшее к большему не ниже [`EVERGREEN`]), - хвоя, остальная -
/// листва и травы. Без ближнего ИК у второго снимка виды прежние.
pub fn two_dates(kinds: &[Kind], centers: &[Vec<f64>], roles: &[Option<&str>], more: &[Option<&str>]) -> Vec<Kind> {
    let veg = |k: Kind| matches!(k, Kind::Conifer | Kind::Broadleaf | Kind::Vegetation);
    if let Some(snow) = winter(centers, roles, more) {
        return kinds.iter().zip(snow).map(|(k, b)| if veg(*k) { if b < WINTER_DARK && *k != Kind::Broadleaf { Kind::Conifer } else { Kind::Broadleaf } } else { *k }).collect();
    }
    let (Some(first), Some(second)) = (at(roles, "nir"), at(more, "nir").map(|n| roles.len() + n)) else {
        return kinds.to_vec();
    };
    kinds
        .iter()
        .zip(centers)
        .map(|(k, row)| {
            if !veg(*k) {
                return *k;
            }
            let (a, b) = (row.get(first).copied().unwrap_or(0.0), row.get(second).copied().unwrap_or(0.0));
            let high = a.max(b);
            let keep = if high > 0.0 { a.min(b) / high } else { 0.0 };
            if keep >= EVERGREEN { Kind::Conifer } else { Kind::Broadleaf }
        })
        .collect()
}

/// Виды цветов, которые различает задача: прочие - в замену.
pub fn scenario_kinds(kinds: &[Kind], scenario: Scenario) -> Vec<Kind> {
    let spec = scenario.spec();
    kinds
        .iter()
        .map(|k| {
            let k = spec.merge.iter().find(|(from, _)| from == k).map_or(*k, |(_, to)| *to);
            if spec.kinds.contains(&k) { k } else { Kind::Other }
        })
        .collect()
}

/// Доля внутренних клеток каждого куска: все соседи в сетке - тот же
/// кусок, край сетки не в счёт. У кольца в одну-две клетки доля близка
/// к нулю.
pub fn inner_shares(pieces: &[i32], rows: usize, cols: usize) -> Vec<f64> {
    let count = pieces.iter().copied().max().map_or(0, |m| usize::try_from(m + 1).unwrap_or(0));
    let mut total = vec![0usize; count];
    let mut found = vec![0usize; count];
    for r in 0..rows {
        for c in 0..cols {
            let at = r * cols + c;
            let Ok(p) = usize::try_from(pieces[at]) else {
                continue;
            };
            total[p] += 1;
            let same = |q: usize| pieces[q] == pieces[at];
            if (r == 0 || same(at - cols)) && (r + 1 == rows || same(at + cols)) && (c == 0 || same(at - 1)) && (c + 1 == cols || same(at + 1)) {
                found[p] += 1;
            }
        }
    }
    found.iter().zip(&total).map(|(f, t)| *f as f64 / (*t).max(1) as f64).collect()
}

/// Доля вида target в полосе шириной width клеток вокруг каждого куска
/// из wanted: (кусок, доля). Полоса - клетки участка вне куска не дальше
/// width шагов по сторонам клеток. Смешанные пиксели края дают тонкое
/// кольцо другого вида, по одной границе кусок его и видел бы.
pub fn ring_shares(pieces: &[i32], rows: usize, cols: usize, piece_kinds: &[Kind], target: Kind, wanted: &[usize], width: usize) -> Vec<(usize, f64)> {
    let mut out = Vec::new();
    for &n in wanted {
        let mut box_ = (usize::MAX, 0, usize::MAX, 0);
        for r in 0..rows {
            for c in 0..cols {
                if usize::try_from(pieces[r * cols + c]).ok() == Some(n) {
                    box_ = (box_.0.min(r), box_.1.max(r), box_.2.min(c), box_.3.max(c));
                }
            }
        }
        if box_.0 == usize::MAX {
            continue;
        }
        let (r0, r1) = (box_.0.saturating_sub(width), (box_.1 + width + 1).min(rows));
        let (c0, c1) = (box_.2.saturating_sub(width), (box_.3 + width + 1).min(cols));
        let (h, w) = (r1 - r0, c1 - c0);
        let piece = |r: usize, c: usize| pieces[(r0 + r) * cols + c0 + c];
        let mine: Vec<bool> = (0..h * w).map(|k| usize::try_from(piece(k / w, k % w)).ok() == Some(n)).collect();
        let mut grown = mine.clone();
        for _ in 0..width {
            let mut step = grown.clone();
            for r in 0..h {
                for c in 0..w {
                    let k = r * w + c;
                    if (r > 0 && grown[k - w]) || (r + 1 < h && grown[k + w]) || (c > 0 && grown[k - 1]) || (c + 1 < w && grown[k + 1]) {
                        step[k] = true;
                    }
                }
            }
            grown = step;
        }
        let (mut total, mut hit) = (0usize, 0usize);
        for k in 0..h * w {
            let p = piece(k / w, k % w);
            if grown[k] && !mine[k] && p >= 0 {
                total += 1;
                if usize::try_from(p).ok().and_then(|p| piece_kinds.get(p)) == Some(&target) {
                    hit += 1;
                }
            }
        }
        out.push((n, if total > 0 { hit as f64 / total as f64 } else { 0.0 }));
    }
    out
}

/// Виды областей по задаче. kinds - виды цветов ([`color_kinds`]),
/// region_colors[n] - цвет области n, parts - номера областей клеток.
/// Почва, вокруг которой в полосе [`RIM`] клеток не меньше
/// [`NEAR_FOREST`] хвойного леса, становится рубкой, если задача
/// различает рубки. Лесом считается только хвоя с толщей: светлая
/// растительность бывает и посевом. Без отделённой хвои рубок нет.
pub fn region_kinds(parts: &[i32], rows: usize, cols: usize, region_colors: &[i16], kinds: &[Kind], scenario: Scenario) -> Vec<Kind> {
    let mut out: Vec<Kind> = region_colors.iter().map(|c| usize::try_from(*c).ok().and_then(|c| kinds.get(c)).copied().unwrap_or(Kind::Other)).collect();
    if scenario.spec().kinds.contains(&Kind::Clearing) && out.contains(&Kind::Conifer) {
        // Цветов больше, чем видов: вырубка делится на области разных
        // оттенков почвы. Доля границы считается по связному куску одного
        // вида.
        let mut codes: Vec<Kind> = out.clone();
        codes.sort_unstable();
        codes.dedup();
        let code_of: Vec<i16> = out.iter().map(|k| codes.iter().position(|c| c == k).map_or(-1, |p| p as i16)).collect();
        // Клетка с номером области вне списка - без области.
        let safe: Vec<i32> = parts.iter().map(|p| if usize::try_from(*p).is_ok_and(|p| p < out.len()) { *p } else { -1 }).collect();
        let (whole, whole_kinds) = segments::dissolve(&safe, rows, cols, &code_of);
        let thick = inner_shares(&whole, rows, cols);
        let piece_kinds: Vec<Kind> = whole_kinds
            .iter()
            .enumerate()
            .map(|(n, c)| {
                let k = usize::try_from(*c).ok().and_then(|c| codes.get(c)).copied().unwrap_or(Kind::Other);
                if k == Kind::Conifer && thick.get(n).copied().unwrap_or(0.0) < THICK { Kind::Other } else { k }
            })
            .collect();
        let soil: Vec<usize> = piece_kinds.iter().enumerate().filter(|(_, k)| **k == Kind::Soil).map(|(n, _)| n).collect();
        let shares = ring_shares(&whole, rows, cols, &piece_kinds, Kind::Conifer, &soil, RIM);
        // Кусок каждой области - по её первой клетке.
        let mut piece = vec![-1i32; out.len()];
        for (p, w) in safe.iter().zip(&whole) {
            if let Ok(p) = usize::try_from(*p)
                && piece[p] < 0
            {
                piece[p] = *w;
            }
        }
        for (n, k) in out.iter_mut().enumerate() {
            let share = usize::try_from(piece[n]).ok().and_then(|p| shares.iter().find(|(q, _)| *q == p)).map_or(0.0, |(_, s)| *s);
            if *k == Kind::Soil && share >= NEAR_FOREST {
                *k = Kind::Clearing;
            }
        }
    }
    scenario_kinds(&out, scenario)
}

/// Группы линейных объектов: ключ и виды областей, из которых линии.
pub const LINE_GROUPS: [(&str, &[Kind]); 3] = [("roads", &[Kind::Soil, Kind::Clearing]), ("water", &[Kind::Water]), ("belts", &[Kind::Broadleaf, Kind::Vegetation])];

/// Цвет линий группы на глобусе.
pub fn line_color(group: &str) -> [u8; 3] {
    match group {
        "roads" => [255, 255, 255],
        "water" => [120, 220, 255],
        _ => [255, 225, 60],
    }
}

/// Маски видов для линий: (группа, маска клеток). region_list[n] - вид
/// области n ([`region_kinds`]).
pub fn line_masks(parts: &[i32], region_list: &[Kind]) -> Vec<(&'static str, Vec<bool>)> {
    LINE_GROUPS
        .iter()
        .map(|(group, members)| {
            let mask = parts.iter().map(|p| usize::try_from(*p).ok().and_then(|p| region_list.get(p)).is_some_and(|k| members.contains(k))).collect();
            (*group, mask)
        })
        .collect()
}

/// Классы слоя: виды областей по порядку [`KIND_ORDER`], которые
/// встретились. Итог - (виды классов, номер класса каждой области).
pub fn classes_of(region_list: &[Kind]) -> (Vec<Kind>, Vec<usize>) {
    let used: Vec<Kind> = KIND_ORDER.iter().copied().filter(|k| region_list.contains(k)).collect();
    let numbers = region_list.iter().map(|k| used.iter().position(|u| u == k).unwrap_or(0)).collect();
    (used, numbers)
}
