// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Настройки, как окно «Свойства вида» модуля (`ui/properties.py`):
//! язык интерфейса, оформление, вертикальный масштаб рельефа, язык
//! подписей на глобусе. Просьба автора от 23 октября 2026 года.
//! Настройки хранятся в хранилище eframe строками «ключ=значение»,
//! чужие и испорченные строки пропускаются.

use eframe::egui;

use crate::basemap::Basemap;
use crate::i18n::{LANGUAGES, Lang, tr};

/// Ключ хранилища.
pub const KEY: &str = "planetx_settings";
/// Вертикальный масштаб рельефа: пределы и шаг, как у модуля.
pub const SCALE_RANGE: (f64, f64) = (0.5, 10.0);
pub const SCALE_STEP: f64 = 0.5;
/// Предел высоты дерева меток при чтении настроек, точки экрана.
pub const PLACES_MAX: f32 = 10000.0;

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
    /// Свет солнца вместо отмывки, строка «Солнце» раздела «Слои».
    pub sun: bool,
    /// Строка «Координатная сетка» раздела «Слои».
    pub grid: bool,
    /// Строка «Атмосфера», у модуля включена по умолчанию.
    pub air: bool,
    /// Флажок строки «Рельеф» раздела «Слои».
    pub relief: bool,
    /// Строки векторной основы, `VectorBase::to_text`.
    pub vector: String,
    /// Формат координат из [`COORD_FORMATS`].
    pub coords: String,
    /// Прежние запросы строки «Поиск», от новых к старым, как у модуля.
    pub history: Vec<String>,
    /// Высота дерева «Моих меток» над разделом «Слои», точки экрана.
    /// 0 - по умолчанию. Граница разделов тянется мышью, как разделитель
    /// панели модуля.
    pub places_height: f32,
    /// Включённые карты картинками, ключи [`crate::maps::ALL`].
    pub maps: Vec<String>,
    /// Ключ основы, `Basemap::key`.
    pub basemap: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self { language: None, labels: String::new(), dark: true, scale: 1.0, sun: false, grid: false, air: true, relief: true, vector: crate::vector::VectorBase::default().to_text(), coords: "decimal".into(), history: Vec::new(), places_height: 0.0, maps: Vec::new(), basemap: "esri".into() }
    }
}

impl Settings {
    /// Язык интерфейса: `PLANETX_LANGUAGE`, выбранный, язык системы.
    pub fn lang(&self) -> Lang {
        Lang::from_env().or_else(|| self.language.as_deref().and_then(Lang::from_code)).unwrap_or_else(Lang::from_system)
    }

