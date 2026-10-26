// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Карточка метки справа от вида: название, описание, поля, кнопки.
//! Перенос `ui/placecard.py` модуля (`PlaceCard`).
//!
//! Щелчок по метке на глобусе без включённого инструмента открывает её
//! карточку. Описание - HTML метки, как в KML и Google Earth, у метки
//! с шаблоном BalloonStyle - по шаблону (`scene::balloon`). HTML
//! показывается текстом без тегов, ссылки - ссылками, их открывает
//! система. «Объясни это» - вопрос помощнику ИИ о метке
//! ([`explain_question`]), ответ - под текстом карточки, как у модуля.
//! Нажатие кнопки - согласие отправить данные этой одной метки.
//!
//! Попадание щелчка в метку считается в пикселях экрана
//! (`scene::editing::hit`) в [`PICK_PIXELS`] логических пикселей, как
//! у модуля.

use eframe::egui;
use globe_core::navigation::Navigator;
use scene::balloon::{self, Piece};
use scene::editing::{self, Pixel, Shape};
use scene::features::Kind;
use scene::kml::{Folder, Item, Place};

use crate::i18n::{Lang, tr, trf};

/// Логических пикселей до метки, в них щелчок попадает.
pub const PICK_PIXELS: f64 = 8.0;
/// Описание и поля метки в вопросе помощнику - не длиннее, знаков,
/// `EXPLAIN_LIMIT` модуля.
pub const EXPLAIN_LIMIT: usize = 2000;
/// Полей метки в вопросе помощнику не больше, `FIELDS_LIMIT` модуля.
pub const FIELDS_LIMIT: usize = 20;

/// Текст не длиннее [`EXPLAIN_LIMIT`] знаков, обрезанный - с многоточием.
fn clip(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= EXPLAIN_LIMIT {
        return text;
    }
    text.chars().take(EXPLAIN_LIMIT).collect::<String>() + "…"
}

/// Вопрос помощнику о метке, `explain_question` модуля: название, вид,
/// середина точек, описание без разметки и поля метки.
pub fn explain_question(lang: Lang, place: &Place) -> String {
    let points = if place.points.is_empty() { vec![(0.0, 0.0)] } else { place.points.clone() };
    let lat = points.iter().map(|p| p.0).sum::<f64>() / points.len() as f64;
    // Середина дуги долгот: путь через линию перемены дат не уводит
    // точку на другую сторону Земли.
    let lons: Vec<f64> = points.iter().map(|p| p.1).collect();
    let lon = middle_lon(&lons);
    let kind = match place.kind {
        Kind::Point => tr(lang, "Placemark"),
        Kind::Line => tr(lang, "Path"),
        Kind::Polygon => tr(lang, "Polygon"),
    };
    let name = if place.name.trim().is_empty() { tr(lang, "Untitled").to_string() } else { place.name.clone() };
    let mut question = trf(
        lang,
        "Explain what this object is and why this place is interesting. {kind} «{name}», point {lat}, {lon}.",
        &[("kind", kind), ("name", &name), ("lat", &format!("{lat:.5}")), ("lon", &format!("{lon:.5}"))],
    );
    let (pieces, data) = content(place);
    let text: Vec<&str> = pieces
        .iter()
        .map(|p| match p {
            Piece::Text(t) | Piece::Link(t, _) => t.as_str(),
        })
        .collect();
    let text = clip(&text.join(" "));
    if !text.is_empty() {
        question.push(' ');
        question.push_str(&trf(lang, "The description of the placemark - {text}", &[("text", &text)]));
    }
    let fields = clip(&data.iter().take(FIELDS_LIMIT).map(|(n, v)| format!("{n} = {v}")).collect::<Vec<_>>().join("; "));
    if !fields.is_empty() {
        question.push(' ');
        question.push_str(&trf(lang, "Object fields - {fields}.", &[("fields", &fields)]));
    }
    question
}

