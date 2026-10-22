// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Наложения картинок, как в Google Earth: на поверхности, на экране
//! и фото с камерой. Перенос `core/overlays.py` модуля.
//!
//! Здесь свойства наложения, углы картинки на поверхности по рамке KML
//! и повороту, место картинки на экране, углы фото в ECEF по камере
//! и полю зрения, поза глобуса для перелёта к фото, рамка 3D-модели
//! на земле.
//!
//! Наложение на поверхности - четыре угла (широта, долгота) в порядке
//! gx:LatLonQuad: левый нижний, правый нижний, правый верхний, левый
//! верхний. LatLonBox с поворотом переводится в те же углы, поворот -
//! против часовой стрелки вокруг середины в местной плоскости.
//!
//! Камера фото - как Camera в KML: широта, долгота, высота над уровнем
//! моря, азимут, наклон (0 - взгляд вниз, 90 - на горизонт), крен. Поле
//! зрения - ViewVolume: левый, правый, нижний, верхний углы в градусах
//! от оси взгляда, near - расстояние до плоскости фото в метрах.

use globe_core::DVec3;
use globe_core::camera::enu;
use globe_core::ellipsoid::{EARTH, Ellipsoid};
use serde_json::{Map, Value, json};

use crate::Rgba;
use crate::measure::destination;

pub const WHITE: Rgba = [255, 255, 255, 255];
/// altitudeMode модели, как в [`crate::models`].
pub const MODEL_MODES: [&str; 3] = ["clampToGround", "relativeToGround", "absolute"];
/// Новая картинка на поверхности - доля ширины видимой полосы.
pub const NEW_SHARE: f64 = 0.5;
/// Плоскость нового фото - доля расстояния до точки взгляда.
pub const NEAR_SHARE: f64 = 0.2;
/// Метров, ближе плоскость нового фото не ставится.
pub const MIN_NEAR: f64 = 10.0;
/// Градусов, круче точка взгляда не берётся по лучу.
pub const MAX_TILT: f64 = 85.0;
/// Наименьший промежуток обновления картинки по ссылке, секунд. Чаще
/// запросы к чужому серверу не уходят.
pub const MIN_REFRESH: f64 = 10.0;
/// Масштаб модели за одно движение ручки не меньше этой доли прежнего:
/// иначе рамка схлопнулась бы в точку и обратно не растянулась.
pub const MIN_SHARE: f64 = 0.01;
/// Ромб поворота - на столько полувысот рамки от середины.
pub const ROTATE_REACH: f64 = 1.25;
/// Знаки углов рамки в порядке углов.
pub const SIGNS: [(f64, f64); 4] = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
/// Середины сторон рамки: юг, восток, север, запад.
pub const SIDES: [(f64, f64); 4] = [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)];

/// Вид наложения.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Ground,
    Screen,
    Photo,
    Model,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Ground, Kind::Screen, Kind::Photo, Kind::Model];

    /// Имя вида в поле файла: ground, screen, photo, model.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Ground => "ground",
            Kind::Screen => "screen",
            Kind::Photo => "photo",
            Kind::Model => "model",
        }
    }

    pub fn parse(text: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == text)
    }
}

/// Место картинки на экране по одной оси KML: x, y и единицы x, y -
/// fraction, pixels или insetPixels.
pub type Xy = (f64, f64, String, String);

fn xy(x: f64, y: f64) -> Xy {
    (x, y, "fraction".into(), "fraction".into())
}

/// Строка пути модели gx:Track: секунды, широта, долгота, высота,
/// азимут (None - по ходу), наклон, крен.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackRow {
    pub t: f64,
    pub lat: f64,
    pub lon: f64,
    pub alt: f64,
    pub heading: Option<f64>,
    pub tilt: f64,
    pub roll: f64,
}

impl TrackRow {
    fn to_json(self) -> Value {
        json!([self.t, self.lat, self.lon, self.alt, self.heading, self.tilt, self.roll])
    }

