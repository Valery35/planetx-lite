// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Экранные органы навигации в правом верхнем углу глобуса, перенос
//! `ui/navpad.py` модуля.
//!
//! Как в Google Earth: кольцо компаса с буквой N, внутри него джойстик
//! взгляда, ниже джойстик сдвига и ползунок высоты с кнопками плюс
//! и минус. Кольцо тянется мышью и поворачивает вид, щелчок по N ставит
//! север вверху, точка в середине кольца - отвесный взгляд. Джойстики
//! и кнопки действуют, пока нажата кнопка мыши или палец, скорость
//! растёт с отклонением. Бегунок ползунка стоит по высоте камеры,
//! перетаскивание задаёт новую. Вдали от курсора органы стоят слабым
//! контуром без заливки. Расчёт - `globe_core::navpad`.

use eframe::egui;
use egui::{Color32, Pos2, Rect, Stroke, Vec2, pos2, vec2};
use globe_core::navigation::{Navigator, Pose, clamp_distance, lifted};
use globe_core::navpad::{
    FAINT, NEAR, angle_delta, look_step, move_step, north_angle, ring_angle, ring_turn, slider_distance, slider_far, slider_share,
    stick, zoom_factor,
};

use crate::i18n::{Lang, tr};

/// Логических пикселей от края глобуса.
pub const MARGIN: f32 = 8.0;
/// Столько пикселей вокруг органов курсор их проявляет.
const NEAR_ZONE: f32 = 48.0;
/// Раскладка в логических пикселях, как у модуля.
pub const WIDTH: f32 = 84.0;
pub const HEIGHT: f32 = 288.0;
const CX: f32 = WIDTH / 2.0;
const RING: Pos2 = pos2(CX, 42.0);
const R_OUT: f32 = 38.0;
const R_IN: f32 = 26.0;
/// Радиус буквы N на кольце.
const R_NORTH: f32 = 32.0;
const LOOK_R: f32 = 18.0;
/// Точка в середине кольца - отвесный взгляд без наклона, как клавиша U.
const CENTER_R: f32 = 5.0;
const MOVE: Pos2 = pos2(CX, 120.0);
const MOVE_R: f32 = 24.0;
const PLUS: Pos2 = pos2(CX, 166.0);
const MINUS: Pos2 = pos2(CX, 274.0);
const BUTTON_R: f32 = 10.0;
/// Верх и низ дорожки ползунка.
const TRACK: (f32, f32) = (184.0, 256.0);
const TRACK_HALF: f32 = 8.0;
/// Самый долгий шаг движения, секунд: после паузы вид не прыгает.
const MAX_DT: f64 = 0.1;

/// Нажатая часть органов.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Ring,
    North,
    Center,
    Look,
    Move,
    Plus,
    Minus,
    Slider,
}

/// Цвета органов: заливка, линия, выделение, кружок и буква N. Как
/// `NAV_*` и `NORTH_*` в `ui/style.py` модуля.
struct Colors {
    fill: Color32,
    line: Color32,
    accent: Color32,
    north_fill: Color32,
    north_text: Color32,
}

fn colors(dark: bool) -> Colors {
    if dark {
        Colors {
            fill: Color32::from_rgb(24, 24, 24),
            line: Color32::WHITE,
            accent: Color32::from_rgb(120, 190, 255),
            north_fill: Color32::from_rgb(240, 240, 240),
            north_text: Color32::from_rgb(30, 30, 30),
        }
    } else {
        Colors {
            fill: Color32::from_rgb(205, 209, 215),
            line: Color32::from_rgb(40, 44, 50),
            accent: Color32::from_rgb(26, 104, 204),
            north_fill: Color32::from_rgb(52, 56, 62),
            north_text: Color32::from_rgb(240, 242, 245),
        }
    }
}

fn with_alpha(c: Color32, a: f64) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}

/// Место органов в окне.
pub fn rect(view: Rect) -> Rect {
    Rect::from_min_size(pos2(view.right() - WIDTH - MARGIN, view.top() + MARGIN), vec2(WIDTH, HEIGHT))
}

