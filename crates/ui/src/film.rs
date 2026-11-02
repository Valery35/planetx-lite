// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Запись тура кадрами PNG в папку, режим кадров `ui/record.py` модуля.
//!
//! Кадры идут по шкале `scene::timeline`. Каждый кадр снимается как
//! снимок вида: камера стоит в позе кадра, снимок ждёт загрузки.
//! Поэтому запись идёт медленнее тура, а готовые кадры не зависят от
//! скорости сети и видеокарты. Рядом с кадрами ложится `frames.json`.
//! Размер кадра - размер вида. Стороны чётные, нечётные не принимают
//! кодеры видео, в которых кадры собирают в ролик.

use crate::app::{crop, to_png};
use eframe::egui;
use crate::i18n::{Lang, tr, trf};
use scene::timeline::{self, Frame, Timeline};
use std::path::{Path, PathBuf};

/// Кадр, который ждёт загрузки дольше, секунды, снимается как есть
/// и попадает в список недогруженных. Иначе один неприходящий тайл
/// вешал бы запись.
pub const FRAME_WAIT: f64 = 20.0;
/// Кадров окна подряд без загрузки перед снимком. Просьбы о тайлах новой
/// позы уходят не в первом её кадре.
pub const IDLE_FRAMES: u32 = 3;

/// Шаг записи кадра.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Камере пора встать в позу кадра.
    Place,
    /// Камера стоит, вид догружается.
    Load,
    /// Снимок просили, ждётся картинка.
    Shoot,
}

/// Идущая запись тура.
pub struct Film {
    pub line: Timeline,
    folder: PathBuf,
    /// Номер кадра, который снимается.
    pub n: usize,
    pub stage: Stage,
    /// Часы окна, когда камера встала в позу кадра.
    since: f64,
    /// Кадр снят не дождавшись загрузки.
    forced: bool,
    /// Шкала времени до записи, она возвращается после.
    pub range: (f64, f64),
    frames: Vec<Frame>,
    incomplete: Vec<usize>,
    size: Option<[usize; 2]>,
}

/// В папке уже есть кадры тура: новая запись смешалась бы с ними.
pub fn taken(folder: &Path) -> bool {
    folder.join(timeline::INDEX_NAME).exists() || folder.join(timeline::frame_name(0)).exists()
}

impl Film {
    pub fn new(line: Timeline, folder: PathBuf, range: (f64, f64)) -> Self {
        Self { line, folder, n: 0, stage: Stage::Place, since: 0.0, forced: false, range, frames: Vec::new(), incomplete: Vec::new(), size: None }
    }

    /// Камера встала в позу кадра.
    pub fn placed(&mut self, now: f64) {
        self.stage = Stage::Load;
        self.since = now;
    }

    /// Пора снимать: вид догружен или кадр ждёт дольше [`FRAME_WAIT`].
    /// idle - кадров окна подряд без загрузки.
    pub fn due(&mut self, idle: u32, now: f64) -> bool {
        let loaded = idle >= IDLE_FRAMES;
        self.forced = !loaded;
        loaded || now - self.since > FRAME_WAIT
    }

    /// Записать кадр в папку и перейти к следующему.
    pub fn write(&mut self, lang: Lang, image: &egui::ColorImage) -> Result<(), String> {
        let [w, h] = image.size;
        let size = [w / 2 * 2, h / 2 * 2];
        if *self.size.get_or_insert(size) != size {
            return Err(trf(lang, "The window size changed during the recording. The recording stopped at frame {n}.", &[("n", &self.n.to_string())]));
        }
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0] as f32, size[1] as f32));
        let png = to_png(&crop(image, rect, 1.0)?)?;
        let path = self.folder.join(timeline::frame_name(self.n));
        std::fs::write(&path, png).map_err(|e| format!("{}: {e}", path.display()))?;
        self.frames.push(self.line.frame(self.n));
        if self.forced {
            self.incomplete.push(self.n);
        }
        self.n += 1;
        self.stage = Stage::Place;
        Ok(())
    }

    /// Все кадры записаны.
    pub fn finished(&self) -> bool {
        self.n >= self.line.count
    }

    /// Дописать `frames.json`. Возвращает текст для пользователя.
    pub fn finish(&self, lang: Lang) -> Result<String, String> {
        let text = self.line.index(self.size.unwrap_or_default(), &self.incomplete, &self.frames);
        let path = self.folder.join(timeline::INDEX_NAME);
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut done = trf(
            lang,
            "The tour is recorded to {folder}. Frame count {count}, {fps} per second.",
            &[("folder", &self.folder.display().to_string()), ("count", &self.n.to_string()), ("fps", &timeline::FPS.to_string())],
        );
        if !self.incomplete.is_empty() {
            done.push(' ');
            done.push_str(&trf(lang, "Frames not fully loaded: {count}.", &[("count", &self.incomplete.len().to_string())]));
        }
        Ok(done)
    }

    /// Текст о прерванной записи.
    pub fn stopped(&self, lang: Lang) -> String {
        trf(lang, "Tour recording stopped at frame {n}.", &[("n", &self.n.to_string())])
    }
}

