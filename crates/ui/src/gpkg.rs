// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Слои GeoPackage как метки: файл - папка, слой - вложенная папка,
//! объект - метка.
//!
//! Название метки - поле `name`, без него - название слоя и номер. Поля
//! объекта - данные метки. Цвет линии и заливки - поле `color` вида
//! `#rrggbb`, если оно есть. Таблицы без геометрии не показываются.
//! У многоугольника первая часть - внешнее кольцо, прочие - дыры.
//! В слое не больше [`MAX_ITEMS`] объектов: дерево меток рисует все
//! строки, тысячи строк замедляют панель.

use data::layers::{LayerData, Shape};
use scene::features::Kind;
use scene::kml::{Folder, Item, Place};

/// Наибольшее количество объектов слоя в дереве меток.
pub const MAX_ITEMS: usize = 2000;

/// Цвет из текста `#rrggbb`, непрозрачный.
pub fn color_of(text: &str) -> Option<[u8; 4]> {
    let hex = text.trim().strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?, 255])
}

/// Папка меток файла GeoPackage name из его слоёв.
pub fn folder(layers: Vec<LayerData>, name: &str) -> Folder {
    let mut root = Folder::new(name);
    for layer in layers {
        if layer.items.iter().all(|i| i.shape == Shape::None) {
            continue;
        }
        let name_field = layer.fields.iter().position(|f| f.eq_ignore_ascii_case("name"));
        let color_field = layer.fields.iter().position(|f| f.eq_ignore_ascii_case("color"));
        let total = layer.items.len();
        let title = if total > MAX_ITEMS {
            format!("{} (первые {MAX_ITEMS} из {total})", layer.name)
        } else {
            layer.name.clone()
        };
        let mut sub = Folder::new(&title);
        for (n, item) in layer.items.into_iter().take(MAX_ITEMS).enumerate() {
            let kind = match item.shape {
                Shape::Point => Kind::Point,
                Shape::Line => Kind::Line,
                Shape::Polygon => Kind::Polygon,
                Shape::None => continue,
            };
            let label = name_field
                .and_then(|i| item.values.get(i))
                .filter(|v| !v.is_empty())
                .cloned()
                .unwrap_or_else(|| format!("{} {}", layer.name, n + 1));
            let color = color_field.and_then(|i| item.values.get(i)).and_then(|v| color_of(v));
            let data: Vec<(String, String)> = layer.fields.iter().cloned().zip(item.values.iter().cloned()).collect();
            let mut parts = item.parts.into_iter();
            match kind {
                Kind::Point => {
                    // Точки многоточия - отдельные метки.
                    for p in parts.filter(|p| !p.is_empty()) {
                        let mut place = Place::new(&label, Kind::Point, p);
                        place.data = Some(data.clone());
                        sub.children.push(Item::Place(place));
                    }
                }
                Kind::Line => {
                    for p in parts.filter(|p| p.len() > 1) {
                        let mut place = Place::new(&label, Kind::Line, p);
                        place.data = Some(data.clone());
                        if let Some(c) = color {
                            place.color = c;
                        }
                        sub.children.push(Item::Place(place));
                    }
                }
                Kind::Polygon => {
                    let Some(outer) = parts.next().filter(|p| p.len() > 2) else {
                        continue;
                    };
                    let holes: Vec<Vec<(f64, f64)>> = parts.filter(|p| p.len() > 2).collect();
                    let mut place = Place::new(&label, Kind::Polygon, outer);
                    place.data = Some(data);
                    if !holes.is_empty() {
                        place.holes = Some(holes);
                    }
                    if let Some(c) = color {
                        place.color = c;
                        place.fill = Some([c[0], c[1], c[2], 100]);
                    }
                    sub.children.push(Item::Place(place));
                }
            }
        }
        root.children.push(Item::Folder(sub));
    }
    root
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{color_of, folder};
    use scene::kml::Item;

    #[test]
    fn colors() {
        assert_eq!(color_of("#ff8000"), Some([255, 128, 0, 255]));
        assert_eq!(color_of("ff8000"), None);
        assert_eq!(color_of("#ff80"), None);
        assert_eq!(color_of("#фф8000"), None);
    }

    #[test]
    fn subsurface_gpkg_becomes_folders_without_tables() {
        let Ok(bytes) = std::fs::read(r"C:\Dev\planetx\planetx\demo\subsurface\perm_subsurface.gpkg") else {
            eprintln!("демо модуля нет, проверка пропущена");
            return;
        };
        let layers = data::layers::read(bytes, "perm_subsurface.gpkg").unwrap();
        let f = folder(layers, "perm_subsurface.gpkg");
        let names: Vec<&str> = f
            .children
            .iter()
            .filter_map(|i| match i {
                Item::Folder(s) => Some(s.name.as_str()),
                _ => None,
            })
            .collect();
        // Таблицы interval, survey, beds без геометрии - не папки.
        assert_eq!(names, ["collar", "sections", "cut", "images"]);
        let Some(Item::Folder(collar)) = f.children.first() else {
            panic!()
        };
        assert_eq!(collar.children.len(), 24);
    }

    #[test]
    fn big_layer_is_cut_with_note() {
        let Ok(bytes) = std::fs::read(r"C:\Dev\planetx\planetx\demo\haul\haul_kachkanar.gpkg") else {
            return;
        };
        let f = folder(data::layers::read(bytes, "haul_kachkanar.gpkg").unwrap(), "haul");
        let Some(Item::Folder(trucks)) = f.children.first() else {
            panic!()
        };
        assert_eq!(trucks.children.len(), super::MAX_ITEMS);
        assert!(trucks.name.contains("5406"), "{}", trucks.name);
    }
}
