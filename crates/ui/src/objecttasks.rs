// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Выделение объектов» подменю «Анализ участка», перенос
//! `ui/objecttasks.py` и `ui/seasons.py` модуля 1.33-1.36.
//!
//! Типовые задачи поверх трассировки ([`crate::aefclass::ClassCalc::objects`]).
//! Задача ставит настройки трассировки, после «Построить» цвета называются
//! и сливаются по правилам `data::objecttasks`, почва среди хвои
//! отделяется рубками. Правила работают только по снимку с известными
//! каналами - снимку Sentinel-2, который окно находит и загружает само
//! ([`crate::s2scenes`]), или растру с подписями каналов.
//!
//! Второй снимок другого сезона идёт в раскладку на цвета вместе с первым
//! и отделяет хвою от листвы. Флажок «Линейные объекты» выдаёт дороги,
//! реки и лесополосы осевыми линиями (`data::linear`). Задача «Заросшие
//! поля» берёт ряд снимков весны и осени за 4 года из таблицы сезонов
//! (`data::fallow`): снимки сезонов ложатся на сетку летнего, но на цвета
//! не раскладываются.

use std::collections::BTreeMap;

use data::aef::Mosaic;
use data::fallow::{self, Season};
use data::linear;
use data::objecttasks::{self as ot, Kind, Scenario};
use data::rasterclass;
use data::sentinel::Scene;
use eframe::egui;

use crate::aefclass::{BASEMAP, Joined, Raster, TraceBase};
use crate::i18n::{Lang, tr, trf};
use crate::s2scenes::{self, Found, SceneFinder};

/// Месяцы, когда хвоя и листва различаются лучше.
const SUMMER: std::ops::RangeInclusive<u32> = 5..=9;
/// Доля участка под снимком, ниже которой «Построить» отказывает. Нашлось
/// у автора модуля 3 ноября 2026 года - снимок прежнего участка накрывал
/// треть нового, классы легли на треть без единого слова.
pub const MIN_COVER: f64 = 0.9;
/// Строк списка снимков на виду.
const SCENE_ROWS: f32 = 110.0;
/// Порядок классов слоя заросших полей.
const FALLOW_ORDER: [Kind; 13] = [Kind::Tilled, Kind::FallowNew, Kind::FallowOld, Kind::Overgrown, Kind::Forest, Kind::Conifer, Kind::Broadleaf, Kind::Vegetation, Kind::Clearing, Kind::Soil, Kind::Water, Kind::Snow, Kind::Other];

/// Роли каналов растра.
pub type Roles = Vec<Option<&'static str>>;
/// Точка сетки (столбец, строка) в широту и долготу.
pub type ToDegrees<'a> = dyn Fn(&(f64, f64)) -> (f64, f64) + 'a;
/// Осевая линия в клетках сетки.
type Line = Vec<(f64, f64)>;
/// Клетки слоя на глобусе и цвета с названиями групп линий.
type LineLayer = (Vec<i32>, Vec<([u8; 3], String)>);

/// Название вида.
pub fn kind_name(lang: Lang, kind: Kind) -> &'static str {
    match kind {
        Kind::Water => tr(lang, "Water"),
        Kind::Conifer => tr(lang, "Coniferous forest"),
        Kind::Broadleaf => tr(lang, "Broadleaf forest and grass"),
        Kind::Vegetation => tr(lang, "Vegetation"),
        Kind::Soil => tr(lang, "Open soil"),
        Kind::Clearing => tr(lang, "Clearcuts and burns"),
        Kind::Snow => tr(lang, "Snow and ice"),
        Kind::Other => tr(lang, "Other"),
        Kind::Tilled => tr(lang, "Ploughland, tilled"),
        Kind::FallowNew => tr(lang, "Abandoned 1-2 years"),
        Kind::FallowOld => tr(lang, "Fallow or meadow"),
        Kind::Overgrown => tr(lang, "Overgrowing with trees"),
        Kind::Forest => tr(lang, "Forest"),
    }
}

/// Название группы линий.
pub fn line_name(lang: Lang, group: &str) -> &'static str {
    match group {
        "roads" => tr(lang, "Roads and cutlines"),
        "water" => tr(lang, "Rivers and canals"),
        _ => tr(lang, "Shelterbelts and cutlines"),
    }
}

/// Название и подсказка задачи.
pub fn scenario_text(lang: Lang, s: Scenario) -> (&'static str, &'static str) {
    match s {
        Scenario::Forest => (tr(lang, "Forests and clearcuts"), tr(lang, "Coniferous forest, broadleaf forest and grass, clearcuts and burns, open soil, water. A clearcut is open soil with at least a quarter of coniferous forest in a band of three cells around it.")),
        Scenario::Fields => (tr(lang, "Fields"), tr(lang, "Vegetation, open soil and ploughland, clearcuts, water. Fields stay separate polygons, the smallest area is larger.")),
        Scenario::Water => (tr(lang, "Water"), tr(lang, "Water and everything else - land.")),
        Scenario::Fallow => (tr(lang, "Overgrown fields"), tr(lang, "Tilled fields, abandoned for 1-2 years, fallow and meadows, overgrowing with trees, forest. Spring and autumn images of 4 years show the year when the field was bare soil for the last time. The field borders are by the summer image.")),
    }
}

/// Название сезона.
fn season_name(lang: Lang, s: Season) -> &'static str {
    match s {
        Season::Spring => tr(lang, "Spring"),
        Season::Autumn => tr(lang, "Autumn"),
    }
}

