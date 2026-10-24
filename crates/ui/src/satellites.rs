// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Строка «Спутники»: группы CelesTrak, загрузка, показ. Перенос
//! `ui/satellites.py` модуля, расчёт - `data::satellites`
//! и `data::sgp4`.
//!
//! Правило CelesTrak - одна загрузка группы на обновление, данные
//! обновляются раз в 2 часа. У программы группа хранится файлом в папке
//! данных (`satellites/<группа>.csv`), файл моложе `REFETCH` читается без
//! сети. После ответа с ошибкой группа не просится до конца того же
//! срока, показывается прежний файл, если он есть. У страницы файлов нет,
//! срок держится до перезагрузки страницы.
//!
//! Начальные величины спутников считаются частями по `INIT_BUDGET`
//! за кадр. Без открытой шкалы времени спутники идут по часам
//! компьютера.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use data::satellites::{self as core, Swarm};

use crate::i18n::{Lang, tr, trf};

/// Шаг пересчёта положений по часам, секунды, `TICK` модуля.
pub const TICK: f64 = 0.25;
/// Шаг при камере следом, секунды.
pub const FOLLOW_TICK: f64 = 0.05;
/// Радиус выбора спутника, логические пиксели.
pub const PICK_PIXELS: f64 = 8.0;
/// Толщина витка и следа, логические пиксели.
pub const PATH_WIDTH: f32 = 1.5;
/// Непрозрачность следа на земле, витка - 255.
pub const TRACK_ALPHA: u8 = 150;
/// Подъём следа над эллипсоидом, метры: точка на самом эллипсоиде
/// попадает в проверку «за Землёй» по ошибке округления.
pub const TRACK_LIFT: f64 = 50.0;
/// Наибольший ответ группы, байт: Starlink - около 2 МБ.
const ANSWER_BYTES: u64 = 32 << 20;

/// Группа: ключ, название, подсказка.
pub fn group_names(lang: Lang) -> Vec<(&'static str, &'static str, &'static str)> {
    core::GROUPS
        .iter()
        .map(|(key, _)| {
            let (name, tip) = match *key {
                "stations" => ("Space stations", "The International Space Station, the Chinese Tiangong station and the spacecraft docked to them."),
                "visual" => ("Bright satellites", "About a hundred satellites and rocket stages visible from the Earth without binoculars."),
                "gnss" => ("Navigation", "GPS, GLONASS, Galileo and BeiDou satellites on medium orbits of about 20 000 km and geosynchronous ones."),
                "weather" => ("Weather satellites", "Weather satellites on polar and geostationary orbits."),
                "resource" => ("Earth observation", "Earth imaging satellites, Landsat and Sentinel among them."),
                "science" => ("Science", "Science satellites and observatories on Earth orbits."),
                "geo" => ("Geostationary", "Active satellites on the geosynchronous orbit, about 36 000 km above the equator. They stay almost still over the Earth."),
                "starlink" => ("Starlink", "SpaceX communication satellites at 340-570 km, thousands of them. Loading and the first calculation take longer than for other groups."),
                _ => ("OneWeb", "OneWeb communication satellites at about 1200 km."),
            };
            (*key, tr(lang, name), tr(lang, tip))
        })
        .collect()
}

pub struct Satellites {
    pub swarm: Swarm,
    pub groups: BTreeSet<String>,
    asks: HashMap<String, Box<dyn net::Request>>,
    /// Ошибки групп для подсказки строки.
    pub errors: BTreeMap<String, String>,
    /// Отказы CelesTrak: группа - секунды UTC отказа.
    failed: HashMap<String, f64>,
    /// Папка файлов групп, у страницы None.
    dir: Option<std::path::PathBuf>,
    /// Выбранный спутник - номер NORAD, у него виток и след.
    pub selected: Option<i64>,
    pub follow: bool,
    /// Виток и след выбранного: ключ (номер, шаг) и путь.
    path: Option<((i64, i64), Option<core::Path>)>,
    /// Посчитаны ли начальные величины.
    pub ready: bool,
    /// Время прошлых положений.
    pub shown_at: f64,
}

