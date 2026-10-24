// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Векторная основа OpenFreeMap: группы раздела «Слои», линии поверх
//! снимка и классы подписей, как `geo_tree` в `ui/panel.py` и наложение
//! `net/overlay.py` модуля.
//!
//! Векторные тайлы те же, что у подписей, и идут через тот же загрузчик.
//! Тайл разбирается один раз при приходе. Картинки линий тайлов кадра
//! рисуются с пределом времени на кадр: на странице нет потоков, а долгая
//! пауза кадра под Wayland закрывала окно программы.
//!
//! В ту же картинку тайла под линиями ложатся карты картинками
//! (crate::maps): своя картинка наложения у тайла одна, как картинка
//! стопки слоёв _overlay_stack модуля.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use data::lines::{self, Group};
use data::rastermaps;
use data::places::Kind;
use crate::maps::Kind as MapKind;
use globe_core::lod::TileKey;

/// Строка группы векторной основы, `VECTOR_GROUPS` модуля.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Row {
    Borders,
    Places,
    WaterNames,
    Roads,
    RoadRefs,
    Railways,
    Airports,
    Rivers,
    Water,
    Peaks,
    Parks,
}

impl Row {
    pub const ALL: [Row; 11] = [
        Row::Borders,
        Row::Places,
        Row::WaterNames,
        Row::Roads,
        Row::RoadRefs,
        Row::Railways,
        Row::Airports,
        Row::Rivers,
        Row::Water,
        Row::Peaks,
        Row::Parks,
    ];

    /// Ключ строки в настройках, как у модуля.
    pub fn key(self) -> &'static str {
        match self {
            Row::Borders => "borders",
            Row::Places => "places",
            Row::WaterNames => "water_names",
            Row::Roads => "roads",
            Row::RoadRefs => "road_refs",
            Row::Railways => "railways",
            Row::Airports => "airports",
            Row::Rivers => "rivers",
            Row::Water => "water",
            Row::Peaks => "peaks",
            Row::Parks => "parks",
        }
    }

    pub fn from_key(key: &str) -> Option<Row> {
        Row::ALL.into_iter().find(|r| r.key() == key)
    }

    /// Группа линий строки.
    pub fn lines(self) -> Option<Group> {
        match self {
            Row::Borders => Some(Group::Borders),
            Row::Roads => Some(Group::Roads),
            Row::Railways => Some(Group::Railways),
            Row::Airports => Some(Group::Airports),
            Row::Rivers => Some(Group::Rivers),
            Row::Water => Some(Group::Water),
            Row::Parks => Some(Group::Parks),
            _ => None,
        }
    }

    /// Классы подписей строки, `LABEL_KINDS` модуля.
    pub fn kinds(self) -> &'static [Kind] {
        match self {
            Row::Places => &[Kind::Country, Kind::Capital, Kind::City, Kind::State, Kind::Town, Kind::Village],
            Row::WaterNames => &[Kind::Water],
            Row::Peaks => &[Kind::Peak],
            Row::RoadRefs => &[Kind::RoadRef],
            Row::Parks => &[Kind::Park],
            Row::Airports => &[Kind::Airport],
            _ => &[],
        }
    }
}

