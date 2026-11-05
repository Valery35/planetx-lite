// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Связь с открытым RustGIS, расчёт без сети. Аналог связи модуля
//! с проектом QGIS: слои проекта на глобусе, вид карты и глобуса
//! в обе стороны. Решение автора от 5 ноября 2026 года.
//!
//! RustGIS отвечает через свой ретранслятор MCP (`ui.mcpRelay`,
//! `crates/assistant/src/relay.rs` RustGIS). Здесь - разбор ответа
//! `list_layers` ([`parse_state`]), стиль слоя из стиля MapLibre, который
//! пишет `share.webStyle` ([`parse_style`]), и перевод вида карты в позу
//! камеры и обратно, как `core/sync.py` модуля.
//!
//! Ответы считаются враждебными: неверное поле - пропуск, а не ошибка.

use serde_json::{Map, Value};

/// Радиус Земли для ширины полосы, метры, `ellipsoid.A` модуля.
const A: f64 = 6_378_137.0;
/// Полоса шире половины экватора не берётся, `MAX_GROUND` модуля.
pub const MAX_GROUND: f64 = 2.0e7;
/// Относительная разница видов, ниже которой виды - один, `SAME_VIEW`
/// модуля. Ответ другой стороны на только что сделанный шаг не даёт
/// нового шага, иначе карта и глобус толкают друг друга по кругу.
pub const SAME_VIEW: f64 = 0.02;

/// Вид карты RustGIS: центр, рамка (запад, юг, восток, север) в градусах
/// и знаменатель масштаба.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub lat: f64,
    pub lon: f64,
    pub extent: [f64; 4],
    pub scale: f64,
}

/// Вид слоя карты.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Point,
    Line,
    Polygon,
    /// Растр, таблица без геометрии и прочее - на глобус не идёт.
    Other,
}

/// Слой проекта RustGIS.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub id: i64,
    pub name: String,
    pub kind: Kind,
    pub visible: bool,
    pub features: u64,
    /// Название отрисовки: «Single Symbol», «Graduated Colors»…
    pub renderer: String,
    /// Поле подписей или пусто.
    pub labels: String,
}

impl Layer {
    /// Слой с геометрией, который ложится на глобус.
    pub fn drawn(&self) -> bool {
        self.kind != Kind::Other
    }

    /// Ключ объектов слоя: другой ключ - объекты читаются заново.
    pub fn data_key(&self) -> String {
        format!("{}|{}|{:?}|{}", self.id, self.name, self.kind, self.features)
    }

    /// Ключ стиля слоя: другой ключ - стиль читается заново. Стиль
    /// читается редко: каждый `share.webStyle` оставляет запись в списке
    /// «Поделиться» проекта.
    pub fn style_key(&self) -> String {
        format!("{}|{}|{}|{}", self.id, self.name, self.renderer, self.labels)
    }
}

/// Состояние проекта: вид и слои сверху вниз.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub view: Option<View>,
    pub layers: Vec<Layer>,
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64().filter(|x| x.is_finite())
}

/// Состояние из текста ответа `list_layers`. Неверный текст - None.
pub fn parse_state(text: &str) -> Option<State> {
    let root: Value = serde_json::from_str(text).ok()?;
    let view = (|| {
        let lat = num(&root["center"]["lat"])?;
        let lon = num(&root["center"]["lon"])?;
        let e = root["extent"].as_array()?;
        let extent = [num(e.first()?)?, num(e.get(1)?)?, num(e.get(2)?)?, num(e.get(3)?)?];
        let scale = num(&root["scale"]).filter(|s| *s > 0.0)?;
        (lat.abs() <= 90.0).then_some(View { lat, lon, extent, scale })
    })();
    let layers = root["layers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| {
            let kind = match l["type"].as_str().unwrap_or("") {
                "Point" | "MultiPoint" => Kind::Point,
                "Polyline" | "LineString" | "MultiLineString" | "Line" => Kind::Line,
                "Polygon" | "MultiPolygon" => Kind::Polygon,
                _ => Kind::Other,
            };
            Some(Layer {
                id: l["id"].as_i64()?,
                name: l["name"].as_str()?.to_string(),
                kind,
                visible: l["visible"].as_bool().unwrap_or(true),
                features: l["features"].as_u64().unwrap_or(0),
                renderer: l["renderer"].as_str().unwrap_or("").to_string(),
                labels: l["labels"].as_str().unwrap_or("").to_string(),
            })
        })
        .collect();
    Some(State { view, layers })
}

