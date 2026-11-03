// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Классификация» - пункт подменю «Анализ участка» меню на
//! глобусе, перенос `ui/aefclass.py` модуля. Расчёт - `data::aef`
//! (`classify`), векторы участка - блоки файлов по сети
//! ([`crate::aefread`]).
//!
//! Просьба автора от 28 октября 2026 года по образцу «Классификации»
//! Google Earth. Пользователь заводит классы с цветом и названием, ставит
//! щелчком по глобусу контрольные точки выбранного класса, щелчок по
//! точке убирает её. «Построить» берёт векторы AlphaEarth участка на
//! выбранный год и относит каждую клетку к классу ближайших контрольных
//! точек. Векторы участка помнятся: новые точки дают новые классы без
//! новой загрузки. Окно - на общем каркасе анализа участка
//! ([`crate::analysis`]), «OK» кладёт в папку расчёта картинку классов
//! и контрольные точки и пишет классы файлом GeoTIFF в зоне UTM файлов.
//!
//! Источник клеток - AlphaEarth или растр, открытый в программе, просьба
//! Василия Швалёва от 31 октября 2026 года. Каналы растра - признаки
//! клетки, расчёт - `data::rasterclass`. Растр пересчитывается на сетку
//! участка и классы считаются частями за кадры.
//!
//! Флажок «Делить на объекты» - просьба автора от 1 ноября 2026 года,
//! перенос из модуля. Участок сначала делится на объекты, похожие по цвету
//! и текстуре (`data::segments`), класс получает объект по средним
//! и разбросам каналов. Итог - многоугольники в «Моих метках» вместо слоя
//! GeoPackage модуля.
//!
//! Окно «Похожие области» - тот же расчёт в режиме поиска по образцу
//! ([`ClassCalc::similar`]), перенос `ui/similar.py` модуля. Источник,
//! участок и объекты общие с классификацией, вместо классов - контуры
//! образцов щелчками и ползунок похожести, ядро - `data::similar`.
//!
//! Окно «Трассировка» - третий режим ([`ClassCalc::tracing`]), перенос
//! `ui/tracing.py` модуля 1.32. Источник - подложка глобуса или растр,
//! у AlphaEarth цвета нет. Снимок участка раскладывается на цвета, на
//! глобусе - цвета клеток, «OK» кладёт гладкие многоугольники областей
//! в «Мои метки» вместо слоя GeoPackage модуля. Ядро - `data::tracing`.
//! Неверный многоугольник модуль чинит MakeValid GDAL, у Lite починки
//! нет: кольцо с самопересечением остаётся как есть.
//!
//! Окно «Распознавание» - четвёртый режим ([`ClassCalc::recognition`]),
//! перенос `ui/recognize.py` модуля 1.38: снимки лета и снега подбирает
//! само окно ([`crate::recognize`]), образцы - точки или контуры.

use std::sync::Arc;

use data::aef::{self, Mosaic, Part, Piece, ZoneIndex};
use data::rasterclass::{self, Problem, Run, Source, Warp};
use data::segments::{self, Cut};
use data::similar;
use data::tilemosaic::{self, TileWarp};
use data::tracing::{self, Traced, Tracer};
use eframe::egui;
use globe_core::DVec3;
use globe_core::lod::TileKey;
use globe_core::navigation::{Navigator, ground_under};
use scene::features::Kind;
use scene::kml::{Item, Place};

use crate::aefcalc::{Indexes, Request, utm_epsg};
use crate::aefread::BlockRead;
use crate::analysis::{self, Button, Saved, SiteEdit};
use crate::basemapread::BaseRead;
use crate::i18n::{Lang, tr, trf};
use crate::objecttasks::Task;
use crate::recognize::{self, Recognize};

/// Участок по умолчанию и наибольший, километры, `CLASS_SIDE` модуля.
const SIDE: f64 = 3.0;
const MOST: f64 = 10.0;
/// Наибольший участок растра, километры, `RASTER_SIDE` модуля. Большую
/// сетку отсекает `rasterclass::too_big`.
const RASTER_MOST: f64 = 1000.0;
/// Цвета новых классов по порядку - выбор помощника модуля.
pub const COLORS: [[u8; 3]; 8] = [[60, 160, 70], [40, 110, 220], [240, 190, 40], [200, 60, 50], [150, 90, 200], [120, 120, 120], [240, 130, 40], [60, 200, 200]];
const OPACITY: f64 = 0.6;
/// Время распаковки каналов за кадр, секунды.
const BUDGET: f64 = 0.012;
/// Логических пикселей вокруг контрольной точки, в них она убирается.
const POINT_PIXELS: f64 = 8.0;
/// Строк растра и клеток классов за один шаг внутри бюджета кадра.
const WARP_ROWS: usize = 4;
const RUN_CELLS: usize = 4096;
/// Граница объектов на глобусе, `BORDER` модуля.
const BORDER: [u8; 4] = [20, 20, 20, 255];
/// Многоугольников в «Моих метках» не больше: большое количество меток
/// занимает кадр. Дальше итог - растр классов.
const MOST_OBJECTS: usize = 5000;
/// Упрощение контура объекта, ячеек. Контур по клеткам идёт ступеньками.
const OUTLINE: f64 = 0.75;
/// Цвета найденного и образцов - выбор помощника модуля.
const FOUND_COLOR: [u8; 3] = [255, 120, 30];
const SAMPLE_COLOR: [u8; 3] = [255, 230, 60];
/// Новая выборка после паузы ползунка, секунды, `PAUSE` модуля.
const PAUSE: f64 = 0.15;
/// Пиксели списка «Пиксель», метры, `PIXELS` модуля. Первые [`FINE`] -
/// только у растра и подложки: у AlphaEarth мельче 10 м данных нет,
/// дальше - уровни AlphaEarth `aef::DETAIL_LEVELS` по порядку.
pub const PIXELS: [f64; 7] = [1.0, 2.0, 5.0, 10.0, 20.0, 40.0, 80.0];
pub const FINE: usize = 3;
/// Источник «Подложка глобуса» в списке, `BASEMAP` модуля.
pub const BASEMAP: &str = "@basemap";
/// Участок трассировки по умолчанию, километры, и непрозрачность её
/// цветов на глобусе - `Area(..., 1.0, ...)` и `OPACITY` окна модуля.
const TRACE_SIDE: f64 = 1.0;
const TRACE_OPACITY: f64 = 0.8;

/// Строк таблицы классов без прокрутки, `TABLE_ROWS` модуля, в точках
/// высоты строки.
const TABLE_HEIGHT: f32 = 6.0 * 24.0;

/// Трассировка: настройки, итог до слияния цветов, таблица классов
/// и итог по таблице для «OK».
struct Trace {
    params: tracing::Params,
    base: Option<TraceBase>,
    /// Строки таблицы классов по убыванию доли участка.
    rows: Vec<ColorRow>,
    joined: Option<Joined>,
    /// Окно «Выделение объектов»: задача поверх трассировки.
    task: Option<Box<Task>>,
    /// Вопрос помощнику о названиях: системный текст и таблица. Его
    /// забирает окно глобуса.
    ask: Option<(String, String)>,
    /// Ответ помощника ждётся.
    waiting: bool,
}

/// Итог трассировки до таблицы классов, `base` модуля 1.33: цвета,
/// их названия и настройки расчёта.
pub struct TraceBase {
    pub done: Traced,
    pub colors: Vec<[u8; 3]>,
    pub hexes: Vec<String>,
    pub names: Vec<String>,
    /// Настройки строками для названия результата.
    pub settings: Vec<String>,
    pub index: Arc<ZoneIndex>,
    pub x0: f64,
    pub y0: f64,
    pub step: f64,
    pub rows: usize,
    pub cols: usize,
}

/// Итог по таблице классов: цвета клеток и областей после слияния,
/// цвета, названия и коды классов, свои поля областей.
#[derive(Clone)]
pub struct Joined {
    pub classes: Vec<i32>,
    pub kinds: Vec<i16>,
    pub colors: Vec<[u8; 3]>,
    pub names: Vec<String>,
    pub hexes: Vec<String>,
    /// Поля областей: имя и значение по номеру области.
    pub fields: Vec<(&'static str, Vec<String>)>,
}

/// Строка таблицы классов: номер цвета, название, доля участка, цвет,
/// с которым слит, -1 - свой класс.
struct ColorRow {
    number: usize,
    name: String,
    share: f64,
    target: i32,
}

/// Подложка на глобусе для источника «Подложка глобуса»: название,
/// источник тайлов, подпись прав.
#[derive(Debug, Clone, PartialEq)]
pub struct Backdrop {
    pub name: String,
    pub source: net::Source,
    pub credit: String,
}

/// Класс: название, цвет, контрольные точки и контуры образцов - у окна
/// «Распознавание».
#[derive(Debug, Clone, PartialEq)]
pub struct Class {
    pub name: String,
    pub color: [u8; 3],
    pub points: Vec<(f64, f64)>,
    pub rings: Vec<Vec<(f64, f64)>>,
}

impl Class {
    pub fn new(name: String, color: [u8; 3]) -> Self {
        Self { name, color, points: Vec::new(), rings: Vec::new() }
    }

    /// Есть ли у класса образцы - точки или контуры.
    pub fn has_samples(&self) -> bool {
        !self.points.is_empty() || !self.rings.is_empty()
    }

    /// Образцов хватает: три точки или контур, `enough` модуля 1.38.
    pub fn enough(&self) -> bool {
        self.points.len() >= aef::MIN_POINTS || !self.rings.is_empty()
    }
}

/// Открытый растр: имя и байты файла.
pub type Raster = (String, Arc<Vec<u8>>);

/// Сетка растра на участке: ключ (растр, рамка, уровень) и сетка.
type OwnGrid = ((String, String, Option<usize>), Arc<ZoneIndex>, Arc<Mosaic<f32>>);

/// Шаг расчёта по растру.
enum OwnStage {
    /// Файл ещё не разобран: кадр со строкой «Читается…» уже показан.
    Read(Arc<Vec<u8>>),
    Warp(Box<(Source, Warp)>),
    Classes { grid: Arc<Mosaic<f32>>, run: Box<Run>, outside: usize, share: f64 },
    /// Тайлы подложки по сети, потом пересчёт на сетку участка. Флаг -
    /// тайлы получены.
    Tiles(Box<(BaseRead, TileWarp, bool)>),
    /// Деление участка на объекты проходами.
    Cut { grid: Arc<Mosaic<f32>>, cut: Box<Cut>, start: usize, share: f64 },
    /// Классы объектов: table - признаки расчёта, plain - средние
    /// и разбросы каналов для полей объектов.
    Kinds { grid: Arc<Mosaic<f32>>, parts: Arc<Vec<i32>>, table: Vec<f32>, plain: Vec<f32>, run: Box<Run>, outside: usize, share: f64 },
    /// Трассировка шагами.
    Trace { grid: Arc<Mosaic<f32>>, tracer: Box<Tracer>, share: f64 },
    /// Каналы второго снимка или снимков сезонов на сетке первого: сетка,
    /// растры в очереди, идущий пересчёт, каналы готовых и доли участка
    /// с данными.
    More { grid: Arc<Mosaic<f32>>, queue: Vec<(String, Arc<Vec<u8>>)>, current: Option<Box<(Source, Warp)>>, parts: Vec<(Vec<f32>, usize)>, shares: Vec<f64> },
}

/// Сетка участка готова к делению на объекты.
struct Ready {
    key: (String, String, Option<usize>),
    index: Arc<ZoneIndex>,
    grid: Arc<Mosaic<f32>>,
    share: f64,
}

/// Настройки деления на объекты: масштаб, доля текстуры в процентах,
/// наименьший объект, слияние соседей одного класса.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Split {
    on: bool,
    scale: u32,
    weight: u32,
    least: u32,
    join: bool,
}

/// Объекты участка для «Моих меток».
pub struct Objects {
    /// Номера объектов клеток, -1 без объекта.
    pub parts: Arc<Vec<i32>>,
    /// Классы объектов.
    pub kinds: Vec<i16>,
    /// Средние и разбросы каналов объектов, по 2 канала на объект.
    pub table: Vec<f32>,
    /// Каналов у растра, 0 - поля каналов не пишутся.
    pub bands: usize,
    /// Похожесть объектов в долях, у окна «Похожие области».
    pub similar: Option<Vec<f64>>,
}

/// Готовый поиск: от него ползунок считает выборку без расчёта.
struct Base {
    grid: Arc<Mosaic<f32>>,
    index: Arc<ZoneIndex>,
    parts: Arc<Vec<i32>>,
    score: Vec<f64>,
    /// Контуров вне объектов участка.
    lost: usize,
    /// Клеток в объектах.
    cells: usize,
}

/// Поиск по образцу: контуры, порог, готовый расчёт.
struct Find {
    /// Замкнутые контуры образцов.
    rings: Vec<Vec<(f64, f64)>>,
    /// Контур, который сейчас обводится.
    ring: Vec<(f64, f64)>,
    /// Порог похожести, проценты.
    limit: u32,
    join: bool,
    base: Option<Base>,
    /// Ползунок сдвинут: когда считать новую выборку.
    moved: Option<web_time::Instant>,
}

/// Расчёт по растру.
struct OwnJob {
    key: (String, String, Option<usize>),
    index: Arc<ZoneIndex>,
    bbox: (f64, f64, f64, f64),
    side: f64,
    stage: OwnStage,
}

/// Векторы участка: ключ (рамка, год, уровень) и мозаика.
type Vectors = ((String, i32, usize), Arc<ZoneIndex>, Arc<Mosaic<i8>>);

/// Загрузка векторов участка.
struct Job {
    zone: String,
    bbox: (f64, f64, f64, f64),
    year: i32,
    level: Option<usize>,
    index: Option<Arc<ZoneIndex>>,
    parts: Vec<Part>,
    next: usize,
    read: Option<(BlockRead, Vec<i8>, u16)>,
    pieces: Vec<Piece<i8>>,
}

/// Слой классов для картинок тайлов.
pub struct Layer {
    pub index: Arc<ZoneIndex>,
    pub step: f64,
    pub x0: f64,
    pub y0: f64,
    pub rows: usize,
    pub cols: usize,
    /// Номер класса клетки, -1 - нет класса.
    pub classes: Vec<i16>,
    pub colors: Vec<[u8; 3]>,
    /// Названия классов по номеру для шкалы на глобусе, пусто - шкалы
    /// нет (похожие области).
    pub names: Vec<String>,
    pub opacity: f64,
    /// Объекты, если участок поделён на них.
    pub objects: Option<Arc<Objects>>,
}

impl Layer {
    /// Номер объекта точек, -1 вне объектов. Граница - по пикселям тайла:
    /// клетка границы на редкой выборке дала бы пунктир.
    fn parts_at(&self, parts: &[i32], points: &[(f64, f64)]) -> Vec<i32> {
        points
            .iter()
            .map(|&(lat, lon)| {
                let (x, y) = self.index.project(lat, lon);
                let (c, r) = (((x - self.x0) / self.step).floor(), ((self.y0 - y) / self.step).floor());
                if c >= 0.0 && r >= 0.0 && (c as usize) < self.cols && (r as usize) < self.rows { parts.get(r as usize * self.cols + c as usize).copied().unwrap_or(-1) } else { -1 }
            })
            .collect()
    }

    /// Цвета точек (широта, долгота) с премноженной альфой.
    pub fn sample(&self, points: &[(f64, f64)]) -> Vec<[u8; 4]> {
        let alpha = (self.opacity * 255.0).round() as u8;
        points
            .iter()
            .map(|&(lat, lon)| {
                let (x, y) = self.index.project(lat, lon);
                let (c, r) = (((x - self.x0) / self.step).floor(), ((self.y0 - y) / self.step).floor());
                if !(c >= 0.0 && r >= 0.0 && (c as usize) < self.cols && (r as usize) < self.rows) {
                    return [0; 4];
                }
                let class = self.classes.get(r as usize * self.cols + c as usize).copied().unwrap_or(-1);
                match usize::try_from(class).ok().and_then(|k| self.colors.get(k)) {
                    Some(rgb) => {
                        let c = |v: u8| (f64::from(v) * self.opacity).round() as u8;
                        [c(rgb[0]), c(rgb[1]), c(rgb[2]), alpha]
                    }
                    None => [0; 4],
                }
            })
            .collect()
    }

    /// Рамка в градусах (юг, север, запад, восток).
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        let (w, h) = (self.step * self.cols as f64, self.step * self.rows as f64);
        let pts: Vec<(f64, f64)> = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].iter().map(|&(dx, dy)| aef::from_utm(self.index.number, self.index.south, self.x0 + dx, self.y0 - dy)).collect();
        let lats = pts.iter().map(|p| p.0);
        let lons = pts.iter().map(|p| p.1);
        (lats.clone().fold(f64::INFINITY, f64::min), lats.fold(f64::NEG_INFINITY, f64::max), lons.clone().fold(f64::INFINITY, f64::min), lons.fold(f64::NEG_INFINITY, f64::max))
    }
}

impl Layer {
    /// Цвета сетки точек lats × lons (строки с севера) с премноженной
    /// альфой и границами объектов.
    pub fn sample_grid(&self, lats: &[f64], lons: &[f64]) -> Vec<[u8; 4]> {
        let points: Vec<(f64, f64)> = lats.iter().flat_map(|&la| lons.iter().map(move |&lo| (la, lo))).collect();
        let mut pixels = self.sample(&points);
        if let Some(objects) = &self.objects {
            let found = self.parts_at(&objects.parts, &points);
            for (pixel, edge) in pixels.iter_mut().zip(segments::borders(&found, lats.len(), lons.len())) {
                if edge {
                    *pixel = BORDER;
                }
            }
        }
        pixels
    }
}

/// Картинки тайлов слоя.
pub fn painter(layer: Arc<Layer>) -> crate::vector::Painter {
    let (south, north, west, east) = layer.bbox();
    Arc::new(move |key: TileKey| {
        let (lats, lons) = crate::pointcalc::tile_axes(key);
        let (top, bottom) = (lats.first().copied()?, lats.last().copied()?);
        let (left, right) = (lons.first().copied()?, lons.last().copied()?);
        if bottom > north || top < south || left > east || right < west {
            return None;
        }
        let pixels = layer.sample_grid(&lats, &lons);
        pixels.iter().any(|p| p[3] > 0).then(|| pixels.concat())
    })
}

/// Сторона картинки предпросмотра поверх слоёв, пиксели.
pub const PREVIEW_SIDE: usize = 2048;

