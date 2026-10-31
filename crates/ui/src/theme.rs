// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Оформление как у модуля (`ui/style.py`, `WINDOWS` и плашки поверх
//! вида). Просьба автора от 23 октября 2026 года - «стиль поближе
//! к Про»: заголовки разделов - синие полосы, отмеченный флажок - синий
//! с белой галочкой, плашки поверх глобуса - стекло со скруглением.
//! Два оформления, как у модуля: «Тёмное» по умолчанию и «Светлое» -
//! светлые окна, серые плашки с тёмными значками.

use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{self, Color32, CornerRadius, Stroke};

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Цвета оформления.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// Фон окон.
    pub back: Color32,
    /// Текст.
    pub text: Color32,
    /// Поле ввода, список.
    pub field: Color32,
    pub border: Color32,
    pub button: Color32,
    pub hover: Color32,
    /// Выделение, нажатое, заголовок раздела.
    pub select: Color32,
    /// Приглушённый текст.
    pub muted: Color32,
    /// Акцент плашек.
    pub accent: Color32,
    /// Плашка поверх глобуса, её рамка и текст.
    pub plate: Color32,
    pub plate_border: Color32,
    pub plate_text: Color32,
    pub menu: Color32,
    pub faint: Color32,
    /// Значки строк и панели.
    pub icon: Color32,
}

/// «Тёмное», цвета модуля.
pub const DARK: Palette = Palette {
    dark: true,
    back: hex(0x262a30),
    text: hex(0xe3e7ec),
    field: hex(0x1d2025),
    border: hex(0x454b54),
    button: hex(0x353a42),
    hover: hex(0x3f454e),
    select: hex(0x2f6fbf),
    muted: hex(0x7f8690),
    accent: hex(0x4aa3ff),
    plate: Color32::from_rgba_premultiplied(25, 28, 32, 215),
    plate_border: Color32::from_rgba_premultiplied(45, 45, 45, 45),
    plate_text: hex(0xeceff3),
    menu: hex(0x24282e),
    faint: hex(0x22262c),
    icon: hex(0xd9dde3),
};

/// «Светлое», цвета модуля: плашки серые, а не белые.
pub const LIGHT: Palette = Palette {
    dark: false,
    back: hex(0xeef0f3),
    text: hex(0x1f2329),
    field: hex(0xffffff),
    border: hex(0xbfc5cd),
    button: hex(0xe1e5ea),
    hover: hex(0xd4d9e0),
    select: hex(0x2f6fbf),
    muted: hex(0x7b828c),
    accent: hex(0x1a68cc),
    plate: Color32::from_rgba_premultiplied(189, 192, 197, 225),
    plate_border: Color32::from_rgba_premultiplied(0, 0, 0, 45),
    plate_text: hex(0x22262c),
    menu: hex(0xe4e7eb),
    faint: hex(0xf5f6f8),
    icon: hex(0x3a3f47),
};

static LIGHT_ON: AtomicBool = AtomicBool::new(false);

/// Цвета текущего оформления.
pub fn pal() -> &'static Palette {
    if LIGHT_ON.load(Ordering::Relaxed) { &LIGHT } else { &DARK }
}

/// Поставить оформление: тёмное или светлое в цветах модуля.
pub fn apply(ctx: &egui::Context, dark: bool) {
    LIGHT_ON.store(!dark, Ordering::Relaxed);
    let p = pal();
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.back;
    v.window_fill = p.menu;
    v.extreme_bg_color = p.field;
    v.faint_bg_color = p.faint;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(6);
    v.selection.bg_fill = p.select;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.hyperlink_color = p.accent;
    v.override_text_color = None;
    // Поля ввода и кнопки - как у модуля: рамка 1 пиксель, скругление
    // 3 пикселя, просьба автора от 23 октября 2026 года «пусть как в Про
    // будут кнопки и поля». Скругление egui по умолчанию было крупнее.
    let radius = CornerRadius::same(3);
    v.widgets.noninteractive.bg_fill = p.back;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.inactive.bg_fill = p.field;
    v.widgets.inactive.weak_bg_fill = p.button;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.inactive.corner_radius = radius;
    v.widgets.hovered.bg_fill = p.hover;
    v.widgets.hovered.weak_bg_fill = p.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.muted);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.hovered.corner_radius = radius;
    v.widgets.active.bg_fill = p.select;
    v.widgets.active.weak_bg_fill = p.select;
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.select);
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.active.corner_radius = radius;
    v.widgets.open.bg_fill = p.select;
    v.widgets.open.weak_bg_fill = p.select;
    v.widgets.open.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.open.corner_radius = radius;
    let theme = if dark { egui::Theme::Dark } else { egui::Theme::Light };
    ctx.set_visuals_of(theme, v);
    ctx.set_theme(if dark { egui::ThemePreference::Dark } else { egui::ThemePreference::Light });
    // Отступы кнопки модуля - 10 на 3 пикселя, поля - 4 на 2.
    ctx.global_style_mut(|s| {
        s.spacing.button_padding = egui::vec2(10.0, 3.0);
        s.spacing.item_spacing = egui::vec2(6.0, 5.0);
        s.spacing.interact_size.y = 22.0;
    });
}