    pub fn to_text(&self) -> String {
        format!(
            "language={}\nlabels={}\ntheme={}\nscale={}\nsun={}\ngrid={}\nair={}\nrelief={}\nvector={}\ncoords={}\nplaces_height={}\nmaps={}\nbasemap={}\n",
            self.language.as_deref().unwrap_or(""),
            self.labels,
            if self.dark { "dark" } else { "light" },
            self.scale,
            self.sun,
            self.grid,
            self.air,
            self.relief,
            self.vector,
            self.coords,
            self.places_height,
            self.maps.join(","),
            self.basemap
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
                "air" => s.air = value != "false",
                "relief" => s.relief = value != "false",
                "coords" if COORD_FORMATS.contains(&value) => s.coords = value.to_string(),
                "vector" => s.vector = crate::vector::VectorBase::from_text(value).to_text(),
                "history" if !value.is_empty() && s.history.len() < scene::searchbar::HISTORY => s.history.push(value.to_string()),
                "basemap" if Basemap::from_key(value).is_some() => s.basemap = value.to_string(),
                "maps" => {
                    s.maps = value.split(',').filter(|k| crate::maps::index(k).is_some()).map(str::to_string).collect();
                }
                "places_height" => {
                    if let Ok(v) = value.parse::<f32>()
                        && v.is_finite()
                    {
                        s.places_height = v.clamp(0.0, PLACES_MAX);
                    }
                }
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

/// Окно «Свойства вида», как `ui/properties.py` модуля: группы «Основа»,
/// «Рельеф», «Подписи», «Координаты», «Оформление» и кнопка «Закрыть».
/// Групп QGIS («Карта QGIS», «Обновление»), недр и помощника у Lite нет.
/// Язык интерфейса модуль берёт у QGIS, у Lite он выбирается в группе
/// «Оформление». Меняет settings и basemap на месте. Возвращает,
/// изменилось ли что-то, и нажата ли кнопка «Источники данных…».
pub fn window(ctx: &egui::Context, lang: Lang, open: &mut bool, settings: &mut Settings, basemap: &mut Basemap) -> (bool, bool) {
    let before = (settings.clone(), *basemap);
    let mut sources = false;
    let mut close = false;
    egui::Window::new(tr(lang, "View properties"))
        .open(open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            group(ui, tr(lang, "Base map"), |ui| {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("properties-basemap").selected_text(crate::layers::basemap_title(lang, *basemap)).show_ui(ui, |ui| {
                        for b in Basemap::all() {
                            ui.selectable_value(basemap, b, crate::layers::basemap_title(lang, b));
                        }
                    });
                    if ui.button(tr(lang, "Data sources…")).clicked() {
                        sources = true;
                    }
                });
            });
            group(ui, tr(lang, "Terrain"), |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr(lang, "Vertical exaggeration"));
                    ui.add_enabled(
                        settings.relief,
                        egui::DragValue::new(&mut settings.scale).range(SCALE_RANGE.0..=SCALE_RANGE.1).speed(SCALE_STEP).fixed_decimals(1),
                    )
                    .on_hover_text(tr(lang, "Height multiplier of the terrain. Above 1 mountains and valleys look more pronounced."));
                });
            });
            group(ui, tr(lang, "Labels"), |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr(lang, "Language")).on_hover_text(tr(
                        lang,
                        "Language of place names on the globe. Without a name in this language the Latin or local name is shown.",
                    ));
                    egui::ComboBox::from_id_salt("settings-labels").selected_text(label_name(lang, &settings.labels)).show_ui(ui, |ui| {
                        for code in std::iter::once("").chain(std::iter::once(data::places::LOCAL)).chain(data::places::LABEL_LANGUAGES) {
                            ui.selectable_value(&mut settings.labels, code.to_string(), label_name(lang, code));
                        }
                    });
                });
            });
            group(ui, tr(lang, "Coordinates"), |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr(lang, "Format"));
                    let name = |key: &str| match key {
                        "dms" => tr(lang, "Degrees, minutes, seconds"),
                        "utm" => "UTM",
                        "mgrs" => "MGRS",
                        _ => tr(lang, "Decimal degrees"),
                    };
                    egui::ComboBox::from_id_salt("properties-coords").selected_text(name(&settings.coords)).show_ui(ui, |ui| {
                        for key in COORD_FORMATS {
                            ui.selectable_value(&mut settings.coords, key.to_string(), name(key));
                        }
                    });
                });
            });
            group(ui, tr(lang, "Appearance"), |ui| {
                egui::Grid::new("properties-look").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label(tr(lang, "Appearance"));
                    let shown = if settings.dark { tr(lang, "Dark") } else { tr(lang, "Light") };
                    egui::ComboBox::from_id_salt("properties-theme").selected_text(shown).show_ui(ui, |ui| {
                        ui.selectable_value(&mut settings.dark, false, tr(lang, "Light"));
                        ui.selectable_value(&mut settings.dark, true, tr(lang, "Dark"));
                    });
                    ui.end_row();
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
                });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if ui.button(tr(lang, "Close")).clicked() {
                    close = true;
                }
            });
        });
    if close {
        *open = false;
    }
    ((settings.clone(), *basemap) != before, sources)
}

/// Форматы координат строки состояния и меню глобуса, `FORMATS` модуля.
pub const COORD_FORMATS: [&str; 4] = ["decimal", "dms", "utm", "mgrs"];

/// Группа окна, как QGroupBox модуля: заголовок и рамка.
fn group(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.label(egui::RichText::new(title).strong());
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_width(340.0);
        add(ui);
    });
    ui.add_space(4.0);
}
#[cfg(test)]
mod tests {
    use super::{SCALE_RANGE, Settings};

    #[test]
    fn settings_round_trip_and_hostile_text() {
        let s = Settings { language: Some("de".into()), labels: "local".into(), dark: false, scale: 2.5, sun: true, grid: true, air: false, relief: false, vector: "roads,railways;1".into(), coords: "utm".into(), history: vec!["Вологда".into(), "Paris".into()], places_height: 240.0, maps: vec!["emag".into(), "hiking".into()], basemap: "esri_topo".into() };
        assert_eq!(Settings::from_text(&s.to_text()), s);
        let bad = Settings::from_text("language=xx\nlabels=../x\ntheme=dark\nscale=1e9\nscale=NaN\nfoo=bar\nno equals\nplaces_height=-5\nplaces_height=NaN\nmaps=emag,../x");
        assert_eq!(bad.maps, ["emag"]);
        assert_eq!(bad.places_height, 0.0);
        assert_eq!(bad.language, None);
        assert_eq!(bad.labels, "");
        assert!(bad.dark);
        assert_eq!(bad.scale, SCALE_RANGE.1);
    }
}
