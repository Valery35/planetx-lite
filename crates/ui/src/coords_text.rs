// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Координаты словами на языке интерфейса: формат свойств вида
//! и полушария переводом, как `format_point` модуля с `tr`.

use crate::i18n::{Lang, tr};

/// Точка в формате key из [`crate::settings::COORD_FORMATS`]. digits -
/// знаков после запятой у десятичных градусов.
pub fn format(lang: Lang, lat: f64, lon: f64, key: &str, digits: usize) -> String {
    let fmt = scene::coords::Format::from_key(key).unwrap_or(scene::coords::Format::Decimal);
    let hemispheres = [tr(lang, "N"), tr(lang, "S"), tr(lang, "E"), tr(lang, "W")];
    scene::coords::format_point(lat, lon, fmt, hemispheres, digits)
}