/// Плашка поверх глобуса: стекло с тонкой рамкой.
pub fn plate() -> egui::Frame {
    let p = pal();
    egui::Frame::new().fill(p.plate).stroke(Stroke::new(1.0, p.plate_border)).corner_radius(6.0).inner_margin(egui::Margin::same(6))
}

/// Заголовок раздела панели - синяя полоса во всю ширину с треугольником,
/// как у разделов «Метки» и «Слои» модуля. Щелчок сворачивает раздел.
/// Возвращает, открыт ли раздел.
pub fn section(ui: &mut egui::Ui, id: &str, title: &str) -> bool {
    let id = ui.make_persistent_id(("section", id));
    let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(id, true));
    let height = 24.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::click());
    if response.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_persisted(id, open));
    }
    let select = pal().select;
    let fill = if response.hovered() { select.gamma_multiply(1.15) } else { select };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::ZERO, fill);
    // Треугольник: вниз - открыт, вправо - свёрнут.
    let c = egui::pos2(rect.left() + 13.0, rect.center().y);
    let s = 4.0;
    let points = if open {
        vec![c + egui::vec2(-s, -s * 0.6), c + egui::vec2(s, -s * 0.6), c + egui::vec2(0.0, s * 0.7)]
    } else {
        vec![c + egui::vec2(-s * 0.6, -s), c + egui::vec2(-s * 0.6, s), c + egui::vec2(s * 0.7, 0.0)]
    };
    painter.add(egui::Shape::convex_polygon(points, Color32::WHITE, Stroke::NONE));
    painter.text(
        egui::pos2(rect.left() + 26.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::new(14.0, egui::FontFamily::Name(crate::fonts::SEMIBOLD.into())),
        Color32::WHITE,
    );
    ui.add_space(2.0);
    open
}

/// Золотая звезда корня «Мои метки», как у модуля.
pub fn star(ui: &mut egui::Ui) {
    let side = 16.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
    let c = rect.center();
    let (outer, inner) = (side * 0.5, side * 0.21);
    let at = |k: usize, r: f32| {
        let a = (k as f32 * 36.0 - 90.0).to_radians();
        c + egui::vec2(a.cos() * r, a.sin() * r)
    };
    let gold = Color32::from_rgb(0xf5, 0xc2, 0x1b);
    let painter = ui.painter();
    // Звезда невыпуклая: середина-пятиугольник и пять лучей-треугольников.
    let middle: Vec<egui::Pos2> = (0..5).map(|k| at(2 * k + 1, inner)).collect();
    painter.add(egui::Shape::convex_polygon(middle, gold, Stroke::NONE));
    for k in 0..5 {
        let tip = vec![at((2 * k + 9) % 10, inner), at(2 * k, outer), at(2 * k + 1, inner)];
        painter.add(egui::Shape::convex_polygon(tip, gold, Stroke::NONE));
    }
}

/// Часы строки «Временные метки», как значок истории у модуля.
pub fn clock_icon(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
    let c = rect.center();
    let stroke = Stroke::new(1.5, pal().icon);
    let painter = ui.painter();
    painter.circle_stroke(c, 6.0, stroke);
    painter.line_segment([c, c + egui::vec2(0.0, -4.0)], stroke);
    painter.line_segment([c, c + egui::vec2(3.0, 1.5)], stroke);
}

