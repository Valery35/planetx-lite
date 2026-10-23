// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Строка «Поиск»: подсказки при вводе, точное совпадение, история.
//! Перенос `core/searchbar.py` модуля без звёзд и созвездий: неба в Lite
//! нет.
//!
//! Подсказки берутся из своих источников - «Моих меток», истории
//! запросов и действий программы. Nominatim при вводе не спрашивается:
//! правила службы запрещают автодополнение, запрос к ней уходит только
//! по Enter. Порядок - сначала совпадение с началом названия, потом
//! с началом слова, потом внутри слова. При равенстве - метки
//! и действия, история, короче выше.

/// Вид подсказки.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// Своя метка, key - её ключ.
    Place,
    /// Действие программы, key - его ключ, words - другие слова.
    Action,
    /// Прежний запрос.
    History,
}

impl Kind {
    fn order(self) -> u8 {
        match self {
            Self::Place | Self::Action => 0,
            Self::History => 2,
        }
    }
}

/// Подсказка строки поиска.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub kind: Kind,
    pub text: String,
    pub key: String,
    pub lat: f64,
    pub lon: f64,
    /// Что сделает действие.
    pub note: String,
    /// Другие слова действия, по ним оно находится.
    pub words: Vec<String>,
}

impl Suggestion {
    pub fn history(text: &str) -> Self {
        Self { kind: Kind::History, text: text.to_string(), key: String::new(), lat: 0.0, lon: 0.0, note: String::new(), words: Vec::new() }
    }

    pub fn action(key: &str, text: &str, note: &str, words: &[&str]) -> Self {
        Self {
            kind: Kind::Action,
            text: text.to_string(),
            key: key.to_string(),
            lat: 0.0,
            lon: 0.0,
            note: note.to_string(),
            words: words.iter().map(|w| w.to_string()).collect(),
        }
    }
}

/// Подсказок в списке.
pub const LIMIT: usize = 12;
/// Запросов в истории.
pub const HISTORY: usize = 30;

/// Строка для сравнения: нижний регистр, ё как е, одиночные пробелы.
pub fn norm(text: &str) -> String {
    text.to_lowercase().replace('ё', "е").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Насколько name подходит к query: 0 - начало названия, 1 - начало
/// слова, 2 - внутри, None - не подходит. query уже в norm.
pub fn rank(query: &str, name: &str) -> Option<u8> {
    let target = norm(name);
    if query.is_empty() || target.is_empty() {
        return None;
    }
    if target.starts_with(query) {
        Some(0)
    } else if format!(" {target}").contains(&format!(" {query}")) {
        Some(1)
    } else if target.contains(query) {
        Some(2)
    } else {
        None
    }
}

/// Насколько действие подходит к query: лучшее из названия и слов, у
/// слов на ступень ниже названия.
pub fn action_rank(query: &str, item: &Suggestion) -> Option<u8> {
    let words = item.words.iter().filter_map(|w| rank(query, w).map(|r| r + 1));
    rank(query, &item.text).into_iter().chain(words).min()
}

/// Подсказки к query. places - (ключ, название, широта, долгота),
/// history - прежние запросы от новых к старым, actions - действия.
/// Пустой query - последние запросы истории.
pub fn suggestions(query: &str, places: &[(String, String, f64, f64)], history: &[String], limit: usize, actions: &[Suggestion]) -> Vec<Suggestion> {
    let q = norm(query);
    if q.is_empty() {
        return history.iter().take(limit).map(|h| Suggestion::history(h)).collect();
    }
    let mut found: Vec<(u8, Suggestion)> = Vec::new();
    for item in actions {
        if let Some(r) = action_rank(&q, item) {
            found.push((r, item.clone()));
        }
    }
    for (key, name, lat, lon) in places {
        if let Some(r) = rank(&q, name) {
            found.push((r, Suggestion { kind: Kind::Place, text: name.clone(), key: key.clone(), lat: *lat, lon: *lon, ..Suggestion::history("") }));
        }
    }
    for text in history {
        if let Some(r) = rank(&q, text).filter(|_| norm(text) != q) {
            found.push((r, Suggestion::history(text)));
        }
    }
    found.sort_by_key(|(r, s)| (*r, s.kind.order(), s.text.chars().count()));
    let mut out: Vec<Suggestion> = Vec::new();
    let mut seen: Vec<(Kind, String, String)> = Vec::new();
    for (_, s) in found {
        let mark = (s.kind, norm(&s.text), s.key.clone());
        if seen.contains(&mark) {
            continue;
        }
        seen.push(mark);
        out.push(s);
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// Подсказки, чьё название совпадает с query целиком. У действия
/// совпадение и с любым словом.
pub fn exact(query: &str, items: &[Suggestion]) -> Vec<Suggestion> {
    let q = norm(query);
    items
        .iter()
        .filter(|s| !q.is_empty() && (norm(&s.text) == q || (s.kind == Kind::Action && s.words.iter().any(|w| norm(w) == q))))
        .cloned()
        .collect()
}

/// Подсказки без прежних запросов, которые повторяют название метки из
/// тех же подсказок.
pub fn drop_repeats(items: &[Suggestion]) -> Vec<Suggestion> {
    let named: Vec<String> = items.iter().filter(|s| s.kind != Kind::History).map(|s| norm(&s.text)).collect();
    items.iter().filter(|s| s.kind != Kind::History || !named.contains(&norm(&s.text))).cloned().collect()
}

/// История с query в начале, без повторов, не длиннее size.
pub fn remember(history: &[String], query: &str, size: usize) -> Vec<String> {
    let q = norm(query);
    if q.is_empty() {
        return history.to_vec();
    }
    std::iter::once(query.trim().to_string()).chain(history.iter().filter(|h| norm(h) != q).cloned()).take(size).collect()
}
