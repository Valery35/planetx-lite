// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Поиск места по названию, перенос поиска окна модуля
//! (`ui/globe/search.py`).
//!
//! Правила Nominatim: запрос только по Enter, не чаще раза
//! в [`SEARCH_INTERVAL`], ответы запоминаются до закрытия программы.
//! Подсказок при вводе нет - правила запрещают автодополнение.
//! Перелёт идёт к первому найденному месту, остальные - списком.

use std::collections::HashMap;

use data::geocode::{self, Place, SEARCH_INTERVAL, SEARCH_LIMIT};
use globe_core::flight::fit_view;
use globe_core::navigation::Pose;

/// Ближе этого расстояния камера к найденному месту не подходит, метры.
pub const SEARCH_MIN_DISTANCE: f64 = 3000.0;
/// Дальше тоже: охват страны вроде России дал бы камеру за Луной.
pub const SEARCH_MAX_DISTANCE: f64 = 1.2e7;

/// Ключ памяти ответов: запрос и язык названий.
type Key = (String, Option<String>);

/// Что сообщить под строкой поиска.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    None,
    Searching(String),
    NotFound(String),
    Failed(String),
}

/// Поиск места: очередь запроса, память ответов, найденные места.
pub struct Search {
    language: Option<String>,
    /// Запрос ждёт своей очереди.
    waiting: Option<Key>,
    /// Запрос отправлен, ответа нет.
    in_flight: Option<Key>,
    /// Ключ последнего запроса пользователя: старый ответ не уводит камеру.
    latest: Option<Key>,
    last_sent: Option<f64>,
    memo: HashMap<Key, Vec<Place>>,
    /// Найденные места последнего запроса.
    pub found: Vec<Place>,
    pub status: Status,
    /// Время отправки каждого запроса к Nominatim, для самопроверки.
    pub sent: Vec<f64>,
    /// Адрес точки для «Сведения о точке»: ждёт очереди, отправлен.
    reverse_waiting: Option<(f64, f64)>,
    reverse_in_flight: Option<(f64, f64)>,
    /// Адрес последней точки «Сведения о точке»: точка и адрес или ошибка.
    pub address: Option<((f64, f64), Result<String, String>)>,
}

impl Search {
    /// language - код языка названий или None - местные названия.
    pub fn new(language: Option<&str>) -> Self {
        Self {
            language: language.map(str::to_string),
            waiting: None,
            in_flight: None,
            latest: None,
            last_sent: None,
            memo: HashMap::new(),
            found: Vec::new(),
            status: Status::None,
            sent: Vec::new(),
            reverse_waiting: None,
            reverse_in_flight: None,
            address: None,
        }
    }

    /// Запрос по Enter. Ответ из памяти даёт место сразу.
    pub fn submit(&mut self, text: &str) -> Option<Place> {
        let query = geocode::normalize(text);
        if query.is_empty() {
            return None;
        }
        let key = (query.clone(), self.language.clone());
        self.latest = Some(key.clone());
        if let Some(places) = self.memo.get(&key).cloned() {
            return self.show(&key, places);
        }
        self.status = Status::Searching(query);
        self.waiting = Some(key);
        None
    }

    /// Адрес точки для «Сведения о точке», как `what_here` модуля: запрос
    /// ждёт той же очереди Nominatim.
    pub fn reverse(&mut self, lat: f64, lon: f64) {
        self.reverse_waiting = Some((lat, lon));
        self.address = None;
    }

    /// Шаг кадра: отправить ждущий запрос, когда подошла его очередь,
    /// и разобрать пришедший ответ. Возвращает место для перелёта.
    pub fn step(&mut self, now: f64, request: &dyn net::Request) -> Option<Place> {
        let mut fly = None;
        if let Some(answer) = request.take() {
            // Ответ на адрес точки.
            if let Some(point) = self.reverse_in_flight.take() {
                self.address = Some((
                    point,
                    answer.map(|bytes| {
                        let data: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
                        geocode::parse_reverse(&data)
                    }),
                ));
                return None;
            }
            let key = self.in_flight.take();
            match answer {
                Ok(bytes) => {
                    let data: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
                    let places = geocode::parse_places(&data);
                    if let Some(key) = key {
                        self.memo.insert(key.clone(), places.clone());
                        if self.latest.as_ref() == Some(&key) {
                            fly = self.show(&key, places);
                        }
                    }
                }
                Err(e) => self.status = Status::Failed(e),
            }
        }
        let ready = self.last_sent.is_none_or(|t| now - t >= SEARCH_INTERVAL);
        if self.in_flight.is_none()
            && self.reverse_in_flight.is_none()
            && ready
            && let Some((lat, lon)) = self.reverse_waiting.take()
        {
            request.start(&geocode::reverse_url(lat, lon, self.language.as_deref(), geocode::REVERSE_ZOOM));
            self.last_sent = Some(now);
            self.sent.push(now);
            self.reverse_in_flight = Some((lat, lon));
            return fly;
        }
        if self.in_flight.is_none()
            && self.reverse_in_flight.is_none()
            && ready
            && let Some(key) = self.waiting.take()
        {
            if let Some(places) = self.memo.get(&key).cloned() {
                return self.show(&key, places).or(fly);
            }
            request.start(&geocode::search_url(&key.0, key.1.as_deref(), SEARCH_LIMIT));
            self.last_sent = Some(now);
            self.sent.push(now);
            self.in_flight = Some(key);
        }
        fly
    }

