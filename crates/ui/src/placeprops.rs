// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Поля окна свойств метки «Моих меток», перенос `ui/placeprops.py`
//! модуля (`PlaceProperties`, `TimeField`): значок, размеры значка
//! и подписи, цвет подписи, заливка, поток по пути, отсчёт высоты,
//! высота с ползунком «Земля - Космос», стена до земли, высота облёта
//! тура, время, «Скрывать дальше», вид метки и поля объекта.
//!
//! Название, описание, цвет, толщину и вершины ведёт окно инструмента
//! ([`crate::tool::Tool`]), это окно - группы под ними. Правки уходят
//! в метку по «OK».
//!
//! Отличия от модуля. Время вводится строкой ISO 8601 UTC, а не полями
//! местных даты и времени. Снимок вида не запоминает строки «Слоёв»:
//! их ключи у Lite свои, флажок слоёв оставляет слои из файла.
//! Значки в списке без картинок.

use eframe::egui;
use scene::features::{self, Kind};
use scene::kml::Place;
use scene::lookat::{Layers, View};
use scene::when::{self, Time};
use scene::{icons, region, tour};

use crate::i18n::{Lang, tr, trf};

/// Цвет подписи по умолчанию - белый, как надпись метки на глобусе.
/// Такой цвет в файл не пишется.
pub const LABEL_COLOR: [u8; 4] = [255, 255, 255, 245];
/// Размеры значка и подписи - доли обычного, как scale KML. 0 - нет.
pub const SCALE_MAX: f64 = 4.0;
pub const SCALE_STEP: f64 = 0.25;
/// Наименьшая абсолютная высота, м: дно Марианского жёлоба около -11 км.
pub const MIN_ALTITUDE: f64 = -11000.0;
/// Заливка многоугольника по умолчанию, `DEFAULT_FILL` модуля.
pub const DEFAULT_FILL: [u8; 4] = [255, 255, 255, 60];
/// Расстояние вида по умолчанию, м.
const DEFAULT_RANGE: f64 = 3000.0;

/// Названия значков по порядку [`icons::ICONS`], `icon_names` модуля.
const ICON_NAMES: [&str; 35] = [
    "Circle", "Pushpin", "Flag", "Information", "Camera", "House", "Peak", "Forest", "Water", "Survey point", "Quarry", "Hiking", "Camp", "Car", "Bus", "Railway", "Tram", "Airport", "Ship",
    "Fuel", "Parking", "Hospital", "Pharmacy", "School", "Church", "Museum", "Hotel", "Restaurant", "Cafe", "Shop", "Police", "Fire station", "Post office", "Skiing", "Swimming",
];

/// Отсчёты высоты и их названия, как список модуля.
const MODES: [(&str, &str); 5] = [
    (features::CLAMP, "Clamped to ground"),
    (features::SEAFLOOR, "Clamped to sea floor"),
    (features::RELATIVE, "Relative to ground"),
    (features::RELATIVE_FLOOR, "Relative to sea floor"),
    (features::ABSOLUTE, "Absolute"),
];

