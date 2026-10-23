// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Метки открытой сцены на глобусе.
//!
//! Сцена `.planetx`, KML или KMZ читается крейтом `scene`. Видимые метки
//! становятся объектами `scene::features::Shape`: пути и многоугольники
//! садятся на рельеф и уходят в видеокарту сетками `render::FeatureMesh`,
//! точки рисует система надписей, как в модуле. Записанный тур - позы
//! камеры, его путь не рисуется.
//!
//! Сетки садятся на высоты, которые есть на момент сборки. Пришли новые
//! высоты - сетки собираются заново, не чаще раза в [`REBUILD_PAUSE`].

use globe_core::navigation::Pose;
use render::{FeatureMesh, PictureMesh};
use scene::overlays;

use crate::pictures;
use scene::features::{self, Kind, Shape};
use scene::kml::{self, Folder, Item, Place};
use scene::when::{self, Time};
use terrain::HeightStore;

/// Пауза между пересадками сеток на новые высоты, секунды.
pub const REBUILD_PAUSE: f64 = 0.5;

/// Точка метки для системы надписей: название, широта, долгота, цвет
/// подписи и цвет значка (IconStyle, без него - жёлтый, как значок
/// строки в «Моих метках»).
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub color: [u8; 4],
    pub icon: [u8; 4],
}

/// Открытый файл: дерево меток и поза камеры сцены.
pub struct Opened {
    pub tree: Folder,
    pub pose: Option<Pose>,
    pub name: String,
}

/// Открыть сцену `.planetx`, KML или KMZ из байтов файла. name - имя
/// файла, по нему различается сцена.
pub fn open(bytes: &[u8], name: &str) -> Result<Opened, String> {
    let lower = name.to_lowercase();
    if lower.ends_with(".csv") {
        // Точки CSV читает встроенный аддин в песочнице.
        let points = addin_host::Addin::load(addin_host::CSV_POINTS).and_then(|mut a| a.read_points(bytes))?;
        let mut tree = Folder::new(name);
        for (n, p) in points.iter().enumerate() {
            let label = if p.name.is_empty() { format!("{} {}", name, n + 1) } else { p.name.clone() };
            tree.children.push(Item::Place(Place::new(&label, Kind::Point, vec![(p.lat, p.lon)])));
        }
        let all: Vec<(f64, f64)> = points.iter().map(|p| (p.lat, p.lon)).collect();
        return Ok(Opened { tree, pose: fit_points(&all, 45.0, 1.6), name: name.to_string() });
    }
    if lower.ends_with(".gpkg") {
        let layers = data::layers::read(bytes.to_vec(), name)?;
        let tree = crate::gpkg::folder(layers, name);
        let points: Vec<(f64, f64)> = timed_shapes(&tree).iter().flat_map(|(s, _)| s.points.iter().copied()).collect();
        // Камера - к охвату всех объектов.
        let pose = fit_points(&points, 45.0, 1.6);
        return Ok(Opened { tree, pose, name: name.to_string() });
    }
    if lower.ends_with(".planetx") {
        let (scene, places) = scene::scene::read_scene(bytes).map_err(|e| format!("{e:?}"))?;
        let [lat, lon, distance, heading, tilt] = scene.camera;
        let tree = if places.is_empty() {
            Folder::new(&scene.name)
        } else {
            kml::read_file(&places, &scene.name).map_err(|e| format!("{e:?}"))?
        };
        let title = if scene.name.is_empty() { name.to_string() } else { scene.name.clone() };
        return Ok(Opened { tree, pose: Some(Pose::new(lat, lon, distance, heading, tilt)), name: title });
    }
    let tree = kml::read_file(bytes, name).map_err(|e| format!("{e:?}"))?;
    let pose = tree.view.map(|v| Pose::new(v[0], v[1], v[2], v[3], v[4]));
    Ok(Opened { tree, pose, name: name.to_string() })
}

/// Папка по пути номеров детей, пустой путь - корень.
fn folder_at_mut<'a>(folder: &'a mut Folder, path: &[usize]) -> Option<&'a mut Folder> {
    match path.split_first() {
        None => Some(folder),
        Some((first, rest)) => match folder.children.get_mut(*first)? {
            Item::Folder(f) => folder_at_mut(f, rest),
            _ => None,
        },
    }
}

/// Объект глобуса по метке KML.
pub fn shape_of(place: &Place) -> Shape {
    let mut s = Shape::new(place.kind, place.points.clone());
    s.color = place.color;
    s.width = place.width;
    s.fill = place.fill;
    s.name = place.name.clone();
    s.height = place.height;
    s.extrude = place.extrude;
    s.icon = place.icon.clone();
    s.alts = place.alts.clone();
    s.flow = place.flow;
    s.mode = place.mode.clone();
    s.holes = place.holes.clone();
    s.icon_scale = place.icon_scale;
    s.label_color = place.label_color;
    s.label_scale = place.label_scale;
    s.icon_heading = place.icon_heading;
    s.highlight = place.highlight.clone();
    s
}

/// Видимые метки дерева: папка со снятым флажком скрывает всё внутри.
/// Записанные туры не рисуются.
pub fn visible_shapes(folder: &Folder) -> Vec<Shape> {
    timed_shapes(folder).into_iter().map(|(s, _)| s).collect()
}

