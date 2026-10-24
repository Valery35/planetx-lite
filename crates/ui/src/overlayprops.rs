// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно свойств наложения и его ручки на глобусе, перенос `OverlayDialog`,
//! `BoxVertices`, `CornerVertices` и `ModelVertices` (`ui/overlays.py`)
//! модуля.
//!
//! Окно немодальное. Правки сразу видны на глобусе: окно глобуса пишет
//! рабочую копию наложения в «Мои метки» после каждой правки, «Отмена»
//! возвращает прежнее. Картинка хранится в файле меток или ссылкой на
//! файл или адрес в сети, как поле «Link» Google Earth. Рамка картинки
//! на поверхности тянется ручками: крест сдвигает, ромб поворачивает,
//! углы и середины сторон растягивают, с Shift - от середины. Четыре
//! свободных угла тянутся каждый сам. Рамка модели - след её коробки на
//! земле, угол меняет масштаб, середина стороны растягивает по оси.

use eframe::egui;
use globe_core::DVec3;
use globe_core::navigation::{Navigator, ground_under};
use scene::kml::Overlay;
use scene::overlays::{self, Handle, Kind};
use scene::when;

use crate::i18n::{Lang, tr, trf};

/// Контур картинки и рамки модели, пока открыто окно, как `OUTLINE`
/// модуля.
pub const OUTLINE: egui::Color32 = egui::Color32::from_rgb(255, 214, 0);
/// Отступ картинки на экране от края вида, пикселей, `MARGIN` модуля.
pub const MARGIN: f64 = 10.0;
/// Новая картинка на экране - под панелью значков, `SCREEN_TOP` модуля.
pub const SCREEN_TOP: f64 = 60.0;
/// Логических пикселей вокруг ручки, в них она хватается.
const GRAB: f64 = 9.0;

/// Угол картинки на экране: ключ, точка картинки, точка вида - доли,
/// `CORNERS` модуля.
/// Угол картинки на экране: название, точка картинки, точка вида.
type Corner = (&'static str, (f64, f64), (f64, f64));

const CORNERS: [Corner; 5] = [
    ("Top left", (0.0, 1.0), (0.0, 1.0)),
    ("Top right", (1.0, 1.0), (1.0, 1.0)),
    ("Bottom left", (0.0, 0.0), (0.0, 0.0)),
    ("Bottom right", (1.0, 0.0), (1.0, 0.0)),
    ("Centre", (0.5, 0.5), (0.5, 0.5)),
];

/// Чем закончилось окно.
pub enum Outcome {
    Open,
    Ok,
    Cancel,
    /// «Обзор…»: выбрать файл картинки или модели.
    Browse,
}

/// Ручка под мышью: какая и номер.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Grip {
    Box(Handle, usize),
    Corner(usize),
}

/// Окно свойств наложения по пути path.
pub struct OverlayProps {
    pub path: Vec<usize>,
    /// Рабочая копия: правки окна и ручек.
    pub edit: Overlay,
    pub original: Overlay,
    /// Картинка или пакет модели для хранения в файле меток.
    pub bytes: Option<Vec<u8>>,
    link: bool,
    path_text: String,
    time_text: String,
    /// Коробка модели (xmin, xmax, ymin, ymax) в её единицах.
    pub extent: Option<[f64; 4]>,
    /// Сведения модели: None - файлов нет, Some(None) - не читается.
    summary: Option<Option<Summary>>,
    /// Правка с прошлого кадра: окно глобуса пишет копию в метки.
    pub changed: bool,
    grip: Option<Grip>,
    hot: Option<Grip>,
}

impl OverlayProps {
    pub fn new(path: Vec<usize>, overlay: &Overlay) -> Self {
        let link = overlay.image.as_ref().is_none_or(Vec::is_empty) && !overlay.href.is_empty();
        let mut out = Self {
            path,
            edit: overlay.clone(),
            original: overlay.clone(),
            bytes: overlay.image.clone().filter(|b| !b.is_empty()),
            link,
            path_text: if link { overlay.href.clone() } else { String::new() },
            time_text: when::pack(overlay.time.as_ref()),
            extent: None,
            summary: None,
            changed: false,
            grip: None,
            hot: None,
        };
        out.model_changed();
        out
    }

