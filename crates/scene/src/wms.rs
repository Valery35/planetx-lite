// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Карта из службы WMS, перенос `core/wms.py` модуля. Окно «Новый
//! источник тайлов» читает GetCapabilities ([`parse`]), даёт выбрать слой
//! и заводит карту. Картинка тайла - GetMap рамкой тайла в EPSG:3857
//! ([`getmap_template`], место {bbox} - `net::template_url`).
//!
//! Многие службы покрывают только свой район. Тайлы вне охвата слоя
//! и мельче [`first_level`] не просятся, под районом видна подложка
//! глобуса.
//!
//! Служба ArcGIS PASDA не называет EPSG:3857 в списке систем, но отдаёт
//! картинки в ней, проверено модулем 23 октября 2026 года. Поэтому без
//! кода Меркатора в списке берётся EPSG:3857.

use crate::xml::{self, Node, local_name};

/// Параметры запроса, которые окно ставит само и снимает с адреса.
const OWN_PARAMS: [&str; 12] = ["service", "request", "version", "layers", "styles", "crs", "srs", "bbox", "width", "height", "format", "transparent"];
const MERCATOR_CODES: [&str; 4] = ["EPSG:3857", "EPSG:900913", "EPSG:102100", "EPSG:102113"];
/// Порядок выбора формата: с прозрачностью первыми.
const FORMATS: [&str; 6] = ["image/png", "image/png32", "image/png24", "image/png8", "image/jpeg", "image/jpg"];
/// Пикселей тайла.
pub const SIDE: u32 = 256;
/// Край Web Mercator по широте.
const LAT_LIMIT: f64 = 85.051_128_779_8;
const MERCATOR_HALF: f64 = 20_037_508.342_789_244;
/// Охват шире этого по долготе и широте - весь мир.
const WORLD_LON: f64 = 350.0;
const WORLD_LAT: f64 = 160.0;
/// Тайл района не больше этого количества охватов: у слоёв PASDA DVRPC
/// (около 200 км) - с уровня 6.
const LEVEL_SPAN: f64 = 2.0;
/// Предельный уровень тайлов службы.
pub const MAX_LEVEL: u32 = 19;
/// Наибольшее количество слоёв ответа.
const MAX_LAYERS: usize = 5000;

/// Ответ не разбирается как GetCapabilities службы WMS.
#[derive(Debug, Clone, PartialEq)]
pub struct WmsError(pub String);

impl std::fmt::Display for WmsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WMS: {}", self.0)
    }
}

impl std::error::Error for WmsError {}

/// Слой выбора: имена для LAYERS (у группы - имена её слоёв), название,
/// охват (запад, юг, восток, север) в градусах, системы координат,
/// группа ли это.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub names: Vec<String>,
    pub title: String,
    pub bbox: Option<[f64; 4]>,
    pub crs: Vec<String>,
    pub group: bool,
}

/// Разобранный ответ GetCapabilities.
#[derive(Debug, Clone, PartialEq)]
pub struct Capabilities {
    pub version: String,
    pub title: String,
    pub formats: Vec<String>,
    pub layers: Vec<Layer>,
    pub attribution: String,
}

fn split_url(text: &str) -> (&str, &str) {
    let text = text.trim();
    let text = text.split('#').next().unwrap_or(text);
    text.split_once('?').unwrap_or((text, ""))
}

/// Адрес службы WMS: параметр service=WMS, запрос GetCapabilities или
/// путь, оканчивающийся на WMSServer или /wms.
pub fn is_wms(text: &str) -> bool {
    let (path, query) = split_url(text);
    let (path, query) = (path.to_lowercase(), query.to_lowercase());
    let path = path.trim_end_matches('/');
    query.contains("service=wms") || query.contains("request=getcapabilities") || path.ends_with("wmsserver") || path.ends_with("/wms")
}

/// Адрес службы без параметров, которые окно ставит само.
pub fn base_url(text: &str) -> String {
    let (path, query) = split_url(text);
    let kept: Vec<&str> = query.split('&').filter(|p| !p.is_empty()).filter(|p| !OWN_PARAMS.contains(&p.split('=').next().unwrap_or("").to_lowercase().as_str())).collect();
    if kept.is_empty() { path.to_string() } else { format!("{path}?{}", kept.join("&")) }
}

fn join(base: &str, query: &str) -> String {
    format!("{base}{}{query}", if base.contains('?') { "&" } else { "?" })
}

/// Адрес GetCapabilities службы base.
pub fn capabilities_url(base: &str) -> String {
    join(base, "SERVICE=WMS&REQUEST=GetCapabilities")
}

