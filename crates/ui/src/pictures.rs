// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Картинки-наложения открытой сцены: GroundOverlay на поверхности,
//! PhotoOverlay перед камерой, ScreenOverlay на экране.
//!
//! Картинка берётся из файла KMZ или грузится по ссылке. Ссылки
//! грузятся по одной, каждая один раз за время открытой сцены. Ответ
//! сервера считается враждебным: размер ответа и картинки ограничен,
//! картинка больше [`MAX_SIDE`] уменьшается.
//!
//! Картинка на поверхности - сетка [`GRID`] x [`GRID`] по углам
//! наложения, углы связаны по широте и долготе линейно, как в Google
//! Earth. Сетка садится на рельеф и подтягивается к глазу, как свои
//! объекты. Фото - плоскость перед камерой фото.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use globe_core::ellipsoid::Ellipsoid;
use render::{PULL, PictureMesh};
use scene::features;
use scene::kml::{Folder, Item, Overlay};
use scene::overlays::{self, Kind};
use scene::when::{self, Time};
use terrain::HeightStore;

/// Предел ответа сервера с картинкой, байт.
pub const MAX_BYTES: u64 = 32 << 20;
/// Предел стороны картинки в видеокарте, пикселей.
pub const MAX_SIDE: u32 = 4096;
/// Предел стороны картинки при разборе, пикселей.
pub const MAX_DECODE: u32 = 16384;
/// Клеток сетки картинки на поверхности по стороне.
pub const GRID: usize = 32;

/// Ключ картинки наложения: картинка из файла и картинка по ссылке
/// не путаются.
pub fn key_of(overlay: &Overlay) -> String {
    match overlay.image.as_ref().filter(|i| !i.is_empty()) {
        // Две картинки из файла с одним адресом не путаются, новая
        // картинка с «Обзор…» - новый ключ.
        Some(bytes) => format!("file:{}#{:016x}", overlay.href, fingerprint(bytes)),
        None => overlay.href.clone(),
    }
}

/// Отпечаток байтов картинки: длина и байты начала и конца.
fn fingerprint(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.len().hash(&mut h);
    bytes[..bytes.len().min(4096)].hash(&mut h);
    bytes[bytes.len().saturating_sub(4096)..].hash(&mut h);
    h.finish()
}

/// Байты файла картинки на диске, в программе, или None.
fn local_bytes(path: &str) -> Option<Vec<u8>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let meta = std::fs::metadata(path).ok()?;
        if meta.len() > MAX_BYTES {
            return None;
        }
        std::fs::read(path).ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        None
    }
}

/// Ссылка грузится из сети: только http и https.
pub fn is_remote(href: &str) -> bool {
    let lower = href.trim().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// Видимые по флажкам наложения дерева с их временем: картинки и 3D-модели.
pub fn timed_overlays(folder: &Folder) -> Vec<(Overlay, Option<Time>)> {
    let mut out = Vec::new();
    collect(folder, &mut out);
    out
}

fn collect(folder: &Folder, out: &mut Vec<(Overlay, Option<Time>)>) {
    for item in &folder.children {
        match item {
            Item::Folder(f) if f.visible => collect(f, out),
            Item::Overlay(o) if o.visible && matches!(o.kind(), Kind::Ground | Kind::Photo | Kind::Screen | Kind::Model) => {
                out.push((o.clone(), o.time.clone()));
            }
            _ => {}
        }
    }
}

/// Наложения в окне шкалы времени, без окна - все.
pub fn in_window(timed: &[(Overlay, Option<Time>)], window: Option<(f64, f64)>) -> Vec<Overlay> {
    timed
        .iter()
        .filter(|(_, t)| window.is_none_or(|(lo, hi)| when::visible(t.as_ref(), lo, hi)))
        .map(|(o, _)| o.clone())
        .collect()
}

/// Разобранная картинка: ширина, высота, RGBA в значениях экрана.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Разобрать картинку из байтов с пределами размера. Больше
/// [`MAX_SIDE`] - уменьшается с сохранением пропорции.
pub fn decode(bytes: &[u8]) -> Result<Image, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODE);
    limits.max_image_height = Some(MAX_DECODE);
    limits.max_alloc = Some(512 << 20);
    reader.limits(limits);
    let mut picture = reader.decode().map_err(|e| e.to_string())?.to_rgba8();
    let (w, h) = picture.dimensions();
    if w > MAX_SIDE || h > MAX_SIDE {
        let scale = f64::from(MAX_SIDE) / f64::from(w.max(h));
        let nw = ((f64::from(w) * scale).round() as u32).clamp(1, MAX_SIDE);
        let nh = ((f64::from(h) * scale).round() as u32).clamp(1, MAX_SIDE);
        picture = image::imageops::resize(&picture, nw, nh, image::imageops::FilterType::Triangle);
    }
    let (width, height) = picture.dimensions();
    Ok(Image { width, height, rgba: picture.into_raw() })
}