/// Видимые метки дерева с их временем.
pub fn timed_shapes(folder: &Folder) -> Vec<(Shape, Option<Time>)> {
    let mut out = Vec::new();
    collect(folder, &mut out);
    out
}

fn collect(folder: &Folder, out: &mut Vec<(Shape, Option<Time>)>) {
    for item in &folder.children {
        match item {
            Item::Folder(f) if f.visible => collect(f, out),
            Item::Place(p) if p.visible && p.tour.is_none() && !p.points.is_empty() => {
                out.push((shape_of(p), p.time.clone()));
            }
            _ => {}
        }
    }
}

/// Объекты в окне шкалы времени: метки вне окна скрыты, путь
/// с промежутком времени вырос на долю, прошедшую к концу окна. Без
/// окна - все.
pub fn in_window(shapes: &[(Shape, Option<Time>)], window: Option<(f64, f64)>) -> Vec<Shape> {
    shapes
        .iter()
        .filter(|(_, t)| window.is_none_or(|(lo, hi)| when::visible(t.as_ref(), lo, hi)))
        .map(|(s, t)| match window {
            Some((_, hi)) => features::grown(s, when::share(t.as_ref(), hi)).into_owned(),
            None => s.clone(),
        })
        .collect()
}

/// Сетка объекта для видеокарты, None у точки без стойки.
pub fn mesh(shape: &Shape, heights: &HeightStore) -> Option<FeatureMesh> {
    let geo = features::geometry(shape)?;
    let heights_at = |lats: &[f64], lons: &[f64]| -> Vec<f64> {
        let points: Vec<(f64, f64)> = lats.iter().copied().zip(lons.iter().copied()).collect();
        heights.heights_at(&points, true)
    };
    let xyz = features::vertices(&geo, shape.height, Some(&heights_at), features::altitude_mode(shape), None, 1.0);
    let (center, offsets) = features::centered(&xyz);
    Some(FeatureMesh {
        center,
        offsets,
        lines: geo.lines,
        fill: geo.triangles,
        walls: geo.wall,
        line_color: shape.color,
        fill_color: shape.fill,
        width: shape.width as f32,
    })
}

/// Ближе этого расстояния перелёт к точке без вида не подходит, метры,
/// как поиск места модуля.
pub const POINT_DISTANCE: f64 = 3000.0;

/// Поза для перелёта к метке, как `place_stop` модуля: вид метки,
/// без него точка с [`POINT_DISTANCE`], путь и многоугольник целиком
/// в кадре. None у метки без точек.
pub fn place_pose(place: &Place, fov_y: f64, aspect: f64) -> Option<Pose> {
    if let Some(v) = place.view {
        return Some(Pose::new(v[0], v[1], v[2], v[3], v[4]));
    }
    fit_points(&place.points, fov_y, aspect)
}

/// Поза над точками: одна точка - с [`POINT_DISTANCE`], несколько -
/// их охват целиком в кадре, взгляд отвесный.
pub fn fit_points(points: &[(f64, f64)], fov_y: f64, aspect: f64) -> Option<Pose> {
    match points {
        [] => None,
        [(lat, lon)] => Some(Pose::new(*lat, *lon, POINT_DISTANCE, 0.0, 0.0)),
        _ => {
            let south = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
            let north = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
            let lons: Vec<f64> = points.iter().map(|p| p.1).collect();
            let (west, east) = globe_core::flight::lon_span(&lons);
            let (lat, lon, d) = globe_core::flight::fit_view(west, south, east, north, fov_y, aspect);
            Some(Pose::new(lat, lon, d.max(POINT_DISTANCE), 0.0, 0.0))
        }
    }
}

/// Поза для перелёта к папке: её вид, без него - охват её меток.
pub fn folder_pose(folder: &Folder, fov_y: f64, aspect: f64) -> Option<Pose> {
    if let Some(v) = folder.view {
        return Some(Pose::new(v[0], v[1], v[2], v[3], v[4]));
    }
    let mut places = Vec::new();
    all_places(folder, &mut places);
    match places.as_slice() {
        [one] => place_pose(one, fov_y, aspect),
        _ => fit_points(&places.iter().flat_map(|p| p.points.iter().copied()).collect::<Vec<_>>(), fov_y, aspect),
    }
}

fn all_places<'a>(folder: &'a Folder, out: &mut Vec<&'a Place>) {
    for item in &folder.children {
        match item {
            Item::Folder(f) => all_places(f, out),
            Item::Place(p) if p.tour.is_none() => out.push(p),
            _ => {}
        }
    }
}

/// Строка дерева по пути номеров детей от корня.
pub fn item_at<'a>(folder: &'a Folder, path: &[usize]) -> Option<&'a Item> {
    let (first, rest) = path.split_first()?;
    let item = folder.children.get(*first)?;
    match (item, rest.is_empty()) {
        (_, true) => Some(item),
        (Item::Folder(f), false) => item_at(f, rest),
        _ => None,
    }
}

/// Строка дерева для правки по пути номеров детей от корня.
fn item_at_mut<'a>(folder: &'a mut Folder, path: &[usize]) -> Option<&'a mut Item> {
    let (last, parent) = path.split_last()?;
    folder_at_mut(folder, parent)?.children.get_mut(*last)
}

