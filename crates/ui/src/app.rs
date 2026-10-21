// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно: глобус с подложкой OSM и навигация мышью и пальцами.

use std::sync::atomic::Ordering;

use eframe::{egui, egui_wgpu};
use globe_core::camera::Camera;
use globe_core::ellipsoid::Ellipsoid;
use globe_core::lod::{self, Memo, Options, TileKey};
use globe_core::navigation::{Navigator, Pose};
use globe_core::{DMat3, DVec3};
use net::TileLoader;
use render::{FrameOptions, Globe, wgpu};

use crate::check::{self, Check};
use crate::i18n::{Lang, OSM_COPYRIGHT, Text, text};
use crate::input::Input;

/// Сколько снимков тайлов держать в видеокарте, сверх нужных кадру
/// старые вытесняются. Снимок 256 × 256 занимает 256 КБ.
pub const TEXTURE_BUDGET: usize = 800;
/// Начальный вид - Пермь из космоса с 20 000 км, отвесно, север вверху.
/// Решение автора от 22 октября 2026 года.
pub const START: (f64, f64, f64) = (check::PERM.0, check::PERM.1, check::SPACE_DISTANCE);

/// Куда отдать отчёт самопроверки: текст и итог, true - пройдена.
pub type Report = Box<dyn FnMut(&str, bool)>;

/// Тайлы уровней 0-2: они нужны кадру всегда, кадр без них нечем
/// закрыть Землю.
fn start_keys() -> Vec<TileKey> {
    (0..=2u32).flat_map(|z| (0..1u32 << z).flat_map(move |x| (0..1u32 << z).map(move |y| (z, x, y)))).collect()
}

pub struct LiteApp {
    lang: Lang,
    render_state: Option<egui_wgpu::RenderState>,
    globe: Option<Globe>,
    texture: Option<egui::TextureId>,
    nav: Navigator,
    loader: Box<dyn TileLoader>,
    input: Input,
    memo: Memo,
    check: Option<Check>,
    report: Report,
}

impl LiteApp {
    /// check - запустить самопроверку, её отчёт уходит в report.
    pub fn new(cc: &eframe::CreationContext<'_>, loader: Box<dyn TileLoader>, check: bool, report: Report) -> Self {
        let render_state = cc.wgpu_render_state.clone();
        let globe = render_state.as_ref().map(|rs| Globe::new(&rs.device, &rs.queue));
        let camera = Camera::new(Ellipsoid::wgs84(), DVec3::ZERO, DMat3::IDENTITY);
        let pose = if check { check::start_pose() } else { Pose::new(START.0, START.1, START.2, 0.0, 0.0) };
        Self {
            lang: Lang::from_system(),
            render_state,
            globe,
            texture: None,
            nav: Navigator::new(camera, pose),
            loader,
            input: Input::default(),
            memo: Memo::new(),
            check: check.then(Check::new),
            report,
        }
    }

    /// Снимки пришедших тайлов - в видеокарту.
    fn upload(&mut self) {
        let Some(globe) = &mut self.globe else {
            return;
        };
        for loaded in self.loader.take() {
            if let Ok(image) = loaded.image {
                globe.upload(loaded.key, image.width, image.height, &image.rgba);
            }
        }
    }

    /// Показ кадра глобуса в egui, текстура регистрируется заново после
    /// смены размера.
    fn show_target(&mut self, recreated: bool) {
        let (Some(rs), Some(globe)) = (&self.render_state, &self.globe) else {
            return;
        };
        let Some(view) = globe.color_view() else {
            return;
        };
        let mut renderer = rs.renderer.write();
        match self.texture {
            Some(id) if recreated => {
                renderer.update_egui_texture_from_wgpu_texture(&rs.device, view, wgpu::FilterMode::Nearest, id);
            }
            Some(_) => {}
            None => {
                self.texture = Some(renderer.register_native_texture(&rs.device, view, wgpu::FilterMode::Nearest));
            }
        }
    }

    fn attribution(&self, ctx: &egui::Context) {
        egui::Area::new(egui::Id::unique("attribution"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-6.0, -6.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_white_alpha(200))
                    .inner_margin(egui::Margin::same(4))
                    .corner_radius(3.0)
                    .show(ui, |ui| {
                        ui.hyperlink_to(
                            egui::RichText::new(text(self.lang, Text::Attribution)).color(egui::Color32::BLACK),
                            OSM_COPYRIGHT,
                        );
                    });
            });
    }

    fn finish_check(&mut self, ctx: &egui::Context) {
        let Some(check) = &self.check else {
            return;
        };
        if !check.done() {
            return;
        }
        let stats = self.loader.stats();
        let (report, ok) = check.report(
            stats.requests_peak.load(Ordering::SeqCst),
            stats.requests_total.load(Ordering::SeqCst),
            stats.disk_hits.load(Ordering::SeqCst),
            (self.nav.camera.width, self.nav.camera.height),
        );
        (self.report)(&report, ok);
        self.check = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for LiteApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.upload();

        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, egui::Sense::click_and_drag());
        // Пикселей кадра глобуса на логический пиксель. Кадр больше
        // наибольшей стороны текстуры видеокарты рисуется в меньшем
        // разрешении, экран телефона бывает больше 4096 пикселей.
        let max_side = f64::from(self.globe.as_ref().map_or(u32::MAX, Globe::max_side));
        let ppp = f64::from(ctx.pixels_per_point());
        let longest = f64::from(rect.width().max(rect.height())) * ppp;
        let ppp = if longest > max_side { ppp * max_side / longest } else { ppp };
        let width = ((f64::from(rect.width()) * ppp).floor() as u32).max(1);
        let height = ((f64::from(rect.height()) * ppp).floor() as u32).max(1);
        self.nav.camera.width = width;
        self.nav.camera.height = height;

        if let Some(check) = &mut self.check {
            check.before_frame(&mut self.nav, now, self.loader.busy());
        } else {
            self.input.handle(&ctx, rect, &mut self.nav, now, ppp);
        }
        let moving = self.nav.step(now);

        let Some(globe) = &mut self.globe else {
            ui.centered_and_justified(|ui| ui.label(text(self.lang, Text::NoGpu)));
            return;
        };
        let selection = {
            let ready = |k: TileKey| k.0 <= 2 || globe.has_texture(k);
            let memo = self.memo.bind(&self.nav.camera, lod::THRESHOLD, 0, &[]);
            lod::select(&self.nav.camera, &ready, Options { memo: Some(memo), ..Options::default() })
        };
        let mut wanted: Vec<(TileKey, f64)> = start_keys()
            .into_iter()
            .filter(|k| !globe.has_texture(*k))
            .map(|k| (k, lod::INFINITE_PRIORITY + 10.0 - f64::from(k.0)))
            .collect();
        wanted.extend(selection.want.iter().copied());
        self.loader.want(&wanted);

        let recreated = globe.resize(width, height);
        let count_holes = self.check.as_ref().is_some_and(Check::count_holes);
        let stats = globe.render(&self.nav.camera, &selection, FrameOptions { count_holes, show_holes: false });
        globe.evict(&selection.keep, TEXTURE_BUDGET);
        self.show_target(recreated || self.texture.is_none());
        if let Some(id) = self.texture {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(id, rect, uv, egui::Color32::WHITE);
        }
        self.attribution(&ctx);

        if let Some(check) = &mut self.check {
            check.after_frame(&stats);
        }
        self.finish_check(&ctx);
        if moving || self.loader.busy() || self.check.is_some() {
            ctx.request_repaint();
        }
    }
}
