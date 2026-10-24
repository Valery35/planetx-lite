// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Горные разработки: контуры по снимкам и шахты и карьеры OSM. Перенос
//! `core/mining.py` модуля.
//!
//! Просьба автора модуля от 11 октября 2026 года - других источников,
//! кроме USGS, «делай оба»:
//! - контуры горных разработок мира, Maus и др. 2022, версия 2 -
//!   44 929 многоугольников карьеров, отвалов, хвостохранилищ и площадок
//!   по мозаике Sentinel-2 2019 года, CC BY-SA 4.0. Файл GeoPackage
//!   24.7 МБ скачивается один раз в профиль и читается слоем GeoPackage;
//! - шахты и карьеры OpenStreetMap - man_made=mine и mineshaft,
//!   landuse=quarry, industrial=mine. Запрос Overpass по тайлам уровня
//!   ZOOM вокруг точки взгляда, только при глазе ниже MAX_ALTITUDE и не
//!   больше MAX_TILES тайлов за раз. Тайл просится один раз за сеанс.
//!   Правила Overpass - разумная частота, свой User-Agent, ODbL.
//!
//! Точки OSM складываются в [`crate::deposits::Deposit`]: цвет - та же
//! группа полезного ископаемого (тег resource или product), так шкала
//! месторождений годится и для них.

use std::collections::HashSet;
use std::f64::consts::PI;

use serde_json::{Map, Value};

use crate::deposits::{Builder, Deposit};

pub const POLYGONS_URL: &str = "https://download.pangaea.de/dataset/942325/files/global_mining_polygons_v2.gpkg";
pub const POLYGONS_FILE: &str = "global_mining_polygons_v2.gpkg";
pub const POLYGONS_ATTRIBUTION: (&str, &str) = ("Mining areas: Maus et al. 2022, CC BY-SA 4.0", "https://doi.org/10.1594/PANGAEA.942325");
pub const OSM_QUERY_URL: &str = "https://overpass-api.de/api/interpreter";
/// Серверы Overpass по очереди: при отказе запрос идёт на следующий.
/// Основной сервер в часы нагрузки отвечает 504, 11 октября 2026 года.
/// Запрос - POST с полем data: kumi.systems на GET отвечает 500.
pub const OSM_SERVERS: [&str; 3] = [OSM_QUERY_URL, "https://overpass.kumi.systems/api/interpreter", "https://overpass.private.coffee/api/interpreter"];
pub const OSM_ATTRIBUTION: (&str, &str) = ("Mines: © OpenStreetMap contributors", "https://www.openstreetmap.org/copyright");
/// Страница объекта OSM: поля `{type}` и `{id}`.
pub const OSM_PAGE: &str = "https://www.openstreetmap.org/{type}/{id}";
/// Тайл около 39 км у экватора.
pub const ZOOM: u32 = 10;
pub const MAX_TILES: usize = 9;
/// Выше глаза шахты OSM не просятся, м.
pub const MAX_ALTITUDE: f64 = 150000.0;
/// Секунд между запросами Overpass.
pub const GAP: f64 = 2.0;
/// Предел запроса на сервере Overpass, с.
pub const TIMEOUT: u32 = 60;
/// Виды объектов OSM: тег, значение, ключ подписи.
pub const KINDS: [(&str, &str, &str); 4] = [("man_made", "mine", "mine"), ("man_made", "mineshaft", "shaft"), ("landuse", "quarry", "quarry"), ("industrial", "mine", "mine")];
/// Наибольший уровень тайлов: номер тайла - в u32.
const ZOOM_LIMIT: u32 = 31;

/// Рамка (юг, запад, север, восток) в градусах.
pub type Bounds = (f64, f64, f64, f64);

/// Объект OSM из ответа Overpass: середина, название, ископаемые, ключ
/// вида, оператор и объект OSM «тип/номер».
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub lat: f64,
    pub lon: f64,
    pub name: String,
    pub resource: String,
    pub kind: &'static str,
    pub operator: String,
    pub osm: String,
}

/// Тайл Web Mercator (x, y) уровня zoom под точкой. Номер - целая часть
/// к нулю, как у модуля, долгота замкнута.
pub fn tile_of(lat: f64, lon: f64, zoom: u32) -> (u32, u32) {
    let n = 1i64 << zoom.min(ZOOM_LIMIT);
    let lat = lat.clamp(-85.0511, 85.0511);
    let x = (((lon + 180.0) / 360.0 * n as f64) as i64).rem_euclid(n);
    let y = ((1.0 - lat.to_radians().tan().asinh() / PI) / 2.0 * n as f64) as i64;
    (x as u32, y.clamp(0, n - 1) as u32)
}

/// Рамка тайла (юг, запад, север, восток) в градусах.
pub fn tile_bounds(x: u32, y: u32, zoom: u32) -> Bounds {
    let n = (1u64 << zoom.min(ZOOM_LIMIT)) as f64;
    let lat_of = |row: f64| (PI * (1.0 - 2.0 * row / n)).sinh().atan().to_degrees();
    let (x, y) = (f64::from(x), f64::from(y));
    (lat_of(y + 1.0), x / n * 360.0 - 180.0, lat_of(y), (x + 1.0) / n * 360.0 - 180.0)
}

