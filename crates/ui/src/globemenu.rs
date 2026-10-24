// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Меню правой кнопки на глобусе, перенос `ui/globemenu.py` модуля.
//!
//! Открывается щелчком правой кнопки без протяжки. Сверху - координаты
//! точки, щелчок копирует их. Пока рисуется фигура - пункты рисования.
//! Над меткой - её название и «Свойства…». Над землёй - метка и линейка
//! отсюда, перелёт и облёт, «Что здесь?». Маршрутов, полёта, видимости,
//! космоснимков, старых карт, ссылок на сайты и номенклатуры листов у Lite
//! нет.

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
}

impl GlobeMenu {
    /// Меню. Возвращает выбор и открыто ли оно ещё.
    pub fn show(&self, ctx: &egui::Context, lang: Lang) -> (Option<Choice>, bool) {
        let mut choice = None;
        let mut copy = false;
        let area = egui::Area::new(egui::Id::unique("planetx-globe-menu")).order(egui::Order::Foreground).fixed_pos(self.at).show(ctx, |ui| {
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                ui.set_min_width(200.0);
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
                    }
                    ui.separator();
                    pick(ui, tr(lang, "What's here?"), Choice::WhatHere);
                }
            });
        });
        if copy {
            choice = Some(Choice::CopyCoords(self.coords.clone()));
        }
        let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p)));
        let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let open = choice.is_none() && !outside && !escape;
        (choice, open)
    }
}