/// Середина дуги долгот: наименьшая дуга, что вмещает все долготы.
fn middle_lon(lons: &[f64]) -> f64 {
    let mut sorted: Vec<f64> = lons.iter().map(|l| (l + 180.0).rem_euclid(360.0) - 180.0).collect();
    sorted.sort_by(f64::total_cmp);
    let (Some(first), Some(last)) = (sorted.first().copied(), sorted.last().copied()) else {
        return 0.0;
    };
    // Самый большой просвет между соседними долготами - вне дуги.
    let mut gap = (first + 360.0 - last, last);
    for w in sorted.windows(2) {
        if w[1] - w[0] > gap.0 {
            gap = (w[1] - w[0], w[0]);
        }
    }
    let start = gap.1 + gap.0;
    let span = 360.0 - gap.0;
    (start + span / 2.0 + 180.0).rem_euclid(360.0) - 180.0
}

/// Что просит карточка.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Fly(Vec<usize>),
    Properties(Vec<usize>),
    /// «Объясни это»: вопрос помощнику о метке.
    Explain(Vec<usize>),
}

/// Метка под пикселем (px, py) кадра или None: путь в дереве «Моих
/// меток». Видимые метки, последняя нарисованная - сверху, точки
/// раньше линий и многоугольников.
pub fn pick(tree: &Folder, nav: &Navigator, px: f64, py: f64, radius: f64) -> Option<Vec<usize>> {
    let mut found: Vec<(Vec<usize>, Kind)> = Vec::new();
    let mut path = Vec::new();
    walk(tree, nav, px, py, radius, &mut path, &mut found);
    // Точка точнее заливки: при попадании в обе берётся точка.
    let rank = |k: &Kind| match k {
        Kind::Point => 0,
        Kind::Line => 1,
        Kind::Polygon => 2,
    };
    found.into_iter().rev().min_by_key(|(_, k)| rank(k)).map(|(p, _)| p)
}

/// Все видимые метки под пикселем, по порядку дерева: для окна
/// «Объекты».
pub fn pick_all(tree: &Folder, nav: &Navigator, px: f64, py: f64, radius: f64) -> Vec<Vec<usize>> {
    let mut found = Vec::new();
    walk(tree, nav, px, py, radius, &mut Vec::new(), &mut found);
    found.into_iter().map(|(p, _)| p).collect()
}

fn walk(folder: &Folder, nav: &Navigator, px: f64, py: f64, radius: f64, path: &mut Vec<usize>, found: &mut Vec<(Vec<usize>, Kind)>) {
    if !folder.visible {
        return;
    }
    for (i, child) in folder.children.iter().enumerate() {
        path.push(i);
        match child {
            Item::Folder(f) => walk(f, nav, px, py, radius, path, found),
            Item::Place(p) if p.visible && !p.points.is_empty() => {
                let pixels: Vec<Pixel> = p.points.iter().map(|q| pixel(nav, *q)).collect();
                let shape = match p.kind {
                    Kind::Point => Shape::Point,
                    Kind::Line => Shape::Line,
                    Kind::Polygon => Shape::Polygon,
                };
                if editing::hit(shape, &pixels, px, py, radius) {
                    found.push((path.clone(), p.kind));
                }
            }
            _ => {}
        }
        path.pop();
    }
}

/// Точка (широта, долгота) на рельефе в пикселях кадра и видна ли.
fn pixel(nav: &Navigator, (lat, lon): (f64, f64)) -> Pixel {
    let h = nav.pose.terrain.as_ref().map_or(0.0, |t| t(lat, lon));
    let point = nav.camera.ellipsoid.geodetic_to_ecef(lat, lon, h);
    let (px, front) = nav.camera.project(point);
    let facing = (nav.camera.eye - point).dot(point.normalize_or_zero()) > 0.0;
    ([px.x, px.y], front && facing)
}

/// Куски описания метки: по шаблону BalloonStyle или описание и поля.
pub fn content(place: &Place) -> (Vec<Piece>, Vec<(String, String)>) {
    let data = place.data.clone().unwrap_or_default();
    if !place.balloon.trim().is_empty() {
        let html = balloon::fill(&place.balloon, &place.name, &place.description, &data);
        return (balloon::pieces(&html), Vec::new());
    }
    (balloon::pieces(&place.description), data)
}

/// Карточка открытой метки.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub path: Vec<usize>,
}

