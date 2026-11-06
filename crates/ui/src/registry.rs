// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Реестр слоёв каталога: одна функция - свой файл и одна строка в [`ALL`],
//! без правки центра. Правило 2 раздела «Как снять перегруз» `doc/PLAN.md`,
//! решение автора от 7 ноября 2026 года.
//!
//! Строка реестра - описание слоя ([`Entry`]): ключ настроек, название,
//! подсказка, раздел каталога, слова поиска команд, источник, легенда.
//! Включённый флажком слой - его состояние ([`Layer`]): шаг кадра,
//! картинка тайлов над картами, рисунок поверх глобуса, загрузка, подпись
//! источника. Окно обходит включённые слои реестра в каждой из этих точек
//! и не знает их по именам. Слой каталога - вариант
//! [`crate::layers::Extra::Plugged`] с номером строки реестра.

use eframe::egui;
use globe_core::camera::Camera;

use crate::i18n::Lang;
use crate::layers::Category;
use crate::thumbs::Recipe;
use crate::vector::Painter;

/// Что нужно слою в кадре.
pub struct Ctx<'a> {
    pub lang: Lang,
    /// Время кадра, секунды.
    pub now: f64,
    /// Правый бегунок открытой шкалы времени, без шкалы - None.
    pub moment: Option<f64>,
    /// Точка взгляда (широта, долгота), высота глаза и ширина вида, метры.
    pub view: (f64, f64, f64, f64),
    /// Запрос в сеть с пределом ответа в байтах.
    pub request: &'a dyn Fn(u64) -> Box<dyn net::Request>,
}

/// Что нужно слою, когда его включают флажком.
pub struct Make<'a> {
    pub lang: Lang,
    /// Папка данных программы, у страницы - None.
    pub data_dir: Option<&'a std::path::Path>,
    /// Запрос в сеть с пределом ответа в байтах.
    pub request: &'a dyn Fn(u64) -> Box<dyn net::Request>,
}

/// Итог шага кадра.
#[derive(Debug, Default)]
pub struct Stepped {
    /// Сообщение в строку состояния.
    pub message: Option<String>,
    /// Картинка тайлов сменилась - картинки тайлов собираются заново.
    pub painters: bool,
    /// Следующий кадр не позже стольких миллисекунд: слой движется.
    pub tick: Option<u64>,
}

/// Описание слоя каталога - строка реестра.
pub trait Entry: Sync {
    /// Ключ в настройках, он же строка в сохранённых флажках.
    fn key(&self) -> &'static str;
    /// Название строки, текст для перевода.
    fn name(&self) -> &'static str;
    /// Подсказка строки, текст для перевода.
    fn tip(&self) -> &'static str;
    /// Раздел каталога «Добавить данные».
    fn category(&self) -> Category;
    /// Источник - вторая строка карточки каталога.
    fn source(&self) -> &'static str;
    /// Другие слова слоя для строки «Поиск», `WORDS` модуля.
    fn words(&self) -> &'static [&'static str] {
        &[]
    }
    /// Превью карточки каталога, без него - подложка.
    fn legend(&self) -> Option<Recipe> {
        None
    }
    /// Место строки: из строк с одним местом включена только одна, новая
    /// снимает прежнюю. None - строк сколько угодно.
    fn slot(&self) -> Option<u8> {
        None
    }
    /// Новое состояние слоя: флажок включён. Второе - сообщение в строку
    /// состояния.
    fn make(&self, cx: &Make<'_>) -> (Box<dyn Layer>, Option<String>);
}

/// Состояние включённого слоя.
pub trait Layer {
    /// Шаг кадра: загрузка, движение.
    fn step(&mut self, cx: &Ctx<'_>) -> Stepped;
    /// Картинка тайлов над картами, без неё - None.
    fn painter(&self) -> Option<Painter> {
        None
    }
    /// Рисунок поверх глобуса в кадре. view - широта, долгота точки
    /// взгляда и ширина вида в метрах.
    fn paint(&self, _painter: &egui::Painter, _rect: egui::Rect, _ppp: f64, _camera: &Camera, _view: (f64, f64, f64)) {}
    /// Шкала в панели шкал вида: заголовок и цвета.
    fn legend(&self, _lang: Lang) -> Option<(String, data::gibs::Colormap)> {
        None
    }
    /// Идёт загрузка: снимок окна ждёт.
    fn busy(&self) -> bool {
        false
    }
    /// Подпись источника внизу вида: название и адрес.
    fn attribution(&self) -> Option<(&'static str, &'static str)> {
        None
    }
    /// Группа окна «Объекты» у точки щелчка (широта, долгота).
    fn identify(&self, _lang: Lang, _lat: f64, _lon: f64) -> Option<(String, Vec<crate::identify::Feature>)> {
        None
    }
}

/// Слои реестра по порядку строк каталога. Новый слой - одна строка.
pub static ALL: &[&dyn Entry] = &[
    &crate::mining::ENTRY,
    &crate::currents::ENTRY,
    &crate::weather::ENTRIES[0],
    &crate::weather::ENTRIES[1],
    &crate::weather::ENTRIES[2],
    &crate::weather::ENTRIES[3],
    &crate::weather::ENTRIES[4],
];

/// Строка реестра номер n.
pub fn get(n: usize) -> Option<&'static dyn Entry> {
    ALL.get(n).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::Extra;

    #[test]
    fn registry_rows_are_catalog_rows_with_their_keys() {
        let keys: Vec<&str> = ALL.iter().map(|e| e.key()).collect();
        let mut unique = keys.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), keys.len(), "ключи реестра повторяются: {keys:?}");
        for (n, e) in ALL.iter().enumerate() {
            // Ключ настроек прежний: сохранённый флажок находит строку.
            assert_eq!(Extra::from_key(e.key()), Some(Extra::Plugged(n)));
            assert_eq!(Extra::Plugged(n).name(), e.name());
            // Название и подсказка переведены.
            let ru = Lang::from_code("ru").unwrap_or(Lang::EN);
            assert_ne!(crate::i18n::tr(ru, e.name()), e.name(), "нет перевода «{}»", e.name());
            assert_ne!(crate::i18n::tr(ru, e.tip()), e.tip(), "нет перевода подсказки «{}»", e.name());
            assert!(!e.source().is_empty());
        }
        // Ключи прежние: сохранённые флажки и сцены находят строки.
        for key in ["mining", "currents", "weather_temperature", "weather_wind", "weather_aurora"] {
            assert!(matches!(Extra::from_key(key), Some(Extra::Plugged(_))), "{key}");
        }
    }
}