    fn from_json(value: &Value) -> Option<TrackRow> {
        let items = value.as_array().filter(|a| a.len() == 7)?;
        let n = |i: usize| items.get(i).and_then(Value::as_f64);
        Some(TrackRow {
            t: n(0)?,
            lat: n(1)?,
            lon: n(2)?,
            alt: n(3)?,
            heading: n(4),
            tilt: n(5)?,
            roll: n(6)?,
        })
    }
}

/// Промежуток обновления картинки по ссылке в секундах: 0 - картинка
/// не обновляется, иначе не меньше [`MIN_REFRESH`]. None - не число.
pub fn refresh_interval(value: Option<f64>) -> f64 {
    match value {
        Some(v) if v.is_finite() && v > 0.0 => v.max(MIN_REFRESH),
        _ => 0.0,
    }
}

/// Свойства наложения без картинки.
///
/// corners - углы картинки на поверхности. color - RGBA 0-255, альфа -
/// непрозрачность. order - порядок рисования. У картинки на экране
/// overlay_xy, screen_xy - места [`Xy`], size - так же, -1 - размер
/// картинки, 0 - по пропорции, rotation - градусы. У фото camera и fov,
/// near. latlon_box - рамка LatLonBox (север, юг, восток, запад,
/// поворот) или None - четыре свободных угла gx:LatLonQuad. С рамкой
/// углы считаются по ней. refresh - промежуток обновления картинки
/// по ссылке в секундах, 0 - не обновляется, как refreshMode onInterval
/// в Icon KML. У 3D-модели camera - место (широта, долгота, высота,
/// азимут, наклон, крен), как Location и Orientation KML, scale -
/// масштаб по осям X, Y, Z, mode - altitudeMode. track - путь модели
/// gx:Track или None.
#[derive(Debug, Clone, PartialEq)]
pub struct Overlay {
    pub kind: Kind,
    pub corners: Vec<(f64, f64)>,
    pub color: Rgba,
    pub order: i64,
    pub overlay_xy: Xy,
    pub screen_xy: Xy,
    pub size: Xy,
    pub rotation: f64,
    pub camera: Option<[f64; 6]>,
    pub fov: [f64; 4],
    pub near: f64,
    pub latlon_box: Option<[f64; 5]>,
    pub refresh: f64,
    pub scale: [f64; 3],
    pub mode: String,
    pub track: Option<Vec<TrackRow>>,
}

impl Overlay {
    /// Наложение вида kind со свойствами по умолчанию.
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            corners: Vec::new(),
            color: WHITE,
            order: 0,
            overlay_xy: xy(0.0, 0.0),
            screen_xy: xy(0.0, 0.0),
            size: xy(-1.0, -1.0),
            rotation: 0.0,
            camera: None,
            fov: [-30.0, 30.0, -20.0, 20.0],
            near: 100.0,
            latlon_box: None,
            refresh: 0.0,
            scale: [1.0, 1.0, 1.0],
            mode: "clampToGround".into(),
            track: None,
        }
    }

    /// Новая рамка, углы - по ней.
    pub fn set_box(&mut self, frame: [f64; 5]) {
        self.latlon_box = Some(frame);
        self.corners = box_corners(frame[0], frame[1], frame[2], frame[3], frame[4]);
    }

    /// Четыре свободных угла, рамки больше нет.
    pub fn set_corners(&mut self, corners: Vec<(f64, f64)>) {
        self.latlon_box = None;
        self.corners = corners;
    }

    /// Свойства для поля файла, JSON.
    pub fn params(&self) -> String {
        let mut data = Map::new();
        data.insert("color".into(), json!(self.color));
        data.insert("order".into(), json!(self.order));
        if self.refresh != 0.0 {
            data.insert("refresh".into(), json!(self.refresh));
        }
        let camera = || self.camera.map_or_else(|| json!([]), |c| json!(c));
        match self.kind {
            Kind::Ground => {
                let corners: Vec<[f64; 2]> = self.corners.iter().map(|c| [c.0, c.1]).collect();
                data.insert("corners".into(), json!(corners));
                if let Some(b) = self.latlon_box {
                    data.insert("box".into(), json!(b));
                }
            }
            Kind::Screen => {
                data.insert("overlay_xy".into(), xy_json(&self.overlay_xy));
                data.insert("screen_xy".into(), xy_json(&self.screen_xy));
                data.insert("size".into(), xy_json(&self.size));
                data.insert("rotation".into(), json!(self.rotation));
            }
            Kind::Model => {
                data.insert("camera".into(), camera());
                data.insert("scale".into(), json!(self.scale));
                data.insert("mode".into(), json!(self.mode));
                if let Some(track) = self.track.as_ref().filter(|t| !t.is_empty()) {
                    data.insert("track".into(), Value::from(track.iter().map(|r| r.to_json()).collect::<Vec<_>>()));
                }
            }
            Kind::Photo => {
                data.insert("camera".into(), camera());
                data.insert("fov".into(), json!(self.fov));
                data.insert("near".into(), json!(self.near));
            }
        }
        Value::Object(data).to_string()
    }
}

