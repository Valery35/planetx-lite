// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Действия программы для строки «Поиск» и Ctrl+K, перенос
//! `ui/commands.py` модуля.
//!
//! Слои, каталог, окна, инструменты панели значков и демо находятся по
//! названию или по другому слову и выполняются из подсказки или по Enter.
//! Подсказка - `scene::searchbar::Suggestion` вида Action, ключ -
//! [`Command`], приписка note - что сделает действие: «включить»,
//! «открыть окно». Слова не показываются и не переводятся: в них русские
//! и английские названия и близкие слова, как у модуля. Действий QGIS,
//! тел глобуса и слоёв, которых у Lite нет, здесь нет.

use scene::searchbar::Suggestion;

use crate::i18n::{Lang, tr};
use crate::layers::Extra;

/// Действие строки «Поиск».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Layer(Extra),
    Catalog,
    Ruler,
    Place,
    SaveView,
    Snapshot,
    CopyView,
    Record,
    Time,
    Identify,
    Properties,
    Sources,
    About,
    SceneSave,
    SceneOpen,
    Fullscreen,
    Share,
    /// Демо: раздел и номер в разделе `crate::demos::MENU`.
    Demo(usize, usize),
}

impl Command {
    /// Ключ подсказки, как у модуля: «layer:air», «tool:ruler».
    pub fn key(self) -> String {
        match self {
            Self::Layer(e) => format!("layer:{}", e.key()),
            Self::Catalog => "open:gallery".into(),
            Self::Ruler => "tool:ruler".into(),
            Self::Place => "tool:place".into(),
            Self::SaveView => "tool:save_view".into(),
            Self::Snapshot => "tool:snapshot".into(),
            Self::CopyView => "tool:copy_view".into(),
            Self::Record => "tool:record".into(),
            Self::Time => "tool:time".into(),
            Self::Identify => "tool:identify".into(),
            Self::Properties => "tool:properties".into(),
            Self::Sources => "tool:sources".into(),
            Self::About => "tool:about".into(),
            Self::SceneSave => "tool:scene_save".into(),
            Self::SceneOpen => "tool:scene_open".into(),
            Self::Fullscreen => "tool:fullscreen".into(),
            Self::Share => "tool:share".into(),
            Self::Demo(s, d) => format!("demo:{s}:{d}"),
        }
    }

    /// Действие по ключу подсказки.
    pub fn from_key(key: &str) -> Option<Self> {
        let mut all: Vec<Command> = Extra::all().into_iter().map(Command::Layer).collect();
        all.extend([
            Command::Catalog,
            Command::Ruler,
            Command::Place,
            Command::SaveView,
            Command::Snapshot,
            Command::CopyView,
            Command::Record,
            Command::Time,
            Command::Identify,
            Command::Properties,
            Command::Sources,
            Command::About,
            Command::SceneSave,
            Command::SceneOpen,
            Command::Fullscreen,
            Command::Share,
        ]);
        for (s, section) in crate::demos::MENU.iter().enumerate() {
            all.extend((0..section.demos.len()).map(|d| Command::Demo(s, d)));
        }
        all.into_iter().find(|c| c.key() == key)
    }
}

/// Что сейчас включено: от этого «включить» или «выключить».
#[derive(Debug, Clone, Default)]
pub struct Now {
    pub extras: std::collections::BTreeSet<Extra>,
    pub recording: bool,
    /// Шкала времени доступна и открыта.
    pub time: Option<bool>,
    pub identify: bool,
}

