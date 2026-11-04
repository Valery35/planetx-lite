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

/// Слой из папки меток расчёта: многоугольники и линии на любой глубине,
/// поля - name и поля меток по порядку первого появления. Цвет метки -
/// поле color «#rrggbb», если его нет: так слой рисует классы своими
/// цветами. None - в папке есть другие метки или меток нет.
pub fn folder_layer(folder: &scene::kml::Folder) -> Option<LayerData> {
    let places = folder.places();
    if places.is_empty() || places.iter().any(|p| !matches!(p.kind, Kind::Polygon | Kind::Line)) {
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
    /// Цвет объекта n: поле color «#rrggbb» слоя, иначе цвет стиля.
    fn item_color(&self, n: usize) -> [u8; 3] {
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
            if self.raised(n).is_some() {
                continue;
            }
            let style = Style { color: self.item_color(n), ..self.style };
            let (color, fill) = (style.line_rgba(), style.fill_rgba());
            match item.shape {
                Geom::Line => out.extend(item.parts.iter().filter(|p| p.len() > 1).map(|p| Line::new(p.clone(), color, self.style.width))),
                Geom::Polygon => {
                    let mut rings = item.parts.iter().filter(|p| p.len() > 2).cloned();
                    if let Some(outer) = rings.next() {
                        out.push(Line::polygon(outer, rings.collect(), color, self.style.width, fill));
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
            .filter(|(n, i)| i.shape == Geom::Point && self.raised(*n).is_none())
            .flat_map(|(_, i)| i.parts.iter().filter_map(|p| p.first().copied()))
            .collect()
    }

    /// Все точки слоя - для охвата.
    pub fn extent_points(&self) -> Vec<(f64, f64)> {
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
            .filter_map(|i| {
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
            let Some(height) = self.raised(n) else {
                continue;
            };
            let mut raise = |kind: Kind, points: Vec<(f64, f64)>, holes: Vec<Vec<(f64, f64)>>| {
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
        for (n, item) in self.data.items.iter().enumerate() {
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
    Up,
    Down,
    Remove,
    /// Флажок строки папки: все её слои показать или спрятать.
    FolderVisible(String, bool),
    /// Свернуть или развернуть папку.
    FolderCollapse(String),
    /// Убрать с глобуса все слои папки.
    FolderRemove(String),
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
}

impl DataLayers {
    fn changed(&mut self) {
        self.lines_dirty = true;
        self.dots_dirty = true;
        self.labels_dirty = true;
        self.shapes_dirty = true;
        self.save_dirty = true;
    }

    /// Слои файла file с геометрией - строками сверху списка. Таблицы
    /// без геометрии пропускаются. Возвращает количество слоёв.
    pub fn add(&mut self, layers: Vec<LayerData>, file: &str) -> usize {
        self.add_to(layers, file, "")
    }

    /// Слои с геометрией в папку folder - строками сверху списка, как
    /// [`DataLayers::add`]. Сюда же ложатся результаты окон анализа.
    pub fn add_to(&mut self, layers: Vec<LayerData>, file: &str, folder: &str) -> usize {
        let shown: Vec<LayerData> = layers.into_iter().filter(|l| l.items.iter().any(|i| i.shape != Geom::None)).collect();
        let single = shown.len() == 1;
        let count = shown.len();
        for (k, data) in shown.into_iter().enumerate() {
            let title = if single { file.to_string() } else { format!("{file} · {}", data.name) };
            let style = Style::new(self.next as usize);
            self.next += 1;
            let layer = DataLayer { id: self.next, title, file: file.to_string(), data, visible: true, style, label: None, extrude: None, folder: folder.to_string(), store: None };
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

    /// Папки в порядке первого слоя каждой, без пустой.
    pub fn folders(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for l in &self.list {
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
                    Act::Visible(on) => self.list[at].visible = *on,
                    Act::Style(s) => self.list[at].style = *s,
                    Act::Label(f) => self.list[at].label = *f,
                    Act::Extrude(e) => self.list[at].extrude = *e,
                    Act::Folder(f) => self.list[at].folder = f.trim().to_string(),
                    Act::Up if at > 0 => self.list.swap(at, at - 1),
                    Act::Down if at + 1 < self.list.len() => self.list.swap(at, at + 1),
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
            .map(|l| {
                let s = &l.style;
                let mut v = serde_json::json!({
                    "title": l.title, "file": l.file, "name": l.data.name, "folder": l.folder, "visible": l.visible,
                    "color": s.color, "width": s.width, "fill": s.fill, "opacity": s.opacity, "size": s.size,
                    "label": l.label, "extrude": l.extrude.map(|(f, k)| serde_json::json!([f, k])),
                });
                if inline {
                    v["data"] = serde_json::Value::String(data::layers::to_geojson(&l.data));
                } else if let Some(store) = &l.store {
                    v["store"] = serde_json::Value::String(store.clone());
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
        self.list.iter().filter(|l| l.store.is_none()).map(|l| (l.id, format!("layer-{}-{}.geojson", web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_millis()), l.id))).collect()
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
            let Some(mut data) = bytes.and_then(|b| data::layers::read(b, "layer.geojson").ok()).and_then(|mut l| (!l.is_empty()).then(|| l.remove(0))) else {
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
            out.list.push(DataLayer { id: out.next, title, file: text_of("file"), data, visible, style, label, extrude, folder: text_of("folder"), store });
        }
        out.collapsed = root.get("collapsed").and_then(serde_json::Value::as_array).into_iter().flatten().filter_map(|f| f.as_str().map(str::to_string)).collect();
        if !out.list.is_empty() {
            out.changed();
        }
        out.save_dirty = false;
        (out, lost)
    }

    /// Линии видимых слоёв для картинки наложения: нижняя строка
    /// списка - внизу стопки.
    pub fn line_set(&self) -> Option<Arc<Vec<Line>>> {
        let lines: Vec<Line> = self.list.iter().rev().filter(|l| l.visible).flat_map(DataLayer::lines).collect();
        (!lines.is_empty()).then(|| Arc::new(lines))
    }

    /// Точки видимых слоёв: точки, цвет, размер, непрозрачность.
    pub fn dot_sets(&self) -> Vec<DotSet> {
        self.list
            .iter()
            .rev()
            .filter(|l| l.visible)
            .map(|l| (l.dots(), l.style.color, l.style.size, l.style.opacity))
            .filter(|(d, _, _, _)| !d.is_empty())
            .collect()
    }

    /// Подписи видимых слоёв у точки взгляда (lat, lon).
    pub fn labels(&self, lat: f64, lon: f64) -> Vec<(String, f64, f64)> {
        self.list.iter().filter(|l| l.visible).flat_map(|l| nearest(l.label_spots(), lat, lon, MAX_LABELS)).collect()
    }

    /// Выдавленные объекты видимых слоёв.
    pub fn shapes(&self, radius: f64) -> Vec<Shape> {
        self.list.iter().filter(|l| l.visible).flat_map(|l| l.extruded(radius)).collect()
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
}

/// Строки «Моих слоёв» в разделе «Слои»: заголовок, слои вне папок,
/// затем папки со своими слоями. menu_open - открытое меню раздела,
/// new_folder - поле имени новой папки, out - действия по номерам слоёв.
pub fn rows(ui: &mut egui::Ui, lang: Lang, layers: &DataLayers, menu_open: &mut Option<String>, new_folder: &mut String, out: &mut Vec<(u64, Act)>) {
    if layers.list.is_empty() {
        return;
    }
    let p = crate::theme::pal();
    ui.add_space(2.0);
    ui.label(egui::RichText::new(tr(lang, "My layers")).size(12.0).color(p.muted))
        .on_hover_text(tr(lang, "Layers of files and results of the analysis windows. They stay between launches, as «My places». A scene does not carry them."));
    let folders = layers.folders();
    for layer in layers.list.iter().filter(|l| l.folder.is_empty()) {
        layer_row(ui, lang, layer, &folders, 4.0, menu_open, new_folder, out);
    }
    for folder in &folders {
        let inside: Vec<&DataLayer> = layers.list.iter().filter(|l| &l.folder == folder).collect();
        let shown = inside.iter().filter(|l| l.visible).count();
        let state = if shown == inside.len() { Some(true) } else if shown == 0 { Some(false) } else { None };
        let open = !layers.collapsed.contains(folder);
        let title = format!("{} {folder}", if open { "▾" } else { "▸" });
        let tip = trf(lang, "Folder of layers: {count}. The check shows or hides all its layers, a click on the name folds the folder.", &[("count", &inside.len().to_string())]);
        let hit = crate::layers::row(ui, 4.0, Some(state), false, &title, "⋯", &tip);
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
            if ui.button(tr(lang, "Remove the folder from the globe")).on_hover_text(tr(lang, "All layers of the folder leave «My layers».")).clicked() {
                out.push((0, Act::FolderRemove(folder.clone())));
                return true;
            }
            false
        });
        if open {
            for layer in inside {
                layer_row(ui, lang, layer, &folders, 22.0, menu_open, new_folder, out);
            }
        }
    }
}

/// Строка слоя с отступом indent и её меню.
#[allow(clippy::too_many_arguments)]
fn layer_row(ui: &mut egui::Ui, lang: Lang, layer: &DataLayer, folders: &[String], indent: f32, menu_open: &mut Option<String>, new_folder: &mut String, out: &mut Vec<(u64, Act)>) {
    let count = layer.data.items.len();
    let tip = trf(
        lang,
        "{file}, layer {layer}, objects: {count}. The check hides the layer, the «⋯» menu sets its style, labels, extrusion and folder.",
        &[("file", &layer.file), ("layer", &layer.data.name), ("count", &count.to_string())],
    );
    let hit = crate::layers::row(ui, indent, Some(Some(layer.visible)), false, &layer.title, "⋯", &tip);
    if hit.check {
        out.push((layer.id, Act::Visible(!layer.visible)));
    }
    let key = format!("data:{}", layer.id);
    if hit.menu || hit.text {
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
    ui.separator();
    chosen |= folder_menu(ui, lang, layer, folders, new_folder, out);
    if ui.button(tr(lang, "Up")).clicked() {
        out.push((id, Act::Up));
        chosen = true;
    }
    if ui.button(tr(lang, "Down")).clicked() {
        out.push((id, Act::Down));
        chosen = true;
    }
    if ui.button(tr(lang, "Copy to My Places")).on_hover_text(tr(lang, "The objects of the layer become placemarks of a folder in «My places»: they can be edited, measured and saved into a scene. A large layer is cut to its first objects.")).clicked() {
        out.push((id, Act::ToPlaces));
        chosen = true;
    }
    if ui.button(tr(lang, "Save as GeoJSON…")).on_hover_text(tr(lang, "The objects of the layer go to a GeoJSON file in WGS84 with all fields. QGIS and other GIS open it.")).clicked() {
        out.push((id, Act::Export));
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
        // Папка с точкой - не слой.
        let point = Place::new("Точка", Kind::Point, vec![(58.0, 56.0)]);
        assert!(folder_layer(&Folder::with("Точки", vec![KmlItem::Place(point)])).is_none());
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
        assert_eq!(layers.dot_sets()[0].0, vec![(60.0, 50.0)]);
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
        assert!((shapes[0].height - 6.0).abs() < 1e-9 && shapes[0].extrude);
        assert_eq!(shapes[0].holes.as_ref().map(Vec::len), Some(1));
        // В картинке наложения осталась только линия.
        assert_eq!(layers.line_set().unwrap().len(), 1);
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
