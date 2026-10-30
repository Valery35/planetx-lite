// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Общий каркас окон анализа участка, перенос `ui/area.py`
//! и `ui/results.py` модуля.
//!
//! Решения автора от 28 октября 2026 года, единые для всех окон
//! подменю «Анализ участка»:
//! - участок - многоугольник ([`SiteEdit`]). При открытии окна это
//!   квадрат с серединой в точке окна, вершины тянутся мышью, кружок на
//!   середине стороны ставит новую вершину. «Квадрат» возвращает
//!   квадрат, «Нарисовать» - вершины щелчками, щелчок по первой
//!   вершине замыкает участок;
//! - «Построить» показывает результат на глобусе, «OK» сохраняет его
//!   и закрывает окно, «Отмена» и крестик убирают результат
//!   ([`buttons`]);
//! - каждый сохранённый расчёт - папка «Инструмент ЧЧ:ММ:СС UTC» вверху
//!   папки «Анализ участка» «Моих меток», в ней контур участка и картинки
//!   слоёв расчёта ([`Saved::folder`]). Удаление папки убирает расчёт
//!   целиком.
//!
//! Участок общий, слово автора от 30 октября 2026 года: нарисовал один
//! раз и считаешь по нему разными окнами. Его хранит окно глобуса
//! ([`share_sites`], [`SiteEdit::open_at`]), смена тела его снимает.
//!
//! Проекта QGIS у Lite нет: слой проекта заменяет картинка на
//! поверхности в «Моих метках», она хранится в их файле KMZ. Значения
//! слоя пишутся рядом файлом GeoTIFF EPSG:4326 (`data::geotiff`) -
//! в программе в папку `results` папки данных, на странице
//! скачиванием. Файл открывают QGIS и модуль.

use eframe::egui;
use globe_core::DVec3;
use globe_core::navigation::{Navigator, ground_under};
use scene::editing::{self, Click, Drawing, Picked, Pixel};
use scene::kml::{Folder, Item, Overlay, Place};
use scene::{features, geodesic, measure, overlays};

use data::geotiff::LatLonGrid;
use data::site::{Problem, Site};

use crate::i18n::{Lang, tr, trf};
use crate::tool::{ARC_POINTS, CLICK_PIXELS, GRAB_PIXELS};

/// Контур участка на глобусе и в «Моих метках», `COLOR` модуля.
pub const SITE_COLOR: [u8; 4] = [255, 255, 255, 255];
/// Папка файлов GeoTIFF в папке данных, `FOLDER` модуля.
pub const FOLDER: &str = "results";

/// Участок окна на глобусе: вершины и их перетаскивание.
pub struct SiteEdit {
    pub site: Site,
    /// Перетаскиваемая вершина.
    drag: Option<usize>,
    /// Вершина, на которой нажата кнопка, и сдвинулась ли мышь.
    pressed: Option<(usize, bool)>,
    /// Точка нажатия, логические пиксели окна.
    press_at: Option<egui::Pos2>,
    /// Участок изменился: результат построен по прежнему.
    pub changed: bool,
    /// Участок изменён в этом окне: его ещё не взяли другие окна.
    edited: bool,
    /// Участок взят общий: строка итога ещё не сказала об этом.
    shared: bool,
}

impl SiteEdit {
    /// Квадрат side_km у точки, наибольший размер most_km.
    pub fn new(lat: f64, lon: f64, side_km: f64, most_km: f64) -> Self {
        Self { site: Site::new(lat, lon, side_km, most_km), drag: None, pressed: None, press_at: None, changed: false, edited: true, shared: false }
    }

    /// Новая точка окна: квадрат у неё.
    pub fn move_to(&mut self, lat: f64, lon: f64) {
        self.site.move_to(lat, lon);
        self.touch();
    }

    /// Окно открыто у точки, `open_at` модуля: точка внутри общего
    /// участка shared берёт его, иначе остаётся квадрат у точки.
    pub fn open_at(&mut self, shared: Option<&[(f64, f64)]>, lat: f64, lon: f64) {
        if let Some(ring) = shared
            && data::site::inside(&[(lat, lon)], ring).first() == Some(&true)
        {
            self.adopt(ring);
            self.shared = true;
        }
    }

