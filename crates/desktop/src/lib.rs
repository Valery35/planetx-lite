// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Запуск настольной программы - общий у PlanetX Lite и RustPlanetX,
//! `doc/EDITIONS.md`. Программы различаются изданием [`ui::Edition`]:
//! названием окна и связью с RustGIS. Только программа, не страница.
//!
//! `--check perm`, `--check elbrus` или `--check scenes` - самопроверка,
//! отчёт в стандартный вывод, код выхода 0 - проверка пройдена. Сцены
//! демо для `scenes` - из папки `--demo папка`, `PLANETX_DEMO` или `demo`
//! рядом с программой, без них - из папки `demo` хранилища
//! planetx-terrain по ссылке. `--demo` принимает и начало адреса.
//! `PLANETX_SCENES=brienz,japan` - только эти сцены.
//! `PLANETX_BUILTIN_FONTS` - встроенный шрифт вместо системного.
//!
//! `--pose широта,долгота,расстояние,азимут,наклон` - начать с этой позы
//! камеры. За наклоном могут идти высота точки взгляда и крен, как
//! в ссылке «Поделиться».
//!
//! `--shot файл.png` - когда место загрузится, снять окно в файл PNG
//! и закрыться. С `--ui` в снимке и левая панель.
//!
//! `программа сцена.planetx` - открыть сцену, KML или KMZ. Растр высот
//! GeoTIFF врезается своим рельефом. Файлов может быть несколько, они
//! открываются по порядку. Файл можно и бросить в окно.
//! `программа https://…/tileset.json` - открыть файл по ссылке: сцену,
//! KML, набор 3D Tiles.
//!
//! `программа mcp [--port N]` - мост клиента MCP (Claude Code, Claude
//! Desktop) к открытой программе, у которой в «Настройках помощника»
//! стоит флажок «Пускать агентов MCP». Окна не открывает. `--mcp` -
//! открыть программу с этим флажком.
//!
//! `--rustgis` - открыть программу со связью с открытым RustGIS
//! (`ui::rustgislink`), у RustPlanetX связь есть всегда. `rustgis-plugin` -
//! команда расширения RustGIS, её запускает RustGIS.

use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use eframe::egui;
use net::native::{Loader, Requests, default_cache_dir};
use net::{ESRI, OPENFREEMAP, OSM, TERRARIUM};
use ui::{Edition, Scenario};

/// Предел файла сцены по ссылке, байт.
const SCENE_BYTES: u64 = 64 << 20;

/// Размер окна, решение автора от 22 октября 2026 года.
const WINDOW: [f32; 2] = [1280.0, 800.0];

/// Имя хранилища eframe и папки данных. Одно у обеих программ: «Мои
/// метки», настройки и «Мои слои» общие, как были до разделения программ,
/// `doc/EDITIONS.md`, шаг 1.
const STORAGE: &str = "PlanetX Lite";
/// Папка данных пользователя для «Моих слоёв».
const USER_DIR: &str = "PlanetX-Lite";

/// Окно на Linux - через X11, если он есть, в том числе XWayland.
///
/// Композитор Wayland отключает клиента, который не успевает читать
/// события: буфер сокета у libwayland мал, и движение мыши заполняет
/// его за доли секунды паузы кадра. Программа получала «Io error:
/// Connection reset by peer» или «Broken pipe» и закрывалась при
/// открытии большого KML (сообщение пользователя Zorin OS 18.1,
/// 23 октября 2026 года). X-сервер копит события, пока клиент занят.
/// `PLANETX_WAYLAND=1` или отсутствие `DISPLAY` - Wayland.
#[cfg(target_os = "linux")]
fn display_server() -> Option<eframe::EventLoopBuilderHook> {
    use winit::platform::x11::EventLoopBuilderExtX11;
    let wayland = std::env::var_os("PLANETX_WAYLAND").is_some_and(|v| v == "1");
    if wayland || std::env::var_os("DISPLAY").is_none() {
        return None;
    }
    Some(Box::new(|builder| {
        builder.with_x11();
    }))
}

#[cfg(not(target_os = "linux"))]
fn display_server() -> Option<eframe::EventLoopBuilderHook> {
    None
}

