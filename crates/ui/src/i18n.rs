// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Тексты для пользователя на языках интерфейса, как в RustGIS
//! (`crates/ui-egui/src/i18n`). Решение автора от 23 октября 2026 года -
//! языки RustGIS без персидского.
//!
//! В коде текст пишется по-английски через [`tr`] или [`trf`].
//! Переводы - каталоги `i18n/<код>.tsv` строками «английский текст,
//! табуляция, перевод». Текст без перевода показывается по-английски,
//! каталог можно дополнять по строке. Подстановки `{name}` в переводе
//! те же, что в английском тексте, их сверяет тест каталогов.
//!
//! Язык по умолчанию - язык системы, на странице - браузера, без
//! перевода на него - английский. `PLANETX_LANGUAGE=de` задаёт язык
//! на один запуск программы.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Язык интерфейса: код ISO 639-1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Lang(&'static str);

/// Языки интерфейса: код и название на самом языке.
pub const LANGUAGES: [(&str, &str); 16] = [
    ("en", "English"),
    ("ru", "Русский"),
    ("de", "Deutsch"),
    ("es", "Español"),
    ("fr", "Français"),
    ("it", "Italiano"),
    ("nl", "Nederlands"),
    ("pt", "Português"),
    ("tr", "Türkçe"),
    ("az", "Azərbaycanca"),
    ("id", "Bahasa Indonesia"),
    ("vi", "Tiếng Việt"),
    ("zh", "简体中文"),
    ("ja", "日本語"),
    ("ko", "한국어"),
    ("ka", "ქართული"),
];

impl Lang {
    pub const EN: Lang = Lang("en");
    pub const RU: Lang = Lang("ru");

    pub fn code(self) -> &'static str {
        self.0
    }

    /// Язык по коду или по началу метки языка: «de», «de-AT», «zh_CN».
    pub fn from_code(code: &str) -> Option<Lang> {
        let head = code.trim().split(['-', '_', '.']).next().unwrap_or("").to_lowercase();
        LANGUAGES.iter().find(|(c, _)| *c == head).map(|(c, _)| Lang(c))
    }

    /// Язык системы или браузера, без перевода на него - английский.
    pub fn from_system() -> Self {
        sys_locale::get_locale().as_deref().and_then(Lang::from_code).unwrap_or(Lang::EN)
    }

    /// Язык на один запуск из `PLANETX_LANGUAGE`.
    pub fn from_env() -> Option<Self> {
        std::env::var("PLANETX_LANGUAGE").ok().as_deref().and_then(Lang::from_code)
    }

    /// Название языка на нём самом.
    pub fn native_name(self) -> &'static str {
        LANGUAGES.iter().find(|(c, _)| *c == self.0).map_or(self.0, |(_, n)| n)
    }

    /// Нужен ли шрифт с иероглифами.
    pub fn needs_cjk(self) -> bool {
        matches!(self.0, "zh" | "ja" | "ko")
    }
}

/// Текст каталога языка code.
fn source(code: &str) -> &'static str {
    match code {
        "ru" => include_str!("../i18n/ru.tsv"),
        "de" => include_str!("../i18n/de.tsv"),
        "es" => include_str!("../i18n/es.tsv"),
        "fr" => include_str!("../i18n/fr.tsv"),
        "it" => include_str!("../i18n/it.tsv"),
        "nl" => include_str!("../i18n/nl.tsv"),
        "pt" => include_str!("../i18n/pt.tsv"),
        "tr" => include_str!("../i18n/tr.tsv"),
        "az" => include_str!("../i18n/az.tsv"),
        "id" => include_str!("../i18n/id.tsv"),
        "vi" => include_str!("../i18n/vi.tsv"),
        "zh" => include_str!("../i18n/zh.tsv"),
        "ja" => include_str!("../i18n/ja.tsv"),
        "ko" => include_str!("../i18n/ko.tsv"),
        "ka" => include_str!("../i18n/ka.tsv"),
        _ => "",
    }
}

