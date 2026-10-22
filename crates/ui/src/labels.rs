// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Надписи пунктов поверх глобуса, перенос `render/labels.py` модуля.
//!
//! Пункты - из векторных тайлов OpenFreeMap (`data::places`). Надписи
//! стоят на экране ровно, как в Google Earth, и идут по важности без
//! наложений. Надпись гаснет, когда пункт уходит к горизонту.
//!
//! Пункт за горой не подписывается. В модуле это запрос перекрытия
//! на видеокарте, здесь - луч на процессоре: от глаза к проверочной
//! точке над пунктом через высоты рельефа. Пункт без проверки
//! не подписывается, пока его не проверят.

use std::collections::{HashMap, HashSet};

use data::places::{self, Kind, Place, PlaceStore, select_labels};
use eframe::egui;
use globe_core::DVec3;
use globe_core::camera::Camera;
use globe_core::ellipsoid::Ellipsoid;
use globe_core::lod::TileKey;
use net::{Content, Loaded};
use terrain::HeightStore;

use crate::i18n::Lang;

/// Проверочная точка придвигается к глазу на такую долю расстояния.
/// Меньший запас путает пункт с самим склоном под ним.
pub const OCCLUSION_MARGIN: f64 = 0.03;
/// Подъём проверочной точки над пунктом, доля расстояния до глаза.
pub const LIFT: f64 = 0.002;
/// Пунктов проверяется за кадр, не больше.
pub const MAX_TESTS: usize = 64;
/// Шагов луча проверки.
pub const RAY_STEPS: usize = 48;
/// Высот пунктов уточняется за кадр после прихода новых высот.
pub const HEIGHTS_PER_FRAME: usize = 80;
/// Пунктов в окне, которые спорят за место.
pub const MAX_CANDIDATES: usize = 600;
/// Зазор между надписями, логических пикселей. При 3 пикселях надписи
/// из космоса шли сплошным ковром, модуль, 26 сентября 2026 года.
pub const PADDING: f32 = 10.0;
/// Сетка проверки наложений, логических пикселей.
pub const CELL: f64 = 64.0;
/// Надпись гаснет у горизонта: мера - синус угла луча зрения над
/// касательной плоскостью в пункте. Ниже FADE_LOW надписи нет, выше
/// FADE_HIGH она целиком.
pub const FADE_LOW: f64 = 0.12;
pub const FADE_HIGH: f64 = 0.35;
/// Обводка текста: цвет и толщина, как в модуле.
pub const HALO: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 0, 0, 140);
pub const HALO_WIDTH: f32 = 1.4;
/// Крупный город - город или столица с рангом не больше BIG_RANK, его
/// надпись крупнее на BIG_STEP.
pub const BIG_RANK: i64 = 4;
pub const BIG_STEP: f32 = 2.0;
/// Табличка номера дороги: европейский маршрут зелёный, остальные тёмные.
pub const SHIELD_E: egui::Color32 = egui::Color32::from_rgba_premultiplied(28, 115, 64, 235);
pub const SHIELD: egui::Color32 = egui::Color32::from_rgba_premultiplied(51, 53, 61, 235);
/// Подпись векторной основы, как в модуле.
pub const ATTRIBUTION: (&str, &str) =
    ("OpenFreeMap © OpenMapTiles, © OpenStreetMap contributors", "https://openfreemap.org/");

/// Значок пункта.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    None,
    Dot,
    Peak,
    Square,
    Shield,
    /// Метка найденного места: красный круг с белым кольцом.
    Pin,
    /// Своя метка: жёлтый круг с тёмным кольцом, как в Google Earth.
    Yellow,
}

/// Своя метка, как в модуле.
pub const MARK: egui::Color32 = egui::Color32::from_rgb(255, 214, 0);

/// Метка найденного места, как в модуле.
pub const PIN: egui::Color32 = egui::Color32::from_rgb(219, 50, 54);

