// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! «Мои слои» - векторные слои в разделе «Слои», как слои проекта QGIS
//! в глобусе модуля. Строка - слой: флажок, название, меню «⋯» со стилем,
//! подписями, выдавливанием, папкой, «Приблизить к слою», «В Мои метки»,
//! «Сохранить как GeoJSON» и «Убрать с глобуса». KML, KMZ и сцены идут
//! в «Мои метки», как у модуля.
//!
//! Слои лежат в папках одного уровня. Строка папки - флажок всех её
//! слоёв, щелчок по названию сворачивает папку.
//!
//! Слои живут между запусками, как «Мои метки», решение автора
//! от 4 ноября 2026 года. Список и стиль - строкой хранилища программы
//! ([`DataLayers::index`]), объекты слоя - текстом GeoJSON: в программе
//! файлом в папке данных, на странице - в той же строке, пока она меньше
//! [`INLINE_LIMIT`]. В сцену слои не пишутся: сцену открывает и модуль.
//!
//! Линии и многоугольники ложатся в картинку наложения тайлов, как слои
//! QGIS в стопке наложения модуля (`net.overlay`), точки - кругами на
//! рельефе. Части многоугольника заливаются по правилу чёт-нечет: дыры
//! и части мультимногоугольника различаются без разметки колец.
//!
//! Подписи по полю - надписи вида, у слоя не больше [`MAX_LABELS`]
//! ближайших к точке взгляда, `core/layer_labels.py` модуля. Выдавливание
//! по полю - призма многоугольника, столбик точки, стенка линии,
//! `core/extrude.py` модуля, не больше [`MAX_EXTRUDED`] объектов слоя.

use std::collections::BTreeSet;
use std::sync::Arc;

use data::layers::{LayerData, Shape as Geom};
use data::linesets::Line;
use eframe::egui;
use scene::features::{Kind, Shape};

use crate::i18n::{Lang, tr, trf};
use crate::identify::Feature;

/// Цвета новых слоёв по кругу.
pub const PALETTE: [[u8; 3]; 8] = [[230, 80, 60], [60, 140, 230], [250, 190, 40], [60, 190, 110], [200, 90, 220], [40, 200, 210], [240, 130, 40], [235, 235, 235]];
/// Подписей слоя, ближайших к точке взгляда, `MAX_PER_LAYER` модуля.
pub const MAX_LABELS: usize = 500;
/// Выдавленных объектов слоя, `MAX_FEATURES` модуля.
pub const MAX_EXTRUDED: usize = 20000;
/// Объектов слоя в окне «Объекты» у одной точки.
const NEAR: usize = 5;
/// Окно подписей - столько ширин видимой полосы, `WINDOW` модуля.
const WINDOW: f64 = 3.0;
/// Подписи выбираются заново при сдвиге на эту долю окна, `STEP` модуля.
const STEP: f64 = 0.5;
const M_PER_DEGREE: f64 = 111_320.0;
/// Есть связь с RustGIS: в меню слоя - «В проект RustGIS». Ставит окно
/// на каждом кадре.
pub static RUSTGIS_LINKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Ключ списка «Моих слоёв» в хранилище программы.
pub const KEY: &str = "my_layers";
/// Папка файлов слоёв в папке данных программы.
pub const DIR: &str = "my-layers";
/// Предел строки списка со слоями внутри на странице, байты: у
/// localStorage около 5 МБ на всё, до 4 МБ берут «Мои метки».
pub const INLINE_LIMIT: usize = 1_000_000;

/// Стиль слоя: цвет, толщина линии в пикселях тайла, доля заливки
/// многоугольника, непрозрачность слоя, размер точки в пикселях.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub color: [u8; 3],
    pub width: f32,
    pub fill: f32,
    pub opacity: f32,
    pub size: f32,
}

impl Style {
    /// Стиль n-го слоя: цвет из [`PALETTE`] по кругу.
    pub fn new(n: usize) -> Style {
        Style { color: PALETTE[n % PALETTE.len()], width: 2.0, fill: 0.35, opacity: 1.0, size: 6.0 }
    }

    fn alpha(&self, share: f32) -> u8 {
        (255.0 * (self.opacity * share).clamp(0.0, 1.0)).round() as u8
    }

    pub fn line_rgba(&self) -> [u8; 4] {
        let [r, g, b] = self.color;
        [r, g, b, self.alpha(1.0)]
    }

    pub fn fill_rgba(&self) -> [u8; 4] {
        let [r, g, b] = self.color;
        [r, g, b, self.alpha(self.fill)]
    }
}

/// Слой файла на глобусе.
pub struct DataLayer {
    pub id: u64,
    pub title: String,
    pub file: String,
    pub data: LayerData,
    pub visible: bool,
    pub style: Style,
    /// Номер поля подписей.
    pub label: Option<usize>,
    /// Номер числового поля высоты и множитель, метры.
    pub extrude: Option<(usize, f64)>,
    /// Папка, пустая - слой вне папок.
    pub folder: String,
    /// Имя файла GeoJSON слоя в папке [`DIR`], None - объекты не записаны.
    pub store: Option<String>,
    /// Номер слоя проекта RustGIS: слой живой, его ведёт связь с RustGIS
    /// ([`crate::rustgislink`]), в хранилище он не пишется.
    pub live: Option<i64>,
    /// Цвета объектов по порядку - отрисовка слоя RustGIS. Пусто - поле
    /// color или цвет стиля.
    pub colors: Vec<[u8; 3]>,
    /// Файл слоя на диске, None - слой не из файла с диска.
    pub source: Option<Source>,
    /// Файл на диске изменился после чтения: строка «Обновить».
    pub stale: bool,
    /// Слой-картинка: растр окна анализа по рамке, объектов у него нет.
    /// Непрозрачность стиля - в оттенке наложения.
    pub picture: Option<scene::kml::Overlay>,
    /// Описание слоя - у результата анализа источник, настройки, классы.
    /// Идёт в подсказку строки.
    pub about: String,
    /// Сторона шторки слоёв, подменю «Шторка» меню слоя модуля.
    pub side: render::Side,
    /// Время объектов: номер поля начала и поля конца, как временные
    /// свойства слоя QGIS у модуля. None - слой без времени, виден всегда.
    pub time: Option<(usize, Option<usize>)>,
    /// Объект вне окна шкалы времени: не рисуется и не выбирается.
    off: Vec<bool>,
}

/// Сторона шторки словом для списка слоёв: left, right или пусто.
fn side_word(side: render::Side) -> &'static str {
    match side {
        render::Side::Left => "left",
        render::Side::Right => "right",
        render::Side::Both => "",
    }
}

/// Поля времени слоя из записи списка слоёв: [начало, конец или null].
fn time_of(v: &serde_json::Value) -> Option<(usize, Option<usize>)> {
    let t = v.get("time")?.as_array()?;
    let field = |x: &serde_json::Value| x.as_u64().and_then(|n| usize::try_from(n).ok());
    Some((field(t.first()?)?, t.get(1).and_then(field)))
}

/// Сторона шторки по слову списка слоёв.
fn side_of(word: &str) -> render::Side {
    match word {
        "left" => render::Side::Left,
        "right" => render::Side::Right,
        _ => render::Side::Both,
    }
}

/// Картинка слоя файлом GeoTIFF в EPSG:4326, `Картинки на поверхности
/// в проект QGIS` модуля: цвета и прозрачность, как на глобусе, без
/// оттенка слоя. Узел сетки - центр пикселя, края крайних пикселей -
/// рамка LatLonBox. Ошибка - ключ перевода: картинки нет, рамка
/// повёрнута или задана углами, картинка не читается.
pub fn picture_geotiff(overlay: &scene::kml::Overlay) -> Result<Vec<u8>, &'static str> {
    let (grid, image) = picture_grid(overlay)?;
    data::geotiff::write_rgba(&grid, image.as_raw()).ok_or("The picture of the layer is not loaded")
}

/// Сетка узлов картинки слоя по её рамке и сама картинка RGBA, ошибки -
/// как у [`picture_geotiff`].
fn picture_grid(overlay: &scene::kml::Overlay) -> Result<(data::geotiff::LatLonGrid, image::RgbaImage), &'static str> {
    let bytes = overlay.image.as_deref().filter(|b| !b.is_empty()).ok_or("The picture of the layer is not loaded")?;
    let [north, south, east, west, _] = overlay.overlay.latlon_box.filter(|b| b[4] == 0.0 && b[0] > b[1]).ok_or("The picture is turned: GeoTIFF needs a frame along parallels and meridians")?;
    let east = if east < west { east + 360.0 } else { east };
    let image = image::load_from_memory(bytes).map_err(|_| "The picture of the layer is not loaded")?.to_rgba8();
    let (cols, rows) = (image.width() as usize, image.height() as usize);
    if cols == 0 || rows == 0 {
        return Err("The picture of the layer is not loaded");
    }
    let (dlat, dlon) = ((north - south) / rows as f64, (east - west) / cols as f64);
    Ok((data::geotiff::LatLonGrid { north: north - dlat / 2.0, west: west + dlon / 2.0, dlat, dlon, cols, rows }, image))
}

/// Файл слоя на диске и время его изменения, секунды Unix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub path: String,
    pub modified: u64,
}

/// Как часто сверять время изменения файлов слоёв, секунды.
pub const SOURCE_CHECK: f64 = 2.0;

/// Время изменения файла path, секунды Unix. На странице файлов нет.
pub fn file_modified(path: &str) -> Option<u64> {
    if cfg!(target_arch = "wasm32") {
        return None;
    }
    let time = std::fs::metadata(path).ok()?.modified().ok()?;
    time.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// Байты файла path или текст ошибки.
pub fn read_file(path: &std::path::Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| e.to_string())
}

pub use data::layers::{Polygon, Ring, polygons};

/// Точки слоя: места, цвет, размер, непрозрачность.
pub type DotSet = (Vec<(f64, f64)>, [u8; 3], f32, f32);

/// Точка внутри частей многоугольника по правилу чёт-нечет.
fn inside_parts(parts: &[Vec<(f64, f64)>], lat: f64, lon: f64) -> bool {
    parts.iter().filter(|p| p.len() > 2).filter(|p| data::site::inside(&[(lat, lon)], p).first().copied().unwrap_or(false)).count() % 2 == 1
}

/// Ключ выбора подписей: тот же, пока вид не сдвинулся на долю [`STEP`]
/// окна и не изменился вдвое по ширине, `key` модуля. width - ширина
/// видимой полосы в метрах.
pub fn label_key(lat: f64, lon: f64, width: f64) -> (i64, i64, i64) {
    let half = WINDOW * width / 2.0 / M_PER_DEGREE;
    let cell = (half * 2.0 * STEP).max(1e-6);
    (width.max(1.0).log2().round() as i64, (lat / cell).round() as i64, (lon / cell).round() as i64)
}

/// count подписей ближе всех к точке (lat, lon), `nearest` модуля.
/// Расстояние - на плоскости с поправкой долготы на широту.
pub fn nearest(mut items: Vec<(String, f64, f64)>, lat: f64, lon: f64, count: usize) -> Vec<(String, f64, f64)> {
    if items.len() <= count {
        return items;
    }
    let cos_lat = lat.to_radians().cos();
    let far = |(_, a, b): &(String, f64, f64)| {
        let dlat = a - lat;
        let dlon = ((b - lon + 180.0).rem_euclid(360.0) - 180.0) * cos_lat;
        dlat * dlat + dlon * dlon
    };
    items.sort_by(|a, b| far(a).total_cmp(&far(b)));
    items.truncate(count);
    items
}

/// Картинка видна в окне шкалы времени window, без окна - всегда.
fn in_window(p: &scene::kml::Overlay, window: Option<(f64, f64)>) -> bool {
    window.is_none_or(|(lo, hi)| scene::when::visible(p.time.as_ref(), lo, hi))
}

/// Поля слоёв меток, часть полей `FIELDS` слоёв «Моих меток» модуля:
/// название, описание, цвет «#rrggbb» и папка - путь через « / ».
pub const PLACE_FIELDS: [&str; 4] = ["name", "description", "color", "folder"];

/// Метки папки на любой глубине - тремя слоями по виду, как «Добавить
/// слои меток в проект» модуля (`add_to_project`, таблицы points, lines,
/// polygons): точки, линии, многоугольники с названиями names. Слой без
/// меток не идёт. Метки без точек пропускаются.
pub fn places_layers(folder: &scene::kml::Folder, names: [&str; 3]) -> Vec<LayerData> {
    fn walk<'a>(f: &'a scene::kml::Folder, path: &str, out: &mut Vec<(&'a scene::kml::Place, String)>) {
        for child in &f.children {
            match child {
                scene::kml::Item::Folder(sub) => {
                    let inner = if path.is_empty() { sub.name.clone() } else { format!("{path} / {}", sub.name) };
                    walk(sub, &inner, out);
                }
                scene::kml::Item::Place(p) if !p.points.is_empty() => out.push((p, path.to_string())),
                _ => {}
            }
        }
    }
    let mut places = Vec::new();
    walk(folder, "", &mut places);
    let mut out = Vec::new();
    for (kind, name) in [Kind::Point, Kind::Line, Kind::Polygon].into_iter().zip(names) {
        let items: Vec<data::layers::Item> = places
            .iter()
            .filter(|(p, _)| p.kind == kind)
            .map(|(p, path)| {
                let [r, g, b, _] = p.color;
                let (shape, parts) = match kind {
                    Kind::Polygon => (Geom::Polygon, std::iter::once(p.points.clone()).chain(p.holes.iter().flatten().cloned()).collect()),
                    Kind::Point => (Geom::Point, vec![p.points.iter().take(1).copied().collect()]),
                    Kind::Line => (Geom::Line, vec![p.points.clone()]),
                };
                let values = vec![p.name.clone(), p.description.clone(), format!("#{r:02x}{g:02x}{b:02x}"), path.clone()];
                data::layers::Item { shape, parts, heights: Vec::new(), values }
            })
            .collect();
        if !items.is_empty() {
            out.push(LayerData { name: name.to_string(), epsg: Some(4326), fields: PLACE_FIELDS.iter().map(|f| f.to_string()).collect(), numeric: vec![false; PLACE_FIELDS.len()], items });
        }
    }
    out
}

