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

    /// Версия для окна «О программе». У RustPlanetX свой номер выпуска
    /// из plugin.json хранилища rustplanetx: его задаёт сборка выпуска
    /// переменной `RUSTPLANETX_VERSION`, рядом - версия Lite внутри.
    /// Без переменной - только версия Lite.
    pub fn version(self) -> String {
        Self::version_of(self, option_env!("RUSTPLANETX_VERSION"))
    }

    fn version_of(self, own: Option<&str>) -> String {
        let lite = env!("CARGO_PKG_VERSION");
        match (self, own.map(str::trim).filter(|v| !v.is_empty())) {
            (Edition::RustPlanetX, Some(v)) => format!("{v} · PlanetX Lite {lite}"),
            (Edition::RustPlanetX, None) => format!("PlanetX Lite {lite}"),
            (Edition::Lite, _) => lite.to_string(),
        }
    }

    /// Хранилище издания: исходный код и сообщения об ошибках.
    pub fn repository(self) -> &'static str {
        match self {
            Edition::Lite => "https://github.com/Valery35/planetx-lite",
            Edition::RustPlanetX => "https://github.com/Valery35/rustplanetx",
        }
    }

    /// Строка о программе под названием, для перевода.
    pub fn tagline(self) -> &'static str {
        match self {
            Edition::Lite => "The PlanetX globe without QGIS.",
            Edition::RustPlanetX => "The PlanetX globe for RustGIS.",
        }
    }

    /// Описание программы, для перевода.
    pub fn about(self) -> &'static str {
        match self {
            Edition::Lite => "A 3D globe for Windows, Linux, macOS and the web: terrain, imagery, scenes, KML and your own data.",
            Edition::RustPlanetX => "A 3D globe linked with the RustGIS map: the layers of the project in their RustGIS look, terrain, imagery, scenes and KML. The globe is PlanetX Lite built for RustGIS.",
        }
    }
}

/// Издание запущенной программы - RustPlanetX. Ставит окно при запуске:
/// подписи меню знают издание без доступа к окну.
static RUSTPLANETX: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Издание запущенной программы.
pub fn set_current(edition: Edition) {
    RUSTPLANETX.store(edition.linked(), std::sync::atomic::Ordering::Relaxed);
}

/// Подсказка пунктов «… в проект RustGIS» у RustPlanetX, ключ перевода.
pub const TO_PROJECT_TIP: &str = "It goes to the open RustGIS project into the group RustPlanetX - globe layers, as the PlanetX plugin puts it into the QGIS project.";

/// Подпись пункта «… в Мои слои» по изданию: у RustPlanetX «Моих
/// слоёв» нет, слой уходит в проект RustGIS. Ключ перевода.
pub fn to_layers(key: &'static str) -> &'static str {
    to_layers_of(RUSTPLANETX.load(std::sync::atomic::Ordering::Relaxed), key)
}

/// Подсказка пункта «… в Мои слои» по изданию, ключ перевода.
pub fn to_layers_tip(key: &'static str) -> &'static str {
    if RUSTPLANETX.load(std::sync::atomic::Ordering::Relaxed) { TO_PROJECT_TIP } else { key }
}

fn to_layers_of(rust: bool, key: &'static str) -> &'static str {
    if !rust {
        return key;
    }
    match key {
        "View objects to My layers" => "View objects to the RustGIS project",
        "View picture to My layers" => "View picture to the RustGIS project",
        "Contours of the view to My layers" => "Contours of the view to the RustGIS project",
        "Placemarks to My layers" => "Placemarks to the RustGIS project",
        "Copy to My layers" => "Copy to the RustGIS project",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::Edition;

    /// У RustPlanetX пункты «… в Мои слои» называют проект RustGIS.
    #[test]
    fn layer_items_name_the_rustgis_project() {
        assert_eq!(super::to_layers_of(false, "Placemarks to My layers"), "Placemarks to My layers");
        assert_eq!(super::to_layers_of(true, "Placemarks to My layers"), "Placemarks to the RustGIS project");
        assert_eq!(super::to_layers_of(true, "Contours of the view to My layers"), "Contours of the view to the RustGIS project");
        assert_eq!(super::to_layers_of(true, "Save as KML…"), "Save as KML…");
    }

    #[test]
    fn editions_differ() {
        assert_eq!(Edition::default(), Edition::Lite);
        assert_eq!((Edition::Lite.name(), Edition::RustPlanetX.name()), ("PlanetX Lite", "RustPlanetX"));
        assert!(Edition::RustPlanetX.linked() && !Edition::Lite.linked());
    }

    /// Окно «О программе» RustPlanetX показывало «PlanetX Lite» и номер
    /// Lite, замечание автора от 8 ноября 2026 года.
    #[test]
    fn rustplanetx_has_its_own_version_and_repository() {
        let lite = env!("CARGO_PKG_VERSION");
        assert_eq!(Edition::RustPlanetX.version_of(Some("0.5.2")), format!("0.5.2 · PlanetX Lite {lite}"));
        assert_eq!(Edition::RustPlanetX.version_of(Some(" ")), format!("PlanetX Lite {lite}"));
        assert_eq!(Edition::Lite.version_of(Some("0.5.2")), lite);
        assert!(Edition::RustPlanetX.repository().ends_with("/rustplanetx") && Edition::Lite.repository().ends_with("/planetx-lite"));
        assert_ne!(Edition::RustPlanetX.about(), Edition::Lite.about());
    }
}