    /// Новая картинка или пакет модели с «Обзор…»: в файле меток -
    /// копия, у ссылки - новый путь, если он есть.
    pub fn set_file(&mut self, bytes: Vec<u8>, name: &str, path: Option<String>) {
        match (self.link, path) {
            (true, Some(p)) => {
                self.path_text = p.clone();
                self.edit.href = p;
                self.edit.image = None;
                self.bytes = Some(bytes);
            }
            _ => {
                self.link = false;
                self.edit.href = name.to_string();
                self.edit.image = Some(bytes.clone());
                self.bytes = Some(bytes);
            }
        }
        self.model_changed();
        self.changed = true;
    }

    /// Пакет модели по ссылке, прочитанный окном глобуса: для рамки.
    pub fn set_model_bytes(&mut self, bytes: Vec<u8>) {
        self.bytes = Some(bytes);
        self.model_changed();
    }

    /// Коробка и сведения модели, как `_model_frame` и `model_summary`
    /// модуля.
    fn model_changed(&mut self) {
        if self.edit.kind() != Kind::Model {
            return;
        }
        self.extent = None;
        self.summary = self.bytes.as_deref().map(|bytes| {
            let parsed = crate::models::parse(bytes).ok()?;
            let [low, high] = scene::models::bounds(&parsed.parts)?;
            self.extent = Some([low.x, high.x, low.y, high.y]);
            let triangles = parsed.parts.iter().map(|p| p.positions.len()).sum::<usize>() / 3;
            Some(Summary { triangles, parts: parsed.parts.len(), images: parsed.images.len(), size: high - low })
        });
    }

