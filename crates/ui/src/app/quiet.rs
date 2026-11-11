// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Программа для тестов: без видеокарты, сеть - ответы по части адреса.
//! Кадр идёт без экрана, нажатие - событиями мыши у надписи кнопки.

use std::cell::RefCell;
use std::rc::Rc;

use eframe::egui;
use globe_core::lod::TileKey;

use super::{LiteApp, Loaders};

/// Тайлов нет, загрузка не идёт.
struct NoTiles(net::Stats);

impl net::TileLoader for NoTiles {
    fn want(&self, _: &[(TileKey, f64)]) {}
    fn take(&self) -> Vec<net::Loaded> {
        Vec::new()
    }
    fn busy(&self) -> bool {
        false
    }
    fn stats(&self) -> &net::Stats {
        &self.0
    }
}

/// Часть адреса и ответ.
pub type Answers = Rc<Vec<(String, Vec<u8>)>>;

/// Запрос: ответ по части адреса, у прочих адресов ответа нет.
struct Answer(Answers, RefCell<Option<String>>, net::Stats);

impl net::Request for Answer {
    fn start(&self, url: &str) {
        *self.1.borrow_mut() = Some(url.to_string());
    }
    fn take(&self) -> Option<Result<Vec<u8>, String>> {
        let url = self.1.borrow().clone()?;
        let body = self.0.iter().find(|(part, _)| url.contains(part.as_str())).map(|(_, b)| b.clone())?;
        *self.1.borrow_mut() = None;
        Some(Ok(body))
    }
    fn stats(&self) -> &net::Stats {
        &self.2
    }
}

/// Программа без сети и видеокарты, запросы отвечают по answers.
pub fn app(answers: Answers) -> LiteApp {
    let tiles = || Box::new(NoTiles(net::Stats::default())) as Box<dyn net::TileLoader>;
    let ask = move |answers: &Answers| Box::new(Answer(answers.clone(), RefCell::new(None), net::Stats::default())) as Box<dyn net::Request>;
    let a = answers.clone();
    let loaders = Loaders {
        esri: tiles(),
        osm: tiles(),
        terrain: tiles(),
        places: tiles(),
        search: ask(&answers),
        pictures: ask(&answers),
        files: ask(&answers),
        make: Box::new(move |_| tiles()),
        request: Box::new(move |_| ask(&a)),
    };
    let cc = eframe::CreationContext::_new_kittest(egui::Context::default());
    LiteApp::new(&cc, loaders, None, Box::new(|_, _| {}))
}

/// Кадр программы без экрана с событиями events, фигуры кадра.
pub fn frame(ctx: &egui::Context, app: &mut LiteApp, events: Vec<egui::Event>) -> Vec<egui::epaint::ClippedShape> {
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0))), events, ..Default::default() };
    let mut frame = eframe::Frame::_new_kittest();
    let mut out = ctx.run_ui(input, |ui| eframe::App::ui(app, ui, &mut frame));
    out.textures_delta.clear();
    out.shapes
}

/// Середина надписи text в кадре.
pub fn text_at(shapes: &[egui::epaint::ClippedShape], text: &str) -> Option<egui::Pos2> {
    fn find(shape: &egui::Shape, text: &str) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(t.pos + t.galley.rect.center().to_vec2()),
            egui::Shape::Vec(v) => v.iter().find_map(|s| find(s, text)),
            _ => None,
        }
    }
    shapes.iter().find_map(|c| find(&c.shape, text))
}

/// Щелчок мышью в точке at: наведение, нажатие и отпускание по кадрам.
pub fn click(ctx: &egui::Context, app: &mut LiteApp, at: egui::Pos2) {
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(ctx, app, vec![egui::Event::PointerMoved(at)]);
    frame(ctx, app, vec![press(true)]);
    frame(ctx, app, vec![press(false)]);
}