/// Предпросмотр слоя картинкой на поверхности поверх картинок «Моих
/// меток», как предпросмотр анализа над слоями проекта модуля 1.30:
/// сохранённый прежде расчёт и снимок из KML его не закрывают. None -
/// сетка слоя крупнее [`PREVIEW_SIDE`], картинка огрубила бы клетки, тогда
/// предпросмотр идёт картинками тайлов.
pub fn preview_overlay(layer: &Layer) -> Option<scene::kml::Overlay> {
    if layer.rows.max(layer.cols) > PREVIEW_SIDE || layer.rows == 0 || layer.cols == 0 {
        return None;
    }
    let (south, north, west, east) = layer.bbox();
    // Пиксель картинки - не крупнее половины клетки по широте и долготе.
    let rows = (2 * layer.rows).clamp(1, PREVIEW_SIDE);
    let cols = (2 * layer.cols).clamp(1, PREVIEW_SIDE);
    let (dlat, dlon) = ((north - south) / rows as f64, (east - west) / cols as f64);
    let lats: Vec<f64> = (0..rows).map(|r| north - (r as f64 + 0.5) * dlat).collect();
    let lons: Vec<f64> = (0..cols).map(|c| west + (c as f64 + 0.5) * dlon).collect();
    let rgba: Vec<u8> = layer.sample_grid(&lats, &lons).into_iter().flat_map(analysis::unpremultiply).collect();
    let image = image::RgbaImage::from_raw(u32::try_from(cols).ok()?, u32::try_from(rows).ok()?, rgba)?;
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).ok()?;
    let mut ground = scene::overlays::Overlay::new(scene::overlays::Kind::Ground);
    ground.set_box([north, south, east, west, 0.0]);
    ground.order = i64::MAX;
    let mut overlay = scene::kml::Overlay::new("preview", ground);
    overlay.href = "planetx:preview".to_string();
    overlay.image = Some(png.into_inner());
    Some(overlay)
}

/// «Уточнить точками»: участок, снимок и классы окна задач.
pub type Refine = (Vec<(f64, f64)>, Option<String>, Vec<([u8; 3], String)>);

/// Окно «Классификация».
pub struct ClassCalc {
    pub open: bool,
    place: String,
    pub site: SiteEdit,
    year: i32,
    level: Option<usize>,
    opacity: f64,
    classes: Vec<Class>,
    /// Класс, точки которого ставит щелчок по глобусу.
    active: usize,
    status: String,
    job: Option<Job>,
    vectors: Option<Vectors>,
    pub layer: Option<Arc<Layer>>,
    pub changed: bool,
    pub saved: Option<Saved>,
    indexes: Indexes,
    /// Растры, открытые в программе, и выбранный, None - AlphaEarth.
    pub rasters: Vec<Raster>,
    source: Option<String>,
    own_job: Option<OwnJob>,
    own_grid: Option<OwnGrid>,
    /// Подпись расчёта в «Моих метках»: год или имя растра.
    label: String,
    split: Split,
    /// Готовые объекты: ключ участка и настроек, номера объектов. Смена
    /// точек пересчитывает только классы.
    pieces: Option<(String, Arc<Vec<i32>>)>,
    /// Окно «Похожие области», None - «Классификация».
    find: Option<Find>,
    /// Пиксель растра у участка, метры: имя источника и пиксель. Строка
    /// под полем «Пиксель» называет по нему шаг «Авто».
    native: Option<(String, f64)>,
    /// Подложка на глобусе, её ставит окно глобуса каждый кадр.
    pub basemap: Option<Backdrop>,
    /// Адреса подложек, условия которых для расчёта приняты, из настроек.
    pub accepted: Vec<String>,
    /// Адрес подложки, условия которой только что приняты: окно глобуса
    /// пишет его в настройки.
    pub accept_new: Option<String>,
    /// Окно условий подложки открыто.
    terms: bool,
    /// Подпись источника расчёта по подложке.
    credit: String,
    /// Окно «Трассировка», None - другие режимы.
    trace: Option<Trace>,
    /// Растры, чьи каналы ложатся на сетку за каналами источника, -
    /// второй снимок или снимки сезонов задачи. Ставит «Построить».
    extras: Vec<String>,
    /// Доли участка с данными источника и растров за ним.
    cover: Vec<f64>,
    /// Загруженные окном снимки: окно глобуса открывает их растрами
    /// программы.
    pub loaded: Vec<Raster>,
    /// Нажата «Классы из трассировки»: классы ставит окно глобуса
    /// ([`ClassCalc::take_traced`]).
    pub wants_traced: bool,
    /// Нажата «Уточнить точками»: «Классификацию» открывает окно глобуса
    /// ([`ClassCalc::take_refine`]).
    wants_refine: bool,
    /// Настройки последнего «Построить» строками для названия результата.
    settings: Vec<String>,
    /// Окно «Распознавание», None - другие режимы.
    recognize: Option<Box<Recognize>>,
}

impl ClassCalc {
    /// Окно «Трассировка», `TraceDialog` модуля. Источник по умолчанию -
    /// подложка глобуса.
    pub fn tracing(lat: f64, lon: f64, place: String, rasters: Vec<Raster>, lang: Lang) -> Self {
        let mut calc = Self::new(lat, lon, place, rasters, lang);
        calc.site = SiteEdit::new(lat, lon, TRACE_SIDE, RASTER_MOST);
        calc.source = Some(BASEMAP.to_string());
        calc.opacity = TRACE_OPACITY;
        calc.trace = Some(Trace { params: tracing::Params::default(), base: None, rows: Vec::new(), joined: None, task: None, ask: None, waiting: false });
        calc
    }

    /// Окно «Выделение объектов», `TaskDialog` модуля 1.33-1.36:
    /// трассировка с типовой задачей. Источник - снимок с подписями
    /// каналов.
    pub fn objects(lat: f64, lon: f64, place: String, rasters: Vec<Raster>, lang: Lang) -> Self {
        let mut calc = Self::tracing(lat, lon, place, rasters, lang);
        calc.source = None;
        if let Some(trace) = &mut calc.trace {
            let task = Task::default();
            let spec = task.scenario.spec();
            trace.params = tracing::Params { colors: spec.colors, smooth: spec.smooth, least: spec.least, ..tracing::Params::default() };
            trace.task = Some(Box::new(task));
        }
        calc
    }

    /// Окно «Распознавание», `RecognizeDialog` модуля 1.38: участок до
    /// [`recognize::SIDE_MOST`], источник - летний снимок таблицы, пиксель
    /// «Авто», деления на объекты нет.
    pub fn recognition(lat: f64, lon: f64, place: String, rasters: Vec<Raster>, lang: Lang) -> Self {
        let mut calc = Self::new(lat, lon, place, rasters, lang);
        calc.site = SiteEdit::new(lat, lon, SIDE, recognize::SIDE_MOST);
        calc.source = None;
        calc.recognize = Some(Box::default());
        calc
    }

    /// Окно «Распознавание».
    fn recognize_mode(&self) -> bool {
        self.recognize.is_some()
    }

    /// Образцов хватает: не меньше двух классов с образцами, у каждого
    /// три точки или контур.
    fn samples_ready(&self) -> bool {
        let rows: Vec<&Class> = self.classes.iter().filter(|c| c.has_samples()).collect();
        rows.len() >= aef::MIN_CLASSES && rows.iter().all(|c| c.enough())
    }

