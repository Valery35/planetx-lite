// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Шрифт интерфейса - системный, как в RustGIS (`render/src/fonts.rs`):
//! Segoe UI, SF, Inter, Noto Sans, Open Sans, DejaVu Sans, что найдётся
//! первым. Заголовки - полужирным начертанием того же семейства. Без
//! них и на странице остаётся встроенный шрифт egui. Шрифты читаются
//! при запуске и в программу не входят. Встроенные шрифты egui остаются
//! запасными для знаков, которых нет в системном.

use eframe::egui;

/// Файлы (обычный, полужирный) по порядку предпочтения.
#[cfg(not(target_arch = "wasm32"))]
const CANDIDATES: &[(&str, &str)] = &[
    ("C:\\Windows\\Fonts\\segoeui.ttf", "C:\\Windows\\Fonts\\seguisb.ttf"),
    ("/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/SFNS.ttf"),
    ("/System/Library/Fonts/Supplemental/Arial.ttf", "/System/Library/Fonts/Supplemental/Arial Bold.ttf"),
    ("/usr/share/fonts/TTF/Inter-Regular.ttf", "/usr/share/fonts/TTF/Inter-SemiBold.ttf"),
    ("/usr/share/fonts/inter/Inter-Regular.ttf", "/usr/share/fonts/inter/Inter-SemiBold.ttf"),
    ("/usr/share/fonts/noto/NotoSans-Regular.ttf", "/usr/share/fonts/noto/NotoSans-SemiBold.ttf"),
    ("/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf", "/usr/share/fonts/truetype/noto/NotoSans-SemiBold.ttf"),
    ("/usr/share/fonts/google-noto/NotoSans-Regular.ttf", "/usr/share/fonts/google-noto/NotoSans-SemiBold.ttf"),
    ("/usr/share/fonts/TTF/OpenSans-Regular.ttf", "/usr/share/fonts/TTF/OpenSans-SemiBold.ttf"),
    ("/usr/share/fonts/truetype/open-sans/OpenSans-Regular.ttf", "/usr/share/fonts/truetype/open-sans/OpenSans-SemiBold.ttf"),
    ("/usr/share/fonts/TTF/DejaVuSans.ttf", "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf"),
    ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"),
];

/// Наибольший файл шрифта, байт.
pub const MAX_FONT: u64 = 32 << 20;

/// Вес полужирного у переменного шрифта, как в RustGIS. Шрифт без оси
/// веса его не замечает.
pub const SEMIBOLD_WEIGHT: f32 = 600.0;

/// Семейство заголовков.
pub const SEMIBOLD: &str = "semibold";

/// Семейство подписей на глобусе - встроенный шрифт egui.
pub const GLOBE: &str = "globe";

/// Шрифт подписей на глобусе размером size.
pub fn globe(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(GLOBE.into()))
}

/// Байты годны как шрифт: разбираются тем же skrifa, что у egui.
pub fn is_font(bytes: &[u8]) -> bool {
    skrifa::FontRef::new(bytes).is_ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn read(path: &str) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_FONT {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    is_font(&bytes).then_some(bytes)
}

/// Системные шрифты (обычный, полужирный), если нашлись.
#[cfg(not(target_arch = "wasm32"))]
fn system() -> Option<(Vec<u8>, Vec<u8>)> {
    // PLANETX_BUILTIN_FONTS - встроенный шрифт egui, как у RustGIS.
    if std::env::var_os("PLANETX_BUILTIN_FONTS").is_some() {
        return None;
    }
    CANDIDATES.iter().find_map(|(r, b)| {
        let regular = read(r)?;
        let semibold = read(b).unwrap_or_else(|| regular.clone());
        Some((regular, semibold))
    })
}

/// На странице файлов шрифтов нет.
#[cfg(target_arch = "wasm32")]
fn system() -> Option<(Vec<u8>, Vec<u8>)> {
    None
}

/// Поставить шрифты интерфейса и заголовки полужирным.
pub fn install(ctx: &egui::Context) {
    use std::sync::Arc;

    use egui::{FontData, FontDefinitions, FontFamily};

    let mut fonts = FontDefinitions::default();
    // Подписи на глобусе - встроенным шрифтом: при полёте они всё время
    // приносят новые знаки, а Segoe UI рисует новый знак дорого.
    // С ним на сценах «Бриенц» и «Японский жёлоб» кадр замирал больше
    // чем на секунду, 23 октября 2026 года.
    if let Some(prop) = fonts.families.get(&FontFamily::Proportional).cloned() {
        fonts.families.insert(FontFamily::Name(GLOBE.into()), prop);
    }
    let mut semi: Vec<String> = Vec::new();
    if let Some((regular, semibold)) = system() {
        fonts.font_data.insert("ui".into(), Arc::new(FontData::from_owned(regular)));
        let weight = egui::epaint::text::VariationCoords::new([(b"wght", SEMIBOLD_WEIGHT)]);
        let semibold = FontData::from_owned(semibold).tweak(egui::FontTweak { coords: weight, ..Default::default() });
        fonts.font_data.insert("ui-semibold".into(), Arc::new(semibold));
        if let Some(prop) = fonts.families.get_mut(&FontFamily::Proportional) {
            prop.insert(0, "ui".into());
        }
        semi.push("ui-semibold".into());
    }
    if let Some(prop) = fonts.families.get(&FontFamily::Proportional) {
        semi.extend(prop.iter().cloned());
    }
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), semi);
    ctx.set_fonts(fonts);
    ctx.global_style_mut(|s| {
        if let Some(h) = s.text_styles.get_mut(&egui::TextStyle::Heading) {
            h.family = FontFamily::Name(SEMIBOLD.into());
        }
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::is_font;

    #[test]
    fn garbage_is_not_a_font() {
        assert!(!is_font(b"not a font at all"));
        assert!(!is_font(&[]));
        assert!(is_font(epaint_default_fonts::UBUNTU_LIGHT));
    }
}