fn color32(c: [u8; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

/// Время метки или вида, как TimeStamp и TimeSpan KML: нет, момент или
/// промежуток, `TimeField` модуля. Строки из файла, которые не правили,
/// остаются как были, в том числе неполные даты «2026-09».
#[derive(Debug, Clone)]
pub struct TimeEdit {
    original: Option<Time>,
    /// 0 - нет, 1 - момент, 2 - промежуток.
    mode: usize,
    begin: String,
    end: String,
    touched: bool,
}

impl TimeEdit {
    pub fn new(time: Option<&Time>) -> Self {
        let now = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
        let text = |s: Option<&String>| s.cloned().unwrap_or_else(|| when::text(now.floor()));
        let mode = match time {
            None => 0,
            Some((a, b)) if a == b => 1,
            Some(_) => 2,
        };
        Self { original: time.cloned(), mode, begin: text(time.map(|t| &t.0)), end: text(time.map(|t| &t.1)), touched: false }
    }

    /// Пара строк [`when`] или None. Неразборчивая строка - прежнее время.
    pub fn value(&self) -> Option<Time> {
        if !self.touched {
            return self.original.clone();
        }
        let ok = |s: &str| when::parse(s.trim()).is_some();
        match self.mode {
            0 => None,
            1 if ok(&self.begin) => Some(when::stamp(self.begin.trim())),
            2 if ok(&self.begin) && ok(&self.end) => Some((self.begin.trim().to_string(), self.end.trim().to_string())),
            _ => self.original.clone(),
        }
    }

    /// Поле: вид времени и строки ISO 8601.
    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang, id: &str, tip: &str) {
        ui.horizontal(|ui| {
            let names = [tr(lang, "None"), tr(lang, "Moment"), tr(lang, "Interval")];
            let before = self.mode;
            egui::ComboBox::from_id_salt(("planetx-time", id)).selected_text(names[self.mode.min(2)]).show_ui(ui, |ui| {
                for (n, name) in names.iter().enumerate() {
                    ui.selectable_value(&mut self.mode, n, *name);
                }
            });
            self.touched |= before != self.mode;
            let field = |ui: &mut egui::Ui, text: &mut String, on: bool| {
                let bad = on && when::parse(text.trim()).is_none();
                let edit = egui::TextEdit::singleline(text).desired_width(150.0).text_color_opt(bad.then_some(egui::Color32::from_rgb(220, 70, 60)));
                ui.add_enabled(on, edit).on_hover_text(tip).changed()
            };
            self.touched |= field(ui, &mut self.begin, self.mode != 0);
            if self.mode == 2 {
                ui.label("-");
                self.touched |= field(ui, &mut self.end, true);
            }
        });
    }
}

/// Поля свойств метки сверх названия, описания, цвета и толщины.
#[derive(Debug, Clone)]
pub struct PlaceFields {
    kind: Kind,
    icon: &'static str,
    icon_scale: f64,
    label_color: egui::Color32,
    label_scale: f64,
    fill: egui::Color32,
    flow: bool,
    mode: &'static str,
    height: f64,
    extrude: bool,
    /// У 3D-пути и 3D-многоугольника высоты у каждой вершины: отсчёт
    /// высоты не меняется.
    has_alts: bool,
    /// Записанный облёт: номера опорных поз и множитель высоты.
    pub tour_keys: Option<Vec<usize>>,
    factor: f64,
    time: TimeEdit,
    /// «Скрывать дальше», км, 0 - не скрывать.
    far: f64,
    old_region: Option<region::Region>,
    has_view: bool,
    view: View,
    old_view: Option<View>,
    view_time: TimeEdit,
    layers: Option<Layers>,
    keep_layers: bool,
    /// Пауза остановки тура, с, меньше нуля - как у тура.
    pause: f64,
    speed: f64,
    data: Vec<(String, String)>,
}

impl PlaceFields {
    pub fn new(place: &Place) -> Self {
        let (pause, speed) = place.pace.unwrap_or((None, 1.0));
        let mode = MODES.iter().map(|(m, _)| *m).find(|m| *m == place.mode).unwrap_or(features::CLAMP);
        Self {
            kind: place.kind,
            icon: icons::normal(&place.icon),
            icon_scale: place.icon_scale,
            label_color: color32(place.label_color.unwrap_or(LABEL_COLOR)),
            label_scale: place.label_scale,
            fill: color32(place.fill.unwrap_or(DEFAULT_FILL)),
            flow: place.flow,
            mode: if place.mode.is_empty() { features::CLAMP } else { mode },
            height: place.height,
            extrude: place.extrude,
            has_alts: place.alts.is_some(),
            tour_keys: place.tour.as_ref().map(|t| tour::key_indices(t, tour::KEY_POINTS)),
            factor: 1.0,
            time: TimeEdit::new(place.time.as_ref()),
            far: region::distance_km(place.region.as_ref()).map_or(0.0, |km| km.min(crate::folderprops::MAX_KM)),
            old_region: place.region,
            has_view: place.view.is_some(),
            view: place.view.unwrap_or_else(|| default_view(&place.points)),
            old_view: place.view,
            view_time: TimeEdit::new(place.view_time.as_ref()),
            layers: place.layers.clone(),
            keep_layers: place.layers.is_some(),
            pause: pause.unwrap_or(-1.0),
            speed,
            data: place.data.clone().unwrap_or_default(),
        }
    }

    /// Опорные точки облёта для правки мышью, как `key_points` модуля.
    pub fn tour_points(place: &Place, keys: &[usize]) -> Vec<(f64, f64)> {
        let samples = place.tour.as_deref().unwrap_or(&[]);
        keys.iter().filter_map(|k| samples.get(*k)).map(|s| (s[1], s[2])).collect()
    }

    /// Группы окна. current - вид глобуса сейчас для «Снимок текущего
    /// вида». point_color - цвет точки (у значка).
    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang, current: View) {
        let group = |ui: &mut egui::Ui, title: &str, body: &mut dyn FnMut(&mut egui::Ui)| {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(title).strong());
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                body(ui);
            });
        };
        group(ui, tr(lang, "Appearance"), &mut |ui| self.look(ui, lang));
        group(ui, tr(lang, "Height"), &mut |ui| self.heights(ui, lang));
        group(ui, tr(lang, "Time and visibility"), &mut |ui| {
            egui::Grid::new("planetx-props-time").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Time"));
                self.time.ui(
                    ui,
                    lang,
                    "own",
                    tr(lang, "The placemark's own time, as TimeStamp and TimeSpan in KML. A placemark with a time shows while its time falls into the interval of the time slider at the top of the view. A placemark without a time always shows."),
                );
                ui.end_row();
                ui.label(tr(lang, "Hide beyond"));
                let none = tr(lang, "do not hide");
                let km = tr(lang, " km");
                ui.add(egui::DragValue::new(&mut self.far).range(0.0..=crate::folderprops::MAX_KM).fixed_decimals(1).custom_formatter(move |v, _| if v <= 0.0 { none.to_string() } else { format!("{v:.1}{km}") }))
                    .on_hover_text(tr(lang, "The placemark is seen only while the eye is closer than this distance, as Region in KML. For a folder - all its placemarks. 0 - always seen."));
                ui.end_row();
            });
        });
        self.view_group(ui, lang, current);
        self.fields_group(ui, lang);
    }

    fn look(&mut self, ui: &mut egui::Ui, lang: Lang) {
        egui::Grid::new("planetx-props-look").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            if self.kind == Kind::Point {
                ui.label(tr(lang, "Icon"));
                let at = icons::ids().position(|i| i == self.icon).unwrap_or(0);
                egui::ComboBox::from_id_salt("planetx-props-icon")
                    .selected_text(tr(lang, ICON_NAMES.get(at).copied().unwrap_or("Circle")))
                    .show_ui(ui, |ui| {
                        for (id, name) in icons::ids().zip(ICON_NAMES) {
                            ui.selectable_value(&mut self.icon, id, tr(lang, name));
                        }
                    })
                    .response
                    .on_hover_text(tr(lang, "Icon of the point on the globe and in the list. In KML it goes as the address of a standard icon of the same theme."));
                ui.end_row();
                scale_row(ui, lang, "Icon size", &mut self.icon_scale, "No icon", "Icon size as a share of the usual one, as the IconStyle scale in KML. 0 - only the label on the globe.");
                ui.label(tr(lang, "Label colour"));
                ui.color_edit_button_srgba(&mut self.label_color)
                    .on_hover_text(tr(lang, "Colour of the placemark label on the globe, as LabelStyle in KML. A dark label gets a light outline."));
                ui.end_row();
                scale_row(ui, lang, "Label size", &mut self.label_scale, "No label", "Label size as a share of the usual one. 0 - only the icon on the globe, the name stays in the list.");
            }
            if self.kind == Kind::Line {
                ui.label("");
                ui.checkbox(&mut self.flow, tr(lang, "Flow along the path")).on_hover_text(tr(
                    lang,
                    "Short dashes of the line colour run along the path from the first point to the last one, like a flow of goods. The path lies on the ground. The setting is kept in KML.",
                ));
                ui.end_row();
            }
            if self.kind == Kind::Polygon {
                ui.label(tr(lang, "Fill"));
                ui.color_edit_button_srgba(&mut self.fill).on_hover_text(tr(lang, "Fill color of the polygon, with its transparency. The wall down to the ground takes this color too."));
                ui.end_row();
            }
        });
    }

    fn heights(&mut self, ui: &mut egui::Ui, lang: Lang) {
        egui::Grid::new("planetx-props-height").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label(tr(lang, "Altitude mode"));
            let title = MODES.iter().find(|(m, _)| *m == self.mode).map_or("", |(_, t)| *t);
            ui.add_enabled_ui(!self.has_alts, |ui| {
                egui::ComboBox::from_id_salt("planetx-props-mode")
                    .selected_text(tr(lang, title))
                    .show_ui(ui, |ui| {
                        for (mode, name) in MODES {
                            ui.selectable_value(&mut self.mode, mode, tr(lang, name));
                        }
                    })
                    .response
                    .on_hover_text(tr(lang, "What the altitude is measured from. Clamped to ground - the placemark lies on the ground, over the sea on the water. Clamped to sea floor - on the floor. Relative to ground and to sea floor - the rise above them. Absolute - the altitude above sea level. The mode is saved in KML."));
            });
            ui.end_row();
            // Высота и стена - в режимах с высотой, отрицательная высота -
            // только у абсолютной, это впадины ниже уровня моря.
            let on = features::uses_height(self.mode);
            if self.mode != features::ABSOLUTE && self.height < 0.0 {
                self.height = 0.0;
            }
            let low = if self.mode == features::ABSOLUTE { MIN_ALTITUDE } else { 0.0 };
            ui.label(tr(lang, "Height"));
            ui.add_enabled(on, egui::DragValue::new(&mut self.height).range(low..=features::MAX_HEIGHT).fixed_decimals(1).suffix(tr(lang, " m")))
                .on_hover_text(tr(lang, "The rise above the surface or the sea floor, in the absolute mode the altitude above sea level."));
            ui.end_row();
            ui.label("");
            ui.add_enabled_ui(on, |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr(lang, "Earth"));
                    let mut share = features::height_share(self.height);
                    let slider = ui.add(egui::Slider::new(&mut share, 0.0..=1.0).show_value(false)).on_hover_text(tr(
                        lang,
                        "Height by slider. The scale is logarithmic, near the ground the step is metres, higher up hundreds of metres and kilometres.",
                    ));
                    if slider.changed() {
                        // Ниже 100 м - метры, выше - десятки метров.
                        let h = features::share_height(share);
                        self.height = if h < 100.0 { h.round() } else { (h / 10.0).round() * 10.0 };
                    }
                    ui.label(tr(lang, "Space"));
                });
            });
            ui.end_row();
            ui.label("");
            ui.add_enabled(on, egui::Checkbox::new(&mut self.extrude, tr(lang, "Extend to ground")))
                .on_hover_text(tr(lang, "A wall from the raised object down to the ground, a post for a point. Works with a height above zero."));
            ui.end_row();
            if self.tour_keys.is_some() {
                ui.label(tr(lang, "Fly-around height"));
                let (low, high) = tour::HEIGHT_FACTORS;
                ui.add(egui::Slider::new(&mut self.factor, low..=high).logarithmic(true).custom_formatter(|v, _| format!("×{v:.2}"))).on_hover_text(tr(
                    lang,
                    "The fly-around camera is closer to the ground or farther from it in all recorded poses. The middle is the recorded height, the ends are four times lower and four times higher. Path points are dragged with the mouse on the globe, the poses between them follow.",
                ));
                ui.end_row();
            }
        });
    }

    fn view_group(&mut self, ui: &mut egui::Ui, lang: Lang, current: View) {
        ui.add_space(4.0);
        ui.checkbox(&mut self.has_view, egui::RichText::new(tr(lang, "Place view")).strong()).on_hover_text(tr(
            lang,
            "Where the camera looks from when it flies to the place or stands at it in a tour. Without a view of its own the camera frames the whole place.",
        ));
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.add_enabled_ui(self.has_view, |ui| {
                crate::folderprops::view_fields(ui, lang, &mut self.view);
                egui::Grid::new("planetx-props-view").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label(tr(lang, "Date/time"));
                    self.view_time.ui(ui, lang, "view", tr(lang, "Date and time of the view. A flight to the placemark and a tour set the time slider to this time."));
                    ui.end_row();
                    ui.label("");
                    let count = scene::lookat::switches(self.layers.as_ref()).values().filter(|on| **on).count();
                    ui.add_enabled(self.layers.is_some(), egui::Checkbox::new(&mut self.keep_layers, trf(lang, "Globe layers from the snapshot, rows switched on: {count}", &[("count", &count.to_string())])))
                        .on_hover_text(tr(lang, "Rows of the Layers section that were switched on at the view snapshot. A flight to the placemark and a tour switch them on and the others off - so a tour stop shows its own layers. A new snapshot of the current view takes them again."));
                    ui.end_row();
                    ui.label(tr(lang, "Tour pause"));
                    let same = tr(lang, "as in the tour");
                    let unit = tr(lang, " s");
                    ui.add(egui::DragValue::new(&mut self.pause).range(-1.0..=120.0).speed(0.1).custom_formatter(move |v, _| if v < 0.0 { same.to_string() } else { format!("{v:.1}{unit}") }))
                        .on_hover_text(tr(lang, "How many seconds the camera stays at this tour stop. As in the tour - the pause from the tour panel."));
                    ui.end_row();
                    ui.label(tr(lang, "Flight speed"));
                    let (low, high) = tour::SPEED_RANGE;
                    ui.add(egui::DragValue::new(&mut self.speed).range(low..=high).speed(0.01).fixed_decimals(2).prefix("×"))
                        .on_hover_text(tr(lang, "How many times faster than usual the tour flies to this placemark. Below one - slower, smoother for a show."));
                    ui.end_row();
                });
            });
            ui.horizontal(|ui| {
                if ui.button(tr(lang, "Snapshot current view")).on_hover_text(tr(lang, "Take the view of the globe now: the point of view, distance, heading and tilt.")).clicked() {
                    self.view = current;
                    self.has_view = true;
                }
                if ui.button(tr(lang, "Reset")).on_hover_text(tr(lang, "Return the view the place had when the window opened.")).clicked() {
                    self.has_view = self.old_view.is_some();
                    if let Some(v) = self.old_view {
                        self.view = v;
                    }
                }
            });
        });
    }

    fn fields_group(&mut self, ui: &mut egui::Ui, lang: Lang) {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tr(lang, "Object fields")).strong()).on_hover_text(tr(
            lang,
            "Object fields, as ExtendedData in KML. The placemark card and the Objects window show them, Explain this passes them to the assistant. A field without a name is not saved.",
        ));
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let mut remove = None;
            egui::Grid::new("planetx-props-fields").num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
                ui.label(egui::RichText::new(tr(lang, "Field")).weak());
                ui.label(egui::RichText::new(tr(lang, "Value")).weak());
                ui.label("");
                ui.end_row();
                for (n, (name, value)) in self.data.iter_mut().enumerate() {
                    ui.add(egui::TextEdit::singleline(name).desired_width(110.0));
                    ui.add(egui::TextEdit::singleline(value).desired_width(150.0));
                    if ui.small_button("×").on_hover_text(tr(lang, "Remove field")).clicked() {
                        remove = Some(n);
                    }
                    ui.end_row();
                }
            });
            if let Some(n) = remove {
                self.data.remove(n);
            }
            if ui.button(tr(lang, "Add field")).clicked() {
                self.data.push((String::new(), String::new()));
            }
        });
    }

    /// Правки окна в метку place. points - вершины формы с глобуса,
    /// у облёта - опорные точки.
    pub fn apply(&self, place: &mut Place, points: &[(f64, f64)]) {
        if self.kind == Kind::Point {
            place.icon = self.icon.to_string();
            place.icon_scale = self.icon_scale;
            let label = self.label_color.to_srgba_unmultiplied();
            place.label_color = (label != LABEL_COLOR).then_some(label);
            place.label_scale = self.label_scale;
        }
        if self.kind == Kind::Line {
            place.flow = self.flow;
        }
        if self.kind == Kind::Polygon {
            place.fill = Some(self.fill.to_srgba_unmultiplied());
        }
        if !self.has_alts {
            place.mode = if self.mode == features::CLAMP && place.mode.is_empty() { String::new() } else { self.mode.to_string() };
        }
        place.height = self.height;
        place.extrude = self.extrude;
        place.time = self.time.value();
        place.region = self.region(points);
        place.view = if self.has_view { scene::lookat::make(self.view[0], self.view[1], self.view[2], self.view[3], self.view[4]) } else { None };
        place.view_time = if self.has_view { self.view_time.value() } else { None };
        place.layers = if self.has_view && self.keep_layers { self.layers.clone() } else { None };
        let pace = (if self.pause < 0.0 { None } else { Some(self.pause) }, self.speed);
        place.pace = (pace != (None, 1.0)).then_some(pace);
        let data: Vec<(String, String)> = self.data.iter().map(|(n, v)| (n.trim().to_string(), v.clone())).filter(|(n, _)| !n.is_empty()).collect();
        place.data = (!data.is_empty()).then_some(data);
        if let (Some(keys), Some(samples)) = (&self.tour_keys, &place.tour) {
            let before = Self::tour_points(place, keys);
            if before != points || (self.factor - 1.0).abs() > 1e-9 {
                let edited = tour::edit_samples(samples, keys, points, self.factor);
                place.points = edited.iter().map(|s| (s[1], s[2])).collect();
                place.tour = Some(edited);
            }
        }
    }

    /// Region по полю, как у окна папки: 0 - без нижнего предела.
    fn region(&self, points: &[(f64, f64)]) -> Option<region::Region> {
        if self.far > 0.0 && !points.is_empty() {
            return Some(region::for_distance(points, self.far, self.old_region.as_ref()));
        }
        if self.far > 0.0 {
            return self.old_region;
        }
        self.old_region.filter(|r| r.max_lod >= 0.0).map(|r| region::Region { min_lod: 0.0, ..r })
    }
}

