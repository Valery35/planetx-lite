// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Архив снимков Wayback»: выпуски Esri World Imagery, в которых
//! снимок у точки взгляда менялся, перенос темы Wayback модуля. Расчёт -
//! `data::wayback`. У модуля выпуски идут строкой тем со шкалой времени,
//! у Lite выбранный выпуск ложится строкой карт, как снимок OpenAerialMap.

use data::wayback::{self, Info, Release};
use eframe::egui;

use crate::i18n::{Lang, tr, trf};

/// Наибольший ответ службы, байт.
const ANSWER_BYTES: u64 = 4 << 20;
/// Наибольшее количество шагов цепочки изменений.
const CHAIN_MOST: usize = 64;

type Requests<'a> = &'a dyn Fn(u64) -> Box<dyn net::Request>;

/// Окно архива.
pub struct Wayback {
    pub open: bool,
    releases: Vec<Release>,
    config: Option<Box<dyn net::Request>>,
    /// Запрос звена цепочки: номер выпуска и запрос.
    link: Option<(u64, Box<dyn net::Request>)>,
    /// Точка и уровень поиска: широта, долгота, уровень.
    place: (f64, f64, u32),
    /// Выпуски с изменениями, новые первыми.
    rows: Vec<Release>,
    steps: usize,
    selected: Option<usize>,
    info_ask: Option<Box<dyn net::Request>>,
    info: Option<Info>,
    status: String,
    /// Выпуск на глобус.
    pub add: Option<Release>,
    /// Новый поиск у нынешней точки взгляда.
    pub search: bool,
}

impl Wayback {
    pub fn new(lat: f64, lon: f64, width: f64, request: Requests) -> Self {
        let ask = request(ANSWER_BYTES);
        ask.start(wayback::CONFIG_URL);
        Self { open: true, releases: Vec::new(), config: Some(ask), link: None, place: (lat, lon, wayback::search_level(width)), rows: Vec::new(), steps: 0, selected: None, info_ask: None, info: None, status: String::new(), add: None, search: false }
    }

    pub fn busy(&self) -> bool {
        self.config.is_some() || self.link.is_some() || self.info_ask.is_some()
    }

    /// Новый поиск у точки.
    pub fn show_view(&mut self, lat: f64, lon: f64, width: f64, request: Requests) {
        self.open = true;
        self.place = (lat, lon, wayback::search_level(width));
        self.rows.clear();
        self.selected = None;
        self.info = None;
        self.info_ask = None;
        self.link = None;
        self.steps = 0;
        self.status.clear();
        if let Some(newest) = self.releases.last().cloned() {
            self.ask_link(&newest, request);
        } else if self.config.is_none() {
            // Список выпусков не пришёл или окно закрыли до ответа: без
            // нового запроса поиск молчал и окно стояло на «0 из 0».
            let ask = request(ANSWER_BYTES);
            ask.start(wayback::CONFIG_URL);
            self.config = Some(ask);
        }
    }

    fn ask_link(&mut self, release: &Release, request: Requests) {
        let (lat, lon, z) = self.place;
        let (x, y) = wayback::tile_of(lat, lon, z);
        let ask = request(ANSWER_BYTES);
        ask.start(&wayback::tilemap_url(release.number, z, x, y));
        self.link = Some((release.number, ask));
        self.steps += 1;
    }

    /// Шаг: список выпусков, звенья цепочки, сведения о снимке.
    pub fn step(&mut self, lang: Lang, request: Requests) {
        if let Some(answer) = self.config.as_ref().and_then(|a| a.take()) {
            self.config = None;
            self.releases = answer.map(|b| wayback::parse_config(&b)).unwrap_or_default();
            match self.releases.last().cloned() {
                Some(newest) => self.ask_link(&newest, request),
                None => self.status = tr(lang, "The Wayback archive did not answer.").to_string(),
            }
        }
        if let Some((number, ask)) = &self.link
            && let Some(answer) = ask.take()
        {
            let number = *number;
            self.link = None;
            let effective = answer.ok().and_then(|b| wayback::parse_tilemap(&b, number));
            if let Some(found) = effective
                && let Some(release) = self.releases.iter().find(|r| r.number == found).cloned()
                && !self.rows.contains(&release)
            {
                self.rows.push(release);
            }
            let next = wayback::changes_next(&self.releases, effective.unwrap_or(number)).cloned();
            if let Some(next) = next.filter(|_| self.steps < CHAIN_MOST) {
                self.ask_link(&next, request);
            }
        }
        if let Some(answer) = self.info_ask.as_ref().and_then(|a| a.take()) {
            self.info_ask = None;
            self.info = answer.ok().and_then(|b| wayback::parse_metadata(&b));
        }
    }

    fn info_text(&self, lang: Lang) -> String {
        let Some(i) = &self.info else {
            return String::new();
        };
        let mut parts = vec![trf(lang, "Shot on {date}", &[("date", &i.date)])];
        parts.extend([i.provider.clone(), i.sensor.clone()].into_iter().filter(|p| !p.is_empty()));
        if let Some(r) = i.resolution {
            parts.push(trf(lang, "pixel {value} m", &[("value", &format!("{r:.2}"))]));
        }
        parts.join(", ")
    }