/// Цвет из строки CSS: #rgb, #rrggbb, #rrggbbaa, rgb() и rgba().
pub fn css_color(text: &str) -> Option<[u8; 4]> {
    let t = text.trim();
    if let Some(hex) = t.strip_prefix('#') {
        let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        return match hex.len() {
            3 => {
                let d = |i: usize| u8::from_str_radix(hex.get(i..=i)?, 16).ok().map(|v| v * 17);
                Some([d(0)?, d(1)?, d(2)?, 255])
            }
            6 => Some([byte(0)?, byte(2)?, byte(4)?, 255]),
            8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
            _ => None,
        };
    }
    let inner = t.strip_prefix("rgba(").or_else(|| t.strip_prefix("rgb("))?.strip_suffix(')')?;
    let parts: Vec<f64> = inner.split(',').map(|p| p.trim().parse::<f64>()).collect::<Result<_, _>>().ok()?;
    let ch = |i: usize| parts.get(i).map(|v| v.clamp(0.0, 255.0).round() as u8);
    let alpha = parts.get(3).map_or(255, |a| (a.clamp(0.0, 1.0) * 255.0).round() as u8);
    (parts.len() == 3 || parts.len() == 4).then(|| Some([ch(0)?, ch(1)?, ch(2)?, alpha])).flatten()
}

/// Значение поля объекта для выражения: число, строка или ничего.
#[derive(Debug, Clone, PartialEq)]
enum Val {
    Num(f64),
    Text(String),
    Null,
}

impl Val {
    fn of(v: &Value) -> Val {
        match v {
            Value::Number(n) => n.as_f64().map_or(Val::Null, Val::Num),
            Value::String(s) => Val::Text(s.clone()),
            Value::Bool(b) => Val::Text(b.to_string()),
            _ => Val::Null,
        }
    }

    fn number(&self) -> Option<f64> {
        match self {
            Val::Num(x) => Some(*x),
            Val::Text(s) => s.trim().parse().ok(),
            Val::Null => None,
        }
    }

    fn text(&self) -> String {
        match self {
            Val::Num(x) if x.fract() == 0.0 && x.abs() < 1e15 => format!("{}", *x as i64),
            Val::Num(x) => x.to_string(),
            Val::Text(s) => s.clone(),
            Val::Null => String::new(),
        }
    }

    fn same(&self, literal: &Value) -> bool {
        match (self, literal) {
            (Val::Num(a), Value::Number(b)) => b.as_f64() == Some(*a),
            (Val::Text(a), Value::String(b)) => a == b,
            (Val::Text(a), Value::Number(b)) => a.trim().parse::<f64>().ok() == b.as_f64(),
            (Val::Num(a), Value::String(b)) => b.trim().parse::<f64>().ok() == Some(*a),
            _ => false,
        }
    }
}

/// Глубина вложенных выражений не больше: ответ враждебен.
const MAX_DEPTH: usize = 32;

/// Значение выражения MapLibre для объекта с полями props: get,
/// to-number, to-string, coalesce, step, match, case, interpolate
/// и сравнения. Незнакомое выражение - ничего.
fn eval(e: &Value, props: &Map<String, Value>, depth: usize) -> Val {
    if depth > MAX_DEPTH {
        return Val::Null;
    }
    let Some(list) = e.as_array() else {
        return Val::of(e);
    };
    let Some(op) = list.first().and_then(Value::as_str) else {
        return Val::Null;
    };
    let arg = |i: usize| list.get(i).map_or(Val::Null, |a| eval(a, props, depth + 1));
    match op {
        "get" => list.get(1).and_then(Value::as_str).and_then(|k| props.get(k)).map_or(Val::Null, Val::of),
        "literal" => list.get(1).map_or(Val::Null, Val::of),
        "to-number" => arg(1).number().map_or(Val::Null, Val::Num),
        "to-string" => Val::Text(arg(1).text()),
        "coalesce" => (1..list.len()).map(arg).find(|v| *v != Val::Null).unwrap_or(Val::Null),
        "step" => {
            let x = arg(1).number();
            let mut out = arg(2);
            let mut i = 3;
            while i + 1 < list.len() {
                match (x, list[i].as_f64()) {
                    (Some(x), Some(stop)) if x >= stop => out = arg(i + 1),
                    _ => break,
                }
                i += 2;
            }
            out
        }
        "match" => {
            let x = arg(1);
            let mut i = 2;
            while i + 1 < list.len() {
                let hit = match &list[i] {
                    Value::Array(many) => many.iter().any(|v| x.same(v)),
                    one => x.same(one),
                };
                if hit {
                    return arg(i + 1);
                }
                i += 2;
            }
            if list.len() > 2 && list.len() % 2 == 1 { arg(list.len() - 1) } else { Val::Null }
        }
        "case" => {
            let mut i = 1;
            while i + 1 < list.len() {
                if truthy(&list[i], props, depth + 1) {
                    return arg(i + 1);
                }
                i += 2;
            }
            if list.len() % 2 == 0 { arg(list.len() - 1) } else { Val::Null }
        }
        "interpolate" => interpolate(list, props, depth),
        _ => Val::Null,
    }
}