/// Слой из папки меток расчёта: многоугольники, линии и точки на любой
/// глубине, поля - name и поля меток по порядку первого появления. Цвет
/// метки - поле color «#rrggbb», если его нет: так слой рисует классы
/// своими цветами. None - в папке есть другие метки или меток нет.
pub fn folder_layer(folder: &scene::kml::Folder) -> Option<LayerData> {
    let places = folder.places();
    if places.is_empty() || places.iter().any(|p| !matches!(p.kind, Kind::Polygon | Kind::Line | Kind::Point) || p.points.is_empty()) {
        return None;
    }
    // В папке есть и другое, кроме меток - картинки, модели: папка
    // остаётся папкой меток.
    if folder.children.iter().any(|c| !matches!(c, scene::kml::Item::Place(_) | scene::kml::Item::Folder(_))) {
        return None;
    }
    let mut fields = vec!["name".to_string()];
    for p in &places {
        for (key, _) in p.data.iter().flatten() {
            if !fields.contains(key) {
                fields.push(key.clone());
            }
        }
    }
    let colored = !fields.iter().any(|f| f.eq_ignore_ascii_case("color"));
    if colored {
        fields.push("color".to_string());
    }
    let items: Vec<data::layers::Item> = places
        .iter()
        .map(|p| {
            let value = |key: &str| p.data.iter().flatten().find(|(k, _)| k == key).map(|(_, v)| v.clone());
            let mut values: Vec<String> = fields.iter().map(|f| if f == "name" { value("name").unwrap_or_else(|| p.name.clone()) } else { value(f).unwrap_or_default() }).collect();
            if colored && let Some(last) = values.last_mut() {
                let [r, g, b, _] = p.color;
                *last = format!("#{r:02x}{g:02x}{b:02x}");
            }
            let (shape, parts) = match p.kind {
                Kind::Polygon => (Geom::Polygon, std::iter::once(p.points.clone()).chain(p.holes.iter().flatten().cloned()).collect()),
                Kind::Point => (Geom::Point, vec![p.points.iter().take(1).copied().collect()]),
                _ => (Geom::Line, vec![p.points.clone()]),
            };
            data::layers::Item { shape, parts, heights: Vec::new(), values }
        })
        .collect();
    let numeric = (0..fields.len())
        .map(|n| {
            let mut filled = items.iter().map(|i| i.values[n].trim()).filter(|v| !v.is_empty()).peekable();
            filled.peek().is_some() && filled.all(|v| v.parse::<f64>().is_ok())
        })
        .collect();
    Some(LayerData { name: folder.name.clone(), epsg: Some(4326), fields, numeric, items })
}

impl DataLayer {
    /// Время объекта n по полям времени слоя: пара строк начала и конца,
    /// без поля конца - момент. None - у слоя или объекта времени нет.
    pub fn item_time(&self, n: usize) -> Option<scene::when::Time> {
        let (start, end) = self.time?;
        let values = &self.data.items.get(n)?.values;
        let begin = values.get(start).map(|v| v.trim().to_string()).unwrap_or_default();
        let finish = end.and_then(|e| values.get(e)).map(|v| v.trim().to_string()).unwrap_or_else(|| begin.clone());
        (!begin.is_empty() || !finish.is_empty()).then_some((begin, finish))
    }

    /// Объекты вне окна шкалы времени window - заново.
    fn refresh_off(&mut self, window: Option<(f64, f64)>) {
        self.off = match (self.time, window) {
            (Some(_), Some((lo, hi))) => (0..self.data.items.len()).map(|n| !scene::when::visible(self.item_time(n).as_ref(), lo, hi)).collect(),
            _ => Vec::new(),
        };
    }

    /// Объект n в окне шкалы времени или слой без времени.
    fn on(&self, n: usize) -> bool {
        !self.off.get(n).copied().unwrap_or(false)
    }

    /// Охват шкалы по временам объектов слоя.
    fn time_extent(&self) -> Option<(f64, f64)> {
        self.time?;
        let times: Vec<Option<scene::when::Time>> = (0..self.data.items.len()).map(|n| self.item_time(n)).collect();
        scene::when::extent(times.iter().map(Option::as_ref))
    }

    /// Цвет объекта n: цвет отрисовки RustGIS, поле color «#rrggbb» слоя,
    /// иначе цвет стиля.
    fn item_color(&self, n: usize) -> [u8; 3] {
        if let Some(c) = self.colors.get(n) {
            return *c;
        }
        let field = self.data.fields.iter().position(|f| f.eq_ignore_ascii_case("color"));
        field
            .and_then(|f| self.data.items.get(n)?.values.get(f))
            .and_then(|v| crate::gpkg::color_of(v))
            .map_or(self.style.color, |[r, g, b, _]| [r, g, b])
    }

    /// Высота объекта n по полю выдавливания, None - не выдавлен.
    fn raised(&self, n: usize) -> Option<f64> {
        let (field, factor) = self.extrude?;
        if n >= MAX_EXTRUDED {
            return None;
        }
        let value = self.data.items.get(n)?.values.get(field)?.trim().parse::<f64>().ok();
        crate::foldercalc::feature_height(value, factor)
    }

    /// Линии и многоугольники для картинки наложения, кроме выдавленных.
    pub fn lines(&self) -> Vec<Line> {
        let mut out = Vec::new();
        for (n, item) in self.data.items.iter().enumerate() {
            if self.raised(n).is_some() || !self.on(n) {
                continue;
            }
            let style = Style { color: self.item_color(n), ..self.style };
            let (color, fill) = (style.line_rgba(), style.fill_rgba());
            match item.shape {
                Geom::Line => out.extend(item.parts.iter().filter(|p| p.len() > 1).map(|p| Line::new(p.clone(), color, self.style.width))),
                // Каждая часть мультимногоугольника - своим многоугольником:
                // рамка линии считается по внешнему кольцу, и далёкая часть
                // (Гвиана у Франции, Чукотка за линией перемены дат у
                // России) не выпадает из тайлов.
                Geom::Polygon => {
                    for (outer, holes) in polygons(&item.parts) {
                        if outer.len() > 2 {
                            out.push(Line::polygon(outer, holes, color, self.style.width, fill));
                        }
                    }
                }
                Geom::Point | Geom::None => {}
            }
        }
        out
    }

    /// Точки слоя, кроме выдавленных.
    pub fn dots(&self) -> Vec<(f64, f64)> {
        self.data
            .items
            .iter()
            .enumerate()
            .filter(|(n, i)| i.shape == Geom::Point && self.raised(*n).is_none() && self.on(*n))
            .flat_map(|(_, i)| i.parts.iter().filter_map(|p| p.first().copied()))
            .collect()
    }

    /// Все точки слоя - для охвата, у картинки - углы её рамки.
    pub fn extent_points(&self) -> Vec<(f64, f64)> {
        if let Some(p) = &self.picture {
            return p.overlay.corners.clone();
        }
        self.data.items.iter().flat_map(|i| i.parts.iter().flatten().copied()).collect()
    }

    /// Место подписи объекта: точка, средняя вершина линии, середина
    /// вершин внешнего кольца многоугольника.
    fn spot(item: &data::layers::Item) -> Option<(f64, f64)> {
        let first = item.parts.first()?;
        match item.shape {
            Geom::Point => first.first().copied(),
            Geom::Line => first.get(first.len() / 2).copied(),
            Geom::Polygon => {
                let n = first.len().max(1) as f64;
                let (a, b) = first.iter().fold((0.0, 0.0), |(a, b), p| (a + p.0, b + p.1));
                Some((a / n, b / n))
            }
            Geom::None => None,
        }
    }

    /// Подписи слоя по полю: текст, широта, долгота.
    pub fn label_spots(&self) -> Vec<(String, f64, f64)> {
        let Some(field) = self.label else {
            return Vec::new();
        };
        self.data
            .items
            .iter()
            .enumerate()
            .filter(|(n, _)| self.on(*n))
            .filter_map(|(_, i)| {
                let text = i.values.get(field)?.trim();
                if text.is_empty() {
                    return None;
                }
                let (lat, lon) = Self::spot(i)?;
                Some((text.to_string(), lat, lon))
            })
            .collect()
    }

    /// Выдавленные объекты: многоугольник - призма, точка - столбик,
    /// линия - стенка. radius - радиус тела, метры.
    pub fn extruded(&self, radius: f64) -> Vec<Shape> {
        if self.extrude.is_none() {
            return Vec::new();
        }
        let column_m = crate::foldercalc::column_radius(&self.dots_all());
        // Тело выдавленного объекта - сплошного цвета слоя.
        let color = self.style.line_rgba();
        let mut out = Vec::new();
        for (n, item) in self.data.items.iter().enumerate().take(MAX_EXTRUDED) {
            let Some(height) = self.raised(n).filter(|_| self.on(n)) else {
                continue;
            };
            let mut raise =|kind: Kind, points: Vec<(f64, f64)>, holes: Vec<Vec<(f64, f64)>>| {
                let mut s = Shape::new(kind, points);
                s.color = color;
                s.width = f64::from(self.style.width);
                if kind == Kind::Polygon {
                    s.fill = Some(color);
                }
                s.holes = (!holes.is_empty()).then_some(holes);
                s.height = height;
                s.extrude = true;
                s.mode = "relativeToGround".to_string();
                out.push(s);
            };
            match item.shape {
                Geom::Point => {
                    for (lat, lon) in item.parts.iter().filter_map(|p| p.first().copied()) {
                        raise(Kind::Polygon, crate::foldercalc::column(lat, lon, column_m, radius), Vec::new());
                    }
                }
                Geom::Line => {
                    for p in item.parts.iter().filter(|p| p.len() > 1) {
                        raise(Kind::Line, p.clone(), Vec::new());
                    }
                }
                Geom::Polygon => {
                    for (outer, holes) in polygons(&item.parts) {
                        raise(Kind::Polygon, outer, holes);
                    }
                }
                Geom::None => {}
            }
        }
        out
    }

    /// Линии и многоугольники слоя, кроме выдавленных, - объектами на
    /// рельефе. Так рисуется слой со стороной шторки: картинка наложения
    /// общая у всех слоёв и шторкой не делится. Не больше
    /// [`MAX_EXTRUDED`] объектов.
    pub fn flat_shapes(&self) -> Vec<Shape> {
        let mut out = Vec::new();
        for (n, item) in self.data.items.iter().enumerate().take(MAX_EXTRUDED) {
            if self.raised(n).is_some() || !self.on(n) {
                continue;
            }
            let style = Style { color: self.item_color(n), ..self.style };
            let (color, fill) = (style.line_rgba(), style.fill_rgba());
            let mut push = |kind: Kind, points: Vec<(f64, f64)>, holes: Vec<Vec<(f64, f64)>>| {
                let mut s = Shape::new(kind, points);
                s.color = color;
                s.width = f64::from(self.style.width);
                if kind == Kind::Polygon && fill[3] > 0 {
                    s.fill = Some(fill);
                }
                s.holes = (!holes.is_empty()).then_some(holes);
                out.push(s);
            };
            match item.shape {
                Geom::Line => item.parts.iter().filter(|p| p.len() > 1).for_each(|p| push(Kind::Line, p.clone(), Vec::new())),
                Geom::Polygon => polygons(&item.parts).into_iter().filter(|(outer, _)| outer.len() > 2).for_each(|(outer, holes)| push(Kind::Polygon, outer, holes)),
                Geom::Point | Geom::None => {}
            }
        }
        out
    }

    /// Все точки слоя без отбора выдавленных - для радиуса столбика.
    fn dots_all(&self) -> Vec<(f64, f64)> {
        self.data.items.iter().filter(|i| i.shape == Geom::Point).flat_map(|i| i.parts.iter().filter_map(|p| p.first().copied())).collect()
    }

    /// Название объекта n: поле подписей, поле name, иначе слой и номер.
    fn feature_name(&self, n: usize) -> String {
        let item = self.data.items.get(n);
        let field = self.label.or_else(|| self.data.fields.iter().position(|f| f.eq_ignore_ascii_case("name")));
        field
            .and_then(|f| item.and_then(|i| i.values.get(f)))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| format!("{} {}", self.data.name, n + 1))
    }

    /// Объекты слоя у точки (lat, lon). gap - зазор от точки объекта до
    /// пикселя щелчка, доля допуска, None - дальше допуска. Линия и контур
    /// многоугольника - ближе metres, многоугольник - и под точкой.
    /// Ближние раньше, не больше [`NEAR`].
    pub fn hits(&self, lat: f64, lon: f64, metres: f64, gap: &dyn Fn(f64, f64) -> Option<f64>) -> Vec<usize> {
        let metres = metres.max(1e-9);
        let outline = |parts: &[Vec<(f64, f64)>]| parts.iter().filter(|p| !p.is_empty()).map(|p| data::linesets::distance(p, lat, lon) / metres).reduce(f64::min).filter(|d| *d <= 1.0);
        let mut near: Vec<(f64, usize)> = Vec::new();
        for (n, item) in self.data.items.iter().enumerate().filter(|(n, _)| self.on(*n)) {
            let score = match item.shape {
                Geom::Point => item.parts.iter().filter_map(|p| p.first()).filter_map(|p| gap(p.0, p.1)).reduce(f64::min),
                Geom::Line => outline(&item.parts),
                Geom::Polygon => outline(&item.parts).or_else(|| inside_parts(&item.parts, lat, lon).then_some(1.0)),
                Geom::None => None,
            };
            if let Some(s) = score {
                near.push((s, n));
            }
        }
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        near.into_iter().take(NEAR).map(|(_, n)| n).collect()
    }

    /// Поля объекта n для окна «Объекты».
    pub fn feature(&self, lang: Lang, n: usize) -> Option<Feature> {
        let item = self.data.items.get(n)?;
        let kind = match item.shape {
            Geom::Point => tr(lang, "Point"),
            Geom::Line => tr(lang, "Line"),
            Geom::Polygon => tr(lang, "Polygon"),
            Geom::None => tr(lang, "Table"),
        };
        let mut values = vec![(tr(lang, "Type").to_string(), kind.to_string())];
        values.extend(self.data.fields.iter().cloned().zip(item.values.iter().cloned()));
        Some((self.feature_name(n), values))
    }
}

/// Что попросило меню строки слоя.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    Visible(bool),
    Zoom,
    Style(Style),
    Label(Option<usize>),
    Extrude(Option<(usize, f64)>),
    /// Слой в папку, пустое имя - вне папок.
    Folder(String),
    ToPlaces,
    /// Сохранить объекты слоя файлом GeoJSON.
    Export,
    /// Сохранить картинку слоя-картинки файлом GeoTIFF.
    ExportPicture,
    /// Слой - в проект открытого RustGIS.
    ToRustGis,
    /// Сторона шторки слоёв.
    Side(render::Side),
    /// Поля времени объектов: начало и конец, None - слой без времени.
    Time(Option<(usize, Option<usize>)>),
    Up,
    Down,
    /// Перетаскивание: слой - перед слоем before в папку folder, None -
    /// в конец папки или списка.
    Move { before: Option<u64>, folder: String },
    /// Щелчок по названию выделяет слой: ползунок прозрачности раздела
    /// - его.
    Select,
    /// Перечитать файл слоя с диска.
    Reload,
    Remove,
    /// Флажок строки папки: все её слои показать или спрятать.
    FolderVisible(String, bool),
    /// Свернуть или развернуть папку.
    FolderCollapse(String),
    /// Убрать с глобуса все слои папки.
    FolderRemove(String),
    /// Все слои папки - в проект открытого RustGIS: так уходят результаты
    /// окон анализа.
    FolderToRustGis(String),
}

