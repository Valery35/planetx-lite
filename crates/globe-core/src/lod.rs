// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Выбор тайлов для кадра по экранной ошибке.
//!
//! Ошибка тайла уровня z - размер его текселя на местности,
//! e = 2πA·cos φ / (256·2^z). Экранная ошибка равна e·f / d, где f -
//! фокусное расстояние в пикселях, d - расстояние от глаза до ближайшей
//! точки тайла. Тайл делится на четыре, если экранная ошибка больше
//! порога.
//!
//! Тайл заменяется детьми только тогда, когда все его видимые дети
//! готовы к рисованию. До того рисуется он сам или его готовый предок.
//! Поэтому дыр в подложке нет по построению.
//!
//! Перенос `planetx/core/lod.py` модуля. Границы тайла считаются
//! от тела камеры при каждом обходе, общего кэша нет.

use std::collections::{HashMap, HashSet};

use glam::DVec3;

use crate::camera::Camera;
use crate::ellipsoid::Ellipsoid;
use crate::tiling::{MAX_LEVEL, lat_of_row, skirt_depth};

/// Порог экранной ошибки, пикселей.
pub const THRESHOLD: f64 = 1.5;
pub const TEXELS: f64 = 256.0;
/// Запас к радиусу описанной сферы и к угловому радиусу тайла. Точки
/// между девятью опорными могут лежать чуть дальше них.
pub const MARGIN: f64 = 1.05;
pub const MIN_DISTANCE: f64 = 0.5;
/// Приоритет тайла, внутри сферы которого глаз.
pub const INFINITE_PRIORITY: f64 = 1.0e9;
/// Слои анализа рельефа - уклон и горизонтали - считаются только для
/// тайлов не дальше этой доли расстояния от глаза до точки взгляда.
/// В модуле при виде к горизонту окно вставало на сотнях дальних
/// тайлов, замечание пользователя от 12 октября 2026 года.
pub const ANALYSIS_REACH: f64 = 3.0;

/// Ключ тайла (z, x, y).
pub type TileKey = (u32, u32, u32);
/// Наименьшая и наибольшая высота рельефа в тайле или осторожная оценка.
pub type Heights<'a> = &'a dyn Fn(TileKey) -> (f64, f64);
/// Экранные ошибки узлов, None - узел не виден.
pub type MemoValues = HashMap<TileKey, Option<f64>>;

/// Границы тайла: описанная сфера, направление и угловой радиус.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileInfo {
    pub center: DVec3,
    pub radius: f64,
    /// Единичное направление из центра тела на центр тайла.
    pub direction: DVec3,
    /// Угловой радиус тайла, радианы.
    pub alpha: f64,
    /// Наибольший размер текселя на местности, по широте, ближайшей
    /// к экватору.
    pub texel: f64,
}

/// Границы тайла (z, x, y) на теле e.
pub fn tile_info(e: &Ellipsoid, z: u32, x: u32, y: u32) -> TileInfo {
    let z = z.min(MAX_LEVEL);
    let n = f64::from(1u32 << z);
    let (x, y) = (f64::from(x), f64::from(y));
    let lats = [0.0, 0.5, 1.0].map(|f| lat_of_row((y + f) / n));
    let lons = [0.0, 0.5, 1.0].map(|f| (x + f) / n * 360.0 - 180.0);
    let points: Vec<DVec3> =
        lats.iter().flat_map(|la| lons.iter().map(|lo| e.geodetic_to_ecef(*la, *lo, 0.0))).collect();
    let center = points[4];
    let radius = points.iter().map(|p| (*p - center).length()).fold(0.0, f64::max) * MARGIN + skirt_depth(e, z);
    let direction = center.normalize_or_zero();
    let alpha = points
        .iter()
        .map(|p| (direction.dot(*p) / p.length()).clamp(-1.0, 1.0).acos())
        .fold(0.0, f64::max);
    let alpha = (alpha * MARGIN).min(std::f64::consts::PI);
    let (north, south) = (lats[0], lats[2]);
    let nearest = if south <= 0.0 && 0.0 <= north { 0.0 } else { north.abs().min(south.abs()) };
    let texel = std::f64::consts::TAU * e.a * nearest.to_radians().cos() / (TEXELS * n);
    TileInfo { center, radius, direction, alpha, texel }
}

/// Дети тайла в порядке: северо-запад, северо-восток, юго-запад,
/// юго-восток.
pub fn children(key: TileKey) -> [TileKey; 4] {
    let (z, x, y) = key;
    [(z + 1, 2 * x, 2 * y), (z + 1, 2 * x + 1, 2 * y), (z + 1, 2 * x, 2 * y + 1), (z + 1, 2 * x + 1, 2 * y + 1)]
}

