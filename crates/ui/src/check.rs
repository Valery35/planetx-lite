// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Самопроверка `--check perm` - признак готовности окна фазы 0.
//!
//! 1. Вид на Пермь из космоса, ожидание загрузки.
//! 2. Перелёт к улицам Перми, 500 м с наклоном 45°.
//! 3. Ожидание загрузки, прогревочный оборот вокруг точки взгляда,
//!    снова ожидание загрузки.
//! 4. Замерный оборот без счётчика дыр: счётчик ждёт видеокарту
//!    и сбивал бы частоту кадров.
//!
//! Дыры считаются в каждом кадре шагов 1-3. Проверка пройдена, если
//! дыр нет ни в одном кадре, в любом окне 1 с замерного оборота
//! не меньше 55 кадров, а одновременных запросов к серверу тайлов
//! не больше двух.

use globe_core::navigation::{Navigator, Pose};
use render::FrameStats;

use crate::input::fly;

pub const PERM: (f64, f64) = (58.0105, 56.2294);
/// Вид из космоса.
pub const SPACE_DISTANCE: f64 = 2.0e7;
/// Вид на улицы.
pub const STREET_DISTANCE: f64 = 500.0;
pub const STREET_TILT: f64 = 45.0;
/// Скорость оборота, градусов в секунду: оборот за 10 с.
pub const SPIN_RATE: f64 = 36.0;
pub const SPIN_TIME: f64 = 10.0;
/// Кадров подряд без загрузки, после которых место считается загруженным.
pub const IDLE_FRAMES: u32 = 10;
/// Предел ожидания загрузки, секунды.
pub const LOAD_TIMEOUT: f64 = 180.0;
/// Критерий частоты кадров - кадров в любом окне 1 с.
pub const MIN_FPS: usize = 55;
/// Предел одновременных запросов к серверу тайлов.
pub const MAX_REQUESTS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    LoadSpace,
    Fly,
    LoadStreet,
    Warm,
    Settle,
    Spin,
    Done,
}

/// Состояние самопроверки.
pub struct Check {
    phase: Phase,
    phase_start: Option<f64>,
    last: Option<f64>,
    idle: u32,
    frames: usize,
    /// Кадров, где счётчики дыр прочитаны. На странице они приходят
    /// не в каждом кадре.
    frames_counted: usize,
    holes_max: u64,
    frames_with_holes: usize,
    /// Наибольшая видимая подстилка. None - не считалась: на странице
    /// подстилку не считают.
    underlay_max: Option<u64>,
    spin_times: Vec<f64>,
    failure: Option<String>,
}

/// Поза старта самопроверки.
pub fn start_pose() -> Pose {
    Pose::new(PERM.0, PERM.1, SPACE_DISTANCE, 0.0, 0.0)
}

/// Как программа называет себя серверу тайлов.
fn identity() -> String {
    if cfg!(target_arch = "wasm32") {
        // Браузер не даёт странице задать User-Agent, сервер опознаёт
        // её по Referer.
        "user_agent=- (браузер, Referer - адрес страницы)".to_string()
    } else {
        format!("user_agent={}", net::user_agent())
    }
}

/// Наименьшее количество кадров в окне 1 с, окна целиком внутри
/// замера.
pub fn min_frames_per_second(times: &[f64]) -> Option<usize> {
    let end = *times.last()?;
    times
        .iter()
        .enumerate()
        .filter(|(_, t)| **t + 1.0 <= end)
        .map(|(i, t)| times[i..].iter().take_while(|u| **u < *t + 1.0).count())
        .min()
}

impl Default for Check {
    fn default() -> Self {
        Self::new()
    }
}

impl Check {
    pub fn new() -> Self {
        Self {
            phase: Phase::LoadSpace,
            phase_start: None,
            last: None,
            idle: 0,
            frames: 0,
            frames_counted: 0,
            holes_max: 0,
            frames_with_holes: 0,
            underlay_max: None,
            spin_times: Vec::new(),
            failure: None,
        }
    }

    /// Считать дыры в этом кадре.
    pub fn count_holes(&self) -> bool {
        matches!(self.phase, Phase::LoadSpace | Phase::Fly | Phase::LoadStreet | Phase::Warm | Phase::Settle)
    }

