// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Обзоры неба HiPS (IVOA Hierarchical Progressive Surveys), перенос
//! `core/hips.py` модуля.
//!
//! Небо нарезано сеткой HEALPix с вложенной нумерацией: 12 граней, у
//! порядка k по 4^k ячеек на грань. Тайл порядка k - картинка 512 × 512
//! ячейки HEALPix, её пиксели - ячейки порядка k + 9. Адрес тайла
//! Norder{k}/Dir{ipix / 10000 * 10000}/Npix{ipix}.jpg. Весь порядок 3 -
//! картинка Norder3/Allsky.jpg, 768 тайлов по 64 пикселя рядами по 27.
//!
//! Точка ячейки - непрерывные координаты грани x, y от 0 до 1 (xyf2loc
//! библиотеки HEALPix), направление - прямое восхождение и склонение
//! J2000 у обзоров в экваториальной системе. Номера ячеек - u64,
//! направления - f64.

use std::f64::consts::PI;

/// Пикселей на сторону тайла.
pub const TILE: u32 = 512;
pub const ALLSKY_ORDER: u32 = 3;
/// Тайлов в ряду Allsky.jpg.
pub const ALLSKY_COLUMNS: u64 = 27;
pub const MIN_ORDER: u32 = 3;
/// Сетка вершин тайла на сторону.
pub const CELL_STEPS: usize = 8;

/// Начальная строка грани и смещение долготы, как jrll и jpll HEALPix.
const JRLL: [f64; 12] = [2.0, 2.0, 2.0, 2.0, 3.0, 3.0, 3.0, 3.0, 4.0, 4.0, 4.0, 4.0];
const JPLL: [f64; 12] = [1.0, 3.0, 5.0, 7.0, 0.0, 2.0, 4.0, 6.0, 1.0, 3.0, 5.0, 7.0];

pub const CDS: &str = "https://alasky.cds.unistra.fr/";
pub const TERRAIN: &str = "https://raw.githubusercontent.com/Valery35/planetx-terrain/main/";

pub const DSS_CREDIT: &str = "DSS: STScI/NASA, CDS";
/// Подпись карты неба слоя starmap, как у модуля.
pub const STARMAP_CREDIT: &str = "NASA/Goddard SVS, Gaia DR2: ESA/Gaia/DPAC";
pub const STARMAP_LINK: &str = "https://svs.gsfc.nasa.gov/4851";
pub const DSS_TERMS: &str = "https://archive.stsci.edu/dss/acknowledging.html";

/// Слой снимков неба. base - адрес HiPS после [`CDS`] или [`TERRAIN`],
/// top - наибольший порядок, allsky - есть ли Allsky.jpg на месте
/// недостающих тайлов, low - наименьший порядок своих тайлов, far
/// и near - поле зрения в градусах, на котором слой начинает проявляться
/// и проявлен целиком.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layer {
    pub key: &'static str,
    pub host: &'static str,
    pub path: &'static str,
    pub top: u32,
    pub allsky: bool,
    pub low: u32,
    pub far: f64,
    pub near: f64,
}

impl Layer {
    /// Адрес HiPS слоя.
    pub fn base(&self) -> String {
        format!("{}{}", self.host, self.path)
    }
}

/// Решение автора модуля от 24 октября 2026 года: без выбора обзора
/// в меню. Издалека картинка Млечного пути со звёздами каталога, ближе
/// карта NASA SVS Deep Star Maps 2020, совсем вблизи DSS - только его
/// загруженные тайлы, на месте прочих видна карта.
pub const LAYERS: [Layer; 2] = [
    Layer { key: "starmap", host: TERRAIN, path: "sky/starmap/", top: 4, allsky: true, low: 3, far: 50.0, near: 35.0 },
    Layer { key: "dss", host: CDS, path: "DSS/DSSColor/", top: 9, allsky: false, low: 5, far: 6.0, near: 3.0 },
];

