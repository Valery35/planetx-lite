// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Загрузка снимков полярных шапок, перенос `ui/polar.py` модуля.
//! Проекции и мозаика - `globe_core::polar`, подгонка цвета к подложке
//! у края - в рендере (`Globe::set_polar_image`).
//!
//! Снимок полюса просится один раз за сеанс, когда шапку видно
//! (`polar::needed`), - 36 тайлов служб Esri, не больше [`AT_ONCE`]
//! запросов сразу. Отказ больше половины тайлов - снимок не ставится,
//! повтор через [`RETRY`] с. Шапка берёт снимок только при подложке-
//! снимке на Земле, при карте - цвет края, как у модуля.

use globe_core::polar::{self, Layout};

/// Секунд до повтора после отказа, `RETRY` модуля.
pub const RETRY: f64 = 300.0;
/// Запросов тайлов полюса сразу.
pub const AT_ONCE: usize = 8;
/// Наибольший ответ на тайл, байт.
const TILE_BYTES: u64 = 2 << 20;

/// Мозаика полюса: ширина, высота, RGBA.
pub type Image = (usize, usize, Vec<u8>);

/// Загрузка тайлов одного полюса.
struct Job {
    layout: Layout,
    /// Тайлы, которые ещё не просились.
    waiting: Vec<(i64, i64, String)>,
    asks: Vec<(i64, i64, Box<dyn net::Request>)>,
    images: Vec<(i64, i64, Vec<u8>)>,
    bad: usize,
    total: usize,
}

#[derive(Default)]
pub struct PolarCaps {
    jobs: [Option<Job>; 2],
    failed_at: [Option<f64>; 2],
    /// Готовые мозаики полюсов, северного и южного.
    pub images: [Option<Image>; 2],
}

/// Тайл RGBA 256 × 256 из PNG или JPEG.
fn decode(bytes: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    (img.dimensions() == (polar::SIDE as u32, polar::SIDE as u32)).then(|| img.into_raw())
}

impl PolarCaps {
    pub fn busy(&self) -> bool {
        self.jobs.iter().any(Option::is_some)
    }

    /// Шаг: попросить снимки видимых шапок, принять тайлы, собрать
    /// мозаику. lat - широта точки взгляда, altitude - высота глаза, м.
    pub fn step(&mut self, lat: f64, altitude: f64, now: f64, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        for north in polar::needed(lat, altitude) {
            let i = usize::from(!north);
            if self.images[i].is_some() || self.jobs[i].is_some() || self.failed_at[i].is_some_and(|t| now - t < RETRY) {
                continue;
            }
            let layout = Layout::new(north, polar::LEVEL);
            let waiting = layout.tiles();
            let total = waiting.len();
            self.jobs[i] = Some(Job { layout, waiting, asks: Vec::new(), images: Vec::new(), bad: 0, total });
        }
        for i in 0..2 {
            let Some(job) = &mut self.jobs[i] else {
                continue;
            };
            while job.asks.len() < AT_ONCE {
                let Some((row, col, url)) = job.waiting.pop() else {
                    break;
                };
                let ask = request(TILE_BYTES);
                ask.start(&url);
                job.asks.push((row, col, ask));
            }
            let mut k = 0;
            while k < job.asks.len() {
                match job.asks[k].2.take() {
                    Some(answer) => {
                        let (row, col, _) = job.asks.swap_remove(k);
                        match answer.ok().and_then(|b| decode(&b)) {
                            Some(rgba) => job.images.push((row, col, rgba)),
                            None => job.bad += 1,
                        }
                    }
                    None => k += 1,
                }
            }
            if !job.waiting.is_empty() || !job.asks.is_empty() {
                continue;
            }
            let Some(job) = self.jobs[i].take() else {
                continue;
            };
            if job.bad * 2 > job.total {
                self.failed_at[i] = Some(now);
                continue;
            }
            self.images[i] = Some(job.layout.mosaic(&job.images));
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Поддельная служба: каждый тайл - PNG одного серого цвета.
    struct Fake {
        url: RefCell<String>,
        stats: net::Stats,
        asked: Rc<RefCell<usize>>,
    }

    impl net::Request for Fake {
        fn start(&self, url: &str) {
            *self.url.borrow_mut() = url.to_string();
            *self.asked.borrow_mut() += 1;
        }
        fn take(&self) -> Option<Result<Vec<u8>, String>> {
            let url = self.url.borrow().clone();
            if url.is_empty() {
                return None;
            }
            let img = image::RgbaImage::from_pixel(256, 256, image::Rgba([90, 100, 110, 255]));
            let mut bytes = Vec::new();
            image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
            Some(Ok(bytes))
        }
        fn stats(&self) -> &net::Stats {
            &self.stats
        }
    }

    #[test]
    fn far_view_loads_both_poles_once() {
        let asked = Rc::new(RefCell::new(0));
        let request = |_: u64| -> Box<dyn net::Request> { Box::new(Fake { url: RefCell::new(String::new()), stats: net::Stats::default(), asked: asked.clone() }) };
        let mut caps = PolarCaps::default();
        // У экватора близко - полюса не нужны.
        caps.step(0.0, 1.0e5, 0.0, &request);
        assert!(!caps.busy());
        for n in 0..20 {
            caps.step(0.0, 2.0e7, f64::from(n), &request);
        }
        assert!(!caps.busy());
        let (w, h, rgba) = caps.images[1].as_ref().unwrap();
        assert_eq!((*w, *h), (6 * 256, 6 * 256));
        assert_eq!(&rgba[..3], &[90, 100, 110]);
        assert!(caps.images[0].is_some());
        assert_eq!(*asked.borrow(), 72);
        // Второй раз за сеанс не просится.
        caps.step(0.0, 2.0e7, 30.0, &request);
        assert_eq!(*asked.borrow(), 72);
    }
}
