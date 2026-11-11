// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Архив снимков Wayback»: выпуски Esri World Imagery, в которых
//! снимок у точки взгляда менялся, перенос темы Wayback модуля. Расчёт -
//! `data::wayback`. У модуля выпуски идут строкой тем со шкалой времени,
//! у Lite выбранный выпуск ложится строкой карт, как снимок OpenAerialMap.
//!
//! Шторка сравнения, как ⇆ темы Wayback модуля (`set_compare`): справа
//! выбранный выпуск, слева по умолчанию выпуск около года старше
//! (`themes.compare_day`). Стороны ложатся парой текстур «после» и «до»,
//! той же, что у снимков событий.

use data::wayback::{self, Info, Release};
use eframe::egui;
use globe_core::lod::TileKey;

use crate::i18n::{Lang, tr, trf};
use crate::pairtiles::PairTiles;

/// Наибольший ответ службы, байт.
const ANSWER_BYTES: u64 = 4 << 20;
/// Наибольшее количество шагов цепочки изменений.
const CHAIN_MOST: usize = 64;
type Requests<'a> = &'a dyn Fn(u64) -> Box<dyn net::Request>;

/// Выпуск левой стороны шторки по умолчанию: новейший из rows (новые
/// первыми) не позже чем за год до right, как `compare_day` модуля.
/// Короткий ряд - самый старый выпуск. None - других выпусков нет.
pub fn compare_release(rows: &[Release], right: &Release) -> Option<Release> {
    let year: i32 = right.day.get(..4)?.parse().ok()?;
    let back = format!("{:04}{}", year - 1, right.day.get(4..).unwrap_or(""));
    rows.iter().find(|r| r.day <= back && r.number != right.number).or_else(|| rows.iter().rev().find(|r| r.number != right.number)).cloned()
}

/// Шторка двух выпусков. Сторона 0 - справа («после»), 1 - слева («до»).
pub struct Compare {
    pub right: Release,
    pub left: Release,
    /// Линия шторки - доля ширины вида от левого края.
    pub split: f32,
    /// Запросы тайлов сторон.
    pending: PairTiles,
}

impl Compare {
    fn new(right: Release, left: Release) -> Self {
        Self { right, left, split: 0.5, pending: PairTiles::default() }
    }

    /// Выпуск стороны.
    pub fn release(&self, side: usize) -> &Release {
        if side == 0 { &self.right } else { &self.left }
    }

    /// Тайлы стороны side: пришедшие картинки RGBA (None - тайла нет)
    /// и новые запросы для keys.
    pub fn tiles(&mut self, side: usize, keys: &[TileKey], request: Requests) -> Vec<(TileKey, Option<Vec<u8>>)> {
        let template = wayback::tile_url(self.release(side).number);
        self.pending.step(side, &template, keys, request)
    }

    /// Идут ли запросы тайлов.
    pub fn busy(&self) -> bool {
        self.pending.busy()
    }
}

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
    /// Шторка двух выпусков, None - снята.
    pub compare: Option<Compare>,
    /// Стороны шторки, у которых сменился выпуск: их тайлы забываются.
    pub changed: [bool; 2],
    /// Шторку не включить: на глобусе снимки события со своей шторкой.
    pub blocked: bool,
}

impl Wayback {
    pub fn new(lat: f64, lon: f64, width: f64, request: Requests) -> Self {
        let ask = request(ANSWER_BYTES);
        ask.start(wayback::CONFIG_URL);
        Self { open: true, releases: Vec::new(), config: Some(ask), link: None, place: (lat, lon, wayback::search_level(width)), rows: Vec::new(), steps: 0, selected: None, info_ask: None, info: None, status: String::new(), add: None, search: false, compare: None, changed: [false; 2], blocked: false }
    }

    pub fn busy(&self) -> bool {
        self.config.is_some() || self.link.is_some() || self.info_ask.is_some() || self.compare.as_ref().is_some_and(Compare::busy)
    }

    /// Включить или снять шторку. Справа - выбранный выпуск, без выбора -
    /// новейший с изменениями. Ложь - двух выпусков нет.
    pub fn set_compare(&mut self, on: bool) -> bool {
        if !on {
            if self.compare.take().is_some() {
                self.changed = [true; 2];
            }
            return true;
        }
        let Some(right) = self.selected.and_then(|n| self.rows.get(n)).or_else(|| self.rows.first()).cloned() else {
            return false;
        };
        let Some(left) = compare_release(&self.rows, &right) else {
            return false;
        };
        self.compare = Some(Compare::new(right, left));
        self.changed = [true; 2];
        true
    }

