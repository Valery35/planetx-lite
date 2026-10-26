// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! «Моё место»: место телефона или компьютера по геолокации браузера
//! на глобусе, камера следом и запись трека. Просьба автора от 26
//! октября 2026 года, у модуля этого нет.
//!
//! Место даёт `navigator.geolocation.watchPosition` страницы. Браузер
//! при первом включении спрашивает разрешение, место никуда не уходит,
//! только на глобус. У программы на компьютере службы места нет -
//! кнопки нет. Первое место - перелёт к нему, потом камера идёт следом,
//! пока флажок стоит. Трек пишется по местам точнее [`MAX_ACCURACY`]
//! метров, когда сдвиг больше [`STEP_METRES`] или погрешности. Трек
//! сохраняется путём в «Мои метки» со временем от первой до последней
//! точки - по шкале времени он растёт.

use eframe::egui;
use scene::features::Kind;
use scene::kml::Place;

use crate::i18n::{Lang, tr, trf};

/// Места грубее, метры, в трек не идут.
pub const MAX_ACCURACY: f64 = 100.0;
/// Наименьший сдвиг для новой точки трека, метры.
pub const STEP_METRES: f64 = 3.0;
/// Расстояние камеры до места при первом перелёте, метры.
pub const FIX_DISTANCE: f64 = 3000.0;
/// Цвет точки места и трека.
pub const COLOUR: [u8; 4] = [30, 140, 255, 255];
/// Средний радиус Земли для длины трека, метры.
const EARTH_RADIUS: f64 = 6_371_008.8;

/// Место: широта, долгота, высота, погрешность в метрах, время
/// в секундах от 1970 года.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fix {
    pub lat: f64,
    pub lon: f64,
    pub alt: Option<f64>,
    pub accuracy: f64,
    pub t: f64,
}

/// Расстояние по дуге большого круга, метры.
pub fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (p1, p2) = (a.0.to_radians(), b.0.to_radians());
    let dp = p2 - p1;
    let dl = (b.1 - a.1).to_radians();
    let h = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS * h.sqrt().min(1.0).asin()
}

/// Пришедшие места и ошибки.
type Inbox = std::rc::Rc<std::cell::RefCell<Vec<Result<Fix, String>>>>;

/// Что просит окно «Моё место».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Сохранить трек в «Мои метки».
    Save,
}

#[derive(Default)]
pub struct Gps {
    /// Слежение за местом включено.
    pub on: bool,
    pub open: bool,
    pub follow: bool,
    pub recording: bool,
    pub fix: Option<Fix>,
    pub track: Vec<Fix>,
    pub error: Option<String>,
    /// К первому месту уже летали.
    pub flown: bool,
    /// Место или трек сменились: точку и линию рисовать заново.
    pub dirty: bool,
    inbox: Inbox,
    #[cfg(target_arch = "wasm32")]
    watch: Option<web::Watch>,
}

