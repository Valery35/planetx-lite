// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Слои точек каталога: землетрясения USGS, вулканы GVP, пожары FIRMS.
//! Перенос `_set_quakes`, `_place_quakes` (`ui/globe/earthdata.py`),
//! `_identify_quakes` (`ui/globe/identify.py`) и тех же частей вулканов
//! и пожаров модуля.
//!
//! Сводка просится при включении строки, точки рисуются кругами
//! с тёмным ободком на рельефе, цвет и размер - по правилам слоя. Точки
//! вне промежутка шкалы времени не видны. Щелчок «Определить объекты»
//! даёт ближайшие точки группой окна «Объекты».

use data::fires::{self, Fire};
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
    pub const ALL: [Layer; 3] = [Layer::Quakes, Layer::Volcanoes, Layer::Fires];

    pub fn key(self) -> &'static str {
        match self {
            Layer::Quakes => "quakes",
            Layer::Volcanoes => "volcanoes",
            Layer::Fires => "fires",
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
        }
    }

    pub fn attribution(self) -> (&'static str, &'static str) {
        match self {
            Layer::Quakes => quakes::ATTRIBUTION,
            Layer::Volcanoes => volcanoes::ATTRIBUTION,
            Layer::Fires => fires::ATTRIBUTION,
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
        }
    }

    /// Источник второй строкой каталога, `SOURCES` модуля.
    pub fn source(self) -> &'static str {
        match self {
            Layer::Quakes => "USGS",
            Layer::Volcanoes => "Smithsonian GVP",
            Layer::Fires => "NASA FIRMS, 24 hours",
        }
    }

    /// Название строки и группы окна «Объекты».
    pub fn name(self) -> &'static str {
        match self {
            Layer::Quakes => "Earthquakes",
            Layer::Volcanoes => "Volcanoes",
            Layer::Fires => "Fires",
        }
    }
}

/// Данные слоя.
pub enum Data {
    Quakes(Vec<Quake>),
    Volcanoes(Vec<Volcano>),
    Fires(Vec<Fire>),
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
        })
    }

    fn dots(&self) -> Vec<Dot> {
        match self {
            Data::Quakes(q) => q.iter().map(|e| Dot { lat: e.lat, lon: e.lon, color: quakes::depth_color(e.depth), size: quakes::size(e.mag) as f32, time: e.time }).collect(),
            Data::Volcanoes(v) => v
                .iter()
                .map(|e| Dot { lat: e.lat, lon: e.lon, color: volcanoes::PERIODS[volcanoes::period_of(e.last)].2, size: volcanoes::size(e.elevation) as f32, time: None })
                .collect(),
            Data::Fires(f) => f.iter().map(|e| Dot { lat: e.lat, lon: e.lon, color: fires::color(e.frp), size: fires::size(e.frp) as f32, time: e.time }).collect(),
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
    /// группы окна «Объекты», не больше NEAR ближайших в группе.
    pub fn identify(&self, lang: Lang, nav: &Navigator, px: f64, py: f64, radius: f64, span: Option<(f64, f64)>) -> Vec<(String, Vec<Feature>)> {
        let height = |lat: f64, lon: f64| nav.pose.terrain.as_ref().map_or(0.0, |t| t(lat, lon));
        let mut out = Vec::new();
        for (layer, state) in &self.layers {
            let State::Ready(data) = state else {
                continue;
            };
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