    /// Название окна в панели свойств.
    pub fn title(&self, lang: Lang) -> &'static str {
        match self.edit.kind() {
            Kind::Ground => tr(lang, "Ground overlay"),
            Kind::Screen => tr(lang, "Screen overlay"),
            Kind::Photo => tr(lang, "Photo"),
            Kind::Model => tr(lang, "3D model"),
        }
    }

    /// Тело окна в панели свойств. Отдаёт итог: «OK», «Отмена»,
    /// «Обзор…» или открыто.
    pub fn body(&mut self, ui: &mut egui::Ui, lang: Lang) -> Outcome {
        let kind = self.edit.kind();
        let mut outcome = Outcome::Open;
        let before = self.edit.clone();
        {
            {
                egui::Grid::new("planetx-overlay-rows").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    self.common_rows(ui, lang, &mut outcome);
                    match kind {
                        Kind::Ground => self.ground_rows(ui, lang),
                        Kind::Screen => self.screen_rows(ui, lang),
                        Kind::Model => self.model_rows(ui, lang),
                        Kind::Photo => {}
                    }
                });
                if kind == Kind::Ground {
                    let hint = if self.edit.overlay.latlon_box.is_some() {
                        "The cross in the middle moves the image, the diamond rotates it, the corners and the middles of the sides stretch it, with Shift from the centre."
                    } else {
                        "The corners of the image are dragged with the mouse on the globe while the window is open."
                    };
                    ui.label(egui::RichText::new(tr(lang, hint)).weak());
                    if self.edit.overlay.latlon_box.is_some()
                        && ui.button(tr(lang, "Convert to four corners")).on_hover_text(tr(lang, "Each corner of the image is dragged with the mouse on its own, the image can become an irregular quadrilateral, as gx:LatLonQuad in KML. It does not convert back to a box.")).clicked()
                    {
                        let corners = self.edit.overlay.corners.clone();
                        self.edit.overlay.set_corners(corners);
                    }
                }
                if kind == Kind::Model
                    && let Some(summary) = &self.summary
                {
                    let text = match summary {
                        Some(s) => s.text(lang, self.edit.overlay.scale),
                        None => tr(lang, "The model cannot be read.").to_string(),
                    };
                    ui.label(egui::RichText::new(text).weak());
                }
            }
            ui.horizontal(|ui| {
                if ui.button(tr(lang, "OK")).clicked() {
                    outcome = Outcome::Ok;
                }
                if ui.button(tr(lang, "Cancel")).clicked() {
                    outcome = Outcome::Cancel;
                }
            });
        }
        if self.edit != before {
            self.changed = true;
        }
        if matches!(outcome, Outcome::Ok) {
            self.edit.time = self.time_value();
        }
        outcome
    }

    fn common_rows(&mut self, ui: &mut egui::Ui, lang: Lang, outcome: &mut Outcome) {
        let model = self.edit.kind() == Kind::Model;
        ui.label(tr(lang, "Name"));
        ui.add(egui::TextEdit::singleline(&mut self.edit.name).desired_width(240.0)).on_hover_text(tr(lang, "The name of the overlay in My Places."));
        ui.end_row();
        ui.label(tr(lang, "Description"));
        ui.add(egui::TextEdit::multiline(&mut self.edit.description).desired_width(240.0).desired_rows(3)).on_hover_text(tr(lang, "The description of the overlay. It goes into KML with the image."));
        ui.end_row();
        ui.label(tr(lang, "Time"));
        let bad = !self.time_text.trim().is_empty() && when::unpack(&self.time_text).is_none_or(|t| when::interval(Some(&t)).is_none());
        ui.add(egui::TextEdit::singleline(&mut self.time_text).hint_text(tr(lang, "1890 or 1890/1910")).desired_width(240.0).text_color_opt(bad.then_some(egui::Color32::from_rgb(220, 70, 60))))
            .on_hover_text(tr(lang, "The time of the image, as in KML. A year or a date is a moment, for a historical map it is the map year. Moment maps of one folder replace each other on the time slider - the last map not later than the slider moment is shown. Two values with a slash are a span, the image is shown within it. An empty field shows the image always."));
        ui.end_row();
        ui.label(tr(lang, "Storage"));
        let link_name = if model { tr(lang, "Link to a file") } else { tr(lang, "Link to a file or address") };
        let tip = if model {
            "Where the model files are. A link is a path to a .dae, .gltf, .glb or .obj file, the model is read from it and the files next to it, an edit in SketchUp or Blender shows on the globe. In the places file is a copy of all model files, it shows without the source file too."
        } else {
            "Where the image lies. In the places file - a copy inside My Places, it is shown without the source file. Link - a file path or a web address, the image is read when shown, an edit of the file shows on the globe."
        };
        let before = self.link;
        egui::ComboBox::from_id_salt("planetx-overlay-store")
            .selected_text(if self.link { link_name } else { tr(lang, "In the places file") })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.link, false, tr(lang, "In the places file"));
                ui.selectable_value(&mut self.link, true, link_name);
            })
            .response
            .on_hover_text(tr(lang, tip));
        if before != self.link {
            if self.link {
                self.path_text = if self.path_text.is_empty() { self.original.href.clone() } else { self.path_text.clone() };
                self.edit.href = self.path_text.clone();
                self.edit.image = None;
            } else {
                // Копия в файл меток - картинка, которая есть сейчас.
                self.edit.image = self.bytes.clone();
            }
        }
        ui.end_row();
        ui.label(if model { tr(lang, "Model file") } else { tr(lang, "Image") });
        ui.horizontal(|ui| {
            let hint = if model { tr(lang, "Path to the model file") } else { tr(lang, "File path or http(s) address") };
            let edit = ui.add_enabled(self.link, egui::TextEdit::singleline(&mut self.path_text).hint_text(hint).desired_width(170.0));
            let tip = if model {
                "Path to a .dae, .gltf, .glb or .obj model file. The model images are read from the files next to it."
            } else {
                "The path to the image file or its web address. The image is read again at each showing."
            };
            let edit = edit.on_hover_text(tr(lang, tip));
            if edit.lost_focus() && self.link && self.path_text.trim() != self.edit.href {
                self.edit.href = self.path_text.trim().to_string();
                self.bytes = None;
            }
            let tip = if model {
                "Choose a model file. In the places file the model is replaced by a copy, a link gets a new path. The place and the scale stay."
            } else {
                "Choose an image file. In the places file the image is replaced with a copy, a link gets the new path. The corners and position stay."
            };
            if ui.button(tr(lang, "Browse…")).on_hover_text(tr(lang, tip)).clicked() {
                *outcome = Outcome::Browse;
            }
        });
        ui.end_row();
        ui.label("");
        ui.label(egui::RichText::new(self.file_text(lang)).weak());
        ui.end_row();
        ui.label(tr(lang, "Refresh"));
        let mut refresh = self.edit.overlay.refresh;
        let none = tr(lang, "no refresh");
        let unit = tr(lang, " s");
        let low = overlays::MIN_REFRESH.to_string();
        let tip = if model {
            "A linked model file is checked at this interval while the model is visible. A changed file is read again. Zero means the file is read when shown. An interval shorter than {low} s is raised to {low} s."
        } else {
            "A linked image is read again after this interval while it is shown. The globe then keeps a fresh image or map that a server or a program updates by itself. Zero - the image is read when shown. An interval shorter than {low} s is raised to {low} s."
        };
        let r = ui.add_enabled(self.link, egui::DragValue::new(&mut refresh).range(0.0..=86400.0).speed(1.0).custom_formatter(move |v, _| if v <= 0.0 { none.to_string() } else { format!("{v:.0}{unit}") }));
        r.on_hover_text(trf(lang, tip, &[("low", &low)]));
        if refresh != self.edit.overlay.refresh {
            self.edit.overlay.refresh = overlays::refresh_interval(Some(refresh));
        }
        ui.end_row();
        ui.label(tr(lang, "Opacity"));
        let mut opacity = (f64::from(self.edit.overlay.color[3]) / 2.55).round();
        if ui.add(egui::Slider::new(&mut opacity, 0.0..=100.0).fixed_decimals(0).suffix(" %")).on_hover_text(tr(lang, "Opacity of the image. At zero the imagery shows through it.")).changed() {
            self.edit.overlay.color[3] = (opacity * 2.55).round().clamp(0.0, 255.0) as u8;
        }
        ui.end_row();
    }

    fn file_text(&self, lang: Lang) -> String {
        let model = self.edit.kind() == Kind::Model;
        if self.link {
            return if self.edit.href.is_empty() { tr(lang, "no link").to_string() } else { trf(lang, "by link {link}", &[("link", &self.edit.href)]) };
        }
        if model {
            return if self.bytes.is_some() { tr(lang, "In the places file").to_lowercase() } else { tr(lang, "no model files").to_string() };
        }
        match self.bytes.as_deref().and_then(|b| image::ImageReader::new(std::io::Cursor::new(b)).with_guessed_format().ok()?.into_dimensions().ok()) {
            Some((w, h)) => trf(lang, "{w} × {h} pixels", &[("w", &w.to_string()), ("h", &h.to_string())]),
            None => tr(lang, "no image").to_string(),
        }
    }

    fn ground_rows(&mut self, ui: &mut egui::Ui, lang: Lang) {
        let Some(mut frame) = self.edit.overlay.latlon_box else {
            return;
        };
        let rows: [(&str, f64, usize); 5] = [("North", 90.0, 0), ("South", 90.0, 1), ("East", 180.0, 2), ("West", 180.0, 3), ("Rotation", 180.0, 4)];
        let mut changed = false;
        for (name, limit, n) in rows {
            ui.label(tr(lang, name));
            let tip = if n == 4 { "Rotation of the image about its centre counterclockwise." } else { "An edge of the image box in degrees. The box is set by the cardinal sides and the rotation, as LatLonBox in KML." };
            let decimals = if n == 4 { 2 } else { 6 };
            changed |= ui.add(egui::DragValue::new(&mut frame[n]).range(-limit..=limit).fixed_decimals(decimals).suffix("°").speed(if n == 4 { 0.5 } else { 0.0001 })).on_hover_text(tr(lang, tip)).changed();
            ui.end_row();
        }
        if changed {
            self.edit.overlay.set_box(frame);
        }
    }

    fn screen_rows(&mut self, ui: &mut egui::Ui, lang: Lang) {
        let o = &mut self.edit.overlay;
        let current = CORNERS.iter().position(|(_, anchor, _)| (o.overlay_xy.0, o.overlay_xy.1) == *anchor);
        ui.label(tr(lang, "Site"));
        let mut chosen = current;
        egui::ComboBox::from_id_salt("planetx-overlay-corner")
            .selected_text(current.map_or("", |n| tr(lang, CORNERS[n].0)))
            .show_ui(ui, |ui| {
                for (n, (name, _, _)) in CORNERS.iter().enumerate() {
                    ui.selectable_value(&mut chosen, Some(n), tr(lang, name));
                }
            })
            .response
            .on_hover_text(tr(lang, "The corner of the view the image is pinned to."));
        ui.end_row();
        ui.label(tr(lang, "Width"));
        let mut width = if o.size.0 > 0.0 && o.size.2 == "fraction" { o.size.0 * 100.0 } else { 0.0 };
        let own = tr(lang, "own size");
        let resized = ui
            .add(egui::DragValue::new(&mut width).range(0.0..=100.0).speed(0.5).custom_formatter(move |v, _| if v <= 0.0 { own.to_string() } else { format!("{v:.0} %") }))
            .on_hover_text(tr(lang, "Width of the image as a share of the view width. Zero - the image at its own size in pixels."))
            .changed();
        ui.end_row();
        if chosen != current || resized {
            let n = chosen.unwrap_or(0);
            let (_, anchor, place) = CORNERS[n];
            let units = |p: f64| if p == 0.5 { "fraction" } else if p == 0.0 { "pixels" } else { "insetPixels" };
            let value = |p: f64| if p == 0.5 { p } else { MARGIN };
            o.overlay_xy = (anchor.0, anchor.1, "fraction".into(), "fraction".into());
            o.screen_xy = (value(place.0), value(place.1), units(place.0).into(), units(place.1).into());
            let share = width / 100.0;
            o.size = if share > 0.0 { (share, 0.0, "fraction".into(), "fraction".into()) } else { (-1.0, -1.0, "fraction".into(), "fraction".into()) };
        }
    }

    fn model_rows(&mut self, ui: &mut egui::Ui, lang: Lang) {
        let o = &mut self.edit.overlay;
        let mut camera = o.camera.unwrap_or([0.0; 6]);
        let rows: [(&str, f64, f64, usize, &str, &str, usize); 6] = [
            ("Latitude", -90.0, 90.0, 7, "°", "Latitude of the model point. The point is the origin of the model file coordinates.", 0),
            ("Longitude", -180.0, 180.0, 7, "°", "Longitude of the model point.", 1),
            ("Height", -11000.0, 100000.0, 2, " m", "Height of the model point. Above terrain is the rise above the ground, above sea level is the height from sea level. On terrain the height has no effect.", 2),
            ("Heading", -360.0, 360.0, 2, "°", "Clockwise turn of the model as seen from above. At zero the Y axis of the model points north.", 3),
            ("Tilt", -180.0, 180.0, 2, "°", "Tilt of the model around its X axis.", 4),
            ("Roll", -180.0, 180.0, 2, "°", "Roll of the model around its Y axis.", 5),
        ];
        let mut changed = false;
        for (name, low, high, decimals, suffix, tip, n) in rows {
            ui.label(tr(lang, name));
            let speed = if n < 2 { 0.00001 } else { 0.1 };
            changed |= ui.add(egui::DragValue::new(&mut camera[n]).range(low..=high).fixed_decimals(decimals).suffix(tr(lang, suffix)).speed(speed)).on_hover_text(tr(lang, tip)).changed();
            ui.end_row();
            if n == 1 {
                ui.label(tr(lang, "Altitude mode"));
                let names = [("clampToGround", "On terrain"), ("relativeToGround", "Above terrain"), ("absolute", "Above sea level")];
                let title = names.iter().find(|(m, _)| *m == o.mode).map_or("", |(_, t)| *t);
                egui::ComboBox::from_id_salt("planetx-model-mode")
                    .selected_text(tr(lang, title))
                    .show_ui(ui, |ui| {
                        for (mode, name) in names {
                            if ui.selectable_label(o.mode == mode, tr(lang, name)).clicked() {
                                o.mode = mode.to_string();
                            }
                        }
                    })
                    .response
                    .on_hover_text(tr(lang, "What the model height is measured from, as altitudeMode in KML."));
                ui.end_row();
            }
        }
        if changed {
            o.camera = Some(camera);
        }
        ui.label(tr(lang, "Scale"));
        let mut scale = o.scale[0];
        if ui.add(egui::DragValue::new(&mut scale).range(0.001..=10000.0).fixed_decimals(3).speed(0.01)).on_hover_text(tr(lang, "Scale of the model. One means the sizes from the model file. The field changes the scale of all axes at once, different axis scales from KML keep their ratio.")).changed() {
            let base = if o.scale[0] != 0.0 { o.scale[0] } else { 1.0 };
            let share = scale / base;
            o.scale = o.scale.map(|v| v * share);
        }
        ui.end_row();
    }

    /// Время из поля: пара строк [`when`] или None. Неверная запись -
    /// прежнее время, как `time_value` модуля.
    fn time_value(&self) -> Option<when::Time> {
        let time = when::unpack(&self.time_text);
        if time.as_ref().is_some_and(|t| when::interval(Some(t)).is_none()) {
            return self.original.time.clone();
        }
        time
    }

    /// Рамка ручек: картинки на поверхности или следа модели.
    fn frame(&self) -> Option<[f64; 5]> {
        let o = &self.edit.overlay;
        match self.edit.kind() {
            Kind::Ground => o.latlon_box,
            Kind::Model => Some(overlays::model_box(&o.camera?, o.scale, self.extent?)),
            _ => None,
        }
    }

    /// Ручки в пикселях кадра: ручка, точка, видна ли.
    fn grips(&self, nav: &Navigator) -> Vec<(Grip, [f64; 2], bool)> {
        let mut out = Vec::new();
        if let Some(frame) = self.frame() {
            let h = overlays::box_handles(frame);
            for (n, p) in h.corners.iter().enumerate() {
                out.push((Grip::Box(Handle::Corner, n), pixel(nav, *p)));
            }
            for (n, p) in h.sides.iter().enumerate() {
                out.push((Grip::Box(Handle::Side, n), pixel(nav, *p)));
            }
            out.push((Grip::Box(Handle::Center, 0), pixel(nav, h.center)));
            out.push((Grip::Box(Handle::Rotate, 0), pixel(nav, h.rotate)));
        } else if self.edit.kind() == Kind::Ground {
            for (n, p) in self.edit.overlay.corners.iter().enumerate() {
                out.push((Grip::Corner(n), pixel(nav, *p)));
            }
        }
        out.into_iter().map(|(g, (p, v))| (g, p, v)).collect()
    }

    /// Мышь над глобусом: ручки хватаются и тянутся. true - мышь занята.
    pub fn input(&mut self, ctx: &egui::Context, response: &egui::Response, rect: egui::Rect, ppp: f64, nav: &Navigator) -> bool {
        let (pressed, down, pos, shift) = ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.interact_pos(), i.modifiers.shift));
        let to_px = |p: egui::Pos2| (f64::from(p.x - rect.min.x) * ppp, f64::from(p.y - rect.min.y) * ppp);
        let on_globe = pos.is_some_and(|p| rect.contains(p) && ctx.layer_id_at(p).is_none_or(|l| l == response.layer_id));
        let near = |grips: &[(Grip, [f64; 2], bool)], p: egui::Pos2| {
            let (px, py) = to_px(p);
            grips
                .iter()
                .filter(|g| g.2)
                .map(|g| (g.0, (g.1[0] - px).hypot(g.1[1] - py)))
                .filter(|(_, d)| *d <= GRAB * ppp)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(g, _)| g)
        };
        let grips = self.grips(nav);
        self.hot = response.hover_pos().and_then(|p| near(&grips, p));
        if pressed && on_globe && let Some(p) = pos {
            self.grip = near(&grips, p);
        }
        let Some(grip) = self.grip else {
            return false;
        };
        if !down {
            self.grip = None;
            return true;
        }
        let ground = pos.and_then(|p| {
            let (px, py) = to_px(p);
            let point = ground_under(&nav.camera, px, py, nav.pose.terrain.as_ref())?;
            let (lat, lon, _) = nav.camera.ellipsoid.ecef_to_geodetic(point);
            Some((lat, lon))
        });
        let Some((lat, lon)) = ground else {
            return true;
        };
        let kind = self.edit.kind();
        let o = &mut self.edit.overlay;
        match grip {
            Grip::Corner(n) => {
                let mut corners = o.corners.clone();
                if let Some(c) = corners.get_mut(n) {
                    *c = (lat, lon);
                }
                o.set_corners(corners);
            }
            Grip::Box(handle, n) => match kind {
                Kind::Model => {
                    if let (Some(camera), Some(extent)) = (o.camera, self.extent) {
                        let (camera, scale) = overlays::model_drag(&camera, o.scale, extent, handle, n, lat, lon, shift);
                        o.camera = Some(camera);
                        o.scale = scale;
                    }
                }
                _ => {
                    if let Some(frame) = o.latlon_box {
                        o.set_box(overlays::box_drag(frame, handle, n, lat, lon, shift));
                    }
                }
            },
        }
        self.changed = true;
        true
    }

    /// Контур и ручки поверх глобуса, подсказка ручки под мышью.
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect, ppp: f64, nav: &Navigator, lang: Lang) {
        let to_screen = |p: [f64; 2]| rect.min + egui::vec2((p[0] / ppp) as f32, (p[1] / ppp) as f32);
        let outline: Vec<(f64, f64)> = match self.frame() {
            Some(frame) if self.edit.kind() == Kind::Model => overlays::box_corners(frame[0], frame[1], frame[2], frame[3], frame[4]),
            _ if self.edit.kind() == Kind::Ground => self.edit.overlay.corners.clone(),
            _ => Vec::new(),
        };
        let stroke = egui::Stroke::new(2.0, OUTLINE);
        for n in 0..outline.len() {
            let (a, b) = (pixel(nav, outline[n]), pixel(nav, outline[(n + 1) % outline.len()]));
            if a.1 && b.1 {
                painter.line_segment([to_screen(a.0), to_screen(b.0)], stroke);
            }
        }
        let dark = egui::Stroke::new(1.0, egui::Color32::from_gray(40));
        for (grip, p, visible) in self.grips(nav) {
            if !visible {
                continue;
            }
            let at = to_screen(p);
            let hot = self.hot == Some(grip) || self.grip == Some(grip);
            let fill = if hot { OUTLINE } else { egui::Color32::WHITE };
            match grip {
                Grip::Box(Handle::Center, _) => {
                    let s = 7.0;
                    painter.line_segment([at - egui::vec2(s, 0.0), at + egui::vec2(s, 0.0)], egui::Stroke::new(3.0, fill));
                    painter.line_segment([at - egui::vec2(0.0, s), at + egui::vec2(0.0, s)], egui::Stroke::new(3.0, fill));
                }
                Grip::Box(Handle::Rotate, _) => {
                    let s = 6.0;
                    let points = vec![at + egui::vec2(0.0, -s), at + egui::vec2(s, 0.0), at + egui::vec2(0.0, s), at + egui::vec2(-s, 0.0)];
                    painter.add(egui::Shape::convex_polygon(points, fill, dark));
                }
                Grip::Box(Handle::Side, _) => {
                    painter.circle(at, 3.5, fill, dark);
                }
                _ => {
                    painter.circle(at, 5.0, fill, dark);
                }
            }
        }
        if let (Some(grip), Some(pos)) = (self.hot, painter.ctx().pointer_hover_pos()) {
            let model = self.edit.kind() == Kind::Model;
            let text = match grip {
                Grip::Box(Handle::Side, _) if model => "Stretch the model along the side",
                Grip::Box(Handle::Side, _) => "Stretch the side",
                Grip::Box(Handle::Center, _) if model => "Move the model",
                Grip::Box(Handle::Center, _) => "Move the image",
                Grip::Box(Handle::Rotate, _) if model => "Rotate the model",
                Grip::Box(Handle::Rotate, _) => "Rotate the image",
                Grip::Box(Handle::Corner, _) if model => "Scale from the corner, with Shift from the centre",
                Grip::Box(Handle::Corner, _) => "Stretch from the corner, with Shift from the centre",
                Grip::Corner(_) => "",
            };
            if !text.is_empty() {
                painter.text(pos + egui::vec2(14.0, 14.0), egui::Align2::LEFT_TOP, tr(lang, text), egui::FontId::proportional(12.0), egui::Color32::WHITE);
            }
        }
    }
}

