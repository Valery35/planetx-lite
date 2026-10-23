// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Инструменты глобуса: линейка, новая метка и правка метки. Перенос
//! окон `ui/measure.py` (`RulerDialog`), `ui/draw.py` (`PlaceDialog`)
//! и вершин `ui/handles.py` (`DrawVertices`, `PropVertices`) модуля.
//!
//! Точки ставит один механизм ([`crate::ruler::Ruler`]): щелчок по
//! глобусу добавляет точку, перетаскивание двигает Землю, за курсором
//! тянется резинка. Вершина хватается в [`GRAB_PIXELS`] логических
//! пикселей и перетаскивается по рельефу. Кружок в середине отрезка
//! ставит новую вершину. Щелчок по первой вершине замыкает фигуру, по
//! последней - завершает путь ([`scene::editing`]). Backspace и Delete
//! убирают последнюю точку - единственная отмена, как у модуля.
//!
//! Фигура инструмента рисуется поверх глобуса плоско, в пикселях
//! экрана, как резинка модуля: линии по дугам между вершинами,
//! полупрозрачная заливка, кружки вершин и середин, у линейки - длины
//! отрезков.

use eframe::egui;
use globe_core::DVec3;
use globe_core::navigation::{Navigator, ground_under};
use scene::editing::{self, Click, Picked, Pixel};
use scene::features::{self, Kind};
use scene::kml::Place;
use scene::{geodesic, measure, placetree};

use crate::i18n::{Lang, tr};
use crate::ruler::{Mode, Row, Ruler, Values, shown};

/// Логических пикселей вокруг вершины, в них она хватается.
pub const GRAB_PIXELS: f64 = 8.0;
/// Сдвиг мыши от точки нажатия, логические пиксели, после которого
/// нажатие на вершину - перетаскивание, а не щелчок, `CLICK_PIXELS`
/// модуля (`ui/handles.py`).
pub const CLICK_PIXELS: f32 = 4.0;
/// Цвет линейки, как у Google Earth: жёлтый.
pub const RULER_COLOR: [u8; 4] = [255, 214, 0, 255];
pub const RULER_FILL: [u8; 4] = [255, 214, 0, 50];
/// Цвета новой метки по умолчанию, `DEFAULT_COLOR` модуля.
pub const POINT_COLOR: [u8; 4] = [255, 214, 0, 255];
pub const LINE_COLOR: [u8; 4] = [255, 214, 0, 255];
pub const POLYGON_COLOR: [u8; 4] = [255, 255, 255, 255];
/// Прозрачность заливки многоугольника, `DEFAULT_FILL` модуля.
pub const FILL_ALPHA: u8 = 60;
pub const DEFAULT_WIDTH: f64 = 2.0;
/// Точек дуги на отрезок фигуры на экране.
pub const ARC_POINTS: usize = 32;

/// Для чего инструмент.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Purpose {
    Ruler,
    /// Новая метка.
    Place,
    /// Правка метки «Моих меток» по пути.
    Edit(Vec<usize>),
}

/// Что окно инструмента просит у окна глобуса.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Новая метка в «Мои метки».
    Add(Place),
    /// Метка по пути заменяется.
    Update(Vec<usize>, Place),
    /// Профиль высот линии или пути линейки.
    Profile,
}

/// Инструмент: точки, окно и вершины на глобусе.
pub struct Tool {
    pub purpose: Purpose,
    pub ruler: Ruler,
    pub open: bool,
    pub name: String,
    /// Название с номером, поставленное само: сменой вида оно меняется,
    /// своё название - нет.
    auto_name: String,
    pub description: String,
    pub color: egui::Color32,
    pub width: f64,
    pub fill: bool,
    /// Единицы длины и площади линейки, номера в [`measure::LENGTH_UNITS`]
    /// и [`measure::AREA_UNITS`].
    pub length_unit: usize,
    pub area_unit: usize,
    /// Метка до правки: остальные её свойства.
    original: Option<Place>,
    /// Перетаскиваемая вершина.
    drag: Option<usize>,
    /// Вершина, на которой нажата кнопка, и сдвинулась ли мышь.
    pressed: Option<(usize, bool)>,
    /// Точка нажатия на вершину, логические пиксели окна.
    press_at: Option<egui::Pos2>,
}