/// Сезон месяца.
fn season_of(month: u32) -> Option<Season> {
    if fallow::SPRING.contains(&month) {
        Some(Season::Spring)
    } else if fallow::AUTUMN.contains(&month) {
        Some(Season::Autumn)
    } else {
        None
    }
}

/// Имя файла без расширения.
fn stem(name: &str) -> &str {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    base.rsplit_once('.').map_or(base, |(s, _)| s)
}

/// Роли каналов и день съёмки растра по подписям каналов и имени файла.
pub fn raster_roles(raster: &Raster) -> (Option<Roles>, Option<String>) {
    let labels = rasterclass::quick_labels(&raster.1);
    (ot::channels(&labels, stem(&raster.0)), ot::day_of(stem(&raster.0)))
}

/// Количество каналов растра.
fn band_count(raster: &Raster) -> usize {
    rasterclass::feature_bands(&raster.1).unwrap_or(0)
}

/// Куда встаёт загруженный снимок.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Target {
    Source,
    Second,
    Season((i32, Season)),
}

/// Таблица сезонов: годы ряда, снимки клеток, подбор.
#[derive(Default)]
struct Seasons {
    /// Годы - новые сверху.
    years: Vec<i32>,
    /// Клетка (год, сезон) - имя растра.
    cells: BTreeMap<(i32, Season), String>,
    /// Клетка, что грузится.
    loading: Option<(i32, Season)>,
    /// Подходящие снимки клеток, лучший первым.
    choices: Vec<((i32, Season), Vec<Found>)>,
    /// Очередь загрузки.
    queue: Vec<((i32, Season), Scene)>,
    /// Подбор идёт: поиск или очередь.
    picking: bool,
}

impl Seasons {
    fn keys(&self) -> Vec<(i32, Season)> {
        self.years.iter().flat_map(|y| [(*y, Season::Spring), (*y, Season::Autumn)]).collect()
    }
}

/// Области заросших полей: виды, последний год голой почвы, доля
/// деревьев.
struct Fields {
    kinds: Vec<Kind>,
    last: Vec<i32>,
    woody: Option<Vec<f64>>,
}

/// Расчёт задачи: каналы снимков и итог правил.
#[derive(Default)]
struct Run {
    roles: Roles,
    month: Option<u32>,
    /// Роли каналов второго снимка и его имя.
    more: Option<(Roles, String)>,
    /// Каналов летнего снимка в сетке заросших полей.
    split: Option<usize>,
    /// Снимки ряда: год, роли, каналов.
    seasons: Vec<(i32, Roles, usize)>,
    notes: Vec<String>,
    rules: Option<Vec<String>>,
    clearing: Vec<usize>,
    lines: Vec<(&'static str, Vec<Line>)>,
    fields: Option<Fields>,
}

/// Задача окна «Выделение объектов».
pub struct Task {
    pub scenario: Scenario,
    /// Второй снимок, None - «Нет».
    pub second: Option<String>,
    pub lines: bool,
    pub line_width: f64,
    /// Тонкие настройки на виду.
    pub more: bool,
    /// Задачу выбирает список «Задача» окна «Распознавание», свой список
    /// задачи не показывается.
    pub fixed: bool,
    finder: SceneFinder,
    target: Target,
    scenes: Vec<Found>,
    seasons: Seasons,
    /// Нажатая кнопка поиска: ждёт участка и запроса в шаге.
    want: Option<Target>,
    want_seasons: bool,
    /// Двойной щелчок по снимку списка.
    want_load: Option<Scene>,
    clock: web_time::Instant,
    /// Загруженные снимки: окно глобуса открывает их растрами программы.
    pub loaded: Vec<Raster>,
    /// Снимок, который встаёт источником.
    pub take_source: Option<String>,
    /// Новая строка состояния окна.
    pub status: Option<String>,
    run: Run,
}

impl Default for Task {
    fn default() -> Self {
        let today = s2scenes::today();
        Self {
            scenario: Scenario::Forest,
            second: None,
            lines: false,
            line_width: linear::WIDTH,
            more: false,
            fixed: false,
            finder: SceneFinder::default(),
            target: Target::Source,
            scenes: Vec::new(),
            seasons: Seasons { years: (0..fallow::YEARS).map(|n| today.0 as i32 - n).collect(), ..Default::default() },
            want: None,
            want_seasons: false,
            want_load: None,
            clock: web_time::Instant::now(),
            loaded: Vec::new(),
            take_source: None,
            status: None,
            run: Run::default(),
        }
    }
}

impl Task {
    pub fn busy(&self) -> bool {
        self.finder.busy() || self.seasons.picking
    }

    pub fn stop(&mut self) {
        self.finder.stop();
        self.seasons.picking = false;
        self.seasons.queue.clear();
        self.seasons.loading = None;
    }

    /// Название задачи.
    pub fn title(&self, lang: Lang) -> &'static str {
        scenario_text(lang, self.scenario).0
    }

    /// Названия цветов по правилам - для помощника.
    pub fn rules(&self) -> Option<&[String]> {
        self.run.rules.as_deref()
    }

    /// Роли каналов сетки для помощника: каналы второго снимка - «nir@2».
    pub fn grid_roles(&self) -> Vec<Option<String>> {
        let mut out: Vec<Option<String>> = self.run.roles.iter().map(|r| r.map(str::to_string)).collect();
        if let Some((more, _)) = &self.run.more {
            out.extend(more.iter().map(|r| r.map(|r| format!("{r}@2"))));
        }
        out
    }