    /// Участок изменён пользователем.
    fn touch(&mut self) {
        self.changed = true;
        self.edited = true;
    }

    /// Участок - общий ring.
    fn adopt(&mut self, ring: &[(f64, f64)]) {
        self.site.ring = ring.to_vec();
        self.site.drawing = false;
        self.changed = true;
        self.edited = false;
    }

    /// Строка итога окна, взявшего общий участок, один раз.
    pub fn shared_note(&mut self, lang: Lang) -> Option<&'static str> {
        std::mem::take(&mut self.shared).then(|| tr(lang, "The area is shared with the previous computation. «Square» and «Draw» start a new one."))
    }

    /// Строка «Участок» окна: сторона квадрата, «Квадрат», «Нарисовать».
    pub fn row(&mut self, ui: &mut egui::Ui, lang: Lang) {
        ui.horizontal(|ui| {
            let most = self.site.most_km.max(0.1);
            let tip = trf(
                lang,
                "Side of the square of the area with its middle at the point of the window. Vertices of the area are dragged with the mouse, the circle in the middle of a side adds a new vertex. The area is not larger than {km} km.",
                &[("km", &format!("{most}"))],
            );
            if ui.add(egui::DragValue::new(&mut self.site.side_km).range(0.1..=most).speed(0.1).fixed_decimals(1).suffix(tr(lang, " km"))).on_hover_text(tip).changed() {
                self.site.reset_square();
                self.touch();
            }
            if ui.button(tr(lang, "Square")).on_hover_text(tr(lang, "The area is a square with the side from the field on the left.")).clicked() {
                self.site.reset_square();
                self.touch();
            }
            if ui
                .button(tr(lang, "Draw"))
                .on_hover_text(tr(lang, "Draw the area as a polygon by clicks on the globe. A click on the first vertex closes it. The result is computed only inside the area."))
                .clicked()
            {
                self.site.start_drawing();
                self.touch();
            }
        });
    }

    /// Подсказка, пока участок рисуется.
    pub fn hint(&self, lang: Lang) -> Option<&'static str> {
        self.site.drawing.then(|| tr(lang, "Click the vertices of the area on the globe. A click on the first vertex closes it."))
    }

    /// Почему по участку считать нельзя, текстом, или None.
    pub fn problem(&self, lang: Lang) -> Option<String> {
        match self.site.check()? {
            Problem::Open => Some(tr(lang, "Close the area by a click on its first vertex.").to_string()),
            Problem::TooLarge(km) => Some(trf(lang, "The area is larger than {km} km, make it smaller.", &[("km", &format!("{km}"))])),
        }
    }

    /// Мышь над глобусом. true - мышь занята участком, навигации она
    /// не достаётся.
    pub fn input(&mut self, ctx: &egui::Context, response: &egui::Response, rect: egui::Rect, ppp: f64, nav: &Navigator) -> bool {
        let to_px = |p: egui::Pos2| (f64::from(p.x - rect.min.x) * ppp, f64::from(p.y - rect.min.y) * ppp);
        let ground = |p: egui::Pos2| {
            let (px, py) = to_px(p);
            let point = ground_under(&nav.camera, px, py, nav.pose.terrain.as_ref())?;
            let (lat, lon, _) = nav.camera.ellipsoid.ecef_to_geodetic(point);
            Some((lat, lon))
        };
        let (pressed, down, released, pos) = ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.primary_released(), i.pointer.interact_pos()));
        // Щелчок по окну или меню над глобусом участку не достаётся.
        let on_globe = pos.is_some_and(|p| rect.contains(p) && ctx.layer_id_at(p).is_none_or(|l| l == response.layer_id));
        if pressed && on_globe && let Some(p) = pos {
            let (px, py) = to_px(p);
            let (vertices, middles) = self.handles(nav);
            self.press_at = Some(p);
            match editing::pick(&vertices, &middles, px, py, GRAB_PIXELS * ppp) {
                Some(Picked::Vertex(i)) => {
                    self.drag = Some(i);
                    self.pressed = Some((i, false));
                }
                Some(Picked::Middle(i)) => {
                    if let Some(point) = ground(p) {
                        let at = (i + 1).min(self.site.ring.len());
                        self.site.ring.insert(at, point);
                        self.drag = Some(at);
                        self.pressed = Some((at, true));
                        self.touch();
                    }
                }
                None => {}
            }
        }
        if let Some(index) = self.drag {
            if down {
                let far = pos.zip(self.press_at).is_some_and(|(p, at)| p.distance(at) > CLICK_PIXELS);
                let moved = self.pressed.is_some_and(|(_, m)| m) || far;
                if moved
                    && let Some(point) = pos.and_then(ground)
                    && let Some(slot) = self.site.ring.get_mut(index)
                {
                    *slot = point;
                    self.touch();
                    if let Some((_, m)) = &mut self.pressed {
                        *m = true;
                    }
                }
                return true;
            }
            self.drag = None;
            if let Some((index, false)) = self.pressed.take()
                && editing::click_result(Drawing::Polygon, self.site.ring.len(), index, !self.site.drawing) == Some(Click::Close)
            {
                self.site.drawing = false;
                self.touch();
            }
            return true;
        }
        if released {
            self.pressed = None;
        }
        if self.site.drawing && response.clicked() && on_globe && let Some(point) = pos.and_then(ground) {
            self.site.ring.push(point);
            self.touch();
        }
        false
    }

    /// Точка (широта, долгота) на рельефе в ECEF.
    fn ecef(nav: &Navigator, (lat, lon): (f64, f64)) -> DVec3 {
        let h = nav.pose.terrain.as_ref().map_or(0.0, |t| t(lat, lon));
        nav.camera.ellipsoid.geodetic_to_ecef(lat, lon, h)
    }

    /// Точка экрана, кадровые пиксели, и видна ли.
    fn pixel(nav: &Navigator, p: (f64, f64)) -> Pixel {
        let point = Self::ecef(nav, p);
        let (px, front) = nav.camera.project(point);
        let facing = (nav.camera.eye - point).dot(point.normalize_or_zero()) > 0.0;
        ([px.x, px.y], front && facing)
    }

    /// Вершины и середины сторон в пикселях кадра. Середины - у
    /// замкнутого участка.
    fn handles(&self, nav: &Navigator) -> (Vec<Pixel>, Vec<Pixel>) {
        let vertices = self.site.ring.iter().map(|p| Self::pixel(nav, *p)).collect();
        let middles = if self.site.closed() { measure::segment_midpoints(&self.site.ring, true).into_iter().map(|p| Self::pixel(nav, p)).collect() } else { Vec::new() };
        (vertices, middles)
    }

    /// Контур участка, вершины и середины сторон поверх глобуса.
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect, ppp: f64, nav: &Navigator) {
        let to_screen = |p: [f64; 2]| rect.min + egui::vec2((p[0] / ppp) as f32, (p[1] / ppp) as f32);
        let [r, g, b, a] = SITE_COLOR;
        let stroke = egui::Stroke::new(2.0, egui::Color32::from_rgba_unmultiplied(r, g, b, a));
        let ring = &self.site.ring;
        let mut pairs: Vec<((f64, f64), (f64, f64))> = ring.windows(2).map(|w| (w[0], w[1])).collect();
        if self.site.closed()
            && let (Some(last), Some(first)) = (ring.last(), ring.first())
        {
            pairs.push((*last, *first));
        }
        for (a, b) in pairs {
            let arc = features::densify(&[a, b], false, geodesic::length(&[a, b]) / ARC_POINTS as f64 + 1e-6, ARC_POINTS + 1);
            let pixels: Vec<Pixel> = arc.iter().map(|p| Self::pixel(nav, *p)).collect();
            for w in pixels.windows(2) {
                if w[0].1 && w[1].1 {
                    painter.line_segment([to_screen(w[0].0), to_screen(w[1].0)], stroke);
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
                let radius = if self.drag == Some(i) { 6.0 } else { 5.0 };
                painter.circle(to_screen(p), radius, egui::Color32::WHITE, egui::Stroke::new(1.5, egui::Color32::from_gray(30)));
            }
        }
    }
}