/// Условие выражения case: ==, !=, <, <=, >, >=, !, all, any, has.
fn truthy(e: &Value, props: &Map<String, Value>, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    let Some(list) = e.as_array() else {
        return e.as_bool().unwrap_or(false);
    };
    let op = list.first().and_then(Value::as_str).unwrap_or("");
    let arg = |i: usize| list.get(i).map_or(Val::Null, |a| eval(a, props, depth + 1));
    let compare = |f: fn(f64, f64) -> bool| match (arg(1).number(), arg(2).number()) {
        (Some(a), Some(b)) => f(a, b),
        _ => false,
    };
    match op {
        "==" => arg(1) == arg(2) || (arg(1).number().is_some() && arg(1).number() == arg(2).number()),
        "!=" => !(arg(1) == arg(2) || (arg(1).number().is_some() && arg(1).number() == arg(2).number())),
        "<" => compare(|a, b| a < b),
        "<=" => compare(|a, b| a <= b),
        ">" => compare(|a, b| a > b),
        ">=" => compare(|a, b| a >= b),
        "!" => !list.get(1).is_some_and(|c| truthy(c, props, depth + 1)),
        "all" => list.iter().skip(1).all(|c| truthy(c, props, depth + 1)),
        "any" => list.iter().skip(1).any(|c| truthy(c, props, depth + 1)),
        "has" => list.get(1).and_then(Value::as_str).is_some_and(|k| props.get(k).is_some_and(|v| !v.is_null())),
        _ => false,
    }
}

/// interpolate linear по числу: числа и цвета между соседними опорами.
fn interpolate(list: &[Value], props: &Map<String, Value>, depth: usize) -> Val {
    let Some(x) = list.get(2).map(|a| eval(a, props, depth + 1)).and_then(|v| v.number()) else {
        return Val::Null;
    };
    let stops: Vec<(f64, Val)> = list.get(3..).unwrap_or_default().chunks(2).filter_map(|c| Some((c.first()?.as_f64()?, eval(c.get(1)?, props, depth + 1)))).collect();
    let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
        return Val::Null;
    };
    if x <= first.0 {
        return first.1.clone();
    }
    if x >= last.0 {
        return last.1.clone();
    }
    for w in stops.windows(2) {
        let ((a, va), (b, vb)) = (&w[0], &w[1]);
        if x >= *a && x <= *b {
            let t = if b > a { (x - a) / (b - a) } else { 0.0 };
            return match (va.number(), vb.number(), css_color(&va.text()), css_color(&vb.text())) {
                (Some(p), Some(q), _, _) => Val::Num(p + (q - p) * t),
                (_, _, Some(p), Some(q)) => {
                    let mix = |i: usize| (f64::from(p[i]) + (f64::from(q[i]) - f64::from(p[i])) * t).round() as u8;
                    Val::Text(format!("#{:02x}{:02x}{:02x}{:02x}", mix(0), mix(1), mix(2), mix(3)))
                }
                _ => va.clone(),
            };
        }
    }
    Val::Null
}

/// Стиль слоя карты из стиля MapLibre `share.webStyle`: выражения цвета
/// и размеров для объекта.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Style {
    fill: Option<Value>,
    fill_opacity: Option<Value>,
    line: Option<Value>,
    line_width: Option<Value>,
    circle: Option<Value>,
    circle_radius: Option<Value>,
    /// Поле подписей из text-field, если это ["get", поле].
    pub label_field: Option<String>,
}