/// Флажок строки: видимость метки, папки или наложения.
fn visible_mut(item: &mut Item) -> &mut bool {
    match item {
        Item::Folder(f) => &mut f.visible,
        Item::Place(p) => &mut p.visible,
        Item::Overlay(o) => &mut o.visible,
    }
}

/// Папка-переключатель: включён ребёнок number - соседи гаснут.
pub fn radio_keep(folder: &mut Folder, number: usize) {
    if !folder.radio {
        return;
    }
    for (i, child) in folder.children.iter_mut().enumerate() {
        if i != number {
            *visible_mut(child) = false;
        }
    }
}

/// Что сделали в панели.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Сменился флажок строки по этому пути.
    Toggled(Vec<usize>),
    /// Двойной щелчок - перелёт к строке.
    Fly(Vec<usize>),
    /// Кнопка тура: тур по папке или записанный тур метки.
    Tour(Vec<usize>),
    /// Удалить строку.
    Remove(Vec<usize>),
    /// Начать переименование строки.
    StartRename(Vec<usize>),
    /// Новое название строки.
    Rename(Vec<usize>, String),
    /// Переименование отменено.
    CancelRename,
    /// Перетащили строку: откуда, куда, в папку или перед строкой.
    Move(Vec<usize>, Vec<usize>, bool),
    /// Сохранить строку как KML или KMZ, пустой путь - все «Мои метки».
    SaveKml(Vec<usize>),
    /// Сохранить строку сценой с текущей камерой.
    SaveScene(Vec<usize>),
    /// Очистить «Мои метки».
    Clear,
    /// Открыть файл в «Мои метки».
    Open,
}

/// Тексты меню строк дерева.
struct Words {
    fly: &'static str,
    tour: &'static str,
    rename: &'static str,
    remove: &'static str,
    save_kml: &'static str,
    save_scene: &'static str,
    open: &'static str,
    clear: &'static str,
}

fn words(lang: crate::i18n::Lang) -> Words {
    match lang {
        crate::i18n::Lang::Ru => Words {
            fly: "Подлететь",
            tour: "Запустить тур",
            rename: "Переименовать",
            remove: "Удалить",
            save_kml: "Сохранить как KML…",
            save_scene: "Сохранить сцену…",
            open: "Открыть…",
            clear: "Очистить «Мои метки»",
        },
        crate::i18n::Lang::En => Words {
            fly: "Fly to",
            tour: "Play tour",
            rename: "Rename",
            remove: "Delete",
            save_kml: "Save as KML…",
            save_scene: "Save scene…",
            open: "Open…",
            clear: "Clear My Places",
        },
    }
}

/// Название корня «Мои метки».
pub fn root_name(lang: crate::i18n::Lang) -> &'static str {
    match lang {
        crate::i18n::Lang::Ru => "Мои метки",
        crate::i18n::Lang::En => "My Places",
    }
}

/// Меню правой кнопки строки, как у модуля в минимуме: перелёт, тур,
/// удаление.
fn row_menu(response: &eframe::egui::Response, w: &Words, path: &[usize], tour: bool, actions: &mut Vec<Action>) {
    response.context_menu(|ui| {
        if ui.button(w.fly).clicked() {
            actions.push(Action::Fly(path.to_vec()));
            ui.close();
        }
        if tour && ui.button(w.tour).clicked() {
            actions.push(Action::Tour(path.to_vec()));
            ui.close();
        }
        ui.separator();
        if ui.button(w.save_kml).clicked() {
            actions.push(Action::SaveKml(path.to_vec()));
            ui.close();
        }
        if ui.button(w.save_scene).clicked() {
            actions.push(Action::SaveScene(path.to_vec()));
            ui.close();
        }
        ui.separator();
        if ui.button(w.rename).clicked() {
            actions.push(Action::StartRename(path.to_vec()));
            ui.close();
        }
        if ui.button(w.remove).clicked() {
            actions.push(Action::Remove(path.to_vec()));
            ui.close();
        }
    });
}

/// Состояние дерева в панели: выделенная строка и переименование.
#[derive(Debug, Default)]
pub struct TreeState {
    /// Выделенная строка, как текущая строка списка модуля.
    pub selected: Option<Vec<usize>>,
    /// Строка, которая переименовывается: путь и новое название.
    pub renaming: Option<(Vec<usize>, String)>,
}

