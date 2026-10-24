// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Маршрут по дорогам, перенос `RouteManager` (`ui/routing.py`) модуля.
//!
//! Расчёт - `data::routing`. Начало и конец ставит меню на глобусе,
//! пункты «Проложить маршрут отсюда» и «Проложить маршрут сюда».
//! Маршрут сначала просится у сервиса OSRM, не чаще раза в секунду.
//! Сервис не ответил, а точки не дальше MAX_DISTANCE, - путь по тайлам
//! дорог векторной основы. Тайлы коридора приходят через загрузчик
//! векторной основы, граф строится сразу, путь ищется частями по кадрам.
//! Готовый путь отдаётся окну, оно кладёт его в «Мои метки». Строка под
//! поиском показывает ход, длину и время и пересчитывает путь другим
//! способом.

use std::collections::HashMap;

use data::routing::{self, Graph, LatLon, Line, Mode, Route, Search, Step};
use eframe::egui;

use crate::i18n::{Lang, tr, trf};

/// Цвет и толщина линии маршрута, как у модуля.
pub const ROUTE_COLOR: [u8; 4] = [0, 120, 255, 255];
pub const ROUTE_WIDTH: f64 = 4.0;
/// Предел ответа сервиса в байтах.
pub const SERVICE_BYTES: u64 = 4 << 20;

/// Точка маршрута: широта, долгота, название (может быть пустым).
pub type End = (f64, f64, String);

/// Кто построил маршрут.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Via {
    Service,
    Tiles,
}

/// Ход расчёта.
enum Stage {
    Idle,
    /// Запрос к сервису ждёт паузы SERVICE_GAP с прошлого.
    Waiting,
    Service(Box<dyn net::Request>),
    /// Тайлы коридора идут через загрузчик векторной основы.
    Tiles,
    /// Поиск пути частями.
    Search(Box<(Graph, Search)>),
}

/// Ссылка строки маршрута под поиском.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    Mode(Mode),
    Fix,
    Clear,
}

/// Маршруты окна глобуса.
pub struct Router {
    pub origin: Option<End>,
    pub target: Option<End>,
    pub mode: Mode,
    /// Ключ тайла - линии дорог.
    lines: HashMap<routing::TileKey, Vec<Line>>,
    keys: Vec<routing::TileKey>,
    half: f64,
    widened: u32,
    stage: Stage,
    /// Время прошлого запроса к сервису.
    service_at: f64,
    service_error: String,
    via: Via,
    /// Строка под поиском и ссылки при ней.
    pub text: String,
    links: bool,
}

/// Название способа для сводки.
fn mode_name(lang: Lang, mode: Mode) -> &'static str {
    match mode {
        Mode::Car => tr(lang, "by car"),
        Mode::Bike => tr(lang, "by bike"),
        Mode::Foot => tr(lang, "on foot"),
    }
}

/// Название способа для ссылки.
fn mode_link(lang: Lang, mode: Mode) -> &'static str {
    match mode {
        Mode::Car => tr(lang, "By car"),
        Mode::Bike => tr(lang, "By bike"),
        Mode::Foot => tr(lang, "On foot"),
    }
}

/// Время в пути: минуты, с часа - часы и минуты, `duration_text` модуля.
pub fn duration_text(lang: Lang, seconds: f64) -> String {
    let minutes = (seconds / 60.0).round().max(0.0) as u64;
    if minutes < 60 {
        return trf(lang, "{value} min", &[("value", &minutes.max(1).to_string())]);
    }
    trf(lang, "{hours} h {minutes} min", &[("hours", &(minutes / 60).to_string()), ("minutes", &(minutes % 60).to_string())])
}

impl Router {
    pub fn new(mode: Mode) -> Self {
        Self {
            origin: None,
            target: None,
            mode,
            lines: HashMap::new(),
            keys: Vec::new(),
            half: 0.0,
            widened: 0,
            stage: Stage::Idle,
            service_at: f64::NEG_INFINITY,
            service_error: String::new(),
            via: Via::Tiles,
            text: String::new(),
            links: false,
        }
    }

