// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Настольная программа PlanetX Lite.
//!
//! `planetx-lite --check perm` - самопроверка окна фазы 0, отчёт
//! в стандартный вывод, код выхода 0 - проверка пройдена.

use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use eframe::egui;
use net::OSM;
use net::native::{Loader, default_cache_dir};

/// Размер окна, решение автора от 22 октября 2026 года.
const WINDOW: [f32; 2] = [1280.0, 800.0];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let check = args.windows(2).any(|w| w[0] == "--check" && w[1] == "perm");
    let result = Arc::new(Mutex::new(None));
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default().with_title("PlanetX Lite").with_inner_size(WINDOW),
        ..Default::default()
    };
    let shared = result.clone();
    let run = eframe::run_native(
        "PlanetX Lite",
        options,
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();
            let loader = Loader::new(OSM, default_cache_dir(), Arc::new(move || ctx.request_repaint()));
            let report: ui::Report = Box::new(move |text, ok| {
                println!("{text}");
                if let Ok(mut r) = shared.lock() {
                    *r = Some(ok);
                }
            });
            Ok(Box::new(ui::LiteApp::new(cc, Box::new(loader), check, report)))
        }),
    );
    if let Err(e) = run {
        eprintln!("PlanetX Lite: {e}");
        return ExitCode::FAILURE;
    }
    let passed = result.lock().map(|r| *r).unwrap_or(None);
    match (check, passed) {
        (false, _) | (true, Some(true)) => ExitCode::SUCCESS,
        (true, _) => ExitCode::FAILURE,
    }
}