fn xy_json(v: &Xy) -> Value {
    json!([v.0, v.1, v.2, v.3])
}

fn numbers<const N: usize>(value: Option<&Value>) -> Option<[f64; N]> {
    let items = value?.as_array().filter(|a| a.len() == N)?;
    let mut out = [0.0; N];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = item.as_f64()?;
    }
    Some(out)
}

fn text_of(value: &Value) -> String {
    value.as_str().map_or_else(|| value.to_string(), String::from)
}

/// Наложение из поля файла. corners - углы из геометрии, они главнее
/// углов в тексте. Неверный JSON даёт свойства по умолчанию.
pub fn from_params(kind: Kind, text: &str, corners: Option<Vec<(f64, f64)>>) -> Overlay {
    let parsed: Value = serde_json::from_str(if text.is_empty() { "{}" } else { text }).unwrap_or(Value::Null);
    let empty = Map::new();
    let data = parsed.as_object().unwrap_or(&empty);
    let mut out = Overlay::new(kind);
    let pair = |name: &str, default: &Xy| -> Xy {
        match data.get(name).and_then(Value::as_array).filter(|a| a.len() == 4) {
            Some(v) => match (v[0].as_f64(), v[1].as_f64()) {
                (Some(x), Some(y)) => (x, y, text_of(&v[2]), text_of(&v[3])),
                _ => default.clone(),
            },
            None => default.clone(),
        }
    };
    out.overlay_xy = pair("overlay_xy", &out.overlay_xy);
    out.screen_xy = pair("screen_xy", &out.screen_xy);
    out.size = pair("size", &out.size);
    if let Some(color) = numbers::<4>(data.get("color")) {
        out.color = color.map(|v| (v as i64 & 255) as u8);
    }
    out.order = data.get("order").and_then(Value::as_f64).map_or(0, |v| v as i64);
    out.rotation = data.get("rotation").and_then(Value::as_f64).unwrap_or(0.0);
    out.camera = numbers::<6>(data.get("camera"));
    if let Some(fov) = numbers::<4>(data.get("fov")) {
        out.fov = fov;
    }
    if let Some(near) = data.get("near").and_then(Value::as_f64).filter(|v| *v != 0.0) {
        out.near = near;
    }
    out.refresh = refresh_interval(data.get("refresh").and_then(Value::as_f64));
    if let Some(scale) = numbers::<3>(data.get("scale")) {
        out.scale = scale;
    }
    if let Some(mode) = data.get("mode").and_then(Value::as_str).filter(|m| MODEL_MODES.contains(m)) {
        out.mode = mode.into();
    }
    out.track = data.get("track").and_then(Value::as_array).map(|rows| rows.iter().filter_map(TrackRow::from_json).collect());
    let given = corners.filter(|c| !c.is_empty()).or_else(|| {
        let rows = data.get("corners")?.as_array()?;
        Some(rows.iter().filter_map(|r| numbers::<2>(Some(r)).map(|[a, b]| (a, b))).collect())
    });
    out.corners = given.unwrap_or_default();
    if let Some(frame) = numbers::<5>(data.get("box")) {
        out.set_box(frame);
    }
    out
}

