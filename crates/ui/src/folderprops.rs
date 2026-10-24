// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Свойства папки» по образцу окна папки Google Earth, перенос
//! `ui/folderprops.py` модуля: название, флажки «Разрешить раскрывать
//! папку» и «Показать содержание как группу переключателей», «Скрывать
//! дальше», вкладки «Описание» и «Вид». Правки идут в «Мои метки» по «OK».

use eframe::egui;
use scene::kml::Folder;

use crate::i18n::{Lang, tr};

/// Наибольшее расстояние поля «Скрывать дальше», км, `MAX_KM` модуля.
pub const MAX_KM: f64 = 20000.0;
/// Вид по умолчанию: точка 0, 0, расстояние 10 000 км.
const DEFAULT_VIEW: [f64; 5] = [0.0, 0.0, 1.0e7, 0.0, 0.0];

/// Окно свойств папки по пути path: правки до «OK».
pub struct FolderProps {
    pub path: Vec<usize>,
    pub name: String,
    pub description: String,
    pub expandable: bool,
    pub radio: bool,
    /// «Скрывать дальше», км, 0 - не скрывать.
    pub far: f64,
    /// Группа «Вид папки» отмечена.
    pub has_view: bool,
    pub view: [f64; 5],
    /// Вид при открытии окна - для «Сброс».
    pub old_view: Option<[f64; 5]>,
    pub old_region: Option<scene::region::Region>,
    /// Вкладка: 0 - «Описание», 1 - «Вид».
    tab: usize,
}

/// Чем закончилось окно.
pub enum Outcome {
    Open,
    Cancel,
    Ok,
}

impl FolderProps {
    pub fn new(path: Vec<usize>, folder: &Folder) -> Self {
        Self {
            path,
            name: folder.name.clone(),
            description: folder.description.clone(),
            expandable: folder.expandable,
            radio: folder.radio,
            far: scene::region::distance_km(folder.region.as_ref()).map_or(0.0, |km| km.min(MAX_KM)),
            has_view: folder.view.is_some(),
            view: folder.view.unwrap_or(DEFAULT_VIEW),
            old_view: folder.view,
            old_region: folder.region,
            tab: 0,
        }
    }

    /// Region по полю, `regionfield.value` модуля: 0 - без нижнего предела.
    /// Region из файла с maxLodPixels остаётся с ним, иначе снимается.
    pub fn region(&self, points: &[(f64, f64)]) -> Option<scene::region::Region> {
        if self.far > 0.0 && !points.is_empty() {
            return Some(scene::region::for_distance(points, self.far, self.old_region.as_ref()));
        }
        if self.far > 0.0 {
            return self.old_region;
        }
        self.old_region.filter(|r| r.max_lod >= 0.0).map(|r| scene::region::Region { min_lod: 0.0, ..r })
    }

    /// Окно. current - вид глобуса сейчас для «Снимок текущего вида».
    pub fn window(&mut self, ctx: &egui::Context, lang: Lang, current: [f64; 5]) -> Outcome {
        let mut outcome = Outcome::Open;
        let mut open = true;
        egui::Window::new(tr(lang, "Folder properties")).open(&mut open).collapsible(false).min_width(460.0).show(ctx, |ui| {
            egui::Grid::new("folder-props").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Name"));
                ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(300.0));
                ui.end_row();
                ui.label("");
                ui.vertical(|ui| {
                    ui.checkbox(&mut self.expandable, tr(lang, "Allow the folder to expand"))
                        .on_hover_text(tr(lang, "Without the check the folder does not expand in the list, its placemarks are shown and hidden by the check of the folder."));
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        ui.add_enabled(self.expandable, egui::Checkbox::new(&mut self.radio, tr(lang, "Show contents as a group of radio buttons")))
                            .on_hover_text(tr(lang, "Only one placemark or folder of this folder is seen on the globe. The check of one row clears the checks of the others."));
                    });
                });
                ui.end_row();
                ui.label(tr(lang, "Hide beyond"));
                let none = tr(lang, "do not hide");
                let km = tr(lang, " km");
                ui.add(egui::DragValue::new(&mut self.far).range(0.0..=MAX_KM).fixed_decimals(1).custom_formatter(move |v, _| if v <= 0.0 { none.to_string() } else { format!("{v:.1}{km}") }))
                    .on_hover_text(tr(lang, "The placemark is seen only while the eye is closer than this distance, as Region in KML. For a folder - all its placemarks. 0 - always seen."));
                ui.end_row();
            });
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, 0, tr(lang, "Description"));
                ui.selectable_value(&mut self.tab, 1, tr(lang, "View"));
            });
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_min_width(440.0);
                if self.tab == 0 {
                    ui.add(egui::TextEdit::multiline(&mut self.description).desired_width(f32::INFINITY).desired_rows(6))
                        .on_hover_text(tr(lang, "Description of the folder. It is kept in My Places and in KML."));
                } else {
                    ui.checkbox(&mut self.has_view, tr(lang, "Folder view")).on_hover_text(tr(
                        lang,
                        "Where the camera looks from when it flies to the folder. The view is set by numbers or by a snapshot of the current view. Without its own view the flight frames all placemarks of the folder.",
                    ));
                    ui.add_enabled_ui(self.has_view, |ui| view_fields(ui, lang, &mut self.view));
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            if ui.button(tr(lang, "Reset")).on_hover_text(tr(lang, "Return the view the folder had when the window opened.")).clicked() {
                                self.has_view = self.old_view.is_some();
                                if let Some(v) = self.old_view {
                                    self.view = v;
                                }
                            }
                            if ui.button(tr(lang, "Snapshot of the current view")).on_hover_text(tr(lang, "Take the view of the globe now: the point of view, distance, heading and tilt.")).clicked() {
                                self.view = current;
                                self.has_view = true;
                            }
                        });
                    });
                }
            });
            ui.horizontal(|ui| {
                if ui.button(tr(lang, "OK")).clicked() {
                    outcome = Outcome::Ok;
                }
                if ui.button(tr(lang, "Cancel")).clicked() {
                    outcome = Outcome::Cancel;
                }
            });
        });
        if !open {
            outcome = Outcome::Cancel;
        }
        outcome
    }
}

/// Пять чисел вида: широта, долгота, расстояние, азимут, наклон.
pub fn view_fields(ui: &mut egui::Ui, lang: Lang, view: &mut [f64; 5]) {
    let rows: [(&str, f64, f64, usize, &str, &str); 5] = [
        ("Latitude", -90.0, 90.0, 6, "°", "Latitude of the point the camera looks at."),
        ("Longitude", -180.0, 180.0, 6, "°", "Longitude of the point the camera looks at."),
        ("Distance", 1.0, 5.0e7, 0, " m", "Distance from the camera to the point of view. More - a wider view."),
        ("Heading", 0.0, 360.0, 1, "°", "Where the camera looks. 0 - north at the top of the frame."),
        ("Tilt", 0.0, scene::lookat::MAX_TILT, 1, "°", "Tilt of the camera. 0 - looking straight down, more - towards the horizon."),
    ];
    egui::Grid::new("view-fields").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
        for ((name, low, high, decimals, suffix, tip), value) in rows.into_iter().zip(view.iter_mut()) {
            ui.label(tr(lang, name));
            let speed = ((high - low) / 1000.0).max(1e-6);
            ui.add(egui::DragValue::new(value).range(low..=high).fixed_decimals(decimals).suffix(tr(lang, suffix)).speed(speed)).on_hover_text(tr(lang, tip));
            ui.end_row();
        }
    });
}
