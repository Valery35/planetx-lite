// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Профиль высот окном: график высоты по расстоянию и итоги. Перенос
//! `ui/elevation.py` модуля (`ProfileChart`, `ProfileDialog`).
//!
//! Точки вдоль линии и высоты - `scene::measure` (`sample_line`,
//! `profile`), уклон меряется на отрезке [`measure::SLOPE_PIXELS`]
//! пикселей тайла высот, как у модуля. Высоты - настоящие, без масштаба
//! рельефа, ниже уровня моря - дно ([`terrain::HeightStore::floor_heights_at`]).
//! Курсор над графиком показывает точку пути на глобусе. Вода ниже
//! уровня моря - от кривой дна до нуля, пунктир уровня моря с подписью,
//! вода своих водоёмов - до их уровня, как у модуля.

use eframe::egui;
use scene::measure::{self, Profile};

use crate::i18n::{Lang, tr, trf};

/// Жёлтый, как линия линейки.
pub const LINE: egui::Color32 = egui::Color32::from_rgb(255, 214, 0);
pub const FILL: egui::Color32 = egui::Color32::from_rgba_premultiplied(70, 60, 0, 70);
pub const WATER: egui::Color32 = egui::Color32::from_rgba_premultiplied(14, 42, 74, 90);
/// Линия уровня воды.
pub const WATER_LINE: egui::Color32 = egui::Color32::from_rgb(30, 90, 170);
/// Число и единица не разрываются переносом.
const NBSP: char = '\u{a0}';

/// Шаг делений оси: 1, 2 или 5 на степень десяти, около count делений.
pub fn nice_step(span: f64, count: f64) -> f64 {
    if span <= 0.0 || !span.is_finite() {
        return 1.0;
    }
    let raw = span / count;
    let power = 10f64.powf(raw.log10().floor());
    for factor in [1.0, 2.0, 5.0, 10.0] {
        if raw <= factor * power {
            return factor * power;
        }
    }
    10.0 * power
}

fn unit(lang: Lang, value: String, km: bool) -> String {
    trf(lang, if km { "{value} km" } else { "{value} m" }, &[("value", &value)]).replace(' ', &NBSP.to_string())
}

/// Расстояние для итогов: от 100 км - целые километры, от 1 км -
/// десятые, меньше - целые метры.
pub fn distance_text(lang: Lang, metres: f64) -> String {
    if metres >= 100_000.0 {
        unit(lang, format!("{:.0}", metres / 1000.0), true)
    } else if metres >= 1000.0 {
        unit(lang, format!("{:.1}", metres / 1000.0), true)
    } else {
        unit(lang, format!("{metres:.0}"), false)
    }
}

/// Подпись деления оси расстояний: знаков после точки столько, сколько
/// нужно шагу делений.
pub fn axis_text(lang: Lang, metres: f64, step: f64) -> String {
    if step >= 1000.0 {
        let digits = if step % 1000.0 == 0.0 { 0 } else { 1 };
        unit(lang, format!("{:.*}", digits, metres / 1000.0), true)
    } else {
        unit(lang, format!("{metres:.0}"), false)
    }
}

pub fn height_text(lang: Lang, metres: f64) -> String {
    unit(lang, format!("{metres:.0}"), false)
}

pub fn percent(value: f64) -> String {
    format!("{:.1}{NBSP}%", 100.0 * value)
}

/// Профиль по точкам линии: выборка, высоты, окно уклона.
/// heights_at - настоящие высоты точек.
pub fn of_line(points: &[(f64, f64)], heights_at: &measure::HeightsAt<'_>) -> Option<Profile> {
    if points.len() < 2 {
        return None;
    }
    let latlon = measure::sample_line(points, measure::PROFILE_POINTS);
    let level = measure::height_level(points, measure::PROFILE_POINTS);
    let lat = latlon.iter().map(|p| p.0).sum::<f64>() / latlon.len().max(1) as f64;
    let window = measure::SLOPE_PIXELS * measure::pixel_size(lat, level);
    Some(measure::profile(&latlon, &heights_at(&latlon), window))
}

