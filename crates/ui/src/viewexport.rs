// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Выгрузка объектов вида в glTF и 3D Tiles, перенос `ui/sceneexport.py`
//! модуля. Идут сетки вида, как они лежат в видеокарте: 3D-здания кадра,
//! подземная модель, неподвижные 3D-модели. Рельеф, снимки и анимация
//! в файл не идут, модели на треках и живые объекты тоже. Высоты -
//! с масштабом рельефа вида. Запись - `scene::sceneexport`.

use scene::sceneexport::Item;

use crate::i18n::{Lang, tr, trf};

/// Нормаль вершины зданий хранится в i8.
const NORMAL_UNIT: f32 = 127.0;

/// Кусок из сетки зданий тайла.
pub fn building_item(mesh: &data::buildings::Mesh) -> Item {
    Item {
        center: mesh.center,
        positions: mesh.vertices.iter().map(|v| v.position).collect(),
        normals: Some(mesh.vertices.iter().map(|v| [0, 1, 2].map(|i| f32::from(v.normal[i]) / NORMAL_UNIT)).collect()),
        colors: Some(mesh.vertices.iter().map(|v| v.color).collect()),
        indices: Some(mesh.indices.clone()),
        name: format!("building {}/{}/{}", mesh.key.0, mesh.key.1, mesh.key.2),
        ..Default::default()
    }
}

/// Кусок из сетки подземной модели.
pub fn sub_item(name: &str, mesh: &render::SubMesh) -> Item {
    Item {
        center: mesh.center.to_array(),
        positions: mesh.vertices.iter().map(|v| v.offset).collect(),
        normals: Some(mesh.vertices.iter().map(|v| v.normal).collect()),
        colors: Some(mesh.vertices.iter().map(|v| v.color).collect()),
        indices: Some(mesh.indices.clone()),
        name: name.to_string(),
        ..Default::default()
    }
}

/// Тайл зданий для луча 3D-линейки.
pub fn solid_tile(mesh: &data::buildings::Mesh) -> crate::tool::SolidTile {
    let center = globe_core::DVec3::from_array(mesh.center);
    let at = |i: u32| mesh.vertices.get(i as usize).map(|v| globe_core::DVec3::new(f64::from(v.position[0]), f64::from(v.position[1]), f64::from(v.position[2])));
    let triangles: Vec<scene::measure3d::Triangle> = mesh.indices.as_chunks::<3>().0.iter().filter_map(|t| Some([center + at(t[0])?, center + at(t[1])?, center + at(t[2])?])).collect();
    let radius = mesh.vertices.iter().map(|v| v.position.iter().map(|c| f64::from(*c).powi(2)).sum::<f64>()).fold(0.0, f64::max).sqrt() + 1.0;
    (center, radius, triangles)
}

/// Картинка RGBA в PNG.
fn png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let img = image::RgbaImage::from_raw(width, height, rgba.to_vec())?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// Неподвижная 3D-модель: часть с картинкой или цветом - кусок.
pub fn model_items(mesh: &render::ModelMesh, name: &str) -> Vec<Item> {
    let mut items = Vec::new();
    for part in mesh.parts.iter().filter(|p| !p.positions.is_empty()) {
        let found = part.image.as_ref().and_then(|key| mesh.images.iter().find(|i| i.0 == *key));
        let picture = found.and_then(|(_, w, h, rgba)| png(*w, *h, rgba));
        let color = part.rgba.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
        items.push(Item {
            center: mesh.center.to_array(),
            positions: part.positions.clone(),
            normals: (part.normals.len() == part.positions.len()).then(|| part.normals.clone()),
            colors: picture.is_none().then(|| vec![color; part.positions.len()]),
            uvs: picture.is_some().then(|| part.uv.clone()),
            picture,
            name: name.to_string(),
            ..Default::default()
        });
    }
    items
}

/// Сообщение после записи: масштаб рельефа вошёл в высоты.
pub fn report(lang: Lang, count: usize, scale: f64) -> String {
    if (scale - 1.0).abs() > 1e-6 {
        trf(lang, "Exported pieces - {count}. The terrain scale {scale} is in the heights.", &[("count", &count.to_string()), ("scale", &format!("{scale}"))])
    } else {
        trf(lang, "Exported pieces - {count}.", &[("count", &count.to_string())])
    }
}

/// Текст окна без объектов.
pub fn nothing(lang: Lang) -> &'static str {
    tr(lang, "There are no objects to export in the view: 3D buildings, a subsurface model or 3D models.")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn building_normals_become_units() {
        let mesh = data::buildings::Mesh {
            key: (15, 1, 2),
            center: [1.0, 2.0, 3.0],
            vertices: vec![data::subsurface::Vertex { position: [1.0, 0.0, 0.0], normal: [127, 0, 0, 0], color: [1, 2, 3, 4] }],
            indices: vec![0, 0, 0],
        };
        let item = building_item(&mesh);
        assert_eq!(item.normals.unwrap()[0], [1.0, 0.0, 0.0]);
        assert_eq!(item.colors.unwrap()[0], [1, 2, 3, 4]);
        assert_eq!(item.name, "building 15/1/2");
    }

    #[test]
    fn model_part_takes_picture_or_color() {
        let part = render::ModelPart { positions: vec![[0.0; 3]; 3], normals: vec![[0.0, 0.0, 1.0]; 3], uv: vec![[0.0; 2]; 3], image: Some("a".into()), rgba: [1.0, 0.0, 0.0, 1.0], track: None };
        let mesh = render::ModelMesh { parts: vec![part.clone(), render::ModelPart { image: None, ..part }], images: vec![("a".into(), 1, 1, vec![9, 9, 9, 255])], ..Default::default() };
        let items = model_items(&mesh, "m");
        assert!(items[0].picture.is_some() && items[0].colors.is_none() && items[0].uvs.is_some());
        assert_eq!(items[1].colors.as_ref().unwrap()[0], [255, 0, 0, 255]);
        assert!(items[1].picture.is_none() && items[1].uvs.is_none());
    }

    #[test]
    fn report_names_terrain_scale() {
        assert_eq!(report(Lang::EN, 3, 1.0), "Exported pieces - 3.");
        assert!(report(Lang::EN, 3, 2.0).contains("scale 2"));
    }
}