/// Место буквы N на кольце при азимуте heading.
fn north_at(heading: f64) -> Pos2 {
    let a = north_angle(heading).to_radians() as f32;
    pos2(RING.x + R_NORTH * a.sin(), RING.y - R_NORTH * a.cos())
}

/// Часть органов под точкой p в координатах органов.
fn hit(p: Pos2, heading: f64) -> Option<Part> {
    let d = p.distance(RING);
    if p.distance(north_at(heading)) <= 9.0 {
        Some(Part::North)
    } else if (R_IN..=R_OUT).contains(&d) {
        Some(Part::Ring)
    } else if d <= CENTER_R {
        Some(Part::Center)
    } else if d < R_IN {
        Some(Part::Look)
    } else if p.distance(MOVE) <= MOVE_R {
        Some(Part::Move)
    } else if p.distance(PLUS) <= BUTTON_R {
        Some(Part::Plus)
    } else if p.distance(MINUS) <= BUTTON_R {
        Some(Part::Minus)
    } else if (p.x - CX).abs() <= TRACK_HALF && (TRACK.0 - 2.0..=TRACK.1 + 2.0).contains(&p.y) {
        Some(Part::Slider)
    } else {
        None
    }
}

/// Состояние органов между кадрами.
#[derive(Debug, Default)]
pub struct NavPad {
    part: Option<Part>,
    stick: (f64, f64),
    ring_last: f64,
    moved: bool,
    last: f64,
    /// Курсор над частью органов.
    hot: bool,
    /// Курсор у органов, они проявлены.
    near: bool,
}