/// Строки каталога: пропуская пустые и комментарии «#». `\n` в тексте -
/// перевод строки.
pub fn parse(text: &'static str) -> HashMap<&'static str, String> {
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .map(|(en, tr)| (en, tr.trim_end_matches('\r').replace("\\n", "\n")))
        .collect()
}

/// Каталог языка, разобранный один раз.
fn catalog(lang: Lang) -> &'static HashMap<&'static str, String> {
    static CATALOGS: OnceLock<HashMap<&'static str, HashMap<&'static str, String>>> = OnceLock::new();
    static EMPTY: OnceLock<HashMap<&'static str, String>> = OnceLock::new();
    let all = CATALOGS.get_or_init(|| LANGUAGES.iter().map(|(c, _)| (*c, parse(source(c)))).collect());
    all.get(lang.0).unwrap_or_else(|| EMPTY.get_or_init(HashMap::new))
}

/// Текст en на языке lang.
pub fn tr(lang: Lang, en: &'static str) -> &'static str {
    match catalog(lang).get(en) {
        Some(t) => t.as_str(),
        None => en,
    }
}

/// Текст en на языке lang с подстановками `{имя}`.
pub fn trf(lang: Lang, en: &'static str, args: &[(&str, &str)]) -> String {
    let mut out = tr(lang, en).to_string();
    for (key, value) in args {
        out = out.replace(&format!("{{{key}}}"), value);
    }
    out
}

/// Сообщение под строкой поиска.
pub fn search_status(lang: Lang, status: &crate::search::Status) -> Option<String> {
    use crate::search::Status;
    Some(match status {
        Status::None => return None,
        Status::Searching(q) => trf(lang, "Searching: {q}", &[("q", q)]),
        Status::NotFound(q) => trf(lang, "Nothing found: {q}", &[("q", q)]),
        Status::Failed(e) => trf(lang, "Search failed: {e}", &[("e", e)]),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{LANGUAGES, Lang, parse, source, tr, trf};

    /// Подстановки `{…}` текста.
    fn holes(text: &str) -> Vec<String> {
        let mut out: Vec<String> = text.split('{').skip(1).filter_map(|s| s.split_once('}').map(|(k, _)| k.to_string())).collect();
        out.sort();
        out
    }

    /// Каждый каталог - строки «текст, табуляция, перевод», без
    /// повторов, с теми же подстановками, что в английском тексте,
    /// и только с текстами, которые есть в русском каталоге.
    #[test]
    fn catalogs_are_well_formed() {
        let ru = parse(source("ru"));
        assert!(ru.len() > 60, "русский каталог неполон: {}", ru.len());
        for (code, _) in LANGUAGES.iter().skip(1) {
            let text = source(code);
            assert!(!text.is_empty(), "{code}: нет каталога");
            let mut seen = std::collections::HashSet::new();
            for line in text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')) {
                let (en, tr) = line.split_once('\t').unwrap_or_else(|| panic!("{code}: нет табуляции: {line}"));
                assert!(seen.insert(en), "{code}: повтор {en}");
                assert!(!tr.trim().is_empty(), "{code}: пустой перевод {en}");
                assert_eq!(holes(en), holes(tr), "{code}: подстановки {en}");
                assert!(ru.contains_key(en), "{code}: текста нет в русском каталоге: {en}");
            }
        }
    }

    #[test]
    fn missing_text_stays_english() {
        assert_eq!(tr(Lang::RU, "Places"), "Метки");
        assert_eq!(tr(Lang::RU, "no such text"), "no such text");
        assert_eq!(tr(Lang::EN, "Places"), "Places");
        assert_eq!(trf(Lang::RU, "Saved: {to}", &[("to", "a.kml")]), "Сохранено: a.kml");
    }

    #[test]
    fn language_from_locale() {
        assert_eq!(Lang::from_code("de-AT"), Some(Lang("de")));
        assert_eq!(Lang::from_code("zh_CN.UTF-8"), Some(Lang("zh")));
        assert_eq!(Lang::from_code("fa"), None);
        assert!(Lang::from_code("ja").unwrap().needs_cjk());
    }
}