impl Gps {
    /// Есть ли служба места: только у страницы с `navigator.geolocation`.
    pub fn available() -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            web::available()
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            false
        }
    }

    /// Включить слежение: браузер спросит разрешение.
    pub fn start(&mut self, ctx: &egui::Context) {
        if self.on {
            return;
        }
        self.on = true;
        self.open = true;
        self.follow = true;
        self.flown = false;
        self.error = None;
        #[cfg(target_arch = "wasm32")]
        {
            self.watch = web::Watch::start(self.inbox.clone(), ctx.clone());
            if self.watch.is_none() {
                self.error = Some("geolocation".into());
                self.on = false;
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = ctx;
            self.on = false;
        }
    }

    /// Выключить слежение. Трек остаётся до сохранения или очистки.
    pub fn stop(&mut self) {
        self.on = false;
        self.recording = false;
        #[cfg(target_arch = "wasm32")]
        {
            self.watch = None;
        }
        self.dirty = true;
    }

    /// Новое место: в трек, если запись идёт и место точное и сдвинулось.
    /// true - место в треке.
    pub fn add(&mut self, fix: Fix) -> bool {
        self.fix = Some(fix);
        self.dirty = true;
        if !self.recording || !(fix.accuracy.is_finite() && fix.accuracy <= MAX_ACCURACY) {
            return false;
        }
        let far = self.track.last().is_none_or(|last| distance((last.lat, last.lon), (fix.lat, fix.lon)) >= STEP_METRES.max(fix.accuracy.min(MAX_ACCURACY) / 2.0));
        if far {
            self.track.push(fix);
        }
        far
    }

    /// Пришедшие места за кадр. Возвращает последнее новое место.
    pub fn step(&mut self) -> Option<Fix> {
        let arrived: Vec<Result<Fix, String>> = self.inbox.borrow_mut().drain(..).collect();
        let mut last = None;
        for item in arrived {
            match item {
                Ok(fix) => {
                    self.error = None;
                    self.add(fix);
                    last = Some(fix);
                }
                Err(e) => self.error = Some(e),
            }
        }
        last
    }

    /// Длина трека, метры.
    pub fn length(&self) -> f64 {
        self.track.windows(2).map(|w| distance((w[0].lat, w[0].lon), (w[1].lat, w[1].lon))).sum()
    }

    /// Трек путём для «Моих меток»: время от первой до последней точки,
    /// высоты точек, если они есть у всех.
    pub fn track_place(&self, lang: Lang) -> Option<Place> {
        let (first, last) = (self.track.first()?, self.track.last()?);
        if self.track.len() < 2 {
            return None;
        }
        let begin = scene::when::text(first.t);
        let name = trf(lang, "GPS track {time}", &[("time", begin.get(..16).unwrap_or(&begin).replace('T', " ").as_str())]);
        let mut place = Place::new(&name, Kind::Line, self.track.iter().map(|f| (f.lat, f.lon)).collect());
        place.color = COLOUR;
        place.width = 3.0;
        place.time = Some((begin, scene::when::text(last.t)));
        let minutes = ((last.t - first.t) / 60.0).round();
        place.description = trf(lang, "Length {km} km, time {minutes} min, points {count}.", &[("km", &format!("{:.2}", self.length() / 1000.0)), ("minutes", &minutes.to_string()), ("count", &self.track.len().to_string())]);
        if self.track.iter().all(|f| f.alt.is_some()) {
            place.alts = Some(self.track.iter().map(|f| f.alt.unwrap_or(0.0)).collect());
            place.mode = "absolute".into();
        }
        Some(place)
    }

    /// Окно «Моё место».
    pub fn window(&mut self, ctx: &egui::Context, lang: Lang) -> Option<Action> {
        if !self.open {
            return None;
        }
        let mut open = true;
        let mut action = None;
        let mut start = false;
        let mut stop = false;
        egui::Window::new(tr(lang, "My location")).open(&mut open).default_width(300.0).show(ctx, |ui| {
            match (&self.fix, &self.error) {
                (_, Some(e)) => {
                    ui.label(trf(lang, "The location is not available: {error}", &[("error", e)]));
                }
                (Some(f), None) => {
                    ui.label(format!("{:.6}, {:.6}", f.lat, f.lon));
                    let mut text = trf(lang, "Accuracy {m} m", &[("m", &format!("{:.0}", f.accuracy))]);
                    if let Some(alt) = f.alt {
                        text.push_str(", ");
                        text.push_str(&trf(lang, "height {m} m", &[("m", &format!("{alt:.0}"))]));
                    }
                    ui.label(text);
                }
                (None, None) if self.on => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.weak(tr(lang, "Waiting for the location…"));
                    });
                }
                _ => {}
            }
            ui.checkbox(&mut self.follow, tr(lang, "Camera follows")).on_hover_text(tr(lang, "The camera keeps the location in the middle, the distance and the tilt stay."));
            let before = self.recording;
            ui.checkbox(&mut self.recording, tr(lang, "Record a track")).on_hover_text(tr(lang, "Locations go into the track while the tick stands. Rough locations are skipped."));
            if self.recording && !before && !self.on {
                start = true;
            }
            if !self.track.is_empty() {
                ui.label(trf(lang, "Track: points {count}, {km} km.", &[("count", &self.track.len().to_string()), ("km", &format!("{:.2}", self.length() / 1000.0))]));
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(self.track.len() >= 2, egui::Button::new(tr(lang, "Save track"))).on_hover_text(tr(lang, "The track goes as a path to My Places, with the time from the first to the last point.")).clicked() {
                    action = Some(Action::Save);
                }
                if !self.track.is_empty() && ui.button(tr(lang, "Clear track")).clicked() {
                    self.track.clear();
                    self.dirty = true;
                }
                if self.on {
                    stop = ui.button(tr(lang, "Turn off")).clicked();
                } else {
                    start = ui.button(tr(lang, "Turn on")).clicked();
                }
            });
            ui.add(egui::Label::new(egui::RichText::new(tr(lang, "The location is given by the browser and goes only to the globe.")).small()).wrap());
        });
        if start {
            self.start(ctx);
        }
        if stop {
            self.stop();
        }
        if !open {
            self.open = false;
        }
        action
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    //! Слежение за местом через `navigator.geolocation` браузера.

    use js_sys::{Function, Object, Reflect};
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast, JsValue};

    use super::{Fix, Inbox};

    fn get(target: &JsValue, key: &str) -> Option<JsValue> {
        Reflect::get(target, &key.into()).ok().filter(|v| !v.is_undefined() && !v.is_null())
    }

    fn geolocation() -> Option<JsValue> {
        get(&get(&js_sys::global(), "navigator")?, "geolocation")
    }

    pub fn available() -> bool {
        geolocation().is_some()
    }

    /// Подписка watchPosition: снимается при drop.
    pub struct Watch {
        id: JsValue,
        _ok: Closure<dyn FnMut(JsValue)>,
        _err: Closure<dyn FnMut(JsValue)>,
    }

    impl Watch {
        pub fn start(inbox: Inbox, ctx: eframe::egui::Context) -> Option<Watch> {
            let geo = geolocation()?;
            let watch: Function = get(&geo, "watchPosition")?.dyn_into().ok()?;
            let (ok_inbox, ok_ctx) = (inbox.clone(), ctx.clone());
            let ok = Closure::<dyn FnMut(JsValue)>::new(move |pos: JsValue| {
                let coords = get(&pos, "coords");
                let num = |key: &str| coords.as_ref().and_then(|c| get(c, key)).and_then(|v| v.as_f64()).filter(|v| v.is_finite());
                let (Some(lat), Some(lon)) = (num("latitude"), num("longitude")) else {
                    return;
                };
                let t = get(&pos, "timestamp").and_then(|v| v.as_f64()).map_or_else(|| js_sys::Date::now() / 1000.0, |ms| ms / 1000.0);
                ok_inbox.borrow_mut().push(Ok(Fix { lat, lon, alt: num("altitude"), accuracy: num("accuracy").unwrap_or(f64::INFINITY), t }));
                ok_ctx.request_repaint();
            });
            let err = Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
                let text = get(&e, "message").and_then(|m| m.as_string()).unwrap_or_else(|| "geolocation".into());
                inbox.borrow_mut().push(Err(text));
                ctx.request_repaint();
            });
            let options = Object::new();
            let _ = Reflect::set(&options, &"enableHighAccuracy".into(), &true.into());
            let _ = Reflect::set(&options, &"maximumAge".into(), &JsValue::from_f64(5000.0));
            let _ = Reflect::set(&options, &"timeout".into(), &JsValue::from_f64(30000.0));
            let id = watch.call3(&geo, ok.as_ref(), err.as_ref(), &options).ok()?;
            Some(Watch { id, _ok: ok, _err: err })
        }
    }

    impl Drop for Watch {
        fn drop(&mut self) {
            if let Some(geo) = geolocation()
                && let Some(clear) = get(&geo, "clearWatch").and_then(|f| f.dyn_into::<Function>().ok())
            {
                let _ = clear.call1(&geo, &self.id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn fix(lat: f64, lon: f64, accuracy: f64, t: f64) -> Fix {
        Fix { lat, lon, alt: None, accuracy, t }
    }

    #[test]
    fn track_takes_precise_moved_points() {
        let mut g = Gps::default();
        // Без записи место только показывается.
        assert!(!g.add(fix(58.0, 56.0, 5.0, 0.0)));
        g.recording = true;
        assert!(g.add(fix(58.0, 56.0, 5.0, 1.0)));
        // Сдвиг на 1 м - не точка.
        assert!(!g.add(fix(58.000009, 56.0, 5.0, 2.0)));
        // Грубое место - не точка.
        assert!(!g.add(fix(58.01, 56.0, 500.0, 3.0)));
        assert!(g.add(fix(58.001, 56.0, 5.0, 60.0)));
        assert_eq!(g.track.len(), 2);
        assert!((g.length() - 111.2).abs() < 1.0, "{}", g.length());
        let place = g.track_place(Lang::EN).unwrap();
        assert_eq!(place.points.len(), 2);
        let (begin, end) = place.time.unwrap();
        assert_eq!(scene::when::parse(&begin), Some(1.0));
        assert_eq!(scene::when::parse(&end), Some(60.0));
        assert!(place.alts.is_none());
    }

    #[test]
    fn program_has_no_location_service() {
        assert!(!Gps::available());
        let mut g = Gps::default();
        g.start(&egui::Context::default());
        assert!(!g.on);
    }
}