/// Вид надписи класса: кегль в логических пикселях, курсив, цвет,
/// размер значка и значок, как STYLES модуля. Жирного шрифта во
/// встроенных шрифтах egui нет.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    pub size: f32,
    pub italic: bool,
    pub color: egui::Color32,
    pub dot: f32,
    pub marker: Marker,
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// Вид надписи пункта. Классы без своих объектов в Lite получают вид
/// населённого пункта.
pub fn style(kind: Kind, big: bool) -> Style {
    let s = |size: f32, italic: bool, color: egui::Color32, dot: f32, marker: Marker| Style {
        size,
        italic,
        color,
        dot,
        marker,
    };
    let base = match kind {
        Kind::Country => s(14.0, false, rgba(255, 244, 214, 235), 0.0, Marker::None),
        Kind::Capital => s(13.0, false, rgba(255, 255, 255, 240), 3.0, Marker::Dot),
        Kind::City => s(13.0, false, rgba(255, 255, 255, 235), 2.5, Marker::Dot),
        Kind::State => s(11.0, true, rgba(230, 230, 230, 200), 0.0, Marker::None),
        Kind::Water => s(12.0, true, rgba(165, 210, 255, 230), 0.0, Marker::None),
        Kind::Town => s(12.0, false, rgba(255, 255, 255, 225), 2.2, Marker::Dot),
        Kind::Park => s(11.0, true, rgba(160, 225, 150, 220), 0.0, Marker::None),
        Kind::Peak => s(11.0, false, rgba(255, 236, 205, 225), 3.2, Marker::Peak),
        Kind::Airport => s(11.0, false, rgba(255, 255, 255, 225), 2.6, Marker::Square),
        Kind::RoadRef => s(10.0, false, rgba(255, 255, 255, 245), 0.0, Marker::Shield),
        Kind::Village => s(11.0, false, rgba(240, 240, 240, 210), 1.8, Marker::Dot),
        Kind::Search => s(13.0, false, rgba(255, 255, 255, 250), 5.0, Marker::Pin),
        Kind::Mark => s(12.0, false, rgba(255, 255, 255, 245), 4.5, Marker::Yellow),
        | Kind::Ruler
        | Kind::Layer
        | Kind::Circle
        | Kind::Grid
        | Kind::Plate
        | Kind::Value
        | Kind::Wiki => s(12.0, false, rgba(255, 255, 255, 245), 0.0, Marker::None),
    };
    if big { Style { size: base.size + BIG_STEP, dot: base.dot + 0.5, ..base } } else { base }
}

/// Классы пунктов основы.
pub const PLACE_KINDS: [Kind; 11] = [
    Kind::Country,
    Kind::Capital,
    Kind::City,
    Kind::State,
    Kind::Water,
    Kind::Town,
    Kind::Park,
    Kind::Peak,
    Kind::Airport,
    Kind::RoadRef,
    Kind::Village,
];

/// Текст надписи. У вершины после названия высота.
pub fn label_text(place: &Place, lang: Lang) -> String {
    match (place.kind, place.info) {
        (Kind::Peak, Some(ele)) => match lang {
            Lang::Ru => format!("{}, {ele} м", place.name),
            Lang::En => format!("{}, {ele} m", place.name),
        },
        _ => place.name.clone(),
    }
}

/// Пункт закрыт рельефом: на отрезке от глаза до точки test есть
/// место, где рельеф выше отрезка. height - высота рельефа по широте
/// и долготе.
pub fn occluded(camera: &Camera, test: DVec3, height: &dyn Fn(f64, f64) -> f64) -> bool {
    let e = &camera.ellipsoid;
    (1..RAY_STEPS).any(|i| {
        let t = i as f64 / RAY_STEPS as f64;
        let p = camera.eye + (test - camera.eye) * t;
        let (lat, lon, h) = e.ecef_to_geodetic(p);
        height(lat, lon) > h
    })
}

type Id = (u64, String);

/// Отметка проверки: глаз с точностью до метра и версия высот.
type Stamp = ([i64; 3], u64);