/// Вид линейки или метки по номеру вкладки.
const RULER_MODES: [Mode; 4] = [Mode::Line, Mode::Path, Mode::Polygon, Mode::Circle];
const PLACE_MODES: [Mode; 4] = [Mode::Point, Mode::Path, Mode::Polygon, Mode::Circle];

fn rgba(c: [u8; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

fn color_of(mode: Mode) -> egui::Color32 {
    rgba(match mode {
        Mode::Point => POINT_COLOR,
        Mode::Line | Mode::Path => LINE_COLOR,
        Mode::Polygon | Mode::Circle => POLYGON_COLOR,
    })
}

impl Tool {
    /// Линейка.
    pub fn ruler() -> Self {
        Self::with(Purpose::Ruler, Mode::Line, rgba(RULER_COLOR))
    }

    /// Закрытый инструмент: окно глобуса снимает прежний.
    pub fn closed() -> Self {
        let mut tool = Self::ruler();
        tool.open = false;
        tool
    }

    /// Новая метка вида mode.
    pub fn place(mode: Mode) -> Self {
        Self::with(Purpose::Place, mode, color_of(mode))
    }

    /// Правка метки по пути: её точки - вершины, рисование завершено.
    pub fn edit(path: Vec<usize>, place: &Place) -> Self {
        let mode = match place.kind {
            Kind::Point => Mode::Point,
            Kind::Line => Mode::Path,
            Kind::Polygon => Mode::Polygon,
        };
        let [r, g, b, a] = place.color;
        let mut tool = Self::with(Purpose::Edit(path), mode, egui::Color32::from_rgba_unmultiplied(r, g, b, a));
        tool.ruler.points = place.points.clone();
        tool.ruler.finished = true;
        tool.name = place.name.clone();
        tool.description = place.description.clone();
        tool.width = place.width;
        tool.fill = place.fill.is_some();
        tool.original = Some(place.clone());
        tool
    }

    fn with(purpose: Purpose, mode: Mode, color: egui::Color32) -> Self {
        Self {
            purpose,
            ruler: Ruler::new(mode),
            open: true,
            name: String::new(),
            auto_name: String::new(),
            description: String::new(),
            color,
            width: DEFAULT_WIDTH,
            fill: true,
            length_unit: 0,
            area_unit: 0,
            original: None,
            drag: None,
            pressed: None,
            press_at: None,
        }
    }

    /// Название с номером для нового объекта, как «Моя метка 3». Своё
    /// название, введённое вручную, сменой вида не затирается.
    fn reset_name(&mut self, lang: Lang, names: &[String]) {
        if self.purpose != Purpose::Place {
            return;
        }
        let text = self.name.trim();
        if !text.is_empty() && text != self.auto_name {
            return;
        }
        let base = match self.ruler.mode {
            Mode::Point => tr(lang, "My placemark"),
            Mode::Path | Mode::Line => tr(lang, "My path"),
            Mode::Polygon => tr(lang, "My polygon"),
            Mode::Circle => tr(lang, "My circle"),
        };
        self.auto_name = placetree::numbered_name(base, names);
        self.name = self.auto_name.clone();
    }

    fn rgba(&self) -> [u8; 4] {
        self.color.to_srgba_unmultiplied()
    }

    /// Метка из фигуры инструмента или None.
    fn place_of(&self, description: String) -> Option<Place> {
        let figure = self.ruler.figure(false)?;
        let mut place = self.original.clone().unwrap_or_else(|| Place::new("", figure.kind, Vec::new()));
        place.kind = figure.kind;
        place.points = figure.points;
        place.name = self.name.trim().to_string();
        place.description = description;
        let [r, g, b, a] = self.rgba();
        place.color = [r, g, b, a];
        place.width = self.width;
        place.fill = (figure.kind == Kind::Polygon && self.fill).then_some([r, g, b, FILL_ALPHA]);
        if self.purpose == Purpose::Ruler {
            place.color = RULER_COLOR;
            place.fill = (figure.kind == Kind::Polygon).then_some(RULER_FILL);
        }
        Some(place)
    }

    /// Окно инструмента. names - названия «Моих меток» для номера,
    /// heights - высоты точек для длины по рельефу и готовы ли они.
    pub fn window(
        &mut self,
        ctx: &egui::Context,
        lang: Lang,
        names: &[String],
        heights: Option<(&measure::HeightsAt<'_>, bool)>,
    ) -> Option<Action> {
        if self.purpose == Purpose::Place && self.auto_name.is_empty() && self.name.is_empty() {
            self.reset_name(lang, names);
        }
        let title = match self.purpose {
            Purpose::Ruler => tr(lang, "Ruler"),
            Purpose::Place => tr(lang, "New placemark"),
            Purpose::Edit(_) => tr(lang, "Properties"),
        };
        let mut open = self.open;
        let mut action = None;
        egui::Window::new(title)
            .id(egui::Id::unique("planetx-tool"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::RIGHT_TOP)
            .default_pos(ctx.content_rect().right_top() + egui::vec2(-crate::navpad::WIDTH - 2.0 * crate::navpad::MARGIN - 8.0, 60.0))
            .show(ctx, |ui| {
                ui.set_max_width(300.0);
                match self.purpose {
                    Purpose::Ruler => action = self.ruler_body(ui, lang, names, heights),
                    _ => action = self.place_body(ui, lang, names),
                }
            });
        // Окно закрывают крестик и кнопки OK и «Отмена».
        self.open &= open;
        action
    }

    /// Вкладки видов. Смена вида убирает точки.
    fn tabs(&mut self, ui: &mut egui::Ui, lang: Lang, modes: &[Mode; 4], names: &[String]) {
        let title = |m: Mode| match m {
            Mode::Point => tr(lang, "Placemark"),
            Mode::Line => tr(lang, "Line"),
            Mode::Path => tr(lang, "Path"),
            Mode::Polygon => tr(lang, "Polygon"),
            Mode::Circle => tr(lang, "Circle"),
        };
        ui.horizontal(|ui| {
            for mode in modes {
                if ui.selectable_label(self.ruler.mode == *mode, title(*mode)).clicked() && self.ruler.mode != *mode {
                    self.ruler.set_mode(*mode);
                    if self.purpose == Purpose::Place {
                        self.color = color_of(*mode);
                        self.reset_name(lang, names);
                    }
                }
            }
        });
    }

    fn ruler_body(
        &mut self,
        ui: &mut egui::Ui,
        lang: Lang,
        names: &[String],
        heights: Option<(&measure::HeightsAt<'_>, bool)>,
    ) -> Option<Action> {
        self.tabs(ui, lang, &RULER_MODES, names);
        ui.separator();
        let values = self.ruler.values(true, heights);
        egui::Grid::new("planetx-ruler-rows").num_columns(3).spacing([8.0, 4.0]).show(ui, |ui| {
            for row in shown(self.ruler.mode) {
                ui.label(row_title(lang, *row, self.ruler.mode));
                let value = row.value(&values);
                if *row == Row::Heading {
                    ui.label(value.map_or_else(|| "-".to_string(), |v| format!("{v:.1}°")));
                    ui.end_row();
                    continue;
                }
                let (units, index) = if row.is_area() {
                    (&measure::AREA_UNITS[..], &mut self.area_unit)
                } else {
                    (&measure::LENGTH_UNITS[..], &mut self.length_unit)
                };
                let code = units.get(*index).map_or("m", |u| u.0);
                let approx = *row == Row::Ground && !values.ground_ready;
                ui.label(value.map_or_else(
                    || "-".to_string(),
                    |v| format!("{}{}", if approx { "≈" } else { "" }, measure::number(measure::convert(v, code, units))),
                ));
                egui::ComboBox::from_id_salt(("planetx-ruler-unit", row.is_area()))
                    .selected_text(unit_name(lang, code))
                    .show_ui(ui, |ui| {
                        for (i, (c, _)) in units.iter().enumerate() {
                            ui.selectable_value(index, i, unit_name(lang, c));
                        }
                    });
                ui.end_row();
            }
        });
        ui.label(egui::RichText::new(self.hint(lang)).weak());
        ui.horizontal(|ui| ui.label(tr(lang, "Name")).labelled_by(ui.text_edit_singleline(&mut self.name).id));
        let mut action = None;
        ui.horizontal(|ui| {
            let can = self.ruler.figure(false).is_some();
            if ui.add_enabled(can, egui::Button::new(tr(lang, "Save"))).on_hover_text(tr(lang, "Put the shape into My Places with the measurement.")).clicked() {
                if self.name.trim().is_empty() {
                    self.name = placetree::numbered_name(tr(lang, "Measurement"), names);
                }
                let text = summary(lang, &self.ruler.values(false, heights), self.ruler.mode, self.length_unit, self.area_unit);
                action = self.place_of(text).map(Action::Add);
                self.ruler.clear();
                self.name.clear();
            }
            if ui.button(tr(lang, "Clear")).on_hover_text(tr(lang, "Remove the points.")).clicked() {
                self.ruler.clear();
            }
            if matches!(self.ruler.mode, Mode::Line | Mode::Path) && ui.button(tr(lang, "Elevation profile")).clicked() {
                action = Some(Action::Profile);
            }
        });
        action
    }

    fn place_body(&mut self, ui: &mut egui::Ui, lang: Lang, names: &[String]) -> Option<Action> {
        if self.purpose == Purpose::Place {
            self.tabs(ui, lang, &PLACE_MODES, names);
            ui.separator();
        }
        ui.label(egui::RichText::new(self.hint(lang)).weak());
        egui::Grid::new("planetx-place-rows").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label(tr(lang, "Name"));
            ui.add(egui::TextEdit::singleline(&mut self.name).hint_text(tr(lang, "Untitled")))
                .on_hover_text(tr(lang, "Name in My Places and the label of the point on the globe."));
            ui.end_row();
            if matches!(self.purpose, Purpose::Edit(_)) {
                ui.label(tr(lang, "Description"));
                ui.add(egui::TextEdit::multiline(&mut self.description).desired_rows(3));
                ui.end_row();
            }
            ui.label(tr(lang, "Color"));
            ui.color_edit_button_srgba(&mut self.color)
                .on_hover_text(tr(lang, "Colour of the line and outline. The fill of a polygon is the same colour, semi-transparent."));
            ui.end_row();
            if self.ruler.mode != Mode::Point {
                ui.label(tr(lang, "Width"));
                ui.add(egui::DragValue::new(&mut self.width).range(1.0..=10.0).speed(0.1).fixed_decimals(1))
                    .on_hover_text(tr(lang, "Width of the line and outline in screen pixels. It does not depend on the scale."));
                ui.end_row();
            }
            if matches!(self.ruler.mode, Mode::Polygon | Mode::Circle) {
                ui.label(tr(lang, "Fill"));
                ui.checkbox(&mut self.fill, "");
                ui.end_row();
            }
        });
        let mut action = None;
        ui.horizontal(|ui| {
            let can = self.ruler.figure(false).is_some();
            let save = if matches!(self.purpose, Purpose::Edit(_)) { tr(lang, "OK") } else { tr(lang, "Save") };
            if ui.add_enabled(can, egui::Button::new(save)).on_hover_text(tr(lang, "Write the object to My Places.")).clicked() {
                let place = self.place_of(self.description.clone());
                match (&self.purpose, place) {
                    (Purpose::Edit(path), Some(place)) => {
                        action = Some(Action::Update(path.clone(), place));
                        self.open = false;
                    }
                    (_, Some(place)) => {
                        action = Some(Action::Add(place));
                        self.ruler.clear();
                        self.name.clear();
                        self.auto_name.clear();
                    }
                    _ => {}
                }
            }
            if self.purpose == Purpose::Place && ui.button(tr(lang, "Clear")).on_hover_text(tr(lang, "Remove the points.")).clicked() {
                self.ruler.clear();
            }
            if matches!(self.purpose, Purpose::Edit(_)) && ui.button(tr(lang, "Cancel")).clicked() {
                self.open = false;
            }
        });
        action
    }

    /// Подсказка под вкладками, как у модуля.
    fn hint(&self, lang: Lang) -> &'static str {
        if self.ruler.finished {
            return tr(lang, "Vertices are dragged with the mouse. A circle in the middle of a segment adds a vertex.");
        }
        match self.ruler.mode {
            Mode::Point => tr(lang, "A click on the globe places the placemark. The placemark is dragged with the mouse."),
            Mode::Line => tr(lang, "Two clicks on the globe set the ends of the line. Points are dragged with the mouse."),
            Mode::Path => tr(lang, "Mark the points of the path with clicks on the globe. A click on the last point finishes the path, on the first one closes the shape."),
            Mode::Polygon => tr(lang, "Mark the vertices of the polygon with clicks on the globe. A click on the first vertex finishes drawing."),
            Mode::Circle => tr(lang, "The first click on the globe sets the centre of the circle, the second one a point of the circle. Both points are dragged with the mouse."),
        }
    }

    /// Мышь и клавиши инструмента над глобусом. rect - место глобуса,
    /// ppp - пикселей кадра на логический пиксель. true - мышь занята
    /// инструментом (тянется вершина), навигации она не достаётся.
    pub fn input(&mut self, ctx: &egui::Context, response: &egui::Response, rect: egui::Rect, ppp: f64, nav: &Navigator) -> bool {
        let to_px = |p: egui::Pos2| (f64::from(p.x - rect.min.x) * ppp, f64::from(p.y - rect.min.y) * ppp);
        let ground = |p: egui::Pos2| {
            let (px, py) = to_px(p);
            let point = ground_under(&nav.camera, px, py, nav.pose.terrain.as_ref())?;
            let (lat, lon, _) = nav.camera.ellipsoid.ecef_to_geodetic(point);
            Some((lat, lon))
        };
        let (pressed, down, released, pos, keys) = ctx.input(|i| {
            let keys = i.key_pressed(egui::Key::Backspace) || i.key_pressed(egui::Key::Delete);
            (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.primary_released(), i.pointer.interact_pos(), keys)
        });
        if keys && !ctx.egui_wants_keyboard_input() {
            self.ruler.remove_last();
        }
        // Щелчок по окну или меню над глобусом глобусу не достаётся.
        let on_globe = pos.is_some_and(|p| rect.contains(p) && ctx.layer_id_at(p).is_none_or(|l| l == response.layer_id));
        self.ruler.cursor = response.hover_pos().and_then(ground);
        let radius = GRAB_PIXELS * ppp;
        if pressed && on_globe && let Some(p) = pos {
            let (px, py) = to_px(p);
            let (vertices, middles) = self.handles(nav);
            self.press_at = Some(p);
            match editing::pick(&vertices, &middles, px, py, radius) {
                Some(Picked::Vertex(i)) => {
                    self.drag = Some(i);
                    self.pressed = Some((i, false));
                }
                Some(Picked::Middle(i)) => {
                    // Кружок середины ставит новую вершину и тянет её.
                    if let Some((lat, lon)) = ground(p) {
                        self.ruler.insert(i + 1, lat, lon);
                        self.drag = Some(i + 1);
                        self.pressed = Some((i + 1, true));
                    }
                }
                None => {}
            }
        }
        if let Some(index) = self.drag {
            if down {
                let far = pos.zip(self.press_at).is_some_and(|(p, at)| p.distance(at) > CLICK_PIXELS);
                let moved = self.pressed.is_some_and(|(_, m)| m) || far;
                if moved && let Some((lat, lon)) = pos.and_then(ground) {
                    self.ruler.move_to(index, lat, lon);
                    if let Some((_, m)) = &mut self.pressed {
                        *m = true;
                    }
                }
                return true;
            }
            self.drag = None;
            if let Some((index, false)) = self.pressed.take() {
                // Щелчок по вершине без перетаскивания.
                match editing::click_result(self.ruler.mode.drawing(), self.ruler.points.len(), index, self.ruler.finished) {
                    Some(Click::Close) => {
                        if self.ruler.mode == Mode::Path {
                            self.ruler.mode = Mode::Polygon;
                        }
                        self.ruler.finish();
                    }
                    Some(Click::Finish) => self.ruler.finish(),
                    None => {}
                }
            }
            return true;
        }
        if released {
            self.pressed = None;
        }
        if response.clicked() && on_globe && let Some((lat, lon)) = pos.and_then(ground) {
            if self.ruler.mode == Mode::Point && !self.ruler.points.is_empty() {
                self.ruler.move_to(0, lat, lon);
            } else {
                self.ruler.add(lat, lon);
            }
        }
        false
    }

    /// Точка (широта, долгота) на рельефе в ECEF.
    fn ecef(nav: &Navigator, (lat, lon): (f64, f64)) -> DVec3 {
        let h = nav.pose.terrain.as_ref().map_or(0.0, |t| t(lat, lon));
        nav.camera.ellipsoid.geodetic_to_ecef(lat, lon, h)
    }

    /// Точка экрана, кадровые пиксели, и видна ли: перед камерой и на
    /// обращённой к глазу стороне Земли.
    fn pixel(nav: &Navigator, p: (f64, f64)) -> Pixel {
        let point = Self::ecef(nav, p);
        let (px, front) = nav.camera.project(point);
        let facing = (nav.camera.eye - point).dot(point.normalize_or_zero()) > 0.0;
        ([px.x, px.y], front && facing)
    }

    /// Вершины и середины отрезков в пикселях кадра. Середины - только
    /// когда рисование завершено.
    fn handles(&self, nav: &Navigator) -> (Vec<Pixel>, Vec<Pixel>) {
        let vertices = self.ruler.points.iter().map(|p| Self::pixel(nav, *p)).collect();
        let closed = matches!(self.ruler.mode, Mode::Polygon);
        let middles = if self.ruler.finished && !matches!(self.ruler.mode, Mode::Point | Mode::Circle) {
            measure::segment_midpoints(&self.ruler.points, closed).into_iter().map(|p| Self::pixel(nav, p)).collect()
        } else {
            Vec::new()
        };
        (vertices, middles)
    }

    /// Фигура, вершины и подписи отрезков поверх глобуса.
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect, ppp: f64, nav: &Navigator, lang: Lang) {
        let to_screen = |p: [f64; 2]| rect.min + egui::vec2((p[0] / ppp) as f32, (p[1] / ppp) as f32);
        let color = if self.purpose == Purpose::Ruler { rgba(RULER_COLOR) } else { self.color };
        let stroke = egui::Stroke::new(self.width as f32, color);
        if let Some(figure) = self.ruler.figure(true) {
            let closed = figure.kind == Kind::Polygon;
            let mut pairs: Vec<((f64, f64), (f64, f64))> = figure.points.windows(2).map(|w| (w[0], w[1])).collect();
            if closed
                && figure.points.len() >= 3
                && let (Some(a), Some(b)) = (figure.points.last(), figure.points.first())
            {
                pairs.push((*a, *b));
            }
            if closed {
                let fill_alpha = if self.purpose == Purpose::Ruler { RULER_FILL[3] } else if self.fill { FILL_ALPHA } else { 0 };
                let pixels: Vec<Pixel> = figure.points.iter().map(|p| Self::pixel(nav, *p)).collect();
                if fill_alpha > 0 && pixels.iter().all(|p| p.1) {
                    let [r, g, b, _] = color.to_srgba_unmultiplied();
                    let fill = egui::Color32::from_rgba_unmultiplied(r, g, b, fill_alpha);
                    let mut mesh = egui::Mesh::default();
                    for (p, _) in &pixels {
                        mesh.colored_vertex(to_screen(*p), fill);
                    }
                    for tri in features::triangulate(&figure.points).as_chunks::<3>().0 {
                        mesh.add_triangle(tri[0], tri[1], tri[2]);
                    }
                    painter.add(egui::Shape::mesh(mesh));
                }
            }
            for (a, b) in pairs {
                let arc = features::densify(&[a, b], false, geodesic::length(&[a, b]) / ARC_POINTS as f64 + 1e-6, ARC_POINTS + 1);
                let pixels: Vec<Pixel> = arc.iter().map(|p| Self::pixel(nav, *p)).collect();
                for w in pixels.windows(2) {
                    if w[0].1 && w[1].1 {
                        painter.line_segment([to_screen(w[0].0), to_screen(w[1].0)], stroke);
                    }
                }
                if self.purpose == Purpose::Ruler && !closed && figure.points.len() > 2 {
                    // Длина отрезка у его середины, как подписи линейки модуля.
                    if let Some(mid) = measure::segment_midpoints(&[a, b], false).first() {
                        let (p, visible) = Self::pixel(nav, *mid);
                        if visible {
                            let code = measure::LENGTH_UNITS.get(self.length_unit).map_or("m", |u| u.0);
                            let text = format!(
                                "{} {}",
                                measure::number(measure::convert(geodesic::length(&[a, b]), code, &measure::LENGTH_UNITS)),
                                unit_short(lang, code)
                            );
                            painter.text(to_screen(p), egui::Align2::CENTER_BOTTOM, text, egui::FontId::proportional(12.0), color);
                        }
                    }
                }
            }
        }
        let (vertices, middles) = self.handles(nav);
        for (p, visible) in middles {
            if visible {
                painter.circle(to_screen(p), 3.5, egui::Color32::from_white_alpha(160), egui::Stroke::new(1.0, egui::Color32::from_gray(40)));
            }
        }
        for (i, (p, visible)) in vertices.into_iter().enumerate() {
            if visible {
                let r = if self.drag == Some(i) { 6.0 } else { 5.0 };
                painter.circle(to_screen(p), r, egui::Color32::WHITE, egui::Stroke::new(1.5, egui::Color32::from_gray(30)));
            }
        }
    }
}

/// Название строки величины.
fn row_title(lang: Lang, row: Row, mode: Mode) -> &'static str {
    match row {
        Row::Length if matches!(mode, Mode::Line | Mode::Path) => tr(lang, "Map length"),
        Row::Length => tr(lang, "Length"),
        Row::Ground => tr(lang, "Ground length"),
        Row::Heading => tr(lang, "Heading"),
        Row::Perimeter => tr(lang, "Perimeter"),
        Row::Area => tr(lang, "Area"),
        Row::Radius => tr(lang, "Radius"),
    }
}

