// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Шкала времени меток вверху вида, как шкала времени Google Earth.
//! Перенос основы `ui/timebar.py` модуля.
//!
//! Шкала охватывает время видимых меток (`scene::when`) от самого
//! раннего до самого позднего. Два бегунка задают промежуток, метки вне
//! него скрыты, метки без времени видны всегда. Закрытая шкала метки
//! не скрывает. Бегунки тянутся по одному или вместе за середину,
//! щелчок по полосе переносит промежуток туда. Кнопка проигрывания
//! двигает промежуток вдоль шкалы, вся шкала проходит за [`PLAY_TIME`].
//! Шкала открывается сама при перелёте к метке или остановке тура
//! с датой.

use eframe::egui;
use scene::when::{self, Time};

/// Секунд на проход промежутка через всю шкалу.
pub const PLAY_TIME: f64 = 20.0;
/// Ширина полосы, логические пиксели.
pub const TRACK_WIDTH: f32 = 320.0;
/// Полуширина бегунка.
pub const HANDLE: f32 = 6.0;
/// Цвет промежутка, как в модуле.
pub const SELECTION: egui::Color32 = egui::Color32::from_rgba_premultiplied(150, 126, 0, 150);

/// Что тянут на полосе.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Grip {
    Low,
    High,
    /// Промежуток целиком, со сдвигом от точки захвата.
    Both(f64),
}

/// Шкала времени.
#[derive(Debug, Default)]
pub struct TimeBar {
    pub open: bool,
    /// Охват по временам меток.
    pub extent: Option<(f64, f64)>,
    /// Промежуток, секунды UTC.
    pub range: (f64, f64),
    /// Показ идёт: время часов окна прошлого шага.
    playing: Option<f64>,
    grip: Option<Grip>,
}

impl TimeBar {
    /// Новый охват по временам меток. Промежуток - во всю шкалу.
    pub fn set_extent(&mut self, extent: Option<(f64, f64)>) {
        self.extent = extent;
        self.playing = None;
        if let Some(e) = extent {
            self.range = e;
        } else {
            self.open = false;
        }
    }

    /// Промежуток снаружи: время метки или остановки тура. Бесконечный
    /// край - край шкалы. Шкала открывается.
    pub fn show_time(&mut self, time: &Time) {
        let (Some((lo, hi)), Some((a, b))) = (when::interval(Some(time)), self.extent) else {
            return;
        };
        let lo = if lo.is_finite() { lo } else { a };
        let hi = if hi.is_finite() { hi } else { b };
        self.range = (lo.min(hi), lo.max(hi));
        self.open = true;
    }

    /// Окно, по которому скрываются метки: промежуток открытой шкалы.
    pub fn window(&self) -> Option<(f64, f64)> {
        self.open.then_some(self.range)
    }

    pub fn playing(&self) -> bool {
        self.playing.is_some()
    }

    /// Шаг показа: промежуток той же ширины сдвигается вправо, у края
    /// показ останавливается. Возвращает, сдвинулся ли.
    pub fn step(&mut self, now: f64) -> bool {
        let (Some(last), Some((a, b))) = (self.playing, self.extent) else {
            return false;
        };
        self.playing = Some(now);
        let span = (b - a).max(1.0);
        let width = self.range.1 - self.range.0;
        let shift = (now - last).max(0.0) * span / PLAY_TIME;
        let hi = (self.range.1 + shift).min(b);
        self.range = (hi - width, hi);
        if hi >= b {
            self.playing = None;
        }
        true
    }

