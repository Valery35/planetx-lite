// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сохранение «Моих меток», как у модуля (`export_kml`, `save_scene`).
//!
//! KML - метки, пути, многоугольники, наложения и туры в формате
//! Google Earth, его читают Google Earth, QGIS и другие программы. KMZ -
//! тот же KML в архиве вместе с картинками наложений и файлами моделей.
//! Сцена `.planetx` - архив ZIP: scene.json с камерой глобуса и названием
//! папки меток и рядом places.kml или places.kmz. Сцену открывают модуль
//! и Lite, камера встаёт, как была при сохранении.
//!
//! В программе файл выбирается окном сохранения системы, на странице
//! браузер скачивает его под предложенным именем.

use globe_core::navigation::Pose;
use scene::kml::{self, Folder};
use scene::scene::{EXTENSION, Places, Scene, write_scene};

/// Что сохраняется.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// KML или KMZ: в программе - по выбору в окне, на странице - KMZ,
    /// если в метках есть картинки или модели, иначе KML.
    Kml,
    /// Сцена `.planetx`.
    Scene,
}

/// Есть ли в дереве картинки или модели, которые KML не удержит.
pub fn has_files(tree: &Folder) -> bool {
    tree.overlays().iter().any(|o| o.image.as_ref().is_some_and(|i| !i.is_empty()))
}

/// Байты KML (kmz = false) или KMZ дерева.
pub fn tree_bytes(tree: &Folder, kmz: bool) -> Result<Vec<u8>, String> {
    if kmz { kml::write_kmz(tree).map_err(|e| format!("{e:?}")) } else { Ok(kml::write_kml(tree).into_bytes()) }
}

/// Байты сцены: камера pose, метки tree, название name.
pub fn scene_bytes(tree: &Folder, pose: &Pose, name: &str) -> Result<Vec<u8>, String> {
    let mut scene = Scene::new(&[pose.lat, pose.lon, pose.distance, pose.heading, pose.tilt]);
    scene.name = name.to_string();
    scene.places = tree.name.clone();
    let result = if has_files(tree) {
        let kmz = kml::write_kmz(tree).map_err(|e| format!("{e:?}"))?;
        write_scene(&scene, Places::Kmz(&kmz))
    } else {
        let text = kml::write_kml(tree);
        write_scene(&scene, Places::Kml(&text))
    };
    result.map_err(|e| e.to_string())
}

/// Имя файла по названию: без знаков, которые не годятся в имени.
pub fn file_stem(name: &str) -> String {
    let clean: String =
        name.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect();
    let clean = clean.trim().trim_matches('.').to_string();
    if clean.is_empty() { "PlanetX".to_string() } else { clean }
}

/// Сохранить дерево tree. pose - камера для сцены. Возвращает, куда
/// сохранено: путь в программе, имя скачанного файла на странице. None -
/// окно сохранения закрыли без выбора.
pub fn save(kind: Kind, tree: &Folder, pose: &Pose) -> Result<Option<String>, String> {
    let stem = file_stem(&tree.name);
    let kmz_default = has_files(tree);
    let default_ext = match kind {
        Kind::Kml if kmz_default => ".kmz",
        Kind::Kml => ".kml",
        Kind::Scene => EXTENSION,
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let dialog = rfd::FileDialog::new().set_file_name(format!("{stem}{default_ext}"));
        let dialog = match kind {
            Kind::Kml if kmz_default => dialog.add_filter("KMZ", &["kmz"]).add_filter("KML", &["kml"]),
            Kind::Kml => dialog.add_filter("KML", &["kml"]).add_filter("KMZ", &["kmz"]),
            Kind::Scene => dialog.add_filter("PlanetX", &["planetx"]),
        };
        let Some(path) = dialog.save_file() else {
            return Ok(None);
        };
        let lower = path.to_string_lossy().to_lowercase();
        let bytes = match kind {
            Kind::Kml => tree_bytes(tree, lower.ends_with(".kmz"))?,
            Kind::Scene => scene_bytes(tree, pose, &stem)?,
        };
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Some(path.display().to_string()))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let bytes = match kind {
            Kind::Kml => tree_bytes(tree, kmz_default)?,
            Kind::Scene => scene_bytes(tree, pose, &stem)?,
        };
        let name = format!("{stem}{default_ext}");
        download(&name, &bytes)?;
        Ok(Some(name))
    }
}