    fn too_few_text(&self, lang: Lang) -> &'static str {
        if self.recognize_mode() { tr(lang, "At least two classes are needed, each with an outline or three points.") } else { tr(lang, "At least two classes of three points each are needed.") }
    }

    /// Снимки «Распознавания» перед расчётом, `build` модуля 1.38: подбор
    /// и загрузка, затем источник - летний снимок, за ним каналы снимка со
    /// снегом. None - снимки ещё подбираются или их нет, причина - в строке
    /// окна.
    fn recognize_shots(&mut self, lang: Lang) -> Option<Vec<String>> {
        let ring = self.site.site.ring.clone();
        let rec = self.recognize.as_mut()?;
        if rec.picker.busy() {
            self.status = tr(lang, "The images are still loading.").to_string();
            return None;
        }
        if !rec.picker.fit(lang, &ring, &self.rasters) {
            rec.pending = true;
            return None;
        }
        let Some(summer) = rec.picker.raster(recognize::Key::Summer).map(str::to_string) else {
            self.status = tr(lang, "No summer image. A click on the row «Summer» - another date or an opened raster.").to_string();
            return None;
        };
        let snow = rec.picker.raster(recognize::Key::Snow).filter(|s| *s != summer).map(str::to_string);
        self.source = Some(summer);
        self.level = None;
        self.split.on = false;
        Some(snow.into_iter().collect())
    }

    /// Шаг подбора снимков «Распознавания»: загруженные снимки - растры
    /// программы, конец подбора продолжает «Построить».
    fn step_recognize(&mut self, lang: Lang, request: Request) {
        let Some(rec) = self.recognize.as_mut() else {
            return;
        };
        rec.picker.step(lang, request);
        let text = rec.picker.status.take();
        let loaded: Vec<Raster> = std::mem::take(&mut rec.picker.loaded);
        let done = std::mem::take(&mut rec.picker.done);
        let go = done && std::mem::take(&mut rec.pending) && rec.picker.raster(recognize::Key::Summer).is_some();
        if let Some(text) = text {
            self.status = text;
        }
        for raster in loaded {
            self.rasters.retain(|r| r.0 != raster.0);
            self.rasters.push(raster.clone());
            self.loaded.push(raster);
        }
        if go && self.open {
            self.build(lang);
        }
    }

    /// Контуры образцов с номером класса среди классов с образцами.
    fn rings(&self) -> Vec<(i32, Vec<(f64, f64)>)> {
        self.classes.iter().filter(|c| c.has_samples()).enumerate().flat_map(|(n, c)| c.rings.iter().map(move |r| (n as i32, r.clone()))).collect()
    }

    /// Контур образца с точкой внутри: (класс, номер), последний
    /// нарисованный - первым, `ring_under` модуля 1.38.
    pub fn ring_under(&self, lat: f64, lon: f64) -> Option<(usize, usize)> {
        self.classes.iter().enumerate().rev().find_map(|(k, c)| c.rings.iter().rposition(|r| data::site::inside(&[(lat, lon)], r).first() == Some(&true)).map(|n| (k, n)))
    }

    /// Окно «Выделение объектов».
    fn task_mode(&self) -> bool {
        self.trace.as_ref().is_some_and(|t| t.task.is_some())
    }

    /// Название окна трассировки или задач.
    fn tool_title(&self, lang: Lang) -> &'static str {
        if self.task_mode() { tr(lang, "Object extraction") } else { tr(lang, "Tracing") }
    }

    /// Вопрос помощнику о названиях классов, его забирает окно глобуса.
    pub fn take_ask(&mut self) -> Option<(String, String)> {
        let trace = self.trace.as_mut()?;
        let ask = trace.ask.take()?;
        trace.waiting = true;
        Some(ask)
    }

    /// Помощник не взял вопрос: занят или не подключён.
    pub fn ask_refused(&mut self, lang: Lang) {
        if let Some(trace) = &mut self.trace {
            trace.waiting = false;
        }
        self.status = tr(lang, "The assistant is busy with the previous question or not connected.").to_string();
    }

    /// Ждёт ли окно ответа помощника.
    pub fn waits_names(&self) -> bool {
        self.trace.as_ref().is_some_and(|t| t.waiting)
    }

    /// Классы трассировки (цвет, название) по убыванию доли - для окна
    /// «Классификация», `class_pairs` модуля. Пусто, если итога нет.
    pub fn class_pairs(&self) -> Vec<([u8; 3], String)> {
        let Some(joined) = self.trace.as_ref().and_then(|t| t.joined.as_ref()) else {
            return Vec::new();
        };
        let shares = color_shares(&joined.classes, joined.colors.len());
        let mut order: Vec<usize> = (0..joined.colors.len()).filter(|n| shares[*n] > 0.0).collect();
        order.sort_by(|a, b| shares[*b].total_cmp(&shares[*a]));
        order.into_iter().map(|n| (joined.colors[n], joined.names[n].clone())).collect()
    }

    /// «Классы из трассировки», `take_traced` модуля 1.33: классы без
    /// точек заменяются классами трассировки с их названиями и цветами.
    pub fn take_traced(&mut self, lang: Lang, pairs: &[([u8; 3], String)]) {
        self.wants_traced = false;
        if pairs.is_empty() {
            self.status = tr(lang, "The «Tracing» window has no built classes.").to_string();
            return;
        }
        self.classes.retain(Class::has_samples);
        for (color, name) in pairs {
            if !self.classes.iter().any(|c| c.name == *name) {
                self.classes.push(Class::new(name.clone(), *color));
            }
        }
        self.active = self.active.min(self.classes.len().saturating_sub(1));
        self.status = trf(lang, "Classes from tracing: {n}.", &[("n", &pairs.len().to_string())]);
    }

    /// «Уточнить точками», `refine` модуля 1.37: участок, снимок и классы
    /// окна задач для окна «Классификация». None - итога нет, причина -
    /// в строке окна.
    pub fn take_refine(&mut self, lang: Lang) -> Option<Refine> {
        if !std::mem::take(&mut self.wants_refine) {
            return None;
        }
        let pairs = self.class_pairs();
        if pairs.is_empty() || self.site.site.ring.len() < 3 {
            self.status = tr(lang, "First press «Build».").to_string();
            return None;
        }
        Some((self.site.site.ring.clone(), self.source.clone(), pairs))
    }

    /// Классы окна задач в «Классификации» с тем же снимком.
    pub fn refine_from(&mut self, lang: Lang, source: Option<String>, pairs: &[([u8; 3], String)]) {
        if source.as_ref().is_some_and(|s| self.rasters.iter().any(|r| r.0 == *s)) {
            self.source = source;
        }
        self.take_traced(lang, pairs);
        self.status = tr(lang, "Classes from the «Object extraction» window. Choose a class with its circle and click its samples on the globe, at least three points per class, then «Build».").to_string();
    }

    /// Ответ помощника о названиях, `_names_came` модуля: названия -
    /// в таблицу, классы с одинаковым названием сливаются с самым крупным.
    pub fn names_came(&mut self, lang: Lang, answer: &str, error: &str) {
        let Some(trace) = &mut self.trace else {
            return;
        };
        trace.waiting = false;
        let count = trace.rows.len();
        if count == 0 {
            return;
        }
        let Some(names) = data::classnames::parse_names(answer, count) else {
            let reason = if !error.is_empty() { error.to_string() } else if answer.is_empty() { tr(lang, "empty answer").to_string() } else { answer.chars().take(200).collect() };
            self.status = trf(lang, "The assistant gave no names: {error}", &[("error", &reason)]);
            return;
        };
        let mut first: Vec<(String, usize)> = Vec::new();
        for row in &mut trace.rows {
            let name = names.get(row.number).cloned().unwrap_or_default();
            row.target = first.iter().find(|(n, _)| *n == name).map_or(-1, |(_, k)| *k as i32);
            if row.target < 0 {
                first.push((name.clone(), row.number));
            }
            row.name = name;
        }
        self.apply_trace(lang);
        self.status = tr(lang, "The names were proposed by the assistant, they can be corrected. Classes with the same name are merged.").to_string();
    }

    /// Окно «Похожие области», `SimilarDialog` модуля.
    pub fn similar(lat: f64, lon: f64, place: String, rasters: Vec<Raster>, lang: Lang) -> Self {
        let mut calc = Self::new(lat, lon, place, rasters, lang);
        calc.split.on = true;
        calc.find = Some(Find { rings: Vec::new(), ring: Vec::new(), limit: similar::LIMIT, join: true, base: None, moved: None });
        calc
    }

    pub fn new(lat: f64, lon: f64, place: String, rasters: Vec<Raster>, lang: Lang) -> Self {
        // Файлы AlphaEarth странице недоступны, источник - только растр.
        let source = if cfg!(target_arch = "wasm32") { rasters.first().map(|r| r.0.clone()) } else { None };
        let mut calc = Self {
            open: true,
            place,
            site: SiteEdit::new(lat, lon, SIDE, MOST),
            year: aef::YEARS[aef::YEARS.len() - 1],
            level: None,
            opacity: OPACITY,
            classes: Vec::new(),
            active: 0,
            status: String::new(),
            job: None,
            vectors: None,
            layer: None,
            changed: false,
            saved: None,
            indexes: Indexes::default(),
            rasters,
            source,
            own_job: None,
            own_grid: None,
            label: String::new(),
            split: Split { on: false, scale: segments::SCALE, weight: (100.0 * segments::WEIGHT) as u32, least: segments::LEAST, join: false },
            pieces: None,
            find: None,
            native: None,
            basemap: None,
            accepted: Vec::new(),
            accept_new: None,
            terms: false,
            credit: String::new(),
            trace: None,
            extras: Vec::new(),
            cover: Vec::new(),
            loaded: Vec::new(),
            wants_traced: false,
            wants_refine: false,
            settings: Vec::new(),
            recognize: None,
        };
        calc.add_class(lang);
        calc.add_class(lang);
        calc.active = 0;
        calc
    }

    fn add_class(&mut self, lang: Lang) {
        let n = self.classes.len();
        self.classes.push(Class::new(trf(lang, "Class {n}", &[("n", &(n + 1).to_string())]), COLORS[n % COLORS.len()]));
        self.active = n;
    }

    pub fn set_point(&mut self, lat: f64, lon: f64, place: String) {
        self.place = place;
        self.status.clear();
        self.open = true;
        self.site.move_to(lat, lon);
    }

    pub fn busy(&self) -> bool {
        self.job.is_some() || self.own_job.is_some()
    }

    /// Идёт поиск или загрузка снимков задачи.
    pub fn finding(&self) -> bool {
        self.trace.as_ref().and_then(|t| t.task.as_ref()).is_some_and(|t| t.busy()) || self.recognize.as_ref().is_some_and(|r| r.picker.busy())
    }

    pub fn site_edit(&mut self) -> Option<&mut SiteEdit> {
        if self.open { Some(&mut self.site) } else { None }
    }

    fn clear(&mut self) {
        self.job = None;
        self.own_job = None;
        if self.layer.take().is_some() {
            self.changed = true;
        }
        if let Some(find) = &mut self.find {
            find.base = None;
            find.ring.clear();
            find.moved = None;
        }
        if let Some(trace) = &mut self.trace {
            trace.base = None;
            trace.rows.clear();
            trace.joined = None;
            if let Some(task) = &mut trace.task {
                task.stop();
            }
        }
        if let Some(rec) = &mut self.recognize {
            rec.picker.stop();
            rec.drawing.clear();
            rec.pending = false;
        }
        self.status.clear();
    }

    /// Ключ векторов участка.
    fn key(&self, level: usize) -> (String, i32, usize) {
        let (w, s, e, n) = self.site.site.bbox();
        (format!("{w:.6},{s:.6},{e:.6},{n:.6}"), self.year, level)
    }

    fn build(&mut self, lang: Lang) {
        self.extras.clear();
        self.cover.clear();
        if let Some(task) = self.trace.as_mut().and_then(|t| t.task.as_mut()) {
            // Задаче нужен снимок с известными каналами, второй снимок или
            // снимки сезонов ложатся на ту же сетку.
            match task.before(lang, &self.rasters, self.source.as_deref()) {
                Ok(extras) => self.extras = extras,
                Err(text) => {
                    self.status = text;
                    return;
                }
            }
        } else if self.trace.is_some() {
            // У трассировки нет ни точек, ни образцов.
        } else if let Some(find) = &self.find {
            if find.rings.is_empty() {
                self.status = tr(lang, "Outline a sample on the globe.").to_string();
                return;
            }
        } else {
            if !self.samples_ready() {
                self.status = self.too_few_text(lang).to_string();
                return;
            }
        }
        if let Some(problem) = self.site.problem(lang) {
            self.status = problem;
            return;
        }
        if self.recognize.is_some() {
            match self.recognize_shots(lang) {
                Some(extras) => self.extras = extras,
                None => return,
            }
        }
        if self.trace.is_none() && self.find.is_none() {
            // Настройки классификации для названия результата,
            // `class_settings` модуля 1.33 и 1.38.
            let classes = self.classes.iter().filter(|c| c.has_samples()).count();
            let points: usize = self.classes.iter().map(|c| c.points.len()).sum();
            let areas: usize = self.classes.iter().map(|c| c.rings.len()).sum();
            self.settings = vec![trf(lang, "classes {n}", &[("n", &classes.to_string())])];
            if points > 0 || areas == 0 {
                self.settings.push(trf(lang, "points {n}", &[("n", &points.to_string())]));
            }
            if areas > 0 {
                self.settings.push(trf(lang, "areas {n}", &[("n", &areas.to_string())]));
            }
            if self.split.on {
                self.settings.push(trf(lang, "objects: scale {scale}, texture {weight} %, from {least} cells", &[("scale", &self.split.scale.to_string()), ("weight", &self.split.weight.to_string()), ("least", &self.split.least.to_string())]));
                if self.split.join {
                    self.settings.push(tr(lang, "neighbours merged").to_string());
                }
            }
        }
        let (lat, lon) = self.site.site.centre();
        if self.source.as_deref() == Some(BASEMAP) {
            self.build_basemap(lang, lat, lon);
            return;
        }
        if let Some(name) = self.source.clone() {
            self.build_raster(lang, name, lat, lon);
            return;
        }
        // Векторы того же участка, года и пикселя уже есть: только классы.
        if let Some((key, _, _)) = &self.vectors
            && *key == self.key(self.ae_level().unwrap_or(key.2))
        {
            self.classify(lang);
            return;
        }
        self.job = Some(Job { zone: aef::zone_of(lon, lat), bbox: self.site.site.bbox(), year: self.year, level: self.ae_level(), index: None, parts: Vec::new(), next: 0, read: None, pieces: Vec::new() });
    }

    /// Уровень AlphaEarth выбранного пикселя, None - «Авто». Пиксель мельче
    /// 10 м у AlphaEarth - «Авто», как `fill_pixels` модуля.
    fn ae_level(&self) -> Option<usize> {
        self.level.and_then(|l| l.checked_sub(FINE))
    }

    /// Строка под полем «Пиксель» у растра и подложки, `pixel_text` модуля:
    /// шаг сетки, у «Авто» - каким он выйдет, и количество клеток участка.
    /// None - у AlphaEarth, при негодном участке и у «Авто», пока пиксель
    /// растра неизвестен.
    fn pixel_text(&self, lang: Lang) -> Option<String> {
        let name = self.source.as_ref()?;
        if self.site.problem(lang).is_some() {
            return None;
        }
        let side = self.site.site.size();
        let base = self.basemap.as_ref().filter(|_| name == BASEMAP);
        let pixel = match self.chosen_pixel() {
            Some(pixel) => pixel,
            None => {
                let native = match base {
                    Some(b) => tilemosaic::tile_pixel(self.site.site.centre().0, b.source.max_level),
                    None => self.native.as_ref().filter(|n| &n.0 == name)?.1,
                };
                rasterclass::auto_pixel(native, side)
            }
        };
        let across = (side / pixel).ceil() as usize;
        let cells = (side / pixel).powi(2);
        let mut text = trf(lang, "Pixel {pixel} m, about {cells} thousand cells.", &[("pixel", &crate::spectral::short(pixel)), ("cells", &format!("{:.0}", cells / 1000.0))]);
        if let Some(b) = base {
            text = format!("{}. {text}", b.name);
        }
        if segments::too_big(across, across) {
            text = format!("{text} {}", tr(lang, "The area will not be divided into objects, a larger pixel or a smaller area is needed."));
        }
        Some(text)
    }

    /// Выбранный пиксель, метры, None - «Авто».
    fn chosen_pixel(&self) -> Option<f64> {
        let level = if self.source.is_some() { self.level } else { self.ae_level().map(|l| l + FINE) };
        level.and_then(|l| PIXELS.get(l).copied())
    }

    /// Классы по каналам открытого растра. Сетка того же участка и пикселя
    /// уже есть - только классы.
    fn build_raster(&mut self, lang: Lang, name: String, lat: f64, lon: f64) {
        let Some(bytes) = self.rasters.iter().find(|r| r.0 == name).map(|r| r.1.clone()) else {
            self.status = tr(lang, "The raster is no longer open.").to_string();
            return;
        };
        let Some(index) = ZoneIndex::bare(&aef::zone_of(lon, lat)) else {
            return;
        };
        let (w, s, e, n) = self.site.site.bbox();
        let key = (name, format!("{w:.6},{s:.6},{e:.6},{n:.6}"), self.level);
        self.job = None;
        // Пиксель задан: сетка известна до чтения файла. У «Авто» её
        // ограничивает сам пиксель.
        if let Some(step) = self.chosen_pixel() {
            let (_, _, cols, rows) = aef::grid_frame(&index, self.site.site.bbox(), step);
            if rasterclass::too_big(rows, cols, 1) {
                self.own_problem(lang, Problem::Big);
                return;
            }
        }
        if let Some((_, index, grid)) = self.own_grid.clone().filter(|g| g.0 == key) {
            self.start_classes(lang, key, index, grid);
            return;
        }
        let (bbox, side) = (self.site.site.bbox(), self.site.site.size());
        self.own_job = Some(OwnJob { key, index: Arc::new(index), bbox, side, stage: OwnStage::Read(bytes) });
        self.status = tr(lang, "Reading the raster of the area…").to_string();
    }

    /// Классы по подложке глобуса, `read_basemap` модуля: уровень тайлов -
    /// по шагу сетки, «Авто» - пиксель верхнего уровня подложки, если
    /// клеток на участке не больше 640 тысяч. Перед первым расчётом по
    /// подложке - окно её условий.
    fn build_basemap(&mut self, lang: Lang, lat: f64, lon: f64) {
        let Some(base) = self.basemap.clone() else {
            self.status = tr(lang, "The basemap tiles were not received.").to_string();
            return;
        };
        if !self.accepted.iter().any(|u| u == base.source.url) {
            self.terms = true;
            return;
        }
        let Some(index) = ZoneIndex::bare(&aef::zone_of(lon, lat)) else {
            return;
        };
        let (bbox, side) = (self.site.site.bbox(), self.site.site.size());
        let pixel = self.chosen_pixel().unwrap_or_else(|| rasterclass::auto_pixel(tilemosaic::tile_pixel(lat, base.source.max_level), side));
        let (w, s, e, n) = bbox;
        let key = (format!("{BASEMAP}:{}", base.source.url), format!("{w:.6},{s:.6},{e:.6},{n:.6}"), self.level);
        self.job = None;
        self.credit = base.credit.clone();
        if let Some((_, index, grid)) = self.own_grid.clone().filter(|g| g.0 == key) {
            self.start_classes(lang, key, index, grid);
            return;
        }
        let (_, _, cols, rows) = aef::grid_frame(&index, bbox, pixel);
        let frame = tilemosaic::span(bbox, tilemosaic::level_for(lat, pixel, base.source.max_level));
        if rasterclass::too_big(rows, cols, 3) || frame.count() > tilemosaic::MAX_TILES {
            self.own_problem(lang, Problem::Big);
            return;
        }
        let warp = TileWarp::new(index.clone(), bbox, pixel);
        self.own_job = Some(OwnJob { key, index: Arc::new(index), bbox, side, stage: OwnStage::Tiles(Box::new((BaseRead::new(base.source, frame), warp, false))) });
        self.status = tr(lang, "Loading the basemap tiles…").to_string();
    }

    /// Название источника по ключу сетки: у подложки - её название.
    fn source_title<'a>(&'a self, key: &'a str) -> &'a str {
        if key.starts_with(BASEMAP) { self.basemap.as_ref().map_or(key, |b| b.name.as_str()) } else { key }
    }

    /// Точки на сетке растра и начало расчёта классов. Отказы - как
    /// `_build_raster` модуля: вне охвата, нет точек на клетках с данными,
    /// каналы без разброса.
    fn start_classes(&mut self, lang: Lang, key: (String, String, Option<usize>), index: Arc<ZoneIndex>, grid: Arc<Mosaic<f32>>) {
        let keep = ring_mask(&index, &grid, &self.site.site.ring);
        // Сетка с каналами других снимков: доля участка - у источника.
        let share = self.cover.first().copied().unwrap_or_else(|| rasterclass::covered(&grid.values, grid.depth, &keep));
        if share <= 0.0 {
            self.own_problem(lang, Problem::Outside);
            return;
        }
        if !self.extras.is_empty() {
            // Каналы второго снимка или снимков сезонов - на ту же сетку.
            let queue: Vec<(String, Arc<Vec<u8>>)> = std::mem::take(&mut self.extras).into_iter().filter_map(|n| self.rasters.iter().find(|r| r.0 == n).cloned()).collect();
            let (bbox, side) = (self.site.site.bbox(), self.site.site.size());
            self.status = tr(lang, "Reading the bands of the other images…").to_string();
            self.own_job = Some(OwnJob { key, index, bbox, side, stage: OwnStage::More { grid, queue, current: None, parts: Vec::new(), shares: vec![share] } });
            return;
        }
        if self.cover.is_empty() {
            self.cover = vec![share];
        }
        if let Some(text) = self.trace.as_ref().and_then(|t| t.task.as_ref()).and_then(|t| t.short_cover(lang, &self.cover)) {
            self.own_job = None;
            if self.layer.take().is_some() {
                self.changed = true;
            }
            self.status = text;
            return;
        }
        if let Some(trace) = &self.trace {
            // Трассировка - по клеткам участка со всеми каналами, сетка не
            // больше, чем у деления на объекты, как у модуля. У заросших
            // полей на цвета идут только каналы летнего снимка.
            if segments::too_big(grid.rows, grid.cols) {
                self.own_problem(lang, Problem::Pieces);
                return;
            }
            let depth = grid.depth.max(1);
            let split = trace.task.as_ref().and_then(|t| t.split()).filter(|s| *s < depth);
            let traced: Vec<f32> = match split {
                Some(s) => grid.values.chunks(depth).flat_map(|cell| cell[..s].iter().copied()).collect(),
                None => grid.values.clone(),
            };
            let used = split.unwrap_or(depth);
            let good: Vec<bool> = traced.chunks(used).zip(&keep).map(|(cell, k)| *k && rasterclass::usable(cell)).collect();
            let tracer = Box::new(Tracer::new(&traced, used, grid.rows, grid.cols, &good, trace.params));
            let (bbox, side) = (self.site.site.bbox(), self.site.site.size());
            self.status = tr(lang, "Splitting the image into colours…").to_string();
            self.own_job = Some(OwnJob { key, index, bbox, side, stage: OwnStage::Trace { grid, tracer, share } });
            return;
        }
        let rows: Vec<&Class> = self.classes.iter().filter(|c| c.has_samples()).collect();
        let points: Vec<(i32, (f64, f64))> = rows.iter().enumerate().flat_map(|(n, c)| c.points.iter().map(move |p| (n as i32, *p))).collect();
        if self.split.on {
            self.start_cut(lang, Ready { key, index, grid, share }, keep, points);
            return;
        }
        let (mut samples, mut labels, mut outside) = rasterclass::samples(&grid, &index, &points);
        // Образцы контуров - клетки внутри, не больше RING_CELLS поровну,
        // `_build_raster(rings=)` модуля 1.38.
        for (label, ring) in self.rings() {
            let got = rasterclass::ring_samples(&grid, &ring_mask(&index, &grid, &ring));
            if got.is_empty() {
                outside += 1;
                continue;
            }
            labels.extend(std::iter::repeat_n(label, got.len()));
            samples.extend(got);
        }
        let mut seen = labels.clone();
        seen.sort_unstable();
        seen.dedup();
        if seen.len() < aef::MIN_CLASSES {
            self.own_problem(lang, Problem::Points);
            return;
        }
        let run = Box::new(Run::new(&grid.values, grid.depth, &samples, &labels, Some(&keep), rasterclass::KNN));
        let (bbox, side) = (self.site.site.bbox(), self.site.site.size());
        self.status = tr(lang, "Computing the classes…").to_string();
        self.own_job = Some(OwnJob { key, index, bbox, side, stage: OwnStage::Classes { grid, run, outside, share } });
    }

    /// Ключ готовых объектов: участок и настройки деления.
    fn split_key(&self, key: &(String, String, Option<usize>)) -> String {
        format!("{key:?}{:?}{:?}", self.site.site.ring, (self.split.scale, self.split.weight, self.split.least))
    }

    /// Деление участка на объекты, `_objects` модуля. Готовые объекты того
    /// же участка и настроек не считаются заново.
    fn start_cut(&mut self, lang: Lang, ready: Ready, keep: Vec<bool>, points: Vec<(i32, (f64, f64))>) {
        if segments::too_big(ready.grid.rows, ready.grid.cols) {
            self.own_problem(lang, Problem::Pieces);
            return;
        }
        let (bbox, side) = (self.site.site.bbox(), self.site.site.size());
        let split_key = self.split_key(&ready.key);
        if let Some((_, parts)) = self.pieces.clone().filter(|p| p.0 == split_key) {
            self.start_kinds(lang, ready, parts, &points);
            return;
        }
        let Ready { key, index, grid, share } = ready;
        let cells = keep.iter().zip(grid.values.chunks(grid.depth.max(1))).filter(|(k, c)| **k && c.iter().all(|v| v.is_finite())).count();
        let params = segments::Params { scale: segments::site_scale(f64::from(self.split.scale), cells), weight: f64::from(self.split.weight) / 100.0, least: self.split.least, shape: segments::SHAPE };
        let cut = Cut::new(&grid.values, grid.depth, grid.rows, grid.cols, Some(&keep), params);
        self.status = tr(lang, "Dividing the area into objects…").to_string();
        let start = cut.edges();
        self.own_job = Some(OwnJob { key, index, bbox, side, stage: OwnStage::Cut { grid, cut: Box::new(cut), start, share } });
    }

    /// Объекты готовы: контрольная точка берёт признаки своего объекта.
    fn start_kinds(&mut self, lang: Lang, ready: Ready, parts: Arc<Vec<i32>>, points: &[(i32, (f64, f64))]) {
        let Ready { key, index, grid, share } = ready;
        if segments::count(&parts) < 2 {
            self.own_problem(lang, Problem::Whole);
            return;
        }
        if self.find.is_some() {
            self.start_scores(lang, self.source_title(&key.0).to_string(), index, grid, parts);
            return;
        }
        let depth = grid.depth.max(1);
        let plain = segments::table(&grid.values, depth, &parts);
        // У снимка к признакам добавляется рисунок объекта, у эмбеддингов
        // AlphaEarth его нет, `_objects(drawing=True)` модуля.
        let (table, width) = self.signs(&grid, &parts, &plain);
        let (mut samples, mut labels, mut outside) = (Vec::new(), Vec::new(), 0);
        for &(label, (lat, lon)) in points {
            let (x, y) = index.project(lat, lon);
            let object = grid.cell_of(x, y).and_then(|(r, c)| usize::try_from(*parts.get(r * grid.cols + c)?).ok());
            match object.and_then(|o| table.get(o * width..(o + 1) * width)) {
                Some(row) => {
                    samples.push(row.to_vec());
                    labels.push(label);
                }
                None => outside += 1,
            }
        }
        let mut seen = labels.clone();
        seen.sort_unstable();
        seen.dedup();
        if seen.len() < aef::MIN_CLASSES {
            self.own_problem(lang, Problem::Points);
            return;
        }
        let run = Box::new(Run::new(&table, width, &samples, &labels, None, rasterclass::KNN));
        let (bbox, side) = (self.site.site.bbox(), self.site.site.size());
        self.status = tr(lang, "Computing the classes…").to_string();
        self.own_job = Some(OwnJob { key, index, bbox, side, stage: OwnStage::Kinds { grid, parts, table, plain, run, outside, share } });
    }

    /// Признаки объектов и их ширина: у растра и подложки за средними
    /// и разбросами каналов plain идёт рисунок объекта
    /// ([`segments::pattern`]), у AlphaEarth - только plain.
    fn signs(&self, grid: &Mosaic<f32>, parts: &[i32], plain: &[f32]) -> (Vec<f32>, usize) {
        let depth = grid.depth.max(1);
        if self.source.is_none() {
            return (plain.to_vec(), 2 * depth);
        }
        let drawing = segments::pattern(&grid.values, depth, parts, grid.rows, grid.cols);
        (segments::with_pattern(plain, 2 * depth, &drawing), 2 * depth + segments::PATTERN)
    }

    /// Похожесть объектов на образцы, `work` окна модуля.
    fn start_scores(&mut self, lang: Lang, label: String, index: Arc<ZoneIndex>, grid: Arc<Mosaic<f32>>, parts: Arc<Vec<i32>>) {
        let depth = grid.depth.max(1);
        let (table, width) = self.signs(&grid, &parts, &segments::table(&grid.values, depth, &parts));
        let rings = self.find.as_ref().map(|f| f.rings.clone()).unwrap_or_default();
        let rows: Vec<Vec<Vec<f32>>> = rings.iter().map(|ring| similar::sample_rows(&table, width, &parts, &ring_mask(&index, &grid, ring))).collect();
        let lost = rows.iter().filter(|r| r.is_empty()).count();
        if lost == rows.len() {
            self.own_problem(lang, Problem::Samples);
            return;
        }
        let samples = rows.concat();
        let weight = f64::from(self.split.weight) / 100.0;
        let Some(score) = similar::scores(&table, width, &segments::areas(&parts), &samples, weight, depth) else {
            self.own_problem(lang, Problem::Flat);
            return;
        };
        let cells = parts.iter().filter(|p| **p >= 0).count();
        self.label = label;
        if let Some(find) = &mut self.find {
            find.base = Some(Base { grid, index, parts, score, lost, cells });
        }
        self.apply(lang);
    }

    /// Выборка по порогу и слиянию окна без расчёта, `apply` модуля.
    fn apply(&mut self, lang: Lang) {
        let Some(find) = &self.find else {
            return;
        };
        let Some(base) = &find.base else {
            return;
        };
        let grid = &base.grid;
        let (shown, mean) = similar::pick(&base.parts, grid.rows, grid.cols, &base.score, f64::from(find.limit) / 100.0, find.join);
        let found = shown.iter().filter(|p| **p >= 0).count();
        let hectares = found as f64 * grid.step * grid.step / 1e4;
        let area = if hectares >= 100.0 { format!("{hectares:.0}") } else { format!("{hectares:.3}") };
        let share = format!("{:.0}", 100.0 * found as f64 / base.cells.max(1) as f64);
        let mut text = trf(lang, "Found objects {n}, {area} ha, {share} % of the area.", &[("n", &mean.len().to_string()), ("area", &area), ("share", &share)]);
        if base.lost > 0 {
            text = format!("{text} {}", trf(lang, "Outlines outside the objects of the area: {n}.", &[("n", &base.lost.to_string())]));
        }
        self.status = format!("{text} {}", analysis::built_text(lang, crate::pointcalc::clock_now()));
        let layer = (!mean.is_empty()).then(|| {
            let classes = shown.iter().map(|p| if *p >= 0 { 0 } else { -1 }).collect();
            let bands = if self.source.is_some() { grid.depth.max(1) } else { 0 };
            let objects = Objects { kinds: vec![0; mean.len()], parts: Arc::new(shown), table: Vec::new(), bands, similar: Some(mean) };
            Arc::new(Layer { index: base.index.clone(), step: grid.step, x0: grid.x0, y0: grid.y0, rows: grid.rows, cols: grid.cols, classes, colors: vec![FOUND_COLOR], names: Vec::new(), opacity: self.opacity, objects: Some(Arc::new(objects)) })
        });
        self.layer = layer;
        self.changed = true;
    }

    /// Точки классов для расчёта: номер класса и место.
    fn points(&self) -> Vec<(i32, (f64, f64))> {
        self.classes.iter().filter(|c| c.has_samples()).enumerate().flat_map(|(n, c)| c.points.iter().map(move |p| (n as i32, *p))).collect()
    }

    /// Классы объектов готовы, `_built` модуля с объектами.
    fn objects_built(&mut self, lang: Lang) {
        let Some(OwnJob { key, index, stage: OwnStage::Kinds { grid, parts, plain: table, run, outside, share, .. }, .. }) = self.own_job.take() else {
            return;
        };
        let (kinds, _) = run.finish();
        if !kinds.iter().any(|k| *k >= 0) {
            self.status = problem_text(lang, Problem::Flat).to_string();
            return;
        }
        let classes: Vec<i16> = parts.iter().map(|p| usize::try_from(*p).ok().and_then(|o| kinds.get(o).copied()).unwrap_or(-1)).collect();
        let depth = grid.depth.max(1);
        let (shown, kinds, table) = if self.split.join {
            let (joined, joined_kinds) = segments::dissolve(&parts, grid.rows, grid.cols, &kinds);
            let joined_table = segments::table(&grid.values, depth, &joined);
            (Arc::new(joined), joined_kinds, joined_table)
        } else {
            (parts, kinds, table)
        };
        let rows: Vec<&Class> = self.classes.iter().filter(|c| c.has_samples()).collect();
        let parts_text = class_shares(&rows, &classes);
        let mut text = if self.source.is_some() {
            trf(lang, "Classes of the raster «{name}»: {parts}. Pixel {pixel} m.", &[("name", self.source_title(&key.0)), ("parts", &parts_text), ("pixel", &format!("{}", (grid.step * 100.0).round() / 100.0))])
        } else {
            trf(lang, "Classes of the year {year}: {parts}.", &[("year", &key.0), ("parts", &parts_text)])
        };
        if share < rasterclass::COVERED {
            text = format!("{text} {}", trf(lang, "The raster covers {share} % of the area.", &[("share", &format!("{:.0}", 100.0 * share))]));
        }
        let sizes = segments::areas(&shown);
        let mean = sizes.iter().sum::<u64>() as f64 / sizes.len().max(1) as f64 * grid.step * grid.step / 1e4;
        let area = if mean >= 10.0 { format!("{mean:.0}") } else { format!("{mean:.2}") };
        text = format!("{text} {}", trf(lang, "Objects {n}, mean {area} ha.", &[("n", &sizes.len().to_string()), ("area", &area)]));
        if outside > 0 && self.source.is_some() {
            text = format!("{text} {}", trf(lang, "Points without raster data: {n}.", &[("n", &outside.to_string())]));
        }
        self.status = format!("{text} {}", analysis::built_text(lang, crate::pointcalc::clock_now()));
        let bands = if self.source.is_some() { depth } else { 0 };
        let objects = Arc::new(Objects { parts: shown, kinds, table, bands, similar: None });
        self.layer = Some(Arc::new(Layer { index, step: grid.step, x0: grid.x0, y0: grid.y0, rows: grid.rows, cols: grid.cols, classes, colors: rows.iter().map(|c| c.color).collect(), names: rows.iter().map(|c| c.name.clone()).collect(), opacity: self.opacity, objects: Some(objects) }));
        self.label = self.source_title(&key.0).to_string();
        self.changed = true;
    }

    /// Отказ снимает прежние классы: «OK» сохранил бы их с новым участком.
    fn own_problem(&mut self, lang: Lang, problem: Problem) {
        self.own_job = None;
        if self.layer.take().is_some() {
            self.changed = true;
        }
        self.status = problem_text(lang, problem).to_string();
    }

    /// Шаг расчёта по растру.
    fn step_own(&mut self, lang: Lang, request: Request) {
        let started = web_time::Instant::now();
        let Some(job) = &mut self.own_job else {
            return;
        };
        let index = job.index.clone();
        match &mut job.stage {
            OwnStage::Tiles(parts) => {
                let (read, warp, got) = &mut **parts;
                if !*got {
                    *got = read.step(request);
                    if !*got {
                        self.status = trf(lang, "Loading the basemap tiles: {n} of {total}.", &[("n", &read.found().to_string()), ("total", &read.total.to_string())]);
                        return;
                    }
                    if read.found() == 0 {
                        self.own_problem(lang, Problem::Tiles);
                        return;
                    }
                }
                let mut done = false;
                while !done && started.elapsed().as_secs_f64() < BUDGET {
                    done = warp.rows(&read.tiles, WARP_ROWS);
                }
                if !done {
                    return;
                }
                let Some(OwnJob { key, index, stage: OwnStage::Tiles(parts), .. }) = self.own_job.take() else {
                    return;
                };
                let grid = Arc::new(parts.1.grid);
                self.own_grid = Some((key.clone(), index.clone(), grid.clone()));
                self.start_classes(lang, key, index, grid);
            }
            OwnStage::Read(bytes) => {
                let (centre, side, bbox, level) = (self.site.site.centre(), job.side, job.bbox, job.key.2);
                let mut native = None;
                let opened = Source::read(bytes, &job.key.0).and_then(|source| {
                    let own = source.pixel(&index, centre.0, centre.1);
                    native = Some(own);
                    let pixel = match level.and_then(|l| PIXELS.get(l)) {
                        Some(pixel) => *pixel,
                        None => rasterclass::auto_pixel(own, side),
                    };
                    let warp = source.warp(&index, bbox, pixel)?;
                    Ok((source, warp))
                });
                let name = job.key.0.clone();
                match opened {
                    Ok(pair) => job.stage = OwnStage::Warp(Box::new(pair)),
                    Err(problem) => self.own_problem(lang, problem),
                }
                if let Some(native) = native {
                    self.native = Some((name, native));
                }
            }
            OwnStage::Warp(pair) => {
                let (source, warp) = &mut **pair;
                let mut done = false;
                while !done && started.elapsed().as_secs_f64() < BUDGET {
                    done = source.warp_rows(warp, WARP_ROWS);
                }
                if !done {
                    return;
                }
                let Some(OwnJob { key, index, stage: OwnStage::Warp(pair), .. }) = self.own_job.take() else {
                    return;
                };
                let grid = Arc::new(pair.1.grid);
                self.own_grid = Some((key.clone(), index.clone(), grid.clone()));
                self.start_classes(lang, key, index, grid);
            }
            OwnStage::Classes { grid, run, .. } => {
                let mut done = false;
                while !done && started.elapsed().as_secs_f64() < BUDGET {
                    done = run.step(&grid.values, RUN_CELLS);
                }
                if done {
                    self.own_built(lang);
                }
            }
            OwnStage::Cut { cut, start, .. } => {
                let mut done = false;
                while !done && started.elapsed().as_secs_f64() < BUDGET {
                    done = cut.step();
                }
                if !done {
                    let left = 100.0 * (1.0 - cut.edges() as f64 / (*start).max(1) as f64);
                    self.status = trf(lang, "Dividing the area into objects: {done} %.", &[("done", &format!("{left:.0}"))]);
                    return;
                }
                let Some(OwnJob { key, index, stage: OwnStage::Cut { grid, cut, share, .. }, .. }) = self.own_job.take() else {
                    return;
                };
                let parts = Arc::new(cut.finish());
                self.pieces = Some((self.split_key(&key), parts.clone()));
                let points = self.points();
                self.start_kinds(lang, Ready { key, index, grid, share }, parts, &points);
            }
            OwnStage::Kinds { table, run, .. } => {
                let mut done = false;
                while !done && started.elapsed().as_secs_f64() < BUDGET {
                    done = run.step(table, RUN_CELLS);
                }
                if done {
                    self.objects_built(lang);
                }
            }
            OwnStage::Trace { tracer, .. } => {
                let mut done = false;
                while !done && started.elapsed().as_secs_f64() < BUDGET {
                    done = tracer.step();
                }
                if done {
                    self.traced_built(lang);
                }
            }
            OwnStage::More { grid, queue, current, parts, shares } => {
                let bbox = job.bbox;
                if current.is_none() {
                    if queue.is_empty() {
                        self.merge_more(lang);
                        return;
                    }
                    let (name, bytes) = queue.remove(0);
                    let fallow = self.trace.as_ref().and_then(|t| t.task.as_ref()).is_some_and(|t| t.split().is_some());
                    let snow = self.recognize.is_some();
                    match Source::read(&bytes, &name).and_then(|s| s.warp(&index, bbox, grid.step).map(|w| (s, w))) {
                        Ok(pair) if (pair.1.grid.rows, pair.1.grid.cols) == (grid.rows, grid.cols) => *current = Some(Box::new(pair)),
                        // Снимок со снегом «Распознавания» не лёг - классы
                        // по летнему, `more_bands` модуля 1.38.
                        _ if snow => {}
                        // Снимок сезона, который не лёг на сетку, - пустые
                        // каналы, расчёт обходится без него.
                        _ if fallow => {
                            let bands = rasterclass::feature_bands(&bytes).unwrap_or(1).max(1);
                            parts.push((vec![f32::NAN; grid.rows * grid.cols * bands], bands));
                            shares.push(0.0);
                        }
                        _ => self.own_problem_text(tr(lang, "The second image is not read or does not cover the area.")),
                    }
                    return;
                }
                let Some(pair) = current else {
                    return;
                };
                let (source, warp) = &mut **pair;
                let mut done = false;
                while !done && started.elapsed().as_secs_f64() < BUDGET {
                    done = source.warp_rows(warp, WARP_ROWS);
                }
                if !done {
                    self.status = trf(lang, "Reading the bands of the other images: {done} %.", &[("done", &format!("{:.0}", 100.0 * warp.done() as f64 / warp.grid.rows.max(1) as f64))]);
                    return;
                }
                let warped = &warp.grid;
                let keep = ring_mask(&index, warped, &self.site.site.ring);
                let share = rasterclass::covered(&warped.values, warped.depth, &keep);
                let fallow = self.trace.as_ref().and_then(|t| t.task.as_ref()).is_some_and(|t| t.split().is_some());
                let empty = !warped.values.iter().any(|v| v.is_finite());
                if empty && self.recognize.is_some() {
                    *current = None;
                    return;
                }
                if !fallow && empty {
                    self.own_problem_text(tr(lang, "The second image is not read or does not cover the area."));
                    return;
                }
                parts.push((warped.values.clone(), warped.depth.max(1)));
                shares.push(share);
                *current = None;
            }
        }
    }

    /// Отказ расчёта текстом.
    fn own_problem_text(&mut self, text: &str) {
        self.own_job = None;
        if self.layer.take().is_some() {
            self.changed = true;
        }
        self.status = text.to_string();
    }

    /// Каналы других снимков за каналами источника, клетка за клеткой.
    fn merge_more(&mut self, lang: Lang) {
        let Some(OwnJob { key, index, stage: OwnStage::More { grid, parts, shares, .. }, .. }) = self.own_job.take() else {
            return;
        };
        let depth = grid.depth.max(1);
        let cells = grid.rows * grid.cols;
        let total = depth + parts.iter().map(|p| p.1).sum::<usize>();
        let mut values = Vec::with_capacity(cells * total);
        for cell in 0..cells {
            values.extend_from_slice(grid.values.get(cell * depth..(cell + 1) * depth).unwrap_or(&[]));
            for (part, bands) in &parts {
                match part.get(cell * bands..(cell + 1) * bands) {
                    Some(v) => values.extend_from_slice(v),
                    None => values.extend(std::iter::repeat_n(f32::NAN, *bands)),
                }
            }
        }
        let merged = Mosaic { x0: grid.x0, y0: grid.y0, step: grid.step, rows: grid.rows, cols: grid.cols, depth: total, values };
        self.cover = shares;
        self.start_classes(lang, key, index, Arc::new(merged));
    }

    /// Трассировка готова, `_built` окна модуля: на глобусе - цвета клеток
    /// после слияния мелких областей.
    fn traced_built(&mut self, lang: Lang) {
        let Some(OwnJob { key, index, stage: OwnStage::Trace { grid, tracer, share }, .. }) = self.own_job.take() else {
            return;
        };
        let Some(done) = tracer.finish() else {
            return;
        };
        let colors = tracing::display_colors(&done.centers);
        let hexes: Vec<String> = colors.iter().map(|c| hex(*c)).collect();
        let count = colors.len();
        let shares = color_shares(&done.classes, count);
        let mut order: Vec<usize> = (0..count).collect();
        order.sort_by(|a, b| shares[*b].total_cmp(&shares[*a]));
        // Название до переименования - номер цвета по доле участка с 1.
        let mut names = vec![String::new(); count];
        for (rank, n) in order.iter().enumerate() {
            names[*n] = trf(lang, "Colour {n}", &[("n", &(rank + 1).to_string())]);
        }
        let regions = done.shapes.len();
        let label = self.source_title(&key.0).to_string();
        let Some(trace) = &mut self.trace else {
            return;
        };
        let p = trace.params;
        let settings = vec![
            trf(lang, "colours {n}", &[("n", &p.colors.to_string())]),
            trf(lang, "smoothing {n}", &[("n", &p.smooth.to_string())]),
            trf(lang, "areas from {n} cells", &[("n", &p.least.to_string())]),
            trf(lang, "rounding {v}", &[("v", &format!("{:.2}", p.corner))]),
            trf(lang, "pixel {pixel} m", &[("pixel", &crate::spectral::short(grid.step))]),
        ];
        let mut base = TraceBase { done, colors, hexes, names, settings, index, x0: grid.x0, y0: grid.y0, step: grid.step, rows: grid.rows, cols: grid.cols };
        let target = match &mut trace.task {
            Some(task) => {
                let title = task.title(lang);
                task.prepare(lang, &mut base, &grid, title)
            }
            None => vec![-1; count],
        };
        trace.rows = order.iter().map(|n| ColorRow { number: *n, name: base.names[*n].clone(), share: shares[*n], target: target.get(*n).copied().unwrap_or(-1) }).collect();
        trace.base = Some(base);
        let mut text = trf(lang, "Colours {colors}, areas {regions}.", &[("colors", &count.to_string()), ("regions", &regions.to_string())]);
        if share < rasterclass::COVERED {
            text = format!("{text} {}", trf(lang, "The raster covers {share} % of the area.", &[("share", &format!("{:.0}", 100.0 * share))]));
        }
        if regions > MOST_OBJECTS {
            text = format!("{text} {}", trf(lang, "There are more than {n} areas, «OK» saves a picture of the colours instead of polygons.", &[("n", &MOST_OBJECTS.to_string())]));
        }
        text = format!("{text} {}", analysis::built_text(lang, crate::pointcalc::clock_now()));
        if let Some(notes) = trace.task.as_ref().map(|t| t.notes()).filter(|n| !n.is_empty()) {
            text = format!("{text} {notes}");
        }
        self.status = text;
        self.label = label;
        self.apply_trace(lang);
    }

    /// Итог по таблице классов, `_apply_classes` модуля 1.33: слитые цвета -
    /// один класс с цветом и названием того, с которым слиты. У задачи -
    /// ещё рубки, виды полей и линии на глобусе.
    fn apply_trace(&mut self, lang: Lang) {
        let Some(trace) = &mut self.trace else {
            return;
        };
        let Some(base) = &trace.base else {
            return;
        };
        let count = base.colors.len();
        let mut target = vec![-1i32; count];
        for row in &trace.rows {
            if let Some(t) = target.get_mut(row.number) {
                *t = row.target;
            }
        }
        let (lookup, keep) = tracing::join_colors(&target);
        let (classes, kinds) = tracing::joined(&base.done.classes, &base.done.kinds, &lookup);
        let name_of = |k: usize| trace.rows.iter().find(|r| r.number == k).map(|r| r.name.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| base.names.get(k).cloned().unwrap_or_default());
        let mut joined = Joined {
            classes,
            kinds,
            colors: keep.iter().map(|k| base.colors[*k]).collect(),
            names: keep.iter().map(|k| name_of(*k)).collect(),
            hexes: keep.iter().map(|k| base.hexes[*k].clone()).collect(),
            fields: Vec::new(),
        };
        let mut shown = joined.classes.clone();
        let mut extra = Vec::new();
        if let Some(task) = &trace.task {
            joined = task.classes(lang, base, joined);
            (shown, extra) = task.line_layer(lang, base, &joined.classes, joined.colors.len());
        }
        let mut colors = joined.colors.clone();
        let mut names = joined.names.clone();
        for (c, n) in extra {
            colors.push(c);
            names.push(n);
        }
        let classes = shown.iter().map(|c| i16::try_from(*c).unwrap_or(-1)).collect();
        self.layer = Some(Arc::new(Layer { index: base.index.clone(), step: base.step, x0: base.x0, y0: base.y0, rows: base.rows, cols: base.cols, classes, colors, names, opacity: self.opacity, objects: None }));
        trace.joined = Some(joined);
        self.changed = true;
    }

    /// «Названия от помощника», `ask_names` модуля 1.33: таблица классов -
    /// в модель одним запросом, ответ - названия в таблице. Снимок и имена
    /// в модель не уходят.
    fn ask_names(&mut self, lang: Lang) {
        let source_is_basemap = self.source.as_deref() == Some(BASEMAP);
        let place = self.site.site.centre();
        let Some(trace) = &mut self.trace else {
            return;
        };
        let Some(base) = &trace.base else {
            self.status = tr(lang, "First press «Build».").to_string();
            return;
        };
        let count = base.colors.len();
        let mut cells = vec![0usize; count];
        for c in &base.done.classes {
            if let Some(n) = usize::try_from(*c).ok().filter(|n| *n < count) {
                cells[n] += 1;
            }
        }
        let mut regions = vec![0usize; count];
        for k in &base.done.kinds {
            if let Some(n) = usize::try_from(*k).ok().filter(|n| *n < count) {
                regions[n] += 1;
            }
        }
        let total = cells.iter().sum::<usize>().max(1);
        let highest = base.done.centers.iter().flatten().copied().fold(f64::NEG_INFINITY, f64::max);
        let ru = lang == Lang::RU;
        let units = if source_is_basemap {
            if ru { "яркость 0-255" } else { "brightness 0-255" }
        } else if highest <= data::objecttasks::REFLECTANCE_MAX {
            if ru { "отражение 0-1" } else { "reflectance 0-1" }
        } else if ru {
            "значения растра"
        } else {
            "raster values"
        };
        let rules = trace.task.as_ref().and_then(|t| t.rules());
        let rows: Vec<data::classnames::Row> = (0..count)
            .map(|n| data::classnames::Row {
                means: base.done.centers.get(n).cloned().unwrap_or_default(),
                share: cells[n] as f64 / total as f64,
                regions: regions[n],
                area_ha: cells[n] as f64 * base.step * base.step / 1e4 / regions[n].max(1) as f64,
                rule: rules.and_then(|r| r.get(n)).cloned(),
            })
            .collect();
        let roles: Option<Vec<Option<String>>> = if source_is_basemap {
            Some(vec![Some("red".into()), Some("green".into()), Some("blue".into())])
        } else {
            trace.task.as_ref().map(|t| t.grid_roles()).or_else(|| {
                let raster = self.source.as_ref().and_then(|s| self.rasters.iter().find(|r| r.0 == *s))?;
                crate::objecttasks::raster_roles(raster).0.map(|r| r.iter().map(|x| x.map(str::to_string)).collect())
            })
        };
        let month = trace.task.as_ref().and_then(|t| t.month());
        let text = data::classnames::question(&rows, roles.as_deref(), units, Some(place), month, ru);
        let system = data::classnames::system_text(ru, lang.english_name());
        trace.ask = Some((system, text));
        self.status = tr(lang, "The assistant is thinking…").to_string();
    }

    /// Классы по растру готовы, `_built` модуля.
    fn own_built(&mut self, lang: Lang) {
        let Some(OwnJob { key, index, stage: OwnStage::Classes { grid, run, outside, share }, .. }) = self.own_job.take() else {
            return;
        };
        let (classes, _) = run.finish();
        if !classes.iter().any(|c| *c >= 0) {
            self.status = problem_text(lang, Problem::Flat).to_string();
            return;
        }
        let rows: Vec<&Class> = self.classes.iter().filter(|c| c.has_samples()).collect();
        let parts = class_shares(&rows, &classes);
        let pixel = format!("{}", (grid.step * 100.0).round() / 100.0);
        let mut text = trf(lang, "Classes of the raster «{name}»: {parts}. Pixel {pixel} m.", &[("name", self.source_title(&key.0)), ("parts", &parts), ("pixel", &pixel)]);
        if share < rasterclass::COVERED {
            text = format!("{text} {}", trf(lang, "The raster covers {share} % of the area.", &[("share", &format!("{:.0}", 100.0 * share))]));
        }
        if outside > 0 {
            text = format!("{text} {}", trf(lang, "Points without raster data: {n}.", &[("n", &outside.to_string())]));
        }
        self.status = format!("{text} {}", analysis::built_text(lang, crate::pointcalc::clock_now()));
        self.layer = Some(Arc::new(Layer { index, step: grid.step, x0: grid.x0, y0: grid.y0, rows: grid.rows, cols: grid.cols, classes, colors: rows.iter().map(|c| c.color).collect(), names: rows.iter().map(|c| c.name.clone()).collect(), opacity: self.opacity, objects: None }));
        self.label = self.source_title(&key.0).to_string();
        self.changed = true;
    }

    /// Шаг загрузки векторов.
    /// Шаг задачи: поиск и загрузка снимков, загруженный снимок - растр
    /// программы и источник.
    fn step_task(&mut self, lang: Lang, request: Request) {
        if !self.task_mode() {
            return;
        }
        let problem = self.site.problem(lang);
        let ring = self.site.site.ring.clone();
        let Some(task) = self.trace.as_mut().and_then(|t| t.task.as_mut()) else {
            return;
        };
        task.step(lang, &ring, problem, request);
        if let Some(text) = task.status.take() {
            self.status = text;
        }
        for raster in task.loaded.drain(..) {
            self.rasters.retain(|r| r.0 != raster.0);
            self.rasters.push(raster.clone());
            self.loaded.push(raster);
        }
        if let Some(name) = task.take_source.take() {
            self.source = Some(name);
        }
    }

    pub fn step(&mut self, lang: Lang, request: Request) {
        self.step_task(lang, request);
        self.step_recognize(lang, request);
        self.step_own(lang, request);
        self.indexes.step();
        let Some(job) = &mut self.job else {
            return;
        };
        if job.index.is_none() {
            match self.indexes.get(&job.zone, request) {
                None => {
                    self.status = tr(lang, "Loading the index of files…").to_string();
                    return;
                }
                Some(None) => {
                    self.status = tr(lang, "There are no files of the data set for this zone.").to_string();
                    self.job = None;
                    return;
                }
                Some(Some(index)) => {
                    let level = job.level.unwrap_or_else(|| aef::auto_level(&index, job.bbox, self.site.site.size()));
                    job.level = Some(level);
                    job.parts = aef::detail_parts(&index, job.bbox, level);
                    job.index = Some(index);
                }
            }
        }
        let (Some(index), Some(level)) = (job.index.clone(), job.level) else {
            return;
        };
        let started = web_time::Instant::now();
        while started.elapsed().as_secs_f64() < BUDGET {
            if job.read.is_none() {
                let Some(&(cell, col, row)) = job.parts.get(job.next) else {
                    break;
                };
                match index.name(cell, job.year).and_then(|n| aef::file_url(job.year, &index.zone, n)) {
                    Some(url) => job.read = Some((BlockRead::new(&url, level, col, row, request), Vec::new(), 0)),
                    None => job.next += 1,
                }
                continue;
            }
            let Some((read, values, band)) = &mut job.read else {
                break;
            };
            if read.block().is_none() {
                read.step(request);
                if let Some(e) = read.error() {
                    self.status = trf(lang, "The files are not read: {e}", &[("e", e)]);
                    self.job = None;
                    return;
                }
                if read.block().is_none() {
                    break;
                }
            }
            let Some(block) = read.block() else {
                break;
            };
            let depth = usize::from(aef::BANDS);
            if values.is_empty() {
                *values = vec![aef::NODATA; block.pixels() * depth];
            }
            if *band < block.samples().min(aef::BANDS) {
                if let Some(plane) = block.band(*band) {
                    for (i, v) in plane.into_iter().enumerate() {
                        if let Some(slot) = values.get_mut(i * depth + usize::from(*band)) {
                            *slot = v;
                        }
                    }
                }
                *band += 1;
                continue;
            }
            if let Some(piece) = block.piece(std::mem::take(values)) {
                job.pieces.push(piece);
            }
            job.read = None;
            job.next += 1;
        }
        if job.next < job.parts.len() {
            self.status = trf(lang, "Loading the vectors of the area: block {n} of {total}.", &[("n", &(job.next + 1).to_string()), ("total", &job.parts.len().to_string())]);
            return;
        }
        let Some(job) = self.job.take() else {
            return;
        };
        let Some(mosaic) = aef::mosaic(&job.pieces, &index, job.bbox, usize::from(aef::BANDS), aef::NODATA) else {
            self.status = tr(lang, "There are no data of the data set in the area for this year.").to_string();
            return;
        };
        let key = self.key(level);
        self.vectors = Some((key, index, Arc::new(mosaic)));
        self.classify(lang);
    }

    /// Классы участка по векторам и точкам, `_built` модуля.
    fn classify(&mut self, lang: Lang) {
        let Some((_, index, grid)) = &self.vectors else {
            return;
        };
        // Объекты AlphaEarth: векторы переводятся в числа, дальше всё
        // общее с растром.
        if self.split.on {
            let (index, values) = (index.clone(), Arc::new(Mosaic { x0: grid.x0, y0: grid.y0, step: grid.step, rows: grid.rows, cols: grid.cols, depth: grid.depth, values: grid.values.iter().map(|v| aef::dequantize(*v)).collect() }));
            let (w, s, e, n) = self.site.site.bbox();
            let key = (self.year.to_string(), format!("{w:.6},{s:.6},{e:.6},{n:.6}"), self.level);
            self.start_classes(lang, key, index, values);
            return;
        }
        let rows: Vec<&Class> = self.classes.iter().filter(|c| c.has_samples()).collect();
        let (mut samples, mut labels, mut outside) = (Vec::new(), Vec::new(), 0);
        for (label, class) in rows.iter().enumerate() {
            for &(lat, lon) in &class.points {
                let (x, y) = index.project(lat, lon);
                match grid.cell_of(x, y).and_then(|(r, c)| grid.at(r, c)) {
                    Some(v) => {
                        samples.push(v.to_vec());
                        labels.push(label as i32);
                    }
                    None => outside += 1,
                }
            }
        }
        if samples.is_empty() {
            self.status = tr(lang, "All points are outside the area.").to_string();
            return;
        }
        let found = aef::classify(&grid.values, grid.depth, &samples, &labels, aef::KNN);
        let mut classes: Vec<i16> = found.iter().map(|c| c.map_or(-1, |v| v.0 as i16)).collect();
        // Вне многоугольника участка классов нет.
        for (v, keep) in classes.iter_mut().zip(ring_mask(index, grid, &self.site.site.ring)) {
            if !keep {
                *v = -1;
            }
        }
        let parts = class_shares(&rows, &classes);
        let mut text = trf(lang, "Classes of the year {year}: {parts}.", &[("year", &self.year.to_string()), ("parts", &parts)]);
        if outside > 0 {
            text = format!("{text} {}", trf(lang, "Points outside the area: {n}.", &[("n", &outside.to_string())]));
        }
        self.status = format!("{text} {}", analysis::built_text(lang, crate::pointcalc::clock_now()));
        self.label = self.year.to_string();
        self.layer = Some(Arc::new(Layer { index: index.clone(), step: grid.step, x0: grid.x0, y0: grid.y0, rows: grid.rows, cols: grid.cols, classes, colors: rows.iter().map(|c| c.color).collect(), names: rows.iter().map(|c| c.name.clone()).collect(), opacity: self.opacity, objects: None }));
        self.changed = true;
    }

    /// Многоугольники объектов папками классов, `write_objects` модуля.
    /// rows - название и цвет класса, table - признаки объектов. None -
    /// объектов больше [`MOST_OBJECTS`].
    fn object_places(&self, lang: Lang, layer: &Layer, objects: &Objects, rows: &[(String, [u8; 3])], table: &[f32]) -> Option<Vec<Item>> {
        let outlines = segments::outlines(&objects.parts, layer.rows, layer.cols);
        if outlines.len() > MOST_OBJECTS {
            return None;
        }
        let sizes = segments::areas(&objects.parts);
        let depth = objects.bands;
        let to_degrees = |ring: &Vec<[f64; 2]>| -> Vec<(f64, f64)> { data::hydro::simplify(ring, OUTLINE).iter().map(|p| aef::from_utm(layer.index.number, layer.index.south, layer.x0 + p[0] * layer.step, layer.y0 - p[1] * layer.step)).collect() };
        let mut folders: Vec<scene::kml::Folder> = rows.iter().map(|c| scene::kml::Folder::new(&trf(lang, "Objects: {name}", &[("name", &c.0)]))).collect();
        for (n, rings) in outlines.iter().enumerate() {
            let kind = objects.kinds.get(n).copied().unwrap_or(-1);
            let (Some(first), Ok(class)) = (rings.first(), usize::try_from(kind)) else {
                continue;
            };
            let (Some(row), Some(folder)) = (rows.get(class), folders.get_mut(class)) else {
                continue;
            };
            let area = sizes.get(n).copied().unwrap_or(0) as f64 * layer.step * layer.step / 1e4;
            let mut place = Place::new(&format!("{} {}", row.0, n + 1), Kind::Polygon, to_degrees(first));
            let alpha = (self.opacity * 255.0).round() as u8;
            let [r, g, b] = row.1;
            place.color = [r, g, b, 255];
            place.fill = Some([r, g, b, alpha]);
            if rings.len() > 1 {
                place.holes = Some(rings[1..].iter().map(to_degrees).collect());
            }
            let mut fields = vec![("object".to_string(), n.to_string()), ("class".to_string(), (class + 1).to_string()), ("name".to_string(), row.0.clone()), ("area_ha".to_string(), format!("{area:.4}"))];
            if let Some(alike) = objects.similar.as_ref().and_then(|s| s.get(n)) {
                fields.push(("similar".to_string(), format!("{:.1}", 100.0 * alike)));
            }
            for band in 0..depth {
                let value = |k: usize| table.get(n * 2 * depth + k).map_or(String::new(), f32::to_string);
                fields.push((format!("mean_{}", band + 1), value(band)));
                fields.push((format!("sd_{}", band + 1), value(depth + band)));
            }
            place.data = Some(fields);
            folder.children.push(Item::Place(place));
        }
        Some(folders.into_iter().map(Item::Folder).collect())
    }

    /// Классы - расчётом для «Моих меток»: GeoTIFF номеров классов в зоне
    /// UTM, картинка, контрольные точки папками классов.
    fn save(&mut self, lang: Lang) {
        if self.trace.is_some() {
            self.save_trace(lang);
            return;
        }
        let Some(layer) = &self.layer else {
            return;
        };
        let tiff = class_tiff(layer);
        let shown = layer.clone();
        let sample = move |points: &[(f64, f64)]| shown.sample(points);
        let named = if self.find.is_some() {
            "Similar areas {year}"
        } else if self.recognize_mode() {
            "Recognition {year}"
        } else {
            "Classification {year}"
        };
        let mut name = trf(lang, named, &[("year", &self.label)]);
        if self.find.is_none() {
            // Название несёт настройки расчёта, `titled` модуля 1.33.
            let mut settings = self.settings.clone();
            settings.push(trf(lang, "pixel {pixel} m", &[("pixel", &crate::spectral::short(layer.step))]));
            name = titled(&name, &settings);
        }
        let rows: Vec<(String, [u8; 3])> = if self.find.is_some() { vec![(tr(lang, "Similar areas").to_string(), FOUND_COLOR)] } else { self.classes.iter().filter(|c| c.has_samples()).map(|c| (c.name.clone(), c.color)).collect() };
        // С объектами итог - многоугольники, растр классов не пишется.
        // Признаки выборки считаются только здесь: ползунку они не нужны.
        let base = self.find.as_ref().and_then(|f| f.base.as_ref());
        let places = layer.objects.as_ref().and_then(|o| {
            let table = base.map_or_else(|| o.table.clone(), |b| segments::table(&b.grid.values, b.grid.depth.max(1), &o.parts));
            self.object_places(lang, layer, o, &rows, &table)
        });
        let mut extra = Vec::new();
        if let Some(find) = &self.find {
            let mut folder = scene::kml::Folder::new(tr(lang, "Samples"));
            for (n, ring) in find.rings.iter().enumerate() {
                let mut place = Place::new(&format!("{} {}", tr(lang, "Samples"), n + 1), Kind::Polygon, ring.clone());
                let [r, g, b] = SAMPLE_COLOR;
                place.color = [r, g, b, 255];
                place.fill = Some([r, g, b, 0]);
                folder.children.push(Item::Place(place));
            }
            extra.push(Item::Folder(folder));
        }
        // Подпись источника подложки - в описании результата, как права
        // слоя (`set_credit` модуля).
        let credit = if self.source.as_deref() == Some(BASEMAP) { self.credit.clone() } else { String::new() };
        let layers = if let Some(mut places) = places {
            for item in &mut places {
                if let Item::Folder(folder) = item {
                    folder.description.clone_from(&credit);
                }
            }
            extra.extend(places);
            Vec::new()
        } else {
            let mut picture = analysis::picture(name, "aef_class", layer.bbox(), layer.step, &sample, tiff);
            picture.summary.clone_from(&credit);
            vec![picture]
        };
        for (n, class) in self.classes.iter().filter(|c| c.has_samples()).enumerate() {
            let mut folder = scene::kml::Folder::new(&trf(lang, "{n} - {name}", &[("n", &n.to_string()), ("name", &class.name)]));
            folder.description = trf(lang, "Value {n} in the GeoTIFF file.", &[("n", &n.to_string())]);
            for &(lat, lon) in &class.points {
                let mut place = Place::new(&class.name, Kind::Point, vec![(lat, lon)]);
                place.color = [class.color[0], class.color[1], class.color[2], 255];
                folder.children.push(Item::Place(place));
            }
            extra.push(Item::Folder(folder));
        }
        // Контуры образцов многоугольниками с классом и названием,
        // `_export_rings` модуля 1.38.
        let samples: Vec<Item> = self
            .classes
            .iter()
            .filter(|c| c.has_samples())
            .enumerate()
            .flat_map(|(n, c)| {
                c.rings.iter().map(move |ring| {
                    let mut place = Place::new(&c.name, Kind::Polygon, ring.clone());
                    let [r, g, b] = c.color;
                    place.color = [r, g, b, 255];
                    place.fill = Some([r, g, b, 0]);
                    place.data = Some(vec![("class".to_string(), (n + 1).to_string()), ("name".to_string(), c.name.clone())]);
                    Item::Place(place)
                })
            })
            .collect();
        if !samples.is_empty() {
            let mut folder = scene::kml::Folder::new(tr(lang, "Class samples"));
            folder.children = samples;
            extra.push(Item::Folder(folder));
        }
        let title = tr(
            lang,
            if self.find.is_some() {
                "Similar areas"
            } else if self.recognize_mode() {
                "Recognition"
            } else {
                "Classification"
            },
        )
        .to_string();
        self.saved = Some(Saved { title, ring: self.site.site.ring.clone(), layers, extra, session: None });
    }

    /// Трассировка - расчётом для «Моих меток», `export` окна модуля:
    /// многоугольники областей с полями cls, color и area_ha. Областей
    /// больше [`MOST_OBJECTS`] - картинка цветов и GeoTIFF их номеров.
    fn save_trace(&mut self, lang: Lang) {
        let title = self.tool_title(lang);
        let (Some(layer), Some(trace)) = (&self.layer, &self.trace) else {
            return;
        };
        let (Some(base), Some(joined)) = (&trace.base, &trace.joined) else {
            return;
        };
        // Название - источник и настройки, описание - ещё классы с долей
        // участка, `set_abstract` модуля 1.33.
        let name = titled(&format!("{title} {}", self.label), &base.settings);
        let mut about = vec![
            trf(lang, "Source - {source}.", &[("source", &self.label)]),
            trf(lang, "Settings - {settings}.", &[("settings", &base.settings.join(", "))]),
            trf(lang, "Classes - {rows}.", &[("rows", &class_text(&joined.classes, &joined.names))]),
        ];
        if self.source.as_deref() == Some(BASEMAP) && !self.credit.is_empty() {
            about.push(self.credit.clone());
        }
        let about = about.join("\n");
        let (layers, mut extra) = match trace_places(layer, base, joined, &name, self.opacity) {
            Some(mut folder) => {
                folder.description = about;
                (Vec::new(), vec![Item::Folder(folder)])
            }
            None => {
                let shown = layer.clone();
                let sample = move |points: &[(f64, f64)]| shown.sample(points);
                let mut picture = analysis::picture(name, "tracing", layer.bbox(), layer.step, &sample, class_tiff(layer));
                picture.summary = about;
                (vec![picture], Vec::new())
            }
        };
        if let Some(task) = &trace.task {
            let to_degrees = |p: &(f64, f64)| aef::from_utm(base.index.number, base.index.south, base.x0 + p.0 * base.step, base.y0 - p.1 * base.step);
            if let Some(folder) = task.line_places(lang, base, &to_degrees) {
                extra.push(Item::Folder(folder));
            }
        }
        self.saved = Some(Saved { title: title.to_string(), ring: self.site.site.ring.clone(), layers, extra, session: None });
    }

    /// Мышь над глобусом: точки выбранного класса щелчком, щелчок по
    /// точке убирает её. Участок - главнее. true - мышь занята.
    pub fn input(&mut self, ctx: &egui::Context, response: &egui::Response, rect: egui::Rect, ppp: f64, nav: &Navigator) -> bool {
        if !self.open {
            return false;
        }
        if self.site.input(ctx, response, rect, ppp, nav) {
            return true;
        }
        if self.site.site.drawing {
            return false;
        }
        let to_px = |p: egui::Pos2| (f64::from(p.x - rect.min.x) * ppp, f64::from(p.y - rect.min.y) * ppp);
        let pos = ctx.input(|i| i.pointer.interact_pos());
        let on_globe = pos.is_some_and(|p| rect.contains(p) && ctx.layer_id_at(p).is_none_or(|l| l == response.layer_id));
        if !(response.clicked() && on_globe) {
            return false;
        }
        let Some(p) = pos else {
            return false;
        };
        let (px, py) = to_px(p);
        let radius = POINT_PIXELS * ppp;
        // Щелчок обводит образец, по первой вершине - замыкает контур.
        if let Some(find) = &mut self.find {
            if find.ring.len() >= 3
                && let Some(first) = find.ring.first()
                && let (pix, true) = pixel(nav, *first)
                && (pix[0] - px).hypot(pix[1] - py) <= radius
            {
                let ring = std::mem::take(&mut find.ring);
                find.rings.push(ring);
                return true;
            }
            if let Some(ground) = ground_under(&nav.camera, px, py, nav.pose.terrain.as_ref()) {
                let (lat, lon, _) = nav.camera.ellipsoid.ecef_to_geodetic(ground);
                find.ring.push((lat, lon));
            }
            return true;
        }
        // Контур образца «Распознавания» обводится: щелчок ставит вершину,
        // по первой - замыкает контур, `ClassSample` модуля 1.38.
        if let Some(rec) = self.recognize.as_mut().filter(|r| r.areas && !r.drawing.is_empty()) {
            if rec.drawing.len() >= 3
                && let Some(first) = rec.drawing.first()
                && let (pix, true) = pixel(nav, *first)
                && (pix[0] - px).hypot(pix[1] - py) <= radius
            {
                let ring = std::mem::take(&mut rec.drawing);
                if let Some(class) = self.classes.get_mut(rec.drawing_class) {
                    class.rings.push(ring);
                }
                return true;
            }
            if let Some(ground) = ground_under(&nav.camera, px, py, nav.pose.terrain.as_ref()) {
                let (lat, lon, _) = nav.camera.ellipsoid.ecef_to_geodetic(ground);
                rec.drawing.push((lat, lon));
            }
            return true;
        }
        // Щелчок по точке убирает её, `point_under` модуля.
        let mut hit = None;
        for (k, class) in self.classes.iter().enumerate() {
            for (i, pt) in class.points.iter().enumerate() {
                let (pix, visible) = pixel(nav, *pt);
                if visible && (pix[0] - px).hypot(pix[1] - py) <= radius {
                    hit = Some((k, i));
                }
            }
        }
        if let Some((k, i)) = hit {
            if let Some(class) = self.classes.get_mut(k) {
                class.points.remove(i);
            }
            return true;
        }
        let Some(ground) = ground_under(&nav.camera, px, py, nav.pose.terrain.as_ref()) else {
            return false;
        };
        let (lat, lon, _) = nav.camera.ellipsoid.ecef_to_geodetic(ground);
        // Области: щелчок внутри контура убирает его, иначе начинает
        // контур выбранного класса.
        if self.recognize.as_ref().is_some_and(|r| r.areas) {
            if let Some((k, n)) = self.ring_under(lat, lon) {
                if let Some(class) = self.classes.get_mut(k) {
                    class.rings.remove(n);
                }
                return true;
            }
            let active = self.active;
            if let Some(rec) = &mut self.recognize {
                rec.drawing = vec![(lat, lon)];
                rec.drawing_class = active;
            }
            return true;
        }
        if let Some(class) = self.classes.get_mut(self.active) {
            class.points.push((lat, lon));
        }
        true
    }

    /// Контрольные точки цветами классов поверх глобуса.
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect, ppp: f64, nav: &Navigator) {
        if !self.open {
            return;
        }
        self.site.paint(painter, rect, ppp, nav);
        let to_screen = |p: [f64; 2]| rect.min + egui::vec2((p[0] / ppp) as f32, (p[1] / ppp) as f32);
        if let Some(find) = &self.find {
            let [r, g, b] = SAMPLE_COLOR;
            let color = egui::Color32::from_rgb(r, g, b);
            let stroke = egui::Stroke::new(2.0, color);
            let line = |ring: &[(f64, f64)], closed: bool| {
                let pixels: Vec<([f64; 2], bool)> = ring.iter().chain(ring.first().filter(|_| closed)).map(|p| pixel(nav, *p)).collect();
                for w in pixels.windows(2) {
                    if w[0].1 && w[1].1 {
                        painter.line_segment([to_screen(w[0].0), to_screen(w[1].0)], stroke);
                    }
                }
            };
            for ring in &find.rings {
                line(ring, true);
            }
            line(&find.ring, false);
            for (n, point) in find.ring.iter().enumerate() {
                let (p, visible) = pixel(nav, *point);
                if visible {
                    // Первая вершина крупнее: щелчок по ней замыкает контур.
                    painter.circle(to_screen(p), if n == 0 { 6.0 } else { 4.0 }, color, egui::Stroke::new(1.5, egui::Color32::from_gray(30)));
                }
            }
        }
        // Контуры образцов - цветом класса по тёмной подложке линии.
        let outline = |ring: &[(f64, f64)], closed: bool, color: egui::Color32| {
            let pixels: Vec<([f64; 2], bool)> = ring.iter().chain(ring.first().filter(|_| closed)).map(|p| pixel(nav, *p)).collect();
            for w in pixels.windows(2) {
                if w[0].1 && w[1].1 {
                    let segment = [to_screen(w[0].0), to_screen(w[1].0)];
                    painter.line_segment(segment, egui::Stroke::new(4.0, egui::Color32::from_black_alpha(140)));
                    painter.line_segment(segment, egui::Stroke::new(2.0, color));
                }
            }
        };
        for class in &self.classes {
            let color = egui::Color32::from_rgb(class.color[0], class.color[1], class.color[2]);
            for ring in &class.rings {
                outline(ring, true, color);
            }
        }
        if let Some(rec) = self.recognize.as_ref().filter(|r| !r.drawing.is_empty()) {
            let [r, g, b] = self.classes.get(rec.drawing_class).map_or(SAMPLE_COLOR, |c| c.color);
            let color = egui::Color32::from_rgb(r, g, b);
            outline(&rec.drawing, false, color);
            for (n, point) in rec.drawing.iter().enumerate() {
                let (p, visible) = pixel(nav, *point);
                if visible {
                    painter.circle(to_screen(p), if n == 0 { 6.0 } else { 4.0 }, color, egui::Stroke::new(1.5, egui::Color32::from_gray(30)));
                }
            }
        }
        for class in &self.classes {
            let fill = egui::Color32::from_rgb(class.color[0], class.color[1], class.color[2]);
            for pt in &class.points {
                let (p, visible) = pixel(nav, *pt);
                if visible {
                    painter.circle(to_screen(p), 5.0, fill, egui::Stroke::new(1.5, egui::Color32::WHITE));
                }
            }
        }
    }

    /// Окно условий подложки перед первым расчётом по ней,
    /// `accept_basemap_use` модуля: условия разных подложек такое
    /// использование разрешают по-разному.
    fn terms_window(&mut self, ctx: &egui::Context, lang: Lang) {
        if !self.terms {
            return;
        }
        let Some(base) = self.basemap.clone() else {
            self.terms = false;
            return;
        };
        let mut answer = None;
        egui::Window::new(tr(lang, "Source terms")).id(egui::Id::unique("planetx-basemap-terms")).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.add(
                egui::Label::new(trf(
                    lang,
                    "The computation will collect the tiles of the basemap «{name}» under the area into a picture in memory, no image file is written. The terms of the basemap may not allow such use, especially for commercial services. The computation goes by your decision, you bear the responsibility for its result and use.",
                    &[("name", &base.name)],
                ))
                .wrap(),
            );
            ui.horizontal(|ui| {
                if ui.button(tr(lang, "Compute")).clicked() {
                    answer = Some(true);
                }
                if ui.button(tr(lang, "Cancel")).clicked() {
                    answer = Some(false);
                }
            });
        });
        match answer {
            Some(true) => {
                self.terms = false;
                let url = base.source.url.to_string();
                if !self.accepted.contains(&url) {
                    self.accepted.push(url.clone());
                }
                self.accept_new = Some(url);
                self.build(lang);
            }
            Some(false) => {
                self.terms = false;
                self.status = tr(lang, "The computation by the basemap was not started.").to_string();
            }
            None => {}
        }
    }

    /// Окно.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang) {
        if !self.open {
            return;
        }
        if let Some(text) = self.site.shared_note(lang) {
            self.status = text.to_string();
        }
        self.site.site.most_km = if self.recognize.is_some() {
            recognize::SIDE_MOST
        } else if self.source.is_some() {
            RASTER_MOST
        } else {
            MOST
        };
        let mut open = true;
        let mut pressed = None;
        let opacity = self.opacity;
        let mut remove = None;
        let mut add = false;
        let searching = self.find.is_some();
        let tracing_mode = self.trace.is_some();
        let task_mode = self.task_mode();
        let recognize_mode = self.recognize_mode();
        // У задачи тонкие настройки свёрнуты под «Дополнительно», просьба
        // автора модуля «система громоздкая, упрости». У «Распознавания»
        // их нет: «пользователя ни разу не грузим лишним».
        let fine = !recognize_mode && (!task_mode || self.trace.as_ref().and_then(|t| t.task.as_ref()).is_some_and(|t| t.more));
        let fallow = self.trace.as_ref().and_then(|t| t.task.as_ref()).is_some_and(|t| t.scenario == data::objecttasks::Scenario::Fallow);
        let mut pick_now = false;
        let mut table_changed = false;
        let mut ask = false;
        let mut refine = false;
        let mut new_task = None;
        let title = if tracing_mode {
            self.tool_title(lang)
        } else {
            tr(
                lang,
                if searching {
                    "Similar areas"
                } else if recognize_mode {
                    "Recognition"
                } else {
                    "Classification"
                },
            )
        };
        let id = if recognize_mode {
            "planetx-recognize"
        } else if task_mode {
            "planetx-objects"
        } else if tracing_mode {
            "planetx-tracing"
        } else if searching {
            "planetx-similar"
        } else {
            "planetx-aef-class"
        };
        egui::Window::new(title).id(egui::Id::unique(id)).open(&mut open).collapsible(false).resizable(false).min_width(400.0).show(ctx, |ui| {
            egui::Grid::new("planetx-aef-class-form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                if recognize_mode {
                    // Снимки, участок и образцы, `RecognizeDialog` модуля
                    // 1.38: источник, год, пиксель, объекты и непрозрачность
                    // спрятаны.
                    ui.label(tr(lang, "Images"));
                    if let Some(rec) = &mut self.recognize {
                        rec.picker.rows(ui, lang, &self.rasters);
                    }
                    ui.end_row();
                    ui.label(tr(lang, "Area|site"));
                    self.site.row(ui, lang);
                    ui.end_row();
                    ui.label(tr(lang, "Samples"));
                    if let Some(rec) = &mut self.recognize {
                        ui.horizontal(|ui| {
                            if ui.radio_value(&mut rec.areas, false, tr(lang, "Points")).on_hover_text(tr(lang, "A click on the globe puts a point of the chosen class. Three points per class are needed.")).changed() {
                                rec.drawing.clear();
                            }
                            ui.radio_value(&mut rec.areas, true, tr(lang, "Areas")).on_hover_text(tr(
                                lang,
                                "A click on the globe starts an outline of the chosen class, a click on the first vertex closes it. The samples of the class are the cells inside the outline, one outline per class is enough. A click inside an outline removes it.",
                            ));
                        });
                    }
                    ui.end_row();
                } else {
                    ui.label(tr(lang, "Point"));
                    ui.label(&self.place);
                    ui.end_row();
                }
                if let Some(task) = self.trace.as_mut().and_then(|t| t.task.as_mut()) {
                    new_task = task.rows(ui, lang, &self.rasters, &mut self.source);
                    ui.label(tr(lang, "Area|site"));
                    self.site.row(ui, lang);
                    ui.end_row();
                }
                if !task_mode && !recognize_mode {
                ui.label(tr(lang, "Source"));
                let shown = match self.source.as_deref() {
                    None => "AlphaEarth",
                    Some(BASEMAP) => tr(lang, "Globe basemap"),
                    Some(name) => name,
                };
                egui::ComboBox::from_id_salt("planetx-aef-class-source")
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        // У эмбеддингов AlphaEarth цвета нет, трассировке
                        // они не годятся, `TraceSource` модуля.
                        if !cfg!(target_arch = "wasm32") && !tracing_mode {
                            ui.selectable_value(&mut self.source, None, "AlphaEarth");
                        }
                        // Подложка глобуса - то, что на виде, `_earth_source`
                        // модуля.
                        ui.selectable_value(&mut self.source, Some(BASEMAP.to_string()), tr(lang, "Globe basemap"));
                        for (name, _) in &self.rasters {
                            ui.selectable_value(&mut self.source, Some(name.clone()), name);
                        }
                    })
                    .response
                    .on_hover_text(if tracing_mode {
                        tr(lang, "The image that is split into colours. The globe basemap is the picture of the current basemap under the area, a raster opened in the program gives its bands, the first three give the colours.")
                    } else if searching {
                        tr(lang, "What colour and texture are taken from. AlphaEarth vectors are loaded from the network, each band of a raster opened in the program is a feature. «Globe basemap» collects the tiles of the basemap under the area into a picture in memory.")
                    } else {
                        tr(lang, "What the cells are divided by. AlphaEarth vectors are loaded from the network. Each band of a raster opened in the program is a feature, the network is not needed. «Globe basemap» collects the tiles of the basemap under the area into a picture in memory. Borders of surveys in the image may be repeated by the classes.")
                    });
                ui.end_row();
                if self.source.is_none() {
                    ui.label(tr(lang, "Year"));
                    egui::ComboBox::from_id_salt("planetx-aef-class-year")
                        .selected_text(self.year.to_string())
                        .show_ui(ui, |ui| {
                            for y in aef::YEARS {
                                ui.selectable_value(&mut self.year, y, y.to_string());
                            }
                        })
                        .response
                        .on_hover_text(tr(lang, "The year by whose vectors the cells are divided into classes. The points are put by the image of the globe, it may be of another year."));
                    ui.end_row();
                }
                ui.label(tr(lang, "Area|site"));
                self.site.row(ui, lang);
                ui.end_row();
                }
                if fine {
                ui.label(tr(lang, "Pixel"));
                let text = match self.chosen_pixel() {
                    None => tr(lang, "Auto").to_string(),
                    Some(pixel) => trf(lang, "{pixel} m", &[("pixel", &format!("{pixel}"))]),
                };
                let first = if self.source.is_some() { 0 } else { FINE };
                egui::ComboBox::from_id_salt("planetx-aef-class-level")
                    .selected_text(text)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.level, None, tr(lang, "Auto"));
                        for (level, pixel) in PIXELS.iter().enumerate().skip(first) {
                            ui.selectable_value(&mut self.level, Some(level), trf(lang, "{pixel} m", &[("pixel", &format!("{pixel}"))]));
                        }
                    })
                    .response
                    .on_hover_text(if tracing_mode {
                        tr(lang, "The pixel of the grid of the area. «Auto» takes the pixel of the raster or of the top level of the basemap if the area has no more than 640 thousand cells. Tracing takes up to 4 million cells.")
                    } else {
                        tr(
                        lang,
                        "The pixel of the grid of the area. With AlphaEarth a block of the file for a year is 40-50 MB at any pixel, «Auto» takes the pixel with the least download at which the area has from 14 to 640 thousand cells. A raster and the globe basemap have pixels of 1, 2 and 5 m, «Auto» takes the pixel of the raster or of the top level of the basemap if the area has no more than 640 thousand cells. Division into objects takes up to 4 million cells.",
                        )
                    });
                ui.end_row();
                if let Some(text) = self.pixel_text(lang) {
                    ui.label("");
                    ui.add(egui::Label::new(text).wrap());
                    ui.end_row();
                }
                }
                if fine && let Some(trace) = &mut self.trace {
                    let p = &mut trace.params;
                    ui.label(tr(lang, "Colours"));
                    ui.add(egui::DragValue::new(&mut p.colors).range(2..=32)).on_hover_text(tr(lang, "How many colours the image is split into. More colours give more detailed areas and more polygons."));
                    ui.end_row();
                    ui.label(tr(lang, "Image smoothing"));
                    ui.add(egui::DragValue::new(&mut p.smooth).range(0..=5)).on_hover_text(tr(lang, "Passes of 3 × 3 smoothing of the image before it is split into colours. Smoothing removes noise, the borders come out smoother, small details disappear."));
                    ui.end_row();
                    ui.label(tr(lang, "Smallest area, cells"));
                    ui.add(egui::DragValue::new(&mut p.least).range(1..=100_000)).on_hover_text(tr(lang, "An area with fewer cells merges with the neighbour along the longest shared border."));
                    ui.end_row();
                    ui.label(tr(lang, "Corner rounding"));
                    ui.add(egui::DragValue::new(&mut p.corner).range(0.0..=1.33).speed(0.05).fixed_decimals(2)).on_hover_text(tr(lang, "Rounding of the outline corners, as in Potrace. 0 gives only corners, a broken line. The larger the value, the more corners become smooth arcs."));
                    ui.end_row();
                }
                if let Some(task) = self.trace.as_mut().and_then(|t| t.task.as_mut()) {
                    task.bottom_rows(ui, lang);
                }
                if !searching && !tracing_mode && !recognize_mode {
                    ui.label("");
                    ui.checkbox(&mut self.split.on, tr(lang, "Divide into objects")).on_hover_text(tr(lang, "The area is first divided into objects similar in colour and texture, a class goes to the whole object. The result is polygons."));
                    ui.end_row();
                }
                if self.split.on {
                    ui.label(tr(lang, "Object scale"));
                    ui.add(egui::DragValue::new(&mut self.split.scale).range(1..=500)).on_hover_text(tr(lang, "The allowed diversity of an object. The larger it is, the larger the objects and the more unlike things they join. The scale is related to the area, so a change of the pixel hardly changes the size of objects."));
                    ui.end_row();
                    ui.label(tr(lang, "Texture share"));
                    ui.add(egui::DragValue::new(&mut self.split.weight).range(0..=100).suffix(" %")).on_hover_text(if searching {
                        tr(lang, "The share of texture in the division into objects and in the comparison with the sample, the rest is colour.")
                    } else {
                        tr(lang, "The share of texture among the features, the rest is colour. Texture is the spread of brightness around a cell, it separates a forest from a meadow of the same colour.")
                    });
                    ui.end_row();
                    ui.label(tr(lang, "Least object, cells"));
                    ui.add(egui::DragValue::new(&mut self.split.least).range(1..=100_000)).on_hover_text(tr(lang, "An object with fewer cells merges with its most similar neighbour."));
                    ui.end_row();
                    if !searching {
                        ui.label("");
                        ui.checkbox(&mut self.split.join, tr(lang, "Merge neighbouring objects of one class")).on_hover_text(tr(lang, "Neighbouring objects of one class become one polygon. Without the flag each field or stand stays a separate object with its own values."));
                        ui.end_row();
                    }
                }
                if let Some(find) = &mut self.find {
                    ui.label(tr(lang, "Samples"));
                    ui.horizontal(|ui| {
                        ui.label(trf(lang, "Contours: {n}", &[("n", &find.rings.len().to_string())]));
                        if ui.button(tr(lang, "Remove samples")).on_hover_text(tr(lang, "Remove all sample outlines.")).clicked() {
                            find.rings.clear();
                            find.ring.clear();
                        }
                    });
                    ui.end_row();
                    ui.label(tr(lang, "Similarity"));
                    let r = ui.add(egui::Slider::new(&mut find.limit, 1..=99).suffix(" %")).on_hover_text(tr(lang, "How much an object must resemble the sample. The selection changes at once, nothing is computed again. To the right - only very similar objects, to the left - wider."));
                    if r.changed() {
                        find.moved = Some(web_time::Instant::now());
                    }
                    ui.end_row();
                    ui.label("");
                    pick_now = ui.checkbox(&mut find.join, tr(lang, "Merge neighbouring found objects")).on_hover_text(tr(lang, "Neighbouring found objects become one area. Without the flag each object stays a separate polygon.")).changed();
                    ui.end_row();
                } else if !recognize_mode {
                    ui.label(tr(lang, "Opacity"));
                    ui.add(egui::Slider::new(&mut self.opacity, 0.0..=1.0).show_value(false)).on_hover_text(tr(lang, "Opacity of the classes on the globe."));
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            if tracing_mode {
                ui.add(egui::Label::new(tr(lang, "The globe shows the colours of the area cells. «OK» puts smooth polygons of the areas into «My Places», neighbouring areas share their border.")).wrap());
                // Таблица классов после «Построить», у заросших полей классы
                // задаёт ряд снимков, таблицы нет.
                if let Some(trace) = &mut self.trace
                    && !trace.rows.is_empty()
                    && !fallow
                {
                    ui.horizontal(|ui| {
                        ui.label(tr(lang, "Classes"));
                        let waiting = trace.waiting;
                        if ui
                            .add_enabled(!waiting, egui::Button::new(tr(lang, "Names from the assistant")))
                            .on_hover_text(tr(lang, "The AI assistant proposes class names by the table - mean band values, share of the area, size of areas, place and month of the image. The image and layer names are not sent to the model, pressing the button is consent to send the table. The names can be corrected."))
                            .clicked()
                        {
                            ask = true;
                        }
                        if task_mode
                            && ui
                                .button(tr(lang, "Refine with points"))
                                .on_hover_text(tr(lang, "Opens the «Classification» window with the same image, area and classes of this window. There the classes are refined with own control points by clicks on the globe."))
                                .clicked()
                        {
                            refine = true;
                        }
                    });
                    let colors = trace.base.as_ref().map(|b| b.colors.clone()).unwrap_or_default();
                    let names: Vec<(usize, String)> = trace.rows.iter().map(|r| (r.number, r.name.clone())).collect();
                    egui::ScrollArea::vertical().id_salt("planetx-trace-table").max_height(TABLE_HEIGHT).show(ui, |ui| {
                        for row in &mut trace.rows {
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                                let [r, g, b] = colors.get(row.number).copied().unwrap_or([128; 3]);
                                ui.painter().rect_filled(rect, 2.0, egui::Color32::from_rgb(r, g, b));
                                if ui.add(egui::TextEdit::singleline(&mut row.name).desired_width(170.0)).on_hover_text(tr(lang, "The class name - in the name field of the polygons and in the scale on the globe.")).changed() {
                                    table_changed = true;
                                }
                                ui.label(format!("{:.0} %", 100.0 * row.share)).on_hover_text(tr(lang, "Share of the area."));
                                let merge = |name: &str| trf(lang, "Merge with «{name}»", &[("name", name)]);
                                let shown = names.iter().find(|(n, _)| *n as i32 == row.target).map_or_else(|| tr(lang, "Own class").to_string(), |(_, name)| merge(name));
                                egui::ComboBox::from_id_salt(("planetx-trace-join", row.number))
                                    .selected_text(shown)
                                    .show_ui(ui, |ui| {
                                        if ui.selectable_value(&mut row.target, -1, tr(lang, "Own class")).changed() {
                                            table_changed = true;
                                        }
                                        for (n, name) in &names {
                                            if *n != row.number && ui.selectable_value(&mut row.target, *n as i32, merge(name)).changed() {
                                                table_changed = true;
                                            }
                                        }
                                    })
                                    .response
                                    .on_hover_text(tr(lang, "Merge this colour with another: the areas of both colours become one class with the name and colour of the one it is merged with."));
                            });
                        }
                    });
                }
            } else if searching {
                ui.add(egui::Label::new(tr(lang, "Click the globe to outline a sample, a click on the first vertex closes the outline. There may be several samples.")).wrap());
            } else {
                ui.label(tr(lang, "Classes"));
                for (k, class) in self.classes.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.active, k, "").on_hover_text(tr(lang, "A click on the globe puts a point of this class."));
                        ui.color_edit_button_srgb(&mut class.color);
                        ui.add(egui::TextEdit::singleline(&mut class.name).desired_width(140.0));
                        if class.rings.is_empty() {
                            ui.label(trf(lang, "points {n}", &[("n", &class.points.len().to_string())])).on_hover_text(tr(lang, "Control points of the class."));
                        } else {
                            ui.label(trf(lang, "points {points}, areas {rings}", &[("points", &class.points.len().to_string()), ("rings", &class.rings.len().to_string())])).on_hover_text(tr(lang, "Points and areas of the class samples."));
                        }
                        if ui.small_button(tr(lang, "Remove")).on_hover_text(tr(lang, "Remove the class and its points.")).clicked() {
                            remove = Some(k);
                        }
                    });
                }
                ui.horizontal(|ui| {
                    if ui.button(tr(lang, "Add a class")).clicked() {
                        add = true;
                    }
                    if !recognize_mode && ui.button(tr(lang, "Classes from tracing")).on_hover_text(tr(lang, "Classes with names and colours from the «Tracing» or «Object extraction» window after «Build». Classes without points are replaced, the points are put by clicks on the globe.")).clicked() {
                        self.wants_traced = true;
                    }
                });
                let hint = if self.recognize.as_ref().is_some_and(|r| r.areas) {
                    tr(lang, "Outline a sample of the chosen class on the globe - by clicks on the vertices, a click on the first one closes the outline. At least two classes are needed, each with an outline or three points.")
                } else {
                    tr(lang, "Click the globe to put a point of the chosen class, a click on a point removes it. At least two classes of three points each are needed.")
                };
                ui.add(egui::Label::new(hint).wrap());
            }
            ui.add_space(4.0);
            ui.add(egui::Label::new(self.site.hint(lang).unwrap_or(&self.status)).wrap());
            if self.source.is_none() && !recognize_mode {
                ui.add(egui::Label::new(egui::RichText::new(format!("{} {}", tr(lang, "Data - AlphaEarth Foundations Satellite Embedding, CC BY 4.0."), aef::CREDIT)).small()).wrap());
            }
            ui.add_space(4.0);
            let tip = if recognize_mode {
                tr(lang, "First the area and the samples are checked, then the summer image and the image with snow of the area are picked and loaded, then each cell goes to the class of the nearest samples by the bands of both images.")
            } else if task_mode {
                tr(lang, "The image of the area is split into colours, the colours are named by the reflectance rules of the task, the areas are outlined with smooth outlines.")
            } else if tracing_mode {
                tr(lang, "The image of the area is split into colours, the areas of one colour are outlined with smooth outlines.")
            } else if searching {
                tr(lang, "The area is divided into objects, each object is compared with the samples in colour and texture.")
            } else if self.source.is_some() {
                tr(lang, "Each cell of the area goes to the class of the nearest control points by the raster bands.")
            } else {
                tr(lang, "Each cell of the area goes to the class of the nearest control points by AlphaEarth vectors.")
            };
            pressed = analysis::buttons(ui, lang, self.layer.is_some() && !self.busy(), tip);
        });
        self.terms_window(ctx, lang);
        if let Some(k) = remove
            && k < self.classes.len()
        {
            self.classes.remove(k);
            self.active = self.active.min(self.classes.len().saturating_sub(1));
        }
        if add {
            self.add_class(lang);
        }
        if let Some(s) = new_task
            && let Some(trace) = &mut self.trace
        {
            let spec = s.spec();
            trace.params.colors = spec.colors;
            trace.params.smooth = spec.smooth;
            trace.params.least = spec.least;
        }
        if table_changed {
            self.apply_trace(lang);
        }
        if ask {
            self.ask_names(lang);
        }
        if refine {
            self.wants_refine = true;
        }
        // Ползунок считает выборку после паузы, флажок - сразу.
        if let Some(find) = &mut self.find
            && let Some(at) = find.moved
        {
            if at.elapsed().as_secs_f64() >= PAUSE {
                find.moved = None;
                pick_now = true;
            } else {
                ctx.request_repaint();
            }
        }
        if pick_now {
            self.apply(lang);
        }
        if (self.opacity - opacity).abs() > 1e-6
            && let Some(old) = &self.layer
        {
            self.layer = Some(Arc::new(Layer { index: old.index.clone(), step: old.step, x0: old.x0, y0: old.y0, rows: old.rows, cols: old.cols, classes: old.classes.clone(), colors: old.colors.clone(), names: old.names.clone(), opacity: self.opacity, objects: old.objects.clone() }));
            self.changed = true;
        }
        match pressed {
            Some(Button::Build) => self.build(lang),
            Some(Button::Ok) => {
                self.save(lang);
                self.clear();
                self.open = false;
            }
            Some(Button::Cancel) => {
                self.clear();
                self.open = false;
            }
            None => {}
        }
        if !open {
            self.clear();
            self.open = false;
        }
        if std::mem::take(&mut self.site.changed) && self.layer.is_some() && !self.busy() {
            self.status = tr(lang, "The area has changed. Press «Build» to compute it again.").to_string();
        }
        if self.busy() || self.finding() || self.waits_names() {
            ctx.request_repaint();
        }
    }
}

