// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Очередь тайлов, кэш и загрузчик на подставном сервере, без выхода
//! в сеть.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use net::native::Loader;
use net::{OSM, Queue, Source, TileLoader};

// Очередь.

#[test]
fn queue_gives_highest_priority_first() {
    let mut q = Queue::new();
    q.set_wanted(&[((3, 1, 1), 5.0), ((3, 2, 2), 50.0), ((4, 1, 1), 5.0), ((3, 2, 2), 1.0)]);
    assert_eq!(q.take_next(), Some((3, 2, 2)));
    // При равном приоритете - в порядке просьбы.
    assert_eq!(q.take_next(), Some((3, 1, 1)));
    assert_eq!(q.take_next(), Some((4, 1, 1)));
    assert_eq!(q.take_next(), None);
    assert_eq!(q.in_flight(), 3);
}

#[test]
fn queue_drops_tiles_no_longer_wanted() {
    let mut q = Queue::new();
    q.set_wanted(&[((3, 1, 1), 5.0), ((3, 2, 2), 4.0)]);
    assert_eq!(q.take_next(), Some((3, 1, 1)));
    q.set_wanted(&[((5, 0, 0), 1.0), ((3, 1, 1), 9.0)]);
    // Начатый тайл второй раз не выдаётся, снятый - тоже.
    assert_eq!(q.take_next(), Some((5, 0, 0)));
    assert_eq!(q.take_next(), None);
    q.done((3, 1, 1), true);
    q.done((5, 0, 0), true);
    q.set_wanted(&[]);
    assert!(q.is_idle());
}

#[test]
fn queue_holds_failed_tile_for_a_while() {
    let mut q = Queue::new();
    q.set_wanted(&[((3, 1, 1), 5.0)]);
    assert_eq!(q.take_next(), Some((3, 1, 1)));
    q.done((3, 1, 1), false);
    q.set_wanted(&[((3, 1, 1), 5.0)]);
    assert_eq!(q.take_next(), None);
    assert_eq!(q.pending(), 0);
}

#[test]
fn failed_tile_waits_longer_each_time() {
    // Сервер высот отвечал по 13-20 с и обрывался, тайл с постоянной
    // паузой 10 с держал загрузчик занятым без конца, 23 октября 2026
    // года. Каждая неудача подряд удваивает паузу, удача её сбрасывает.
    use web_time::{Duration, Instant};
    let key = (3, 1, 1);
    let mut q = Queue::new();
    let t0 = Instant::now();
    q.set_wanted(&[(key, 5.0)]);
    assert_eq!(q.take_next_at(t0), Some(key));
    q.done_at(key, false, t0);
    assert_eq!(q.retry_in(key, t0), Some(Duration::from_secs(10)));
    let t1 = t0 + Duration::from_secs(11);
    q.set_wanted(&[(key, 5.0)]);
    assert_eq!(q.take_next_at(t1), Some(key));
    q.done_at(key, false, t1);
    assert_eq!(q.retry_in(key, t1), Some(Duration::from_secs(20)));
    q.set_wanted(&[(key, 5.0)]);
    assert_eq!(q.take_next_at(t1 + Duration::from_secs(15)), None);
    let t2 = t1 + Duration::from_secs(21);
    assert_eq!(q.take_next_at(t2), Some(key));
    q.done_at(key, true, t2);
    assert_eq!(q.retry_in(key, t2), None);
    assert_eq!(net::retry_delay(1), Duration::from_secs(10));
    assert_eq!(net::retry_delay(3), Duration::from_secs(40));
    assert_eq!(net::retry_delay(30), net::MAX_RETRY);
}

#[test]
fn queue_ignores_nan_priority() {
    let mut q = Queue::new();
    q.set_wanted(&[((3, 1, 1), f64::NAN), ((3, 2, 2), 1.0)]);
    assert_eq!(q.take_next(), Some((3, 2, 2)));
    assert_eq!(q.take_next(), None);
}

// Адреса и кэш.

#[test]
fn osm_rules() {
    assert_eq!(OSM.max_requests, 2);
    assert_eq!(net::tile_url(&OSM, (16, 43004, 19735)), "https://tile.openstreetmap.org/16/43004/19735.png");
    assert_eq!(net::user_agent(), "PlanetX-Lite/0.10.0 (+https://github.com/Valery35/planetx-lite)");
    let path = net::cache_path(&PathBuf::from("c"), &OSM, (16, 43004, 19735));
    assert_eq!(path, PathBuf::from("c").join("osm").join("16").join("43004").join("19735.png"));
}