fn child<'a>(node: &'a Node, tag: &str) -> Option<&'a Node> {
    node.children.iter().find(|c| local_name(&c.tag) == tag)
}

fn text(node: &Node, tag: &str) -> String {
    child(node, tag).map(|c| c.text.trim().to_string()).unwrap_or_default()
}

fn attr<'a>(node: &'a Node, name: &str) -> &'a str {
    node.attrib.iter().find(|(k, _)| local_name(k) == name).map_or("", |(_, v)| v.as_str())
}

/// Охват слоя в градусах: EX_GeographicBoundingBox (1.3.0) или
/// LatLonBoundingBox (1.1.1).
fn layer_box(node: &Node) -> Option<[f64; 4]> {
    let values: Vec<Option<f64>> = if let Some(geo) = child(node, "EX_GeographicBoundingBox") {
        ["westBoundLongitude", "southBoundLatitude", "eastBoundLongitude", "northBoundLatitude"].iter().map(|t| text(geo, t).parse().ok()).collect()
    } else {
        let old = child(node, "LatLonBoundingBox")?;
        ["minx", "miny", "maxx", "maxy"].iter().map(|k| attr(old, k).trim().parse().ok()).collect()
    };
    let [w, s, e, n] = [values[0]?, values[1]?, values[2]?, values[3]?];
    (w.is_finite() && s.is_finite() && e.is_finite() && n.is_finite() && w < e && s < n).then_some([w, s, e, n])
}

fn union(boxes: &[Option<[f64; 4]>]) -> Option<[f64; 4]> {
    let boxes: Vec<[f64; 4]> = boxes.iter().flatten().copied().collect();
    let first = boxes.first()?;
    Some(boxes.iter().fold(*first, |a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]))
}

/// Слои дерева. Системы и охват наследуются от родителя. Группа
/// с именованными слоями внутри даёт строку «все слои группы».
fn layers(node: &Node, crs: &[String], bbox: Option<[f64; 4]>, out: &mut Vec<Layer>, depth: usize) {
    if depth > 32 || out.len() >= MAX_LAYERS {
        return;
    }
    let mut crs = crs.to_vec();
    for c in node.children.iter().filter(|c| matches!(local_name(&c.tag), "CRS" | "SRS")) {
        let code = c.text.trim().to_uppercase();
        if !code.is_empty() && !crs.contains(&code) {
            crs.push(code);
        }
    }
    let bbox = layer_box(node).or(bbox);
    let name = text(node, "Name");
    let title = Some(text(node, "Title")).filter(|t| !t.is_empty()).unwrap_or_else(|| name.clone());
    let start = out.len();
    if !name.is_empty() {
        out.push(Layer { names: vec![name.clone()], title: title.clone(), bbox, crs: crs.clone(), group: false });
    }
    for c in node.children.iter().filter(|c| local_name(&c.tag) == "Layer") {
        layers(c, &crs, bbox, out, depth + 1);
    }
    let inner: Vec<&Layer> = out[start..].iter().filter(|l| !l.group).collect();
    if name.is_empty() && inner.len() > 1 {
        let names = inner.iter().flat_map(|l| l.names.iter().cloned()).collect();
        let boxes: Vec<Option<[f64; 4]>> = inner.iter().map(|l| l.bbox).collect();
        out.insert(start, Layer { names, title, bbox: union(&boxes).or(bbox), crs, group: true });
    }
}

/// Разбор ответа GetCapabilities. Ответ считается враждебным.
pub fn parse(data: &[u8]) -> Result<Capabilities, WmsError> {
    let root = xml::parse(data).map_err(|e| WmsError(e.to_string()))?;
    let tag = local_name(&root.tag);
    if !matches!(tag, "WMS_Capabilities" | "WMT_MS_Capabilities") {
        if matches!(tag, "ServiceExceptionReport" | "ExceptionReport") {
            let mut words = Vec::new();
            collect_text(&root, &mut words);
            return Err(WmsError(if words.is_empty() { "exception".into() } else { words.join(" ") }));
        }
        return Err(WmsError(format!("not WMS capabilities: {tag}")));
    }
    let mut title = child(&root, "Service").map(|s| text(s, "Title")).unwrap_or_default();
    let capability = child(&root, "Capability").ok_or_else(|| WmsError("no Capability".into()))?;
    let formats = child(capability, "Request")
        .and_then(|r| child(r, "GetMap"))
        .map(|g| g.children.iter().filter(|f| local_name(&f.tag) == "Format").map(|f| f.text.trim().to_lowercase()).filter(|f| !f.is_empty()).collect())
        .unwrap_or_default();
    let mut found = Vec::new();
    let mut attribution = String::new();
    for node in capability.children.iter().filter(|c| local_name(&c.tag) == "Layer") {
        layers(node, &[], None, &mut found, 0);
        if let Some(credit) = child(node, "Attribution") {
            attribution = text(credit, "Title");
        }
        if title.is_empty() {
            title = text(node, "Title");
        }
    }
    if found.is_empty() {
        return Err(WmsError("no named layers".into()));
    }
    let version = Some(attr(&root, "version").to_string()).filter(|v| !v.is_empty()).unwrap_or_else(|| "1.3.0".into());
    Ok(Capabilities { version, title, formats, layers: found, attribution })
}