/// Слои «Моих слоёв». Флаги *_dirty снимает тот, кто перестроил своё:
/// картинки наложения, точки, подписи, выдавленные сетки, хранилище.
#[derive(Default)]
pub struct DataLayers {
    pub list: Vec<DataLayer>,
    next: u64,
    pub lines_dirty: bool,
    pub dots_dirty: bool,
    pub labels_dirty: bool,
    pub shapes_dirty: bool,
    /// Список или слои изменились - записать в хранилище.
    pub save_dirty: bool,
    /// Ключ последнего выбора подписей, [`label_key`].
    pub labels_key: Option<(i64, i64, i64)>,
    /// Свёрнутые папки.
    pub collapsed: BTreeSet<String>,
    /// Файлы убранных слоёв, которые надо удалить из папки [`DIR`].
    pub dropped: Vec<String>,
    /// Выделенный слой раздела.
    pub selected: Option<u64>,
    /// Когда последний раз сверялись файлы слоёв, секунды кадра.
    checked_at: Option<f64>,
    /// Окно шкалы времени, по нему слои со временем прячут объекты.
    window: Option<(f64, f64)>,
    /// Охват шкалы по временам объектов слоёв, считается при смене слоёв.
    times: Option<(f64, f64)>,
}

impl DataLayers {
    /// Новое окно шкалы времени. У слоёв со временем объекты вне окна
    /// прячутся, картинки и надписи собираются заново.
    pub fn set_window(&mut self, window: Option<(f64, f64)>) {
        if window == self.window {
            return;
        }
        self.window = window;
        // Пересборка - только когда набор видимых объектов сменился:
        // шкала при проигрывании двигается каждый кадр, а сброс картинок
        // наложения каждый кадр не давал им собраться, слой пропадал.
        let mut moved = false;
        for l in self.list.iter_mut().filter(|l| l.time.is_some()) {
            let before = std::mem::take(&mut l.off);
            l.refresh_off(window);
            moved |= before != l.off;
        }
        if moved {
            self.lines_dirty = true;
            self.dots_dirty = true;
            self.labels_dirty = true;
            self.shapes_dirty = true;
        }
    }

    /// Слои одной папки - подряд, папка стоит на месте своего первого
    /// слоя. Тогда порядок строк раздела - порядок отрисовки, как у слоёв
    /// проекта QGIS: верхняя строка поверх.
    fn normalize(&mut self) {
        let mut rest = std::mem::take(&mut self.list);
        let mut out: Vec<DataLayer> = Vec::with_capacity(rest.len());
        while !rest.is_empty() {
            let first = rest.remove(0);
            if first.folder.is_empty() {
                out.push(first);
                continue;
            }
            let folder = first.folder.clone();
            out.push(first);
            let (same, other): (Vec<DataLayer>, Vec<DataLayer>) = rest.into_iter().partition(|l| l.folder == folder);
            out.extend(same);
            rest = other;
        }
        self.list = out;
    }

    /// Начало и конец (не включая) строки at в порядке раздела: слой вне
    /// папок - сам, слой чужой папки - вся папка.
    fn unit(&self, at: usize, own: &str) -> (usize, usize) {
        let folder = self.list.get(at).map_or("", |l| l.folder.as_str());
        if folder.is_empty() || folder == own {
            return (at, at + 1);
        }
        let start = self.list.iter().position(|l| l.folder == folder).unwrap_or(at);
        let end = self.list.iter().rposition(|l| l.folder == folder).map_or(at + 1, |e| e + 1);
        (start, end)
    }

