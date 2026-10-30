// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Старые карты «Это Место»»: каталог карт сайта etomesto.com
//! у точки, перенос `ui/etomesto.py` модуля.
//!
//! Пункт «Старые карты «Это Место»…» меню на глобусе. Окно одно,
//! новая точка заменяет прежнюю. Список - год и название, строка поиска
//! отбирает по словам. Щелчок по названию открывает страницу карты
//! браузером, программа карт сайта не показывает.

use eframe::egui;

use crate::i18n::{Lang, tr, trf};

/// Наибольший ответ списка, байт.
const ANSWER_BYTES: u64 = 4 << 20;

pub struct EtomestoWindow {
    point: (f64, f64),
    items: Vec<data::etomesto::Item>,
    ask: Option<Box<dyn net::Request>>,
    error: Option<String>,
    search: String,
}

impl EtomestoWindow {
    /// Окно и запрос списка у точки.
    pub fn new(lat: f64, lon: f64, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Self {
        let ask = request(ANSWER_BYTES);
        ask.start(&data::etomesto::list_url(lat, lon));
        Self { point: (lat, lon), items: Vec::new(), ask: Some(ask), error: None, search: String::new() }
    }

    /// Окно. Возвращает, открыто ли оно.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang) -> bool {
        if let Some(ask) = &self.ask
            && let Some(answer) = ask.take()
        {
            self.ask = None;
            match answer {
                Ok(data) => {
                    self.items = data::etomesto::parse_list(&data);
                    // От старых карт к новым, как список сайта.
                    self.items.sort_by_key(|i| i.year.unwrap_or(i32::MIN));
                }
                Err(e) => self.error = Some(e),
            }
        }
        if self.ask.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let mut open = true;
        let mut page = None;
        egui::Window::new(tr(lang, "Eto Mesto old maps")).open(&mut open).collapsible(false).default_size([560.0, 520.0]).show(ctx, |ui| {
            ui.label(tr(lang, "Maps of the Eto Mesto site (etomesto.com) that cover the point. A map opens on the site, where it can be overlaid on a modern map and downloaded under the site rules."));
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(tr(lang, "Search by title or year")).desired_width(f32::INFINITY))
                .on_hover_text(tr(lang, "Keeps the maps whose title or year holds all the words."));
            let shown: Vec<&data::etomesto::Item> = self.items.iter().filter(|i| data::etomesto::matches(i, &self.search)).collect();
            egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
                egui::Grid::new("planetx-etomesto").num_columns(2).striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
                    ui.strong(tr(lang, "Year"));
                    ui.strong(tr(lang, "Name"));
                    ui.end_row();
                    for item in &shown {
                        ui.label(item.year.map_or(String::new(), |y| y.to_string()));
                        if ui.link(&item.title).on_hover_text(tr(lang, "Opens the page of the chosen map on etomesto.com in the browser.")).clicked() {
                            page = Some(data::etomesto::page_url(&item.name, self.point.0, self.point.1));
                        }
                        ui.end_row();
                    }
                });
            });
            let status = if self.ask.is_some() {
                tr(lang, "Asking Eto Mesto…").to_string()
            } else if let Some(e) = &self.error {
                trf(lang, "The site did not answer: {error}", &[("error", e)])
            } else if self.items.is_empty() {
                tr(lang, "The site has no maps at this point.").to_string()
            } else {
                trf(lang, "Maps at the point {total}, shown {shown}. A click on a title opens the map on the site.", &[("total", &self.items.len().to_string()), ("shown", &shown.len().to_string())])
            };
            ui.label(status);
        });
        if let Some(url) = page {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
        open
    }
}
