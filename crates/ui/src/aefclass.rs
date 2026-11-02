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

use std::sync::Arc;

use data::aef::{self, Mosaic, Part, Piece, ZoneIndex};
use data::rasterclass::{self, Problem, Run, Source, Warp};
use data::segments::{self, Cut};
use data::similar;
use data::tilemosaic::{self, TileWarp};
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

/// Подложка на глобусе для источника «Подложка глобуса»: название,
/// источник тайлов, подпись прав.
#[derive(Debug, Clone, PartialEq)]
pub struct Backdrop {
    pub name: String,
    pub source: net::Source,
    pub credit: String,
}

/// Класс: название, цвет, контрольные точки.
#[derive(Debug, Clone, PartialEq)]
pub struct Class {
    pub name: String,
    pub color: [u8; 3],
    pub points: Vec<(f64, f64)>,
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
}

impl ClassCalc {
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
        };
        calc.add_class(lang);
        calc.add_class(lang);
        calc.active = 0;
        calc
    }

    fn add_class(&mut self, lang: Lang) {
        let n = self.classes.len();
        self.classes.push(Class { name: trf(lang, "Class {n}", &[("n", &(n + 1).to_string())]), color: COLORS[n % COLORS.len()], points: Vec::new() });
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
        self.status.clear();
    }

    /// Ключ векторов участка.
    fn key(&self, level: usize) -> (String, i32, usize) {
        let (w, s, e, n) = self.site.site.bbox();
        (format!("{w:.6},{s:.6},{e:.6},{n:.6}"), self.year, level)
    }

    fn build(&mut self, lang: Lang) {
        if let Some(find) = &self.find {
            if find.rings.is_empty() {
                self.status = tr(lang, "Outline a sample on the globe.").to_string();
                return;
            }
        } else {
            let ready = self.classes.iter().filter(|c| !c.points.is_empty()).count() >= aef::MIN_CLASSES && self.classes.iter().filter(|c| !c.points.is_empty()).all(|c| c.points.len() >= aef::MIN_POINTS);
            if !ready {
                self.status = tr(lang, "At least two classes of three points each are needed.").to_string();
                return;
            }
        }
        if let Some(problem) = self.site.problem(lang) {
            self.status = problem;
            return;
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
        let share = rasterclass::covered(&grid.values, grid.depth, &keep);
        if share <= 0.0 {
            self.own_problem(lang, Problem::Outside);
            return;
        }
        let rows: Vec<&Class> = self.classes.iter().filter(|c| !c.points.is_empty()).collect();
        let points: Vec<(i32, (f64, f64))> = rows.iter().enumerate().flat_map(|(n, c)| c.points.iter().map(move |p| (n as i32, *p))).collect();
        if self.split.on {
            self.start_cut(lang, Ready { key, index, grid, share }, keep, points);
            return;
        }
        let (samples, labels, outside) = rasterclass::samples(&grid, &index, &points);
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
            Arc::new(Layer { index: base.index.clone(), step: grid.step, x0: grid.x0, y0: grid.y0, rows: grid.rows, cols: grid.cols, classes, colors: vec![FOUND_COLOR], opacity: self.opacity, objects: Some(Arc::new(objects)) })
        });
        self.layer = layer;
        self.changed = true;
    }

    /// Точки классов для расчёта: номер класса и место.
    fn points(&self) -> Vec<(i32, (f64, f64))> {
        self.classes.iter().filter(|c| !c.points.is_empty()).enumerate().flat_map(|(n, c)| c.points.iter().map(move |p| (n as i32, *p))).collect()
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
        let rows: Vec<&Class> = self.classes.iter().filter(|c| !c.points.is_empty()).collect();
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
        self.layer = Some(Arc::new(Layer { index, step: grid.step, x0: grid.x0, y0: grid.y0, rows: grid.rows, cols: grid.cols, classes, colors: rows.iter().map(|c| c.color).collect(), opacity: self.opacity, objects: Some(objects) }));
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
        }
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
        let rows: Vec<&Class> = self.classes.iter().filter(|c| !c.points.is_empty()).collect();
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
        self.layer = Some(Arc::new(Layer { index, step: grid.step, x0: grid.x0, y0: grid.y0, rows: grid.rows, cols: grid.cols, classes, colors: rows.iter().map(|c| c.color).collect(), opacity: self.opacity, objects: None }));
        self.label = self.source_title(&key.0).to_string();
        self.changed = true;
    }

    /// Шаг загрузки векторов.
    pub fn step(&mut self, lang: Lang, request: Request) {
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
        let rows: Vec<&Class> = self.classes.iter().filter(|c| !c.points.is_empty()).collect();
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
        self.layer = Some(Arc::new(Layer { index: index.clone(), step: grid.step, x0: grid.x0, y0: grid.y0, rows: grid.rows, cols: grid.cols, classes, colors: rows.iter().map(|c| c.color).collect(), opacity: self.opacity, objects: None }));
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
        let Some(layer) = &self.layer else {
            return;
        };
        let grid = data::geotiff::MercatorGrid { x0: layer.x0 + layer.step / 2.0, y0: layer.y0 - layer.step / 2.0, step: layer.step, cols: layer.cols, rows: layer.rows };
        let bytes: Vec<u8> = layer.classes.iter().map(|c| u8::try_from(*c).unwrap_or(data::geotiff::BYTE_NODATA)).collect();
        let tiff = data::geotiff::write_byte_projected(&grid, utm_epsg(&layer.index), &bytes).unwrap_or_default();
        let shown = layer.clone();
        let sample = move |points: &[(f64, f64)]| shown.sample(points);
        let name = trf(lang, if self.find.is_some() { "Similar areas {year}" } else { "Classification {year}" }, &[("year", &self.label)]);
        let rows: Vec<(String, [u8; 3])> = if self.find.is_some() { vec![(tr(lang, "Similar areas").to_string(), FOUND_COLOR)] } else { self.classes.iter().filter(|c| !c.points.is_empty()).map(|c| (c.name.clone(), c.color)).collect() };
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
        for (n, class) in self.classes.iter().filter(|c| !c.points.is_empty()).enumerate() {
            let mut folder = scene::kml::Folder::new(&trf(lang, "{n} - {name}", &[("n", &n.to_string()), ("name", &class.name)]));
            folder.description = trf(lang, "Value {n} in the GeoTIFF file.", &[("n", &n.to_string())]);
            for &(lat, lon) in &class.points {
                let mut place = Place::new(&class.name, Kind::Point, vec![(lat, lon)]);
                place.color = [class.color[0], class.color[1], class.color[2], 255];
                folder.children.push(Item::Place(place));
            }
            extra.push(Item::Folder(folder));
        }
        let title = tr(lang, if self.find.is_some() { "Similar areas" } else { "Classification" }).to_string();
        self.saved = Some(Saved { title, ring: self.site.site.ring.clone(), layers, extra, session: None });
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
        self.site.site.most_km = if self.source.is_some() { RASTER_MOST } else { MOST };
        let mut open = true;
        let mut pressed = None;
        let opacity = self.opacity;
        let mut remove = None;
        let mut add = false;
        let searching = self.find.is_some();
        let mut pick_now = false;
        let title = tr(lang, if searching { "Similar areas" } else { "Classification" });
        let id = if searching { "planetx-similar" } else { "planetx-aef-class" };
        egui::Window::new(title).id(egui::Id::unique(id)).open(&mut open).collapsible(false).resizable(false).min_width(400.0).show(ctx, |ui| {
            egui::Grid::new("planetx-aef-class-form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Point"));
                ui.label(&self.place);
                ui.end_row();
                ui.label(tr(lang, "Source"));
                let shown = match self.source.as_deref() {
                    None => "AlphaEarth",
                    Some(BASEMAP) => tr(lang, "Globe basemap"),
                    Some(name) => name,
                };
                egui::ComboBox::from_id_salt("planetx-aef-class-source")
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        if !cfg!(target_arch = "wasm32") {
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
                    .on_hover_text(if searching {
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
                    .on_hover_text(tr(
                        lang,
                        "The pixel of the grid of the area. With AlphaEarth a block of the file for a year is 40-50 MB at any pixel, «Auto» takes the pixel with the least download at which the area has from 14 to 640 thousand cells. A raster and the globe basemap have pixels of 1, 2 and 5 m, «Auto» takes the pixel of the raster or of the top level of the basemap if the area has no more than 640 thousand cells. Division into objects takes up to 4 million cells.",
                    ));
                ui.end_row();
                if let Some(text) = self.pixel_text(lang) {
                    ui.label("");
                    ui.add(egui::Label::new(text).wrap());
                    ui.end_row();
                }
                if !searching {
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
                } else {
                    ui.label(tr(lang, "Opacity"));
                    ui.add(egui::Slider::new(&mut self.opacity, 0.0..=1.0).show_value(false)).on_hover_text(tr(lang, "Opacity of the classes on the globe."));
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            if searching {
                ui.add(egui::Label::new(tr(lang, "Click the globe to outline a sample, a click on the first vertex closes the outline. There may be several samples.")).wrap());
            } else {
                ui.label(tr(lang, "Classes"));
                for (k, class) in self.classes.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.active, k, "").on_hover_text(tr(lang, "A click on the globe puts a point of this class."));
                        ui.color_edit_button_srgb(&mut class.color);
                        ui.add(egui::TextEdit::singleline(&mut class.name).desired_width(140.0));
                        ui.label(trf(lang, "points {n}", &[("n", &class.points.len().to_string())]));
                        if ui.small_button(tr(lang, "Remove")).on_hover_text(tr(lang, "Remove the class and its points.")).clicked() {
                            remove = Some(k);
                        }
                    });
                }
                if ui.button(tr(lang, "Add a class")).clicked() {
                    add = true;
                }
                ui.add(egui::Label::new(tr(lang, "Click the globe to put a point of the chosen class, a click on a point removes it. At least two classes of three points each are needed.")).wrap());
            }
            ui.add_space(4.0);
            ui.add(egui::Label::new(self.site.hint(lang).unwrap_or(&self.status)).wrap());
            if self.source.is_none() {
                ui.add(egui::Label::new(egui::RichText::new(format!("{} {}", tr(lang, "Data - AlphaEarth Foundations Satellite Embedding, CC BY 4.0."), aef::CREDIT)).small()).wrap());
            }
            ui.add_space(4.0);
            let tip = if searching {
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
            self.layer = Some(Arc::new(Layer { index: old.index.clone(), step: old.step, x0: old.x0, y0: old.y0, rows: old.rows, cols: old.cols, classes: old.classes.clone(), colors: old.colors.clone(), opacity: self.opacity, objects: old.objects.clone() }));
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
        if self.busy() {
            ctx.request_repaint();
        }
    }
}

impl analysis::SiteTool for ClassCalc {
    fn id(&self) -> &'static str {
        if self.find.is_some() { "similar" } else { "aef_class" }
    }
    fn shown(&self) -> bool {
        self.layer.is_some()
    }
    fn layer(&self) -> Option<Arc<Layer>> {
        self.layer.clone()
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
