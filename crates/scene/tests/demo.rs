// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сцены демо модуля - проверочный набор Lite. Каждая сцена читается,
//! количество папок, меток, туров и наложений совпадает с модулем.
//! Эталон снят модулем в QGIS 4.0.3 22 октября 2026 года: `read_scene`
//! и `kml.read_file` модуля на тех же файлах.
//!
//! Папка демо - `PLANETX_DEMO` или `C:\Dev\planetx\planetx\demo`. Без неё
//! тест сообщает об этом и проходит: на машине CI модуля нет.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use scene::kml::{self, Folder, Item};
use scene::scene::read_scene;

#[derive(Debug, Default, PartialEq, Eq)]
struct Counts {
    folders: usize,
    places: usize,
    tours: usize,
    overlays: usize,
}

fn count(folder: &Folder, out: &mut Counts) {
    for item in &folder.children {
        match item {
            Item::Folder(f) => {
                out.folders += 1;
                count(f, out);
            }
            Item::Place(p) => {
                out.places += 1;
                out.tours += usize::from(p.tour.is_some());
            }
            Item::Overlay(_) => out.overlays += 1,
        }
    }
}

/// Эталон модуля: сцена, папки, метки, туры, наложения.
const MODULE: [(&str, usize, usize, usize, usize); 22] = [
    ("aral", 1, 0, 0, 5),
    ("bocachica", 0, 7, 1, 0),
    ("brienz", 0, 2, 0, 0),
    ("flows", 0, 7, 0, 0),
    ("haul_kachkanar", 0, 2, 0, 6),
    ("haul_lebedinsky", 0, 2, 0, 6),
    ("haulqgis_kachkanar", 0, 1, 0, 0),
    ("haulqgis_lebedinsky", 0, 1, 0, 0),
    ("japan", 0, 4, 0, 0),
    ("jezero", 0, 9, 1, 0),
    ("mars", 0, 7, 0, 0),
    ("models", 0, 0, 0, 2),
    ("moon", 0, 8, 0, 0),
    ("moscow", 0, 2, 0, 0),
    ("perm", 2, 12, 1, 0),
    ("quarry", 0, 3, 0, 0),
    ("sky", 0, 7, 0, 0),
    ("subsurface", 0, 5, 0, 0),
    ("superoverlay", 1, 0, 0, 0),
    ("trains", 0, 3, 0, 0),
    ("turbines", 0, 0, 0, 3),
    ("vegas", 0, 8, 0, 0),
];

#[test]
fn demo_scenes_read_like_module() {
    let dir = std::env::var_os("PLANETX_DEMO").map_or_else(|| PathBuf::from(r"C:\Dev\planetx\planetx\demo"), PathBuf::from);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("папки демо модуля нет, проверка пропущена");
        return;
    };
    let mut files: Vec<PathBuf> =
        entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "planetx")).collect();
    files.sort();
    assert!(!files.is_empty());
    for path in files {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let bytes = std::fs::read(&path).unwrap();
        let (scene, places) = read_scene(&bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let mut counts = Counts::default();
        if !places.is_empty() {
            let tree = kml::read_file(&places, &name).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            count(&tree, &mut counts);
        }
        println!(
            "{name} камера {:?} папок {} меток {} туров {} наложений {}",
            scene.camera, counts.folders, counts.places, counts.tours, counts.overlays
        );
        let (_, f, p, t, o) = MODULE.iter().find(|m| m.0 == name).unwrap_or_else(|| panic!("{name}: нет в эталоне"));
        assert_eq!(counts, Counts { folders: *f, places: *p, tours: *t, overlays: *o }, "{name}");
    }
}
