// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Дерево меток: порядок, перенос, обход. Перенос
//! `tests/test_placetree.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, HashSet};

use scene::placetree::{self as pt, Node, Plan};

/// Корень: a, F1 (b, F2 (c)), d.
fn tree() -> Vec<Node> {
    vec![
        Node::new("point:1", None, Some(0), "a"),
        Node::new("folder:1", None, Some(1), "F1"),
        Node::new("line:2", Some("folder:1"), Some(0), "b"),
        Node::new("folder:2", Some("folder:1"), Some(1), "F2"),
        Node::new("point:3", Some("folder:2"), Some(0), "c"),
        Node::new("polygon:4", None, Some(2), "d"),
    ]
}

fn keys(nodes: &[&Node]) -> Vec<String> {
    nodes.iter().map(|n| n.key.clone()).collect()
}

fn apply(nodes: &mut [Node], plan: &Plan) {
    for n in nodes.iter_mut() {
        if let Some((parent, pos)) = plan.get(&n.key) {
            n.parent = parent.clone();
            n.position = Some(*pos);
        }
    }
}

fn states(items: &[(&str, bool)]) -> BTreeMap<String, bool> {
    items.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn set(items: &[&str]) -> HashSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn walk_depth_first() {
    let t = tree();
    assert_eq!(keys(&pt::walk(&t, None)), ["point:1", "folder:1", "line:2", "folder:2", "point:3", "polygon:4"]);
    assert_eq!(pt::descendants(&t, "folder:1"), ["line:2", "folder:2", "point:3"]);
}

#[test]
fn unnumbered_after_numbered_by_name() {
    let nodes = vec![
        Node::new("point:9", None, None, "Б"),
        Node::new("point:8", None, None, "А"),
        Node::new("point:7", None, Some(5), "Я"),
    ];
    assert_eq!(keys(&pt::children(&nodes, None)), ["point:7", "point:8", "point:9"]);
    assert_eq!(pt::next_position(&nodes, None), 6);
    assert_eq!(pt::next_position(&[], None), 0);
}

#[test]
fn move_down_within_parent() {
    let mut nodes = tree();
    let plan = pt::move_plan(&nodes, "point:1", None, 3);
    apply(&mut nodes, &plan);
    assert_eq!(keys(&pt::children(&nodes, None)), ["folder:1", "polygon:4", "point:1"]);
}

#[test]
fn move_into_folder_and_out() {
    let mut nodes = tree();
    let plan = pt::move_plan(&nodes, "polygon:4", Some("folder:2"), 0);
    apply(&mut nodes, &plan);
    assert_eq!(keys(&pt::children(&nodes, Some("folder:2"))), ["polygon:4", "point:3"]);
    let plan = pt::move_plan(&nodes, "point:3", None, 0);
    apply(&mut nodes, &plan);
    assert_eq!(keys(&pt::children(&nodes, None)), ["point:3", "point:1", "folder:1"]);
}

#[test]
fn folder_not_into_itself_or_descendant() {
    let nodes = tree();
    assert!(pt::move_plan(&nodes, "folder:1", Some("folder:1"), 0).is_empty());
    assert!(pt::move_plan(&nodes, "folder:1", Some("folder:2"), 0).is_empty());
    let plan = pt::move_plan(&nodes, "folder:2", None, 0);
    assert_eq!(plan["folder:2"], (None, 0));
}

#[test]
fn same_place_is_no_change() {
    assert!(pt::move_plan(&tree(), "folder:1", None, 1).is_empty());
    assert!(pt::move_plan(&tree(), "folder:1", None, 2).is_empty());
}

#[test]
fn top_keys_drop_inside_of_chosen_folder() {
    assert_eq!(pt::top_keys(&tree(), &["point:3", "folder:1", "polygon:4"]), ["folder:1", "polygon:4"]);
}

#[test]
fn chosen_go_together_in_list_order() {
    let mut nodes = tree();
    let plan = pt::move_many_plan(&nodes, &["polygon:4", "point:1"], Some("folder:2"), 1);
    apply(&mut nodes, &plan);
    assert_eq!(keys(&pt::children(&nodes, Some("folder:2"))), ["point:3", "point:1", "polygon:4"]);
    assert_eq!(keys(&pt::children(&nodes, None)), ["folder:1"]);
}

#[test]
fn index_counts_moved_siblings() {
    let mut nodes = tree();
    // a и d в конец корня: перед ними остаётся одна F1.
    let plan = pt::move_many_plan(&nodes, &["point:1", "polygon:4"], None, 3);
    apply(&mut nodes, &plan);
    assert_eq!(keys(&pt::children(&nodes, None)), ["folder:1", "point:1", "polygon:4"]);
}

#[test]
fn folder_with_its_child_not_into_itself() {
    assert!(pt::move_many_plan(&tree(), &["folder:1", "point:1"], Some("folder:2"), 0).is_empty());
}

#[test]
fn numbered_first_and_next() {
    assert_eq!(pt::numbered_name::<&str>("Моя метка", &[]), "Моя метка 1");
    assert_eq!(pt::numbered_name("Моя метка", &["Моя метка 1", "Моя метка 4", "Пермь"]), "Моя метка 5");
}

#[test]
fn other_names_do_not_count() {
    let names = ["Моя метка", "Моя метка 2a", "Мой путь 7", "Моя метка -3"];
    assert_eq!(pt::numbered_name("Моя метка", &names), "Моя метка 1");
}

#[test]
fn sort_plan() {
    let nodes = vec![
        Node::new("point:1", None, Some(0), "в"),
        Node::new("folder:1", None, Some(1), "А"),
        Node::new("line:2", None, Some(2), "б"),
    ];
    let expected: BTreeMap<String, i64> =
        [("folder:1", 0), ("line:2", 1), ("point:1", 2)].into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    assert_eq!(pt::sort_plan(&nodes, None), expected);
}

#[test]
fn radio_one_turned_on_switches_siblings_off() {
    let out = pt::radio_states(&tree(), &set(&["folder:1"]), &states(&[("folder:2", true)]), &set(&[]));
    assert_eq!(out, states(&[("folder:2", true), ("line:2", false)]));
}

#[test]
fn radio_folder_checked_keeps_first() {
    let out = pt::radio_states(
        &tree(),
        &set(&["folder:1"]),
        &states(&[("line:2", true), ("folder:2", true), ("point:3", true)]),
        &set(&[]),
    );
    assert!(out["line:2"]);
    assert!(!out["folder:2"]);
}

#[test]
fn radio_choice_is_not_cleared_by_its_own_click() {
    let out = pt::radio_states(&tree(), &set(&["folder:1"]), &states(&[("line:2", false)]), &set(&[]));
    assert_eq!(out, states(&[("line:2", true)]));
    // Флажок самой папки гасит всё.
    let out = pt::radio_states(
        &tree(),
        &set(&["folder:1"]),
        &states(&[("folder:1", false), ("line:2", false), ("folder:2", false)]),
        &set(&[]),
    );
    assert!(!out.values().any(|v| *v));
}

#[test]
fn radio_folder_unchecked_hides_children() {
    let out = pt::radio_states(&tree(), &set(&["folder:1"]), &states(&[("folder:1", false)]), &set(&["line:2"]));
    assert_eq!(out, states(&[("folder:1", false), ("line:2", false), ("folder:2", false)]));
}

#[test]
fn radio_folder_checked_shows_first_once() {
    let out = pt::radio_states(&tree(), &set(&["folder:1"]), &states(&[("folder:1", true)]), &set(&[]));
    assert_eq!(out, states(&[("folder:1", true), ("line:2", true)]));
    // Уже виден второй - отметка папки ничего не меняет.
    let out = pt::radio_states(&tree(), &set(&["folder:1"]), &states(&[("folder:1", true)]), &set(&["folder:2"]));
    assert_eq!(out, states(&[("folder:1", true)]));
}

#[test]
fn plain_folder_untouched() {
    let s = states(&[("line:2", true), ("folder:2", true)]);
    assert_eq!(pt::radio_states(&tree(), &set(&[]), &s, &set(&[])), s);
}
