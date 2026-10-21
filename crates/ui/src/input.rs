// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Мышь и клавиши, раскладка Google Earth, как в модуле
//! (`render/view.py`).
//!
//! - Левая кнопка - захват Земли, бросок даёт инерцию.
//! - Колесо - плавное приближение к точке под курсором.
//! - Средняя кнопка или Shift с левой - поворот и наклон.
//! - Ctrl с левой - сдвиг вида параллельно экрану.
//! - Ctrl и Shift с левой - взгляд по сторонам.
//! - Правая кнопка с перетаскиванием - приближение к точке нажатия.
//! - Двойной щелчок левой - перелёт к точке с приближением, правой -
//!   отдаление.
//! - Пальцы, как в Google Earth на телефоне: один - захват, щипок -
//!   приближение к середине щипка, поворот двух пальцев - азимут, два
//!   пальца вверх и вниз - наклон.
//! - Стрелки - сдвиг, с Shift - поворот и наклон, с Ctrl - взгляд
//!   по сторонам. PageUp, PageDown, плюс и минус - приближение и
//!   отдаление. N - север вверху, U - взгляд отвесно, R - то и другое,
//!   пробел - остановка.

use eframe::egui;
use globe_core::flight::{Flight, Motion};
use globe_core::navigation::{Navigator, Pose, ground_under};

/// Один щелчок колеса приближает на 20 %.
pub const WHEEL_STEP: f64 = 0.8;
/// Поворот и наклон мышью, градусов на логический пиксель.
pub const DEGREES_PER_PIXEL: f64 = 0.25;
/// Взгляд по сторонам, градусов на логический пиксель.
pub const LOOK_PER_PIXEL: f64 = 0.1;
/// Правая кнопка: e^(0.005·200) = 2.7 раза на 200 логических пикселей.
pub const RIGHT_ZOOM: f64 = 0.005;
/// Двойной щелчок приближает в 2.5 раза, правой - отдаляет.
pub const DOUBLE_ZOOM: f64 = 0.4;
/// Доля ширины видимой полосы на нажатие стрелки.
pub const KEY_PAN: f64 = 0.05;
/// Градусов на нажатие Shift и Ctrl со стрелками.
pub const KEY_TURN: f64 = 3.0;
/// Логических пикселей колеса на одну строку прокрутки.
pub const POINTS_PER_LINE: f64 = 50.0;

/// Что делает зажатая кнопка.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Grab,
    Turn,
    Shift,
    Look,
    /// Точка нажатия и прежняя высота курсора.
    Zoom { x0: f64, y0: f64 },
}

/// Состояние мыши между кадрами.
#[derive(Debug, Default)]
pub struct Input {
    mode: Option<Mode>,
    last: Option<(f64, f64)>,
}

/// Перелёт к позе.
pub fn fly(nav: &mut Navigator, end: Pose, now: f64) {
    let flight = Flight::new(&nav.camera.ellipsoid, &nav.pose, end, nav.camera.fov_y);
    nav.start_flight(Motion::Flight(flight), now);
}

impl Input {
    /// Разобрать ввод кадра. rect - место глобуса в окне, ppp - пикселей
    /// кадра глобуса на логический пиксель. true - вид изменился.
    pub fn handle(&mut self, ctx: &egui::Context, rect: egui::Rect, nav: &mut Navigator, now: f64, ppp: f64) -> bool {
        let (events, pointer, mods) = ctx.input(|i| (i.events.clone(), i.pointer.clone(), i.modifiers));
        let to_px = |p: egui::Pos2| (f64::from(p.x - rect.min.x) * ppp, f64::from(p.y - rect.min.y) * ppp);
        let inside = pointer.hover_pos().is_some_and(|p| rect.contains(p));
        let mut changed = false;

        if let Some(touch) = ctx.multi_touch() {
            // Два пальца и больше. Захват первым пальцем снимается без
            // инерции, движения указателя в этом кадре не разбираются.
            if self.mode.take().is_some() {
                nav.stop();
            }
            self.last = None;
            let (cx, cy) = to_px(touch.center_pos);
            let zoom = f64::from(touch.zoom_delta);
            if zoom > 0.0 && (zoom - 1.0).abs() > 1e-6 {
                nav.zoom_now(cx, cy, 1.0 / zoom);
            }
            // Пальцы по часовой стрелке тянут карту за собой, азимут
            // верха экрана уменьшается.
            let d_heading = -f64::from(touch.rotation_delta).to_degrees();
            let d_tilt = -f64::from(touch.translation_delta.y) * DEGREES_PER_PIXEL;
            if d_heading != 0.0 || d_tilt != 0.0 {
                nav.turn(d_heading, d_tilt);
            }
            return true;
        }

        for event in &events {
            match event {
                egui::Event::PointerButton { pos, button, pressed: true, .. } if rect.contains(*pos) => {
                    let (px, py) = to_px(*pos);
                    changed |= self.press(nav, *button, mods, px, py, now);
                }
                egui::Event::PointerButton { pressed: false, .. } => {
                    if self.mode.take().is_some() {
                        nav.release(now);
                        changed = true;
                    }
                    self.last = None;
                }
                egui::Event::PointerMoved(pos) => {
                    let (px, py) = to_px(*pos);
                    changed |= self.drag(nav, px, py, ppp, now);
                }
                egui::Event::MouseWheel { unit, delta, .. } if inside => {
                    let notches = match unit {
                        egui::MouseWheelUnit::Line => f64::from(delta.y),
                        egui::MouseWheelUnit::Point => f64::from(delta.y) / POINTS_PER_LINE,
                        egui::MouseWheelUnit::Page => f64::from(delta.y) * 3.0,
                    };
                    if notches != 0.0
                        && let Some(pos) = pointer.hover_pos()
                    {
                        let (px, py) = to_px(pos);
                        nav.wheel(px, py, WHEEL_STEP.powf(notches), now);
                        changed = true;
                    }
                }
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    changed |= key_press(nav, *key, *modifiers, now);
                }
                _ => {}
            }
        }