/// Изменённый замкнутый участок - общий, `_share` модуля: его хранит
/// shared и берут открытые окна edits, кроме рисующих.
pub fn share_sites(shared: &mut Option<Vec<(f64, f64)>>, mut edits: Vec<&mut SiteEdit>) {
    let Some(ring) = edits.iter_mut().find(|s| s.edited && s.site.closed()).map(|s| {
        s.edited = false;
        s.site.ring.clone()
    }) else {
        return;
    };
    for s in edits {
        if !s.site.drawing && s.site.ring != ring {
            s.adopt(&ring);
        }
    }
    *shared = Some(ring);
}

/// Кнопка окна анализа.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Build,
    Ok,
    Cancel,
}

/// Кнопки «Построить», «OK» и «Отмена», `keep_buttons` модуля. OK
/// доступна, когда есть что сохранить.
pub fn buttons(ui: &mut egui::Ui, lang: Lang, can_save: bool, build_tip: &str) -> Option<Button> {
    let mut out = None;
    ui.horizontal(|ui| {
        if ui.button(tr(lang, "Build")).on_hover_text(build_tip).clicked() {
            out = Some(Button::Build);
        }
        let ok_tip = tr(
            lang,
            "Save the result and close the window. The computation goes as a folder into the folder «Area analysis» of «My places» together with the outline of the area, the values - as a GeoTIFF file. Deleting the folder removes the computation entirely.",
        );
        if ui.add_enabled(can_save, egui::Button::new(tr(lang, "OK"))).on_hover_text(ok_tip).clicked() {
            out = Some(Button::Ok);
        }
        if ui.button(tr(lang, "Cancel")).on_hover_text(tr(lang, "Remove the result from the globe and close the window.")).clicked() {
            out = Some(Button::Cancel);
        }
    });
    out
}

