// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Тени в кадре при свете солнца - карта глубины со стороны солнца,
//! перенос `core/shadowmap.py` модуля.
//!
//! Карта глубины - ортогональная проекция вдоль лучей солнца на квадрат со
//! стороной 2·radius вокруг точки взгляда. Тени - только вблизи камеры:
//! radius - доля [`SHARE`] расстояния до точки взгляда, от [`NEAR`] до
//! [`REACH`], дальше [`FROM`] теней нет.
//!
//! Мировые координаты в видеокарту не попадают. Матрица света тайла -
//! поворот в оси света и сдвиг «центр тайла - центр карты», посчитанный
//! в f64. Центр карты привязан к сетке текселей в осях света: при сдвиге
//! вида тень не дрожит.

use glam::{DMat4, DVec3, DVec4};

/// Текселей карты на сторону.
pub const MAP_SIZE: u32 = 2048;
/// Радиус карты в долях расстояния до точки взгляда.
pub const SHARE: f64 = 1.2;
/// Наименьший радиус, м.
pub const NEAR: f64 = 300.0;
/// Наибольший радиус, м.
pub const REACH: f64 = 5000.0;
/// Дальше этого расстояния до точки взгляда теней нет, м.
pub const FROM: f64 = 40000.0;
/// Синус высоты солнца, ниже которой теней нет, высота 1°.
pub const MIN_SUN: f64 = 0.01745240643728351;
/// Синус высоты солнца, с которой считается глубина.
pub const LOW_SUN: f64 = 0.08;
/// Наибольшая полуглубина вдоль лучей, м.
pub const DEPTH_MAX: f64 = 80000.0;
/// Доля радиуса, с которой тени гаснут к краю.
pub const FADE: f64 = 0.8;
/// Сдвиг точки по нормали перед сравнением, текселей.
pub const OFFSET_TEXELS: f64 = 1.5;
/// Запас глубины при сравнении, в размерах текселя.
pub const BIAS_TEXELS: f64 = 0.5;
/// Ступеней радиуса на удвоение - карта не дрожит.
pub const RADIUS_STEPS: f64 = 4.0;

/// Радиус карты по расстоянию до точки взгляда или None - далеко. Радиус
/// идёт ступенями 2^(1/[`RADIUS_STEPS`]), так приближение колесом не
/// меняет масштаб карты каждый кадр.
pub fn radius_for(distance: f64) -> Option<f64> {
    if !(distance > 0.0 && distance <= FROM) {
        return None;
    }
    let raw = (distance * SHARE).clamp(NEAR, REACH);
    let step = 2.0f64.powf((raw.log2() * RADIUS_STEPS).ceil() / RADIUS_STEPS);
    Some(step.min(REACH))
}

/// Оси света и матрицы карты теней.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightFrame {
    /// Центр карты в ECEF, м.
    pub center: DVec3,
    /// Полусторона квадрата карты, м.
    pub radius: f64,
    /// Текселей карты на сторону.
    pub size: u32,
    /// Сторона текселя, м.
    pub texel: f64,
    /// Полуглубина вдоль лучей, м.
    pub depth: f64,
    /// Синус высоты солнца над точкой взгляда.
    pub sin_sun: f64,
    /// Строки поворота в оси света, каждая поделена на масштаб оси.
    axes: [DVec3; 3],
}

impl LightFrame {
    /// center - точка взгляда в ECEF, м, sun - направление на солнце
    /// в ECEF, radius - полусторона квадрата карты, м.
    pub fn new(center: DVec3, sun: DVec3, radius: f64, size: u32) -> Self {
        let sun = sun.normalize_or_zero();
        let forward = -sun;
        // Оси света - только от направления солнца, не от центра карты:
        // иначе при сдвиге вида они поворачивались, и проекция центра
        // Земли уходила на текселы - сетка карты и тени дрожали.
        let cross = forward.cross(DVec3::Z);
        let right = if cross.length() < 1e-6 { DVec3::X } else { cross.normalize() };
        let up = right.cross(forward);
        let sin_sun = sun.dot(center.normalize_or_zero());
        let texel = 2.0 * radius / f64::from(size.max(1));
        // Привязка центра к сетке текселей в осях света.
        let (a, b) = (center.dot(right), center.dot(up));
        let snapped = center + right * ((a / texel).round() * texel - a) + up * ((b / texel).round() * texel - b);
        let depth = (radius / sin_sun.max(LOW_SUN)).min(DEPTH_MAX) + radius;
        Self { center: snapped, radius, size, texel, depth, sin_sun, axes: [right / radius, up / radius, forward / depth] }
    }

    /// Солнце над горизонтом точки взгляда.
    pub fn active(&self) -> bool {
        self.sin_sun > MIN_SUN
    }

    /// Матрица света тайла с центром center: точка от центра тайла
    /// в координаты отсечения карты, каждая от -1 до 1.
    pub fn matrix(&self, center: DVec3) -> DMat4 {
        let [a, b, c] = self.axes;
        let shift = center - self.center;
        DMat4::from_cols(
            DVec4::new(a.x, b.x, c.x, 0.0),
            DVec4::new(a.y, b.y, c.y, 0.0),
            DVec4::new(a.z, b.z, c.z, 0.0),
            DVec4::new(a.dot(shift), b.dot(shift), c.dot(shift), 1.0),
        )
    }

    /// Точка ECEF в координаты отсечения карты.
    pub fn project(&self, point: DVec3) -> DVec3 {
        let [a, b, c] = self.axes;
        let p = point - self.center;
        DVec3::new(a.dot(p), b.dot(p), c.dot(p))
    }

    /// Сдвиг точки по нормали перед сравнением, м.
    pub fn offset(&self) -> f64 {
        OFFSET_TEXELS * self.texel
    }

    /// Запас глубины при сравнении в долях глубины карты 0-1.
    pub fn bias(&self) -> f64 {
        BIAS_TEXELS * self.texel / (2.0 * self.depth)
    }

    /// Значения шейдера: тексель в долях карты, запас глубины, доля радиуса
    /// начала угасания, сдвиг по нормали.
    pub fn shader_params(&self) -> [f32; 4] {
        [1.0 / self.size.max(1) as f32, self.bias() as f32, FADE as f32, self.offset() as f32]
    }
}