/// Поле выражения подписи: ["get", f], ["to-string", ["get", f]].
fn field_of(e: &Value) -> Option<String> {
    let list = e.as_array()?;
    match list.first()?.as_str()? {
        "get" => list.get(1)?.as_str().map(str::to_string),
        "to-string" | "coalesce" => list.get(1).and_then(field_of),
        _ => None,
    }
}

/// Стиль из текста стиля MapLibre. Неверный текст - пустой стиль.
pub fn parse_style(text: &str) -> Style {
    let mut out = Style::default();
    let Ok(root) = serde_json::from_str::<Value>(text) else {
        return out;
    };
    for layer in root["layers"].as_array().into_iter().flatten() {
        let paint = &layer["paint"];
        let take = |key: &str| paint.get(key).cloned();
        match layer["type"].as_str().unwrap_or("") {
            "fill" => {
                out.fill = out.fill.take().or_else(|| take("fill-color"));
                out.fill_opacity = out.fill_opacity.take().or_else(|| take("fill-opacity"));
            }
            "line" => {
                out.line = out.line.take().or_else(|| take("line-color"));
                out.line_width = out.line_width.take().or_else(|| take("line-width"));
            }
            "circle" => {
                out.circle = out.circle.take().or_else(|| take("circle-color"));
                out.circle_radius = out.circle_radius.take().or_else(|| take("circle-radius"));
            }
            "symbol" => {
                out.label_field = out.label_field.take().or_else(|| layer["layout"].get("text-field").and_then(field_of));
            }
            _ => {}
        }
    }
    out
}

impl Style {
    fn color(e: &Option<Value>, props: &Map<String, Value>) -> Option<[u8; 4]> {
        e.as_ref().and_then(|e| css_color(&eval(e, props, 0).text()))
    }

    fn number(e: &Option<Value>, props: &Map<String, Value>) -> Option<f64> {
        e.as_ref().and_then(|e| eval(e, props, 0).number()).filter(|x| x.is_finite())
    }

    /// Цвет объекта: заливка, иначе круг точки, иначе линия.
    pub fn color_of(&self, props: &Map<String, Value>) -> Option<[u8; 4]> {
        Self::color(&self.fill, props).or_else(|| Self::color(&self.circle, props)).or_else(|| Self::color(&self.line, props))
    }

    /// Цвет линии объекта или контура многоугольника.
    pub fn line_of(&self, props: &Map<String, Value>) -> Option<[u8; 4]> {
        Self::color(&self.line, props)
    }

    /// Непрозрачность заливки, 0..1.
    pub fn fill_opacity(&self) -> Option<f64> {
        Self::number(&self.fill_opacity, &Map::new()).map(|x| x.clamp(0.0, 1.0))
    }

    /// Толщина линии в пикселях.
    pub fn line_width(&self) -> Option<f64> {
        Self::number(&self.line_width, &Map::new()).map(|x| x.clamp(0.1, 50.0))
    }

    /// Радиус круга точки в пикселях, для объекта без полей.
    pub fn circle_radius(&self) -> Option<f64> {
        Self::number(&self.circle_radius, &Map::new()).map(|x| x.clamp(0.5, 100.0))
    }
}

