// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Снимки ASTER окна «Минералы по спектру»: свой поиск подходящих сцен
//! участка и загрузка всех каналов, перенос `ui/asterscenes.py` модуля
//! 1.31.
//!
//! Просьба автора модуля от 2 ноября 2026 года - путь через окно
//! «Космоснимки» неочевиден, окну нужна своя выборка, только подходящие
//! снимки. Подходящий снимок - сцена ASTER L1T Planetary Computer
//! 2000-2006 годов с каналами VNIR и SWIR, днём, малооблачная
//! ([`aster::suits_minerals`]), с данными не меньше [`aster::PICK_VALID`]
//! процентов участка. Доля данных - по каналу 1 файла VNIR с шагом 90 м,
//! как облачность сцены окна модуля. Отказ каталога повторяется
//! [`SEARCH_TRIES`] раза с растущей паузой.
//!
//! Загрузка - файлы VNIR, SWIR и TIR по окну участка, каждый канал - своим
//! чтением с общим заголовком файла. Значения - [`aster::all_bands`],
//! файл - многоканальный GeoTIFF с подписями каналов в памяти
//! ([`data::geotiff::write_bands_projected`]). Окно открывает его как
//! растр программы, без записи на диск.

use std::sync::Arc;

use data::cog::CogHeader;
use data::sentinel::{self as s2, Raw, Scene, aster};
use serde_json::Value;

use crate::cogread::{BandRead, HeaderRead};
use crate::i18n::{Lang, tr, trf};

/// Повторов поиска при отказе каталога и пауза перед повтором, с,
/// растущая с номером, `SEARCH_TRIES` и `SEARCH_PAUSE` модуля.
pub const SEARCH_TRIES: u32 = 3;
const SEARCH_PAUSE: f64 = 3.0;
/// Сцен в ответе каталога.
const LIMIT: usize = 100;
/// Предел ответа каталога, байты.
const ANSWER: u64 = 16 << 20;
/// Чтений окна одновременно.
const WORKERS: usize = 4;

type Request<'a> = &'a dyn Fn(u64) -> Box<dyn net::Request>;

/// Проверка покрытия участка сценой: окно канала 1 файла VNIR.
struct Check {
    scene: Scene,
    xy: Vec<(f64, f64)>,
    bbox: [f64; 4],
    res: f64,
    read: Option<BandRead>,
}

/// Файл сцены при загрузке: адрес, номера каналов, заголовок и чтения
/// каналов.
struct FileRead {
    name: String,
    href: String,
    count: usize,
    header: Option<HeaderRead>,
    reads: Vec<BandRead>,
}

/// Загрузка всех каналов сцены.
struct Load {
    scene: Scene,
    xy: Vec<(f64, f64)>,
    bbox: [f64; 4],
    res: f64,
    files: Vec<FileRead>,
}

/// Поиск подходящих сцен ASTER участка и загрузка всех каналов.
#[derive(Default)]
pub struct AsterScenes {
    ring: Vec<(f64, f64)>,
    search: Option<Box<dyn net::Request>>,
    body: Option<Value>,
    tries: u32,
    retry_at: Option<f64>,
    scenes: Vec<Scene>,
    token: Option<(String, f64)>,
    token_ask: Option<Box<dyn net::Request>>,
    checks: Vec<Check>,
    result: Vec<(Scene, f64)>,
    load: Option<Load>,
    /// Новый список окна: сцены и доля участка с данными, %.
    pub found: Option<Vec<(Scene, f64)>>,
    /// Новая строка состояния окна.
    pub status: Option<String>,
    /// Готовый снимок: имя файла и байты GeoTIFF.
    pub loaded: Option<(String, Vec<u8>)>,
}

impl AsterScenes {
    pub fn busy(&self) -> bool {
        self.search.is_some() || self.retry_at.is_some() || self.token_ask.is_some() || !self.checks.is_empty() || self.load.is_some()
    }

    /// Загрузка снимка идёт.
    pub fn loading(&self) -> bool {
        self.load.is_some()
    }

    /// Остановить поиск и загрузку, ключ SAS остаётся.
    pub fn stop(&mut self) {
        self.search = None;
        self.body = None;
        self.retry_at = None;
        self.token_ask = None;
        self.checks.clear();
        self.load = None;
    }

    /// Поиск подходящих сцен над участком ring, `search` модуля.
    pub fn search(&mut self, lang: Lang, ring: &[(f64, f64)], request: Request) {
        self.stop();
        self.tries = 0;
        self.ring = ring.to_vec();
        self.scenes.clear();
        self.result.clear();
        let body = s2::search_body(&s2::geojson(ring), aster::FIRST_DAY, aster::LAST_DAY, aster::PICK_CLOUD, LIMIT, aster::MISSION);
        self.ask(body, request);
        self.status = Some(tr(lang, "Searching for ASTER images of the area…").to_string());
    }

