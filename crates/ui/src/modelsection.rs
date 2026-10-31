// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Разрез модели»: подземная модель вниз вдоль пути, перенос
//! `ui/subsection.py` модуля (`build`, `ModelChart`, `ModelSectionDialog`).
//!
//! Окно «Разрез» режет Землю на сотни километров, это окно - модель
//! подземного режима на сотни метров: рельеф, пласты между кровлями, стволы
//! скважин и тоннели из полосы вдоль линии. Расстояния и отметки - в метрах.
//! Курсор над графиком отмечает точку на глобусе. Слои разреза выбираются
//! флажками в окне, правая кнопка задаёт цвет пласта, породы между
//! пластами и линии кровли. Разрез пишется картинкой PNG и таблицей CSV
//! отметок по точкам линии.
//!
//! Слои с Z - линии и многоугольники с отметками из файла модели. Растров
//! проекта QGIS у Lite нет, кровли - из таблицы пластов модели.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use data::drillholes;
use data::section;
use data::subsurface as core;
use eframe::egui;

use crate::i18n::{Lang, tr, trf};
use crate::subsurface::{self, Ground, HOST_COLOR, Model};

/// Точек вдоль линии.
const SAMPLES: usize = 400;
/// Полоса скважин и тоннелей вдоль линии, м, выбор помощника модуля.
const BAND: f64 = 200.0;
/// Доля высоты графика над наивысшей точкой рельефа.
const HEAD: f64 = 0.08;
/// На столько метров ниже самого глубокого тоннеля, если нет пластов.
const TUNNEL_MARGIN: f64 = 20.0;
/// Кровель в подписи под курсором.
const HOVER_BEDS: usize = 4;
/// Строк подписи цветов, не больше.
const LEGEND_ROWS: usize = 12;
/// Линии кровель и подошв, пиксели.
const LINE_WIDTH: f32 = 1.6;
/// Линия под курсором для меню цвета, пиксели.
const LINE_PICK: f64 = 6.0;
/// Высота строки итогов и кнопок под графиком.
const FOOT: f32 = 40.0;
/// Ширина списка слоёв слева.
const LIST_WIDTH: f32 = 190.0;
const HOVER: egui::Color32 = egui::Color32::from_rgb(220, 60, 40);

/// Промежуток разреза: код, цвет, верх и низ по точкам линии.
pub type Bed = (String, [u8; 3], Vec<f64>, Vec<f64>);
/// Кусок ствола: расстояния, отметки, цвет.
pub type Piece = (Vec<f64>, Vec<f64>, [u8; 3]);
/// Тоннель на разрезе: название, расстояния, отметки, цвет.
pub type TunnelRun = (String, Vec<f64>, Vec<f64>, [u8; 3]);
/// Часть объекта с Z: слой, расстояния, отметки, цвет, многоугольник ли.
pub type Fence = (String, Vec<f64>, Vec<f64>, [u8; 3], bool);

/// Разрез модели, `ModelSection` модуля. distance - м от начала линии.
pub struct ModelSection {
    pub distance: Vec<f64>,
    pub lats: Vec<f64>,
    pub lons: Vec<f64>,
    /// Отметки рельефа.
    pub ground: Vec<f64>,
    pub beds: Vec<Bed>,
    /// Ключ своего цвета каждого промежутка: код пласта или
    /// [`core::HOST`] - порода между пластами.
    pub owners: Vec<String>,
    /// Кровли и подошвы: код, цвет, отметки.
    pub lines: Vec<(String, [u8; 3], Vec<f64>)>,
    /// Отметка низа модели.
    pub bottom: f64,
    /// Стволы: название и куски по интервалам.
    pub holes: Vec<(String, Vec<Piece>)>,
    pub tunnels: Vec<TunnelRun>,
    /// Диаметр тоннеля по названию, м.
    pub diameters: HashMap<String, f64>,
    /// Части объектов с Z в полосе.
    pub fences: Vec<Fence>,
}

/// Ключ слоя с Z среди выключенных: коды кровель с ним не совпадут.
fn z_key(layer: &str) -> String {
    format!("z:{layer}")
}