impl Satellites {
    pub fn new(groups: &[String], dir: Option<std::path::PathBuf>) -> Self {
        let known: BTreeSet<String> = groups.iter().filter(|g| core::GROUPS.iter().any(|(k, _)| k == g)).cloned().collect();
        Self {
            swarm: Swarm::new(),
            groups: known,
            asks: HashMap::new(),
            errors: BTreeMap::new(),
            failed: HashMap::new(),
            dir,
            selected: None,
            follow: false,
            path: None,
            ready: false,
            shown_at: f64::NEG_INFINITY,
        }
    }

    /// Идёт загрузка или первый расчёт.
    pub fn busy(&self) -> bool {
        !self.asks.is_empty() || (!self.groups.is_empty() && !self.ready && !self.errors.len().eq(&self.groups.len()))
    }

    /// Новые группы: снятые уходят, новые просятся.
    pub fn set_groups(&mut self, groups: BTreeSet<String>) {
        self.asks.retain(|g, _| groups.contains(g));
        self.swarm.keep(groups.iter());
        self.groups = groups;
        self.ready = false;
        self.path = None;
    }

    fn file(&self, group: &str) -> Option<std::path::PathBuf> {
        Some(self.dir.as_ref()?.join(format!("{group}.csv")))
    }

    /// У страницы файлов нет.
    #[cfg(target_arch = "wasm32")]
    fn read_file(&self, _group: &str, _now: f64) -> Option<(Vec<data::sgp4::Element>, f64)> {
        None
    }

    /// Элементы группы из файла и возраст файла в секундах.
    #[cfg(not(target_arch = "wasm32"))]
    fn read_file(&self, group: &str, now: f64) -> Option<(Vec<data::sgp4::Element>, f64)> {
        let path = self.file(group)?;
        let text = std::fs::read_to_string(&path).ok()?;
        let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
        let age = now - modified.duration_since(web_time::UNIX_EPOCH).ok()?.as_secs_f64();
        let elements = data::sgp4::parse_csv(&text);
        (!elements.is_empty()).then_some((elements, age))
    }

    /// Отказ CelesTrak: из памяти или файла отметки.
    fn failed_at(&self, group: &str) -> f64 {
        let from_file = self.file(group).and_then(|p| std::fs::read_to_string(p.with_extension("failed")).ok()).and_then(|t| t.trim().parse::<f64>().ok());
        self.failed.get(group).copied().or(from_file).unwrap_or(f64::NEG_INFINITY)
    }

    /// Шаг: ответы групп, новые запросы, начальные величины частями.
    /// now - секунды UTC.
    pub fn step(&mut self, lang: Lang, now: f64, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        let arrived: Vec<(String, Result<Vec<u8>, String>)> = self.asks.iter().filter_map(|(g, a)| a.take().map(|r| (g.clone(), r))).collect();
        for (group, answer) in arrived {
            self.asks.remove(&group);
            let text = answer.as_ref().ok().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
            let elements = data::sgp4::parse_csv(&text);
            if elements.is_empty() {
                self.failed.insert(group.clone(), now);
                if let Some(p) = self.file(&group) {
                    let _ = std::fs::write(p.with_extension("failed"), now.to_string());
                }
                self.errors.insert(group.clone(), answer.err().unwrap_or_else(|| tr(lang, "The answer has no orbital elements.").to_string()));
                if let Some((old, _)) = self.read_file(&group, now) {
                    self.use_group(&group, old);
                }
                continue;
            }
            self.errors.remove(&group);
            if let Some(p) = self.file(&group) {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if let Err(e) = std::fs::write(&p, &text) {
                    // Без файла группа покажется, но в следующий раз уйдёт
                    // в сеть раньше срока.
                    self.errors.insert(group.clone(), e.to_string());
                }
            }
            self.use_group(&group, elements);
        }
        let missing: Vec<String> = self.groups.iter().filter(|g| !self.asks.contains_key(*g) && !self.swarm.group_names().contains(&g.as_str()) && !self.errors.contains_key(*g)).cloned().collect();
        for group in missing {
            let file = self.read_file(&group, now);
            if let Some((elements, age)) = &file
                && *age < core::REFETCH
            {
                // Файл моложе обновления CelesTrak - сеть не нужна.
                self.use_group(&group, elements.clone());
                continue;
            }
            if now - self.failed_at(&group) < core::REFETCH {
                // После ошибки CelesTrak просит не спрашивать снова.
                self.errors.insert(group.clone(), tr(lang, "CelesTrak refused the group, a new request goes 2 hours after the refusal.").to_string());
                if let Some((elements, _)) = file {
                    self.use_group(&group, elements);
                }
                continue;
            }
            let ask = request(ANSWER_BYTES);
            ask.start(&core::feed(&group));
            self.asks.insert(group, ask);
        }
        if !self.ready && !self.swarm.group_names().is_empty() {
            let start = web_time::Instant::now();
            while start.elapsed().as_secs_f64() < 0.004 {
                if self.swarm.init_some(core::INIT_BUDGET) {
                    self.ready = true;
                    if self.selected.is_some_and(|n| self.swarm.index_of(n).is_none()) {
                        // Группа выбранного спутника снята.
                        self.selected = None;
                        self.follow = false;
                    }
                    self.path = None;
                    break;
                }
            }
        }
    }