    fn ask(&mut self, body: Value, request: Request) {
        let ask = request(ANSWER);
        ask.send(s2::search_url(aster::MISSION), &s2::SEARCH_HEADERS, Some(body.to_string().as_bytes()));
        self.search = Some(ask);
        self.body = Some(body);
    }

    /// Загрузить все каналы сцены по участку ring.
    pub fn load(&mut self, lang: Lang, scene: &Scene, ring: &[(f64, f64)]) {
        if self.load.is_some() {
            return;
        }
        let Some(epsg) = scene.epsg else {
            self.status = Some(tr(lang, "The scene has no UTM system.").to_string());
            return;
        };
        let Ok(xy) = s2::ring_utm(ring, epsg) else {
            self.status = Some(tr(lang, "The area is outside the UTM zone of the scene.").to_string());
            return;
        };
        let res = s2::step_for(&xy, aster::MISSION, &["all_bands"]);
        let Some(bbox) = s2::bounds(&xy, res) else {
            return;
        };
        let keys = aster::needed(&["all_bands"], Some(&scene.assets));
        let files = aster::files_of(&keys).into_iter().filter(|(f, _)| scene.assets.contains(f)).map(|(name, bands)| (name, bands.len())).collect::<Vec<_>>();
        // Чтения начинаются, когда есть ключ SAS.
        self.load = Some(Load {
            scene: scene.clone(),
            xy,
            bbox,
            res,
            files: files.into_iter().map(|(name, count)| FileRead { name, href: String::new(), count, header: None, reads: Vec::new() }).collect(),
        });
        self.status = Some(trf(lang, "Loading the bands of the image {when}…", &[("when", day(scene))]));
    }

    /// Ключ SAS свежий.
    fn token_ok(&self, now: f64) -> Option<String> {
        self.token.as_ref().filter(|(_, at)| now - at < s2::SAS_LIFE as f64).map(|(t, _)| t.clone())
    }

    /// Шаг: повтор поиска, ответы каталога, ключ SAS, проверки покрытия,
    /// загрузка каналов.
    pub fn step(&mut self, lang: Lang, now: f64, request: Request) {
        if self.retry_at.is_some_and(|at| now >= at) {
            self.retry_at = None;
            if let Some(body) = self.body.take() {
                self.ask(body, request);
            }
        }
        self.step_search(lang, now, request);
        if let Some(ask) = &self.token_ask
            && let Some(answer) = ask.take()
        {
            self.token_ask = None;
            match answer.ok().and_then(|b| s2::parse_token(&b)) {
                Some(t) => self.token = Some((t, now)),
                None => {
                    // Список окна пустеет, только если ключ ждали проверки
                    // покрытия: отказ при загрузке список не трогает.
                    if !self.checks.is_empty() {
                        self.checks.clear();
                        self.result.clear();
                        self.found = Some(Vec::new());
                    }
                    self.load = None;
                    self.status = Some(tr(lang, "Planetary Computer did not give a key for the ASTER files. Repeat later.").to_string());
                }
            }
        }
        let wants_key = !self.checks.is_empty() || self.load.is_some();
        let Some(sas) = self.token_ok(now) else {
            if wants_key && self.token_ask.is_none() {
                let first = self.checks.first().map(|c| &c.scene).or(self.load.as_ref().map(|l| &l.scene)).and_then(|s| s.assets.get("VNIR")).and_then(|a| a.href.first()).map(str::to_string);
                let ask = request(1 << 20);
                ask.start(&s2::sas_url(aster::MISSION, first.as_deref()));
                self.token_ask = Some(ask);
            }
            return;
        };
        self.step_checks(lang, &sas, request);
        self.step_load(lang, &sas, request);
    }

