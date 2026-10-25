// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Живые объекты: суда Балтики, самолёты, поезда Финляндии. Перенос
//! `LiveManager` (`ui/live.py`) и строки поездов (`ui/trains.py`) модуля.
//!
//! Служба опрашивается раз в poll секунд. Издалека объект - точка цвета
//! группы с подписью, не больше [`MAX_POINTS`] ближайших к точке
//! взгляда. Вблизи (вид уже model_view) - модель, которая идёт своим
//! курсом между ответами. Самолёт в воздухе стоит на своей высоте, судно,
//! поезд и борт на земле - на поверхности. Камера следом держит объект
//! в середине вида с азимутом по его курсу (`data::movers::chase_on`).
//!
//! Нет пока: рельсов OSM у поездов (поезд идёт своим курсом, а не по
//! рельсам), записи облёта по своим часам.

use std::collections::{BTreeMap, HashMap};

use data::movers::{self, Movers};
use data::{aircraft, trains, vessels};
use globe_core::{DMat3, DVec3};

use crate::i18n::{Lang, tr, trf};

/// Точек издалека не больше, ближайших к точке взгляда.
pub const MAX_POINTS: usize = 600;
/// Точки - не дальше стольких полос вида, модели - стольких.
pub const POINT_REACH: f64 = 3.0;
pub const MODEL_REACH: f64 = 1.5;
/// Размер точки, логические пиксели.
pub const POINT_SIZE: f32 = 7.0;
/// Пикселей до объекта при щелчке.
pub const PICK_PIXELS: f64 = 10.0;
/// Длин объекта от глаза, ближе - модель без точки.
pub const SEEN: f64 = 150.0;
/// Секунд между пересборками точек.
pub const VIEW_TICK: f64 = 1.0;
/// Расстояние камеры следом до объекта при включении и пределы колеса,
/// `ui/chase.py` модуля.
pub const START_REACH: f64 = 1500.0;
pub const MAX_REACH: f64 = 2.0e7;
pub const WHEEL_STEP: f64 = 0.8;
/// Предел ответа службы.
pub const ANSWER_BYTES: u64 = 64 << 20;

/// Строка живых объектов.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Vessels,
    Aircraft,
    Trains,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Vessels, Kind::Aircraft, Kind::Trains];

    pub fn key(self) -> &'static str {
        match self {
            Kind::Vessels => "vessels",
            Kind::Aircraft => "aircraft",
            Kind::Trains => "trains",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Vessels => "Baltic ships",
            Kind::Aircraft => "Aircraft",
            Kind::Trains => "Trains of Finland",
        }
    }

    /// Подсказка строки каталога, `gallery_items` модуля.
    pub fn tip(self) -> &'static str {
        match self {
            Kind::Vessels => "Ships of the Gulfs of Finland and Bothnia and the Baltic near Finland by AIS signals, Fintraffic Digitraffic open data, the positions every 20 s. From afar a ship is a dot in the colour of its type with its name, close up a schematic model of its length that keeps its course. A click shows the type, call sign, destination, size and speed.",
            Kind::Aircraft => "Aircraft by ADS-B signals of the adsb.lol volunteer network in a circle around the point of view, every 10 s. An aircraft in the air is shown at its altitude, close up as a schematic model of the size of its category with its bank. A click shows the call sign, type, registration, altitude and speed. The network has almost no receivers over Russia.",
            Kind::Trains => "Trains of Finland in real time from Fintraffic Digitraffic open data, the positions are updated every 10 s. From afar a train is a dot in the colour of its category with its number, close up it is a car model that moves along OpenStreetMap rails between updates. A click on a train shows its type, number and speed.",
        }
    }

    /// Источник второй строкой каталога.
    pub fn source(self) -> &'static str {
        match self {
            Kind::Vessels => "Fintraffic Digitraffic, AIS",
            Kind::Aircraft => "adsb.lol, ADS-B",
            Kind::Trains => "Fintraffic Digitraffic",
        }
    }

    pub fn attribution(self) -> (&'static str, &'static str) {
        match self {
            Kind::Vessels => vessels::ATTRIBUTION,
            Kind::Aircraft => aircraft::ATTRIBUTION,
            Kind::Trains => trains::ATTRIBUTION,
        }
    }

    /// Секунд между опросами.
    fn poll(self) -> f64 {
        match self {
            Kind::Vessels => vessels::POLL,
            Kind::Aircraft => aircraft::POLL,
            Kind::Trains => trains::POLL,
        }
    }

    /// Вид уже этого, метры, - модели.
    fn model_view(self) -> f64 {
        match self {
            Kind::Vessels => 20000.0,
            Kind::Aircraft => 15000.0,
            Kind::Trains => 10000.0,
        }
    }
}

