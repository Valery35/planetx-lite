// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Чтение и запись KML и KMZ для «Моих меток», как в Google Earth.
//! Перенос `core/kml.py` модуля.
//!
//! Чтение даёт дерево: папки [`Folder`], метки [`Place`] и наложения
//! [`Overlay`]. Document и Folder становятся папками, Placemark -
//! меткой. Point, LineString и Polygon с дырами innerBoundaryIs
//! переносятся, MultiGeometry разбивается на отдельные метки с тем же
//! названием. Стиль берётся из styleUrl, у StyleMap - вариант normal,
//! вариант highlight - вид точки под курсором, и из вложенного Style.
//! LookAt метки становится её видом ([`crate::lookat`]): точка взгляда
//! longitude и latitude, расстояние range, азимут heading, наклон tilt.
//! Без точки взгляда ею считается сама метка. Camera метки, папки и шага
//! тура переводится в тот же вид, записывается вид как LookAt.
//!
//! GroundOverlay, ScreenOverlay и PhotoOverlay становятся наложениями
//! ([`crate::overlays`]), картинка из KMZ берётся по href внутри архива.
//! Placemark с Model - 3D-модель, наложение вида model
//! ([`crate::models`]): место Location, Orientation, Scale
//! и altitudeMode, файл модели по Link и картинки по его ссылкам
//! с заменами ResourceMap. Из KMZ файлы модели ложатся пакетом в image
//! наложения, у простого KML href остаётся ссылкой на файл рядом.
//! gx:Track с Model - модель, что едет по треку.
//!
//! Значок точки - Icon в IconStyle ([`crate::icons`]). Время метки -
//! TimeStamp или TimeSpan, время вида - gx:TimeStamp или gx:TimeSpan
//! внутри LookAt. Время папки достаётся её меткам без своего времени.
//! gx:Tour - записанный тур: позы LookAt у gx:FlyTo по времени, gx:Wait
//! держит позу, flyToMode bounce - перелёт с подъёмом.
//! gx:AnimatedUpdate - изменения меток по ходу тура
//! ([`crate::tour::animated`]). Цель изменения - id метки, стиля или
//! геометрии, она сводится к id меток. Запись даёт меткам с id свои id
//! стилей и точки, изменения тура пишутся по ним.
//!
//! Режим высоты altitudeMode и gx:altitudeMode переносится как есть:
//! у relativeToGround, relativeToSeaFloor и absolute высота первой
//! вершины становится высотой метки, extrude - стеной до земли. Линия
//! и многоугольник с absolute и высотой у каждой вершины - 3D-объект.
//!
//! Цвет в KML записан как aabbggrr, в метках - RGBA 0-255. Поля объекта,
//! то есть Data и SchemaData/SimpleData в ExtendedData, переносятся
//! парами (имя, значение). Служебные поля с приставкой «planetx:» в них
//! не входят.
//!
//! KMZ - архив ZIP, из него читается первый файл .kml, doc.kml раньше
//! других. Разбор XML - [`crate::xml`], сущности DTD запрещены. Файл,
//! который не разобрался, правится от частых ошибок моделей помощника -
//! разметки HTML и сущностей HTML в name и description - и читается
//! ещё раз.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

use globe_core::ellipsoid::Ellipsoid;
use quick_xml::escape::resolve_html5_entity;
use serde_json::{Map, Value};

use crate::features::{Highlight, Kind};
use crate::lookat::{self, Layers, View, number_text};
use crate::overlays::{self, TrackRow};
use crate::region::{self, Region};
use crate::tour::{Pace, Sample, Update, pace_parse, pace_text, truthy};
use crate::util::{Archive, dirname, fmt_g, normpath, unquote, write_zip};
use crate::when::{self, Time};
use crate::xml::{self, Node};
use crate::{Rgba, icons, models};

pub const NS: &str = "http://www.opengis.net/kml/2.2";
pub const GX: &str = "http://www.google.com/kml/ext/2.2";
/// Цвета Google Earth по умолчанию: жёлтая линия, белая заливка.
pub const LINE_COLOR: Rgba = [255, 255, 0, 255];
pub const FILL_COLOR: Rgba = [255, 255, 255, 64];
pub const LINE_WIDTH: f64 = 2.0;
pub const FLOW_DATA: &str = "planetx:flow";
pub const LAYERS_DATA: &str = "planetx:layers";
pub const PACE_DATA: &str = "planetx:pace";
pub const RELATIVE: [&str; 2] = ["relativeToGround", "relativeToSeaFloor"];
pub const MODES: [&str; 5] = crate::features::MODES;
/// Геометрии KML. По стандарту они лежат внутри Placemark.
pub const GEOMETRY: [&str; 7] = ["Point", "LineString", "LinearRing", "Polygon", "MultiGeometry", "Track", "MultiTrack"];

/// Файл не читается как KML или KMZ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KmlError(pub String);

impl fmt::Display for KmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for KmlError {}

fn error(text: impl fmt::Display) -> KmlError {
    KmlError(text.to_string())
}

/// Добавки записанного тура: номера поз с перелётом с подъёмом
/// и изменения меток.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TourExtra {
    pub bounce: Vec<usize>,
    pub updates: Vec<Update>,
}

/// Узел дерева меток.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Folder(Folder),
    Place(Place),
    Overlay(Overlay),
}

/// Папка KML: название, флажок, дети.
///
/// description - описание, view - вид или None. Стиль списка, как в окне
/// свойств папки Google Earth: radio - содержимое группой
/// переключателей (listItemType radioFolder), expandable - папку можно
/// раскрыть (без него checkHideChildren). region - Region или None.
/// Сетевая ссылка (NetworkLink): link - адрес документа, refresh -
/// промежуток обновления в секундах, 0 - один раз. Детей у неё в файле
/// нет, их загружает окно глобуса. view_mode - viewRefreshMode Link,
/// «onRegion» - ссылка суперналожения.
#[derive(Debug, Clone, PartialEq)]
pub struct Folder {
    pub name: String,
    pub visible: bool,
    pub children: Vec<Item>,
    pub description: String,
    pub view: Option<View>,
    pub radio: bool,
    pub expandable: bool,
    pub region: Option<Region>,
    pub link: String,
    pub refresh: f64,
    pub view_mode: String,
}

impl Folder {
    /// Видимая пустая папка.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            visible: true,
            children: Vec::new(),
            description: String::new(),
            view: None,
            radio: false,
            expandable: true,
            region: None,
            link: String::new(),
            refresh: 0.0,
            view_mode: String::new(),
        }
    }

    /// Видимая папка с детьми.
    pub fn with(name: &str, children: Vec<Item>) -> Self {
        let mut out = Self::new(name);
        out.children = children;
        out
    }

    /// Все метки внутри, на любой глубине.
    pub fn places(&self) -> Vec<&Place> {
        let mut out = Vec::new();
        for child in &self.children {
            match child {
                Item::Folder(f) => out.extend(f.places()),
                Item::Place(p) => out.push(p),
                Item::Overlay(_) => {}
            }
        }
        out
    }

    /// Все наложения внутри, на любой глубине.
    pub fn overlays(&self) -> Vec<&Overlay> {
        let mut out = Vec::new();
        for child in &self.children {
            match child {
                Item::Folder(f) => out.extend(f.overlays()),
                Item::Overlay(o) => out.push(o),
                Item::Place(_) => {}
            }
        }
        out
    }

    /// То же для правки.
    pub fn overlays_mut(&mut self) -> Vec<&mut Overlay> {
        let mut out = Vec::new();
        collect_overlays(self, &mut out);
        out
    }
}

fn collect_overlays<'a>(folder: &'a mut Folder, out: &mut Vec<&'a mut Overlay>) {
    for child in &mut folder.children {
        match child {
            Item::Folder(f) => collect_overlays(f, out),
            Item::Overlay(o) => out.push(o),
            Item::Place(_) => {}
        }
    }
}

/// Метка KML в виде «Моих меток».
///
/// points - (широта, долгота). color, fill - RGBA, fill только
/// у многоугольника. view - вид или None. icon - значок точки. time
/// и view_time - время метки и вида или None. tour - записанный тур:
/// позы (время, широта, долгота, расстояние, азимут, наклон), тогда
/// points - точки взгляда. alts - высоты вершин 3D-линии или
/// 3D-многоугольника, в KML это altitudeMode absolute. Прочие поля -
/// как у [`crate::features::Shape`]. data - поля объекта парами, pace -
/// темп остановки тура, layers - слои вида, flow - поток по пути,
/// balloon - шаблон всплывающего окна, kml_id - id метки в KML,
/// tour_extra - добавки записанного тура.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: String,
    pub kind: Kind,
    pub points: Vec<(f64, f64)>,
    pub color: Rgba,
    pub width: f64,
    pub fill: Option<Rgba>,
    pub visible: bool,
    pub view: Option<View>,
    pub description: String,
    pub height: f64,
    pub extrude: bool,
    pub icon: String,
    pub time: Option<Time>,
    pub view_time: Option<Time>,
    pub tour: Option<Vec<Sample>>,
    pub alts: Option<Vec<f64>>,
    pub region: Option<Region>,
    pub flow: bool,
    pub layers: Option<Layers>,
    pub pace: Option<Pace>,
    pub mode: String,
    pub holes: Option<Vec<Vec<(f64, f64)>>>,
    pub data: Option<Vec<(String, String)>>,
    pub icon_scale: f64,
    pub label_color: Option<Rgba>,
    pub label_scale: f64,
    pub icon_heading: f64,
    pub balloon: String,
    pub kml_id: String,
    pub tour_extra: Option<TourExtra>,
    pub highlight: Option<Highlight>,
}

impl Place {
    /// Метка со свойствами по умолчанию.
    pub fn new(name: &str, kind: Kind, points: Vec<(f64, f64)>) -> Self {
        Self {
            name: name.into(),
            kind,
            points,
            color: LINE_COLOR,
            width: LINE_WIDTH,
            fill: None,
            visible: true,
            view: None,
            description: String::new(),
            height: 0.0,
            extrude: false,
            icon: icons::DEFAULT.into(),
            time: None,
            view_time: None,
            tour: None,
            alts: None,
            region: None,
            flow: false,
            layers: None,
            pace: None,
            mode: String::new(),
            holes: None,
            data: None,
            icon_scale: 1.0,
            label_color: None,
            label_scale: 1.0,
            icon_heading: 0.0,
            balloon: String::new(),
            kml_id: String::new(),
            tour_extra: None,
            highlight: None,
        }
    }
}

/// Наложение KML: GroundOverlay, ScreenOverlay, PhotoOverlay или
/// 3D-модель.
///
/// overlay - свойства наложения. image - байты картинки или пакет
/// модели, None - нет. href - адрес картинки из файла. aliases - замены
/// путей ResourceMap модели: путь в модели - путь файла. region -
/// Region наложения или None.
#[derive(Debug, Clone, PartialEq)]
pub struct Overlay {
    pub name: String,
    pub overlay: overlays::Overlay,
    pub image: Option<Vec<u8>>,
    pub href: String,
    pub visible: bool,
    pub description: String,
    pub view: Option<View>,
    pub time: Option<Time>,
    pub aliases: BTreeMap<String, String>,
    pub region: Option<Region>,
}

