// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Пункты из векторных тайлов для надписей на глобусе, перенос
//! `core/places.py` модуля.
//!
//! Точки берутся из слоёв схемы OpenMapTiles, их отдаёт OpenFreeMap.
//! Надписи стоят на экране ровно, как в Google Earth. Какие из них
//! видны, решает [`select_labels`]: надписи идут по важности, каждая
//! следующая ставится, только если не задевает уже поставленные.

use std::collections::{HashMap, HashSet};

use crate::mvt::{self, DecodeError, Geometry, Value};

/// Последний уровень векторных тайлов OpenFreeMap.
pub const MAX_LEVEL: u32 = 14;
/// Уровень тайла надписей на столько грубее тайла подложки. Тайл схемы
/// OpenMapTiles рассчитан на 512 пикселей, тайл подложки - 256. Ещё
/// уровень снимает три четверти запросов, а тайл уровня z - 2 несёт
/// пункты, которые помещаются на карте этого масштаба.
pub const LEVEL_OFFSET: u32 = 2;

/// Класс пункта, по порядку важности. Внутри класса важнее меньший
/// ранг. Порядок - как в модуле, классы Lite без своих объектов
/// (метки, линейка, сетка и другие) стоят на своих местах заранее.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Search,
    Mark,
    Ruler,
    Layer,
    Circle,
    Grid,
    Plate,
    Value,
    Country,
    Capital,
    City,
    State,
    Water,
    Town,
    Park,
    Peak,
    Airport,
    RoadRef,
    Village,
    Wiki,
}

impl Kind {
    pub const ALL: [Kind; 20] = [
        Kind::Search,
        Kind::Mark,
        Kind::Ruler,
        Kind::Layer,
        Kind::Circle,
        Kind::Grid,
        Kind::Plate,
        Kind::Value,
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
        Kind::Wiki,
    ];

    /// Место в очереди важности.
    pub fn tier(self) -> usize {
        self as usize
    }

    /// Наименьший уровень тайла пунктов, из которого класс подписывается.
    /// Области в тайлах уровней 0-2 есть, но из космоса их названия
    /// заслоняли страны, 26 сентября 2026 года.
    pub fn min_level(self) -> u32 {
        match self {
            Kind::State => 3,
            _ => 0,
        }
    }

    /// Наибольшая высота глаза над эллипсоидом в метрах, до которой
    /// класс подписывается. Величины назначил автор модуля 27 сентября
    /// 2026 года.
    pub fn max_height(self) -> Option<f64> {
        match self {
            Kind::Peak => Some(4.0e5),
            Kind::Airport | Kind::RoadRef => Some(1.0e6),
            Kind::Park => Some(3.0e6),
            _ => None,
        }
    }
}

/// Классы из kinds, которые подписываются с высоты height, в метрах.
pub fn kinds_at(kinds: &HashSet<Kind>, height: f64) -> HashSet<Kind> {
    kinds.iter().copied().filter(|k| k.max_height().is_none_or(|m| height <= m)).collect()
}

/// Подписываемый пункт. id - номер объекта в тайле или 0. rank - ранг
/// OpenMapTiles, меньше - важнее, без ранга 99. lat, lon в градусах.
/// info - высота вершины в метрах, у остальных None.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub id: u64,
    pub name: String,
    pub kind: Kind,
    pub rank: i64,
    pub lat: f64,
    pub lon: f64,
    pub info: Option<i64>,
}

impl Place {
    pub fn new(id: u64, name: &str, kind: Kind, rank: i64, lat: f64, lon: f64) -> Self {
        Self { id, name: name.to_string(), kind, rank, lat, lon, info: None }
    }
}

/// Слои векторного тайла с пунктами.
pub const LAYERS: [&str; 6] = ["place", "mountain_peak", "water_name", "park", "aerodrome_label", "transportation_name"];
/// Нужные атрибуты, кроме названий на языках.
const WANTED: [&str; 6] = ["class", "capital", "rank", "name", "ele", "ref"];
/// Слои, где пункт берётся и с линии. В остальных только точки.
const LINE_LAYERS: [&str; 2] = ["water_name", "transportation_name"];

fn water_rank(class: &str) -> Option<i64> {
    match class {
        "ocean" => Some(1),
        "sea" => Some(2),
        "bay" | "strait" => Some(4),
        "lake" => Some(5),
        _ => None,
    }
}

