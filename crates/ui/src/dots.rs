// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Слои точек и линий каталога: землетрясения USGS, вулканы GVP, пожары
//! FIRMS, активные разломы GEM, границы плит PB2002, подводные кабели OSM.
//! Перенос `_set_quakes`, `_place_quakes` (`ui/globe/earthdata.py`),
//! `_identify_quakes` (`ui/globe/identify.py`), `_identify_faults`,
//! `_identify_plates`, `_identify_cables` (`ui/globe/resources.py`)
//! и тех же частей вулканов и пожаров модуля.
//!
//! Сводка просится при включении строки, точки рисуются кругами
//! с тёмным ободком на рельефе, цвет и размер - по правилам слоя. Точки
//! вне промежутка шкалы времени не видны. Линии ложатся в картинку
//! наложения тайлов глобуса. Щелчок «Определить объекты» даёт ближайшие
//! точки и ближайшую линию группами окна «Объекты».

use std::sync::Arc;

use data::faults::{self, Fault};
use data::fires::{self, Fire};
use data::linesets::Line;
use data::plates::{self, Boundary, Cable, Plate};
use data::quakes::{self, Quake};
use data::volcanoes::{self, Volcano};
use globe_core::DVec3;
use globe_core::navigation::Navigator;

use crate::i18n::{Lang, tr, trf};
use crate::identify::Feature;

/// Слой точек.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Layer {
    Quakes,
    Volcanoes,
    Fires,
    Faults,
    Plates,
    Cables,
}

/// Непрозрачность кругов, выбор помощника.
const ALPHA: f32 = 0.9;
/// Сдвиг глубины точек к глазу в долях глубины кадра. При 3e-4 горы
/// Тибета с высоты 9000 км срезали низ кругов. Обратная сторона Земли
/// дальше на доли в сотые, она остаётся закрытой. Выбор помощника.
const LIFT: f32 = 2e-3;
/// Подсказка пожаров, карты витрины модуля.
const FIRES_TIP: &str = "Fire spots of the last 24 hours from VIIRS images of the NOAA-20 satellite, the NASA FIRMS feed. The color and size of a dot show the radiative power.";
/// Ближайших точек в окне «Объекты», `QUAKES_NEAR` модуля.
const NEAR: usize = 20;

impl Layer {
    pub const ALL: [Layer; 6] = [Layer::Quakes, Layer::Volcanoes, Layer::Fires, Layer::Faults, Layer::Plates, Layer::Cables];