    fn enter(&mut self, phase: Phase, now: f64) {
        self.phase = phase;
        self.phase_start = Some(now);
        self.idle = 0;
    }

    /// Шаг сценария до кадра. loading - загрузка тайлов идёт.
    pub fn before_frame(&mut self, nav: &mut Navigator, now: f64, loading: bool) {
        let start = *self.phase_start.get_or_insert(now);
        let dt = self.last.map_or(0.0, |l| (now - l).max(0.0));
        self.last = Some(now);
        self.idle = if loading { 0 } else { self.idle + 1 };
        let loaded = self.idle >= IDLE_FRAMES;
        let timed_out = now - start > LOAD_TIMEOUT;
        match self.phase {
            Phase::LoadSpace | Phase::LoadStreet | Phase::Settle if timed_out => {
                self.failure = Some(format!("загрузка не закончилась за {LOAD_TIMEOUT} с, шаг {:?}", self.phase));
                self.enter(Phase::Done, now);
            }
            Phase::LoadSpace if loaded => {
                fly(nav, Pose::new(PERM.0, PERM.1, STREET_DISTANCE, 0.0, STREET_TILT), now);
                self.enter(Phase::Fly, now);
            }
            Phase::Fly if !nav.has_flight() => self.enter(Phase::LoadStreet, now),
            Phase::LoadStreet if loaded => self.enter(Phase::Warm, now),
            Phase::Warm | Phase::Spin => {
                nav.turn(SPIN_RATE * dt, 0.0);
                if now - start >= SPIN_TIME {
                    let next = if self.phase == Phase::Warm { Phase::Settle } else { Phase::Done };
                    self.enter(next, now);
                } else if self.phase == Phase::Spin {
                    self.spin_times.push(now);
                }
            }
            Phase::Settle if loaded => self.enter(Phase::Spin, now),
            _ => {}
        }
    }

    /// Итог кадра.
    pub fn after_frame(&mut self, stats: &FrameStats) {
        self.frames += 1;
        if let Some(holes) = stats.holes {
            self.frames_counted += 1;
            self.holes_max = self.holes_max.max(holes);
            self.frames_with_holes += usize::from(holes > 0);
        }
        if let Some(underlay) = stats.underlay {
            self.underlay_max = Some(self.underlay_max.unwrap_or(0).max(underlay));
        }
    }

    pub fn done(&self) -> bool {
        self.phase == Phase::Done
    }

    /// Отчёт и итог: true - проверка пройдена.
    pub fn report(&self, requests_peak: usize, requests_total: usize, disk_hits: usize, size: (u32, u32)) -> (String, bool) {
        let fps = min_frames_per_second(&self.spin_times);
        let ok = self.failure.is_none()
            && self.frames_counted > 0
            && self.holes_max == 0
            && fps.is_some_and(|f| f >= MIN_FPS)
            && requests_peak <= MAX_REQUESTS;
        let mut out = vec![
            format!("frame_size={}x{}", size.0, size.1),
            format!("frames={} frames_counted={}", self.frames, self.frames_counted),
            format!("holes_max={} frames_with_holes={}", self.holes_max, self.frames_with_holes),
            format!("underlay_max={}", self.underlay_max.map_or("-".to_string(), |u| u.to_string())),
            format!("fps_min_1s={} spin_frames={}", fps.map_or("-".to_string(), |f| f.to_string()), self.spin_times.len()),
            format!("requests_peak={requests_peak} requests_total={requests_total} disk_hits={disk_hits}"),
            identity(),
        ];
        if let Some(f) = &self.failure {
            out.push(format!("failure={f}"));
        }
        out.push(format!("result={}", if ok { "ok" } else { "fail" }));
        (out.join("\n"), ok)
    }
}

#[cfg(test)]
mod tests {
    use super::min_frames_per_second;

    #[test]
    fn frames_per_second_windows() {
        let even: Vec<f64> = (0..300).map(|i| f64::from(i) / 60.0).collect();
        assert_eq!(min_frames_per_second(&even), Some(60));
        let mut stall = even.clone();
        stall.retain(|t| !(2.0..2.2).contains(t));
        assert_eq!(min_frames_per_second(&stall), Some(48));
        assert_eq!(min_frames_per_second(&[0.0, 0.5]), None);
    }
}
