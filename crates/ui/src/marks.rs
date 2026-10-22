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
use render::FeatureMesh;
use scene::features::{self, Kind, Shape};
use scene::kml::{self, Folder, Item, Place};
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
    let mut out = Vec::new();
    collect(folder, &mut out);
    out
}

fn collect(folder: &Folder, out: &mut Vec<Shape>) {
    for item in &folder.children {
        match item {
            Item::Folder(f) if f.visible => collect(f, out),
            Item::Place(p) if p.visible && p.tour.is_none() && !p.points.is_empty() => out.push(shape_of(p)),
            _ => {}
        }
    }
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

/// Метки открытой сцены: объекты, сетки, точки.
#[derive(Default)]
pub struct Marks {
    shapes: Vec<Shape>,
    /// Версия высот и время последней сборки сеток.
    built: Option<(u64, f64)>,
    /// Сетки нужно собрать заново.
    dirty: bool,
    /// Название открытого файла.
    pub name: String,
}

impl Marks {
    /// Показать метки дерева tree.
    pub fn set(&mut self, tree: &Folder, name: &str) {
        self.shapes = visible_shapes(tree);
        self.name = name.to_string();
        self.dirty = true;
        self.built = None;
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
    fn broken_file_is_an_error() {
        assert!(open(b"<kml><Placemark>", "bad.kml").is_err() || visible_shapes(&open(b"<kml><Placemark>", "bad.kml").unwrap().tree).is_empty());
        assert!(open(b"PK\x03\x04garbage", "bad.planetx").is_err());
    }
}
