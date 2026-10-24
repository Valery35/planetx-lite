// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! «Номенклатура листа»: номера листов карт в точке, перенос
//! `ui/sheets.py` модуля. Подменю на глобусе копирует номер щелчком,
//! окно показывает все системы и масштабы.

use eframe::egui;

use crate::i18n::{Lang, tr, trf};

/// Система разграфки словами.
pub fn system_name(lang: Lang, system: &str) -> &'static str {
    match system {
        "imw" => tr(lang, "International Map of the World (IMW)"),
        "russian" => tr(lang, "Russian sheet designation"),
        "jog" => tr(lang, "NATO, JOG"),
        _ => tr(lang, "MGRS, 100 km square"),
    }
}

/// «1:1 000 000» или пустая строка.
pub fn scale_text(scale: Option<u32>) -> String {
    let Some(scale) = scale else {
        return String::new();
    };
    let digits = scale.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    format!("1:{out}")
}

/// Строки листов в точке на языке программы.
pub fn rows(lang: Lang, lat: f64, lon: f64) -> Vec<data::sheets::Row> {
    let mark = format!(" ({})", tr(lang, "S. H."));
    data::sheets::sheets(lat, lon, &mark)
}

/// Пункты подменю: номер и система с масштабом. Систем без листа нет.
pub fn items(lang: Lang, lat: f64, lon: f64) -> Vec<(String, String)> {
    rows(lang, lat, lon)
        .into_iter()
        .filter_map(|(system, scale, number)| {
            let number = number?;
            let mut label = system_name(lang, system).to_string();
            if scale.is_some() {
                label = format!("{label}, {}", scale_text(scale));
            }
            Some((number, label))
        })
        .collect()
}

/// Окно номеров листов. Окно одно, новая точка заменяет прежнюю.
pub struct SheetsWindow {
    /// Точка словами.
    pub place: String,
    rows: Vec<data::sheets::Row>,
    status: Option<String>,
}

impl SheetsWindow {
    pub fn new(lang: Lang, lat: f64, lon: f64, place: String) -> Self {
        Self { place, rows: rows(lang, lat, lon), status: None }
    }

    /// Окно. Возвращает номер, который скопировали, и открыто ли окно.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang) -> (Option<String>, bool) {
        let mut open = true;
        let mut copied = None;
        egui::Window::new(tr(lang, "Map sheet designation")).open(&mut open).collapsible(false).default_width(520.0).show(ctx, |ui| {
            ui.label(&self.place);
            egui::Grid::new("planetx-sheets").num_columns(3).striped(true).spacing([14.0, 4.0]).show(ui, |ui| {
                ui.strong(tr(lang, "System"));
                ui.strong(tr(lang, "Scale"));
                ui.strong(tr(lang, "Sheet number"));
                ui.end_row();
                for (system, scale, number) in &self.rows {
                    ui.label(system_name(lang, system));
                    ui.label(scale_text(*scale));
                    match number {
                        Some(n) => {
                            if ui.link(n).on_hover_text(tr(lang, "A click on a row copies the sheet number to the clipboard.")).clicked() {
                                copied = Some(n.clone());
                            }
                        }
                        None => {
                            ui.label(tr(lang, "no sheet"));
                        }
                    }
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            let text = self.status.clone().unwrap_or_else(|| {
                tr(lang, "A click on a row copies the sheet number. Sheets of the Russian State Geological Map at 1:1,000,000 and 1:200,000 use the Russian designation too.").to_string()
            });
            ui.label(text);
        });
        if let Some(n) = &copied {
            ctx.copy_text(n.clone());
            self.status = Some(trf(lang, "Copied: {number}", &[("number", n)]));
        }
        (copied, open)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_groups_thousands() {
        assert_eq!(scale_text(Some(1000000)), "1:1 000 000");
        assert_eq!(scale_text(Some(250000)), "1:250 000");
        assert_eq!(scale_text(None), "");
    }

    #[test]
    fn menu_items_skip_missing_sheets() {
        // Москва: все системы дают лист.
        let all = items(Lang::RU, 55.75, 37.62);
        assert_eq!(all.len(), 6);
        assert!(all.iter().any(|(n, _)| n == "N-37"));
    }
}