/// Метров в градусе широты и долготы на широте lat, шар.
pub fn metres_per_degree(lat: f64) -> (f64, f64) {
    let north = std::f64::consts::PI * EARTH.a / 180.0;
    (north, north * lat.to_radians().cos().max(1e-6))
}

fn wrap(lon: f64) -> f64 {
    (lon + 180.0).rem_euclid(360.0) - 180.0
}

/// Углы LatLonBox с поворотом rotation градусов против часовой стрелки
/// вокруг середины.
pub fn box_corners(north: f64, south: f64, east: f64, west: f64, rotation: f64) -> Vec<(f64, f64)> {
    let east = if east < west { east + 360.0 } else { east };
    let lat0 = (north + south) / 2.0;
    let lon0 = (east + west) / 2.0;
    let (my, mx) = metres_per_degree(lat0);
    let (sin_a, cos_a) = rotation.to_radians().sin_cos();
    [(south, west), (south, east), (north, east), (north, west)]
        .into_iter()
        .map(|(lat, lon)| {
            let x = (lon - lon0) * mx;
            let y = (lat - lat0) * my;
            let xr = x * cos_a - y * sin_a;
            let yr = x * sin_a + y * cos_a;
            (lat0 + yr / my, wrap(lon0 + xr / mx))
        })
        .collect()
}

/// Рамка картинки как у Google Earth: широта и долгота середины,
/// полуширина и полувысота в метрах местной плоскости, поворот
/// в радианах против часовой стрелки. frame - (север, юг, восток,
/// запад, поворот).
pub fn box_frame(frame: [f64; 5]) -> (f64, f64, f64, f64, f64) {
    let [north, south, east, west, rotation] = frame;
    let east = if east < west { east + 360.0 } else { east };
    let lat0 = (north + south) / 2.0;
    let lon0 = (east + west) / 2.0;
    let (my, mx) = metres_per_degree(lat0);
    (lat0, lon0, (east - west) / 2.0 * mx, (north - south) / 2.0 * my, rotation.to_radians())
}

/// Рамка (север, юг, восток, запад, поворот) по середине, полуразмерам
/// в метрах и повороту в радианах. scale_lat - широта, по которой метры
/// переводятся в градусы, без неё - широта середины. Правка рамки берёт
/// широту прежней середины, иначе края по долготе уплывали бы при
/// растяжении по широте.
pub fn frame_box(lat0: f64, lon0: f64, half_x: f64, half_y: f64, angle: f64, scale_lat: Option<f64>) -> [f64; 5] {
    let (my, mx) = metres_per_degree(scale_lat.unwrap_or(lat0));
    let half_x = half_x.abs().max(1e-3);
    let half_y = half_y.abs().max(1e-3);
    let east = lon0 + half_x / mx;
    let west = lon0 - half_x / mx;
    [(lat0 + half_y / my).min(89.9), (lat0 - half_y / my).max(-89.9), wrap(east), wrap(west), angle.to_degrees()]
}

/// Точка (lat, lon) в местной плоскости у (lat0, lon0), метры: восток,
/// север.
pub fn local(lat0: f64, lon0: f64, lat: f64, lon: f64) -> (f64, f64) {
    let (my, mx) = metres_per_degree(lat0);
    let dlon = (lon - lon0 + 180.0).rem_euclid(360.0) - 180.0;
    (dlon * mx, (lat - lat0) * my)
}

fn turn(x: f64, y: f64, angle: f64) -> (f64, f64) {
    let (s, c) = angle.sin_cos();
    (x * c - y * s, x * s + y * c)
}

/// Ручки рамки: углы в порядке углов, середины сторон (юг, восток,
/// север, запад), середина и ромб поворота над северной стороной.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Handles {
    pub corners: [(f64, f64); 4],
    pub sides: [(f64, f64); 4],
    pub center: (f64, f64),
    pub rotate: (f64, f64),
}

