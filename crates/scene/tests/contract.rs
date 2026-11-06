// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Контракт сцены линейки PlanetX - хранилище `planetx-scene`.
//!
//! Каждая проверочная сцена из `scenes/` читается Lite, разбор
//! сравнивается с `expected/<сцена>.json`. Ожидаемый разбор снят ядром
//! модуля PlanetX (`tools/expected.py` хранилища): `scene` - словарь
//! `Scene.to_dict()`, `places` - дерево меток.
//!
//! Папка хранилища - `PLANETX_SCENE` или `planetx-scene` рядом с Lite.
//! Если `PLANETX_SCENE` задана, папка обязательна, так работает CI.
//! Без переменной и без папки тест сообщает об этом и проходит.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use scene::kml::{self, Folder, Item};
use scene::scene::read_scene;
use serde_json::{json, Value};

/// Дерево меток в том виде, в каком его пишет `tools/expected.py`.
#[derive(Default)]
struct Tree {
    folders: usize,
    places: usize,
    tours: usize,
    overlays: usize,
    place_kinds: BTreeMap<String, usize>,
    overlay_kinds: BTreeMap<String, usize>,
    names: Vec<String>,
}

impl Tree {
    fn walk(&mut self, folder: &Folder) {
        for item in &folder.children {
            match item {
                Item::Folder(f) => {
                    self.folders += 1;
                    self.walk(f);
                }
                Item::Place(p) => {
                    self.places += 1;
                    self.tours += usize::from(p.tour.is_some());
                    *self.place_kinds.entry(p.kind.as_str().to_string()).or_default() += 1;
                    self.names.push(p.name.clone());
                }
                Item::Overlay(o) => {
                    self.overlays += 1;
                    *self.overlay_kinds.entry(o.overlay.kind.as_str().to_string()).or_default() += 1;
                }
            }
        }
    }

    fn to_value(&self) -> Value {
        json!({
            "folders": self.folders,
            "places": self.places,
            "tours": self.tours,
            "overlays": self.overlays,
            "place_kinds": self.place_kinds,
            "overlay_kinds": self.overlay_kinds,
            "names": self.names,
        })
    }
}

/// Папка хранилища контракта или None, если её нет и она не требуется.
fn contract_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PLANETX_SCENE") {
        let dir = PathBuf::from(dir);
        assert!(dir.join("scenes").is_dir(), "PLANETX_SCENE задана, а папки {} нет", dir.join("scenes").display());
        return Some(dir);
    }
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("..").join("planetx-scene");
    if dir.join("scenes").is_dir() {
        Some(dir)
    } else {
        eprintln!("хранилища planetx-scene нет, проверка пропущена");
        None
    }
}

#[test]
fn scenes_read_like_contract() {
    let Some(dir) = contract_dir() else {
        return;
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir.join("scenes"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "planetx"))
        .collect();
    files.sort();
    assert!(!files.is_empty());
    let mut failed = Vec::new();
    for path in &files {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let expected_path = dir.join("expected").join(format!("{name}.json"));
        let text = std::fs::read_to_string(&expected_path).unwrap_or_else(|e| panic!("{name}: нет ожидаемого разбора: {e}"));
        let expected: Value = serde_json::from_str(&text).unwrap();
        let bytes = std::fs::read(path).unwrap();
        let (scene, places) = read_scene(&bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let mut tree = Tree::default();
        if !places.is_empty() {
            let root = kml::read_file(&places, &scene.places).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            tree.walk(&root);
        }
        if scene.to_dict() != expected["scene"] {
            failed.push(format!("{name}: scene.json\n  Lite   {}\n  модуль {}", scene.to_dict(), expected["scene"]));
        }
        if tree.to_value() != expected["places"] {
            failed.push(format!("{name}: метки\n  Lite   {}\n  модуль {}", tree.to_value(), expected["places"]));
        }
    }
    assert!(failed.is_empty(), "разбор расходится с контрактом:\n{}", failed.join("\n"));
    println!("сцен по контракту {}", files.len());
}