/// Сохранить байты файлом: в программе - окном выбора файла с именем
/// name и фильтром (название, расширение), на странице - скачиванием.
/// Возвращает, куда сохранено, None - окно закрыли без выбора.
pub fn save_bytes(name: &str, filter: (&str, &str), bytes: &[u8]) -> Result<Option<String>, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(path) = rfd::FileDialog::new().set_file_name(name).add_filter(filter.0, &[filter.1]).save_file() else {
            return Ok(None);
        };
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Some(path.display().to_string()))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = filter;
        download(name, bytes)?;
        Ok(Some(name.to_string()))
    }
}

/// Скачать байты файлом name: ссылка на Blob, щелчок по ней.
#[cfg(target_arch = "wasm32")]
fn download(name: &str, bytes: &[u8]) -> Result<(), String> {
    use wasm_bindgen::JsCast;
    let fail = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes));
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts).map_err(fail)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(fail)?;
    let document = web_sys::window().and_then(|w| w.document()).ok_or("нет документа")?;
    let link = document.create_element("a").map_err(fail)?.dyn_into::<web_sys::HtmlAnchorElement>().map_err(|_| "нет ссылки")?;
    link.set_href(&url);
    link.set_download(name);
    link.click();
    web_sys::Url::revoke_object_url(&url).map_err(fail)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{file_stem, scene_bytes, tree_bytes};
    use globe_core::navigation::Pose;
    use scene::features::Kind;
    use scene::kml::{Folder, Item, Place};

    fn tree() -> Folder {
        let mut f = Folder::new("Японский жёлоб");
        f.children.push(Item::Place(Place::new("Вулкан Фудзи", Kind::Point, vec![(35.36, 138.73)])));
        f
    }

    /// Сохранённое читается обратно: KML, KMZ и сцена с камерой.
    #[test]
    fn saved_files_open_again() {
        let t = tree();
        for kmz in [false, true] {
            let back = scene::kml::read_file(&tree_bytes(&t, kmz).unwrap(), "x").unwrap();
            assert_eq!(back.name, "Японский жёлоб");
        }
        let pose = Pose::new(35.0, 139.0, 120_000.0, 10.0, 45.0);
        let bytes = scene_bytes(&t, &pose, "Япония").unwrap();
        let opened = crate::marks::open(&bytes, "Япония.planetx").unwrap();
        let p = opened.pose.unwrap();
        assert_eq!((p.lat, p.lon, p.distance, p.heading, p.tilt), (35.0, 139.0, 120_000.0, 10.0, 45.0));
        assert_eq!(opened.name, "Япония");
        assert!(matches!(opened.tree.children.first(), Some(Item::Place(p)) if p.name == "Вулкан Фудзи"));
    }

    /// Сцена демо с 3D-моделями: после сохранения модели внутри файла.
    #[test]
    fn models_survive_scene_save() {
        let Ok(bytes) = std::fs::read(r"C:\Dev\planetx\planetx\demo\models.planetx") else {
            eprintln!("демо модуля нет, проверка пропущена");
            return;
        };
        let opened = crate::marks::open(&bytes, "models.planetx").unwrap();
        let before = opened.tree.overlays().iter().filter(|o| o.image.as_ref().is_some_and(|i| !i.is_empty())).count();
        assert!(before > 0);
        let pose = opened.pose.clone().unwrap();
        let again = crate::marks::open(&scene_bytes(&opened.tree, &pose, "Модели").unwrap(), "Модели.planetx").unwrap();
        let after = again.tree.overlays().iter().filter(|o| o.image.as_ref().is_some_and(|i| !i.is_empty())).count();
        assert_eq!(before, after);
        let kmz = scene::kml::read_file(&tree_bytes(&opened.tree, true).unwrap(), "m.kmz").unwrap();
        assert_eq!(kmz.overlays().len(), opened.tree.overlays().len());
    }

    #[test]
    fn file_names_are_clean() {
        assert_eq!(file_stem("PlanetX: демо, Японский жёлоб"), "PlanetX_ демо, Японский жёлоб");
        assert_eq!(file_stem("  "), "PlanetX");
        assert_eq!(file_stem("a/b\\c?"), "a_b_c_");
    }
}