/// Объекты строки.
enum Fleet {
    Vessels(Movers<i64, vessels::Info>),
    Aircraft(Movers<String, aircraft::Info>),
    Trains(trains::Fleet),
}

/// Объект на момент: ключ, место, азимут (NaN - нет), высота над уровнем
/// моря (NaN - на поверхности), тангаж и крен, группа, подпись, размеры
/// модели (ширина, длина, высота) и цвет.
#[derive(Debug, Clone)]
pub struct Object {
    pub key: String,
    pub lat: f64,
    pub lon: f64,
    pub heading: f64,
    pub alt: f64,
    pub pitch: f64,
    pub roll: f64,
    pub group: String,
    pub label: String,
    pub size: (f64, f64, f64),
    pub color: [f64; 3],
}

/// Модель группы для видеокарты: части в осях `frame(0, 0)` и матрица.
struct Unit {
    parts: Vec<render::ModelPart>,
}

/// Строка живых объектов на глобусе.
pub struct Live {
    pub kind: Kind,
    fleet: Fleet,
    ask: Option<Box<dyn net::Request>>,
    info_ask: Option<Box<dyn net::Request>>,
    asked_at: f64,
    info_at: f64,
    pub error: String,
    /// Объект, за которым идёт камера, и расстояние до него.
    pub followed: Option<String>,
    pub reach: f64,
    /// Точки собрать заново.
    pub dirty: bool,
    built_at: f64,
    /// Ключи моделей в видеокарте по порядку.
    pub model_keys: Vec<String>,
    units: HashMap<String, Unit>,
}