        for (button, zoom) in [(egui::PointerButton::Primary, DOUBLE_ZOOM), (egui::PointerButton::Secondary, 1.0 / DOUBLE_ZOOM)]
        {
            if !pointer.button_double_clicked(button) || mods.shift || mods.command || mods.ctrl {
                continue;
            }
            let Some(pos) = pointer.interact_pos().filter(|p| rect.contains(*p)) else {
                continue;
            };
            let pose = nav.pose.clone();
            let target = if button == egui::PointerButton::Primary {
                let (px, py) = to_px(pos);
                ground_under(&nav.camera, px, py, pose.terrain.as_ref())
                    .map(|p| nav.camera.ellipsoid.ecef_to_geodetic(p))
                    .map(|(lat, lon, _)| (lat, lon))
            } else {
                Some((pose.lat, pose.lon))
            };
            if let Some((lat, lon)) = target {
                fly(nav, Pose::new(lat, lon, pose.distance * zoom, pose.heading, pose.tilt), now);
                changed = true;
            }
        }
        changed
    }

    fn press(&mut self, nav: &mut Navigator, button: egui::PointerButton, mods: egui::Modifiers, px: f64, py: f64, now: f64) -> bool {
        let ctrl = mods.ctrl || mods.command;
        let mode = match button {
            egui::PointerButton::Primary if ctrl && mods.shift => Mode::Look,
            egui::PointerButton::Primary if ctrl => Mode::Shift,
            egui::PointerButton::Primary if mods.shift => Mode::Turn,
            egui::PointerButton::Middle => Mode::Turn,
            egui::PointerButton::Secondary => Mode::Zoom { x0: px, y0: py },
            egui::PointerButton::Primary => Mode::Grab,
            _ => return false,
        };
        match mode {
            // Поворот не обрывает начатое приближение колесом.
            Mode::Turn => nav.stop_inertia(),
            Mode::Grab => {
                nav.press(px, py, now);
            }
            _ => nav.stop(),
        }
        self.mode = Some(mode);
        self.last = Some((px, py));
        true
    }

    fn drag(&mut self, nav: &mut Navigator, px: f64, py: f64, ppp: f64, now: f64) -> bool {
        let (Some(mode), Some((x0, y0))) = (self.mode, self.last) else {
            return false;
        };
        match mode {
            Mode::Grab => {
                self.last = Some((px, py));
                nav.drag(px, py, now)
            }
            Mode::Turn => {
                self.last = Some((px, py));
                let step = DEGREES_PER_PIXEL / ppp;
                nav.turn((px - x0) * step, (y0 - py) * step);
                true
            }
            Mode::Look => {
                self.last = Some((px, py));
                let step = LOOK_PER_PIXEL / ppp;
                nav.look_around((px - x0) * step, (y0 - py) * step)
            }
            Mode::Shift => {
                if nav.shift_by(px - x0, py - y0) {
                    self.last = Some((px, py));
                    return true;
                }
                false
            }
            Mode::Zoom { x0: zx, y0: zy } => {
                // Вверх - ближе, вниз - дальше, к точке нажатия.
                self.last = Some((px, py));
                let factor = ((py - y0) * RIGHT_ZOOM / ppp).exp();
                nav.zoom_now(zx, zy, factor)
            }
        }
    }
}

fn key_press(nav: &mut Navigator, key: egui::Key, mods: egui::Modifiers, now: f64) -> bool {
    let ctrl = mods.ctrl || mods.command;
    let arrow = match key {
        egui::Key::ArrowLeft => Some((-1.0, 0.0)),
        egui::Key::ArrowRight => Some((1.0, 0.0)),
        egui::Key::ArrowUp => Some((0.0, 1.0)),
        egui::Key::ArrowDown => Some((0.0, -1.0)),
        _ => None,
    };
    if let Some((dx, dy)) = arrow {
        if ctrl {
            nav.look_around(dx * KEY_TURN, dy * KEY_TURN);
        } else if mods.shift {
            nav.turn(dx * KEY_TURN, dy * KEY_TURN);
        } else {
            let width = 2.0 * nav.pose.distance * (nav.camera.fov_y.to_radians() / 2.0).tan();
            nav.pan_by(dy * KEY_PAN * width, dx * KEY_PAN * width);
        }
        return true;
    }
    let centre = (f64::from(nav.camera.width) / 2.0, f64::from(nav.camera.height) / 2.0);
    let pose = nav.pose.clone();
    match key {
        egui::Key::PageUp | egui::Key::Plus | egui::Key::Equals => nav.wheel(centre.0, centre.1, WHEEL_STEP, now),
        egui::Key::PageDown | egui::Key::Minus => nav.wheel(centre.0, centre.1, 1.0 / WHEEL_STEP, now),
        egui::Key::N => fly(nav, Pose::new(pose.lat, pose.lon, pose.distance, 0.0, pose.tilt), now),
        egui::Key::U => fly(nav, Pose::new(pose.lat, pose.lon, pose.distance, pose.heading, 0.0), now),
        egui::Key::R => fly(nav, Pose::new(pose.lat, pose.lon, pose.distance, 0.0, 0.0), now),
        egui::Key::Space => nav.stop(),
        _ => return false,
    }
    true
}