impl analysis::SiteTool for ClassCalc {
    fn id(&self) -> &'static str {
        if self.recognize_mode() {
            "recognize"
        } else if self.task_mode() {
            "objects"
        } else if self.trace.is_some() {
            "tracing"
        } else if self.find.is_some() {
            "similar"
        } else {
            "aef_class"
        }
    }
    fn shown(&self) -> bool {
        self.layer.is_some()
    }
    fn layer(&self) -> Option<Arc<Layer>> {
        self.layer.clone()
    }
    fn legend_title(&self, lang: Lang) -> Option<&'static str> {
        if self.trace.is_some() {
            return Some(self.tool_title(lang));
        }
        self.find.is_none().then(|| tr(lang, if self.recognize_mode() { "Recognition" } else { "Classification" }))
    }
    fn painter(&self) -> Option<crate::vector::Painter> {
        self.layer.clone().map(painter)
    }
    fn drop_result(&mut self) {
        if self.layer.take().is_some() {
            self.changed = true;
        }
        if let Some(find) = &mut self.find {
            find.base = None;
        }
        if let Some(trace) = &mut self.trace {
            trace.base = None;
            trace.rows.clear();
            trace.joined = None;
        }
    }
}

/// Клетки сетки внутри многоугольника участка, `ring_mask` модуля.
pub fn ring_mask<T: Copy>(index: &ZoneIndex, grid: &Mosaic<T>, ring: &[(f64, f64)]) -> Vec<bool> {
    let mut out = Vec::with_capacity(grid.rows * grid.cols);
    for r in 0..grid.rows {
        let y = grid.y0 - (r as f64 + 0.5) * grid.step;
        let points: Vec<(f64, f64)> = (0..grid.cols).map(|c| aef::from_utm(index.number, index.south, grid.x0 + (c as f64 + 0.5) * grid.step, y)).collect();
        out.extend(data::site::inside(&points, ring));
    }
    out
}

