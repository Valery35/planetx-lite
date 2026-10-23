// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Оформление как у модуля в тёмной теме (`ui/style.py`, `WINDOWS`
//! и плашки поверх вида). Просьба автора от 23 октября 2026 года -
//! «стиль поближе к Про»: окна тёмно-серые, заголовки разделов - синие
//! полосы, отмеченный флажок - синий с белой галочкой, плашки поверх
//! глобуса - тёмное стекло со скруглением.

use eframe::egui::{self, Color32, CornerRadius, Stroke};

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Фон окон.
pub const BACK: Color32 = hex(0x262a30);
/// Текст.
pub const TEXT: Color32 = hex(0xe3e7ec);
/// Поле ввода, список.
pub const FIELD: Color32 = hex(0x1d2025);
/// Рамка.
pub const BORDER: Color32 = hex(0x454b54);
/// Кнопка.
pub const BUTTON: Color32 = hex(0x353a42);
/// Кнопка под курсором.
pub const HOVER: Color32 = hex(0x3f454e);
/// Выделение, нажатое, заголовок раздела.
pub const SELECT: Color32 = hex(0x2f6fbf);
/// Приглушённый текст.
pub const MUTED: Color32 = hex(0x7f8690);
/// Акцент плашек.
pub const ACCENT: Color32 = hex(0x4aa3ff);
/// Плашка поверх глобуса и её рамка.
pub const PLATE: Color32 = Color32::from_rgba_premultiplied(25, 28, 32, 215);
pub const PLATE_BORDER: Color32 = Color32::from_rgba_premultiplied(45, 45, 45, 45);
/// Меню.
pub const MENU: Color32 = hex(0x24282e);

/// Поставить оформление: тёмная тема в цветах модуля.
pub fn apply(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BACK;
    v.window_fill = MENU;
    v.extreme_bg_color = FIELD;
    v.faint_bg_color = hex(0x22262c);
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(6);
    v.selection.bg_fill = SELECT;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.hyperlink_color = ACCENT;
    v.override_text_color = None;
    // Поля ввода и кнопки - как у модуля: рамка 1 пиксель, скругление
    // 3 пикселя, просьба автора от 23 октября 2026 года «пусть как в Про
    // будут кнопки и поля». Скругление egui по умолчанию было крупнее.
    let radius = CornerRadius::same(3);
    v.widgets.noninteractive.bg_fill = BACK;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.bg_fill = FIELD;
    v.widgets.inactive.weak_bg_fill = BUTTON;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.corner_radius = radius;
    v.widgets.hovered.bg_fill = HOVER;
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, MUTED);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.corner_radius = radius;
    v.widgets.active.bg_fill = SELECT;
    v.widgets.active.weak_bg_fill = SELECT;
    v.widgets.active.bg_stroke = Stroke::new(1.0, SELECT);
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.active.corner_radius = radius;
    v.widgets.open.bg_fill = SELECT;
    v.widgets.open.weak_bg_fill = SELECT;
    v.widgets.open.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.open.corner_radius = radius;
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.set_theme(egui::ThemePreference::Dark);
    // Отступы кнопки модуля - 10 на 3 пикселя, поля - 4 на 2.
    ctx.global_style_mut(|s| {
        s.spacing.button_padding = egui::vec2(10.0, 3.0);
        s.spacing.item_spacing = egui::vec2(6.0, 5.0);
        s.spacing.interact_size.y = 22.0;
    });
}

/// Плашка поверх глобуса: тёмное стекло, тонкая светлая рамка.
pub fn plate() -> egui::Frame {
    egui::Frame::new()
        .fill(PLATE)
        .stroke(Stroke::new(1.0, PLATE_BORDER))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::same(6))
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
    let fill = if response.hovered() { SELECT.gamma_multiply(1.15) } else { SELECT };
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

/// Значок папки: светлый контур с ушком, чтобы папку было видно среди
/// меток.
pub fn folder_icon(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
    let body = egui::Rect::from_min_max(rect.left_top() + egui::vec2(1.5, 4.5), rect.right_bottom() - egui::vec2(1.5, 2.5));
    let tab = egui::Rect::from_min_size(rect.left_top() + egui::vec2(1.5, 2.5), egui::vec2(6.0, 3.0));
    let fill = Color32::from_rgb(0xd9, 0xdd, 0xe3);
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
    let light = Color32::from_rgb(0xd9, 0xdd, 0xe3);
    // Тёмный цвет метки на тёмной панели не виден: значок светлеет.
    // Точка - ровно цвета значка на глобусе, тёмную видно по светлой
    // обводке. Линию тёмного цвета на тёмной панели не видно, она светлеет.
    let exact = color.to_opaque();
    let sum = u16::from(color.r()) + u16::from(color.g()) + u16::from(color.b());
    let color = if sum < 150 { light } else { exact };
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
            painter.add(egui::Shape::convex_polygon(vec![p(3.0, 12.0), p(7.0, 6.5), p(11.0, 12.0)], ACCENT, Stroke::NONE));
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

/// Флажок как у модуля: пустой - тёмное поле с серой рамкой, отмеченный -
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
    let painter = ui.painter();
    if *checked {
        painter.rect_filled(at, CornerRadius::same(2), SELECT);
        let a = at.left_top() + egui::vec2(side * 0.22, side * 0.52);
        let b = at.left_top() + egui::vec2(side * 0.42, side * 0.72);
        let c = at.left_top() + egui::vec2(side * 0.80, side * 0.30);
        painter.line(vec![a, b, c], Stroke::new(1.8, Color32::WHITE));
    } else {
        let border = if response.hovered() { TEXT } else { MUTED };
        painter.rect(at, CornerRadius::same(2), FIELD, Stroke::new(1.0, border), egui::StrokeKind::Inside);
    }
    response
}
