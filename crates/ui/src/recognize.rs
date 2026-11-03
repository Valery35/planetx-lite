// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Распознавание» подменю «Анализ участка», перенос
//! `ui/recognize.py` модуля 1.38.
//!
//! Решение автора модуля от 4 ноября 2026 года - распознавание растёт
//! ступенями «точки - области - задания», «пользователя ни разу не грузим
//! лишним, максимально просто и компактно». Снимки окно подбирает
//! и загружает само по «Построить», когда участок, классы и образцы уже
//! заданы. Таблица «Снимки» - лето и март - апрель со снегом, без снега
//! классы по летнему. Растр программы с подписями каналов и днём нужного
//! месяца в имени берётся без загрузки. Классы - окно «Классификация»
//! ([`crate::aefclass::ClassCalc`]) без выбора источника: каналы летнего
//! снимка и снимка со снегом - признаки клетки.
//!
//! Образцы - точки или области. В областях щелчок по глобусу начинает
//! контур выбранного класса, щелчок по первой вершине замыкает его,
//! щелчок внутри готового контура убирает его. Образцы контура - клетки
//! внутри ([`data::rasterclass::ring_samples`]).
//!
//! Третья ступень, модуль 1.39 - список «Задача». Задача («Леса и рубки»,
//! «Поля», «Вода», [`TASKS`]) сама заводит свои классы и считает их
//! правилами «Выделения объектов» по тем же снимкам лета и снега
//! ([`data::objecttasks::cell_kinds`]), образцы не нужны. Образец точкой
//! или областью поправляет класс: «Построить» берёт до [`TASK_SAMPLES`]
//! клеток каждого класса правил образцами, добавляет образцы пользователя
//! и считает классы ближайшими образцами. Заросших полей в списке нет -
//! им нужен ряд снимков весны и осени за 4 года.
//!
//! Модуль проверяет, накрывает ли снимок проекта участок, до расчёта.
//! Рамки растра Lite до чтения файла не знает: покрытие называет строка
//! итога после расчёта.

use data::objecttasks::Scenario;
use eframe::egui;

use crate::aefclass::Raster;
use crate::i18n::{Lang, tr, trf};
use crate::objecttasks::raster_roles;
use crate::s2scenes::{self, Found, SceneFinder};

/// Наибольшая сторона участка, км: загрузка идёт по участку. Выбор
/// помощника модуля.
pub const SIDE_MOST: f64 = 20.0;
/// Дат в меню строки.
pub const CHOICES: usize = 12;
/// Снег - март и апрель, как у второго снимка «Выделения объектов».
pub const SNOW: [u32; 2] = [3, 4];
/// Сцен в ответе поиска - все подходящие.
const SHOWN: usize = 10_000;
/// Задачи списка «Задача».
pub const TASKS: [Scenario; 3] = [Scenario::Forest, Scenario::Fields, Scenario::Water];
/// Клеток класса правил образцами, если есть свои образцы, - выбор
/// помощника модуля 1.39.
pub const TASK_SAMPLES: usize = 60;

/// Роли каналов растра, `None` у растра без подписей.
pub type Roles = Vec<Option<&'static str>>;

/// Строка таблицы «Снимки».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Summer,
    Snow,
}

pub const KEYS: [Key; 2] = [Key::Summer, Key::Snow];

impl Key {
    /// Месяцы по порядку предпочтения.
    pub fn months(self) -> &'static [u32] {
        match self {
            Key::Summer => &s2scenes::SUMMER,
            Key::Snow => &SNOW,
        }
    }

    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Key::Summer => tr(lang, "Summer"),
            Key::Snow => tr(lang, "March - April, snow"),
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// «2025-07-21» - «21.07.2025».
pub fn day_text(day: &str) -> String {
    match (day.get(..4), day.get(5..7), day.get(8..10)) {
        (Some(y), Some(m), Some(d)) => format!("{d}.{m}.{y}"),
        _ => String::new(),
    }
}

/// Месяц растра по дню в имени файла.
fn raster_month(raster: &Raster) -> Option<u32> {
    raster_roles(raster).1.and_then(|d| d.get(5..7)?.parse().ok())
}

/// Растр с подписями каналов, где есть ближний ИК, `suitable_rasters`
/// модуля.
pub fn suitable(raster: &Raster) -> bool {
    raster_roles(raster).0.is_some_and(|roles| roles.contains(&Some("nir")))
}