/// Доля дыры hole в текстурных координатах тайла key: (u0, v0, сторона).
/// v идёт от северного края, как у сетки тайла. Дыра не глубже key -
/// весь тайл.
pub fn hole_rect(key: TileKey, hole: TileKey) -> (f64, f64, f64) {
    let Some(shift) = hole.0.checked_sub(key.0).filter(|s| *s <= MAX_LEVEL) else {
        return (0.0, 0.0, 1.0);
    };
    let n = f64::from(1u32 << shift);
    let u = (f64::from(hole.1) - f64::from(key.1) * n) / n;
    let v = (f64::from(hole.2) - f64::from(key.2) * n) / n;
    (u, v, 1.0 / n)
}

/// Приоритет загрузки - экранная ошибка, всегда конечная.
///
/// У тайла, внутри сферы которого стоит глаз, ошибка бесконечна. Такие
/// тайлы идут первыми, из них раньше грубые.
pub fn priority(error: f64, z: u32) -> f64 {
    if error.is_infinite() { INFINITE_PRIORITY - f64::from(z) } else { error }
}

/// Тайлы из keys, у которых ближайшая точка описанной сферы не дальше
/// reach от глаза.
pub fn near_keys(e: &Ellipsoid, keys: &[TileKey], eye: DVec3, reach: f64) -> HashSet<TileKey> {
    keys.iter()
        .copied()
        .filter(|&(z, x, y)| {
            let info = tile_info(e, z, x, y);
            (info.center - eye).length() - info.radius <= reach
        })
        .collect()
}

/// Сфера тайла с рельефом: центр на середине слоя высот, радиус больше
/// на половину толщины слоя.
///
/// Полная высота гор в радиусе дробила бы до уровня 19 все тайлы
/// в пределах этой высоты от камеры. Толщина слоя в тайле в горах -
/// сотни метров, а не километры.
fn lifted(info: &TileInfo, low: f64, high: f64) -> (DVec3, f64) {
    if low == 0.0 && high == 0.0 {
        return (info.center, info.radius);
    }
    let mid = 0.5 * (low + high);
    (info.center + info.direction * mid, info.radius + 0.5 * (high - low))
}

/// Величины камеры, общие для всех тайлов кадра.
struct Frame {
    ellipsoid: Ellipsoid,
    eye: DVec3,
    eye_dir: DVec3,
    /// Оси камеры в ECEF: вправо, вверх, назад.
    axes: [DVec3; 3],
    focal: f64,
    /// Плоскости боков пирамиды в осях камеры, нормали внутрь.
    planes: [DVec3; 4],
    /// Угол от точки под камерой до горизонта на сфере радиуса B.
    /// Меньший радиус даёт больший угол, отсечение остаётся осторожным.
    horizon: f64,
    b: f64,
}

impl Frame {
    fn new(camera: &Camera) -> Self {
        let t = (camera.fov_y.to_radians() / 2.0).tan();
        let (tx, ty) = (t * camera.aspect(), t);
        let sx = 1.0 / (1.0 + tx * tx).sqrt();
        let sy = 1.0 / (1.0 + ty * ty).sqrt();
        let dist = camera.eye.length();
        let b = camera.ellipsoid.b;
        Self {
            ellipsoid: camera.ellipsoid,
            eye: camera.eye,
            eye_dir: camera.eye / dist,
            axes: [camera.rotation.x_axis, camera.rotation.y_axis, camera.rotation.z_axis],
            focal: f64::from(camera.height) / (2.0 * t),
            planes: [
                DVec3::new(sx, 0.0, -tx * sx),
                DVec3::new(-sx, 0.0, -tx * sx),
                DVec3::new(0.0, sy, -ty * sy),
                DVec3::new(0.0, -sy, -ty * sy),
            ],
            horizon: if dist > b { (b / dist).min(1.0).acos() } else { std::f64::consts::PI },
            b,
        }
    }