/// Поле размера от 0 до SCALE_MAX, 0 показывается словом zero.
fn scale_row(ui: &mut egui::Ui, lang: Lang, name: &'static str, value: &mut f64, zero: &'static str, tip: &'static str) {
    ui.label(tr(lang, name));
    let zero = tr(lang, zero);
    ui.add(egui::DragValue::new(value).range(0.0..=SCALE_MAX).speed(SCALE_STEP / 10.0).custom_formatter(move |v, _| if v <= 0.0 { zero.to_string() } else { format!("{v:.2}") }))
        .on_hover_text(tr(lang, tip));
    ui.end_row();
}

/// Вид без своего вида метки: точка взгляда - середина охвата, отвесно,
/// `_default_view` модуля.
fn default_view(points: &[(f64, f64)]) -> View {
    let (n, s, e, w) = region::bounding_box(points);
    let lat = if s.is_finite() && n.is_finite() { (s + n) / 2.0 } else { 0.0 };
    let lon = if w.is_finite() && e.is_finite() { (w + e) / 2.0 } else { 0.0 };
    [lat, lon, DEFAULT_RANGE, 0.0, 0.0]
}

#[cfg(test)]
mod tests {
    use super::{PlaceFields, TimeEdit};
    use scene::features::Kind;
    use scene::kml::Place;

    #[test]
    fn untouched_fields_keep_the_place() {
        let mut place = Place::new("a", Kind::Point, vec![(58.0, 56.0)]);
        place.time = Some(("2026-09".into(), "2026-09".into()));
        place.data = Some(vec![("kind".into(), "mine".into())]);
        place.icon = "flag".into();
        let fields = PlaceFields::new(&place);
        let mut back = place.clone();
        fields.apply(&mut back, &place.points);
        // Неполная дата из файла остаётся, как в модуле.
        assert_eq!(back.time, place.time);
        assert_eq!(back.data, place.data);
        assert_eq!(back.icon, "flag");
        assert_eq!(back.view, None);
    }

    #[test]
    fn time_edit_values() {
        let mut edit = TimeEdit::new(None);
        assert_eq!(edit.value(), None);
        edit.touched = true;
        edit.mode = 1;
        edit.begin = "2026-10-24T12:00:00Z".into();
        assert_eq!(edit.value(), Some(("2026-10-24T12:00:00Z".into(), "2026-10-24T12:00:00Z".into())));
        edit.mode = 2;
        edit.end = "не дата".into();
        // Неразборчивая строка - прежнее время.
        assert_eq!(edit.value(), None);
    }
}