/// Нарисованная надпись: текст и точка привязки в логических пикселях.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub text: String,
    pub at: egui::Pos2,
}

/// Надписи пунктов: тайлы пунктов, высоты, проверки видимости.
pub struct Labels {
    lang: Lang,
    languages: Vec<String>,
    store: PlaceStore,
    /// Точка и нормаль пункта и версия высот, по которой они взяты.
    points: HashMap<Id, (u64, DVec3, DVec3)>,
    /// Итог проверки: закрыт ли пункт и отметка проверки.
    hidden: HashMap<Id, (bool, Stamp)>,
    cursor: usize,
    shown: HashSet<Id>,
    /// Надписи последнего кадра.
    pub drawn: Vec<Shown>,
    /// Кадр не окончательный: проверки или высоты не успели.
    pub pending: bool,
    /// Метка найденного места.
    search: Option<Place>,
    /// Точки меток открытой сцены.
    marks: Vec<Place>,
}

impl Labels {
    pub fn new(lang: Lang) -> Self {
        let code = match lang {
            Lang::Ru => "ru",
            Lang::En => "en",
        };
        Self {
            lang,
            languages: places::name_languages(places::AS_INTERFACE, code),
            store: PlaceStore::default(),
            points: HashMap::new(),
            hidden: HashMap::new(),
            cursor: 0,
            shown: HashSet::new(),
            drawn: Vec::new(),
            pending: false,
            search: None,
            marks: Vec::new(),
        }
    }

    /// Точки меток открытой сцены: название, широта, долгота.
    pub fn set_marks(&mut self, points: &[(String, f64, f64)]) {
        self.marks = points
            .iter()
            .enumerate()
            .map(|(i, (name, lat, lon))| Place::new(u64::MAX - 1 - i as u64, name, Kind::Mark, 0, *lat, *lon))
            .collect();
    }

    /// Метка найденного места, одна на окно. None - метки нет.
    pub fn set_search(&mut self, found: Option<(&str, f64, f64)>) {
        self.search = found.map(|(name, lat, lon)| Place::new(u64::MAX, name, Kind::Search, 0, lat, lon));
    }

    /// Тайлы пунктов, которые нужны тайлам кадра draw, грубые раньше.
    pub fn wanted(&self, draw: &[TileKey]) -> Vec<(TileKey, f64)> {
        let mut out: Vec<(TileKey, f64)> = self
            .store
            .wanted(draw)
            .into_iter()
            .filter(|k| !self.store.has(*k))
            .map(|k| (k, 100.0 - f64::from(k.0)))
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        out
    }

    /// Пришедший тайл пунктов. Тайл, которого нет, и тайл, который
    /// не разбирается, дают пустой список: снова они не просятся.
    pub fn add(&mut self, loaded: Loaded) {
        let found = match loaded.content {
            Ok(Content::Bytes(bytes)) => places::decode_places(loaded.key, &bytes, &self.languages).unwrap_or_default(),
            Ok(Content::Image(_)) => Vec::new(),
            Err(_) if loaded.missing => Vec::new(),
            Err(_) => return,
        };
        self.store.add(loaded.key, found);
    }