/// Новый номер сессии окна анализа для [`Saved::session`].
pub fn new_session() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Метки из текста KML для папки расчёта: папка верхнего уровня -
/// одной строкой, у документа - его строки.
pub fn kml_items(text: &str) -> Vec<Item> {
    let Ok(root) = scene::kml::read_file(text.as_bytes(), "result.kml") else {
        return Vec::new();
    };
    if text.contains("<Document>") { root.children } else { vec![Item::Folder(root)] }
}

/// Время суток UTC «ЧЧ:ММ:СС» момента unix, секунды.
pub fn clock(unix: f64) -> String {
    let s = unix.rem_euclid(86400.0).floor() as u32;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// Отметка конца расчёта для строки окна, `built_text` модуля:
/// повторное «Построить» идёт мгновенно, без отметки не видно, что
/// расчёт прошёл.
pub fn built_text(lang: Lang, unix: f64) -> String {
    trf(lang, "Built at {time} UTC.", &[("time", &clock(unix))])
}

/// Сводка значений участка строкой окна и описания слоя, `stats_text`
/// модуля: title - величина, unit - единица. Пусто, если значений нет.
pub fn stats_text(lang: Lang, title: &str, values: &[f64], unit: &str) -> String {
    let Some(rep) = data::stats::describe(values) else {
        return String::new();
    };
    let show = data::stats::rounded;
    trf(
        lang,
        "{title} - mean {mean}{unit}, spread {std}{unit}, quantiles 10, 25, 50, 75 and 90 % - {p10}, {q1}, {median}, {q3}, {p90}{unit}.",
        &[("title", title), ("mean", &show(rep.mean)), ("std", &show(rep.std)), ("p10", &show(rep.p10)), ("q1", &show(rep.q1)), ("median", &show(rep.median)), ("q3", &show(rep.q3)), ("p90", &show(rep.p90)), ("unit", unit)],
    )
}

/// Слой результата: картинка на глобусе и значения для GeoTIFF.
/// rgba - цвета узлов сетки без премножения, строки с севера.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultLayer {
    pub name: String,
    pub grid: LatLonGrid,
    pub rgba: Vec<[u8; 4]>,
    /// Байты GeoTIFF или пусто - слой без файла.
    pub tiff: Vec<u8>,
    /// Начало имени файла: инструмент латиницей, как `stem` модуля.
    pub stem: String,
    /// Сводка значений для описания слоя или пусто.
    pub summary: String,
}

