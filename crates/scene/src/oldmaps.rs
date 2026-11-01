// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Исторические карты по годам, перенос `core/oldmaps.py` модуля.
//!
//! Карта с годом - картинка на поверхности с моментом времени. Карты одной
//! папки «Моих меток» сменяют друг друга. При открытой шкале времени видна
//! последняя карта не позже правого бегунка, до года первой карты группы
//! не видно ни одной. Карта с промежутком времени этому правилу
//! не подчиняется, она видна в своём промежутке, как метка.

use std::collections::{BTreeSet, HashMap};
use std::hash::Hash;

use crate::when::{self, Time};

/// Время-момент карты года: пара строк «1890».
pub fn year_time(year: i64) -> Time {
    let text = if year > 0 { format!("{year:04}") } else { format!("-{:04}", 1 - year) };
    when::stamp(&text)
}

/// Подпись года момента: «1890», до н. э. - «-500».
pub fn year_label(seconds: f64) -> String {
    let year = when::civil(seconds).0;
    (if year > 0 { year } else { year - 1 }).to_string()
}

/// Секунды момента карты или None, если время не момент.
pub fn start(time: Option<&Time>) -> Option<f64> {
    if !when::is_stamp(time) {
        return None;
    }
    when::parse(&time?.0)
}

/// Номера видимых карт из entries - (группа, секунды момента). moment
/// None - шкала закрыта, видны все. В группе - карта с наибольшим моментом
/// не позже moment, при равных - все такие.
pub fn shown<G: Eq + Hash>(entries: &[(G, f64)], moment: Option<f64>) -> BTreeSet<usize> {
    let Some(moment) = moment else {
        return (0..entries.len()).collect();
    };
    let mut best: HashMap<&G, f64> = HashMap::new();
    for (group, at) in entries {
        if *at <= moment && best.get(group).is_none_or(|b| at > b) {
            best.insert(group, *at);
        }
    }
    entries.iter().enumerate().filter(|(_, (g, at))| best.get(g) == Some(at)).map(|(i, _)| i).collect()
}

/// Соседний момент карт для шага шкалы ◂ ▸ или None. delta -1 - ближайший
/// раньше moment, +1 - ближайший позже.
pub fn step(moments: &[f64], moment: f64, delta: i32) -> Option<f64> {
    let finite = moments.iter().copied().filter(|m| m.is_finite());
    if delta < 0 { finite.filter(|m| *m < moment - 0.5).reduce(f64::max) } else { finite.filter(|m| *m > moment + 0.5).reduce(f64::min) }
}

/// Год из строки поля: «1890», «-500», пусто - Ok(None). Астрономический
/// год до н. э. на 1 больше.
pub fn parse_year(text: &str) -> Result<Option<i64>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let year: i64 = text.parse().map_err(|_| text.to_string())?;
    if year == 0 || year.abs() > 99999 {
        return Err(text.to_string());
    }
    Ok(Some(if year > 0 { year } else { year + 1 }))
}
