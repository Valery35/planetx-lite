// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Дерево меток: папки и метки, порядок, перенос, обход. Перенос
//! `core/placetree.py` модуля.
//!
//! Узел - метка или папка с ключом, ключом родительской папки (None -
//! корень), номером среди соседей и названием. Метки и папки одного
//! родителя нумеруются вместе, как строки списка. Узлы без номера стоят
//! после нумерованных по названию. Папки как в Google Earth: любая
//! вложенность, перенос, тур по папке, удаление вместе с содержимым.

use std::collections::{BTreeMap, HashMap, HashSet};

/// Вид ключа папки: «folder:<номер>».
pub const FOLDER: &str = "folder";

pub fn is_folder(key: &str) -> bool {
    key.strip_prefix(FOLDER).is_some_and(|rest| rest.starts_with(':'))
}

/// Узел дерева: ключ, родитель, номер среди соседей, название.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub key: String,
    pub parent: Option<String>,
    pub position: Option<i64>,
    pub name: String,
}

impl Node {
    pub fn new(key: &str, parent: Option<&str>, position: Option<i64>, name: &str) -> Self {
        Self { key: key.into(), parent: parent.map(Into::into), position, name: name.into() }
    }

    fn order(&self) -> (bool, i64, String) {
        (self.position.is_none(), self.position.unwrap_or(0), self.name.to_lowercase())
    }
}

/// Перенос: ключ - новый родитель и номер.
pub type Plan = BTreeMap<String, (Option<String>, i64)>;

/// Дети папки parent по порядку списка.
pub fn children<'a>(nodes: &'a [Node], parent: Option<&str>) -> Vec<&'a Node> {
    let mut out: Vec<&Node> = nodes.iter().filter(|n| n.parent.as_deref() == parent).collect();
    out.sort_by_key(|n| n.order());
    out
}

/// Дети каждой папки по порядку списка, один проход по узлам.
pub fn index(nodes: &[Node]) -> HashMap<Option<String>, Vec<&Node>> {
    let mut out: HashMap<Option<String>, Vec<&Node>> = HashMap::new();
    for node in nodes {
        out.entry(node.parent.clone()).or_default().push(node);
    }
    for kids in out.values_mut() {
        kids.sort_by_key(|n| n.order());
    }
    out
}

/// Все узлы под parent в порядке списка, папка раньше своих детей.
pub fn walk<'a>(nodes: &'a [Node], parent: Option<&str>) -> Vec<&'a Node> {
    let kids = index(nodes);
    walk_index(&kids, parent)
}

fn walk_index<'a>(kids: &HashMap<Option<String>, Vec<&'a Node>>, parent: Option<&str>) -> Vec<&'a Node> {
    let mut out = Vec::new();
    let mut stack: Vec<&Node> = kids.get(&parent.map(String::from)).into_iter().flatten().rev().copied().collect();
    while let Some(node) = stack.pop() {
        out.push(node);
        if is_folder(&node.key) {
            stack.extend(kids.get(&Some(node.key.clone())).into_iter().flatten().rev().copied());
        }
    }
    out
}

/// Ключи всех узлов внутри папки folder, на любой глубине.
pub fn descendants(nodes: &[Node], folder: &str) -> Vec<String> {
    walk(nodes, Some(folder)).into_iter().map(|n| n.key.clone()).collect()
}

/// Выбранные ключи без тех, что лежат внутри выбранной папки, в порядке
/// списка.
pub fn top_keys(nodes: &[Node], keys: &[&str]) -> Vec<String> {
    let chosen: HashSet<&str> = keys.iter().copied().collect();
    let kids = index(nodes);
    let mut inside: HashSet<String> = HashSet::new();
    for key in &chosen {
        if is_folder(key) {
            inside.extend(walk_index(&kids, Some(key)).into_iter().map(|n| n.key.clone()));
        }
    }
    walk_index(&kids, None)
        .into_iter()
        .filter(|n| chosen.contains(n.key.as_str()) && !inside.contains(&n.key))
        .map(|n| n.key.clone())
        .collect()
}

/// Новые (родитель, номер) узлов после переноса key в parent перед его
/// ребёнком номер index. Пустой план - переноса нет. Папку нельзя
/// перенести в неё саму и в её потомков.
pub fn move_plan(nodes: &[Node], key: &str, parent: Option<&str>, index: usize) -> Plan {
    move_many_plan(nodes, &[key], parent, index)
}

