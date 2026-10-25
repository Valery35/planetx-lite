// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Облака точек лидара по сети: плитки COPC у точки из каталогов IGN
//! LiDAR HD (Франция) и USGS 3DEP (США), перенос `core/lidarnet.py`
//! модуля.
//!
//! IGN - слой WFS `IGNF_LIDAR-HD_METADONNEE:metadata` Géoplateforme,
//! у плитки 1 км поле `url_npl` - файл COPC, без ключа, Licence Ouverte
//! 2.0. USGS 3DEP - коллекция `3dep-lidar-copc` STAC Planetary Computer,
//! файлы в Azure по ключу SAS без учётной записи, общественное достояние
//! США. Файлы читаются частями по HTTP ([`crate::cloud::SparseCloud`]),
//! целиком они не скачиваются. Ответы каталогов считаются враждебными -
//! записи без нужных полей пропускаются.

use serde_json::{Value, json};

/// Источник IGN LiDAR HD.
pub const IGN: &str = "ign";
/// Источник USGS 3DEP.
pub const USGS: &str = "3dep";
/// Служба WFS Géoplateforme.
pub const IGN_WFS: &str = "https://data.geopf.fr/wfs/ows";
/// Слой плиток LiDAR HD.
pub const IGN_LAYER: &str = "IGNF_LIDAR-HD_METADONNEE:metadata";
/// Поиск STAC Planetary Computer, запрос POST.
pub const PC_SEARCH: &str = "https://planetarycomputer.microsoft.com/api/stac/v1/search";
/// Коллекция плиток COPC 3DEP.
pub const PC_COLLECTION: &str = "3dep-lidar-copc";
/// Ключ SAS хранилища файлов 3DEP: учётная запись и контейнер Azure.
pub const PC_SAS: &str = "https://planetarycomputer.microsoft.com/api/sas/v1/token/usgslidareuwest/usgs-3dep-copc";
/// Узел хранилища файлов 3DEP.
pub const PC_HOST: &str = "usgslidareuwest.blob.core.windows.net";
/// Сторона участка поиска у точки, м.
pub const SIDE: f64 = 1500.0;
/// Плиток у точки в проект.
pub const MAX_TILES: usize = 4;
/// Плиток IGN у точки. Сервер IGN 25 октября 2026 года отвечал 429 уже
/// на одном процессе PDAL, плитки IGN 1 км и плотные - у точки их две.
pub const IGN_TILES: usize = 2;
/// Записей каталога на запрос.
pub const SEARCH_LIMIT: usize = 50;

/// Подпись источника и адрес его страницы, у неизвестного - пустые.
pub fn credit(source: &str) -> (&'static str, &'static str) {
    match source {
        IGN => ("LiDAR HD © IGN, Licence Ouverte 2.0", "https://geoservices.ign.fr/lidarhd"),
        USGS => ("USGS 3DEP, public domain", "https://www.usgs.gov/3d-elevation-program"),
        _ => ("", ""),
    }
}

/// Плитка облака.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    /// Источник - [`IGN`] или [`USGS`].
    pub source: &'static str,
    /// Название для списка.
    pub name: String,
    /// Адрес файла COPC без ключа.
    pub url: String,
    /// Рамка - запад, юг, восток, север в градусах.
    pub bbox: [f64; 4],
    /// Количество точек по каталогу, 0 - неизвестно.
    pub points: u64,
    /// Дата или год съёмки строкой.
    pub date: String,
}

/// Квадрат стороны side метров вокруг точки - запад, юг, восток, север
/// в градусах.
pub fn box_around(lat: f64, lon: f64, side: f64) -> [f64; 4] {
    let half = 0.5 * side / 111_320.0;
    let across = half / lat.to_radians().cos().max(1e-6);
    [lon - across, lat - half, lon + across, lat + half]
}