/// Окно профиля: название пути и его точки.
#[derive(Debug, Clone)]
pub struct ProfileWindow {
    pub title: String,
    pub points: Vec<(f64, f64)>,
    /// Точка пути под курсором графика: широта, долгота.
    pub hovered: Option<(f64, f64)>,
    /// Профиль линейки: точки идут за ней.
    pub ruler: bool,
}

impl ProfileWindow {
    /// Окно. ready - все тайлы высот на месте. false в open - закрыли.
    /// lakes - свои водоёмы с батиметрией.
    pub fn window(&mut self, ctx: &egui::Context, lang: Lang, heights_at: &measure::HeightsAt<'_>, lakes: &[data::bathymetry::Lake], ready: bool, open: &mut bool) {
        let title = if self.title.is_empty() {
            tr(lang, "Elevation profile").to_string()
        } else {
            trf(lang, "Elevation profile: {name}", &[("name", &self.title)])
        };
        let profile = of_line(&self.points, heights_at);
        self.hovered = None;
        egui::Window::new(title)
            .id(egui::Id::unique("planetx-profile"))
            .open(open)
            .collapsible(false)
            .default_size([560.0, 260.0])
            .pivot(egui::Align2::CENTER_BOTTOM)
            .default_pos(ctx.content_rect().center_bottom() - egui::vec2(0.0, 40.0))
            .show(ctx, |ui| {
                let Some(p) = profile.as_ref().filter(|p| p.distance.len() >= 2) else {
                    ui.label(tr(lang, "Mark at least two points on the globe."));
                    return;
                };
                let water = data::bathymetry::profile_water(&p.latlon, &p.height, lakes);
                self.hovered = chart(ui, lang, p, &water);
                let mut text = trf(
                    lang,
                    "Map length {flat}, ground length {ground}. Lowest height {low}, highest {high}. Gain {gain}, loss {loss}. Mean slope {mean}, steepest {max}.",
                    &[
                        ("flat", &distance_text(lang, p.flat)),
                        ("ground", &distance_text(lang, p.ground)),
                        ("low", &height_text(lang, p.low)),
                        ("high", &height_text(lang, p.high)),
                        ("gain", &height_text(lang, p.gain)),
                        ("loss", &height_text(lang, p.loss)),
                        ("mean", &percent(p.mean_slope)),
                        ("max", &percent(p.max_slope)),
                    ],
                );
                if !ready {
                    text.push(' ');
                    text.push_str(tr(lang, "Heights are still loading, the numbers will be refined."));
                }
                ui.label(text);
            });
    }
}

/// Область между кривой профиля и уровнем level на точках from..=to:
/// вершины кривой не выше уровня, как вода модуля.
fn water_mesh(line: &[egui::Pos2], level_y: f32, from: usize, to: usize, color: egui::Color32) -> egui::Mesh {
    let mut mesh = egui::Mesh::default();
    for point in &line[from..=to] {
        mesh.colored_vertex(egui::pos2(point.x, point.y.max(level_y)), color);
        mesh.colored_vertex(egui::pos2(point.x, level_y), color);
    }
    for i in 0..(to - from) as u32 {
        let k = 2 * i;
        mesh.add_triangle(k, k + 1, k + 2);
        mesh.add_triangle(k + 1, k + 3, k + 2);
    }
    mesh
}

/// Пунктир от a до b.
fn dashed(painter: &egui::Painter, a: egui::Pos2, b: egui::Pos2) {
    painter.extend(egui::Shape::dashed_line(&[a, b], egui::Stroke::new(1.2, WATER_LINE), 6.0, 4.0));
}

