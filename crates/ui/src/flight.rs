// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Режим полёта: свой самолёт с клавиатуры, пункт «Полёт» меню
//! на глобусе. Перенос `FlightSim` (`ui/flightsim.py`) и индикатора на
//! стекле (`ui/flighthud.py`) модуля, ядро - `data::flightsim`.
//!
//! Клавиши на виде: стрелка вверх - нос вниз, вниз - нос вверх, влево
//! и вправо - крен, PageUp и PageDown - тяга, V - вид из кабины и
//! обратно, H - приборы, Esc - выход. Камера идёт следом сзади и сверху
//! (`data::movers::chase_on`), колесо меняет расстояние. Из кабины глаз
//! стоит в самолёте, взгляд - по курсу с тангажом и креном самолёта:
//! точка взгляда - в воздухе в [`COCKPIT_LOOK`] метрах впереди, наклон
//! 90° плюс тангаж, как в имитаторе Google Earth Pro.

use data::flightsim::{self, Controls, Plane};
use eframe::egui;
use globe_core::DVec3;

use crate::i18n::{Lang, tr, trf};

/// Цвет условной модели самолёта, `COLOR` модуля.
pub const COLOR: [f64; 3] = [0.96, 0.45, 0.15];
/// Метров от камеры до самолёта при старте, наклон камеры.
pub const CHASE_REACH: f64 = 150.0;
pub const CHASE_TILT: f64 = 75.0;
/// Метров от глаза до точки взгляда в кабине.
pub const COCKPIT_LOOK: f64 = 1000.0;
/// Знак крена позы вида при крене самолёта вправо.
pub const COCKPIT_ROLL: f64 = -1.0;
/// Секунд надписи о касании земли.
pub const END_SHOW: f64 = 3.0;
/// Шаги и поля лент индикатора, как у модуля.
const LADDER_STEP: f64 = 5.0;
const LADDER_REACH: f64 = 25.0;
const HEADING_STEP: f64 = 5.0;
const HEADING_HALF: f64 = 30.0;
const SPEED_STEP: f64 = 20.0;
const SPEED_HALF: f64 = 100.0;
const ALT_STEP: f64 = 100.0;
const ALT_HALF: f64 = 500.0;
/// Логических пикселей кадра на масштаб 1.
const BASE: f32 = 800.0;

/// Полёт своего самолёта.
pub struct Flight {
    pub plane: Plane,
    pub cockpit: bool,
    pub hud: bool,
    /// Расстояние камеры до самолёта.
    pub reach: f64,
    last: f64,
    /// Момент касания земли.
    ended: Option<f64>,
    /// Наклон камеры сзади до входа в кабину.
    tilt: f64,
    /// Модель надо поставить в видеокарту.
    pub fresh: bool,
}

/// Что окно делает с камерой после шага.
pub enum Camera {
    /// Сзади: точка взгляда, расстояние, азимут, наклон - через show.
    Chase(f64, f64, f64, f64, f64),
    /// Из кабины: точка взгляда в воздухе на высоте h, расстояние,
    /// азимут, наклон, крен.
    Cockpit { lat: f64, lon: f64, h: f64, distance: f64, heading: f64, tilt: f64, roll: f64 },
}

impl Flight {
    /// Полёт с точки на START_HEIGHT над поверхностью ground, курс -
    /// азимут вида, kind - вид самолёта.
    pub fn start(lat: f64, lon: f64, ground: f64, heading: f64, kind: &str, now: f64) -> Flight {
        let plane = Plane::new(lat, lon, ground + flightsim::START_HEIGHT, heading, None, Some(kind));
        Flight { plane, cockpit: false, hud: true, reach: CHASE_REACH, last: now, ended: None, tilt: CHASE_TILT, fresh: true }
    }

