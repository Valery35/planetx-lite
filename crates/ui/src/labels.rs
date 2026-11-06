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
/// Отступ надписи от края кадра, логических пикселей.
pub const EDGE: f32 = 4.0;
/// Сетка проверки наложений, логических пикселей.
pub const CELL: f64 = 64.0;
/// Надпись гаснет у горизонта: мера - синус угла луча зрения над
/// касательной плоскостью в пункте. Ниже FADE_LOW надписи нет, выше
/// FADE_HIGH она целиком.
pub const FADE_LOW: f64 = 0.12;
pub const FADE_HIGH: f64 = 0.35;
/// Обводка значков: цвет и толщина, как в модуле.
pub const HALO: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 0, 0, 140);
pub const HALO_WIDTH: f32 = 1.4;
/// Мягкая тень светлых надписей вместо обводки, как у Google Earth,
/// модуль 1.38: тёмный след текста толщиной SHADOW_SPREAD размыт
/// на SHADOW_BLUR логических пикселей, непрозрачность SHADOW_ALPHA.
/// Размытия на картинке у egui нет, тень - копии текста по кольцам
/// с убывающей непрозрачностью [`SHADOW_RINGS`].
pub const SHADOW_SPREAD: f32 = 1.0;
pub const SHADOW_BLUR: f32 = 2.0;
pub const SHADOW_ALPHA: f32 = 0.85;
/// Кольца тени: радиус в логических пикселях и доля SHADOW_ALPHA одной
/// копии. Восемь копий кольца 1 дают у текста почти полную тень.
pub const SHADOW_RINGS: [(f32, f32); 3] = [(1.0, 0.26), (2.0, 0.12), (3.0, 0.05)];
/// Разрядка прописных подписей стран, областей и районов, логических
/// пикселей.
pub const SPACING: f32 = 1.0;
/// Подпись реки поворачивается шагами TURN_STEP градусов.
pub const TURN_STEP: f64 = 5.0;
/// Подписей класса в кадре, не больше, на кадр 1920 × 1080 логических
/// пикселей, у другого кадра - в долю его площади, `KIND_CAPS` модуля.
/// Река одного названия подписывается в кадре один раз.
pub fn kind_cap(kind: Kind) -> Option<usize> {
    match kind {
        Kind::Suburb => Some(6),
        Kind::River => Some(3),
        Kind::Village => Some(8),
        Kind::Town => Some(12),
        _ => None,
    }
}
pub const CAP_AREA: f32 = 1920.0 * 1080.0;
/// Крупный город - город или столица с рангом не больше BIG_RANK, его
/// надпись крупнее на BIG_STEP, модуль 1.38.
pub const BIG_RANK: i64 = 4;
pub const BIG_STEP: f32 = 7.0;
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

/// Подпись класса прописными с разрядкой, `CAPS` модуля.
pub fn caps(kind: Kind) -> bool {
    matches!(kind, Kind::Country | Kind::State | Kind::Suburb)
}