/// Название строки: подпись или поле переименования. Enter или щелчок
/// мимо - новое название, Esc - отмена. Щелчок выделяет строку, двойной
/// щелчок - перелёт, правая кнопка - меню. Протяжка переносит строку:
/// строка остаётся на месте, у указателя - её название, как в списке
/// модуля. Курсор над строкой обычный, ладошка - только у глобуса.
fn row_name(
    ui: &mut eframe::egui::Ui,
    text: &str,
    path: &[usize],
    tour: bool,
    w: &Words,
    state: &mut TreeState,
    actions: &mut Vec<Action>,
) {
    use eframe::egui;
    if let Some((at, name)) = state.renaming.as_mut().filter(|(at, _)| at.as_slice() == path) {
        let edit = ui.add(egui::TextEdit::singleline(name).desired_width(ui.available_width().max(80.0)));
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            actions.push(Action::CancelRename);
        } else if edit.lost_focus() {
            actions.push(Action::Rename(at.clone(), name.clone()));
        } else if !edit.has_focus() {
            edit.request_focus();
        }
        return;
    }
    // Фон выделения - под текстом: место под него занимается до подписи.
    let background = ui.painter().add(egui::Shape::Noop);
    let label = ui.add(egui::Label::new(text).truncate().selectable(false).sense(egui::Sense::click_and_drag()));
    if label.hovered() || label.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
    }
    let selected = state.selected.as_deref() == Some(path);
    if selected {
        let rect = label.rect.expand2(egui::vec2(3.0, 1.0));
        ui.painter().set(background, egui::Shape::rect_filled(rect, 2.0, crate::theme::SELECT));
    }
    if label.clicked() || label.secondary_clicked() || label.drag_started() {
        state.selected = Some(path.to_vec());
    }
    if label.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), path.to_vec());
    }
    if label.dragged()
        && let Some(at) = ui.ctx().pointer_interact_pos()
    {
        // Название переносимой строки у указателя.
        let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("marks-drag-ghost"));
        let painter = ui.ctx().layer_painter(layer);
        let galley = painter.layout_no_wrap(text.to_string(), egui::FontId::proportional(13.0), crate::theme::TEXT);
        let rect = egui::Rect::from_min_size(at + egui::vec2(14.0, 6.0), galley.size()).expand(3.0);
        painter.rect_filled(rect, 2.0, crate::theme::PLATE);
        painter.galley(rect.min + egui::vec2(3.0, 3.0), galley, crate::theme::TEXT);
    }
    if label.double_clicked() {
        actions.push(Action::Fly(path.to_vec()));
    }
    row_menu(&label, w, path, tour, actions);
}

/// Строка принимает перетащенную: линия над строкой - перед ней,
/// рамка папки - в неё.
fn drop_target(ui: &eframe::egui::Ui, row: &eframe::egui::Response, path: &[usize], into: bool, actions: &mut Vec<Action>) {
    use eframe::egui;
    let Some(dragged) = row.dnd_hover_payload::<Vec<usize>>() else {
        return;
    };
    if path.starts_with(&dragged) {
        return;
    }
    let stroke = egui::Stroke::new(2.0, crate::theme::ACCENT);
    if into {
        ui.painter().rect_stroke(row.rect, 2.0, stroke, egui::StrokeKind::Inside);
    } else {
        ui.painter().hline(row.rect.x_range(), row.rect.top(), stroke);
    }
    if let Some(from) = row.dnd_release_payload::<Vec<usize>>() {
        actions.push(Action::Move((*from).clone(), path.to_vec(), into));
    }
}

/// Корень «Мои метки» со звездой, флажком всего дерева и меню: открыть
/// файл, тур, очистить. Внутри - дерево [`tree_ui`].
pub fn root_ui(
    ui: &mut eframe::egui::Ui,
    root: &mut Folder,
    lang: crate::i18n::Lang,
    state: &mut TreeState,
    actions: &mut Vec<Action>,
) {
    use eframe::egui;
    let w = words(lang);
    let id = ui.make_persistent_id("marks-root");
    let header = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);
    let (_, head, _) = header
        .show_header(ui, |ui| {
            if crate::theme::checkbox(ui, &mut root.visible).changed() {
                actions.push(Action::Toggled(Vec::new()));
            }
            crate::theme::star(ui);
            let label = ui.add(
                egui::Label::new(egui::RichText::new(root_name(lang)).strong()).selectable(false).sense(egui::Sense::click()),
            );
            // Курсор обычный, как над строками дерева.
            if label.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
            }
            label.context_menu(|ui| {
                if ui.button(w.open).clicked() {
                    actions.push(Action::Open);
                    ui.close();
                }
                if !root.children.is_empty() {
                    if ui.button(w.tour).clicked() {
                        actions.push(Action::Tour(Vec::new()));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(w.save_kml).clicked() {
                        actions.push(Action::SaveKml(Vec::new()));
                        ui.close();
                    }
                    if ui.button(w.save_scene).clicked() {
                        actions.push(Action::SaveScene(Vec::new()));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(w.clear).clicked() {
                        actions.push(Action::Clear);
                        ui.close();
                    }
                }
            });
        })
        .body(|ui| tree_ui(ui, root, &mut Vec::new(), lang, state, actions));
    // Брошенная на корень строка ложится в конец «Моих меток».
    drop_target(ui, &head.response, &[], true, actions);
}