pub fn rgba(c: [f64; 3]) -> [u8; 3] {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn when(t: f64) -> String {
    scene::when::text(t)
}

/// Секунды UTC сейчас.
pub fn clock() -> f64 {
    web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

impl Live {
    pub fn new(kind: Kind) -> Live {
        let fleet = match kind {
            Kind::Vessels => Fleet::Vessels(Movers::new(vessels::EXTRAPOLATE, vessels::STALE)),
            Kind::Aircraft => Fleet::Aircraft(Movers::new(aircraft::EXTRAPOLATE, aircraft::STALE)),
            Kind::Trains => Fleet::Trains(trains::Fleet::default()),
        };
        Live {
            kind,
            fleet,
            ask: None,
            info_ask: None,
            asked_at: f64::NEG_INFINITY,
            info_at: f64::NEG_INFINITY,
            error: String::new(),
            followed: None,
            reach: START_REACH,
            dirty: true,
            built_at: f64::NEG_INFINITY,
            model_keys: Vec::new(),
            units: HashMap::new(),
        }
    }

    /// Объектов в строке.
    pub fn count(&self) -> usize {
        match &self.fleet {
            Fleet::Vessels(m) => m.len(),
            Fleet::Aircraft(m) => m.len(),
            Fleet::Trains(f) => f.trains.len(),
        }
    }

    /// Ответ ещё не пришёл.
    pub fn busy(&self) -> bool {
        self.ask.is_some()
    }

    /// Шаг кадра: опрос службы и ответы. now - секунды часов окна,
    /// view - точка взгляда и ширина вида.
    pub fn step(&mut self, now: f64, view: (f64, f64, f64), request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        let clock = clock();
        if self.ask.is_none() && now - self.asked_at >= self.kind.poll() {
            self.asked_at = now;
            let ask = request(ANSWER_BYTES);
            match self.kind {
                Kind::Vessels => ask.send(vessels::LOCATIONS_URL, &vessels::HEADERS, None),
                Kind::Aircraft => ask.start(&aircraft::point_url(view.0, view.1, (2.0 * view.2).max(20000.0))),
                Kind::Trains => ask.send(trains::LATEST_URL, &trains::HEADERS, None),
            }
            self.ask = Some(ask);
        }
        let info_poll = match self.kind {
            Kind::Vessels => Some(vessels::INFO_POLL),
            Kind::Trains => Some(trains::TYPES_POLL),
            Kind::Aircraft => None,
        };
        if let Some(poll) = info_poll
            && self.info_ask.is_none()
            && now - self.info_at >= poll
        {
            self.info_at = now;
            let ask = request(ANSWER_BYTES);
            match self.kind {
                Kind::Vessels => ask.send(vessels::VESSELS_URL, &vessels::HEADERS, None),
                _ => {
                    let body = trains::types_body();
                    ask.send(trains::GRAPHQL_URL, &trains::GRAPHQL_HEADERS, Some(body.as_bytes()));
                }
            }
            self.info_ask = Some(ask);
        }
        if let Some(answer) = self.ask.as_ref().and_then(|a| a.take()) {
            self.ask = None;
            match answer {
                Ok(bytes) => {
                    self.error.clear();
                    match &mut self.fleet {
                        Fleet::Vessels(m) => m.update(vessels::parse_locations(&bytes), now, clock, false),
                        Fleet::Aircraft(m) => {
                            let (samples, info) = aircraft::parse(&bytes);
                            m.info.extend(info);
                            m.update(samples, now, clock, true);
                        }
                        Fleet::Trains(f) => f.update(trains::parse_latest(&bytes), now, clock),
                    }
                    self.dirty = true;
                }
                Err(e) => self.error = e,
            }
        }
        if let Some(answer) = self.info_ask.as_ref().and_then(|a| a.take()) {
            self.info_ask = None;
            if let Ok(bytes) = answer {
                match &mut self.fleet {
                    Fleet::Vessels(m) => m.info.extend(vessels::parse_vessels(&bytes)),
                    Fleet::Trains(f) => f.info.extend(trains::parse_types(&bytes)),
                    Fleet::Aircraft(_) => {}
                }
                self.units.clear();
                self.model_keys.clear();
                self.dirty = true;
            }
        }
        if now - self.built_at >= VIEW_TICK {
            self.dirty = true;
        }
    }

    /// Точки собраны в этот момент.
    pub fn built(&mut self, now: f64) {
        self.built_at = now;
        self.dirty = false;
    }

    /// Объекты на момент.
    pub fn objects(&mut self, now: f64) -> Vec<Object> {
        let clock = clock();
        match &mut self.fleet {
            Fleet::Vessels(m) => {
                let shown = m.positions(now, clock);
                shown
                    .into_iter()
                    .map(|(key, lat, lon, heading, alt)| {
                        let info = m.info.get(&key);
                        let group = info.map_or("other", vessels::Info::group);
                        Object { key: key.to_string(), lat, lon, heading, alt, pitch: 0.0, roll: 0.0, group: group.to_string(), label: vessels::label(key, info), size: vessels::ship_size(info), color: vessels::group_color(group) }
                    })
                    .collect()
            }
            Fleet::Aircraft(m) => {
                let shown = m.positions(now, clock);
                shown
                    .into_iter()
                    .map(|(key, lat, lon, heading, alt)| {
                        let info = m.info.get(&key);
                        let group = info.map_or("other", aircraft::Info::group);
                        let (pitch, roll) = m.attitude(&key).unwrap_or((0.0, 0.0));
                        let label = aircraft::label(&key, info);
                        Object { key, lat, lon, heading, alt, pitch, roll, group: group.to_string(), label, size: aircraft::plane_size(info.map_or("", |i| i.category.as_str())), color: aircraft::group_color(group) }
                    })
                    .collect()
            }
            Fleet::Trains(f) => {
                let shown = f.positions(now, clock);
                shown
                    .into_iter()
                    .map(|(key, lat, lon, heading)| {
                        let info = f.info.get(&key);
                        let category = info.map_or("", |i| i.category.as_str()).to_string();
                        let cars = trains::cars(&category) as f64;
                        Object {
                            key: format!("{}|{}", key.0, key.1),
                            lat,
                            lon,
                            heading,
                            alt: f64::NAN,
                            pitch: 0.0,
                            roll: 0.0,
                            label: trains::label(&key, info),
                            color: trains::category_color(&category),
                            group: category,
                            size: (3.2, 26.0 * cars.max(1.0), 4.3),
                        }
                    })
                    .collect()
            }
        }
    }

    /// Поля окна «Объекты» объекта key.
    pub fn values(&self, lang: Lang, key: &str) -> Vec<(String, String)> {
        let row = |a: &'static str, b: String| (tr(lang, a).to_string(), b);
        let mut out = Vec::new();
        match &self.fleet {
            Fleet::Vessels(m) => {
                let Ok(k) = key.parse::<i64>() else {
                    return out;
                };
                let Some(item) = m.items.get(&k) else {
                    return out;
                };
                let s = &item.sample;
                if let Some(i) = m.info.get(&k) {
                    if !i.call.is_empty() {
                        out.push(row("Call sign", i.call.clone()));
                    }
                    if i.imo > 0 {
                        out.push(row("IMO", i.imo.to_string()));
                    }
                    if !i.destination.is_empty() {
                        out.push(row("Destination", i.destination.clone()));
                    }
                    if i.length > 0.0 {
                        out.push(row("Size", trf(lang, "{length} × {width} m", &[("length", &format!("{:.0}", i.length)), ("width", &format!("{:.0}", i.width))])));
                    }
                }
                out.push(row("MMSI", k.to_string()));
                out.push(row("Speed", trf(lang, "{value} kn", &[("value", &format!("{:.1}", s.speed / movers::KNOT))])));
                if s.course.is_finite() {
                    out.push(row("Course", format!("{:.0}°", s.course)));
                }
                out.push(row("Position, UTC", when(s.t)));
            }
            Fleet::Aircraft(m) => {
                let Some(item) = m.items.get(key) else {
                    return out;
                };
                let s = &item.sample;
                if let Some(i) = m.info.get(key) {
                    if !i.kind.is_empty() {
                        out.push(row("Type", i.kind.clone()));
                    }
                    if !i.reg.is_empty() {
                        out.push(row("Registration", i.reg.clone()));
                    }
                }
                out.push(row("ICAO code", key.to_uppercase()));
                let alt = if s.alt.is_finite() {
                    trf(lang, "{metres} m ({feet} ft)", &[("metres", &format!("{:.0}", s.alt)), ("feet", &format!("{:.0}", s.alt / movers::FOOT))])
                } else {
                    tr(lang, "on the ground").to_string()
                };
                out.push(row("Altitude", alt));
                out.push(row("Speed", trf(lang, "{value} km/h", &[("value", &format!("{:.0}", s.speed * 3.6))])));
                if s.course.is_finite() {
                    out.push(row("Course", format!("{:.0}°", s.course)));
                }
                out.push(row("Position, UTC", when(s.t)));
            }
            Fleet::Trains(f) => {
                let Some((number, date)) = key.split_once('|') else {
                    return out;
                };
                let Ok(number) = number.parse::<i64>() else {
                    return out;
                };
                let k: trains::TrainKey = (number, date.to_string());
                let Some(train) = f.trains.get(&k) else {
                    return out;
                };
                if let Some(i) = f.info.get(&k) {
                    if !i.kind.is_empty() {
                        out.push(row("Type", i.kind.clone()));
                    }
                    if !i.operator.is_empty() {
                        out.push(row("Operator", i.operator.clone()));
                    }
                }
                out.push(row("Number", number.to_string()));
                out.push(row("Speed", trf(lang, "{value} km/h", &[("value", &format!("{:.0}", train.sample.speed))])));
                out.push(row("Position, UTC", when(train.sample.t)));
            }
        }
        out
    }

    /// Модель группы: части в осях `frame(0, 0)`.
    fn unit(&mut self, group: &str, color: [f64; 3]) -> &Unit {
        let kind = self.kind;
        self.units.entry(group.to_string()).or_insert_with(|| {
            let parts = unit_parts(kind, color);
            let base = base();
            let parts = parts
                .into_iter()
                .filter(|p| p.positions.len() >= 3)
                .map(|p| {
                    let pos: Vec<DVec3> = p.positions.iter().map(|v| DVec3::from_array(*v)).collect();
                    let normals = scene::models::flat_normals(&pos);
                    let f = |v: DVec3| [v.x as f32, v.y as f32, v.z as f32];
                    render::ModelPart {
                        positions: pos.iter().map(|v| f(base * *v)).collect(),
                        normals: normals.iter().map(|n| f((base * *n).normalize_or_zero())).collect(),
                        uv: Vec::new(),
                        image: None,
                        rgba: p.rgba.map(|c| c as f32),
                        track: None,
                    }
                })
                .collect();
            Unit { parts }
        })
    }

    /// Модели объектов objects для видеокарты: части и место.
    pub fn meshes(&mut self, objects: &[&Object]) -> Vec<render::ModelMesh> {
        objects
            .iter()
            .map(|o| render::ModelMesh { center: DVec3::ZERO, parts: self.unit(&o.group, o.color).parts.clone(), images: Vec::new(), sun: [0.0, 0.0, 1.0], orient: DMat3::IDENTITY })
            .collect()
    }

    /// Колесо в камере следом: расстояние до объекта.
    pub fn wheel(&mut self, notches: f64) {
        self.reach = (self.reach * WHEEL_STEP.powf(notches)).clamp(movers::CHASE_MIN, MAX_REACH);
    }
}

/// Части модели единичного размера: размер объекта в метрах растягивает
/// их в [`orient`]. Вагон `train_parts` - в метрах, он делится на свои
/// размеры: иначе поезд из пяти вагонов выходил длиной больше 3 км,
/// 25 м вагона на 130 м масштаба.
fn unit_parts(kind: Kind, color: [f64; 3]) -> Vec<data::trains::Part> {
    match kind {
        Kind::Vessels => vessels::unit_ship_parts(color),
        Kind::Aircraft => aircraft::unit_plane_parts(color),
        Kind::Trains => {
            let scale = [trains::CAR_WIDTH, trains::CAR_LENGTH, trains::CAR_HEIGHT];
            let mut parts = trains::train_parts(color, true);
            for part in &mut parts {
                for v in &mut part.positions {
                    for (c, s) in v.iter_mut().zip(scale) {
                        *c /= s;
                    }
                }
            }
            parts
        }
    }
}

/// Матрица осей модели `frame(0, 0)` модуля.
pub fn base() -> DMat3 {
    scene::models::frame(0.0, 0.0, 0.0, 0.0, 0.0, [1.0, 1.0, 1.0])
}

/// Поворот модели объекта на его месте.
pub fn orient(o: &Object) -> DMat3 {
    let heading = if o.heading.is_finite() { o.heading } else { 0.0 };
    scene::models::moved(&base(), o.lat, o.lon, heading, o.pitch, o.roll, [o.size.0, o.size.1, o.size.2])
}

/// Объекты для точек издалека: в пределах POINT_REACH полос вида от
/// точки взгляда, кроме объекта под камерой следом и моделей, ближайшие
/// первыми, не больше MAX_POINTS.
pub fn nearest<'a>(objects: &'a [Object], view: (f64, f64, f64), skip: &dyn Fn(&Object) -> bool) -> Vec<&'a Object> {
    let reach = POINT_REACH * view.2;
    let mut found: Vec<(f64, &Object)> = objects.iter().map(|o| (trains::metres(view.0, view.1, o.lat, o.lon), o)).filter(|(gap, o)| *gap <= reach && !skip(o)).collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.into_iter().take(MAX_POINTS).map(|(_, o)| o).collect()
}

