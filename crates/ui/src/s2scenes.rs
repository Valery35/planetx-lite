// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Снимки Sentinel-2 окна «Выделение объектов»: свой поиск подходящих
//! сцен участка и загрузка сочетания каналов, перенос `ui/s2finder.py`
//! модуля 1.34-1.36.
//!
//! Просьба автора модуля от 3 ноября 2026 года - «система громоздкая,
//! упрости»: снимок для задачи не надо искать окном «Космоснимки».
//! Подходящая сцена - Sentinel-2 L2A Earth Search за последние годы
//! в месяцах сезона, солнце не ниже [`SUN_MIN`], облачность над участком
//! по маске SCL не больше [`CLOUD_MAX`], данные не меньше [`VALID_MIN`]
//! участка. Отказ каталога повторяется [`SEARCH_TRIES`] раза с растущей
//! паузой, отказ файлов при загрузке - [`LOAD_TRIES`] раза.
//!
//! Загрузка - каналы сочетания по окну участка в системе UTM сцены,
//! отражение, многоканальный GeoTIFF с подписями каналов «B11 swir16»
//! в памяти. Имя файла - «S2_ГГГГММДД_тайл_сочетание.tif», по нему окно
//! узнаёт день и сочетание, как у файлов «Космоснимков» модуля.

use data::sentinel::{self as s2, Mission, Raw, Scene};
use serde_json::Value;

use crate::cogread::BandRead;
use crate::i18n::{Lang, tr, trf};

/// Лет назад от сегодня у поиска летнего и второго снимка.
pub const YEARS: i32 = 3;
/// Облачность сцены в каталоге, %.
const SCENE_CLOUD: f64 = 40.0;
/// Облачность над участком, %.
pub const CLOUD_MAX: f64 = 5.0;
/// Участка с данными, %.
pub const VALID_MIN: f64 = 90.0;
/// Высота солнца, градусы.
const SUN_MIN: f64 = 20.0;
/// Снимков в списке.
pub const SHOWN: usize = 8;
/// Повторов поиска и пауза перед повтором, с, растущая с номером.
const SEARCH_TRIES: u32 = 3;
const SEARCH_PAUSE: f64 = 3.0;
/// Попыток загрузки снимка и пауза между ними, с.
const LOAD_TRIES: u32 = 3;
const LOAD_PAUSE: f64 = 3.0;
/// Сцен в ответе каталога.
const LIMIT: usize = 100;
/// Предел ответа каталога, байты.
const ANSWER: u64 = 16 << 20;
/// Чтений окна одновременно.
const WORKERS: usize = 4;
/// Сочетание летнего и второго снимка: B11 B8 B2, хвоя, листва и почва
/// различимы.
pub const PRODUCT: &str = "agriculture";
/// Сочетание снимков сезонов: B12 B8A B4, красный и ближний ИК для голой
/// почвы, SWIR для деревьев.
pub const SEASON_PRODUCT: &str = "swir";
/// Месяцы лета по порядку - июль лучше всего.
pub const SUMMER: [u32; 3] = [7, 6, 8];
/// Второй снимок - март и апрель, пока лежит снег, и октябрь без снега:
/// решение автора модуля «у нас снег сходит в конце апреля».
pub const OFF_SEASON: [u32; 3] = [3, 4, 10];

type Request<'a> = &'a dyn Fn(u64) -> Box<dyn net::Request>;

/// Найденная сцена: сцена, облачность над участком и данные, %.
pub type Found = (Scene, f64, f64);

/// Проверка облачности участка: окно маски SCL.
struct Check {
    scene: Scene,
    xy: Vec<(f64, f64)>,
    bbox: [f64; 4],
    res: f64,
    read: Option<BandRead>,
}

/// Загрузка каналов сочетания.
struct Load {
    scene: Scene,
    product: String,
    xy: Vec<(f64, f64)>,
    bbox: [f64; 4],
    res: f64,
    bands: Vec<(String, Option<BandRead>)>,
    ring: Vec<(f64, f64)>,
}

/// Повтор загрузки: когда, сцена, участок, сочетание.
struct Retry {
    at: f64,
    scene: Scene,
    ring: Vec<(f64, f64)>,
    product: String,
}

/// Порядок найденных снимков: облачность, месяц по порядку months, новые
/// раньше, `order` модуля.
pub fn order(found: &mut [Found], months: &[u32]) {
    let month = |s: &Scene| s.when.get(5..7).and_then(|m| m.parse::<u32>().ok()).and_then(|m| months.iter().position(|x| *x == m)).unwrap_or(months.len());
    found.sort_by(|a, b| (a.1.round() as i64, month(&a.0)).cmp(&(b.1.round() as i64, month(&b.0))).then_with(|| b.0.when.cmp(&a.0.when)));
}