/// Номера классов слоя файлом GeoTIFF в зоне UTM.
fn class_tiff(layer: &Layer) -> Vec<u8> {
    let grid = data::geotiff::MercatorGrid { x0: layer.x0 + layer.step / 2.0, y0: layer.y0 - layer.step / 2.0, step: layer.step, cols: layer.cols, rows: layer.rows };
    let bytes: Vec<u8> = layer.classes.iter().map(|c| u8::try_from(*c).unwrap_or(data::geotiff::BYTE_NODATA)).collect();
    data::geotiff::write_byte_projected(&grid, utm_epsg(&layer.index), &bytes).unwrap_or_default()
}

/// Многоугольники трассировки папкой «Моих меток», `write_traced` модуля.
/// Кольца - углы клеток сетки слоя, контур - цветом заливки, как
/// `_stroke_as_fill` модуля. None - областей больше [`MOST_OBJECTS`].
fn trace_places(layer: &Layer, base: &TraceBase, joined: &Joined, name: &str, opacity: f64) -> Option<scene::kml::Folder> {
    let done = &base.done;
    if done.shapes.len() > MOST_OBJECTS {
        return None;
    }
    let to_degrees = |ring: &Vec<(f64, f64)>| -> Vec<(f64, f64)> { ring.iter().map(|p| aef::from_utm(layer.index.number, layer.index.south, layer.x0 + p.0 * layer.step, layer.y0 - p.1 * layer.step)).collect() };
    let alpha = (opacity * 255.0).round() as u8;
    let mut folder = scene::kml::Folder::new(name);
    for (region, (outer, holes)) in &done.shapes {
        let Some(kind) = joined.kinds.get(*region).and_then(|k| usize::try_from(*k).ok()) else {
            continue;
        };
        let [r, g, b] = joined.colors.get(kind).copied().unwrap_or([128; 3]);
        let class = joined.names.get(kind).cloned().unwrap_or_default();
        let color = joined.hexes.get(kind).cloned().unwrap_or_default();
        let mut place = Place::new(&format!("{class} {}", region + 1), Kind::Polygon, to_degrees(outer));
        place.color = [r, g, b, 255];
        place.fill = Some([r, g, b, alpha]);
        if !holes.is_empty() {
            place.holes = Some(holes.iter().map(to_degrees).collect());
        }
        let cells = tracing::ring_area(outer).abs() - holes.iter().map(|h| tracing::ring_area(h).abs()).sum::<f64>();
        let area = cells * layer.step * layer.step / 1e4;
        let mut fields = vec![("cls".to_string(), (kind + 1).to_string()), ("name".to_string(), class), ("color".to_string(), color), ("area_ha".to_string(), format!("{area:.4}"))];
        for (field, values) in &joined.fields {
            fields.push(((*field).to_string(), values.get(*region).cloned().unwrap_or_default()));
        }
        place.data = Some(fields);
        folder.children.push(Item::Place(place));
    }
    Some(folder)
}