    /// Может ли тайл попасть в кадр.
    ///
    /// low и high - наименьшая и наибольшая высота рельефа в тайле.
    /// Вершина высотой high видна из-за горизонта ещё на угол
    /// acos(B / (B + high)). Сфера тайла для пирамиды поднимается
    /// на середину слоя высот и раздувается на половину его толщины.
    fn visible(&self, z: u32, info: &TileInfo, low: f64, high: f64) -> bool {
        let angle = self.eye_dir.dot(info.direction).clamp(-1.0, 1.0).acos();
        let beyond = if high > 0.0 { (self.b / (self.b + high)).acos() } else { 0.0 };
        if angle > self.horizon + info.alpha + beyond {
            return false;
        }
        if z <= 2 {
            return true;
        }
        let (center, r) = lifted(info, low, high);
        let v = center - self.eye;
        let cam = DVec3::new(self.axes[0].dot(v), self.axes[1].dot(v), self.axes[2].dot(v));
        if cam.z > r {
            return false;
        }
        self.planes.iter().all(|p| p.dot(cam) >= -r)
    }

    fn error(&self, info: &TileInfo, low: f64, high: f64) -> f64 {
        let (center, radius) = lifted(info, low, high);
        let d = (center - self.eye).length() - radius;
        if d <= MIN_DISTANCE {
            return f64::INFINITY;
        }
        info.texel * self.focal / d
    }

    /// Экранная ошибка тайла или None, если он точно не виден.
    fn probe(&self, key: TileKey, check: bool, heights: Option<Heights<'_>>) -> Option<f64> {
        let info = tile_info(&self.ellipsoid, key.0, key.1, key.2);
        let (low, high) = heights.map_or((0.0, 0.0), |h| h(key));
        if check && !self.visible(key.0, &info, low, high) {
            return None;
        }
        Some(self.error(&info, low, high))
    }
}

/// Итог выбора.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    /// Тайлы для рисования, без перекрытий.
    pub draw: Vec<TileKey>,
    /// По одному на тайл draw: None - тайл рисуется целиком, ключ
    /// потомка - тайл рисуется заплаткой только на месте этого потомка,
    /// его снимок ещё не готов. Тайл с заплатками может стоять в draw
    /// несколько раз.
    pub holes: Vec<Option<TileKey>>,
    /// Тайлы для загрузки с приоритетом, в порядке обхода: предок раньше
    /// потомка.
    pub want: Vec<(TileKey, f64)>,
    /// Все нужные кадру тайлы, их нельзя вытеснять из памяти.
    pub keep: HashSet<TileKey>,
    /// Количество просмотренных узлов дерева.
    pub visited: usize,
}

impl Selection {
    /// Приоритет тайла в очереди загрузки или None.
    pub fn wanted(&self, key: TileKey) -> Option<f64> {
        self.want.iter().find(|(k, _)| *k == key).map(|(_, p)| *p)
    }
}

/// Настройки выбора.
pub struct Options<'a> {
    pub threshold: f64,
    pub max_level: u32,
    /// Высоты рельефа в тайле, None - рельефа нет.
    pub heights: Option<Heights<'a>>,
    /// Память из [`Memo::bind`] для этой камеры и этих высот.
    pub memo: Option<&'a mut MemoValues>,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Self { threshold: THRESHOLD, max_level: MAX_LEVEL, heights: None, memo: None }
    }
}

struct Walk<'a, 'b> {
    frame: Frame,
    ready: &'a dyn Fn(TileKey) -> bool,
    options: Options<'b>,
    out: Selection,
}

impl Walk<'_, '_> {
    fn probe_kid(&mut self, kid: TileKey) -> Option<f64> {
        let heights = self.options.heights;
        match self.options.memo.as_deref_mut() {
            None => self.frame.probe(kid, true, heights),
            Some(memo) => *memo.entry(kid).or_insert_with(|| self.frame.probe(kid, true, heights)),
        }
    }