/// Точка сетки: широта и долгота по долям u (слева направо) и v (снизу
/// вверх) между углами левый нижний, правый нижний, правый верхний,
/// левый верхний. Долготы углов развёрнуты от первого угла, сетка
/// через линию перемены дат не рвётся.
pub fn bilinear(corners: &[(f64, f64)], u: f64, v: f64) -> Option<(f64, f64)> {
    let [a, b, c, d] = corners else {
        return None;
    };
    let unwrap = |lon: f64| a.1 + (lon - a.1 + 180.0).rem_euclid(360.0) - 180.0;
    let (b, c, d) = ((b.0, unwrap(b.1)), (c.0, unwrap(c.1)), (d.0, unwrap(d.1)));
    let lerp = |p: (f64, f64), q: (f64, f64), t: f64| (p.0 + (q.0 - p.0) * t, p.1 + (q.1 - p.1) * t);
    let bottom = lerp(*a, b, u);
    let top = lerp(d, c, u);
    let (lat, lon) = lerp(bottom, top, v);
    Some((lat.clamp(-90.0, 90.0), (lon + 180.0).rem_euclid(360.0) - 180.0))
}

/// Сетка картинки на поверхности по рельефу. None - углов не четыре.
pub fn ground_mesh(overlay: &Overlay, key: &str, heights: &HeightStore) -> Option<PictureMesh> {
    let corners = &overlay.overlay.corners;
    let n = GRID;
    let mut points = Vec::with_capacity((n + 1) * (n + 1));
    let mut uv = Vec::with_capacity(points.capacity());
    for j in 0..=n {
        let v = j as f64 / n as f64;
        for i in 0..=n {
            let u = i as f64 / n as f64;
            points.push(bilinear(corners, u, v)?);
            uv.push([u as f32, (1.0 - v) as f32]);
        }
    }
    let ground = heights.heights_at(&points, true);
    let e = Ellipsoid::wgs84();
    let xyz: Vec<_> =
        points.iter().zip(&ground).map(|((lat, lon), h)| e.geodetic_to_ecef(*lat, *lon, if h.is_finite() { *h } else { 0.0 })).collect();
    let (center, offsets) = features::centered(&xyz);
    let row = (n + 1) as u32;
    let mut indices = Vec::with_capacity(n * n * 6);
    for j in 0..n as u32 {
        for i in 0..n as u32 {
            let a = j * row + i;
            indices.extend_from_slice(&[a, a + 1, a + row + 1, a, a + row + 1, a + row]);
        }
    }
    Some(PictureMesh { key: key.to_string(), center, offsets, uv, indices, tint: overlay.overlay.color, pull: PULL, order: overlay.overlay.order })
}

/// Плоскость фото перед его камерой. None - камеры нет.
pub fn photo_mesh(overlay: &Overlay, key: &str) -> Option<PictureMesh> {
    let o = &overlay.overlay;
    let mesh = overlays::photo_mesh(&o.camera?, o.fov, o.near);
    Some(PictureMesh {
        key: key.to_string(),
        center: mesh.center,
        offsets: mesh.positions.to_vec(),
        uv: mesh.uv.to_vec(),
        indices: mesh.indices.to_vec(),
        tint: o.color,
        pull: 0.0,
        order: o.order,
    })
}

/// Состояние картинки по ключу.
#[derive(Debug, Clone)]
pub enum State {
    /// Ждёт очереди или ответа.
    Waiting,
    Ready(Arc<Image>),
    /// Не загрузилась или не разобралась, текст ошибки.
    Failed(String),
}

/// Картинки наложений открытой сцены и их загрузка.
#[derive(Default)]
pub struct Pictures {
    pub states: HashMap<String, State>,
    queue: VecDeque<String>,
    in_flight: Option<String>,
    /// Ключи, чьи картинки пришли и ещё не ушли в видеокарту.
    pub fresh: Vec<String>,
    /// Количество начатых запросов, для самопроверки.
    pub sent: usize,
    /// Картинки по ссылке с обновлением: ключ - промежуток и время
    /// последнего запроса.
    refresh: HashMap<String, (f64, f64)>,
}