/// Доля проявления слоя при поле зрения fov, от 0 до 1, плавно.
pub fn share(layer: &Layer, fov: f64) -> f64 {
    if fov >= layer.far {
        return 0.0;
    }
    if fov <= layer.near {
        return 1.0;
    }
    let t = (layer.far - fov) / (layer.far - layer.near);
    t * t * (3.0 - 2.0 * t)
}

pub fn tile_url(base: &str, order: u32, ipix: u64) -> String {
    format!("{base}Norder{order}/Dir{}/Npix{ipix}.jpg", ipix / 10000 * 10000)
}

pub fn allsky_url(base: &str) -> String {
    format!("{base}Norder{ALLSKY_ORDER}/Allsky.jpg")
}

/// Чётные биты числа подряд - обратное чередованию битов.
fn compress(value: u64) -> u64 {
    (0..32).fold(0, |out, bit| out | (((value >> (2 * bit)) & 1) << bit))
}

/// Ячейка вложенной нумерации: грань и номер ix, iy в грани.
pub fn nest_xyf(order: u32, ipix: u64) -> (usize, u64, u64) {
    let face = (ipix >> (2 * order)) as usize;
    let inner = ipix & ((1u64 << (2 * order)) - 1);
    (face, compress(inner), compress(inner >> 1))
}

/// Направление точки грани face с координатами x, y от 0 до 1.
pub fn face_direction(face: usize, x: f64, y: f64) -> [f64; 3] {
    let face = face.min(11);
    let jr = JRLL[face] - x - y;
    let (nr, z) = if jr < 1.0 {
        (jr, 1.0 - jr * jr / 3.0)
    } else if jr > 3.0 {
        let nr = 4.0 - jr;
        (nr, nr * nr / 3.0 - 1.0)
    } else {
        (1.0, (2.0 - jr) * 2.0 / 3.0)
    };
    let tmp = (JPLL[face] * nr + x - y).rem_euclid(8.0);
    let phi = if nr < 1e-15 { 0.0 } else { PI / 4.0 * tmp / nr };
    let st = (1.0 - z * z).max(0.0).sqrt();
    [st * phi.cos(), st * phi.sin(), z]
}

/// Доли x, y грани внутри тайла для пикселя col, row картинки (ряд
/// сверху): x растёт вниз по рядам, y - вправо по столбцам. Модуль
/// сверил 24 октября 2026 года по ярким звёздам на тайлах DSS порядка 3 -
/// из восьми поворотов и отражений только этот ставит Бетельгейзе, Вегу,
/// Арктур и Капеллу ближе 0.05° к каталогу.
pub fn pixel_face(col: f64, row: f64, size: f64) -> (f64, f64) {
    (row / size, col / size)
}

/// Вершины тайла: направления и доли картинки u, v ((steps+1)² штук),
/// v сверху вниз. Треугольники - [`mesh_indices`].
pub fn tile_mesh(order: u32, ipix: u64, steps: usize) -> (Vec<[f64; 3]>, Vec<[f64; 2]>) {
    let (face, ix, iy) = nest_xyf(order, ipix);
    let side = (1u64 << order) as f64;
    let n = steps.max(1);
    let mut dirs = Vec::with_capacity((n + 1) * (n + 1));
    let mut uv = Vec::with_capacity((n + 1) * (n + 1));
    for r in 0..=n {
        for c in 0..=n {
            let (u, v) = (c as f64 / n as f64, r as f64 / n as f64);
            let (fx, fy) = pixel_face(u * f64::from(TILE), v * f64::from(TILE), f64::from(TILE));
            dirs.push(face_direction(face, (ix as f64 + fx) / side, (iy as f64 + fy) / side));
            uv.push([u, v]);
        }
    }
    (dirs, uv)
}

