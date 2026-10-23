// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Знак PlanetX и подпись источников в правом нижнем углу, как в окне
//! глобуса модуля (`ui/window.py`, `ui/snapshot.py`).
//!
//! В самом углу - знак PlanetX: значок шара с орбитами и название
//! светлым текстом с тенью, фон прозрачный. Левее - подпись источников
//! неярким светлым текстом с тенью, без плашки, как у Google Earth.
//! Подпись - короткие названия одной строкой ([`short_credits`]), вся
//! строка открывает окно «Источники данных» с полными подписями
//! и ссылками. Решения автора модуля от 16 и 17 октября 2026 года.

use eframe::egui;

use crate::i18n::{Lang, tr};

/// Отступ знака и подписи от края кадра, логические пиксели.
pub const MARGIN: f32 = 8.0;
/// Высота знака PlanetX и доля высоты под название.
pub const BRAND_HEIGHT: f32 = 26.0;
pub const BRAND_TEXT: f32 = 0.62;
/// Название в знаке.
pub const BRAND: &str = "PlanetX";
/// Кегль подписи источников, логические пиксели.
pub const ATTRIBUTION_FONT: f32 = 11.0;
/// Цвет подписи: неяркий светлый, #e4e4e4, непрозрачность 215 из 255.
pub const CREDIT_COLOR: egui::Color32 = egui::Color32::from_rgba_premultiplied(192, 192, 192, 215);
/// Тень текста.
pub const SHADOW: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 0, 0, 170);

/// Значок PlanetX, картинка модуля `planetx/icon.png` правообладателя
/// ООО «Информ++».
const ICON_PNG: &[u8] = include_bytes!("../assets/planetx.png");

/// Подпись источника высот, как в модуле: название наборов данных
/// и ссылка на их перечень.
pub const TERRAIN: (&str, &str) =
    ("Terrain: Mapzen, SRTM, GMTED, ETOPO1 and others", "https://github.com/tilezen/joerd/blob/master/docs/attribution.md");

/// Строки подписи шириной не больше room: перенос только между
/// источниками, источник не разрывается. width - ширина текста.
pub fn break_lines(pieces: &[String], room: f32, mut width: impl FnMut(&str) -> f32) -> String {
    let mut lines: Vec<String> = Vec::new();
    for piece in pieces {
        match lines.last_mut() {
            Some(last) if width(&format!("{last} · {piece}")) <= room => {
                last.push_str(" · ");
                last.push_str(piece);
            }
            _ => lines.push(piece.clone()),
        }
    }
    lines.join("\n")
}

/// Короткая подпись источников на карте: названия без ссылок, лицензий
/// и списков авторов, повтор один раз. Перенос
/// `core/sources.short_credits` модуля. credits - подписи текстом
/// по порядку, первые keep (подложка и рельеф) остаются целиком,
/// у прочих снимается метка вида «Gravity: » и остаётся первое название
/// до запятой. Части через «;» - отдельные источники. Подпись с «©»
/// остаётся целой, условия OpenStreetMap требуют её как есть.
pub fn short_credits(credits: &[&str], keep: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (n, credit) in credits.iter().enumerate() {
        for chunk in credit.split(" · ").map(str::trim).filter(|c| !c.is_empty()) {
            let found: Vec<String> = if n < keep {
                vec![chunk.to_string()]
            } else {
                chunk
                    .split(';')
                    .filter_map(|piece| {
                        let piece = piece.trim();
                        let piece = match piece.split_once(": ") {
                            Some((head, rest)) if !head.contains(',') => rest,
                            _ => piece,
                        };
                        let piece = piece.trim();
                        let piece = if piece.contains('©') { piece } else { piece.split(',').next().unwrap_or("").trim() };
                        (!piece.is_empty()).then(|| piece.to_string())
                    })
                    .collect()
            };
            for piece in found {
                if !out.contains(&piece) {
                    out.push(piece);
                }
            }
        }
    }
    out
}

/// Значок окна программы - тот же шар с орбитами. Значок egui
/// по умолчанию автор счёл неподходящим, 22 октября 2026 года.
pub fn window_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png).ok()?.to_rgba8();
    Some(egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() })
}

/// Знак, подпись и окна «Источники данных» и «О программе».
#[derive(Default)]
pub struct Credits {
    icon: Option<egui::TextureHandle>,
    /// Место знака и подписи на экране прошлого кадра.
    pub rect: Option<egui::Rect>,
    sources_open: bool,
    about_open: bool,
    /// Окно «О программе» попросила открыть панель инструментов.
    pub about_request: bool,
}

/// Текст со светлым цветом и тёмной тенью, как `_shadow_text` модуля.
fn shadow_text(painter: &egui::Painter, pos: egui::Pos2, galley: &std::sync::Arc<egui::Galley>, color: egui::Color32) {
    for d in [egui::vec2(1.0, 1.0), egui::vec2(-0.5, 0.5)] {
        painter.galley_with_override_text_color(pos + d, galley.clone(), SHADOW);
    }
    painter.galley_with_override_text_color(pos, galley.clone(), color);
}