/// Поиск подходящих сцен Sentinel-2 участка и загрузка сочетания.
#[derive(Default)]
pub struct SceneFinder {
    ring: Vec<(f64, f64)>,
    months: Vec<u32>,
    shown: usize,
    search: Option<Box<dyn net::Request>>,
    body: Option<Value>,
    tries: u32,
    retry_at: Option<f64>,
    scenes: Vec<Scene>,
    checks: Vec<Check>,
    result: Vec<Found>,
    load: Option<Load>,
    load_tries: u32,
    load_at: Option<Retry>,
    /// Новый список: сцены с облачностью и данными.
    pub found: Option<Vec<Found>>,
    /// Новая строка состояния окна.
    pub status: Option<String>,
    /// Готовый снимок: имя файла и байты GeoTIFF.
    pub loaded: Option<(String, Vec<u8>)>,
    /// Снимок не загрузился и после повторов.
    pub failed: bool,
}

impl SceneFinder {
    pub fn busy(&self) -> bool {
        self.search.is_some() || self.retry_at.is_some() || !self.checks.is_empty() || self.load.is_some() || self.load_at.is_some()
    }

    /// Остановить поиск и загрузку.
    pub fn stop(&mut self) {
        self.search = None;
        self.body = None;
        self.retry_at = None;
        self.checks.clear();
        self.load = None;
        self.load_at = None;
    }

    /// Поиск сцен над участком ring в месяцах months за years лет до
    /// сегодня, в списке - не больше shown.
    pub fn search(&mut self, lang: Lang, ring: &[(f64, f64)], months: &[u32], years: i32, shown: usize, request: Request) {
        let today = today();
        self.stop();
        self.tries = 0;
        self.ring = ring.to_vec();
        self.months = months.to_vec();
        self.shown = shown;
        self.scenes.clear();
        self.result.clear();
        let start = format!("{:04}-{:02}-{:02}", today.0 - i64::from(years), today.1, today.2.min(28));
        let end = format!("{:04}-{:02}-{:02}", today.0, today.1, today.2);
        let body = s2::search_body(&s2::geojson(ring), &start, &end, SCENE_CLOUD, LIMIT, Mission::Sentinel2);
        self.ask(body, request);
        self.status = Some(tr(lang, "Searching for Sentinel-2 images of the area…").to_string());
    }

    fn ask(&mut self, body: Value, request: Request) {
        let ask = request(ANSWER);
        ask.send(s2::search_url(Mission::Sentinel2), &s2::SEARCH_HEADERS, Some(body.to_string().as_bytes()));
        self.search = Some(ask);
        self.body = Some(body);
    }

    /// Загрузить сочетание product сцены по участку ring.
    pub fn load(&mut self, lang: Lang, scene: &Scene, ring: &[(f64, f64)], product: &str) {
        if self.load.is_some() {
            return;
        }
        self.load_tries = 1;
        self.failed = false;
        self.start_load(lang, scene, ring, product);
    }

    fn start_load(&mut self, lang: Lang, scene: &Scene, ring: &[(f64, f64)], product: &str) {
        let Some(epsg) = scene.epsg else {
            self.status = Some(tr(lang, "The scene has no UTM system.").to_string());
            self.failed = true;
            return;
        };
        let Ok(xy) = s2::ring_utm(ring, epsg) else {
            self.status = Some(tr(lang, "The area is outside the UTM zone of the scene.").to_string());
            self.failed = true;
            return;
        };
        let res = s2::step_for(&xy, Mission::Sentinel2, &[product]);
        let Some(bbox) = s2::bounds(&xy, res) else {
            self.failed = true;
            return;
        };
        let bands = s2::needed(&[product], Mission::Sentinel2, false).into_iter().map(|k| (k, None)).collect();
        self.load = Some(Load { scene: scene.clone(), product: product.to_string(), xy, bbox, res, bands, ring: ring.to_vec() });
        self.status = Some(trf(lang, "Loading the image {when}…", &[("when", day(scene))]));
    }

    /// Шаг: повторы, ответы каталога, проверки облачности, загрузка.
    pub fn step(&mut self, lang: Lang, now: f64, request: Request) {
        if self.retry_at.is_some_and(|at| now >= at) {
            self.retry_at = None;
            if let Some(body) = self.body.take() {
                self.ask(body, request);
            }
        }
        if self.load_at.as_ref().is_some_and(|r| now >= r.at)
            && let Some(retry) = self.load_at.take()
        {
            self.start_load(lang, &retry.scene, &retry.ring, &retry.product);
        }
        self.step_search(lang, now, request);
        self.step_checks(lang, request);
        self.step_load(lang, now, request);
    }