/// График профиля. water - уровень своих водоёмов в точках или NaN.
/// Возвращает точку пути под курсором.
fn chart(ui: &mut egui::Ui, lang: Lang, p: &Profile, water: &[f64]) -> Option<(f64, f64)> {
    let width = ui.available_width().max(200.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 180.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let text_color = ui.visuals().text_color();
    let font = egui::FontId::proportional(11.0);
    let plot = egui::Rect::from_min_max(rect.min + egui::vec2(52.0, 8.0), rect.max - egui::vec2(8.0, 20.0));
    let (x0, x1) = (0.0, p.flat.max(1.0));
    // Ниже уровня моря в шкалу входит ноль - видна вода над дном,
    // уровень своего водоёма тоже входит.
    let mut high = if p.low < 0.0 { p.high.max(0.0) } else { p.high };
    high = water.iter().copied().filter(|w| w.is_finite()).fold(high, f64::max);
    let pad = ((high - p.low) * 0.08).max(1.0);
    let (y0, y1) = (p.low - pad, high + pad);
    let to_screen = |d: f64, h: f64| {
        egui::pos2(
            plot.left() + ((d - x0) / (x1 - x0)) as f32 * plot.width(),
            plot.bottom() - ((h - y0) / (y1 - y0)) as f32 * plot.height(),
        )
    };
    let grid = egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(128, 128, 128, 90));
    let step = nice_step(y1 - y0, 4.0);
    let mut h = (y0 / step).ceil() * step;
    while h <= y1 {
        let y = to_screen(x0, h).y;
        painter.line_segment([egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)], grid);
        painter.text(egui::pos2(plot.left() - 4.0, y), egui::Align2::RIGHT_CENTER, height_text(lang, h), font.clone(), text_color);
        h += step;
    }
    let step = nice_step(x1 - x0, 5.0);
    let mut d = 0.0;
    while d <= x1 {
        let x = to_screen(d, y0).x;
        painter.line_segment([egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())], grid);
        painter.text(egui::pos2(x, plot.bottom() + 2.0), egui::Align2::CENTER_TOP, axis_text(lang, d, step), font.clone(), text_color);
        d += step;
    }
    let line: Vec<egui::Pos2> = p.distance.iter().zip(&p.height).map(|(d, h)| to_screen(*d, *h)).collect();
    let mut mesh = egui::Mesh::default();
    for point in &line {
        mesh.colored_vertex(*point, FILL);
        mesh.colored_vertex(egui::pos2(point.x, plot.bottom()), FILL);
    }
    for i in 0..line.len().saturating_sub(1) as u32 {
        let k = 2 * i;
        mesh.add_triangle(k, k + 1, k + 2);
        mesh.add_triangle(k + 1, k + 3, k + 2);
    }
    painter.add(egui::Shape::mesh(mesh));
    if p.low < 0.0 && line.len() >= 2 {
        // Ниже уровня моря - вода от кривой до нуля. Сухие впадины ниже
        // нуля тоже заливаются, высоты не отличают их от моря.
        let sea = to_screen(x0, 0.0).y;
        painter.add(egui::Shape::mesh(water_mesh(&line, sea, 0, line.len() - 1, WATER)));
        dashed(&painter, egui::pos2(plot.left(), sea), egui::pos2(plot.right(), sea));
        painter.text(egui::pos2(plot.right() - 4.0, sea - 2.0), egui::Align2::RIGHT_BOTTOM, tr(lang, "sea level"), font.clone(), WATER_LINE);
    }
    // Свои водоёмы - на каждом отрезке подряд мокрых точек.
    let mut start: Option<usize> = None;
    for i in 0..=water.len().min(line.len()) {
        let wet = water.get(i).is_some_and(|w| w.is_finite()) && i < line.len();
        if wet {
            start.get_or_insert(i);
            continue;
        }
        let Some(first) = start.take() else {
            continue;
        };
        let last = i - 1;
        let level = water[first..=last].iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let y = to_screen(x0, level).y;
        painter.add(egui::Shape::mesh(water_mesh(&line, y, first, last, WATER)));
        let (a, b) = (egui::pos2(line[first].x, y), egui::pos2(line[last].x, y));
        dashed(&painter, a, b);
        if b.x - a.x > 120.0 {
            painter.text(egui::pos2(b.x - 4.0, y - 2.0), egui::Align2::RIGHT_BOTTOM, trf(lang, "water level {level}", &[("level", &height_text(lang, level))]), font.clone(), WATER_LINE);
        }
    }
    painter.add(egui::Shape::line(line, egui::Stroke::new(1.6, LINE)));
    let pos = response.hover_pos().filter(|p| plot.contains(*p))?;
    let target = x0 + f64::from((pos.x - plot.left()) / plot.width()) * (x1 - x0);
    let i = p.distance.partition_point(|d| *d < target).min(p.distance.len() - 1);
    let point = to_screen(p.distance[i], p.height[i]);
    painter.line_segment([egui::pos2(point.x, plot.top()), egui::pos2(point.x, plot.bottom())], egui::Stroke::new(1.0, egui::Color32::from_rgb(220, 60, 40)));
    painter.circle_filled(point, 3.5, egui::Color32::from_rgb(220, 60, 40));
    let text = trf(
        lang,
        "{distance}, height {height}, slope {slope}",
        &[("distance", &distance_text(lang, p.distance[i])), ("height", &height_text(lang, p.height[i])), ("slope", &percent(slope(p, i)))],
    );
    painter.text(plot.right_top() + egui::vec2(-4.0, 2.0), egui::Align2::RIGHT_TOP, text, font, text_color);
    p.latlon.get(i).copied()
}