/// Вид надписи пункта. Классы без своих объектов в Lite получают вид
/// населённого пункта. Населённые пункты - без точек, ранг виден по
/// кеглю, как у Google Earth, модуль 1.38.
pub fn style(kind: Kind, big: bool) -> Style {
    let s = |size: f32, italic: bool, color: egui::Color32, dot: f32, marker: Marker| Style {
        size,
        italic,
        color,
        dot,
        marker,
    };
    let base = match kind {
        Kind::Country => s(15.0, false, rgba(255, 248, 230, 240), 0.0, Marker::None),
        Kind::Capital => s(17.0, false, rgba(255, 255, 255, 250), 0.0, Marker::None),
        Kind::City => s(15.0, false, rgba(255, 255, 255, 245), 0.0, Marker::None),
        Kind::State => s(12.0, false, rgba(235, 235, 235, 215), 0.0, Marker::None),
        Kind::Water => s(13.0, true, rgba(190, 222, 255, 235), 0.0, Marker::None),
        Kind::Town => s(13.0, false, rgba(255, 255, 255, 240), 0.0, Marker::None),
        Kind::Suburb => s(11.0, false, rgba(245, 245, 245, 230), 0.0, Marker::None),
        Kind::River => s(12.0, false, rgba(205, 228, 255, 240), 0.0, Marker::None),
        Kind::Park => s(11.0, true, rgba(160, 225, 150, 220), 0.0, Marker::None),
        Kind::Peak => s(11.0, false, rgba(255, 236, 205, 225), 3.2, Marker::Peak),
        Kind::Airport => s(11.0, false, rgba(255, 255, 255, 225), 2.6, Marker::Square),
        Kind::RoadRef => s(10.0, false, rgba(255, 255, 255, 245), 0.0, Marker::Shield),
        Kind::Village => s(12.0, false, rgba(240, 240, 240, 225), 0.0, Marker::None),
        Kind::Search => s(13.0, false, rgba(255, 255, 255, 250), 5.0, Marker::Pin),
        Kind::Mark => s(12.0, false, rgba(255, 255, 255, 245), 4.5, Marker::Yellow),
        // Название литосферной плиты. У модуля надпись жирная, у Lite
        // жирного начертания нет.
        Kind::Plate => s(13.0, false, rgba(255, 200, 140, 240), 0.0, Marker::None),
        // Подпись линии сетки и геофизики, без значка.
        Kind::Grid => s(11.0, false, rgba(255, 255, 255, 235), 0.0, Marker::None),
        | Kind::Ruler
        | Kind::Layer
        | Kind::Circle
        | Kind::Value
        | Kind::Wiki => s(12.0, false, rgba(255, 255, 255, 245), 0.0, Marker::None),
    };
    if big { Style { size: base.size + BIG_STEP, ..base } } else { base }
}

/// Подписи основы издалека мельче, `label_scale` модуля
/// (`core/places.py`), замечание автора от 7 ноября 2026 года «подписи
/// огромные». До LABEL_NEAR метров высоты глаза кегль полный, к LABEL_FAR
/// уменьшается до доли LABEL_SMALLEST по логарифму высоты, ступенями
/// LABEL_STEP. Свои метки, линейка, подписи слоёв, сетки и погоды
/// не уменьшаются.
pub const LABEL_NEAR: f64 = 300e3;
pub const LABEL_FAR: f64 = 6000e3;
pub const LABEL_SMALLEST: f64 = 0.7;
pub const LABEL_STEP: f64 = 0.1;

/// Доля кегля подписей основы при высоте глаза altitude, метры.
pub fn label_scale(altitude: f64) -> f32 {
    if altitude.is_nan() || altitude <= LABEL_NEAR {
        return 1.0;
    }
    let share = ((altitude / LABEL_NEAR).ln() / (LABEL_FAR / LABEL_NEAR).ln()).min(1.0);
    let value = 1.0 - (1.0 - LABEL_SMALLEST) * share;
    ((value / LABEL_STEP).round() * LABEL_STEP) as f32
}

/// Уменьшается ли подпись класса издалека, `SHRINK` модуля.
pub fn shrinks(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Country
            | Kind::Capital
            | Kind::City
            | Kind::State
            | Kind::Water
            | Kind::Town
            | Kind::River
            | Kind::Suburb
            | Kind::Park
            | Kind::Peak
            | Kind::Airport
            | Kind::RoadRef
            | Kind::Village
            | Kind::Wiki
            | Kind::Plate
    )
}

/// Классы пунктов основы.
pub const PLACE_KINDS: [Kind; 13] = [
    Kind::Country,
    Kind::Capital,
    Kind::City,
    Kind::State,
    Kind::Water,
    Kind::Town,
    Kind::River,
    Kind::Suburb,
    Kind::Park,
    Kind::Peak,
    Kind::Airport,
    Kind::RoadRef,
    Kind::Village,
];

