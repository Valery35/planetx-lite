// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Издание программы - PlanetX Lite или RustPlanetX, `doc/EDITIONS.md`.
//! Ядро и интерфейс общие, различие - здесь: окно спрашивает издание,
//! а не проверяет флаги по месту. Решения автора от 6 ноября 2026 года -
//! RustPlanetX - отдельное приложение, работает со слоями и видом, как
//! модуль PlanetX для QGIS, своего импорта векторных форматов у него нет.
//!
//! Шаг 1 плана различает название и связь с RustGIS. Раздел слоёв,
//! синхронизация по кнопке и открытие файлов расходятся следующими
//! шагами.

/// Издание программы.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Edition {
    /// Самостоятельный глобус: программа и страница.
    #[default]
    Lite,
    /// Глобус для RustGIS, только программа.
    RustPlanetX,
}

impl Edition {
    /// Название программы в окне и сообщениях, не переводится.
    pub fn name(self) -> &'static str {
        match self {
            Edition::Lite => "PlanetX Lite",
            Edition::RustPlanetX => "RustPlanetX",
        }
    }

    /// Связь с открытым RustGIS есть всегда, без флажка «Связь с
    /// открытым RustGIS».
    pub fn linked(self) -> bool {
        self == Edition::RustPlanetX
    }
}

#[cfg(test)]
mod tests {
    use super::Edition;

    #[test]
    fn editions_differ() {
        assert_eq!(Edition::default(), Edition::Lite);
        assert_eq!((Edition::Lite.name(), Edition::RustPlanetX.name()), ("PlanetX Lite", "RustPlanetX"));
        assert!(Edition::RustPlanetX.linked() && !Edition::Lite.linked());
    }
}