    pub fn key(self) -> &'static str {
        match self {
            Layer::Quakes => "quakes",
            Layer::Volcanoes => "volcanoes",
            Layer::Fires => "fires",
            Layer::Faults => "faults",
            Layer::Plates => "plates",
            Layer::Cables => "cables",
        }
    }

    pub fn from_key(key: &str) -> Option<Layer> {
        Self::ALL.into_iter().find(|l| l.key() == key)
    }

    /// Адрес сводки.
    pub fn url(self) -> &'static str {
        match self {
            Layer::Quakes => quakes::FEED,
            Layer::Volcanoes => volcanoes::URL,
            Layer::Fires => fires::FEED,
            Layer::Faults => faults::URL,
            Layer::Plates => plates::PLATES_URL,
            Layer::Cables => plates::CABLES_URL,
        }
    }

    /// Предел ответа сводки, байты: файл разломов около 11 МБ, вулканов -
    /// около 2 МБ.
    pub fn bytes(self) -> u64 {
        if self == Layer::Faults { 32 << 20 } else { 8 << 20 }
    }

    pub fn attribution(self) -> (&'static str, &'static str) {
        match self {
            Layer::Quakes => quakes::ATTRIBUTION,
            Layer::Volcanoes => volcanoes::ATTRIBUTION,
            Layer::Fires => fires::ATTRIBUTION,
            Layer::Faults => faults::ATTRIBUTION,
            Layer::Plates => plates::PLATES_ATTRIBUTION,
            Layer::Cables => plates::CABLES_ATTRIBUTION,
        }
    }

    /// Подсказка строки, `layer_names` и `map_names` модуля.
    pub fn tip(self) -> &'static str {
        match self {
            Layer::Quakes => {
                "Earthquakes of magnitude 4.5 and above over the last 30 days from the USGS feed. A circle marks the epicentre on the surface. The focus is seen at its depth in the removed sector of the Earth cutaway row. The colour shows the focus depth, the size shows the magnitude. The feed is loaded when the row is switched on."
            }
            Layer::Volcanoes => {
                "About 1200 volcanoes that erupted in the last 12 thousand years after the Global Volcanism Program list of the Smithsonian Institution. The colour is the time of the last eruption, the size is the volcano height. A click on a dot shows the volcano type, the year of the last eruption, the tectonic setting and the volcano page on the GVP site. The first switch-on downloads about 2 MB."
            }
            Layer::Fires => FIRES_TIP,
            Layer::Faults => {
                "About 13.7 thousand active faults of the world from the GEM Global Active Faults database compiled from regional catalogues. The colour shows the kind of slip. It is a normal fault, a reverse fault or thrust, a strike-slip fault, a spreading ridge and a fold. A click on a line shows the name, the kind of slip, the slip rate in mm/yr, the dip and the source catalogue. The first switch-on downloads about 11 MB."
            }
            // Надписей названий плит в Lite пока нет, фраза о них из
            // текста модуля снята.
            Layer::Plates => {
                "Lithospheric plate boundaries after the PB2002 model. Red - plates moving apart at ridges and rifts, green - plates sliding along transform faults, blue - plates converging in subduction and collision zones. The Features window shows the boundary type and the plate speed."
            }
            Layer::Cables => {
                "Submarine communication lines (turquoise) and power cables (purple) from OpenStreetMap, visible from any altitude. OSM holds part of the world's cables, most of them in the Atlantic and near Europe. A click on a line shows the name, owner and a link to the OpenStreetMap object."
            }
        }
    }

    /// Источник второй строкой каталога, `SOURCES` модуля.
    pub fn source(self) -> &'static str {
        match self {
            Layer::Quakes => "USGS",
            Layer::Volcanoes => "Smithsonian GVP",
            Layer::Fires => "NASA FIRMS, 24 hours",
            Layer::Faults => "GEM Global Active Faults",
            Layer::Plates => "PB2002",
            Layer::Cables => "OpenStreetMap",
        }
    }

    /// Название строки и группы окна «Объекты».
    pub fn name(self) -> &'static str {
        match self {
            Layer::Quakes => "Earthquakes",
            Layer::Volcanoes => "Volcanoes",
            Layer::Fires => "Fires",
            Layer::Faults => "Active faults",
            Layer::Plates => "Plate boundaries",
            Layer::Cables => "Submarine cables",
        }
    }

    /// Название группы окна «Объекты».
    fn group(self) -> &'static str {
        match self {
            Layer::Plates => "Plate boundary",
            l => l.name(),
        }
    }
}

/// Названия видов смещения, `fault_names` модуля.
pub fn fault_name(kind: &str) -> &'static str {
    match kind {
        "normal" => "Normal faults",
        "reverse" => "Reverse faults, thrusts, subduction",
        "strike" => "Strike-slip faults",
        "ridge" => "Spreading ridges",
        "fold" => "Folds",
        _ => "Kind of slip not given",
    }
}

/// Названия групп и классов границ плит, `_plate_names` модуля.
fn plate_name(code: &str) -> &'static str {
    match code {
        "divergent" => "Plates moving apart",
        "transform" => "Plates sliding",
        "convergent" => "Plates converging",
        "OSR" => "Oceanic spreading ridge",
        "CRB" => "Continental rift",
        "OTF" => "Oceanic transform fault",
        "CTF" => "Continental transform fault",
        "SUB" => "Subduction zone",
        "OCB" => "Oceanic convergent boundary",
        "CCB" => "Continental collision",
        _ => "",
    }
}

