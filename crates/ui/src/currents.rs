// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Строка «Течения»: геострофические течения NOAA CoastWatch белыми
//! бегущими струйками над заливкой скорости. Перенос строки течений
//! `ui/flow.py` и `render/flow.py` модуля, расчёт - `data::flow`.
//!
//! Сутки - правый бегунок открытой шкалы времени, без шкалы - последние
//! готовые. Файл суток около 4 МБ netCDF, в памяти не больше
//! `flow::DAYS_KEPT` суток. Частицы шагают раз в `flow::TICK_MS`, рамка
//! рождения - вид: выше `flow::DISK_FROM` - видимый диск, ниже - рамка
//! вида. Ширина вида сменилась больше чем в `flow::RESET_ZOOM` раз -
//! частицы рождаются заново. Струйки рисует экран поверх глобуса,
//! заливку - слой над картами ([`crate::vector::Painter`]).

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui;
use globe_core::DVec3;

use data::flow::{self, Particles, VectorGrid};

use crate::i18n::{Lang, tr};

/// Наибольший ответ, байт.
const ANSWER_BYTES: u64 = 32 << 20;
/// Толщина следа и сторона головы, логические пиксели, `LINE_WIDTH`
/// и `HEAD_SIZE` модуля.
pub const LINE_WIDTH: f32 = 1.8;
pub const HEAD_SIZE: f32 = 2.5;
/// Медленнее струйка бледнее до доли SLOW_FLOOR, `slow_fade` модуля.
const CURRENT_SLOW: f64 = 0.2;
const SLOW_FLOOR: f64 = 0.3;
/// Зерно частиц.
const SEED: u64 = 20261012;

/// Множитель непрозрачности струйки по скорости, м/с.
fn slow_fade(speed: f64) -> f64 {
    (speed / CURRENT_SLOW).clamp(SLOW_FLOOR, 1.0)
}

pub struct Currents {
    /// Запрос суток: сутки (пусто - последние) и запрос.
    ask: Option<(String, Box<dyn net::Request>)>,
    days: HashMap<String, Arc<VectorGrid>>,
    order: Vec<String>,
    /// Последние готовые сутки службы.
    last: Option<String>,
    /// Показанные сутки.
    pub day: Option<String>,
    /// Сутки, которые попросил бегунок, и когда.
    wanted: Option<(String, f64)>,
    particles: Particles,
    width_at_reset: f64,
    ticked: f64,
    /// Поле сменилось: заливка рисуется заново.
    pub changed: bool,
    failed: bool,
}

impl Default for Currents {
    fn default() -> Self {
        Self {
            ask: None,
            days: HashMap::new(),
            order: Vec::new(),
            last: None,
            day: None,
            wanted: None,
            particles: Particles::new(flow::CURRENTS_COUNT, SEED),
            width_at_reset: 0.0,
            ticked: f64::NEG_INFINITY,
            changed: false,
            failed: false,
        }
    }
}

impl Currents {
    /// Идёт загрузка.
    pub fn busy(&self) -> bool {
        self.ask.is_some()
    }

    /// Поле показанных суток.
    pub fn grid(&self) -> Option<Arc<VectorGrid>> {
        self.day.as_ref().and_then(|d| self.days.get(d)).cloned()
    }