/// Значок папки с ушком, чтобы папку было видно среди меток.
pub fn folder_icon(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
    let body = egui::Rect::from_min_max(rect.left_top() + egui::vec2(1.5, 4.5), rect.right_bottom() - egui::vec2(1.5, 2.5));
    let tab = egui::Rect::from_min_size(rect.left_top() + egui::vec2(1.5, 2.5), egui::vec2(6.0, 3.0));
    let fill = if pal().dark { pal().icon } else { hex(0xe0b33a) };
    let painter = ui.painter();
    painter.rect_filled(tab, CornerRadius::same(1), fill);
    painter.rect_filled(body, CornerRadius::same(1), fill);
}

/// Вид строки «Моих меток» для значка, как `place_icon` модуля.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowIcon {
    Point,
    Line,
    Polygon,
    Tour,
    Ground,
    Photo,
    Screen,
    Model,
}

/// Значок строки: вид объекта цветом метки, как у модуля. color - цвет
/// линии или значка метки, fill - заливка многоугольника.
pub fn row_icon(ui: &mut egui::Ui, kind: RowIcon, color: Color32, fill: Option<Color32>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
    let p = |x: f32, y: f32| rect.left_top() + egui::vec2(x, y);
    let light = pal().icon;
    // Точка - ровно цвета значка на глобусе, её видно по обводке. Линия
    // цвета, близкого к фону панели, не видна - она берёт цвет значков.
    let exact = color.to_opaque();
    let sum = u16::from(color.r()) + u16::from(color.g()) + u16::from(color.b());
    let lost = if pal().dark { sum < 150 } else { sum > 690 };
    let color = if lost { light } else { exact };
    let line = Stroke::new(1.6, color);
    let thin = Stroke::new(1.2, light);
    let painter = ui.painter();
    match kind {
        RowIcon::Point => {
            painter.circle(p(8.0, 8.0), 4.5, exact, Stroke::new(1.0, light.gamma_multiply(0.6)));
        }
        RowIcon::Line => {
            let pts = vec![p(2.0, 12.0), p(6.0, 5.0), p(10.0, 10.0), p(14.0, 3.5)];
            painter.line(pts.clone(), line);
            for q in pts {
                painter.circle_filled(q, 1.6, color);
            }
        }
        RowIcon::Polygon => {
            let pts = vec![p(2.5, 11.5), p(5.0, 3.0), p(13.5, 4.5), p(12.0, 13.0)];
            let fill = fill.map_or(color.gamma_multiply(0.45), |f| f.to_opaque().gamma_multiply(0.8));
            painter.add(egui::Shape::convex_polygon(pts, fill, line));
        }
        RowIcon::Tour => {
            // Плёнка с кадрами.
            painter.rect_stroke(egui::Rect::from_min_max(p(2.0, 3.0), p(14.0, 13.0)), 1.0, thin, egui::StrokeKind::Inside);
            for x in [4.0, 7.0, 10.0, 13.0] {
                painter.line_segment([p(x - 1.0, 4.5), p(x, 4.5)], thin);
                painter.line_segment([p(x - 1.0, 11.5), p(x, 11.5)], thin);
            }
            painter.add(egui::Shape::convex_polygon(vec![p(6.5, 6.0), p(6.5, 10.0), p(10.0, 8.0)], light, Stroke::NONE));
        }
        RowIcon::Ground => {
            // Картинка: рамка, гора и солнце.
            painter.rect_stroke(egui::Rect::from_min_max(p(1.5, 3.0), p(14.5, 13.0)), 1.0, thin, egui::StrokeKind::Inside);
            painter.add(egui::Shape::convex_polygon(vec![p(3.0, 12.0), p(7.0, 6.5), p(11.0, 12.0)], pal().accent, Stroke::NONE));
            painter.circle_filled(p(11.5, 6.0), 1.6, Color32::from_rgb(0xf5, 0xc2, 0x1b));
        }
        RowIcon::Photo => {
            painter.rect_stroke(egui::Rect::from_min_max(p(1.5, 5.0), p(14.5, 13.5)), 1.5, thin, egui::StrokeKind::Inside);
            painter.rect_filled(egui::Rect::from_min_max(p(5.0, 3.0), p(10.0, 5.0)), 0.5, light);
            painter.circle_stroke(p(8.0, 9.2), 2.6, thin);
        }
        RowIcon::Screen => {
            painter.rect_stroke(egui::Rect::from_min_max(p(1.5, 2.5), p(14.5, 11.0)), 1.0, thin, egui::StrokeKind::Inside);
            painter.line_segment([p(8.0, 11.0), p(8.0, 13.5)], thin);
            painter.line_segment([p(5.0, 13.5), p(11.0, 13.5)], thin);
        }
        RowIcon::Model => {
            // Куб в изометрии.
            let (top, left, right) = (
                vec![p(8.0, 2.0), p(14.0, 5.0), p(8.0, 8.0), p(2.0, 5.0)],
                vec![p(2.0, 5.0), p(8.0, 8.0), p(8.0, 14.5), p(2.0, 11.5)],
                vec![p(8.0, 8.0), p(14.0, 5.0), p(14.0, 11.5), p(8.0, 14.5)],
            );
            painter.add(egui::Shape::convex_polygon(top, light, Stroke::NONE));
            painter.add(egui::Shape::convex_polygon(left, light.gamma_multiply(0.7), Stroke::NONE));
            painter.add(egui::Shape::convex_polygon(right, light.gamma_multiply(0.5), Stroke::NONE));
        }
    }
}