/// Сведения модели: треугольники, материалы, картинки, размер коробки
/// в единицах файла.
struct Summary {
    triangles: usize,
    parts: usize,
    images: usize,
    size: DVec3,
}

impl Summary {
    /// Текст под полями, как `model_summary` модуля: размеры в метрах.
    fn text(&self, lang: Lang, scale: [f64; 3]) -> String {
        let s = self.size * DVec3::new(scale[0].abs(), scale[1].abs(), scale[2].abs());
        trf(
            lang,
            "Triangles {count}, materials {parts}, images {images}. Size {x} × {y} × {z} m.",
            &[
                ("count", &self.triangles.to_string()),
                ("parts", &self.parts.to_string()),
                ("images", &self.images.to_string()),
                ("x", &format!("{:.1}", s.x)),
                ("y", &format!("{:.1}", s.y)),
                ("z", &format!("{:.1}", s.z)),
            ],
        )
    }
}

/// Точка экрана, кадровые пиксели, и видна ли.
fn pixel(nav: &Navigator, (lat, lon): (f64, f64)) -> ([f64; 2], bool) {
    let h = nav.pose.terrain.as_ref().map_or(0.0, |t| t(lat, lon));
    let point = nav.camera.ellipsoid.geodetic_to_ecef(lat, lon, h);
    let (px, front) = nav.camera.project(point);
    let facing = (nav.camera.eye - point).dot(point.normalize_or_zero()) > 0.0;
    ([px.x, px.y], front && facing)
}