const PARK_CLASSES: [&str; 4] = ["national_park", "nature_reserve", "protected_area", "wildlife_refuge"];

fn airport_rank(class: &str) -> i64 {
    match class {
        "international" => 1,
        "public" => 2,
        "regional" => 3,
        "military" => 4,
        _ => 5,
    }
}

fn road_rank(class: &str) -> Option<i64> {
    match class {
        "motorway" => Some(1),
        "trunk" => Some(2),
        "primary" => Some(3),
        _ => None,
    }
}

type Attrs = HashMap<String, Value>;

fn text<'a>(attrs: &'a Attrs, key: &str) -> Option<&'a str> {
    attrs.get(key).and_then(Value::as_str)
}

/// Класс населённого пункта или None, если пункт не показывается.
pub fn place_kind(attrs: &Attrs) -> Option<Kind> {
    let class = text(attrs, "class")?;
    let capital = match attrs.get("capital") {
        Some(Value::Int(2)) => true,
        Some(Value::Float(v)) => *v == 2.0,
        Some(Value::Str(s)) => s == "2",
        _ => false,
    };
    match class {
        "city" if capital => Some(Kind::Capital),
        "province" | "state" => Some(Kind::State),
        "country" => Some(Kind::Country),
        "capital" => Some(Kind::Capital),
        "city" => Some(Kind::City),
        "town" => Some(Kind::Town),
        "village" => Some(Kind::Village),
        _ => None,
    }
}

/// Языки подписей, коды полей name:xx OpenMapTiles. Состав назначил
/// автор модуля 27 сентября 2026 года.
pub const LABEL_LANGUAGES: [&str; 15] =
    ["ru", "en", "de", "fr", "es", "it", "pt", "pl", "uk", "kk", "tr", "ar", "zh", "ja", "ko"];
/// Выбор языка: язык интерфейса или местные названия.
pub const AS_INTERFACE: &str = "";
pub const LOCAL: &str = "local";
/// Языки на латинице. Без названия на выбранном языке у них ставится
/// название латиницей, у остальных - местное.
const LATIN_SCRIPT: [&str; 8] = ["en", "de", "fr", "es", "it", "pt", "pl", "tr"];

/// Языки названий по порядку поиска для [`place_name`]. choice - код
/// из [`LABEL_LANGUAGES`], [`AS_INTERFACE`] или [`LOCAL`]. Язык
/// интерфейса не из списка даёт английский. Пустой набор - местные
/// названия.
pub fn name_languages(choice: &str, ui_language: &str) -> Vec<String> {
    if choice == LOCAL {
        return Vec::new();
    }
    let code = if choice.is_empty() { ui_language } else { choice };
    let code = if LABEL_LANGUAGES.contains(&code) { code } else { "en" };
    if LATIN_SCRIPT.contains(&code) { vec![code.to_string(), "latin".to_string()] } else { vec![code.to_string()] }
}

/// Название на первом найденном языке из languages или основное.
pub fn place_name(attrs: &Attrs, languages: &[String]) -> Option<String> {
    for language in languages {
        for field in [format!("name:{language}"), format!("name_{language}")] {
            if let Some(v) = text(attrs, &field).filter(|v| !v.is_empty()) {
                return Some(v.to_string());
            }
        }
    }
    text(attrs, "name").filter(|v| !v.is_empty()).map(str::to_string)
}

/// Широта и долгота точки тайла в единицах слоя.
pub fn tile_point(z: u32, x: u32, y: u32, px: f64, py: f64, extent: u32) -> (f64, f64) {
    let n = f64::from(1u32 << z.min(31));
    let u = (f64::from(x) + px / f64::from(extent)) / n;
    let v = (f64::from(y) + py / f64::from(extent)) / n;
    let lon = u * 360.0 - 180.0;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * v)).sinh().atan().to_degrees();
    (lat, lon)
}

fn rank(attrs: &Attrs) -> i64 {
    attrs.get("rank").and_then(Value::as_f64).map_or(99, |r| r as i64)
}