/// Флажок как у модуля: пустой - поле с серой рамкой, отмеченный -
/// синий квадрат с белой галочкой.
pub fn checkbox(ui: &mut egui::Ui, checked: &mut bool) -> egui::Response {
    let side = 14.0;
    let (rect, mut response) =
        ui.allocate_exact_size(egui::vec2(side + 4.0, ui.spacing().interact_size.y.max(side)), egui::Sense::click());
    if response.clicked() {
        *checked = !*checked;
        response.mark_changed();
    }
    let at = egui::Rect::from_center_size(rect.center(), egui::vec2(side, side));
    let p = pal();
    let painter = ui.painter();
    if *checked {
        painter.rect_filled(at, CornerRadius::same(2), p.select);
        let a = at.left_top() + egui::vec2(side * 0.22, side * 0.52);
        let b = at.left_top() + egui::vec2(side * 0.42, side * 0.72);
        let c = at.left_top() + egui::vec2(side * 0.80, side * 0.30);
        painter.line(vec![a, b, c], Stroke::new(1.8, Color32::WHITE));
    } else {
        let border = if response.hovered() { p.text } else { p.muted };
        painter.rect(at, CornerRadius::same(2), p.field, Stroke::new(1.0, border), egui::StrokeKind::Inside);
    }
    response
}

/// Значок панели значков, набор модуля (`ui/style.py`, `_draw`): светлые
/// линии на плашке, у светлого оформления - тёмные. Порядок вариантов -
/// порядок кнопок панели модуля (`ui/toolbar.py`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// Скрыть или показать боковую панель.
    Sidebar,
    /// Определить объекты: прицел.
    Identify,
    /// Линейка: планка с делениями наискось.
    Ruler,
    /// Новая метка: булавка, у кнопки меню.
    Place,
    /// Сохранить вид: закладка.
    Bookmark,
    /// Записать облёт: круг с красной точкой.
    Record,
    /// Шкала времени: часы.
    Time,
    /// Снимок вида: фотоаппарат.
    Camera,
    /// Копия вида в буфер обмена: два листа.
    Copy,
    /// Сцена: коробка с треугольником, у кнопки меню.
    Scene,
    /// Демо: академическая шапочка, у кнопки меню.
    Demo,
    /// Тело глобуса: планета с кольцом, у кнопки меню.
    Body,
    /// Свойства вида: шестерня.
    Settings,
    /// Во весь экран.
    Fullscreen,
    /// О программе.
    Info,
    /// Прочие значки на узком экране: три точки.
    More,
    /// Значки шкалы времени и панели тура: к началу, к концу,
    /// проигрывание, пауза, остановка, по кругу, подписи.
    Prev,
    Next,
    Play,
    Pause,
    Stop,
    Loop,
    Captions,
}

/// Сторона значка, логических пикселей, как `ICON_SIDE` модуля.
pub const ICON_SIDE: f32 = 20.0;
/// Толщина линии значка в долях клетки 24, как `STROKE` модуля.
pub const STROKE: f32 = 1.7;
/// Красная точка значка записи, `RECORD` модуля.
pub const RECORD: Color32 = Color32::from_rgb(240, 86, 72);

