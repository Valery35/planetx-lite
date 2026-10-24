// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Кадастр Росреестра: адрес и разбор сведений об участке. Перенос
//! `tests/test_cadastre.py` модуля.

use data::cadastre;

/// Ответ службы 13 октября 2026 года у Перми, контур сокращён до пяти
/// точек.
const ANSWER: &str = r#"{"type": "FeatureCollection", "features": [{
 "id": 49685861, "type": "Feature",
 "geometry": {"type": "Polygon", "coordinates": [[
   [6261773.53, 7969657.97], [6261726.05, 7969634.44],
   [6261733.47, 7969477.28], [6261867.92, 7969545.83],
   [6261773.53, 7969657.97]]],
   "crs": {"type": "name", "properties": {"name": "EPSG:3857"}}},
 "properties": {
   "category": 36368, "categoryName": "Земельные участки ЕГРН",
   "label": "59:01:4410120:9",
   "options": {
     "area": null, "cad_num": "59:01:4410120:9",
     "cost_application_date": "2023-01-01",
     "cost_determination_date": "2022-01-01",
     "cost_index": 11880.86, "cost_value": 51337211.59,
     "declared_area": null,
     "land_record_category_type": "Земли населенных пунктов",
     "land_record_reg_date": "2003-05-15",
     "ownership_type": "Частная",
     "permitted_use_established_by_document": "Под жилой дом со встроенными помещениями",
     "quarter_cad_number": "59:01:4410120",
     "readable_address": "Пермский край, г. Пермь, Ленинский р-н, ул. Луначарского, 34",
     "right_type": "Общая долевая собственность",
     "specified_area": 4321, "status": "Ранее учтенный"}}}]}"#;

/// Значение параметра name адреса url.
fn param<'a>(url: &'a str, name: &str) -> Option<&'a str> {
    url.split_once('?')?.1.split('&').find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
}

#[test]
fn url_has_format_and_point_in_middle() {
    // Без FORMAT служба отвечает 500 «invalid request».
    let url = cadastre::info_url(58.0105, 56.2502);
    assert_eq!(param(&url, "FORMAT"), Some("image/png"));
    assert_eq!(param(&url, "REQUEST"), Some("GetFeatureInfo"));
    assert_eq!(param(&url, "QUERY_LAYERS"), Some(cadastre::LAYER.to_string().as_str()));
    let bbox: Vec<f64> = param(&url, "BBOX").unwrap_or("").split(',').filter_map(|v| v.parse().ok()).collect();
    assert_eq!(bbox.len(), 4);
    let [x0, y0, x1, y1] = bbox[..] else { return };
    let (x, y) = cadastre::mercator(58.0105, 56.2502);
    assert!(((x0 + x1) / 2.0 - x).abs() < 0.05);
    assert!(((y0 + y1) / 2.0 - y).abs() < 0.05);
    // Окно - 256 м местности, в метрах Web Mercator шире на 1/cos.
    assert!((x1 - x0 - 256.0 / 0.5299).abs() <= 1.0);
    assert_eq!(param(&url, "I"), Some("128"));
}

#[test]
fn tile_url_has_layer() {
    assert!(cadastre::URL.contains("LAYERS=36048&"));
    assert!(cadastre::URL.contains("BBOX={bbox}"));
}

#[test]
fn parcel_values_and_outline() {
    let parcels = cadastre::parse_info(ANSWER.as_bytes()).unwrap_or_default();
    assert_eq!(parcels.len(), 1);
    let Some(parcel) = parcels.first() else { return };
    assert_eq!(parcel.number, "59:01:4410120:9");
    let value = |key: &str| parcel.values.iter().find(|v| v.0 == key).map(|v| v.1.as_str());
    assert_eq!(value("specified_area"), Some("4 321 м²"));
    assert_eq!(value("cost_value"), Some("51 337 211.59 ₽"));
    assert_eq!(value("cost_index"), Some("11 880.86 ₽/м²"));
    assert_eq!(value("declared_area"), None);
    let keys: Vec<&str> = parcel.values.iter().map(|v| v.0).collect();
    assert_eq!(keys[..2], ["cad_num", "readable_address"]);
    let ring = parcel.rings.first().cloned().unwrap_or_default();
    assert_eq!(ring.len(), 4);
    // Контур - в сотне метров от точки запроса 58.0105, 56.2502.
    let (lat, lon) = ring[0];
    assert!((lat - 58.0111).abs() < 5e-4);
    assert!((lon - 56.2502).abs() < 5e-4);
}

#[test]
fn empty_and_bad_answers() {
    assert_eq!(cadastre::parse_info(br#"{"type": "FeatureCollection", "features": []}"#), Ok(Vec::new()));
    assert!(cadastre::parse_info(b"<ServiceExceptionReport/>").is_err());
    assert!(cadastre::parse_info(br#"{"type": "Feature"}"#).is_err());
    // Испорченный контур и число в номере не роняют разбор.
    let odd = br#"{"type": "FeatureCollection", "features": [5, {"properties": {"label": 7, "options": {"status": true, "cost_value": -1234.5}},
      "geometry": {"type": "MultiPolygon", "coordinates": [[], [[[0, 0], ["x", 1], [1, 0], [1, 1]]]]}}]}"#;
    let parcels = cadastre::parse_info(odd).unwrap_or_default();
    assert_eq!(parcels.len(), 1);
    let Some(parcel) = parcels.first() else { return };
    assert_eq!(parcel.number, "7");
    assert_eq!(parcel.values, vec![("cost_value", "-1 234.50 ₽".to_string()), ("status", "True".to_string())]);
    assert_eq!(parcel.rings.len(), 1);
}

#[test]
fn mercator_round_trip() {
    let (x, y) = cadastre::mercator(58.0105, 56.2502);
    let (lat, lon) = cadastre::geographic(x, y);
    assert!((lat - 58.0105).abs() < 1e-9);
    assert!((lon - 56.2502).abs() < 1e-9);
}

#[test]
fn map_link_points_at_parcel() {
    let link = cadastre::map_link(58.0105, 56.2502);
    assert!(link.starts_with("https://nspd.gov.ru/map?"));
    assert!(link.contains("active_layers=36048"));
}
