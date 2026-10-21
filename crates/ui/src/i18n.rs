// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Тексты для пользователя на русском и английском.

/// Язык текстов.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ru,
    En,
}

impl Lang {
    /// Язык системы: русский для русской системы, иначе английский.
    pub fn from_system() -> Self {
        match sys_locale::get_locale() {
            Some(locale) if locale.to_lowercase().starts_with("ru") => Lang::Ru,
            _ => Lang::En,
        }
    }
}

/// Тексты программы.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Text {
    /// Подпись источника подложки.
    Attribution,
    /// Нет видеокарты с поддержкой wgpu.
    NoGpu,
}

pub fn text(lang: Lang, t: Text) -> &'static str {
    match (lang, t) {
        (Lang::Ru, Text::Attribution) => "© участники OpenStreetMap",
        (Lang::En, Text::Attribution) => "© OpenStreetMap contributors",
        (Lang::Ru, Text::NoGpu) => "Видеокарта не поддерживает wgpu, глобус не рисуется.",
        (Lang::En, Text::NoGpu) => "The graphics card does not support wgpu, the globe cannot be drawn.",
    }
}

/// Страница условий OpenStreetMap для подписи.
pub const OSM_COPYRIGHT: &str = "https://www.openstreetmap.org/copyright";
