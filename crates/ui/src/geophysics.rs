// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Геофизика на глобусе: магнитные полюса и их след, линии равного
//! склонения и стрелки скоростей плит. Перенос `_pole_shapes`,
//! `_decl_shapes`, `_identify_magnetic` (`ui/globe/layers.py`)
//! и `_update_plate_arrows`, `_motion_rows` (`ui/globe/resources.py`)
//! модуля.
//!
//! Сводки строк приходят слоями [`crate::dots`]. Год - правый бегунок
//! открытой шкалы времени, без неё - часы компьютера. Линии склонения
//! ложатся в картинку наложения тайлов и считаются заново при смене
//! целого года. След полюсов и стрелки - линии на рельефе, как пути
//! меток. Поле до 1900 года - модель gufm1, она скачивается с сайта
//! авторов при первом таком годе. Сайт не отдаёт её странице (CORS),
//! на странице поле начинается с 1900 года.

use std::sync::Arc;

use data::geomag::{self, Combined, Gufm, Model, Poles, Source};
use data::linesets::Line;
use data::platemotion::{self, Motion};
use scene::features::{Kind, Shape};

use crate::i18n::{Lang, tr, trf};
use crate::identify::Feature;

/// Цвета и толщины, выбор помощника модуля.
const POLE_NORTH: [u8; 4] = [235, 70, 60, 255];
const POLE_SOUTH: [u8; 4] = [70, 130, 235, 255];
const POLE_WIDTH: f64 = 2.0;
const DECL_EAST: [u8; 4] = [215, 70, 60, 220];
const DECL_WEST: [u8; 4] = [60, 110, 215, 220];
const DECL_ZERO: [u8; 4] = [40, 170, 70, 255];
const DECL_WIDTH: f32 = 1.4;
const DECL_ZERO_WIDTH: f32 = 2.6;
/// Подпись у линии склонения не короче стольких точек.
const DECL_LABEL_POINTS: usize = 8;
const MOTION_COLOR: [u8; 4] = [255, 225, 60, 235];
const MOTION_WIDTH: f64 = 1.8;
/// Размер точки полюса, логических пикселей.
pub const POLE_DOT: f32 = 7.0;

/// Модель gufm1: не просилась, идёт, пришла, не пришла.
enum GufmState {
    None,
    Asking(Box<dyn net::Request>),
    Ready(Gufm),
    Failed,
}

/// Подпись на глобусе: текст, широта, долгота.
pub type Label = (String, f64, f64);

/// Линии склонения: год и наличие gufm1, линии и подписи.
struct Decl {
    key: (i64, bool),
    lines: Arc<Vec<Line>>,
    labels: Vec<Label>,
}

/// След и точки полюсов: сотые года и начало следа, объекты, точки
/// (широта, долгота, цвет) и подписи.
struct PoleMarks {
    key: (i64, Option<i64>),
    shapes: Vec<Shape>,
    points: Vec<(f64, f64, [u8; 3])>,
    labels: Vec<Label>,
}

/// Стрелки скоростей: шаг и узел вида, объекты, подписи.
struct Arrows {
    key: (i64, i64, i64),
    shapes: Vec<Shape>,
    labels: Vec<Label>,
}

/// Нарисованная геофизика и её кэш.
pub struct Geo {
    gufm: GufmState,
    /// Модель поля для расчёта: IGRF включённых строк с gufm1, если она
    /// пришла. Ключ - адрес модели IGRF и наличие gufm1.
    combined: Option<((usize, bool), Arc<Combined>)>,
    decl: Option<Decl>,
    poles: Option<PoleMarks>,
    motion: Option<Arrows>,
    /// Объекты на рельефе сменились: сетки строятся заново.
    pub shapes_dirty: bool,
    /// Линии склонения сменились: картинки наложения рисуются заново.
    pub lines_dirty: bool,
}

impl Default for Geo {
    fn default() -> Self {
        Self { gufm: GufmState::None, combined: None, decl: None, poles: None, motion: None, shapes_dirty: false, lines_dirty: false }
    }
}

/// Вид для стрелок: середина и ширина в метрах.
pub struct View {
    pub lat: f64,
    pub lon: f64,
    pub width: f64,
}

