// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Шкалы слоёв в левом нижнем углу вида одной панелью, как `LegendPanel`
//! (`ui/legend.py`) модуля: полоса цветов с делениями, строки с цветом
//! класса, шкала темы GIBS. Крестик шкалы убирает слой с глобуса,
//! решение автора модуля от 7 октября 2026 года. У шкалы глубины
//! землетрясений крестика нет, как у модуля. Размеры - как у модуля.

use data::rastermaps::{self, Reading};
use eframe::egui;

use crate::dots::Layer;
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
/// Квадрат класса темы, столбец и строк в столбце, как `ThemeLegend` модуля.
const SWATCH: f32 = 10.0;
const COLUMN: f32 = 170.0;
const ROWS: usize = 9;

/// Шкала одного слоя.
enum Scale {
    /// Полоса цветов: заголовок, точки шкалы, деления, шкала
    /// в логарифме значения.
    Bar { title: &'static str, stops: &'static [rastermaps::Stop], ticks: &'static [f64], log: bool },
    /// Строки с кружком цвета: заголовок, строки, подвал.
    Rows { title: &'static str, rows: Vec<([u8; 3], &'static str)>, footer: &'static str, lines: bool },
    /// Тема GIBS: название с датой и шкала цветовой карты, если пришла.
    Theme { title: String, map: Option<data::gibs::Colormap> },
    /// Горизонтали: сечение в середине вида и цвет линии.
    Contours { step: f64, color: [u8; 3] },
    /// Клетки цветов с подписями: уклон (подпись у левого края клетки)
    /// или экспозиция (подпись посередине), `SlopeLegend` модуля.
    Cells { title: &'static str, cells: Vec<([u8; 3], &'static str)>, centered: bool },
}