impl ResultLayer {
    /// Картинка PNG слоя или None - цветов не столько, сколько узлов.
    pub fn png(&self) -> Option<Vec<u8>> {
        let (w, h) = (u32::try_from(self.grid.cols).ok()?, u32::try_from(self.grid.rows).ok()?);
        let bytes: Vec<u8> = self.rgba.iter().flatten().copied().collect();
        let img = image::RgbaImage::from_raw(w, h, bytes)?;
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).ok()?;
        Some(out.into_inner())
    }

    /// Рамка картинки LatLonBox (север, юг, восток, запад, поворот):
    /// края крайних пикселей.
    pub fn frame(&self) -> [f64; 5] {
        let g = &self.grid;
        let north = g.north + g.dlat / 2.0;
        let west = g.west - g.dlon / 2.0;
        [north, north - g.dlat * g.rows as f64, west + g.dlon * g.cols as f64, west, 0.0]
    }
}

/// Цвета точек (широта, долгота) с премноженной альфой, как у картинок
/// тайлов слоя.
pub type Sampler<'a> = &'a dyn Fn(&[(f64, f64)]) -> Vec<[u8; 4]>;

/// Наибольшая сторона картинки слоя в «Моих метках», пиксели.
pub const PICTURE_SIDE: usize = 1024;

/// Слой картинкой по рамке (юг, север, запад, восток): узлы - с шагом
/// по широте и долготе, сторона не больше [`PICTURE_SIDE`] и не мельче
/// шага cell метров. sample - цвета точек (широта, долгота) с премноженной
/// альфой, как у картинок тайлов слоя. tiff - файл значений.
pub fn picture(name: String, stem: &str, bbox: (f64, f64, f64, f64), cell: f64, sample: Sampler, tiff: Vec<u8>) -> ResultLayer {
    let (south, north, west, east) = bbox;
    let height_m = (north - south) * data::site::METERS_PER_DEGREE;
    let width_m = (east - west) * data::site::METERS_PER_DEGREE * ((south + north) / 2.0).to_radians().cos();
    let cell = cell.max(1.0).max(height_m.max(width_m) / PICTURE_SIDE as f64);
    let rows = ((height_m / cell).ceil() as usize).clamp(1, PICTURE_SIDE);
    let cols = ((width_m / cell).ceil() as usize).clamp(1, PICTURE_SIDE);
    let (dlat, dlon) = ((north - south) / rows as f64, (east - west) / cols as f64);
    let grid = LatLonGrid { north: north - dlat / 2.0, west: west + dlon / 2.0, dlat, dlon, cols, rows };
    let points: Vec<(f64, f64)> = (0..rows).flat_map(|r| (0..cols).map(move |c| (grid.north - r as f64 * dlat, grid.west + c as f64 * dlon))).collect();
    let rgba = sample(&points).into_iter().map(unpremultiply).collect();
    ResultLayer { name, grid, rgba, tiff, stem: stem.to_string(), summary: String::new() }
}

/// Цвет без премножения альфы - для картинки PNG.
pub fn unpremultiply(p: [u8; 4]) -> [u8; 4] {
    let a = p[3];
    if a == 0 {
        return [0; 4];
    }
    let c = |v: u8| ((f64::from(v) * 255.0 / f64::from(a)).round().min(255.0)) as u8;
    [c(p[0]), c(p[1]), c(p[2]), a]
}

/// Сохранённый расчёт: инструмент, участок, слои и свои метки
/// (вершины, створы).
#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    pub title: String,
    pub ring: Vec<(f64, f64)>,
    pub layers: Vec<ResultLayer>,
    pub extra: Vec<Item>,
    /// Сессия окна, `Session` модуля: расчёты одной сессии ложатся в одну
    /// папку, пока она есть. None - каждый расчёт - своя папка.
    pub session: Option<u64>,
}