    fn changed(&mut self) {
        self.normalize();
        let window = self.window;
        for l in &mut self.list {
            l.refresh_off(window);
        }
        self.times = self.list.iter().filter(|l| l.visible).filter_map(DataLayer::time_extent).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)));
        self.lines_dirty = true;
        self.dots_dirty = true;
        self.labels_dirty = true;
        self.shapes_dirty = true;
        self.save_dirty = true;
    }

    /// Слои файла file с геометрией - строками сверху списка. Таблицы
    /// без геометрии пропускаются. Возвращает количество слоёв.
    pub fn add(&mut self, layers: Vec<LayerData>, file: &str) -> usize {
        self.add_with(layers, file, "", None)
    }

    /// Слои файла с диска source - как [`DataLayers::add`], файл
    /// сверяется [`DataLayers::check_sources`].
    pub fn add_from(&mut self, layers: Vec<LayerData>, file: &str, source: Option<Source>) -> usize {
        self.add_with(layers, file, "", source)
    }

    /// Слои с геометрией в папку folder - строками сверху списка, как
    /// [`DataLayers::add`]. Сюда же ложатся результаты окон анализа.
    pub fn add_to(&mut self, layers: Vec<LayerData>, file: &str, folder: &str) -> usize {
        self.add_with(layers, file, folder, None)
    }

    fn add_with(&mut self, layers: Vec<LayerData>, file: &str, folder: &str, source: Option<Source>) -> usize {
        let shown: Vec<LayerData> = layers.into_iter().filter(|l| l.items.iter().any(|i| i.shape != Geom::None)).collect();
        let single = shown.len() == 1;
        let count = shown.len();
        for (k, data) in shown.into_iter().enumerate() {
            let title = if single { file.to_string() } else { format!("{file} · {}", data.name) };
            let style = Style::new(self.next as usize);
            self.next += 1;
            let layer = DataLayer {
                id: self.next,
                title,
                file: file.to_string(),
                data,
                visible: true,
                style,
                label: None,
                extrude: None,
                folder: folder.to_string(),
                store: None,
                live: None,
                colors: Vec::new(),
                source: source.clone(),
                stale: false,
                picture: None,
                about: String::new(),
                side: render::Side::Both,
                time: None,
                off: Vec::new(),
            };
            self.list.insert(k, layer);
        }
        if count > 0 {
            self.collapsed.remove(folder);
            self.changed();
        }
        count
    }

    pub fn get(&self, id: u64) -> Option<&DataLayer> {
        self.list.iter().find(|l| l.id == id)
    }

    /// Растр окна анализа - слоем-картинкой в папку folder, под её
    /// векторными слоями: линии и контуры видны поверх картинки.
    pub fn add_picture(&mut self, overlay: scene::kml::Overlay, folder: &str) -> u64 {
        self.next += 1;
        let mut style = Style::new(self.next as usize);
        style.opacity = f32::from(overlay.overlay.color[3]) / 255.0;
        let at = self.list.iter().rposition(|l| l.folder == folder && !folder.is_empty()).map_or(0, |p| p + 1);
        let layer = DataLayer {
            id: self.next,
            title: overlay.name.clone(),
            file: overlay.name.clone(),
            data: LayerData { name: overlay.name.clone(), epsg: Some(4326), fields: Vec::new(), numeric: Vec::new(), items: Vec::new() },
            visible: true,
            style,
            label: None,
            extrude: None,
            folder: folder.to_string(),
            store: None,
            live: None,
            colors: Vec::new(),
            source: None,
            stale: false,
            about: overlay.description.clone(),
            side: render::Side::Both,
            time: None,
            off: Vec::new(),
            picture: Some(overlay),
        };
        self.list.insert(at, layer);
        self.collapsed.remove(folder);
        self.changed();
        self.next
    }

    /// Слой-картинка id файлом GeoTIFF: имя файла и байты. Ошибка - ключ
    /// перевода.
    pub fn picture_geotiff(&self, id: u64) -> Result<(String, Vec<u8>), &'static str> {
        let layer = self.get(id).ok_or("The picture of the layer is not loaded")?;
        let overlay = layer.picture.as_ref().ok_or("The picture of the layer is not loaded")?;
        Ok((format!("{}.tif", data::layers::stem(&layer.title)), picture_geotiff(overlay)?))
    }

    /// Описание about слою title папки folder - подсказка его строки.
    pub fn set_about(&mut self, folder: &str, title: &str, about: &str) {
        if let Some(l) = self.list.iter_mut().find(|l| l.folder == folder && l.title == title) {
            l.about = about.to_string();
            self.save_dirty = true;
        }
    }

    /// Отпечаток слоёв-картинок: номера, флажки, непрозрачность, порядок
    /// и видимость в окне шкалы времени window. Сменился - сетки картинок
    /// собираются заново. Без картинок - 0.
    pub fn pictures_version(&self, window: Option<(f64, f64)>) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let mut any = false;
        for l in &self.list {
            if let Some(p) = &l.picture {
                any = true;
                (l.id, l.visible, p.overlay.color[3], in_window(p, window), side_word(l.side)).hash(&mut h);
            }
        }
        if any { h.finish() | 1 } else { 0 }
    }

    /// Картинки видимых слоёв-картинок в окне шкалы времени window, без
    /// окна - все, нижняя строка - первой. Картинка без времени видна
    /// всегда, как метка.
    pub fn pictures(&self, window: Option<(f64, f64)>) -> Vec<&scene::kml::Overlay> {
        self.picture_sides(window).into_iter().map(|(p, _)| p).collect()
    }

    /// Картинки, как [`DataLayers::pictures`], со стороной шторки слоя.
    pub fn picture_sides(&self, window: Option<(f64, f64)>) -> Vec<(&scene::kml::Overlay, render::Side)> {
        self.list.iter().rev().filter(|l| l.visible).filter_map(|l| l.picture.as_ref().map(|p| (p, l.side))).filter(|(p, _)| in_window(p, window)).collect()
    }

    /// Охват шкалы времени по временам слоёв-картинок и объектов слоёв
    /// со временем, как у меток.
    pub fn extent(&self) -> Option<(f64, f64)> {
        let pictures = scene::when::extent(self.list.iter().filter(|l| l.visible).filter_map(|l| l.picture.as_ref()).map(|p| p.time.as_ref()));
        [pictures, self.times].into_iter().flatten().reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
    }

    /// Папки в порядке первого слоя каждой, без пустой. Живые слои RustGIS
    /// - в своём разделе панели, их папки здесь нет.
    pub fn folders(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for l in self.list.iter().filter(|l| l.live.is_none()) {
            if !l.folder.is_empty() && !out.contains(&l.folder) {
                out.push(l.folder.clone());
            }
        }
        out
    }

    /// Убрать слой с места at, файл его объектов - в [`DataLayers::dropped`].
    fn take(&mut self, at: usize) -> DataLayer {
        let layer = self.list.remove(at);
        if let Some(name) = &layer.store {
            self.dropped.push(name.clone());
        }
        layer
    }

    /// Применить действие меню к слою id или к папке. Возвращает
    /// убранные слои.
    pub fn apply(&mut self, id: u64, act: &Act) -> Vec<DataLayer> {
        let mut removed = Vec::new();
        match act {
            Act::FolderVisible(folder, on) => self.list.iter_mut().filter(|l| &l.folder == folder).for_each(|l| l.visible = *on),
            Act::FolderCollapse(folder) => {
                if !self.collapsed.remove(folder) {
                    self.collapsed.insert(folder.clone());
                }
                self.save_dirty = true;
                return removed;
            }
            Act::FolderRemove(folder) => {
                while let Some(at) = self.list.iter().position(|l| &l.folder == folder) {
                    removed.push(self.take(at));
                }
                self.collapsed.remove(folder);
            }
            _ => {
                let Some(at) = self.list.iter().position(|l| l.id == id) else {
                    return removed;
                };
                match act {
                    Act::Select => {
                        self.selected = Some(id);
                        return removed;
                    }
                    Act::Visible(on) => self.list[at].visible = *on,
                    Act::Style(s) => {
                        let layer = &mut self.list[at];
                        layer.style = *s;
                        if let Some(p) = &mut layer.picture {
                            p.overlay.color[3] = (s.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
                        }
                    }
                    Act::Label(f) => self.list[at].label = *f,
                    Act::Extrude(e) => self.list[at].extrude = *e,
                    Act::Side(s) => self.list[at].side = *s,
                    Act::Time(t) => self.list[at].time = *t,
                    Act::Folder(f) => self.list[at].folder = f.trim().to_string(),
                    // Вверх и вниз - через соседнюю строку раздела: чужая
                    // папка проходится целиком.
                    Act::Up if at > 0 => {
                        let own = self.list[at].folder.clone();
                        let (start, _) = self.unit(at - 1, &own);
                        let layer = self.list.remove(at);
                        self.list.insert(start, layer);
                    }
                    Act::Down if at + 1 < self.list.len() => {
                        let own = self.list[at].folder.clone();
                        let (_, end) = self.unit(at + 1, &own);
                        let layer = self.list.remove(at);
                        self.list.insert((end - 1).min(self.list.len()), layer);
                    }
                    Act::Move { before, folder } => {
                        if *before == Some(id) {
                            return removed;
                        }
                        let mut layer = self.list.remove(at);
                        layer.folder = folder.trim().to_string();
                        let end_of_folder = || self.list.iter().rposition(|l| !layer.folder.is_empty() && l.folder == layer.folder).map(|p| p + 1);
                        let to = before.and_then(|b| self.list.iter().position(|l| l.id == b)).or_else(end_of_folder).unwrap_or(self.list.len());
                        self.list.insert(to, layer);
                    }
                    Act::Remove => removed.push(self.take(at)),
                    _ => return removed,
                }
            }
        }
        self.changed();
        removed
    }

    /// Список слоёв текстом JSON для хранилища: название, файл, папка,
    /// стиль, подписи, выдавливание и имя файла объектов. inline - объекты
    /// слоя текстом GeoJSON прямо в списке, как на странице. Свёрнутые
    /// папки - тоже в списке.
    pub fn index(&self, inline: bool) -> String {
        let layers: Vec<serde_json::Value> = self
            .list
            .iter()
            .filter(|l| l.live.is_none())
            // Картинки на странице не хранятся: строке хранилища они
            // велики.
            .filter(|l| !(inline && l.picture.is_some()))
            .map(|l| {
                let s = &l.style;
                let mut v = serde_json::json!({
                    "title": l.title, "file": l.file, "name": l.data.name, "folder": l.folder, "visible": l.visible,
                    "color": s.color, "width": s.width, "fill": s.fill, "opacity": s.opacity, "size": s.size,
                    "label": l.label, "extrude": l.extrude.map(|(f, k)| serde_json::json!([f, k])),
                });
                if !l.about.is_empty() {
                    v["about"] = serde_json::Value::String(l.about.clone());
                }
                if l.side != render::Side::Both {
                    v["side"] = serde_json::Value::String(side_word(l.side).to_string());
                }
                if let Some((start, end)) = l.time {
                    v["time"] = serde_json::json!([start, end]);
                }
                if let Some(p) = &l.picture {
                    v["picture"] = serde_json::json!({ "corners": p.overlay.corners, "box": p.overlay.latlon_box, "description": p.description });
                    if let Some((begin, end)) = &p.time {
                        v["picture"]["time"] = serde_json::json!([begin, end]);
                    }
                    if let Some(store) = &l.store {
                        v["store"] = serde_json::Value::String(store.clone());
                    }
                } else if inline {
                    v["data"] = serde_json::Value::String(data::layers::to_geojson(&l.data));
                } else if let Some(store) = &l.store {
                    v["store"] = serde_json::Value::String(store.clone());
                }
                if let Some(s) = &l.source {
                    v["path"] = serde_json::Value::String(s.path.clone());
                    v["modified"] = serde_json::Value::from(s.modified);
                }
                v
            })
            .collect();
        serde_json::json!({ "layers": layers, "collapsed": self.collapsed }).to_string()
    }

    /// Слои прошлого запуска saved - под слоями, открытыми до них, с новыми
    /// номерами. Свёрнутые папки - из saved.
    pub fn restore(&mut self, saved: DataLayers) {
        for mut layer in saved.list {
            self.next += 1;
            layer.id = self.next;
            self.list.push(layer);
        }
        self.collapsed.extend(saved.collapsed);
        self.changed();
    }

    /// Слои без записанных объектов: номер слоя и имя файла, которое ему
    /// даётся. Имя записывается в слой - после записи файла.
    pub fn unstored(&self) -> Vec<(u64, String)> {
        let ms = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        self.list.iter().filter(|l| l.store.is_none() && l.live.is_none()).map(|l| (l.id, format!("layer-{ms}-{}.{}", l.id, if l.picture.is_some() { "png" } else { "geojson" }))).collect()
    }

    /// Байты файла объектов слоя для папки [`DIR`]: картинка слоя-картинки
    /// или текст GeoJSON.
    pub fn stored_bytes(layer: &DataLayer) -> Vec<u8> {
        match &layer.picture {
            Some(p) => p.image.clone().unwrap_or_default(),
            None => data::layers::to_geojson(&layer.data).into_bytes(),
        }
    }

    /// Объекты слоя id записаны файлом name.
    pub fn stored(&mut self, id: u64, name: String) {
        if let Some(l) = self.list.iter_mut().find(|l| l.id == id) {
            l.store = Some(name);
        }
    }

    /// Слои из списка text ([`DataLayers::index`]). read - байты файла
    /// объектов по имени. Слой, чьи объекты не прочлись, пропускается,
    /// его название - во втором значении.
    pub fn from_index(text: &str, read: &dyn Fn(&str) -> Option<Vec<u8>>) -> (DataLayers, Vec<String>) {
        let mut out = DataLayers::default();
        let mut lost = Vec::new();
        let Ok(root) = serde_json::from_str::<serde_json::Value>(text) else {
            return (out, lost);
        };
        let f32_of = |v: &serde_json::Value, key: &str, default: f32| v.get(key).and_then(serde_json::Value::as_f64).map_or(default, |x| x as f32);
        for v in root.get("layers").and_then(serde_json::Value::as_array).into_iter().flatten() {
            let text_of = |key: &str| v.get(key).and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
            let title = text_of("title");
            let store = v.get("store").and_then(serde_json::Value::as_str).map(str::to_string);
            let bytes = match (v.get("data").and_then(serde_json::Value::as_str), &store) {
                (Some(inline), _) => Some(inline.as_bytes().to_vec()),
                (None, Some(name)) => read(name),
                (None, None) => None,
            };
            // Слой-картинка: картинка из файла, рамка из списка.
            let picture = match v.get("picture") {
                Some(p) => {
                    let Some(image) = bytes.clone().filter(|b| !b.is_empty()) else {
                        lost.push(title);
                        continue;
                    };
                    let mut ground = scene::overlays::Overlay::new(scene::overlays::Kind::Ground);
                    let floats = |key: &str| p.get(key).and_then(serde_json::Value::as_array).map(|a| a.iter().filter_map(serde_json::Value::as_f64).collect::<Vec<f64>>());
                    match floats("box").filter(|b| b.len() == 5) {
                        Some(b) => ground.set_box([b[0], b[1], b[2], b[3], b[4]]),
                        None => {
                            let corners = p.get("corners").and_then(serde_json::Value::as_array).map(|c| c.iter().filter_map(|x| Some((x.get(0)?.as_f64()?, x.get(1)?.as_f64()?))).collect::<Vec<_>>()).unwrap_or_default();
                            ground.set_corners(corners);
                        }
                    }
                    let mut overlay = scene::kml::Overlay::new(&title, ground);
                    overlay.image = Some(image);
                    overlay.description = p.get("description").and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
                    overlay.time = p.get("time").and_then(serde_json::Value::as_array).and_then(|t| Some((t.first()?.as_str()?.to_string(), t.get(1)?.as_str()?.to_string())));
                    Some(overlay)
                }
                None => None,
            };
            let parsed = if picture.is_some() {
                Some(LayerData { name: String::new(), epsg: Some(4326), fields: Vec::new(), numeric: Vec::new(), items: Vec::new() })
            } else {
                bytes.and_then(|b| data::layers::read(b, "layer.geojson").ok()).and_then(|mut l| (!l.is_empty()).then(|| l.remove(0)))
            };
            let Some(mut data) = parsed else {
                lost.push(title);
                continue;
            };
            data.name = text_of("name");
            let mut style = Style::new(out.next as usize);
            if let Some(c) = v.get("color").and_then(serde_json::Value::as_array).filter(|c| c.len() == 3) {
                let ch = |n: usize| c[n].as_u64().map_or(0, |x| x.min(255) as u8);
                style.color = [ch(0), ch(1), ch(2)];
            }
            style.width = f32_of(v, "width", style.width);
            style.fill = f32_of(v, "fill", style.fill);
            style.opacity = f32_of(v, "opacity", style.opacity);
            style.size = f32_of(v, "size", style.size);
            let fields = data.fields.len();
            let label = v.get("label").and_then(serde_json::Value::as_u64).map(|n| n as usize).filter(|n| *n < fields);
            let extrude = v
                .get("extrude")
                .and_then(serde_json::Value::as_array)
                .and_then(|e| Some((e.first()?.as_u64()? as usize, e.get(1)?.as_f64()?)))
                .filter(|(n, _)| *n < fields);
            out.next += 1;
            let visible = v.get("visible").and_then(serde_json::Value::as_bool).unwrap_or(true);
            let path = text_of("path");
            let source = v.get("modified").and_then(serde_json::Value::as_u64).filter(|_| !path.is_empty()).map(|modified| Source { path, modified });
            out.list.push(DataLayer {
                id: out.next,
                title,
                file: text_of("file"),
                data,
                visible,
                style,
                label,
                extrude,
                folder: text_of("folder"),
                store,
                live: None,
                colors: Vec::new(),
                source,
                stale: false,
                picture: picture.map(|mut p| {
                    p.overlay.color[3] = (style.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
                    p
                }),
                about: text_of("about"),
                side: side_of(&text_of("side")),
                time: time_of(v),
                off: Vec::new(),
            });
        }
        out.collapsed = root.get("collapsed").and_then(serde_json::Value::as_array).into_iter().flatten().filter_map(|f| f.as_str().map(str::to_string)).collect();
        if !out.list.is_empty() {
            out.changed();
        }
        out.save_dirty = false;
        (out, lost)
    }

    /// Сверить файлы слоёв с диском не чаще [`SOURCE_CHECK`]: слой, чей
    /// файл изменился после чтения, получает строку «Обновить». modified -
    /// время изменения файла по пути, None - файла нет. true - отметки
    /// изменились.
    pub fn check_sources(&mut self, now: f64, modified: &dyn Fn(&str) -> Option<u64>) -> bool {
        if self.checked_at.is_some_and(|at| now - at < SOURCE_CHECK && now >= at) {
            return false;
        }
        self.checked_at = Some(now);
        let mut changed = false;
        for l in &mut self.list {
            let Some(s) = &l.source else {
                continue;
            };
            let stale = modified(&s.path).is_some_and(|m| m != s.modified);
            changed |= stale != l.stale;
            l.stale = stale;
        }
        changed
    }

    /// Перечитанный файл слоя id: из слоёв файла layers берётся слой
    /// с тем же именем, у файла с одним слоем - он. Стиль, папка и место
    /// остаются, поля подписей и выдавливания - если они ещё есть. false -
    /// подходящего слоя в файле нет.
    pub fn reload(&mut self, id: u64, layers: Vec<LayerData>, modified: u64) -> bool {
        let Some(at) = self.list.iter().position(|l| l.id == id) else {
            return false;
        };
        let single = layers.len() == 1;
        let name = self.list[at].data.name.clone();
        let Some(mut data) = layers.into_iter().find(|d| single || d.name == name) else {
            return false;
        };
        data.name = name;
        let fields = data.fields.len();
        let numeric = |n: usize| data.numeric.get(n).copied().unwrap_or(false);
        let extrude = self.list[at].extrude.filter(|(n, _)| *n < fields && numeric(*n));
        let layer = &mut self.list[at];
        layer.label = layer.label.filter(|n| *n < fields);
        layer.extrude = extrude;
        layer.data = data;
        layer.stale = false;
        if let Some(s) = &mut layer.source {
            s.modified = modified;
        }
        // Объекты в хранилище - заново.
        if let Some(old) = layer.store.take() {
            self.dropped.push(old);
        }
        self.changed();
        true
    }

    /// Линии видимых слоёв для картинки наложения: нижняя строка
    /// списка - внизу стопки.
    pub fn line_set(&self) -> Option<Arc<Vec<Line>>> {
        // Слой со стороной шторки - объектами на рельефе (`shapes`).
        let lines: Vec<Line> = self.list.iter().rev().filter(|l| l.visible && l.side == render::Side::Both).flat_map(DataLayer::lines).collect();
        (!lines.is_empty()).then(|| Arc::new(lines))
    }

    /// Точки видимых слоёв: точки, цвет, размер, непрозрачность, сторона
    /// шторки.
    pub fn dot_sets(&self) -> Vec<(DotSet, render::Side)> {
        self.list
            .iter()
            .rev()
            .filter(|l| l.visible)
            .map(|l| ((l.dots(), l.style.color, l.style.size, l.style.opacity), l.side))
            .filter(|((d, _, _, _), _)| !d.is_empty())
            .collect()
    }

    /// Есть слой со стороной шторки: видна кнопка «Шторка слоёв».
    pub fn has_sides(&self) -> bool {
        self.list.iter().any(|l| l.side != render::Side::Both)
    }

    /// Подписи видимых слоёв у точки взгляда (lat, lon).
    pub fn labels(&self, lat: f64, lon: f64) -> Vec<(String, f64, f64)> {
        self.list.iter().filter(|l| l.visible).flat_map(|l| nearest(l.label_spots(), lat, lon, MAX_LABELS)).collect()
    }

    /// Объекты видимых слоёв на рельефе со стороной шторки: выдавленные
    /// и все линии и многоугольники слоя со стороной.
    pub fn shapes(&self, radius: f64) -> Vec<(Shape, render::Side)> {
        self.list
            .iter()
            .filter(|l| l.visible)
            .flat_map(|l| {
                let flat = if l.side == render::Side::Both { Vec::new() } else { l.flat_shapes() };
                l.extruded(radius).into_iter().chain(flat).map(move |s| (s, l.side))
            })
            .collect()
    }

    /// Группы окна «Объекты» у точки (lat, lon): название слоя и его
    /// объекты.
    pub fn identify(&self, lang: Lang, lat: f64, lon: f64, metres: f64, gap: &dyn Fn(f64, f64) -> Option<f64>) -> Vec<(String, Vec<Feature>)> {
        self.list
            .iter()
            .filter(|l| l.visible)
            .filter_map(|l| {
                let features: Vec<Feature> = l.hits(lat, lon, metres, gap).into_iter().filter_map(|n| l.feature(lang, n)).collect();
                (!features.is_empty()).then(|| (l.title.clone(), features))
            })
            .collect()
    }

    /// Живой слой RustGIS номер live в папке folder: заменяется на месте,
    /// новый - сверху папки и выключен, как слои проекта QGIS в модуле:
    /// глобус показывает его, когда пользователь поставит флажок в разделе
    /// «Проект RustGIS». Стиль и подписи - из RustGIS.
    pub fn set_live(&mut self, folder: &str, live: i64, title: &str, layer: (LayerData, Vec<[u8; 3]>), style: Style, label: Option<usize>) {
        let (data, colors) = layer;
        if let Some(l) = self.list.iter_mut().find(|l| l.live == Some(live)) {
            l.title = title.to_string();
            l.data = data;
            l.colors = colors;
            // Прозрачность ставит пользователь глобуса, новый стиль её
            // не сбрасывает.
            l.style = Style { opacity: l.style.opacity, ..style };
            l.label = label;
        } else {
            self.next += 1;
            let at = self.list.iter().position(|l| l.folder == folder).unwrap_or(0);
            let layer = DataLayer {
                id: self.next,
                title: title.to_string(),
                file: title.to_string(),
                data,
                visible: false,
                style,
                label,
                extrude: None,
                folder: folder.to_string(),
                store: None,
                live: Some(live),
                colors,
                source: None,
                stale: false,
                picture: None,
                about: String::new(),
                side: render::Side::Both,
                time: None,
                off: Vec::new(),
            };
            self.list.insert(at, layer);
        }
        self.changed();
    }

    /// Поле подписей живого слоя RustGIS номер live: Some(None) - без
    /// подписей, None - слоя нет.
    pub fn live_label(&self, live: i64) -> Option<Option<usize>> {
        self.list.iter().find(|l| l.live == Some(live)).map(|l| l.label)
    }

    /// Живые слои - по проекту RustGIS: order - номера слоёв сверху вниз.
    /// Слоя нет в order - он убирается, порядок - как в RustGIS. Флажки
    /// ставит пользователь в разделе «Проект RustGIS». Возвращает true,
    /// если что-то изменилось.
    pub fn sync_live(&mut self, order: &[i64]) -> bool {
        let before: Vec<Option<i64>> = self.list.iter().map(|l| l.live).collect();
        self.list.retain(|l| l.live.is_none_or(|id| order.contains(&id)));
        // Живые слои занимают свои места в списке в порядке RustGIS.
        let slots: Vec<usize> = (0..self.list.len()).filter(|&i| self.list[i].live.is_some()).collect();
        let rank = |l: &DataLayer| l.live.and_then(|id| order.iter().position(|o| *o == id)).unwrap_or(usize::MAX);
        let mut live: Vec<DataLayer> = Vec::new();
        for &i in slots.iter().rev() {
            live.push(self.list.remove(i));
        }
        live.sort_by_key(rank);
        for (slot, layer) in slots.into_iter().zip(live) {
            self.list.insert(slot, layer);
        }
        let after: Vec<Option<i64>> = self.list.iter().map(|l| l.live).collect();
        let changed = before != after;
        if changed {
            self.changed();
        }
        changed
    }

    /// Показать или спрятать живой слой RustGIS номер live.
    pub fn show_live(&mut self, live: i64, on: bool) {
        if let Some(l) = self.list.iter_mut().find(|l| l.live == Some(live))
            && l.visible != on
        {
            l.visible = on;
            self.changed();
        }
    }

    /// Непрозрачность живого слоя RustGIS номер live, 0..1.
    pub fn live_opacity(&self, live: i64) -> Option<f32> {
        self.list.iter().find(|l| l.live == Some(live)).map(|l| l.style.opacity)
    }

    pub fn set_live_opacity(&mut self, live: i64, opacity: f32) {
        if let Some(l) = self.list.iter_mut().find(|l| l.live == Some(live))
            && l.style.opacity != opacity
        {
            l.style.opacity = opacity.clamp(0.05, 1.0);
            self.changed();
        }
    }

    /// Показан ли живой слой RustGIS номер live; None - объекты ещё не
    /// пришли.
    pub fn live_shown(&self, live: i64) -> Option<bool> {
        self.list.iter().find(|l| l.live == Some(live)).map(|l| l.visible)
    }

    /// Живой слой RustGIS номер live.
    pub fn live_layer(&self, live: i64) -> Option<&DataLayer> {
        self.list.iter().find(|l| l.live == Some(live))
    }

    /// Убрать все живые слои: связь выключена.
    pub fn drop_live(&mut self) {
        let had = self.list.iter().any(|l| l.live.is_some());
        self.list.retain(|l| l.live.is_none());
        if had {
            self.changed();
        }
    }

    /// Первый объект живого видимого слоя у точки: номер слоя RustGIS
    /// и номер объекта, он же строка слоя в RustGIS.
    pub fn live_hit(&self, lat: f64, lon: f64, metres: f64, gap: &dyn Fn(f64, f64) -> Option<f64>) -> Option<(i64, usize)> {
        self.list.iter().filter(|l| l.visible).find_map(|l| Some((l.live?, *l.hits(lat, lon, metres, gap).first()?)))
    }
}

/// Что тянут мышью в разделе: номер слоя.
#[derive(Debug, Clone, Copy)]
struct Dragged(u64);

/// Сброс перетаскиваемого слоя на строку rect: черта места - над строкой,
/// отпущенный слой - перед before в папку folder.
fn drop_target(ui: &egui::Ui, response: &egui::Response, before: Option<u64>, folder: &str, out: &mut Vec<(u64, Act)>) {
    if response.dnd_hover_payload::<Dragged>().is_some() {
        let r = response.rect;
        ui.painter().hline(r.x_range(), r.top(), egui::Stroke::new(2.0, crate::theme::pal().accent));
    }
    if let Some(d) = response.dnd_release_payload::<Dragged>() {
        out.push((d.0, Act::Move { before, folder: folder.to_string() }));
    }
}

/// Количество строк раздела «Мои слои» - для его высоты.
pub fn row_count(layers: &DataLayers) -> usize {
    let own = || layers.list.iter().filter(|l| l.live.is_none());
    let folders = layers.folders();
    let hidden = own().filter(|l| layers.collapsed.contains(&l.folder)).count();
    own().count() + folders.len() - hidden
}

/// Строки раздела «Мои слои» в порядке отрисовки, верхняя строка поверх:
/// слои вне папок и папки со своими слоями, как «Проект QGIS» модуля.
/// Строку слоя можно перетащить на другое место и в другую папку. Щелчок
/// по названию выделяет слой, двойной - перелёт к нему, «⋯» и правая
/// кнопка - меню. Под строками - прозрачность выделенного слоя.
/// menu_open - открытое меню раздела, new_folder - поле имени новой
/// папки, out - действия по номерам слоёв.
pub fn rows(ui: &mut egui::Ui, lang: Lang, layers: &DataLayers, menu_open: &mut Option<String>, new_folder: &mut String, out: &mut Vec<(u64, Act)>) {
    let p = crate::theme::pal();
    if layers.list.iter().all(|l| l.live.is_some()) {
        ui.label(egui::RichText::new(tr(lang, "Open or drop a GeoPackage, Shapefile, GeoJSON, GPX, MapInfo, FlatGeobuf or DXF file: its layers come here. Results of the analysis windows come here too.")).color(p.muted));
        return;
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    if ui.ctx().dragged_id().is_some() && egui::DragAndDrop::has_payload_of_type::<Dragged>(ui.ctx()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    let folders = layers.folders();
    let own: Vec<&DataLayer> = layers.list.iter().filter(|l| l.live.is_none()).collect();
    let mut at = 0;
    while let Some(first) = own.get(at) {
        if first.folder.is_empty() {
            layer_row(ui, lang, layers, first, &folders, 4.0, menu_open, new_folder, out);
            at += 1;
            continue;
        }
        let folder = &first.folder;
        let inside: Vec<&DataLayer> = own.iter().skip(at).take_while(|l| &l.folder == folder).copied().collect();
        at += inside.len().max(1);
        let shown = inside.iter().filter(|l| l.visible).count();
        let state = if shown == inside.len() { Some(true) } else if shown == 0 { Some(false) } else { None };
        let open = !layers.collapsed.contains(folder);
        let title = format!("{} {folder}", if open { "▾" } else { "▸" });
        let tip = trf(lang, "Folder of layers: {count}. The check shows or hides all its layers, a click on the name folds the folder.", &[("count", &inside.len().to_string())]);
        let hit = crate::layers::row(ui, 4.0, Some(state), false, &title, "⋯", &tip);
        // Слой, отпущенный на строку папки, - в начало папки.
        drop_target(ui, &hit.response, inside.first().map(|l| l.id), folder, out);
        if hit.check {
            out.push((0, Act::FolderVisible(folder.clone(), state != Some(true))));
        }
        if hit.text {
            out.push((0, Act::FolderCollapse(folder.clone())));
        }
        let key = format!("data-folder:{folder}");
        if hit.menu {
            *menu_open = Some(key.clone());
        }
        crate::layers::popup(ui, menu_open, &key, hit.rect, |ui| {
            if RUSTGIS_LINKED.load(std::sync::atomic::Ordering::Relaxed)
                && ui.button(tr(lang, "Folder to the RustGIS project")).on_hover_text(tr(lang, "Every layer of the folder goes to the open RustGIS project as a new layer, with all fields.")).clicked()
            {
                out.push((0, Act::FolderToRustGis(folder.clone())));
                return true;
            }
            if ui.button(tr(lang, "Remove the folder from the globe")).on_hover_text(tr(lang, "All layers of the folder leave «My layers».")).clicked() {
                out.push((0, Act::FolderRemove(folder.clone())));
                return true;
            }
            false
        });
        if open {
            for layer in inside {
                layer_row(ui, lang, layers, layer, &folders, 22.0, menu_open, new_folder, out);
            }
        }
    }
    // Место под строками: слой, отпущенный здесь, - в конец вне папок.
    let (_, end) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 10.0), egui::Sense::hover());
    drop_target(ui, &end, None, "", out);
    opacity_row(ui, lang, layers, out);
}

/// Ползунок непрозрачности выделенного слоя под строками, как ползунок
/// под деревом «Моих меток» и под слоями проекта у модуля.
fn opacity_row(ui: &mut egui::Ui, lang: Lang, layers: &DataLayers, out: &mut Vec<(u64, Act)>) {
    let selected = layers.selected.and_then(|id| layers.get(id)).filter(|l| l.live.is_none());
    ui.horizontal(|ui| {
        let mut value = selected.map_or(100.0, |l| (l.style.opacity * 100.0).round());
        // Ширина ползунка - доступная без подписи значения, измеренной
        // по шрифту. Строка шире панели раздвигала панель каждый кадр,
        // пока та не занимала всё окно.
        let text = format!("{value:.0} %");
        let font = egui::TextStyle::Body.resolve(ui.style());
        let label_width = ui.painter().layout_no_wrap(text.clone(), font, egui::Color32::WHITE).size().x;
        let gap = ui.spacing().item_spacing.x;
        ui.spacing_mut().slider_width = (ui.available_width() - label_width - 2.0 * gap - 4.0).max(40.0);
        let tip = match selected {
            Some(l) => trf(lang, "Opacity of the layer {layer}. To the right the layer is opaque.", &[("layer", &l.title)]),
            None => tr(lang, "A click on the name of a layer selects it, the slider sets its opacity.").to_string(),
        };
        let slider = egui::Slider::new(&mut value, 5.0..=100.0).show_value(false);
        let changed = ui.add_enabled(selected.is_some(), slider).on_hover_text(tip).changed();
        ui.label(text);
        if let (true, Some(l)) = (changed, selected) {
            let mut style = l.style;
            style.opacity = value / 100.0;
            out.push((l.id, Act::Style(style)));
        }
    });
}

/// Строка слоя с отступом indent и её меню.
#[allow(clippy::too_many_arguments)]
fn layer_row(ui: &mut egui::Ui, lang: Lang, layers: &DataLayers, layer: &DataLayer, folders: &[String], indent: f32, menu_open: &mut Option<String>, new_folder: &mut String, out: &mut Vec<(u64, Act)>) {
    let count = layer.data.items.len();
    let mut tip = trf(
        lang,
        "{file}, layer {layer}, objects: {count}. The check hides the layer, a double click flies to it, the row can be dragged to another place, the «⋯» menu sets its style, labels, extrusion and folder.",
        &[("file", &layer.file), ("layer", &layer.data.name), ("count", &count.to_string())],
    );
    if !layer.about.is_empty() {
        tip = format!("{}\n\n{tip}", layer.about);
    }
    let hit = crate::layers::drag_row(ui, layers.selected == Some(layer.id), indent, Some(Some(layer.visible)), &layer.title, "⋯", &tip);
    if hit.response.dragged() {
        hit.response.dnd_set_drag_payload(Dragged(layer.id));
    }
    drop_target(ui, &hit.response, Some(layer.id), &layer.folder, out);
    if hit.check {
        out.push((layer.id, Act::Visible(!layer.visible)));
    }
    if hit.text {
        out.push((layer.id, Act::Select));
    }
    if hit.response.double_clicked() {
        out.push((layer.id, Act::Zoom));
    }
    // Файл слоя изменился на диске - строка «Обновить», как у слоёв
    // проекта модуля при смене проекта.
    if layer.stale {
        ui.horizontal(|ui| {
            ui.add_space(indent + 20.0);
            if ui.link(tr(lang, "Refresh|layer")).on_hover_text(tr(lang, "The file of the layer changed on the disk. A click reads it again, the style and the folder stay.")).clicked() {
                out.push((layer.id, Act::Reload));
            }
            ui.label(egui::RichText::new(tr(lang, "the file changed")).color(crate::theme::pal().muted));
        });
    }
    let key = format!("data:{}", layer.id);
    if hit.menu {
        *menu_open = Some(key.clone());
    }
    crate::layers::popup(ui, menu_open, &key, hit.rect, |ui| menu(ui, lang, layer, folders, new_folder, out));
}

/// Подменю «Папка»: вне папок, папки списка и новая папка по имени.
/// true - папка выбрана.
fn folder_menu(ui: &mut egui::Ui, lang: Lang, layer: &DataLayer, folders: &[String], new_folder: &mut String, out: &mut Vec<(u64, Act)>) -> bool {
    let mut chosen = false;
    let name = if layer.folder.is_empty() { tr(lang, "No") } else { layer.folder.as_str() };
    ui.menu_button(trf(lang, "Folder: {folder}", &[("folder", name)]), |ui| {
        if ui.radio(layer.folder.is_empty(), tr(lang, "Outside folders")).clicked() {
            out.push((layer.id, Act::Folder(String::new())));
            chosen = true;
        }
        for f in folders {
            if ui.radio(&layer.folder == f, f.as_str()).clicked() {
                out.push((layer.id, Act::Folder(f.clone())));
                chosen = true;
            }
        }
    })
    .response
    .on_hover_text(tr(lang, "Folders of one level gather layers: a check of the folder shows or hides them all."));
    // Поле новой папки - в самом меню: щелчок в подменю закрыл бы меню
    // строки, подменю лежит вне его рамки.
    ui.horizontal(|ui| {
        let edit = ui.add(egui::TextEdit::singleline(new_folder).desired_width(140.0).hint_text(tr(lang, "New folder")));
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let ok = !new_folder.trim().is_empty();
        if (ui.add_enabled(ok, egui::Button::new(tr(lang, "Move"))).on_hover_text(tr(lang, "The layer moves to a new folder with this name.")).clicked() || enter) && ok {
            out.push((layer.id, Act::Folder(std::mem::take(new_folder))));
            chosen = true;
        }
    });
    chosen
}

/// Меню строки слоя. true - пункт выбран, меню закрывается.
fn menu(ui: &mut egui::Ui, lang: Lang, layer: &DataLayer, folders: &[String], new_folder: &mut String, out: &mut Vec<(u64, Act)>) -> bool {
    let id = layer.id;
    if ui.button(tr(lang, "Zoom to layer")).clicked() {
        out.push((id, Act::Zoom));
        return true;
    }
    ui.separator();
    if layer.picture.is_some() {
        return picture_menu(ui, lang, layer, folders, new_folder, out);
    }
    let mut style = layer.style;
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(tr(lang, "Colour"));
        for c in PALETTE {
            let swatch = egui::Color32::from_rgb(c[0], c[1], c[2]);
            let (rect, response) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
            let stroke = if c == style.color { egui::Stroke::new(2.0, crate::theme::pal().text) } else { egui::Stroke::NONE };
            ui.painter().rect(rect, 2.0, swatch, stroke, egui::StrokeKind::Outside);
            if response.clicked() {
                style.color = c;
                changed = true;
            }
        }
    });
    egui::Grid::new(("data-style", id)).num_columns(2).show(ui, |ui| {
        let mut slider = |ui: &mut egui::Ui, name: &'static str, value: &mut f32, range: std::ops::RangeInclusive<f32>, percent: bool, tip: &'static str| {
            ui.label(tr(lang, name));
            let mut shown = if percent { *value * 100.0 } else { *value };
            let range = if percent { (*range.start() * 100.0)..=(*range.end() * 100.0) } else { range };
            let s = egui::Slider::new(&mut shown, range).fixed_decimals(if percent { 0 } else { 1 }).suffix(if percent { " %" } else { "" });
            if ui.add(s).on_hover_text(tr(lang, tip)).changed() {
                *value = if percent { shown / 100.0 } else { shown };
                changed = true;
            }
            ui.end_row();
        };
        slider(ui, "Line width", &mut style.width, 0.0..=8.0, false, "Width of lines and outlines in pixels. Zero - polygons without an outline.");
        slider(ui, "Fill", &mut style.fill, 0.0..=1.0, true, "Fill of polygons, a share of the layer opacity. Zero - outlines only.");
        slider(ui, "Point size", &mut style.size, 2.0..=16.0, false, "Diameter of points in pixels.");
        slider(ui, "Opacity", &mut style.opacity, 0.05..=1.0, true, "Opacity of the layer. To the right the layer is opaque, to the left more transparent.");
    });
    if changed {
        out.push((id, Act::Style(style)));
    }
    ui.separator();
    let mut chosen = labels_extrude(ui, lang, layer, out);
    chosen |= time_menu(ui, lang, layer, out);
    ui.separator();
    chosen |= folder_menu(ui, lang, layer, folders, new_folder, out);
    chosen |= side_menu(ui, lang, layer, out);
    if ui.button(tr(lang, "Up")).clicked() {
        out.push((id, Act::Up));
        chosen = true;
    }
    if ui.button(tr(lang, "Down")).clicked() {
        out.push((id, Act::Down));
        chosen = true;
    }
    chosen |= copy_save(ui, lang, id, out);
    if RUSTGIS_LINKED.load(std::sync::atomic::Ordering::Relaxed)
        && layer.live.is_none()
        && ui.button(tr(lang, "To the RustGIS project")).on_hover_text(tr(lang, "The objects of the layer with all fields go to the open RustGIS project as a new layer.")).clicked()
    {
        out.push((id, Act::ToRustGis));
        chosen = true;
    }
    if ui.button(tr(lang, "Remove from the globe")).clicked() {
        out.push((id, Act::Remove));
        chosen = true;
    }
    chosen
}

