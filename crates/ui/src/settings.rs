// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Настройки, как окно «Свойства вида» модуля (`ui/properties.py`):
//! язык интерфейса, оформление, вертикальный масштаб рельефа, язык
//! подписей на глобусе. Просьба автора от 23 октября 2026 года.
//! Настройки хранятся в хранилище eframe строками «ключ=значение»,
//! чужие и испорченные строки пропускаются.

use eframe::egui;

use crate::i18n::{LANGUAGES, Lang, tr};

/// Ключ хранилища.
pub const KEY: &str = "planetx_settings";
/// Вертикальный масштаб рельефа: пределы и шаг, как у модуля.
pub const SCALE_RANGE: (f64, f64) = (0.5, 10.0);
pub const SCALE_STEP: f64 = 0.5;

/// Языки подписей сверх языков интерфейса: коды OpenMapTiles модуля.
const LABEL_EXTRA: [(&str, &str); 4] = [("pl", "Polski"), ("uk", "Українська"), ("kk", "Қазақша"), ("ar", "العربية")];

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Код языка интерфейса, None - язык системы.
    pub language: Option<String>,
    /// Язык подписей: "" - как интерфейс, "local" - местные, иначе код.
    pub labels: String,
    pub dark: bool,
    /// Вертикальный масштаб рельефа.
    pub scale: f64,
    /// Свет солнца вместо отмывки, как «Солнце» модуля.
    pub sun: bool,
    /// Координатная сетка, как «Сетка» модуля.
    pub grid: bool,
    /// Прежние запросы строки «Поиск», от новых к старым, как у модуля.
    pub history: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { language: None, labels: String::new(), dark: true, scale: 1.0, sun: false, grid: false, history: Vec::new() }
    }
}

impl Settings {
    /// Язык интерфейса: `PLANETX_LANGUAGE`, выбранный, язык системы.
    pub fn lang(&self) -> Lang {
        Lang::from_env().or_else(|| self.language.as_deref().and_then(Lang::from_code)).unwrap_or_else(Lang::from_system)
    }

    pub fn to_text(&self) -> String {
        format!(
            "language={}\nlabels={}\ntheme={}\nscale={}\nsun={}\ngrid={}\n",
            self.language.as_deref().unwrap_or(""),
            self.labels,
            if self.dark { "dark" } else { "light" },
            self.scale,
            self.sun,
            self.grid
        ) + &self.history.iter().map(|h| format!("history={h}\n")).collect::<String>()
    }

    /// Настройки из строки хранилища. Строка считается враждебной:
    /// незнакомые ключи и значения вне пределов пропускаются.
    pub fn from_text(text: &str) -> Self {
        let mut s = Settings::default();
        for (key, value) in text.lines().filter_map(|l| l.split_once('=')) {
            let value = value.trim();
            match key.trim() {
                "language" => s.language = Lang::from_code(value).map(|l| l.code().to_string()),
                "labels" => {
                    if value.is_empty() || value == data::places::LOCAL || data::places::LABEL_LANGUAGES.contains(&value) {
                        s.labels = value.to_string();
                    }
                }
                "theme" => s.dark = value != "light",
                "sun" => s.sun = value == "true",
                "grid" => s.grid = value == "true",
                "history" if !value.is_empty() && s.history.len() < scene::searchbar::HISTORY => s.history.push(value.to_string()),
                "scale" => {
                    if let Ok(v) = value.parse::<f64>()
                        && v.is_finite()
                    {
                        s.scale = v.clamp(SCALE_RANGE.0, SCALE_RANGE.1);
                    }
                }
                _ => {}
            }
        }
        s
    }
}

/// Название языка подписей по коду.
fn label_name(lang: Lang, code: &str) -> String {
    match code {
        "" => tr(lang, "As the interface").to_string(),
        c if c == data::places::LOCAL => tr(lang, "Local names").to_string(),
        c => LANGUAGES.iter().chain(LABEL_EXTRA.iter()).find(|(k, _)| *k == c).map_or(c.to_string(), |(_, n)| n.to_string()),
    }
}

/// Окно «Настройки». Меняет settings на месте, возвращает, изменилось ли
/// что-то.
pub fn window(ctx: &egui::Context, lang: Lang, open: &mut bool, settings: &mut Settings) -> bool {
    let before = settings.clone();
    egui::Window::new(tr(lang, "Settings"))
        .open(open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
        egui::Grid::new("planetx-settings").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label(tr(lang, "Interface language"));
            let current = settings.language.as_deref().and_then(Lang::from_code);
            let shown = current.map_or(tr(lang, "As in the system"), Lang::native_name);
            egui::ComboBox::from_id_salt("settings-language").selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut settings.language, None, tr(lang, "As in the system"));
                for (code, name) in LANGUAGES {
                    ui.selectable_value(&mut settings.language, Some(code.to_string()), name);
                }
            });
            ui.end_row();

            ui.label(tr(lang, "Label language")).on_hover_text(tr(
                lang,
                "Language of place names on the globe. Without a name in this language the Latin or local name is shown.",
            ));
            egui::ComboBox::from_id_salt("settings-labels").selected_text(label_name(lang, &settings.labels)).show_ui(ui, |ui| {
                for code in std::iter::once("").chain(std::iter::once(data::places::LOCAL)).chain(data::places::LABEL_LANGUAGES) {
                    ui.selectable_value(&mut settings.labels, code.to_string(), label_name(lang, code));
                }
            });
            ui.end_row();

            ui.label(tr(lang, "Appearance"));
            ui.horizontal(|ui| {
                ui.selectable_value(&mut settings.dark, true, tr(lang, "Dark"));
                ui.selectable_value(&mut settings.dark, false, tr(lang, "Light"));
            });
            ui.end_row();

            ui.label(tr(lang, "Vertical exaggeration"))
                .on_hover_text(tr(lang, "Height multiplier of the terrain. Above 1 mountains and valleys look more pronounced."));
            ui.add(
                egui::Slider::new(&mut settings.scale, SCALE_RANGE.0..=SCALE_RANGE.1)
                    .step_by(SCALE_STEP)
                    .fixed_decimals(1)
                    .suffix(" ×"),
            );
            ui.end_row();

            ui.label(tr(lang, "View"));
            ui.vertical(|ui| {
                ui.checkbox(&mut settings.sun, tr(lang, "Sun"))
                    .on_hover_text(tr(lang, "Terrain lit by the sun at the moment of the time bar or now. Night is dark."));
                ui.checkbox(&mut settings.grid, tr(lang, "Coordinate grid"))
                    .on_hover_text(tr(lang, "Parallels and meridians with labels, the equator, the tropics and the polar circles."));
            });
            ui.end_row();
        });
    });
    *settings != before
}

#[cfg(test)]
mod tests {
    use super::{SCALE_RANGE, Settings};

    #[test]
    fn settings_round_trip_and_hostile_text() {
        let s = Settings { language: Some("de".into()), labels: "local".into(), dark: false, scale: 2.5, sun: true, grid: true, history: vec!["Вологда".into(), "Paris".into()] };
        assert_eq!(Settings::from_text(&s.to_text()), s);
        let bad = Settings::from_text("language=xx\nlabels=../x\ntheme=dark\nscale=1e9\nscale=NaN\nfoo=bar\nno equals");
        assert_eq!(bad.language, None);
        assert_eq!(bad.labels, "");
        assert!(bad.dark);
        assert_eq!(bad.scale, SCALE_RANGE.1);
    }
}