/// Класс, название, ранг и сведения пункта из объекта слоя. None -
/// объект не подписывается.
fn read_place(layer: &str, attrs: &Attrs, languages: &[String]) -> Option<(Kind, Option<String>, i64, Option<i64>)> {
    let class = text(attrs, "class").unwrap_or("");
    match layer {
        "place" => place_kind(attrs).map(|k| (k, place_name(attrs, languages), rank(attrs), None)),
        "mountain_peak" => {
            let ele = attrs.get("ele").and_then(Value::as_f64).map(|e| e as i64);
            Some((Kind::Peak, place_name(attrs, languages), rank(attrs), ele))
        }
        "water_name" => water_rank(class).map(|r| (Kind::Water, place_name(attrs, languages), r, None)),
        "park" => {
            PARK_CLASSES.contains(&class).then(|| (Kind::Park, place_name(attrs, languages), rank(attrs), None))
        }
        "aerodrome_label" => Some((Kind::Airport, place_name(attrs, languages), airport_rank(class), None)),
        "transportation_name" => {
            let reference = text(attrs, "ref").unwrap_or("").split(';').next().unwrap_or("").trim().to_string();
            let r = road_rank(class)?;
            (!reference.is_empty()).then_some((Kind::RoadRef, Some(reference), r, None))
        }
        _ => None,
    }
}

/// Пункты векторного тайла key = (z, x, y). У линии, например номера
/// дороги или названия водохранилища, пункт стоит на средней вершине.
/// Порядок - как в тайле: слои по порядку, объекты по порядку.
pub fn decode_places(key: (u32, u32, u32), data: &[u8], languages: &[String]) -> Result<Vec<Place>, DecodeError> {
    let (z, x, y) = key;
    let mut wanted: HashSet<String> = WANTED.iter().map(|s| s.to_string()).collect();
    for language in languages {
        wanted.insert(format!("name:{language}"));
        wanted.insert(format!("name_{language}"));
    }
    let names: HashSet<&str> = LAYERS.into_iter().collect();
    let layers = mvt::decode_layers(data, &names, Some(&wanted))?;
    let mut ordered: Vec<(&String, &mvt::Layer)> = layers.iter().collect();
    ordered.sort_by_key(|(name, _)| LAYERS.iter().position(|l| l == name));
    let mut out = Vec::new();
    for (layer, content) in ordered {
        for f in &content.features {
            let line = f.geometry == Geometry::Line;
            if line && !LINE_LAYERS.contains(&layer.as_str()) {
                continue;
            }
            let Some((kind, Some(name), rank, info)) = read_place(layer, &f.attrs, languages) else {
                continue;
            };
            let point = if line { f.points.get(f.points.len() / 2) } else { f.points.first() };
            let Some(&(px, py)) = point else {
                continue;
            };
            // Точки в полосе за краем тайла повторяют соседей.
            let extent = i64::from(content.extent);
            if !(0..extent).contains(&px) || !(0..extent).contains(&py) {
                continue;
            }
            let (lat, lon) = tile_point(z, x, y, px as f64, py as f64, content.extent);
            out.push(Place { id: f.id, name, kind, rank, lat, lon, info });
        }
    }
    Ok(out)
}

/// Уровень тайла надписей для тайла подложки уровня z.
pub fn label_level(z: u32, max_level: u32) -> u32 {
    z.saturating_sub(LEVEL_OFFSET).min(max_level)
}

/// Ключ сортировки: сначала важные.
pub fn importance(place: &Place) -> (usize, i64, &str) {
    (place.kind.tier(), place.rank, place.name.as_str())
}

/// Один и тот же пункт в тайлах разных уровней. Номер объекта
/// в OpenMapTiles - номер точки OSM, он один на всех уровнях.
pub fn identity(place: &Place) -> (u64, String) {
    (place.id, place.name.clone())
}

type Key = (u32, u32, u32);

/// Запомненный список пунктов: набор тайлов и классы, по которым он
/// собран, и сам список.
type Collected = ((Vec<Key>, Option<Vec<Kind>>), std::sync::Arc<Vec<Place>>);

/// Загруженные тайлы пунктов и пункты для кадра.
pub struct PlaceStore {
    pub max_level: u32,
    tiles: HashMap<Key, Vec<Place>>,
    collected: Option<Collected>,
}

impl Default for PlaceStore {
    fn default() -> Self {
        Self::new(MAX_LEVEL)
    }
}

impl PlaceStore {
    pub fn new(max_level: u32) -> Self {
        Self { max_level, tiles: HashMap::new(), collected: None }
    }

    pub fn add(&mut self, key: Key, places: Vec<Place>) {
        self.tiles.insert(key, places);
        self.collected = None;
    }