    /// Шаг: сутки по бегунку, загрузка, шаг частиц. moment - правый бегунок
    /// шкалы времени или None, view - точка взгляда, высота глаза и ширина
    /// вида в метрах. Возвращает сообщение пользователю.
    pub fn step(&mut self, lang: Lang, now: f64, moment: Option<f64>, view: (f64, f64, f64, f64), request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Option<String> {
        let mut message = None;
        if let Some((day, ask)) = &self.ask
            && let Some(answer) = ask.take()
        {
            let day = day.clone();
            self.ask = None;
            match answer.map_err(|e| e.to_string()).and_then(|b| flow::read_currents(&b).map_err(|e| e.0)) {
                Ok(c) => {
                    if day.is_empty() {
                        self.last = Some(c.day.clone());
                    }
                    let key = c.day.clone();
                    self.days.insert(key.clone(), Arc::new(c.grid));
                    self.order.retain(|d| *d != key);
                    self.order.push(key.clone());
                    while self.order.len() > flow::DAYS_KEPT {
                        let old = self.order.remove(0);
                        self.days.remove(&old);
                    }
                    if self.day.as_deref() != Some(&key) {
                        self.day = Some(key);
                        self.changed = true;
                    }
                }
                Err(_) => {
                    self.failed = true;
                    message = Some(tr(lang, "Currents did not load, check the network and switch the row on again.").to_string());
                }
            }
        }
        // Сутки по бегунку: запрос после паузы движения бегунка.
        match (&self.last, self.ask.is_some()) {
            (None, false) if !self.failed => {
                let ask = request(ANSWER_BYTES);
                ask.start(&flow::currents_url(None));
                self.ask = Some((String::new(), ask));
                message = Some(tr(lang, "Loading currents, about 4 MB…").to_string());
            }
            (Some(last), false) => {
                let day = flow::current_day(moment, last);
                if self.days.contains_key(&day) {
                    if self.day.as_deref() != Some(&day) {
                        self.day = Some(day);
                        self.changed = true;
                    }
                    self.wanted = None;
                } else if self.wanted.as_ref().is_none_or(|(d, _)| *d != day) {
                    self.wanted = Some((day, now));
                } else if let Some((d, at)) = &self.wanted
                    && now - at >= f64::from(flow::DAY_PAUSE_MS) / 1000.0
                {
                    let ask = request(ANSWER_BYTES);
                    ask.start(&flow::currents_url(Some(d)));
                    self.ask = Some((d.clone(), ask));
                    self.wanted = None;
                }
            }
            _ => {}
        }
        // Шаг частиц.
        let dt = f64::from(flow::TICK_MS) / 1000.0;
        if now - self.ticked < dt {
            return message;
        }
        self.ticked = now;
        let Some(grid) = self.grid() else {
            return message;
        };
        let (lat, lon, altitude, width) = view;
        let frame = if altitude > flow::DISK_FROM {
            let angle = (flow::RADIUS / (flow::RADIUS + altitude)).acos().to_degrees() * flow::HORIZON_SHARE;
            flow::cap_box(lat, lon, angle)
        } else {
            let dlat = (width * 0.75 / flow::RADIUS).to_degrees().min(80.0);
            let dlon = (dlat / lat.to_radians().cos().max(0.05)).min(180.0);
            flow::rect_box(lon - dlon, (lat - dlat).max(-89.0), lon + dlon, (lat + dlat).min(89.0))
        };
        let zoomed = self.width_at_reset <= 0.0 || width / self.width_at_reset > flow::RESET_ZOOM || self.width_at_reset / width > flow::RESET_ZOOM;
        if self.particles.frame.is_none() || zoomed {
            self.particles.reset(frame);
            self.width_at_reset = width;
        } else {
            self.particles.frame = Some(frame);
        }
        let metres = flow::display_metres(width.min(2e7), flow::CURRENTS_TYPICAL, grid.step, dt, Some(flow::CELL_SHARE));
        self.particles.step(&grid, dt, metres);
        message
    }

    /// Струйки на экране: белые следы, голова - точка, хвост бледнеет.
    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect, ppp: f64, camera: &globe_core::camera::Camera) {
        let p = &self.particles;
        if p.frame.is_none() {
            return;
        }
        let e = camera.ellipsoid;
        let eye = camera.eye;
        let to_screen = |la: f64, lo: f64| -> Option<(egui::Pos2, DVec3)> {
            let point = e.geodetic_to_ecef(la, lo, 0.0);
            let (q, front) = camera.project(point);
            front.then(|| (rect.min + egui::vec2((q.x / ppp) as f32, (q.y / ppp) as f32), point))
        };
        let mut mesh = egui::Mesh::default();
        let half = LINE_WIDTH / 2.0;
        let trail = flow::TRAIL;
        for i in 0..p.count {
            let Some((hlat, hlon)) = p.point(i, 0) else {
                continue;
            };
            let Some((head, point)) = to_screen(hlat, hlon) else {
                continue;
            };
            // Сторона шара за горизонтом отбрасывается.
            if point.dot(eye - point) <= 0.0 || !rect.contains(head) {
                continue;
            }
            let speed = p.speed.get(i).copied().unwrap_or(0.0);
            let fade = slow_fade(speed);
            let depth = (p.age.get(i).copied().unwrap_or(0).max(0) as usize).min(trail - 1);
            let mut prev = head;
            let mut prev_lon = hlon;
            for j in 1..=depth {
                let Some((la, lo)) = p.point(i, j) else {
                    break;
                };
                if (lo - prev_lon).abs() >= 90.0 {
                    break;
                }
                let Some((next, _)) = to_screen(la, lo) else {
                    break;
                };
                let share = (j - 1) as f64 / (trail as f64 - 1.0);
                let alpha = ((1.0 - share) * fade * 255.0) as u8;
                let color = egui::Color32::from_white_alpha(alpha);
                let tail = egui::Color32::from_white_alpha((f32::from(alpha) * 0.7) as u8);
                let d = next - prev;
                let length = d.length();
                if length > 0.01 {
                    let n = egui::vec2(-d.y, d.x) / length * half;
                    let base = mesh.vertices.len() as u32;
                    mesh.colored_vertex(prev + n, color);
                    mesh.colored_vertex(prev - n, color);
                    mesh.colored_vertex(next - n, tail);
                    mesh.colored_vertex(next + n, tail);
                    mesh.add_triangle(base, base + 1, base + 2);
                    mesh.add_triangle(base, base + 2, base + 3);
                }
                prev = next;
                prev_lon = lo;
            }
            painter.circle_filled(head, HEAD_SIZE / 2.0, egui::Color32::from_white_alpha((fade * 255.0) as u8));
        }
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// Широты строк и долготы столбцов центров пикселей тайла.
fn tile_axes(key: globe_core::lod::TileKey) -> (Vec<f64>, Vec<f64>) {
    let (z, x, y) = key;
    let n = f64::from(1u32 << z.min(30));
    let size = 256usize;
    let lats = (0..size).map(|r| (std::f64::consts::PI * (1.0 - 2.0 * (f64::from(y) + (r as f64 + 0.5) / size as f64) / n)).sinh().atan().to_degrees()).collect();
    let lons = (0..size).map(|c| (f64::from(x) + (c as f64 + 0.5) / size as f64) / n * 360.0 - 180.0).collect();
    (lats, lons)
}

/// Заливка скорости слоем над картами.
pub fn painter(grid: Arc<VectorGrid>) -> crate::vector::Painter {
    let job = Arc::new(flow::fill_job(grid));
    Arc::new(move |key| {
        let (lats, lons) = tile_axes(key);
        let all_lats: Vec<f64> = lats.iter().flat_map(|la| std::iter::repeat_n(*la, lons.len())).collect();
        let all_lons: Vec<f64> = lats.iter().flat_map(|_| lons.iter().copied()).collect();
        let pixels = flow::speed_rgba(&job, &all_lats, &all_lons);
        pixels.iter().any(|p| p[3] > 0).then(|| pixels.concat())
    })
}