/// Название единицы для списка.
fn unit_name(lang: Lang, code: &str) -> &'static str {
    match code {
        "km" => tr(lang, "Kilometres"),
        "mi" => tr(lang, "Miles"),
        "nmi" => tr(lang, "Nautical miles"),
        "m2" => tr(lang, "Square metres"),
        "ha" => tr(lang, "Hectares"),
        "km2" => tr(lang, "Square kilometres"),
        "mi2" => tr(lang, "Square miles"),
        _ => tr(lang, "Metres"),
    }
}

/// Краткое название единицы для текста измерения.
fn unit_short(lang: Lang, code: &str) -> &'static str {
    match code {
        "km" => tr(lang, "km"),
        "mi" => tr(lang, "mi"),
        "nmi" => tr(lang, "nmi"),
        "m2" => tr(lang, "m²"),
        "ha" => tr(lang, "ha"),
        "km2" => tr(lang, "km²"),
        "mi2" => tr(lang, "sq mi"),
        _ => tr(lang, "m"),
    }
}

/// Текст измерения для «Моих меток», `summary` модуля: «Длина на карте
/// 1.25 км, длина по рельефу 1.27 км».
pub fn summary(lang: Lang, values: &Values, mode: Mode, length_unit: usize, area_unit: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    for row in shown(mode) {
        let Some(value) = row.value(values) else {
            continue;
        };
        let mut title = row_title(lang, *row, mode).to_string();
        if !parts.is_empty() {
            let mut chars = title.chars();
            title = chars.next().map(|c| c.to_lowercase().chain(chars).collect()).unwrap_or_default();
        }
        if *row == Row::Heading {
            parts.push(format!("{title} {value:.1}°"));
            continue;
        }
        let (units, index) = if row.is_area() { (&measure::AREA_UNITS[..], area_unit) } else { (&measure::LENGTH_UNITS[..], length_unit) };
        let code = units.get(index).map_or("m", |u| u.0);
        parts.push(format!("{title} {} {}", measure::number(measure::convert(value, code, units)), unit_short(lang, code)));
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::summary;
    use crate::i18n::Lang;
    use crate::ruler::{Mode, Values};

    #[test]
    fn summary_text() {
        let v = Values { length: Some(1250.0), ground: Some(1270.0), heading: Some(45.04), ..Values::default() };
        assert_eq!(summary(Lang::EN, &v, Mode::Line, 1, 0), "Map length 1.25 km, ground length 1.27 km, heading 45.0°");
        let v = Values { perimeter: Some(400.0), area: Some(10_000.0), ..Values::default() };
        assert_eq!(summary(Lang::EN, &v, Mode::Polygon, 0, 1), "Perimeter 400.00 m, area 1.00 ha");
    }
}
