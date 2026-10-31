// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Шкала времени меток вверху вида, как шкала времени Google Earth.
//! Перенос `ui/timebar.py` модуля.
//!
//! Шкала охватывает время видимых меток (`scene::when`) от самого
//! раннего до самого позднего. Два бегунка задают промежуток, метки вне
//! него скрыты, метки без времени видны всегда. Закрытая шкала метки
//! не скрывает. Бегунки тянутся по одному или вместе за середину,
//! щелчок по полосе переносит промежуток туда. У шкалы-момента один
//! бегунок, щелчок и протяжка ставят момент.
//!
//! Кнопки, как у модуля: к началу, проигрывание, к концу, по кругу,
//! скорость. Скорость - из `scene::clock::RATES` или вся шкала за
//! `scene::clock::FIT_SECONDS`. Промежуток на всю шкалу сдвинуть некуда,
//! поэтому его левый край стоит, а правый идёт от начала к концу.
//! Шкала открывается сама при перелёте к метке или остановке тура
//! с датой.

use eframe::egui;
use scene::clock;
use scene::when::{self, Time};

use crate::i18n::{Lang, tr};

/// Ширина полосы, логические пиксели.
pub const TRACK_WIDTH: f32 = 320.0;
/// Наименьшая полоса на узком экране.
const MIN_TRACK: f32 = 120.0;
/// Поля шкалы: от края окна и внутри её плашки.
const BAR_MARGIN: f32 = 48.0;
/// Сколько нужно шкале в одну строку: кнопки, полоса, момент, скорость.
pub const ONE_ROW: f32 = 760.0;
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
    /// Момент шкалы-момента.
    Point,
}

/// Шкала времени.
#[derive(Debug, Default)]
pub struct TimeBar {
    pub open: bool,
    /// Охват по временам меток.
    pub extent: Option<(f64, f64)>,
    /// Промежуток, секунды UTC. У шкалы-момента края равны.
    pub range: (f64, f64),
    /// Шкала-момент: один бегунок.
    pub point: bool,
    /// Скорость показа, во сколько раз быстрее часов, None - вся шкала
    /// за `FIT_SECONDS`.
    pub rate: Option<f64>,
    /// Проигрывание по кругу.
    pub looping: bool,
    /// Показ идёт: время часов окна прошлого шага.
    playing: Option<f64>,
    /// Левый край показа промежутка на всю шкалу, иначе None.
    sweep: Option<f64>,
    grip: Option<Grip>,
}

impl TimeBar {
    /// Новый охват по временам меток. Промежуток - во всю шкалу,
    /// у шкалы-момента момент остаётся в пределах охвата.
    pub fn set_extent(&mut self, extent: Option<(f64, f64)>) {
        self.extent = extent;
        self.stop();
        match extent {
            Some((a, b)) if self.point => {
                let t = self.range.1.clamp(a, b);
                self.range = (t, t);
            }
            Some(e) => self.range = e,
            None => self.open = false,
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
        self.range = if self.point { (hi, hi) } else { (lo.min(hi), lo.max(hi)) };
        self.open = true;
    }

    /// Один бегунок в момент at, как `set_point` модуля. Выход из
    /// режима - промежуток от начала шкалы до момента.
    pub fn set_point(&mut self, on: bool, at: Option<f64>) {
        self.point = on;
        let moment = at.unwrap_or(self.range.1);
        let a = self.extent.map_or(moment, |e| e.0);
        self.range = if on { (moment, moment) } else { (a, moment) };
    }

    /// Окно, по которому скрываются метки: промежуток открытой шкалы.
    pub fn window(&self) -> Option<(f64, f64)> {
        self.open.then_some(self.range)
    }

    pub fn playing(&self) -> bool {
        self.playing.is_some()
    }

    pub fn stop(&mut self) {
        self.playing = None;
        self.sweep = None;
    }

    /// Начать или остановить показ, как `toggle` модуля.
    pub fn toggle(&mut self, now: f64) {
        if self.playing.is_some() {
            self.stop();
            return;
        }
        let Some((a, b)) = self.extent else {
            return;
        };
        self.sweep = None;
        let (lo, hi) = self.range;
        if !self.point && lo <= a && hi >= b {
            // Промежуток на всю шкалу сдвинуть некуда, показ стоял бы.
            // Левый край стоит в начале, правый идёт от него к концу.
            self.sweep = Some(a);
            self.range = (a, a);
        } else if hi >= b {
            // Проигрывание с начала шкалы, ширина промежутка та же.
            self.range = (a, a + (hi - lo));
        }
        self.playing = Some(now);
    }

    /// Шаг показа, как `_step` модуля. Возвращает, сдвинулся ли.
    pub fn step(&mut self, now: f64) -> bool {
        let (Some(last), Some(span)) = (self.playing, self.extent) else {
            return false;
        };
        self.playing = Some(now);
        let step = clock::shift(span, self.rate, (now - last).max(0.0), clock::FIT_SECONDS);
        if let Some(start) = self.sweep {
            let (hi, going) = clock::sweep(start, self.range.1, span, step, self.looping);
            self.range = (start, hi);
            if !going {
                self.stop();
            }
            return true;
        }
        let (lo, hi, _, going) = clock::advance(self.range.0, self.range.1, span, step, false, self.looping);
        self.range = (lo, hi);
        if !going {
            self.stop();
        }
        true
    }

    /// Промежуток той же ширины - в начало шкалы.
    pub fn to_start(&mut self) {
        if let Some((a, _)) = self.extent {
            let width = self.range.1 - self.range.0;
            self.range = (a, a + width);
        }
    }

    /// Промежуток той же ширины - в конец шкалы.
    pub fn to_end(&mut self) {
        self.stop();
        if let Some((_, b)) = self.extent {
            let width = self.range.1 - self.range.0;
            self.range = (b - width, b);
        }
    }

    /// Шкала поверх вида. Возвращает, сменился ли промежуток.
    /// Закрыть её можно и здесь, кнопкой ⏹ модуля.
    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang, now: f64) -> bool {
        let Some((a, b)) = self.extent else {
            return false;
        };
        let before = self.range;
        let room = ui.ctx().content_rect().width() - BAR_MARGIN;
        // На узком экране полоса - своей строкой: в одну строку она уходила
        // за край телефона вместе с кнопкой закрытия.
        if room < ONE_ROW {
            ui.vertical(|ui| {
                ui.set_max_width(room);
                ui.horizontal_wrapped(|ui| {
                    self.buttons(ui, lang, now);
                    self.readout(ui, lang, a, b);
                });
                self.track(ui, a, b, track_width(room));
            });
            return before != self.range;
        }
        ui.horizontal(|ui| {
            self.buttons(ui, lang, now);
            self.track(ui, a, b, TRACK_WIDTH);
            self.readout(ui, lang, a, b);
        });
        before != self.range
    }

