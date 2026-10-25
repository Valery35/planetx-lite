// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Меню правой кнопки на глобусе, перенос `ui/globemenu.py` модуля.
//!
//! Открывается щелчком правой кнопки без протяжки. Сверху - координаты
//! точки, щелчок копирует их. Пока рисуется фигура - пункты рисования.
//! Над меткой - её название и «Свойства…». Над землёй - метка и линейка
//! отсюда, перелёт и облёт, маршрут, «Что здесь?». Над моделью на пути
//! или при камере следом - «Камера следом», `model_track_items` модуля.

use eframe::egui;

use crate::i18n::{Lang, tr};

/// Что выбрали в меню.
#[derive(Debug, Clone, PartialEq)]
pub enum Choice {
    CopyCoords(String),
    RemoveVertex(usize),
    Finish,
    Continue,
    Save,
    Clear,
    Properties(Vec<usize>),
    AddPlacemark,
    Measure,
    FlyHere,
    Spin,
    WhatHere,
    /// «Проложить маршрут отсюда» и «сюда», `route_items` модуля.
    RouteFrom,
    RouteTo,
    /// «Камера следом» за моделью на пути: номер или None - отпустить.
    Follow(Option<usize>),
    /// «Камера следом» за живым объектом: строка и ключ или None.
    FollowLive(Option<(crate::live::Kind, String)>),
    /// «Полёт отсюда»: вид самолёта `data::flightsim`.
    FlyFrom(&'static str),
    /// Номер листа из «Номенклатуры листа» - в буфер обмена.
    CopySheet(String),
    /// «Все номера в окне…».
    SheetsWindow,
    /// Свой пункт «Открыть в браузере»: адрес-шаблон.
    OpenLink(String),
    /// «Добавить или изменить сайты…».
    EditLinks,
    /// «Видимость отсюда…».
    Viewshed,
    /// «Инсоляция здесь…».
    Insolation,
    /// «Старые карты «Это Место» здесь…».
    Etomesto,
    /// «Исторические карты Рамсея здесь…».
    Rumsey,
    /// «Космоснимки здесь…».
    Sentinel,
    /// «Облака точек лидара здесь».
    Lidar,
    /// «Орбита и след» спутника: номер NORAD или None - снять.
    SatPath(Option<i64>),
    /// «Камера следом» за спутником: номер NORAD или None - отпустить.
    SatFollow(Option<i64>),
}

/// Рисование под меню: вершина под курсором, завершено ли, можно ли
/// сохранить, точка ли это.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drawing {
    pub vertex: Option<usize>,
    pub finished: bool,
    pub can_save: bool,
    pub point: bool,
}

/// Открытое меню: место на экране, точка земли, метка под курсором.
#[derive(Debug, Clone, PartialEq)]
pub struct GlobeMenu {
    pub at: egui::Pos2,
    pub point: Option<(f64, f64)>,
    /// Координаты точки словами, 6 знаков, без высоты.
    pub coords: String,
    pub place: Option<(Vec<usize>, String)>,
    pub drawing: Option<Drawing>,
    /// Модель на пути под курсором или та, за которой идёт камера:
    /// номер, название, идёт ли камера следом.
    pub mover: Option<(usize, String, bool)>,
    /// Живой объект под курсором или тот, за которым идёт камера: строка,
    /// ключ, подпись, идёт ли камера следом, `live_items` модуля.
    pub live: Option<(crate::live::Kind, String, String, bool)>,
    /// Листы карт в точке: номер и система с масштабом, `sheet_items`.
    pub sheets: Vec<(String, String)>,
    /// Свои пункты «Открыть в браузере»: название и шаблон, `link_items`.
    pub links: Vec<(String, String)>,
    /// Спутник под курсором или выбранный: номер NORAD, название,
    /// показаны ли виток и след, идёт ли камера следом,
    /// `satellite_items` модуля.
    pub satellite: Option<(i64, String, bool, bool)>,
}