/// Новое наложение по виду, как `add_overlay` и `add_model` модуля:
/// картинка на поверхности - рамкой в половину ширины вида, фото - по
/// камере глобуса, картинка на экране - в левом верхнем углу, модель -
/// в точке взгляда. aspect - ширина / высота картинки.
pub fn new_overlay(kind: Kind, name: &str, nav: &Navigator, aspect: f64) -> Overlay {
    let pose = &nav.pose;
    let camera = &nav.camera;
    let mut o = overlays::Overlay::new(kind);
    match kind {
        Kind::Ground => {
            let visible = 2.0 * pose.distance * (camera.fov_y.to_radians() / 2.0).tan() * camera.aspect();
            o.set_box(overlays::fit_box(pose.lat, pose.lon, overlays::NEW_SHARE * visible, aspect));
        }
        Kind::Photo => {
            let (lat, lon, alt) = camera.ellipsoid.ecef_to_geodetic(camera.eye);
            let (shot, fov, near) = overlays::view_camera(lat, lon, alt, pose.heading, pose.tilt, camera.fov_y, aspect, pose.distance);
            o.camera = Some(shot);
            o.fov = fov;
            o.near = near;
        }
        Kind::Screen => {
            o.overlay_xy = (0.0, 1.0, "fraction".into(), "fraction".into());
            o.screen_xy = (MARGIN, SCREEN_TOP, "pixels".into(), "insetPixels".into());
        }
        Kind::Model => {
            o.camera = Some([pose.lat, pose.lon, 0.0, 0.0, 0.0, 0.0]);
        }
    }
    let mut out = Overlay::new(name, o);
    if kind != Kind::Screen {
        out.view = scene::lookat::make(pose.lat, pose.lon, pose.distance, pose.heading, pose.tilt);
    }
    out
}