    /// Кнопки показа: в начало, пуск, в конец, по кругу.
    fn buttons(&mut self, ui: &mut egui::Ui, lang: Lang, now: f64) {
        {
            if icon_button(ui, Icon::ToStart).on_hover_text(tr(lang, "To the start of the bar")).clicked() {
                self.to_start();
            }
            let play = if self.playing.is_some() { Icon::Pause } else { Icon::Play };
            if icon_button(ui, play).on_hover_text(tr(lang, "Play: the range moves along the bar, placemarks appear and hide by their time.")).clicked() {
                self.toggle(now);
            }
            if icon_button(ui, Icon::ToEnd).on_hover_text(tr(lang, "To the end of the bar")).clicked() {
                self.to_end();
            }
            let looping = icon_toggle(ui, Icon::Loop, self.looping)
                .on_hover_text(tr(lang, "Play in a loop: at the end of the bar the range starts again from the beginning."));
            if looping.clicked() {
                self.looping = !self.looping;
            }
        }
    }

    /// Момент подписью, скорость показа и кнопка закрытия.
    fn readout(&mut self, ui: &mut egui::Ui, lang: Lang, a: f64, b: f64) {
        {
            let span = (b - a).max(1.0);
            if self.range.1 - self.range.0 < 1.0 {
                ui.label(short(self.range.0, span));
            } else {
                ui.label(format!("{} - {}", short(self.range.0, span), short(self.range.1, span)));
            }
            let fit = tr(lang, "Whole bar in 20 s");
            let name = |r: Option<f64>| r.map_or_else(|| fit.to_string(), |r| format!("×{r}"));
            egui::ComboBox::from_id_salt("planetx-time-rate")
                .selected_text(name(self.rate))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.rate, None, fit);
                    for r in clock::RATES {
                        ui.selectable_value(&mut self.rate, Some(r), name(Some(r)));
                    }
                })
                .response
                .on_hover_text(tr(lang, "Playback speed. ×1 - time runs as on the clock, ×3600 - an hour per second, ×86400 - a day per second."));
            if icon_button(ui, Icon::Stop).on_hover_text(tr(lang, "Close the time bar. A closed bar does not hide placemarks.")).clicked() {
                self.stop();
                self.open = false;
            }
        }
    }

    /// Полоса с бегунками.
    fn track(&mut self, ui: &mut egui::Ui, a: f64, b: f64, width: f32) {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 18.0), egui::Sense::click_and_drag());
        let span = (b - a).max(1.0);
        let x_of = |t: f64| rect.left() + ((t - a) / span).clamp(0.0, 1.0) as f32 * rect.width();
        let t_of = |x: f32| a + f64::from(((x - rect.left()) / rect.width()).clamp(0.0, 1.0)) * span;
        let painter = ui.painter();
        painter.rect_filled(rect.shrink2(egui::vec2(0.0, 6.0)), 3.0, egui::Color32::from_gray(90));
        let (x0, x1) = (x_of(self.range.0), x_of(self.range.1));
        if !self.point {
            painter.rect_filled(egui::Rect::from_x_y_ranges(x0..=x1.max(x0 + 1.0), rect.y_range()), 2.0, SELECTION);
        }
        let handles: &[f32] = if self.point { &[x1] } else { &[x0, x1] };
        let fill = if self.point { SELECTION } else { egui::Color32::WHITE };
        for &x in handles {
            painter.rect_filled(egui::Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(HANDLE, rect.height())), 2.0, fill);
        }
        if let Some(pos) = response.interact_pointer_pos() {
            let t = t_of(pos.x);
            if response.drag_started() || response.clicked() {
                self.grip = Some(if self.point {
                    Grip::Point
                } else if (pos.x - x1).abs() <= HANDLE && ((pos.x - x1).abs() <= (pos.x - x0).abs() || pos.x > x1) {
                    Grip::High
                } else if (pos.x - x0).abs() <= HANDLE {
                    Grip::Low
                } else if pos.x > x0 && pos.x < x1 {
                    Grip::Both(t - self.range.0)
                } else {
                    // Щелчок по полосе переносит промежуток туда.
                    Grip::Both((self.range.1 - self.range.0) / 2.0)
                });
            }
            let width = self.range.1 - self.range.0;
            match self.grip {
                Some(Grip::Point) => self.range = (t, t),
                Some(Grip::Low) => self.range.0 = t.min(self.range.1),
                Some(Grip::High) => self.range.1 = t.max(self.range.0),
                Some(Grip::Both(offset)) => {
                    let lo = (t - offset).clamp(a, b - width);
                    self.range = (lo, lo + width);
                }
                None => {}
            }
            self.stop();
        }
        if response.drag_stopped() || response.clicked() {
            self.grip = None;
        }
    }
}