    /// Тайлы для рисования - пары (ключ, дыра или None) - и дыры: места,
    /// которые ничем не закрыты, ключи неготовых тайлов.
    ///
    /// Готовый тайл закрывает дыры потомков заплаткой - собой, но только
    /// на месте дыры. В модуле прежде он рисовался целиком вместо всех
    /// потомков, и при облёте вид мигал размытыми пятнами, замечание
    /// автора от 18 октября 2026 года.
    fn visit(&mut self, key: TileKey, error: f64) -> (Vec<(TileKey, Option<TileKey>)>, Vec<TileKey>) {
        self.out.visited += 1;
        let z = key.0;
        self.out.keep.insert(key);
        let is_ready = (self.ready)(key);
        if !is_ready {
            self.out.want.push((key, priority(error, z)));
        }
        if !(z < self.options.max_level.min(MAX_LEVEL) && error > self.options.threshold) {
            return if is_ready { (vec![(key, None)], Vec::new()) } else { (Vec::new(), vec![key]) };
        }
        let kids: Vec<(TileKey, f64)> =
            children(key).into_iter().filter_map(|kid| self.probe_kid(kid).map(|e| (kid, e))).collect();
        if kids.is_empty() {
            // Дети вместе покрывают тайл целиком, и каждое отсечение
            // осторожное. Раз не виден ни один ребёнок, не виден и тайл.
            // Его сфера задела пирамиду под землёй.
            self.out.keep.remove(&key);
            if let Some(pos) = self.out.want.iter().rposition(|(k, _)| *k == key) {
                self.out.want.remove(pos);
            }
            return (Vec::new(), Vec::new());
        }
        if !is_ready && !kids.iter().any(|(kid, _)| (self.ready)(*kid)) {
            // Фронт загрузки. Ниже неготового тайла без готовых детей
            // рисовать нечего, а просить его потомков рано: они
            // понадобятся, только когда придёт он сам. Без этого правила
            // камера у самой земли просила тысячи тайлов всех уровней.
            return (Vec::new(), vec![key]);
        }
        let mut draws = Vec::new();
        let mut holes = Vec::new();
        for (kid, kid_error) in kids {
            let (d, h) = self.visit(kid, kid_error);
            draws.extend(d);
            holes.extend(h);
        }
        if !holes.is_empty() && is_ready {
            draws.extend(holes.into_iter().map(|h| (key, Some(h))));
            return (draws, Vec::new());
        }
        (draws, holes)
    }
}

/// Тайлы для кадра.
///
/// ready(key) говорит, готов ли тайл к рисованию. Тайл уровня 0 должен
/// быть готов всегда, иначе кадру нечем закрыть тело.
pub fn select(camera: &Camera, ready: &dyn Fn(TileKey) -> bool, options: Options<'_>) -> Selection {
    let frame = Frame::new(camera);
    let root = (0, 0, 0);
    let error = frame.probe(root, false, options.heights).unwrap_or(f64::INFINITY);
    let mut walk = Walk { frame, ready, options, out: Selection::default() };
    let (pairs, _) = walk.visit(root, error);
    let mut out = walk.out;
    (out.draw, out.holes) = pairs.into_iter().unzip();
    out
}

/// Ключ памяти: камера, порог и версия высот.
#[derive(Debug, Clone, PartialEq)]
struct MemoKey {
    eye: [f64; 3],
    rotation: [f64; 9],
    width: u32,
    height: u32,
    fov_y: f64,
    threshold: f64,
    heights_version: u64,
}

/// Память ошибок и видимости узлов для неподвижной камеры.
///
/// Ошибка тайла и то, виден ли он, зависят от камеры и высот, но не
/// от готовности тайлов. Во время загрузки камера стоит, а кадры идут
/// на каждый пришедший тайл. Память держит их между такими кадрами.
/// Она сбрасывается, когда меняется ключ: камера, порог или версия
/// высот, которую даёт вызывающий.
#[derive(Debug, Clone, Default)]
pub struct Memo {
    key: Option<MemoKey>,
    values: MemoValues,
}

impl Memo {
    pub const LIMIT: usize = 100_000;

    pub fn new() -> Self {
        Self::default()
    }

    /// Память для [`select`] с этой камерой и этими высотами.
    ///
    /// added - ключи тайлов высот, пришедших с прошлого вызова. Если
    /// камера та же, забываются только узлы под этими тайлами: их размах
    /// высот мог измениться. Узлы грубее тайла высот берут высоты уровня
    /// выше и не меняются. Если версия выросла не на количество ключей
    /// в added, память сбрасывается целиком.
    pub fn bind(&mut self, camera: &Camera, threshold: f64, heights_version: u64, added: &[TileKey]) -> &mut MemoValues {
        let key = MemoKey {
            eye: camera.eye.to_array(),
            rotation: camera.rotation.to_cols_array(),
            width: camera.width,
            height: camera.height,
            fov_y: camera.fov_y,
            threshold,
            heights_version,
        };
        let same_camera = self.key.as_ref().is_some_and(|k| MemoKey { heights_version: k.heights_version, ..key.clone() } == *k);
        let grew_by_added = self
            .key
            .as_ref()
            .is_some_and(|k| heights_version.checked_sub(k.heights_version) == Some(added.len() as u64));
        if !same_camera || !grew_by_added || self.values.len() > Self::LIMIT {
            self.values.clear();
        } else {
            for &(hz, hx, hy) in added {
                self.values.retain(|&(z, x, y), _| {
                    let Some(shift) = z.checked_sub(hz).filter(|s| *s < 32) else {
                        return true;
                    };
                    (x >> shift, y >> shift) != (hx, hy)
                });
            }
        }
        self.key = Some(key);
        &mut self.values
    }
}