    fn step_search(&mut self, lang: Lang, now: f64, request: Request) {
        let Some(ask) = &self.search else {
            return;
        };
        let Some(answer) = ask.take() else {
            return;
        };
        self.search = None;
        match answer.and_then(|b| serde_json::from_slice::<Value>(&b).map_err(|e| e.to_string())) {
            Err(e) => {
                // Planetary Computer под нагрузкой отвечает 502-504, повтор
                // обычно проходит - замечание автора модуля от 2 ноября
                // 2026 года.
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
                self.scenes.extend(scenes.into_iter().filter(aster::suits_minerals));
                if let Some(next) = following
                    && self.scenes.len() < s2::MAX_SCENES
                {
                    self.ask(next, request);
                    return;
                }
                self.body = None;
                if self.scenes.is_empty() {
                    self.status = Some(trf(lang, "There are no suitable ASTER images: scenes of 2000-2006 with SWIR bands, in daytime, with cloud cover up to {cloud} % are needed.", &[("cloud", &format!("{:.0}", aster::PICK_CLOUD))]));
                    self.found = Some(Vec::new());
                    return;
                }
                self.status = Some(trf(lang, "Scenes {count}, the coverage of the area is checked…", &[("count", &self.scenes.len().to_string())]));
                let (_, res) = s2::cloud_asset(aster::MISSION);
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

    /// Проверки покрытия: окна канала 1 по [`WORKERS`] сразу.
    fn step_checks(&mut self, lang: Lang, sas: &str, request: Request) {
        if self.checks.is_empty() {
            return;
        }
        let mut running = self.checks.iter().filter(|c| c.read.as_ref().is_some_and(|r| r.result().is_none() && r.error().is_none())).count();
        let (key, _) = s2::cloud_asset(aster::MISSION);
        for check in &mut self.checks {
            match &mut check.read {
                Some(r) => r.step(request),
                None if running < WORKERS => {
                    let href = check.scene.assets.get(key).and_then(|a| s2::sign(&a.href, sas).first().map(str::to_string));
                    if let Some(href) = href {
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
                // Без адреса канала сцена не проверяется.
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
            let valid = s2::mask(&check.xy, check.bbox, check.res).and_then(|inside| s2::clouds_of(&check.scene, raw, &inside)).map(|(_, v)| v);
            if let Some(v) = valid.filter(|v| *v >= aster::PICK_VALID) {
                self.result.push((check.scene, v));
            }
        }
        self.checks = kept;
        if self.checks.is_empty() {
            self.finish_checks(lang);
        }
    }

    fn finish_checks(&mut self, lang: Lang) {
        let mut found = std::mem::take(&mut self.result);
        aster::pick_order(&mut found);
        found.truncate(aster::PICK_SHOWN);
        self.status = Some(if found.is_empty() {
            trf(lang, "There are no suitable ASTER images over the area: the scenes cover less than {valid} % of the area.", &[("valid", &format!("{:.0}", aster::PICK_VALID))])
        } else {
            trf(lang, "Suitable ASTER images: {count}. Choose an image and press «Load».", &[("count", &found.len().to_string())])
        });
        self.found = Some(found);
    }

    /// Загрузка: заголовки файлов, окна каналов, файл GeoTIFF.
    fn step_load(&mut self, lang: Lang, sas: &str, request: Request) {
        let Some(load) = &mut self.load else {
            return;
        };
        let mut error = None;
        for file in &mut load.files {
            if file.header.is_none() {
                let Some(href) = load.scene.assets.get(&file.name).and_then(|a| s2::sign(&a.href, sas).first().map(str::to_string)) else {
                    error = Some(file.name.clone());
                    break;
                };
                file.header = Some(HeaderRead::new(&href, request));
                file.href = href;
            }
            let Some(header) = &mut file.header else {
                continue;
            };
            header.step(request);
            match header.result() {
                None => {}
                Some(Err(e)) => {
                    error = Some(format!("{}: {e}", file.name));
                    break;
                }
                Some(Ok(header)) => {
                    if file.reads.is_empty() {
                        file.reads = start_reads(&file.href, header.clone(), file.count, load.bbox, load.res, request);
                    }
                    for read in &mut file.reads {
                        read.step(request);
                        if let Some(e) = read.error() {
                            error = Some(format!("{}: {e}", file.name));
                        }
                    }
                }
            }
        }
        if let Some(e) = error {
            self.load = None;
            self.status = Some(trf(lang, "The image was not loaded: {error}", &[("error", &e)]));
            return;
        }
        let total: usize = load.files.iter().map(|f| f.count).sum();
        let done: usize = load.files.iter().map(|f| f.reads.iter().filter(|r| r.result().is_some()).count()).sum();
        if done < total {
            self.status = Some(trf(lang, "Loading the bands of the image {when}: {done} of {total}", &[("when", day(&load.scene)), ("done", &done.to_string()), ("total", &total.to_string())]));
            return;
        }
        let Some(load) = self.load.take() else {
            return;
        };
        match build(&load) {
            Some(made) => {
                self.status = Some(trf(lang, "The image «{name}» is loaded. Click it on the globe.", &[("name", &made.0)]));
                self.loaded = Some(made);
            }
            None => self.status = Some(tr(lang, "The area has no data in this scene.").to_string()),
        }
    }
}

/// Чтения каналов файла с общим заголовком.
fn start_reads(href: &str, header: Arc<CogHeader>, count: usize, bbox: [f64; 4], res: f64, request: Request) -> Vec<BandRead> {
    (0..count).filter_map(|n| u16::try_from(n).ok()).map(|n| BandRead::with_header_sample(href, header.clone(), bbox, res, true, n, request)).collect()
}

/// День съёмки сцены «ГГГГ-ММ-ДД».
fn day(scene: &Scene) -> &str {
    scene.when.get(..10).unwrap_or(&scene.when)
}

/// Каналы прочитаны: значения, файл GeoTIFF и его имя.
fn build(load: &Load) -> Option<(String, Vec<u8>)> {
    let files: Vec<(String, Vec<Raw>)> = load.files.iter().map(|f| (f.name.clone(), f.reads.iter().filter_map(|r| r.result().cloned()).collect())).collect();
    let inside = s2::mask(&load.xy, load.bbox, load.res)?;
    let bands = aster::all_bands(&load.scene, &files, &inside)?;
    if !bands.iter().any(|(_, g)| g.data().iter().any(|v| v.is_finite())) {
        return None;
    }
    let (rows, cols) = inside.shape();
    let grid = data::geotiff::MercatorGrid { x0: load.bbox[0] + load.res / 2.0, y0: load.bbox[3] - load.res / 2.0, step: load.res, cols, rows };
    let epsg = u16::try_from(load.scene.epsg?).ok()?;
    let values: Vec<(String, Vec<f32>)> = bands.into_iter().filter(|(_, g)| g.shape() == (rows, cols)).map(|(label, g)| (label, g.data().to_vec())).collect();
    let bytes = data::geotiff::write_bands_projected(&grid, epsg, &values)?;
    let name = format!("ASTER {} {}.tif", day(&load.scene), load.scene.tile).replace(['/', '\\', ':'], "_");
    Some((name, bytes))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    use super::*;

    /// Каталог: ответы по очереди и количество запросов.
    #[derive(Default)]
    struct Catalogue {
        answers: RefCell<VecDeque<Result<Vec<u8>, String>>>,
        sent: RefCell<usize>,
    }

    struct Fake {
        catalogue: Rc<Catalogue>,
        ready: RefCell<Option<Result<Vec<u8>, String>>>,
        stats: net::Stats,
    }

    impl net::Request for Fake {
        fn start(&self, url: &str) {
            self.send(url, &[], None);
        }
        fn send(&self, _url: &str, _headers: &[(&str, &str)], _body: Option<&[u8]>) {
            *self.catalogue.sent.borrow_mut() += 1;
            *self.ready.borrow_mut() = self.catalogue.answers.borrow_mut().pop_front();
        }
        fn take(&self) -> Option<Result<Vec<u8>, String>> {
            self.ready.borrow_mut().take()
        }
        fn stats(&self) -> &net::Stats {
            &self.stats
        }
    }

    fn run(answers: Vec<Result<Vec<u8>, String>>) -> (usize, AsterScenes) {
        let catalogue = Rc::new(Catalogue { answers: RefCell::new(answers.into()), sent: RefCell::new(0) });
        let shared = catalogue.clone();
        let request = move |_: u64| Box::new(Fake { catalogue: shared.clone(), ready: RefCell::new(None), stats: net::Stats::default() }) as Box<dyn net::Request>;
        let mut scenes = AsterScenes::default();
        let ring = s2::square(37.2, -117.2, 3000.0);
        scenes.search(Lang::EN, &ring, &request);
        for now in 0..40 {
            scenes.step(Lang::EN, f64::from(now), &request);
        }
        let sent = *catalogue.sent.borrow();
        (sent, scenes)
    }

    #[test]
    fn busy_catalogue_is_asked_again() {
        // Сторож шагов `spectral_aster_retry` модуля 1.31: каталог дважды
        // ответил 504, третий запрос прошёл. На прежнем коде запрос был
        // один и список оставался пустым с отказом.
        let empty = br#"{"type": "FeatureCollection", "features": []}"#.to_vec();
        let (sent, mut scenes) = run(vec![Err("HTTP 504".into()), Err("HTTP 504".into()), Ok(empty)]);
        assert_eq!(sent, 3);
        assert_eq!(scenes.found.take(), Some(Vec::new()));
        assert!(scenes.status.take().is_some_and(|s| s.starts_with("There are no suitable ASTER images:")));
        assert!(!scenes.busy());
        // Отказ после всех повторов - строка отказа и пустой список.
        let (sent, mut scenes) = run(vec![Err("HTTP 504".into()); 5]);
        assert_eq!(sent, 1 + SEARCH_TRIES as usize);
        assert_eq!(scenes.found.take(), Some(Vec::new()));
        assert!(scenes.status.take().is_some_and(|s| s.contains("504")));
    }
}