/// Значение для строки запроса, как `urlencode` Python: буквы, цифры
/// и `_.-~` как есть, пробел - `+`, прочее - `%XX`.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Запрос WFS плиток IGN в рамке bbox. В WFS 2.0 у EPSG:4326 порядок
/// осей - широта, долгота.
pub fn ign_url(bbox: [f64; 4]) -> String {
    let [west, south, east, north] = bbox;
    let count = SEARCH_LIMIT.to_string();
    let frame = format!("{south:.6},{west:.6},{north:.6},{east:.6},urn:ogc:def:crs:EPSG::4326");
    let params = [
        ("SERVICE", "WFS"),
        ("VERSION", "2.0.0"),
        ("REQUEST", "GetFeature"),
        ("TYPENAMES", IGN_LAYER),
        ("OUTPUTFORMAT", "application/json"),
        ("COUNT", count.as_str()),
        ("BBOX", frame.as_str()),
    ];
    let query: Vec<String> = params.iter().map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect();
    format!("{IGN_WFS}?{}", query.join("&"))
}

/// Рамка точек геометрии GeoJSON любой вложенности, None - точек нет.
fn ring_box(geometry: &Value) -> Option<[f64; 4]> {
    fn walk(node: &Value, depth: usize, out: &mut [f64; 4], found: &mut bool) {
        // Вложенность GeoJSON не глубже четырёх уровней, глубже - мусор.
        let Some(items) = node.as_array() else { return };
        if depth > 8 {
            return;
        }
        if items.first().is_some_and(Value::is_number) {
            let (Some(x), Some(y)) = (items.first().and_then(Value::as_f64), items.get(1).and_then(Value::as_f64)) else { return };
            if x.is_finite() && y.is_finite() {
                *out = [out[0].min(x), out[1].min(y), out[2].max(x), out[3].max(y)];
                *found = true;
            }
            return;
        }
        for part in items {
            walk(part, depth + 1, out, found);
        }
    }
    let mut out = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut found = false;
    walk(geometry.get("coordinates")?, 0, &mut out, &mut found);
    found.then_some(out)
}

