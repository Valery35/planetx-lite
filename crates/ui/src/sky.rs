// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Вид звёздного неба из центра небесной сферы, перенос `show_sky`
//! и `_leave_sky` (`ui/globe/body.py`), `_render_sky` (`render/view.py`),
//! подписей `ui/skylabels.py`, карточки `DeepSkyCard` (`ui/placecard.py`)
//! и поиска в небе (`ui/globe/search.py`) модуля.
//!
//! Взгляд - [`data::skyview::SkyView`]: протяжка тянет небо за курсором,
//! колесо меняет поле зрения, двойной щелчок приближает вдвое. Кадр -
//! Млечный путь, звёзды каталога, линии созвездий и светила в системе
//! J2000. Светила пересчитываются раз в BODIES_PERIOD секунд на момент
//! шкалы времени. Подписи - светила, имена ярких звёзд, созвездия
//! и объекты глубокого неба кружком своего размера, пересекающиеся
//! подписи отбрасываются по старшинству, как у модуля. Щелчок по объекту
//! открывает его карточку. Поиск неба ищет по каталогу имена объектов,
//! звёзд и созвездий, прочее - службой имён Sesame CDS.
//!
//! Нет пока: снимков неба HiPS и плавного перелёта взгляда.

use data::deepsky::{self, DeepObject};
use data::skydata::{self, Constellations};
use data::skyview::SkyView;
use eframe::egui;
use globe_core::navigation::Pose;

use crate::i18n::{Lang, tr, trf};

/// Предел ответа файлов каталогов и Sesame.
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
/// Наименьший радиус кружка объекта, точки экрана.
const DEEP_DOT: f32 = 3.0;
/// Допуск щелчка по объекту, точки экрана.
const PICK: f32 = 8.0;
/// Поле зрения перелёта к созвездию и звезде, градусы.
const CONSTELLATION_FOV: f64 = 40.0;
const STAR_FOV: f64 = 8.0;
const DEEP_COLOR: egui::Color32 = egui::Color32::from_rgb(230, 180, 110);

/// Подпись неба: ранг, направление, текст, цвет, сдвиг от точки,
/// кружок объекта глубокого неба - номер и радиус.
struct Label {
    rank: f64,
    dir: [f64; 3],
    text: String,
    color: egui::Color32,
    shift: f32,
    spot: Option<(usize, f32)>,
}

/// Что подписывать: флажки меню «Тело».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shown {
    pub constellations: bool,
    pub deep: bool,
}

/// Загрузка: идёт или пришла.
type Load<T> = Option<Result<T, Box<dyn net::Request>>>;

/// Вид неба: взгляд, поза глобуса до входа, каталоги, карточка.
pub struct Sky {
    pub view: SkyView,
    pub globe_pose: Pose,
    constellations: Load<Constellations>,
    deep: Load<Vec<DeepObject>>,
    /// Линии созвездий уже в видеокарте.
    pub lines_sent: bool,
    /// Момент, на который посчитаны светила.
    pub bodies_at: Option<f64>,
    /// Объекты на экране прошлого кадра: номер, место, радиус.
    spots: Vec<(usize, egui::Pos2, f32)>,
    /// Карточка объекта глубокого неба: номер в каталоге.
    pub card: Option<usize>,
    /// Запрос службы имён Sesame и искомое имя.
    sesame: Option<(Box<dyn net::Request>, String)>,
    /// Имя, которого не нашёл поиск неба.
    pub not_found: Option<String>,
}

/// Начать запрос url, если его ещё не было.
fn start<T>(slot: &mut Load<T>, url: &str, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
    if slot.is_none() {
        let r = request(ANSWER_BYTES);
        r.start(url);
        *slot = Some(Err(r));
    }
}

/// Ответ запроса в данные, ошибка - пустые данные: второй раз не просится.
fn arrive<T: Default>(slot: &mut Load<T>, parse: impl Fn(&[u8]) -> Result<T, String>) {
    if let Some(Err(r)) = slot
        && let Some(answer) = r.take()
    {
        *slot = Some(Ok(answer.and_then(|b| parse(&b)).unwrap_or_default()));
    }
}

impl Sky {
    pub fn new(view: SkyView, globe_pose: Pose) -> Self {
        Self { view, globe_pose, constellations: None, deep: None, lines_sent: false, bodies_at: None, spots: Vec::new(), card: None, sesame: None, not_found: None }
    }

