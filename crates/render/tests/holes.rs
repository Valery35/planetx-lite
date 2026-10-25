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
use std::sync::{Mutex, MutexGuard};

use render::{FrameOptions, Globe, wgpu};

const PERM: (f64, f64) = (58.0105, 56.2294);

/// Видеокарта, очередь и признак точного счёта пикселей. На OpenGL
/// запрос перекрытия отвечает только 0 или 1 - есть пиксели или нет.
/// Так на машине CI, где видеокарта - программная Mesa.
///
/// Тесты берут видеокарту по одному: при чтении кадра параллельно
/// с другими тестами драйвер под Windows зависал.
fn device() -> Option<(wgpu::Device, wgpu::Queue, bool, MutexGuard<'static, ()>)> {
    static GPU: Mutex<()> = Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
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
    Some((device, queue, exact, guard))
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
    let stats = globe.render(camera, selection, None, FrameOptions { count_holes: true, show_holes: false, air: false, ratio: 1.0, sun: None, water: None });
    assert!(stats.draws > 0);
    (stats.holes.unwrap(), stats.underlay.unwrap())
}

#[test]
fn no_holes_by_construction() {
    let Some((device, queue, _, _gpu)) = device() else {
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
    let Some((device, queue, _, _gpu)) = device() else {
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

/// Пиксель в середине кадра глобуса, RGBA.
fn center_pixel(device: &wgpu::Device, queue: &wgpu::Queue, globe: &Globe) -> [u8; 4] {
    let texture = globe.color_texture().unwrap();
    let (width, height) = (texture.width(), texture.height());
    let row = (4 * width).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(height) },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = slice.get_mapped_range().unwrap();
    let at = (row * (height / 2) + 4 * (width / 2)) as usize;
    [data[at], data[at + 1], data[at + 2], data[at + 3]]
}

#[test]
fn picture_keeps_its_brightness() {
    // egui показывает кадр глобуса как есть, без перевода из sRGB. Кадр
    // в формате sRGB выходил на экране втрое темнее снимка, 22 октября
    // 2026 года. Серый 128 снимка - серый 128 кадра.
    assert!(!render::COLOR_FORMAT.is_srgb());
    let Some((device, queue, _, _gpu)) = device() else {
        eprintln!("видеокарты нет, проверка пропущена");
        return;
    };
    let mut globe = Globe::new(&device, &queue);
    let cam = view(PERM.0, PERM.1, 500.0, 0.0, 0.0);
    let sel = lod::select(&cam, &|_| true, Options::default());
    let rgba = vec![128u8; 256 * 256 * 4];
    for key in &sel.draw {
        globe.upload(*key, 256, 256, &rgba);
    }
    globe.render(&cam, &sel, None, FrameOptions { count_holes: false, show_holes: false, air: false, ratio: 1.0, sun: None, water: None });
    let pixel = center_pixel(&device, &queue, &globe);
    eprintln!("пиксель в середине кадра: {pixel:?}");
    for c in &pixel[..3] {
        assert!(c.abs_diff(128) <= 3, "{pixel:?}");
    }
}

#[test]
fn start_levels_are_never_evicted() {
    // Снимков больше бюджета, а тайлов кадра нет: уходят снимки
    // глубоких уровней, уровни 0-2 остаются.
    let Some((device, queue, _, _gpu)) = device() else {
        eprintln!("видеокарты нет, проверка пропущена");
        return;
    };
    let mut globe = Globe::new(&device, &queue);
    let rgba = vec![100u8; 256 * 256 * 4];
    let start: Vec<TileKey> =
        (0..=2u32).flat_map(|z| (0..1u32 << z).flat_map(move |x| (0..1u32 << z).map(move |y| (z, x, y)))).collect();
    for key in &start {
        globe.upload(*key, 256, 256, &rgba);
    }
    for x in 0..30 {
        globe.upload((10, x, 0), 256, 256, &rgba);
    }
    globe.evict(&std::collections::HashSet::new(), 5);
    assert!(start.iter().all(|k| globe.has_texture(*k)));
    assert_eq!(globe.texture_count(), start.len());
}

#[test]
fn feature_line_and_fill_are_drawn() {
    // Камера отвесно в 500 м над Пермью. Линия толщиной 9 пикселей через
    // середину кадра - середина красная. Заливка треугольником вокруг
    // середины без линии - середина зелёная.
    let Some((device, queue, _, _gpu)) = device() else {
        eprintln!("видеокарты нет, проверка пропущена");
        return;
    };
    let mut globe = Globe::new(&device, &queue);
    let cam = view(PERM.0, PERM.1, 500.0, 0.0, 0.0);
    let e = Ellipsoid::wgs84();
    let center = e.geodetic_to_ecef(PERM.0, PERM.1, 0.0);
    let at = |dlat: f64, dlon: f64| {
        let p = e.geodetic_to_ecef(PERM.0 + dlat, PERM.1 + dlon, 0.0) - center;
        [p.x as f32, p.y as f32, p.z as f32]
    };
    let line = render::FeatureMesh {
        center,
        offsets: vec![at(0.0, -0.004), at(0.0, 0.004)],
        lines: vec![0, 1],
        line_color: [255, 0, 0, 255],
        width: 9.0,
        ..Default::default()
    };
    globe.set_features(&[line]);
    let frame = grab(&mut globe, &cam, false);
    let c = rgb(&frame, f64::from(cam.width) / 2.0, f64::from(cam.height) / 2.0);
    assert!(c[0] > 200 && c[1] < 60 && c[2] < 60, "{c:?}");
    let fill = render::FeatureMesh {
        center,
        offsets: vec![at(-0.002, -0.004), at(-0.002, 0.004), at(0.003, 0.0)],
        fill: vec![0, 1, 2],
        fill_color: Some([0, 255, 0, 255]),
        ..Default::default()
    };
    globe.set_features(&[fill]);
    let frame = grab(&mut globe, &cam, false);
    let c = rgb(&frame, f64::from(cam.width) / 2.0, f64::from(cam.height) / 2.0);
    assert!(c[1] > 200 && c[0] < 60, "{c:?}");
    // Индексы за пределами вершин не роняют кадр.
    let broken = render::FeatureMesh { center, offsets: vec![at(0.0, 0.0)], lines: vec![0, 7], fill: vec![0, 1, 9], ..Default::default() };
    globe.set_features(&[broken]);
    grab(&mut globe, &cam, false);
}

#[test]
fn picture_lies_with_west_left_and_waits_for_image() {
    // Картинка 2x1: левая половина красная, правая синяя, на квадрате
    // вокруг середины кадра. Камера отвесно, север вверху: слева
    // в кадре красное, справа синее. Без картинки сетка не рисуется.
    let Some((device, queue, _, _gpu)) = device() else {
        eprintln!("видеокарты нет, проверка пропущена");
        return;
    };
    let mut globe = Globe::new(&device, &queue);
    let cam = view(PERM.0, PERM.1, 3000.0, 0.0, 0.0);
    let e = Ellipsoid::wgs84();
    let center = e.geodetic_to_ecef(PERM.0, PERM.1, 0.0);
    let at = |dlat: f64, dlon: f64| {
        let p = e.geodetic_to_ecef(PERM.0 + dlat, PERM.1 + dlon, 0.0) - center;
        [p.x as f32, p.y as f32, p.z as f32]
    };
    let mesh = render::PictureMesh {
        key: "k".into(),
        center,
        offsets: vec![at(-0.01, -0.01), at(-0.01, 0.01), at(0.01, 0.01), at(0.01, -0.01)],
        uv: vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
        indices: vec![0, 1, 2, 0, 2, 3],
        tint: [255, 255, 255, 255],
        pull: render::PULL,
        order: 0,
    };
    globe.set_pictures(&[mesh]);
    // Точки кадра в серединах половин картинки, u = 0.25 и 0.75.
    let pixel = |dlon: f64| cam.project(e.geodetic_to_ecef(PERM.0, PERM.1 + dlon, 0.0)).0;
    let (west, east) = (pixel(-0.005), pixel(0.005));
    let frame = grab(&mut globe, &cam, false);
    let left = rgb(&frame, west.x, west.y);
    assert!(!(left[0] > 200 && left[2] < 60), "картинки ещё нет: {left:?}");
    assert!(globe.set_picture_image("k", 2, 1, &[255, 0, 0, 255, 0, 0, 255, 255]));
    assert!(!globe.set_picture_image("bad", 2, 2, &[0; 4]));
    let frame = grab(&mut globe, &cam, false);
    let left = rgb(&frame, west.x, west.y);
    let right = rgb(&frame, east.x, east.y);
    assert!(left[0] > 200 && left[2] < 60, "{left:?}");
    assert!(right[2] > 200 && right[0] < 60, "{right:?}");
}

#[test]
fn points_are_drawn_in_their_color() {
    // Облако из красных точек сеткой 100 × 100 м с шагом 2 м вокруг
    // середины кадра, точка 4 пикселя: середина кадра красная.
    let Some((device, queue, _, _gpu)) = device() else {
        eprintln!("видеокарты нет, проверка пропущена");
        return;
    };
    let mut globe = Globe::new(&device, &queue);
    let cam = view(PERM.0, PERM.1, 500.0, 0.0, 0.0);
    let e = Ellipsoid::wgs84();
    let center = e.geodetic_to_ecef(PERM.0, PERM.1, 0.0);
    let mut offsets = Vec::new();
    for i in -25..=25 {
        for j in -25..=25 {
            let p = e.geodetic_to_ecef(PERM.0 + f64::from(i) * 2.0 / 111_320.0, PERM.1 + f64::from(j) * 2.0 / 58_500.0, 1.0) - center;
            offsets.push([p.x as f32, p.y as f32, p.z as f32]);
        }
    }
    let colors = vec![[255, 0, 0]; offsets.len()];
    globe.set_points(&[render::PointsMesh { center, offsets, colors, size: 4.0, ..Default::default() }]);
    let frame = grab(&mut globe, &cam, false);
    let c = rgb(&frame, f64::from(cam.width) / 2.0, f64::from(cam.height) / 2.0);
    assert!(c[0] > 200 && c[1] < 60 && c[2] < 60, "{c:?}");
}

#[test]
fn model_is_drawn_lit_from_the_sun() {
    // Крыша куба 40 м над точкой взгляда, солнце отвесно: середина
    // кадра - красный цвет материала полной яркости.
    let Some((device, queue, _, _gpu)) = device() else {
        eprintln!("видеокарты нет, проверка пропущена");
        return;
    };
    let mut globe = Globe::new(&device, &queue);
    let cam = view(PERM.0, PERM.1, 500.0, 0.0, 0.0);
    let e = Ellipsoid::wgs84();
    let center = e.geodetic_to_ecef(PERM.0, PERM.1, 20.0);
    let up = (e.geodetic_to_ecef(PERM.0, PERM.1, 21.0) - center).normalize();
    let east = globe_core::DVec3::Z.cross(up).normalize();
    let north = up.cross(east);
    let corner = |x: f64, y: f64| {
        let p = east * x + north * y + up * 20.0;
        [p.x as f32, p.y as f32, p.z as f32]
    };
    let roof = vec![corner(-20.0, -20.0), corner(20.0, -20.0), corner(20.0, 20.0), corner(-20.0, -20.0), corner(20.0, 20.0), corner(-20.0, 20.0)];
    let n = [up.x as f32, up.y as f32, up.z as f32];
    let part = render::ModelPart { normals: vec![n; roof.len()], uv: vec![[0.0, 0.0]; roof.len()], positions: roof, image: None, rgba: [1.0, 0.0, 0.0, 1.0], track: None };
    globe.set_models(&[render::ModelMesh { center, parts: vec![part], images: Vec::new(), sun: n, ..Default::default() }]);
    let frame = grab(&mut globe, &cam, false);
    let c = rgb(&frame, f64::from(cam.width) / 2.0, f64::from(cam.height) / 2.0);
    assert!(c[0] > 240 && c[1] < 30 && c[2] < 30, "{c:?}");
}

/// Снятый кадр с атмосферой или без.
fn grab(globe: &mut Globe, cam: &Camera, air: bool) -> render::Frame {
    let sel = lod::select(cam, &|_| true, Options::default());
    globe.grab();
    globe.render(cam, &sel, None, FrameOptions { count_holes: false, show_holes: false, air, ratio: 1.0, sun: None, water: None });
    globe.take_grab().unwrap()
}

fn rgb(frame: &render::Frame, x: f64, y: f64) -> [u8; 3] {
    let at = 4 * (y.round() as usize * frame.width as usize + x.round() as usize);
    [frame.rgba[at], frame.rgba[at + 1], frame.rgba[at + 2]]
}

#[test]
fn halo_at_the_rim_and_black_space() {
    // Пороги модуля (`tools/qgis_phase1.py`): гало в 1 % радиуса диска
    // за краем синее, в 30 % - чёрный космос. Без атмосферы гало нет.
    let Some((device, queue, _, _gpu)) = device() else {
        eprintln!("видеокарты нет, проверка пропущена");
        return;
    };
    let mut globe = Globe::new(&device, &queue);
    let cam = view(PERM.0, PERM.1, 2.0e7, 0.0, 0.0);
    let angle = (Ellipsoid::wgs84().a / cam.eye.length()).asin();
    let radius = angle.tan() / (cam.fov_y.to_radians() / 2.0).tan() * f64::from(cam.height) / 2.0;
    let (cx, cy) = (f64::from(cam.width) / 2.0, f64::from(cam.height) / 2.0);
    let frame = grab(&mut globe, &cam, true);
    // Слева от диска, по горизонтали через середину.
    let halo = rgb(&frame, cx - 1.01 * radius, cy);
    let space = rgb(&frame, cx - 1.3 * radius, cy);
    eprintln!("гало {halo:?}, космос {space:?}");
    assert!(halo[2] >= 100 && halo[2] > halo[0] + 20, "{halo:?}");
    assert!(space.iter().all(|c| *c <= 10), "{space:?}");
    let frame = grab(&mut globe, &cam, false);
    assert_eq!(rgb(&frame, cx - 1.01 * radius, cy), [0, 0, 0]);
}

#[test]
fn blue_sky_over_the_horizon() {
    // Небо у верхнего края кадра с наклоном 80° голубое, порог модуля.
    let Some((device, queue, _, _gpu)) = device() else {
        eprintln!("видеокарты нет, проверка пропущена");
        return;
    };
    let mut globe = Globe::new(&device, &queue);
    let cam = view(43.24778, 42.44001, 2000.0, 0.0, 80.0);
    let frame = grab(&mut globe, &cam, true);
    let sky = rgb(&frame, f64::from(cam.width) / 2.0, 5.0);
    eprintln!("небо {sky:?}");
    assert!(sky[2] >= 150 && sky[2] > sky[0] + 40, "{sky:?}");
}

#[test]
fn missing_tile_is_counted() {
    // Сторож счётчика: без тайла под камерой дыра есть.
    let Some((device, queue, exact, _gpu)) = device() else {
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

#[test]
fn cut_opens_the_surface_and_glass_does_not() {
    // Маска подземного режима: вырез убирает поверхность, под ней видна
    // подстилка, прозрачная поверхность глубину пишет, подстилку
    // закрывает. Подземная сетка рисуется отдельной отрисовкой.
    let Some((device, queue, _, _gpu)) = device() else {
        eprintln!("видеокарты нет, проверка пропущена");
        return;
    };
    let mut globe = Globe::new(&device, &queue);
    let cam = view(PERM.0, PERM.1, 500.0, 0.0, 0.0);
    let sel = lod::select(&cam, &|_| true, Options::default());
    let (_, before) = holes(&mut globe, &cam, &sel);
    // Рамка маски вокруг Перми, середина картинки - вырез, край пуст.
    let (u, v) = terrain::mercator_share(PERM.0, PERM.1);
    let d = 1.0e-4;
    let bounds = [u - d, v - d, u + d, v + d];
    let side = 16usize;
    let paint = |px: [u8; 4]| -> Vec<u8> {
        (0..side * side).flat_map(|i| { let (r, c) = (i / side, i % side); if r == 0 || c == 0 || r == side - 1 || c == side - 1 { [0; 4] } else { px } }).collect()
    };
    let hole = paint([0, 0, 0, 255]);
    globe.set_cut(Some((bounds, side as u32, side as u32, &hole)));
    let (_, cut) = holes(&mut globe, &cam, &sel);
    assert!(cut > before, "{cut} > {before}");
    let glass = paint([255, 0, 0, 0]);
    globe.set_cut(Some((bounds, side as u32, side as u32, &glass)));
    globe.set_opacity(0.3);
    let (_, see) = holes(&mut globe, &cam, &sel);
    assert_eq!(see, before);
    globe.set_cut(None);
    let base = globe.render(&cam, &sel, None, FrameOptions::default()).draws;
    let center = Ellipsoid::wgs84().geodetic_to_ecef(PERM.0, PERM.1, -50.0);
    let v = |x: f32, y: f32| render::SubVertex { offset: [x, y, 0.0], normal: [0.0, 0.0, 1.0], color: [200, 100, 50, 255] };
    let mesh = render::SubMesh { center, vertices: vec![v(0.0, 0.0), v(30.0, 0.0), v(0.0, 30.0)], indices: vec![0, 1, 2], cull_front: false, bias: false, xray: false };
    globe.set_subsurface("wells", Some(&mesh));
    assert_eq!(globe.render(&cam, &sel, None, FrameOptions::default()).draws, base + 1);
    globe.set_subsurface("wells", None);
    assert_eq!(globe.render(&cam, &sel, None, FrameOptions::default()).draws, base);
}
#[test]
fn xray_mesh_shows_through_the_surface() {
    // Красный квадрат 200 × 200 м в 50 м под поверхностью у точки взгляда:
    // обычную сетку поверхность закрывает, сетку xray - нет, как стенку
    // разреза модуля.
    let Some((device, queue, _, _gpu)) = device() else {
        eprintln!("видеокарты нет, проверка пропущена");
        return;
    };
    let mut globe = Globe::new(&device, &queue);
    let cam = view(PERM.0, PERM.1, 500.0, 0.0, 0.0);
    let e = Ellipsoid::wgs84();
    let center = e.geodetic_to_ecef(PERM.0, PERM.1, -50.0);
    let up = (e.geodetic_to_ecef(PERM.0, PERM.1, -49.0) - center).normalize();
    let east = globe_core::DVec3::Z.cross(up).normalize();
    let north = up.cross(east);
    let v = |x: f64, y: f64| {
        let p = east * x + north * y;
        render::SubVertex { offset: [p.x as f32, p.y as f32, p.z as f32], normal: [up.x as f32, up.y as f32, up.z as f32], color: [255, 0, 0, 255] }
    };
    let vertices = vec![v(-100.0, -100.0), v(100.0, -100.0), v(100.0, 100.0), v(-100.0, 100.0)];
    let square = |xray| render::SubMesh { center, vertices: vertices.clone(), indices: vec![0, 1, 2, 0, 2, 3], cull_front: false, bias: false, xray };
    let (cx, cy) = (f64::from(cam.width) / 2.0, f64::from(cam.height) / 2.0);
    globe.set_subsurface("wall", Some(&square(false)));
    let hidden = rgb(&grab(&mut globe, &cam, false), cx, cy);
    globe.set_subsurface("wall", Some(&square(true)));
    let seen = rgb(&grab(&mut globe, &cam, false), cx, cy);
    eprintln!("под поверхностью {hidden:?}, сквозь неё {seen:?}");
    let red = |c: [u8; 3]| i32::from(c[0]) - i32::from(c[1]).max(i32::from(c[2]));
    assert!(red(hidden) < 40, "{hidden:?}");
    assert!(red(seen) > 80, "{seen:?}");
}

#[test]
fn wedge_removes_the_surface() {
    // Разрез Земли: внутри сектора поверхности нет, под ней видна
    // подстилка, вне сектора всё как было.
    let Some((device, queue, exact, _gpu)) = device() else {
        eprintln!("видеокарты нет, проверка пропущена");
        return;
    };
    // На OpenGL счётчик говорит только, что подстилка видна, а она видна
    // и без разреза: сравнивать количество там нельзя.
    if !exact {
        eprintln!("счётчик OpenGL не считает пиксели, проверка пропущена");
        return;
    }
    let mut globe = Globe::new(&device, &queue);
    let cam = view(PERM.0, PERM.1, 2.0e7, 0.0, 0.0);
    let sel = lod::select(&cam, &|k: TileKey| k.0 <= 3, Options::default());
    let (_, before) = holes(&mut globe, &cam, &sel);
    // Сектор 90° по долготе у Перми, от экватора до 80° северной широты.
    let lon = PERM.1.to_radians();
    globe.set_wedge(Some([lon.cos(), lon.sin(), 45f64.to_radians().cos(), 0.0, 80f64.to_radians().sin()]));
    let (_, open) = holes(&mut globe, &cam, &sel);
    assert!(open > before, "{open} > {before}");
    // Сектор по ту сторону Земли не виден.
    let far = (PERM.1 + 180.0).to_radians();
    globe.set_wedge(Some([far.cos(), far.sin(), 45f64.to_radians().cos(), 0.0, 80f64.to_radians().sin()]));
    let (_, back) = holes(&mut globe, &cam, &sel);
    assert_eq!(back, before);
}