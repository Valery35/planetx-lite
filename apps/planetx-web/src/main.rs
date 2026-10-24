// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Страница PlanetX Lite. Собирается Trunk под `wasm32-unknown-unknown`.
//!
//! Параметры адреса:
//! - `?check=perm` - самопроверка, отчёт в консоль браузера, поверх
//!   страницы и в заголовок окна;
//! - `?gl` - только WebGL2, как у телефона на странице по http;
//! - `?scene=адрес` - открыть сцену `.planetx`, KML или KMZ по ссылке.
//!   Параметров `scene` может быть несколько, они открываются по порядку;
//! - `?pose=широта,долгота,расстояние,азимут,наклон` - поза камеры,
//!   как у ссылки «Поделиться» (`ui::share`);
//! - `?demo=адрес` - откуда `?check=scenes` берёт сцены демо, по умолчанию
//!   папка `demo` хранилища planetx-terrain.
//!
//! Ошибка программы тоже выводится поверх страницы и в заголовок, её
//! видно и на телефоне без консоли.

#[cfg(target_arch = "wasm32")]
fn main() {
    use std::rc::Rc;
    use std::sync::Arc;

    use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
    use wasm_bindgen::JsCast;

    /// Текст - в консоль, поверх страницы и в заголовок окна.
    fn show(text: &str) {
        web_sys::console::log_1(&text.into());
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        if let Some(out) = doc.get_element_by_id("report") {
            out.set_text_content(Some(text));
        }
        doc.set_title(&text.replace('\n', " "));
    }

    std::panic::set_hook(Box::new(|info| show(&format!("PlanetX Lite, ошибка: {info}"))));

    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let search = window.location().search().unwrap_or_default();
    let params: Vec<&str> = search.trim_start_matches('?').split('&').collect();
    let scenario = params.iter().find_map(|p| p.strip_prefix("check=")).and_then(ui::Scenario::from_name);
    let gl_only = params.contains(&"gl");
    // Адреса в параметрах закодированы, как их кодирует браузер.
    let decoded = |key: &str| {
        params.iter().find_map(|p| p.strip_prefix(key)).map(|v| js_sys::decode_uri_component(v).map_or_else(|_| v.to_string(), String::from))
    };
    let query = ui::share::parse_query(&search);
    let demo = decoded("demo=").unwrap_or_else(|| ui::demos::BASE.to_string());
    let Some(canvas) = document.get_element_by_id("globe").and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
    else {
        return;
    };

    // На WebGL2 egui-wgpu просит сторону текстуры 8192, а видеокарта
    // телефона может дать меньше, и тогда устройства нет. Просим то,
    // что видеокарта даёт.
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    if gl_only {
        setup.instance_descriptor.backends = wgpu::Backends::GL;
    }
    setup.device_descriptor = Arc::new(|adapter| {
        let limits = if adapter.get_info().backend == wgpu::Backend::Gl {
            adapter.limits()
        } else {
            wgpu::Limits { max_texture_dimension_2d: 8192, ..wgpu::Limits::default() }
        };
        wgpu::DeviceDescriptor { label: Some("planetx"), required_limits: limits, ..Default::default() }
    });
    let options = eframe::WebOptions {
        wgpu_options: WgpuConfiguration { wgpu_setup: WgpuSetup::CreateNew(setup), ..Default::default() },
        ..Default::default()
    };

    wasm_bindgen_futures::spawn_local(async move {
        let started = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(move |cc| {
                    let ctx = cc.egui_ctx.clone();
                    let repaint: Rc<dyn Fn()> = Rc::new(move || ctx.request_repaint());
                    let loaders = ui::Loaders {
                        esri: Box::new(net::web::Loader::new(net::ESRI, repaint.clone())),
                        osm: Box::new(net::web::Loader::new(net::OSM, repaint.clone())),
                        terrain: Box::new(net::web::Loader::new(net::TERRARIUM, repaint.clone())),
                        places: Box::new(net::web::Loader::new(net::OPENFREEMAP, repaint.clone())),
                        search: Box::new(net::web::Requests::new(repaint.clone())),
                        pictures: Box::new(net::web::Requests::new(repaint.clone())),
                        files: Box::new(net::web::Requests::new(repaint.clone())),
                        make: {
                            let repaint = repaint.clone();
                            Box::new(move |source| Box::new(net::web::Loader::new(source, repaint.clone())))
                        },
                        request: Box::new(move |_| Box::new(net::web::Requests::new(repaint.clone()))),
                    };
                    let report: ui::Report = Box::new(move |text, _ok| show(text));
                    let mut app = ui::LiteApp::new(cc, loaders, scenario, report).with_scenes(&demo);
                    if let Some(pose) = query.pose.clone() {
                        app = app.with_pose(pose);
                    }
                    for url in &query.scenes {
                        app = app.with_scene_url(url);
                    }
                    Ok(Box::new(app))
                }),
            )
            .await;
        // Программа запущена или не запустилась: значок загрузки не нужен.
        if let Some(loading) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id("loading")) {
            loading.remove();
        }
        if let Err(e) = started {
            show(&format!("PlanetX Lite, ошибка запуска: {e:?}"));
        }
    });
}

/// Страница собирается только под WebAssembly.
#[cfg(not(target_arch = "wasm32"))]
fn main() {}