    /// Шаг загрузок. Возвращает новые отрезки линий созвездий: файл
    /// пришёл или флажок lines сменился, пусто - линий нет.
    pub fn step_data(&mut self, request: &dyn Fn(u64) -> Box<dyn net::Request>, lines: bool) -> Option<Vec<[f32; 3]>> {
        start(&mut self.constellations, skydata::URL, request);
        start(&mut self.deep, deepsky::URL, request);
        arrive(&mut self.constellations, skydata::parse);
        arrive(&mut self.deep, deepsky::parse);
        if let Some((r, name)) = &self.sesame
            && let Some(answer) = r.take()
        {
            match answer.ok().and_then(|b| deepsky::parse_sesame(&String::from_utf8_lossy(&b))) {
                Some((ra, dec, _)) => self.view.set(ra, dec, Some(deepsky::FOV_UNKNOWN)),
                None => self.not_found = Some(name.clone()),
            }
            self.sesame = None;
        }
        match &self.constellations {
            Some(Ok(c)) if lines && !self.lines_sent => {
                self.lines_sent = true;
                Some(skydata::segments(c).iter().map(|d| d.map(|v| v as f32)).collect())
            }
            _ if !lines && self.lines_sent => {
                self.lines_sent = false;
                Some(Vec::new())
            }
            _ => None,
        }
    }

    pub fn busy(&self) -> bool {
        !matches!(self.constellations, Some(Ok(_))) || !matches!(self.deep, Some(Ok(_))) || self.sesame.is_some()
    }

    fn deep(&self) -> &[DeepObject] {
        match &self.deep {
            Some(Ok(d)) => d,
            _ => &[],
        }
    }