/// Перо значка: клетка 24 модуля, сжатая до стороны значка.
struct Pen<'a> {
    painter: &'a egui::Painter,
    origin: egui::Pos2,
    k: f32,
    color: Color32,
    /// Поворот вокруг точки (cx, cy) клетки, градусы по часовой.
    turn: Option<(f32, f32, f32)>,
}

impl Pen<'_> {
    fn at(&self, x: f32, y: f32) -> egui::Pos2 {
        let (x, y) = match self.turn {
            Some((cx, cy, deg)) => {
                let (s, c) = deg.to_radians().sin_cos();
                (cx + x * c - y * s, cy + x * s + y * c)
            }
            None => (x, y),
        };
        self.origin + egui::vec2(x * self.k, y * self.k)
    }

    fn stroke(&self) -> Stroke {
        Stroke::new(STROKE * self.k, self.color)
    }

    fn line(&self, points: &[(f32, f32)]) {
        let pts = points.iter().map(|(x, y)| self.at(*x, *y)).collect();
        self.painter.add(egui::Shape::line(pts, self.stroke()));
    }

    /// Многоугольник с заливкой цветом линии, кисть fill модуля.
    fn filled(&self, points: &[(f32, f32)]) {
        let pts = points.iter().map(|(x, y)| self.at(*x, *y)).collect();
        self.painter.add(egui::Shape::convex_polygon(pts, self.color, self.stroke()));
    }

    /// Скруглённый прямоугольник с заливкой.
    fn bar(&self, x: f32, y: f32, w: f32, h: f32, r: f32) {
        let rect = egui::Rect::from_min_max(self.at(x, y), self.at(x + w, y + h));
        self.painter.rect_filled(rect.expand(STROKE * self.k / 2.0), r * self.k, self.color);
    }

    /// Наконечник стрелки в точке (x, y), направление angle по часовой
    /// от оси x, `_arrow` модуля.
    fn arrow(&self, x: f32, y: f32, angle: f32, size: f32) {
        for side in [-1.0f32, 1.0] {
            let b = (angle + side * 150.0).to_radians();
            self.line(&[(x, y), (x + size * b.cos(), y + size * b.sin())]);
        }
    }

    fn closed(&self, points: &[(f32, f32)]) {
        let pts = points.iter().map(|(x, y)| self.at(*x, *y)).collect();
        self.painter.add(egui::Shape::closed_line(pts, self.stroke()));
    }

    fn circle(&self, x: f32, y: f32, r: f32) {
        self.painter.circle_stroke(self.at(x, y), r * self.k, self.stroke());
    }

    /// Круг с заливкой цветом линии и обводкой, как кисть fill модуля:
    /// обводка Qt наполовину снаружи круга.
    fn dot(&self, x: f32, y: f32, r: f32) {
        self.painter.circle_filled(self.at(x, y), (r + STROKE / 2.0) * self.k, self.color);
    }

    /// Дуга эллипса в прямоугольнике (x, y, w, h), углы Qt: от трёх
    /// часов против часовой стрелки, градусы.
    fn arc(&self, x: f32, y: f32, w: f32, h: f32, start: f32, span: f32) {
        let (cx, cy, rx, ry) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
        let n = ((span.abs() / 10.0).ceil() as usize).max(2);
        let pts: Vec<(f32, f32)> = (0..=n)
            .map(|i| {
                let a = (start + span * i as f32 / n as f32).to_radians();
                (cx + rx * a.cos(), cy - ry * a.sin())
            })
            .collect();
        self.line(&pts);
    }

    /// Скруглённый прямоугольник (x, y, w, h) с радиусом r.
    fn rounded(&self, x: f32, y: f32, w: f32, h: f32, r: f32) {
        let mut pts = Vec::new();
        for (cx, cy, from) in [(x + w - r, y + r, 0.0), (x + r, y + r, 90.0), (x + r, y + h - r, 180.0), (x + w - r, y + h - r, 270.0)] {
            for i in 0..=4 {
                let a = (from + 22.5 * i as f32).to_radians();
                pts.push((cx + r * a.cos(), cy - r * a.sin()));
            }
        }
        self.closed(&pts);
    }

    /// Кубическая кривая Безье из точки a в d.
    fn cubic(a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)) -> Vec<(f32, f32)> {
        (0..=12)
            .map(|i| {
                let t = i as f32 / 12.0;
                let u = 1.0 - t;
                let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
                (w[0] * a.0 + w[1] * b.0 + w[2] * c.0 + w[3] * d.0, w[0] * a.1 + w[1] * b.1 + w[2] * c.1 + w[3] * d.1)
            })
            .collect()
    }
}

