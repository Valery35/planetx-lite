// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Батиметрия водоёма»: что в растре и уровень воды. Перенос
//! `BathymetryDialog` (`ui/bathymetry.py`) модуля.
//!
//! Пункт «Батиметрия водоёма…» у своего растра в меню строки «Рельеф».
//! Растр ложится в рельеф дном водоёма, глубины - отметками дна, над ним -
//! вода на уровне водоёма. У модуля уровень по умолчанию - высота
//! рельефа в середине растра. В Lite растр уже свой рельеф, высота
//! в середине - его же значение, поэтому уровень берётся по рельефу
//! у края растра.

use data::bathymetry::Kind;
use eframe::egui;

use crate::i18n::{Lang, tr, trf};

/// Итог окна.
pub enum Answer {
    /// Растр - батиметрия: что в нём и уровень воды.
    Set(Kind, f64),
    /// Растр перестаёт быть рельефом глобуса.
    Remove,
    Cancel,
}

/// Окно батиметрии растра name.
pub struct Dialog {
    pub name: String,
    kind: Kind,
    level: f64,
    /// Батиметрия у растра уже есть: кнопка «Убрать» доступна.
    set: bool,
}

impl Dialog {
    /// Окно с прежними настройками setting или уровнем по умолчанию.
    pub fn new(name: &str, setting: Option<(Kind, f64)>, level: f64) -> Dialog {
        let (kind, level) = setting.unwrap_or((Kind::Depth, level));
        Dialog { name: name.to_string(), kind, level, set: setting.is_some() }
    }

    /// Тело окна в панели свойств. Возвращает итог, когда окно закрыли.
    pub fn body(&mut self, ui: &mut egui::Ui, lang: Lang) -> Option<Answer> {
        let mut answer = None;
        {
            ui.label(trf(
                lang,
                "The raster \"{name}\" goes into the globe terrain as the bed of the water body, water is drawn above it at the water body level. The default level is the terrain height at the edge of the raster, for a lake it is usually the water surface.",
                &[("name", &self.name)],
            ));
            ui.add_space(6.0);
            ui.radio_value(&mut self.kind, Kind::Depth, tr(lang, "Values are depths"))
                .on_hover_text(tr(lang, "The depth below the water surface, with a plus or minus sign. The bed elevation is the water level minus the depth."));
            ui.radio_value(&mut self.kind, Kind::Bed, tr(lang, "Values are bed elevations"))
                .on_hover_text(tr(lang, "The bed height above sea level, like the terrain. The water level is needed only for the water surface."));
            ui.horizontal(|ui| {
                ui.label(tr(lang, "Water level"));
                ui.add(egui::DragValue::new(&mut self.level).range(-500.0..=9000.0).speed(0.1).fixed_decimals(2).suffix(tr(lang, " m")))
                    .on_hover_text(tr(lang, "The height of the water surface above sea level. The water is drawn at this height over all points where the bed is lower."));
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(tr(lang, "OK")).clicked() {
                    answer = Some(Answer::Set(self.kind, self.level));
                }
                if ui.button(tr(lang, "Cancel")).clicked() {
                    answer = Some(Answer::Cancel);
                }
                if ui
                    .add_enabled(self.set, egui::Button::new(tr(lang, "Remove bathymetry")))
                    .on_hover_text(tr(lang, "The raster is no longer the globe terrain, no water is drawn over it."))
                    .clicked()
                {
                    answer = Some(Answer::Remove);
                }
            });
        }
        answer
    }
}
