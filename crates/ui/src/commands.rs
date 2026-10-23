// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Действия программы для строки «Поиск» и Ctrl+K, перенос
//! `ui/commands.py` модуля.
//!
//! Инструменты, окна, переключатели вида, демо и основы находятся по
//! названию или по другому слову и выполняются из подсказки или по
//! Enter. Подсказка - `scene::searchbar::Suggestion` вида Action, ключ -
//! [`Command`]. Слова не показываются и не переводятся: в них русские
//! и английские названия и близкие слова, как у модуля.

use scene::searchbar::Suggestion;

use crate::basemap::Basemap;
use crate::i18n::{Lang, tr};

/// Действие строки «Поиск».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Ruler,
    Placemark,
    Sun,
    Grid,
    Snapshot,
    Record,
    Share,
    Settings,
    Dark,
    Light,
    Open,
    About,
    Manual,
    Sources,
    Fullscreen,
    /// Демо: раздел и номер в разделе `crate::demos::MENU`.
    Demo(usize, usize),
    Basemap(Basemap),
}

impl Command {
    /// Ключ подсказки.
    pub fn key(self) -> String {
        match self {
            Self::Demo(s, d) => format!("demo:{s}:{d}"),
            Self::Basemap(b) => format!("basemap:{}", b.key()),
            other => format!("{other:?}").to_lowercase(),
        }
    }

    /// Действие по ключу подсказки.
    pub fn from_key(key: &str) -> Option<Self> {
        all_commands().into_iter().find(|c| c.key() == key)
    }
}

fn all_commands() -> Vec<Command> {
    let mut out = vec![
        Command::Ruler,
        Command::Placemark,
        Command::Sun,
        Command::Grid,
        Command::Snapshot,
        Command::Record,
        Command::Share,
        Command::Settings,
        Command::Dark,
        Command::Light,
        Command::Open,
        Command::About,
        Command::Manual,
        Command::Sources,
        Command::Fullscreen,
    ];
    for (s, section) in crate::demos::MENU.iter().enumerate() {
        out.extend((0..section.demos.len()).map(|d| Command::Demo(s, d)));
    }
    out.extend(Basemap::ALL.iter().map(|b| Command::Basemap(*b)));
    out
}

/// Подсказки всех действий на языке интерфейса. sun, grid - включены ли
/// они сейчас: «включить» или «выключить» зависит от вида.
pub fn actions(lang: Lang, sun: bool, grid: bool) -> Vec<Suggestion> {
    let on_off = |on: bool| if on { tr(lang, "Turn off") } else { tr(lang, "Turn on") };
    all_commands()
        .into_iter()
        .map(|c| {
            let (text, note, words): (String, String, &[&str]) = match c {
                Command::Ruler => (
                    tr(lang, "Ruler").into(),
                    tr(lang, "Length, area, ground length").into(),
                    &["линейка", "измерить", "расстояние", "площадь", "ruler", "measure", "distance", "area"],
                ),
                Command::Placemark => (
                    tr(lang, "New placemark").into(),
                    tr(lang, "Placemark, path, polygon or circle").into(),
                    &["метка", "добавить", "нарисовать", "путь", "многоугольник", "placemark", "add", "draw", "path", "polygon"],
                ),
                Command::Sun => (tr(lang, "Sun").into(), on_off(sun).into(), &["солнце", "ночь", "день", "освещение", "sun", "night", "daylight"]),
                Command::Grid => {
                    (tr(lang, "Coordinate grid").into(), on_off(grid).into(), &["сетка", "параллели", "меридианы", "grid", "graticule"])
                }
                Command::Snapshot => (
                    tr(lang, "Snapshot of the view").into(),
                    tr(lang, "PNG file").into(),
                    &["снимок", "картинка", "скриншот", "png", "snapshot", "screenshot"],
                ),
                Command::Record => (tr(lang, "Record a tour").into(), String::new(), &["запись", "записать", "тур", "record", "tour"]),
                Command::Share => (tr(lang, "Share").into(), tr(lang, "Link to this view").into(), &["ссылка", "поделиться", "share", "link"]),
                Command::Settings => (tr(lang, "Settings").into(), String::new(), &["настройки", "язык", "параметры", "settings", "language", "options"]),
                Command::Dark => (tr(lang, "Dark").into(), tr(lang, "Appearance").into(), &["тёмная тема", "темная", "dark theme"]),
                Command::Light => (tr(lang, "Light").into(), tr(lang, "Appearance").into(), &["светлая тема", "light theme"]),
                Command::Open => (tr(lang, "Open…").into(), String::new(), &["открыть", "файл", "kml", "kmz", "сцена", "open", "file", "scene"]),
                Command::About => (tr(lang, "About").into(), String::new(), &["о программе", "версия", "about", "version"]),
                Command::Manual => (tr(lang, "User guide").into(), String::new(), &["руководство", "справка", "помощь", "manual", "help", "guide"]),
                Command::Sources => (tr(lang, "Data sources").into(), String::new(), &["источники", "данные", "лицензии", "sources", "credits"]),
                Command::Fullscreen => (tr(lang, "Full screen").into(), String::new(), &["весь экран", "полный экран", "fullscreen", "full screen"]),
                Command::Demo(s, d) => {
                    let section = &crate::demos::MENU[s];
                    (tr(lang, section.demos[d].en).into(), tr(lang, section.en).into(), &["демо", "пример", "demo", "example"])
                }
                Command::Basemap(b) => (b.name(lang).to_string(), tr(lang, "Basemap").into(), &["основа", "подложка", "карта", "basemap", "map"]),
            };
            Suggestion::action(&c.key(), &text, &note, words)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{Command, actions};
    use crate::i18n::Lang;
    use scene::searchbar as sb;

    #[test]
    fn keys_round_trip_and_words_find() {
        for s in actions(Lang::EN, false, false) {
            assert!(Command::from_key(&s.key).is_some(), "{}", s.key);
        }
        let found = sb::suggestions("измерить", &[], &[], sb::LIMIT, &actions(Lang::RU, false, false));
        assert_eq!(Command::from_key(&found[0].key), Some(Command::Ruler));
        let found = sb::suggestions("самосвалы", &[], &[], sb::LIMIT, &actions(Lang::RU, false, false));
        assert!(found.iter().any(|s| matches!(Command::from_key(&s.key), Some(Command::Demo(..)))));
    }
}