impl Geo {
    /// Пересчитать на год year (с дробью) с началом следа start,
    /// у вида view. request - новый запрос для gufm1. Возвращает текст
    /// ошибки gufm1, если она не пришла.
    pub fn update(&mut self, dots: &crate::dots::Dots, lang: Lang, year: f64, start: Option<f64>, view: &View, request: &dyn Fn() -> Box<dyn net::Request>) -> Option<String> {
        let mut message = None;
        if let GufmState::Asking(r) = &self.gufm
            && let Some(answer) = r.take()
        {
            self.gufm = match answer.and_then(|b| geomag::parse_gufm(&String::from_utf8_lossy(&b))) {
                Ok(g) => GufmState::Ready(g),
                Err(e) => {
                    message = Some(trf(lang, "The field before 1900 is not loaded: {error}", &[("error", &e)]));
                    GufmState::Failed
                }
            };
        }
        let magnetic = dots.magnetic();
        let model = magnetic.as_ref().map(|(m, _, _)| m.clone());
        let combined = model.map(|m| self.combined_for(&m));
        // Поле до первого года IGRF - gufm1, просится один раз.
        if let (Some(c), GufmState::None) = (&combined, &self.gufm)
            && year < c.igrf.first()
        {
            let r = request();
            r.start(geomag::GUFM_URL);
            self.gufm = GufmState::Asking(r);
        }
        self.update_decl(combined.as_deref().filter(|_| magnetic.as_ref().is_some_and(|m| m.2)), year);
        let poles = magnetic.as_ref().and_then(|m| m.1.clone());
        self.update_poles(lang, combined.as_deref(), poles.as_deref(), year, start);
        self.update_motion(dots.motion().as_deref(), view);
        message
    }

    /// Модель поля с gufm1, если она пришла.
    fn combined_for(&mut self, model: &Arc<Model>) -> Arc<Combined> {
        let gufm = match &self.gufm {
            GufmState::Ready(g) => Some(g.clone()),
            _ => None,
        };
        let key = (Arc::as_ptr(model) as usize, gufm.is_some());
        match &self.combined {
            Some((k, c)) if *k == key => c.clone(),
            _ => {
                let c = Arc::new(Combined { igrf: (**model).clone(), gufm });
                self.combined = Some((key, c.clone()));
                c
            }
        }
    }

    /// Линии склонения на целый год, как `_decl_shapes` модуля: до 1900
    /// года - gufm1, пока её нет - линий нет, после конца IGRF - линии
    /// её последнего года.
    fn update_decl(&mut self, model: Option<&Combined>, year: f64) {
        let Some(model) = model else {
            if self.decl.take().is_some() {
                self.lines_dirty = true;
            }
            return;
        };
        let whole = year.floor() as i64;
        let key = (whole, model.gufm.is_some());
        if self.decl.as_ref().is_some_and(|d| d.key == key) {
            return;
        }
        let year_in = (whole as f64).min(model.last());
        let mut lines = Vec::new();
        let mut labels = Vec::new();
        if let Some(coeffs) = model.at(year_in) {
            let (lats, lons, values) = geomag::declination_grid(&coeffs, model.nmax_at(year_in), geomag::GRID_STEP);
            for (level, points) in geomag::isogonic_lines(&lats, &lons, &values, geomag::ISOGONIC_STEP) {
                let (color, width) = if level == 0.0 {
                    (DECL_ZERO, DECL_ZERO_WIDTH)
                } else {
                    (if level > 0.0 { DECL_EAST } else { DECL_WEST }, DECL_WIDTH)
                };
                if points.len() >= DECL_LABEL_POINTS {
                    let (lat, lon) = points[points.len() / 2];
                    let text = if level == 0.0 { "0°".to_string() } else { format!("{:+.0}°", level) };
                    labels.push((text, lat, lon));
                }
                lines.push(Line::new(points, color, width));
            }
        }
        self.decl = Some(Decl { key, lines: Arc::new(lines), labels });
        self.lines_dirty = true;
    }

    /// След и точка каждого полюса на год, след - от start до года, как
    /// `_pole_shapes` модуля.
    fn update_poles(&mut self, lang: Lang, model: Option<&Combined>, poles: Option<&Poles>, year: f64, start: Option<f64>) {
        let (Some(model), Some(poles)) = (model, poles) else {
            if self.poles.take().is_some() {
                self.shapes_dirty = true;
            }
            return;
        };
        let key = ((year * 100.0).round() as i64, start.map(|s| (s * 100.0).round() as i64));
        if self.poles.as_ref().is_some_and(|p| p.key == key) {
            return;
        }
        let mut shapes = Vec::new();
        let mut points = Vec::new();
        let mut labels = Vec::new();
        for (track, color, name) in [(&poles.north, POLE_NORTH, "North magnetic pole, {year}"), (&poles.south, POLE_SOUTH, "South magnetic pole, {year}")] {
            let Some(first) = track.first() else {
                continue;
            };
            let year_in = year.max(first.0).min(model.last());
            let point = geomag::pole(model, track, year_in);
            let mut path = geomag::trail(track, year_in, start);
            if let Some(p) = point {
                path.push(p);
            }
            if path.len() > 1 {
                let mut line = Shape::new(Kind::Line, path);
                line.color = color;
                line.width = POLE_WIDTH;
                shapes.push(line);
            }
            if let Some((lat, lon)) = point {
                points.push((lat, lon, [color[0], color[1], color[2]]));
                labels.push((trf(lang, name, &[("year", &format!("{}", year_in.trunc()))]), lat, lon));
            }
        }
        self.poles = Some(PoleMarks { key, shapes, points, labels });
        self.shapes_dirty = true;
    }

