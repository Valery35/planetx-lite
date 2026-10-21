// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Страница PlanetX Lite. Собирается Trunk под `wasm32-unknown-unknown`.
//!
//! Параметры адреса:
//! - `?check=perm` - самопроверка, отчёт в консоль браузера, поверх
//!   страницы и в заголовок окна;
//! - `?gl` - только WebGL2, как у телефона на странице по http.
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
    let has = |name: &str| search.trim_start_matches('?').split('&').any(|p| p == name);
    let check = has("check=perm");
    let gl_only = has("gl");
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
                    let loader = net::web::Loader::new(net::OSM, Rc::new(move || ctx.request_repaint()));
                    let report: ui::Report = Box::new(move |text, _ok| show(text));
                    Ok(Box::new(ui::LiteApp::new(cc, Box::new(loader), check, report)))
                }),
            )
            .await;
        if let Err(e) = started {
            show(&format!("PlanetX Lite, ошибка запуска: {e:?}"));
        }
    });
}

/// Страница собирается только под WebAssembly.
#[cfg(not(target_arch = "wasm32"))]
fn main() {}