impl Saved {
    /// Папка расчёта «Инструмент ЧЧ:ММ:СС UTC»: контур участка, картинки
    /// слоёв, свои метки. files - куда записан GeoTIFF каждого слоя,
    /// пусто - не записан.
    pub fn folder(&self, lang: Lang, unix: f64, files: &[String]) -> Folder {
        let mut folder = Folder::new(&format!("{} {} UTC", self.title, clock(unix)));
        if self.ring.len() >= 3 {
            let mut outline = Place::new(tr(lang, "Area|site"), features::Kind::Polygon, self.ring.clone());
            outline.color = SITE_COLOR;
            outline.width = 2.0;
            outline.fill = Some([255, 255, 255, 0]);
            folder.children.push(Item::Place(outline));
        }
        for (i, layer) in self.layers.iter().enumerate() {
            let Some(png) = layer.png() else {
                continue;
            };
            let mut ground = overlays::Overlay::new(overlays::Kind::Ground);
            ground.set_box(layer.frame());
            let mut item = Overlay::new(&layer.name, ground);
            item.image = Some(png);
            let file = files.get(i).filter(|p| !p.is_empty()).map(|path| trf(lang, "Values - the GeoTIFF file {path}.", &[("path", path)]));
            item.description = [Some(layer.summary.clone()).filter(|s| !s.is_empty()), file].into_iter().flatten().collect::<Vec<_>>().join(" ");
            folder.children.push(Item::Overlay(item));
        }
        folder.children.extend(self.extra.iter().cloned());
        folder
    }
}