/// Значок tool в квадрате rect стороны значка, как `_draw` модуля.
/// menu - треугольник меню в правом нижнем углу.
pub fn paint_tool(painter: &egui::Painter, rect: egui::Rect, tool: Tool, menu: bool, color: Color32) {
    let k = rect.width() / 24.0;
    let mut pen = Pen { painter, origin: rect.left_top(), k, color, turn: None };
    match tool {
        Tool::Sidebar => {
            pen.rounded(3.0, 5.0, 18.0, 14.0, 2.0);
            pen.line(&[(9.0, 5.0), (9.0, 19.0)]);
        }
        Tool::Identify => {
            pen.circle(12.0, 12.0, 6.5);
            for seg in [[(12.0, 2.5), (12.0, 7.5)], [(12.0, 16.5), (12.0, 21.5)], [(2.5, 12.0), (7.5, 12.0)], [(16.5, 12.0), (21.5, 12.0)]] {
                pen.line(&seg);
            }
            pen.dot(12.0, 12.0, 1.3);
        }
        Tool::Ruler => {
            pen.turn = Some((12.0, 12.0, -30.0));
            pen.rounded(-9.5, -3.5, 19.0, 7.0, 1.5);
            for (x, tall) in [(-6.0, 3.0), (-3.0, 2.0), (0.0, 3.0), (3.0, 2.0), (6.0, 3.0)] {
                pen.line(&[(x, -3.5), (x, -3.5 + tall)]);
            }
            pen.turn = None;
        }
        Tool::Place => {
            let mut pts = Pen::cubic((12.0, 21.0), (7.0, 15.0), (5.0, 12.0), (5.0, 9.5));
            // Дуга от левой точки через верх к правой, как arcTo модуля.
            pts.extend((1..=18).map(|i| {
                let a = (180.0 - 10.0 * i as f32).to_radians();
                (12.0 + 7.0 * a.cos(), 9.5 - 7.0 * a.sin())
            }));
            pts.extend(Pen::cubic((19.0, 9.5), (19.0, 12.0), (17.0, 15.0), (12.0, 21.0)).into_iter().skip(1));
            pen.line(&pts);
            pen.circle(12.0, 9.5, 2.6);
        }
        Tool::Bookmark => pen.closed(&[(6.0, 3.5), (18.0, 3.5), (18.0, 20.5), (12.0, 15.5), (6.0, 20.5)]),
        Tool::Record => {
            pen.circle(12.0, 12.0, 8.0);
            painter.circle_filled(pen.at(12.0, 12.0), 4.0 * k, RECORD);
        }
        Tool::Time => {
            pen.circle(12.0, 12.0, 8.5);
            pen.line(&[(12.0, 12.0), (12.0, 7.0)]);
            pen.line(&[(12.0, 12.0), (15.5, 14.0)]);
        }
        Tool::Camera => {
            pen.rounded(3.0, 7.0, 18.0, 12.0, 2.0);
            pen.line(&[(8.5, 7.0), (10.0, 4.5), (14.0, 4.5), (15.5, 7.0)]);
            pen.circle(12.0, 13.0, 3.6);
        }
        Tool::Copy => {
            pen.rounded(4.0, 3.0, 12.0, 14.0, 1.5);
            pen.rounded(8.0, 7.0, 12.0, 14.0, 1.5);
        }
        Tool::Scene => {
            pen.rounded(3.0, 7.0, 18.0, 13.0, 2.0);
            pen.line(&[(3.0, 10.5), (21.0, 10.5)]);
            pen.line(&[(3.0, 7.0), (5.0, 4.0), (19.0, 4.0), (21.0, 7.0)]);
            pen.closed(&[(10.0, 13.0), (10.0, 18.0), (14.5, 15.5)]);
        }
        Tool::Demo => {
            pen.closed(&[(2.5, 9.5), (12.0, 5.0), (21.5, 9.5), (12.0, 14.0)]);
            pen.line(&[(6.5, 11.5), (6.5, 16.0)]);
            pen.arc(6.5, 12.0, 11.0, 7.0, 180.0, 180.0);
            pen.line(&[(17.5, 11.5), (17.5, 15.5)]);
            pen.line(&[(21.5, 9.5), (21.5, 15.0)]);
        }
        Tool::Body => {
            pen.circle(12.0, 12.0, 5.5);
            // Кольцо наискось: дуги эллипса перед шаром и по краям.
            let ring: Vec<(f32, f32)> = (0..=24)
                .map(|i| {
                    let a = (15.0 * i as f32).to_radians();
                    let (x, y) = (10.0 * a.cos(), 3.2 * a.sin());
                    let (s, c) = (-20f32).to_radians().sin_cos();
                    (12.0 + x * c - y * s, 12.0 + x * s + y * c)
                })
                .collect();
            pen.line(&ring[..15]);
            pen.line(&ring[22..]);
        }
        Tool::Settings => {
            pen.circle(12.0, 12.0, 3.0);
            pen.circle(12.0, 12.0, 6.5);
            for i in 0..8 {
                let (s, c) = (45.0 * i as f32).to_radians().sin_cos();
                pen.line(&[(12.0 + 6.5 * c, 12.0 + 6.5 * s), (12.0 + 9.0 * c, 12.0 + 9.0 * s)]);
            }
        }
        Tool::Fullscreen => {
            for (x, y, dx, dy) in [(4.0, 4.0, 1.0, 1.0), (20.0, 4.0, -1.0, 1.0), (4.0, 20.0, 1.0, -1.0), (20.0, 20.0, -1.0, -1.0)] {
                pen.line(&[(x + 5.0 * dx, y), (x, y), (x, y + 5.0 * dy)]);
            }
        }
        Tool::Info => {
            pen.circle(12.0, 12.0, 8.5);
            pen.line(&[(12.0, 10.5), (12.0, 16.5)]);
            pen.dot(12.0, 7.6, 0.9);
        }
        Tool::More => {
            for x in [5.5, 12.0, 18.5] {
                pen.dot(x, 12.0, 1.1);
            }
        }
        Tool::Prev => {
            pen.line(&[(6.5, 6.0), (6.5, 18.0)]);
            pen.filled(&[(18.0, 6.0), (18.0, 18.0), (9.0, 12.0)]);
        }
        Tool::Next => {
            pen.line(&[(17.5, 6.0), (17.5, 18.0)]);
            pen.filled(&[(6.0, 6.0), (6.0, 18.0), (15.0, 12.0)]);
        }
        Tool::Play => pen.filled(&[(7.5, 5.0), (7.5, 19.0), (19.0, 12.0)]),
        Tool::Pause => {
            pen.bar(6.5, 5.5, 3.5, 13.0, 1.0);
            pen.bar(14.0, 5.5, 3.5, 13.0, 1.0);
        }
        Tool::Stop => pen.bar(6.5, 6.5, 11.0, 11.0, 1.5),
        Tool::Loop => {
            pen.arc(5.0, 5.0, 14.0, 14.0, 40.0, 260.0);
            pen.arrow(17.4, 7.6, 50.0, 3.5);
            pen.arc(5.0, 5.0, 14.0, 14.0, 220.0, 40.0);
            pen.arrow(6.6, 16.4, 230.0, 3.5);
        }
        Tool::Captions => {
            pen.rounded(3.0, 6.0, 18.0, 12.0, 2.0);
            pen.line(&[(6.5, 11.0), (17.5, 11.0)]);
            pen.line(&[(6.5, 14.5), (13.5, 14.5)]);
        }
    }
    if menu {
        let pts = vec![pen.at(17.5, 19.0), pen.at(23.5, 19.0), pen.at(20.5, 23.0)];
        painter.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
    }
}