    /// Запрос ждёт очереди или ответа.
    pub fn busy(&self) -> bool {
        self.waiting.is_some() || self.in_flight.is_some() || self.reverse_waiting.is_some() || self.reverse_in_flight.is_some()
    }

    fn show(&mut self, key: &Key, places: Vec<Place>) -> Option<Place> {
        self.status = if places.is_empty() { Status::NotFound(key.0.clone()) } else { Status::None };
        self.found = places;
        self.found.first().cloned()
    }
}

/// Поза камеры над найденным местом: охват места целиком в кадре,
/// взгляд отвесный, север вверху.
pub fn place_pose(place: &Place, fov_y: f64, aspect: f64) -> Pose {
    let distance = place
        .bbox
        .map_or(SEARCH_MIN_DISTANCE, |(w, s, e, n)| fit_view(w, s, e, n, fov_y, aspect).2)
        .clamp(SEARCH_MIN_DISTANCE, SEARCH_MAX_DISTANCE);
    Pose::new(place.lat, place.lon, distance, 0.0, 0.0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::cell::RefCell;
    use std::collections::VecDeque;

    use super::{Search, Status, place_pose};
    use net::{Request, Stats};

    /// Подставной Nominatim: помнит адреса, отвечает по команде.
    #[derive(Default)]
    struct Fake {
        urls: RefCell<Vec<String>>,
        answers: RefCell<VecDeque<Result<Vec<u8>, String>>>,
        stats: Stats,
    }

    impl Request for Fake {
        fn start(&self, url: &str) {
            self.urls.borrow_mut().push(url.to_string());
        }
        fn take(&self) -> Option<Result<Vec<u8>, String>> {
            self.answers.borrow_mut().pop_front()
        }
        fn stats(&self) -> &Stats {
            &self.stats
        }
    }

    const PERM: &str = r#"[{"lat":"58.0103211","lon":"56.2341778","name":"Пермь",
        "display_name":"Пермь, Пермский край, Россия",
        "boundingbox":["57.8637","58.1766","55.8046","56.6573"]}]"#;

    #[test]
    fn not_more_than_once_a_second() {
        // Новый запрос каждые 0.2 с в течение 5 с, ответ приходит через
        // кадр после отправки. К Nominatim уходит не чаще раза в секунду.
        let fake = Fake::default();
        let mut s = Search::new(Some("ru"));
        let mut answered = 0;
        for i in 0..100 {
            let t = f64::from(i) * 0.05;
            if i % 4 == 0 {
                s.submit(&format!("место {i}"));
            }
            if fake.urls.borrow().len() > answered {
                fake.answers.borrow_mut().push_back(Ok(b"[]".to_vec()));
                answered += 1;
            }
            s.step(t, &fake);
        }
        let gaps: Vec<f64> = s.sent.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.iter().all(|g| *g >= 1.0 - 1e-9), "{gaps:?}");
        assert!(s.sent.len() >= 4, "{:?}", s.sent);
    }

    #[test]
    fn answer_flies_to_first_place_and_is_remembered() {
        let fake = Fake::default();
        let mut s = Search::new(Some("ru"));
        assert!(s.submit("  Пермь ").is_none());
        assert_eq!(s.status, Status::Searching("Пермь".into()));
        assert!(s.step(0.0, &fake).is_none());
        assert_eq!(fake.urls.borrow().len(), 1);
        assert!(fake.urls.borrow()[0].contains("accept-language=ru"));
        fake.answers.borrow_mut().push_back(Ok(PERM.as_bytes().to_vec()));
        let place = s.step(0.1, &fake).unwrap();
        assert_eq!(place.name, "Пермь");
        // Второй раз - из памяти, без запроса.
        assert_eq!(s.submit("Пермь").unwrap().name, "Пермь");
        s.step(5.0, &fake);
        assert_eq!(fake.urls.borrow().len(), 1);
    }

    #[test]
    fn empty_answer_and_failure_are_reported() {
        let fake = Fake::default();
        let mut s = Search::new(None);
        s.submit("Нигдеград");
        s.step(0.0, &fake);
        fake.answers.borrow_mut().push_back(Ok(b"{not json".to_vec()));
        assert!(s.step(0.1, &fake).is_none());
        assert_eq!(s.status, Status::NotFound("Нигдеград".into()));
        s.submit("Пермь");
        s.step(2.0, &fake);
        fake.answers.borrow_mut().push_back(Err("сеть".into()));
        s.step(2.1, &fake);
        assert_eq!(s.status, Status::Failed("сеть".into()));
        assert!(!fake.urls.borrow()[1].contains("accept-language"));
    }

    #[test]
    fn pose_fits_the_place() {
        let place = data::geocode::parse_places(&serde_json::from_str(PERM).unwrap()).remove(0);
        let pose = place_pose(&place, 45.0, 1.6);
        assert!((super::SEARCH_MIN_DISTANCE..=super::SEARCH_MAX_DISTANCE).contains(&pose.distance));
        assert!(pose.distance > 20_000.0);
        let mut point = place.clone();
        point.bbox = None;
        assert_eq!(place_pose(&point, 45.0, 1.6).distance, super::SEARCH_MIN_DISTANCE);
    }
}
