// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Вид звёздного неба из центра небесной сферы, перенос `show_sky`
//! и `_leave_sky` (`ui/globe/body.py`), `_render_sky` (`render/view.py`)
//! и подписей `ui/skylabels.py` модуля.
//!
//! Взгляд - [`data::skyview::SkyView`]: протяжка тянет небо за курсором,
//! колесо меняет поле зрения, двойной щелчок приближает вдвое. Кадр -
//! звёзды каталога, линии созвездий и светила в системе J2000. Светила
//! пересчитываются раз в BODIES_PERIOD секунд на момент шкалы времени.
//! Подписи - светила, имена ярких звёзд и созвездий, пересекающиеся
//! подписи отбрасываются по старшинству, как у модуля.
//!
//! Нет пока: Млечного пути, снимков неба HiPS, объектов глубокого неба,
//! поиска в небе.

use data::skydata::{self, Constellations};
use data::skyview::SkyView;
use eframe::egui;
use globe_core::navigation::Pose;

use crate::i18n::{Lang, tr};

/// Предел ответа файла созвездий.
const ANSWER_BYTES: u64 = 4 << 20;
/// Светила пересчитываются не чаще, секунд, как у модуля.
pub const BODIES_PERIOD: f64 = 30.0;
/// Шаг колеса: поле зрения умножается на exp(-шаг · прокрутка).
const WHEEL: f64 = 0.0025;
/// Имена звёзд ярче этой величины подписываются всегда, прочие - при
/// поле зрения уже NAMED_FOV градусов.
const BRIGHT_NAME: f64 = 1.5;
const NAMED_FOV: f64 = 40.0;
const LABEL_FONT: f32 = 12.0;

/// Вид неба: взгляд, поза глобуса до входа, созвездия.
pub struct Sky {
    pub view: SkyView,
    pub globe_pose: Pose,
    constellations: Option<Result<Constellations, Box<dyn net::Request>>>,
    /// Линии созвездий уже в видеокарте.
    pub lines_sent: bool,
    /// Момент, на который посчитаны светила.
    pub bodies_at: Option<f64>,
}

impl Sky {
    pub fn new(view: SkyView, globe_pose: Pose) -> Self {
        Self { view, globe_pose, constellations: None, lines_sent: false, bodies_at: None }
    }

    /// Шаг загрузки созвездий. Возвращает отрезки линий, когда файл пришёл.
    pub fn step_data(&mut self, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Option<Vec<[f32; 3]>> {
        if self.constellations.is_none() {
            let r = request(ANSWER_BYTES);
            r.start(skydata::URL);
            self.constellations = Some(Err(r));
        }
        if let Some(Err(r)) = &self.constellations
            && let Some(answer) = r.take()
        {
            match answer.and_then(|b| skydata::parse(&b)) {
                Ok(c) => self.constellations = Some(Ok(c)),
                // Файла нет - неба без созвездий, второй раз не просится.
                Err(_) => self.constellations = Some(Ok(Constellations::default())),
            }
        }
        match &self.constellations {
            Some(Ok(c)) if !self.lines_sent => {
                self.lines_sent = true;
                Some(skydata::segments(c).iter().map(|d| d.map(|v| v as f32)).collect())
            }
            _ => None,
        }
    }

    pub fn busy(&self) -> bool {
        matches!(self.constellations, Some(Err(_)) | None)
    }

    /// Ввод мыши над небом: протяжка, колесо, двойной щелчок. ppp -
    /// пикселей кадра на логический пиксель. Возвращает, двигалось ли небо.
    pub fn input(&mut self, ctx: &egui::Context, response: &egui::Response, ppp: f64, height: f64) -> bool {
        let mut moved = false;
        if response.dragged() {
            let d = response.drag_delta();
            if d != egui::Vec2::ZERO {
                self.view.drag(f64::from(d.x) * ppp, f64::from(d.y) * ppp, height);
                moved = true;
            }
        }
        if response.hovered() {
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.view.zoom((-WHEEL * f64::from(scroll)).exp());
                moved = true;
            }
        }
        if response.double_clicked() {
            self.view.zoom(0.5);
            moved = true;
        }
        moved
    }

    /// Подписи неба: светила, созвездия, имена звёзд. Пересекающиеся
    /// отбрасываются по старшинству - меньший ранг раньше.
    pub fn labels(&self, painter: &egui::Painter, rect: egui::Rect, ppp: f64, lang: Lang, moment: f64) {
        let (w, h) = (f64::from(rect.width()) * ppp, f64::from(rect.height()) * ppp);
        let russian = lang == Lang::RU;
        // Ранг, направление, текст, цвет, сдвиг подписи от точки.
        let mut wanted: Vec<(f64, [f64; 3], String, egui::Color32, f32)> = Vec::new();
        for body in skydata::bodies(moment) {
            let c = body.color.map(|v| (v * 255.0) as u8);
            wanted.push((0.0, body.direction, tr(lang, body_label(body.key)).to_string(), egui::Color32::from_rgb(c[0], c[1], c[2]), body.size / 2.0 + 3.0));
        }
        if let Some(Ok(c)) = &self.constellations {
            for n in &c.names {
                let text = if russian && !n.ru.is_empty() { &n.ru } else { &n.en };
                wanted.push((20.0 + n.rank, skydata::direction(n.ra, n.dec), text.to_uppercase(), egui::Color32::from_rgb(120, 150, 205), 0.0));
            }
            for s in &c.stars {
                if s.rank > BRIGHT_NAME && self.view.fov > NAMED_FOV {
                    continue;
                }
                let text = if russian && !s.ru.is_empty() { &s.ru } else { &s.en };
                wanted.push((10.0 + s.rank, skydata::direction(s.ra, s.dec), text.clone(), egui::Color32::from_rgb(210, 210, 200), 4.0));
            }
        }
        wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
        let font = egui::FontId::proportional(LABEL_FONT);
        let mut placed: Vec<egui::Rect> = Vec::new();
        for (_, dir, text, color, shift) in wanted {
            let ([x, y], front) = self.view.project(dir, w, h);
            if !front {
                continue;
            }
            let at = rect.min + egui::vec2((x / ppp) as f32 + shift, (y / ppp) as f32 - shift);
            if !rect.contains(at) {
                continue;
            }
            let galley = painter.layout_no_wrap(text, font.clone(), color);
            let r = egui::Rect::from_min_size(at, galley.size()).expand(2.0);
            if placed.iter().any(|p| p.intersects(r)) {
                continue;
            }
            placed.push(r);
            painter.galley(at, galley, color);
        }
    }
}

/// Название светила по ключу [`data::ephemeris::BODIES`], ключ перевода.
pub fn body_label(key: &str) -> &'static str {
    match key {
        "sun" => "Sun",
        "moon" => "Moon",
        "mercury" => "Mercury",
        "venus" => "Venus",
        "mars" => "Mars",
        "jupiter" => "Jupiter",
        "saturn" => "Saturn",
        "uranus" => "Uranus",
        _ => "Neptune",
    }
}

/// Светила для видеокарты на момент.
pub fn body_points(moment: f64) -> Vec<render::StarPoint> {
    skydata::bodies(moment)
        .into_iter()
        .map(|b| render::StarPoint { dir: b.direction.map(|v| v as f32), size: b.size, color: [b.color[0], b.color[1], b.color[2], 1.0] })
        .collect()
}