/// Шкала темы: ключ темы, название с датой, цветовая карта.
pub type ThemeScale = (&'static str, String, Option<data::gibs::Colormap>);

/// Шкала карты картинками.
fn scale_of(key: &str, kind: Kind) -> Option<Scale> {
    match kind {
        Kind::Field(f) => Some(Scale::Bar { title: crate::mapvalue::title(key), stops: f.stops, ticks: f.ticks, log: false }),
        Kind::Map(m) => match m.reading {
            Reading::Classes(classes) => Some(Scale::Rows {
                title: crate::mapvalue::title(key),
                rows: classes.iter().map(|(code, c)| (*c, crate::mapvalue::class_name(code))).collect(),
                footer: "Click - the class at the point",
                lines: false,
            }),
            _ => None,
        },
        Kind::Terrain(crate::maps::Analysis::Slope) => Some(Scale::Cells {
            title: "Slope, °",
            cells: data::slope::SLOPE_STOPS.iter().zip(["0", "2", "5", "10", "15", "25", "35"]).map(|(s, l)| (s.1, l)).collect(),
            centered: false,
        }),
        Kind::Terrain(crate::maps::Analysis::Aspect) => {
            // Стороны света - с контекстом: «N» и «E» уже заняты подписями координат.
            let names = ["N|compass", "NE|compass", "E|compass", "SE|compass", "S|compass", "SW|compass", "W|compass", "NW|compass"];
            let mut cells = vec![(data::slope::FLAT_COLOR, "")];
            cells.extend(data::slope::ASPECT_STOPS.iter().zip(names).map(|(s, n)| (s.1, n)));
            Some(Scale::Cells { title: "Aspect, flat is grey", cells, centered: true })
        }
        _ => None,
    }
}

/// Шкала слоя точек или линий, `QuakeLegend`, `VolcanoLegend`,
/// `FireLegend` и `FaultLegend` модуля. У границ плит и кабелей шкалы нет.
fn dots_scale(layer: Layer) -> Option<Scale> {
    Some(match layer {
        Layer::Quakes => Scale::Bar { title: "Focus depth, km", stops: &data::quakes::DEPTH_STOPS, ticks: &data::quakes::DEPTH_TICKS, log: false },
        Layer::Fires => Scale::Bar { title: "Fire power, MW", stops: &data::fires::FRP_STOPS, ticks: &data::fires::FRP_TICKS, log: true },
        Layer::Volcanoes => {
            let names = ["Erupted since 1900", "Erupted in 1500-1899", "Erupted before 1500", "Eruption date unknown"];
            Scale::Rows {
                title: "Holocene volcanoes, GVP",
                rows: data::volcanoes::PERIODS.iter().zip(names).map(|(p, n)| (p.2, n)).collect(),
                footer: "Dot size - volcano height",
                lines: false,
            }
        }
        Layer::Faults => Scale::Rows {
            title: "GEM active faults",
            rows: data::faults::KINDS.iter().map(|k| {
                let c = data::faults::color(k);
                ([c[0], c[1], c[2]], crate::dots::fault_name(k))
            }).collect(),
            footer: "A click on a line - details",
            lines: true,
        },
        Layer::Plates | Layer::Cables => return None,
    })
}

impl Scale {
    fn height(&self) -> f32 {
        match self {
            Scale::Bar { .. } => LINE * 2.0 + BAR_HEIGHT + 2.0 + PAD,
            Scale::Rows { rows, footer, .. } => LINE * (rows.len() as f32 + 1.0 + if footer.is_empty() { 0.0 } else { 1.0 }) + PAD,
            Scale::Theme { map, .. } => match map {
                Some(m) if m.kind != "classification" => LINE + PAD + LINE + BAR_HEIGHT + 2.0,
                Some(m) => LINE + PAD + m.names.len().min(ROWS) as f32 * LINE,
                None => LINE + PAD,
            },
            Scale::Contours { .. } => 2.0 * LINE + PAD,
            Scale::Cells { .. } => LINE * 2.0 + BAR_HEIGHT + 2.0 + PAD,
        }
    }
}

/// Значение шкалы для сравнения: само или его логарифм.
fn value(v: f64, log: bool) -> f64 {
    if log { v.max(1e-9).log10() } else { v }
}

/// Место значения на шкале от 0 до 1, `share` модуля.
fn share(stops: &[rastermaps::Stop], v: f64, log: bool) -> f32 {
    let (Some(low), Some(high)) = (stops.first(), stops.last()) else {
        return 0.0;
    };
    let (low, high) = (value(low.0, log), value(high.0, log));
    if high <= low {
        return 0.0;
    }
    ((value(v, log) - low) / (high - low)).clamp(0.0, 1.0) as f32
}

/// Цвет шкалы в доле t от 0 до 1, между точками линейно.
fn color_at(stops: &[rastermaps::Stop], t: f32, log: bool) -> egui::Color32 {
    let positions: Vec<f32> = stops.iter().map(|s| share(stops, s.0, log)).collect();
    let mut c = stops.last().map_or([0; 3], |s| s.1);
    for (n, pair) in stops.windows(2).enumerate() {
        let (a, b) = (positions[n], positions[n + 1]);
        if t <= b {
            let f = if b > a { ((t - a) / (b - a)).clamp(0.0, 1.0) } else { 0.0 };
            c = [0, 1, 2].map(|k| (f32::from(pair[0].1[k]) + (f32::from(pair[1].1[k]) - f32::from(pair[0].1[k])) * f).round() as u8);
            break;
        }
    }
    if t <= 0.0
        && let Some(first) = stops.first()
    {
        c = first.1;
    }
    color(c)
}

fn color(c: [u8; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(c[0], c[1], c[2])
}

/// Слои со шкалами: карты, тема GIBS, слои точек, горизонтали -
/// сечение в середине вида и цвет линии.
pub struct Shown<'a> {
    pub maps: &'a [(&'static str, Kind)],
    pub theme: Option<ThemeScale>,
    pub dots: &'a [Layer],
    pub contours: Option<(f64, [u8; 3])>,
}