    /// Шаг полёта: клавиши, физика, касание. Возвращает false, когда
    /// полёт кончился и режим надо снять.
    pub fn step(&mut self, now: f64, controls: Controls, ground: f64) -> bool {
        let dt = (now - self.last).clamp(0.0, 0.5);
        self.last = now;
        if let Some(at) = self.ended {
            return now - at < END_SHOW;
        }
        flightsim::step(&mut self.plane, dt, controls, ground);
        if self.plane.crashed {
            self.ended = Some(now);
        }
        true
    }

    /// Касание земли было.
    pub fn crashed(&self) -> bool {
        self.ended.is_some()
    }

    /// Вид из кабины или сзади. tilt - нынешний наклон камеры.
    pub fn set_cockpit(&mut self, on: bool, tilt: f64) {
        if on == self.cockpit {
            return;
        }
        if on {
            self.tilt = tilt;
        }
        self.cockpit = on;
        self.fresh = true;
    }

    /// Наклон камеры сзади после выхода из кабины.
    pub fn chase_tilt(&self) -> f64 {
        self.tilt
    }

    /// Камера на кадр. alt - высота самолёта на экране, ground - высота
    /// поверхности на экране в точке, tilt - нынешний наклон.
    pub fn camera(&self, alt: f64, ground: &dyn Fn(f64, f64) -> f64, tilt: f64) -> Camera {
        let p = &self.plane;
        if self.cockpit {
            let rad = p.pitch.to_radians();
            let (la, lo) = flightsim::destination(p.lat, p.lon, p.heading, COCKPIT_LOOK * rad.cos(), p.radius);
            let back = data::trains::bearing(la, lo, p.lat, p.lon);
            return Camera::Cockpit { lat: la, lon: lo, h: alt + COCKPIT_LOOK * rad.sin(), distance: COCKPIT_LOOK, heading: (back + 180.0) % 360.0, tilt: 90.0 + p.pitch, roll: COCKPIT_ROLL * p.roll };
        }
        let (la, lo, look, distance) = data::movers::chase_on(p.lat, p.lon, p.heading, alt, ground, self.reach, tilt);
        Camera::Chase(la, lo, distance, look, tilt)
    }

    /// Колесо сзади: расстояние до самолёта.
    pub fn wheel(&mut self, notches: f64) {
        if !self.cockpit {
            self.reach = (self.reach * crate::live::WHEEL_STEP.powf(notches)).clamp(data::movers::CHASE_MIN, crate::live::MAX_REACH);
        }
    }

    /// Модель самолёта для видеокарты: условная модель `unit_plane_parts`.
    pub fn mesh(&self) -> render::ModelMesh {
        let base = crate::live::base();
        let parts = data::aircraft::unit_plane_parts(COLOR)
            .into_iter()
            .filter(|p| p.positions.len() >= 3)
            .map(|p| {
                let pos: Vec<DVec3> = p.positions.iter().map(|v| DVec3::from_array(*v)).collect();
                let normals = scene::models::flat_normals(&pos);
                let f = |v: DVec3| [v.x as f32, v.y as f32, v.z as f32];
                render::ModelPart {
                    positions: pos.iter().map(|v| f(base * *v)).collect(),
                    normals: normals.iter().map(|n| f((base * *n).normalize_or_zero())).collect(),
                    uv: Vec::new(),
                    image: None,
                    rgba: p.rgba.map(|c| c as f32),
                    track: None,
                }
            })
            .collect();
        render::ModelMesh { center: DVec3::ZERO, parts, images: Vec::new(), sun: [0.0, 0.0, 1.0], orient: globe_core::DMat3::IDENTITY }
    }

    /// Поворот модели на кадр.
    pub fn orient(&self) -> globe_core::DMat3 {
        let p = &self.plane;
        let length = p.spec.length;
        scene::models::moved(&crate::live::base(), p.lat, p.lon, p.heading, p.pitch, p.roll, [length, length, length])
    }

