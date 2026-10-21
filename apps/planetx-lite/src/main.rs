// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Настольная программа PlanetX Lite.
//!
//! `planetx-lite --check perm` или `--check elbrus` - самопроверка,
//! отчёт в стандартный вывод, код выхода 0 - проверка пройдена.
//!
//! `planetx-lite --pose широта,долгота,расстояние,азимут,наклон` -
//! начать с этой позы камеры.
//!
//! `planetx-lite --shot файл.png` - когда место загрузится, снять окно
//! в файл PNG и закрыться.

use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use eframe::egui;
use net::native::{Loader, default_cache_dir};
use net::{ESRI, OPENFREEMAP, OSM, TERRARIUM};
use ui::Scenario;

/// Размер окна, решение автора от 22 октября 2026 года.
const WINDOW: [f32; 2] = [1280.0, 800.0];

/// Поза из строки «широта,долгота,расстояние,азимут,наклон».
fn parse_pose(text: &str) -> Option<globe_core::navigation::Pose> {
    let v: Vec<f64> = text.split(',').map(|p| p.trim().parse::<f64>()).collect::<Result<_, _>>().ok()?;
    let [lat, lon, distance, heading, tilt] = v.as_slice() else {
        return None;
    };
    let ok = v.iter().all(|x| x.is_finite()) && lat.abs() <= 90.0 && *distance > 0.0;
    ok.then(|| globe_core::navigation::Pose::new(*lat, *lon, *distance, *heading, *tilt))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let check_arg = args.windows(2).find(|w| w[0] == "--check").map(|w| w[1].clone());
    let scenario = check_arg.as_deref().and_then(Scenario::from_name);
    if let (Some(name), None) = (&check_arg, scenario) {
        eprintln!("PlanetX Lite: неизвестная самопроверка {name}, есть perm и elbrus");
        return ExitCode::FAILURE;
    }
    let check = scenario.is_some();
    let pose = match args.windows(2).find(|w| w[0] == "--pose").map(|w| parse_pose(&w[1])) {
        Some(Some(p)) => Some(p),
        Some(None) => {
            eprintln!("PlanetX Lite: поза - широта,долгота,расстояние,азимут,наклон");
            return ExitCode::FAILURE;
        }
        None => None,
    };
    let shot = args.windows(2).find(|w| w[0] == "--shot").map(|w| std::path::PathBuf::from(&w[1]));
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
            let repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(move || ctx.request_repaint());
            let loaders = ui::Loaders {
                esri: Box::new(Loader::new(ESRI, default_cache_dir(), repaint.clone())),
                osm: Box::new(Loader::new(OSM, default_cache_dir(), repaint.clone())),
                terrain: Box::new(Loader::new(TERRARIUM, default_cache_dir(), repaint.clone())),
                places: Box::new(Loader::new(OPENFREEMAP, default_cache_dir(), repaint)),
            };
            let report: ui::Report = Box::new(move |text, ok| {
                println!("{text}");
                if let Ok(mut r) = shared.lock() {
                    *r = Some(ok);
                }
            });
            let mut app = ui::LiteApp::new(cc, loaders, scenario, report);
            if let Some(p) = pose {
                app = app.with_pose(p);
            }
            if let Some(path) = shot {
                app = app.with_shot(Box::new(move |image| {
                    let [w, h] = image.size;
                    let saved = image::save_buffer(&path, image.as_raw(), w as u32, h as u32, image::ExtendedColorType::Rgba8);
                    match saved {
                        Ok(()) => println!("shot={}", path.display()),
                        Err(e) => eprintln!("PlanetX Lite: снимок не записан: {e}"),
                    }
                }));
            }
            Ok(Box::new(app))
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