/// Цвет «#rrggbb» объекта для поля color слоя Lite.
pub fn hex(c: [u8; 4]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Ширина и высота рамки вида карты на местности, метры: долгота - по
/// параллели центра, широта - по меридиану.
pub fn ground_size(view: &View) -> (f64, f64) {
    let [west, south, east, north] = view.extent;
    let mut dlon = east - west;
    if dlon < 0.0 {
        dlon += 360.0;
    }
    let width = dlon.min(360.0).to_radians() * A * view.lat.to_radians().cos().abs().max(1e-6);
    let height = (north - south).abs().min(180.0).to_radians() * A;
    (width.min(MAX_GROUND), height.min(MAX_GROUND))
}

/// Расстояние камеры, с которого полоса width × height видна целиком,
/// `distance_for` модуля. fov_y - угол обзора по вертикали в градусах,
/// aspect - ширина кадра к высоте.
pub fn distance_for(width: f64, height: f64, fov_y: f64, aspect: f64) -> f64 {
    let half = (fov_y.to_radians() / 2.0).tan().max(1e-6);
    height.max(width / aspect.max(1e-6)).max(1.0) / (2.0 * half)
}

/// Знаменатель масштаба карты для ширины полосы глобуса width в метрах:
/// метров на пиксель карты - как у последнего вида карты last.
pub fn scale_for(width: f64, last: &View) -> f64 {
    let (seen, _) = ground_size(last);
    last.scale * width.max(1.0) / seen.max(1.0)
}

/// Расстояние по дуге большого круга, метры.
fn arc(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let h = ((p2 - p1) / 2.0).sin().powi(2) + p1.cos() * p2.cos() * ((lon2 - lon1).to_radians() / 2.0).sin().powi(2);
    2.0 * A * h.sqrt().min(1.0).asin()
}

/// Совпадают ли виды (широта, долгота, ширина полосы) с точностью
/// [`SAME_VIEW`], `same_view` модуля: сдвиг центра - в долях ширины,
/// размер - отношением.
pub fn same_view(a: (f64, f64, f64), b: (f64, f64, f64)) -> bool {
    let size = a.2.min(b.2).max(1.0);
    let shift = arc(a.0, a.1, b.0, b.1) / size;
    let ratio = (a.2.max(1.0) / b.2.max(1.0)).ln().abs();
    shift < SAME_VIEW && ratio < SAME_VIEW
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const LIST: &str = r#"{"activeMap":"World","center":{"lat":18.0,"lon":10.0},"extent":[-93.9,-52.8,113.9,69.9],"scale":88312391.0,
        "layers":[{"id":4,"name":"Cities","type":"Point","visible":true,"features":243,"renderer":"Proportional Symbols","labels":null},
        {"id":9,"name":"DEM","type":"Raster","visible":true,"features":0,"renderer":"Stretch"},
        {"id":1,"name":"Countries","type":"Polygon","visible":false,"features":177,"renderer":"Graduated Colors","labels":"name"},
        {"name":"no id"}]}"#;

    #[test]
    fn state_of_list_layers() {
        let s = parse_state(LIST).unwrap_or(State { view: None, layers: Vec::new() });
        let v = s.view.unwrap_or(View { lat: 0.0, lon: 0.0, extent: [0.0; 4], scale: 0.0 });
        assert_eq!((v.lat, v.lon, v.scale), (18.0, 10.0, 88312391.0));
        assert_eq!(s.layers.len(), 3);
        assert_eq!(s.layers[0].kind, Kind::Point);
        assert!(!s.layers[1].drawn());
        assert!(!s.layers[2].visible);
        assert_eq!(s.layers[2].labels, "name");
        assert_ne!(s.layers[0].style_key(), s.layers[2].style_key());
        assert!(parse_state("not json").is_none());
        assert!(parse_state(r#"{"layers":[]}"#).is_some_and(|s| s.view.is_none()));
    }

    #[test]
    fn colors() {
        assert_eq!(css_color("#EEF8F6"), Some([0xee, 0xf8, 0xf6, 255]));
        assert_eq!(css_color("#fff"), Some([255, 255, 255, 255]));
        assert_eq!(css_color("#11223380"), Some([0x11, 0x22, 0x33, 0x80]));
        assert_eq!(css_color("rgba(255,255,255,0.5)"), Some([255, 255, 255, 128]));
        assert_eq!(css_color("rgb(1, 2, 3)"), Some([1, 2, 3, 255]));
        for bad in ["", "#12", "#gggggg", "rgb(1,2)", "red", "rgba(1,2,3,4,5)"] {
            assert_eq!(css_color(bad), None, "{bad}");
        }
    }

    /// Стиль слоя Countries образца RustGIS: ступени цвета по pop_est.
    const COUNTRIES: &str = r##"{"layers":[
        {"id":"Countries-fill","type":"fill","paint":{"fill-color":["step",["to-number",["get","pop_est"]],"#EEF8F6",23568378.0,"#A6DDD4",69625582.0,"#52B3B6"],"fill-opacity":1.0}},
        {"id":"Countries-outline","type":"line","paint":{"line-color":"#7D8C96","line-width":0.6}},
        {"id":"Countries-labels","type":"symbol","layout":{"text-field":["to-string",["get","name"]]}}]}"##;

    fn props(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn graduated_colors() {
        let s = parse_style(COUNTRIES);
        assert_eq!(s.color_of(&props(json!({"pop_est": 1000}))), css_color("#EEF8F6"));
        assert_eq!(s.color_of(&props(json!({"pop_est": 23568378}))), css_color("#A6DDD4"));
        assert_eq!(s.color_of(&props(json!({"pop_est": "1e9"}))), css_color("#52B3B6"));
        // Без поля - первый цвет ступеней.
        assert_eq!(s.color_of(&props(json!({}))), css_color("#EEF8F6"));
        assert_eq!(s.line_of(&props(json!({}))), css_color("#7D8C96"));
        assert_eq!(s.fill_opacity(), Some(1.0));
        assert_eq!(s.line_width(), Some(0.6));
        assert_eq!(s.label_field.as_deref(), Some("name"));
    }

    #[test]
    fn unique_values_and_case() {
        let m = parse_style(r##"{"layers":[{"type":"fill","paint":{"fill-color":["match",["get","continent"],"Asia","#ff0000",["Europe","Africa"],"#00ff00","#0000ff"]}}]}"##);
        assert_eq!(m.color_of(&props(json!({"continent": "Asia"}))), Some([255, 0, 0, 255]));
        assert_eq!(m.color_of(&props(json!({"continent": "Africa"}))), Some([0, 255, 0, 255]));
        assert_eq!(m.color_of(&props(json!({"continent": "Oceania"}))), Some([0, 0, 255, 255]));
        let c = parse_style(r##"{"layers":[{"type":"circle","paint":{"circle-color":["case",[">=",["get","pop"],100],"#111111","#222222"],"circle-radius":4}}]}"##);
        assert_eq!(c.color_of(&props(json!({"pop": 150}))), css_color("#111111"));
        assert_eq!(c.color_of(&props(json!({"pop": 5}))), css_color("#222222"));
        assert_eq!(c.circle_radius(), Some(4.0));
        let i = parse_style(r##"{"layers":[{"type":"fill","paint":{"fill-color":["interpolate",["linear"],["get","v"],0,"#000000",10,"#ffffff"]}}]}"##);
        assert_eq!(i.color_of(&props(json!({"v": 5}))), Some([128, 128, 128, 255]));
    }

    #[test]
    fn hostile_styles() {
        assert_eq!(parse_style("oops"), Style::default());
        let mut deep = json!("#ffffff");
        for _ in 0..100 {
            deep = json!(["coalesce", deep]);
        }
        let s = parse_style(&json!({"layers": [{"type": "fill", "paint": {"fill-color": deep}}]}).to_string());
        assert_eq!(s.color_of(&Map::new()), None);
        let odd = parse_style(r#"{"layers":[{"type":"fill","paint":{"fill-color":["step"]}},{"type":"line","paint":{"line-color":["match"]}}]}"#);
        assert_eq!(odd.color_of(&Map::new()), None);
    }

    #[test]
    fn view_and_camera() {
        let v = View { lat: 0.0, lon: 0.0, extent: [-1.0, -0.5, 1.0, 0.5], scale: 1.0e6 };
        let (w, h) = ground_size(&v);
        assert!((w / (2.0f64.to_radians() * A) - 1.0).abs() < 1e-9);
        assert!((h / (1.0f64.to_radians() * A) - 1.0).abs() < 1e-9);
        // Полоса ложится в кадр по большей стороне.
        let d = distance_for(w, h, 45.0, 2.0);
        assert!((2.0 * d * (22.5f64).to_radians().tan() - h.max(w / 2.0)).abs() < 1e-6);
        // Через линию перемены дат.
        let dateline = View { extent: [179.0, -0.5, -179.0, 0.5], ..v };
        assert!((ground_size(&dateline).0 / w - 1.0).abs() < 1e-9);
        // Вдвое шире полоса - вдвое больше знаменатель.
        assert!((scale_for(2.0 * w, &v) / 2.0e6 - 1.0).abs() < 1e-9);
        assert!(ground_size(&View { extent: [-180.0, -90.0, 180.0, 90.0], ..v }).0 <= MAX_GROUND);
    }

    #[test]
    fn same_views() {
        assert!(same_view((58.0, 56.0, 10_000.0), (58.0, 56.0001, 10_100.0)));
        assert!(!same_view((58.0, 56.0, 10_000.0), (58.0, 56.01, 10_000.0)));
        assert!(!same_view((58.0, 56.0, 10_000.0), (58.0, 56.0, 11_000.0)));
    }
}