/// Ручки рамки (широта, долгота).
pub fn box_handles(frame: [f64; 5]) -> Handles {
    let (lat0, lon0, hx, hy, angle) = box_frame(frame);
    let (my, mx) = metres_per_degree(lat0);
    let point = |u: f64, v: f64| {
        let (x, y) = turn(u, v, angle);
        (lat0 + y / my, wrap(lon0 + x / mx))
    };
    Handles {
        corners: SIGNS.map(|(sx, sy)| point(sx * hx, sy * hy)),
        sides: SIDES.map(|(sx, sy)| point(sx * hx, sy * hy)),
        center: (lat0, wrap(lon0)),
        rotate: point(0.0, ROTATE_REACH * hy),
    }
}

/// Ручка рамки.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    Corner,
    Side,
    Center,
    Rotate,
}

/// Новая рамка, когда ручку handle номер index тянут в точку (lat,
/// lon). Угол и сторона растягивают рамку от противоположного угла или
/// стороны, centred - от середины, как с Shift у Google Earth.
pub fn box_drag(frame: [f64; 5], handle: Handle, index: usize, lat: f64, lon: f64, centred: bool) -> [f64; 5] {
    let (lat0, lon0, hx, hy, angle) = box_frame(frame);
    if handle == Handle::Center {
        return frame_box(lat, lon, hx, hy, angle, Some(lat0));
    }
    let (x, y) = local(lat0, lon0, lat, lon);
    if handle == Handle::Rotate {
        return frame_box(lat0, lon0, hx, hy, (-x).atan2(y), None);
    }
    let (u, v) = turn(x, y, -angle);
    let (cu, cv, nhx, nhy);
    if handle == Handle::Corner {
        let (sx, sy) = SIGNS[index % 4];
        if centred {
            return frame_box(lat0, lon0, u.abs(), v.abs(), angle, Some(lat0));
        }
        let (ox, oy) = (-sx * hx, -sy * hy);
        (cu, cv) = ((u + ox) / 2.0, (v + oy) / 2.0);
        (nhx, nhy) = ((u - ox).abs() / 2.0, (v - oy).abs() / 2.0);
    } else {
        let (sx, sy) = SIDES[index % 4];
        let (mut a, mut b, mut c, mut d) = (0.0, 0.0, hx, hy);
        if sx != 0.0 {
            if centred {
                c = u.abs();
            } else {
                (a, c) = ((u - sx * hx) / 2.0, (u + sx * hx).abs() / 2.0);
            }
        } else if centred {
            d = v.abs();
        } else {
            (b, d) = ((v - sy * hy) / 2.0, (v + sy * hy).abs() / 2.0);
        }
        (cu, cv, nhx, nhy) = (a, b, c, d);
    }
    let (cx, cy) = turn(cu, cv, angle);
    let (my, mx) = metres_per_degree(lat0);
    frame_box(lat0 + cy / my, lon0 + cx / mx, nhx, nhy, angle, Some(lat0))
}

/// Рамка (север, юг, восток, запад, поворот) следа 3D-модели на земле
/// по азимуту. camera - место [`Overlay::camera`], scale - масштаб
/// по осям, extent - (xmin, xmax, ymin, ymax) модели в её единицах.
/// Наклон и крен рамка не учитывает.
pub fn model_box(camera: &[f64; 6], scale: [f64; 3], extent: [f64; 4]) -> [f64; 5] {
    let (lat, lon, heading) = (camera[0], camera[1], camera[3]);
    let [xmin, xmax, ymin, ymax] = extent;
    let (sx, sy) = (scale[0], scale[1]);
    let angle = -heading.to_radians();
    let (east, north) = turn((xmin + xmax) / 2.0 * sx, (ymin + ymax) / 2.0 * sy, angle);
    let (my, mx) = metres_per_degree(lat);
    frame_box(
        lat + north / my,
        lon + east / mx,
        (xmax - xmin) / 2.0 * sx.abs(),
        (ymax - ymin) / 2.0 * sy.abs(),
        angle,
        Some(lat),
    )
}