/// Что убрать с глобуса крестиком шкалы.
pub enum Closed {
    Map(&'static str),
    Dots(Layer),
}

/// Панель шкал над подписью источников: тема, карты maps, слои точек
/// dots. credits - место подписи. Возвращает слой, чей крестик нажат.
pub fn show(
    ctx: &egui::Context,
    lang: Lang,
    view: egui::Rect,
    credits: Option<egui::Rect>,
    shown: Shown<'_>,
) -> Option<Closed> {
    let Shown { maps, theme, dots, contours } = shown;
    // Шкала, что убирает крестик, и признак крестика.
    let mut scales: Vec<(Option<Closed>, Scale)> = Vec::new();
    if let Some((key, title, map)) = theme {
        scales.push((Some(Closed::Map(key)), Scale::Theme { title, map }));
    }
    // У шкалы уклона крестика нет, как у модуля.
    scales.extend(maps.iter().filter_map(|(k, kind)| scale_of(k, *kind).map(|s| ((!matches!(s, Scale::Cells { .. })).then_some(Closed::Map(k)), s))));
    if let Some((step, color)) = contours {
        scales.push((Some(Closed::Map("contours")), Scale::Contours { step, color }));
    }
    scales.extend(dots.iter().filter_map(|l| Some(((*l != Layer::Quakes).then_some(Closed::Dots(*l)), dots_scale(*l)?))));
    if scales.is_empty() {
        return None;
    }
    let p = crate::theme::pal();
    let font = egui::FontId::proportional(FONT);
    let mut width = BAR_WIDTH + 2.0 * PAD + 8.0;
    let measure = |text: &str| ctx.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font.clone(), p.plate_text).size().x);
    for (_, s) in &scales {
        match s {
            Scale::Theme { title, map } => {
                width = width.max(measure(title) + 2.0 * PAD + CLOSE + 6.0);
                if let Some(m) = map.as_ref().filter(|m| m.kind == "classification") {
                    width = width.max(m.names.len().div_ceil(ROWS) as f32 * COLUMN + 2.0 * PAD);
                }
            }
            Scale::Rows { title, rows, footer, .. } => {
                let texts = [tr(lang, title), tr(lang, footer)].into_iter().chain(rows.iter().map(|r| tr(lang, r.1)));
                let longest = texts.filter(|t| !t.is_empty()).map(measure).fold(0.0f32, f32::max);
                width = width.max(longest + PAD + 26.0 + PAD + 16.0);
            }
            Scale::Contours { step, .. } => {
                let text = contours_text(lang, *step);
                width = width.max(PAD + 30.0 + measure(&text) + PAD + CLOSE);
            }
            Scale::Bar { .. } | Scale::Cells { .. } => {}
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
        ui.painter().rect_filled(rect, 3.0, p.plate);
        let mut top = rect.top() + PAD / 2.0;
        for (n, (close_what, scale)) in scales.into_iter().enumerate() {
            let painter = ui.painter();
            let section = egui::Rect::from_min_size(egui::pos2(rect.left(), top), egui::vec2(width, scale.height()));
            let x = section.left() + PAD;
            let mut y = section.top() + PAD / 2.0;
            let text = |pos: egui::Pos2, s: &str, c: egui::Color32| painter.text(pos, egui::Align2::LEFT_TOP, s, font.clone(), c);
            match &scale {
                Scale::Bar { title, stops, ticks, log } => {
                    text(egui::pos2(x, y), tr(lang, title), p.plate_text);
                    y += LINE + 1.0;
                    let bar = egui::Rect::from_min_size(egui::pos2(x + 4.0, y), egui::vec2(BAR_WIDTH, BAR_HEIGHT));
                    // Полоса - отрезками по одному пикселю.
                    for i in 0..BAR_WIDTH as usize {
                        let t = (i as f32 + 0.5) / BAR_WIDTH;
                        let r = egui::Rect::from_min_size(egui::pos2(bar.left() + i as f32, bar.top()), egui::vec2(1.0, BAR_HEIGHT));
                        painter.rect_filled(r, 0.0, color_at(stops, t, *log));
                    }
                    let signed = stops.first().is_some_and(|s| s.0 < 0.0);
                    for tick in ticks.iter() {
                        let tx = bar.left() + share(stops, *tick, *log) * BAR_WIDTH;
                        painter.line_segment([egui::pos2(tx, bar.bottom()), egui::pos2(tx, bar.bottom() + 2.0)], egui::Stroke::new(1.0, p.plate_text));
                        let label = if *tick != 0.0 && signed { format!("{tick:+}") } else { format!("{tick}") };
                        let w = measure(&label);
                        let left = (tx - w / 2.0).clamp(section.left(), section.right() - w);
                        text(egui::pos2(left, bar.bottom() + 2.0), &label, p.plate_text);
                    }
                }
                Scale::Theme { title, map } => {
                    text(egui::pos2(x, y), title, p.plate_text);
                    y += LINE + 1.0;
                    match map {
                        Some(m) if m.kind != "classification" => {
                            let bar = egui::Rect::from_min_size(egui::pos2(x + 4.0, y), egui::vec2(BAR_WIDTH, BAR_HEIGHT));
                            let count = m.colors.len().max(1);
                            for i in 0..BAR_WIDTH as usize {
                                let at = (i as f32 + 0.5) / BAR_WIDTH * count.saturating_sub(1) as f32;
                                let (a, b) = (at.floor() as usize, (at.floor() as usize + 1).min(count - 1));
                                let f = at - at.floor();
                                let (ca, cb) = (m.colors.get(a).copied().unwrap_or([0; 3]), m.colors.get(b).copied().unwrap_or([0; 3]));
                                let mix = |k: usize| (f32::from(ca[k]) + (f32::from(cb[k]) - f32::from(ca[k])) * f) as u8;
                                let r = egui::Rect::from_min_size(egui::pos2(bar.left() + i as f32, bar.top()), egui::vec2(1.0, BAR_HEIGHT));
                                painter.rect_filled(r, 0.0, egui::Color32::from_rgb(mix(0), mix(1), mix(2)));
                            }
                            // Подпись, которая налезла бы на прежнюю, пропускается.
                            let mut right = f32::NEG_INFINITY;
                            for (at, label) in &m.labels {
                                let tx = bar.left() + *at as f32 * BAR_WIDTH;
                                let w = measure(label);
                                let left = (tx - w / 2.0).clamp(section.left() + 1.0, section.right() - w - 1.0);
                                if left < right + 4.0 {
                                    continue;
                                }
                                right = left + w;
                                painter.line_segment([egui::pos2(tx, bar.bottom()), egui::pos2(tx, bar.bottom() + 2.0)], egui::Stroke::new(1.0, p.plate_text));
                                text(egui::pos2(left, bar.bottom() + 2.0), label, p.plate_text);
                            }
                        }
                        Some(m) => {
                            for (i, (c, name)) in m.colors.iter().zip(&m.names).enumerate() {
                                let cx = x + (i / ROWS) as f32 * COLUMN;
                                let cy = y + (i % ROWS) as f32 * LINE;
                                let sw = egui::Rect::from_min_size(egui::pos2(cx, cy + (LINE - SWATCH) / 2.0), egui::vec2(SWATCH, SWATCH));
                                painter.rect(sw, 0.0, color(*c), egui::Stroke::new(1.0, p.muted), egui::StrokeKind::Inside);
                                let clip = painter.with_clip_rect(egui::Rect::from_min_size(egui::pos2(cx + SWATCH + 5.0, cy), egui::vec2(COLUMN - SWATCH - 10.0, LINE)));
                                clip.text(egui::pos2(cx + SWATCH + 5.0, cy), egui::Align2::LEFT_TOP, name, font.clone(), p.plate_text);
                            }
                        }
                        None => {}
                    }
                }
                Scale::Cells { title, cells, centered } => {
                    text(egui::pos2(x, y), tr(lang, title), p.plate_text);
                    y += LINE + 1.0;
                    let w = BAR_WIDTH / cells.len().max(1) as f32;
                    for (n, (c, label)) in cells.iter().enumerate() {
                        let cx = x + 4.0 + n as f32 * w;
                        painter.rect_filled(egui::Rect::from_min_size(egui::pos2(cx, y), egui::vec2(w, BAR_HEIGHT)), 0.0, color(*c));
                        if !label.is_empty() {
                            let label = tr(lang, label);
                            let left = if *centered { cx + (w - measure(label)) / 2.0 } else { cx };
                            text(egui::pos2(left, y + BAR_HEIGHT + 2.0), label, p.plate_text);
                        }
                    }
                }
                Scale::Contours { step, color: c } => {
                    text(egui::pos2(x, y), tr(lang, "Contours"), p.plate_text);
                    y += LINE;
                    let mid = y + LINE / 2.0;
                    painter.line_segment([egui::pos2(x, mid), egui::pos2(x + 24.0, mid)], egui::Stroke::new(2.0, color(*c)));
                    text(egui::pos2(x + 30.0, y), &contours_text(lang, *step), p.plate_text);
                }
                Scale::Rows { title, rows, footer, lines } => {
                    text(egui::pos2(x, y), tr(lang, title), p.plate_text);
                    y += LINE;
                    for (c, name) in rows {
                        let mid = y + LINE / 2.0;
                        if *lines {
                            // Отрезок линии толщиной 2, «line» строк модуля.
                            painter.line_segment([egui::pos2(x + 2.0, mid), egui::pos2(x + 18.0, mid)], egui::Stroke::new(2.0, color(*c)));
                        } else {
                            // Кружок с тёмным ободком, `darker(250)` модуля.
                            let rim = egui::Color32::from_rgb(c[0] / 5 * 2, c[1] / 5 * 2, c[2] / 5 * 2);
                            painter.circle(egui::pos2(x + 10.0, mid), 4.0, color(*c), egui::Stroke::new(1.0, rim));
                        }
                        text(egui::pos2(x + 26.0, y), tr(lang, name), p.plate_text);
                        y += LINE;
                    }
                    if !footer.is_empty() {
                        text(egui::pos2(x, y), tr(lang, footer), p.plate_text);
                    }
                }
            }
            if let Some(what) = close_what {
                // Крестик: щелчок убирает слой. Знака ✕ в шрифтах нет -
                // крестик из двух линий.
                let close = egui::Rect::from_min_size(egui::pos2(section.right() - CLOSE - 3.0, section.top() + 3.0), egui::vec2(CLOSE, CLOSE));
                let response = ui.interact(close, egui::Id::unique(("legend-close", n)), egui::Sense::click());
                let painter = ui.painter();
                painter.circle_filled(close.center(), CLOSE / 2.0, if response.hovered() { p.hover } else { p.plate });
                let r = close.shrink(4.0);
                let stroke = egui::Stroke::new(1.3, p.plate_text);
                painter.line_segment([r.left_top(), r.right_bottom()], stroke);
                painter.line_segment([r.right_top(), r.left_bottom()], stroke);
                if response.on_hover_text(tr(lang, "Remove from the globe")).clicked() {
                    closed = Some(what);
                }
            }
            top += scale.height();
        }
    });
    closed
}

/// Сечение горизонталей словами, `ContourLegend` модуля.
fn contours_text(lang: Lang, step: f64) -> String {
    let index = data::contours::index_step(step);
    crate::i18n::trf(lang, "every {step} m, thicker every {index} m", &[("step", &data::contours::label_text(step)), ("index", &data::contours::label_text(index))])
}