    fn step_search(&mut self, lang: Lang, now: f64, request: Request) {
        let Some(answer) = self.search.as_ref().and_then(|a| a.take()) else {
            return;
        };
        self.search = None;
        match answer.and_then(|b| serde_json::from_slice::<Value>(&b).map_err(|e| e.to_string())) {
            Err(e) => {
                if self.tries < SEARCH_TRIES && self.body.is_some() {
                    self.tries += 1;
                    self.retry_at = Some(now + SEARCH_PAUSE * f64::from(self.tries));
                    self.status = Some(trf(lang, "The catalogue is busy, repeat of the search {n} of {total}…", &[("n", &self.tries.to_string()), ("total", &SEARCH_TRIES.to_string())]));
                    return;
                }
                self.body = None;
                self.status = Some(trf(lang, "The catalogue did not answer: {error}", &[("error", &e)]));
                self.found = Some(Vec::new());
            }
            Ok(page) => {
                self.tries = 0;
                let (scenes, following) = s2::parse_page(&page);
                let months = self.months.clone();
                self.scenes.extend(scenes.into_iter().filter(|s| s.when.get(5..7).and_then(|m| m.parse::<u32>().ok()).is_some_and(|m| months.contains(&m)) && s.sun.is_none_or(|sun| sun >= SUN_MIN)));
                if let Some(next) = following
                    && self.scenes.len() < s2::MAX_SCENES
                {
                    self.ask(next, request);
                    return;
                }
                self.body = None;
                if self.scenes.is_empty() {
                    self.status = Some(tr(lang, "There are no Sentinel-2 scenes of the needed season over the area.").to_string());
                    self.found = Some(Vec::new());
                    return;
                }
                self.status = Some(trf(lang, "Scenes {count}, the cloud cover over the area is counted…", &[("count", &self.scenes.len().to_string())]));
                let (_, res) = s2::cloud_asset(Mission::Sentinel2);
                self.checks = self
                    .scenes
                    .iter()
                    .filter_map(|s| {
                        let xy = s2::ring_utm(&self.ring, s.epsg?).ok()?;
                        let bbox = s2::bounds(&xy, res)?;
                        Some(Check { scene: s.clone(), xy, bbox, res, read: None })
                    })
                    .collect();
                if self.checks.is_empty() {
                    self.finish_checks(lang);
                }
            }
        }
    }

    /// Проверки облачности: окна маски SCL по [`WORKERS`] сразу.
    fn step_checks(&mut self, lang: Lang, request: Request) {
        if self.checks.is_empty() {
            return;
        }
        let (key, _) = s2::cloud_asset(Mission::Sentinel2);
        let mut running = self.checks.iter().filter(|c| c.read.as_ref().is_some_and(|r| r.result().is_none() && r.error().is_none())).count();
        for check in &mut self.checks {
            match &mut check.read {
                Some(r) => r.step(request),
                None if running < WORKERS => {
                    if let Some(href) = check.scene.assets.get(key).and_then(|a| a.href.first().map(str::to_string)) {
                        check.read = Some(BandRead::new(&href, check.bbox, check.res, true, request));
                        running += 1;
                    }
                }
                None => {}
            }
        }
        let mut kept = Vec::new();
        for check in std::mem::take(&mut self.checks) {
            let Some(read) = &check.read else {
                if check.scene.assets.contains(key) {
                    kept.push(check);
                }
                continue;
            };
            if read.error().is_some() {
                continue;
            }
            let Some(raw) = read.result() else {
                kept.push(check);
                continue;
            };
            let counted = s2::mask(&check.xy, check.bbox, check.res).and_then(|inside| s2::clouds_of(&check.scene, raw, &inside));
            if let Some((cloud, valid)) = counted
                && cloud <= CLOUD_MAX
                && valid >= VALID_MIN
            {
                self.result.push((check.scene, cloud, valid));
            }
        }
        self.checks = kept;
        if self.checks.is_empty() {
            self.finish_checks(lang);
        }
    }

    fn finish_checks(&mut self, lang: Lang) {
        let mut found = std::mem::take(&mut self.result);
        order(&mut found, &self.months);
        found.truncate(self.shown);
        self.status = Some(if found.is_empty() {
            trf(lang, "There are no suitable images: over the area the scenes have cloud cover above {cloud} % or less than {valid} % of data.", &[("cloud", &format!("{CLOUD_MAX:.0}")), ("valid", &format!("{VALID_MIN:.0}"))])
        } else {
            trf(lang, "Suitable images: {count}. A double click on an image loads it.", &[("count", &found.len().to_string())])
        });
        self.found = Some(found);
    }

