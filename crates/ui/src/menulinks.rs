// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! «Открыть в браузере»: свои пункты меню на глобусе, перенос
//! `ui/menulinks.py` модуля. Пункты - `data::menulinks`, хранятся
//! в настройках строкой JSON. Адрес открывает браузер, программа сама
//! в сеть не ходит.

use eframe::egui;

use crate::i18n::{Lang, tr, trf};

/// Причина, по которой пункт не годится, словами.
pub fn reason_text(lang: Lang, reason: &str) -> &'static str {
    match reason {
        "name" => tr(lang, "The item has no name."),
        "scheme" => tr(lang, "The address does not start with https:// or http://."),
        "place" => tr(lang, "The address has no {lat} or {lon}."),
        _ => tr(lang, "The address has extra braces or places other than {lat}, {lon} and {zoom}."),
    }
}

/// Окно списка: строки название - адрес, правка на месте.
pub struct LinksWindow {
    rows: Vec<(String, String)>,
    /// Отмеченные к удалению строки.
    marked: Vec<bool>,
    status: Option<String>,
}

impl LinksWindow {
    pub fn new(items: Vec<(String, String)>) -> Self {
        let marked = vec![false; items.len()];
        Self { rows: items, marked, status: None }
    }

    fn add(&mut self, name: &str, template: &str) {
        self.rows.push((name.to_string(), template.to_string()));
        self.marked.push(false);
    }

    /// Строки окна без пустых.
    fn items(&self) -> Vec<(String, String)> {
        self.rows.iter().map(|(n, t)| (n.trim().to_string(), t.trim().to_string())).filter(|(n, t)| !n.is_empty() || !t.is_empty()).collect()
    }

    /// Окно. Возвращает список после «ОК» и открыто ли окно.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang) -> (Option<Vec<(String, String)>>, bool) {
        let mut open = true;
        let mut done = None;
        egui::Window::new(tr(lang, "Sites of the Open in browser menu")).open(&mut open).collapsible(false).default_width(720.0).show(ctx, |ui| {
            ui.label(tr(lang, "Each row is an item of the Open in browser submenu of the globe menu. The item opens the site at the point under the cursor. Add site puts a ready site or an empty row for your own address. In the address {lat} and {lon} are the latitude and longitude in degrees, {zoom} is the map scale by the view."));
            ui.add_space(4.0);
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                egui::Grid::new("planetx-menu-links").num_columns(3).striped(true).spacing([8.0, 4.0]).show(ui, |ui| {
                    ui.label("");
                    ui.strong(tr(lang, "Name"));
                    ui.strong(tr(lang, "Address"));
                    ui.end_row();
                    for ((name, template), mark) in self.rows.iter_mut().zip(self.marked.iter_mut()) {
                        ui.checkbox(mark, "");
                        ui.add(egui::TextEdit::singleline(name).desired_width(160.0));
                        ui.add(egui::TextEdit::singleline(template).desired_width(480.0));
                        ui.end_row();
                    }
                });
            });
            ui.horizontal(|ui| {
                let mut add = None;
                ui.menu_button(tr(lang, "Add site"), |ui| {
                    for (ru, en, template) in data::menulinks::PRESETS {
                        let name = if lang == Lang::RU { ru } else { en };
                        if ui.button(name).clicked() {
                            add = Some((name, template));
                        }
                    }
                    ui.separator();
                    if ui.button(tr(lang, "Own address")).clicked() {
                        add = Some(("", "https://"));
                    }
                })
                .response
                .on_hover_text(tr(lang, "A ready site from the list or an empty row for your own address."));
                if let Some((name, template)) = add {
                    self.add(name, template);
                }
                if ui.add_enabled(self.marked.contains(&true), egui::Button::new(tr(lang, "Delete"))).on_hover_text(tr(lang, "Delete the selected rows.")).clicked() {
                    let mut keep = self.marked.iter().map(|m| !m);
                    self.rows.retain(|_| keep.next().unwrap_or(true));
                    self.marked = vec![false; self.rows.len()];
                }
            });
            if let Some(s) = &self.status {
                ui.colored_label(crate::theme::pal().accent, s);
            }
            ui.horizontal(|ui| {
                if ui.button(tr(lang, "OK")).clicked() {
                    done = Some(true);
                }
                if ui.button(tr(lang, "Cancel")).clicked() {
                    done = Some(false);
                }
            });
        });
        match done {
            Some(true) => {
                let items = self.items();
                let bad = items.iter().enumerate().find_map(|(n, (name, template))| data::menulinks::check(name, template).map(|r| (n, r)));
                if let Some((n, reason)) = bad {
                    self.status = Some(trf(lang, "Row {n}: {reason}", &[("n", &(n + 1).to_string()), ("reason", reason_text(lang, reason))]));
                    return (None, open);
                }
                (Some(items), false)
            }
            Some(false) => (None, false),
            None => (None, open),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_rows_are_dropped() {
        let mut w = LinksWindow::new(vec![("OSM".into(), " https://osm.org/{lat}/{lon} ".into())]);
        w.add("", "");
        assert_eq!(w.items(), vec![("OSM".to_string(), "https://osm.org/{lat}/{lon}".to_string())]);
    }
}