/// Значение строкой, как `str` Python: строка как есть, число и прочее -
/// текстом JSON, null и отсутствие - пусто.
fn text(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// Первые n знаков строки.
fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Количество точек из числа или строки, иначе 0.
fn count(v: Option<&Value>) -> u64 {
    match v {
        Some(Value::Number(n)) => n.as_u64().or_else(|| n.as_f64().filter(|f| f.is_finite() && *f >= 0.0).map(|f| f as u64)).unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

/// Последняя часть адреса после `/`.
fn last_part(url: &str) -> &str {
    url.rsplit('/').next().unwrap_or(url)
}

/// Записи `features` ответа JSON.
fn features(data: &[u8]) -> Result<Vec<Value>, String> {
    let doc: Value = serde_json::from_slice(data).map_err(|e| format!("ответ каталога - не JSON: {e}"))?;
    Ok(match doc.get("features") {
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    })
}

/// Плитки IGN из ответа WFS GeoJSON.
pub fn parse_ign(data: &[u8]) -> Result<Vec<Tile>, String> {
    let mut out = Vec::new();
    for feature in features(data)? {
        let props = feature.get("properties").filter(|p| p.is_object());
        let prop = |k: &str| props.and_then(|p| p.get(k));
        let mut url = prop("url_npl").and_then(Value::as_str).unwrap_or_default().to_string();
        let Some(bbox) = feature.get("geometry").and_then(ring_box) else { continue };
        if !url.ends_with(".copc.laz") {
            continue;
        }
        if let Some(rest) = url.strip_prefix("http://") {
            url = format!("https://{rest}");
        }
        let date = head(&text(prop("date_debut_acquisition")), 10);
        let mut nw = text(prop("coordonnees_nw"));
        if nw.is_empty() {
            nw = last_part(&url).to_string();
        }
        out.push(Tile { source: IGN, name: format!("LiDAR HD {nw}"), url, bbox, points: count(prop("nombre_points")), date });
    }
    Ok(out)
}

/// Тело поиска STAC плиток 3DEP в рамке bbox, текст JSON.
pub fn pc_body(bbox: [f64; 4]) -> String {
    let [west, south, east, north] = bbox;
    let ring = [[west, south], [east, south], [east, north], [west, north], [west, south]];
    json!({
        "collections": [PC_COLLECTION],
        "limit": SEARCH_LIMIT,
        "intersects": {"type": "Polygon", "coordinates": [ring]},
    })
    .to_string()
}

/// Плитки 3DEP из ответа поиска STAC. Рамка записи - шесть чисел,
/// с отметками.
pub fn parse_pc(data: &[u8]) -> Result<Vec<Tile>, String> {
    let mut out = Vec::new();
    for item in features(data)? {
        let href = item.get("assets").and_then(|a| a.get("data")).and_then(|d| d.get("href")).and_then(Value::as_str).unwrap_or_default();
        let numbers: Vec<f64> = match item.get("bbox") {
            Some(Value::Array(b)) => b.iter().map(|v| v.as_f64().unwrap_or(f64::NAN)).collect(),
            _ => Vec::new(),
        };
        if href.is_empty() || !href.starts_with("https://") || numbers.len() < 4 {
            continue;
        }
        let bbox = if numbers.len() == 6 { [numbers[0], numbers[1], numbers[3], numbers[4]] } else { [numbers[0], numbers[1], numbers[2], numbers[3]] };
        if bbox.iter().any(|v| !v.is_finite()) {
            continue;
        }
        let props = item.get("properties").filter(|p| p.is_object());
        let prop = |k: &str| props.and_then(|p| p.get(k)).filter(|v| !v.is_null() && v.as_str() != Some(""));
        let date = head(&text(prop("start_datetime").or_else(|| prop("datetime"))), 4);
        let mut name = text(item.get("id"));
        if name.is_empty() {
            name = last_part(href).to_string();
        }
        out.push(Tile { source: USGS, name, url: href.to_string(), bbox, points: count(prop("pc:count")), date });
    }
    Ok(out)
}

/// Ключ SAS из ответа [`PC_SAS`] (поле JSON `token`), None - его нет.
pub fn parse_token(data: &[u8]) -> Option<String> {
    let doc: Value = serde_json::from_slice(data).ok()?;
    doc.get("token").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string)
}

/// Файлу нужен ключ SAS Planetary Computer.
pub fn needs_key(url: &str) -> bool {
    let rest = url.split_once("://").map_or("", |(_, r)| r);
    rest.split('/').next().unwrap_or_default().eq_ignore_ascii_case(PC_HOST)
}

/// Адрес файла с ключом SAS вместо прежнего.
pub fn with_key(url: &str, key: &str) -> String {
    let base = url.split('?').next().unwrap_or(url);
    if !key.is_empty() && needs_key(base) { format!("{base}?{key}") } else { base.to_string() }
}

/// Плитки у точки: накрывающие точку первыми, затем по расстоянию до
/// середины, не больше limit, повторы одного файла - один раз. У 3DEP
/// один участок бывает снят несколько раз - остаются все съёмки.
pub fn choose(tiles: &[Tile], lat: f64, lon: f64, limit: usize) -> Vec<Tile> {
    let cos = lat.to_radians().cos();
    let rank = |t: &Tile| {
        let [west, south, east, north] = t.bbox;
        let inside = west <= lon && lon <= east && south <= lat && lat <= north;
        let x = (0.5 * (west + east) - lon) * cos;
        let y = 0.5 * (south + north) - lat;
        (!inside, x * x + y * y)
    };
    let mut order: Vec<(bool, f64, &Tile)> = tiles.iter().map(|t| {
        let (outside, d) = rank(t);
        (outside, d, t)
    }).collect();
    order.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut out: Vec<Tile> = Vec::new();
    for (_, _, tile) in order {
        if out.len() >= limit {
            break;
        }
        if out.iter().any(|t| t.url == tile.url) {
            continue;
        }
        let cap = if tile.source == IGN { IGN_TILES } else { limit };
        if out.iter().filter(|t| t.source == tile.source).count() >= cap {
            continue;
        }
        out.push(tile.clone());
    }
    out
}
