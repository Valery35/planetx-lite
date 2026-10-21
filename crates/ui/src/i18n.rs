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
    /// Нет видеокарты с поддержкой wgpu.
    NoGpu,
    /// Подпись выбора основы.
    Basemap,
}

pub fn text(lang: Lang, t: Text) -> &'static str {
    match (lang, t) {
        (Lang::Ru, Text::NoGpu) => "Видеокарта не поддерживает wgpu, глобус не рисуется.",
        (Lang::En, Text::NoGpu) => "The graphics card does not support wgpu, the globe cannot be drawn.",
        (Lang::Ru, Text::Basemap) => "Основа",
        (Lang::En, Text::Basemap) => "Basemap",
    }
}

/// Подпись источника высот, как в модуле. Это название наборов данных,
/// а не текст программы, поэтому без перевода.
pub const TERRAIN_SOURCES: &str = "Terrain: Mapzen, SRTM, GMTED, ETOPO1 and others";