/// Глобус со связью с RustGIS - отдельным процессом этой же программы.
/// RustGIS останавливает расширение-процесс вскоре после ответа, глобус
/// должен его пережить: свой процесс без консоли, вне задания RustGIS,
/// если задание это разрешает.
fn launch_linked() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let spawn = |flags: u32| {
        let mut cmd = std::process::Command::new(&exe);
        cmd.arg("--rustgis").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(flags);
        }
        #[cfg(not(windows))]
        let _ = flags;
        cmd.spawn().map(|_| ())
    };
    // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP, сначала с
    // CREATE_BREAKAWAY_FROM_JOB.
    const DETACHED: u32 = 0x0000_0008 | 0x0000_0200;
    const BREAKAWAY: u32 = 0x0100_0000;
    spawn(DETACHED | BREAKAWAY).or_else(|_| spawn(DETACHED)).map_err(|e| e.to_string())
}

/// Настольная программа издания edition: ключи командной строки, окно,
/// загрузчики. Код выхода - итог самопроверки или запуска.
pub fn main(edition: Edition) -> ExitCode {
    let title = edition.name();
    let args: Vec<String> = std::env::args().collect();
    // `mcp` - мост stdio клиента MCP к открытой программе, без окна
    // (`ui::mcprelay`). Порт - `--port N` или PLANETX_MCP_PORT.
    if args.get(1).is_some_and(|a| a == "mcp") {
        let port = args.windows(2).find(|w| w[0] == "--port").and_then(|w| w[1].parse().ok()).unwrap_or_else(ui::mcprelay::port);
        return match ui::mcprelay::bridge(port) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{title} MCP: {e}");
                ExitCode::FAILURE
            }
        };
    }
    // `rustgis-plugin` - команда расширения RustGIS: запрос RustGIS
    // на stdin, ответ на stdout, глобус со связью - отдельным процессом
    // (`ui::rustgislink::plugin`).
    if args.get(1).is_some_and(|a| a == "rustgis-plugin") {
        let relay_on = ui::rustgislink::token_path(ui::rustgislink::port()).is_file();
        let stdin = std::io::stdin();
        let mut input = stdin.lock();
        let mut output = std::io::stdout();
        return match ui::rustgislink::plugin(&mut input, &mut output, relay_on, &launch_linked) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{title} RustGIS: {e}");
                ExitCode::FAILURE
            }
        };
    }
    let check_arg = args.windows(2).find(|w| w[0] == "--check").map(|w| w[1].clone());
    let scenario = check_arg.as_deref().and_then(Scenario::from_name);
    if let (Some(name), None) = (&check_arg, scenario) {
        eprintln!("{title}: неизвестная самопроверка {name}, есть perm, elbrus, scenes и addin");
        return ExitCode::FAILURE;
    }
    let check = scenario.is_some();
    let pose = match args.windows(2).find(|w| w[0] == "--pose").map(|w| ui::share::parse_pose(&w[1])) {
        Some(Some(p)) => Some(p),
        Some(None) => {
            eprintln!("{title}: поза - широта,долгота,расстояние,азимут,наклон");
            return ExitCode::FAILURE;
        }
        None => None,
    };
    let shot = args.windows(2).find(|w| w[0] == "--shot").map(|w| std::path::PathBuf::from(&w[1]));
    // Снимки события «источник/id» при запуске, например для снимка кадра.
    let event = args.windows(2).find(|w| w[0] == "--event").map(|w| w[1].clone());
    // Сцены демо: --demo, PLANETX_DEMO, «demo» рядом с программой, иначе
    // planetx-terrain по ссылке.
    let demo = args
        .windows(2)
        .find(|w| w[0] == "--demo")
        .map(|w| w[1].clone())
        .or_else(|| std::env::var_os("PLANETX_DEMO").map(|d| d.to_string_lossy().into_owned()))
        .or_else(|| {
            let dir = std::env::current_exe().ok()?.parent()?.join("demo");
            dir.is_dir().then(|| dir.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| ui::demos::BASE.to_string());
    // Кэш файлов демо - рядом с кэшем тайлов.
    let demo_cache = default_cache_dir().and_then(|d| Some(d.parent()?.join("demo")));
    // Файлы - аргументы без «--», которые не значения флагов, по порядку:
    // растр высот, потом сцена.
    const VALUED: [&str; 5] = ["--check", "--pose", "--shot", "--demo", "--event"];
    let plain: Vec<&String> = args
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(i, a)| !a.starts_with("--") && !args.get(i - 1).is_some_and(|prev| VALUED.contains(&prev.as_str())))
        .map(|(_, a)| a)
        .collect();
    // Ссылки открываются по сети, как `?scene=` страницы: сцена, KML,
    // набор 3D Tiles.
    let (links, paths): (Vec<&String>, Vec<&String>) = plain.into_iter().partition(|a| ui::tilesets::is_link(a));
    let links: Vec<String> = links.into_iter().cloned().collect();
    let files: Vec<std::path::PathBuf> = paths.into_iter().map(std::path::PathBuf::from).collect();
    let mut opened = Vec::new();
    for path in &files {
        match std::fs::read(path) {
            Ok(bytes) => opened.push((bytes, path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()))),
            Err(e) => {
                eprintln!("{title}: {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        }
    }
    let result = Arc::new(Mutex::new(None));
    let mut viewport = egui::ViewportBuilder::default().with_title(title).with_inner_size(WINDOW);
    if let Some(icon) = ui::credits::window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport,
        event_loop_builder: display_server(),
        ..Default::default()
    };
    let shared = result.clone();
    let run = eframe::run_native(
        STORAGE,
        options,
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();
            let repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(move || ctx.request_repaint());
            let loaders = ui::Loaders {
                esri: Box::new(Loader::new(ESRI, default_cache_dir(), repaint.clone())),
                osm: Box::new(Loader::new(OSM, default_cache_dir(), repaint.clone())),
                terrain: Box::new(Loader::new(TERRARIUM, default_cache_dir(), repaint.clone())),
                places: Box::new(Loader::new(OPENFREEMAP, default_cache_dir(), repaint.clone())),
                search: Box::new(Requests::new(repaint.clone())),
                pictures: Box::new(Requests::with_limit(repaint.clone(), ui::pictures::MAX_BYTES)),
                files: Box::new(Requests::with_limit(repaint.clone(), SCENE_BYTES)),
                make: {
                    let repaint = repaint.clone();
                    Box::new(move |source| Box::new(Loader::new(source, default_cache_dir(), repaint.clone())))
                },
                request: Box::new(move |limit| Box::new(Requests::with_limit(repaint.clone(), limit))),
            };
            let report: ui::Report = Box::new(move |text, ok| {
                println!("{text}");
                if let Ok(mut r) = shared.lock() {
                    *r = Some(ok);
                }
            });
            let mut app = ui::LiteApp::new(cc, loaders, scenario, report).with_edition(edition).with_scenes(&demo);
            if let Some(dir) = demo_cache {
                app = app.with_demo_cache(dir);
            }
            // «Мои слои» - в папке данных пользователя, не в кэше.
            if let Some(dir) = dirs::data_dir() {
                app = app.with_user_dir(dir.join(USER_DIR));
            }
            // Соседние файлы шейпфайла - из папки первого файла.
            if let Some(dir) = files.first().and_then(|p| p.parent()) {
                app = app.with_open_dir(if dir.as_os_str().is_empty() { std::path::PathBuf::from(".") } else { dir.to_path_buf() });
            }
            for (bytes, name) in opened {
                app = app.with_file(bytes, &name);
            }
            for link in &links {
                app = app.with_scene_url(link);
            }
            if let Some(p) = pose {
                app = app.with_pose(p);
            }
            if let Some(key) = &event {
                app = app.with_event(key);
            }
            if args.iter().any(|a| a == "--ui") {
                app = app.with_shot_ui();
            }
            if args.iter().any(|a| a == "--mcp") {
                app = app.with_mcp();
            }
            // У RustPlanetX связь с RustGIS есть всегда, у Lite - по ключу.
            if edition.linked() || args.iter().any(|a| a == "--rustgis") {
                app = app.with_rustgis();
            }
            if let Some(path) = shot {
                app = app.with_shot(Box::new(move |image| {
                    let [w, h] = image.size;
                    let saved = image::save_buffer(&path, image.as_raw(), w as u32, h as u32, image::ExtendedColorType::Rgba8);
                    match saved {
                        Ok(()) => println!("shot={}", path.display()),
                        Err(e) => eprintln!("{title}: снимок не записан: {e}"),
                    }
                }));
            }
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = run {
        eprintln!("{title}: {e}");
        return ExitCode::FAILURE;
    }
    let passed = result.lock().map(|r| *r).unwrap_or(None);
    match (check, passed) {
        (false, _) | (true, Some(true)) => ExitCode::SUCCESS,
        (true, _) => ExitCode::FAILURE,
    }
}
