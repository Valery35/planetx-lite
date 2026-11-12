// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Своя папка программы: `--profile папка` или переменная
//! `PLANETX_LITE_HOME`. В ней лежат настройки eframe (`data/app.ron`)
//! и «Мои слои» с результатами расчётов (`PlanetX-Lite`). Без неё -
//! общие папки пользователя, на Windows `%APPDATA%\PlanetX Lite`
//! и `%APPDATA%\PlanetX-Lite`. Кэш тайлов остаётся общим: своя папка
//! не качает тайлы заново. Нужна проверочным копиям, они не трогают
//! настройки автора, просьба управления линейкой от 13 ноября 2026 года.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Переменная окружения своей папки.
pub const ENV: &str = "PLANETX_LITE_HOME";
/// Ключ командной строки своей папки, сильнее переменной.
pub const FLAG: &str = "--profile";
/// Папка настроек eframe внутри своей папки.
const DATA: &str = "data";

/// Своя папка по ключу args и значению переменной env. Пустое значение -
/// папки нет.
pub fn chosen(args: &[String], env: Option<OsString>) -> Option<PathBuf> {
    let from_flag = args.windows(2).find(|w| w[0] == FLAG).map(|w| OsString::from(&w[1]));
    from_flag.or(env).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Папки программы.
#[derive(Debug, Clone, PartialEq)]
pub struct Dirs {
    /// Папка настроек eframe с `app.ron`.
    pub storage: Option<PathBuf>,
    /// Файл настроек eframe, если папка своя. None - eframe берёт
    /// папку по умолчанию сам.
    pub storage_file: Option<PathBuf>,
    /// Папка «Моих слоёв» и результатов.
    pub user: Option<PathBuf>,
}

/// Папки при своей папке home. Без неё: storage_default - папка настроек
/// eframe по умолчанию, data_default - папка данных пользователя.
pub fn dirs(home: Option<&Path>, storage_default: Option<PathBuf>, data_default: Option<PathBuf>, user_dir: &str) -> Dirs {
    match home {
        Some(h) => Dirs { storage: Some(h.join(DATA)), storage_file: Some(h.join(DATA).join(crate::window::FILE)), user: Some(h.join(user_dir)) },
        None => Dirs { storage: storage_default, storage_file: None, user: data_default.map(|d| d.join(user_dir)) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn profile_flag_beats_variable_and_empty_means_none() {
        assert_eq!(chosen(&args(&["lite", "--profile", "C:/check"]), Some("D:/env".into())), Some(PathBuf::from("C:/check")));
        assert_eq!(chosen(&args(&["lite"]), Some("D:/env".into())), Some(PathBuf::from("D:/env")));
        assert_eq!(chosen(&args(&["lite"]), Some(OsString::new())), None);
        assert_eq!(chosen(&args(&["lite"]), None), None);
        // Ключ без значения - не своя папка.
        assert_eq!(chosen(&args(&["lite", "--profile"]), None), None);
    }

    /// Проверочная копия писала настройки и «Мои слои» в папки автора.
    #[test]
    fn own_home_keeps_everything_inside_it() {
        let home = PathBuf::from("C:/tmp/lite-check");
        let appdata = PathBuf::from("C:/Users/a/AppData/Roaming");
        let d = dirs(Some(&home), Some(appdata.join("PlanetX Lite").join("data")), Some(appdata.clone()), "PlanetX-Lite");
        for path in [d.storage.as_ref(), d.storage_file.as_ref(), d.user.as_ref()].into_iter().flatten() {
            assert!(path.starts_with(&home), "{path:?} вне своей папки");
            assert!(!path.starts_with(&appdata), "{path:?} в папке автора");
        }
        assert_eq!(d.storage_file, Some(home.join("data").join("app.ron")));
        // Без своей папки - общие папки, файл настроек выбирает eframe.
        let shared = dirs(None, Some(appdata.join("PlanetX Lite").join("data")), Some(appdata.clone()), "PlanetX-Lite");
        assert_eq!(shared.storage_file, None);
        assert_eq!(shared.user, Some(appdata.join("PlanetX-Lite")));
    }
}