/// Число без лишних нулей, как `{:g}` Python для обычных величин.
fn short(v: f64) -> String {
    let text = format!("{v:.6}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Данные слоя.
pub enum Data {
    Quakes(Vec<Quake>),
    Volcanoes(Vec<Volcano>),
    Fires(Vec<Fire>),
    Faults(Vec<Fault>, Arc<Vec<Line>>),
    Plates(Vec<Boundary>, Arc<Vec<Line>>, Vec<Plate>),
    Cables(Vec<Cable>, Arc<Vec<Line>>),
}

/// Состояние слоя: сводка идёт, пришла или не пришла.
pub enum State {
    Asking(Box<dyn net::Request>),
    Ready(Data),
    Failed(String),
}

/// Точка для отрисовки и выбора: место, цвет, размер, время, номер
/// записи.
struct Dot {
    lat: f64,
    lon: f64,
    color: [u8; 3],
    size: f32,
    time: Option<f64>,
}

impl Data {
    fn parse(layer: Layer, bytes: &[u8]) -> Result<Data, String> {
        Ok(match layer {
            Layer::Quakes => Data::Quakes(quakes::parse(bytes)),
            Layer::Volcanoes => Data::Volcanoes(volcanoes::parse(bytes)?),
            Layer::Fires => Data::Fires(fires::parse(&String::from_utf8_lossy(bytes))),
            Layer::Faults => {
                let (f, l) = faults::parse(bytes)?;
                Data::Faults(f, Arc::new(l))
            }
            Layer::Plates => {
                let (b, l, p) = plates::parse_plates(bytes)?;
                Data::Plates(b, Arc::new(l), p)
            }
            Layer::Cables => {
                let (c, l, _) = plates::parse_cables(bytes)?;
                Data::Cables(c, Arc::new(l))
            }
        })
    }

    /// Линии слоя линий.
    fn lines(&self) -> Option<&Arc<Vec<Line>>> {
        match self {
            Data::Faults(_, l) | Data::Plates(_, l, _) | Data::Cables(_, l) => Some(l),
            _ => None,
        }
    }

    fn dots(&self) -> Vec<Dot> {
        match self {
            Data::Quakes(q) => q.iter().map(|e| Dot { lat: e.lat, lon: e.lon, color: quakes::depth_color(e.depth), size: quakes::size(e.mag) as f32, time: e.time }).collect(),
            Data::Volcanoes(v) => v
                .iter()
                .map(|e| Dot { lat: e.lat, lon: e.lon, color: volcanoes::PERIODS[volcanoes::period_of(e.last)].2, size: volcanoes::size(e.elevation) as f32, time: None })
                .collect(),
            Data::Fires(f) => f.iter().map(|e| Dot { lat: e.lat, lon: e.lon, color: fires::color(e.frp), size: fires::size(e.frp) as f32, time: e.time }).collect(),
            Data::Faults(..) | Data::Plates(..) | Data::Cables(..) => Vec::new(),
        }
    }

    /// Поля записи n окна «Объекты»: название и строки, как
    /// `_quakes_near`, `_identify_volcanoes` и `_identify_fires` модуля.
    fn feature(&self, lang: Lang, n: usize) -> Option<Feature> {
        let row = |a: &'static str, b: String| (tr(lang, a).to_string(), b);
        Some(match self {
            Data::Quakes(q) => {
                let e = q.get(n)?;
                let mut values = vec![row("Magnitude", format!("{:.1}", e.mag)), row("Focus depth", trf(lang, "{value} km", &[("value", &format!("{:.0}", e.depth))]))];
                if let Some(t) = e.time {
                    values.push(row("Time, UTC", scene::when::text(t)));
                }
                if !e.url.is_empty() {
                    values.push(row("USGS page", e.url.clone()));
                }
                (if e.place.is_empty() { format!("M {:.1}", e.mag) } else { e.place.clone() }, values)
            }
            Data::Volcanoes(v) => {
                let e = v.get(n)?;
                let mut values = Vec::new();
                if !e.kind.is_empty() {
                    values.push(row("Type", e.kind.clone()));
                }
                let last = match e.last {
                    Some(y) if y.round() <= 0.0 => trf(lang, "{year} BCE", &[("year", &format!("{:.0}", -y.round()))]),
                    Some(y) => format!("{:.0}", y.round()),
                    None => tr(lang, "date unknown").to_string(),
                };
                values.push(row("Last eruption", last));
                if let Some(h) = e.elevation {
                    values.push(row("Height", trf(lang, "{value} m", &[("value", &format!("{h:.0}"))])));
                }
                for (name, value) in [("Country", &e.country), ("Setting", &e.setting), ("Confidence", &e.evidence)] {
                    if !value.is_empty() {
                        values.push(row(name, value.clone()));
                    }
                }
                values.push(row("Dataset", "Global Volcanism Program".into()));
                if !e.number.is_empty() {
                    values.push(row("Source", format!("{}{}", volcanoes::PAGE, e.number)));
                }
                (e.name.clone(), values)
            }
            Data::Fires(f) => {
                let e = f.get(n)?;
                let mut values = vec![row("Radiative power", trf(lang, "{value} MW", &[("value", &format!("{:.1}", e.frp))]))];
                if let Some(t) = e.time {
                    values.push(row("Image time, UTC", scene::when::text(t)));
                }
                if !e.confidence.is_empty() {
                    values.push(row("Confidence", e.confidence.clone()));
                }
                values.push(row("Image", tr(lang, if e.night { "night" } else { "day" }).to_string()));
                (format!("{:.4}, {:.4}", e.lat, e.lon), values)
            }
            Data::Faults(f, _) => {
                let e = f.get(n)?;
                let mut values = vec![row("Kind of slip", tr(lang, fault_name(e.kind)).to_string())];
                if !e.slip_type.is_empty() {
                    values.push(row("Database entry", e.slip_type.clone()));
                }
                if let Some(r) = e.rate {
                    values.push(row("Slip rate", trf(lang, "{value} mm/yr", &[("value", &short(r))])));
                }
                if let Some(d) = e.dip {
                    let mut dip = format!("{}°", short(d));
                    if !e.dip_dir.is_empty() {
                        dip += &format!(" ({})", e.dip_dir);
                    }
                    values.push(row("Dip", dip));
                }
                if !e.catalog.is_empty() {
                    values.push(row("Catalogue", e.catalog.clone()));
                }
                values.push(row("Dataset", "GEM Global Active Faults".into()));
                (if e.name.is_empty() { tr(lang, "Untitled").to_string() } else { e.name.clone() }, values)
            }
            Data::Plates(b, _, _) => {
                let e = b.get(n)?;
                let name = plate_name(&e.code);
                let values = vec![
                    row("Type", if name.is_empty() { e.code.clone() } else { tr(lang, name).to_string() }),
                    row("Plates", e.pair.clone()),
                    row("Speed", trf(lang, "{value} mm/yr", &[("value", &format!("{:.0}", e.speed.round_ties_even()))])),
                ];
                (tr(lang, plate_name(plates::group(&e.code).unwrap_or(""))).to_string(), values)
            }
            Data::Cables(c, _) => {
                let e = c.get(n)?;
                let kind = if e.kind == "power" { "Power cable" } else { "Communication line" };
                let mut values = vec![row("Kind", tr(lang, kind).to_string())];
                if !e.operator.is_empty() {
                    values.push(row("Owner", e.operator.clone()));
                }
                if let Some(kv) = plates::kilovolts(&e.voltage) {
                    values.push(row("Voltage", trf(lang, "{value} kV", &[("value", &short(kv))])));
                }
                values.push(row("OSM object", format!("https://www.openstreetmap.org/{}", e.osm)));
                (if e.name.is_empty() { tr(lang, "Untitled").to_string() } else { e.name.clone() }, values)
            }
        })
    }
}
/// Слои точек на глобусе.
#[derive(Default)]
pub struct Dots {
    layers: Vec<(Layer, State)>,
    /// Набор для видеокарты изменился.
    pub dirty: bool,
}

impl Dots {
    /// Включить слой: запрос сводки, если её ещё нет.
    pub fn enable(&mut self, layer: Layer, request: Box<dyn net::Request>) {
        if self.layers.iter().any(|(l, s)| *l == layer && !matches!(s, State::Failed(_))) {
            return;
        }
        self.layers.retain(|(l, _)| *l != layer);
        request.start(layer.url());
        self.layers.push((layer, State::Asking(request)));
    }

    /// Убрать слой с глобуса.
    pub fn disable(&mut self, layer: Layer) {
        let before = self.layers.len();
        self.layers.retain(|(l, _)| *l != layer);
        self.dirty |= before != self.layers.len();
    }

    /// Включённые слои.
    pub fn keys(&self) -> Vec<Layer> {
        self.layers.iter().map(|(l, _)| *l).collect()
    }

    /// Сводка идёт.
    pub fn busy(&self) -> bool {
        self.layers.iter().any(|(_, s)| matches!(s, State::Asking(_)))
    }

    /// Принять пришедшие сводки. Возвращает текст ошибки слоя, если
    /// сводка не пришла или не разобрана.
    pub fn poll(&mut self, lang: Lang) -> Option<String> {
        let mut error = None;
        for (layer, state) in &mut self.layers {
            if let State::Asking(request) = state
                && let Some(answer) = request.take()
            {
                *state = match answer.and_then(|b| Data::parse(*layer, &b)) {
                    Ok(d) => State::Ready(d),
                    Err(e) => {
                        error = Some(trf(lang, "{layer}: not received: {error}", &[("layer", tr(lang, layer.name())), ("error", &e)]));
                        State::Failed(e)
                    }
                };
                self.dirty = true;
            }
        }
        error
    }

    /// Линии включённых слоёв линий для картинок наложения, по порядку
    /// включения.
    pub fn line_sets(&self) -> Vec<Arc<Vec<Line>>> {
        self.layers
            .iter()
            .filter_map(|(_, s)| match s {
                State::Ready(d) => d.lines().cloned(),
                _ => None,
            })
            .collect()
    }

    /// Облака точек для видеокарты: точки на рельефе height, в окне
    /// времени span.
    pub fn meshes(&self, ellipsoid: &globe_core::ellipsoid::Ellipsoid, height: &dyn Fn(f64, f64) -> f64, span: Option<(f64, f64)>) -> Vec<render::PointsMesh> {
        self.layers
            .iter()
            .filter_map(|(_, s)| match s {
                State::Ready(d) => Some(d),
                _ => None,
            })
            .filter_map(|data| {
                let dots: Vec<Dot> = data.dots().into_iter().filter(|d| quakes::in_window(d.time, span)).collect();
                if dots.is_empty() {
                    return None;
                }
                let points: Vec<DVec3> = dots.iter().map(|d| ellipsoid.geodetic_to_ecef(d.lat, d.lon, height(d.lat, d.lon))).collect();
                let center = points.iter().fold(DVec3::ZERO, |a, p| a + *p) / points.len() as f64;
                Some(render::PointsMesh {
                    center,
                    offsets: points.iter().map(|p| (*p - center).as_vec3().to_array()).collect(),
                    colors: dots.iter().map(|d| d.color).collect(),
                    size: 4.0,
                    sizes: dots.iter().map(|d| d.size).collect(),
                    dot: Some(render::Dot { alpha: ALPHA, lift: LIFT }),
                })
            })
            .collect()
    }

    /// Точки у пикселя (px, py) кадра в радиусе radius пикселей кадра -
    /// группы окна «Объекты», не больше NEAR ближайших в группе. Линия -
    /// ближайшая к точке at под пикселем в том же радиусе.
    pub fn identify(&self, lang: Lang, nav: &Navigator, at: (f64, f64), pixel: (f64, f64), radius: f64, span: Option<(f64, f64)>) -> Vec<(String, Vec<Feature>)> {
        let (px, py) = pixel;
        let height = |lat: f64, lon: f64| nav.pose.terrain.as_ref().map_or(0.0, |t| t(lat, lon));
        let mut out = Vec::new();
        for (layer, state) in &self.layers {
            let State::Ready(data) = state else {
                continue;
            };
            if let Some(lines) = data.lines() {
                // Допуск в метрах - radius пикселей кадра у точки at.
                let p = nav.camera.ellipsoid.geodetic_to_ecef(at.0, at.1, height(at.0, at.1));
                let scale = 2.0 * (nav.camera.fov_y.to_radians() / 2.0).tan() / f64::from(nav.camera.height.max(1));
                let metres = radius * (nav.camera.eye - p).length() * scale;
                if let Some(feature) = data::linesets::nearest(lines, at.0, at.1, metres).and_then(|n| data.feature(lang, n)) {
                    out.push((tr(lang, layer.group()).to_string(), vec![feature]));
                }
                continue;
            }
            let mut near: Vec<(f64, usize)> = data
                .dots()
                .iter()
                .enumerate()
                .filter(|(_, d)| quakes::in_window(d.time, span))
                .filter_map(|(n, d)| {
                    let p = nav.camera.ellipsoid.geodetic_to_ecef(d.lat, d.lon, height(d.lat, d.lon));
                    let (pixel, front) = nav.camera.project(p);
                    let facing = (nav.camera.eye - p).dot(p.normalize_or_zero()) > 0.0;
                    let gap = (pixel.x - px).hypot(pixel.y - py);
                    (front && facing && gap <= radius).then_some((gap, n))
                })
                .collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            let features: Vec<Feature> = near.iter().take(NEAR).filter_map(|(_, n)| data.feature(lang, *n)).collect();
            if !features.is_empty() {
                out.push((tr(lang, layer.name()).to_string(), features));
            }
        }
        out
    }
}