    /// Строки показаний внизу вида или надпись о касании.
    pub fn lines(&self, lang: Lang, ground: f64) -> Vec<String> {
        if self.crashed() {
            return vec![tr(lang, "Touched the ground - the flight is over.").to_string()];
        }
        let keys = tr(lang, "↑ ↓ - nose down and up · ← → - bank · PageUp, PageDown - throttle · V - cockpit view · H - instruments · Esc - exit").to_string();
        if self.cockpit && self.hud {
            return vec![keys];
        }
        let r = flightsim::readout(&self.plane, ground);
        vec![
            trf(
                lang,
                "{speed} km/h · altitude {alt} m, above ground {height} m · heading {heading}° · climb {climb} m/s · throttle {throttle} %",
                &[
                    ("speed", &format!("{:.0}", r.speed)),
                    ("alt", &format!("{:.0}", r.alt)),
                    ("height", &format!("{:.0}", r.height)),
                    ("heading", &format!("{:.0}", r.heading)),
                    ("climb", &format!("{:+.1}", r.climb)),
                    ("throttle", &format!("{:.0}", r.throttle)),
                ],
            ),
            keys,
        ]
    }

    /// Индикатор на стекле, `FlightHud` модуля: шкала тангажа с креном,
    /// знак самолёта, ленты курса, скорости и высоты, подъём, тяга
    /// и высота над землёй. focal - фокусное расстояние кадра
    /// в логических пикселях.
    pub fn hud(&self, painter: &egui::Painter, rect: egui::Rect, lang: Lang, ground: f64, focal: f32) {
        if !(self.cockpit && self.hud) || self.crashed() {
            return;
        }
        let r = flightsim::readout(&self.plane, ground);
        let shadow = egui::Color32::from_rgba_unmultiplied(0, 30, 0, 150);
        let green = egui::Color32::from_rgba_unmultiplied(90, 255, 110, 235);
        for (color, shift) in [(shadow, 1.2f32), (green, 0.0)] {
            self.draw_hud(painter, rect, lang, &r, focal, color, shift);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_hud(&self, painter: &egui::Painter, rect: egui::Rect, lang: Lang, r: &flightsim::Readout, focal: f32, color: egui::Color32, shift: f32) {
        let k = (rect.width().min(rect.height()) / BASE).clamp(0.6, 1.4);
        let stroke = egui::Stroke::new(1.6 * k, color);
        let font = egui::FontId::proportional((13.0 * k).max(9.0));
        let c = rect.center() + egui::vec2(shift, shift);
        let text = |at: egui::Pos2, align: egui::Align2, s: &str| {
            painter.text(at, align, s, font.clone(), color);
        };
        // Шкала тангажа, повёрнутая на крен, в поле между лентами.
        let clip = egui::Rect::from_min_max(egui::pos2(c.x - 225.0 * k, rect.top() + 70.0 * k), egui::pos2(c.x + 225.0 * k, rect.bottom() - 110.0 * k));
        let clipped = painter.with_clip_rect(clip);
        let (s, co) = (-(self.plane.roll as f32)).to_radians().sin_cos();
        let rot = |x: f32, y: f32| c + egui::vec2(x * co - y * s, x * s + y * co);
        let gap = 55.0 * k;
        for (mark, angle) in flightsim::ladder(self.plane.pitch, LADDER_STEP, LADDER_REACH) {
            let y = -(flightsim::ladder_offset(angle, f64::from(focal)) as f32);
            let half = if mark == 0.0 { 190.0 } else if (mark.round() as i64) % 10 == 0 { 110.0 } else { 85.0 } * k;
            let seg = |a: f32, b: f32| {
                if mark < 0.0 {
                    let mut x = a;
                    while x < b {
                        clipped.line_segment([rot(x, y), rot((x + 6.0 * k).min(b), y)], stroke);
                        x += 10.0 * k;
                    }
                } else {
                    clipped.line_segment([rot(a, y), rot(b, y)], stroke);
                }
            };
            seg(-half, -gap);
            seg(gap, half);
            if mark != 0.0 {
                let tip = if mark < 0.0 { 8.0 * k } else { -8.0 * k };
                clipped.line_segment([rot(-half, y), rot(-half, y - tip)], stroke);
                clipped.line_segment([rot(half, y), rot(half, y - tip)], stroke);
                if (mark.round() as i64) % 10 == 0 {
                    let label = format!("{:.0}", mark.abs());
                    clipped.text(rot(-half - 8.0 * k, y), egui::Align2::RIGHT_CENTER, &label, font.clone(), color);
                    clipped.text(rot(half + 8.0 * k, y), egui::Align2::LEFT_CENTER, &label, font.clone(), color);
                }
            }
        }
        // Знак самолёта - крылья и точка, неподвижен в середине.
        let (wing, drop) = (45.0 * k, 10.0 * k);
        let pts = [(-wing, 0.0), (-14.0 * k, 0.0), (-7.0 * k, drop), (0.0, 0.0), (7.0 * k, drop), (14.0 * k, 0.0), (wing, 0.0)];
        for w in pts.windows(2) {
            painter.line_segment([c + egui::vec2(w[0].0, w[0].1), c + egui::vec2(w[1].0, w[1].1)], stroke);
        }
        // Лента курса сверху.
        let top = rect.top() + 36.0 * k + shift;
        let scale = 6.0 * k;
        for (mark, offset) in flightsim::heading_tape(r.heading, HEADING_STEP, HEADING_HALF) {
            let x = c.x + offset as f32 * scale;
            let whole = (mark.round() as i64) % 10 == 0;
            painter.line_segment([egui::pos2(x, top), egui::pos2(x, top + if whole { 12.0 } else { 6.0 } * k)], stroke);
            if whole {
                let m = (mark.round() as i64).rem_euclid(360);
                let label = match m {
                    0 => "N".to_string(),
                    90 => "E".to_string(),
                    180 => "S".to_string(),
                    270 => "W".to_string(),
                    _ => format!("{:02}", m / 10),
                };
                text(egui::pos2(x, top + 22.0 * k), egui::Align2::CENTER_CENTER, &label);
            }
        }
        let boxed = egui::Rect::from_center_size(egui::pos2(c.x, top - 16.0 * k), egui::vec2(52.0 * k, 20.0 * k));
        painter.rect_stroke(boxed, 0.0, stroke, egui::StrokeKind::Middle);
        text(boxed.center(), egui::Align2::CENTER_CENTER, &format!("{:03.0}", r.heading.rem_euclid(360.0)));
        painter.line_segment([egui::pos2(c.x, top - 6.0 * k), egui::pos2(c.x, top)], stroke);
        // Ленты скорости слева и высоты справа.
        let tape = |x: f32, value: f64, step: f64, half: f64, left: bool, unit: &str| {
            let per = 120.0 * k / half as f32;
            let side = if left { -1.0 } else { 1.0 };
            painter.line_segment([egui::pos2(x, c.y - 120.0 * k), egui::pos2(x, c.y + 120.0 * k)], stroke);
            for (mark, offset) in flightsim::tape(value, step, half) {
                let y = c.y - offset as f32 * per;
                painter.line_segment([egui::pos2(x, y), egui::pos2(x + side * 10.0 * k, y)], stroke);
                if (y - c.y).abs() < 18.0 * k {
                    continue;
                }
                let align = if left { egui::Align2::RIGHT_CENTER } else { egui::Align2::LEFT_CENTER };
                text(egui::pos2(x + side * 14.0 * k, y), align, &format!("{mark:.0}"));
            }
            let b = egui::Rect::from_min_size(egui::pos2(if left { x - 78.0 * k } else { x + 6.0 * k }, c.y - 12.0 * k), egui::vec2(72.0 * k, 24.0 * k));
            painter.rect_stroke(b, 0.0, stroke, egui::StrokeKind::Middle);
            text(b.center(), egui::Align2::CENTER_CENTER, &format!("{value:.0}"));
            text(egui::pos2(x, c.y - 139.0 * k), egui::Align2::CENTER_CENTER, unit);
        };
        tape(c.x - 250.0 * k, r.speed, SPEED_STEP, SPEED_HALF, true, tr(lang, "km/h"));
        tape(c.x + 250.0 * k, r.alt, ALT_STEP, ALT_HALF, false, tr(lang, "m"));
        text(egui::pos2(c.x + 250.0 * k, c.y + 140.0 * k), egui::Align2::CENTER_CENTER, &trf(lang, "Climb {climb} m/s", &[("climb", &format!("{:+.1}", r.climb))]));
        // Тяга.
        let (tx, ty, th) = (c.x - 330.0 * k, c.y + 150.0 * k, 90.0 * k);
        painter.rect_stroke(egui::Rect::from_min_size(egui::pos2(tx - 6.0 * k, ty - th), egui::vec2(12.0 * k, th)), 0.0, stroke, egui::StrokeKind::Middle);
        let level = ty - th * (r.throttle.clamp(0.0, 100.0) / 100.0) as f32;
        painter.line_segment([egui::pos2(tx - 12.0 * k, level), egui::pos2(tx + 12.0 * k, level)], stroke);
        text(egui::pos2(tx, ty + 14.0 * k), egui::Align2::CENTER_CENTER, &trf(lang, "Throttle {throttle} %", &[("throttle", &format!("{:.0}", r.throttle))]));
        text(egui::pos2(c.x, rect.bottom() - 92.0 * k), egui::Align2::CENTER_CENTER, &trf(lang, "Above ground {height} m", &[("height", &format!("{:.0}", r.height))]));
    }
}

/// Клавиши полёта на кадр, `_keys` модуля: тангаж, крен, тяга.
pub fn controls(ctx: &egui::Context) -> Controls {
    ctx.input(|i| {
        let axis = |minus: egui::Key, plus: egui::Key| f64::from(i8::from(i.key_down(plus)) - i8::from(i.key_down(minus)));
        Controls {
            pitch: axis(egui::Key::ArrowUp, egui::Key::ArrowDown),
            roll: axis(egui::Key::ArrowLeft, egui::Key::ArrowRight),
            throttle: axis(egui::Key::PageDown, egui::Key::PageUp),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flight_runs_and_ends_on_the_ground() {
        let mut f = Flight::start(60.0, 30.0, 100.0, 90.0, flightsim::FIGHTER, 0.0);
        assert!((f.plane.alt - 1100.0).abs() < 1e-9);
        let level = Controls { pitch: 0.0, roll: 0.0, throttle: 0.0 };
        for n in 1..20 {
            assert!(f.step(f64::from(n) * 0.1, level, 100.0));
        }
        assert!(f.plane.lon > 30.0);
        let flat = |_: f64, _: f64| 100.0;
        assert!(matches!(f.camera(f.plane.alt, &flat, 75.0), Camera::Chase(..)));
        f.set_cockpit(true, 75.0);
        assert!(matches!(f.camera(f.plane.alt, &flat, 75.0), Camera::Cockpit { .. }));
        assert!(!f.mesh().parts.is_empty());
        assert_eq!(f.lines(Lang::EN, 100.0).len(), 1);
        // Нос вниз до земли - касание, потом показ надписи и конец.
        let dive = Controls { pitch: -1.0, roll: 0.0, throttle: 0.0 };
        let mut t = 2.0;
        while !f.crashed() && t < 600.0 {
            f.step(t, dive, 100.0);
            t += 0.1;
        }
        assert!(f.crashed());
        assert!(f.step(t + 1.0, dive, 100.0));
        assert!(!f.step(t + END_SHOW + 1.0, dive, 100.0));
    }
}