impl Pictures {
    /// Картинки наложений «Моих меток»: новые из файла разбираются сразу,
    /// по ссылке встают в очередь. Картинки, которые остались в метках,
    /// сохраняются, удалённых - забываются. Пришедший позже ответ
    /// удалённой картинки отбрасывается.
    pub fn set(&mut self, overlays: &[&Overlay]) {
        let keys: std::collections::HashSet<String> = overlays
            .iter()
            .filter(|o| matches!(o.kind(), Kind::Ground | Kind::Photo | Kind::Screen))
            .map(|o| key_of(o))
            .collect();
        self.states.retain(|k, _| keys.contains(k));
        self.queue.retain(|k| keys.contains(k));
        self.fresh.retain(|k| keys.contains(k));
        self.refresh.clear();
        for o in overlays.iter().filter(|o| o.overlay.refresh > 0.0 && o.image.as_ref().is_none_or(Vec::is_empty)) {
            self.refresh.insert(key_of(o), (overlays::refresh_interval(Some(o.overlay.refresh)), self.refresh_at(&key_of(o))));
        }
        for o in overlays {
            if !matches!(o.kind(), Kind::Ground | Kind::Photo | Kind::Screen) {
                continue;
            }
            let key = key_of(o);
            if self.states.contains_key(&key) {
                continue;
            }
            let state = match &o.image {
                Some(bytes) if !bytes.is_empty() => match decode(bytes) {
                    Ok(image) => {
                        self.fresh.push(key.clone());
                        State::Ready(Arc::new(image))
                    }
                    Err(e) => State::Failed(e),
                },
                _ if is_remote(&o.href) => {
                    self.queue.push_back(key.clone());
                    State::Waiting
                }
                // Ссылка на файл на диске - в программе.
                _ if let Some(bytes) = crate::netlinks::local_path(&o.href).and_then(|p| local_bytes(&p)) => match decode(&bytes) {
                    Ok(image) => {
                        self.fresh.push(key.clone());
                        State::Ready(Arc::new(image))
                    }
                    Err(e) => State::Failed(e),
                },
                _ => State::Failed(format!("картинки нет: {}", o.href)),
            };
            self.states.insert(key, state);
        }
    }

    /// Шаг загрузки: забрать ответ, начать следующий запрос.
    /// Возвращает, пришла ли картинка.
    pub fn step(&mut self, request: &dyn net::Request) -> bool {
        let mut arrived = false;
        if let Some(answer) = request.take() {
            // Ответ без ожидающего ключа - от прежней сцены.
            if let Some(key) = self.in_flight.take()
                && self.states.contains_key(&key)
            {
                let state = match answer.and_then(|bytes| decode(&bytes)) {
                    Ok(image) => {
                        self.fresh.push(key.clone());
                        arrived = true;
                        State::Ready(Arc::new(image))
                    }
                    Err(e) => State::Failed(e),
                };
                self.states.insert(key, state);
            }
        }
        if self.in_flight.is_none()
            && let Some(key) = self.queue.pop_front()
        {
            request.start(&key);
            self.sent += 1;
            self.in_flight = Some(key);
        }
        arrived
    }

    /// Время последнего запроса картинки с обновлением, 0 - не было.
    fn refresh_at(&self, key: &str) -> f64 {
        self.refresh.get(key).map_or(0.0, |r| r.1)
    }

    /// Картинки по ссылке с обновлением, чей срок вышел, встают в очередь
    /// заново, как refreshMode onInterval KML. Прежняя картинка видна до
    /// прихода новой. now - секунды.
    pub fn due(&mut self, now: f64) {
        for (key, (step, at)) in &mut self.refresh {
            if *at == 0.0 {
                *at = now;
                continue;
            }
            if now - *at >= *step && self.in_flight.as_deref() != Some(key.as_str()) && !self.queue.contains(key) {
                *at = now;
                if is_remote(key) {
                    self.queue.push_back(key.clone());
                } else if let Some(Ok(image)) = crate::netlinks::local_path(key).and_then(|p| local_bytes(&p)).map(|b| decode(&b)) {
                    self.states.insert(key.clone(), State::Ready(Arc::new(image)));
                    self.fresh.push(key.clone());
                }
            }
        }
    }

    /// Загрузка идёт или ждёт.
    pub fn busy(&self) -> bool {
        self.in_flight.is_some() || !self.queue.is_empty()
    }

