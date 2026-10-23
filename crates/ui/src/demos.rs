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

/// Демо меню.
pub struct Demo {
    pub ru: &'static str,
    pub en: &'static str,
    /// Пути файлов в папке демо, открываются по порядку: растр высот
    /// раньше сцены.
    pub files: &'static [&'static str],
}

/// Раздел меню.
pub struct Section {
    pub ru: &'static str,
    pub en: &'static str,
    pub demos: &'static [Demo],
}

pub const MENU: [Section; 6] = [
    Section {
        ru: "Метки и туры",
        en: "Placemarks and tours",
        demos: &[
            Demo { ru: "Бока-Чика, Starbase", en: "Boca Chica, Starbase", files: &["bocachica.planetx"] },
            Demo { ru: "Поток грузов, схема", en: "Flow of goods, a scheme", files: &["flows.planetx"] },
        ],
    },
    Section {
        ru: "Снимки и карты",
        en: "Imagery and maps",
        demos: &[
            Demo { ru: "Аральское море", en: "Aral Sea", files: &["aral.planetx"] },
            Demo { ru: "Суперналожение, Маунтин-Вью", en: "Super-overlay, Mountain View", files: &["superoverlay.planetx"] },
        ],
    },
    Section {
        ru: "Недра",
        en: "Inside the Earth",
        demos: &[
            Demo { ru: "Японский жёлоб", en: "Japan Trench", files: &["japan.planetx"] },
            Demo { ru: "Тоннели Vegas Loop", en: "Vegas Loop tunnels", files: &["vegas/vegas_loop.gpkg"] },
        ],
    },
    Section {
        ru: "Свой рельеф",
        en: "Own terrain",
        demos: &[
            Demo { ru: "Карьер, свой рельеф", en: "Quarry, own terrain", files: &["quarry/quarry_dem.tif", "quarry.planetx"] },
            Demo {
                ru: "Бриенцское озеро, своя батиметрия",
                en: "Lake Brienz, own bathymetry",
                files: &["brienz/brienzersee_bed.tif", "brienz.planetx"],
            },
        ],
    },
    Section {
        ru: "3D-модели и движение",
        en: "3D models and motion",
        demos: &[
            Demo { ru: "3D-модели, Суздаль и Пермь", en: "3D models, Suzdal and Perm", files: &["models.planetx"] },
            Demo { ru: "Анимированная модель, ветряки", en: "Animated model, wind turbines", files: &["turbines.planetx"] },
        ],
    },
    Section {
        ru: "Самосвалы в карьерах",
        en: "Haul trucks in open pits",
        demos: &[
            Demo { ru: "Самосвалы, Качканар", en: "Haul trucks, Kachkanar", files: &["haul/haul_kachkanar.gpkg"] },
            Demo { ru: "Самосвалы, Лебединский", en: "Haul trucks, Lebedinsky", files: &["haul/haul_lebedinsky.gpkg"] },
        ],
    },
];

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
            assert!(!demo.files.is_empty(), "{}", demo.ru);
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