/// Дерево меток в панели: флажки и названия, папки со значком
/// раскрываются, у строк - меню правой кнопки.
pub fn tree_ui(
    ui: &mut eframe::egui::Ui,
    folder: &mut Folder,
    path: &mut Vec<usize>,
    lang: crate::i18n::Lang,
    state: &mut TreeState,
    actions: &mut Vec<Action>,
) {
    use eframe::egui;
    let w = words(lang);
    for (i, item) in folder.children.iter_mut().enumerate() {
        path.push(i);
        match item {
            Item::Folder(f) => {
                let id = ui.make_persistent_id(("marks-folder", path.clone()));
                // Папка сцены раскрыта, вложенные - свёрнуты, как у модуля.
                let header = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, path.len() == 1);
                let (_, head, _) = header
                    .show_header(ui, |ui| {
                        if crate::theme::checkbox(ui, &mut f.visible).changed() {
                            actions.push(Action::Toggled(path.clone()));
                        }
                        crate::theme::folder_icon(ui);
                        row_name(ui, &f.name, path, true, &w, state, actions);
                    })
                    .body(|ui| tree_ui(ui, f, path, lang, state, actions));
                drop_target(ui, &head.response, path, true, actions);
            }
            Item::Place(p) => {
                let row = ui.horizontal(|ui| {
                    if crate::theme::checkbox(ui, &mut p.visible).changed() {
                        actions.push(Action::Toggled(path.clone()));
                    }
                    let icon = if p.tour.is_some() {
                        crate::theme::RowIcon::Tour
                    } else {
                        match p.kind {
                            Kind::Point => crate::theme::RowIcon::Point,
                            Kind::Line => crate::theme::RowIcon::Line,
                            Kind::Polygon => crate::theme::RowIcon::Polygon,
                        }
                    };
                    let rgba = |c: [u8; 4]| egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
                    crate::theme::row_icon(ui, icon, rgba(p.color), p.fill.map(rgba));
                    let name = if p.name.is_empty() { "-" } else { p.name.as_str() };
                    row_name(ui, name, path, p.tour.is_some(), &w, state, actions);
                    if p.tour.is_some() && crate::timeline::icon_button(ui, crate::timeline::Icon::Play).clicked() {
                        actions.push(Action::Tour(path.clone()));
                    }
                });
                drop_target(ui, &row.response, path, false, actions);
            }
            Item::Overlay(o) => {
                let row = ui.horizontal(|ui| {
                    if crate::theme::checkbox(ui, &mut o.visible).changed() {
                        actions.push(Action::Toggled(path.clone()));
                    }
                    let icon = match o.kind() {
                        overlays::Kind::Ground => crate::theme::RowIcon::Ground,
                        overlays::Kind::Photo => crate::theme::RowIcon::Photo,
                        overlays::Kind::Screen => crate::theme::RowIcon::Screen,
                        overlays::Kind::Model => crate::theme::RowIcon::Model,
                    };
                    crate::theme::row_icon(ui, icon, crate::theme::TEXT, None);
                    let name = if o.name.is_empty() { "-" } else { o.name.as_str() };
                    row_name(ui, name, path, false, &w, state, actions);
                });
                drop_target(ui, &row.response, path, false, actions);
            }
        }
        path.pop();
    }
}

/// Метки открытой сцены: дерево, объекты, сетки, точки.
#[derive(Default)]
pub struct Marks {
    /// Дерево меток открытого файла.
    pub tree: Option<Folder>,
    /// Видимые по флажкам метки с временем.
    timed: Vec<(Shape, Option<Time>)>,
    /// Окно шкалы времени, None - шкала закрыта.
    window: Option<(f64, f64)>,
    /// Метки кадра: видимые и в окне шкалы.
    shapes: Vec<Shape>,
    /// Видимые по флажкам наложения-картинки с временем.
    timed_overlays: Vec<(kml::Overlay, Option<Time>)>,
    /// Наложения кадра: видимые и в окне шкалы.
    overlays: Vec<kml::Overlay>,
    /// Версия высот и время последней сборки сеток.
    built: Option<(u64, f64)>,
    /// Сетки нужно собрать заново.
    dirty: bool,
    /// Название открытого файла.
    pub name: String,
}

impl Marks {
    /// Показать метки дерева tree.
    pub fn set(&mut self, tree: Folder, name: &str) {
        self.tree = Some(tree);
        self.name = name.to_string();
        self.refresh();
    }

    /// Корень «Мои метки», как у модуля: открытые сцены и файлы лежат
    /// в нём папками. root - название корня на языке интерфейса.
    pub fn root(&mut self, root: &str) -> &mut Folder {
        self.tree.get_or_insert_with(|| Folder::new(root))
    }

    /// Положить открытое дерево папкой в конец «Моих меток». Папка без
    /// названия получает название файла.
    pub fn add(&mut self, mut tree: Folder, name: &str, root: &str) {
        if tree.name.trim().is_empty() {
            tree.name = name.to_string();
        }
        self.root(root).children.push(Item::Folder(tree));
        self.name = name.to_string();
        self.refresh();
    }

    /// Последняя добавленная папка - открытая последней сцена.
    pub fn last(&self) -> Option<&Folder> {
        match self.tree.as_ref()?.children.last()? {
            Item::Folder(f) => Some(f),
            _ => None,
        }
    }

    /// Удалить строку по пути. false - строки нет.
    pub fn remove(&mut self, path: &[usize]) -> bool {
        let (Some(tree), Some((last, parent))) = (self.tree.as_mut(), path.split_last()) else {
            return false;
        };
        let Some(folder) = folder_at_mut(tree, parent) else {
            return false;
        };
        if *last >= folder.children.len() {
            return false;
        }
        folder.children.remove(*last);
        self.refresh();
        true
    }

    /// Переименовать строку по пути. Пустое название не ставится.
    pub fn rename(&mut self, path: &[usize], name: &str) -> bool {
        let name = name.trim();
        let Some(tree) = self.tree.as_mut() else {
            return false;
        };
        if name.is_empty() {
            return false;
        }
        let slot = match item_at_mut(tree, path) {
            Some(Item::Folder(f)) => &mut f.name,
            Some(Item::Place(p)) => &mut p.name,
            Some(Item::Overlay(o)) => &mut o.name,
            None => return false,
        };
        *slot = name.to_string();
        self.refresh();
        true
    }