    /// Выбрать и нарисовать надписи кадра. rect - место кадра в окне,
    /// ppp - пикселей кадра на логический пиксель.
    pub fn draw(
        &mut self,
        painter: &egui::Painter,
        rect: egui::Rect,
        ppp: f32,
        camera: &Camera,
        draw: &[TileKey],
        heights: &HeightStore,
    ) {
        self.drawn.clear();
        self.pending = false;
        let altitude = camera.altitude();
        let kinds = places::kinds_at(&PLACE_KINDS.into_iter().collect(), altitude);
        let collected = self.store.collect(draw, Some(&kinds));
        let search = self.search.clone();
        let marks = self.marks.clone();
        let all: Vec<&Place> = search.iter().chain(marks.iter()).chain(collected.iter()).collect();
        if all.is_empty() {
            self.shown.clear();
            return;
        }
        let version = heights.version();
        self.update_points(&all, heights, version, &camera.ellipsoid);
        let (w, h) = (f64::from(camera.width), f64::from(camera.height));
        // Кандидаты: перед камерой, на обращённой к глазу стороне,
        // в кадре с полем 50 пикселей.
        let mut candidates = Vec::new();
        for &place in &all {
            let id = places::identity(place);
            let Some(&(_, point, normal)) = self.points.get(&id) else {
                continue;
            };
            let v = point - camera.eye;
            let dist = v.length().max(1e-9);
            let rise = -v.dot(normal) / dist;
            if rise <= FADE_LOW {
                continue;
            }
            let (px, front) = camera.project(point);
            if !front || px.x < -50.0 || px.x > w + 50.0 || px.y < -50.0 || px.y > h + 50.0 {
                continue;
            }
            candidates.push((place, id, point, normal, dist, rise, px));
            if candidates.len() >= MAX_CANDIDATES {
                break;
            }
        }
        let stamp: Stamp = ([camera.eye.x.round() as i64, camera.eye.y.round() as i64, camera.eye.z.round() as i64], version);
        self.test(camera, heights, &candidates.iter().map(|c| (c.1.clone(), c.2, c.3, c.4)).collect::<Vec<_>>(), stamp);
        let mut keep = Vec::new();
        let mut unknown = false;
        for c in &candidates {
            match self.hidden.get(&c.1) {
                Some((false, s)) => {
                    unknown |= *s != stamp;
                    keep.push(c);
                }
                Some((true, s)) => unknown |= *s != stamp,
                None => unknown = true,
            }
        }
        let mut layouts = Vec::new();
        let mut boxes = Vec::new();
        let mut tiers = Vec::new();
        for c in &keep {
            let place = c.0;
            let big = matches!(place.kind, Kind::City | Kind::Capital) && place.rank <= BIG_RANK;
            let st = style(place.kind, big);
            let text = label_text(place, self.lang);
            let galley = painter.layout_job(job(&text, st, egui::Color32::WHITE));
            let size = galley.size();
            let (bw, bh, ax, ay) = layout_box(st, size);
            let at = rect.min + egui::vec2(c.6.x as f32 / ppp, c.6.y as f32 / ppp);
            let x0 = (at.x - ax).round();
            let y0 = (at.y - ay).round();
            boxes.push([
                f64::from(x0 - PADDING),
                f64::from(y0 - PADDING),
                f64::from(x0 + bw + PADDING),
                f64::from(y0 + bh + PADDING),
            ]);
            tiers.push(place.kind.tier());
            layouts.push((text, st, galley, egui::pos2(x0, y0), at));
        }
        let previous: HashSet<usize> = keep.iter().enumerate().filter(|(_, c)| self.shown.contains(&c.1)).map(|(n, _)| n).collect();
        let chosen = select_labels(&boxes, &tiers, &previous, CELL);
        let mut shown = HashSet::new();
        for n in chosen {
            let c = keep[n];
            let alpha = ((c.5 - FADE_LOW) / (FADE_HIGH - FADE_LOW)).clamp(0.0, 1.0) as f32;
            let (text, st, galley, corner, at) = &layouts[n];
            paint(painter, *st, galley.clone(), *corner, text, alpha);
            shown.insert(c.1.clone());
            self.drawn.push(Shown { text: text.clone(), at: *at });
        }
        self.shown = shown;
        self.pending = unknown;
    }

