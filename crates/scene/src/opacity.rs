// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Непрозрачность выделенных «Моих меток» ползунком под списком, как
//! ползунок под «Местами» Google Earth Pro. Перенос `core/opacity.py`
//! модуля.
//!
//! Непрозрачность объекта - альфа его цветов, так её хранит и KML. У метки
//! это цвет линии или значка и цвет заливки, у картинки - её цвет.
//! Ползунок показывает непрозрачность выделения - наибольшую альфу среди
//! выделенных объектов. Движение ползунка умножает альфы, снятые при
//! выделении, на отношение нового значения к прежнему, поэтому заливка
//! остаётся прозрачнее линии в той же доле. Из полной прозрачности альфа
//! ставится прямо значением ползунка, заливка - в [`FILL_SHARE`] от него.

use crate::Rgba;

/// Доля альфы заливки от линии после полной прозрачности.
pub const FILL_SHARE: f64 = 0.5;

/// Непрозрачность объекта 0..1 по его цветам - наибольшая альфа. None
/// пропускается.
pub fn opacity_of(colors: &[Option<Rgba>]) -> f64 {
    colors.iter().flatten().map(|c| f64::from(c[3])).fold(None, |m: Option<f64>, a| Some(m.map_or(a, |m| m.max(a)))).map_or(1.0, |a| a / 255.0)
}

/// Непрозрачность выделения: items - цвета объектов.
pub fn selection_opacity(items: &[Vec<Option<Rgba>>]) -> f64 {
    items.iter().map(|c| opacity_of(c)).fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v)))).unwrap_or(1.0)
}

/// Цвета colors (линия, затем прочие) при смене непрозрачности выделения
/// с base на value, обе 0..1. None остаётся None.
pub fn scaled(colors: &[Option<Rgba>], base: f64, value: f64) -> Vec<Option<Rgba>> {
    colors
        .iter()
        .enumerate()
        .map(|(n, color)| {
            color.map(|c| {
                let alpha = if base > 0.0 {
                    f64::from(c[3]) * value / base
                } else {
                    255.0 * value * if n == 0 { 1.0 } else { FILL_SHARE }
                };
                [c[0], c[1], c[2], alpha.clamp(0.0, 255.0).round() as u8]
            })
        })
        .collect()
}