/// Текст надписи. У вершины после названия высота.
pub fn label_text(place: &Place, lang: Lang) -> String {
    match (place.kind, place.info) {
        (Kind::Peak, Some(ele)) => crate::i18n::trf(lang, "{name}, {ele} m", &[("name", &place.name), ("ele", &ele.to_string())]),
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
    /// Разрез Земли: пункты внутри сектора не подписываются.
    pub wedge: Option<data::cutaway::Wedge>,
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
    /// Названия литосферных плит, строка «Границы плит».
    plates: Vec<Place>,
    /// Подписи геофизики: склонение, скорости плит, полюса, как
    /// `geo_marks` модуля.
    geo: Vec<Place>,
    /// Подписи слоёв «Моих данных» по полю.
    layers: Vec<Place>,
    /// Цвет значка метки по номеру пункта.
    mark_colors: HashMap<u64, egui::Color32>,
    /// Классы подписей по включённым строкам векторной основы, как
    /// `label_kinds` модуля.
    pub kinds: HashSet<Kind>,
    /// Пришедшие тайлы, ещё не разобранные, и их ключи.
    queue: std::collections::VecDeque<(TileKey, Option<std::sync::Arc<Vec<u8>>>)>,
    queued_keys: HashSet<TileKey>,
}

impl Labels {
    /// Подписи на языке интерфейса lang. choice - язык названий:
    /// [`places::AS_INTERFACE`], [`places::LOCAL`] или код языка.
    pub fn new(lang: Lang, choice: &str) -> Self {
        Self {
            wedge: None,
            lang,
            languages: places::name_languages(choice, lang.code()),
            store: PlaceStore::default(),
            points: HashMap::new(),
            hidden: HashMap::new(),
            cursor: 0,
            shown: HashSet::new(),
            drawn: Vec::new(),
            pending: false,
            search: None,
            marks: Vec::new(),
            plates: Vec::new(),
            geo: Vec::new(),
            layers: Vec::new(),
            mark_colors: HashMap::new(),
            kinds: crate::vector::VectorBase::default().label_kinds(),
            queue: std::collections::VecDeque::new(),
            queued_keys: HashSet::new(),
        }
    }

    /// Точки «Моих меток»: название, широта, долгота, цвет значка.
    /// Значок на глобусе - цвета метки, как значок её строки в дереве.
    pub fn set_marks(&mut self, points: &[(String, f64, f64, [u8; 4])]) {
        self.mark_colors.clear();
        self.marks = points
            .iter()
            .enumerate()
            .map(|(i, (name, lat, lon, icon))| {
                let id = u64::MAX - 1 - i as u64;
                self.mark_colors.insert(id, egui::Color32::from_rgb(icon[0], icon[1], icon[2]));
                Place::new(id, name, Kind::Mark, 0, *lat, *lon)
            })
            .collect();
    }

    /// Названия плит: название, широта, долгота, как `plate_marks`
    /// модуля. Пустой список - строка «Границы плит» снята.
    pub fn set_plates(&mut self, plates: &[(String, f64, f64)]) {
        self.plates = plates.iter().enumerate().map(|(n, (name, lat, lon))| Place::new(u64::MAX - 1_000_000 - n as u64, name, Kind::Plate, 1, *lat, *lon)).collect();
    }

    /// Подписи геофизики: текст, широта, долгота.
    pub fn set_geo(&mut self, labels: &[(String, f64, f64)]) {
        self.geo = labels.iter().enumerate().map(|(n, (text, lat, lon))| Place::new(u64::MAX - 2_000_000 - n as u64, text, Kind::Grid, 1, *lat, *lon)).collect();
    }

    /// Подписи слоёв «Моих данных»: текст, широта, долгота, без значка,
    /// как подписи слоёв проекта модуля.
    pub fn set_layers(&mut self, labels: &[(String, f64, f64)]) {
        self.layers = labels.iter().enumerate().map(|(n, (text, lat, lon))| Place::new(u64::MAX - 3_000_000 - n as u64, text, Kind::Layer, 1, *lat, *lon)).collect();
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
            .filter(|k| !self.store.has(*k) && !self.queued_keys.contains(k))
            .map(|k| (k, 100.0 - f64::from(k.0)))
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        out
    }

    /// Пришедший тайл пунктов встаёт в очередь разбора: за кадр их
    /// приходило до сотни, разбор сразу занимал до 60 мс главного потока.
    /// bytes - байты тайла, общие с линиями, None - тайла нет.
    pub fn add(&mut self, key: TileKey, bytes: Option<std::sync::Arc<Vec<u8>>>) {
        self.queued_keys.insert(key);
        self.queue.push_back((key, bytes));
    }

    /// Разобрать тайлы очереди, пока не вышло время budget в секундах.
    /// Возвращает, осталась ли очередь.
    pub fn decode_queued(&mut self, budget: f64) -> bool {
        let start = web_time::Instant::now();
        while let Some((key, bytes)) = self.queue.pop_front() {
            self.queued_keys.remove(&key);
            let found = bytes.map_or_else(Vec::new, |b| places::decode_places(key, &b, &self.languages).unwrap_or_default());
            self.store.add(key, found);
            if start.elapsed().as_secs_f64() > budget {
                break;
            }
        }
        !self.queue.is_empty()
    }

    /// Выбрать и нарисовать надписи кадра. rect - место кадра в окне,
    /// ppp - пикселей кадра на логический пиксель. blocked - места
    /// поверх кадра, под которыми надписей нет.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        painter: &egui::Painter,
        rect: egui::Rect,
        ppp: f32,
        blocked: &[egui::Rect],
        camera: &Camera,
        draw: &[TileKey],
        heights: &HeightStore,
    ) {
        self.drawn.clear();
        // Неразобранные тайлы - кадр не окончательный.
        self.pending = !self.queue.is_empty();
        let altitude = camera.altitude();
        let kinds = places::kinds_at(&self.kinds, altitude);
        let collected = self.store.collect(draw, Some(&kinds));
        let search = self.search.clone();
        let marks = self.marks.clone();
        let plates = self.plates.clone();
        let geo = self.geo.clone();
        let layers = self.layers.clone();
        let all: Vec<&Place> = search.iter().chain(marks.iter()).chain(plates.iter()).chain(geo.iter()).chain(layers.iter()).chain(collected.iter()).collect();
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
            // Пункты вынутого сектора висели бы над его гранями, как
            // у модуля.
            if self.wedge.as_ref().is_some_and(|w| data::cutaway::inside(w, place.lat, place.lon)) {
                continue;
            }
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
        // Надпись целиком в кадре и не под занятыми местами, обрезанных
        // надписей нет.
        let room = rect.shrink(EDGE);
        let scale = label_scale(altitude);
        let mut layouts = Vec::new();
        let mut boxes = Vec::new();
        let mut tiers = Vec::new();
        let mut placed = Vec::new();
        for c in keep {
            let place = c.0;
            let big = matches!(place.kind, Kind::City | Kind::Capital) && place.rank <= BIG_RANK;
            let small = scale < 1.0 && shrinks(place.kind);
            let mut st = style(place.kind, big);
            if small {
                st.size *= scale;
                st.dot *= scale;
            }
            let text = label_text(place, self.lang);
            let galley = painter.layout_job(job(&text, st, egui::Color32::WHITE, caps(place.kind)));
            let size = galley.size();
            let turn = river_turn(camera, place);
            let (bw, bh, ax, ay) = if turn != 0.0 { turned_box(st, size, turn) } else { layout_box(st, size) };
            let at = rect.min + egui::vec2(c.6.x as f32 / ppp, c.6.y as f32 / ppp);
            let x0 = (at.x - ax).round();
            let y0 = (at.y - ay).round();
            let area = egui::Rect::from_min_size(egui::pos2(x0, y0), egui::vec2(bw, bh));
            if !room.contains_rect(area) || blocked.iter().any(|b| b.intersects(area)) {
                continue;
            }
            placed.push(c);
            // Уменьшенная издалека подпись основы при отборе занимает
            // место прежнего кегля, как у модуля: иначе на освободившемся
            // месте вставали новые мелкие подписи и кадр не становился
            // чище.
            let (gx, gy) = if small { (bw * (0.5 / scale - 0.5), bh * (0.5 / scale - 0.5)) } else { (0.0, 0.0) };
            boxes.push([
                f64::from(x0 - PADDING - gx),
                f64::from(y0 - PADDING - gy),
                f64::from(x0 + bw + PADDING + gx),
                f64::from(y0 + bh + PADDING + gy),
            ]);
            tiers.push(place.kind.tier());
            let mark = self.mark_colors.get(&place.id).copied().filter(|_| place.kind == Kind::Mark);
            layouts.push((text, st, galley, egui::pos2(x0, y0), at, mark, turn));
        }
        let previous: HashSet<usize> = placed.iter().enumerate().filter(|(_, c)| self.shown.contains(&c.1)).map(|(n, _)| n).collect();
        let share = rect.area() / CAP_AREA;
        let chosen = thin(select_labels(&boxes, &tiers, &previous, CELL), &placed.iter().map(|c| c.0).collect::<Vec<_>>(), share);
        let mut shown = HashSet::new();
        for n in chosen {
            let Some(c) = placed.get(n) else {
                continue;
            };
            let alpha = ((c.5 - FADE_LOW) / (FADE_HIGH - FADE_LOW)).clamp(0.0, 1.0) as f32;
            let (text, st, galley, corner, at, mark, turn) = &layouts[n];
            if *turn != 0.0 {
                paint_turned(painter, *st, galley.clone(), *at, *turn, alpha);
            } else {
                paint(painter, *st, galley.clone(), *corner, text, alpha, *mark);
            }
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
            // Подпись над дном моря стоит на воде, как `label_heights`
            // модуля.
            let point = e.geodetic_to_ecef(place.lat, place.lon, h.max(0.0));
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

/// Текст надписи в разметке egui. capital - прописными с разрядкой.
fn job(text: &str, st: Style, color: egui::Color32, capital: bool) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let shown = if capital { text.to_uppercase() } else { text.to_string() };
    job.append(
        &shown,
        0.0,
        egui::TextFormat {
            font_id: crate::fonts::globe(st.size),
            color,
            italics: st.italic,
            extra_letter_spacing: if capital { SPACING } else { 0.0 },
            ..Default::default()
        },
    );
    job
}

/// Поле вокруг текста: тень с размытием, у таблички номера дороги своё.
fn field(st: Style) -> f32 {
    if st.marker == Marker::Shield { HALO_WIDTH } else { SHADOW_SPREAD + 2.0 * SHADOW_BLUR }
}

/// Поворот подписи реки по руслу на экране, радианы по часовой стрелке:
/// текст не вверх ногами, шагами [`TURN_STEP`]. 0 - не река или русло
/// не перед камерой.
pub fn river_turn(camera: &Camera, place: &Place) -> f32 {
    let Some([a_lat, a_lon, b_lat, b_lon, _]) = place.course.filter(|_| place.kind == Kind::River) else {
        return 0.0;
    };
    let e = &camera.ellipsoid;
    let (a, a_front) = camera.project(e.geodetic_to_ecef(a_lat, a_lon, 0.0));
    let (b, b_front) = camera.project(e.geodetic_to_ecef(b_lat, b_lon, 0.0));
    if !(a_front && b_front) {
        return 0.0;
    }
    let angle = (b.y - a.y).atan2(b.x - a.x).to_degrees();
    let angle = (angle + 90.0).rem_euclid(180.0) - 90.0;
    ((angle / TURN_STEP).round() * TURN_STEP).to_radians() as f32
}

/// Рамка повёрнутой на turn подписи и привязка - её середина.
fn turned_box(st: Style, text: egui::Vec2, turn: f32) -> (f32, f32, f32, f32) {
    let (w, h, _, _) = layout_box(st, text);
    let (c, s) = (turn.cos().abs(), turn.sin().abs());
    let (rw, rh) = ((w * c + h * s).ceil() + 1.0, (w * s + h * c).ceil() + 1.0);
    (rw, rh, rw / 2.0, rh / 2.0)
}

/// Поставленные надписи chosen без лишних мелких, `_thin` модуля: не
/// больше [`kind_cap`] на класс в долю share площади кадра, река одного
/// названия один раз.
pub fn thin(chosen: Vec<usize>, places: &[&Place], share: f32) -> Vec<usize> {
    let mut counts: HashMap<Kind, usize> = HashMap::new();
    let mut rivers: HashSet<&str> = HashSet::new();
    chosen
        .into_iter()
        .filter(|&n| {
            let Some(place) = places.get(n) else {
                return false;
            };
            let Some(cap) = kind_cap(place.kind) else {
                return true;
            };
            let count = counts.entry(place.kind).or_default();
            if *count >= ((cap as f32 * share).round() as usize).max(1) {
                return false;
            }
            if place.kind == Kind::River && !rivers.insert(place.name.as_str()) {
                return false;
            }
            *count += 1;
            true
        })
        .collect()
}

/// Мягкая тень текста galley с левым верхним углом pos, повёрнутого
/// на turn вокруг pos.
fn shadow(painter: &egui::Painter, galley: &std::sync::Arc<egui::Galley>, pos: egui::Pos2, turn: f32, alpha: f32) {
    for (radius, share) in SHADOW_RINGS {
        let color = egui::Color32::from_black_alpha((255.0 * SHADOW_ALPHA * share * alpha).round() as u8);
        for k in 0..8 {
            let a = k as f32 * std::f32::consts::FRAC_PI_4;
            let at = pos + egui::vec2(a.cos(), a.sin()) * radius;
            painter.add(egui::epaint::TextShape::new(at, galley.clone(), color).with_override_text_color(color).with_angle(turn));
        }
    }
}

/// Нарисовать подпись реки серединой в anchor, повёрнутую на turn.
fn paint_turned(painter: &egui::Painter, st: Style, galley: std::sync::Arc<egui::Galley>, anchor: egui::Pos2, turn: f32, alpha: f32) {
    let half = galley.size() / 2.0;
    let (c, s) = (turn.cos(), turn.sin());
    let pos = anchor - egui::vec2(half.x * c - half.y * s, half.x * s + half.y * c);
    shadow(painter, &galley, pos, turn, alpha);
    let color = fade(st.color, alpha);
    painter.add(egui::epaint::TextShape::new(pos, galley, color).with_override_text_color(color).with_angle(turn));
}

/// Размер надписи со значком и точка привязки в ней, как `_Style.layout`
/// модуля.
fn layout_box(st: Style, text: egui::Vec2) -> (f32, f32, f32, f32) {
    let halo = field(st);
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

/// Нарисовать надпись с левым верхним углом corner. mark - цвет значка
/// своей метки, без него - жёлтый [`MARK`].
fn paint(
    painter: &egui::Painter,
    st: Style,
    galley: std::sync::Arc<egui::Galley>,
    corner: egui::Pos2,
    text: &str,
    alpha: f32,
    mark: Option<egui::Color32>,
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
    let text_x = corner.x + field(st) + if st.marker == Marker::None { 0.0 } else { 2.0 * st.dot + 3.0 };
    let pos = egui::pos2(text_x, corner.y + (h - size.y) / 2.0);
    shadow(painter, &galley, pos, 0.0, alpha);
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
            painter.circle(anchor, r, fade(mark.unwrap_or(MARK), alpha), ring);
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
    fn base_labels_shrink_with_altitude() {
        // Как `label_scale` модуля: полный кегль до 300 км, 0.7 с 6000 км,
        // ступени по 0.1.
        assert_eq!(super::label_scale(100e3), 1.0);
        assert_eq!(super::label_scale(300e3), 1.0);
        assert_eq!(super::label_scale(6000e3), 0.7);
        assert_eq!(super::label_scale(20_000e3), 0.7);
        assert!((super::label_scale(2000e3) - 0.8).abs() < 1e-6);
        assert!(super::shrinks(Kind::Country) && !super::shrinks(Kind::Mark) && !super::shrinks(Kind::Layer));
    }

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
        // Населённые пункты без точек, ранг - кеглем, модуль 1.38.
        for kind in [Kind::Capital, Kind::City, Kind::Town, Kind::Village, Kind::Suburb] {
            assert_eq!(style(kind, false).marker, Marker::None, "{kind:?}");
        }
        assert!(super::caps(Kind::Suburb) && super::caps(Kind::State) && !super::caps(Kind::City));
    }

    #[test]
    fn thin_caps_small_kinds_and_rivers_once() {
        // Сторож `_thin` модуля: районов не больше 6 на кадр 1920 × 1080,
        // река одного названия один раз, города без предела.
        use data::places::Place;
        let mut all: Vec<Place> = (0..10).map(|n| Place::new(n, &format!("Район {n}"), Kind::Suburb, 1, 0.0, 0.0)).collect();
        all.extend((0..3).map(|n| Place::new(100 + n, "Кама", Kind::River, 1, 0.0, 0.0)));
        all.extend((0..20).map(|n| Place::new(200 + n, &format!("Город {n}"), Kind::City, 1, 0.0, 0.0)));
        let refs: Vec<&Place> = all.iter().collect();
        let out = super::thin((0..refs.len()).collect(), &refs, 1.0);
        let count = |k: Kind| out.iter().filter(|&&n| refs[n].kind == k).count();
        assert_eq!((count(Kind::Suburb), count(Kind::River), count(Kind::City)), (6, 1, 20));
        // Кадр вчетверо меньше - районов вчетверо меньше, но не меньше одного.
        let small = super::thin((0..refs.len()).collect(), &refs, 0.25);
        assert_eq!(small.iter().filter(|&&n| refs[n].kind == Kind::Suburb).count(), 2);
        let tiny = super::thin((0..refs.len()).collect(), &refs, 0.01);
        assert_eq!(tiny.iter().filter(|&&n| refs[n].kind == Kind::Suburb).count(), 1);
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