/// Пункты «Копия в Мои метки» и «Сохранить в GeoJSON…» меню слоя.
fn copy_save(ui: &mut egui::Ui, lang: Lang, id: u64, out: &mut Vec<(u64, Act)>) -> bool {
    if ui.button(tr(lang, "Copy to My Places")).on_hover_text(tr(lang, "The objects of the layer become placemarks of a folder in «My places»: they can be edited, measured and saved into a scene. A large layer is cut to its first objects.")).clicked() {
        out.push((id, Act::ToPlaces));
        return true;
    }
    if ui.button(tr(lang, "Save as GeoJSON…")).on_hover_text(tr(lang, "The objects of the layer go to a GeoJSON file in WGS84 with all fields. QGIS and other GIS open it.")).clicked() {
        out.push((id, Act::Export));
        return true;
    }
    false
}

/// Меню слоя проекта RustGIS в разделе «Проект RustGIS», как меню слоя
/// раздела «Проект QGIS» модуля: перелёт, непрозрачность, подписи,
/// выдавливание, копия в «Мои метки», файл GeoJSON. Цвета слоя - из
/// RustGIS, их меню не меняет. true - пункт выбран, меню закрывается.
pub fn live_menu(ui: &mut egui::Ui, lang: Lang, layer: &DataLayer, out: &mut Vec<(u64, Act)>) -> bool {
    let id = layer.id;
    if ui.button(tr(lang, "Zoom to layer")).clicked() {
        out.push((id, Act::Zoom));
        return true;
    }
    ui.separator();
    let mut percent = (layer.style.opacity * 100.0).round();
    ui.horizontal(|ui| {
        ui.label(tr(lang, "Opacity"));
        let slider = ui.add(egui::Slider::new(&mut percent, 5.0..=100.0).fixed_decimals(0).suffix(" %"));
        if slider.on_hover_text(tr(lang, "Opacity of the layer on the globe. In the mode Layers as on the RustGIS map - on the map too.")).changed() {
            let mut style = layer.style;
            style.opacity = percent / 100.0;
            out.push((id, Act::Style(style)));
        }
    });
    ui.separator();
    let mut chosen = labels_extrude(ui, lang, layer, out);
    ui.separator();
    chosen |= copy_save(ui, lang, id, out);
    // Смену цветов без смены отрисовщика RustGIS не сообщает: оформление
    // перечитывается по просьбе.
    if ui.button(tr(lang, "Re-read the look from RustGIS")).on_hover_text(tr(lang, "The features and the colours of the layer are read from RustGIS again. Needed after colours change in RustGIS without a change of the renderer.")).clicked() {
        out.push((id, Act::Reload));
        chosen = true;
    }
    chosen
}