/// Цвет строкой «#rrggbb».
fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Доли участка цветов 0..count по клеткам classes, `shares` модуля.
fn color_shares(classes: &[i32], count: usize) -> Vec<f64> {
    let mut found = vec![0usize; count];
    for c in classes {
        if let Some(n) = usize::try_from(*c).ok().filter(|n| *n < count) {
            found[n] += 1;
        }
    }
    let total = found.iter().sum::<usize>().max(1);
    found.iter().map(|n| *n as f64 / total as f64).collect()
}

/// Название результата с настройками расчёта «Название (a, b)», `titled`
/// модуля 1.33: по названию видно, при каких настройках получен итог.
pub fn titled(title: &str, settings: &[String]) -> String {
    let parts: Vec<&str> = settings.iter().map(String::as_str).filter(|s| !s.is_empty()).collect();
    if parts.is_empty() { title.to_string() } else { format!("{title} ({})", parts.join(", ")) }
}

/// Классы с долей участка строкой: «Хвойный лес 43 %, Вода 21 %», по
/// убыванию доли, `class_rows` модуля.
fn class_text(classes: &[i32], names: &[String]) -> String {
    let shares = color_shares(classes, names.len());
    let mut order: Vec<usize> = (0..names.len()).filter(|n| shares[*n] > 0.0).collect();
    order.sort_by(|a, b| shares[*b].total_cmp(&shares[*a]));
    order.iter().map(|n| format!("{} {:.0} %", names[*n], 100.0 * shares[*n])).collect::<Vec<_>>().join(", ")
}