    /// Ввод мыши над небом: протяжка, колесо, двойной щелчок, щелчок
    /// по объекту. ppp - пикселей кадра на логический пиксель. Возвращает,
    /// двигалось ли небо.
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
        } else if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let hit = self.spots.iter().filter(|(_, at, r)| at.distance(pos) <= r.max(PICK)).min_by(|a, b| a.1.distance(pos).total_cmp(&b.1.distance(pos)));
            if let Some((n, _, _)) = hit {
                self.card = Some(*n);
            }
        }
        moved
    }

    /// Подписи неба: светила, созвездия, имена звёзд, объекты глубокого
    /// неба кружком своего размера. Пересекающиеся отбрасываются
    /// по старшинству - меньший ранг раньше.
    pub fn labels(&mut self, painter: &egui::Painter, rect: egui::Rect, ppp: f64, lang: Lang, moment: f64, shown: Shown) {
        let (w, h) = (f64::from(rect.width()) * ppp, f64::from(rect.height()) * ppp);
        let russian = lang == Lang::RU;
        let per_pixel = self.view.radians_per_pixel(h) * ppp;
        let mut wanted: Vec<Label> = Vec::new();
        for body in skydata::bodies(moment) {
            let c = body.color.map(|v| (v * 255.0) as u8);
            wanted.push(Label { rank: 0.0, dir: body.direction, text: tr(lang, body_label(body.key)).to_string(), color: egui::Color32::from_rgb(c[0], c[1], c[2]), shift: body.size / 2.0 + 3.0, spot: None });
        }
        if let Some(Ok(c)) = &self.constellations {
            if shown.constellations {
                for n in &c.names {
                    let text = if russian && !n.ru.is_empty() { &n.ru } else { &n.en };
                    wanted.push(Label { rank: 20.0 + n.rank, dir: skydata::direction(n.ra, n.dec), text: text.to_uppercase(), color: egui::Color32::from_rgb(120, 150, 205), shift: 0.0, spot: None });
                }
            }
            for s in &c.stars {
                if s.rank > BRIGHT_NAME && self.view.fov > NAMED_FOV {
                    continue;
                }
                let text = if russian && !s.ru.is_empty() { &s.ru } else { &s.en };
                wanted.push(Label { rank: 10.0 + s.rank, dir: skydata::direction(s.ra, s.dec), text: text.clone(), color: egui::Color32::from_rgb(210, 210, 200), shift: 4.0, spot: None });
            }
        }
        if shown.deep {
            for (n, obj) in self.deep().iter().enumerate().filter(|(_, o)| deepsky::shown(o, self.view.fov)) {
                let radius = (deepsky::radius_pixels(obj, per_pixel) as f32).max(DEEP_DOT);
                wanted.push(Label { rank: deepsky::rank(obj), dir: skydata::direction(obj.ra, obj.dec), text: deepsky::title(obj, russian).to_string(), color: DEEP_COLOR, shift: radius + 2.0, spot: Some((n, radius)) });
            }
        }
        wanted.sort_by(|a, b| a.rank.total_cmp(&b.rank));
        let font = egui::FontId::proportional(LABEL_FONT);
        let mut placed: Vec<egui::Rect> = Vec::new();
        self.spots.clear();
        for Label { dir, text, color, shift, spot, .. } in wanted {
            let ([x, y], front) = self.view.project(dir, w, h);
            if !front {
                continue;
            }
            let point = rect.min + egui::vec2((x / ppp) as f32, (y / ppp) as f32);
            if !rect.contains(point) {
                continue;
            }
            // Кружок объекта виден всегда, подпись - если есть место.
            if let Some((n, radius)) = spot {
                painter.circle_stroke(point, radius, egui::Stroke::new(1.0, color));
                self.spots.push((n, point, radius));
            }
            let at = point + egui::vec2(shift, -shift);
            let galley = painter.layout_no_wrap(text, font.clone(), color);
            let r = egui::Rect::from_min_size(at, galley.size()).expand(2.0);
            if placed.iter().any(|p| p.intersects(r)) {
                continue;
            }
            placed.push(r);
            painter.galley(at, galley, color);
        }
    }

    /// Поиск неба, как у модуля: имя объекта каталога, звезды или
    /// созвездия - сразу, прочее - запросом к Sesame.
    pub fn search(&mut self, text: &str, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        let query = text.trim().to_lowercase();
        self.not_found = None;
        if query.is_empty() {
            return;
        }
        let compact = query.replace(' ', "");
        if let Some(obj) = self.deep().iter().find(|o| deepsky::names(o).iter().any(|n| n.to_lowercase() == query || n.to_lowercase().replace(' ', "") == compact)) {
            let (ra, dec, fov) = (obj.ra, obj.dec, deepsky::fov_for(obj));
            self.view.set(ra, dec, Some(fov));
            return;
        }
        if let Some(Ok(c)) = &self.constellations {
            let same = |n: &skydata::Named| n.ru.to_lowercase() == query || n.en.to_lowercase() == query;
            let found = c.names.iter().find(|n| same(n)).map(|n| (n.ra, n.dec, CONSTELLATION_FOV)).or_else(|| c.stars.iter().find(|s| same(s)).map(|s| (s.ra, s.dec, STAR_FOV)));
            if let Some((ra, dec, fov)) = found {
                self.view.set(ra, dec, Some(fov));
                return;
            }
        }
        let r = request(ANSWER_BYTES);
        r.start(&deepsky::sesame_url(text));
        self.sesame = Some((r, text.trim().to_string()));
    }

    /// Карточка объекта глубокого неба, `DeepSkyCard` модуля.
    pub fn card(&mut self, ctx: &egui::Context, lang: Lang) {
        let Some(n) = self.card else {
            return;
        };
        let Some(obj) = self.deep().get(n).cloned() else {
            self.card = None;
            return;
        };
        let russian = lang == Lang::RU;
        let mut open = true;
        let mut fly = false;
        egui::Window::new(deepsky::title(&obj, russian).to_string()).id(egui::Id::unique("planetx-deep-card")).open(&mut open).resizable(false).show(ctx, |ui| {
            let mut rows: Vec<(&str, String)> = vec![
                ("Type", tr(lang, match deepsky::kind_name(&obj.kind) { "Other" => "Other|deepsky", kind => kind }).to_string()),
                ("Designation", [obj.label.as_str(), obj.other.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ")),
                ("Constellation", obj.constellation.clone()),
            ];
            let other = if russian { &obj.en } else { &obj.ru };
            if !other.is_empty() && russian {
                rows.push(("Other name", other.clone()));
            }
            if let Some(mag) = obj.mag {
                rows.push(("Magnitude|sky", format!("{mag:.1}")));
            }
            if let Some(size) = obj.size {
                rows.push(("Size|sky", trf(lang, "{size}′", &[("size", &format!("{size:.1}"))])));
            }
            rows.push(("Right ascension", deepsky::ra_text(obj.ra)));
            rows.push(("Declination", deepsky::dec_text(obj.dec)));
            egui::Grid::new("deep-card").num_columns(2).show(ui, |ui| {
                for (name, value) in rows {
                    ui.label(tr(lang, name));
                    ui.label(value);
                    ui.end_row();
                }
            });
            ui.hyperlink_to(tr(lang, "Object in SIMBAD"), deepsky::simbad_url(&obj));
            ui.small(tr(lang, "Catalogue: OpenNGC, CC BY-SA 4.0"));
            if ui.button(tr(lang, "Fly to")).clicked() {
                fly = true;
            }
        });
        if fly {
            self.view.set(obj.ra, obj.dec, Some(deepsky::fov_for(&obj)));
        }
        if !open {
            self.card = None;
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