    /// Точки пунктов по рельефу. Новый пункт получает точку сразу,
    /// после прихода новых высот уточняется не больше
    /// [`HEIGHTS_PER_FRAME`] пунктов за кадр.
    fn update_points(&mut self, all: &[&Place], heights: &HeightStore, version: u64, e: &Ellipsoid) {
        if self.points.len() > 50_000 {
            self.points.clear();
        }
        let mut todo: Vec<&Place> = all.iter().copied().filter(|p| !self.points.contains_key(&places::identity(p))).collect();
        let stale: Vec<&Place> = all
            .iter()
            .copied()
            .filter(|p| self.points.get(&places::identity(p)).is_some_and(|(v, _, _)| *v != version))
            .take(HEIGHTS_PER_FRAME)
            .collect();
        if stale.len() == HEIGHTS_PER_FRAME {
            self.pending = true;
        }
        todo.extend(stale);
        if todo.is_empty() {
            return;
        }
        let at: Vec<(f64, f64)> = todo.iter().map(|p| (p.lat, p.lon)).collect();
        let hs = heights.heights_at(&at, true);
        for (place, h) in todo.into_iter().zip(hs) {
            let point = e.geodetic_to_ecef(place.lat, place.lon, h);
            let normal = e.surface_normal(place.lat, place.lon);
            self.points.insert(places::identity(place), (version, point, normal));
        }
    }

    /// Проверки видимости: сначала пункты без проверки, затем
    /// устаревшие и проверенные по кругу, не больше [`MAX_TESTS`].
    fn test(&mut self, camera: &Camera, heights: &HeightStore, items: &[(Id, DVec3, DVec3, f64)], stamp: Stamp) {
        if self.hidden.len() > 50_000 {
            self.hidden.clear();
        }
        let mut fresh = Vec::new();
        let mut stale = Vec::new();
        for (n, item) in items.iter().enumerate() {
            match self.hidden.get(&item.0) {
                None => fresh.push(n),
                Some((_, s)) if *s != stamp => stale.push(n),
                Some(_) => {}
            }
        }
        let mut order = fresh;
        if !stale.is_empty() {
            let start = self.cursor % stale.len();
            order.extend(stale[start..].iter().chain(&stale[..start]));
        }
        self.cursor = self.cursor.wrapping_add(MAX_TESTS);
        let height = |lat: f64, lon: f64| heights.height_at(lat, lon);
        for &n in order.iter().take(MAX_TESTS) {
            let (id, point, normal, dist) = &items[n];
            let lifted = *point + *normal * (LIFT * dist);
            let test = camera.eye + (lifted - camera.eye) * (1.0 - OCCLUSION_MARGIN);
            self.hidden.insert(id.clone(), (occluded(camera, test, &height), stamp));
        }
    }
}

/// Текст надписи в разметке egui.
fn job(text: &str, st: Style, color: egui::Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        text,
        0.0,
        egui::TextFormat { font_id: egui::FontId::proportional(st.size), color, italics: st.italic, ..Default::default() },
    );
    job
}

/// Размер надписи со значком и точка привязки в ней, как `_Style.layout`
/// модуля.
fn layout_box(st: Style, text: egui::Vec2) -> (f32, f32, f32, f32) {
    let halo = HALO_WIDTH;
    let gap = 3.0;
    let pad = 3.0;
    match st.marker {
        Marker::Shield => {
            let (w, h) = (text.x + 2.0 * pad + 2.0, text.y + pad + 2.0);
            (w, h, w / 2.0, h / 2.0)
        }
        Marker::None => {
            let (w, h) = (text.x + 2.0 * halo, text.y + 2.0 * halo);
            (w, h, w / 2.0, h / 2.0)
        }
        _ => {
            let w = halo + 2.0 * st.dot + gap + text.x + halo;
            let h = text.y.max(2.0 * st.dot) + 2.0 * halo;
            (w, h, halo + st.dot, h / 2.0)
        }
    }
}

fn fade(color: egui::Color32, alpha: f32) -> egui::Color32 {
    color.gamma_multiply(alpha)
}

