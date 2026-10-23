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

/// Значок панели инструментов, набор модуля (`ui/style.py`, `_draw`):
/// светлые линии на плашке, у светлого оформления - тёмные.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// Левая панель.
    Sidebar,
    /// Открыть файл.
    Open,
    /// Демо, академическая шапочка.
    Demo,
    /// Настройки.
    Settings,
    /// Во весь экран.
    Fullscreen,
    /// О программе.
    Info,
}

/// Кнопка панели инструментов 28 × 28 со значком tool. on - кнопка
/// включена, подсвечена синим.
pub fn tool_button(ui: &mut egui::Ui, tool: Tool, on: bool, tip: &str) -> egui::Response {
    let side = 28.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
    let p = pal();
    let painter = ui.painter();
    if on {
        painter.rect_filled(rect, 4.0, p.accent.gamma_multiply(0.45));
    } else if response.hovered() {
        painter.rect_filled(rect, 4.0, if p.dark { Color32::from_white_alpha(30) } else { Color32::from_black_alpha(24) });
    }
    // Клетка значка 24 × 24 с отступом 2 пикселя, как в модуле.
    let o = rect.left_top() + egui::vec2(2.0, 2.0);
    let q = |x: f32, y: f32| o + egui::vec2(x, y);
    let s = Stroke::new(1.7, p.plate_text);
    match tool {
        Tool::Sidebar => {
            painter.rect_stroke(egui::Rect::from_min_max(q(3.0, 5.0), q(21.0, 19.0)), 2.0, s, egui::StrokeKind::Middle);
            painter.line_segment([q(9.0, 5.0), q(9.0, 19.0)], s);
        }
        Tool::Open => {
            // Папка с открытой крышкой.
            painter.line(vec![q(3.0, 18.0), q(3.0, 6.0), q(9.0, 6.0), q(11.0, 8.0), q(18.0, 8.0), q(18.0, 11.0)], s);
            painter.line(vec![q(3.0, 18.0), q(6.5, 11.0), q(21.5, 11.0), q(18.0, 18.0), q(3.0, 18.0)], s);
        }
        Tool::Demo => {
            painter.line(vec![q(2.5, 9.5), q(12.0, 5.0), q(21.5, 9.5), q(12.0, 14.0), q(2.5, 9.5)], s);
            painter.line_segment([q(6.5, 11.5), q(6.5, 16.0)], s);
            painter.line_segment([q(17.5, 11.5), q(17.5, 15.5)], s);
            painter.line(vec![q(6.5, 16.0), q(9.0, 18.0), q(12.0, 18.6), q(15.0, 18.0), q(17.5, 15.5)], s);
            painter.line_segment([q(21.5, 9.5), q(21.5, 15.0)], s);
        }
        Tool::Settings => {
            painter.circle_stroke(q(12.0, 12.0), 3.0, s);
            painter.circle_stroke(q(12.0, 12.0), 6.5, s);
            for k in 0..8 {
                let a = (45.0 * k as f32).to_radians();
                painter.line_segment([q(12.0 + 6.5 * a.cos(), 12.0 + 6.5 * a.sin()), q(12.0 + 9.0 * a.cos(), 12.0 + 9.0 * a.sin())], s);
            }
        }
        Tool::Fullscreen => {
            for (x, y, dx, dy) in [(4.0, 4.0, 1.0, 1.0), (20.0, 4.0, -1.0, 1.0), (4.0, 20.0, 1.0, -1.0), (20.0, 20.0, -1.0, -1.0)] {
                painter.line_segment([q(x, y), q(x + 5.0 * dx, y)], s);
                painter.line_segment([q(x, y), q(x, y + 5.0 * dy)], s);
            }
        }
        Tool::Info => {
            painter.circle_stroke(q(12.0, 12.0), 8.5, s);
            painter.line_segment([q(12.0, 10.5), q(12.0, 16.5)], s);
            painter.circle_filled(q(12.0, 7.6), 1.1, p.plate_text);
        }
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
    }
    response.on_hover_text(tip)
}