    pub fn set_origin(&mut self, lang: Lang, end: End, now: f64) {
        self.origin = Some(end);
        self.show(tr(lang, "The route start is set. The end is the Directions to here item of the globe menu."), true);
        self.maybe_start(lang, now);
    }

    pub fn set_target(&mut self, lang: Lang, end: End, now: f64) {
        self.target = Some(end);
        if self.origin.is_none() {
            self.show(tr(lang, "The route end is set. The start is the Directions from here item of the globe menu."), true);
        }
        self.maybe_start(lang, now);
    }

    pub fn set_mode(&mut self, lang: Lang, mode: Mode, now: f64) {
        self.mode = mode;
        self.maybe_start(lang, now);
    }

    pub fn clear(&mut self) {
        self.stage = Stage::Idle;
        self.origin = None;
        self.target = None;
        self.text.clear();
    }

    /// Расчёт идёт или ждёт.
    pub fn busy(&self) -> bool {
        !matches!(self.stage, Stage::Idle)
    }

    fn ends(&self) -> Option<(LatLon, LatLon)> {
        Some((self.origin.as_ref().map(|o| (o.0, o.1))?, self.target.as_ref().map(|t| (t.0, t.1))?))
    }

    fn maybe_start(&mut self, lang: Lang, _now: f64) {
        if self.ends().is_none() {
            return;
        }
        self.service_error.clear();
        self.show(tr(lang, "Route: asking the service…"), false);
        self.stage = Stage::Waiting;
    }

    /// Шаг кадра: запрос к сервису, ответ, поиск пути. Готовый маршрут
    /// и сводка - результат. request - новый разовый запрос.
    pub fn step(&mut self, lang: Lang, now: f64, request: &dyn Fn() -> Box<dyn net::Request>) -> Option<(Route, String)> {
        let (a, b) = self.ends()?;
        match &mut self.stage {
            Stage::Idle | Stage::Tiles => None,
            Stage::Waiting => {
                if now - self.service_at < routing::SERVICE_GAP {
                    return None;
                }
                self.service_at = now;
                let ask = request();
                ask.start(&routing::service_url(routing::ROUTER_URL, self.mode, a, b));
                self.stage = Stage::Service(ask);
                None
            }
            Stage::Service(ask) => {
                let answer = ask.take()?;
                match answer.and_then(|bytes| routing::parse_service(&bytes)) {
                    Ok(route) => {
                        self.via = Via::Service;
                        self.stage = Stage::Idle;
                        Some(self.done(lang, route))
                    }
                    Err(error) => {
                        self.service_error = if error.is_empty() { "no data".into() } else { error };
                        self.stage = Stage::Idle;
                        self.tiles(lang, a, b);
                        None
                    }
                }
            }
            Stage::Search(pair) => match pair.1.step(&pair.0, routing::STEP_POPS) {
                Step::Going => None,
                Step::Done(Some(route)) => {
                    self.via = Via::Tiles;
                    self.stage = Stage::Idle;
                    Some(self.done(lang, route))
                }
                Step::Done(None) => {
                    self.stage = Stage::Idle;
                    self.no_path(lang, a, b);
                    None
                }
            },
        }
    }

    /// Маршрут по тайлам дорог векторной основы.
    fn tiles(&mut self, lang: Lang, a: LatLon, b: LatLon) {
        let distance = routing::metres(a, b);
        if distance > routing::MAX_DISTANCE {
            let limit = format!("{:.0}", routing::MAX_DISTANCE / 1000.0);
            self.show(
                trf(
                    lang,
                    "No route: the service answered '{error}', and over the roads of the vector base a route is built up to {limit} km in a straight line.",
                    &[("error", &self.service_error), ("limit", &limit)],
                ),
                true,
            );
            return;
        }
        self.half = routing::CORRIDOR_MIN.max(routing::CORRIDOR_SHARE * distance);
        self.widened = 0;
        self.corridor(lang, a, b);
    }