/// Треугольники сетки (steps+1)² вершин.
pub fn mesh_indices(steps: usize) -> Vec<u32> {
    let n = (steps + 1) as u32;
    let mut out = Vec::with_capacity(steps * steps * 6);
    for r in 0..steps as u32 {
        for c in 0..steps as u32 {
            let i = r * n + c;
            out.extend([i, i + 1, i + n, i + 1, i + n + 1, i + n]);
        }
    }
    out
}

/// Сторона ячейки порядка order, радианы.
pub fn cell_size(order: u32) -> f64 {
    (4.0 * PI / (12.0 * 4f64.powi(order as i32))).sqrt()
}

/// Порядок тайлов, у которых пиксель не крупнее полутора пикселей
/// экрана. None - хватает Allsky.jpg порядка 3 по 64 пикселя.
pub fn order_for(radians_per_pixel: f64, top: u32) -> Option<u32> {
    let allsky = cell_size(ALLSKY_ORDER) / f64::from(TILE / 8);
    if allsky <= radians_per_pixel * 1.5 {
        return None;
    }
    Some((MIN_ORDER..=top).find(|o| cell_size(*o) / f64::from(TILE) <= radians_per_pixel * 1.5).unwrap_or(top))
}

/// Направление середины ячейки.
pub fn center(order: u32, ipix: u64) -> [f64; 3] {
    let (face, ix, iy) = nest_xyf(order, ipix);
    let side = (1u64 << order) as f64;
    face_direction(face, (ix as f64 + 0.5) / side, (iy as f64 + 0.5) / side)
}

/// Ячейки порядка order, задевающие конус вокруг направления axis
/// с полууглом reach в радианах. Спуск от порядка 0 по детям.
pub fn visible(order: u32, axis: [f64; 3], reach: f64) -> Vec<u64> {
    let mut cells: Vec<u64> = (0..12).collect();
    for level in 0..=order {
        if level > 0 {
            cells = cells.iter().flat_map(|c| (0..4).map(move |k| c * 4 + k)).collect();
        }
        // Ячейка вписана в круг радиусом около стороны.
        let radius = cell_size(level) * if level < 2 { 1.6 } else { 1.0 };
        let limit = (reach + radius).min(PI).cos();
        cells.retain(|c| {
            let d = center(level, *c);
            d[0] * axis[0] + d[1] * axis[1] + d[2] * axis[2] >= limit
        });
    }
    cells
}

