// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Строка «Поиск», перенос `tests/test_searchbar.py` модуля без неба.

use scene::searchbar::{self as sb, Kind, Suggestion};

fn places() -> Vec<(String, String, f64, f64)> {
    [("point:1", "Вокзал Пермь-1", 58.02, 56.25), ("point:2", "Пермский университет", 58.0, 56.18), ("line:3", "Путь вдоль Камы", 58.05, 56.3), ("point:4", "Ёлка на эспланаде", 58.0, 56.22)]
        .iter()
        .map(|(k, n, a, b)| (k.to_string(), n.to_string(), *a, *b))
        .collect()
}

fn texts(found: &[Suggestion]) -> Vec<String> {
    found.iter().map(|s| s.text.clone()).collect()
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn order_prefix_word_inside() {
    assert_eq!(texts(&sb::suggestions("перм", &places(), &[], sb::LIMIT, &[])), ["Пермский университет", "Вокзал Пермь-1"]);
    assert_eq!(texts(&sb::suggestions("амы", &places(), &[], sb::LIMIT, &[])), ["Путь вдоль Камы"]);
}

#[test]
fn yo_and_case() {
    assert_eq!(sb::suggestions("ЕЛКА", &places(), &[], sb::LIMIT, &[])[0].key, "point:4");
}

#[test]
fn places_before_history_and_no_echo() {
    let history = strings(&["пермь", "Пермский край"]);
    let found = sb::suggestions("пермь", &places(), &history, sb::LIMIT, &[]);
    assert_eq!(found[0].kind, Kind::Place);
    // Запрос, равный введённому, в подсказки не идёт.
    assert!(!found.iter().any(|s| s.kind == Kind::History && s.text == "пермь"));
}

#[test]
fn empty_query_shows_history() {
    let found = sb::suggestions("", &places(), &strings(&["Казань", "Пермь"]), sb::LIMIT, &[]);
    assert_eq!(texts(&found), ["Казань", "Пермь"]);
    assert!(found.iter().all(|s| s.kind == Kind::History));
}

#[test]
fn limit() {
    let many: Vec<(String, String, f64, f64)> = (0..50).map(|n| (format!("point:{n}"), format!("Метка {n}"), 0.0, 0.0)).collect();
    assert_eq!(sb::suggestions("метка", &many, &[], sb::LIMIT, &[]).len(), sb::LIMIT);
}

#[test]
fn exact() {
    let items = sb::suggestions("вокзал пермь-1", &places(), &[], sb::LIMIT, &[]);
    assert_eq!(sb::exact("Вокзал  Пермь-1", &items)[0].key, "point:1");
    assert!(sb::exact("Вокзал", &items).is_empty());
}

#[test]
fn exact_takes_first_of_same_names() {
    let twins: Vec<(String, String, f64, f64)> =
        [("point:7", "Дом"), ("point:8", "дом"), ("point:9", "Домик")].iter().map(|(k, n)| (k.to_string(), n.to_string(), 0.0, 0.0)).collect();
    let found = sb::exact("дом", &sb::suggestions("дом", &twins, &[], sb::LIMIT, &[]));
    assert_eq!(found.iter().map(|s| s.key.as_str()).collect::<Vec<_>>(), ["point:7", "point:8"]);
}

#[test]
fn drop_repeats() {
    let history = strings(&["Вокзал Пермь-1", "вокзал Казань"]);
    let found = sb::suggestions("вокзал", &places(), &history, sb::LIMIT, &[]);
    let pairs = |f: &[Suggestion]| f.iter().map(|s| (s.kind, s.text.clone())).collect::<Vec<_>>();
    assert_eq!(
        pairs(&found),
        [(Kind::Place, "Вокзал Пермь-1".to_string()), (Kind::History, "вокзал Казань".to_string()), (Kind::History, "Вокзал Пермь-1".to_string())]
    );
    assert_eq!(pairs(&sb::drop_repeats(&found)), [(Kind::Place, "Вокзал Пермь-1".to_string()), (Kind::History, "вокзал Казань".to_string())]);
    let alone = sb::suggestions("вокзал", &[], &history, sb::LIMIT, &[]);
    assert_eq!(sb::drop_repeats(&alone), alone);
}

#[test]
fn actions_found_by_words() {
    let actions = [Suggestion::action("ruler", "Линейка", "Расстояние и площадь", &["ruler", "measure", "измерить"])];
    let found = sb::suggestions("измер", &[], &[], sb::LIMIT, &actions);
    assert_eq!(found[0].key, "ruler");
    assert_eq!(sb::exact("ruler", &found)[0].key, "ruler");
}

#[test]
fn remember_moves_to_front_and_limits() {
    assert_eq!(sb::remember(&strings(&["Казань", "Пермь"]), " пермь ", sb::HISTORY), ["пермь", "Казань"]);
    let h = sb::remember(&(0..40).map(|n| n.to_string()).collect::<Vec<_>>(), "новый", sb::HISTORY);
    assert_eq!(h.len(), sb::HISTORY);
    assert_eq!(h[0], "новый");
    assert_eq!(sb::remember(&strings(&["А"]), "  ", sb::HISTORY), ["А"]);
}