/// Подменю «Время» и «Конец времени» по полям объектов, как временные
/// свойства слоя QGIS у модуля: объекты вне окна шкалы времени
/// прячутся. true - поле выбрано.
fn time_menu(ui: &mut egui::Ui, lang: Lang, layer: &DataLayer, out: &mut Vec<(u64, Act)>) -> bool {
    let id = layer.id;
    let mut chosen = false;
    let name = |n: Option<usize>| n.and_then(|n| layer.data.fields.get(n)).map_or_else(|| tr(lang, "No").to_string(), String::clone);
    let start = layer.time.map(|t| t.0);
    ui.menu_button(trf(lang, "Time: {field}", &[("field", &name(start))]), |ui| {
        if ui.radio(start.is_none(), tr(lang, "No")).clicked() {
            out.push((id, Act::Time(None)));
            chosen = true;
        }
        for (n, f) in layer.data.fields.iter().enumerate() {
            if ui.radio(start == Some(n), f.as_str()).clicked() {
                out.push((id, Act::Time(Some((n, layer.time.and_then(|t| t.1))))));
                chosen = true;
            }
        }
    })
    .response
    .on_hover_text(tr(lang, "A field with the date or time of the objects. The objects follow the time slider: outside its window they are hidden. A date - like 2025-06-01, a year - like 1890."));
    if let Some((start, end)) = layer.time {
        ui.menu_button(trf(lang, "End of time: {field}", &[("field", &name(end))]), |ui| {
            if ui.radio(end.is_none(), tr(lang, "No")).clicked() {
                out.push((id, Act::Time(Some((start, None)))));
                chosen = true;
            }
            for (n, f) in layer.data.fields.iter().enumerate() {
                if ui.radio(end == Some(n), f.as_str()).clicked() {
                    out.push((id, Act::Time(Some((start, Some(n))))));
                    chosen = true;
                }
            }
        })
        .response
        .on_hover_text(tr(lang, "A field with the end of the time of the objects. Without it an object is a moment."));
    }
    chosen
}

/// Подменю подписей и выдавливания по полю, множитель высоты.
fn labels_extrude(ui: &mut egui::Ui, lang: Lang, layer: &DataLayer, out: &mut Vec<(u64, Act)>) -> bool {
    let id = layer.id;
    let mut chosen = false;
    let label_name = layer.label.and_then(|f| layer.data.fields.get(f)).map_or(tr(lang, "No"), String::as_str);
    ui.menu_button(trf(lang, "Labels: {field}", &[("field", label_name)]), |ui| {
        if ui.radio(layer.label.is_none(), tr(lang, "No")).clicked() {
            out.push((id, Act::Label(None)));
            chosen = true;
        }
        for (n, f) in layer.data.fields.iter().enumerate() {
            if ui.radio(layer.label == Some(n), f.as_str()).clicked() {
                out.push((id, Act::Label(Some(n))));
                chosen = true;
            }
        }
    })
    .response
    .on_hover_text(tr(lang, "A field whose value labels the objects on the globe. Of each layer the labels nearest to the view point are shown."));
    let numeric: Vec<(usize, &String)> = layer.data.fields.iter().enumerate().filter(|(n, _)| layer.data.numeric.get(*n).copied().unwrap_or(false)).collect();
    let factor = layer.extrude.map_or(1.0, |(_, f)| f);
    let extrude_name = layer.extrude.and_then(|(f, _)| layer.data.fields.get(f)).map_or(tr(lang, "No"), String::as_str);
    ui.add_enabled_ui(!numeric.is_empty(), |ui| {
        ui.menu_button(trf(lang, "Extrusion: {field}", &[("field", extrude_name)]), |ui| {
            if ui.radio(layer.extrude.is_none(), tr(lang, "No")).clicked() {
                out.push((id, Act::Extrude(None)));
                chosen = true;
            }
            for (n, f) in &numeric {
                if ui.radio(layer.extrude.is_some_and(|(e, _)| e == *n), f.as_str()).clicked() {
                    out.push((id, Act::Extrude(Some((*n, factor)))));
                    chosen = true;
                }
            }
        })
        .response
        .on_hover_text(tr(lang, "Objects rise above the terrain to the height from a numeric field: a polygon as a prism, a point as a column, a line as a wall. An object with an empty, zero or negative value stays flat."));
    });
    if let Some((field, factor)) = layer.extrude {
        ui.horizontal(|ui| {
            ui.label(tr(lang, "Factor, m"));
            let mut value = factor;
            let drag = ui.add(egui::DragValue::new(&mut value).range(0.001..=100000.0).speed(0.01).fixed_decimals(3));
            if drag.on_hover_text(tr(lang, "The height in metres is the field value times the factor. For a field of storeys the factor is about 3.")).changed() {
                out.push((id, Act::Extrude(Some((field, value)))));
            }
        });
    }
    chosen
}

/// Меню слоя-картинки: объектов, подписей и стиля линий у неё нет.
/// Остаются непрозрачность, папка, порядок, копия в «Мои метки».
/// Подменю «Шторка» меню слоя, как у модуля: слой слева, справа или
/// по обе стороны линии шторки. true - сторона выбрана.
fn side_menu(ui: &mut egui::Ui, lang: Lang, layer: &DataLayer, out: &mut Vec<(u64, Act)>) -> bool {
    let mut chosen = false;
    ui.menu_button(tr(lang, "Swipe"), |ui| {
        for (side, name, tip) in [
            (render::Side::Left, "Left", "The layer is seen only left of the swipe line. Right of it - layers «Right» and layers without a side."),
            (render::Side::Right, "Right", "The layer is seen only right of the swipe line."),
            (render::Side::Both, "Both sides", "The layer is seen on both sides of the swipe line."),
        ] {
            if ui.radio(layer.side == side, tr(lang, name)).on_hover_text(tr(lang, tip)).clicked() {
                out.push((layer.id, Act::Side(side)));
                chosen = true;
            }
        }
    });
    chosen
}