/// Доли Allsky.jpg под тайлом порядка 3: u0, v0, du, dv.
pub fn allsky_rect(ipix: u64, width: f64, height: f64) -> [f64; 4] {
    let side = width / ALLSKY_COLUMNS as f64;
    let (col, row) = ((ipix % ALLSKY_COLUMNS) as f64, (ipix / ALLSKY_COLUMNS) as f64);
    [col * side / width, row * side / height, side / width, side / height]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn lat_lon(d: [f64; 3]) -> (f64, f64) {
        (d[2].asin().to_degrees(), d[1].atan2(d[0]).to_degrees().rem_euclid(360.0))
    }

    fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    #[test]
    fn face_corners() {
        // Середина грани 4 - экватор на долготе 0, грани 0 - z 2/3 на 45°,
        // южный угол грани 0 - экватор на 45°.
        let (lat, lon) = lat_lon(face_direction(4, 0.5, 0.5));
        assert!(lat.abs() < 1e-9 && lon.abs() < 1e-9);
        let d = face_direction(0, 0.5, 0.5);
        assert!((d[2] - 2.0 / 3.0).abs() < 1e-12);
        assert!((lat_lon(d).1 - 45.0).abs() < 1e-9);
        assert!(lat_lon(face_direction(0, 0.0, 0.0)).0.abs() < 1e-9);
        assert!((face_direction(0, 1.0, 1.0)[2] - 1.0).abs() < 1e-12);
        assert!((face_direction(8, 0.0, 0.0)[2] + 1.0).abs() < 1e-12);
    }

    #[test]
    fn equal_areas() {
        // Середины ячеек порядка 2 расставлены равномерно: ближайший сосед
        // у всех на одном расстоянии с точностью 40 %.
        let c: Vec<[f64; 3]> = (0..192).map(|i| center(2, i)).collect();
        let near: Vec<f64> = c
            .iter()
            .enumerate()
            .map(|(i, a)| c.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, b)| dot(*a, *b)).fold(-1.0, f64::max).acos())
            .collect();
        let (lo, hi) = near.iter().fold((f64::INFINITY, 0.0f64), |(l, h), x| (l.min(*x), h.max(*x)));
        assert!(hi / lo < 1.4);
        assert!(c.iter().all(|v| (dot(*v, *v) - 1.0).abs() < 1e-12));
    }

    #[test]
    fn children_inside_parent() {
        let parent = center(3, 100);
        assert!((400..404).all(|k| dot(center(4, k), parent).acos() < cell_size(3)));
    }

    #[test]
    fn nest_bits() {
        // Чётные биты - ix, нечётные - iy: 1011 даёт ix 01, iy 11.
        assert_eq!(nest_xyf(2, 16 * 5 + 0b1011), (5, 1, 3));
    }

    #[test]
    fn mesh_matches_centers() {
        let (dirs, uv) = tile_mesh(3, 327, 8);
        assert_eq!(dirs.len(), 81);
        assert!(dot(dirs[40], center(3, 327)) > 1e-6f64.cos());
        assert_eq!(mesh_indices(8).len(), 8 * 8 * 6);
        assert_eq!(uv[0], [0.0, 0.0]);
    }

    #[test]
    fn url() {
        assert_eq!(tile_url("b/", 5, 12345), "b/Norder5/Dir10000/Npix12345.jpg");
        assert_eq!(allsky_url("b/"), "b/Norder3/Allsky.jpg");
        assert_eq!(LAYERS[1].base(), "https://alasky.cds.unistra.fr/DSS/DSSColor/");
    }

    #[test]
    fn layers_fade_in_order() {
        // Издалека снимков нет, ближе - карта, совсем вблизи - DSS.
        let [starmap, dss] = LAYERS;
        assert_eq!(share(&starmap, 70.0), 0.0);
        assert_eq!(share(&starmap, 20.0), 1.0);
        assert_eq!(share(&dss, 20.0), 0.0);
        assert_eq!(share(&dss, 1.0), 1.0);
        assert!((share(&starmap, (starmap.far + starmap.near) / 2.0) - 0.5).abs() < 1e-12);
        // Карта проявлена раньше, чем начинается DSS.
        assert!(dss.far <= starmap.near);
        assert!(starmap.allsky && !dss.allsky);
    }

    #[test]
    fn order_for_view() {
        // Широкий вид - Allsky.jpg, поле 1° на 1000 пикселей - порядок
        // с пикселем не крупнее 1.5 пикселя экрана, не глубже обзора.
        assert_eq!(order_for(110f64.to_radians() / 800.0, 9), None);
        assert_eq!(order_for(70f64.to_radians() / 1800.0, 9), Some(3));
        assert_eq!(order_for(1f64.to_radians() / 1000.0, 9), Some(7));
        assert_eq!(order_for(1e-8, 9), Some(9));
    }

    #[test]
    fn visible_cone() {
        let axis = center(5, 5000);
        let cells = visible(5, axis, 2f64.to_radians());
        assert!(cells.contains(&5000));
        assert!(cells.len() < 60);
        assert!(cells.iter().all(|c| dot(center(5, *c), axis).acos().to_degrees() < 6.0));
        assert_eq!(visible(3, axis, PI).len(), 768);
    }

    #[test]
    fn allsky_rect_of_tile() {
        let [u0, v0, du, _] = allsky_rect(28, 1728.0, 1856.0);
        assert!((u0 - 64.0 / 1728.0).abs() < 1e-12);
        assert!((v0 - 64.0 / 1856.0).abs() < 1e-12);
        assert!((du - 64.0 / 1728.0).abs() < 1e-12);
    }
}