    pub fn month(&self) -> Option<u32> {
        self.run.month
    }

    /// Каналов сетки, которые раскладываются на цвета, None - все.
    pub fn split(&self) -> Option<usize> {
        self.run.split
    }

    /// Замечания правил к строке итога.
    pub fn notes(&self) -> String {
        self.run.notes.join(" ")
    }

    /// Подходящие для правил растры: ближний ИК и, если нужно, красный
    /// или SWIR.
    fn suitable(rasters: &[Raster], need_red_or_swir: bool) -> Vec<String> {
        rasters
            .iter()
            .filter(|r| {
                let (roles, _) = raster_roles(r);
                roles.is_some_and(|roles| roles.contains(&Some("nir")) && (!need_red_or_swir || ot::usable(&roles)))
            })
            .map(|r| r.0.clone())
            .collect()
    }

    /// Строки окна над полями трассировки: задача, снимок, поиск, второй
    /// снимок или таблица сезонов. Итог - новая задача: окно ставит её
    /// настройки трассировки.
    pub fn rows(&mut self, ui: &mut egui::Ui, lang: Lang, rasters: &[Raster], source: &mut Option<String>) -> Option<Scenario> {
        let mut changed = None;
        if !self.fixed {
            ui.label(tr(lang, "Task"));
            let (title, _) = scenario_text(lang, self.scenario);
            egui::ComboBox::from_id_salt("planetx-objects-task")
                .selected_text(title)
                .show_ui(ui, |ui| {
                    for s in ot::SCENARIOS {
                        let (name, tip) = scenario_text(lang, s);
                        if ui.selectable_value(&mut self.scenario, s, name).on_hover_text(tip).clicked() {
                            changed = Some(s);
                        }
                    }
                })
                .response
                .on_hover_text(tr(lang, "A typical task sets the number of colours and the smallest area and after «Build» names the classes by the reflectance of the image."));
            ui.end_row();
            ui.label("");
            ui.add(egui::Label::new(egui::RichText::new(scenario_text(lang, self.scenario).1).small()).wrap());
            ui.end_row();
        }
        // Источник - только подходящие снимки.
        let suitable = Self::suitable(rasters, true);
        if source.as_deref().is_none_or(|s| s == BASEMAP || !suitable.iter().any(|n| n == s)) {
            *source = suitable.last().cloned();
        }
        ui.label(tr(lang, "Image"));
        let shown = source.clone().unwrap_or_else(|| tr(lang, "No suitable image is open").to_string());
        egui::ComboBox::from_id_salt("planetx-objects-source")
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for name in &suitable {
                    ui.selectable_value(source, Some(name.clone()), name);
                }
            })
            .response
            .on_hover_text(tr(lang, "A Sentinel-2 or Landsat image with labelled bands. The list has only images in which the window recognised the near infrared and the red or SWIR by the labels."));
        ui.end_row();
        ui.label("");
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(egui::RichText::new(self.note(lang, rasters, source.as_deref())).small()).wrap());
        });
        ui.end_row();
        ui.label("");
        if ui
            .add_enabled(!self.busy(), egui::Button::new(tr(lang, "Find an image")))
            .on_hover_text(tr(lang, "Summer Sentinel-2 images of the area for three years, cloud cover over the area up to 5 %. A double click on an image loads the composite «Agriculture» (B11 B8 B2) and sets it as the source."))
            .clicked()
        {
            self.want = Some(Target::Source);
        }
        ui.end_row();
        if !self.scenes.is_empty() {
            ui.label("");
            egui::ScrollArea::vertical().id_salt("planetx-objects-scenes").max_height(SCENE_ROWS).show(ui, |ui| {
                for (scene, cloud, valid) in &self.scenes {
                    let text = trf(lang, "{day} - cloud cover over the area {cloud} %, data {valid} %", &[("day", s2scenes::day(scene)), ("cloud", &format!("{cloud:.0}")), ("valid", &format!("{valid:.0}"))]);
                    if ui.add(egui::Label::new(text).sense(egui::Sense::click())).on_hover_text(tr(lang, "A double click loads the image.")).double_clicked() {
                        self.want_load = Some(scene.clone());
                    }
                }
            });
            ui.end_row();
        }
        if self.scenario == Scenario::Fallow {
            self.season_rows(ui, lang, rasters);
        } else {
            self.second_row(ui, lang, rasters, source.as_deref());
        }
        changed
    }

    /// Строка «Второй снимок».
    fn second_row(&mut self, ui: &mut egui::Ui, lang: Lang, rasters: &[Raster], source: Option<&str>) {
        let list: Vec<String> = Self::suitable(rasters, false).into_iter().filter(|n| Some(n.as_str()) != source).collect();
        if self.second.as_ref().is_some_and(|s| !list.contains(s)) {
            self.second = None;
        }
        ui.label(tr(lang, "Second image"));
        ui.horizontal(|ui| {
            let none = tr(lang, "None").to_string();
            egui::ComboBox::from_id_salt("planetx-objects-second")
                .selected_text(self.second.clone().unwrap_or(none.clone()))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.second, None, none);
                    for name in &list {
                        ui.selectable_value(&mut self.second, Some(name.clone()), name);
                    }
                })
                .response
                .on_hover_text(tr(lang, "An image of the same place in another season, better with snow. Its bands go to the same grid of the area and are split into colours together with the first. On snow coniferous forest is what stayed dark, without snow - what hardly changed in the near infrared."));
            if ui
                .add_enabled(!self.busy(), egui::Button::new(tr(lang, "Find")))
                .on_hover_text(tr(lang, "Sentinel-2 images of the area in March, April and October for three years. On snow coniferous forest is dark, bare broadleaf forest and fields are light. In October without snow broadleaf forest loses the near infrared, coniferous does not. A double click loads the image and sets it as the second."))
                .clicked()
            {
                self.want = Some(Target::Second);
            }
        });
        ui.end_row();
    }

    /// Строки «Весна и осень»: подбор и таблица сезонов.
    fn season_rows(&mut self, ui: &mut egui::Ui, lang: Lang, rasters: &[Raster]) {
        self.take_project(rasters);
        let keys = self.seasons.keys();
        let chosen = keys.iter().filter(|k| self.seasons.cells.contains_key(k)).count();
        ui.label(tr(lang, "Spring and autumn"));
        ui.horizontal(|ui| {
            ui.label(trf(lang, "Images {count} of {total}", &[("count", &chosen.to_string()), ("total", &keys.len().to_string())]));
            if ui
                .add_enabled(!self.busy(), egui::Button::new(tr(lang, "Pick images")))
                .on_hover_text(tr(lang, "Searches for Sentinel-2 images of the area in spring and autumn for 4 years, takes the clearest into each empty cell and loads it. Images that are already open are not loaded again."))
                .clicked()
            {
                self.want_seasons = true;
            }
        });
        ui.end_row();
        ui.label("");
        let mut take: Option<((i32, Season), Option<Scene>)> = None;
        egui::Grid::new("planetx-objects-seasons").num_columns(3).spacing([12.0, 2.0]).show(ui, |ui| {
            ui.label("");
            ui.label(season_name(lang, Season::Spring));
            ui.label(season_name(lang, Season::Autumn));
            ui.end_row();
            for year in self.seasons.years.clone() {
                ui.label(year.to_string());
                for season in [Season::Spring, Season::Autumn] {
                    let key = (year, season);
                    let text = if self.seasons.loading == Some(key) {
                        tr(lang, "loading…").to_string()
                    } else if let Some(name) = self.seasons.cells.get(&key) {
                        ot::day_of(stem(name)).map_or_else(|| "✓".to_string(), |d| format!("{}.{}", &d[8..10], &d[5..7]))
                    } else {
                        "-".to_string()
                    };
                    let tip = self.seasons.cells.get(&key).cloned().unwrap_or_else(|| tr(lang, "No image").to_string());
                    ui.menu_button(text, |ui| {
                        let list = self.seasons.choices.iter().find(|(k, _)| *k == key).map(|(_, l)| l.clone()).unwrap_or_default();
                        for (scene, cloud, _) in list.iter().take(12) {
                            if ui.button(trf(lang, "{day} - cloud cover over the area {cloud} %", &[("day", s2scenes::day(scene)), ("cloud", &format!("{cloud:.0}"))])).clicked() {
                                take = Some((key, Some(scene.clone())));
                                ui.close();
                            }
                        }
                        if self.seasons.choices.is_empty() {
                            ui.add_enabled(false, egui::Button::new(tr(lang, "First «Pick images»")));
                        }
                        ui.separator();
                        if ui.button(tr(lang, "No image")).clicked() {
                            take = Some((key, None));
                            ui.close();
                        }
                    })
                    .response
                    .on_hover_text(format!("{tip}\n{}", tr(lang, "Images of spring (April - May) and autumn (September - October) by years. Ploughland at this time is bare soil, fallow is old grass and shrubs. A click on a cell - other dates of the season.")));
                }
                ui.end_row();
            }
        });
        ui.end_row();
        if let Some((key, scene)) = take
            && !self.busy()
        {
            match scene {
                Some(scene) => {
                    self.seasons.queue = vec![(key, scene)];
                    self.seasons.picking = true;
                }
                None => {
                    self.seasons.cells.remove(&key);
                }
            }
        }
    }

    /// Пустые клетки таблицы берут открытые снимки с красным и ближним ИК
    /// по дню в имени файла: что уже загружено, заново не грузится.
    fn take_project(&mut self, rasters: &[Raster]) {
        self.seasons.cells.retain(|_, name| rasters.iter().any(|r| r.0 == *name));
        for raster in rasters {
            let (roles, day) = raster_roles(raster);
            let (Some(roles), Some(day)) = (roles, day) else {
                continue;
            };
            if !roles.contains(&Some("red")) || !roles.contains(&Some("nir")) {
                continue;
            }
            let (Ok(year), Some(season)) = (day[..4].parse::<i32>(), day.get(5..7).and_then(|m| m.parse().ok()).and_then(season_of)) else {
                continue;
            };
            if self.seasons.years.contains(&year) {
                self.seasons.cells.entry((year, season)).or_insert_with(|| raster.0.clone());
            }
        }
    }

    /// Строка под снимком: подходит ли он и что окно узнало.
    fn note(&self, lang: Lang, rasters: &[Raster], source: Option<&str>) -> String {
        let Some(raster) = source.and_then(|s| rasters.iter().find(|r| r.0 == s)) else {
            return tr(lang, "No open image has the near infrared and the red or SWIR. «Find an image» finds a summer Sentinel-2 image of the area.").to_string();
        };
        let (roles, day) = raster_roles(raster);
        let names: Vec<String> = ot::role_names(&roles.unwrap_or_default()).iter().map(|r| data::classnames::role_name(r, lang == Lang::RU)).collect();
        let mut text = trf(lang, "Suitable. Bands - {names}.", &[("names", &names.join(", "))]);
        if let Some(day) = day {
            text = format!("{text} {}", trf(lang, "Image of {day}.", &[("day", &day)]));
            if day.get(5..7).and_then(|m| m.parse::<u32>().ok()).is_some_and(|m| !SUMMER.contains(&m)) {
                text = format!("{text} {}", tr(lang, "The image is not a summer one, coniferous and broadleaf forest differ worse."));
            }
        }
        text
    }

    /// Нижние строки: линейные объекты, ширина, «Дополнительно».
    pub fn bottom_rows(&mut self, ui: &mut egui::Ui, lang: Lang) {
        ui.label("");
        ui.checkbox(&mut self.lines, tr(lang, "Linear objects")).on_hover_text(tr(lang, "Roads and cutlines, rivers and canals, shelterbelts - as centre lines, a separate folder. A line is a narrow part of soil, water or vegetation, a connected piece of a line is not shorter than 200 m."));
        ui.end_row();
        if self.more && self.lines {
            ui.label(tr(lang, "Line width up to"));
            ui.add(egui::DragValue::new(&mut self.line_width).range(5.0..=500.0).suffix(tr(lang, " m"))).on_hover_text(tr(lang, "An object narrower than this is a line. The width is at least two pixels of the grid, otherwise the line breaks."));
            ui.end_row();
        }
        ui.label("");
        ui.checkbox(&mut self.more, tr(lang, "More")).on_hover_text(tr(lang, "Fine settings: pixel, colours, smoothing, smallest area, corner rounding, line width. The task sets them itself."));
        ui.end_row();
    }

    /// Шаг: кнопки поиска, ответы каталога, загрузка, очередь сезонов.
    pub fn step(&mut self, lang: Lang, ring: &[(f64, f64)], problem: Option<String>, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        let now = self.clock.elapsed().as_secs_f64();
        if let Some(target) = self.want.take() {
            match &problem {
                Some(p) => self.status = Some(p.clone()),
                None => {
                    self.target = target;
                    self.scenes.clear();
                    let months: &[u32] = if target == Target::Source { &s2scenes::SUMMER } else { &s2scenes::OFF_SEASON };
                    self.finder.search(lang, ring, months, s2scenes::YEARS, s2scenes::SHOWN, request);
                }
            }
        }
        if std::mem::take(&mut self.want_seasons) {
            match &problem {
                Some(p) => self.status = Some(p.clone()),
                None => {
                    self.seasons.queue.clear();
                    self.seasons.picking = true;
                    self.target = Target::Season((0, Season::Spring));
                    let months: Vec<u32> = fallow::SPRING.iter().chain(&fallow::AUTUMN).copied().collect();
                    self.finder.search(lang, ring, &months, fallow::YEARS, usize::MAX, request);
                }
            }
        }
        if let Some(scene) = self.want_load.take()
            && !self.finder.busy()
        {
            self.finder.load(lang, &scene, ring, s2scenes::PRODUCT);
        }
        // Следующий снимок очереди сезонов.
        if self.seasons.picking && !self.finder.busy() && self.seasons.loading.is_none() && self.finder.found.is_none() && self.finder.loaded.is_none() {
            if self.seasons.queue.is_empty() {
                if !self.seasons.choices.is_empty() || matches!(self.target, Target::Season(_)) {
                    self.seasons.picking = false;
                    self.report_seasons(lang);
                }
            } else {
                let (key, scene) = self.seasons.queue.remove(0);
                self.seasons.loading = Some(key);
                self.target = Target::Season(key);
                self.status = Some(trf(lang, "Loading the image of {season} {year}: {day}…", &[("season", &season_name(lang, key.1).to_lowercase()), ("year", &key.0.to_string()), ("day", s2scenes::day(&scene))]));
                self.finder.load(lang, &scene, ring, s2scenes::SEASON_PRODUCT);
            }
        }
        self.finder.step(lang, now, request);
        if let Some(text) = self.finder.status.take() {
            self.status = Some(text);
        }
        if let Some(found) = self.finder.found.take() {
            if let Target::Season(_) = self.target {
                let rows: Vec<(String, f64, usize)> = found.iter().enumerate().map(|(n, f)| (s2scenes::day(&f.0).to_string(), f.1, n)).collect();
                self.seasons.choices = fallow::pick(&rows).into_iter().map(|(k, ids)| (k, ids.into_iter().filter_map(|i| found.get(i).cloned()).collect())).collect();
                let keys = self.seasons.keys();
                self.seasons.queue = keys.iter().filter(|k| !self.seasons.cells.contains_key(k)).filter_map(|k| self.seasons.choices.iter().find(|(c, _)| c == k).and_then(|(_, l)| l.first()).map(|f| (*k, f.0.clone()))).collect();
                if self.seasons.queue.is_empty() {
                    self.seasons.picking = false;
                    self.report_seasons(lang);
                }
            } else {
                self.scenes = found;
            }
        }
        if let Some((name, bytes)) = self.finder.loaded.take() {
            let raster: Raster = (name.clone(), std::sync::Arc::new(bytes));
            self.loaded.push(raster);
            match self.target {
                Target::Source => {
                    self.take_source = Some(name.clone());
                    self.status = Some(trf(lang, "The image is loaded and set as the source: {name}.", &[("name", &name)]));
                    self.scenes.clear();
                }
                Target::Second => {
                    self.second = Some(name.clone());
                    self.status = Some(trf(lang, "The second image is loaded: {name}.", &[("name", &name)]));
                    self.scenes.clear();
                }
                Target::Season(key) => {
                    self.seasons.cells.insert(key, name);
                    self.seasons.loading = None;
                }
            }
        }
        if std::mem::take(&mut self.finder.failed) {
            self.seasons.loading = None;
        }
    }

    fn report_seasons(&mut self, lang: Lang) {
        let keys = self.seasons.keys();
        let count = keys.iter().filter(|k| self.seasons.cells.contains_key(k)).count();
        self.status = Some(if count > 0 {
            trf(lang, "Season images {count} of {total}. Empty cells are seasons without a clear image over the area, the computation goes without them.", &[("count", &count.to_string()), ("total", &keys.len().to_string())])
        } else {
            tr(lang, "No clear spring and autumn images were found over the area.").to_string()
        });
    }

    /// Проверка перед «Построить»: снимок с нужными каналами, второй снимок
    /// или снимки сезонов. Итог - имена растров, чьи каналы ложатся на
    /// сетку за каналами снимка, или текст отказа.
    pub fn before(&mut self, lang: Lang, rasters: &[Raster], source: Option<&str>) -> Result<Vec<String>, String> {
        let Some(raster) = source.filter(|s| *s != BASEMAP).and_then(|s| rasters.iter().find(|r| r.0 == s)) else {
            return Err(tr(lang, "No image. The «Find an image» button finds and loads a summer Sentinel-2 image of the area.").to_string());
        };
        let (roles, day) = raster_roles(raster);
        let Some(roles) = roles.filter(|r| ot::usable(r)) else {
            return Err(tr(lang, "The near infrared and the red or SWIR of the raster are not recognised - an image with labelled bands is needed.").to_string());
        };
        self.run = Run { month: day.as_deref().and_then(|d| d.get(5..7)?.parse().ok()), ..Default::default() };
        let mut extras = Vec::new();
        if self.scenario == Scenario::Fallow {
            self.run.split = Some(roles.len());
            let keys = self.seasons.keys();
            for key in keys {
                let Some(name) = self.seasons.cells.get(&key) else {
                    continue;
                };
                let Some(other) = rasters.iter().find(|r| r.0 == *name && r.0 != raster.0) else {
                    continue;
                };
                let (Some(r), _) = raster_roles(other) else {
                    continue;
                };
                if r.contains(&Some("red")) && r.contains(&Some("nir")) {
                    self.run.seasons.push((key.0, r, band_count(other)));
                    extras.push(other.0.clone());
                }
            }
            if extras.is_empty() {
                return Err(tr(lang, "No spring and autumn images. «Pick images» finds and loads them for the area.").to_string());
            }
        } else if let Some(name) = &self.second {
            if *name == raster.0 {
                return Err(tr(lang, "The second image is the same as the first.").to_string());
            }
            let Some(other) = rasters.iter().find(|r| r.0 == *name) else {
                return Err(tr(lang, "The second image is no longer open.").to_string());
            };
            let Some(more) = raster_roles(other).0.filter(|r| r.contains(&Some("nir"))) else {
                return Err(tr(lang, "The near infrared of the second image is not recognised - an image with labelled bands is needed.").to_string());
            };
            self.run.more = Some((more, name.clone()));
            extras.push(name.clone());
        }
        self.run.roles = roles;
        Ok(extras)
    }

    /// Отказ, если снимок накрывает меньше [`MIN_COVER`] участка. shares -
    /// доли участка с данными снимка и каналов за ним.
    pub fn short_cover(&self, lang: Lang, shares: &[f64]) -> Option<String> {
        let checked = if self.scenario == Scenario::Fallow { 1 } else { shares.len() };
        shares.iter().take(checked).enumerate().find(|(_, s)| **s < MIN_COVER).map(|(n, s)| {
            let which = if n == 0 { tr(lang, "The image") } else { tr(lang, "The second image") };
            trf(lang, "{which} covers only {share} % of the area. «Find an image» loads an image of the whole area, or outline the area inside the image.", &[("which", which), ("share", &format!("{:.0}", 100.0 * s))])
        })
    }

    /// Названия цветов по видам задачи, цвета одного вида сливаются
    /// с самым крупным из них, области почвы среди хвои - рубки, `prepare`
    /// модуля. Итог - слияние цветов для таблицы классов.
    pub fn prepare(&mut self, lang: Lang, base: &mut TraceBase, grid: &Mosaic<f32>, task_title: &str) -> Vec<i32> {
        let count = base.colors.len();
        let centers = base.done.centers.clone();
        self.run.clearing.clear();
        self.run.notes.clear();
        self.run.rules = None;
        self.run.lines.clear();
        self.run.fields = None;
        let highest = centers.iter().flatten().copied().fold(f64::NEG_INFINITY, f64::max);
        if count == 0 || highest > ot::REFLECTANCE_MAX {
            self.run.notes.push(tr(lang, "The raster values do not look like reflectance, the classes are not named.").to_string());
            return vec![-1; count];
        }
        let (rows, cols) = (base.rows, base.cols);
        if self.scenario == Scenario::Fallow {
            self.fallow(lang, base, grid, task_title);
            return vec![-1; count];
        }
        let first = self.run.roles.len();
        let summer: Vec<Vec<f64>> = centers.iter().map(|c| c.iter().take(first).copied().collect()).collect();
        let mut raw = ot::color_kinds(&summer, &self.run.roles);
        if let Some((more, _)) = &self.run.more {
            raw = ot::two_dates(&raw, &centers, &self.run.roles, more);
        }
        let kinds = ot::scenario_kinds(&raw, self.scenario);
        let regions = ot::region_kinds(&base.done.parts, rows, cols, &base.done.kinds, &raw, self.scenario);
        self.run.clearing = regions.iter().enumerate().filter(|(_, k)| **k == Kind::Clearing).map(|(n, _)| n).collect();
        let mut cells = vec![0usize; count];
        for c in &base.done.classes {
            if let Some(n) = usize::try_from(*c).ok().filter(|n| *n < count) {
                cells[n] += 1;
            }
        }
        let mut order: Vec<usize> = (0..count).collect();
        order.sort_by(|a, b| cells[*b].cmp(&cells[*a]));
        let mut target = vec![-1i32; count];
        let mut first_of: Vec<(Kind, usize)> = Vec::new();
        for n in order {
            match first_of.iter().find(|(k, _)| *k == kinds[n]) {
                Some((_, m)) => target[n] = *m as i32,
                None => first_of.push((kinds[n], n)),
            }
        }
        base.names = kinds.iter().map(|k| kind_name(lang, *k).to_string()).collect();
        self.run.rules = Some(base.names.clone());
        base.colors = kinds.iter().map(|k| k.color()).collect();
        base.hexes = base.colors.iter().map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}")).collect();
        let mut settings = vec![trf(lang, "task «{task}»", &[("task", task_title)])];
        if let Some((_, name)) = &self.run.more {
            settings.push(trf(lang, "second image «{name}»", &[("name", name)]));
        }
        settings.append(&mut base.settings);
        base.settings = settings;
        if let Some((more, _)) = &self.run.more {
            self.run.notes.push(if ot::winter(&centers, &self.run.roles, more).is_some() { tr(lang, "Coniferous forest is told from broadleaf by the snow of the second image.") } else { tr(lang, "Coniferous forest is told from broadleaf by two images.") }.to_string());
        } else if self.run.month.is_some_and(|m| !SUMMER.contains(&m)) && self.scenario.spec().kinds.contains(&Kind::Conifer) {
            self.run.notes.push(tr(lang, "The image is not a summer one, coniferous and broadleaf forest differ worse.").to_string());
        }
        if !self.run.clearing.is_empty() {
            let clearing = &self.run.clearing;
            let cells = base.done.parts.iter().filter(|p| usize::try_from(**p).is_ok_and(|p| clearing.contains(&p))).count();
            self.run.notes.push(trf(lang, "Clearcuts and burns - {area} ha.", &[("area", &format!("{:.0}", cells as f64 * base.step * base.step / 1e4))]));
        }
        if self.lines {
            self.find_lines(lang, base, &regions);
        }
        target
    }

    /// Заросшие поля: летние виды областей, по снимкам весны и осени - год
    /// последней голой почвы каждой области, `_fallow` модуля.
    fn fallow(&mut self, lang: Lang, base: &mut TraceBase, grid: &Mosaic<f32>, task_title: &str) {
        let (rows, cols) = (base.rows, base.cols);
        let raw = ot::color_kinds(&base.done.centers, &self.run.roles);
        let summer = ot::region_kinds(&base.done.parts, rows, cols, &base.done.kinds, &raw, Scenario::Fallow);
        let depth = grid.depth.max(1);
        let column = |band: usize| -> Vec<f64> { grid.values.chunks(depth).map(|cell| cell.get(band).map_or(f64::NAN, |v| f64::from(*v))).collect() };
        let parts = &base.done.parts;
        let count = summer.len();
        let mut at = self.run.split.unwrap_or(0);
        let mut scenes: Vec<(i32, Vec<f64>)> = Vec::new();
        let mut stacks: Vec<(Vec<Vec<f64>>, Roles)> = Vec::new();
        for (year, roles, bands) in &self.run.seasons {
            let values: Vec<Vec<f64>> = (at..at + bands).map(column).collect();
            at += bands;
            if let Some(cells) = fallow::bare_cells(&values, roles)
                && cells.iter().any(|v| v.is_finite())
            {
                scenes.push((*year, cells));
            }
            stacks.push((values, roles.clone()));
        }
        let refs: Vec<fallow::Scene> = stacks.iter().map(|(v, r)| (v.as_slice(), r.as_slice())).collect();
        let summer_bands: Vec<Vec<f64>> = (0..self.run.split.unwrap_or(0)).map(column).collect();
        let trees = fallow::season_woody(&refs).or_else(|| fallow::woody_cells(&summer_bands, &self.run.roles));
        let woody = fallow::woody_shares(parts, trees.as_deref(), count);
        let years = fallow::tilled_years(parts, &scenes, count);
        let newest = scenes.iter().map(|(y, _)| *y).max().unwrap_or(0);
        let (kinds, last) = fallow::field_kinds(&summer, &years, woody.as_deref(), newest);
        self.run.fields = Some(Fields { kinds, last, woody });
        let mut settings = vec![trf(lang, "task «{task}»", &[("task", task_title)]), trf(lang, "spring and autumn images {count}", &[("count", &scenes.len().to_string())])];
        settings.append(&mut base.settings);
        base.settings = settings;
        let mut span: Vec<i32> = scenes.iter().map(|(y, _)| *y).collect();
        span.sort_unstable();
        match (span.first(), span.last()) {
            (Some(a), Some(b)) => self.run.notes.push(trf(lang, "Spring and autumn images {count}, years {first}-{last}.", &[("count", &scenes.len().to_string()), ("first", &a.to_string()), ("last", &b.to_string())])),
            _ => self.run.notes.push(tr(lang, "The spring and autumn images did not fall on the area, the classes are by the summer image.").to_string()),
        }
        if self.lines {
            self.find_lines(lang, base, &summer);
        }
    }

    /// Осевые линии узких частей почвы, воды и растительности.
    fn find_lines(&mut self, lang: Lang, base: &TraceBase, regions: &[Kind]) {
        let step = base.step.max(1e-9);
        let width = self.line_width / step;
        let least = linear::LENGTH / step;
        for (group, mask) in ot::line_masks(&base.done.parts, regions) {
            let found = linear::lines(&mask, base.rows, base.cols, width, least);
            if !found.is_empty() {
                self.run.lines.push((group, found));
            }
        }
        let km: f64 = self.run.lines.iter().flat_map(|(_, p)| p.iter()).map(|p| linear::length_m(p, step)).sum::<f64>() / 1000.0;
        self.run.notes.push(if self.run.lines.is_empty() { tr(lang, "No linear objects were found.").to_string() } else { trf(lang, "Lines {km} km.", &[("km", &format!("{km:.1}"))]) });
    }

    /// Классы таблицы плюс рубки отдельным классом, у заросших полей -
    /// классы по видам полей и поля слоя, `_apply_classes` модуля.
    pub fn classes(&self, lang: Lang, base: &TraceBase, mut out: Joined) -> Joined {
        let parts = &base.done.parts;
        if let Some(fields) = &self.run.fields {
            let used: Vec<Kind> = FALLOW_ORDER.iter().copied().filter(|k| fields.kinds.contains(k)).collect();
            let region: Vec<i16> = fields.kinds.iter().map(|k| used.iter().position(|u| u == k).map_or(-1, |p| p as i16)).collect();
            out.classes = parts.iter().map(|p| usize::try_from(*p).ok().and_then(|p| region.get(p)).map_or(-1, |k| i32::from(*k))).collect();
            out.kinds = region;
            out.colors = used.iter().map(|k| k.color()).collect();
            out.hexes = out.colors.iter().map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}")).collect();
            out.names = used.iter().map(|k| kind_name(lang, *k).to_string()).collect();
            out.fields = vec![
                ("last_bare", fields.last.iter().map(|y| y.to_string()).collect()),
                ("woody_pct", fields.kinds.iter().enumerate().map(|(n, _)| format!("{:.1}", 100.0 * fields.woody.as_ref().and_then(|w| w.get(n)).copied().unwrap_or(0.0))).collect()),
            ];
        }
        if !self.run.clearing.is_empty() {
            let class = out.colors.len() as i16;
            for n in &self.run.clearing {
                if let Some(k) = out.kinds.get_mut(*n) {
                    *k = class;
                }
            }
            let kinds = &out.kinds;
            out.classes = parts.iter().zip(&out.classes).map(|(p, c)| usize::try_from(*p).ok().and_then(|p| kinds.get(p)).map_or(*c, |k| i32::from(*k))).collect();
            let color = Kind::Clearing.color();
            out.colors.push(color);
            out.hexes.push(format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2]));
            out.names.push(kind_name(lang, Kind::Clearing).to_string());
        }
        out
    }

    /// Линии на глобусе поверх классов: клетки под линиями получают
    /// классы за классами областей. Итог - (классы клеток, цвета и названия
    /// групп линий).
    pub fn line_layer(&self, lang: Lang, base: &TraceBase, classes: &[i32], first: usize) -> LineLayer {
        let mut out = classes.to_vec();
        let mut extra = Vec::new();
        for (group, paths) in &self.run.lines {
            let class = (first + extra.len()) as i32;
            for (k, on) in linear::cells_of(paths, base.rows, base.cols).into_iter().enumerate() {
                if on && out.get(k).is_some_and(|c| *c >= 0) {
                    out[k] = class;
                }
            }
            extra.push((ot::line_color(group), line_name(lang, group).to_string()));
        }
        (out, extra)
    }

    /// Линии папкой «Моих меток»: поля kind, name, length_m.
    pub fn line_places(&self, lang: Lang, base: &TraceBase, to_degrees: &ToDegrees) -> Option<scene::kml::Folder> {
        if self.run.lines.is_empty() {
            return None;
        }
        let mut folder = scene::kml::Folder::new(tr(lang, "Lines"));
        for (group, paths) in &self.run.lines {
            let [r, g, b] = ot::line_color(group);
            let name = line_name(lang, group);
            for path in paths {
                let mut place = scene::kml::Place::new(name, scene::features::Kind::Line, path.iter().map(to_degrees).collect());
                place.color = [r, g, b, 255];
                place.data = Some(vec![("kind".to_string(), (*group).to_string()), ("name".to_string(), name.to_string()), ("length_m".to_string(), format!("{:.1}", linear::length_m(path, base.step)))]);
                folder.children.push(scene::kml::Item::Place(place));
            }
        }
        Some(folder)
    }
}