/// Строка группы: строка, название, подсказка.
pub type TreeRow = (Row, &'static str, &'static str);
/// Группа: название, подсказка, строки.
pub type TreeGroup = (&'static str, &'static str, &'static [TreeRow]);

/// Группы раздела «Слои»: название, подсказка и строки - название
/// и подсказка, `geo_tree` модуля. Тексты - ключи перевода.
pub const TREE: [TreeGroup; 3] = [
    (
        "Borders and names",
        "Borders of countries and regions and labels on the globe.",
        &[
            (Row::Borders, "Borders", "Borders of countries bright yellow, borders of regions thin white. Sea borders are not drawn."),
            (Row::Places, "Settlements", "Names of countries, regions, cities, towns and villages. Smaller places appear when zooming in."),
            (Row::WaterNames, "Water names", "Names of seas, lakes and reservoirs, in light blue italics."),
        ],
    ),
    (
        "Transport",
        "Roads, their numbers, railways and airports.",
        &[
            (Row::Roads, "Roads", "Motorways yellow, main roads light yellow, the rest thin white. Over a city the roads cover the basemap with a dense mesh."),
            (Row::RoadRefs, "Road numbers", "Plates with numbers of motorways and main roads. They are labelled from a height below 1000 km."),
            (Row::Railways, "Railways", "A light line with a dark dash. They disappear together with motorways when zooming out. Station and access tracks are not drawn."),
            (Row::Airports, "Airports", "Names of airports with a square icon, runways when zooming in. They are labelled from a height below 1000 km."),
        ],
    ),
    (
        "Nature",
        "Rivers, peaks and protected areas.",
        &[
            (Row::Rivers, "Rivers", "Narrow rivers as blue lines. They appear from about 600 m per pixel."),
            (Row::Water, "Lakes and reservoirs", "Shores of lakes, reservoirs and wide rivers outlined in blue. Reservoirs are cut into pieces in the data, and the outline runs along the cuts too."),
            (Row::Peaks, "Peaks", "Peaks and volcanoes with their height in metres, a triangle icon. They are labelled from a height below 400 km."),
            (Row::Parks, "Reserves and national parks", "Nature reserves, national parks and wildlife refuges, a green outline and a name. Names are seen below 3000 km, the outline together with motorways."),
        ],
    ),
];

/// Строки по умолчанию: границы и населённые пункты, как у модуля
/// (`base.py`).
pub const DEFAULT: [Row; 2] = [Row::Borders, Row::Places];

/// Разобранных векторных тайлов в памяти не больше стольких.
const KEEP_TILES: usize = 3000;

/// Векторная основа: включённые строки, разобранные тайлы и картинки
/// линий тайлов глобуса.
pub struct VectorBase {
    /// Включённые строки.
    pub rows: BTreeSet<Row>,
    /// Группы раздела «Слои», которые видны: в них что-то включено или
    /// было включено до «Убрать с глобуса», как у модуля.
    pub seen: BTreeSet<usize>,
    tiles: HashMap<TileKey, Arc<lines::Tile>>,
    /// Пришедшие, ещё не разобранные тайлы: разбор идёт в бюджете кадра,
    /// как подготовка картинок в пуле потоков у модуля.
    raw: HashMap<TileKey, Arc<Vec<u8>>>,
    /// Тайлы глобуса, чья картинка готова: true - картинка в видеокарте,
    /// false - рисовать нечего.
    drawn: HashMap<TileKey, bool>,
    /// Картинки в работе у рабочих потоков.
    pending: HashSet<TileKey>,
    /// Готовые картинки, которые ждут загрузки в видеокарту.
    ready: Vec<Done>,
    /// Номер набора групп: ответ потока под прежние группы отбрасывается.
    generation: u64,
    /// Карты под линиями снизу вверх: ключ строки и вид.
    maps: Vec<(&'static str, MapKind)>,
    /// Тайлы карт: пришедшие и уже раскрашенные.
    rasters: HashMap<(&'static str, TileKey), Raster>,
    #[cfg(not(target_arch = "wasm32"))]
    pool: Option<Pool>,
}

/// Тайл карты: пришедший RGBA или картинка слоя, None - рисовать нечего.
#[derive(Clone)]
enum Raster {
    Raw(Arc<Vec<u8>>),
    Ready(Option<Arc<Vec<u8>>>),
}

/// Тайл карты для тайла глобуса key: свой или предок на глубшем уровне
/// карты, глубина и клетка доли.
fn raster_of(key: TileKey, max_level: u32) -> (TileKey, u32, (u32, u32)) {
    let (z, x, y) = key;
    let depth = z.saturating_sub(max_level);
    let cells = 1u32 << depth.min(31);
    ((z - depth, x >> depth, y >> depth), depth, (x % cells, y % cells))
}

/// Векторный тайл для задания: ещё байты или уже разобранный.
enum Source {
    Raw(Arc<Vec<u8>>),
    Tile(Arc<lines::Tile>),
}

/// Задание рабочего потока: картинка тайла key из тайлов sources.
struct Job {
    generation: u64,
    key: TileKey,
    groups: HashSet<Group>,
    sources: Vec<(TileKey, Source)>,
    /// Карты снизу вверх: ключ, вид, тайл карты, глубина, клетка, тайл.
    maps: Vec<MapPart>,
}

/// Картинка слоя RGBA 256 × 256 с премноженной альфой.
type Picture = Arc<Vec<u8>>;
/// Клетка доли предка: столбец и строка.
type Cell = (u32, u32);
/// Раскрашенный тайл карты: (ключ карты, тайл) и картинка.
type Painted = ((&'static str, TileKey), Option<Picture>);

/// Карта в задании: ключ, вид, тайл карты, глубина и клетка доли, тайл.
struct MapPart {
    id: &'static str,
    kind: MapKind,
    source: TileKey,
    depth: u32,
    cell: Cell,
    raster: Raster,
}

/// Готовая картинка и разобранные по дороге тайлы.
struct Done {
    /// Поколение набора групп. На странице работа идёт без потоков
    /// и устаревших результатов не бывает.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    generation: u64,
    key: TileKey,
    decoded: Vec<(TileKey, Arc<lines::Tile>)>,
    /// Раскрашенные по дороге тайлы карт.
    painted: Vec<Painted>,
    picture: Option<Vec<u8>>,
}

/// Разобрать и нарисовать задание.
fn work(job: Job) -> Done {
    let mut decoded = Vec::new();
    let mut tiles: HashMap<TileKey, Arc<lines::Tile>> = HashMap::new();
    for (k, s) in job.sources {
        let tile = match s {
            Source::Tile(t) => t,
            Source::Raw(bytes) => {
                let t = Arc::new(lines::decode(&bytes, &job.groups).unwrap_or_default());
                decoded.push((k, t.clone()));
                t
            }
        };
        tiles.insert(k, tile);
    }
    let top = if job.groups.is_empty() { None } else { lines::draw(job.key, &|k| tiles.get(&k).cloned(), &job.groups) };
    if job.maps.is_empty() {
        return Done { generation: job.generation, key: job.key, decoded, painted: Vec::new(), picture: top };
    }
    let mut painted = Vec::new();
    let mut layers: Vec<(Picture, u32, Cell)> = Vec::new();
    for MapPart { id, kind, source, depth, cell, raster } in job.maps {
        let ready = match raster {
            Raster::Ready(r) => r,
            Raster::Raw(rgba) => {
                let r = kind.picture(&rgba).map(Arc::new);
                painted.push(((id, source), r.clone()));
                r
            }
        };
        if let Some(r) = ready {
            layers.push((r, depth, cell));
        }
    }
    let mut pieces: Vec<rastermaps::Piece<'_>> = layers.iter().map(|(r, depth, cell)| rastermaps::Piece { rgba: r, depth: *depth, cell: *cell }).collect();
    if let Some(t) = &top {
        pieces.push(rastermaps::Piece { rgba: t, depth: 0, cell: (0, 0) });
    }
    let picture = rastermaps::compose(&pieces);
    Done { generation: job.generation, key: job.key, decoded, painted, picture }
}

/// Заданий у рабочих потоков сразу, как `MAX_JOBS` наложения модуля.
pub const MAX_JOBS: usize = 6;
/// Картинок в видеокарту за кадр: в ускоренном туре готовых приходило
/// много сразу, и кадр затягивался на их загрузке.
pub const UPLOADS: usize = 4;
/// Рабочих потоков линий.
#[cfg(not(target_arch = "wasm32"))]
const THREADS: usize = 2;

/// Рабочие потоки: как потоки QGIS, в которых модуль рисует картинки
/// наложения, главный поток получает готовое.
#[cfg(not(target_arch = "wasm32"))]
struct Pool {
    jobs: std::sync::mpsc::Sender<Job>,
    done: std::sync::mpsc::Receiver<Done>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Pool {
    fn new() -> Option<Self> {
        let (jobs, rx) = std::sync::mpsc::channel::<Job>();
        let (tx, done) = std::sync::mpsc::channel::<Done>();
        let rx = Arc::new(std::sync::Mutex::new(rx));
        for n in 0..THREADS {
            let (rx, tx) = (rx.clone(), tx.clone());
            std::thread::Builder::new()
                .name(format!("planetx-lines-{n}"))
                .spawn(move || {
                    loop {
                        let job = match rx.lock() {
                            Ok(r) => r.recv(),
                            Err(_) => return,
                        };
                        let Ok(job) = job else {
                            return;
                        };
                        if tx.send(work(job)).is_err() {
                            return;
                        }
                    }
                })
                .ok()?;
        }
        Some(Self { jobs, done })
    }
}

impl Default for VectorBase {
    fn default() -> Self {
        Self::new(DEFAULT.into_iter().collect())
    }
}

impl VectorBase {
    pub fn new(rows: BTreeSet<Row>) -> Self {
        let mut out = Self { rows: BTreeSet::new(), seen: BTreeSet::new(), tiles: HashMap::new(),
            raw: HashMap::new(),
            drawn: HashMap::new(),
            pending: HashSet::new(),
            ready: Vec::new(),
            generation: 0,
            maps: Vec::new(),
            rasters: HashMap::new(),
            #[cfg(not(target_arch = "wasm32"))]
            pool: Pool::new(),
        };
        out.set_rows(rows);
        out
    }

    /// Новые включённые строки. Картинки линий рисуются заново.
    /// Возвращает, сменились ли группы линий.
    pub fn set_rows(&mut self, rows: BTreeSet<Row>) -> bool {
        let before = self.line_groups();
        self.rows = rows;
        for (n, (_, _, group)) in TREE.iter().enumerate() {
            if group.iter().any(|(r, _, _)| self.rows.contains(r)) {
                self.seen.insert(n);
            }
        }
        let changed = before != self.line_groups();
        if changed {
            // Тайлы разобраны под прежние группы: запрашиваются заново,
            // их отдаёт кэш загрузчика.
            self.drawn.clear();
            self.tiles.clear();
            self.raw.clear();
            self.pending.clear();
            self.ready.clear();
            self.generation += 1;
        }
        changed
    }

    /// Группы линий включённых строк.
    pub fn line_groups(&self) -> HashSet<Group> {
        self.rows.iter().filter_map(|r| r.lines()).collect()
    }

    /// Классы подписей включённых строк.
    pub fn label_kinds(&self) -> HashSet<Kind> {
        self.rows.iter().flat_map(|r| r.kinds().iter().copied()).collect()
    }

    /// Новые карты под линиями снизу вверх. Картинки тайлов рисуются
    /// заново, разобранные векторные тайлы остаются. Возвращает, сменился
    /// ли набор.
    pub fn set_maps(&mut self, maps: Vec<(&'static str, MapKind)>) -> bool {
        if maps == self.maps {
            return false;
        }
        let keep: HashSet<&'static str> = maps.iter().map(|(k, _)| *k).collect();
        self.rasters.retain(|(k, _), _| keep.contains(k));
        self.maps = maps;
        self.drawn.clear();
        self.pending.clear();
        self.ready.clear();
        self.generation += 1;
        true
    }

    /// Ключи карт под линиями.
    pub fn map_keys(&self) -> Vec<&'static str> {
        self.maps.iter().map(|(k, _)| *k).collect()
    }

    /// Есть что рисовать: линии или карты.
    fn active(&self) -> bool {
        !self.maps.is_empty() || !self.line_groups().is_empty()
    }

    /// Тайлы карт, нужные картинкам тайлов кадра draw: ключ карты, тайл,
    /// приоритет.
    pub fn maps_wanted(&self, draw: &[TileKey]) -> Vec<(&'static str, TileKey, f64)> {
        let mut out = Vec::new();
        let mut seen: HashSet<(&'static str, TileKey)> = HashSet::new();
        for key in draw {
            if self.drawn.contains_key(key) {
                continue;
            }
            for (id, kind) in &self.maps {
                let (source, _, _) = raster_of(*key, kind.max_level());
                if !self.rasters.contains_key(&(*id, source)) && seen.insert((*id, source)) {
                    out.push((*id, source, 50.0 - f64::from(source.0)));
                }
            }
        }
        out
    }

    /// Пришедший тайл карты id: RGBA 256 × 256 или None - тайла нет.
    pub fn add_map(&mut self, id: &'static str, key: TileKey, rgba: Option<Vec<u8>>) {
        if !self.maps.iter().any(|(k, _)| *k == id) {
            return;
        }
        let raster = match rgba {
            Some(r) if r.len() == (lines::TILE_SIZE * lines::TILE_SIZE * 4) as usize => Raster::Raw(Arc::new(r)),
            _ => Raster::Ready(None),
        };
        self.rasters.insert((id, key), raster);
    }

    /// Векторные тайлы, нужные картинкам линий тайлов кадра draw.
    pub fn wanted(&self, draw: &[TileKey]) -> Vec<(TileKey, f64)> {
        let groups = self.line_groups();
        if groups.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<(TileKey, f64)> = Vec::new();
        let mut seen: HashSet<TileKey> = HashSet::new();
        for key in draw {
            if self.drawn.contains_key(key) {
                continue;
            }
            for source in lines::sources(*key, &groups) {
                if !self.tiles.contains_key(&source) && !self.raw.contains_key(&source) && seen.insert(source) {
                    out.push((source, 50.0 - f64::from(source.0)));
                }
            }
        }
        out
    }

    /// Пришедший векторный тайл: байты общие с подписями, None - тайла
    /// нет или он не векторный, он пуст и снова не просится.
    pub fn add(&mut self, key: TileKey, bytes: Option<Arc<Vec<u8>>>) {
        match bytes {
            Some(b) => {
                self.raw.insert(key, b);
            }
            None => {
                self.tiles.insert(key, Arc::new(lines::Tile::default()));
            }
        }
    }
    /// Нарисовать картинки линий тайлов кадра draw, пока не вышло время
    /// budget в секундах, и положить их в видеокарту. Картинка,
    /// вытесненная видеокартой, рисуется снова.
    pub fn step(&mut self, draw: &[TileKey], budget: f64, globe: &mut render::Globe) {
        let groups = self.line_groups();
        self.drawn.retain(|k, on| !*on || globe.has_overlay(*k));
        if !self.active() {
            return;
        }
        let start = web_time::Instant::now();
        // Готовое от рабочих потоков.
        #[cfg(not(target_arch = "wasm32"))]
        let (threaded, dones): (bool, Vec<Done>) = match &self.pool {
            Some(pool) => {
                self.ready.extend(pool.done.try_iter());
                let n = self.ready.len().min(UPLOADS);
                (true, self.ready.drain(..n).collect())
            }
            None => (false, Vec::new()),
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            for done in dones {
                self.pending.remove(&done.key);
                if done.generation != self.generation {
                    continue;
                }
                for (k, t) in done.decoded {
                    self.raw.remove(&k);
                    self.tiles.insert(k, t);
                }
                for (k, r) in done.painted {
                    self.rasters.insert(k, Raster::Ready(r));
                }
                self.finish(globe, done.key, done.picture);
            }
        }
        #[cfg(target_arch = "wasm32")]
        let threaded = false;
        for key in draw {
            if self.drawn.contains_key(key) || self.pending.contains(key) {
                continue;
            }
            let sources = lines::sources(*key, &groups);
            if !sources.iter().all(|s| self.tiles.contains_key(s) || self.raw.contains_key(s)) {
                continue;
            }
            let mut maps = Vec::with_capacity(self.maps.len());
            for (id, kind) in &self.maps {
                let (source, depth, cell) = raster_of(*key, kind.max_level());
                if let Some(r) = self.rasters.get(&(*id, source)) {
                    maps.push(MapPart { id, kind: *kind, source, depth, cell, raster: r.clone() });
                }
            }
            if maps.len() < self.maps.len() {
                continue;
            }
            let job = Job {
                maps,
                generation: self.generation,
                key: *key,
                groups: groups.clone(),
                sources: sources
                    .iter()
                    .filter_map(|s| match (self.tiles.get(s), self.raw.get(s)) {
                        (Some(t), _) => Some((*s, Source::Tile(t.clone()))),
                        (None, Some(b)) => Some((*s, Source::Raw(b.clone()))),
                        _ => None,
                    })
                    .collect(),
            };
            if threaded {
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(pool) = &self.pool {
                    if self.pending.len() >= MAX_JOBS {
                        break;
                    }
                    if pool.jobs.send(job).is_ok() {
                        self.pending.insert(*key);
                    }
                }
                continue;
            }
            // Без потоков, на странице, - в бюджете кадра.
            let done = work(job);
            for (k, t) in done.decoded {
                self.raw.remove(&k);
                self.tiles.insert(k, t);
            }
            for (k, r) in done.painted {
                self.rasters.insert(k, Raster::Ready(r));
            }
            self.finish(globe, done.key, done.picture);
            if start.elapsed().as_secs_f64() > budget {
                break;
            }
        }
        if self.tiles.len() > KEEP_TILES {
            let keep: HashSet<TileKey> = draw.iter().flat_map(|k| lines::sources(*k, &groups)).collect();
            self.tiles.retain(|k, _| keep.contains(k));
            self.raw.retain(|k, _| keep.contains(k));
        }
        if self.rasters.len() > KEEP_TILES {
            let keep: HashSet<(&'static str, TileKey)> =
                draw.iter().flat_map(|k| self.maps.iter().map(|(id, kind)| (*id, raster_of(*k, kind.max_level()).0))).collect();
            self.rasters.retain(|k, _| keep.contains(k));
        }
    }

    /// Картинка тайла готова: в видеокарту, если на ней что-то есть.
    fn finish(&mut self, globe: &mut render::Globe, key: TileKey, picture: Option<Vec<u8>>) {
        if let Some(rgba) = &picture {
            globe.set_overlay(key, lines::TILE_SIZE, lines::TILE_SIZE, rgba);
        }
        self.drawn.insert(key, picture.is_some());
    }

    /// Все картинки линий готовы для тайлов draw.
    pub fn ready(&self, draw: &[TileKey]) -> bool {
        !self.active() || draw.iter().all(|k| self.drawn.contains_key(k))
    }

    /// Строки и группы в текст настроек.
    pub fn to_text(&self) -> String {
        let rows: Vec<&str> = self.rows.iter().map(|r| r.key()).collect();
        let seen: Vec<String> = self.seen.iter().map(usize::to_string).collect();
        format!("{};{}", rows.join(","), seen.join(","))
    }

    /// Строки и группы из текста настроек, чужое пропускается.
    pub fn from_text(text: &str) -> Self {
        let (rows, seen) = text.split_once(';').unwrap_or((text, ""));
        let mut out = Self::new(rows.split(',').filter_map(Row::from_key).collect());
        out.seen.extend(seen.split(',').filter_map(|s| s.parse::<usize>().ok()).filter(|n| *n < TREE.len()));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{Row, VectorBase};
    use data::places::Kind;

    #[test]
    fn defaults_and_text_round_trip() {
        let v = VectorBase::default();
        assert!(v.label_kinds().contains(&Kind::City));
        assert!(!v.label_kinds().contains(&Kind::RoadRef));
        assert_eq!(v.seen.iter().copied().collect::<Vec<_>>(), [0]);
        let mut w = VectorBase::default();
        w.set_rows([Row::Roads, Row::RoadRefs].into());
        let back = VectorBase::from_text(&w.to_text());
        assert_eq!(back.rows, w.rows);
        // Группа «Границы и названия» видна и после снятия флажков.
        assert_eq!(back.seen.iter().copied().collect::<Vec<_>>(), [0, 1]);
    }
}