impl GlobeMenu {
    /// Меню. Возвращает выбор и открыто ли оно ещё.
    pub fn show(&self, ctx: &egui::Context, lang: Lang) -> (Option<Choice>, bool) {
        let mut choice = None;
        let mut copy = false;
        let mut follow = None;
        let area = egui::Area::new(egui::Id::unique("planetx-globe-menu")).order(egui::Order::Foreground).fixed_pos(self.at).show(ctx, |ui| {
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                // Пункты - строки меню без рамок во всю ширину, как
                // у контекстного меню egui.
                egui::containers::menu::menu_style(ui.style_mut());
                ui.set_min_width(200.0);
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                    let mut pick = |ui: &mut egui::Ui, text: &str, c: Choice| {
                        if ui.button(text).clicked() {
                            choice = Some(c);
                        }
                    };
                    if self.point.is_some() {
                        if ui.button(&self.coords).on_hover_text(tr(lang, "Copy the coordinates to the clipboard.")).clicked() {
                            copy = true;
                        }
                        ui.separator();
                    }
                    if let Some(d) = self.drawing {
                        if let Some(v) = d.vertex {
                            pick(ui, tr(lang, "Delete vertex"), Choice::RemoveVertex(v));
                        }
                        if !d.point {
                            if d.finished {
                                pick(ui, tr(lang, "Continue drawing"), Choice::Continue);
                            } else {
                                pick(ui, tr(lang, "Finish drawing"), Choice::Finish);
                            }
                        }
                        ui.add_enabled_ui(d.can_save, |ui| pick(ui, tr(lang, "Save"), Choice::Save));
                        pick(ui, tr(lang, "Clear"), Choice::Clear);
                    } else if let Some((path, name)) = &self.place {
                        ui.add_enabled(false, egui::Button::new(name.as_str()));
                        pick(ui, tr(lang, "Properties…"), Choice::Properties(path.clone()));
                    }
                    if self.drawing.is_none()
                        && let Some((number, name, path, followed)) = &self.satellite
                    {
                        ui.separator();
                        ui.add_enabled(false, egui::Button::new(name.as_str()));
                        let mut on = *path;
                        if ui.checkbox(&mut on, tr(lang, "Orbit and ground track")).clicked() {
                            follow = Some(Choice::SatPath(on.then_some(*number)));
                        }
                        let mut on = *followed;
                        if ui.checkbox(&mut on, tr(lang, "Camera follows")).clicked() {
                            follow = Some(Choice::SatFollow(on.then_some(*number)));
                        }
                    }
                    if self.drawing.is_none()
                        && let Some((kind, key, label, followed)) = &self.live
                    {
                        ui.separator();
                        ui.add_enabled(false, egui::Button::new(label.as_str()));
                        let mut on = *followed;
                        if ui.checkbox(&mut on, tr(lang, "Camera follows")).clicked() {
                            follow = Some(Choice::FollowLive(on.then(|| (*kind, key.clone()))));
                        }
                    }
                    if self.drawing.is_none()
                        && let Some((index, name, followed)) = &self.mover
                    {
                        ui.separator();
                        let title = if name.is_empty() { tr(lang, "Model") } else { name.as_str() };
                        ui.add_enabled(false, egui::Button::new(title));
                        let mut on = *followed;
                        if ui.checkbox(&mut on, tr(lang, "Camera follows")).clicked() {
                            follow = Some(Choice::Follow(on.then_some(*index)));
                        }
                    }
                    if self.point.is_some() {
                        ui.separator();
                        if self.drawing.is_none() {
                            pick(ui, tr(lang, "Add placemark here"), Choice::AddPlacemark);
                            pick(ui, tr(lang, "Measure distance"), Choice::Measure);
                            ui.separator();
                        }
                        pick(ui, tr(lang, "Fly here"), Choice::FlyHere);
                        pick(ui, tr(lang, "Orbit around"), Choice::Spin);
                        if self.drawing.is_none() {
                            ui.separator();
                            pick(ui, tr(lang, "Directions from here"), Choice::RouteFrom);
                            pick(ui, tr(lang, "Directions to here"), Choice::RouteTo);
                            // «Полёт отсюда», `flightsim` модуля.
                            ui.menu_button(tr(lang, "Fly from here"), |ui| {
                                if ui.button(tr(lang, "Fighter")).on_hover_text(tr(lang, "Up to 2000 km/h, start at 900 km/h, sharp bank.")).clicked() {
                                    follow = Some(Choice::FlyFrom(data::flightsim::FIGHTER));
                                }
                                if ui.button(tr(lang, "Propeller plane")).on_hover_text(tr(lang, "Up to 340 km/h, start at 250 km/h, smooth.")).clicked() {
                                    follow = Some(Choice::FlyFrom(data::flightsim::LIGHT));
                                }
                            });
                        }
                        ui.separator();
                        pick(ui, tr(lang, "What's here?"), Choice::WhatHere);
                        // Окна расчёта у точки - одной группой, старые карты -
                        // другой, как у модуля.
                        if self.drawing.is_none() {
                            ui.separator();
                            pick(ui, tr(lang, "Viewshed from here…"), Choice::Viewshed);
                            pick(ui, tr(lang, "Satellite images here…"), Choice::Sentinel);
                        pick(ui, tr(lang, "Lidar point clouds here"), Choice::Lidar);
                            pick(ui, tr(lang, "Insolation here…"), Choice::Insolation);
                            ui.separator();
                            // Карты Рамсея (IIIF) и «Это место» не отдают данные
                            // странице, у неё пункты - заглушки.
                            if cfg!(target_arch = "wasm32") {
                                for text in ["Rumsey historical maps here…", "Eto Mesto old maps here…"] {
                                    ui.add_enabled(false, egui::Button::new(tr(lang, text))).on_disabled_hover_text(tr(lang, crate::layers::DESKTOP_ONLY));
                                }
                            } else {
                                pick(ui, tr(lang, "Rumsey historical maps here…"), Choice::Rumsey);
                                pick(ui, tr(lang, "Eto Mesto old maps here…"), Choice::Etomesto);
                            }
                        }
                        ui.separator();
                        // «Открыть в браузере» и «Номенклатура листа», link_items
                        // и sheet_items модуля.
                        ui.menu_button(tr(lang, "Open in browser"), |ui| {
                            for (name, template) in &self.links {
                                if ui.button(name.as_str()).clicked() {
                                    follow = Some(Choice::OpenLink(template.clone()));
                                }
                            }
                            if !self.links.is_empty() {
                                ui.separator();
                            }
                            if ui.button(tr(lang, "Add or edit sites…")).clicked() {
                                follow = Some(Choice::EditLinks);
                            }
                        });
                        if self.drawing.is_none() {
                            ui.menu_button(tr(lang, "Map sheet designation"), |ui| {
                                for (number, label) in &self.sheets {
                                    if ui.button(format!("{number}    {label}")).clicked() {
                                        follow = Some(Choice::CopySheet(number.clone()));
                                    }
                                }
                                ui.separator();
                                if ui.button(tr(lang, "All numbers in a window…")).clicked() {
                                    follow = Some(Choice::SheetsWindow);
                                }
                            });
                        }
                    }
                });
            });
        });
        if copy {
            choice = Some(Choice::CopyCoords(self.coords.clone()));
        }
        if follow.is_some() {
            choice = follow;
        }
        let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p)));
        let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let open = choice.is_none() && !outside && !escape;
        (choice, open)
    }
}