/// Ширина полосы на узком экране: вся строка, но не шире обычной.
fn track_width(room: f32) -> f32 {
    room.clamp(MIN_TRACK, TRACK_WIDTH)
}

/// Значок кнопки, рисуется фигурой: знаков ▶ ⏸ ⏹ в шрифте программы нет.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Play,
    Pause,
    Stop,
    ToStart,
    ToEnd,
    Loop,
    Captions,
}

/// Кнопка со значком.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon) -> egui::Response {
    icon_toggle(ui, icon, false)
}

/// Кнопка со значком, on - нажата, подсвечена.
pub fn icon_toggle(ui: &mut egui::Ui, icon: Icon, on: bool) -> egui::Response {
    // Значки набора модуля (`ui/style.py`), как у панели значков.
    use crate::theme::Tool;
    let tool = match icon {
        Icon::Play => Tool::Play,
        Icon::Pause => Tool::Pause,
        Icon::Stop => Tool::Stop,
        Icon::ToStart => Tool::Prev,
        Icon::ToEnd => Tool::Next,
        Icon::Loop => Tool::Loop,
        Icon::Captions => Tool::Captions,
    };
    crate::theme::tool_button(ui, tool, on, false, true, "")
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
    use super::{TimeBar, short};
    use scene::clock::FIT_SECONDS;

    /// Полоса укладывается в ширину телефона: в 375 точек обычная шкала
    /// уходила за край экрана.
    #[test]
    fn track_fits_a_narrow_screen() {
        assert_eq!(super::track_width(375.0 - super::BAR_MARGIN), 320.0);
        assert_eq!(super::track_width(280.0), 280.0);
        assert_eq!(super::track_width(40.0), super::MIN_TRACK);
    }

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
        bar.toggle(0.0);
        bar.step(FIT_SECONDS / 2.0);
        assert!((bar.range.1 - 600.0).abs() < 1e-9 && (bar.range.0 - 500.0).abs() < 1e-9);
        bar.step(FIT_SECONDS * 2.0);
        assert_eq!(bar.range, (900.0, 1000.0));
        assert!(!bar.playing());
    }

    #[test]
    fn full_range_sweeps_right_edge() {
        // Промежуток на всю шкалу: левый край стоит, правый идёт.
        let mut bar = TimeBar { open: true, ..TimeBar::default() };
        bar.set_extent(Some((0.0, 1000.0)));
        bar.toggle(0.0);
        assert_eq!(bar.range, (0.0, 0.0));
        bar.step(FIT_SECONDS / 4.0);
        assert!((bar.range.1 - 250.0).abs() < 1e-9 && bar.range.0 == 0.0);
    }

    #[test]
    fn point_runs_at_rate_and_loops() {
        let mut bar = TimeBar { open: true, rate: Some(10.0), looping: true, ..TimeBar::default() };
        bar.set_extent(Some((0.0, 100.0)));
        bar.set_point(true, Some(10.0));
        bar.toggle(0.0);
        bar.step(2.0);
        assert_eq!(bar.range, (30.0, 30.0));
        bar.step(10.0);
        // За 8 с ещё 80 с времени - за краем, по кругу с начала.
        assert_eq!(bar.range, (0.0, 0.0));
        assert!(bar.playing());
    }

    #[test]
    fn labels() {
        assert_eq!(short(0.0, 3600.0), "1970-01-01 00:00 UTC");
        assert_eq!(short(0.0, 1.0e9), "1970-01-01");
    }
}