/// Уклон в точке i на окне профиля, как `_slope` модуля.
fn slope(p: &Profile, i: usize) -> f64 {
    let n = p.distance.len();
    let target = p.distance[i] + p.window;
    let j = p.distance.partition_point(|d| *d < target).max(i + 1).min(n - 1);
    let k = if j > i { i } else { i.saturating_sub(1) };
    let run = p.distance[j] - p.distance[k];
    if run > 0.0 { (p.height[j] - p.height[k]).abs() / run } else { 0.0 }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{axis_text, distance_text, nice_step, of_line, percent};
    use crate::i18n::Lang;

    #[test]
    fn steps_and_texts() {
        assert_eq!(nice_step(4.47, 1.0), 5.0);
        assert_eq!(nice_step(2.0, 1.0), 2.0);
        assert_eq!(nice_step(1000.0, 5.0), 200.0);
        assert_eq!(nice_step(0.0, 5.0), 1.0);
        assert_eq!(distance_text(Lang::EN, 123_456.0), "123\u{a0}km");
        assert_eq!(distance_text(Lang::EN, 1234.0), "1.2\u{a0}km");
        assert_eq!(distance_text(Lang::EN, 999.4), "999\u{a0}m");
        assert_eq!(axis_text(Lang::EN, 2500.0, 500.0), "2500\u{a0}m");
        assert_eq!(axis_text(Lang::EN, 4000.0, 2000.0), "4\u{a0}km");
        assert_eq!(percent(0.1234), "12.3\u{a0}%");
    }

    #[test]
    fn ramp_profile() {
        let end = scene::measure::destination(58.0, 56.0, 0.0, 1000.0);
        let points = [(58.0, 56.0), end];
        let p = of_line(&points, &|pts: &[(f64, f64)]| pts.iter().map(|q| (q.0 - 58.0) / (end.0 - 58.0) * 100.0).collect()).unwrap();
        assert!((p.gain - 100.0).abs() < 1e-6);
        assert!((p.mean_slope - 0.1).abs() < 0.002);
        assert!(of_line(&points[..1], &|_: &[(f64, f64)]| Vec::new()).is_none());
    }
}
