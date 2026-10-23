// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Демо: сцены модуля из папки `demo` хранилища planetx-terrain.
//!
//! Решение автора от 23 октября 2026 года - одно место демо для модуля
//! и Lite, демо в выпуск Lite не входит. Файл демо скачивается при
//! первом открытии. Программа хранит его в папке кэша, страница
//! полагается на кэш браузера. Раскладка - как в папке `planetx/demo`
//! модуля, меню - как у модуля, без демо, которые Lite пока не
//! показывает: подземный вид, поезда вживую, исторические карты, Марс,
//! Луна и небо.

/// Начало адреса файлов демо.
pub const BASE: &str = "https://raw.githubusercontent.com/Valery35/planetx-terrain/main/demo/";

/// Демо меню. en - название по-английски, ключ перевода [crate::i18n::tr].
pub struct Demo {
    pub en: &'static str,
    /// Пути файлов в папке демо, открываются по порядку: растр высот
    /// раньше сцены.
    pub files: &'static [&'static str],
}

/// Раздел меню, en - как у [Demo].
pub struct Section {
    pub en: &'static str,
    pub demos: &'static [Demo],
}

pub const MENU: [Section; 6] = [
    Section {
        en: "Placemarks and tours",
        demos: &[
            Demo { en: "Boca Chica, Starbase", files: &["bocachica.planetx"] },
            Demo { en: "Flow of goods, a scheme", files: &["flows.planetx"] },
        ],
    },
    Section {
        en: "Imagery and maps",
        demos: &[
            Demo { en: "Aral Sea", files: &["aral.planetx"] },
            Demo { en: "Super-overlay, Mountain View", files: &["superoverlay.planetx"] },
        ],
    },
    Section {
        en: "Inside the Earth",
        demos: &[
            Demo { en: "Japan Trench", files: &["japan.planetx"] },
            Demo { en: "Vegas Loop tunnels", files: &["vegas/vegas_loop.gpkg"] },
        ],
    },
    Section {
        en: "Own terrain",
        demos: &[
            Demo { en: "Quarry, own terrain", files: &["quarry/quarry_dem.tif", "quarry.planetx"] },
            Demo {
                en: "Lake Brienz, own bathymetry",
                files: &["brienz/brienzersee_bed.tif", "brienz.planetx"],
            },
        ],
    },
    Section {
        en: "3D models and motion",
        demos: &[
            Demo { en: "3D models, Suzdal and Perm", files: &["models.planetx"] },
            Demo { en: "Animated model, wind turbines", files: &["turbines.planetx"] },
        ],
    },
    Section {
        en: "Haul trucks in open pits",
        demos: &[
            // Самосвалы - пути KML с моделью, как демо модуля
            // (`_play_model_trucks`): шкала идёт ×10 по кругу рейса.
            Demo { en: "Haul trucks, Kachkanar", files: &["haul_kachkanar.planetx"] },
            Demo { en: "Haul trucks, Lebedinsky", files: &["haul_lebedinsky.planetx"] },
        ],
    },
];

/// Демо самосвалов: после открытия шкала проигрывает рейс по кругу.
pub fn is_haul(demo: &Demo) -> bool {
    demo.files.iter().any(|f| name(f).starts_with("haul_"))
}

/// Адрес файла демо.
pub fn url(path: &str) -> String {
    format!("{BASE}{path}")
}

/// Имя файла по пути демо.
pub fn name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Файл демо из кэша dir.
#[cfg(not(target_arch = "wasm32"))]
pub fn cached(dir: &std::path::Path, path: &str) -> Option<Vec<u8>> {
    std::fs::read(dir.join(path)).ok().filter(|b| !b.is_empty())
}

/// Записать файл демо в кэш dir: сначала во временный файл, потом
/// переименованием, так оборванная запись не сойдёт за файл.
#[cfg(not(target_arch = "wasm32"))]
pub fn store(dir: &std::path::Path, path: &str, bytes: &[u8]) -> std::io::Result<()> {
    let target = dir.join(path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let part = target.with_extension("part");
    std::fs::write(&part, bytes)?;
    std::fs::rename(&part, &target)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{MENU, name, url};

    #[test]
    fn url_keeps_demo_layout() {
        assert_eq!(
            url("quarry/quarry_dem.tif"),
            "https://raw.githubusercontent.com/Valery35/planetx-terrain/main/demo/quarry/quarry_dem.tif"
        );
        assert_eq!(name("quarry/quarry_dem.tif"), "quarry_dem.tif");
        assert_eq!(name("aral.planetx"), "aral.planetx");
    }

    /// Каждый файл меню есть в папке демо модуля, той же, что лежит
    /// в planetx-terrain.
    #[test]
    fn menu_files_exist_in_module_demo() {
        let dir = std::env::var_os("PLANETX_DEMO")
            .map_or_else(|| std::path::PathBuf::from(r"C:\Dev\planetx\planetx\demo"), std::path::PathBuf::from);
        if !dir.exists() {
            eprintln!("демо модуля нет, проверка пропущена");
            return;
        }
        for demo in MENU.iter().flat_map(|s| s.demos) {
            assert!(!demo.files.is_empty(), "{}", demo.en);
            for f in demo.files {
                assert!(dir.join(f).is_file(), "{f}");
            }
        }
    }

    #[test]
    fn cache_round_trip() {
        let dir = std::env::temp_dir().join(format!("planetx-lite-demo-{}", std::process::id()));
        assert!(super::cached(&dir, "quarry/quarry_dem.tif").is_none());
        super::store(&dir, "quarry/quarry_dem.tif", b"tif").unwrap();
        assert_eq!(super::cached(&dir, "quarry/quarry_dem.tif").as_deref(), Some(&b"tif"[..]));
        assert!(!dir.join("quarry/quarry_dem.part").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
