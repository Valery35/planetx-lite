// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Дыр в подложке нет по построению: кадр рисуется на видеокарте без
//! окна, счётчик дыр - запрос перекрытия. Снимков тайлов нет, тайлы
//! рисуются ровным цветом, на дыры это не влияет.
//!
//! Без видеокарты тест сообщает об этом и проходит: проверять нечем.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use globe_core::camera::Camera;
use globe_core::ellipsoid::Ellipsoid;
use globe_core::lod::{self, Options, TileKey};
use globe_core::tiling as tl;
use render::{FrameOptions, Globe, wgpu};

const PERM: (f64, f64) = (58.0105, 56.2294);

/// Видеокарта, очередь и признак точного счёта пикселей. На OpenGL
/// запрос перекрытия отвечает только 0 или 1 - есть пиксели или нет.
/// Так на машине CI, где видеокарта - программная Mesa.
fn device() -> Option<(wgpu::Device, wgpu::Queue, bool)> {
    // WGPU_BACKEND выбирает бэкенд, например gl - как у машины CI.
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    if let Some(backends) = wgpu::Backends::from_env() {
        desc.backends = backends;
    }
    let instance = wgpu::Instance::new(desc);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
        apply_limit_buckets: false,
    }))
    .ok()?;
    let exact = adapter.get_info().backend != wgpu::Backend::Gl;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
    Some((device, queue, exact))
}

fn view(lat: f64, lon: f64, distance: f64, heading: f64, tilt: f64) -> Camera {
    Camera::look_at(Ellipsoid::wgs84(), lat, lon, distance, heading, tilt, 0.0).with_viewport(960, 540)
}

type CameraCase = (&'static str, (f64, f64), f64, f64, f64);

const CAMERAS: [CameraCase; 8] = [
    ("космос над Пермью", PERM, 2.0e7, 0.0, 0.0),
    ("500 м над Пермью", PERM, 500.0, 0.0, 0.0),
    ("2 км над Пермью, наклон 70°", PERM, 2000.0, 30.0, 70.0),
    ("50 км, наклон 85°", PERM, 50000.0, 200.0, 85.0),
    ("линия смены дат", (10.0, 179.9), 3000.0, 0.0, 40.0),
    ("у края Mercator", (84.0, 20.0), 20000.0, 0.0, 30.0),
    ("северный полюс из космоса", (89.0, 0.0), 1.5e7, 0.0, 0.0),
    ("экватор с высоты 1000 км", (0.0, 0.0), 1.0e6, 0.0, 0.0),
];

fn half_ready(key: TileKey) -> bool {
    key.0 <= 2 || (key.1 ^ key.2).is_multiple_of(2)
}

/// Дыры и видимая подстилка.
fn holes(globe: &mut Globe, camera: &Camera, selection: &lod::Selection) -> (u64, u64) {
    let stats = globe.render(camera, selection, None, FrameOptions { count_holes: true, show_holes: false });
    assert!(stats.draws > 0);
    (stats.holes.unwrap(), stats.underlay.unwrap())
}

#[test]
fn no_holes_by_construction() {
    let Some((device, queue, _)) = device() else {
        eprintln!("видеокарты нет, проверка дыр пропущена");
        return;
    };
    let mut globe = Globe::new(&device, &queue);
    for ready in [&(|_: TileKey| true) as &dyn Fn(TileKey) -> bool, &half_ready, &|k: TileKey| k.0 <= 2] {
        for (name, (lat, lon), distance, heading, tilt) in CAMERAS {
            let cam = view(lat, lon, distance, heading, tilt);
            let sel = lod::select(&cam, ready, Options::default());
            let (count, underlay) = holes(&mut globe, &cam, &sel);
            eprintln!("{name}: дыр {count}, подстилки {underlay}");
            assert_eq!(count, 0, "{name}");
        }
    }
}

#[test]
fn missing_tile_takes_ancestor_picture() {
    // Тайла нет у источника: он готов к рисованию, если есть снимок
    // предка не глубже 6 уровней, и рисуется без дыр.
    let Some((device, queue, _)) = device() else {
        eprintln!("видеокарты нет, проверка пропущена");
        return;
    };
    let mut globe = Globe::new(&device, &queue);
    let rgba = vec![200u8; 256 * 256 * 4];
    globe.upload((10, 600, 300), 256, 256, &rgba);
    assert!(!globe.has_texture((12, 2401, 1202)));
    globe.mark_missing((12, 2401, 1202));
    assert!(globe.has_texture((12, 2401, 1202)));
    // Глубже 6 уровней предок не берётся.
    globe.mark_missing((17, 600 << 7, 300 << 7));
    assert!(!globe.has_texture((17, 600 << 7, 300 << 7)));
    let cam = view(PERM.0, PERM.1, 2.0e7, 0.0, 0.0);
    let sel = lod::select(&cam, &|_| true, Options::default());
    assert_eq!(holes(&mut globe, &cam, &sel).0, 0);
    globe.clear_textures();
    assert!(!globe.has_texture((12, 2401, 1202)));
}

#[test]
fn missing_tile_is_counted() {
    // Сторож счётчика: без тайла под камерой дыра есть.
    let Some((device, queue, exact)) = device() else {
        eprintln!("видеокарты нет, проверка дыр пропущена");
        return;
    };
    let mut globe = Globe::new(&device, &queue);
    let cam = view(PERM.0, PERM.1, 500.0, 0.0, 0.0);
    let mut sel = lod::select(&cam, &|_| true, Options::default());
    let deepest = sel.draw.iter().map(|k| k.0).max().unwrap();
    let (x, y) = tl::lonlat_to_tile(PERM.0, PERM.1, deepest);
    let pos = sel.draw.iter().position(|k| *k == (deepest, x, y)).unwrap();
    sel.draw.remove(pos);
    sel.holes.remove(pos);
    let (count, underlay) = holes(&mut globe, &cam, &sel);
    eprintln!("без тайла: дыр {count}, подстилки {underlay}");
    assert_eq!(count, 0);
    // Тайл закрывает тысячи пикселей. На OpenGL счётчик говорит только,
    // что подстилка видна.
    let floor = if exact { 1000 } else { 0 };
    assert!(underlay > floor, "{underlay}");
}