/// Отрезки подряд идущих true: (начало, конец не включая).
fn runs(mask: &[bool]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, on) in mask.iter().copied().chain([false]).enumerate() {
        match (on, start) {
            (true, None) => start = Some(i),
            (false, Some(a)) => {
                out.push((a, i));
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// Разрез модели вдоль points (широта, долгота) или None, `build` модуля.
/// ground - настоящие отметки рельефа, band - полоса скважин и тоннелей,
/// м, off - коды выключенных кровель и ключи слоёв с Z, holes_on -
/// скважины на разрезе.
pub fn build(model: &Model, ground: &Ground<'_>, points: &[(f64, f64)], band: f64, off: &BTreeSet<String>, holes_on: bool) -> Option<ModelSection> {
    let (km, lats, lons) = section::along(points, SAMPLES)?;
    let distance: Vec<f64> = km.iter().map(|k| k * 1000.0).collect();
    let pairs = |la: &[f64], lo: &[f64]| -> Vec<(f64, f64)> { la.iter().copied().zip(lo.iter().copied()).collect() };
    let g = ground(&pairs(&lats, &lons));
    if g.len() != lats.len() {
        return None;
    }
    let tunnel_line = |t: &subsurface::Tunnel| {
        let (tl, to): (Vec<f64>, Vec<f64>) = t.points.iter().map(|p| (p[0], p[1])).unzip();
        let tg = ground(&pairs(&tl, &to));
        let share = core::along(&tl, &to);
        let z: Vec<f64> = tg.iter().zip(&share).map(|(g, s)| core::tunnel_z(*g, t.depths.0, t.depths.1, *s)).collect();
        (tl, to, z)
    };
    let mut bottom = model.bottom();
    if model.holes.is_empty() && model.horizons.is_empty() && !model.tunnels.is_empty() {
        // Модель из одних тоннелей: низ - под самым глубоким из них.
        let low = model.tunnels.iter().map(|t| tunnel_line(t).2.iter().copied().fold(f64::INFINITY, f64::min) - t.diameter).fold(f64::INFINITY, f64::min);
        bottom = low - TUNNEL_MARGIN;
    }
    let horizons: Vec<_> = model.horizons.iter().filter(|h| !off.contains(&h.code)).collect();
    let mut surfaces = vec![g.clone()];
    surfaces.extend(horizons.iter().map(|h| h.at(&lats, &lons)));
    surfaces.push(vec![bottom; lats.len()]);
    let stack = core::stack(&surfaces);
    let mut codes = vec![model.beds.first().cloned().unwrap_or_default()];
    codes.extend(horizons.iter().map(|h| h.code.clone()));
    let fills = core::section_fills(&codes, |c| model.color(c), &HOST_COLOR);
    let beds: Vec<Bed> = codes
        .iter()
        .zip(&fills)
        .zip(stack.windows(2))
        .map(|((code, (_, color)), pair)| (code.clone(), *color, pair[0].clone(), pair[1].clone()))
        .collect();
    let owners = fills.iter().map(|(key, _)| key.clone()).collect();
    // Линии кровель и подошв - по поверхностям под рельефом.
    let lines = codes.iter().zip(&stack).skip(1).map(|(code, z)| (code.clone(), core::line_color(model.color(code)), z.clone())).collect();
    let mut holes = Vec::new();
    for (hole, lat0, lon0) in model.holes.iter().filter(|_| holes_on) {
        let place = |nodes: &[[f64; 2]]| -> (Vec<f64>, Vec<f64>) { nodes.iter().map(|p| subsurface::to_latlon(*lat0, *lon0, p[0], p[1])).unzip() };
        let axis: Vec<[f64; 2]> = hole.axis.iter().map(|n| [n[1], n[2]]).collect();
        let (hl, ho) = place(&axis);
        let gap = core::to_line(&lats, &lons, &distance, &hl, &ho).iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        if gap > band / 2.0 {
            continue;
        }
        let whole = [(0.0, hole.eoh, String::new())];
        let spans: Vec<(f64, f64, String)> = hole.intervals.iter().map(|i| (i.start, i.end, i.code.clone())).collect();
        let spans = if spans.is_empty() { &whole[..] } else { &spans[..] };
        let mut pieces = Vec::new();
        for (start, end, code) in spans {
            let part = drillholes::piece(&hole.axis, *start, *end);
            let flat: Vec<[f64; 2]> = part.iter().map(|p| [p[0], p[1]]).collect();
            let (pl, po) = place(&flat);
            let d = core::to_line(&lats, &lons, &distance, &pl, &po).iter().map(|p| p.0).collect();
            pieces.push((d, part.iter().map(|p| p[2]).collect(), model.color(code)));
        }
        holes.push((hole.hole_id.clone(), pieces));
    }
    let mut tunnels = Vec::new();
    let mut diameters: HashMap<String, f64> = HashMap::new();
    for t in &model.tunnels {
        let slot = diameters.entry(t.name.clone()).or_insert(0.0);
        *slot = slot.max(t.diameter);
        let (tl, to, z) = tunnel_line(t);
        let found = core::to_line(&lats, &lons, &distance, &tl, &to);
        let near: Vec<bool> = found.iter().map(|p| p.1 <= band / 2.0).collect();
        for (a, b) in runs(&near).into_iter().filter(|(a, b)| b - a >= 2) {
            tunnels.push((t.name.clone(), found[a..b].iter().map(|p| p.0).collect(), z[a..b].to_vec(), t.color));
        }
    }
    let mut fences = Vec::new();
    for shape in model.zshapes.iter().filter(|s| !off.contains(&z_key(&s.layer))) {
        for part in &shape.parts {
            if let Some((d, z)) = core::fence_part(part, &lats, &lons, &distance, band) {
                fences.push((shape.layer.clone(), d, z, shape.color, shape.polygon));
            }
        }
    }
    Some(ModelSection { distance, lats, lons, ground: g, beds, owners, lines, bottom, holes, tunnels, diameters, fences })
}

/// Поле CSV: в кавычках, если в нём запятая, кавычка или перевод строки.
fn field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) { format!("\"{}\"", text.replace('"', "\"\"")) } else { text.to_string() }
}

/// Отметки разреза таблицей CSV, `save_table` модуля: расстояние, широта,
/// долгота, рельеф и кровли пластов, нет кровли - пустое поле.
pub fn table_text(s: &ModelSection) -> String {
    let rows = s.beds.get(1..).unwrap_or(&[]);
    let mut head = vec!["distance_m".to_string(), "lat".into(), "lon".into(), "ground_m".into()];
    head.extend(rows.iter().map(|(code, ..)| field(if code.is_empty() { "top" } else { code })));
    let mut out = head.join(",") + "\n";
    for i in 0..s.distance.len() {
        let mut row = vec![format!("{:.1}", s.distance[i]), format!("{:.7}", s.lats[i]), format!("{:.7}", s.lons[i]), format!("{:.2}", s.ground[i])];
        row.extend(rows.iter().map(|(_, _, top, bottom)| if top[i].is_finite() && top[i] > bottom[i] { format!("{:.2}", top[i]) } else { String::new() }));
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}

/// Свой цвет: заливка и линия.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Own {
    fill: Option<[u8; 3]>,
    line: Option<[u8; 3]>,
}

/// Что перестраивает разрез: полоса, выключенные кровли, скважины, номер
/// модели.
type Key = (u64, BTreeSet<String>, bool, u64);
/// Картинка кадра, которую ждёт «Картинка…».
type Shot = Arc<Mutex<Option<Result<egui::ColorImage, String>>>>;

/// Окно «Разрез модели».
pub struct ModelSectionWindow {
    pub title: String,
    points: Vec<(f64, f64)>,
    band: f64,
    /// Коды выключенных кровель.
    off: BTreeSet<String>,
    holes_on: bool,
    built: Option<ModelSection>,
    key: Option<Key>,
    built_at: f64,
    built_version: u64,
    /// Точка разреза под курсором: широта, долгота, м вдоль линии.
    pub hovered: Option<(f64, f64, f64)>,
    own: HashMap<String, Own>,
    /// Свои цвета сменились: их забирает окно глобуса в настройки.
    pub colors_changed: bool,
    /// Промежуток и линия под правой кнопкой.
    menu: (Option<usize>, Option<String>),
    /// Кровли модели и есть ли скважины - для списка слоёв.
    roofs: Vec<String>,
    has_holes: bool,
    /// Слои с Z модели.
    zlayers: Vec<String>,
    plot_rect: egui::Rect,
    shot: Option<Shot>,
    status: String,
}

/// Отметка в метрах словами.
fn metres_text(lang: Lang, value: f64) -> String {
    trf(lang, "{value} m", &[("value", &format!("{value:.0}"))])
}

fn rgb(c: [u8; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(c[0], c[1], c[2])
}

impl ModelSectionWindow {
    /// Окно для пути points. colors - свои цвета из настроек: ключ,
    /// заливка, линия.
    pub fn new(title: String, points: Vec<(f64, f64)>, colors: &[crate::settings::SectionColor]) -> Self {
        let own = colors.iter().map(|(key, fill, line)| (key.clone(), Own { fill: *fill, line: *line })).collect();
        Self {
            title,
            points,
            band: BAND,
            off: BTreeSet::new(),
            holes_on: true,
            built: None,
            key: None,
            built_at: f64::NEG_INFINITY,
            built_version: u64::MAX,
            hovered: None,
            own,
            colors_changed: false,
            menu: (None, None),
            roofs: Vec::new(),
            has_holes: false,
            zlayers: Vec::new(),
            plot_rect: egui::Rect::NOTHING,
            shot: None,
            status: String::new(),
        }
    }

    /// Другой путь в открытом окне.
    pub fn set_source(&mut self, title: String, points: Vec<(f64, f64)>) {
        self.title = title;
        self.points = points;
        self.key = None;
        self.hovered = None;
    }

    /// Свои цвета для настроек.
    pub fn colors(&self) -> Vec<crate::settings::SectionColor> {
        let mut out: Vec<_> = self.own.iter().map(|(key, c)| (key.clone(), c.fill, c.line)).collect();
        out.sort();
        out
    }

    /// Шаг кадра: разрез заново, если сменились слои, полоса, модель или
    /// пришли высоты. generation - номер модели, version - версия высот.
    pub fn step(&mut self, model: Option<&Model>, generation: u64, ground: &Ground<'_>, version: u64, now: f64) {
        let Some(model) = model else {
            self.built = None;
            self.key = None;
            return;
        };
        self.roofs = model.horizons.iter().map(|h| h.code.clone()).collect();
        self.has_holes = !model.holes.is_empty();
        self.zlayers.clear();
        for shape in &model.zshapes {
            if !self.zlayers.contains(&shape.layer) {
                self.zlayers.push(shape.layer.clone());
            }
        }
        let key = (self.band.to_bits(), self.off.clone(), self.holes_on, generation);
        let newer = version != self.built_version && now - self.built_at > subsurface::REBUILD_GAP;
        if self.key.as_ref() == Some(&key) && !newer {
            return;
        }
        self.built = build(model, ground, &self.points, self.band, &self.off, self.holes_on);
        self.key = Some(key);
        self.built_at = now;
        self.built_version = version;
    }

    fn file_name(&self, ext: &str) -> String {
        let name: String = self.title.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
        format!("{}.{ext}", if name.is_empty() { "section" } else { &name })
    }

    /// Итог записи файла в строку окна.
    fn saved(&mut self, lang: Lang, result: Result<Option<String>, String>) {
        match result {
            Ok(Some(to)) => self.status = trf(lang, "Saved: {to}", &[("to", &to)]),
            Ok(None) => {}
            Err(e) => self.status = trf(lang, "Not saved: {e}", &[("e", &e)]),
        }
    }

    /// Готовая картинка графика - файлом PNG.
    fn write_shot(&mut self, lang: Lang) {
        let got = self.shot.as_ref().and_then(|cell| cell.lock().ok().and_then(|mut slot| slot.take()));
        let Some(result) = got else {
            return;
        };
        self.shot = None;
        let bytes = result.and_then(|image| {
            let (w, h) = (image.size[0] as u32, image.size[1] as u32);
            let raw: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            let picture = image::RgbaImage::from_raw(w, h, raw).ok_or("image")?;
            let mut out = std::io::Cursor::new(Vec::new());
            picture.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
            Ok(out.into_inner())
        });
        let name = self.file_name("png");
        let saved = bytes.and_then(|b| crate::export::save_bytes(&name, ("PNG", "png"), &b));
        self.saved(lang, saved);
    }

    fn fill_of(&self, s: &ModelSection, k: usize) -> [u8; 3] {
        let key = s.owners.get(k).or_else(|| s.beds.get(k).map(|b| &b.0));
        key.and_then(|key| self.own.get(key)).and_then(|o| o.fill).or_else(|| s.beds.get(k).map(|b| b.1)).unwrap_or(HOST_COLOR)
    }

    fn line_of(&self, s: &ModelSection, code: &str) -> [u8; 3] {
        self.own.get(code).and_then(|o| o.line).or_else(|| s.lines.iter().find(|l| l.0 == code).map(|l| l.1)).unwrap_or([40, 40, 40])
    }

    /// Окно. false в open - закрыли.
    pub fn window(&mut self, ctx: &egui::Context, lang: Lang, open: &mut bool) {
        self.write_shot(lang);
        if self.shot.is_some() {
            ctx.request_repaint();
        }
        let title = if self.title.is_empty() { tr(lang, "Model section").to_string() } else { trf(lang, "Model section: {name}", &[("name", &self.title)]) };
        self.hovered = None;
        egui::Window::new(title).id(egui::Id::unique("planetx-model-section")).open(open).collapsible(false).default_size([880.0, 460.0]).resizable(true).min_height(320.0).show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| self.list(ui, lang));
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(tr(lang, "Band of drill holes and objects"));
                        ui.add(egui::DragValue::new(&mut self.band).range(10.0..=5000.0).speed(10.0).fixed_decimals(0).suffix(tr(lang, " m")))
                            .on_hover_text(tr(lang, "Width of the band along the line from which drill holes and tunnels are moved onto the section. A wider band gives more drill holes, but distant ones do not lie where the line is."));
                    });
                    let hovered = self.chart(ui, lang);
                    if let (Some(i), Some(s)) = (hovered, &self.built) {
                        self.hovered = Some((s.lats[i], s.lons[i], s.distance[i]));
                    }
                });
            });
            self.foot(ui, ctx, lang);
        });
    }

    /// Список слоёв разреза с флажками слева.
    fn list(&mut self, ui: &mut egui::Ui, lang: Lang) {
        ui.set_width(LIST_WIDTH);
        ui.set_min_height((ui.available_height() - FOOT).max(120.0));
        let roofs = self.roofs.clone();
        egui::ScrollArea::vertical().id_salt("model-section-layers").auto_shrink([false, true]).show(ui, |ui| {
            if !roofs.is_empty() {
                ui.strong(tr(lang, "Roofs"));
                for code in &roofs {
                    let mut on = !self.off.contains(code);
                    if ui.checkbox(&mut on, code.as_str()).changed() {
                        if on {
                            self.off.remove(code);
                        } else {
                            self.off.insert(code.clone());
                        }
                    }
                }
            }
            if self.has_holes {
                ui.strong(tr(lang, "Drill holes"));
                ui.checkbox(&mut self.holes_on, tr(lang, "Drill holes of the model"));
            }
            if !self.zlayers.is_empty() {
                ui.strong(tr(lang, "Layers with Z"));
                for layer in self.zlayers.clone() {
                    let key = z_key(&layer);
                    let mut on = !self.off.contains(&key);
                    if ui.checkbox(&mut on, layer).changed() {
                        if on {
                            self.off.remove(&key);
                        } else {
                            self.off.insert(key);
                        }
                    }
                }
            }
            if roofs.is_empty() && !self.has_holes && self.zlayers.is_empty() {
                ui.label(tr(lang, "The model has no roofs and drill holes."));
            }
        });
    }

    /// Итог и кнопки под графиком.
    fn foot(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, lang: Lang) {
        ui.horizontal(|ui| {
            let text = match &self.built {
                None => tr(lang, "A path of at least two points and an open subsurface model are needed.").to_string(),
                Some(s) => trf(
                    lang,
                    "Length {length}. Roofs {beds}, drill holes in the band {holes}, tunnels {tunnels}.",
                    &[
                        ("length", &crate::profile::distance_text(lang, s.distance.last().copied().unwrap_or(0.0))),
                        ("beds", &s.beds.len().saturating_sub(1).to_string()),
                        ("holes", &s.holes.len().to_string()),
                        ("tunnels", &s.tunnels.len().to_string()),
                    ],
                ),
            };
            let fences = self.built.as_ref().map_or(0, |s| s.fences.len());
            let text = if fences > 0 { format!("{text} {}", trf(lang, "Parts of objects with Z {count}.", &[("count", &fences.to_string())])) } else { text };
            ui.label(if self.status.is_empty() { text } else { format!("{text} {}", self.status) });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let table = ui.add_enabled(self.built.is_some(), egui::Button::new(tr(lang, "CSV table…"))).on_hover_text(tr(lang, "Heights by the points of the line - distance, latitude, longitude, terrain and bed roofs, no roof - an empty field."));
                let picture = ui.add_enabled(self.built.is_some(), egui::Button::new(tr(lang, "Picture…"))).on_hover_text(tr(lang, "The section as a PNG picture of the window size."));
                if table.clicked()
                    && let Some(s) = &self.built
                {
                    let text = table_text(s);
                    let name = self.file_name("csv");
                    let saved = crate::export::save_bytes(&name, ("CSV", "csv"), text.as_bytes());
                    self.saved(lang, saved);
                }
                if picture.clicked() {
                    let cell: Shot = Arc::new(Mutex::new(None));
                    let (sink, rect, ppp) = (cell.clone(), self.plot_rect, ctx.pixels_per_point());
                    ctx.request_screenshot(move |image| {
                        if let Ok(mut slot) = sink.lock() {
                            *slot = Some(crate::app::crop(&image, rect, ppp));
                        }
                    });
                    self.shot = Some(cell);
                }
            });
        });
    }

    /// Промежуток и ближайшая линия под точкой графика, `_at` модуля.
    fn at(s: &ModelSection, plot: egui::Rect, range: (f64, f64), pos: egui::Pos2) -> (Option<usize>, Option<String>) {
        let share = f64::from((pos.x - plot.left()) / plot.width());
        let x1 = s.distance.last().copied().unwrap_or(1.0).max(1.0);
        if !(0.0..=1.0).contains(&share) || s.distance.is_empty() {
            return (None, None);
        }
        let i = s.distance.partition_point(|d| *d < share * x1).min(s.distance.len() - 1);
        let (high, low) = range;
        let z = high - f64::from((pos.y - plot.top()) / plot.height()) * (high - low);
        let found = s.beds.iter().rposition(|(_, _, top, bottom)| top[i].is_finite() && bottom[i] <= z && z <= top[i] && top[i] > bottom[i]);
        let mut best = LINE_PICK / f64::from(plot.height()) * (high - low);
        let mut near = None;
        for (code, _, zs) in &s.lines {
            if zs[i].is_finite() && (zs[i] - z).abs() <= best {
                best = (zs[i] - z).abs();
                near = Some(code.clone());
            }
        }
        (found, near)
    }

    /// Правая кнопка: цвет пласта, породы или линии, сброс своих цветов,
    /// `contextMenuEvent` модуля.
    fn color_menu(&mut self, ui: &mut egui::Ui, lang: Lang, s: &ModelSection) {
        let (k, line) = self.menu.clone();
        if let Some(k) = k {
            let key = s.owners.get(k).cloned().unwrap_or_default();
            let text = if key == core::HOST {
                tr(lang, "Color of the rock between beds").to_string()
            } else {
                let (name, _) = core::bed_key(&key);
                trf(lang, "Color of the bed «{name}»", &[("name", if name.is_empty() { &key } else { &name })])
            };
            let mut color = self.fill_of(s, k);
            ui.horizontal(|ui| {
                if ui.color_edit_button_srgb(&mut color).changed() {
                    self.own.entry(key).or_default().fill = Some(color);
                    self.colors_changed = true;
                }
                ui.label(text);
            });
        }
        if let Some(line) = line {
            let mut color = self.line_of(s, &line);
            ui.horizontal(|ui| {
                if ui.color_edit_button_srgb(&mut color).changed() {
                    self.own.entry(line.clone()).or_default().line = Some(color);
                    self.colors_changed = true;
                }
                ui.label(trf(lang, "Color of the line «{name}»", &[("name", &line)]));
            });
        }
        if !self.own.is_empty() {
            ui.separator();
            if ui.button(tr(lang, "Default section colors")).clicked() {
                self.own.clear();
                self.colors_changed = true;
                ui.close();
            }
        }
    }

    /// График разреза, `paintEvent` модуля. Возвращает номер точки линии
    /// под курсором.
    fn chart(&mut self, ui: &mut egui::Ui, lang: Lang) -> Option<usize> {
        let size = egui::vec2(ui.available_width().max(320.0), (ui.available_height() - FOOT).max(200.0));
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        self.plot_rect = rect;
        let painter = ui.painter_at(rect);
        let Some(s) = self.built.take() else {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, tr(lang, "No data"), egui::FontId::proportional(13.0), ui.visuals().text_color());
            return None;
        };
        let hovered = self.paint(ui, lang, &painter, rect, &response, &s);
        self.built = Some(s);
        hovered
    }

    fn paint(&mut self, ui: &egui::Ui, lang: Lang, painter: &egui::Painter, rect: egui::Rect, response: &egui::Response, s: &ModelSection) -> Option<usize> {
        let text_color = ui.visuals().text_color();
        let font = egui::FontId::proportional(11.0);
        let plot = egui::Rect::from_min_max(rect.min + egui::vec2(60.0, 12.0), rect.max - egui::vec2(14.0, 30.0));
        let x1 = s.distance.last().copied().unwrap_or(1.0).max(1.0);
        let top = s.ground.iter().copied().filter(|v| v.is_finite()).fold(f64::NEG_INFINITY, f64::max);
        let (top, low) = if top.is_finite() { (top, s.bottom.min(top - 1.0)) } else { (s.bottom + 1.0, s.bottom) };
        // Объекты с Z тоже входят в график.
        let fence_z = || s.fences.iter().flat_map(|f| f.2.iter().copied()).filter(|z| z.is_finite());
        let top = fence_z().fold(top, f64::max);
        let low = fence_z().fold(low, f64::min);
        let high = top + (top - low) * HEAD;
        let range = (high, low);
        // Отметка вверх: заливка общая с окном «Разрез» идёт по глубине -z.
        let to_screen = |d: f64, depth: f64| {
            let z = (-depth).clamp(low, high);
            egui::pos2(plot.left() + (d / x1) as f32 * plot.width(), plot.top() + ((high - z) / (high - low)) as f32 * plot.height())
        };
        let negative = |v: &[f64]| -> Vec<f64> { v.iter().map(|z| -z).collect() };
        for (k, (_, _, upper, lower)) in s.beds.iter().enumerate() {
            crate::section::fill(painter, &to_screen, &s.distance, &negative(upper), &negative(lower), self.fill_of(s, k));
        }
        let path = |d: &[f64], z: &[f64]| -> Vec<egui::Pos2> { d.iter().zip(z).map(|(d, z)| to_screen(*d, -*z)).collect() };
        // Стенки и линии с отметками - цветом объекта.
        for (_, d, z, color, polygon) in &s.fences {
            let points = path(d, z);
            if *polygon {
                let flat: Vec<(f64, f64)> = points.iter().map(|p| (f64::from(p.x), f64::from(p.y))).collect();
                let mut mesh = egui::Mesh::default();
                for p in &points {
                    mesh.colored_vertex(*p, rgb(*color));
                }
                mesh.indices = scene::features::triangulate(&flat);
                painter.add(egui::Shape::mesh(mesh));
                painter.add(egui::Shape::closed_line(points, egui::Stroke::new(0.8, rgb(core::line_color(*color)))));
            } else {
                painter.add(egui::Shape::line(points, egui::Stroke::new(2.0, rgb(*color))));
            }
        }
        // Кровли и подошвы - линиями своего цвета.
        for (code, _, z) in &s.lines {
            let good: Vec<bool> = z.iter().map(|v| v.is_finite()).collect();
            let stroke = egui::Stroke::new(LINE_WIDTH, rgb(self.line_of(s, code)));
            for (a, b) in runs(&good).into_iter().filter(|(a, b)| b - a >= 2) {
                painter.add(egui::Shape::line(path(&s.distance[a..b], &z[a..b]), stroke));
            }
        }
        // Рельеф линией поверх пластов.
        painter.add(egui::Shape::line(path(&s.distance, &s.ground), egui::Stroke::new(1.5, egui::Color32::from_rgb(60, 40, 20))));
        // Стволы - куски цвета интервала с тёмной обводкой.
        for (_, pieces) in &s.holes {
            for (d, z, color) in pieces.iter().filter(|p| p.0.len() >= 2) {
                painter.add(egui::Shape::line(path(d, z), egui::Stroke::new(5.0, egui::Color32::from_gray(30))));
                painter.add(egui::Shape::line(path(d, z), egui::Stroke::new(3.0, rgb(*color))));
            }
        }
        // Тоннель - своим диаметром в масштабе графика, не тоньше 4 px.
        let per_metre = f64::from(plot.height()) / (high - low).max(1e-9);
        for (name, d, z, color) in &s.tunnels {
            let width = (s.diameters.get(name).copied().unwrap_or(0.0) * per_metre).max(4.0) as f32;
            painter.add(egui::Shape::line(path(d, z), egui::Stroke::new(width, rgb(*color))));
        }
        for (name, pieces) in &s.holes {
            if let Some((Some(d), Some(z))) = pieces.first().map(|p| (p.0.first(), p.1.first())) {
                painter.text(to_screen(*d, -*z) - egui::vec2(0.0, 4.0), egui::Align2::CENTER_BOTTOM, name, font.clone(), text_color);
            }
        }
        // Сетка и подписи осей.
        let grid = egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(128, 128, 128, 90));
        let step = crate::profile::nice_step(high - low, 6.0);
        let mut z = (low / step).ceil() * step;
        while z <= high + 1e-9 {
            let y = to_screen(0.0, -z).y;
            painter.line_segment([egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)], grid);
            painter.text(egui::pos2(plot.left() - 6.0, y), egui::Align2::RIGHT_CENTER, metres_text(lang, z), font.clone(), text_color);
            z += step;
        }
        let step = crate::profile::nice_step(x1, 6.0);
        let mut d = 0.0;
        while d <= x1 + 1e-9 {
            let x = to_screen(d, 0.0).x;
            painter.line_segment([egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())], grid);
            painter.text(egui::pos2(x, plot.bottom() + 4.0), egui::Align2::CENTER_TOP, crate::profile::axis_text(lang, d, step), font.clone(), text_color);
            d += step;
        }
        self.legend(ui, lang, painter, plot, s);
        if response.secondary_clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            self.menu = Self::at(s, plot, range, pos);
        }
        response.context_menu(|ui| self.color_menu(ui, lang, s));
        let pos = response.hover_pos().filter(|p| plot.x_range().contains(p.x))?;
        let target = f64::from((pos.x - plot.left()) / plot.width()) * x1;
        let i = s.distance.partition_point(|d| *d < target).min(s.distance.len().checked_sub(1)?);
        let x = to_screen(s.distance[i], 0.0).x;
        painter.line_segment([egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())], egui::Stroke::new(1.2, HOVER));
        let mut parts = vec![crate::profile::distance_text(lang, s.distance[i]), trf(lang, "terrain {value}", &[("value", &metres_text(lang, s.ground[i]))])];
        for (code, _, upper, lower) in s.beds.get(1..).unwrap_or(&[]).iter().filter(|b| !b.0.is_empty() && b.2[i].is_finite() && b.2[i] > b.3[i]).take(HOVER_BEDS) {
            let _ = lower;
            parts.push(format!("{code} {}", metres_text(lang, upper[i])));
        }
        painter.text(plot.left_top() + egui::vec2(6.0, 2.0), egui::Align2::LEFT_TOP, parts.join(", "), font, text_color);
        Some(i)
    }

    /// Подписи цветов в правом верхнем углу графика, `_legend` модуля.
    fn legend(&self, ui: &egui::Ui, lang: Lang, painter: &egui::Painter, plot: egui::Rect, s: &ModelSection) {
        let mut rows: Vec<(String, [u8; 3])> = Vec::new();
        for (k, (code, _, upper, lower)) in s.beds.iter().enumerate() {
            let name = if s.owners.get(k).is_some_and(|o| o == core::HOST) {
                tr(lang, "Between beds").to_string()
            } else if code.is_empty() {
                continue;
            } else {
                let (name, _) = core::bed_key(code);
                if name.is_empty() { code.clone() } else { name }
            };
            if !rows.iter().any(|r| r.0 == name) && upper.iter().zip(lower).any(|(a, b)| a > b) {
                rows.push((name, self.fill_of(s, k)));
            }
        }
        for (name, _, _, color, _) in &s.fences {
            if !rows.iter().any(|r| r.0 == *name) {
                rows.push((name.clone(), *color));
            }
        }
        rows.truncate(LEGEND_ROWS);
        if rows.is_empty() {
            return;
        }
        let font = egui::FontId::proportional(11.0);
        let dark = egui::Color32::from_gray(30);
        let galleys: Vec<_> = rows.iter().map(|(name, _)| ui.fonts_mut(|f| f.layout_no_wrap(name.clone(), font.clone(), dark))).collect();
        let width = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max) + 32.0;
        let line = 15.0;
        let area = egui::Rect::from_min_size(egui::pos2(plot.right() - width - 6.0, plot.top() + 22.0), egui::vec2(width, line * rows.len() as f32 + 6.0));
        painter.rect_filled(area, 2.0, egui::Color32::from_white_alpha(200));
        for (k, ((_, color), galley)) in rows.iter().zip(galleys).enumerate() {
            let y = area.top() + 3.0 + k as f32 * line;
            painter.rect_filled(egui::Rect::from_min_size(egui::pos2(area.left() + 5.0, y + 2.0), egui::vec2(16.0, line - 5.0)), 0.0, rgb(*color));
            painter.galley(egui::pos2(area.left() + 26.0, y), galley, dark);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::BTreeSet;

    use super::{build, runs, table_text};
    use crate::subsurface::{Model, Tunnel, ZShape};

    fn tunnel(name: &str, lat: f64) -> Tunnel {
        Tunnel { name: name.into(), points: vec![[lat, 56.0], [lat, 56.01]], depths: (30.0, 50.0), diameter: 4.0, color: [10, 20, 30] }
    }

    #[test]
    fn runs_of_true() {
        assert_eq!(runs(&[true, true, false, false, true]), [(0, 2), (4, 5)]);
        assert!(runs(&[false]).is_empty());
    }

    /// Модель из одних тоннелей: низ под самым глубоким, тоннель вне полосы
    /// на разрез не попадает.
    #[test]
    fn tunnels_in_the_band_and_bottom_under_them() {
        let model = Model { tunnels: vec![tunnel("Рядом", 58.0), tunnel("Далеко", 58.1)], ..Model::default() };
        let ground = |points: &[(f64, f64)]| vec![100.0; points.len()];
        let s = build(&model, &ground, &[(58.0, 56.0), (58.0, 56.01)], 200.0, &BTreeSet::new(), true).unwrap();
        assert_eq!(s.distance.len(), super::SAMPLES);
        assert!((s.distance.last().unwrap() - 590.0).abs() < 10.0, "{:?}", s.distance.last());
        assert_eq!(s.tunnels.len(), 1);
        assert_eq!(s.tunnels[0].0, "Рядом");
        // Глубина до 50 м, диаметр 4 м, запас 20 м.
        assert!((s.bottom - (100.0 - 50.0 - 4.0 - 20.0)).abs() < 1.0, "{}", s.bottom);
        assert_eq!(s.diameters["Рядом"], 4.0);
        // Без кровель - один промежуток от рельефа до низа.
        assert_eq!(s.beds.len(), 1);
        assert_eq!(s.beds[0].2[0], 100.0);
        assert!(s.lines.is_empty());
        // Путь из одной точки разреза не даёт.
        assert!(build(&model, &ground, &[(58.0, 56.0)], 200.0, &BTreeSet::new(), true).is_none());
    }

    /// Часть объекта с Z в полосе ложится на разрез и в график, дальняя
    /// и выключенная - нет.
    #[test]
    fn z_layers_in_the_band() {
        let part = |lat: f64| vec![[lat, 56.002, -300.0], [lat, 56.004, -320.0]];
        let shape = |layer: &str, lat: f64| ZShape { layer: layer.into(), color: [1, 2, 3], polygon: false, parts: vec![part(lat)] };
        let model = Model { tunnels: vec![tunnel("Т", 58.0)], zshapes: vec![shape("Рядом", 58.0), shape("Далеко", 58.1)], ..Model::default() };
        let ground = |points: &[(f64, f64)]| vec![100.0; points.len()];
        let line = [(58.0, 56.0), (58.0, 56.01)];
        let s = build(&model, &ground, &line, 200.0, &BTreeSet::new(), true).unwrap();
        assert_eq!(s.fences.len(), 1);
        assert_eq!(s.fences[0].0, "Рядом");
        assert_eq!(s.fences[0].2, [-300.0, -320.0]);
        let off = BTreeSet::from([super::z_key("Рядом")]);
        assert!(build(&model, &ground, &line, 200.0, &off, true).unwrap().fences.is_empty());
    }

    #[test]
    fn table_has_a_row_per_point() {
        let model = Model { tunnels: vec![tunnel("Т", 58.0)], ..Model::default() };
        let ground = |points: &[(f64, f64)]| vec![100.0; points.len()];
        let s = build(&model, &ground, &[(58.0, 56.0), (58.0, 56.01)], 200.0, &BTreeSet::new(), true).unwrap();
        let text = table_text(&s);
        assert!(text.starts_with("distance_m,lat,lon,ground_m\n0.0,58.0000000,56.0000000,100.00\n"), "{}", &text[..80]);
        assert_eq!(text.lines().count(), super::SAMPLES + 1);
        assert_eq!(super::field("a,\"b"), "\"a,\"\"b\"");
    }
}