    pub fn has(&self, key: Key) -> bool {
        self.tiles.contains_key(&key)
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    fn key(&self, key: Key) -> Key {
        let (z, x, y) = key;
        let level = label_level(z, self.max_level);
        let shift = z - level;
        (level, x >> shift, y >> shift)
    }

    /// Ключи тайлов пунктов для тайлов подложки draw.
    pub fn wanted(&self, draw: &[Key]) -> HashSet<Key> {
        draw.iter().map(|k| self.key(*k)).collect()
    }

    /// Готовый тайл пунктов для тайла подложки key или его предок.
    pub fn source(&self, key: Key) -> Option<Key> {
        let (mut z, mut x, mut y) = self.key(key);
        loop {
            if self.tiles.contains_key(&(z, x, y)) {
                return Some((z, x, y));
            }
            if z == 0 {
                return None;
            }
            (z, x, y) = (z - 1, x >> 1, y >> 1);
        }
    }

    /// Пункты для тайлов draw по порядку важности, без повторов. kinds -
    /// нужные классы, None - все. Из тайлов разных уровней пункт берётся
    /// из самого точного. Список помнится, пока набор тайлов пунктов
    /// и классы те же.
    pub fn collect(&mut self, draw: &[Key], kinds: Option<&HashSet<Kind>>) -> std::sync::Arc<Vec<Place>> {
        let mut sources: Vec<Key> = draw.iter().filter_map(|k| self.source(*k)).collect();
        sources.sort_unstable();
        sources.dedup();
        let kinds_key = kinds.map(|k| {
            let mut v: Vec<Kind> = k.iter().copied().collect();
            v.sort_unstable();
            v
        });
        let memo = (sources, kinds_key);
        if let Some((m, out)) = &self.collected
            && *m == memo
        {
            return out.clone();
        }
        let mut seen: HashMap<(u64, String), Place> = HashMap::new();
        for key in memo.0.iter().rev() {
            for place in self.tiles.get(key).into_iter().flatten() {
                if key.0 < place.kind.min_level() || kinds.is_some_and(|k| !k.contains(&place.kind)) {
                    continue;
                }
                seen.entry(identity(place)).or_insert_with(|| place.clone());
            }
        }
        let mut out: Vec<Place> = seen.into_values().collect();
        out.sort_by(|a, b| importance(a).cmp(&importance(b)));
        let out = std::sync::Arc::new(out);
        self.collected = Some((memo, out.clone()));
        out
    }

    /// Забыть тайлы вне keep, если их больше limit.
    pub fn trim(&mut self, keep: &HashSet<Key>, limit: usize) {
        if self.tiles.len() > limit {
            self.tiles.retain(|k, _| keep.contains(k));
            self.collected = None;
        }
    }
}

/// Прямоугольник надписи в пикселях: x0, y0, x1, y1.
pub type LabelBox = [f64; 4];

/// Надписи без наложений, жадно по порядку boxes. boxes уже по порядку
/// важности, tiers - место класса каждой надписи в очереди, previous -
/// номера надписей, видных в прошлом кадре. Внутри класса они ставятся
/// первыми, так надпись не мигает, когда соседняя того же класса
/// сдвигается на пиксель. Возвращает номера поставленных надписей
/// в порядке постановки. Проверка идёт по сетке ячеек cell пикселей.
pub fn select_labels(boxes: &[LabelBox], tiers: &[usize], previous: &HashSet<usize>, cell: f64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..boxes.len().min(tiers.len())).collect();
    order.sort_by_key(|&i| (tiers[i], !previous.contains(&i), i));
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let mut placed = Vec::new();
    for i in order {
        let [x0, y0, x1, y1] = boxes[i];
        if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) {
            continue;
        }
        let cx = ((x0 / cell).floor() as i64)..=((x1 / cell).floor() as i64);
        let cy = ((y0 / cell).floor() as i64)..=((y1 / cell).floor() as i64);
        let cells: Vec<(i64, i64)> = cx.flat_map(|a| cy.clone().map(move |b| (a, b))).collect();
        let clash = cells.iter().any(|c| {
            grid.get(c).into_iter().flatten().any(|&j| {
                let [a0, b0, a1, b1] = boxes[j];
                x0 < a1 && a0 < x1 && y0 < b1 && b0 < y1
            })
        });
        if clash {
            continue;
        }
        placed.push(i);
        for c in cells {
            grid.entry(c).or_default().push(i);
        }
    }
    placed
}
