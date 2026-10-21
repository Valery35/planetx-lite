// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Самопроверки - признаки готовности фаз.
//!
//! `perm` (фаза 0): вид на Пермь из космоса, перелёт к улицам, оборот
//! вокруг точки взгляда. Дыр нет ни в одном кадре, в любом окне 1 с
//! замерного оборота не меньше 55 кадров, к серверу тайлов не больше
//! двух запросов сразу.
//!
//! `elbrus` (фаза 1), позы и пороги проверки фазы 1 модуля
//! (`tools/qgis_phase1.py`): спуск к Эльбрусу из космоса, ожидание
//! загрузки, замер высоты вершины, низкий полёт. Дыр нет, глаз не ниже
//! 50 м над рельефом, наибольшая высота в радиусе 1 км 5642 ± 60 м
//! на уровне высот не ниже 12, в низком полёте не меньше 55 кадров
//! в любом окне 1 с.
//!
//! Дыры считаются, пока частота кадров не мерится: счётчик ждёт
//! видеокарту и сбивал бы её.

use globe_core::lod::TileKey;
use globe_core::navigation::{MIN_ALTITUDE, Navigator, Pose, clearance};
use render::FrameStats;
use terrain::HeightStore;

use crate::input::fly;

pub const PERM: (f64, f64) = (58.0105, 56.2294);
/// Вершина Эльбруса, как в проверке модуля.
pub const ELBRUS: (f64, f64) = (43.34778, 42.44001);
/// Высота Эльбруса и допуск замера, метры.
pub const ELBRUS_HEIGHT: f64 = 5642.0;
pub const ELBRUS_TOLERANCE: f64 = 60.0;
/// Уровень тайла высот на вершине не ниже этого.
pub const ELBRUS_MIN_LEVEL: u32 = 12;
/// Вид из космоса.
pub const SPACE_DISTANCE: f64 = 2.0e7;
/// Вид на улицы.
pub const STREET_DISTANCE: f64 = 500.0;
pub const STREET_TILT: f64 = 45.0;
/// Скорость оборота, градусов в секунду: оборот за 10 с.
pub const SPIN_RATE: f64 = 36.0;
pub const SPIN_TIME: f64 = 10.0;
/// Спуск к Эльбрусу: расстояние, азимут и наклон в начале и в конце.
pub const DESCENT: ((f64, f64, f64), (f64, f64, f64)) = ((2.0e6, 0.0, 0.0), (1500.0, 90.0, 60.0));
pub const DESCENT_TIME: f64 = 20.0;
/// Низкий полёт: расстояние, наклон, размах по широте, время.
pub const LOW_FLIGHT: (f64, f64, f64, f64) = (400.0, 80.0, 0.05, 12.0);
/// Замер высоты вершины: радиус и шаг, метры.
pub const SUMMIT_RADIUS: f64 = 1000.0;
pub const SUMMIT_STEP: f64 = 20.0;
/// Кадров подряд без загрузки, после которых место считается загруженным.
pub const IDLE_FRAMES: u32 = 10;
/// Предел ожидания загрузки, секунды.
pub const LOAD_TIMEOUT: f64 = 180.0;
/// Критерий частоты кадров - кадров в любом окне 1 с.
pub const MIN_FPS: usize = 55;
/// Предел одновременных запросов к серверу тайлов.
pub const MAX_REQUESTS: usize = 2;
const M_PER_DEGREE: f64 = 111_320.0;

/// Какая самопроверка.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    Perm,
    Elbrus,
}

