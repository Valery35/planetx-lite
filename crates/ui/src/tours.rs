// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Туры: записанный тур метки и тур по папке, как в модуле
//! (`ui/globe/pictures.py`, `_tour_stops`, `place_stop`).
//!
//! Тур по папке идёт по отмеченным меткам папки и вложенных папок
//! в порядке списка. Путь - остановка-путь, камера едет вдоль линии.
//! У метки с видом камера встаёт в её вид, точка без вида - с 3 км,
//! многоугольник - целиком в кадре. Записанный тур (gx:Tour) - позы
//! камеры по времени. Поза тура - функция времени тура, проигрыватель
//! только ведёт часы тура: пуск, пауза, продолжение.

use globe_core::navigation::Pose;
use scene::features::Kind;
use scene::kml::{Folder, Item, Place};
use scene::tour::{PATH_TILT, PAUSE, Stop, Tour};

use crate::marks;

/// Остановка тура по метке. along - путь проезжается вдоль линии.
/// None у метки без точек.
pub fn place_stop(place: &Place, along: bool, fov_y: f64, aspect: f64) -> Option<Stop> {
    let mut stop = if along && place.kind == Kind::Line && place.points.len() > 1 {
        Stop::path(&place.name, &place.points, PATH_TILT)
    } else {
        let pose = marks::place_pose(place, fov_y, aspect)?;
        Stop::new(&place.name, pose.lat, pose.lon, pose.distance, pose.heading, pose.tilt)
    };
    stop.time = place.view_time.clone().or_else(|| place.time.clone());
    stop.description = place.description.clone();
    stop.layers = place.layers.clone();
    if let Some((pause, speed)) = place.pace {
        stop.pause = pause;
        stop.speed = speed;
    }
    Some(stop)
}

/// Остановки тура по папке: отмеченные метки папки и вложенных
/// отмеченных папок в порядке списка, без записанных туров.
pub fn folder_stops(folder: &Folder, fov_y: f64, aspect: f64) -> Vec<Stop> {
    let mut out = Vec::new();
    collect(folder, fov_y, aspect, &mut out);
    out
}

fn collect(folder: &Folder, fov_y: f64, aspect: f64, out: &mut Vec<Stop>) {
    for item in &folder.children {
        match item {
            Item::Folder(f) if f.visible => collect(f, fov_y, aspect, out),
            Item::Place(p) if p.visible && p.tour.is_none() => {
                if let Some(stop) = place_stop(p, true, fov_y, aspect) {
                    out.push(stop);
                }
            }
            _ => {}
        }
    }
}

/// Записанный тур метки, None у метки без тура.
pub fn recorded_stop(place: &Place) -> Option<Stop> {
    let samples = place.tour.as_ref().filter(|s| !s.is_empty())?;
    let (bounce, updates) = place.tour_extra.as_ref().map_or((Vec::new(), Vec::new()), |e| (e.bounce.clone(), e.updates.clone()));
    Some(Stop::recorded(&place.name, samples, &bounce, updates))
}

/// Проигрыватель тура: часы тура, пауза, продолжение.
pub struct Player {
    pub tour: Tour,
    pub name: String,
    /// Время часов окна, когда тур был в начале.
    start: f64,
    /// Время тура на паузе.
    paused: Option<f64>,
}

impl Player {
    /// Тур от позы start по остановкам, пауза на остановке - [`PAUSE`].
    pub fn new(name: &str, start_pose: &Pose, stops: Vec<Stop>, fov_y: f64, now: f64) -> Option<Self> {
        if stops.is_empty() {
            return None;
        }
        let tour = Tour::with_fov(start_pose, stops, PAUSE, fov_y);
        Some(Self { tour, name: name.to_string(), start: now, paused: None })
    }

    /// Время тура.
    pub fn time(&self, now: f64) -> f64 {
        self.paused.unwrap_or(now - self.start).clamp(0.0, self.tour.duration)
    }

    pub fn paused(&self) -> bool {
        self.paused.is_some()
    }

    /// Пауза или продолжение с того же места.
    pub fn toggle(&mut self, now: f64) {
        match self.paused.take() {
            Some(t) => self.start = now - t,
            None => self.paused = Some((now - self.start).clamp(0.0, self.tour.duration)),
        }
    }

    /// Тур дошёл до конца.
    pub fn done(&self, now: f64) -> bool {
        self.paused.is_none() && now - self.start >= self.tour.duration
    }