    /// Окно.
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang, request: Requests) {
        if !self.open {
            return;
        }
        let mut open = true;
        let mut chosen = None;
        let mut add = false;
        egui::Window::new(tr(lang, "Wayback imagery archive")).id(egui::Id::unique("planetx-wayback")).open(&mut open).default_width(420.0).show(ctx, |ui| {
            let text = if !self.status.is_empty() {
                self.status.clone()
            } else if self.busy() {
                trf(lang, "Searching for releases with changes: found {count}…", &[("count", &self.rows.len().to_string())])
            } else {
                trf(lang, "Releases with changes at the view point: {count} of {total}.", &[("count", &self.rows.len().to_string()), ("total", &self.releases.len().to_string())])
            };
            ui.add(egui::Label::new(text).wrap());
            egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show(ui, |ui| {
                for (n, r) in self.rows.iter().enumerate() {
                    let row = ui.selectable_label(self.selected == Some(n), &r.day);
                    if row.clicked() {
                        chosen = Some(n);
                    }
                    if row.double_clicked() {
                        chosen = Some(n);
                        add = true;
                    }
                }
            });
            ui.add(egui::Label::new(self.info_text(lang)).wrap());
            ui.horizontal(|ui| {
                if ui.button(tr(lang, "Search in the view")).on_hover_text(tr(lang, "Releases with changes at the present view point. The width of the view sets the size of the searched place.")).clicked() {
                    self.search = true;
                }
                if ui.add_enabled(self.selected.is_some(), egui::Button::new(tr(lang, "On the globe"))).on_hover_text(tr(lang, "The release lies as a tile layer over the base map, a row in the catalogue.")).clicked() {
                    add = true;
                }
            });
            ui.add(egui::Label::new(tr(lang, "Releases of Esri World Imagery since 2014. A release where the place did not change gives the tile of an earlier one, so only releases with changes are listed.")).wrap());
        });
        if let Some(n) = chosen {
            self.selected = Some(n);
            self.info = None;
            self.info_ask = None;
            let (lat, lon, _) = self.place;
            if let Some(url) = self.rows.get(n).and_then(|r| wayback::metadata_url(r, lat, lon, wayback::LOCAL_LEVEL)) {
                let ask = request(ANSWER_BYTES);
                ask.start(&url);
                self.info_ask = Some(ask);
            }
        }
        if add {
            self.add = self.selected.and_then(|n| self.rows.get(n)).cloned();
        }
        if !open {
            self.open = false;
            self.config = None;
            self.link = None;
            self.info_ask = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Часть адреса и ответ.
    type Answers = Vec<(String, Vec<u8>)>;

    /// Ответы по части адреса.
    #[derive(Clone)]
    struct Stub(Rc<RefCell<Answers>>, Rc<RefCell<Vec<String>>>, Rc<net::Stats>, Rc<RefCell<String>>);

    impl net::Request for Stub {
        fn start(&self, url: &str) {
            self.1.borrow_mut().push(url.to_string());
            *self.3.borrow_mut() = url.to_string();
        }
        fn take(&self) -> Option<Result<Vec<u8>, String>> {
            let url = self.3.borrow().clone();
            self.0.borrow().iter().find(|(part, _)| url.contains(part.as_str())).map(|(_, body)| Ok(body.clone())).or(Some(Err("нет".to_string())))
        }
        fn stats(&self) -> &net::Stats {
            &self.2
        }
    }

    #[test]
    fn chain_lists_only_releases_with_changes() {
        let config = br#"{"30": {"itemTitle": "Wayback 2026-01-01"}, "20": {"itemTitle": "Wayback 2020-01-01"}, "15": {"itemTitle": "Wayback 2018-01-01"}, "10": {"itemTitle": "Wayback 2015-01-01"}}"#.to_vec();
        let answers = vec![
            ("waybackconfig".to_string(), config),
            // Новейший отдаёт тайл выпуска 20, старше него - свой тайл у 10.
            ("tilemap/30/".to_string(), br#"{"data":[1],"select":[20]}"#.to_vec()),
            ("tilemap/15/".to_string(), br#"{"data":[1],"select":[10]}"#.to_vec()),
        ];
        let urls = Rc::new(RefCell::new(Vec::new()));
        let stub = Stub(Rc::new(RefCell::new(answers)), urls.clone(), Rc::new(net::Stats::default()), Rc::new(RefCell::new(String::new())));
        let request = move |_: u64| -> Box<dyn net::Request> {
            let mut s = stub.clone();
            s.3 = Rc::new(RefCell::new(String::new()));
            Box::new(s)
        };
        let mut w = Wayback::new(58.0, 56.0, 1000.0, &request);
        for _ in 0..10 {
            w.step(Lang::EN, &request);
        }
        assert_eq!(w.rows.iter().map(|r| r.number).collect::<Vec<_>>(), [20, 10]);
        assert!(!w.busy());
        assert_eq!(urls.borrow().iter().filter(|u| u.contains("tilemap")).count(), 2);
    }

    #[test]
    fn search_asks_releases_again_after_failure() {
        let config = br#"{"30": {"itemTitle": "Wayback 2026-01-01"}}"#.to_vec();
        let answers = Rc::new(RefCell::new(Vec::new()));
        let urls = Rc::new(RefCell::new(Vec::new()));
        let stub = Stub(answers.clone(), urls.clone(), Rc::new(net::Stats::default()), Rc::new(RefCell::new(String::new())));
        let request = move |_: u64| -> Box<dyn net::Request> {
            let mut s = stub.clone();
            s.3 = Rc::new(RefCell::new(String::new()));
            Box::new(s)
        };
        // Первый запрос списка без ответа.
        let mut w = Wayback::new(58.0, 56.0, 1000.0, &request);
        w.step(Lang::EN, &request);
        assert!(!w.status.is_empty());
        // Сеть вернулась, поиск в виде заново просит список.
        answers.borrow_mut().push(("waybackconfig".to_string(), config));
        w.show_view(58.0, 56.0, 1000.0, &request);
        w.step(Lang::EN, &request);
        assert_eq!(w.releases.len(), 1);
        assert_eq!(urls.borrow().iter().filter(|u| u.contains("waybackconfig")).count(), 2);
    }
}