/// Имя файла GeoTIFF слоя: начало stem и время UTC момента unix,
/// `result_path` модуля.
pub fn file_name(stem: &str, unix: f64) -> String {
    let day = (unix / 86400.0).floor() as i64;
    let (y, m, d) = crate::pointcalc::civil_from_days(day);
    format!("{stem}_{y:04}{m:02}{d:02}_{}.tif", clock(unix).replace(':', ""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer() -> ResultLayer {
        let grid = LatLonGrid { north: 58.01, west: 56.24, dlat: 0.001, dlon: 0.002, cols: 3, rows: 2 };
        ResultLayer { name: "Тени".into(), grid, rgba: vec![[10, 20, 30, 255]; 6], tiff: Vec::new(), stem: "shadows".into(), summary: "Тени - сводка.".into() }
    }

    #[test]
    fn kml_items_keep_folder_or_document_rows() {
        let folder = "<?xml version=\"1.0\"?><kml xmlns=\"http://www.opengis.net/kml/2.2\"><Folder><name>Сеть</name><Placemark><name>a</name><Point><coordinates>56,58,0</coordinates></Point></Placemark></Folder></kml>";
        let items = kml_items(folder);
        assert!(matches!(items.as_slice(), [Item::Folder(f)] if f.name == "Сеть" && f.children.len() == 1));
        let doc = "<?xml version=\"1.0\"?><kml xmlns=\"http://www.opengis.net/kml/2.2\"><Document><Placemark><name>a</name><Point><coordinates>56,58,0</coordinates></Point></Placemark></Document></kml>";
        assert!(matches!(kml_items(doc).as_slice(), [Item::Place(p)] if p.name == "a"));
        assert!(new_session() != new_session());
    }

    #[test]
    fn picture_samples_its_box() {
        let red = |p: &[(f64, f64)]| p.iter().map(|(lat, _)| if *lat > 58.0 { [100, 0, 0, 128] } else { [0; 4] }).collect();
        let layer = picture("x".into(), "x", (57.99, 58.01, 56.24, 56.26), 10.0, &red, Vec::new());
        assert!(layer.grid.cols > 10 && layer.grid.rows > 10);
        assert_eq!(layer.rgba.len(), layer.grid.cols * layer.grid.rows);
        // Север - первая строка, цвет без премножения.
        assert_eq!(layer.rgba[0], [199, 0, 0, 128]);
        assert_eq!(layer.rgba.last().copied(), Some([0; 4]));
        let f = layer.frame();
        assert!((f[0] - 58.01).abs() < 1e-9 && (f[3] - 56.24).abs() < 1e-9);
    }

    #[test]
    fn clock_and_file_name() {
        assert_eq!(clock(20750.0 * 86400.0 + 3723.0), "01:02:03");
        assert_eq!(file_name("shadows", 20750.0 * 86400.0 + 3723.0), "shadows_20261024_010203.tif");
    }

    #[test]
    fn point_inside_takes_the_shared_area() {
        let ring = data::site::square_ring(58.0, 56.25, 4000.0);
        let mut a = SiteEdit::new(58.001, 56.251, 1.0, 20.0);
        a.open_at(Some(&ring), 58.001, 56.251);
        assert_eq!(a.site.ring, ring);
        assert!(a.changed);
        assert!(a.shared_note(Lang::RU).is_some_and(|t| t.starts_with("Участок общий")));
        assert!(a.shared_note(Lang::RU).is_none());
        // Снаружи - свой квадрат.
        let mut b = SiteEdit::new(60.0, 56.25, 1.0, 20.0);
        let square = b.site.ring.clone();
        b.open_at(Some(&ring), 60.0, 56.25);
        assert_eq!(b.site.ring, square);
        assert!(b.shared_note(Lang::RU).is_none());
    }

    #[test]
    fn edited_area_goes_to_open_windows() {
        let mut shared = None;
        let mut a = SiteEdit::new(58.0, 56.25, 1.0, 20.0);
        let mut b = SiteEdit::new(58.5, 56.25, 2.0, 20.0);
        let mut drawing = SiteEdit::new(59.0, 56.25, 2.0, 20.0);
        drawing.site.start_drawing();
        let ring = a.site.ring.clone();
        share_sites(&mut shared, vec![&mut a, &mut b, &mut drawing]);
        assert_eq!(shared.as_ref(), Some(&ring));
        assert_eq!(b.site.ring, ring);
        assert!(b.changed);
        assert!(drawing.site.ring.is_empty());
        // Окно, взявшее участок, не отдаёт его обратно: второй проход
        // ничего не меняет.
        b.changed = false;
        share_sites(&mut shared, vec![&mut a, &mut b, &mut drawing]);
        assert!(!b.changed);
        // Незамкнутый участок никому не идёт, замкнутый идёт всем.
        drawing.site.ring = vec![(59.0, 56.0), (59.0, 56.1)];
        drawing.touch();
        share_sites(&mut shared, vec![&mut a, &mut b, &mut drawing]);
        assert_eq!(a.site.ring, ring);
        drawing.site.ring.push((59.1, 56.05));
        drawing.site.drawing = false;
        share_sites(&mut shared, vec![&mut a, &mut b, &mut drawing]);
        assert_eq!(a.site.ring.len(), 3);
        assert_eq!(shared.map(|r| r.len()), Some(3));
    }

    #[test]
    fn stats_line_or_nothing() {
        let text = stats_text(Lang::RU, "Уклон", &[1.0, 2.0, 3.0, f64::NAN], "°");
        assert_eq!(text, "Уклон - среднее 2.00°, разброс 0.82°, квантили 10, 25, 50, 75 и 90 % - 1.20, 1.50, 2.00, 2.50, 2.80°.");
        assert!(stats_text(Lang::RU, "Уклон", &[f64::NAN], "°").is_empty());
    }

    #[test]
    fn frame_covers_pixels() {
        let f = layer().frame();
        assert!((f[0] - 58.0105).abs() < 1e-12 && (f[1] - 58.0085).abs() < 1e-12);
        assert!((f[3] - 56.239).abs() < 1e-12 && (f[2] - 56.245).abs() < 1e-12);
    }

    #[test]
    fn folder_holds_outline_and_picture() {
        let saved = Saved { title: "Тени".into(), ring: data::site::square_ring(58.0, 56.25, 1000.0), layers: vec![layer()], extra: Vec::new(), session: None };
        let folder = saved.folder(Lang::RU, 3723.0, &["C:/r/shadows.tif".into()]);
        assert_eq!(folder.name, "Тени 01:02:03 UTC");
        assert!(matches!(folder.children.first(), Some(Item::Place(p)) if p.points.len() == 4));
        assert!(matches!(folder.children.get(1), Some(Item::Overlay(o)) if o.image.as_ref().is_some_and(|i| i.starts_with(b"\x89PNG")) && o.description.starts_with("Тени - сводка. ") && o.description.contains("shadows.tif")));
        // Папка переживает запись KMZ «Моих меток».
        let kmz = scene::kml::write_kmz(&folder);
        assert!(kmz.is_ok());
        let back = kmz.ok().and_then(|b| scene::kml::read_file(&b, "x.kmz").ok());
        assert!(back.is_some_and(|f| f.overlays().len() == 1));
    }
}