/// Снимок строки: растр программы, облачность над участком, состояние
/// и участок, для которого окно его загрузило (None - растр программы).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Shot {
    pub raster: Option<String>,
    pub cloud: Option<f64>,
    pub state: Option<String>,
    pub ring: Option<Vec<(f64, f64)>>,
    /// Выбрано «Без снимка»: строка не ищется. У модуля «Построить» ищет
    /// её снова - отказ от снимка со снегом тогда не держится.
    pub skip: bool,
}

type Request<'a> = &'a dyn Fn(u64) -> Box<dyn net::Request>;

/// Подбор снимков в таблицу, `ShotPicker` модуля: один поиск на все
/// строки, лучший снимок в каждую пустую строку, загрузка по очереди.
pub struct Picker {
    pub shots: [Shot; 2],
    choices: [Vec<Found>; 2],
    queue: Vec<(Key, Found)>,
    loading: Option<(Key, f64)>,
    ring: Vec<(f64, f64)>,
    /// Участок последнего поиска.
    searched: Option<Vec<(f64, f64)>>,
    /// Поиск ждёт запроса в шаге: месяцы.
    want: Option<Vec<u32>>,
    finder: SceneFinder,
    clock: web_time::Instant,
    /// Подбор закончен: «Построить» идёт дальше само.
    pub done: bool,
    /// Новая строка состояния окна.
    pub status: Option<String>,
    /// Загруженные снимки: окно открывает их растрами программы.
    pub loaded: Vec<Raster>,
}

impl Default for Picker {
    fn default() -> Self {
        Self {
            shots: Default::default(),
            choices: Default::default(),
            queue: Vec::new(),
            loading: None,
            ring: Vec::new(),
            searched: None,
            want: None,
            finder: SceneFinder::default(),
            clock: web_time::Instant::now(),
            done: false,
            status: None,
            loaded: Vec::new(),
        }
    }
}

impl Picker {
    pub fn busy(&self) -> bool {
        self.finder.busy() || self.want.is_some() || !self.queue.is_empty() || self.loading.is_some()
    }

    pub fn stop(&mut self) {
        self.finder.stop();
        self.want = None;
        self.queue.clear();
        if let Some((key, _)) = self.loading.take() {
            self.shots[key.index()].state = None;
        }
    }

    pub fn raster(&self, key: Key) -> Option<&str> {
        self.shots[key.index()].raster.as_deref()
    }

    pub(crate) fn set(&mut self, key: Key, raster: Option<String>, cloud: Option<f64>, ring: Option<Vec<(f64, f64)>>) {
        self.shots[key.index()] = Shot { raster, cloud, state: None, ring, skip: false };
    }

    /// «Без снимка»: строка пуста и не ищется.
    pub fn skip(&mut self, key: Key) {
        self.set(key, None, None, None);
        self.shots[key.index()].skip = true;
    }

    fn missing(&self) -> Vec<Key> {
        KEYS.into_iter().filter(|k| self.raster(*k).is_none() && !self.shots[k.index()].skip).collect()
    }

    /// Снимки таблицы для участка ring, `fit` модуля: растр, которого
    /// больше нет, и снимок, загруженный для другого участка, уходят,
    /// пустые строки берут растр программы или ищутся. Поиск - один раз
    /// на участок. true - снимки уже есть, иначе подбор начат, конец -
    /// [`Picker::done`].
    pub fn fit(&mut self, lang: Lang, ring: &[(f64, f64)], rasters: &[Raster]) -> bool {
        for key in KEYS {
            let shot = &self.shots[key.index()];
            let gone = shot.raster.as_ref().is_some_and(|n| !rasters.iter().any(|r| r.0 == *n));
            let other = shot.ring.as_ref().is_some_and(|r| r.as_slice() != ring);
            if gone || other {
                let skip = shot.skip;
                self.set(key, None, None, None);
                self.shots[key.index()].skip = skip;
            }
        }
        self.take_open(ring, rasters);
        let missing = self.missing();
        if missing.is_empty() {
            return true;
        }
        if self.busy() && self.ring == ring {
            return false;
        }
        self.stop();
        self.ring = ring.to_vec();
        if self.searched.as_deref() == Some(ring) && self.choices.iter().any(|c| !c.is_empty()) {
            self.fill(lang);
            return false;
        }
        let mut months: Vec<u32> = missing.iter().flat_map(|k| k.months().iter().copied()).collect();
        months.sort_unstable();
        months.dedup();
        for key in &missing {
            self.shots[key.index()].state = Some(tr(lang, "search…").to_string());
        }
        self.want = Some(months);
        false
    }