/// Кнопка панели значков, как QToolButton плашки модуля: значок
/// [`ICON_SIDE`] с полем 2 пикселя, под курсором - светлая подложка,
/// включённая - синяя (`HOVER`, `CHECKED` модуля), скругление 4.
pub fn tool_button(ui: &mut egui::Ui, tool: Tool, on: bool, menu: bool, enabled: bool, tip: &str) -> egui::Response {
    // Шаг кнопок модуля на снимке - 26 логических пикселей: значок 20,
    // поле 2 и рамка кнопки Qt по пикселю.
    let side = ICON_SIDE + 6.0;
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), sense);
    let p = pal();
    let painter = ui.painter();
    if on {
        let checked = if p.dark { Color32::from_rgba_unmultiplied(74, 163, 255, 110) } else { Color32::from_rgba_unmultiplied(26, 104, 204, 70) };
        painter.rect_filled(rect, 4.0, checked);
    } else if response.hovered() && enabled {
        painter.rect_filled(rect, 4.0, if p.dark { Color32::from_white_alpha(30) } else { Color32::from_black_alpha(24) });
    }
    let color = if enabled { p.plate_text } else { p.muted };
    paint_tool(painter, rect.shrink(3.0), tool, menu, color);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
    }
    if tip.is_empty() {
        return response;
    }
    // Наведения у пальца нет: подсказку даёт долгое нажатие, кнопка
    // при этом не срабатывает.
    if response.long_touched() {
        let now = ui.input(|i| i.time);
        ui.ctx().data_mut(|d| d.insert_temp(touch_tip_id(), (tip.to_string(), now)));
    }
    response.on_hover_text(tip)
}