/// Место модели, у которой след - рамка frame.
fn model_place(camera: &[f64; 6], scale: [f64; 3], extent: [f64; 4], frame: [f64; 5]) -> [f64; 6] {
    let (lat0, lon0, _, _, angle) = box_frame(frame);
    let [xmin, xmax, ymin, ymax] = extent;
    let (east, north) = turn((xmin + xmax) / 2.0 * scale[0], (ymin + ymax) / 2.0 * scale[1], angle);
    let (my, mx) = metres_per_degree(lat0);
    let lon = wrap(lon0 - east / mx);
    let heading = wrap(-angle.to_degrees());
    [lat0 - north / my, lon, camera[2], heading, camera[4], camera[5]]
}

/// Новое место и масштаб модели, когда ручку её рамки handle номер
/// index тянут в точку (lat, lon). Середина сдвигает, ромб
/// поворачивает, середина стороны растягивает модель по одной оси
/// от противоположной стороны. Угол меняет масштаб всех трёх осей
/// одинаково, противоположный угол стоит на месте, centred - середина
/// стоит на месте.
#[allow(clippy::too_many_arguments)]
pub fn model_drag(
    camera: &[f64; 6],
    scale: [f64; 3],
    extent: [f64; 4],
    handle: Handle,
    index: usize,
    lat: f64,
    lon: f64,
    centred: bool,
) -> ([f64; 6], [f64; 3]) {
    let frame = model_box(camera, scale, extent);
    let (lat0, lon0, hx, hy, angle) = box_frame(frame);
    let (new, ratios) = match handle {
        Handle::Corner => {
            let (x, y) = local(lat0, lon0, lat, lon);
            let (u, v) = turn(x, y, -angle);
            let (sx, sy) = SIGNS[index % 4];
            let (mut share, cu, cv);
            if centred {
                share = (u.abs() / hx + v.abs() / hy) / 2.0;
                (cu, cv) = (0.0, 0.0);
            } else {
                let (ox, oy) = (-sx * hx, -sy * hy);
                share = ((u - ox).abs() / hx + (v - oy).abs() / hy) / 4.0;
                share = share.max(MIN_SHARE);
                (cu, cv) = (ox + sx * share * hx, oy + sy * share * hy);
            }
            share = share.max(MIN_SHARE);
            let (cx, cy) = turn(cu, cv, angle);
            let (my, mx) = metres_per_degree(lat0);
            let new = frame_box(lat0 + cy / my, lon0 + cx / mx, share * hx, share * hy, angle, Some(lat0));
            (new, [share; 3])
        }
        Handle::Side => {
            let new = box_drag(frame, handle, index, lat, lon, centred);
            let (_, _, nhx, nhy, _) = box_frame(new);
            (new, [(nhx / hx).max(MIN_SHARE), (nhy / hy).max(MIN_SHARE), 1.0])
        }
        // Сдвиг и поворот масштаба не меняют, хотя метры в градусе
        // долготы на новой широте другие.
        Handle::Center | Handle::Rotate => (box_drag(frame, handle, index, lat, lon, centred), [1.0; 3]),
    };
    let scale = [scale[0] * ratios[0], scale[1] * ratios[1], scale[2] * ratios[2]];
    (model_place(camera, scale, extent, new), scale)
}

/// Рамка LatLonBox без поворота (север, юг, восток, запад), если углы -
/// такая рамка, иначе None.
pub fn corners_box(corners: &[(f64, f64)]) -> Option<[f64; 4]> {
    let [(s1, w1), (s2, e1), (n1, e2), (n2, w2)] = corners else {
        return None;
    };
    let close = 1e-9;
    let same = (s1 - s2).abs() < close && (n1 - n2).abs() < close && (w1 - w2).abs() < close && (e1 - e2).abs() < close;
    (same && n1 > s1).then_some([*n1, *s1, *e1, *w1])
}

