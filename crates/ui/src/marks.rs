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

/// Точка метки для системы надписей: название, широта, долгота, цвет.
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub color: [u8; 4],
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
}

/// Дерево меток в панели: флажки и названия, папки раскрываются.
pub fn tree_ui(ui: &mut eframe::egui::Ui, folder: &mut Folder, path: &mut Vec<usize>, actions: &mut Vec<Action>) {
    use eframe::egui;
    for (i, item) in folder.children.iter_mut().enumerate() {
        path.push(i);
        match item {
            Item::Folder(f) => {
                let id = ui.make_persistent_id(("marks-folder", path.clone()));
                let header = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);
                let (_, head, _) = header
                    .show_header(ui, |ui| {
                        if ui.checkbox(&mut f.visible, "").changed() {
                            actions.push(Action::Toggled(path.clone()));
                        }
                        let label = ui.add(egui::Label::new(egui::RichText::new(&f.name).strong()).sense(egui::Sense::click()));
                        if label.double_clicked() {
                            actions.push(Action::Fly(path.clone()));
                        }
                        if crate::timeline::icon_button(ui, crate::timeline::Icon::Play).clicked() {
                            actions.push(Action::Tour(path.clone()));
                        }
                    })
                    .body(|ui| tree_ui(ui, f, path, actions));
                let _ = head;
            }
            Item::Place(p) => {
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut p.visible, "").changed() {
                        actions.push(Action::Toggled(path.clone()));
                    }
                    let name = if p.name.is_empty() { "-" } else { p.name.as_str() };
                    let label = ui.add(egui::Label::new(name).sense(egui::Sense::click()));
                    if label.double_clicked() {
                        actions.push(Action::Fly(path.clone()));
                    }
                    if p.tour.is_some() && crate::timeline::icon_button(ui, crate::timeline::Icon::Play).clicked() {
                        actions.push(Action::Tour(path.clone()));
                    }
                });
            }
            Item::Overlay(o) => {
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut o.visible, "").changed() {
                        actions.push(Action::Toggled(path.clone()));
                    }
                    let name = if o.name.is_empty() { "-" } else { o.name.as_str() };
                    if ui.add(egui::Label::new(name).sense(egui::Sense::click())).double_clicked() {
                        actions.push(Action::Fly(path.clone()));
                    }
                });
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

    /// Объекты заново по флажкам дерева.
    pub fn refresh(&mut self) {
        self.timed = self.tree.as_ref().map(timed_shapes).unwrap_or_default();
        self.shapes = in_window(&self.timed, self.window);
        self.timed_overlays = self.tree.as_ref().map(pictures::timed_overlays).unwrap_or_default();
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
        match item_at(self.tree.as_ref()?, path)? {
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
                Some(Point { name: s.name.clone(), lat, lon, color: s.label_color.unwrap_or([255, 255, 255, 245]) })
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

    #[test]
    fn broken_file_is_an_error() {
        assert!(open(b"<kml><Placemark>", "bad.kml").is_err() || visible_shapes(&open(b"<kml><Placemark>", "bad.kml").unwrap().tree).is_empty());
        assert!(open(b"PK\x03\x04garbage", "bad.planetx").is_err());
    }
}
