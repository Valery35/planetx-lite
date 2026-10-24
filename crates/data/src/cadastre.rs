// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Кадастр Росреестра - слой земельных участков НСПД поверх снимка.
//! Перенос `core/cadastre.py` модуля.
//!
//! Служба - WMS Национальной системы пространственных данных
//! (nspd.gov.ru), она заменила публичную кадастровую карту. Проверено
//! модулем 13 октября 2026 года у Перми:
//! - без заголовка Referer с адресом карты НСПД ответ 403;
//! - сертификат сервера выдан «Russian Trusted Sub CA» Минцифры,
//!   корневой «Russian Trusted Root CA» в Windows по умолчанию не входит.
//!   Модуль держит его файлом CA_FILE с отпечатком SHA-1 CA_SHA1
//!   и доверяет ему только в запросах к HOST;
//! - на частые запросы подряд служба отвечает кодом 597, поэтому запросы
//!   идут по одному с паузой GAP, а тайлы мельче MIN_LEVEL не просятся;
//! - слой 36048 - земельные участки. Номера других слоёв не
//!   подтверждены, список слоёв (GetCapabilities) служба не отдала.

use std::f64::consts::PI;

use serde_json::Value;

pub const HOST: &str = "nspd.gov.ru";
/// Земельные участки.
pub const LAYER: u32 = 36048;
/// Картинка тайла WMS слоя LAYER: поле `{bbox}` - рамка в EPSG:3857.
pub const URL: &str = "https://nspd.gov.ru/api/aeggis/v3/36048/wms?SERVICE=WMS&REQUEST=GetMap&VERSION=1.3.0&LAYERS=36048&STYLES=&FORMAT=image/png&TRANSPARENT=true&CRS=EPSG:3857&BBOX={bbox}&WIDTH=256&HEIGHT=256";
pub const REFERER: &str = "https://nspd.gov.ru/map";
pub const ATTRIBUTION: (&str, &str) = ("Rosreestr NSPD", "https://nspd.gov.ru/");
/// Мельче участки не просятся: их тысячи в тайле.
pub const MIN_LEVEL: u32 = 14;
/// Глубже - часть картинки уровня 18.
pub const MAX_LEVEL: u32 = 18;
/// Секунд между запросами к службе.
pub const GAP: f64 = 0.5;
/// Имя файла корневого сертификата у модуля.
pub const CA_FILE: &str = "russian_trusted_root_ca.pem";
/// Отпечаток SHA-1 корневого сертификата, байты через двоеточие.
pub const CA_SHA1: &str = "8F:F9:15:CC:AB:7B:C1:6F:8C:5C:80:99:D5:3E:0E:11:5B:3A:EC:2F";
/// Сведения об участке - GetFeatureInfo той же службы, проверено модулем
/// 13 октября 2026 года. Без FORMAT служба отвечает 500 «invalid
/// request» с кодом 1201. Поля `{layer}`, `{bbox}`, `{size}`, `{middle}`.
pub const INFO_URL: &str = "https://nspd.gov.ru/api/aeggis/v3/{layer}/wms?SERVICE=WMS&REQUEST=GetFeatureInfo&VERSION=1.3.0&LAYERS={layer}&QUERY_LAYERS={layer}&STYLES=&FORMAT=image/png&TRANSPARENT=true&INFO_FORMAT=application/json&FEATURE_COUNT=5&CRS=EPSG:3857&BBOX={bbox}&WIDTH={size}&HEIGHT={size}&I={middle}&J={middle}";
/// Окно запроса сведений - INFO_PIXELS пикселей по INFO_METRES метров
/// местности, точка - середина окна.
pub const INFO_PIXELS: u32 = 256;
pub const INFO_METRES: f64 = 1.0;
/// Страница карты НСПД на точке со слоем участков: поля `{x}`, `{y}`
/// в EPSG:3857 и `{layer}`.
pub const MAP_LINK: &str = "https://nspd.gov.ru/map?zoom=18&coordinate_x={x}&coordinate_y={y}&theme_id=1&is_copy_url=true&active_layers={layer}";
/// Поля сведений об участке по порядку: ключ ответа и единицы. Подписи
/// полей - у окна.
pub const FIELDS: [(&str, &str); 15] = [
    ("cad_num", ""),
    ("readable_address", ""),
    ("specified_area", "м²"),
    ("declared_area", "м²"),
    ("land_record_category_type", ""),
    ("permitted_use_established_by_document", ""),
    ("ownership_type", ""),
    ("right_type", ""),
    ("cost_value", "₽"),
    ("cost_index", "₽/м²"),
    ("cost_determination_date", ""),
    ("cost_application_date", ""),
    ("land_record_reg_date", ""),
    ("status", ""),
    ("quarter_cad_number", ""),
];
/// Радиус сферы Web Mercator, м.
pub const EARTH: f64 = 6378137.0;

/// Участок: кадастровый номер, сведения (ключ поля FIELDS, текст)
/// и внешние кольца контура точками (широта, долгота) без повтора первой.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parcel {
    pub number: String,
    pub values: Vec<(&'static str, String)>,
    pub rings: Vec<Vec<(f64, f64)>>,
}

/// Точка в EPSG:3857, метры.
pub fn mercator(lat: f64, lon: f64) -> (f64, f64) {
    let lat = lat.clamp(-85.0511, 85.0511);
    (lon.to_radians() * EARTH, (PI / 4.0 + lat.to_radians() / 2.0).tan().ln() * EARTH)
}

