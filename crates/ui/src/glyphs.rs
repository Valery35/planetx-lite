// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Цифры подписей горизонталей для рабочих потоков, как `digit_glyphs`
//! (`ui/contours.py`) модуля: альфа цифр, минуса и точки, рисуется один
//! раз, потоки только читают.
//!
//! Знаки - встроенным шрифтом egui Ubuntu Light: контуры читает skrifa,
//! заливает tiny-skia. Модуль рисует подписи полужирным, здесь тонкий
//! шрифт утолщается на пиксель по горизонтали.

use data::contours::Glyphs;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, MetadataProvider};

/// Кегль цифр в пикселях тайла, `LABEL_PIXELS` модуля.
pub const LABEL_PIXELS: f32 = 11.0;
/// Знаки подписей.
const CHARS: &str = "0123456789-.";

/// Перо skrifa в путь tiny-skia: ось y знака вверх, картинки - вниз.
struct Pen {
    path: tiny_skia::PathBuilder,
    top: f32,
}

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to(x, self.top - y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to(x, self.top - y);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.path.quad_to(cx0, self.top - cy0, x, self.top - y);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.path.cubic_to(cx0, self.top - cy0, cx1, self.top - cy1, x, self.top - y);
    }
    fn close(&mut self) {
        self.path.close();
    }
}

/// Цифры подписей. None - шрифт не прочитан, подписей не будет.
pub fn digits() -> Option<Glyphs> {
    let font = FontRef::new(epaint_default_fonts::UBUNTU_LIGHT).ok()?;
    let size = Size::new(LABEL_PIXELS);
    let location = LocationRef::default();
    let ascent = font.metrics(size, location).ascent;
    let height = ascent.ceil() as usize + 1;
    let advances = font.glyph_metrics(size, location);
    let outlines = font.outline_glyphs();
    let mut out = Glyphs { height, ..Glyphs::default() };
    for ch in CHARS.chars() {
        let gid = font.charmap().map(ch)?;
        // Утолщение на пиксель - и знак на пиксель шире.
        let width = advances.advance_width(gid).unwrap_or(LABEL_PIXELS / 2.0).ceil().max(1.0) as usize + 1;
        let mut pen = Pen { path: tiny_skia::PathBuilder::new(), top: ascent };
        outlines.get(gid)?.draw(DrawSettings::unhinted(size, location), &mut pen).ok()?;
        let mut alpha = vec![0f32; width * height];
        if let Some(path) = pen.path.finish() {
            let mut pixmap = tiny_skia::Pixmap::new(width as u32, height as u32)?;
            let mut paint = tiny_skia::Paint::default();
            paint.set_color_rgba8(255, 255, 255, 255);
            paint.anti_alias = true;
            pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
            for (i, px) in pixmap.pixels().iter().enumerate() {
                alpha[i] = f32::from(px.alpha()) / 255.0;
            }
        }
        // Полужирный: альфа - наибольшая из своей и соседа слева.
        let bold: Vec<f32> = (0..width * height).map(|i| if i % width == 0 { alpha[i] } else { alpha[i].max(alpha[i - 1]) }).collect();
        out.chars.insert(ch, (width, bold));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn digits_have_ink() {
        let g = super::digits();
        assert!(g.is_some());
        let Some(g) = g else {
            return;
        };
        assert!(g.height >= 9);
        for ch in "0123456789-.".chars() {
            let (w, a) = g.chars.get(&ch).cloned().unwrap_or_default();
            assert_eq!(a.len(), w * g.height);
            assert!(a.iter().any(|v| *v > 0.5), "{ch}");
        }
    }
}