    /// Растр программы с подписями каналов и днём нужного месяца - в пустую
    /// строку без загрузки, `_take_project` модуля.
    fn take_open(&mut self, ring: &[(f64, f64)], rasters: &[Raster]) {
        for key in KEYS {
            if self.raster(key).is_some() || self.shots[key.index()].skip {
                continue;
            }
            let used: Vec<String> = self.shots.iter().filter_map(|s| s.raster.clone()).collect();
            let found = rasters.iter().find(|r| !used.contains(&r.0) && raster_month(r).is_some_and(|m| key.months().contains(&m)) && suitable(r));
            if let Some(r) = found {
                // Снимок, загруженный окном для этого участка, остаётся
                // своим: другой участок его снимет.
                let own = self.loaded_ring(&r.0).filter(|own| own.as_slice() == ring);
                self.set(key, Some(r.0.clone()), None, own);
            }
        }
    }

    /// Участок, для которого окно загрузило растр name.
    fn loaded_ring(&self, name: &str) -> Option<Vec<(f64, f64)>> {
        self.shots.iter().find(|s| s.raster.as_deref() == Some(name)).and_then(|s| s.ring.clone())
    }

    /// Лучший снимок в каждую пустую строку, очередь загрузки.
    fn fill(&mut self, lang: Lang) {
        self.queue.clear();
        for key in self.missing() {
            match self.choices[key.index()].first() {
                Some(found) => self.queue.push((key, found.clone())),
                None => {
                    self.shots[key.index()].state = Some(tr(lang, if key == Key::Summer { "no clear image" } else { "none, classes by summer" }).to_string());
                }
            }
        }
        self.next(lang);
    }

    /// Следующий снимок очереди или конец подбора.
    fn next(&mut self, lang: Lang) {
        if self.queue.is_empty() {
            self.loading = None;
            if self.raster(Key::Summer).is_none() {
                self.status = Some(tr(lang, "There is no summer image of the area: Sentinel-2 scenes of three years have more than 5 % of clouds over the area. A click on the row «Summer» - an opened raster.").to_string());
            }
            self.done = true;
            return;
        }
        let (key, (scene, cloud, _)) = self.queue.remove(0);
        self.loading = Some((key, cloud));
        self.shots[key.index()].state = Some(tr(lang, "loading…").to_string());
        self.status = Some(trf(lang, "Loading the image «{row}» {day}…", &[("row", key.name(lang)), ("day", &day_text(s2scenes::day(&scene)))]));
        let ring = self.ring.clone();
        self.finder.load(lang, &scene, &ring, s2scenes::PRODUCT);
    }

    /// Шаг: поиск, ответы каталога, загрузка.
    pub fn step(&mut self, lang: Lang, request: Request) {
        if let Some(months) = self.want.take() {
            let ring = self.ring.clone();
            self.finder.search(lang, &ring, &months, s2scenes::YEARS, SHOWN, request);
        }
        let now = self.clock.elapsed().as_secs_f64();
        self.finder.step(lang, now, request);
        if let Some(text) = self.finder.status.take() {
            self.status = Some(text);
        }
        if let Some(found) = self.finder.found.take() {
            self.searched = Some(self.ring.clone());
            for key in KEYS {
                let mut rows: Vec<Found> = found.iter().filter(|f| f.0.when.get(5..7).and_then(|m| m.parse::<u32>().ok()).is_some_and(|m| key.months().contains(&m))).cloned().collect();
                s2scenes::order(&mut rows, key.months());
                rows.truncate(CHOICES);
                self.choices[key.index()] = rows;
            }
            self.fill(lang);
        }
        if let Some((name, bytes)) = self.finder.loaded.take() {
            if let Some((key, cloud)) = self.loading.take() {
                self.set(key, Some(name.clone()), Some(cloud), Some(self.ring.clone()));
            }
            self.loaded.push((name, std::sync::Arc::new(bytes)));
            self.next(lang);
        }
        if std::mem::take(&mut self.finder.failed) {
            if let Some((key, _)) = self.loading.take() {
                self.shots[key.index()].state = Some(tr(lang, "not loaded").to_string());
            }
            self.next(lang);
        }
    }