/// Подсказка кнопки записи на панели тура.
pub fn hint(lang: Lang) -> &'static str {
    tr(lang, "Record the tour as PNG frames to a folder, 25 frames per second of the tour, in the size of the view. Every frame waits for loading, so the recording takes longer than the tour.")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{Film, Stage, taken};
    use crate::i18n::Lang;
    use eframe::egui;
    use scene::timeline::{FPS, INDEX_NAME, Timeline, frame_name};
    use scene::tour::{FOV_Y, Stop};
    use std::path::PathBuf;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("planetx-film-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn film(dir: &std::path::Path) -> Film {
        let stops = vec![Stop::new("A", 58.0, 56.2, 5000.0, 0.0, 0.0), Stop::new("B", 58.1, 56.4, 8000.0, 0.0, 0.0)];
        Film::new(Timeline::new(stops, 0.5, None, FPS, FOV_Y).unwrap(), dir.to_path_buf(), (0.0, 0.0))
    }

    fn image(w: usize, h: usize) -> egui::ColorImage {
        egui::ColorImage::new([w, h], vec![egui::Color32::from_rgb(10, 20, 30); w * h])
    }

    #[test]
    fn frame_goes_to_a_numbered_png_with_even_sides() {
        let dir = folder("even");
        let mut f = film(&dir);
        f.write(Lang::EN, &image(101, 51)).unwrap();
        let png = image::open(dir.join(frame_name(0))).unwrap();
        assert_eq!((png.width(), png.height()), (100, 50));
        assert_eq!((f.n, f.stage), (1, Stage::Place));
        assert!(!f.finished());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn frame_of_another_size_stops_the_recording() {
        let dir = folder("size");
        let mut f = film(&dir);
        f.write(Lang::EN, &image(100, 50)).unwrap();
        assert!(f.write(Lang::EN, &image(200, 100)).is_err());
        assert!(!dir.join(frame_name(1)).exists());
        assert_eq!(f.n, 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn frame_waits_for_loading_and_then_is_forced() {
        let dir = folder("wait");
        let mut f = film(&dir);
        f.placed(100.0);
        assert!(!f.due(0, 101.0));
        assert!(f.due(super::IDLE_FRAMES, 101.0));
        assert!(!f.forced);
        assert!(f.due(0, 100.0 + super::FRAME_WAIT + 1.0));
        assert!(f.forced);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn index_lists_frames_and_forced_ones() {
        let dir = folder("index");
        let mut f = film(&dir);
        f.write(Lang::EN, &image(100, 50)).unwrap();
        f.placed(0.0);
        assert!(f.due(0, super::FRAME_WAIT + 1.0));
        f.write(Lang::EN, &image(100, 50)).unwrap();
        let text = f.finish(Lang::EN).unwrap();
        assert!(text.contains('1'), "{text}");
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join(INDEX_NAME)).unwrap()).unwrap();
        assert_eq!(value["width"], 100);
        assert_eq!(value["incomplete"], serde_json::json!([1]));
        assert_eq!(value["frames"].as_array().unwrap().len(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn folder_with_frames_is_taken() {
        let dir = folder("taken");
        assert!(!taken(&dir));
        std::fs::write(dir.join(frame_name(0)), b"").unwrap();
        assert!(taken(&dir));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