    /// Стрелки скоростей у вида, строятся заново при смене шага узлов
    /// или узла вида, как `_update_plate_arrows` модуля.
    fn update_motion(&mut self, motion: Option<&Motion>, view: &View) {
        let Some(motion) = motion else {
            if self.motion.take().is_some() {
                self.shapes_dirty = true;
            }
            return;
        };
        let step = platemotion::step_for(view.width);
        let key = ((step * 100.0).round() as i64, (view.lat / step).round() as i64, (view.lon / step).round() as i64);
        if self.motion.as_ref().is_some_and(|m| m.key == key) {
            return;
        }
        let mut shapes = Vec::new();
        let mut labels = Vec::new();
        for arrow in platemotion::arrows(motion, view.lat, view.lon, view.width) {
            for points in arrow.lines {
                let mut line = Shape::new(Kind::Line, points);
                line.color = MOTION_COLOR;
                line.width = MOTION_WIDTH;
                shapes.push(line);
            }
            labels.push((format!("{:.0}", arrow.speed), arrow.lat, arrow.lon));
        }
        self.motion = Some(Arrows { key, shapes, labels });
        self.shapes_dirty = true;
    }

    /// Линии склонения для картинок наложения.
    pub fn line_set(&self) -> Option<Arc<Vec<Line>>> {
        self.decl.as_ref().map(|d| d.lines.clone())
    }

    /// Объекты на рельефе: след полюсов и стрелки.
    pub fn shapes(&self) -> Vec<Shape> {
        let poles = self.poles.iter().flat_map(|p| p.shapes.iter());
        let motion = self.motion.iter().flat_map(|m| m.shapes.iter());
        poles.chain(motion).cloned().collect()
    }

    /// Точки полюсов: широта, долгота, цвет.
    pub fn points(&self) -> Vec<(f64, f64, [u8; 3])> {
        self.poles.as_ref().map(|p| p.points.clone()).unwrap_or_default()
    }

    /// Подписи: полюса первыми - у полюса сходятся линии склонения и их
    /// подписи вытесняли название, потом скорости и значения склонения.
    pub fn labels(&self) -> Vec<Label> {
        let mut out: Vec<Label> = self.poles.iter().flat_map(|p| p.labels.iter().cloned()).collect();
        out.extend(self.motion.iter().flat_map(|m| m.labels.iter().cloned()));
        out.extend(self.decl.iter().flat_map(|d| d.labels.iter().cloned()));
        out
    }

    /// Группы окна «Объекты» в точке на год: поле в точке, как
    /// `_identify_magnetic`, и плита со скоростью, как `_motion_rows`
    /// модуля.
    pub fn identify(&self, dots: &crate::dots::Dots, lang: Lang, lat: f64, lon: f64, year: f64) -> Vec<(String, Vec<Feature>)> {
        let row = |a: &'static str, b: String| (tr(lang, a).to_string(), b);
        let mut out = Vec::new();
        if let Some((_, c)) = &self.combined
            && dots.magnetic().is_some()
        {
            let values = match geomag::field(c.as_ref(), lat, lon, year, 0.0) {
                None => vec![row("Year", format!("{}", year.trunc())), row("Model", tr(lang, "gufm1 from 1590, IGRF-14 from 1900 to 2030").to_string())],
                Some(f) => {
                    let name = if year < c.igrf.first() { "gufm1" } else { "IGRF-14" };
                    vec![
                        row("Year", format!("{}", year.trunc())),
                        row("Declination", format!("{:+.1}°", f.declination)),
                        row("Inclination", format!("{:+.1}°", f.inclination)),
                        row("Total intensity", trf(lang, "{value} nT", &[("value", &format!("{:.0}", f.total))])),
                        row("Horizontal component", trf(lang, "{value} nT", &[("value", &format!("{:.0}", f.horizontal))])),
                        row("Model", name.to_string()),
                    ]
                }
            };
            out.push((tr(lang, "Magnetic field").to_string(), vec![(tr(lang, "Field at the point").to_string(), values)]));
        }
        if let Some(motion) = dots.motion()
            && let Some((code, east, north)) = motion.velocity(lat, lon)
        {
            let (speed, azimuth) = platemotion::speed_azimuth(east, north);
            let name = motion.names.get(code).cloned().unwrap_or_default();
            let values = vec![
                row("Plate", format!("{name} ({code})")),
                row("Plate speed", trf(lang, "{value} mm/yr", &[("value", &format!("{speed:.1}"))])),
                row("Direction", format!("{azimuth:.0}°")),
            ];
            out.push((tr(lang, "Plate motion").to_string(), vec![(name, values)]));
        }
        out
    }
}