    /// Строки таблицы «Снимки»: что нужно, дата с облачностью, состояние.
    /// Щелчок по названию строки - другие даты, растр программы, «Без
    /// снимка».
    pub fn rows(&mut self, ui: &mut egui::Ui, lang: Lang, rasters: &[Raster]) {
        let tip = tr(lang, "Sentinel-2 images of the area are picked and loaded by themselves on «Build»: the clearest over the area in three years. An opened raster of the right month is taken without loading. A click on a row - other dates, an opened raster or «No image».");
        let busy = self.busy();
        let mut pick: Option<(Key, Option<Found>, Option<Option<String>>)> = None;
        egui::Grid::new("planetx-recognize-shots").num_columns(3).spacing([10.0, 2.0]).show(ui, |ui| {
            for key in KEYS {
                let shot = &self.shots[key.index()];
                let day = shot.raster.as_ref().and_then(|n| rasters.iter().find(|r| r.0 == *n)).and_then(|r| raster_roles(r).1).map(|d| day_text(&d)).unwrap_or_default();
                let mut date = if !day.is_empty() {
                    day
                } else if shot.raster.is_some() {
                    "✓".to_string()
                } else {
                    "-".to_string()
                };
                if let Some(cloud) = shot.cloud {
                    date = format!("{date}{}", trf(lang, ", clouds {cloud} %", &[("cloud", &format!("{cloud:.0}"))]));
                }
                let state = shot.state.clone().unwrap_or_else(|| {
                    tr(
                        lang,
                        if shot.raster.is_some() {
                            "ready"
                        } else if shot.skip {
                            "No image"
                        } else {
                            "picked on «Build»"
                        },
                    )
                    .to_string()
                });
                let name = shot.raster.clone().unwrap_or_default();
                ui.add_enabled_ui(!busy, |ui| {
                    ui.menu_button(key.name(lang), |ui| {
                        for found in &self.choices[key.index()] {
                            let text = trf(lang, "{day} - clouds over the area {cloud} %", &[("day", &day_text(s2scenes::day(&found.0))), ("cloud", &format!("{:.0}", found.1))]);
                            if ui.button(text).clicked() {
                                pick = Some((key, Some(found.clone()), None));
                                ui.close();
                            }
                        }
                        let open: Vec<&Raster> = rasters.iter().filter(|r| suitable(r)).collect();
                        ui.add_enabled_ui(!open.is_empty(), |ui| {
                            ui.menu_button(tr(lang, "Opened raster"), |ui| {
                                for r in open {
                                    if ui.button(&r.0).clicked() {
                                        pick = Some((key, None, Some(Some(r.0.clone()))));
                                        ui.close();
                                    }
                                }
                            });
                        });
                        ui.separator();
                        if ui.button(tr(lang, "No image")).clicked() {
                            pick = Some((key, None, None));
                            ui.close();
                        }
                    })
                    .response
                    .on_hover_text(tip);
                });
                ui.label(date).on_hover_text(name);
                ui.label(state);
                ui.end_row();
            }
        });
        match pick {
            Some((key, Some(found), _)) => {
                self.queue = vec![(key, found)];
                self.next(lang);
                self.done = false;
            }
            Some((key, None, Some(raster))) => self.set(key, raster, None, None),
            Some((key, None, None)) => self.skip(key),
            None => {}
        }
    }
}

/// Состояние окна «Распознавание» у окна «Классификация»: снимки, образцы
/// точками или областями, контур, который обводится.
#[derive(Default)]
pub struct Recognize {
    pub picker: Picker,
    /// Образцы - области, иначе точки.
    pub areas: bool,
    /// Контур образца, который обводится, и номер его класса.
    pub drawing: Vec<(f64, f64)>,
    pub drawing_class: usize,
    /// «Построить» ждёт подбора снимков.
    pub pending: bool,
    /// Задача, None - свои образцы.
    pub task: Option<Scenario>,
    /// Роли каналов летнего снимка и снимка со снегом расчёта задачи.
    pub roles: Option<(Roles, Option<Roles>)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn day_text_and_months() {
        assert_eq!(day_text("2025-07-21"), "21.07.2025");
        assert_eq!(day_text("2025"), "");
        assert_eq!(Key::Summer.months(), &[7, 6, 8]);
        assert_eq!(Key::Snow.months(), &[3, 4]);
    }
}