impl Overlay {
    pub fn new(name: &str, overlay: overlays::Overlay) -> Self {
        Self {
            name: name.into(),
            overlay,
            image: None,
            href: String::new(),
            visible: true,
            description: String::new(),
            view: None,
            time: None,
            aliases: BTreeMap::new(),
            region: None,
        }
    }

    pub fn kind(&self) -> overlays::Kind {
        self.overlay.kind
    }
}

/// Текст для XML: &, <, > заменены.
pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn attr_escape(text: &str) -> String {
    escape(text).replace('"', "&quot;")
}

// Правка KML, который не разобрался.

/// Текст сущностей и кодов символов HTML, как html.unescape в Python.
/// Понимаются записи с точкой с запятой в конце.
fn html_unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest.find(';').filter(|e| *e > 1 && *e < 40);
        let resolved = end.and_then(|e| {
            let name = &rest[1..e];
            if let Some(num) = name.strip_prefix('#') {
                let code = match num.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => num.parse::<u32>().ok(),
                }?;
                Some(char::from_u32(code).filter(|c| *c != '\0').unwrap_or('\u{FFFD}').to_string())
            } else if name.chars().all(|c| c.is_ascii_alphanumeric()) {
                resolve_html5_entity(name).map(String::from)
            } else {
                None
            }
        });
        match (resolved, end) {
            (Some(text), Some(e)) => {
                out.push_str(&text);
                rest = &rest[e + 1..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Замена тегов `<br>`, `<p>`, `</p>` на перевод строки и удаление
/// прочих тегов.
fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        match tail.find('>') {
            Some(close) => {
                let inner = tail[1..close].trim_start().to_lowercase();
                let brk = ["br", "/p", "p"].iter().any(|w| {
                    inner.strip_prefix(w).is_some_and(|after| after.chars().next().is_none_or(|c| !is_word(c)))
                });
                if brk {
                    out.push('\n');
                }
                rest = &tail[close + 1..];
            }
            None => {
                out.push_str(tail);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Текст name или description без разметки HTML, для XML.
fn plain(text: &str) -> String {
    if text.trim_start().starts_with("<![CDATA[") {
        return text.to_string();
    }
    escape(&html_unescape(&strip_tags(text)))
}

/// Тексты name и description с разметкой HTML внутри - без неё.
fn mend_texts(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < text.len() {
        if bytes[i] == b'<'
            && let Some((open_end, close_start, close_end)) = text_tag(text, i)
        {
            out.push_str(&text[i..open_end]);
            out.push_str(&plain(&text[open_end..close_start]));
            out.push_str(&text[close_start..close_end]);
            i = close_end;
            continue;
        }
        let ch_len = text[i..].chars().next().map_or(1, char::len_utf8);
        out.push_str(&text[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// Тег name или description с позиции at: конец открывающего тега,
/// начало и конец закрывающего.
fn text_tag(text: &str, at: usize) -> Option<(usize, usize, usize)> {
    let after = &text[at + 1..];
    let word = ["name", "description"].into_iter().find(|w| after.starts_with(w))?;
    let rest = &after[word.len()..];
    if rest.chars().next().is_some_and(is_word) {
        return None;
    }
    let open_end = at + 1 + word.len() + rest.find('>')? + 1;
    let closing = format!("</{word}");
    let mut from = open_end;
    loop {
        let found = from + text[from..].find(&closing)?;
        let tail = &text[found + closing.len()..];
        let spaces = tail.len() - tail.trim_start().len();
        if tail[spaces..].starts_with('>') {
            return Some((open_end, found, found + closing.len() + spaces + 1));
        }
        from = found + closing.len();
    }
}

/// Сущности HTML вида &name; вне пяти сущностей XML заменяются текстом.
/// Голые & становятся &amp;.
fn mend_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 1..];
        let name_len = tail.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(tail.len());
        let named = name_len > 0 && tail.starts_with(|c: char| c.is_ascii_alphabetic()) && tail[name_len..].starts_with(';');
        if named {
            let name = &tail[..name_len];
            if ["amp", "lt", "gt", "quot", "apos"].contains(&name) {
                out.push('&');
                out.push_str(name);
                out.push(';');
            } else {
                out.push_str(&escape(&html_unescape(&format!("&{name};"))));
            }
            rest = &tail[name_len + 1..];
            continue;
        }
        let numeric = tail.strip_prefix('#').and_then(|n| {
            let (digits, hex) = match n.strip_prefix('x') {
                Some(h) => (h, true),
                None => (n, false),
            };
            let len = digits.find(|c: char| if hex { !c.is_ascii_hexdigit() } else { !c.is_ascii_digit() }).unwrap_or(digits.len());
            (len > 0 && digits[len..].starts_with(';')).then_some(())
        });
        if numeric.is_none() {
            out.push_str("&amp;");
        } else {
            out.push('&');
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// Текст KML, который разбор не принял, после правки частых ошибок
/// моделей помощника: разметка HTML и голые &, < в name и description,
/// сущности HTML вроде &nbsp; и &mdash;. Одна такая ошибка теряла весь
/// документ. Файл с объявлениями сущностей не правится.
fn mend(data: &[u8]) -> Option<String> {
    let text = xml::decode(data).unwrap_or_else(|_| String::from_utf8_lossy(data).into_owned());
    if text.contains("<!ENTITY") {
        return None;
    }
    let text = mend_entities(&mend_texts(&text));
    let start = text.len() - text.trim_start().len();
    if text[start..].starts_with("<?xml")
        && let Some(close) = text[start..].find('>')
        && text[..start + close].ends_with('?')
    {
        return Some(text[start + close + 1..].to_string());
    }
    Some(text)
}

// Чтение.

fn text(node: Option<&Node>, name: &str) -> String {
    node.map_or(String::new(), |n| n.text_of(name))
}

fn text_or(node: Option<&Node>, name: &str, default: &str) -> String {
    node.and_then(|n| n.child(name)).map_or(default.to_string(), |c| c.text.trim().to_string())
}

fn float_opt(node: Option<&Node>, name: &str) -> Option<f64> {
    text(node, name).parse().ok()
}

fn float(node: Option<&Node>, name: &str, default: f64) -> f64 {
    float_opt(node, name).unwrap_or(default)
}

/// RGBA из цвета KML aabbggrr или None.
pub fn kml_color(text: &str) -> Option<Rgba> {
    let text = text.trim().trim_start_matches('#');
    if text.len() != 8 || !text.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&text[i..i + 2], 16).ok();
    Some([byte(6)?, byte(4)?, byte(2)?, byte(0)?])
}

/// Цвет KML aabbggrr из RGBA.
pub fn color_kml(rgba: Rgba) -> String {
    let [r, g, b, a] = rgba;
    format!("{a:02x}{b:02x}{g:02x}{r:02x}")
}

/// (широта, долгота) из «lon,lat[,alt]» или None, если не число или
/// вне пределов. Долгота 180-360 - запись 0-360, она переводится в ±180.
fn pair(item: &str) -> Option<(f64, f64)> {
    let item = item.replace('\u{2212}', "-");
    let mut parts = item.split(',');
    let lon: f64 = parts.next()?.trim().parse().ok()?;
    let lat: f64 = parts.next()?.trim().parse().ok()?;
    let lon = if 180.0 < lon.abs() && lon.abs() <= 360.0 { (lon + 180.0).rem_euclid(360.0) - 180.0 } else { lon };
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then_some((lat, lon))
}

/// Записи «lon,lat[,alt]» из coordinates узла. Пробелы вокруг запятых
/// снимаются: модели помощника пишут «lon, lat, alt», 4 октября 2026
/// года так пропали все метки документа о Колумбе.
fn items(node: Option<&Node>) -> Vec<String> {
    let raw = text(node, "coordinates");
    let joined = raw.split(',').map(str::trim).collect::<Vec<_>>().join(",");
    joined.split_whitespace().map(String::from).collect()
}

/// Вершины (широта, долгота) из coordinates.
fn coords(node: Option<&Node>) -> Vec<(f64, f64)> {
    items(node).iter().filter_map(|i| pair(i)).collect()
}

/// Режим высоты геометрии: altitudeMode или gx:altitudeMode.
fn mode_of(node: &Node) -> String {
    let mode = node.text_of("altitudeMode");
    if MODES.contains(&mode.as_str()) { mode } else { "clampToGround".into() }
}

/// Высота, выдавливание и режим высоты геометрии node.
fn raise(node: &Node, coords_node: &Node) -> (f64, bool, String) {
    let mode = mode_of(node);
    if !RELATIVE.contains(&mode.as_str()) && mode != "absolute" {
        return (0.0, false, mode);
    }
    let first = items(Some(coords_node)).into_iter().next().unwrap_or_default();
    let parts: Vec<&str> = first.split(',').collect();
    let mut height = parts.get(2).and_then(|p| p.trim().parse::<f64>().ok()).unwrap_or(0.0);
    if mode != "absolute" {
        height = height.max(0.0);
    }
    (height, text_or(Some(node), "extrude", "0") == "1", mode)
}

/// Высоты вершин линии или многоугольника с altitudeMode absolute, если
/// высота есть у каждой из count вершин, иначе None.
fn alts(node: &Node, coords_node: &Node, count: usize) -> Option<Vec<f64>> {
    if mode_of(node) != "absolute" {
        return None;
    }
    let mut out = Vec::new();
    for item in items(Some(coords_node)) {
        if pair(&item).is_none() {
            continue;
        }
        out.push(item.split(',').nth(2)?.trim().parse::<f64>().ok()?);
    }
    if out.len() == count + 1 {
        out.pop(); // замкнутое кольцо повторяет первую
    }
    (out.len() == count).then_some(out)
}

/// Узлы внешних колец Polygon с coordinates. По стандарту это
/// outerBoundaryIs/LinearRing, модели пишут и LinearRing без
/// outerBoundaryIs, и coordinates прямо в Polygon.
fn outer_rings(polygon: &Node) -> Vec<&Node> {
    let found: Vec<&Node> = polygon.children_named("outerBoundaryIs").map(|o| o.child("LinearRing").unwrap_or(o)).collect();
    if !found.is_empty() {
        return found;
    }
    let found: Vec<&Node> = polygon.children_named("LinearRing").collect();
    if !found.is_empty() {
        return found;
    }
    if polygon.child("coordinates").is_some() { vec![polygon] } else { Vec::new() }
}

/// Вершины кольца без повтора первой.
fn ring(node: &Node) -> Vec<(f64, f64)> {
    let mut points = coords(Some(node));
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    points
}

/// Дыры Polygon: кольца innerBoundaryIs не меньше чем из трёх вершин.
fn holes(polygon: &Node) -> Option<Vec<Vec<(f64, f64)>>> {
    let mut out = Vec::new();
    for inner in polygon.children_named("innerBoundaryIs") {
        let rings: Vec<&Node> = inner.children_named("LinearRing").collect();
        let rings = if rings.is_empty() { vec![inner] } else { rings };
        for r in rings {
            let points = ring(r);
            if points.len() >= 3 {
                out.push(points);
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Геометрия Placemark.
struct Geom {
    kind: Kind,
    points: Vec<(f64, f64)>,
    height: f64,
    extrude: bool,
    alts: Option<Vec<f64>>,
    mode: String,
    holes: Option<Vec<Vec<(f64, f64)>>>,
}

/// Все геометрии Placemark, MultiGeometry по частям.
fn geometries(node: &Node) -> Vec<Geom> {
    let mut out = Vec::new();
    for child in &node.children {
        match child.tag.as_str() {
            "Point" => {
                let points: Vec<(f64, f64)> = coords(Some(child)).into_iter().take(1).collect();
                if !points.is_empty() {
                    let (height, extrude, mode) = raise(child, child);
                    out.push(Geom { kind: Kind::Point, points, height, extrude, alts: None, mode, holes: None });
                }
            }
            "LineString" | "LinearRing" => {
                let points = coords(Some(child));
                if points.len() >= 2 {
                    let (height, extrude, mode) = raise(child, child);
                    let a = alts(child, child, points.len());
                    out.push(Geom { kind: Kind::Line, points, height, extrude, alts: a, mode, holes: None });
                }
            }
            "Polygon" => {
                let mut inner = holes(child);
                for r in outer_rings(child) {
                    let points = ring(r);
                    if points.len() >= 3 {
                        let (height, extrude, mode) = raise(child, r);
                        let a = alts(child, r, points.len());
                        // Дыры - у первого контура.
                        out.push(Geom { kind: Kind::Polygon, points, height, extrude, alts: a, mode, holes: inner.take() });
                    }
                }
            }
            "MultiGeometry" | "MultiTrack" => out.extend(geometries(child)),
            "Track" => {
                // gx:Track - путь точками с моментами времени. На глобусе
                // это линия, время - от первого момента до последнего.
                let points = track_points(child);
                if points.len() >= 2 {
                    out.push(Geom {
                        kind: Kind::Line,
                        points,
                        height: 0.0,
                        extrude: false,
                        alts: None,
                        mode: "clampToGround".into(),
                        holes: None,
                    });
                }
            }
            _ => {}
        }
    }
    out
}

/// Вершины gx:Track: gx:coord «долгота широта высота» через пробел.
fn track_points(node: &Node) -> Vec<(f64, f64)> {
    node.children_named("coord").filter_map(|c| pair(&c.text.split_whitespace().collect::<Vec<_>>().join(","))).collect()
}

/// Промежуток времени треков узла: первый и последний when gx:Track,
/// в том числе внутри gx:MultiTrack и MultiGeometry.
fn track_time(node: &Node) -> Option<Time> {
    let mut whens: Vec<String> = Vec::new();
    for child in &node.children {
        match child.tag.as_str() {
            "Track" => whens.extend(child.children_named("when").map(|w| w.text.trim().to_string())),
            "MultiTrack" | "MultiGeometry" => {
                if let Some((a, b)) = track_time(child) {
                    whens.extend([a, b]);
                }
            }
            _ => {}
        }
    }
    whens.retain(|w| !w.is_empty());
    Some((whens.first()?.clone(), whens.last()?.clone()))
}

/// Стиль KML: поля, которые в нём записаны.
#[derive(Debug, Clone, Default, PartialEq)]
struct Style {
    color: Option<Rgba>,
    width: Option<f64>,
    fill: Option<Rgba>,
    filled: Option<bool>,
    icon: Option<Rgba>,
    href: Option<String>,
    icon_scale: Option<f64>,
    icon_heading: Option<f64>,
    balloon: Option<String>,
    label_color: Option<Rgba>,
    label_scale: Option<f64>,
    list_type: Option<String>,
    highlight: Option<Box<Style>>,
}

impl Style {
    /// Поля other, которые в нём есть, поверх своих.
    fn update(&mut self, other: &Style) {
        macro_rules! take {
            ($($f:ident),*) => {$(if other.$f.is_some() { self.$f = other.$f.clone(); })*};
        }
        take!(color, width, fill, filled, icon, href, icon_scale, icon_heading, balloon, label_color, label_scale, list_type, highlight);
    }

    fn is_empty(&self) -> bool {
        *self == Style::default()
    }
}

/// Стиль узла Style.
fn style_of(node: &Node) -> Style {
    let mut style = Style::default();
    if let Some(line) = node.child("LineStyle") {
        style.color = kml_color(&line.text_of("color"));
        style.width = float_opt(Some(line), "width");
    }
    if let Some(poly) = node.child("PolyStyle") {
        style.fill = kml_color(&poly.text_of("color"));
        style.filled = Some(text_or(Some(poly), "fill", "1") != "0");
    }
    if let Some(icon) = node.child("IconStyle") {
        style.icon = kml_color(&icon.text_of("color"));
        style.href = Some(text(icon.child("Icon"), "href")).filter(|h| !h.is_empty());
        style.icon_scale = float_opt(Some(icon), "scale");
        style.icon_heading = float_opt(Some(icon), "heading");
    }
    if let Some(balloon) = node.child("BalloonStyle") {
        style.balloon = Some(balloon.text_of("text")).filter(|t| !t.is_empty());
    }
    if let Some(label) = node.child("LabelStyle") {
        style.label_color = kml_color(&label.text_of("color"));
        style.label_scale = float_opt(Some(label), "scale");
    }
    if let Some(listing) = node.child("ListStyle") {
        style.list_type = Some(listing.text_of("listItemType")).filter(|t| !t.is_empty());
    }
    style
}

/// Вариант пары StyleMap: свой Style или id стиля.
#[derive(Debug, Clone)]
enum StyleRef {
    Inline(Style),
    Id(String),
}

/// Пары StyleMap: normal и highlight.
fn style_map(node: &Node) -> (Option<StyleRef>, Option<StyleRef>) {
    let (mut normal, mut highlight) = (None, None);
    for pair in node.children_named("Pair") {
        let key = pair.text_of("key");
        let slot = match key.as_str() {
            "normal" => &mut normal,
            "highlight" => &mut highlight,
            _ => continue,
        };
        *slot = match pair.child("Style") {
            Some(inline) => Some(StyleRef::Inline(style_of(inline))),
            None => {
                let id = pair.text_of("styleUrl").trim_start_matches('#').to_string();
                (!id.is_empty()).then_some(StyleRef::Id(id))
            }
        };
    }
    (normal, highlight)
}

/// Стиль StyleMap: вариант normal, вариант highlight - в поле highlight,
/// если он есть.
fn mapped(normal: &Option<StyleRef>, highlight: &Option<StyleRef>, styles: &HashMap<String, Style>) -> Style {
    let pick = |item: &Option<StyleRef>| match item {
        Some(StyleRef::Inline(s)) => s.clone(),
        Some(StyleRef::Id(id)) => styles.get(id).cloned().unwrap_or_default(),
        None => Style::default(),
    };
    let mut style = pick(normal);
    style.highlight = None;
    let mut lit = pick(highlight);
    lit.highlight = None;
    if !lit.is_empty() {
        style.highlight = Some(Box::new(lit));
    }
    style
}

/// Стили документа по id: Style и StyleMap.
fn styles(root: &Node) -> HashMap<String, Style> {
    let mut styles = HashMap::new();
    let mut maps = Vec::new();
    for node in root.iter() {
        let Some(id) = node.get("id").filter(|i| !i.is_empty()) else {
            continue;
        };
        match node.tag.as_str() {
            "Style" => {
                styles.insert(id.to_string(), style_of(node));
            }
            "StyleMap" => maps.push((id.to_string(), style_map(node))),
            _ => {}
        }
    }
    for (id, (normal, highlight)) in maps {
        let style = mapped(&normal, &highlight, &styles);
        styles.insert(id, style);
    }
    styles
}

/// Вид точки под курсором из варианта highlight StyleMap или None.
fn highlight_of(style: &Style) -> Option<Highlight> {
    let lit = style.highlight.as_ref()?;
    let out = Highlight {
        color: lit.icon,
        icon: lit.href.as_deref().map(|h| icons::from_href(Some(h)).to_string()),
        scale: lit.icon_scale,
        label: lit.label_color,
        label_scale: lit.label_scale,
    };
    (out != Highlight::default()).then_some(out)
}

/// Вид из LookAt или Camera. Без точки взгляда LookAt ею считается
/// anchor.
fn view_of(node: &Node, anchor: Option<(f64, f64)>) -> Option<View> {
    let Some(look) = node.child("LookAt") else {
        let camera = node.child("Camera")?;
        let c = Some(camera);
        return lookat::from_camera(
            &Ellipsoid::wgs84(),
            float(c, "latitude", 0.0),
            float(c, "longitude", 0.0),
            float(c, "altitude", 0.0),
            float(c, "heading", 0.0),
            float(c, "tilt", 0.0),
        );
    };
    let l = Some(look);
    let (lat0, lon0) = anchor.unwrap_or((0.0, 0.0));
    lookat::make(
        float(l, "latitude", lat0),
        float(l, "longitude", lon0),
        float(l, "range", 0.0),
        float(l, "heading", 0.0),
        float(l, "tilt", 0.0),
    )
}

/// Время узла: TimeStamp - момент, TimeSpan - промежуток.
fn time_of(node: &Node) -> Option<Time> {
    if let Some(stamp) = node.child("TimeStamp") {
        let when = stamp.text_of("when");
        if !when.is_empty() {
            return Some((when.clone(), when));
        }
    }
    let span = node.child("TimeSpan")?;
    let (begin, end) = (span.text_of("begin"), span.text_of("end"));
    (!begin.is_empty() || !end.is_empty()).then_some((begin, end))
}

/// Время, записанное внутри геометрии Placemark: модели кладут TimeSpan
/// этапа пути в его LineString.
fn geometry_time(node: &Node) -> Option<Time> {
    node.children
        .iter()
        .filter(|c| GEOMETRY.contains(&c.tag.as_str()) && c.tag != "Track")
        .find_map(|c| time_of(c).or_else(|| geometry_time(c)))
}

/// Время вида: gx:TimeStamp или gx:TimeSpan внутри LookAt или Camera.
fn view_time(node: &Node) -> Option<Time> {
    time_of(node.child("LookAt").or_else(|| node.child("Camera"))?)
}

/// Region узла: рамка LatLonAltBox и Lod, или None. Без Lod пределов
/// нет, без рамки Region не действует.
fn region_of(node: &Node) -> Option<Region> {
    let found = node.child("Region")?;
    let rect = found.child("LatLonAltBox");
    let lod = found.child("Lod");
    Some(Region::new(
        float_opt(rect, "north")?,
        float_opt(rect, "south")?,
        float_opt(rect, "east")?,
        float_opt(rect, "west")?,
        float(lod, "minLodPixels", 0.0),
        float(lod, "maxLodPixels", -1.0),
    ))
}

fn visible(node: &Node) -> bool {
    text_or(Some(node), "visibility", "1") != "0"
}

/// Значение Data name из ExtendedData метки или None.
fn data_value(node: &Node, name: &str) -> Option<String> {
    node.child("ExtendedData")?.children_named("Data").find(|d| d.get("name") == Some(name)).map(|d| d.text_of("value"))
}

/// Поля объекта из ExtendedData: Data (подпись - displayName, иначе
/// name) и SimpleData внутри SchemaData.
fn fields(node: &Node) -> Option<Vec<(String, String)>> {
    let data = node.child("ExtendedData")?;
    let mut out = Vec::new();
    for item in &data.children {
        match item.tag.as_str() {
            "Data" => {
                let name = item.get("name").unwrap_or_default();
                if name.starts_with("planetx:") {
                    continue;
                }
                let display = item.text_of("displayName");
                let label = if display.is_empty() { name.to_string() } else { display };
                if !label.is_empty() {
                    out.push((label, item.text_of("value")));
                }
            }
            "SchemaData" => {
                for simple in item.children_named("SimpleData") {
                    let name = simple.get("name").unwrap_or_default();
                    if !name.is_empty() {
                        out.push((name.to_string(), simple.text.trim().to_string()));
                    }
                }
            }
            _ => {}
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Поток по пути: Data FLOW_DATA со значением 1.
fn flow_of(node: &Node) -> bool {
    data_value(node, FLOW_DATA).is_some_and(|v| v == "1" || v == "true")
}

fn placemark(node: &Node, styles: &HashMap<String, Style>, inherited: Option<&Time>) -> Vec<Item> {
    let mut style = styles.get(node.text_of("styleUrl").trim_start_matches('#')).cloned().unwrap_or_default();
    if let Some(inline) = node.child("Style") {
        style.update(&style_of(inline));
    }
    if let Some(inline) = node.child("StyleMap") {
        let (normal, lit) = style_map(inline);
        style.update(&mapped(&normal, &lit, styles));
    }
    let name = node.text_of("name");
    if let Some(model) = node.child("Model") {
        return model_item(node, model, inherited).map(Item::Overlay).into_iter().collect();
    }
    let moving = track_models(node);
    if !moving.is_empty() {
        return moving.into_iter().map(Item::Overlay).collect();
    }
    let mut found = geometries(node);
    if found.is_empty() {
        let spot = match node.child("LookAt") {
            Some(look) => pair(&format!("{},{}", look.text_of("longitude"), look.text_of("latitude"))),
            None => view_of(node, None).map(|v| (v[0], v[1])),
        };
        if let Some(spot) = spot {
            // Метка одним видом, без геометрии: точкой взгляда.
            found.push(Geom {
                kind: Kind::Point,
                points: vec![spot],
                height: 0.0,
                extrude: false,
                alts: None,
                mode: "clampToGround".into(),
                holes: None,
            });
        }
    }
    let time = time_of(node).or_else(|| geometry_time(node)).or_else(|| track_time(node)).or_else(|| inherited.cloned());
    let mut out = Vec::new();
    for g in found {
        let point = g.kind == Kind::Point;
        let mut place = Place::new(&name, g.kind, g.points);
        place.view = view_of(node, place.points.first().copied());
        place.color = (if point { style.icon } else { style.color }).unwrap_or(LINE_COLOR);
        if g.kind == Kind::Polygon && style.filled.unwrap_or(true) {
            place.fill = Some(style.fill.unwrap_or(FILL_COLOR));
        }
        place.width = style.width.unwrap_or(LINE_WIDTH);
        place.visible = visible(node);
        place.description = node.text_of("description");
        place.height = g.height;
        place.extrude = g.extrude;
        place.mode = g.mode;
        place.icon = if point { icons::from_href(style.href.as_deref()).to_string() } else { icons::DEFAULT.into() };
        place.time = time.clone();
        place.view_time = view_time(node);
        place.alts = g.alts;
        place.region = region_of(node);
        place.flow = g.kind == Kind::Line && flow_of(node);
        place.layers = lookat::layers_parse(&data_value(node, LAYERS_DATA).unwrap_or_default());
        place.pace = pace_parse(data_value(node, PACE_DATA).as_deref());
        place.holes = g.holes;
        place.data = fields(node);
        place.icon_scale = style.icon_scale.unwrap_or(1.0);
        place.label_color = style.label_color;
        place.label_scale = style.label_scale.unwrap_or(1.0);
        place.icon_heading = if point { style.icon_heading.unwrap_or(0.0) } else { 0.0 };
        place.balloon = style.balloon.clone().unwrap_or_default();
        place.kml_id = node.get("id").unwrap_or_default().to_string();
        place.highlight = if point { highlight_of(&style) } else { None };
        out.push(Item::Place(place));
    }
    out
}

/// Метки из геометрии, которая лежит в папке без Placemark.
///
/// Стандарт так не разрешает, но модели помощника кладут линию маршрута
/// прямо в Folder, стиль - атрибутом styleUrl. 4 октября 2026 года так
/// пропали четыре маршрута плаваний Колумба. Метка получает название,
/// описание, стиль и время самой геометрии, без них - название и время
/// папки.
fn bare(geometry: &Node, parent: &Node, styles: &HashMap<String, Style>, inherited: Option<&Time>) -> Vec<Item> {
    let mut holder = Node::new("Placemark", Vec::new());
    for tag in ["name", "description", "styleUrl", "Style", "TimeStamp", "TimeSpan"] {
        if let Some(own) = geometry.child(tag) {
            holder.children.push(own.clone());
        }
    }
    if holder.child("name").is_none() {
        let mut name = Node::new("name", Vec::new());
        name.text = parent.text_of("name");
        holder.children.push(name);
    }
    if holder.child("styleUrl").is_none()
        && let Some(url) = geometry.get("styleUrl").filter(|u| !u.is_empty())
    {
        let mut node = Node::new("styleUrl", Vec::new());
        node.text = url.to_string();
        holder.children.push(node);
    }
    holder.children.push(geometry.clone());
    placemark(&holder, styles, inherited)
}

/// Записанный тур из gx:Tour: позы LookAt или Camera у gx:FlyTo
/// по времени, gx:Wait держит позу. Прочие шаги пропускаются.
fn tour_of(node: &Node) -> Option<Place> {
    let playlist = node.child("Playlist")?;
    let mut samples: Vec<Sample> = Vec::new();
    let mut bounce = Vec::new();
    let mut updates = Vec::new();
    let mut t = 0.0;
    for step in &playlist.children {
        let duration = float(Some(step), "duration", 0.0).max(0.0);
        match step.tag.as_str() {
            "AnimatedUpdate" => {
                // Идёт вместе со следующими шагами, время тура не двигает.
                let start = t + float(Some(step), "delayedStart", 0.0).max(0.0);
                let changes = step.child("Update").map(|u| u.children_named("Change").collect::<Vec<_>>()).unwrap_or_default();
                for change in changes {
                    for item in &change.children {
                        let values = changes_of(item);
                        let target = item.get("targetId").unwrap_or_default();
                        if !values.is_empty() && !target.is_empty() {
                            updates.push(Update { t: start, d: duration, ids: vec![target.to_string()], set: values });
                        }
                    }
                }
            }
            "FlyTo" => {
                let pose = match step.child("LookAt") {
                    Some(look) => {
                        let l = Some(look);
                        Some([
                            float(l, "latitude", 0.0),
                            float(l, "longitude", 0.0),
                            float(l, "range", 1000.0),
                            float(l, "heading", 0.0),
                            float(l, "tilt", 0.0),
                        ])
                    }
                    None => view_of(step, None),
                };
                let Some(pose) = pose else {
                    continue;
                };
                t += duration;
                if step.text_of("flyToMode") == "bounce" && !samples.is_empty() {
                    bounce.push(samples.len());
                }
                samples.push([t, pose[0], pose[1], pose[2], pose[3], pose[4]]);
            }
            "Wait" => {
                if let Some(last) = samples.last().copied() {
                    t += duration;
                    samples.push([t, last[1], last[2], last[3], last[4], last[5]]);
                }
            }
            _ => {}
        }
    }
    if samples.len() < 2 {
        return None;
    }
    let points = samples.iter().map(|s| (s[1], s[2])).collect();
    let mut place = Place::new(&node.text_of("name"), Kind::Line, points);
    place.visible = visible(node);
    place.description = node.text_of("description");
    place.tour = Some(samples);
    if !bounce.is_empty() || !updates.is_empty() {
        place.tour_extra = Some(TourExtra { bounce, updates });
    }
    Some(place)
}

/// Свойства из элемента Change gx:AnimatedUpdate, ключи
/// [`crate::tour::UPDATE_KEYS`].
fn changes_of(item: &Node) -> Map<String, Value> {
    let mut out = Map::new();
    if item.child("visibility").is_some() {
        out.insert("visible".into(), Value::Bool(visible(item)));
    }
    let color = kml_color(&item.text_of("color")).map(|c| Value::from(c.to_vec()));
    let me = Some(item);
    match item.tag.as_str() {
        "IconStyle" => {
            if let Some(c) = color {
                out.insert("color".into(), c);
            }
            if let Some(scale) = float_opt(me, "scale") {
                out.insert("icon_scale".into(), Value::from(scale));
            }
        }
        "LineStyle" => {
            if let Some(c) = color {
                out.insert("color".into(), c);
            }
            if let Some(width) = float_opt(me, "width") {
                out.insert("width".into(), Value::from(width));
            }
        }
        "PolyStyle" => {
            if let Some(c) = color {
                out.insert("fill".into(), c);
            }
        }
        "Point" => {
            if let Some((lat, lon)) = coords(me).first() {
                out.insert("points".into(), Value::from(vec![vec![*lat, *lon]]));
            }
        }
        _ => {}
    }
    out
}

/// id элемента - id меток: метка - сама, её стили и геометрии, общий
/// Style и StyleMap - метки, которые на них ссылаются.
fn owners(root: &Node) -> HashMap<String, Vec<String>> {
    let nodes = root.iter();
    let mut owners: HashMap<String, Vec<String>> = HashMap::new();
    let mut users: HashMap<String, Vec<String>> = HashMap::new();
    let mut maps: HashMap<String, String> = HashMap::new();
    for node in &nodes {
        if node.tag == "StyleMap"
            && let Some(id) = node.get("id").filter(|i| !i.is_empty())
        {
            for pair in node.children_named("Pair") {
                if pair.text_of("key") == "normal" {
                    maps.insert(id.to_string(), pair.text_of("styleUrl").trim_start_matches('#').to_string());
                }
            }
        }
    }
    for node in &nodes {
        let Some(pid) = node.get("id").filter(|i| node.tag == "Placemark" && !i.is_empty()) else {
            continue;
        };
        for inner in node.iter() {
            if let Some(id) = inner.get("id").filter(|i| !i.is_empty()) {
                owners.entry(id.to_string()).or_default().push(pid.to_string());
            }
        }
        let url = node.text_of("styleUrl").trim_start_matches('#').to_string();
        let mapped = maps.get(&url).cloned();
        for sid in [Some(url), mapped].into_iter().flatten().filter(|s| !s.is_empty()) {
            users.entry(sid).or_default().push(pid.to_string());
        }
    }
    for node in &nodes {
        if !(node.tag == "Style" || node.tag == "StyleMap") {
            continue;
        }
        let Some(id) = node.get("id").filter(|i| !i.is_empty()) else {
            continue;
        };
        let pids = users.get(id).cloned().unwrap_or_default();
        for inner in node.iter() {
            if let Some(iid) = inner.get("id").filter(|i| !i.is_empty()) {
                owners.entry(iid.to_string()).or_default().extend(pids.iter().cloned());
            }
        }
    }
    owners.into_iter().map(|(k, v)| (k, v.into_iter().collect::<BTreeSet<_>>().into_iter().collect())).collect()
}

/// Цели изменений туров папки - id меток.
fn resolve_updates(folder: &mut Folder, owners: &HashMap<String, Vec<String>>) {
    for child in &mut folder.children {
        match child {
            Item::Folder(f) => resolve_updates(f, owners),
            Item::Place(p) => {
                if let Some(extra) = p.tour_extra.as_mut() {
                    extra.updates.retain_mut(|u| {
                        let ids = u.ids.first().and_then(|t| owners.get(t)).cloned().unwrap_or_default();
                        u.ids = ids;
                        !u.ids.is_empty()
                    });
                }
            }
            Item::Overlay(_) => {}
        }
    }
}

/// listItemType стиля папки: свой Style или Style по styleUrl.
fn list_type(node: &Node, styles: &HashMap<String, Style>) -> String {
    let mut style = styles.get(node.text_of("styleUrl").trim_start_matches('#')).cloned().unwrap_or_default();
    if let Some(inline) = node.child("Style") {
        style.update(&style_of(inline));
    }
    style.list_type.unwrap_or_default()
}

/// overlayXY, screenXY или size: (x, y, единицы x, единицы y).
fn xy_of(node: &Node, name: &str, default: &overlays::Xy) -> overlays::Xy {
    let Some(item) = node.child(name) else {
        return default.clone();
    };
    let number = |key: &str, fallback: f64| item.get(key).map_or(Some(fallback), |v| v.trim().parse::<f64>().ok());
    match (number("x", default.0), number("y", default.1)) {
        (Some(x), Some(y)) => {
            (x, y, item.get("xunits").unwrap_or("fraction").to_string(), item.get("yunits").unwrap_or("fraction").to_string())
        }
        _ => default.clone(),
    }
}

/// Наложение из GroundOverlay, ScreenOverlay или PhotoOverlay, или None,
/// если места наложения в нём нет.
fn overlay_of(node: &Node, inherited: Option<&Time>) -> Option<Overlay> {
    let icon = node.child("Icon");
    let href = text(icon, "href");
    let color = kml_color(&node.text_of("color")).unwrap_or(overlays::WHITE);
    let order = float(Some(node), "drawOrder", 0.0) as i64;
    let mut overlay;
    match node.tag.as_str() {
        "GroundOverlay" => {
            overlay = overlays::Overlay::new(overlays::Kind::Ground);
            if let Some(quad) = node.child("LatLonQuad") {
                let corners: Vec<(f64, f64)> = coords(Some(quad)).into_iter().take(4).collect();
                if corners.len() != 4 {
                    return None;
                }
                overlay.set_corners(corners);
            } else {
                let b = Some(node.child("LatLonBox")?);
                overlay.set_box([
                    float(b, "north", 0.0),
                    float(b, "south", 0.0),
                    float(b, "east", 0.0),
                    float(b, "west", 0.0),
                    float(b, "rotation", 0.0),
                ]);
            }
        }
        "ScreenOverlay" => {
            overlay = overlays::Overlay::new(overlays::Kind::Screen);
            overlay.overlay_xy = xy_of(node, "overlayXY", &overlay.overlay_xy);
            overlay.screen_xy = xy_of(node, "screenXY", &overlay.screen_xy);
            overlay.size = xy_of(node, "size", &overlay.size);
            overlay.rotation = float(Some(node), "rotation", 0.0);
        }
        _ => {
            let camera = node.child("Camera")?;
            overlay = overlays::Overlay::new(overlays::Kind::Photo);
            if let Some(volume) = node.child("ViewVolume") {
                let v = Some(volume);
                let base = overlay.fov;
                overlay.fov = [
                    float(v, "leftFov", base[0]),
                    float(v, "rightFov", base[1]),
                    float(v, "bottomFov", base[2]),
                    float(v, "topFov", base[3]),
                ];
                overlay.near = float(v, "near", overlay.near);
            }
            let c = Some(camera);
            overlay.camera = Some([
                float(c, "latitude", 0.0),
                float(c, "longitude", 0.0),
                float(c, "altitude", 0.0),
                float(c, "heading", 0.0),
                float(c, "tilt", 0.0),
                float(c, "roll", 0.0),
            ]);
        }
    }
    overlay.color = color;
    overlay.order = order;
    if icon.is_some_and(|i| i.text_of("refreshMode") == "onInterval") {
        // Без refreshInterval в KML действует 4 с, модуль поднимает
        // промежуток до MIN_REFRESH.
        overlay.refresh = overlays::refresh_interval(Some(float(icon, "refreshInterval", 4.0)));
    }
    let anchor = overlay.corners.first().copied().or_else(|| overlay.camera.map(|c| (c[0], c[1])));
    let mut item = Overlay::new(&node.text_of("name"), overlay);
    item.href = href;
    item.visible = visible(node);
    item.description = node.text_of("description");
    item.view = view_of(node, anchor);
    item.time = time_of(node).or_else(|| inherited.cloned());
    item.region = region_of(node);
    Some(item)
}

/// Строки пути gx:Track, по времени.
fn track_rows(track: &Node) -> Vec<TrackRow> {
    let stamps: Vec<Option<f64>> = track.children_named("when").map(|w| when::parse(w.text.trim())).collect();
    let coords: Vec<Option<(f64, f64, f64)>> = track
        .children_named("coord")
        .map(|item| {
            let parts: Vec<f64> = item.text.split_whitespace().map(|p| p.parse::<f64>()).collect::<Result<_, _>>().ok()?;
            match parts.as_slice() {
                [lon, lat] => Some((*lat, *lon, 0.0)),
                [lon, lat, alt, ..] => Some((*lat, *lon, *alt)),
                _ => None,
            }
        })
        .collect();
    let angles: Vec<Option<Vec<f64>>> = track
        .children_named("angles")
        .map(|item| item.text.split_whitespace().take(3).map(|v| v.parse::<f64>()).collect::<Result<Vec<_>, _>>().ok())
        .collect();
    let mut rows = Vec::new();
    for (n, (stamp, coord)) in stamps.iter().zip(&coords).enumerate() {
        let (Some(t), Some((lat, lon, alt))) = (stamp, coord) else {
            continue;
        };
        let angle = angles.get(n).cloned().flatten().filter(|a| !a.is_empty());
        let (heading, tilt, roll) = match angle {
            Some(a) => (Some(a[0]), a.get(1).copied().unwrap_or(0.0), a.get(2).copied().unwrap_or(0.0)),
            None => (None, 0.0, 0.0),
        };
        rows.push(TrackRow { t: *t, lat: *lat, lon: *lon, alt: *alt, heading, tilt, roll });
    }
    rows.sort_by(|a, b| a.t.total_cmp(&b.t));
    rows
}

/// Замены путей ResourceMap модели.
fn resource_map(model: &Node) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for alias in model.child("ResourceMap").map(|r| r.children_named("Alias").collect::<Vec<_>>()).unwrap_or_default() {
        let source = unquote(&alias.text_of("sourceHref"));
        let target = unquote(&alias.text_of("targetHref"));
        if !source.is_empty() && !target.is_empty() {
            out.insert(source, target);
        }
    }
    out
}

fn model_scale(model: &Node) -> [f64; 3] {
    let scale = model.child("Scale");
    ["x", "y", "z"].map(|axis| {
        let v = float(scale, axis, 1.0);
        if v == 0.0 { 1.0 } else { v }
    })
}

/// 3D-модели, что едут по gx:Track с Model внутри Placemark: путь
/// в [`overlays::Overlay::track`], время - от первой точки до последней.
/// Линии пути у такого трека нет.
fn track_models(node: &Node) -> Vec<Overlay> {
    let mut found = Vec::new();
    for track in node.iter().into_iter().filter(|n| n.tag == "Track") {
        let model = track.child("Model");
        let href = text(model.and_then(|m| m.child("Link")), "href");
        let rows = track_rows(track);
        let (Some(model), false, true) = (model, href.is_empty(), rows.len() >= 2) else {
            continue;
        };
        let mode = text_or(Some(track), "altitudeMode", "clampToGround");
        let mut overlay = overlays::Overlay::new(overlays::Kind::Model);
        overlay.mode = if models::MODES.contains(&mode.as_str()) { mode } else { "clampToGround".into() };
        let (first, last) = (rows[0], rows[rows.len() - 1]);
        overlay.camera = Some([first.lat, first.lon, first.alt, first.heading.unwrap_or(0.0), first.tilt, first.roll]);
        overlay.scale = model_scale(model);
        overlay.track = Some(rows);
        let mut item = Overlay::new(&node.text_of("name"), overlay);
        item.href = href;
        item.visible = visible(node);
        item.description = node.text_of("description");
        item.view = view_of(node, Some((first.lat, first.lon)));
        item.time = Some((when::text(first.t), when::text(last.t)));
        item.aliases = resource_map(model);
        found.push(item);
    }
    found
}

/// Наложение вида model из Placemark с Model или None без места
/// и файла.
fn model_item(node: &Node, model: &Node, inherited: Option<&Time>) -> Option<Overlay> {
    let location = model.child("Location")?;
    let href = text(model.child("Link"), "href");
    if href.is_empty() {
        return None;
    }
    let orientation = model.child("Orientation");
    let mut mode = text_or(Some(model), "altitudeMode", "clampToGround");
    if mode == "relativeToSeaFloor" {
        mode = "relativeToGround".into();
    }
    if !models::MODES.contains(&mode.as_str()) {
        mode = "clampToGround".into();
    }
    let l = Some(location);
    let (lat, lon) = (float(l, "latitude", 0.0), float(l, "longitude", 0.0));
    let mut overlay = overlays::Overlay::new(overlays::Kind::Model);
    overlay.camera = Some([
        lat,
        lon,
        float(l, "altitude", 0.0),
        float(orientation, "heading", 0.0),
        float(orientation, "tilt", 0.0),
        float(orientation, "roll", 0.0),
    ]);
    overlay.scale = model_scale(model);
    overlay.mode = mode;
    let mut item = Overlay::new(&node.text_of("name"), overlay);
    item.href = href;
    item.visible = visible(node);
    item.description = node.text_of("description");
    item.view = view_of(node, Some((lat, lon)));
    item.time = time_of(node).or_else(|| inherited.cloned());
    item.aliases = resource_map(model);
    Some(item)
}

fn walk(node: &Node, styles: &HashMap<String, Style>, folder: &mut Folder, inherited: Option<&Time>) {
    for child in &node.children {
        match child.tag.as_str() {
            "Folder" | "Document" => {
                let kind = list_type(child, styles);
                let mut sub = Folder::new(&child.text_of("name"));
                sub.visible = visible(child);
                sub.description = child.text_of("description");
                sub.view = view_of(child, None);
                sub.radio = kind == "radioFolder";
                sub.expandable = kind != "checkHideChildren";
                sub.region = region_of(child);
                let own = time_of(child).or_else(|| inherited.cloned());
                walk(child, styles, &mut sub, own.as_ref());
                folder.children.push(Item::Folder(sub));
            }
            "NetworkLink" => {
                if let Some(sub) = network_link(child) {
                    folder.children.push(Item::Folder(sub));
                }
            }
            "Placemark" => folder.children.extend(placemark(child, styles, inherited)),
            "Tour" => {
                if let Some(tour) = tour_of(child) {
                    folder.children.push(Item::Place(tour));
                }
            }
            "GroundOverlay" | "ScreenOverlay" | "PhotoOverlay" => {
                if let Some(item) = overlay_of(child, inherited) {
                    folder.children.push(Item::Overlay(item));
                }
            }
            tag if GEOMETRY.contains(&tag) => folder.children.extend(bare(child, node, styles, inherited)),
            _ => {}
        }
    }
}

/// Папка сетевой ссылки: адрес из Link или старого Url, промежуток
/// по refreshMode onInterval, иначе 0. viewRefreshMode onRegion -
/// ссылка суперналожения, прочие режимы не читаются. Без адреса - None.
fn network_link(node: &Node) -> Option<Folder> {
    let link = node.child("Link").or_else(|| node.child("Url"));
    let href = text(link, "href");
    if href.is_empty() {
        return None;
    }
    let refresh = if text(link, "refreshMode") == "onInterval" { float(link, "refreshInterval", 4.0).max(0.0) } else { 0.0 };
    let mode = text(link, "viewRefreshMode");
    let mut out = Folder::new(&node.text_of("name"));
    out.visible = visible(node);
    out.description = node.text_of("description");
    out.view = view_of(node, None);
    out.region = region_of(node);
    out.link = href;
    out.refresh = refresh;
    out.view_mode = if mode == "onRegion" { mode } else { String::new() };
    Some(out)
}

/// Дерево из байтов KML. name - название корня.
pub fn read_kml(data: &[u8], name: &str) -> Result<Folder, KmlError> {
    let root = match xml::parse(data) {
        Ok(root) => root,
        Err(e) => {
            let mended = mend(data).ok_or_else(|| error(&e))?;
            xml::parse_str(&mended).map_err(error)?
        }
    };
    if root.tag != "kml" {
        return Err(error("not KML"));
    }
    let styles = styles(&root);
    let mut top = Folder::new(name);
    walk(&root, &styles, &mut top, None);
    resolve_updates(&mut top, &owners(&root));
    // Один Document - его содержимое и есть корень. Название - имя
    // документа, как в Google Earth, без него - name, обычно имя файла.
    if let [Item::Folder(only)] = top.children.as_slice()
        && only.link.is_empty()
    {
        let mut out = only.clone();
        if out.name.is_empty() {
            out.name = name.to_string();
        }
        out.refresh = 0.0;
        out.view_mode = String::new();
        return Ok(out);
    }
    Ok(top)
}

/// Имя файла архива по href от папки base или None.
fn in_archive(files: &HashMap<String, String>, base: &str, href: &str) -> Option<String> {
    let mut reference = unquote(&href.replace('\\', "/"));
    while let Some(rest) = reference.strip_prefix("./") {
        reference = rest.to_string();
    }
    if reference.is_empty() {
        return None;
    }
    let joined = if base.is_empty() { reference.clone() } else { normpath(&format!("{base}{reference}")) };
    files.get(&joined.to_lowercase()).or_else(|| files.get(&reference.to_lowercase())).cloned()
}

/// Дерево из байтов KMZ.
pub fn read_kmz(data: &[u8], name: &str) -> Result<Folder, KmlError> {
    let mut archive = Archive::open(data).map_err(error)?;
    let mut names: Vec<String> = archive.names().iter().filter(|n| n.to_lowercase().ends_with(".kml")).cloned().collect();
    names.sort_by(|a, b| (a.to_lowercase() != "doc.kml", a).cmp(&(b.to_lowercase() != "doc.kml", b)));
    let first = names.first().ok_or_else(|| error("no KML in KMZ"))?.clone();
    let mut tree = read_kml(&archive.read(&first).map_err(error)?, name)?;
    // Картинки наложений - файлы архива по href, путь от папки файла KML.
    let files: HashMap<String, String> = archive.names().iter().map(|n| (n.replace('\\', "/").to_lowercase(), n.clone())).collect();
    let base = first.rsplit_once('/').map_or(String::new(), |(dir, _)| format!("{dir}/"));
    let archive = RefCell::new(archive);
    for item in tree.overlays_mut() {
        let Some(found) = in_archive(&files, &base, &item.href) else {
            continue;
        };
        let bytes = archive.try_borrow_mut().map_err(error)?.read(&found).map_err(error)?;
        if item.kind() != overlays::Kind::Model {
            item.image = Some(bytes);
            continue;
        }
        let folder = dirname(&found);
        let prefix = if folder.is_empty() { String::new() } else { format!("{folder}/") };
        let fetch = |reference: &str| -> Option<Vec<u8>> {
            let path = in_archive(&files, &prefix, reference)?;
            archive.try_borrow_mut().ok()?.read(&path).ok()
        };
        item.image = models::collect(&found, &bytes, &fetch, &item.aliases).ok();
    }
    Ok(tree)
}

/// Убрать из дерева модели без файлов: в простом KML файл по ссылке
/// не нашёлся.
pub fn drop_empty_models(folder: &mut Folder) {
    folder.children.retain(|child| {
        !matches!(child, Item::Overlay(o) if o.kind() == overlays::Kind::Model && o.image.as_ref().is_none_or(Vec::is_empty))
    });
    for child in &mut folder.children {
        if let Item::Folder(f) = child {
            drop_empty_models(f);
        }
    }
}

/// KML или KMZ по содержимому: KMZ начинается с подписи ZIP.
pub fn read_file(data: &[u8], name: &str) -> Result<Folder, KmlError> {
    if data.starts_with(b"PK") { read_kmz(data, name) } else { read_kml(data, name) }
}

// Запись.

fn coord_text(points: &[(f64, f64)], close: bool, height: f64, alts: Option<&[f64]>) -> String {
    let mut pts = points.to_vec();
    if close && let Some(first) = points.first() {
        pts.push(*first);
    }
    if let Some(alts) = alts {
        let mut heights = alts.to_vec();
        if close && let Some(first) = alts.first() {
            heights.push(*first);
        }
        return pts.iter().zip(heights).map(|((lat, lon), h)| format!("{lon:.8},{lat:.8},{h:.3}")).collect::<Vec<_>>().join(" ");
    }
    let h = fmt_g(height);
    pts.iter().map(|(lat, lon)| format!("{lon:.8},{lat:.8},{h}")).collect::<Vec<_>>().join(" ")
}

/// Высоты 3D-объекта или None.
fn space(place: &Place) -> Option<&[f64]> {
    let alts = place.alts.as_deref()?;
    (place.kind != Kind::Point && alts.len() == place.points.len()).then_some(alts)
}

/// Режим высоты метки для записи.
fn place_mode(place: &Place) -> &str {
    if MODES.contains(&place.mode.as_str()) {
        return &place.mode;
    }
    if place.height != 0.0 { "relativeToGround" } else { "clampToGround" }
}

/// Режим высоты и выдавливание геометрии для записи. Режимы дна -
/// gx:altitudeMode, как пишет Google Earth.
fn raise_kml(place: &Place) -> String {
    let mode = place_mode(place);
    if mode == "clampToGround" {
        return String::new();
    }
    let tag = if mode.contains("SeaFloor") { "gx:altitudeMode" } else { "altitudeMode" };
    let extrude = if mode != "clampToSeaFloor" { format!("<extrude>{}</extrude>", u8::from(place.extrude)) } else { String::new() };
    format!("{extrude}<{tag}>{mode}</{tag}>")
}

/// TimeStamp или TimeSpan для записи. prefix «gx:» - время вида.
fn time_kml(time: Option<&Time>, prefix: &str) -> String {
    let Some((a, b)) = time.filter(|(a, b)| !a.is_empty() || !b.is_empty()) else {
        return String::new();
    };
    if a == b {
        return format!("<{prefix}TimeStamp><when>{}</when></{prefix}TimeStamp>", escape(a));
    }
    let parts: String = [("begin", a), ("end", b)]
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(tag, v)| format!("<{tag}>{}</{tag}>", escape(v)))
        .collect();
    format!("<{prefix}TimeSpan>{parts}</{prefix}TimeSpan>")
}

fn placemark_kml(place: &Place, indent: usize) -> String {
    let pad = "  ".repeat(indent);
    let mut style;
    if place.kind == Kind::Point {
        let scale = if place.icon_scale != 1.0 { format!("<scale>{}</scale>", fmt_g(place.icon_scale)) } else { String::new() };
        let heading = if place.icon_heading != 0.0 { format!("<heading>{}</heading>", fmt_g(place.icon_heading)) } else { String::new() };
        style = format!(
            "<IconStyle><color>{}</color>{scale}{heading}<Icon><href>{}</href></Icon></IconStyle>",
            color_kml(place.color),
            escape(&icons::href(&place.icon))
        );
        if place.label_color.is_some() || place.label_scale != 1.0 {
            style += &format!(
                "<LabelStyle>{}{}</LabelStyle>",
                place.label_color.map_or(String::new(), |c| format!("<color>{}</color>", color_kml(c))),
                if place.label_scale != 1.0 { format!("<scale>{}</scale>", fmt_g(place.label_scale)) } else { String::new() }
            );
        }
    } else {
        style = format!("<LineStyle><color>{}</color><width>{}</width></LineStyle>", color_kml(place.color), fmt_g(place.width));
        if place.kind == Kind::Polygon {
            style += &format!(
                "<PolyStyle><color>{}</color><fill>{}</fill></PolyStyle>",
                color_kml(place.fill.unwrap_or(FILL_COLOR)),
                u8::from(place.fill.is_some())
            );
        }
    }
    if !place.balloon.is_empty() {
        style += &format!("<BalloonStyle><text>{}</text></BalloonStyle>", escape(&place.balloon));
    }
    let mut style = format!("<Style>{style}</Style>");
    if place.kind == Kind::Point
        && let Some(lit) = &place.highlight
    {
        style = format!(
            "<StyleMap><Pair><key>normal</key>{style}</Pair><Pair><key>highlight</key><Style>{}</Style></Pair></StyleMap>",
            highlight_kml(place, lit)
        );
    }
    let mut mode = raise_kml(place);
    let h = if ["relativeToGround", "relativeToSeaFloor", "absolute"].contains(&place_mode(place)) { place.height } else { 0.0 };
    let alts = space(place);
    // 3D-объект: прямые отрезки на своих высотах, без посадки на рельеф.
    let mut tessellate = "<tessellate>1</tessellate>";
    if alts.is_some() {
        mode = "<altitudeMode>absolute</altitudeMode>".into();
        tessellate = "";
    }
    let geometry = match place.kind {
        Kind::Point => format!("<Point>{mode}<coordinates>{}</coordinates></Point>", coord_text(&place.points, false, h, None)),
        Kind::Line => format!(
            "<LineString>{tessellate}{mode}<coordinates>{}</coordinates></LineString>",
            coord_text(&place.points, false, h, alts)
        ),
        Kind::Polygon => {
            // Дыры у 3D-многоугольника не пишутся, высот у них нет.
            let holes: String = place
                .holes
                .iter()
                .flatten()
                .filter(|hole| alts.is_none() && hole.len() >= 3)
                .map(|hole| {
                    format!(
                        "<innerBoundaryIs><LinearRing><coordinates>{}</coordinates></LinearRing></innerBoundaryIs>",
                        coord_text(hole, true, h, None)
                    )
                })
                .collect();
            format!(
                "<Polygon>{tessellate}{mode}<outerBoundaryIs><LinearRing><coordinates>{}</coordinates></LinearRing></outerBoundaryIs>{holes}</Polygon>",
                coord_text(&place.points, true, h, alts)
            )
        }
    };
    let look = place.view.as_ref().map_or(String::new(), |v| {
        format!(
            "<LookAt>{}<longitude>{:.8}</longitude><latitude>{:.8}</latitude><altitude>0</altitude><heading>{}</heading><tilt>{}</tilt><range>{}</range></LookAt>",
            time_kml(place.view_time.as_ref(), "gx:"),
            v[1],
            v[0],
            fmt_g(v[3]),
            fmt_g(v[4]),
            fmt_g(v[2])
        )
    });
    let mut parts = vec!["<Placemark>".to_string(), format!("<name>{}</name>", escape(&place.name))];
    if !place.description.is_empty() {
        parts.push(format!("<description>{}</description>", escape(&place.description)));
    }
    let mut data: Vec<(String, String)> =
        place.data.iter().flatten().filter(|(n, _)| !n.is_empty() && !n.starts_with("planetx:")).cloned().collect();
    if place.kind == Kind::Line && place.flow {
        data.push((FLOW_DATA.into(), "1".into()));
    }
    let layers = lookat::layers_text(place.layers.as_ref());
    if !layers.is_empty() {
        data.push((LAYERS_DATA.into(), layers));
    }
    let pace = pace_text(place.pace);
    if !pace.is_empty() {
        data.push((PACE_DATA.into(), pace));
    }
    let extended = if data.is_empty() {
        String::new()
    } else {
        let inner: String =
            data.iter().map(|(n, v)| format!("<Data name=\"{}\"><value>{}</value></Data>", attr_escape(n), escape(v))).collect();
        format!("<ExtendedData>{inner}</ExtendedData>")
    };
    parts.extend([
        format!("<visibility>{}</visibility>", u8::from(place.visible)),
        region::kml(place.region.as_ref()),
        time_kml(place.time.as_ref(), ""),
        look,
        style,
        extended,
        geometry,
        "</Placemark>".to_string(),
    ]);
    let mut text: String = parts.concat();
    let kid = attr_escape(&place.kml_id);
    if !kid.is_empty() {
        // Свои id метки, стилей и точки - цели gx:AnimatedUpdate.
        for (tag, suffix) in [("Placemark", ""), ("IconStyle", "_icon"), ("LineStyle", "_line"), ("PolyStyle", "_poly"), ("Point", "_geom")] {
            text = text.replacen(&format!("<{tag}>"), &format!("<{tag} id=\"{kid}{suffix}\">"), 1);
        }
    }
    pad + &text
}

/// Стиль highlight точки: значок и подпись под курсором, недостающее -
/// как в обычном виде.
fn highlight_kml(place: &Place, lit: &Highlight) -> String {
    let color = lit.color.unwrap_or(place.color);
    let icon = lit.icon.clone().filter(|i| !i.is_empty()).unwrap_or_else(|| place.icon.clone());
    let scale = lit.scale.unwrap_or(place.icon_scale);
    let mut text = format!(
        "<IconStyle><color>{}</color><scale>{}</scale><Icon><href>{}</href></Icon></IconStyle>",
        color_kml(color),
        fmt_g(scale),
        escape(&icons::href(&icon))
    );
    if lit.label.is_some() || lit.label_scale.is_some() {
        text += &format!(
            "<LabelStyle>{}<scale>{}</scale></LabelStyle>",
            lit.label.map_or(String::new(), |c| format!("<color>{}</color>", color_kml(c))),
            fmt_g(lit.label_scale.unwrap_or(place.label_scale))
        );
    }
    text
}

/// Записанный тур как gx:Tour: gx:FlyTo плавно к каждой позе.
fn tour_kml(place: &Place, samples: &[Sample], indent: usize) -> String {
    let pad = "  ".repeat(indent);
    let extra = place.tour_extra.clone().unwrap_or_default();
    let bounce: BTreeSet<usize> = extra.bounce.iter().copied().collect();
    let mut updates = extra.updates;
    updates.sort_by(|a, b| a.t.total_cmp(&b.t));
    let mut updates = updates.into_iter().peekable();
    let mut steps = String::new();
    let mut last: Option<f64> = None;
    for (n, s) in samples.iter().enumerate() {
        let [t, lat, lon, distance, heading, tilt] = *s;
        let duration = last.map_or(0.0, |l| t - l);
        last = Some(t);
        steps += &format!(
            "<gx:FlyTo><gx:duration>{duration:.3}</gx:duration><gx:flyToMode>{}</gx:flyToMode><LookAt><longitude>{lon:.8}</longitude><latitude>{lat:.8}</latitude><altitude>0</altitude><heading>{heading:.3}</heading><tilt>{tilt:.3}</tilt><range>{distance:.2}</range></LookAt></gx:FlyTo>",
            if bounce.contains(&n) { "bounce" } else { "smooth" }
        );
        // Изменения, что начинаются до следующей позы, - сразу после
        // этой, с задержкой gx:delayedStart.
        let upto = samples.get(n + 1).map_or(f64::INFINITY, |next| next[0]);
        while let Some(update) = updates.next_if(|u| u.t < upto) {
            steps += &update_kml(&update, t);
        }
    }
    format!("{pad}<gx:Tour><name>{}</name><gx:Playlist>{steps}</gx:Playlist></gx:Tour>", escape(&place.name))
}

fn rgba_of(value: &Value) -> Option<Rgba> {
    let items = value.as_array().filter(|a| a.len() == 4)?;
    let mut out = [0u8; 4];
    for (slot, v) in out.iter_mut().zip(items) {
        *slot = (v.as_f64()? as i64 & 255) as u8;
    }
    Some(out)
}

/// gx:AnimatedUpdate изменения update после позы тура в момент now.
fn update_kml(update: &Update, now: f64) -> String {
    let values = &update.set;
    let mut parts = String::new();
    let number = |key: &str| values.get(key).and_then(Value::as_f64);
    let color = values.get("color").and_then(rgba_of);
    for target in &update.ids {
        let tid = attr_escape(target);
        if let Some(v) = values.get("visible") {
            parts += &format!("<Placemark targetId=\"{tid}\"><visibility>{}</visibility></Placemark>", u8::from(truthy(v)));
        }
        let icon = format!(
            "{}{}",
            color.map_or(String::new(), |c| format!("<color>{}</color>", color_kml(c))),
            number("icon_scale").map_or(String::new(), |s| format!("<scale>{}</scale>", fmt_g(s)))
        );
        if !icon.is_empty() {
            parts += &format!("<IconStyle targetId=\"{tid}_icon\">{icon}</IconStyle>");
            if let Some(c) = color {
                parts += &format!("<LineStyle targetId=\"{tid}_line\"><color>{}</color></LineStyle>", color_kml(c));
            }
        }
        if let Some(width) = number("width") {
            parts += &format!("<LineStyle targetId=\"{tid}_line\"><width>{}</width></LineStyle>", fmt_g(width));
        }
        if let Some(fill) = values.get("fill").and_then(rgba_of) {
            parts += &format!("<PolyStyle targetId=\"{tid}_poly\"><color>{}</color></PolyStyle>", color_kml(fill));
        }
        let first = values.get("points").and_then(Value::as_array).and_then(|p| p.first()).and_then(Value::as_array);
        if let Some([lat, lon, ..]) = first.map(Vec::as_slice)
            && let (Some(lat), Some(lon)) = (lat.as_f64(), lon.as_f64())
        {
            parts += &format!("<Point targetId=\"{tid}_geom\"><coordinates>{lon:.8},{lat:.8}</coordinates></Point>");
        }
    }
    let delay = (update.t - now).max(0.0);
    let delayed = if delay > 0.0 { format!("<gx:delayedStart>{delay:.3}</gx:delayedStart>") } else { String::new() };
    format!(
        "<gx:AnimatedUpdate><gx:duration>{:.3}</gx:duration><Update><targetHref/><Change>{parts}</Change></Update>{delayed}</gx:AnimatedUpdate>",
        update.d
    )
}

/// LookAt вида.
fn look_kml(view: &View) -> String {
    let [lat, lon, distance, heading, tilt] = *view;
    format!(
        "<LookAt><longitude>{lon:.8}</longitude><latitude>{lat:.8}</latitude><altitude>0</altitude><heading>{}</heading><tilt>{}</tilt><range>{}</range></LookAt>",
        fmt_g(heading),
        fmt_g(tilt),
        fmt_g(distance)
    )
}

/// Расширение картинки по первым байтам.
pub fn image_ext(data: &[u8]) -> &'static str {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if data.starts_with(b"GIF") {
        "gif"
    } else {
        "jpg"
    }
}

fn xy_kml(tag: &str, value: &overlays::Xy) -> String {
    let (x, y, xu, yu) = value;
    format!("<{tag} x=\"{}\" y=\"{}\" xunits=\"{}\" yunits=\"{}\"/>", fmt_g(*x), fmt_g(*y), escape(xu), escape(yu))
}

/// Наложение KML, href - адрес картинки в файле.
fn overlay_kml(item: &Overlay, indent: usize, href: &str) -> String {
    let pad = "  ".repeat(indent);
    let overlay = &item.overlay;
    let tag = match overlay.kind {
        overlays::Kind::Ground => "GroundOverlay",
        overlays::Kind::Screen => "ScreenOverlay",
        _ => "PhotoOverlay",
    };
    let mut parts = vec![format!("<{tag}>"), format!("<name>{}</name>", escape(&item.name))];
    if !item.description.is_empty() {
        parts.push(format!("<description>{}</description>", escape(&item.description)));
    }
    parts.push(format!("<visibility>{}</visibility>", u8::from(item.visible)));
    parts.push(time_kml(item.time.as_ref(), ""));
    if let Some(view) = &item.view {
        parts.push(look_kml(view));
    }
    parts.push(format!("<color>{}</color>", color_kml(overlay.color)));
    parts.push(format!("<drawOrder>{}</drawOrder>", overlay.order));
    if !href.is_empty() && overlay.refresh != 0.0 && item.image.is_none() {
        // Картинка по ссылке обновляется через промежуток.
        parts.push(format!(
            "<Icon><href>{}</href><refreshMode>onInterval</refreshMode><refreshInterval>{}</refreshInterval></Icon>",
            escape(href),
            fmt_g(overlay.refresh)
        ));
    } else if !href.is_empty() {
        parts.push(format!("<Icon><href>{}</href></Icon>", escape(href)));
    }
    match overlay.kind {
        overlays::Kind::Ground => {
            let frame = overlay.latlon_box.or_else(|| overlays::corners_box(&overlay.corners).map(|[n, s, e, w]| [n, s, e, w, 0.0]));
            match frame {
                Some([n, s, e, w, r]) => parts.push(format!(
                    "<LatLonBox><north>{n:.8}</north><south>{s:.8}</south><east>{e:.8}</east><west>{w:.8}</west><rotation>{}</rotation></LatLonBox>",
                    fmt_g(r)
                )),
                None => parts.push(format!(
                    "<gx:LatLonQuad><coordinates>{}</coordinates></gx:LatLonQuad>",
                    coord_text(&overlay.corners, false, 0.0, None)
                )),
            }
        }
        overlays::Kind::Screen => {
            parts.push(xy_kml("overlayXY", &overlay.overlay_xy));
            parts.push(xy_kml("screenXY", &overlay.screen_xy));
            parts.push(xy_kml("size", &overlay.size));
            parts.push(format!("<rotation>{}</rotation>", fmt_g(overlay.rotation)));
        }
        _ => {
            let [lat, lon, alt, heading, tilt, roll] = overlay.camera.unwrap_or_default();
            let [left, right, bottom, top] = overlay.fov;
            parts.push(format!(
                "<Camera><longitude>{lon:.8}</longitude><latitude>{lat:.8}</latitude><altitude>{alt:.3}</altitude><heading>{}</heading><tilt>{}</tilt><roll>{}</roll><altitudeMode>absolute</altitudeMode></Camera>",
                fmt_g(heading),
                fmt_g(tilt),
                fmt_g(roll)
            ));
            parts.push(format!(
                "<ViewVolume><leftFov>{}</leftFov><rightFov>{}</rightFov><bottomFov>{}</bottomFov><topFov>{}</topFov><near>{}</near></ViewVolume>",
                fmt_g(left),
                fmt_g(right),
                fmt_g(bottom),
                fmt_g(top),
                fmt_g(overlay.near)
            ));
            parts.push(format!("<Point><coordinates>{lon:.8},{lat:.8},{alt:.3}</coordinates></Point>"));
        }
    }
    parts.push(format!("</{tag}>"));
    pad + &parts.concat()
}

fn resource_map_kml(aliases: Option<&BTreeMap<String, String>>) -> String {
    let Some(aliases) = aliases.filter(|a| !a.is_empty()) else {
        return String::new();
    };
    let inner: String = aliases
        .iter()
        .map(|(source, target)| format!("<Alias><targetHref>{}</targetHref><sourceHref>{}</sourceHref></Alias>", escape(target), escape(source)))
        .collect();
    format!("<ResourceMap>{inner}</ResourceMap>")
}

/// Placemark с Model, href - адрес главного файла модели, aliases -
/// ResourceMap: путь в модели - путь файла.
pub fn model_kml(item: &Overlay, indent: usize, href: &str, aliases: Option<&BTreeMap<String, String>>) -> String {
    let pad = "  ".repeat(indent);
    let overlay = &item.overlay;
    let [lat, lon, alt, heading, tilt, roll] = overlay.camera.unwrap_or_default();
    let [sx, sy, sz] = overlay.scale.map(number_text);
    let scale = format!("<Scale><x>{sx}</x><y>{sy}</y><z>{sz}</z></Scale>");
    let link = format!("<Link><href>{}</href></Link>", escape(href));
    let mut parts = vec!["<Placemark>".to_string(), format!("<name>{}</name>", escape(&item.name))];
    if !item.description.is_empty() {
        parts.push(format!("<description>{}</description>", escape(&item.description)));
    }
    parts.push(format!("<visibility>{}</visibility>", u8::from(item.visible)));
    parts.push(time_kml(item.time.as_ref(), ""));
    if let Some(view) = &item.view {
        parts.push(look_kml(view));
    }
    if let Some(rows) = overlay.track.as_ref().filter(|r| !r.is_empty()) {
        // Модель едет по треку: gx:Track с when, gx:coord, gx:angles.
        parts.push(format!("<gx:Track><altitudeMode>{}</altitudeMode>", escape(&overlay.mode)));
        parts.extend(rows.iter().map(|r| format!("<when>{}</when>", when::text(r.t))));
        parts.extend(
            rows.iter().map(|r| format!("<gx:coord>{} {} {}</gx:coord>", number_text(r.lon), number_text(r.lat), number_text(r.alt))),
        );
        if rows.iter().all(|r| r.heading.is_some()) {
            parts.extend(rows.iter().map(|r| {
                format!(
                    "<gx:angles>{} {} {}</gx:angles>",
                    number_text(r.heading.unwrap_or(0.0)),
                    number_text(r.tilt),
                    number_text(r.roll)
                )
            }));
        }
        parts.extend(["<Model>".to_string(), scale, link, resource_map_kml(aliases)]);
        parts.extend(["</Model>".to_string(), "</gx:Track>".to_string(), "</Placemark>".to_string()]);
        return pad + &parts.concat();
    }
    parts.push(format!("<Model><altitudeMode>{}</altitudeMode>", escape(&overlay.mode)));
    parts.push(format!(
        "<Location><longitude>{}</longitude><latitude>{}</latitude><altitude>{}</altitude></Location>",
        number_text(lon),
        number_text(lat),
        number_text(alt)
    ));
    parts.push(format!(
        "<Orientation><heading>{}</heading><tilt>{}</tilt><roll>{}</roll></Orientation>",
        number_text(heading),
        number_text(tilt),
        number_text(roll)
    ));
    parts.extend([scale, link, resource_map_kml(aliases)]);
    parts.extend(["</Model>".to_string(), "</Placemark>".to_string()]);
    pad + &parts.concat()
}

/// Файлы модели для записи: адрес главного файла, ResourceMap и пары
/// (путь, байты).
pub type ModelFiles = (String, BTreeMap<String, String>, Vec<(String, Vec<u8>)>);

/// Файлы модели наложения item для записи рядом с KML: адрес главного
/// файла, ResourceMap и пары (путь, байты). prefix - папка модели
/// в архиве или рядом с файлом, с косой чертой в конце.
pub fn model_files(item: &Overlay, prefix: &str) -> Result<ModelFiles, models::ModelError> {
    let package = models::unpack(item.image.as_deref().unwrap_or_default())?;
    let aliases = package.aliases.iter().map(|(k, v)| (k.clone(), format!("{prefix}{v}"))).collect();
    let files = package.files.into_iter().map(|(p, d)| (format!("{prefix}{p}"), d)).collect();
    Ok((format!("{prefix}{}", package.main), aliases, files))
}

/// Адрес картинки наложения или файла модели с ResourceMap для записи.
#[derive(Debug, Clone, PartialEq)]
pub enum Href {
    Image(String),
    Model(String, BTreeMap<String, String>),
}

/// NetworkLink папки сетевой ссылки, без содержимого.
fn link_kml(folder: &Folder, indent: usize) -> Vec<String> {
    let pad = "  ".repeat(indent);
    let mut lines = vec![
        format!("{pad}<NetworkLink>"),
        format!("{pad}  <name>{}</name>", escape(&folder.name)),
        format!("{pad}  <visibility>{}</visibility>", u8::from(folder.visible)),
    ];
    if !folder.description.is_empty() {
        lines.push(format!("{pad}  <description>{}</description>", escape(&folder.description)));
    }
    if let Some(view) = &folder.view {
        lines.push(format!("{pad}  {}", look_kml(view)));
    }
    if folder.region.is_some() {
        lines.push(format!("{pad}  {}", region::kml(folder.region.as_ref())));
    }
    lines.push(format!("{pad}  <Link><href>{}</href>", escape(&folder.link)));
    if folder.refresh != 0.0 {
        lines.push(format!(
            "{pad}    <refreshMode>onInterval</refreshMode><refreshInterval>{}</refreshInterval>",
            fmt_g(folder.refresh)
        ));
    }
    if !folder.view_mode.is_empty() {
        lines.push(format!("{pad}    <viewRefreshMode>{}</viewRefreshMode>", escape(&folder.view_mode)));
    }
    lines.push(format!("{pad}  </Link>"));
    lines.push(format!("{pad}</NetworkLink>"));
    lines
}

fn folder_kml(folder: &Folder, indent: usize, tag: &str, hrefs: &HashMap<usize, Href>, counter: &mut usize) -> Vec<String> {
    if !folder.link.is_empty() && tag == "Folder" {
        *counter += folder.overlays().len();
        return link_kml(folder, indent);
    }
    let pad = "  ".repeat(indent);
    let mut lines = vec![
        format!("{pad}<{tag}>"),
        format!("{pad}  <name>{}</name>", escape(&folder.name)),
        format!("{pad}  <visibility>{}</visibility>", u8::from(folder.visible)),
    ];
    if !folder.description.is_empty() {
        lines.push(format!("{pad}  <description>{}</description>", escape(&folder.description)));
    }
    if let Some(view) = &folder.view {
        lines.push(format!("{pad}  {}", look_kml(view)));
    }
    if folder.region.is_some() {
        lines.push(format!("{pad}  {}", region::kml(folder.region.as_ref())));
    }
    let kind = if folder.radio {
        "radioFolder"
    } else if !folder.expandable {
        "checkHideChildren"
    } else {
        ""
    };
    if !kind.is_empty() {
        lines.push(format!("{pad}  <Style><ListStyle><listItemType>{kind}</listItemType></ListStyle></Style>"));
    }
    for child in &folder.children {
        match child {
            Item::Folder(f) => lines.extend(folder_kml(f, indent + 1, "Folder", hrefs, counter)),
            Item::Overlay(o) => {
                let n = *counter;
                *counter += 1;
                if o.kind() == overlays::Kind::Model {
                    let line = match hrefs.get(&n) {
                        Some(Href::Model(href, aliases)) => model_kml(o, indent + 1, href, Some(aliases)),
                        Some(Href::Image(href)) => model_kml(o, indent + 1, href, None),
                        None => model_kml(o, indent + 1, &o.href, Some(&o.aliases)),
                    };
                    lines.push(line);
                } else {
                    let href = match hrefs.get(&n) {
                        Some(Href::Image(href) | Href::Model(href, _)) => href.as_str(),
                        None => o.href.as_str(),
                    };
                    lines.push(overlay_kml(o, indent + 1, href));
                }
            }
            Item::Place(p) => match p.tour.as_ref().filter(|t| !t.is_empty()) {
                Some(samples) => lines.push(tour_kml(p, samples, indent + 1)),
                None => lines.push(placemark_kml(p, indent + 1)),
            },
        }
    }
    lines.push(format!("{pad}</{tag}>"));
    lines
}

/// Текст KML папки: Document с вложенными Folder и Placemark. Адреса
/// картинок наложений - href из исходного файла.
pub fn write_kml(folder: &Folder) -> String {
    write_kml_with(folder, &HashMap::new())
}

/// Текст KML папки. hrefs - адреса картинок наложений по номеру
/// наложения в [`Folder::overlays`], без них - href из исходного файла.
/// У модели - адрес главного файла и ResourceMap.
pub fn write_kml_with(folder: &Folder, hrefs: &HashMap<usize, Href>) -> String {
    let mut lines = vec![r#"<?xml version="1.0" encoding="UTF-8"?>"#.to_string(), format!(r#"<kml xmlns="{NS}" xmlns:gx="{GX}">"#)];
    let mut counter = 0;
    lines.extend(folder_kml(folder, 1, "Document", hrefs, &mut counter));
    lines.push("</kml>".into());
    lines.join("\n") + "\n"
}

/// Байты KMZ: архив с doc.kml, картинками наложений в files/ и файлами
/// моделей в models/.
pub fn write_kmz(folder: &Folder) -> Result<Vec<u8>, KmlError> {
    let mut hrefs = HashMap::new();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for (n, item) in folder.overlays().into_iter().enumerate() {
        let Some(image) = item.image.as_ref().filter(|i| !i.is_empty()) else {
            continue;
        };
        if item.kind() == overlays::Kind::Model {
            let (href, aliases, found) = model_files(item, &format!("models/model{}/", n + 1)).map_err(error)?;
            hrefs.insert(n, Href::Model(href, aliases));
            files.extend(found);
        } else {
            let name = format!("files/overlay{}.{}", n + 1, image_ext(image));
            hrefs.insert(n, Href::Image(name.clone()));
            files.push((name, image.clone()));
        }
    }
    let doc = write_kml_with(folder, &hrefs);
    let entries = std::iter::once(("doc.kml", doc.as_bytes())).chain(files.iter().map(|(n, d)| (n.as_str(), d.as_slice())));
    write_zip(entries).map_err(error)
}