/// Другие слова слоёв, `WORDS` модуля.
fn layer_words(extra: Extra) -> &'static [&'static str] {
    match extra {
        Extra::Stars => &["звёзды", "небо", "млечный путь", "stars", "milky way"],
        Extra::Air => &["атмосфера", "воздух", "дымка", "гало", "atmosphere", "haze", "air"],
        Extra::Sun => &["ночь", "день", "освещение", "тень", "огни городов", "sun", "night", "city lights"],
        Extra::Grid => &["сетка", "градусы", "экватор", "graticule", "grid"],
        Extra::Live(crate::live::Kind::Vessels) => &["суда", "корабли", "ais", "балтика", "ships", "vessels"],
        Extra::Live(crate::live::Kind::Aircraft) => &["самолёты", "самолеты", "авиация", "ads-b", "aircraft", "planes", "flights"],
        Extra::Live(crate::live::Kind::Trains) => &["поезда", "железная дорога", "финляндия", "trains", "railway"],
        Extra::Geoid => &["геоид", "фигура земли", "egm2008", "geoid"],
        Extra::Paleo => &["палеогеография", "палеокарта", "древние материки", "пангея", "paleogeography", "paleomap", "pangaea"],
        Extra::Cutaway => &["разрез земли", "разрез", "ядро", "мантия", "кора", "cutaway", "core", "mantle", "crust"],
        Extra::Currents => &["течения", "океан", "гольфстрим", "currents", "ocean"],
        Extra::Buildings => &["здания", "дома", "3d", "buildings"],
        Extra::Overture => &["здания", "overture", "buildings", "3d"],
        Extra::Mining => &["карьеры", "отвалы", "горные работы", "mining", "pits"],
        Extra::Satellites => &["мкс", "спутники", "орбиты", "starlink", "iss", "satellites"],
        Extra::Wiki => &["википедия", "энциклопедия", "статьи", "достопримечательности", "wikipedia", "encyclopedia", "articles", "landmarks"],
        Extra::Map(_) | Extra::Dots(_) => key_words(extra.key()),
    }
}

/// Слова слоёв каталога по ключу, `WORDS` модуля.
fn key_words(key: &str) -> &'static [&'static str] {
    match key {
        "glim" => &["литология", "литологическая карта", "породы", "типы пород", "glim", "lithology", "rock types"],
        "emag" => &["магнитные аномалии", "магнитка", "аэромагнитка", "магниторазведка", "emag", "magnetic anomaly"],
        "gravity" => &["сила тяжести", "гравитация", "гравиметрия", "аномалии силы тяжести", "xgm2019e", "gravity", "free-air"],
        "bouguer" => &["буге", "аномалия буге", "аномалии буге", "bouguer"],
        "iscgem" => &["исторические землетрясения", "землетрясения 1904", "каталог землетрясений", "iscgem", "historical earthquakes"],
        "seafloor_age" => &["возраст дна", "возраст океанического дна", "возраст коры", "спрединг", "seafloor age", "ocean age"],
        "heatflow" => &["тепловой поток", "геотермия", "теплопоток", "heat flow", "geothermal"],
        "worldcover" => &["покрытие земель", "растительность", "лес", "пашня", "застройка", "землепользование", "worldcover", "land cover"],
        "water_transitions" => &["изменения воды", "исчезнувшие озёра", "новые водохранилища", "обмеление", "water change", "water transitions"],
        "soil_clay" => &["глина", "почвы", "почва", "гранулометрия", "soilgrids", "clay", "soil"],
        "soil_sand" => &["песок", "песчаные почвы", "sand"],
        "soil_soc" => &["органический углерод", "гумус", "углерод почвы", "soil carbon", "soc"],
        "soil_ph" => &["кислотность", "ph", "кислые почвы", "soil ph", "acidity"],
        "sea_marks" => &["морские знаки", "буи", "маяки", "фарватер", "openseamap", "sea marks", "buoys"],
        "hiking" => &["пешие маршруты", "тропы", "туризм", "походы", "hiking", "trails"],
        "cycling" => &["велосипед", "веломаршруты", "cycling", "bike routes"],
        "quakes" => &["землетрясения", "сейсмика", "очаги", "earthquakes"],
        "volcanoes" => &["вулканы", "вулкан", "извержения", "gvp", "volcanoes", "volcano"],
        "fires" => &["пожары", "огонь", "fires", "firms"],
        "plates" => &["тектоника", "плиты", "plates", "tectonics"],
        "faults" => &["разломы", "активные разломы", "сбросы", "надвиги", "сдвиги", "gem", "faults", "active faults"],
        "cables" => &["подводные кабели", "кабели", "submarine cables"],
        "deposits" => &["полезные ископаемые", "руды", "рудники", "usgs", "mrds", "minerals", "deposits"],
        "magpoles" => &["магнитный полюс", "магнитные полюса", "компас", "геомагнетизм", "igrf", "magnetic pole", "compass"],
        "magdecl" => &["склонение", "изогоны", "линии склонения", "агона", "магнитное склонение", "declination", "isogonic"],
        "platemotion" => &["скорости плит", "движение плит", "дрейф", "дрейф континентов", "morvel", "plate motion"],
        "plants" => &["электростанции", "энергетика", "тэц", "гэс", "аэс", "power plants"],
        "osm_mines" => &["шахты", "карьеры", "mines", "quarries"],
        "infra_osm" => &["лэп", "электросети", "подстанции", "трубопроводы", "газопровод", "нефтепровод", "power lines", "pipelines"],
        _ => &[],
    }
}