    fn corridor(&mut self, lang: Lang, a: LatLon, b: LatLon) {
        let keys = routing::corridor(a, b, self.half);
        if keys.len() > routing::MAX_TILES {
            self.show(
                trf(lang, "No route: the corridor between the points is {count} road tiles, the limit is {limit}.", &[("count", &keys.len().to_string()), ("limit", &routing::MAX_TILES.to_string())]),
                false,
            );
            return;
        }
        self.keys = keys;
        self.stage = Stage::Tiles;
        self.progress(lang);
    }

    /// Тайлы коридора, которых ещё нет, для загрузчика векторной основы.
    pub fn wanted(&self) -> Vec<routing::TileKey> {
        if !matches!(self.stage, Stage::Tiles) {
            return Vec::new();
        }
        self.keys.iter().copied().filter(|k| !self.lines.contains_key(k)).collect()
    }

    /// Пришёл тайл векторной основы. bytes None - тайла нет у источника.
    /// Последний тайл коридора начинает расчёт.
    pub fn add_tile(&mut self, lang: Lang, key: routing::TileKey, bytes: Option<&[u8]>) {
        if !matches!(self.stage, Stage::Tiles) || !self.keys.contains(&key) || self.lines.contains_key(&key) {
            return;
        }
        self.lines.insert(key, bytes.and_then(|b| routing::decode(key, b).ok()).unwrap_or_default());
        self.progress(lang);
    }

    /// Ход загрузки тайлов, все пришли - граф и поиск.
    fn progress(&mut self, lang: Lang) {
        let done = self.keys.iter().filter(|k| self.lines.contains_key(k)).count();
        if done < self.keys.len() {
            self.show(trf(lang, "Route: road tiles {done} of {total}", &[("done", &done.to_string()), ("total", &self.keys.len().to_string())]), false);
            return;
        }
        let Some((a, b)) = self.ends() else {
            return;
        };
        let lines: Vec<Line> = self.keys.iter().filter_map(|k| self.lines.get(k)).flatten().cloned().collect();
        self.show(tr(lang, "Route: calculating…"), false);
        match routing::build(&lines, self.mode) {
            Some(graph) => {
                let search = Search::new(&graph, a, b, self.mode);
                self.stage = Stage::Search(Box::new((graph, search)));
            }
            None => {
                self.stage = Stage::Idle;
                self.no_path(lang, a, b);
            }
        }
    }

    /// Пути в коридоре нет: шире вдвое, до WIDEN раз.
    fn no_path(&mut self, lang: Lang, a: LatLon, b: LatLon) {
        if self.widened < routing::WIDEN {
            self.widened += 1;
            self.half *= 2.0;
            self.corridor(lang, a, b);
            return;
        }
        let reach = format!("{:.0}", routing::REACH);
        self.show(trf(lang, "No route found: the points are not linked by the vector base roads or lie more than {reach} m from them.", &[("reach", &reach)]), true);
    }

    /// Сводка готового маршрута.
    fn done(&mut self, lang: Lang, route: Route) -> (Route, String) {
        let name = |end: &Option<End>| end.as_ref().map(|e| e.2.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| tr(lang, "point").to_string());
        let mut text = trf(
            lang,
            "{a} - {b}, {km} km, {time} {mode}",
            &[
                ("a", &name(&self.origin)),
                ("b", &name(&self.target)),
                ("km", &format!("{:.1}", route.length / 1000.0)),
                ("time", &duration_text(lang, route.seconds)),
                ("mode", mode_name(lang, self.mode)),
            ],
        );
        if self.via == Via::Service {
            text.push_str(". ");
            text.push_str(tr(lang, "OSRM service, data © OpenStreetMap"));
        } else if !self.service_error.is_empty() {
            text.push_str(". ");
            text.push_str(&trf(lang, "The service did not answer ({error}), route over road tiles", &[("error", &self.service_error)]));
        }
        (route, text)
    }

    /// Строка под поиском: ход или итог. links - ссылки способов.
    pub fn show(&mut self, text: impl Into<String>, links: bool) {
        self.text = text.into();
        self.links = links;
    }