/// Доли классов строкой итога: «Лес 60%, Вода 40%».
fn class_shares(rows: &[&Class], classes: &[i16]) -> String {
    let counts: Vec<usize> = (0..rows.len()).map(|k| classes.iter().filter(|c| **c == k as i16).count()).collect();
    let total = counts.iter().sum::<usize>().max(1);
    let parts: Vec<String> = rows.iter().zip(&counts).map(|(c, n)| format!("{} {:.0}%", c.name, 100.0 * *n as f64 / total as f64)).collect();
    parts.join(", ")
}

/// Текст строки итога для отказа расчёта по растру, `problem_text`
/// модуля.
pub fn problem_text(lang: Lang, problem: Problem) -> &'static str {
    match problem {
        Problem::Unreadable => tr(lang, "The raster is not read."),
        Problem::NoCrs => tr(lang, "The raster has no coordinate system."),
        Problem::Big => tr(lang, "The grid of the area does not fit in memory, take a larger pixel."),
        Problem::Outside => tr(lang, "The area is outside the raster."),
        Problem::Points => tr(lang, "At least two classes with points on cells with data are needed."),
        Problem::Flat => tr(lang, "The raster bands do not change in the area, the classes cannot be told apart."),
        Problem::Whole => tr(lang, "The whole area came out as one object, reduce the object scale."),
        Problem::Pieces => tr(lang, "For objects the grid of the area is too large, take a larger pixel or «Auto»."),
        Problem::Samples => tr(lang, "There are no objects of the area in the sample outlines."),
        Problem::Tiles => tr(lang, "The basemap tiles were not received."),
    }
}