/// Точка EPSG:3857 в (широта, долгота).
pub fn geographic(x: f64, y: f64) -> (f64, f64) {
    ((2.0 * (y / EARTH).exp().atan() - PI / 2.0).to_degrees(), (x / EARTH).to_degrees())
}

/// Адрес сведений об участках под точкой.
pub fn info_url(lat: f64, lon: f64) -> String {
    let (x, y) = mercator(lat, lon);
    // Метр Web Mercator на широте lat короче метра местности.
    let half = f64::from(INFO_PIXELS) / 2.0 * INFO_METRES / lat.to_radians().cos();
    let bbox = format!("{:.2},{:.2},{:.2},{:.2}", x - half, y - half, x + half, y + half);
    INFO_URL
        .replace("{layer}", &LAYER.to_string())
        .replace("{bbox}", &bbox)
        .replace("{size}", &INFO_PIXELS.to_string())
        .replace("{middle}", &(INFO_PIXELS / 2).to_string())
}

/// Карта НСПД на точке со слоем участков.
pub fn map_link(lat: f64, lon: f64) -> String {
    let (x, y) = mercator(lat, lon);
    MAP_LINK.replace("{x}", &format!("{x:.0}")).replace("{y}", &format!("{y:.0}")).replace("{layer}", &LAYER.to_string())
}

/// Число с двумя знаками, тысячи через пробел, «.00» отбрасывается.
fn number(value: f64) -> String {
    let text = format!("{value:.2}");
    let (sign, rest) = match text.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", text.as_str()),
    };
    let (whole, fraction) = rest.split_once('.').unwrap_or((rest, ""));
    let mut grouped = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            grouped.push(' ');
        }
        grouped.push(c);
    }
    if fraction.is_empty() || fraction == "00" { format!("{sign}{grouped}") } else { format!("{sign}{grouped}.{fraction}") }
}

/// Значение JSON строкой, как `str()` модуля для строк, чисел и флагов.
fn plain(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(n) if n.is_f64() => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e16 => format!("{f:.1}"),
            _ => n.to_string(),
        },
        Value::Null => "None".into(),
        _ => value.to_string(),
    }
}

/// Текст поля с единицами или None - поля нет или оно пустое.
fn text(value: Option<&Value>, unit: &str) -> Option<String> {
    let value = value?;
    let shown = match value {
        Value::Null => return None,
        Value::String(s) if s.is_empty() => return None,
        Value::Number(n) => number(n.as_f64()?),
        _ => plain(value),
    };
    Some(if unit.is_empty() { shown } else { format!("{shown} {unit}") })
}

/// Истина по правилам Python: не null, не false, не ноль, не пусто.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Внешние кольца многоугольника GeoJSON в EPSG:3857 - списки (широта,
/// долгота) без повтора первой точки. Точка не из двух чисел
/// пропускается.
fn rings(geometry: Option<&Value>) -> Vec<Vec<(f64, f64)>> {
    let Some(geometry) = geometry.and_then(Value::as_object) else {
        return Vec::new();
    };
    let coords = geometry.get("coordinates").and_then(Value::as_array);
    let polygons: Vec<&Value> = match (geometry.get("type").and_then(Value::as_str), coords) {
        (Some("Polygon"), Some(_)) => geometry.get("coordinates").into_iter().collect(),
        (Some("MultiPolygon"), Some(list)) => list.iter().collect(),
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    for polygon in polygons {
        let Some(outer) = polygon.as_array().and_then(|p| p.first()).and_then(Value::as_array) else {
            continue;
        };
        let mut ring: Vec<(f64, f64)> = outer
            .iter()
            .filter_map(|p| {
                let p = p.as_array()?;
                Some(geographic(p.first()?.as_f64()?, p.get(1)?.as_f64()?))
            })
            .collect();
        if ring.len() > 1 && ring.first() == ring.last() {
            ring.pop();
        }
        if ring.len() >= 3 {
            out.push(ring);
        }
    }
    out
}

/// Ответ GetFeatureInfo - участки. Ошибка, если ответ не
/// FeatureCollection. Ответ считается враждебным.
pub fn parse_info(data: &[u8]) -> Result<Vec<Parcel>, String> {
    let json: Value = serde_json::from_slice(data).map_err(|e| e.to_string())?;
    if json.get("type").and_then(Value::as_str) != Some("FeatureCollection") {
        return Err("not a FeatureCollection".into());
    }
    let features = json.get("features").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    Ok(features
        .iter()
        .filter_map(Value::as_object)
        .map(|feature| {
            let field = |map: Option<&Value>, key: &str| map.and_then(Value::as_object).and_then(|m| m.get(key)).filter(|v| truthy(v)).cloned();
            let props = feature.get("properties");
            let options = props.and_then(|p| p.get("options"));
            let number = field(options, "cad_num").or_else(|| field(props, "label")).map(|v| plain(&v)).unwrap_or_default();
            let values = FIELDS
                .iter()
                .filter_map(|(key, unit)| text(options.and_then(Value::as_object).and_then(|m| m.get(*key)), unit).map(|t| (*key, t)))
                .collect();
            Parcel { number, values, rings: rings(feature.get("geometry")) }
        })
        .collect())
}
