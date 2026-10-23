// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Координатная сетка на глобусе, как «Сетка» модуля
//! (`ui/globe/themes.py`, `_update_grid`). Линии и подписи -
//! `scene::graticule`: параллели и меридианы светлые, экватор, тропики
//! и полярные круги - жёлтые, подписи с полушарием на языке интерфейса.
//! Сетка рисуется поверх глобуса в пикселях экрана, точки - на рельефе.

use eframe::egui;
use globe_core::navigation::Navigator;
use scene::graticule::{self, Axis};

use crate::i18n::{Lang, tr, trf};

/// Цвет сетки, `GRID_COLOR` модуля.
pub const GRID_COLOR: egui::Color32 = egui::Color32::from_rgb(220, 220, 220);
/// Цвет особых параллелей, `CIRCLE_COLOR` модуля.
pub const CIRCLE_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 230, 0);
pub const GRID_WIDTH: f32 = 1.0;

/// Подпись линии сетки с полушарием, `_grid_text` модуля.
pub fn line_text(lang: Lang, axis: Axis, value: f64, step: f64) -> String {
    let angle = graticule::angle_text(value, step);
    match axis {
        Axis::Lat if value.abs() < 1e-9 => angle,
        Axis::Lat if value > 0.0 => trf(lang, "{angle} N", &[("angle", &angle)]),
        Axis::Lat => trf(lang, "{angle} S", &[("angle", &angle)]),
        Axis::Lon => {
            let v = graticule::normal_lon(value);
            if v.abs() < 1e-9 || (v.abs() - 180.0).abs() < 1e-9 {
                graticule::angle_text(v.abs(), step)
            } else if v > 0.0 {
                trf(lang, "{angle} E", &[("angle", &graticule::angle_text(v, step))])
            } else {
                trf(lang, "{angle} W", &[("angle", &graticule::angle_text(v, step))])
            }
        }
    }
}

/// Название особой параллели.
pub fn circle_name(lang: Lang, name: &str) -> &'static str {
    match name {
        "equator" => tr(lang, "Equator"),
        "cancer" => tr(lang, "Tropic of Cancer"),
        "capricorn" => tr(lang, "Tropic of Capricorn"),
        "arctic" => tr(lang, "Arctic Circle"),
        _ => tr(lang, "Antarctic Circle"),
    }
}

/// Точка (широта, долгота) на рельефе в логических пикселях окна, если
/// она видна.
fn screen(nav: &Navigator, rect: egui::Rect, ppp: f64, (lat, lon): (f64, f64)) -> Option<egui::Pos2> {
    let h = nav.pose.terrain.as_ref().map_or(0.0, |t| t(lat, lon));
    let point = nav.camera.ellipsoid.geodetic_to_ecef(lat, lon, h);
    let (px, front) = nav.camera.project(point);
    let facing = (nav.camera.eye - point).dot(point.normalize_or_zero()) > 0.0;
    (front && facing).then(|| rect.min + egui::vec2((px.x / ppp) as f32, (px.y / ppp) as f32))
}

/// Сетка поверх глобуса.
pub fn paint(painter: &egui::Painter, rect: egui::Rect, ppp: f64, nav: &Navigator, lang: Lang) {
    let pose = &nav.pose;
    let width = 2.0 * pose.distance * (nav.camera.fov_y.to_radians() / 2.0).tan();
    let (step, lines) = graticule::lines(pose.lat, pose.lon, width);
    let polyline = |points: &[(f64, f64)], color: egui::Color32| {
        let stroke = egui::Stroke::new(GRID_WIDTH, color);
        let mut run: Vec<egui::Pos2> = Vec::new();
        for p in points {
            match screen(nav, rect, ppp, *p) {
                Some(at) => run.push(at),
                None => {
                    if run.len() > 1 {
                        painter.add(egui::Shape::line(std::mem::take(&mut run), stroke));
                    }
                    run.clear();
                }
            }
        }
        if run.len() > 1 {
            painter.add(egui::Shape::line(run, stroke));
        }
    };
    let faint = GRID_COLOR.gamma_multiply(0.6);
    for (_, _, points) in &lines {
        polyline(points, faint);
    }
    for (_, _, points) in graticule::circles(pose.lat, pose.lon, width) {
        polyline(&points, CIRCLE_COLOR);
    }
    let font = egui::FontId::proportional(11.0);
    for (lat, lon, axis, value) in graticule::labels(pose.lat, pose.lon, width) {
        if let Some(at) = screen(nav, rect, ppp, (lat, graticule::normal_lon(lon))) {
            painter.text(at + egui::vec2(3.0, -2.0), egui::Align2::LEFT_BOTTOM, line_text(lang, axis, value, step), font.clone(), GRID_COLOR);
        }
    }
    for (lat, lon, name) in graticule::circle_labels(pose.lat, pose.lon, width) {
        if let Some(at) = screen(nav, rect, ppp, (lat, graticule::normal_lon(lon))) {
            painter.text(at + egui::vec2(0.0, -2.0), egui::Align2::CENTER_BOTTOM, circle_name(lang, name), font.clone(), CIRCLE_COLOR);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::line_text;
    use crate::i18n::Lang;
    use scene::graticule::Axis;

    #[test]
    fn hemisphere_letters() {
        assert_eq!(line_text(Lang::EN, Axis::Lat, 30.0, 10.0), "30° N");
        assert_eq!(line_text(Lang::EN, Axis::Lat, -30.0, 10.0), "30° S");
        assert_eq!(line_text(Lang::EN, Axis::Lat, 0.0, 10.0), "0°");
        assert_eq!(line_text(Lang::EN, Axis::Lon, -60.0, 10.0), "60° W");
        assert_eq!(line_text(Lang::EN, Axis::Lon, 180.0, 10.0), "180°");
        assert_eq!(line_text(Lang::EN, Axis::Lon, 390.0, 10.0), "30° E");
    }
}