impl Credits {
    fn icon(&mut self, ctx: &egui::Context) -> Option<egui::TextureId> {
        if self.icon.is_none() {
            let img = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png).ok()?.to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            self.icon = Some(ctx.load_texture("planetx-icon", color, egui::TextureOptions::LINEAR));
        }
        self.icon.as_ref().map(egui::TextureHandle::id)
    }

    /// Знак и подпись в правом нижнем углу. parts - подписи источников
    /// с ссылками по порядку, первые keep остаются целиком. view - место
    /// вида в окне.
    pub fn show(&mut self, ctx: &egui::Context, view: egui::Rect, lang: Lang, parts: &[(&str, &str)], keep: usize) {
        let icon = self.icon(ctx);
        let texts: Vec<&str> = parts.iter().map(|(t, _)| *t).collect();
        let pieces = short_credits(&texts, keep);
        // Ширина вида без боковых панелей.
        let screen = view;
        let mut open_sources = false;
        let mut open_about = false;
        // Место знака и строки: их прямоугольники, а не вся область.
        let mut used = egui::Rect::NOTHING;
        egui::Area::new(egui::Id::unique("planetx-credits"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-MARGIN, -MARGIN))
            .show(ctx, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Max), |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(MARGIN / 2.0, 0.0);
                    // Знак PlanetX: значок и название.
                    let h = BRAND_HEIGHT;
                    let name = ui.painter().layout_no_wrap(
                        BRAND.to_string(),
                        egui::FontId::proportional(h * BRAND_TEXT),
                        egui::Color32::WHITE,
                    );
                    let gap = h * 0.18;
                    let size = egui::vec2(h + gap + name.size().x + 2.0, h);
                    let (rect, brand) = ui.allocate_exact_size(size, egui::Sense::click());
                    used = used.union(rect);
                    if let Some(id) = icon {
                        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                        ui.painter().image(id, egui::Rect::from_min_size(rect.min, egui::vec2(h, h)), uv, egui::Color32::WHITE);
                    }
                    let at = egui::pos2(rect.min.x + h + gap, rect.center().y - name.size().y / 2.0);
                    shadow_text(ui.painter(), at, &name, egui::Color32::WHITE);
                    if brand.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        open_about = true;
                    }
                    // Подпись источников левее знака, переносится по ширине.
                    if !pieces.is_empty() {
                        let room = (screen.width() - size.x - 3.0 * MARGIN).max(80.0);
                        let font = egui::FontId::proportional(ATTRIBUTION_FONT);
                        let painter = ui.painter();
                        let text = break_lines(&pieces, room, |s| painter.layout_no_wrap(s.to_string(), font.clone(), CREDIT_COLOR).size().x);
                        let mut job = egui::text::LayoutJob::simple(text, font, CREDIT_COLOR, f32::INFINITY);
                        job.halign = egui::Align::RIGHT;
                        let galley = ui.painter().layout_job(job);
                        let (rect, credit) = ui.allocate_exact_size(galley.size(), egui::Sense::click());
                        used = used.union(rect);
                        // Строки выровнены вправо от правого края места.
                        let pos = egui::pos2(rect.max.x, rect.min.y);
                        shadow_text(ui.painter(), pos, &galley, CREDIT_COLOR);
                        let tip = tr(lang, "Sources of what is on the globe. Click to open the Data sources window.");
                        if credit.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked() {
                            open_sources = true;
                        }
                    }
                });
            });
        self.rect = used.is_positive().then(|| used.expand(MARGIN / 2.0));
        self.sources_open |= open_sources;
        self.about_open |= open_about;
        self.about_open |= std::mem::take(&mut self.about_request);
        self.windows(ctx, lang, parts);
    }

    fn windows(&mut self, ctx: &egui::Context, lang: Lang, parts: &[(&str, &str)]) {
        let (sources, about) = (tr(lang, "Data sources"), tr(lang, "About"));
        egui::Window::new(sources).open(&mut self.sources_open).collapsible(false).resizable(false).show(ctx, |ui| {
            for (text, link) in parts {
                if link.is_empty() {
                    ui.label(*text);
                } else {
                    ui.hyperlink_to(*text, *link);
                }
            }
        });
        egui::Window::new(about).open(&mut self.about_open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.heading(format!("PlanetX Lite {}", env!("CARGO_PKG_VERSION")));
            ui.label(tr(lang, "The PlanetX globe without QGIS."));
            ui.label(tr(lang, "© 2026 Inform++ LLC. Apache 2.0."));
            ui.hyperlink("https://github.com/Valery35/planetx-lite");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{break_lines, short_credits};

    #[test]
    fn credits_break_only_between_sources() {
        let pieces: Vec<String> = ["Esri", "Terrain", "© OpenStreetMap contributors"].map(String::from).to_vec();
        let width = |s: &str| s.chars().count() as f32;
        assert_eq!(break_lines(&pieces, 100.0, width), "Esri · Terrain · © OpenStreetMap contributors");
        // Источник не разрывается, на новую строку переходит целиком.
        assert_eq!(break_lines(&pieces, 20.0, width), "Esri · Terrain\n© OpenStreetMap contributors");
    }

    #[test]
    fn short_credits_keep_first_and_copyright() {
        let credits = [
            "Esri, Vantor, Earthstar Geographics, and the GIS User Community",
            "Terrain: Mapzen, SRTM, GMTED, ETOPO1 and others",
            "OpenFreeMap © OpenMapTiles, © OpenStreetMap contributors",
            "Gravity: ICGEM, GFZ Potsdam; Clouds: NASA GIBS, EOSDIS",
            "OpenFreeMap © OpenMapTiles, © OpenStreetMap contributors",
        ];
        assert_eq!(
            short_credits(&credits, 2),
            [
                "Esri, Vantor, Earthstar Geographics, and the GIS User Community",
                "Terrain: Mapzen, SRTM, GMTED, ETOPO1 and others",
                "OpenFreeMap © OpenMapTiles, © OpenStreetMap contributors",
                "ICGEM",
                "NASA GIBS",
            ]
        );
        assert!(short_credits(&["", " · "], 1).is_empty());
    }
}