    /// Перенести строку from: into - в конец папки to, иначе - перед
    /// строкой to. Папка не переносится в саму себя и в свою вложенную.
    pub fn move_to(&mut self, from: &[usize], to: &[usize], into: bool) -> bool {
        let Some(tree) = self.tree.as_mut() else {
            return false;
        };
        if from.is_empty() || to.starts_with(from) || item_at(tree, from).is_none() {
            return false;
        }
        // Цель проверяется до того, как строка вынута: строка не теряется.
        let target_ok = if into {
            to.is_empty() || matches!(item_at(tree, to), Some(Item::Folder(_)))
        } else {
            item_at(tree, to).is_some()
        };
        if !target_ok {
            return false;
        }
        let (last, parent) = match from.split_last() {
            Some(split) => split,
            None => return false,
        };
        let Some(folder) = folder_at_mut(tree, parent) else {
            return false;
        };
        if *last >= folder.children.len() {
            return false;
        }
        let item = folder.children.remove(*last);
        // Строки после вынутой сдвинулись на одну вверх.
        let mut to = to.to_vec();
        let depth = parent.len();
        if to.len() > depth && to[..depth] == *parent && to[depth] > *last {
            to[depth] -= 1;
        }
        let placed = if into {
            folder_at_mut(tree, &to).map(|f| f.children.push(item))
        } else {
            match to.split_last() {
                Some((index, dest)) => folder_at_mut(tree, dest).map(|f| {
                    let at = (*index).min(f.children.len());
                    f.children.insert(at, item);
                }),
                None => None,
            }
        };
        self.refresh();
        placed.is_some()
    }

    /// Что сохраняется по пути: пустой путь - все «Мои метки», папка -
    /// она сама, метка или наложение - папка с ним одним под его
    /// названием.
    pub fn export_tree(&self, path: &[usize]) -> Option<Folder> {
        let tree = self.tree.as_ref()?;
        if path.is_empty() {
            return Some(tree.clone());
        }
        let item = item_at(tree, path)?;
        let name = match item {
            Item::Folder(f) => return Some(f.clone()),
            Item::Place(p) => p.name.clone(),
            Item::Overlay(o) => o.name.clone(),
        };
        let mut folder = Folder::new(&name);
        folder.children.push(item.clone());
        Some(folder)
    }

    /// Очистить «Мои метки»: корень остаётся пустым.
    pub fn clear(&mut self) {
        if let Some(tree) = self.tree.as_mut() {
            tree.children.clear();
            tree.visible = true;
        }
        self.name.clear();
        self.refresh();
    }

    /// Объекты заново по флажкам дерева. Снятый флажок корня скрывает всё.
    pub fn refresh(&mut self) {
        let shown = self.tree.as_ref().filter(|t| t.visible);
        self.timed = shown.map(timed_shapes).unwrap_or_default();
        self.shapes = in_window(&self.timed, self.window);
        self.timed_overlays = shown.map(pictures::timed_overlays).unwrap_or_default();
        self.overlays = pictures::in_window(&self.timed_overlays, self.window);
        self.dirty = true;
        self.built = None;
    }

    /// Окно шкалы времени. Возвращает, сменились ли метки кадра.
    pub fn set_window(&mut self, window: Option<(f64, f64)>) -> bool {
        if window == self.window {
            return false;
        }
        self.window = window;
        self.shapes = in_window(&self.timed, window);
        self.overlays = pictures::in_window(&self.timed_overlays, window);
        self.dirty = true;
        true
    }

    /// Охват шкалы по временам видимых меток и наложений.
    pub fn extent(&self) -> Option<(f64, f64)> {
        when::extent(self.timed.iter().map(|(_, t)| t.as_ref()).chain(self.timed_overlays.iter().map(|(_, t)| t.as_ref())))
    }

    /// Все наложения-картинки дерева, видимые и скрытые.
    pub fn all_overlays(&self) -> Vec<&kml::Overlay> {
        self.tree.as_ref().map(Folder::overlays).unwrap_or_default()
    }

    /// Наложения кадра: видимые и в окне шкалы.
    pub fn overlays(&self) -> &[kml::Overlay] {
        &self.overlays
    }

    /// Сетки картинок кадра на поверхности и фото.
    pub fn picture_meshes(&self, heights: &HeightStore) -> Vec<PictureMesh> {
        self.overlays
            .iter()
            .filter_map(|o| {
                let key = pictures::key_of(o);
                match o.kind() {
                    overlays::Kind::Ground => pictures::ground_mesh(o, &key, heights),
                    overlays::Kind::Photo => pictures::photo_mesh(o, &key),
                    _ => None,
                }
            })
            .collect()
    }

    /// Флажок строки сменился: в папке-переключателе соседи гаснут,
    /// объекты собираются заново.
    pub fn toggled(&mut self, path: &[usize]) {
        if let (Some(tree), Some((last, parent))) = (self.tree.as_mut(), path.split_last()) {
            let checked = item_at(tree, path).is_some_and(|item| match item {
                Item::Folder(f) => f.visible,
                Item::Place(p) => p.visible,
                Item::Overlay(o) => o.visible,
            });
            if checked && let Some(folder) = folder_at_mut(tree, parent) {
                radio_keep(folder, *last);
            }
        }
        self.refresh();
    }