/// Нарисовать надпись с левым верхним углом corner.
fn paint(
    painter: &egui::Painter,
    st: Style,
    galley: std::sync::Arc<egui::Galley>,
    corner: egui::Pos2,
    text: &str,
    alpha: f32,
) {
    let size = galley.size();
    let (w, h, ax, ay) = layout_box(st, size);
    let anchor = corner + egui::vec2(ax, ay);
    let halo = fade(HALO, alpha);
    let color = fade(st.color, alpha);
    if st.marker == Marker::Shield {
        let fill = if text.starts_with('E') { SHIELD_E } else { SHIELD };
        let r = egui::Rect::from_min_size(corner, egui::vec2(w, h));
        painter.rect(
            r.shrink(0.5),
            3.0,
            fade(fill, alpha),
            egui::Stroke::new(1.0, fade(egui::Color32::from_white_alpha(200), alpha)),
            egui::StrokeKind::Inside,
        );
        let pos = egui::pos2(corner.x + 4.0, corner.y + (h - size.y) / 2.0);
        painter.galley_with_override_text_color(pos, galley, color);
        return;
    }
    let text_x = corner.x + HALO_WIDTH + if st.marker == Marker::None { 0.0 } else { 2.0 * st.dot + 3.0 };
    let pos = egui::pos2(text_x, corner.y + (h - size.y) / 2.0);
    // Обводка: текст тёмным цветом со сдвигом во все стороны.
    for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (1.0, -1.0)] {
        painter.galley_with_override_text_color(pos + egui::vec2(dx, dy), galley.clone(), halo);
    }
    painter.galley_with_override_text_color(pos, galley, color);
    let r = st.dot;
    let stroke = egui::Stroke::new(HALO_WIDTH * 0.8, halo);
    match st.marker {
        Marker::Dot => {
            painter.circle(anchor, r, color, stroke);
        }
        Marker::Peak => {
            let points =
                vec![anchor + egui::vec2(0.0, -r), anchor + egui::vec2(r, r * 0.8), anchor + egui::vec2(-r, r * 0.8)];
            painter.add(egui::Shape::convex_polygon(points, color, stroke));
        }
        Marker::Square => {
            let rect = egui::Rect::from_center_size(anchor, egui::vec2(1.6 * r, 1.6 * r));
            painter.rect(rect, 0.0, color, stroke, egui::StrokeKind::Middle);
        }
        Marker::Pin => {
            let ring = egui::Stroke::new(r * 0.45, fade(egui::Color32::from_white_alpha(240), alpha));
            painter.circle(anchor, r, fade(PIN, alpha), ring);
        }
        Marker::Yellow => {
            let ring = egui::Stroke::new(r * 0.35, fade(egui::Color32::from_rgba_unmultiplied(60, 40, 0, 230), alpha));
            painter.circle(anchor, r, fade(MARK, alpha), ring);
        }
        Marker::None | Marker::Shield => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, Marker, OCCLUSION_MARGIN, occluded, style};
    use globe_core::camera::Camera;
    use globe_core::ellipsoid::Ellipsoid;

    #[test]
    fn every_kind_has_style() {
        // Перенос `test_label_kinds.py`: у каждого класса есть вид. Там
        // класс без вида ронял кадр, здесь вид даёт полное сопоставление.
        for kind in Kind::ALL {
            let st = style(kind, false);
            assert!(st.size >= 10.0, "{kind:?}");
            assert_eq!(st.dot == 0.0, matches!(st.marker, Marker::None | Marker::Shield), "{kind:?}");
        }
        assert!(style(Kind::City, true).size > style(Kind::City, false).size);
    }

    #[test]
    fn ridge_hides_the_place_behind_it() {
        // Глаз в 1 км над точкой, пункт в 20 км к северу на уровне моря.
        // Хребет высотой 2 км на полпути закрывает пункт, ровная земля -
        // нет.
        let e = Ellipsoid::wgs84();
        let camera = Camera::look_at(e, 45.0, 10.0, 1000.0, 0.0, 0.0, 0.0);
        let place = e.geodetic_to_ecef(45.18, 10.0, 0.0);
        let test = camera.eye + (place - camera.eye) * (1.0 - OCCLUSION_MARGIN);
        assert!(!occluded(&camera, test, &|_, _| 0.0));
        let ridge = |lat: f64, _lon: f64| if (45.08..45.10).contains(&lat) { 2000.0 } else { 0.0 };
        assert!(occluded(&camera, test, &ridge));
    }
}