impl Scenario {
    /// Сценарий по имени из командной строки или адреса страницы.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "perm" => Some(Self::Perm),
            "elbrus" => Some(Self::Elbrus),
            _ => None,
        }
    }

    pub fn start_pose(self) -> Pose {
        match self {
            Self::Perm => Pose::new(PERM.0, PERM.1, SPACE_DISTANCE, 0.0, 0.0),
            Self::Elbrus => {
                let (d, h, t) = DESCENT.0;
                Pose::new(ELBRUS.0, ELBRUS.1, d, h, t)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Ожидание загрузки на месте старта.
    LoadStart,
    /// Пермь: перелёт к улицам.
    Fly,
    /// Ожидание загрузки на месте.
    LoadPlace,
    /// Пермь: прогревочный оборот.
    Warm,
    /// Пермь: ожидание загрузки после оборота.
    Settle,
    /// Пермь: замерный оборот.
    Spin,
    /// Эльбрус: спуск.
    Descent,
    /// Эльбрус: низкий полёт, замер кадров.
    LowFlight,
    Done,
}

/// Состояние самопроверки.
pub struct Check {
    scenario: Scenario,
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
    /// Наименьший зазор глаза над рельефом.
    clearance_min: f64,
    /// Наибольшая высота в радиусе 1 км от вершины и уровень тайла высот.
    summit: Option<(f64, u32)>,
    fps_times: Vec<f64>,
    failure: Option<String>,
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

/// Наибольшая высота рельефа в круге радиуса radius вокруг точки
/// с шагом step и уровень тайла высот в центре.
pub fn summit_height(store: &HeightStore, lat: f64, lon: f64, radius: f64, step: f64) -> (f64, u32) {
    let n = (radius / step).ceil() as i64;
    let per_lon = M_PER_DEGREE * lat.to_radians().cos().max(1e-6);
    let mut top = f64::NEG_INFINITY;
    for i in -n..=n {
        for j in -n..=n {
            let (dy, dx) = (i as f64 * step, j as f64 * step);
            if dx * dx + dy * dy <= radius * radius {
                top = top.max(store.height_at(lat + dy / M_PER_DEGREE, lon + dx / per_lon));
            }
        }
    }
    let (u, v) = terrain::mercator_share(lat, lon);
    let key: TileKey = (17, (u * f64::from(1u32 << 17)) as u32, (v * f64::from(1u32 << 17)) as u32);
    (top, store.best(key).map_or(0, |t| t.z))
}

/// Плавная доля: 0 и 1 на концах.
fn ease(share: f64) -> f64 {
    let s = share.clamp(0.0, 1.0);
    s * s * (3.0 - 2.0 * s)
}

impl Check {
    pub fn new(scenario: Scenario) -> Self {
        Self {
            scenario,
            phase: Phase::LoadStart,
            phase_start: None,
            last: None,
            idle: 0,
            frames: 0,
            frames_counted: 0,
            holes_max: 0,
            frames_with_holes: 0,
            underlay_max: None,
            clearance_min: f64::INFINITY,
            summit: None,
            fps_times: Vec::new(),
            failure: None,
        }
    }

    /// Считать дыры в этом кадре.
    pub fn count_holes(&self) -> bool {
        !matches!(self.phase, Phase::Spin | Phase::LowFlight | Phase::Done)
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
        let t = now - start;
        if matches!(self.phase, Phase::LoadStart | Phase::LoadPlace | Phase::Settle) && t > LOAD_TIMEOUT {
            self.failure = Some(format!("загрузка не закончилась за {LOAD_TIMEOUT} с, шаг {:?}", self.phase));
            self.enter(Phase::Done, now);
            return;
        }
        match (self.scenario, self.phase) {
            (Scenario::Perm, Phase::LoadStart) if loaded => {
                fly(nav, Pose::new(PERM.0, PERM.1, STREET_DISTANCE, 0.0, STREET_TILT), now);
                self.enter(Phase::Fly, now);
            }
            (Scenario::Perm, Phase::Fly) if !nav.has_flight() => self.enter(Phase::LoadPlace, now),
            (Scenario::Perm, Phase::LoadPlace) if loaded => self.enter(Phase::Warm, now),
            (Scenario::Perm, Phase::Warm | Phase::Spin) => {
                nav.turn(SPIN_RATE * dt, 0.0);
                if t >= SPIN_TIME {
                    let next = if self.phase == Phase::Warm { Phase::Settle } else { Phase::Done };
                    self.enter(next, now);
                } else if self.phase == Phase::Spin {
                    self.fps_times.push(now);
                }
            }
            (Scenario::Perm, Phase::Settle) if loaded => self.enter(Phase::Spin, now),
            (Scenario::Elbrus, Phase::LoadStart) if loaded => self.enter(Phase::Descent, now),
            (Scenario::Elbrus, Phase::Descent) => {
                // Расстояние по экспоненте, азимут и наклон плавно.
                let ((d0, h0, t0), (d1, h1, t1)) = DESCENT;
                let s = (t / DESCENT_TIME).min(1.0);
                let distance = d0 * (d1 / d0).powf(s);
                let k = ease(s);
                nav.show(Pose::new(ELBRUS.0, ELBRUS.1, distance, h0 + (h1 - h0) * k, t0 + (t1 - t0) * k));
                if s >= 1.0 {
                    self.enter(Phase::LoadPlace, now);
                }
            }
            (Scenario::Elbrus, Phase::LoadPlace) if loaded => self.enter(Phase::LowFlight, now),
            (Scenario::Elbrus, Phase::LowFlight) => {
                let (distance, tilt, span, time) = LOW_FLIGHT;
                let s = (t / time).min(1.0);
                let lat = ELBRUS.0 - span + 2.0 * span * s;
                nav.show(Pose::new(lat, ELBRUS.1, distance, 0.0, tilt));
                self.fps_times.push(now);
                if s >= 1.0 {
                    self.enter(Phase::Done, now);
                }
            }
            _ => {}
        }
    }

    /// Итог кадра.
    pub fn after_frame(&mut self, stats: &FrameStats, nav: &Navigator, store: &HeightStore) {
        self.frames += 1;
        if let Some(holes) = stats.holes {
            self.frames_counted += 1;
            self.holes_max = self.holes_max.max(holes);
            self.frames_with_holes += usize::from(holes > 0);
        }
        if let Some(underlay) = stats.underlay {
            self.underlay_max = Some(self.underlay_max.unwrap_or(0).max(underlay));
        }
        if self.scenario == Scenario::Elbrus && self.phase != Phase::LoadStart {
            let c = clearance(&nav.camera.ellipsoid, nav.camera.eye, nav.pose.terrain.as_ref());
            self.clearance_min = self.clearance_min.min(c);
        }
        if self.scenario == Scenario::Elbrus && self.phase == Phase::LowFlight && self.summit.is_none() {
            self.summit = Some(summit_height(store, ELBRUS.0, ELBRUS.1, SUMMIT_RADIUS, SUMMIT_STEP));
        }
    }

    pub fn done(&self) -> bool {
        self.phase == Phase::Done
    }

    /// Отчёт и итог: true - проверка пройдена.
    pub fn report(&self, requests_peak: usize, requests_total: usize, disk_hits: usize, size: (u32, u32)) -> (String, bool) {
        let fps = min_frames_per_second(&self.fps_times);
        let mut ok = self.failure.is_none()
            && self.frames_counted > 0
            && self.holes_max == 0
            && fps.is_some_and(|f| f >= MIN_FPS)
            && requests_peak <= MAX_REQUESTS;
        let mut out = vec![
            format!("check={:?}", self.scenario).to_lowercase(),
            format!("frame_size={}x{}", size.0, size.1),
            format!("frames={} frames_counted={}", self.frames, self.frames_counted),
            format!("holes_max={} frames_with_holes={}", self.holes_max, self.frames_with_holes),
            format!("underlay_max={}", self.underlay_max.map_or("-".to_string(), |u| u.to_string())),
            format!("fps_min_1s={} fps_frames={}", fps.map_or("-".to_string(), |f| f.to_string()), self.fps_times.len()),
        ];
        if self.scenario == Scenario::Elbrus {
            let summit_ok = self.summit.is_some_and(|(h, level)| {
                (h - ELBRUS_HEIGHT).abs() <= ELBRUS_TOLERANCE && level >= ELBRUS_MIN_LEVEL
            });
            let clear_ok = self.clearance_min >= MIN_ALTITUDE - 0.5;
            ok = ok && summit_ok && clear_ok;
            let (h, level) = self.summit.unwrap_or((f64::NAN, 0));
            out.push(format!("summit_m={h:.0} height_level={level} expected={ELBRUS_HEIGHT}±{ELBRUS_TOLERANCE}"));
            out.push(format!("clearance_min_m={:.1}", self.clearance_min));
        }
        out.push(format!("requests_peak={requests_peak} requests_total={requests_total} disk_hits={disk_hits}"));
        out.push(identity());
        if let Some(f) = &self.failure {
            out.push(format!("failure={f}"));
        }
        out.push(format!("result={}", if ok { "ok" } else { "fail" }));
        (out.join("\n"), ok)
    }
}

#[cfg(test)]
mod tests {
    use super::{Scenario, min_frames_per_second, summit_height};
    use terrain::{HeightStore, HeightTile, SIZE};

    #[test]
    fn frames_per_second_windows() {
        let even: Vec<f64> = (0..300).map(|i| f64::from(i) / 60.0).collect();
        assert_eq!(min_frames_per_second(&even), Some(60));
        let mut stall = even.clone();
        stall.retain(|t| !(2.0..2.2).contains(t));
        assert_eq!(min_frames_per_second(&stall), Some(48));
        assert_eq!(min_frames_per_second(&[0.0, 0.5]), None);
    }

    #[test]
    fn scenario_names() {
        assert_eq!(Scenario::from_name("perm"), Some(Scenario::Perm));
        assert_eq!(Scenario::from_name("elbrus"), Some(Scenario::Elbrus));
        assert_eq!(Scenario::from_name("mars"), None);
    }

    #[test]
    fn summit_is_the_highest_point_around() {
        let mut store = HeightStore::new();
        let flat = HeightTile::new(0, 0, 0, vec![100.0; SIZE * SIZE]);
        store.add(flat.unwrap_or_else(|| HeightTile { z: 0, x: 0, y: 0, heights: vec![], low: 0.0, high: 0.0 }));
        let (h, level) = summit_height(&store, 43.35, 42.44, 1000.0, 20.0);
        assert!((h - 100.0).abs() < 1e-3);
        assert_eq!(level, 0);
    }
}