#[test]
fn invalid_keys_are_not_requested() {
    assert!(net::valid_key(&OSM, (2, 3, 3)));
    assert!(!net::valid_key(&OSM, (2, 4, 0)));
    assert!(!net::valid_key(&OSM, (20, 0, 0)));
    assert!(!net::valid_key(&OSM, (40, 0, 0)));
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

#[test]
fn decode_accepts_tile_and_rejects_garbage() {
    let image = net::decode_tile(&png(256, 256)).unwrap();
    assert_eq!((image.width, image.height, image.rgba.len()), (256, 256, 256 * 256 * 4));
    assert_eq!(&image.rgba[..4], &[10, 20, 30, 255]);
    assert!(net::decode_tile(b"not a png").is_err());
    assert!(net::decode_tile(&[]).is_err());
    assert!(net::decode_tile(&png(512, 256)).is_err());
    // Тайл 512 × 512 - вдвое мельче, тот же цвет.
    let big = net::decode_tile(&png(512, 512)).unwrap_or_else(|_| net::Image { width: 0, height: 0, rgba: Vec::new() });
    assert_eq!((big.width, big.height), (256, 256));
    assert_eq!(&big.rgba[..4], &image.rgba[..4]);
    let mut cut = png(256, 256);
    cut.truncate(cut.len() / 2);
    assert!(net::decode_tile(&cut).is_err());
}

// Загрузчик на подставном сервере.

struct Server {
    url: &'static str,
    peak: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    agents: Arc<Mutex<Vec<String>>>,
}

/// Сервер на свободном порту. Каждый ответ задерживается, чтобы
/// запросы перекрывались, если загрузчик их пускает.
fn server(delay: Duration) -> Server {
    let http = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
    let port = http.server_addr().to_ip().unwrap().port();
    let url: &'static str = Box::leak(format!("http://127.0.0.1:{port}/{{z}}/{{x}}/{{y}}.png").into_boxed_str());
    let now = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let total = Arc::new(AtomicUsize::new(0));
    let agents = Arc::new(Mutex::new(Vec::new()));
    let body = png(256, 256);
    let (p, t, a) = (peak.clone(), total.clone(), agents.clone());
    std::thread::spawn(move || {
        for request in http.incoming_requests() {
            let (now, peak, total, agents, body) = (now.clone(), p.clone(), t.clone(), a.clone(), body.clone());
            std::thread::spawn(move || {
                let n = now.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(n, Ordering::SeqCst);
                total.fetch_add(1, Ordering::SeqCst);
                let agent = request
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("User-Agent"))
                    .map(|h| h.value.to_string())
                    .unwrap_or_default();
                agents.lock().unwrap().push(agent);
                std::thread::sleep(delay);
                now.fetch_sub(1, Ordering::SeqCst);
                let _ = request.respond(tiny_http::Response::from_data(body));
            });
        }
    });
    Server { url, peak, total, agents }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("planetx-lite-net-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn wait_all(loader: &Loader, count: usize) -> Vec<net::Loaded> {
    let started = Instant::now();
    let mut got = Vec::new();
    while got.len() < count {
        got.extend(loader.take());
        assert!(started.elapsed() < Duration::from_secs(30), "пришло {} из {count}", got.len());
        std::thread::sleep(Duration::from_millis(5));
    }
    got
}

fn keys(count: u32) -> Vec<((u32, u32, u32), f64)> {
    (0..count).map(|i| ((5, i, 3), f64::from(count - i))).collect()
}

#[test]
fn loader_keeps_two_requests_and_sends_user_agent() {
    let s = server(Duration::from_millis(80));
    let source = Source { name: "test", url: s.url, max_requests: 2, max_level: 19, ext: "png", placeholders: false, raw: false, tilejson: None };
    let loader = Loader::new(source, None, Arc::new(|| {}));
    loader.want(&keys(12));
    let got = wait_all(&loader, 12);
    assert!(got.iter().all(|l| l.content.is_ok()));
    assert_eq!(s.total.load(Ordering::SeqCst), 12);
    // Перекрытие было, но не больше двух запросов сразу.
    assert_eq!(s.peak.load(Ordering::SeqCst), 2);
    assert_eq!(loader.stats().requests_peak.load(Ordering::SeqCst), 2);
    let agents = s.agents.lock().unwrap();
    assert!(agents.iter().all(|a| *a == net::user_agent()), "{agents:?}");
}

#[test]
fn disk_cache_spares_the_server() {
    let s = server(Duration::from_millis(5));
    let source = Source { name: "test", url: s.url, max_requests: 2, max_level: 19, ext: "png", placeholders: false, raw: false, tilejson: None };
    let dir = temp_dir("cache");
    let first = Loader::new(source, Some(dir.clone()), Arc::new(|| {}));
    first.want(&keys(6));
    wait_all(&first, 6);
    assert_eq!(s.total.load(Ordering::SeqCst), 6);
    assert!(net::cache_path(&dir, &source, (5, 0, 3)).exists());
    let second = Loader::new(source, Some(dir.clone()), Arc::new(|| {}));
    second.want(&keys(6));
    let got = wait_all(&second, 6);
    assert!(got.iter().all(|l| l.content.is_ok()));
    assert_eq!(s.total.load(Ordering::SeqCst), 6);
    assert_eq!(second.stats().disk_hits.load(Ordering::SeqCst), 6);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dead_server_gives_error_not_crash() {
    // Порт, на котором никто не слушает.
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let url: &'static str = Box::leak(format!("http://127.0.0.1:{port}/{{z}}/{{x}}/{{y}}.png").into_boxed_str());
    let source = Source { name: "dead", url, max_requests: 2, max_level: 19, ext: "png", placeholders: false, raw: false, tilejson: None };
    let loader = Loader::new(source, None, Arc::new(|| {}));
    loader.want(&keys(2));
    let got = wait_all(&loader, 2);
    assert!(got.iter().all(|l| l.content.is_err()));
    // Неудачные тайлы отложены, загрузчик не крутит их по кругу.
    assert!(!loader.busy());
}

// Тайла нет у источника: ответ 404 и заглушка Esri.

/// Сервер, который на запрос с путём отвечает кодом и телом answer.
fn answering(answer: fn(&str) -> (u16, Vec<u8>)) -> (&'static str, Arc<AtomicUsize>) {
    let http = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
    let port = http.server_addr().to_ip().unwrap().port();
    let url: &'static str = Box::leak(format!("http://127.0.0.1:{port}/{{z}}/{{x}}/{{y}}").into_boxed_str());
    let total = Arc::new(AtomicUsize::new(0));
    let t = total.clone();
    std::thread::spawn(move || {
        for request in http.incoming_requests() {
            t.fetch_add(1, Ordering::SeqCst);
            let (code, body) = answer(request.url());
            let _ = request.respond(tiny_http::Response::from_data(body).with_status_code(code));
        }
    });
    (url, total)
}

fn gray_png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(256, 256, image::Rgba([204, 204, 204, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

#[test]
fn not_found_is_missing_and_not_retried() {
    let (url, total) = answering(|_| (404, b"no".to_vec()));
    let source = Source { name: "nf", url, max_requests: 2, max_level: 19, ext: "png", placeholders: false, raw: false, tilejson: None };
    let loader = Loader::new(source, None, Arc::new(|| {}));
    loader.want(&keys(3));
    let got = wait_all(&loader, 3);
    assert!(got.iter().all(|l| l.missing && l.content.is_err()));
    // Тайл, которого нет, больше не просится.
    loader.want(&keys(3));
    std::thread::sleep(Duration::from_millis(300));
    assert!(loader.take().is_empty());
    assert_eq!(total.load(Ordering::SeqCst), 3);
    assert!(!loader.busy());
}

#[test]
fn placeholder_is_missing_only_where_expected() {
    let (url, _) = answering(|_| (200, gray_png()));
    let esri_like = Source { name: "ph", url, max_requests: 2, max_level: 19, ext: "png", placeholders: true, raw: false, tilejson: None };
    let loader = Loader::new(esri_like, None, Arc::new(|| {}));
    loader.want(&keys(2));
    assert!(wait_all(&loader, 2).iter().all(|l| l.missing));
    // У источника без заглушек та же серая картинка - обычный тайл.
    let plain = Source { placeholders: false, ..esri_like };
    let loader = Loader::new(plain, None, Arc::new(|| {}));
    loader.want(&keys(2));
    assert!(wait_all(&loader, 2).iter().all(|l| !l.missing && l.content.is_ok()));
}

#[test]
fn esri_address_and_jpeg() {
    assert_eq!(
        net::tile_url(&net::ESRI, (16, 43004, 19735)),
        "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/16/19735/43004"
    );
    assert_eq!(net::cache_path(&PathBuf::from("c"), &net::ESRI, (1, 0, 1)), PathBuf::from("c").join("esri").join("1").join("0").join("1.jpg"));
    let img = image::RgbImage::from_pixel(256, 256, image::Rgb([30, 90, 40]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
    let tile = net::decode_tile(&out.into_inner()).unwrap();
    assert_eq!((tile.width, tile.height), (256, 256));
    assert!(net::decode_tile(b"GIF89a....").is_err());
}
#[test]
fn tilejson_template_is_read_safely() {
    let json = r#"{"tilejson":"3.0.0","tiles":["https:\/\/tiles.openfreemap.org\/planet\/20261015_001001_pt\/{z}\/{x}\/{y}.pbf"],"maxzoom":14}"#;
    assert_eq!(
        net::tilejson_template(json).as_deref(),
        Some("https://tiles.openfreemap.org/planet/20261015_001001_pt/{z}/{x}/{y}.pbf")
    );
    // Враждебный и битый ответ - шаблона нет, сбоя нет.
    for bad in [
        "",
        "{",
        r#"{"tiles":[]}"#,
        r#"{"tiles":["https://x/{z}/{x}"]}"#,
        r#"{"tiles":["ftp://x/{z}/{x}/{y}"]}"#,
        r#"{"tiles":["https://x/{z}/{x}/{y}"#,
        r#"{"tiles":"#,
    ] {
        assert_eq!(net::tilejson_template(bad), None, "{bad}");
    }
}

#[test]
fn tilejson_source_gives_raw_bytes() {
    // Сервер отдаёт TileJSON по /tilejson и байты тайла по остальным
    // путям. Адрес тайлов загрузчик берёт из TileJSON один раз.
    let http = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
    let port = http.server_addr().to_ip().unwrap().port();
    let tilejson: &'static str = Box::leak(format!("http://127.0.0.1:{port}/tilejson").into_boxed_str());
    let asked = Arc::new(AtomicUsize::new(0));
    let a = asked.clone();
    std::thread::spawn(move || {
        for request in http.incoming_requests() {
            let body = if request.url() == "/tilejson" {
                a.fetch_add(1, Ordering::SeqCst);
                format!(r#"{{"tiles":["http://127.0.0.1:{port}/v1/{{z}}/{{x}}/{{y}}.pbf"]}}"#).into_bytes()
            } else {
                request.url().as_bytes().to_vec()
            };
            let _ = request.respond(tiny_http::Response::from_data(body));
        }
    });
    let source = Source { name: "vt", url: "", max_requests: 2, max_level: 14, ext: "pbf", placeholders: false, raw: true, tilejson: Some(tilejson) };
    let loader = Loader::new(source, None, Arc::new(|| {}));
    loader.want(&keys(4));
    let got = wait_all(&loader, 4);
    for l in &got {
        let (z, x, y) = l.key;
        match &l.content {
            Ok(net::Content::Bytes(b)) => assert_eq!(b, format!("/v1/{z}/{x}/{y}.pbf").as_bytes()),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(asked.load(Ordering::SeqCst), 1);
}
#[test]
fn wms_bbox_is_tile_frame_in_metres() {
    // Тайл 1/0/0 - северо-западная четверть мира.
    let url = net::template_url("https://x/?BBOX={bbox}&W=256", (1, 0, 0));
    assert_eq!(url, "https://x/?BBOX=-20037508.34,0.00,0.00,20037508.34&W=256");
    let b = net::tile_bbox((0, 0, 0));
    assert!((b[2] - net::MERCATOR_HALF).abs() < 1e-6 && (b[1] + net::MERCATOR_HALF).abs() < 1e-6);
}

#[test]
fn tms_rows_flip() {
    // {-y} - ряд снизу вверх, как у TMS: на уровне 2 ряд 0 - это ряд 3.
    assert_eq!(net::template_url("https://t/{z}/{x}/{-y}.png", (2, 1, 0)), "https://t/2/1/3.png");
    assert_eq!(net::template_url("https://t/{z}/{x}/{y}.png", (2, 1, 0)), "https://t/2/1/0.png");
    // Ряд за краем уровня не уходит в минус.
    assert_eq!(net::template_url("{-y}", (1, 0, 9)), "0");
}