    /// Загрузка каналов и файл GeoTIFF. Отказ сервера повторяется: каналы
    /// Sentinel-2 на AWS изредка не отдаются с первого раза (прогоны
    /// модуля 3 ноября 2026 года).
    fn step_load(&mut self, lang: Lang, now: f64, request: Request) {
        let Some(load) = &mut self.load else {
            return;
        };
        let mut running = load.bands.iter().filter(|(_, r)| r.as_ref().is_some_and(|r| r.result().is_none() && r.error().is_none())).count();
        let mut error = None;
        for (key, read) in &mut load.bands {
            match read {
                Some(r) => {
                    r.step(request);
                    if let Some(e) = r.error() {
                        error = Some(format!("{key}: {e}"));
                    }
                }
                None if running < WORKERS => {
                    let Some(href) = load.scene.assets.get(key).and_then(|a| a.href.first().map(str::to_string)) else {
                        error = Some(key.clone());
                        break;
                    };
                    *read = Some(BandRead::new(&href, load.bbox, load.res, false, request));
                    running += 1;
                }
                None => {}
            }
        }
        if let Some(e) = error {
            let Some(load) = self.load.take() else {
                return;
            };
            if self.load_tries < LOAD_TRIES {
                self.load_tries += 1;
                self.status = Some(trf(lang, "The server did not give the image, repeat {n} of {total}…", &[("n", &(self.load_tries - 1).to_string()), ("total", &(LOAD_TRIES - 1).to_string())]));
                self.load_at = Some(Retry { at: now + LOAD_PAUSE, scene: load.scene, ring: load.ring, product: load.product });
                return;
            }
            self.status = Some(trf(lang, "The image was not loaded - the server did not give the bands {error}. Repeat the double click later or take another image.", &[("error", &e)]));
            self.failed = true;
            return;
        }
        let done = load.bands.iter().filter(|(_, r)| r.as_ref().is_some_and(|r| r.result().is_some())).count();
        if done < load.bands.len() {
            self.status = Some(trf(lang, "Loading the image {when}: bands {done} of {total}", &[("when", day(&load.scene)), ("done", &done.to_string()), ("total", &load.bands.len().to_string())]));
            return;
        }
        let Some(load) = self.load.take() else {
            return;
        };
        match build(&load) {
            Some(made) => {
                self.status = Some(trf(lang, "The image «{name}» is loaded.", &[("name", &made.0)]));
                self.loaded = Some(made);
            }
            None => {
                self.status = Some(tr(lang, "The area has no data in this scene.").to_string());
                self.failed = true;
            }
        }
    }
}

/// Сегодня (год, месяц, день) по часам компьютера.
pub fn today() -> (i64, u32, u32) {
    let secs = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
    crate::pointcalc::civil_from_days((secs / 86400.0).floor() as i64)
}

/// День съёмки сцены «ГГГГ-ММ-ДД».
pub fn day(scene: &Scene) -> &str {
    scene.when.get(..10).unwrap_or(&scene.when)
}

/// Каналы прочитаны: отражение, файл GeoTIFF и его имя.
fn build(load: &Load) -> Option<(String, Vec<u8>)> {
    let inside = s2::mask(&load.xy, load.bbox, load.res)?;
    let (rows, cols) = inside.shape();
    let mut values: Vec<(String, Vec<f32>)> = Vec::new();
    for (key, read) in &load.bands {
        let raw: &Raw = read.as_ref()?.result()?;
        let asset = load.scene.assets.get(key)?;
        let grid = s2::reflectance(raw, asset);
        if grid.shape() != (rows, cols) {
            return None;
        }
        let band: Vec<f32> = grid.data().iter().zip(inside.data()).map(|(v, m)| if *m { *v } else { f32::NAN }).collect();
        values.push((s2::band_label(&load.scene, key), band));
    }
    if !values.iter().any(|(_, b)| b.iter().any(|v| v.is_finite())) {
        return None;
    }
    let grid = data::geotiff::MercatorGrid { x0: load.bbox[0] + load.res / 2.0, y0: load.bbox[3] - load.res / 2.0, step: load.res, cols, rows };
    let epsg = u16::try_from(load.scene.epsg?).ok()?;
    let bytes = data::geotiff::write_bands_projected(&grid, epsg, &values)?;
    let name = format!("S2_{}_{}_{}.tif", day(&load.scene).replace('-', ""), load.scene.tile, load.product).replace(['/', '\\', ':', ' '], "_");
    Some((name, bytes))
}