/// Подсказки всех действий на языке интерфейса, `build` модуля.
pub fn actions(lang: Lang, now: &Now) -> Vec<Suggestion> {
    let mut out = Vec::new();
    let mut add = |c: Command, title: &str, note: &str, words: &[&str]| {
        out.push(Suggestion::action(&c.key(), title, note, words));
    };
    let on_off = |on: bool| if on { tr(lang, "turn off") } else { tr(lang, "turn on") };
    for extra in Extra::all() {
        let on = now.extras.contains(&extra);
        add(Command::Layer(extra), tr(lang, extra.name()), on_off(on), layer_words(extra));
    }
    add(
        Command::Catalog,
        tr(lang, "Add data"),
        tr(lang, "open the catalogue"),
        &["витрина", "каталог", "gallery", "карты и слои", "add data"],
    );
    let open = tr(lang, "open the window");
    let run = tr(lang, "run");
    add(Command::Ruler, tr(lang, "Ruler"), open, &["измерить", "расстояние", "площадь", "длина", "ruler", "measure"]);
    add(
        Command::Place,
        tr(lang, "New placemark"),
        open,
        &["метка", "путь", "многоугольник", "круг", "нарисовать", "placemark", "path", "polygon"],
    );
    add(Command::SaveView, tr(lang, "Save view"), run, &["закладка", "вид", "bookmark"]);
    add(Command::Snapshot, tr(lang, "View snapshot"), open, &["скриншот", "картинка", "png", "screenshot"]);
    add(
        Command::CopyView,
        tr(lang, "Copy the view to the clipboard"),
        run,
        &["копия экрана", "копия вида", "скриншот", "буфер обмена", "copy", "clipboard", "screenshot"],
    );
    add(
        Command::Record,
        tr(lang, "Record a flight"),
        if now.recording { tr(lang, "stop") } else { tr(lang, "start") },
        &["тур", "запись", "облёт", "tour", "record"],
    );
    if let Some(open_now) = now.time {
        add(
            Command::Time,
            tr(lang, "Time slider"),
            if open_now { tr(lang, "close") } else { tr(lang, "open") },
            &["время", "дата", "time", "timeline"],
        );
    }
    add(Command::Identify, tr(lang, "Identify features"), on_off(now.identify), &["опрос", "сведения", "identify"]);
    add(
        Command::Properties,
        tr(lang, "View properties"),
        open,
        &["настройки", "рельеф", "подписи", "язык", "settings", "properties"],
    );
    add(
        Command::Sources,
        tr(lang, "Data sources"),
        open,
        &["подложка", "основа", "рельеф", "dem", "sources", "basemap"],
    );
    add(Command::About, tr(lang, "About"), open, &["версия", "about", "version"]);
    add(Command::SceneSave, tr(lang, "Save Scene"), open, &["сцена", "scene"]);
    add(Command::SceneOpen, tr(lang, "Open Scene"), open, &["сцена", "scene"]);
    add(
        Command::Fullscreen,
        tr(lang, "Full screen"),
        run,
        &["полный экран", "весь экран", "показ", "презентация", "f11", "fullscreen", "full screen", "presentation"],
    );
    add(
        Command::Share,
        tr(lang, "Share view"),
        open,
        &["html", "страница", "браузер", "отправить", "share", "web", "page", "browser"],
    );
    for (s, section) in crate::demos::MENU.iter().enumerate() {
        for (d, demo) in section.demos.iter().enumerate() {
            add(Command::Demo(s, d), tr(lang, demo.en), tr(lang, "open the demo"), &["демо", "пример", "demo"]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{Command, Now, actions};
    use crate::i18n::Lang;
    use scene::searchbar as sb;

    #[test]
    fn keys_round_trip_and_words_find() {
        let now = Now { time: Some(false), ..Now::default() };
        for s in actions(Lang::EN, &now) {
            assert!(Command::from_key(&s.key).is_some(), "{}", s.key);
        }
        let found = sb::suggestions("измерить", &[], &[], sb::LIMIT, &actions(Lang::RU, &now));
        assert_eq!(Command::from_key(&found[0].key), Some(Command::Ruler));
        let found = sb::suggestions("самосвалы", &[], &[], sb::LIMIT, &actions(Lang::RU, &now));
        assert!(found.iter().any(|s| matches!(Command::from_key(&s.key), Some(Command::Demo(..)))));
    }
}