fn picture_menu(ui: &mut egui::Ui, lang: Lang, layer: &DataLayer, folders: &[String], new_folder: &mut String, out: &mut Vec<(u64, Act)>) -> bool {
    let id = layer.id;
    if let Some(text) = layer.picture.as_ref().map(|p| p.description.as_str()).filter(|d| !d.is_empty()) {
        ui.label(egui::RichText::new(text).color(crate::theme::pal().muted));
    }
    let mut style = layer.style;
    let mut percent = (style.opacity * 100.0).round();
    ui.horizontal(|ui| {
        ui.label(tr(lang, "Opacity"));
        if ui.add(egui::Slider::new(&mut percent, 5.0..=100.0).fixed_decimals(0).suffix(" %")).changed() {
            style.opacity = percent / 100.0;
            out.push((id, Act::Style(style)));
        }
    });
    ui.separator();
    let mut chosen = folder_menu(ui, lang, layer, folders, new_folder, out);
    chosen |= side_menu(ui, lang, layer, out);
    for (name, act) in [("Up", Act::Up), ("Down", Act::Down)] {
        if ui.button(tr(lang, name)).clicked() {
            out.push((id, act));
            chosen = true;
        }
    }
    if ui.button(tr(lang, "Copy to My Places")).on_hover_text(tr(lang, "The picture becomes a ground overlay in «My places»: it goes into a scene and a KMZ file.")).clicked() {
        out.push((id, Act::ToPlaces));
        chosen = true;
    }
    if ui.button(tr(lang, "Save as GeoTIFF…")).on_hover_text(tr(lang, "The picture of the layer goes to a GeoTIFF file in WGS84 with the colours and the transparency of the globe. QGIS and other GIS open it in its place.")).clicked() {
        out.push((id, Act::ExportPicture));
        chosen = true;
    }
    if ui.button(tr(lang, "Remove from the globe")).clicked() {
        out.push((id, Act::Remove));
        chosen = true;
    }
    chosen
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use data::layers::Item;

    fn item(shape: Geom, parts: Vec<Vec<(f64, f64)>>, values: &[&str]) -> Item {
        Item { shape, parts, heights: Vec::new(), values: values.iter().map(|v| v.to_string()).collect() }
    }

    fn square(lat: f64, lon: f64, side: f64) -> Vec<(f64, f64)> {
        vec![(lat, lon), (lat, lon + side), (lat + side, lon + side), (lat + side, lon)]
    }

    fn layer() -> LayerData {
        LayerData {
            name: "ponds".into(),
            epsg: Some(4326),
            fields: vec!["name".into(), "h".into()],
            numeric: vec![false, true],
            items: vec![
                item(Geom::Polygon, vec![square(58.0, 56.0, 1.0), square(58.4, 56.4, 0.2)], &["Пруд", "3"]),
                item(Geom::Point, vec![vec![(60.0, 50.0)]], &["Скважина", ""]),
                item(Geom::Line, vec![vec![(61.0, 50.0), (61.0, 51.0)]], &["Дорога", "-1"]),
            ],
        }
    }

    #[test]
    fn multipolygon_rings_become_outers_and_holes() {
        // Два квадрата рядом и дыра во втором.
        let parts = vec![square(0.0, 0.0, 1.0), square(0.0, 5.0, 2.0), square(0.5, 5.5, 0.5)];
        let found = polygons(&parts);
        assert_eq!(found.len(), 2);
        assert!(found[0].1.is_empty());
        assert_eq!(found[1].1.len(), 1);
        // Остров в дыре - снова внешнее кольцо.
        let island = vec![square(0.0, 0.0, 4.0), square(1.0, 1.0, 2.0), square(1.5, 1.5, 0.5)];
        let found = polygons(&island);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].1.len(), 1);
    }

    #[test]
    fn layers_from_file_with_style_and_order() {
        let mut layers = DataLayers::default();
        let table = LayerData { name: "beds".into(), epsg: None, fields: vec![], numeric: vec![], items: vec![item(Geom::None, vec![], &[])] };
        assert_eq!(layers.add(vec![layer(), table], "ponds.gpkg"), 1);
        assert_eq!(layers.list[0].title, "ponds.gpkg");
        assert_eq!(layers.add(vec![layer(), layer()], "two.gpkg"), 2);
        // Новые слои - сверху, цвета разные.
        assert_eq!(layers.list[0].title, "two.gpkg · ponds");
        assert_ne!(layers.list[0].style.color, layers.list[1].style.color);
        let id = layers.list[2].id;
        layers.apply(id, &Act::Up);
        assert_eq!(layers.list[1].id, id);
        assert_eq!(layers.apply(id, &Act::Remove).len(), 1);
        assert_eq!(layers.list.len(), 2);
    }

    #[test]
    fn folders_gather_layers() {
        let mut layers = DataLayers::default();
        layers.add(vec![layer()], "a.gpkg");
        layers.add_to(vec![layer(), layer()], "b.gpkg", "Результаты");
        assert_eq!(layers.folders(), ["Результаты"]);
        let a = layers.list.iter().find(|l| l.folder.is_empty()).map(|l| l.id).unwrap();
        layers.apply(a, &Act::Folder(" Пруды ".into()));
        assert_eq!(layers.folders(), ["Результаты", "Пруды"]);
        // Флажок папки прячет все её слои, другие не трогает.
        layers.apply(0, &Act::FolderVisible("Результаты".into(), false));
        assert_eq!(layers.list.iter().filter(|l| l.visible).count(), 1);
        layers.apply(0, &Act::FolderCollapse("Пруды".into()));
        assert!(layers.collapsed.contains("Пруды"));
        // Папка уходит с глобуса со всеми слоями, их файлы - к удалению.
        for l in layers.list.iter_mut() {
            l.store = Some(format!("{}.geojson", l.id));
        }
        assert_eq!(layers.apply(0, &Act::FolderRemove("Результаты".into())).len(), 2);
        assert_eq!(layers.list.len(), 1);
        assert_eq!(layers.dropped.len(), 2);
    }

    #[test]
    fn changed_file_gives_refresh_and_reload_keeps_style() {
        let mut layers = DataLayers::default();
        let source = Source { path: "C:/data/ponds.gpkg".into(), modified: 100 };
        layers.add_from(vec![layer()], "ponds.gpkg", Some(source.clone()));
        layers.add(vec![layer()], "memory");
        let id = id_of(&layers, "ponds.gpkg");
        layers.apply(id, &Act::Style(Style { color: [1, 2, 3], ..Style::new(0) }));
        layers.apply(id, &Act::Label(Some(1)));
        layers.apply(id, &Act::Down);
        layers.list.iter_mut().for_each(|l| l.store = Some(format!("{}.geojson", l.id)));
        // Время то же - строки нет, другое - есть, сверка не чаще SOURCE_CHECK.
        assert!(!layers.check_sources(0.0, &|_| Some(100)));
        assert!(!layers.check_sources(1.0, &|_| Some(200)));
        assert!(layers.check_sources(2.5, &|p| (p == "C:/data/ponds.gpkg").then_some(200)));
        assert!(layers.get(id).unwrap().stale);
        assert!(!layers.list.iter().any(|l| l.source.is_none() && l.stale));
        // Путь и время файла - в хранилище и обратно.
        let (back, _) = DataLayers::from_index(&layers.index(true), &|_| None);
        assert_eq!(back.list.iter().find(|l| l.title == "ponds.gpkg").and_then(|l| l.source.clone()), Some(source));
        // Перечитанный слой: новые объекты, стиль, подписи и место - прежние,
        // объекты в хранилище - заново.
        let mut fresh = layer();
        fresh.items.truncate(1);
        assert!(layers.reload(id, vec![fresh], 200));
        let l = layers.get(id).unwrap();
        assert_eq!((l.data.items.len(), l.style.color, l.label, l.stale), (1, [1, 2, 3], Some(1), false));
        assert_eq!(l.source.as_ref().map(|s| s.modified), Some(200));
        assert!(l.store.is_none());
        assert_eq!(layers.dropped, [format!("{id}.geojson")]);
        assert_eq!(order(&layers), ["memory", "ponds.gpkg"]);
        // В файле нет слоя с этим именем, а слоёв больше одного - отказ.
        let mut other = layer();
        other.name = "other".into();
        assert!(!layers.reload(id, vec![other.clone(), other], 300));
    }

    #[test]
    fn picture_layer_saves_as_geotiff_in_its_frame() {
        // Картинка 4 x 2: левый верхний пиксель красный, правый нижний
        // прозрачный. Рамка - края крайних пикселей.
        let mut img = image::RgbaImage::from_pixel(4, 2, image::Rgba([10, 20, 30, 255]));
        img.put_pixel(0, 0, image::Rgba([250, 0, 0, 255]));
        img.put_pixel(3, 1, image::Rgba([0, 0, 0, 0]));
        let mut png = std::io::Cursor::new(Vec::new());
        assert!(img.write_to(&mut png, image::ImageFormat::Png).is_ok());
        let mut ground = scene::overlays::Overlay::new(scene::overlays::Kind::Ground);
        ground.set_box([58.2, 58.0, 56.4, 56.0, 0.0]);
        let mut picture = scene::kml::Overlay::new("Тени", ground.clone());
        picture.image = Some(png.into_inner());
        let mut layers = DataLayers::default();
        let pid = layers.add_picture(picture.clone(), "");
        let saved = layers.picture_geotiff(pid);
        assert!(saved.as_ref().is_ok_and(|(name, bytes)| name == "Тени.tif" && bytes.starts_with(b"II*\0")), "{saved:?}");
        // Узлы - центры пикселей: первый на полшага от северо-западного угла.
        let (g, img) = picture_grid(&picture).unwrap();
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(close(g.north, 58.15) && close(g.west, 56.05) && close(g.dlat, 0.1) && close(g.dlon, 0.1) && (g.cols, g.rows) == (4, 2), "{g:?}");
        assert_eq!(img.get_pixel(0, 0).0, [250, 0, 0, 255]);
        assert_eq!(img.get_pixel(3, 1).0[3], 0);
        // Повёрнутая рамка и пустая картинка - ошибка с ключом перевода.
        ground.set_box([58.2, 58.0, 56.4, 56.0, 15.0]);
        picture.overlay = ground;
        assert!(picture_geotiff(&picture).is_err());
        picture.image = None;
        assert_eq!(picture_geotiff(&picture), Err("The picture of the layer is not loaded"));
    }

    #[test]
    fn analysis_picture_is_a_layer_under_its_vectors() {
        let mut layers = DataLayers::default();
        let mut ground = scene::overlays::Overlay::new(scene::overlays::Kind::Ground);
        ground.set_box([58.1, 58.0, 56.3, 56.2, 0.0]);
        let mut picture = scene::kml::Overlay::new("Тени", ground);
        picture.image = Some(b"\x89PNG-bytes".to_vec());
        let folder = "Тени 10:00:00 UTC";
        let pid = layers.add_picture(picture, folder);
        layers.add_to(vec![layer()], "Участок", folder);
        assert_eq!(order(&layers), [format!("{folder}/Участок"), format!("{folder}/Тени")]);
        // Охват - углы рамки, у картинки нет линий и точек.
        assert_eq!(layers.get(pid).unwrap().extent_points().len(), 4);
        assert!(layers.get(pid).unwrap().lines().is_empty());
        // Непрозрачность - в оттенке картинки, отпечаток меняется.
        let before = layers.pictures_version(None);
        assert_ne!(before, 0);
        layers.apply(pid, &Act::Style(Style { opacity: 0.5, ..Style::new(0) }));
        assert_eq!(layers.get(pid).unwrap().picture.as_ref().unwrap().overlay.color[3], 128);
        assert_ne!(layers.pictures_version(None), before);
        layers.apply(pid, &Act::Visible(false));
        assert!(layers.pictures(None).is_empty());
        layers.apply(pid, &Act::Visible(true));
        assert_eq!(layers.pictures(None).len(), 1);
        // В папку данных - картинкой, в списке - рамка, обратно - тот же слой.
        let stored: Vec<(u64, String)> = layers.unstored();
        let name = stored.iter().find(|(id, _)| *id == pid).map(|(_, n)| n.clone()).unwrap();
        assert!(name.ends_with(".png"));
        assert_eq!(DataLayers::stored_bytes(layers.get(pid).unwrap()), b"\x89PNG-bytes");
        for (id, n) in stored {
            layers.stored(id, n);
        }
        let files: std::collections::HashMap<String, Vec<u8>> = layers.list.iter().map(|l| (l.store.clone().unwrap(), DataLayers::stored_bytes(l))).collect();
        let (back, lost) = DataLayers::from_index(&layers.index(false), &|n| files.get(n).cloned());
        assert!(lost.is_empty(), "{lost:?}");
        let p = back.list.iter().find(|l| l.picture.is_some()).unwrap();
        assert_eq!((p.title.as_str(), p.folder.as_str()), ("Тени", folder));
        let o = p.picture.as_ref().unwrap();
        assert_eq!(o.overlay.latlon_box, Some([58.1, 58.0, 56.3, 56.2, 0.0]));
        assert_eq!(o.overlay.color[3], 128);
        // На странице картинки в строку хранилища не идут.
        assert!(!layers.index(true).contains("picture"));
        // Точки меток расчёта - тоже слой.
        let mut marks = scene::kml::Folder::new("Вершины");
        marks.children.push(scene::kml::Item::Place(scene::kml::Place::new("1", Kind::Point, vec![(58.0, 56.0)])));
        let points = folder_layer(&marks).unwrap();
        assert_eq!(points.items[0].shape, Geom::Point);
    }

    /// Названия слоёв с папками в порядке списка.
    fn order(layers: &DataLayers) -> Vec<String> {
        layers.list.iter().map(|l| if l.folder.is_empty() { l.title.clone() } else { format!("{}/{}", l.folder, l.title) }).collect()
    }

    /// Слой по названию.
    fn id_of(layers: &DataLayers, title: &str) -> u64 {
        layers.list.iter().find(|l| l.title == title).map(|l| l.id).unwrap()
    }

    #[test]
    fn rows_order_is_drawing_order_with_folders_together() {
        let mut layers = DataLayers::default();
        for name in ["c", "b", "a"] {
            layers.add(vec![layer()], name);
        }
        layers.add_to(vec![layer()], "f2", "F");
        layers.add_to(vec![layer()], "f1", "F");
        assert_eq!(order(&layers), ["F/f1", "F/f2", "a", "b", "c"]);
        // Слой в папку - к своей папке, папка не рвётся.
        layers.apply(id_of(&layers, "b"), &Act::Folder("F".into()));
        assert_eq!(order(&layers), ["F/f1", "F/f2", "F/b", "a", "c"]);
        // «Вниз» слоя вне папок под папкой проходит её целиком, «Вверх» -
        // тоже.
        layers.apply(id_of(&layers, "a"), &Act::Up);
        assert_eq!(order(&layers), ["a", "F/f1", "F/f2", "F/b", "c"]);
        layers.apply(id_of(&layers, "a"), &Act::Down);
        assert_eq!(order(&layers), ["F/f1", "F/f2", "F/b", "a", "c"]);
        // Внутри папки - на одну строку.
        layers.apply(id_of(&layers, "f2"), &Act::Down);
        assert_eq!(order(&layers), ["F/f1", "F/b", "F/f2", "a", "c"]);
        // Перетаскивание: перед слоем в его папку, в конец папки, в конец
        // списка вне папок, на себя - ничего.
        layers.apply(id_of(&layers, "c"), &Act::Move { before: Some(id_of(&layers, "b")), folder: "F".into() });
        assert_eq!(order(&layers), ["F/f1", "F/c", "F/b", "F/f2", "a"]);
        layers.apply(id_of(&layers, "f1"), &Act::Move { before: None, folder: "F".into() });
        assert_eq!(order(&layers), ["F/c", "F/b", "F/f2", "F/f1", "a"]);
        layers.apply(id_of(&layers, "b"), &Act::Move { before: None, folder: String::new() });
        assert_eq!(order(&layers), ["F/c", "F/f2", "F/f1", "a", "b"]);
        let a = id_of(&layers, "a");
        layers.apply(a, &Act::Move { before: Some(a), folder: "F".into() });
        assert_eq!(order(&layers), ["F/c", "F/f2", "F/f1", "a", "b"]);
        // Нижняя строка - внизу стопки линий.
        let set = layers.line_set().unwrap();
        assert_eq!(set.first().map(|l| l.color), layers.list.last().map(|l| l.style.line_rgba()));
        // Выделение не меняет список и не просит записи.
        layers.save_dirty = false;
        layers.apply(a, &Act::Select);
        assert_eq!(layers.selected, Some(a));
        assert!(!layers.save_dirty);
        assert_eq!(row_count(&layers), 6);
        layers.apply(0, &Act::FolderCollapse("F".into()));
        assert_eq!(row_count(&layers), 3);
    }

    #[test]
    fn dated_picture_follows_the_time_slider_and_keeps_its_day() {
        let mut ground = scene::overlays::Overlay::new(scene::overlays::Kind::Ground);
        ground.set_box([58.1, 58.0, 56.3, 56.2, 0.0]);
        let mut image = scene::kml::Overlay::new("Снимок", ground.clone());
        image.image = Some(b"\x89PNG".to_vec());
        image.time = Some(("2025-06-01".into(), "2025-06-01".into()));
        let mut trend = scene::kml::Overlay::new("Тренд", ground);
        trend.image = Some(b"\x89PNG".to_vec());
        let mut layers = DataLayers::default();
        layers.add_picture(image, "Космоснимки");
        layers.add_picture(trend, "Карты тренда");
        // Охват шкалы - день снимка, карта без времени видна всегда.
        let (lo, hi) = layers.extent().unwrap();
        assert!(hi >= lo);
        let before = (lo - 400.0 * 86400.0, lo - 300.0 * 86400.0);
        assert_eq!(layers.pictures(Some(before)).iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Тренд"]);
        assert_eq!(layers.pictures(Some((lo, hi))).len(), 2);
        assert_ne!(layers.pictures_version(Some(before)), layers.pictures_version(Some((lo, hi))));
        // День снимка - в списке хранилища и обратно.
        for (id, n) in layers.unstored() {
            layers.stored(id, n);
        }
        let files: std::collections::HashMap<String, Vec<u8>> = layers.list.iter().map(|l| (l.store.clone().unwrap(), DataLayers::stored_bytes(l))).collect();
        let (back, _) = DataLayers::from_index(&layers.index(false), &|n| files.get(n).cloned());
        let times: Vec<Option<scene::when::Time>> = back.list.iter().map(|l| l.picture.as_ref().unwrap().time.clone()).collect();
        assert!(times.contains(&Some(("2025-06-01".into(), "2025-06-01".into()))) && times.contains(&None));
    }

    #[test]
    fn places_become_three_layers_by_kind() {
        use scene::kml::{Folder, Item as KmlItem, Place};
        let mut lake = Place::new("Озеро", Kind::Polygon, square(58.0, 56.0, 0.1));
        lake.color = [30, 60, 220, 255];
        lake.description = "Пресное".into();
        let road = Place::new("Дорога", Kind::Line, vec![(58.0, 56.0), (58.1, 56.1)]);
        let well = Place::new("Колодец", Kind::Point, vec![(58.05, 56.05)]);
        let empty = Place::new("Пусто", Kind::Line, Vec::new());
        let inner = Folder::with("Север", vec![KmlItem::Folder(Folder::with("Озёра", vec![KmlItem::Place(lake)])), KmlItem::Place(well)]);
        let root = Folder::with("Мои метки", vec![KmlItem::Folder(inner), KmlItem::Place(road), KmlItem::Place(empty)]);
        let layers = places_layers(&root, ["точки", "линии", "многоугольники"]);
        let names: Vec<&str> = layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["точки", "линии", "многоугольники"]);
        assert!(layers.iter().all(|l| l.fields == PLACE_FIELDS && l.items.len() == 1));
        assert_eq!(layers[0].items[0].values, ["Колодец", "", "#ffff00", "Север"]);
        assert_eq!(layers[1].items[0].shape, Geom::Line);
        assert_eq!(layers[2].items[0].values, ["Озеро", "Пресное", "#1e3cdc", "Север / Озёра"]);
        // Папка без многоугольников - без их слоя.
        let only = Folder::with("Путь", vec![KmlItem::Place(Place::new("Дорога", Kind::Line, vec![(58.0, 56.0), (58.1, 56.1)]))]);
        assert_eq!(places_layers(&only, ["т", "л", "м"]).len(), 1);
    }

    #[test]
    fn result_folder_becomes_a_coloured_layer() {
        use scene::kml::{Folder, Item as KmlItem, Place};
        let mut forest = Place::new("Лес 1", Kind::Polygon, square(58.0, 56.0, 0.1));
        forest.color = [40, 120, 40, 255];
        forest.holes = Some(vec![square(58.02, 56.02, 0.01)]);
        forest.data = Some(vec![("cls".into(), "1".into()), ("name".into(), "Лес".into()), ("area_ha".into(), "12.5".into())]);
        let mut water = Place::new("Вода 2", Kind::Polygon, square(58.2, 56.0, 0.1));
        water.color = [30, 60, 220, 255];
        water.data = Some(vec![("cls".into(), "2".into()), ("name".into(), "Вода".into()), ("area_ha".into(), "3".into())]);
        let road = Place::new("Дорога", Kind::Line, vec![(58.0, 56.0), (58.1, 56.1)]);
        let folder = Folder::with("Классы", vec![KmlItem::Folder(Folder::with("1 - Лес", vec![KmlItem::Place(forest), KmlItem::Place(water)])), KmlItem::Place(road)]);
        let layer = folder_layer(&folder).unwrap();
        assert_eq!(layer.name, "Классы");
        assert_eq!(layer.fields, ["name", "cls", "area_ha", "color"]);
        assert_eq!(layer.numeric, [false, true, true, false]);
        assert_eq!(layer.items[0].parts.len(), 2);
        assert_eq!(layer.items[0].values, ["Лес", "1", "12.5", "#287828"]);
        // Метка без полей - название из имени метки.
        assert_eq!(layer.items[2].values[0], "Дорога");
        assert_eq!(layer.items[2].shape, Geom::Line);
        // Слой рисует объекты цветом из поля color.
        let mut layers = DataLayers::default();
        layers.add_to(vec![layer], "Классы", "Распознавание");
        let set = layers.line_set().unwrap();
        assert_eq!(&set[0].color[..3], &[40, 120, 40]);
        assert_eq!(&set[1].color[..3], &[30, 60, 220]);
        // Папка с картинкой рядом с метками - не слой: картинка осталась бы
        // без места.
        let point = Place::new("Точка", Kind::Point, vec![(58.0, 56.0)]);
        let picture = scene::kml::Overlay::new("Снимок", scene::overlays::Overlay::new(scene::overlays::Kind::Ground));
        assert!(folder_layer(&Folder::with("Точки", vec![KmlItem::Place(point), KmlItem::Overlay(picture)])).is_none());
    }

    /// Мультимногоугольник с далёкой первой частью - Франция с Гвианой
    /// первой, как в Natural Earth: каждая часть - своя линия со своей
    /// рамкой. Прежде вторая часть шла дырой первой, рамка - по Гвиане,
    /// и тайлы над Францией пропускали её.
    #[test]
    fn multipolygon_parts_keep_their_tiles() {
        let guiana = vec![(2.0, -54.0), (5.5, -54.0), (5.5, -51.6), (2.0, -51.6)];
        let france = vec![(42.5, -4.5), (51.0, -4.5), (51.0, 8.0), (42.5, 8.0)];
        let item = data::layers::Item { shape: Geom::Polygon, parts: vec![guiana, france], heights: Vec::new(), values: vec!["France".into()] };
        let data = LayerData { name: "Countries".into(), epsg: Some(4326), fields: vec!["name".into()], numeric: vec![false], items: vec![item] };
        let mut layers = DataLayers::default();
        layers.add(vec![data], "countries.geojson");
        let set = layers.line_set().unwrap();
        assert_eq!(set.len(), 2);
        let (s, w, n, e) = set[1].bbox;
        assert!(s <= 43.0 && n >= 50.0 && w <= -4.0 && e >= 7.0, "{:?}", set[1].bbox);
    }

    /// Живые слои RustGIS: на месте при обновлении, порядок и видимость
    /// по проекту, в хранилище не пишутся.
    #[test]
    fn live_layers_follow_rustgis() {
        let mut layers = DataLayers::default();
        layers.add(vec![layer()], "ponds.gpkg");
        let own = layers.line_set().map_or(0, |s| s.len());
        layers.set_live("RustGIS", 4, "Cities", (layer(), Vec::new()), Style::new(0), None);
        layers.set_live("RustGIS", 1, "Countries", (layer(), vec![[1, 2, 3]]), Style::new(1), Some(0));
        assert_eq!(layers.list.len(), 3);
        // Тот же слой заново - на месте.
        layers.set_live("RustGIS", 4, "Cities", (layer(), Vec::new()), Style::new(0), None);
        assert_eq!(layers.list.len(), 3);
        // Новые слои выключены, как слои проекта QGIS в модуле.
        assert_eq!(layers.live_shown(4), Some(false));
        assert_eq!(layers.line_set().map_or(0, |s| s.len()), own);
        layers.show_live(1, true);
        assert_eq!(layers.live_shown(1), Some(true));
        assert_eq!(layers.live_shown(7), None);
        // Порядок RustGIS: Cities над Countries.
        assert!(layers.sync_live(&[4, 1]));
        let live: Vec<Option<i64>> = layers.list.iter().filter(|l| l.live.is_some()).map(|l| l.live).collect();
        assert_eq!(live, vec![Some(4), Some(1)]);
        assert!(!layers.sync_live(&[4, 1]));
        // Флажок пользователя опрос не сбрасывает.
        assert_eq!(layers.live_shown(1), Some(true));
        // Живые слои не пишутся, не ждут записи и не видны в «Моих слоях».
        assert!(!layers.index(false).contains("Cities"));
        assert_eq!(layers.unstored().len(), 1);
        assert!(layers.folders().is_empty());
        // Слой убран из проекта - убран с глобуса.
        layers.sync_live(&[1]);
        assert!(layers.list.iter().all(|l| l.live != Some(4)));
        layers.drop_live();
        assert_eq!(layers.list.len(), 1);
    }

    #[test]
    fn index_restores_layers_files_and_inline() {
        let mut layers = DataLayers::default();
        layers.add_to(vec![layer()], "ponds.gpkg", "Воды");
        let id = layers.list[0].id;
        layers.apply(id, &Act::Label(Some(0)));
        layers.apply(id, &Act::Extrude(Some((1, 2.5))));
        layers.apply(id, &Act::Visible(false));
        let mut style = layers.list[0].style;
        style.color = [1, 2, 3];
        style.opacity = 0.5;
        layers.apply(id, &Act::Style(style));
        layers.apply(0, &Act::FolderCollapse("Воды".into()));
        // В программе - файл объектов по имени.
        let files = layers.unstored();
        assert_eq!(files.len(), 1);
        let text = data::layers::to_geojson(&layers.list[0].data);
        layers.stored(files[0].0, files[0].1.clone());
        assert!(layers.unstored().is_empty());
        let name = files[0].1.clone();
        let read = |n: &str| (n == name).then(|| text.clone().into_bytes());
        let (back, lost) = DataLayers::from_index(&layers.index(false), &read);
        assert!(lost.is_empty());
        let l = &back.list[0];
        assert_eq!((l.title.as_str(), l.folder.as_str(), l.data.name.as_str()), ("ponds.gpkg", "Воды", "ponds"));
        assert_eq!((l.label, l.extrude, l.visible), (Some(0), Some((1, 2.5)), false));
        assert_eq!((l.style.color, l.style.opacity), ([1, 2, 3], 0.5));
        assert_eq!(l.data.items.len(), 3);
        assert!(back.collapsed.contains("Воды") && !back.save_dirty);
        // На странице объекты - внутри списка, файлы не нужны.
        let (inline, _) = DataLayers::from_index(&layers.index(true), &|_| None);
        assert_eq!(inline.list[0].data.items.len(), 3);
        // Пропавший файл - слой называется в потерянных.
        let (gone, lost) = DataLayers::from_index(&layers.index(false), &|_| None);
        assert!(gone.list.is_empty());
        assert_eq!(lost, ["ponds.gpkg"]);
    }

    #[test]
    fn overlay_lines_dots_and_hidden_layer() {
        let mut layers = DataLayers::default();
        layers.add(vec![layer()], "a.gpkg");
        let set = layers.line_set().unwrap();
        // Многоугольник с дырой - одна запись с заливкой, линия - одна.
        assert_eq!(set.len(), 2);
        assert_eq!(set[0].holes.len(), 1);
        assert!(set[0].fill.is_some() && set[1].fill.is_none());
        assert_eq!(layers.dot_sets()[0].0.0, vec![(60.0, 50.0)]);
        let id = layers.list[0].id;
        layers.apply(id, &Act::Visible(false));
        assert!(layers.line_set().is_none() && layers.dot_sets().is_empty());
    }

    #[test]
    fn extrusion_takes_positive_heights_out_of_the_overlay() {
        let mut layers = DataLayers::default();
        layers.add(vec![layer()], "a.gpkg");
        let id = layers.list[0].id;
        layers.apply(id, &Act::Extrude(Some((1, 2.0))));
        let shapes = layers.shapes(6_371_000.0);
        // Пруд - призма 6 м с дырой, у точки пусто, у линии высота меньше нуля.
        assert_eq!(shapes.len(), 1);
        assert!((shapes[0].0.height - 6.0).abs() < 1e-9 && shapes[0].0.extrude);
        assert_eq!(shapes[0].0.holes.as_ref().map(Vec::len), Some(1));
        // В картинке наложения осталась только линия.
        assert_eq!(layers.line_set().unwrap().len(), 1);
    }

    #[test]
    fn timed_layer_hides_objects_outside_the_slider_window() {
        let point = |lat: f64, day: &str, end: &str| data::layers::Item { shape: Geom::Point, parts: vec![vec![(lat, 56.0)]], heights: Vec::new(), values: vec![format!("п{lat}"), day.into(), end.into()] };
        let data = LayerData {
            name: "Съёмки".into(),
            epsg: Some(4326),
            fields: vec!["name".into(), "day".into(), "till".into()],
            numeric: vec![false; 3],
            items: vec![point(58.0, "2020-05-01", ""), point(58.1, "2024-05-01", ""), point(58.2, "", "")],
        };
        let mut layers = DataLayers::default();
        layers.add(vec![data], "s.gpkg");
        let id = layers.list[0].id;
        assert_eq!(layers.extent(), None);
        layers.apply(id, &Act::Time(Some((1, None))));
        let (lo, hi) = layers.extent().unwrap();
        assert!(lo < hi);
        // Окно вокруг 2020 года: точка 2024 года спрятана, точка без
        // времени видна всегда.
        let year = 365.25 * 86400.0;
        layers.set_window(Some((lo - year / 2.0, lo + year / 2.0)));
        assert!(layers.dots_dirty);
        let dots = &layers.dot_sets()[0].0.0;
        assert_eq!(dots.len(), 2);
        assert!(!dots.contains(&(58.1, 56.0)));
        assert!(layers.list[0].hits(58.1, 56.0, 100.0, &|_, _| Some(0.0)).iter().all(|n| *n != 1));
        // Сдвиг окна без смены видимых объектов картинки не пересобирает:
        // при проигрывании шкалы слой иначе пропадал.
        layers.dots_dirty = false;
        layers.lines_dirty = false;
        layers.set_window(Some((lo - year / 2.0 + 10.0, lo + year / 2.0 + 10.0)));
        assert!(!layers.dots_dirty && !layers.lines_dirty);
        // Конец времени: точка 2020 года до 2025 года видна в окне 2024.
        layers.list[0].data.items[0].values[2] = "2025-01-01".into();
        layers.apply(id, &Act::Time(Some((1, Some(2)))));
        layers.set_window(Some((hi - 10.0, hi + 10.0)));
        assert_eq!(layers.dot_sets()[0].0.0.len(), 3);
        // Поля времени - в списке хранилища и обратно.
        let text = layers.index(true);
        let (back, _) = DataLayers::from_index(&text, &|_| None);
        assert_eq!(back.list[0].time, Some((1, Some(2))));
        // Без окна шкалы видно всё.
        layers.set_window(None);
        assert_eq!(layers.dot_sets()[0].0.0.len(), 3);
    }

    #[test]
    fn side_layer_leaves_the_overlay_for_shapes_and_keeps_its_side() {
        let mut layers = DataLayers::default();
        layers.add(vec![layer()], "a.gpkg");
        let id = layers.list[0].id;
        assert!(!layers.has_sides() && layers.shapes(6_371_000.0).is_empty());
        layers.apply(id, &Act::Side(render::Side::Left));
        assert!(layers.has_sides());
        // Линии и многоугольники - объектами на рельефе со стороной,
        // картинка наложения пуста, точки - со стороной.
        assert!(layers.line_set().is_none());
        let shapes = layers.shapes(6_371_000.0);
        assert_eq!(shapes.len(), 2);
        assert!(shapes.iter().all(|(s, side)| *side == render::Side::Left && !s.extrude));
        assert!(shapes.iter().any(|(s, _)| s.kind == Kind::Polygon && s.fill.is_some() && s.holes.as_ref().map(Vec::len) == Some(1)));
        assert_eq!(layers.dot_sets()[0].1, render::Side::Left);
        // Сторона - в списке хранилища и обратно.
        let text = layers.index(true);
        assert!(text.contains("\"side\":\"left\""));
        let (back, _) = DataLayers::from_index(&text, &|_| None);
        assert_eq!(back.list[0].side, render::Side::Left);
        layers.apply(id, &Act::Side(render::Side::Both));
        assert!(!layers.has_sides() && layers.line_set().is_some());
    }

    #[test]
    fn labels_by_field_nearest_first() {
        let mut layers = DataLayers::default();
        layers.add(vec![layer()], "a.gpkg");
        let id = layers.list[0].id;
        assert!(layers.labels(58.0, 56.0).is_empty());
        layers.apply(id, &Act::Label(Some(0)));
        let labels = layers.labels(58.0, 56.0);
        assert_eq!(labels.len(), 3);
        let near = nearest(labels, 61.0, 50.5, 1);
        assert_eq!(near[0].0, "Дорога");
        // Ключ не меняется при малом сдвиге и меняется при большом.
        assert_eq!(label_key(58.0, 56.0, 10_000.0), label_key(58.001, 56.001, 10_000.0));
        assert_ne!(label_key(58.0, 56.0, 10_000.0), label_key(58.5, 56.0, 10_000.0));
    }

    #[test]
    fn identify_inside_polygon_not_in_hole() {
        let mut layers = DataLayers::default();
        layers.add(vec![layer()], "a.gpkg");
        let none = |_: f64, _: f64| None;
        let found = layers.identify(Lang::EN, 58.1, 56.1, 10.0, &none);
        assert_eq!(found[0].1[0].0, "Пруд");
        assert!(found[0].1[0].1.contains(&("h".to_string(), "3".to_string())));
        // В дыре и далеко от контура - ничего.
        assert!(layers.identify(Lang::EN, 58.5, 56.5, 10.0, &none).is_empty());
        // Точка - по зазору в долях допуска.
        let close = |lat: f64, _: f64| (lat == 60.0).then_some(0.5);
        assert_eq!(layers.identify(Lang::EN, 60.0, 50.0, 10.0, &close)[0].1[0].0, "Скважина");
    }
}
