// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Шкалы карт в левом нижнем углу вида одной панелью, как `LegendPanel`
//! (`ui/legend.py`) модуля: шкала карты значений - полоса цветов
//! с делениями, шкала карты классами - строки с цветом класса. Крестик
//! шкалы убирает карту с глобуса, решение автора модуля от 7 октября
//! 2026 года. Размеры - как у модуля.

use data::rastermaps::{self, Reading};
use eframe::egui;

use crate::i18n::{Lang, tr};
use crate::maps::Kind;

/// Длина полосы шкалы.
const BAR_WIDTH: f32 = 180.0;
const BAR_HEIGHT: f32 = 9.0;
const PAD: f32 = 6.0;
/// Высота строки текста.
const LINE: f32 = 16.0;
/// Сторона крестика.
const CLOSE: f32 = 14.0;
const FONT: f32 = 12.0;

/// Шкала одной карты.
enum Scale {
    Bar { title: &'static str, stops: &'static [rastermaps::Stop], ticks: &'static [f64] },
    Classes { title: &'static str, rows: Vec<([u8; 3], &'static str)> },
    /// Тема GIBS: название с датой и шкала цветовой карты, если пришла.
    Theme { title: String, map: Option<data::gibs::Colormap> },
}

/// Шкала темы: ключ темы, название с датой, цветовая карта.
pub type ThemeScale = (&'static str, String, Option<data::gibs::Colormap>);

/// Квадрат класса темы, столбец и строк в столбце, как `ThemeLegend` модуля.
const SWATCH: f32 = 10.0;
const COLUMN: f32 = 170.0;
const ROWS: usize = 9;

fn scale_of(key: &str, kind: Kind) -> Option<Scale> {
    match kind {
        Kind::Field(f) => Some(Scale::Bar { title: crate::mapvalue::title(key), stops: f.stops, ticks: f.ticks }),
        Kind::Map(m) => match m.reading {
            Reading::Classes(classes) => {
                Some(Scale::Classes { title: crate::mapvalue::title(key), rows: classes.iter().map(|(code, c)| (*c, crate::mapvalue::class_name(code))).collect() })
            }
            _ => None,
        },
        _ => None,
    }
}

impl Scale {
    fn height(&self) -> f32 {
        match self {
            Scale::Bar { .. } => LINE * 2.0 + BAR_HEIGHT + 2.0 + PAD,
            Scale::Classes { rows, .. } => LINE * (rows.len() as f32 + 2.0) + PAD,
            Scale::Theme { map, .. } => match map {
                Some(m) if m.kind != "classification" => LINE + PAD + LINE + BAR_HEIGHT + 2.0,
                Some(m) => LINE + PAD + m.names.len().min(ROWS) as f32 * LINE,
                None => LINE + PAD,
            },
        }
    }
}

/// Место значения на шкале от 0 до 1, `share` модуля.
fn share(stops: &[rastermaps::Stop], value: f64) -> f32 {
    let (Some(low), Some(high)) = (stops.first(), stops.last()) else {
        return 0.0;
    };
    if high.0 <= low.0 {
        return 0.0;
    }
    ((value - low.0) / (high.0 - low.0)).clamp(0.0, 1.0) as f32
}

fn color(c: [u8; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(c[0], c[1], c[2])
}

/// Панель шкал карт maps над подписью источников. credits - место
/// подписи. Возвращает ключ карты, чей крестик нажат.
pub fn show(ctx: &egui::Context, lang: Lang, view: egui::Rect, credits: Option<egui::Rect>, maps: &[(&'static str, Kind)], theme: Option<ThemeScale>) -> Option<&'static str> {
    let mut scales: Vec<(&'static str, Scale)> = Vec::new();
    if let Some((key, title, map)) = theme {
        scales.push((key, Scale::Theme { title, map }));
    }
    scales.extend(maps.iter().filter_map(|(k, kind)| scale_of(k, *kind).map(|s| (*k, s))));
    if scales.is_empty() {
        return None;
    }
    let p = crate::theme::pal();
    let font = egui::FontId::proportional(FONT);
    let mut width = BAR_WIDTH + 2.0 * PAD + 8.0;
    let measure = |text: &str| ctx.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font.clone(), p.plate_text).size().x);
    for (_, s) in &scales {
        if let Scale::Theme { title, map } = s {
            width = width.max(measure(title) + 2.0 * PAD + CLOSE + 6.0);
            if let Some(m) = map.as_ref().filter(|m| m.kind == "classification") {
                let columns = m.names.len().div_ceil(ROWS) as f32;
                width = width.max(columns * COLUMN + 2.0 * PAD);
            }
        }
        if let Scale::Classes { title, rows } = s {
            let texts = std::iter::once(tr(lang, title)).chain(std::iter::once(tr(lang, "Click - the class at the point"))).chain(rows.iter().map(|r| tr(lang, r.1)));
            let longest = texts.map(measure).fold(0.0f32, f32::max);
            width = width.max(longest + PAD + 26.0 + PAD + 16.0);
        }
    }
    let height: f32 = scales.iter().map(|(_, s)| s.height()).sum::<f32>() + PAD;
    // Подпись источников под панелью: панель выше неё, как у модуля.
    let mut bottom = view.bottom() - crate::credits::MARGIN;
    if let Some(c) = credits
        && c.left() < view.left() + crate::credits::MARGIN + width
    {
        bottom = c.top() - crate::credits::MARGIN / 2.0;
    }
    let rect = egui::Rect::from_min_size(egui::pos2(view.left() + crate::credits::MARGIN, bottom - height), egui::vec2(width, height));
    let mut closed = None;
    egui::Area::new(egui::Id::unique("planetx-legends")).order(egui::Order::Foreground).fixed_pos(rect.min).show(ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(rect.size(), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 3.0, p.plate);
        let mut top = rect.top() + PAD / 2.0;
        for (key, scale) in &scales {
            let section = egui::Rect::from_min_size(egui::pos2(rect.left(), top), egui::vec2(width, scale.height()));
            let close = egui::Rect::from_min_size(egui::pos2(section.right() - CLOSE - 3.0, section.top() + 3.0), egui::vec2(CLOSE, CLOSE));
            let x = section.left() + PAD;
            let mut y = section.top() + PAD / 2.0;
            match scale {
                Scale::Bar { title, stops, ticks } => {
                    painter.text(egui::pos2(x, y), egui::Align2::LEFT_TOP, tr(lang, title), font.clone(), p.plate_text);
                    y += LINE + 1.0;
                    let bar = egui::Rect::from_min_size(egui::pos2(x + 4.0, y), egui::vec2(BAR_WIDTH, BAR_HEIGHT));
                    // Полоса - отрезками по одному пикселю между точками шкалы.
                    let steps = BAR_WIDTH as usize;
                    for i in 0..steps {
                        let t = (i as f64 + 0.5) / steps as f64;
                        let (low, high) = (stops.first().map_or(0.0, |s| s.0), stops.last().map_or(1.0, |s| s.0));
                        let c = rastermaps::color(&rastermaps::Field { stops, opacity: 1.0, ..rastermaps::FIELDS[0] }, Some(low + (high - low) * t));
                        let r = egui::Rect::from_min_size(egui::pos2(bar.left() + i as f32, bar.top()), egui::vec2(1.0, BAR_HEIGHT));
                        painter.rect_filled(r, 0.0, egui::Color32::from_rgb(c[0], c[1], c[2]));
                    }
                    let signed = stops.first().is_some_and(|s| s.0 < 0.0);
                    for tick in ticks.iter() {
                        let tx = bar.left() + share(stops, *tick) * BAR_WIDTH;
                        painter.line_segment([egui::pos2(tx, bar.bottom()), egui::pos2(tx, bar.bottom() + 2.0)], egui::Stroke::new(1.0, p.plate_text));
                        let text = if *tick != 0.0 && signed { format!("{tick:+}") } else { format!("{tick}") };
                        painter.text(egui::pos2(tx, bar.bottom() + 2.0), egui::Align2::CENTER_TOP, text, font.clone(), p.plate_text);
                    }
                }
                Scale::Theme { title, map } => {
                    painter.text(egui::pos2(x, y), egui::Align2::LEFT_TOP, title.as_str(), font.clone(), p.plate_text);
                    y += LINE + 1.0;
                    match map {
                        Some(m) if m.kind != "classification" => {
                            let bar = egui::Rect::from_min_size(egui::pos2(x + 4.0, y), egui::vec2(BAR_WIDTH, BAR_HEIGHT));
                            let n = m.colors.len().max(1);
                            for i in 0..BAR_WIDTH as usize {
                                let at = (i as f32 + 0.5) / BAR_WIDTH * (n.saturating_sub(1)) as f32;
                                let (a, b) = (at.floor() as usize, (at.floor() as usize + 1).min(n - 1));
                                let f = at - at.floor();
                                let (ca, cb) = (m.colors.get(a).copied().unwrap_or([0; 3]), m.colors.get(b).copied().unwrap_or([0; 3]));
                                let mix = |k: usize| (f32::from(ca[k]) + (f32::from(cb[k]) - f32::from(ca[k])) * f) as u8;
                                let r = egui::Rect::from_min_size(egui::pos2(bar.left() + i as f32, bar.top()), egui::vec2(1.0, BAR_HEIGHT));
                                painter.rect_filled(r, 0.0, egui::Color32::from_rgb(mix(0), mix(1), mix(2)));
                            }
                            // Подпись, которая налезла бы на прежнюю, пропускается.
                            let mut right = f32::NEG_INFINITY;
                            for (share, text) in &m.labels {
                                let tx = bar.left() + *share as f32 * BAR_WIDTH;
                                let w = measure(text);
                                let left = (tx - w / 2.0).clamp(section.left() + 1.0, section.right() - w - 1.0);
                                if left < right + 4.0 {
                                    continue;
                                }
                                right = left + w;
                                painter.line_segment([egui::pos2(tx, bar.bottom()), egui::pos2(tx, bar.bottom() + 2.0)], egui::Stroke::new(1.0, p.plate_text));
                                painter.text(egui::pos2(left, bar.bottom() + 2.0), egui::Align2::LEFT_TOP, text, font.clone(), p.plate_text);
                            }
                        }
                        Some(m) => {
                            for (n, (c, name)) in m.colors.iter().zip(&m.names).enumerate() {
                                let cx = x + (n / ROWS) as f32 * COLUMN;
                                let cy = y + (n % ROWS) as f32 * LINE;
                                let sw = egui::Rect::from_min_size(egui::pos2(cx, cy + (LINE - SWATCH) / 2.0), egui::vec2(SWATCH, SWATCH));
                                painter.rect(sw, 0.0, color(*c), egui::Stroke::new(1.0, p.muted), egui::StrokeKind::Inside);
                                let clip = painter.with_clip_rect(egui::Rect::from_min_size(egui::pos2(cx + SWATCH + 5.0, cy), egui::vec2(COLUMN - SWATCH - 10.0, LINE)));
                                clip.text(egui::pos2(cx + SWATCH + 5.0, cy), egui::Align2::LEFT_TOP, name, font.clone(), p.plate_text);
                            }
                        }
                        None => {}
                    }
                }
                Scale::Classes { title, rows } => {
                    painter.text(egui::pos2(x, y), egui::Align2::LEFT_TOP, tr(lang, title), font.clone(), p.plate_text);
                    y += LINE;
                    for (c, name) in rows {
                        painter.circle_filled(egui::pos2(x + 9.0, y + LINE / 2.0), 5.0, color(*c));
                        painter.text(egui::pos2(x + 26.0, y), egui::Align2::LEFT_TOP, tr(lang, name), font.clone(), p.plate_text);
                        y += LINE;
                    }
                    painter.text(egui::pos2(x, y), egui::Align2::LEFT_TOP, tr(lang, "Click - the class at the point"), font.clone(), p.muted);
                }
            }
            // Крестик: щелчок убирает карту.
            let response = ui.interact(close, egui::Id::unique(("legend-close", *key)), egui::Sense::click());
            let painter = ui.painter();
            painter.circle_filled(close.center(), CLOSE / 2.0, if response.hovered() { p.hover } else { p.plate });
            // Знака ✕ в шрифтах нет: крестик - две линии.
            let r = close.shrink(4.0);
            let stroke = egui::Stroke::new(1.3, p.plate_text);
            painter.line_segment([r.left_top(), r.right_bottom()], stroke);
            painter.line_segment([r.right_top(), r.left_bottom()], stroke);
            if response.on_hover_text(tr(lang, "Remove from the globe")).clicked() {
                closed = Some(*key);
            }
            top += scale.height();
        }
    });
    closed
}