/// То же для нескольких выбранных узлов. Они встают подряд в порядке
/// списка, вложенные в выбранную папку едут вместе с ней.
pub fn move_many_plan(nodes: &[Node], keys: &[&str], parent: Option<&str>, index: usize) -> Plan {
    let by_key: HashMap<&str, &Node> = nodes.iter().map(|n| (n.key.as_str(), n)).collect();
    if parent.is_some_and(|p| !by_key.contains_key(p)) {
        return Plan::new();
    }
    let present: Vec<&str> = keys.iter().copied().filter(|k| by_key.contains_key(k)).collect();
    let keys = top_keys(nodes, &present);
    if keys.is_empty() {
        return Plan::new();
    }
    for key in &keys {
        if is_folder(key) && parent.is_some_and(|p| p == key || descendants(nodes, key).iter().any(|d| d == p)) {
            return Plan::new();
        }
    }
    let moved: Vec<&Node> = keys.iter().filter_map(|k| by_key.get(k.as_str()).copied()).collect();
    let is_moved = |n: &Node| moved.iter().any(|m| m.key == n.key);
    let siblings = children(nodes, parent);
    let mut at = index.min(siblings.len());
    at -= siblings[..at].iter().filter(|n| is_moved(n)).count();
    let mut order: Vec<&Node> = siblings.into_iter().filter(|n| !is_moved(n)).collect();
    for (i, m) in moved.iter().enumerate() {
        order.insert(at + i, m);
    }
    let mut plan = Plan::new();
    for (n, item) in order.iter().enumerate() {
        let n = n as i64;
        if item.parent.as_deref() != parent || item.position != Some(n) {
            plan.insert(item.key.clone(), (parent.map(String::from), n));
        }
    }
    plan
}

/// Номера детей папки parent по названию от А до Я, папки и метки
/// вместе, без учёта регистра.
pub fn sort_plan(nodes: &[Node], parent: Option<&str>) -> BTreeMap<String, i64> {
    let mut kids = children(nodes, parent);
    kids.sort_by(|a, b| (a.name.to_lowercase(), &a.key).cmp(&(b.name.to_lowercase(), &b.key)));
    kids.into_iter().enumerate().map(|(i, n)| (n.key.clone(), i as i64)).collect()
}

/// Флажки с учётом папок-переключателей radio: в такой папке включён
/// не больше чем один ребёнок. Включили одного - соседи гаснут, включили
/// сразу несколько - остаётся первый по списку. Выбранный ребёнок
/// щелчком по нему не гаснет. Снятый флажок папки гасит всех детей,
/// поставленный включает первого, если ни один не виден. states -
/// ключ - включён, shown - ключи, видимые до щелчка.
pub fn radio_states(
    nodes: &[Node],
    radio: &HashSet<String>,
    states: &BTreeMap<String, bool>,
    shown: &HashSet<String>,
) -> BTreeMap<String, bool> {
    let mut out = states.clone();
    for folder in radio {
        let kids: Vec<String> = children(nodes, Some(folder)).into_iter().map(|n| n.key.clone()).collect();
        if let Some(&on) = states.get(folder)
            && !kids.iter().any(|k| states.contains_key(k))
        {
            if !on {
                for k in &kids {
                    out.insert(k.clone(), false);
                }
            } else if let Some(first) = kids.first()
                && !kids.iter().any(|k| shown.contains(k))
            {
                out.insert(first.clone(), true);
            }
            continue;
        }
        let lit: Vec<&String> = kids.iter().filter(|k| states.get(*k) == Some(&true)).collect();
        let Some(keep) = lit.first() else {
            if !states.contains_key(folder) {
                for key in kids.iter().filter(|k| states.contains_key(*k)) {
                    out.insert(key.clone(), true);
                }
            }
            continue;
        };
        let keep = (*keep).clone();
        for key in &kids {
            out.insert(key.clone(), *key == keep);
        }
    }
    out
}

/// Номер для нового узла в конце папки parent.
pub fn next_position(nodes: &[Node], parent: Option<&str>) -> i64 {
    let kids = children(nodes, parent);
    let last = kids.len() as i64 - 1;
    kids.iter().filter_map(|n| n.position).chain([last]).max().unwrap_or(last) + 1
}

/// Название «base N» для новой метки, как «Моя метка 3». N - следующий
/// номер после наибольшего среди названий вида «base число».
pub fn numbered_name<S: AsRef<str>>(base: &str, names: &[S]) -> String {
    let prefix = format!("{base} ");
    let top = names
        .iter()
        .filter_map(|n| n.as_ref().strip_prefix(&prefix))
        .filter(|tail| !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()))
        .filter_map(|tail| tail.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    format!("{prefix}{}", top + 1)
}