#[cfg(test)]
mod tests {
    use globe_core::camera::Camera;
    use globe_core::ellipsoid::Ellipsoid;
    use globe_core::navigation::{Navigator, Pose};
    use globe_core::{DMat3, DVec3};
    use scene::overlays::Kind;

    use super::{OverlayProps, new_overlay};

    fn nav() -> Navigator {
        let mut nav = Navigator::new(Camera::new(Ellipsoid::wgs84(), DVec3::ZERO, DMat3::IDENTITY), Pose::new(58.0, 56.0, 10000.0, 0.0, 0.0));
        nav.camera.width = 800;
        nav.camera.height = 600;
        nav
    }

    #[test]
    fn new_overlays_follow_the_view() {
        let nav = nav();
        // Картинка на поверхности - рамкой у точки взгляда, по пропорции.
        let ground = new_overlay(Kind::Ground, "map", &nav, 2.0);
        let frame = ground.overlay.latlon_box.unwrap_or_default();
        assert!(frame[0] > 58.0 && frame[1] < 58.0 && frame[2] > 56.0 && frame[3] < 56.0, "{frame:?}");
        assert_eq!(ground.overlay.corners.len(), 4);
        assert!(ground.view.is_some());
        // Картинка на экране - в левом верхнем углу, без вида.
        let screen = new_overlay(Kind::Screen, "logo", &nav, 1.0);
        assert_eq!((screen.overlay.overlay_xy.0, screen.overlay.overlay_xy.1), (0.0, 1.0));
        assert!(screen.view.is_none());
        // Модель - в точке взгляда.
        let model = new_overlay(Kind::Model, "house", &nav, 1.0);
        assert_eq!(model.overlay.camera.map(|c| (c[0], c[1])), Some((58.0, 56.0)));
        // Фото - по камере глобуса.
        assert!(new_overlay(Kind::Photo, "shot", &nav, 1.5).overlay.camera.is_some());
    }

    #[test]
    fn browse_replaces_the_image() {
        let mut o = new_overlay(Kind::Ground, "map", &nav(), 1.0);
        o.href = "a.png".into();
        o.image = Some(vec![1, 2, 3]);
        let mut props = OverlayProps::new(vec![0], &o);
        props.set_file(vec![4, 5], "b.png", Some("C:/b.png".into()));
        // Картинка в файле меток заменяется копией, углы остаются.
        assert_eq!(props.edit.image.as_deref(), Some(&[4u8, 5][..]));
        assert_eq!(props.edit.href, "b.png");
        assert_eq!(props.edit.overlay.corners, o.overlay.corners);
        assert!(props.changed);
    }
}