    /// Шкала поверх вида. Возвращает, сменился ли промежуток.
    pub fn ui(&mut self, ui: &mut egui::Ui, now: f64) -> bool {
        let Some((a, b)) = self.extent else {
            return false;
        };
        let mut changed = false;
        ui.horizontal(|ui| {
            let play = if self.playing.is_some() { "⏸" } else { "▶" };
            if ui.button(play).clicked() && self.playing.take().is_none() {
                // С конца шкалы показ идёт сначала.
                if self.range.1 >= b {
                    let width = self.range.1 - self.range.0;
                    self.range = (a, a + width);
                    changed = true;
                }
                self.playing = Some(now);
            }
            let (rect, response) = ui.allocate_exact_size(egui::vec2(TRACK_WIDTH, 18.0), egui::Sense::click_and_drag());
            let span = (b - a).max(1.0);
            let x_of = |t: f64| rect.left() + ((t - a) / span).clamp(0.0, 1.0) as f32 * rect.width();
            let t_of = |x: f32| a + f64::from(((x - rect.left()) / rect.width()).clamp(0.0, 1.0)) * span;
            let painter = ui.painter();
            painter.rect_filled(rect.shrink2(egui::vec2(0.0, 6.0)), 3.0, egui::Color32::from_gray(90));
            let (x0, x1) = (x_of(self.range.0), x_of(self.range.1));
            painter.rect_filled(egui::Rect::from_x_y_ranges(x0..=x1.max(x0 + 1.0), rect.y_range()), 2.0, SELECTION);
            for x in [x0, x1] {
                painter.rect_filled(
                    egui::Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(2.0 * HANDLE / 2.0, rect.height())),
                    2.0,
                    egui::Color32::WHITE,
                );
            }
            if let Some(pos) = response.interact_pointer_pos() {
                let t = t_of(pos.x);
                if response.drag_started() || response.clicked() {
                    self.grip = Some(if (pos.x - x0).abs() <= HANDLE {
                        Grip::Low
                    } else if (pos.x - x1).abs() <= HANDLE {
                        Grip::High
                    } else if pos.x > x0 && pos.x < x1 {
                        Grip::Both(t - self.range.0)
                    } else {
                        // Щелчок по полосе переносит промежуток туда.
                        let width = self.range.1 - self.range.0;
                        Grip::Both(width / 2.0)
                    });
                }
                let width = self.range.1 - self.range.0;
                let before = self.range;
                match self.grip {
                    Some(Grip::Low) => self.range.0 = t.min(self.range.1),
                    Some(Grip::High) => self.range.1 = t.max(self.range.0),
                    Some(Grip::Both(offset)) => {
                        let lo = (t - offset).clamp(a, b - width);
                        self.range = (lo, lo + width);
                    }
                    None => {}
                }
                changed |= before != self.range;
                self.playing = None;
            }
            if response.drag_stopped() || response.clicked() {
                self.grip = None;
            }
            ui.label(format!("{} - {}", short(self.range.0, span), short(self.range.1, span)));
        });
        changed
    }
}

/// Дата для подписи: время суток - если охват шкалы короче двух суток.
pub fn short(seconds: f64, span: f64) -> String {
    let full = when::text(seconds);
    let date = full.get(..10).unwrap_or(&full);
    if span < 2.0 * 86_400.0 {
        format!("{} {} UTC", date, full.get(11..16).unwrap_or(""))
    } else {
        date.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{PLAY_TIME, TimeBar, short};

    #[test]
    fn opens_on_time_and_hides_nothing_closed() {
        let mut bar = TimeBar::default();
        bar.set_extent(Some((0.0, 1000.0)));
        assert_eq!(bar.window(), None);
        bar.show_time(&("1970-01-01T00:01:40Z".to_string(), String::new()));
        assert_eq!(bar.window(), Some((100.0, 1000.0)));
        bar.set_extent(None);
        assert!(!bar.open);
    }

    #[test]
    fn play_moves_window_to_the_end() {
        let mut bar = TimeBar { open: true, ..TimeBar::default() };
        bar.set_extent(Some((0.0, 1000.0)));
        bar.range = (0.0, 100.0);
        bar.playing = Some(0.0);
        bar.step(PLAY_TIME / 2.0);
        assert!((bar.range.1 - 600.0).abs() < 1e-9 && (bar.range.0 - 500.0).abs() < 1e-9);
        bar.step(PLAY_TIME * 2.0);
        assert_eq!(bar.range, (900.0, 1000.0));
        assert!(!bar.playing());
    }

    #[test]
    fn labels() {
        assert_eq!(short(0.0, 3600.0), "1970-01-01 00:00 UTC");
        assert_eq!(short(0.0, 1.0e9), "1970-01-01");
    }
}
