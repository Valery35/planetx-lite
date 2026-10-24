// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Объекты» кнопки «Определить объекты» и пункта «Что здесь?»,
//! перенос `IdentifyDialog` (`ui/identify.py`) и групп «Мои метки»
//! и «Место» (`ui/globe/identify.py`) модуля.
//!
//! Сверху - точка словами, ниже - дерево «Объект» | «Значение»: группа,
//! её объекты, поля объекта. Двойной щелчок по строке копирует значение
//! в буфер обмена, ссылку открывает в браузере. Групп слоёв QGIS,
//! землетрясений, спутников и недр у Lite нет.

use eframe::egui;

use crate::i18n::{Lang, tr};

/// Объект группы: название и поля (название поля, значение).
pub type Feature = (String, Vec<(String, String)>);

/// Окно «Объекты».
pub struct Objects {
    /// Точка словами: координаты и высота.
    pub point: String,
    /// Группы по порядку: название и объекты.
    pub groups: Vec<(String, Vec<Feature>)>,
    /// Точка для адреса «Что здесь?».
    pub at: (f64, f64),
}

/// Расстояние словами, `distance_text` модуля: до 10 км - в метрах,
/// дальше - в километрах с пробелом между разрядами.
pub fn distance_text(lang: Lang, metres: f64) -> String {
    if metres < 10_000.0 {
        format!("{:.0} {}", metres, tr(lang, "m"))
    } else {
        format!("{} {}", crate::app::group_thousands((metres / 1000.0).round() as i64), tr(lang, "km"))
    }
}

/// Группа «Мои метки», `_identify_places` модуля: тип, координаты точки,
/// длина пути или периметр многоугольника, описание и поля метки.
pub fn places_group(lang: Lang, places: &[&scene::kml::Place]) -> Option<(String, Vec<Feature>)> {
    use scene::features::Kind;
    let features: Vec<Feature> = places
        .iter()
        .map(|p| {
            let kind = match p.kind {
                Kind::Point => tr(lang, "Placemark"),
                Kind::Line => tr(lang, "Path"),
                Kind::Polygon => tr(lang, "Polygon"),
            };
            let mut values = vec![(tr(lang, "Type").to_string(), kind.to_string())];
            match p.kind {
                Kind::Point => {
                    if let Some((a, b)) = p.points.first() {
                        values.push((tr(lang, "Coordinates").to_string(), format!("{a:.5}, {b:.5}")));
                    }
                }
                Kind::Line => values.push((tr(lang, "Length").to_string(), distance_text(lang, scene::measure::sphere_length(&p.points)))),
                Kind::Polygon => {
                    let mut ring = p.points.clone();
                    if let Some(first) = ring.first().copied() {
                        ring.push(first);
                    }
                    values.push((tr(lang, "Perimeter").to_string(), distance_text(lang, scene::measure::sphere_length(&ring))));
                }
            }
            if !p.description.trim().is_empty() {
                values.push((tr(lang, "Description").to_string(), p.description.clone()));
            }
            values.extend(p.data.iter().flatten().cloned());
            let name = if p.name.trim().is_empty() { tr(lang, "Untitled").to_string() } else { p.name.clone() };
            (name, values)
        })
        .collect();
    (!features.is_empty()).then(|| (tr(lang, "My Places").to_string(), features))
}

/// Группа «Место», объект «Под точкой»: адрес после «Что здесь?»,
/// высота или глубина.
pub fn place_group(lang: Lang, height: Option<f64>, address: Option<String>) -> (String, Vec<Feature>) {
    let mut values = Vec::new();
    if let Some(a) = address {
        values.push((tr(lang, "Address").to_string(), a));
    }
    match height {
        Some(h) if h < -0.5 => values.push((tr(lang, "Depth").to_string(), format!("{:.0} {}", -h, tr(lang, "m")))),
        Some(h) => values.push((tr(lang, "Height").to_string(), format!("{:.0} {}", h.max(0.0), tr(lang, "m")))),
        None => {}
    }
    (tr(lang, "Site").to_string(), vec![(tr(lang, "Under the point").to_string(), values)])
}

impl Objects {
    /// Окно. Возвращает false, когда его закрыли.
    pub fn window(&mut self, ctx: &egui::Context, lang: Lang) -> bool {
        let mut open = true;
        egui::Window::new(tr(lang, "Features")).open(&mut open).default_size([420.0, 360.0]).max_width(520.0).show(ctx, |ui| {
            ui.add(egui::Label::new(&self.point).selectable(true));
            ui.separator();
            let total: usize = self.groups.iter().map(|(_, f)| f.len()).sum();
            egui::ScrollArea::vertical().show(ui, |ui| {
                let tip = tr(lang, "A double click on a row copies the value to the clipboard, a link opens in the browser.");
                if self.groups.iter().all(|(_, f)| f.iter().all(|(_, v)| v.is_empty())) && total == 0 {
                    ui.label(tr(lang, "No features at the point"));
                    return;
                }
                egui::Grid::new("objects-head").num_columns(2).show(ui, |ui| {
                    ui.label(egui::RichText::new(tr(lang, "Feature")).strong());
                    ui.label(egui::RichText::new(tr(lang, "Value")).strong());
                    ui.end_row();
                });
                for (g, (group, features)) in self.groups.iter().enumerate() {
                    egui::CollapsingHeader::new(group.as_str()).id_salt(("objects-group", g)).default_open(true).show(ui, |ui| {
                        for (f, (name, fields)) in features.iter().enumerate() {
                            egui::CollapsingHeader::new(name.as_str()).id_salt(("objects-feature", g, f)).default_open(total <= 5).show(ui, |ui| {
                                egui::Grid::new(("objects-fields", g, f)).num_columns(2).striped(true).show(ui, |ui| {
                                    for (field, value) in fields {
                                        ui.label(field.as_str());
                                        let link = value.starts_with("http://") || value.starts_with("https://");
                                        let text = if link { egui::RichText::new(value.as_str()).color(crate::theme::pal().accent) } else { egui::RichText::new(value.as_str()) };
                                        let r = ui.add(egui::Label::new(text).wrap().sense(egui::Sense::click())).on_hover_text(tip);
                                        if r.double_clicked() {
                                            if link {
                                                ctx.open_url(egui::OpenUrl::new_tab(value.as_str()));
                                            } else {
                                                ctx.copy_text(value.clone());
                                            }
                                        }
                                        ui.end_row();
                                    }
                                });
                            });
                        }
                    });
                }
            });
        });
        open
    }
}