impl Card {
    /// Окно карточки. false в open - карточку закрыли. answer - ответ
    /// помощника о метке, пустой - ответ ждётся, None - не спрашивали.
    pub fn window(&self, ctx: &egui::Context, lang: Lang, place: &Place, answer: Option<&str>, open: &mut bool) -> Option<Action> {
        let mut action = None;
        let title = if place.name.trim().is_empty() { tr(lang, "Untitled").to_string() } else { place.name.clone() };
        egui::Window::new(title)
            .id(egui::Id::unique("planetx-card"))
            .open(open)
            .collapsible(false)
            .resizable(true)
            .default_width(320.0)
            .pivot(egui::Align2::RIGHT_TOP)
            .default_pos(ctx.content_rect().right_top() + egui::vec2(-crate::navpad::WIDTH - 2.0 * crate::navpad::MARGIN - 8.0, 60.0))
            .show(ctx, |ui| {
                let (pieces, data) = content(place);
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for piece in &pieces {
                            match piece {
                                Piece::Text(t) => {
                                    ui.label(t);
                                }
                                Piece::Link(t, href) => {
                                    ui.hyperlink_to(t, href);
                                }
                            }
                        }
                    });
                    if !data.is_empty() {
                        ui.add_space(6.0);
                        egui::Grid::new("planetx-card-fields").num_columns(2).spacing([8.0, 2.0]).striped(true).show(ui, |ui| {
                            for (name, value) in &data {
                                ui.label(egui::RichText::new(name).strong());
                                ui.label(value);
                                ui.end_row();
                            }
                        });
                    }
                    if let Some(text) = answer {
                        ui.add_space(6.0);
                        ui.separator();
                        if text.is_empty() {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.weak(tr(lang, "Waiting for the answer…"));
                            });
                        } else {
                            ui.label(text);
                        }
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(tr(lang, "Explain this")).on_hover_text(tr(lang, "A question to the assistant about this placemark: its name, kind, point, description and fields go to the server of the chosen model.")).clicked() {
                        action = Some(Action::Explain(self.path.clone()));
                    }
                    if ui.button(tr(lang, "Fly to")).clicked() {
                        action = Some(Action::Fly(self.path.clone()));
                    }
                    if ui.button(tr(lang, "Properties…")).clicked() {
                        action = Some(Action::Properties(self.path.clone()));
                    }
                });
            });
        action
    }
}

#[cfg(test)]
mod tests {
    use super::content;
    use scene::balloon::Piece;
    use scene::features::Kind;
    use scene::kml::Place;

    /// Вопрос о метке: название, середина точек, описание и поля, путь
    /// через линию перемены дат - середина у неё, а не на другой стороне.
    #[test]
    fn explain_question_carries_the_place() {
        let mut p = Place::new("Ла-Манш", Kind::Line, vec![(50.0, 179.0), (51.0, -179.0)]);
        p.description = "<b>пролив</b>".into();
        p.data = Some(vec![("ширина".into(), "32 км".into())]);
        let q = super::explain_question(crate::i18n::Lang::EN, &p);
        assert!(q.contains("Ла-Манш") && q.contains("50.50000") && q.contains("180.00000"), "{q}");
        assert!(q.contains("пролив") && !q.contains("<b>") && q.contains("ширина = 32 км"), "{q}");
        assert!((super::middle_lon(&[10.0, 20.0]) - 15.0).abs() < 1e-9);
    }

    #[test]
    fn balloon_template_wins_over_fields() {
        let mut p = Place::new("Скв. 7", Kind::Point, vec![(58.0, 56.0)]);
        p.description = "керн".into();
        p.data = Some(vec![("depth".into(), "412.5".into())]);
        let (pieces, data) = content(&p);
        assert_eq!(pieces, [Piece::Text("керн".into())]);
        assert_eq!(data.len(), 1);
        p.balloon = "<b>$[name]</b>: $[depth] м".into();
        let (pieces, data) = content(&p);
        assert_eq!(pieces, [Piece::Text("Скв. 7: 412.5 м".into())]);
        assert!(data.is_empty());
    }
}
