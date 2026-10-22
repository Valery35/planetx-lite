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
    /// Подсказка в пустой строке поиска.
    SearchHint,
}

pub fn text(lang: Lang, t: Text) -> &'static str {
    match (lang, t) {
        (Lang::Ru, Text::NoGpu) => "Видеокарта не поддерживает wgpu, глобус не рисуется.",
        (Lang::En, Text::NoGpu) => "The graphics card does not support wgpu, the globe cannot be drawn.",
        (Lang::Ru, Text::Basemap) => "Основа",
        (Lang::En, Text::Basemap) => "Basemap",
        (Lang::Ru, Text::SearchHint) => "Поиск места, Enter",
        (Lang::En, Text::SearchHint) => "Find a place, Enter",
    }
}

/// Сообщение под строкой поиска.
pub fn search_status(lang: Lang, status: &crate::search::Status) -> Option<String> {
    use crate::search::Status;
    Some(match (lang, status) {
        (_, Status::None) => return None,
        (Lang::Ru, Status::Searching(q)) => format!("Поиск: {q}"),
        (Lang::En, Status::Searching(q)) => format!("Searching: {q}"),
        (Lang::Ru, Status::NotFound(q)) => format!("Ничего не найдено: {q}"),
        (Lang::En, Status::NotFound(q)) => format!("Nothing found: {q}"),
        (Lang::Ru, Status::Failed(e)) => format!("Поиск не удался: {e}"),
        (Lang::En, Status::Failed(e)) => format!("Search failed: {e}"),
    })
}