/// Точка (широта, долгота) на экране, кадровые пиксели, и видна ли.
fn pixel(nav: &Navigator, (lat, lon): (f64, f64)) -> ([f64; 2], bool) {
    let h = nav.pose.terrain.as_ref().map_or(0.0, |t| t(lat, lon));
    let point: DVec3 = nav.camera.ellipsoid.geodetic_to_ecef(lat, lon, h);
    let (px, front) = nav.camera.project(point);
    let facing = (nav.camera.eye - point).dot(point.normalize_or_zero()) > 0.0;
    ([px.x, px.y], front && facing)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Запрос без сети: ответа нет никогда.
    struct Silent(net::Stats);

    impl net::Request for Silent {
        fn start(&self, _url: &str) {}
        fn take(&self) -> Option<Result<Vec<u8>, String>> {
            None
        }
        fn stats(&self) -> &net::Stats {
            &self.0
        }
    }

    const LAT: f64 = 58.0;
    const LON: f64 = 56.25;

    /// Снимок «Сельское хозяйство» 2 × 2 км в UTM с подписями каналов:
    /// хвоя слева, листва справа, вырубка в хвое, вода внизу, дорога
    /// в три клетки поперёк листвы.
    fn summer() -> Raster {
        let Some(index) = ZoneIndex::bare(&aef::zone_of(LON, LAT)) else {
            return (String::new(), Arc::new(Vec::new()));
        };
        let (cx, cy) = index.project(LAT, LON);
        let (rows, cols) = (200usize, 200usize);
        let mut bands: Vec<Vec<f32>> = (0..3).map(|_| Vec::with_capacity(rows * cols)).collect();
        for r in 0..rows {
            for c in 0..cols {
                let cell: [f32; 3] = if (135..145).contains(&r) {
                    [0.01, 0.02, 0.04]
                } else if (60..90).contains(&r) && (40..70).contains(&c) {
                    [0.30, 0.22, 0.08]
                } else if c < 100 {
                    [0.08, 0.20, 0.02]
                } else if (120..123).contains(&r) {
                    [0.30, 0.22, 0.08]
                } else {
                    [0.17, 0.38, 0.03]
                };
                for (b, v) in bands.iter_mut().zip(cell) {
                    b.push(v);
                }
            }
        }
        let labels = ["B11 swir16", "B8 nir", "B2 blue"];
        let values: Vec<(String, Vec<f32>)> = labels.iter().map(|l| l.to_string()).zip(bands).collect();
        let grid = data::geotiff::MercatorGrid { x0: cx - 1000.0 + 5.0, y0: cy + 1000.0 - 5.0, step: 10.0, cols, rows };
        let bytes = data::geotiff::write_bands_projected(&grid, utm_epsg(&index), &values).unwrap_or_default();
        ("S2_20250721_40VDK_agriculture.tif".to_string(), Arc::new(bytes))
    }

    /// Кадр окна без экрана.
    fn frame(ctx: &egui::Context, calc: &mut ClassCalc, lang: Lang) {
        let mut out = ctx.run_pass(egui::RawInput::default(), |ctx| calc.show(ctx, lang));
        out.textures_delta.clear();
    }

    #[test]
    fn objects_window_names_classes_and_saves() {
        // Окно «Выделение объектов» по снимку с подписями каналов: виды по
        // отражению, рубка в хвое, линии, таблица классов и «OK».
        let lang = Lang::EN;
        let raster = summer();
        assert!(!raster.1.is_empty());
        let mut calc = ClassCalc::objects(LAT, LON, "test".into(), vec![raster.clone()], lang);
        let ctx = egui::Context::default();
        frame(&ctx, &mut calc, lang);
        assert_eq!(calc.source.as_deref(), Some(raster.0.as_str()));
        if let Some(task) = calc.trace.as_mut().and_then(|t| t.task.as_mut()) {
            task.lines = true;
        }
        calc.build(lang);
        let request = |_: u64| Box::new(Silent(net::Stats::default())) as Box<dyn net::Request>;
        for _ in 0..5000 {
            calc.step(lang, &request);
            if !calc.busy() {
                break;
            }
        }
        assert!(!calc.busy(), "{}", calc.status);
        let names = calc.trace.as_ref().and_then(|t| t.joined.as_ref()).map(|j| j.names.clone()).unwrap_or_default();
        for name in ["Coniferous forest", "Broadleaf forest and grass", "Water", "Clearcuts and burns"] {
            assert!(names.iter().any(|n| n == name), "{name} нет в {names:?}, {}", calc.status);
        }
        assert!(calc.status.contains("Lines"), "{}", calc.status);
        // Таблица и кнопки на виду, слияние двух классов - один класс.
        frame(&ctx, &mut calc, lang);
        let before = names.len();
        if let Some(trace) = &mut calc.trace
            && trace.rows.len() >= 2
        {
            let first = trace.rows[0].number as i32;
            trace.rows[1].target = first;
        }
        calc.apply_trace(lang);
        let after = calc.trace.as_ref().and_then(|t| t.joined.as_ref()).map_or(0, |j| j.names.len());
        assert!(after < before, "{before} -> {after}");
        assert!(!calc.class_pairs().is_empty());
        calc.save(lang);
        let saved = calc.saved.take();
        let folders: Vec<String> = saved.iter().flat_map(|s| s.extra.iter()).filter_map(|i| if let Item::Folder(f) = i { Some(f.name.clone()) } else { None }).collect();
        assert!(folders.iter().any(|n| n.starts_with("Object extraction S2_") && n.contains("task «Forests and clearcuts»")), "{folders:?}");
        assert!(folders.iter().any(|n| n == "Lines"), "{folders:?}");
        // «Уточнить точками»: участок, снимок и классы - окну «Классификация».
        calc.wants_refine = true;
        let refined = calc.take_refine(lang);
        assert!(refined.is_some(), "нет классов для «Классификации»");
        let Some((ring, source, pairs)) = refined else {
            return;
        };
        let mut classes = ClassCalc::new(LAT, LON, "test".into(), vec![raster.clone()], lang);
        classes.site.site.ring = ring;
        classes.refine_from(lang, source, &pairs);
        assert_eq!(classes.source.as_deref(), Some(raster.0.as_str()));
        assert_eq!(classes.classes.len(), pairs.len());
    }

    #[test]
    fn short_cover_is_refused() {
        // Снимок накрывает часть участка: «Построить» отказывает.
        let lang = Lang::EN;
        let raster = summer();
        let mut calc = ClassCalc::objects(LAT + 0.012, LON, "test".into(), vec![raster.clone()], lang);
        calc.source = Some(raster.0.clone());
        calc.build(lang);
        let request = |_: u64| Box::new(Silent(net::Stats::default())) as Box<dyn net::Request>;
        for _ in 0..5000 {
            calc.step(lang, &request);
            if !calc.busy() {
                break;
            }
        }
        assert!(calc.layer.is_none());
        assert!(calc.status.contains("covers only"), "{}", calc.status);
    }

    /// Снимок 2 × 2 км: слева left, посередине right, справа field.
    fn image(name: &str, left: [f32; 3], right: [f32; 3], field: [f32; 3]) -> Raster {
        let Some(index) = ZoneIndex::bare(&aef::zone_of(LON, LAT)) else {
            return (String::new(), Arc::new(Vec::new()));
        };
        let (cx, cy) = index.project(LAT, LON);
        let (rows, cols) = (200usize, 200usize);
        let mut bands: Vec<Vec<f32>> = (0..3).map(|_| Vec::with_capacity(rows * cols)).collect();
        for _ in 0..rows {
            for c in 0..cols {
                for (b, v) in bands.iter_mut().zip(if c < 100 { left } else if c < 130 { right } else { field }) {
                    b.push(v);
                }
            }
        }
        let values: Vec<(String, Vec<f32>)> = ["B11 swir16", "B8 nir", "B2 blue"].iter().map(|l| l.to_string()).zip(bands).collect();
        let grid = data::geotiff::MercatorGrid { x0: cx - 1000.0 + 5.0, y0: cy + 1000.0 - 5.0, step: 10.0, cols, rows };
        let bytes = data::geotiff::write_bands_projected(&grid, utm_epsg(&index), &values).unwrap_or_default();
        (name.to_string(), Arc::new(bytes))
    }

    #[test]
    fn snow_of_second_image_splits_conifers() {
        // Летом хвоя и листва одной яркости ИК, в марте хвоя тёмная на
        // снегу, голый лиственный лес светлый, `test_snow_tells_conifer`
        // модуля на сетке окна.
        let lang = Lang::EN;
        let summer = image("S2_20250721_40VDK_agriculture.tif", [0.17, 0.32, 0.03], [0.17, 0.33, 0.03], [0.30, 0.22, 0.08]);
        let march = image("S2_20260317_40VDK_agriculture.tif", [0.10, 0.22, 0.12], [0.18, 0.55, 0.45], [0.08, 0.80, 0.85]);
        let mut calc = ClassCalc::objects(LAT, LON, "test".into(), vec![summer.clone(), march.clone()], lang);
        calc.source = Some(summer.0.clone());
        if let Some(task) = calc.trace.as_mut().and_then(|t| t.task.as_mut()) {
            task.second = Some(march.0.clone());
        }
        calc.build(lang);
        let request = |_: u64| Box::new(Silent(net::Stats::default())) as Box<dyn net::Request>;
        for _ in 0..5000 {
            calc.step(lang, &request);
            if !calc.busy() {
                break;
            }
        }
        assert!(calc.status.contains("by the snow of the second image"), "{}", calc.status);
        let names = calc.trace.as_ref().and_then(|t| t.joined.as_ref()).map(|j| j.names.clone()).unwrap_or_default();
        assert!(names.contains(&"Coniferous forest".to_string()) && names.contains(&"Broadleaf forest and grass".to_string()), "{names:?}");
    }

    #[test]
    fn recognition_by_areas_takes_snow_image() {
        // Окно «Распознавание» модуля 1.38: без образцов отказ, снимки лета
        // и снега - из открытых растров без поиска, контур одного класса
        // и три точки другого, хвоя и листва летом одной яркости - их
        // делит снимок со снегом. «OK» пишет и контуры образцов.
        let lang = Lang::EN;
        let summer = image("S2_20250721_40VDK_agriculture.tif", [0.17, 0.32, 0.03], [0.17, 0.33, 0.03], [0.30, 0.22, 0.08]);
        let march = image("S2_20260317_40VDK_agriculture.tif", [0.10, 0.22, 0.12], [0.18, 0.55, 0.45], [0.08, 0.80, 0.85]);
        let mut calc = ClassCalc::recognition(LAT, LON, "test".into(), vec![summer.clone(), march.clone()], lang);
        let ctx = egui::Context::default();
        frame(&ctx, &mut calc, lang);
        calc.build(lang);
        assert!(calc.status.contains("each with an outline or three points"), "{}", calc.status);
        // Участок 1.8 км внутри снимка, хвоя - контур в левой части,
        // листва - три точки в средней полосе.
        let Some(index) = ZoneIndex::bare(&aef::zone_of(LON, LAT)) else {
            return;
        };
        let (cx, cy) = index.project(LAT, LON);
        let at = |x: f64, y: f64| aef::from_utm(index.number, index.south, cx + x, cy + y);
        calc.site.site.ring = vec![at(-900.0, -900.0), at(-900.0, 900.0), at(900.0, 900.0), at(900.0, -900.0)];
        calc.classes[0].rings.push(vec![at(-800.0, -300.0), at(-800.0, 300.0), at(-300.0, 300.0), at(-300.0, -300.0)]);
        calc.classes[1].points = vec![at(50.0, 0.0), at(150.0, 200.0), at(250.0, -200.0)];
        assert_eq!(calc.ring_under(at(-500.0, 0.0).0, at(-500.0, 0.0).1), Some((0, 0)));
        calc.build(lang);
        let request = |_: u64| Box::new(Silent(net::Stats::default())) as Box<dyn net::Request>;
        for _ in 0..5000 {
            calc.step(lang, &request);
            if !calc.busy() {
                break;
            }
        }
        assert_eq!(calc.source.as_deref(), Some(summer.0.as_str()));
        assert!(calc.layer.is_some(), "нет слоя: {}", calc.status);
        let Some(layer) = calc.layer.clone() else {
            return;
        };
        let class_at = |x: f64, y: f64| {
            let (lat, lon) = at(x, y);
            let (px, py) = index.project(lat, lon);
            let c = ((px - layer.x0) / layer.step) as usize;
            let r = ((layer.y0 - py) / layer.step) as usize;
            layer.classes.get(r * layer.cols + c).copied().unwrap_or(-1)
        };
        // Хвоя за контуром и листва без точек - по снегу.
        assert_eq!(class_at(-100.0, 700.0), 0, "{}", calc.status);
        assert_eq!(class_at(200.0, 700.0), 1, "{}", calc.status);
        calc.save(lang);
        let saved = calc.saved.take();
        let folders: Vec<String> = saved.iter().flat_map(|s| s.extra.iter()).filter_map(|i| if let Item::Folder(f) = i { Some(f.name.clone()) } else { None }).collect();
        assert!(folders.iter().any(|n| n == "Class samples"), "{folders:?}");
        assert!(saved.as_ref().is_some_and(|s| s.title == "Recognition" && s.layers.iter().any(|l| l.name.contains("areas 1") && l.name.contains("points 3"))));
        // Снимка со снегом нет - строка «Снег» ищется, лето остаётся.
        let mut picker = recognize::Picker::default();
        let ring = calc.site.site.ring.clone();
        assert!(!picker.fit(lang, &ring, std::slice::from_ref(&summer)));
        assert!(picker.busy());
        assert_eq!(picker.raster(recognize::Key::Summer), Some(summer.0.as_str()));
        // Снимок окна для другого участка уходит.
        let mut picker = recognize::Picker::default();
        picker.set(recognize::Key::Summer, Some(summer.0.clone()), Some(1.0), Some(data::site::square_ring(59.0, LON, 1000.0)));
        picker.set(recognize::Key::Snow, Some(march.0.clone()), None, None);
        assert!(picker.fit(lang, &ring, &[summer.clone(), march.clone()]));
        assert_eq!(picker.shots[0].cloud, None);
    }

    /// Снимок сезона «SWIR» (B12 B8A B4) 2 × 2 км: слева left, справа right.
    fn season(name: &str, left: [f32; 3], right: [f32; 3]) -> Raster {
        let Some(index) = ZoneIndex::bare(&aef::zone_of(LON, LAT)) else {
            return (String::new(), Arc::new(Vec::new()));
        };
        let (cx, cy) = index.project(LAT, LON);
        let (rows, cols) = (200usize, 200usize);
        let mut bands: Vec<Vec<f32>> = (0..3).map(|_| Vec::with_capacity(rows * cols)).collect();
        for _ in 0..rows {
            for c in 0..cols {
                for (b, v) in bands.iter_mut().zip(if c < 100 { left } else { right }) {
                    b.push(v);
                }
            }
        }
        let values: Vec<(String, Vec<f32>)> = ["B12 swir22", "B8A nir08", "B4 red"].iter().map(|l| l.to_string()).zip(bands).collect();
        let grid = data::geotiff::MercatorGrid { x0: cx - 1000.0 + 5.0, y0: cy + 1000.0 - 5.0, step: 10.0, cols, rows };
        let bytes = data::geotiff::write_bands_projected(&grid, utm_epsg(&index), &values).unwrap_or_default();
        (name.to_string(), Arc::new(bytes))
    }

    #[test]
    fn overgrown_fields_by_season_images() {
        // Два поля одного летнего вида: левое голое весной 2026 года -
        // пашня, правое ни разу не голое - залежь. Снимки сезонов клетки
        // таблицы берут из открытых растров по дню в имени.
        let lang = Lang::EN;
        let summer = image("S2_20250721_40VDK_agriculture.tif", [0.17, 0.38, 0.03], [0.18, 0.37, 0.03], [0.17, 0.38, 0.03]);
        let year = crate::s2scenes::today().0;
        let spring = season(&format!("S2_{year}0525_40VDK_swir.tif"), [0.20, 0.17, 0.12], [0.14, 0.30, 0.06]);
        let autumn = season(&format!("S2_{}1004_40VDK_swir.tif", year - 1), [0.20, 0.17, 0.12], [0.14, 0.30, 0.06]);
        let mut calc = ClassCalc::objects(LAT, LON, "test".into(), vec![summer.clone(), spring, autumn], lang);
        if let Some(task) = calc.trace.as_mut().and_then(|t| t.task.as_mut()) {
            task.scenario = data::objecttasks::Scenario::Fallow;
        }
        let ctx = egui::Context::default();
        let mut out = ctx.run_pass(egui::RawInput::default(), |ctx| calc.show(ctx, lang));
        out.textures_delta.clear();
        calc.source = Some(summer.0.clone());
        calc.build(lang);
        let request = |_: u64| Box::new(Silent(net::Stats::default())) as Box<dyn net::Request>;
        for _ in 0..5000 {
            calc.step(lang, &request);
            if !calc.busy() {
                break;
            }
        }
        assert!(calc.status.contains("Spring and autumn images 2"), "{}", calc.status);
        let joined = calc.trace.as_ref().and_then(|t| t.joined.clone());
        let names = joined.as_ref().map(|j| j.names.clone()).unwrap_or_default();
        assert!(names.contains(&"Ploughland, tilled".to_string()) && names.contains(&"Fallow or meadow".to_string()), "{names:?}");
        assert!(joined.is_some_and(|j| j.fields.iter().any(|(n, _)| *n == "last_bare")));
    }
}