fn collect_text(node: &Node, out: &mut Vec<String>) {
    if !node.text.trim().is_empty() && out.len() < 20 {
        out.push(node.text.trim().to_string());
    }
    for c in &node.children {
        collect_text(c, out);
    }
}

/// Код Web Mercator из списка систем слоя, без него EPSG:3857.
pub fn mercator_code(crs: &[String]) -> &'static str {
    MERCATOR_CODES.iter().find(|c| crs.iter().any(|x| x == *c)).copied().unwrap_or(MERCATOR_CODES[0])
}

/// Формат картинки: с прозрачностью первыми.
pub fn pick_format(formats: &[String]) -> &'static str {
    FORMATS.iter().find(|f| formats.iter().any(|x| x == *f)).copied().unwrap_or(FORMATS[0])
}

/// Знаки значения параметра запроса, как `quote` модуля: буквы, цифры
/// и «-_.~» без замены, safe - тоже.
fn quote(text: &str, safe: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) || safe.as_bytes().contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Адрес GetMap с местом {bbox} - рамкой тайла в метрах EPSG:3857.
pub fn getmap_template(base: &str, version: &str, names: &[String], crs: &str, format: &str) -> String {
    let key = if version.starts_with("1.3") { "CRS" } else { "SRS" };
    let query = [
        "SERVICE=WMS".to_string(),
        format!("VERSION={version}"),
        "REQUEST=GetMap".into(),
        format!("LAYERS={}", quote(&names.join(","), ",")),
        "STYLES=".into(),
        format!("FORMAT={}", quote(format, "")),
        "TRANSPARENT=TRUE".into(),
        format!("WIDTH={SIDE}"),
        format!("HEIGHT={SIDE}"),
        format!("{key}={crs}"),
        "BBOX={bbox}".into(),
    ]
    .join("&");
    join(base, &query)
}

/// Точка в метрах EPSG:3857.
pub fn mercator(lon: f64, lat: f64) -> (f64, f64) {
    let lat = lat.clamp(-LAT_LIMIT, LAT_LIMIT);
    let x = lon.to_radians() * MERCATOR_HALF / std::f64::consts::PI;
    let y = (std::f64::consts::FRAC_PI_4 + lat.to_radians() / 2.0).tan().ln() * MERCATOR_HALF / std::f64::consts::PI;
    (x, y)
}

/// Охват в градусах - рамка в метрах EPSG:3857.
pub fn mercator_box(bbox: [f64; 4]) -> [f64; 4] {
    let (w, s) = mercator(bbox[0], bbox[1]);
    let (e, n) = mercator(bbox[2], bbox[3]);
    [w, s, e, n]
}

/// Охват - весь мир или неизвестен.
pub fn is_world(bbox: Option<[f64; 4]>) -> bool {
    bbox.is_none_or(|b| b[2] - b[0] >= WORLD_LON && b[3] - b[1] >= WORLD_LAT)
}

/// Уровень, с которого тайл не больше [`LEVEL_SPAN`] охватов района.
pub fn first_level(bbox: [f64; 4]) -> u32 {
    let [w, s, e, n] = mercator_box(bbox);
    let span = (e - w).max(n - s).max(1.0) * LEVEL_SPAN;
    let level = (2.0 * MERCATOR_HALF / span).log2().floor();
    level.clamp(0.0, f64::from(MAX_LEVEL)) as u32
}

/// Служба района категории «Локальные данные», `LOCAL` модуля: ключ,
/// название и описание (ключи перевода), адрес, слои, охват в градусах,
/// подпись.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Local {
    pub key: &'static str,
    pub name: &'static str,
    pub about: &'static str,
    pub base: &'static str,
    pub layers: &'static [&'static str],
    pub bbox: [f64; 4],
    pub credit: &'static str,
}