/// Рамка новой картинки на поверхности: середина (lat, lon), ширина
/// width метров, высота - по пропорции aspect = ширина / высота.
pub fn fit_box(lat: f64, lon: f64, width: f64, aspect: f64) -> [f64; 5] {
    frame_box(lat, lon, width / 2.0, width / aspect.max(1e-6) / 2.0, 0.0, None)
}

/// Углы новой картинки на поверхности: середина (lat, lon), ширина
/// width метров, высота - по пропорции aspect = ширина / высота.
pub fn fit_corners(lat: f64, lon: f64, width: f64, aspect: f64) -> Vec<(f64, f64)> {
    let (my, mx) = metres_per_degree(lat);
    let half_x = width / 2.0 / mx;
    let half_y = width / aspect.max(1e-6) / 2.0 / my;
    let north = (lat + half_y).min(89.0);
    let south = (lat - half_y).max(-89.0);
    box_corners(north, south, lon + half_x, lon - half_x, 0.0)
}

/// Геопривязка картинки размером 1 × 1 по углам, если они -
/// параллелограмм: (x0, dx по столбцу, dx по строке, y0, dy по столбцу,
/// dy по строке) в градусах, как геопреобразование GDAL на картинку
/// width × height после деления на размеры. Иначе None.
pub fn affine(corners: &[(f64, f64)]) -> Option<[f64; 6]> {
    let [(y_ll, x_ll), (y_lr, x_lr), (y_ur, x_ur), (y_ul, x_ul)] = corners else {
        return None;
    };
    // Параллелограмм: верхний правый угол - сумма сторон от левого
    // нижнего.
    if (x_ul + x_lr - x_ll - x_ur).abs() > 1e-7 || (y_ul + y_lr - y_ll - y_ur).abs() > 1e-7 {
        return None;
    }
    Some([*x_ul, x_ur - x_ul, x_ll - x_ul, *y_ul, y_ur - y_ul, y_ll - y_ul])
}

fn length(value: f64, units: &str, total: f64, own: f64) -> f64 {
    match units {
        "pixels" => value,
        "insetPixels" => total - value,
        "fraction" => value * total,
        _ => value * own,
    }
}

/// Место картинки на экране: (x, y, ширина, высота) в пикселях от
/// левого верхнего угла вида. Отсчёт KML - от левого нижнего угла.
pub fn screen_rect(overlay: &Overlay, view_w: f64, view_h: f64, image_w: f64, image_h: f64) -> (f64, f64, f64, f64) {
    let (sx, sy, sxu, syu) = &overlay.size;
    let size = |s: f64, units: &str, view: f64, image: f64| {
        if s < 0.0 {
            Some(image)
        } else if s == 0.0 {
            None
        } else {
            Some(length(s, units, view, image))
        }
    };
    let width = size(*sx, sxu, view_w, image_w);
    let height = size(*sy, syu, view_h, image_h);
    let ratio = image_w / image_h.max(1.0);
    let (width, height) = match (width, height) {
        (None, None) => (image_w, image_h),
        (None, Some(h)) => (h * ratio, h),
        (Some(w), None) => (w, w / ratio),
        (Some(w), Some(h)) => (w, h),
    };
    let (ox, oy, oxu, oyu) = &overlay.overlay_xy;
    let (px, py, pxu, pyu) = &overlay.screen_xy;
    let anchor_x = length(*ox, oxu, width, width);
    let anchor_y = length(*oy, oyu, height, height);
    let at_x = length(*px, pxu, view_w, view_w);
    let at_y = length(*py, pyu, view_h, view_h);
    let left = at_x - anchor_x;
    let bottom = at_y - anchor_y;
    (left, view_h - bottom - height, width, height)
}