/// Подсказка долгого нажатия видна столько секунд.
pub const TOUCH_TIP_SECONDS: f64 = 4.0;

fn touch_tip_id() -> egui::Id {
    egui::Id::unique("planetx-touch-tip")
}

/// Подсказка последнего долгого нажатия, пока она не устарела.
pub fn touch_tip(ctx: &egui::Context) -> Option<String> {
    let now = ctx.input(|i| i.time);
    let (text, at) = ctx.data(|d| d.get_temp::<(String, f64)>(touch_tip_id()))?;
    if now - at < TOUCH_TIP_SECONDS {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(TOUCH_TIP_SECONDS - (now - at)));
        Some(text)
    } else {
        None
    }
}

/// Убрать подсказку долгого нажатия: касание в другом месте.
pub fn clear_touch_tip(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<(String, f64)>(touch_tip_id()));
}

/// Строка меню «…» узкого экрана: значок и название, как пункт меню.
/// Нажатие закрывает меню.
pub fn tool_row(ui: &mut egui::Ui, tool: Tool, on: bool, enabled: bool, name: &str, tip: &str) -> egui::Response {
    let side = ICON_SIDE + 6.0;
    let text = egui::WidgetText::from(name).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button);
    let width = (side + 6.0 + text.size().x + 10.0).max(ui.available_width());
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, side), sense);
    let visuals = ui.visuals();
    let color = if enabled { visuals.text_color() } else { visuals.weak_text_color() };
    if on {
        ui.painter().rect_filled(rect, 4.0, visuals.selection.bg_fill.gamma_multiply(0.6));
    } else if response.hovered() && enabled {
        ui.painter().rect_filled(rect, 4.0, visuals.widgets.hovered.weak_bg_fill);
    }
    let icon = egui::Rect::from_min_size(rect.min, egui::vec2(side, side));
    paint_tool(ui.painter(), icon.shrink(3.0), tool, false, color);
    let at = egui::pos2(icon.max.x + 6.0, rect.center().y - text.size().y / 2.0);
    ui.painter().galley(at, text, color);
    if response.clicked() {
        ui.close();
    }
    if tip.is_empty() { response } else { response.on_hover_text(tip) }
}

/// Значок перед подменю в меню «…», как у строк [`tool_row`].
pub fn menu_icon(ui: &mut egui::Ui, tool: Tool) {
    let side = ICON_SIDE + 6.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
    let color = ui.visuals().text_color();
    paint_tool(ui.painter(), rect.shrink(3.0), tool, false, color);
}

/// Кнопка панели или строка меню «…»: rows - строка с названием.
pub fn tool_item(ui: &mut egui::Ui, rows: bool, tool: Tool, on: bool, enabled: bool, name: &str, tip: &str) -> egui::Response {
    if rows { tool_row(ui, tool, on, enabled, name, tip) } else { tool_button(ui, tool, on, false, enabled, tip) }
}