/// Ключи моделей: объекты в пределах MODEL_REACH полос вида, когда вид
/// уже model_view, и объект под камерой следом.
pub fn model_keys(kind: Kind, objects: &[Object], view: (f64, f64, f64), followed: Option<&str>) -> Vec<String> {
    let mut keys: Vec<String> = if view.2 < kind.model_view() {
        objects.iter().filter(|o| trains::metres(view.0, view.1, o.lat, o.lon) <= MODEL_REACH * view.2).map(|o| o.key.clone()).collect()
    } else {
        Vec::new()
    };
    if let Some(f) = followed
        && objects.iter().any(|o| o.key == f)
        && !keys.iter().any(|k| k == f)
    {
        keys.push(f.to_string());
    }
    keys.sort();
    keys
}

/// Вблизи ли вид: модели видны.
pub fn near(kind: Kind, width: f64) -> bool {
    width < kind.model_view()
}

/// Объекты по ключам для быстрого поиска.
pub fn by_key(objects: &[Object]) -> BTreeMap<&str, &Object> {
    objects.iter().map(|o| (o.key.as_str(), o)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(std::cell::RefCell<Option<Result<Vec<u8>, String>>>, net::Stats);

    impl net::Request for Fixed {
        fn start(&self, _url: &str) {}
        fn take(&self) -> Option<Result<Vec<u8>, String>> {
            self.0.borrow_mut().take()
        }
        fn stats(&self) -> &net::Stats {
            &self.1
        }
    }

    #[test]
    fn unit_models_are_one_long() {
        // Модели всех видов - длиной 1 по оси Y, размер в метрах их
        // растягивает. Вагон в метрах давал поезд длиной в километры,
        // замечено по демо поездов 25 октября 2026 года.
        for kind in Kind::ALL {
            let ys: Vec<f64> = unit_parts(kind, [1.0, 0.0, 0.0]).iter().flat_map(|p| p.positions.iter().map(|v| v[1])).collect();
            let (lo, hi) = ys.iter().fold((f64::MAX, f64::MIN), |(lo, hi), y| (lo.min(*y), hi.max(*y)));
            assert!((hi - lo - 1.0).abs() < 0.05, "{kind:?}: {}", hi - lo);
        }
    }

    #[test]
    fn aircraft_answer_gives_objects_and_points() {
        let body = r#"{"ac":[{"hex":"abc123","lat":60.1,"lon":24.9,"gs":250,"track":90,"alt_baro":10000,"t":"A320","r":"OH-LXA","flight":"FIN1  ","category":"A3"}],"now":1800000000000}"#.to_string();
        let ask = move |_: u64| Box::new(Fixed(std::cell::RefCell::new(Some(Ok(body.clone().into_bytes()))), net::Stats::default())) as Box<dyn net::Request>;
        let mut live = Live::new(Kind::Aircraft);
        live.step(0.0, (60.0, 25.0, 50000.0), &ask);
        live.step(0.1, (60.0, 25.0, 50000.0), &ask);
        assert_eq!(live.count(), 1, "{}", live.error);
        let objects = live.objects(0.2);
        assert_eq!(objects.len(), 1);
        assert!(objects[0].alt > 3000.0);
        let points = nearest(&objects, (60.0, 25.0, 50000.0), &|_| false);
        assert_eq!(points.len(), 1);
        assert!(!near(Kind::Aircraft, 50000.0));
        assert_eq!(model_keys(Kind::Aircraft, &objects, (60.1, 24.9, 5000.0), None), vec!["abc123".to_string()]);
        let meshes = live.meshes(&[&objects[0]]);
        assert!(!meshes[0].parts.is_empty());
        assert!(!live.values(Lang::EN, "abc123").is_empty());
        live.wheel(100.0);
        assert!(live.reach >= movers::CHASE_MIN);
    }
}