    pub fn image(&self, key: &str) -> Option<&Arc<Image>> {
        match self.states.get(key)? {
            State::Ready(image) => Some(image),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{Pictures, State, bilinear, decode, ground_mesh, in_window, is_remote, key_of, timed_overlays};
    use scene::kml::{Folder, Item, Overlay};
    use scene::overlays::{self, Kind};
    use terrain::HeightStore;

    fn ground(name: &str, href: &str) -> Overlay {
        let mut o = overlays::Overlay::new(Kind::Ground);
        o.set_box([47.0, 43.0, 62.5, 57.5, 0.0]);
        let mut item = Overlay::new(name, o);
        item.href = href.into();
        item
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn grid_follows_box_and_wraps_dateline() {
        let o = ground("a", "https://x/a.jpg");
        assert_eq!(bilinear(&o.overlay.corners, 0.0, 0.0), Some((43.0, 57.5)));
        let (lat, lon) = bilinear(&o.overlay.corners, 0.5, 0.5).unwrap();
        assert!((lat - 45.0).abs() < 1e-9 && (lon - 60.0).abs() < 1e-9);
        // Рамка через линию перемены дат: середина - у 180, а не у 0.
        let corners = [(0.0, 170.0), (0.0, -170.0), (10.0, -170.0), (10.0, 170.0)];
        let (_, lon) = bilinear(&corners, 0.5, 0.0).unwrap();
        assert!((lon.abs() - 180.0).abs() < 1e-9, "{lon}");
        assert_eq!(bilinear(&corners[..3], 0.5, 0.5), None);
    }

    #[test]
    fn ground_mesh_covers_box_with_picture_top_up() {
        let o = ground("a", "https://x/a.jpg");
        let mesh = ground_mesh(&o, "k", &HeightStore::default()).unwrap();
        let n = super::GRID + 1;
        assert_eq!(mesh.offsets.len(), n * n);
        assert_eq!(mesh.indices.len(), super::GRID * super::GRID * 6);
        // Первая вершина - юго-запад, низ картинки.
        assert_eq!(mesh.uv[0], [0.0, 1.0]);
        assert_eq!(mesh.uv[n * n - 1], [1.0, 0.0]);
    }

    #[test]
    fn visible_overlays_by_flags_radio_and_time() {
        let mut folder = Folder::new("Снимки");
        let mut a = ground("2000", "https://x/2000.jpg");
        a.time = Some(("2000-08-24".into(), "2000-08-24".into()));
        let mut b = ground("2014", "https://x/2014.jpg");
        b.visible = false;
        folder.children.push(Item::Overlay(a));
        folder.children.push(Item::Overlay(b));
        let timed = timed_overlays(&folder);
        assert_eq!(timed.len(), 1);
        assert_eq!(in_window(&timed, None).len(), 1);
        assert!(in_window(&timed, Some((1.0e9, 1.1e9))).is_empty());
    }

    #[test]
    fn decode_limits_and_shrinks() {
        assert!(decode(b"not an image").is_err());
        let big = decode(&png(5000, 100)).unwrap();
        assert_eq!((big.width, big.height), (super::MAX_SIDE, 82));
        assert_eq!(big.rgba.len(), (big.width * big.height * 4) as usize);
    }

    #[test]
    fn keys_and_links() {
        let mut o = ground("a", "files/a.png");
        assert_eq!(key_of(&o), "files/a.png");
        o.image = Some(png(2, 2));
        let first = key_of(&o);
        assert!(first.starts_with("file:files/a.png#"), "{first}");
        // Новая картинка с тем же адресом - новый ключ: «Обзор…» окна
        // свойств сменил картинку, а глобус показывал прежнюю.
        o.image = Some(png(3, 2));
        assert_ne!(key_of(&o), first);
        assert!(is_remote("HTTPS://x/y") && !is_remote("file:///c:/a.png") && !is_remote("a.png"));
    }

    struct Fake {
        started: std::cell::RefCell<Vec<String>>,
        answers: std::cell::RefCell<Vec<Result<Vec<u8>, String>>>,
        stats: net::Stats,
    }

    impl net::Request for Fake {
        fn start(&self, url: &str) {
            self.started.borrow_mut().push(url.to_string());
        }
        fn take(&self) -> Option<Result<Vec<u8>, String>> {
            self.answers.borrow_mut().pop()
        }
        fn stats(&self) -> &net::Stats {
            &self.stats
        }
    }

    #[test]
    fn loads_one_by_one_and_drops_stale_answers() {
        let fake = Fake { started: Default::default(), answers: Default::default(), stats: net::Stats::default() };
        let (a, b) = (ground("a", "https://x/a.png"), ground("b", "https://x/b.png"));
        let mut p = Pictures::default();
        p.set(&[&a, &b]);
        assert!(!p.step(&fake));
        assert_eq!(*fake.started.borrow(), ["https://x/a.png"]);
        // Пока ответа нет, второй запрос не уходит.
        assert!(!p.step(&fake));
        assert_eq!(fake.started.borrow().len(), 1);
        fake.answers.borrow_mut().push(Ok(png(4, 4)));
        assert!(p.step(&fake));
        assert!(p.image("https://x/a.png").is_some());
        assert_eq!(fake.started.borrow().len(), 2);
        fake.answers.borrow_mut().push(Err("HTTP 500".into()));
        assert!(!p.step(&fake));
        assert!(matches!(p.states.get("https://x/b.png"), Some(State::Failed(_))));
        assert!(!p.busy());
        // Ответ на запрос прежней сцены отбрасывается.
        p.set(&[&a]);
        p.step(&fake);
        p.set(&[&b]);
        fake.answers.borrow_mut().push(Ok(png(4, 4)));
        p.step(&fake);
        assert!(p.image("https://x/a.png").is_none());
    }
}