impl NavPad {
    /// Мышь и палец органов. view - место глобуса, layer - его слой:
    /// нажатие над окном органам не достаётся. true - органы держат
    /// указатель, глобус его не получает.
    pub fn input(&mut self, ctx: &egui::Context, view: Rect, layer: egui::LayerId, nav: &mut Navigator, now: f64) -> bool {
        let origin = rect(view).min;
        let (pressed, down, pos) = ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.latest_pos()));
        let local = pos.map(|p| (p - origin).to_pos2());
        let free = pos.is_some_and(|p| ctx.layer_id_at(p).is_none_or(|l| l == layer));
        self.hot = free && local.is_some_and(|p| hit(p, nav.pose.heading).is_some());
        self.near = free && pos.is_some_and(|p| rect(view).expand(NEAR_ZONE).contains(p));
        if self.part.is_none() {
            let Some(p) = local.filter(|_| pressed && free) else {
                return false;
            };
            let Some(part) = hit(p, nav.pose.heading) else {
                return false;
            };
            self.part = Some(part);
            self.moved = false;
            self.last = now;
            nav.stop();
            match part {
                Part::Ring | Part::North => self.ring_last = ring_angle(f64::from(p.x - RING.x), f64::from(p.y - RING.y)),
                Part::Slider => self.slide(nav, p.y),
                _ => {}
            }
            self.aim(p);
            return true;
        }
        let Some(part) = self.part else {
            return false;
        };
        if !down {
            // Отпускание: N и середина действуют щелчком.
            let pose = nav.pose.clone();
            if part == Part::North && !self.moved {
                crate::input::fly(nav, Pose::new(pose.lat, pose.lon, pose.distance, 0.0, pose.tilt), now);
            } else if part == Part::Center && local.and_then(|p| hit(p, pose.heading)) == Some(Part::Center) {
                crate::input::fly(nav, Pose::new(pose.lat, pose.lon, pose.distance, pose.heading, 0.0), now);
            }
            self.part = None;
            self.stick = (0.0, 0.0);
            return true;
        }
        if let Some(p) = local {
            match part {
                Part::Ring | Part::North => {
                    let angle = ring_angle(f64::from(p.x - RING.x), f64::from(p.y - RING.y));
                    let delta = angle_delta(self.ring_last, angle);
                    if delta.abs() > 0.5 || self.moved {
                        self.moved = true;
                        self.part = Some(Part::Ring);
                        self.ring_last = angle;
                        nav.turn(ring_turn(0.0, delta), 0.0);
                    }
                }
                Part::Slider => self.slide(nav, p.y),
                _ => self.aim(p),
            }
        }
        self.tick(nav, now);
        ctx.request_repaint();
        true
    }

    fn aim(&mut self, p: Pos2) {
        let (centre, radius) = match self.part {
            Some(Part::Look) => (RING, LOOK_R),
            Some(Part::Move) => (MOVE, MOVE_R),
            _ => return,
        };
        self.stick = stick(f64::from(p.x - centre.x), f64::from(p.y - centre.y), f64::from(radius));
    }

    fn slide(&self, nav: &mut Navigator, y: f32) {
        let share = f64::from((y - TRACK.0) / (TRACK.1 - TRACK.0));
        zoom_to(nav, slider_distance(share, NEAR, slider_far(nav.camera.ellipsoid.a)));
    }

    /// Шаг джойстиков и кнопок за время с прошлого кадра.
    fn tick(&mut self, nav: &mut Navigator, now: f64) {
        let dt = (now - self.last).clamp(0.0, MAX_DT);
        self.last = now;
        let (sx, sy) = self.stick;
        match self.part {
            Some(Part::Look) if sx != 0.0 || sy != 0.0 => {
                let (dh, dt) = look_step(sx, sy, dt);
                nav.look_around(dh, dt);
            }
            Some(Part::Move) if sx != 0.0 || sy != 0.0 => {
                let width = 2.0 * nav.pose.distance * (nav.camera.fov_y.to_radians() / 2.0).tan();
                let (forward, right) = move_step(sx, sy, dt, width, nav.camera.ellipsoid.a);
                nav.pan_by(forward, right);
            }
            Some(Part::Plus) => zoom_to(nav, nav.pose.distance * zoom_factor(1.0, dt)),
            Some(Part::Minus) => zoom_to(nav, nav.pose.distance * zoom_factor(-1.0, dt)),
            _ => {}
        }
    }

    /// Подсказка органов, когда курсор над ними.
    pub fn tooltip(&self, ui: &mut egui::Ui, view: Rect, lang: Lang) {
        if self.hot && self.part.is_none() {
            ui.interact(rect(view), egui::Id::unique("planetx-navpad"), egui::Sense::hover()).on_hover_text(tr(
                lang,
                "The ring turns the view, the letter N puts north up. The stick inside the ring turns the gaze, the dot in its middle returns a straight down view. The lower stick moves the view. The slider sets the height, plus and minus zoom in and out.",
            ));
        }
    }

    /// Нарисовать органы.
    pub fn paint(&self, painter: &egui::Painter, view: Rect, nav: &Navigator, dark: bool) {
        let origin = rect(view).min.to_vec2();
        let at = |p: Pos2| p + origin;
        let active = self.hot || self.part.is_some();
        let faint = !(active || self.near);
        let alpha = if faint {
            FAINT
        } else if active {
            1.0
        } else {
            0.65
        };
        let c = colors(dark);
        // Вдали от курсора остаётся контур без заливки.
        let fill = if faint { Color32::TRANSPARENT } else { with_alpha(c.fill, 150.0 / 255.0 * alpha) };
        let line = with_alpha(c.line, 210.0 / 255.0 * alpha);
        let accent = with_alpha(c.accent, 230.0 / 255.0 * alpha);
        let pen = Stroke::new(1.2, line);
        painter.circle(at(RING), R_OUT - 0.5, fill, pen);
        painter.circle_stroke(at(RING), R_IN, pen);
        // Светлый кружок с тёмной буквой, как в Google Earth.
        let north = at(north_at(nav.pose.heading));
        painter.circle_filled(north, 7.0, with_alpha(c.north_fill, 235.0 / 255.0 * alpha));
        painter.text(north, egui::Align2::CENTER_CENTER, "N", egui::FontId::proportional(10.0), with_alpha(c.north_text, alpha));
        self.stick_marks(painter, at(RING), LOOK_R - 2.0, line, accent, self.part == Some(Part::Look));
        if self.part == Some(Part::Center) {
            painter.circle_filled(at(RING), 4.0, accent);
        }
        painter.circle(at(MOVE), MOVE_R - 0.5, fill, pen);
        self.stick_marks(painter, at(MOVE), MOVE_R - 4.0, line, accent, self.part == Some(Part::Move));
        for (centre, part) in [(PLUS, Part::Plus), (MINUS, Part::Minus)] {
            let centre = at(centre);
            painter.circle(centre, BUTTON_R - 0.5, if self.part == Some(part) { accent } else { fill }, pen);
            let sign = Stroke::new(2.0, line);
            painter.line_segment([centre - vec2(5.0, 0.0), centre + vec2(5.0, 0.0)], sign);
            if part == Part::Plus {
                painter.line_segment([centre - vec2(0.0, 5.0), centre + vec2(0.0, 5.0)], sign);
            }
        }
        // Ползунок высоты.
        let track = Rect::from_min_max(at(pos2(CX - 3.0, TRACK.0)), at(pos2(CX + 3.0, TRACK.1)));
        painter.rect(track, 3.0, fill, pen, egui::StrokeKind::Middle);
        let share = slider_share(nav.pose.distance, NEAR, slider_far(nav.camera.ellipsoid.a)) as f32;
        let y = TRACK.0 + share * (TRACK.1 - TRACK.0);
        let knob = Rect::from_center_size(at(pos2(CX, y)), vec2(2.0 * TRACK_HALF, 8.0));
        painter.rect_filled(knob, 3.0, if self.part == Some(Part::Slider) { accent } else { line });
    }

    /// Четыре стрелки и шарик джойстика, сдвинутый при нажатии.
    fn stick_marks(&self, painter: &egui::Painter, centre: Pos2, reach: f32, line: Color32, accent: Color32, active: bool) {
        for (dx, dy) in [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)] {
            let dir = Vec2::new(dx, dy);
            let side = Vec2::new(-dy, dx) * 4.0;
            let tip = centre + dir * reach;
            let base = centre + dir * (reach - 6.0);
            painter.add(egui::Shape::convex_polygon(vec![tip, base + side, base - side], line, Stroke::NONE));
        }
        let (sx, sy) = if active { self.stick } else { (0.0, 0.0) };
        let ball = centre + vec2(sx as f32, sy as f32) * (reach - 8.0);
        painter.circle_filled(ball, 4.0, if active { accent } else { line });
    }
}