    /// Поза камеры сейчас.
    pub fn pose(&self, now: f64) -> Pose {
        self.tour.pose_at(self.time(now))
    }

    /// Подпись остановки и её непрозрачность.
    pub fn caption(&self, now: f64) -> Option<(&str, f64)> {
        let (i, alpha) = self.tour.caption_at(self.time(now))?;
        Some((self.tour.stops.get(i)?.name.as_str(), alpha))
    }

    /// Остановка, к которой летит или на которой стоит камера.
    pub fn index(&self, now: f64) -> Option<usize> {
        self.tour.index_at(self.time(now))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{Player, folder_stops, place_stop, recorded_stop};
    use globe_core::navigation::Pose;
    use scene::features::Kind;
    use scene::kml::{Folder, Item, Place};

    fn folder() -> Folder {
        let mut f = Folder::new("Прогулка");
        f.children.push(Item::Place(Place::new("Вокзал", Kind::Point, vec![(58.0, 56.2)])));
        let mut hidden = Place::new("Скрыта", Kind::Point, vec![(58.1, 56.2)]);
        hidden.visible = false;
        f.children.push(Item::Place(hidden));
        f.children.push(Item::Place(Place::new("Маршрут", Kind::Line, vec![(58.0, 56.2), (58.01, 56.25)])));
        f
    }

    #[test]
    fn folder_tour_goes_by_checked_places_in_order() {
        let stops = folder_stops(&folder(), 45.0, 1.6);
        let names: Vec<&str> = stops.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Вокзал", "Маршрут"]);
        // Путь проезжается: у остановки-пути есть проезд.
        assert!(stops[1].glide > 0.0);
    }

    #[test]
    fn player_pauses_and_ends() {
        let stops = folder_stops(&folder(), 45.0, 1.6);
        let start = Pose::new(58.0, 56.2, 2.0e7, 0.0, 0.0);
        let mut p = Player::new("Прогулка", &start, stops, 45.0, 100.0).unwrap();
        let d = p.tour.duration;
        assert!(d > 0.0 && !p.done(100.0));
        p.toggle(101.0);
        assert!(p.paused());
        // На паузе время тура не идёт.
        assert!((p.time(500.0) - 1.0).abs() < 1e-9);
        p.toggle(500.0);
        assert!((p.time(502.0) - 3.0).abs() < 1e-9);
        assert!(p.done(500.0 + d));
        assert!(Player::new("пусто", &start, Vec::new(), 45.0, 0.0).is_none());
    }

    #[test]
    fn stop_takes_time_and_pace() {
        let mut place = Place::new("Театр", Kind::Point, vec![(58.0, 56.2)]);
        place.time = Some(("2026-09-30".into(), "2026-09-30".into()));
        place.pace = Some((Some(7.0), 2.0));
        let stop = place_stop(&place, true, 45.0, 1.6).unwrap();
        assert_eq!(stop.time, place.time);
        assert_eq!((stop.pause, stop.speed), (Some(7.0), 2.0));
        assert!(recorded_stop(&place).is_none());
    }

    #[test]
    fn perm_demo_recorded_tour_plays() {
        let Ok(bytes) = std::fs::read(r"C:\Dev\planetx\planetx\demo\perm.planetx") else {
            eprintln!("демо модуля нет, проверка пропущена");
            return;
        };
        let opened = crate::marks::open(&bytes, "perm.planetx").unwrap();
        let mut tours = Vec::new();
        fn find(f: &Folder, out: &mut Vec<Place>) {
            for item in &f.children {
                match item {
                    Item::Folder(g) => find(g, out),
                    Item::Place(p) if p.tour.is_some() => out.push(p.clone()),
                    _ => {}
                }
            }
        }
        find(&opened.tree, &mut tours);
        assert_eq!(tours.len(), 1);
        let stop = recorded_stop(&tours[0]).unwrap();
        let start = opened.pose.unwrap();
        let p = Player::new(&tours[0].name, &start, vec![stop], 45.0, 0.0).unwrap();
        assert!(p.tour.duration > 5.0, "{}", p.tour.duration);
        // Поза в конце - последняя поза записи.
        let end = p.pose(p.tour.duration + 1.0);
        let last = tours[0].tour.as_ref().unwrap().last().copied().unwrap();
        assert!((end.lat - last[1]).abs() < 1e-6 && (end.lon - last[2]).abs() < 1e-6);
    }
}