/// Тайлы вокруг точки взгляда: ближние первыми, не больше limit. Выше
/// MAX_ALTITUDE - ни одного.
pub fn view_tiles(lat: f64, lon: f64, altitude: f64, zoom: u32, limit: usize) -> Vec<(u32, u32)> {
    if altitude > MAX_ALTITUDE {
        return Vec::new();
    }
    let (cx, cy) = tile_of(lat, lon, zoom);
    let (cx, cy) = (i64::from(cx), i64::from(cy));
    let n = 1i64 << zoom.min(ZOOM_LIMIT);
    let reach: i64 = if limit <= 9 { 1 } else { 2 };
    let mut tiles: Vec<(i64, i64)> = Vec::new();
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            if (0..n).contains(&(cy + dy)) {
                tiles.push(((cx + dx).rem_euclid(n), cy + dy));
            }
        }
    }
    // Расстояние - по номеру после замыкания долготы, как у модуля.
    tiles.sort_by_key(|t| (t.0 - cx).pow(2) + (t.1 - cy).pow(2));
    tiles.into_iter().take(limit).map(|(x, y)| (x as u32, y as u32)).collect()
}

/// Общая рамка нескольких тайлов - один запрос Overpass вместо запроса
/// на каждый тайл. Тайлы через линию перемены дат не объединяются -
/// берётся первый. Пустой список - None.
pub fn union(boxes: &[Bounds]) -> Option<Bounds> {
    let first = *boxes.first()?;
    let south = boxes.iter().map(|b| b.0).fold(f64::INFINITY, f64::min);
    let west = boxes.iter().map(|b| b.1).fold(f64::INFINITY, f64::min);
    let north = boxes.iter().map(|b| b.2).fold(f64::NEG_INFINITY, f64::max);
    let east = boxes.iter().map(|b| b.3).fold(f64::NEG_INFINITY, f64::max);
    if east - west > 180.0 {
        return Some(first);
    }
    Some((south, west, north, east))
}

/// Текст запроса Overpass QL для рамки: шахты, рудники и карьеры
/// с серединой и тегами.
pub fn query(bounds: Bounds) -> String {
    let (s, w, n, e) = bounds;
    let area = format!("({s:.5},{w:.5},{n:.5},{e:.5})");
    let parts: String = KINDS.iter().map(|(key, value, _)| format!(r#"nwr["{key}"="{value}"]{area};"#)).collect();
    format!("[out:json][timeout:{TIMEOUT}];({parts});out center tags;")
}

/// Ключ вида объекта OSM или пустая строка.
pub fn kind_of(tags: &Map<String, Value>) -> &'static str {
    KINDS.iter().find(|(key, value, _)| tags.get(*key).and_then(Value::as_str) == Some(*value)).map_or("", |k| k.2)
}

/// Значение для «тип/номер»: строка, число, прочее - «None», как
/// у модуля при отсутствии поля.
fn part(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => "None".to_string(),
    }
}

/// Объекты ответа Overpass (JSON). Объект без середины пропускается.
/// Ответ считается враждебным.
pub fn parse(data: &[u8]) -> Vec<Item> {
    let Ok(json) = serde_json::from_slice::<Value>(data) else {
        return Vec::new();
    };
    let Some(elements) = json.get("elements").and_then(Value::as_array) else {
        return Vec::new();
    };
    let empty = Map::new();
    elements
        .iter()
        .filter_map(|element| {
            let element = element.as_object()?;
            let center = element.get("center").and_then(Value::as_object).filter(|c| !c.is_empty()).unwrap_or(element);
            let (lat, lon) = (center.get("lat")?.as_f64()?, center.get("lon")?.as_f64()?);
            let tags = element.get("tags").and_then(Value::as_object).unwrap_or(&empty);
            let tag = |key: &str| tags.get(key).and_then(Value::as_str).filter(|s| !s.is_empty());
            let resource = tag("resource").or_else(|| tag("product")).unwrap_or("");
            Some(Item {
                lat,
                lon,
                name: tag("name:ru").or_else(|| tag("name")).unwrap_or("").to_string(),
                resource: resource.replace(';', ", "),
                kind: kind_of(tags),
                operator: tag("operator").unwrap_or("").to_string(),
                osm: format!("{}/{}", part(element.get("type")), part(element.get("id"))),
            })
        })
        .collect()
}

/// Объекты OSM в месторождения. Поля записи - как у рудников мира:
/// название, страна (пусто), ископаемые, вид, стадия (пусто), номер
/// (пусто), оператор, мощность (пусто), точность (пусто), объект OSM.
/// Повторы одного объекта из соседних тайлов отбрасываются.
pub fn to_deposits(items: &[Item]) -> Vec<Deposit> {
    let mut seen = HashSet::new();
    let mut out = Builder::default();
    for item in items {
        if !seen.insert(item.osm.as_str()) {
            continue;
        }
        let empty = String::new;
        let fields = [item.name.clone(), empty(), item.resource.clone(), item.kind.to_string(), empty(), empty(), item.operator.clone(), empty(), empty(), item.osm.clone()];
        out.add((Some(item.lat), Some(item.lon)), &item.resource, (false, 0, false), &fields);
    }
    out.out
}

/// Страница объекта OSM «тип/номер» или None.
pub fn osm_page(osm: &str) -> Option<String> {
    let (kind, id) = osm.split_once('/')?;
    Some(OSM_PAGE.replace("{type}", kind).replace("{id}", id))
}