    /// Выпуск стороны шторки side.
    fn set_side(&mut self, side: usize, release: Release) {
        let Some(c) = &mut self.compare else {
            return;
        };
        let slot = if side == 0 { &mut c.right } else { &mut c.left };
        if *slot != release {
            *slot = release;
            c.pending.clear(side);
            if let Some(f) = self.changed.get_mut(side) {
                *f = true;
            }
        }
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
        let mut swipe = None;
        let mut sides: Vec<(usize, Release)> = Vec::new();
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
                let mut on = self.compare.is_some();
                let can = self.rows.len() >= 2 || on;
                if ui.add_enabled(can, egui::Button::selectable(on, tr(lang, "⇆ Swipe"))).on_hover_text(tr(lang, "Two releases with a swipe: the chosen one right of the line, a release about a year older left of it. The line is dragged with the mouse, the releases of the sides are chosen below.")).clicked() {
                    on = !on;
                    swipe = Some(on);
                }
            });
            if self.blocked {
                ui.add(egui::Label::new(tr(lang, "The swipe is taken by the images of an event. Remove the event images to compare releases.")).wrap());
            }
            if let Some(c) = &self.compare {
                let (left, right) = (c.left.clone(), c.right.clone());
                ui.horizontal(|ui| {
                    for (side, label, current) in [(1usize, tr(lang, "Left"), left), (0usize, tr(lang, "Right"), right)] {
                        ui.label(label);
                        egui::ComboBox::from_id_salt(("planetx-wayback-side", side)).selected_text(current.day.clone()).show_ui(ui, |ui| {
                            for r in &self.rows {
                                if ui.selectable_label(r.number == current.number, &r.day).clicked() {
                                    sides.push((side, r.clone()));
                                }
                            }
                        });
                    }
                });
            }
            ui.add(egui::Label::new(tr(lang, "Releases of Esri World Imagery since 2014. A release where the place did not change gives the tile of an earlier one, so only releases with changes are listed.")).wrap());
        });
        if let Some(on) = swipe {
            self.set_compare(on);
        }
        for (side, release) in sides {
            self.set_side(side, release);
        }
        if let Some(n) = chosen {
            self.selected = Some(n);
            // Выбор в списке - выпуск справа от шторки.
            if let Some(r) = self.rows.get(n).cloned() {
                self.set_side(0, r);
            }
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
            self.set_compare(false);
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

    fn release(number: u64, day: &str) -> Release {
        Release { number, day: day.into(), meta: String::new() }
    }

    #[test]
    fn swipe_left_is_a_release_about_a_year_older() {
        // Новые первыми, как строки окна.
        let rows = [release(40, "2026-03-01"), release(30, "2025-06-01"), release(20, "2025-02-01"), release(10, "2019-01-01")];
        assert_eq!(compare_release(&rows, &rows[0]).map(|r| r.number), Some(20));
        // Старше на год ничего нет - самый старый выпуск.
        assert_eq!(compare_release(&rows, &rows[3]).map(|r| r.number), Some(20));
        assert_eq!(compare_release(&rows[..1], &rows[0]), None);
    }

    #[test]
    fn swipe_sides_fetch_tiles_of_their_releases() {
        let answers = Rc::new(RefCell::new(Vec::new()));
        let urls = Rc::new(RefCell::new(Vec::new()));
        let stub = Stub(answers, urls.clone(), Rc::new(net::Stats::default()), Rc::new(RefCell::new(String::new())));
        let request = move |_: u64| -> Box<dyn net::Request> {
            let mut s = stub.clone();
            s.3 = Rc::new(RefCell::new(String::new()));
            Box::new(s)
        };
        let mut w = Wayback::new(58.0, 56.0, 1000.0, &request);
        w.rows = vec![release(40, "2026-03-01"), release(20, "2025-02-01")];
        assert!(w.set_compare(true));
        assert_eq!(w.changed, [true; 2]);
        assert!(w.compare.is_some());
        let mut c = w.compare.take().unwrap_or_else(|| Compare::new(release(0, ""), release(0, "")));
        assert_eq!((c.right.number, c.left.number), (40, 20));
        let keys = [(16, 44_000, 19_000), (16, 44_001, 19_000)];
        // Ответов нет: запросы уходят, второй раз не повторяются, тайл
        // без ответа приходит пустым.
        assert!(c.tiles(0, &keys, &request).is_empty());
        let got = c.tiles(1, &keys, &request);
        assert!(got.is_empty());
        let sent = urls.borrow().iter().filter(|u| u.contains("/tile/")).cloned().collect::<Vec<_>>();
        assert_eq!(sent.len(), 4, "{sent:?}");
        assert!(sent.iter().any(|u| u.contains("/tile/40/16/19000/44000")), "{sent:?}");
        assert!(sent.iter().any(|u| u.contains("/tile/20/16/19000/44001")), "{sent:?}");
        // Ответ-ошибка - тайл пуст, запрос снят.
        let done = c.tiles(0, &[], &request);
        assert_eq!(done.len(), 2);
        assert!(done.iter().all(|(_, rgba)| rgba.is_none()));
        w.compare = Some(c);
        w.changed = [false; 2];
        assert!(w.set_compare(false));
        assert!(w.compare.is_none());
        assert_eq!(w.changed, [true; 2]);
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
