// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Одна шкала времени записи тура, `core/timeline.py` модуля.
//!
//! Время записи идёт кадрами. Кадр n - момент тура n / fps: поза камеры
//! из тура, время данных - промежуток шкалы времени, растянутый на
//! длительность тура. Без промежутка время данных стоит.
//!
//! Запись начинается с первой остановки тура, а не с позы камеры
//! в момент записи. Иначе повтор записи начинался бы с другой позы.

use crate::tour::{Stop, Tour};
use globe_core::navigation::Pose;
use serde_json::json;

/// Кадров в секунду времени тура.
pub const FPS: u32 = 25;
/// Файл рядом с кадрами: частота, размер, позы, недогруженные кадры.
pub const INDEX_NAME: &str = "frames.json";

/// Имя файла кадра n. Нули впереди держат порядок файлов в папке.
pub fn frame_name(n: usize) -> String {
    format!("frame_{n:05}.png")
}

/// Кадр записи: номер, время тура, поза камеры, время данных.
#[derive(Debug, Clone)]
pub struct Frame {
    pub number: usize,
    pub t: f64,
    pub pose: Pose,
    pub moment: Option<f64>,
}

/// Кадры тура. data - начало и конец времени данных, секунды.
#[derive(Debug, Clone)]
pub struct Timeline {
    pub tour: Tour,
    pub fps: f64,
    pub data: Option<(f64, f64)>,
    /// Количество кадров.
    pub count: usize,
}

impl Timeline {
    /// Шкала по остановкам stops с паузой pause. Без остановок - None.
    pub fn new(stops: Vec<Stop>, pause: f64, data: Option<(f64, f64)>, fps: u32, fov_y: f64) -> Option<Self> {
        let first = stops.first()?.pose_at(0.0);
        let start = Pose::new(first.lat, first.lon, first.distance, first.heading, first.tilt);
        let tour = Tour::with_fov(&start, stops, pause, fov_y);
        let fps = f64::from(fps);
        let count = (tour.duration * fps).floor() as usize + 1;
        Some(Self { tour, fps, data, count })
    }

    /// Время данных на момент тура t.
    fn moment(&self, t: f64) -> Option<f64> {
        let (begin, end) = self.data?;
        let share = if self.tour.duration > 0.0 { t / self.tour.duration } else { 1.0 };
        Some(begin + (end - begin) * share.clamp(0.0, 1.0))
    }

    /// Кадр номер n.
    pub fn frame(&self, n: usize) -> Frame {
        let t = n as f64 / self.fps;
        Frame { number: n, t, pose: self.tour.pose_at(t), moment: self.moment(t) }
    }

    /// Текст `frames.json`: по нему повтор записи сверяется с первой.
    /// size - ширина и высота кадра, incomplete - номера недогруженных
    /// кадров.
    pub fn index(&self, size: [usize; 2], incomplete: &[usize], frames: &[Frame]) -> String {
        let list: Vec<_> = frames
            .iter()
            .map(|f| {
                let p = &f.pose;
                json!({"pose": [p.lat, p.lon, p.distance, p.heading, p.tilt], "moment": f.moment})
            })
            .collect();
        json!({
            "fps": self.fps,
            "count": self.count,
            "width": size[0],
            "height": size[1],
            "incomplete": incomplete,
            "frames": list,
        })
        .to_string()
    }
}