    /// Время строки по пути: время вида метки или её время.
    pub fn time_of(&self, path: &[usize]) -> Option<Time> {
        match item_at(self.tree.as_ref()?, path)? {
            Item::Place(p) => p.view_time.clone().or_else(|| p.time.clone()),
            Item::Overlay(o) => o.time.clone(),
            Item::Folder(_) => None,
        }
    }

    /// Остановки тура строки: папка - её отмеченные метки, метка -
    /// её записанный тур. Название тура и остановки.
    pub fn tour_of(&self, path: &[usize], fov_y: f64, aspect: f64) -> Option<(String, Vec<scene::tour::Stop>)> {
        let tree = self.tree.as_ref()?;
        if path.is_empty() {
            return Some((tree.name.clone(), crate::tours::folder_stops(tree, fov_y, aspect)));
        }
        match item_at(tree, path)? {
            Item::Folder(f) => Some((f.name.clone(), crate::tours::folder_stops(f, fov_y, aspect))),
            Item::Place(p) => Some((p.name.clone(), vec![crate::tours::recorded_stop(p)?])),
            Item::Overlay(_) => None,
        }
    }

    /// Поза для перелёта к строке по пути.
    pub fn pose_of(&self, path: &[usize], fov_y: f64, aspect: f64) -> Option<Pose> {
        match item_at(self.tree.as_ref()?, path)? {
            Item::Folder(f) => folder_pose(f, fov_y, aspect),
            Item::Place(p) => place_pose(p, fov_y, aspect),
            Item::Overlay(o) => o.view.map(|v| Pose::new(v[0], v[1], v[2], v[3], v[4])),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    /// Сетки кадра, если их пора собрать: метки сменились или пришли
    /// новые высоты и прошла пауза [`REBUILD_PAUSE`]. None - прежние
    /// сетки годны.
    pub fn meshes(&mut self, heights: &HeightStore, now: f64) -> Option<Vec<FeatureMesh>> {
        let version = heights.version();
        let due = match self.built {
            None => true,
            Some((v, at)) => v != version && now - at >= REBUILD_PAUSE,
        };
        if !(self.dirty || due) {
            return None;
        }
        self.dirty = false;
        self.built = Some((version, now));
        Some(self.shapes.iter().filter_map(|s| mesh(s, heights)).collect())
    }

    /// Точки меток для надписей.
    pub fn points(&self) -> Vec<Point> {
        self.shapes
            .iter()
            .filter(|s| s.kind == Kind::Point)
            .filter_map(|s| {
                let (lat, lon) = *s.points.first()?;
                Some(Point {
                    name: s.name.clone(),
                    lat,
                    lon,
                    color: s.label_color.unwrap_or([255, 255, 255, 245]),
                    icon: s.color,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{open, visible_shapes};
    use scene::features::Kind;

    const KML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<kml xmlns="http://www.opengis.net/kml/2.2"><Document><name>Тест</name>
<Placemark><name>Точка</name><Point><coordinates>56.2,58.0</coordinates></Point></Placemark>
<Folder><name>Скрытая</name><visibility>0</visibility>
<Placemark><name>Путь</name><LineString><coordinates>56.2,58.0 56.3,58.1</coordinates></LineString></Placemark>
</Folder>
<Placemark><name>Поле</name><Polygon><outerBoundaryIs><LinearRing><coordinates>56,58 56.1,58 56.1,58.1 56,58</coordinates></LinearRing></outerBoundaryIs></Polygon></Placemark>
</Document></kml>"#;

    #[test]
    fn hidden_folder_hides_its_marks() {
        let opened = open(KML.as_bytes(), "test.kml").unwrap();
        let shapes = visible_shapes(&opened.tree);
        let kinds: Vec<Kind> = shapes.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, [Kind::Point, Kind::Polygon]);
        assert!(opened.pose.is_none());
    }

    #[test]
    fn hidden_folder_unchecked_and_checked() {
        let mut marks = super::Marks::default();
        let opened = open(KML.as_bytes(), "test.kml").unwrap();
        marks.set(opened.tree, "test.kml");
        assert_eq!(marks.points().len(), 1);
        // Флажок скрытой папки поставлен - путь виден.
        if let Some(scene::kml::Item::Folder(f)) = marks.tree.as_mut().and_then(|t| t.children.get_mut(1)) {
            f.visible = true;
        }
        marks.toggled(&[1]);
        assert_eq!(super::visible_shapes(marks.tree.as_ref().unwrap()).len(), 3);
    }

    #[test]
    fn radio_folder_keeps_one_child() {
        let mut folder = scene::kml::Folder::new("Снимки");
        folder.radio = true;
        for n in ["a", "b", "c"] {
            folder.children.push(scene::kml::Item::Place(scene::kml::Place::new(n, Kind::Point, vec![(0.0, 0.0)])));
        }
        super::radio_keep(&mut folder, 1);
        let shown: Vec<bool> = folder
            .children
            .iter()
            .map(|i| matches!(i, scene::kml::Item::Place(p) if p.visible))
            .collect();
        assert_eq!(shown, [false, true, false]);
    }

    #[test]
    fn fly_targets() {
        let point = scene::kml::Place::new("т", Kind::Point, vec![(58.0, 56.0)]);
        let pose = super::place_pose(&point, 45.0, 1.6).unwrap();
        assert_eq!((pose.lat, pose.lon, pose.distance), (58.0, 56.0, super::POINT_DISTANCE));
        let line = scene::kml::Place::new("п", Kind::Line, vec![(58.0, 56.0), (58.5, 57.0)]);
        let pose = super::place_pose(&line, 45.0, 1.6).unwrap();
        assert!((pose.lat - 58.25).abs() < 1e-9 && pose.distance > 50_000.0);
        let mut viewed = point.clone();
        viewed.view = Some([1.0, 2.0, 3000.0, 10.0, 20.0]);
        assert_eq!(super::place_pose(&viewed, 45.0, 1.6).unwrap().heading, 10.0);
        let empty = scene::kml::Place::new("п", Kind::Line, vec![]);
        assert!(super::place_pose(&empty, 45.0, 1.6).is_none());
    }

    /// Сцены копятся папками в «Моих метках», удаление и очистка
    /// убирают их метки с глобуса, снятый флажок корня скрывает всё.
    #[test]
    fn my_places_add_remove_clear() {
        let mut marks = super::Marks::default();
        let first = open(KML.as_bytes(), "a.kml").unwrap();
        marks.add(first.tree, "a.kml", "Мои метки");
        let second = open(KML.as_bytes(), "b.kml").unwrap();
        marks.add(second.tree, "b.kml", "Мои метки");
        let root = marks.tree.as_ref().unwrap();
        assert_eq!(root.name, "Мои метки");
        assert_eq!(root.children.len(), 2);
        assert_eq!(marks.points().len(), 2);
        assert_eq!(marks.last().unwrap().name, "Тест");
        // Метка внутри второй сцены.
        assert!(marks.remove(&[1, 0]));
        assert_eq!(marks.points().len(), 1);
        assert!(!marks.remove(&[5]));
        assert!(!marks.remove(&[]));
        marks.tree.as_mut().unwrap().visible = false;
        marks.toggled(&[]);
        assert!(marks.points().is_empty());
        marks.clear();
        assert!(marks.tree.as_ref().unwrap().children.is_empty());
        assert!(marks.tree.as_ref().unwrap().visible);
        assert!(marks.points().is_empty());
    }

    fn names(folder: &scene::kml::Folder) -> Vec<String> {
        folder
            .children
            .iter()
            .map(|i| match i {
                scene::kml::Item::Folder(f) => format!("[{}]", f.name),
                scene::kml::Item::Place(p) => p.name.clone(),
                scene::kml::Item::Overlay(o) => o.name.clone(),
            })
            .collect()
    }

    /// Переименование и перетаскивание строк «Моих меток».
    #[test]
    fn rename_and_move() {
        let mut marks = super::Marks::default();
        marks.add(open(KML.as_bytes(), "a.kml").unwrap().tree, "a.kml", "Мои метки");
        marks.add(open(KML.as_bytes(), "b.kml").unwrap().tree, "b.kml", "Мои метки");
        assert!(marks.rename(&[0], "  Первая  "));
        assert!(!marks.rename(&[0], "   "));
        assert!(!marks.rename(&[9], "x"));
        let root = || marks.tree.as_ref().unwrap();
        assert_eq!(names(root()), ["[Первая]", "[Тест]"]);
        // Точка второй сцены - перед первой сценой.
        assert!(marks.move_to(&[1, 0], &[0], false));
        assert_eq!(names(marks.tree.as_ref().unwrap()), ["Точка", "[Первая]", "[Тест]"]);
        // Вторая сцена - внутрь первой, в конец.
        assert!(marks.move_to(&[2], &[1], true));
        let root = marks.tree.as_ref().unwrap();
        assert_eq!(names(root), ["Точка", "[Первая]"]);
        let Some(scene::kml::Item::Folder(first)) = root.children.get(1) else { panic!("нет папки") };
        assert_eq!(names(first).last().unwrap(), "[Тест]");
        // Папка - не в саму себя и не в свою вложенную, не в метку.
        assert!(!marks.move_to(&[1], &[1], true));
        assert!(!marks.move_to(&[1], &[1, 3], true));
        assert!(!marks.move_to(&[1], &[0], true));
        // Строка в корень - в конец «Моих меток».
        assert!(marks.move_to(&[0], &[], true));
        assert_eq!(names(marks.tree.as_ref().unwrap()), ["[Первая]", "Точка"]);
        // Вниз по тому же уровню: перед строкой после неё.
        marks.add(open(KML.as_bytes(), "c.kml").unwrap().tree, "c.kml", "Мои метки");
        assert!(marks.move_to(&[0], &[2], false));
        assert_eq!(names(marks.tree.as_ref().unwrap()), ["Точка", "[Первая]", "[Тест]"]);
    }

    #[test]
    fn broken_file_is_an_error() {
        assert!(open(b"<kml><Placemark>", "bad.kml").is_err() || visible_shapes(&open(b"<kml><Placemark>", "bad.kml").unwrap().tree).is_empty());
        assert!(open(b"PK\x03\x04garbage", "bad.planetx").is_err());
    }
}