/// Службы районов. PASDA (Pennsylvania Spatial Data Access) - ортофото
/// DVRPC весны 2020 года, Nearmap, 9 округов вокруг Филадельфии,
/// GetCapabilities от 23 октября 2026 года.
pub const LOCAL: [Local; 1] = [Local {
    key: "pasda_dvrpc2020",
    name: "PASDA DVRPC 2020, Philadelphia",
    about: "Spring 2020 orthophoto of nine counties around Philadelphia (USA), the Pennsylvania Spatial Data Access service. The service is slow, pictures arrive in a few seconds.",
    base: "https://imagery.pasda.psu.edu/arcgis/services/pasda/DVRPC2020/MapServer/WMSServer",
    layers: &["0", "1", "2", "3", "4", "5", "6", "7", "8"],
    bbox: [-76.145206, 39.514075, -74.385339, 40.608742],
    credit: "PASDA, DVRPC, Nearmap 2020",
}];

impl Local {
    /// Адрес GetMap службы района.
    pub fn template(&self) -> String {
        let names: Vec<String> = self.layers.iter().map(|s| s.to_string()).collect();
        getmap_template(self.base, "1.3.0", &names, MERCATOR_CODES[0], "image/png")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;

    const CAPS: &str = r#"<?xml version="1.0"?>
<WMS_Capabilities version="1.3.0" xmlns="http://www.opengis.net/wms">
<Service><Title>DVRPC 2020</Title></Service>
<Capability><Request><GetMap><Format>image/jpeg</Format><Format>image/png</Format></GetMap></Request>
<Layer><Title>Orthos</Title><CRS>EPSG:4326</CRS>
<EX_GeographicBoundingBox><westBoundLongitude>-76.1</westBoundLongitude><eastBoundLongitude>-74.4</eastBoundLongitude><southBoundLatitude>39.5</southBoundLatitude><northBoundLatitude>40.6</northBoundLatitude></EX_GeographicBoundingBox>
<Layer><Name>0</Name><Title>Bucks</Title><CRS>EPSG:3857</CRS></Layer>
<Layer><Name>1</Name><Title>Chester</Title>
<EX_GeographicBoundingBox><westBoundLongitude>-76.0</westBoundLongitude><eastBoundLongitude>-75.4</eastBoundLongitude><southBoundLatitude>39.7</southBoundLatitude><northBoundLatitude>40.2</northBoundLatitude></EX_GeographicBoundingBox></Layer>
<Attribution><Title>DVRPC</Title></Attribution>
</Layer></Capability></WMS_Capabilities>"#;

    #[test]
    fn address_and_layers() {
        assert!(is_wms("https://imagery.pasda.psu.edu/arcgis/services/pasda/DVRPC2020/MapServer/WMSServer"));
        assert!(is_wms("https://x.org/geo/ows?service=WMS&request=GetCapabilities"));
        assert!(!is_wms("https://tile.example.org/{z}/{x}/{y}.png"));
        assert_eq!(base_url("https://x.org/wms?SERVICE=WMS&request=GetCapabilities&map=a.map"), "https://x.org/wms?map=a.map");
        let caps = parse(CAPS.as_bytes()).expect("ответ");
        assert_eq!(caps.title, "DVRPC 2020");
        assert_eq!(caps.attribution, "DVRPC");
        // Группа без имени - строка всех слоёв первой.
        assert_eq!(caps.layers.len(), 3);
        assert!(caps.layers[0].group);
        assert_eq!(caps.layers[0].names, ["0", "1"]);
        assert_eq!(caps.layers[1].bbox, Some([-76.1, 39.5, -74.4, 40.6]));
        assert!(caps.layers[1].crs.contains(&"EPSG:3857".to_string()));
        assert_eq!(mercator_code(&caps.layers[2].crs), "EPSG:3857");
        assert_eq!(pick_format(&caps.formats), "image/png");
    }

    #[test]
    fn getmap_and_levels() {
        let t = getmap_template("https://x.org/wms?map=a", "1.1.1", &["a b".into(), "c".into()], "EPSG:3857", "image/png");
        assert!(t.starts_with("https://x.org/wms?map=a&SERVICE=WMS&VERSION=1.1.1&REQUEST=GetMap&LAYERS=a%20b,c&STYLES=&FORMAT=image%2Fpng"));
        assert!(t.ends_with("SRS=EPSG:3857&BBOX={bbox}"));
        assert_eq!(first_level(LOCAL[0].bbox), 6);
        assert!(is_world(None) && is_world(Some([-180.0, -85.0, 180.0, 85.0])) && !is_world(Some(LOCAL[0].bbox)));
        assert!(parse(b"<ServiceExceptionReport><ServiceException>Bad</ServiceException></ServiceExceptionReport>").is_err());
        assert!(parse(b"<html>").is_err());
    }
}
