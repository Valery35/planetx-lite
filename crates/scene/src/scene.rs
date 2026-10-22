// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сцена - вид глобуса целиком, который сохраняется и передаётся.
//! Перенос `core/scene.py` модуля.
//!
//! Сцена - тело, камера, промежуток шкалы времени, слои проекта
//! на глобусе ссылкой на источник, настройки вида и название папки
//! меток сцены. Сами метки лежат рядом в KML ([`crate::kml`]).
//!
//! Файл сцены - архив ZIP с scene.json и places.kml. Папка с картинками
//! наложений или 3D-моделями ложится вместо places.kml архивом
//! places.kmz с файлами внутри. Номер формата [`FORMAT`] растёт при
//! несовместимых изменениях, более новый формат не читается.

use std::fmt;

use serde_json::{Map, Value, json};

use crate::util::{Archive, write_zip};

pub const FORMAT: i64 = 1;
pub const SCENE_FILE: &str = "scene.json";
pub const PLACES_FILE: &str = "places.kml";
pub const KMZ_FILE: &str = "places.kmz";
pub const EXTENSION: &str = ".planetx";

/// Файл не читается как сцена PlanetX.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneError(pub String);

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SceneError {}

fn error(text: impl fmt::Display) -> SceneError {
    SceneError(text.to_string())
}

/// Число поля key: число, не логическое. Без поля - default, без
/// default - ошибка.
fn number(data: &Map<String, Value>, key: &str, default: Option<f64>) -> Result<f64, SceneError> {
    match (data.get(key).and_then(|v| if v.is_number() { v.as_f64() } else { None }), default) {
        (Some(v), _) => Ok(v),
        (None, Some(d)) => Ok(d),
        (None, None) => Err(error(format!("bad {key}"))),
    }
}

/// Сцена глобуса.
///
/// camera - широта, долгота, расстояние, азимут, наклон. time -
/// промежуток открытой шкалы времени глобуса или None: start, end -
/// время KML в UTC. В сценах до 6 октября 2026 года это словарь
/// временного контроллера QGIS: mode, start, end, frame, step. layers -
/// словари name, provider, source, kind. view - настройки вида:
/// basemap, relief, scale, groups, language, extras. places - название
/// папки меток сцены или пустая строка. body - ключ тела
/// из [`globe_core::ellipsoid`], камера и метки сцены стоят на нём.
/// В сценах до планет тела нет, это Земля.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub camera: [f64; 5],
    pub time: Option<Map<String, Value>>,
    pub layers: Vec<Map<String, Value>>,
    pub view: Map<String, Value>,
    pub places: String,
    pub name: String,
    pub body: String,
}

impl Scene {
    /// Сцена Земли с камерой из трёх-пяти чисел: азимут и наклон
    /// по умолчанию нули.
    pub fn new(camera: &[f64]) -> Self {
        let mut cam = [0.0; 5];
        for (slot, v) in cam.iter_mut().zip(camera) {
            *slot = *v;
        }
        Self {
            camera: cam,
            time: None,
            layers: Vec::new(),
            view: Map::new(),
            places: String::new(),
            name: String::new(),
            body: "earth".into(),
        }
    }

    /// Словарь для scene.json.
    pub fn to_dict(&self) -> Value {
        let [lat, lon, distance, heading, tilt] = self.camera;
        json!({
            "format": FORMAT,
            "name": self.name,
            "camera": {"lat": lat, "lon": lon, "distance": distance, "heading": heading, "tilt": tilt, "body": self.body},
            "time": self.time,
            "layers": self.layers,
            "view": self.view,
            "places": self.places,
        })
    }

    /// Сцена из словаря scene.json.
    pub fn from_dict(data: &Value) -> Result<Scene, SceneError> {
        let data = data.as_object().ok_or_else(|| error("not a scene"))?;
        let version = data.get("format").and_then(Value::as_i64).filter(|v| *v >= 1).ok_or_else(|| error("no format"))?;
        if version > FORMAT {
            return Err(error(format!("format {version} is newer than {FORMAT}")));
        }
        let cam = data.get("camera").and_then(Value::as_object).ok_or_else(|| error("no camera"))?;
        let camera = [
            number(cam, "lat", None)?,
            number(cam, "lon", None)?,
            number(cam, "distance", None)?,
            number(cam, "heading", Some(0.0))?,
            number(cam, "tilt", Some(0.0))?,
        ];
        if !((-90.0..=90.0).contains(&camera[0]) && (-180.0..=180.0).contains(&camera[1]) && camera[2] > 0.0) {
            return Err(error("camera out of range"));
        }
        let layers = data
            .get("layers")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_object)
                    .filter(|l| l.get("source").is_some_and(crate::tour::truthy))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let text = |key: &str| data.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        Ok(Scene {
            camera,
            time: data.get("time").and_then(Value::as_object).cloned(),
            layers,
            view: data.get("view").and_then(Value::as_object).cloned().unwrap_or_default(),
            places: text("places"),
            name: text("name"),
            body: cam.get("body").and_then(Value::as_str).unwrap_or("earth").to_string(),
        })
    }
}

/// Метки сцены для записи: текст KML или байты KMZ.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Places<'a> {
    None,
    Kml(&'a str),
    Kmz(&'a [u8]),
}

/// Байты файла сцены: scene.json и, если есть, метки. Текст -
/// places.kml, байты KMZ ([`crate::kml::write_kmz`]) - places.kmz.
pub fn write_scene(scene: &Scene, places: Places<'_>) -> Result<Vec<u8>, SceneError> {
    let json = serde_json::to_vec_pretty(&scene.to_dict()).map_err(error)?;
    let mut entries: Vec<(&str, &[u8])> = vec![(SCENE_FILE, json.as_slice())];
    match places {
        Places::Kmz(data) if !data.is_empty() => entries.push((KMZ_FILE, data)),
        Places::Kml(text) if !text.is_empty() => entries.push((PLACES_FILE, text.as_bytes())),
        _ => {}
    }
    write_zip(entries).map_err(error)
}

/// Сцена и байты меток (или пустые) из байтов файла сцены: KML или KMZ,
/// их различает [`crate::kml::read_file`].
pub fn read_scene(data: &[u8]) -> Result<(Scene, Vec<u8>), SceneError> {
    let mut archive = Archive::open(data).map_err(error)?;
    let names = archive.names().to_vec();
    if !names.iter().any(|n| n == SCENE_FILE) {
        return Err(error("no scene.json"));
    }
    let raw = archive.read(SCENE_FILE).map_err(error)?;
    let text = std::str::from_utf8(&raw).map_err(error)?;
    let value: Value = serde_json::from_str(text).map_err(error)?;
    let places = if names.iter().any(|n| n == KMZ_FILE) {
        archive.read(KMZ_FILE).map_err(error)?
    } else if names.iter().any(|n| n == PLACES_FILE) {
        archive.read(PLACES_FILE).map_err(error)?
    } else {
        Vec::new()
    };
    Ok((Scene::from_dict(&value)?, places))
}