    /// Строка маршрута под поиском с ссылками, как `set_answer` модуля.
    pub fn ui(&self, ui: &mut egui::Ui, lang: Lang) -> Option<Link> {
        if self.text.is_empty() {
            return None;
        }
        let mut chosen = None;
        ui.label(egui::RichText::new(&self.text).small());
        ui.horizontal_wrapped(|ui| {
            if self.links && self.ends().is_some() {
                for mode in routing::MODES {
                    if mode != self.mode && ui.link(mode_link(lang, mode)).clicked() {
                        chosen = Some(Link::Mode(mode));
                    }
                }
                if self.via == Via::Service && !self.busy() && ui.link(tr(lang, "Map error")).on_hover_text(routing::FIX_MAP).clicked() {
                    chosen = Some(Link::Fix);
                }
            }
            if (self.origin.is_some() || self.target.is_some()) && ui.link(tr(lang, "Clear|route")).clicked() {
                chosen = Some(Link::Clear);
            }
        });
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;

    struct Fixed(std::cell::RefCell<Option<Result<Vec<u8>, String>>>, net::Stats);

    impl net::Request for Fixed {
        fn start(&self, _url: &str) {}
        fn take(&self) -> Option<Result<Vec<u8>, String>> {
            self.0.borrow_mut().take()
        }
        fn stats(&self) -> &net::Stats {
            &self.1
        }
    }

    fn answer(body: &str) -> impl Fn() -> Box<dyn net::Request> {
        let body = body.to_string();
        move || Box::new(Fixed(std::cell::RefCell::new(Some(Ok(body.clone().into_bytes()))), net::Stats::default())) as Box<dyn net::Request>
    }

    #[test]
    fn service_route_comes_with_summary() {
        let lang = Lang::EN;
        let mut r = Router::new(Mode::Car);
        r.set_origin(lang, (59.0, 30.0, "A".into()), 0.0);
        assert!(!r.busy());
        r.set_target(lang, (59.01, 30.02, String::new()), 0.0);
        assert!(r.busy());
        let json = r#"{"code":"Ok","routes":[{"distance":2500.0,"duration":420.0,"geometry":{"type":"LineString","coordinates":[[30.0,59.0],[30.02,59.01]]}}]}"#;
        let ask = answer(json);
        assert!(r.step(lang, 10.0, &ask).is_none());
        let got = r.step(lang, 10.1, &ask);
        assert!(got.is_some());
        let Some((route, text)) = got else {
            return;
        };
        assert_eq!(route.points.len(), 2);
        assert!(text.starts_with("A - point, 2.5 km, 7 min by car"), "{text}");
        assert!(!r.busy());
    }

    #[test]
    fn service_error_falls_back_to_tiles() {
        let lang = Lang::EN;
        let mut r = Router::new(Mode::Foot);
        r.set_origin(lang, (59.0, 30.0, String::new()), 0.0);
        r.set_target(lang, (59.001, 30.001, String::new()), 0.0);
        let ask = answer(r#"{"code":"NoRoute"}"#);
        assert!(r.step(lang, 5.0, &ask).is_none());
        assert!(r.step(lang, 5.1, &ask).is_none());
        let keys = r.wanted();
        assert!(!keys.is_empty());
        // Пустые тайлы: дорог нет, коридор шире, потом «не найден».
        for _ in 0..=routing::WIDEN {
            for key in r.wanted() {
                r.add_tile(lang, key, None);
            }
        }
        assert!(!r.busy());
        assert!(r.text.contains("No route found"), "{}", r.text);
    }

    #[test]
    fn far_points_without_service_give_a_message() {
        let lang = Lang::EN;
        let mut r = Router::new(Mode::Car);
        r.set_origin(lang, (59.0, 30.0, String::new()), 0.0);
        r.set_target(lang, (55.75, 37.6, String::new()), 0.0);
        let ask = answer("not json");
        r.step(lang, 2.0, &ask);
        r.step(lang, 2.1, &ask);
        assert!(r.wanted().is_empty());
        assert!(r.text.contains("no data"), "{}", r.text);
        r.clear();
        assert!(r.text.is_empty() && r.origin.is_none());
    }

    #[test]
    fn duration() {
        assert_eq!(duration_text(Lang::EN, 20.0), "1 min");
        assert_eq!(duration_text(Lang::EN, 3600.0 + 125.0), "1 h 2 min");
    }
}