    fn use_group(&mut self, group: &str, elements: Vec<data::sgp4::Element>) {
        if !self.groups.contains(group) {
            return;
        }
        self.swarm.set_group(group, elements);
        self.ready = false;
    }

    /// Положения на момент: точки ECEF, годные и не за Землёй для глаза.
    pub fn points(&mut self, moment: f64, eye: [f64; 3]) -> Vec<(usize, [f64; 3])> {
        if !self.ready {
            return Vec::new();
        }
        let (points, ok) = self.swarm.positions(moment);
        let hidden = core::hidden(eye, &points);
        points.into_iter().enumerate().filter(|(i, _)| ok.get(*i).copied().unwrap_or(false) && !hidden.get(*i).copied().unwrap_or(true)).collect()
    }

    /// Виток и след выбранного спутника на момент, пересчёт по
    /// `ORBIT_REFRESH`.
    pub fn path(&mut self, moment: f64) -> Option<&core::Path> {
        let number = self.selected.filter(|_| self.ready)?;
        let index = self.swarm.index_of(number)?;
        let key = (number, (moment / core::ORBIT_REFRESH).floor() as i64);
        if self.path.as_ref().is_none_or(|(k, _)| *k != key) {
            self.path = Some((key, core::path(self.swarm.sats(), index, moment)));
        }
        self.path.as_ref().and_then(|(_, p)| p.as_ref())
    }

    /// Выбрать спутник или снять выбор и камеру следом.
    pub fn select(&mut self, number: Option<i64>) {
        self.selected = number;
        if number.is_none() {
            self.follow = false;
        }
        self.path = None;
    }

    /// Название спутника по номеру NORAD.
    pub fn name_of(&self, number: i64) -> Option<String> {
        self.swarm.index_of(number).and_then(|i| self.swarm.names.get(i).cloned())
    }

    /// Поля окна «Объекты» спутника index на момент.
    pub fn values(&self, lang: Lang, index: usize, moment: f64) -> Option<(String, Vec<(String, String)>)> {
        let info = self.swarm.info(index, moment)?;
        let names = group_names(lang);
        let group = names.iter().find(|(k, _, _)| *k == info.group).map_or(info.group.clone(), |(_, n, _)| n.to_string());
        let values = vec![
            (tr(lang, "NORAD number").to_string(), info.number.to_string()),
            (tr(lang, "Group").to_string(), group),
            (tr(lang, "Height").to_string(), trf(lang, "{value} km", &[("value", &format!("{:.0}", info.height))])),
            (tr(lang, "Speed").to_string(), trf(lang, "{value} km/s", &[("value", &format!("{:.2}", info.speed))])),
            (tr(lang, "Elements age").to_string(), trf(lang, "{value} d", &[("value", &format!("{:.1}", info.age))])),
        ];
        Some((info.name, values))
    }
}