/// Оси фото в ECEF: глаз, вперёд, вправо, вверх.
fn photo_frame(camera: &[f64; 6]) -> (DVec3, DVec3, DVec3, DVec3) {
    let [lat, lon, alt, heading, tilt, roll] = *camera;
    let [east, north, up] = enu(lat, lon);
    let (sh, ch) = heading.to_radians().sin_cos();
    let (st, ct) = tilt.to_radians().sin_cos();
    let forward = east * (sh * st) + north * (ch * st) - up * ct;
    let right = east * ch - north * sh;
    let top = right.cross(forward);
    let (sr, cr) = roll.to_radians().sin_cos();
    let (right, top) = (right * cr + top * sr, right * -sr + top * cr);
    let eye = Ellipsoid::wgs84().geodetic_to_ecef(lat, lon, alt);
    (eye, forward, right, top)
}

/// Углы плоскости фото в ECEF в порядке левый нижний, правый нижний,
/// правый верхний, левый верхний.
pub fn photo_corners(camera: &[f64; 6], fov: [f64; 4], near: f64) -> [DVec3; 4] {
    let (eye, forward, right, top) = photo_frame(camera);
    let [left, right_fov, bottom, top_fov] = fov.map(f64::to_radians);
    [(left, bottom), (right_fov, bottom), (right_fov, top_fov), (left, top_fov)].map(|(x, y)| {
        let a = near * x.tan();
        let b = near * y.tan();
        eye + forward * near + right * a + top * b
    })
}

/// Плоскость фото для видеокарты: центр в ECEF и вершины смещениями
/// от него. Верх картинки - вверху плоскости.
#[derive(Debug, Clone, PartialEq)]
pub struct PhotoMesh {
    pub center: DVec3,
    pub positions: [[f32; 3]; 4],
    pub uv: [[f32; 2]; 4],
    pub indices: [u32; 6],
}

/// Плоскость фото для видеокарты.
pub fn photo_mesh(camera: &[f64; 6], fov: [f64; 4], near: f64) -> PhotoMesh {
    let corners = photo_corners(camera, fov, near);
    let center = corners.iter().fold(DVec3::ZERO, |a, b| a + *b) / 4.0;
    PhotoMesh {
        center,
        positions: corners.map(|c| (c - center).as_vec3().to_array()),
        uv: [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
        indices: [0, 1, 2, 0, 2, 3],
    }
}

/// Поза глобуса для перелёта к фото: (широта, долгота точки взгляда,
/// расстояние, азимут, наклон), глаз - в точке камеры. Точка взгляда -
/// там, где взгляд встречает землю. Взгляд выше [`MAX_TILT`] упирается
/// в точку на 10 расстояний near.
pub fn photo_pose(camera: &[f64; 6], near: f64) -> [f64; 5] {
    let [lat, lon, alt, heading, tilt, _] = *camera;
    let tilt = tilt.clamp(0.0, MAX_TILT);
    let height = alt.max(1.0);
    let slant = height / tilt.to_radians().cos();
    let distance = if tilt < MAX_TILT { slant } else { (10.0 * near).max(slant) };
    let ground = distance * tilt.to_radians().sin();
    if ground < 0.01 {
        return [lat, lon, distance, heading, tilt];
    }
    let (look_lat, look_lon) = destination(lat, lon, heading, ground);
    [look_lat, look_lon, distance, heading, tilt]
}

/// Камера, поле зрения и near нового фото по нынешнему виду: глаз,
/// азимут и наклон камеры глобуса, вертикальное поле vfov градусов,
/// пропорция картинки aspect = ширина / высота, distance - расстояние
/// до точки взгляда.
#[allow(clippy::too_many_arguments)]
pub fn view_camera(
    eye_lat: f64,
    eye_lon: f64,
    eye_alt: f64,
    heading: f64,
    tilt: f64,
    vfov: f64,
    aspect: f64,
    distance: f64,
) -> ([f64; 6], [f64; 4], f64) {
    let half_v = vfov / 2.0;
    let half_h = (half_v.to_radians().tan() * aspect).atan().to_degrees();
    let near = MIN_NEAR.max(NEAR_SHARE * distance);
    ([eye_lat, eye_lon, eye_alt, heading, tilt, 0.0], [-half_h, half_h, -half_v, half_v], near)
}