/// Расстояние камеры distance, точка в середине вида на месте. Если
/// в середине вида небо, расстояние ставится как есть.
fn zoom_to(nav: &mut Navigator, distance: f64) {
    let factor = distance / nav.pose.distance;
    let (w, h) = (f64::from(nav.camera.width) / 2.0, f64::from(nav.camera.height) / 2.0);
    if !nav.zoom_now(w, h, factor) {
        let mut pose = nav.pose.clone();
        pose.distance = clamp_distance(&nav.camera.ellipsoid, &pose, distance);
        let pose = lifted(&nav.camera.ellipsoid, pose);
        nav.set_pose(pose);
    }
}

#[cfg(test)]
mod tests {
    use super::{Part, hit};
    use super::{BUTTON_R, MINUS, MOVE, PLUS, RING, TRACK};
    use eframe::egui::pos2;

    #[test]
    fn parts_are_found() {
        // Север вверху: буква N над серединой кольца.
        assert_eq!(hit(pos2(RING.x, RING.y - 32.0), 0.0), Some(Part::North));
        assert_eq!(hit(pos2(RING.x, RING.y + 32.0), 0.0), Some(Part::Ring));
        assert_eq!(hit(RING, 0.0), Some(Part::Center));
        assert_eq!(hit(pos2(RING.x + 12.0, RING.y), 0.0), Some(Part::Look));
        assert_eq!(hit(MOVE, 0.0), Some(Part::Move));
        assert_eq!(hit(PLUS, 0.0), Some(Part::Plus));
        assert_eq!(hit(MINUS, 0.0), Some(Part::Minus));
        assert_eq!(hit(pos2(RING.x, (TRACK.0 + TRACK.1) / 2.0), 0.0), Some(Part::Slider));
        // Промежуток между органами - глобусу.
        assert_eq!(hit(pos2(RING.x + BUTTON_R + 20.0, PLUS.y), 0.0), None);
        // Азимут 90°: буква N слева.
        assert_eq!(hit(pos2(RING.x - 32.0, RING.y), 90.0), Some(Part::North));
    }
}
