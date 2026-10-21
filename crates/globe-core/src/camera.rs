// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Камера глобуса и матрицы с отсчётом от глаза.
//!
//! Камера хранит положение глаза в ECEF в f64 и ориентацию матрицей
//! 3×3. Столбцы матрицы - оси камеры в ECEF, это вправо, вверх и назад.
//! Камера смотрит против оси «назад», как принято в OpenGL.
//!
//! Мировые координаты в шейдер не попадают. Для каждого тайла матрица
//! считается в f64 от разности «центр тайла - глаз» и только потом
//! округляется до f32.
//!
//! Экранные координаты - пиксели окна, начало в левом верхнем углу,
//! ось y вниз.
//!
//! Перенос `planetx/core/camera.py` модуля. Матрицы glam хранятся
//! по столбцам, у модуля - по строкам NumPy.

use glam::{DMat3, DMat4, DVec2, DVec3, Mat4};

use crate::ellipsoid::Ellipsoid;

/// Отношение дальней плоскости отсечения к ближней. При 24 битах
/// глубины это оставляет различимыми слои на ближней дистанции.
pub const MAX_DEPTH_RATIO: f64 = 1e5;
pub const MIN_NEAR: f64 = 0.5;
/// Высота Эвереста с запасом. Рельеф за горизонтом эллипсоида виден
/// до расстояния горизонта этой высоты.
pub const MAX_TERRAIN: f64 = 9000.0;
/// Ближняя плоскость в доле расстояния до ближайшего рельефа. Запас
/// на рельеф между точками опроса и на сетку тайла, собранную
/// из высот грубее тех, по которым считается расстояние.
pub const NEAR_SHARE: f64 = 0.5;

/// Оси восток, север, вверх в точке эллипсоида.
pub fn enu(lat: f64, lon: f64) -> [DVec3; 3] {
    let (sl, cl) = lat.to_radians().sin_cos();
    let (so, co) = lon.to_radians().sin_cos();
    [DVec3::new(-so, co, 0.0), DVec3::new(-sl * co, -sl * so, cl), DVec3::new(cl * co, cl * so, sl)]
}

/// Ориентация камеры над точкой.
///
/// heading - азимут верха экрана от севера по часовой стрелке, градусы.
/// tilt - отклонение взгляда от отвесного, 0 смотрит вниз, 90 на
/// горизонт. Крен нулевой.
pub fn orientation(lat: f64, lon: f64, heading: f64, tilt: f64) -> DMat3 {
    let [east, north, up] = enu(lat, lon);
    let (sp, cp) = heading.to_radians().sin_cos();
    let (st, ct) = tilt.to_radians().sin_cos();
    let screen_up = north * cp + east * sp;
    let right = east * cp - north * sp;
    let forward = -up * ct + screen_up * st;
    let cam_up = up * st + screen_up * ct;
    DMat3::from_cols(right, cam_up, -forward)
}

/// Матрица проекции OpenGL.
pub fn perspective(fov_y: f64, aspect: f64, near: f64, far: f64) -> DMat4 {
    let f = 1.0 / (fov_y.to_radians() / 2.0).tan();
    DMat4::from_cols_array(&[
        f / aspect,
        0.0,
        0.0,
        0.0,
        0.0,
        f,
        0.0,
        0.0,
        0.0,
        0.0,
        (far + near) / (near - far),
        -1.0,
        0.0,
        0.0,
        2.0 * far * near / (near - far),
        0.0,
    ])
}

/// Ближняя и дальняя плоскости отсечения.
///
/// altitude - высота глаза над эллипсоидом. Дальняя плоскость стоит
/// за горизонтом на расстоянии, с которого ещё видна вершина высотой
/// [`MAX_TERRAIN`]. Без рельефа ближе высоты камеры поверхности нет,
/// и ближняя плоскость берётся на 0.8 высоты. С рельефом nearest -
/// расстояние до ближайшего рельефа, ближняя плоскость берётся
/// на [`NEAR_SHARE`] от него. Отношение дальней к ближней не больше
/// [`MAX_DEPTH_RATIO`]. reach - дальность, до которой видно внутрь тела,
/// при разрезе - расстояние до дальнего края шара.
pub fn clip_range(e: &Ellipsoid, altitude: f64, nearest: Option<f64>, reach: Option<f64>) -> (f64, f64) {
    let h = altitude.max(1.0);
    let mut far = (h * (2.0 * e.a + h)).sqrt() + (MAX_TERRAIN * (2.0 * e.a + MAX_TERRAIN)).sqrt();
    if let Some(reach) = reach {
        far = far.max(reach);
    }
    let close = nearest.map_or(0.8 * h, |n| NEAR_SHARE * n);
    let near = MIN_NEAR.max(close.min(0.8 * h)).max(far / MAX_DEPTH_RATIO);
    (near, far)
}

/// Положение и ориентация камеры, размер окна и угол обзора.
#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    /// Тело, над которым висит камера.
    pub ellipsoid: Ellipsoid,
    /// Глаз в ECEF.
    pub eye: DVec3,
    /// Оси камеры в ECEF по столбцам - вправо, вверх, назад.
    pub rotation: DMat3,
    /// Размер окна в пикселях.
    pub width: u32,
    pub height: u32,
    /// Угол обзора по вертикали, градусы.
    pub fov_y: f64,
    /// Расстояние до ближайшего рельефа, от него считается ближняя
    /// плоскость. None - рельефа нет.
    pub nearest: Option<f64>,
    /// Разрез тела: внутренность шара видна, дальняя плоскость
    /// отодвигается за центр.
    pub deep: bool,
}

impl Camera {
    /// Камера с окном 1920 × 1080 и углом обзора 45°.
    pub fn new(ellipsoid: Ellipsoid, eye: DVec3, rotation: DMat3) -> Self {
        Self { ellipsoid, eye, rotation, width: 1920, height: 1080, fov_y: 45.0, nearest: None, deep: false }
    }

    pub fn with_viewport(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Камера, которая смотрит на точку на высоте h с расстояния distance.
    pub fn look_at(ellipsoid: Ellipsoid, lat: f64, lon: f64, distance: f64, heading: f64, tilt: f64, h: f64) -> Self {
        let mut cam = Self::new(ellipsoid, DVec3::ZERO, DMat3::IDENTITY);
        cam.set_look_at(lat, lon, distance, heading, tilt, h);
        cam
    }

    /// Поставить камеру так же, как [`Camera::look_at`], сохранив окно.
    pub fn set_look_at(&mut self, lat: f64, lon: f64, distance: f64, heading: f64, tilt: f64, h: f64) {
        self.rotation = orientation(lat, lon, heading, tilt);
        self.eye = self.ellipsoid.geodetic_to_ecef(lat, lon, h) + self.rotation.z_axis * distance;
    }

    pub fn aspect(&self) -> f64 {
        f64::from(self.width) / f64::from(self.height)
    }

    pub fn forward(&self) -> DVec3 {
        -self.rotation.z_axis
    }

    /// Высота глаза над эллипсоидом.
    pub fn altitude(&self) -> f64 {
        self.ellipsoid.ecef_to_geodetic(self.eye).2
    }

    pub fn projection(&self) -> DMat4 {
        let reach = self.deep.then(|| self.eye.length() + self.ellipsoid.a);
        let (near, far) = clip_range(&self.ellipsoid, self.altitude(), self.nearest, reach);
        perspective(self.fov_y, self.aspect(), near, far)
    }

    /// Вид из глаза на тайл с центром center, f64.
    ///
    /// Сдвиг «центр - глаз» считается в f64 до поворота. Именно здесь
    /// исчезают миллионы метров, которые f32 не удержал бы. scale
    /// сжимает тайл к центру тела, так рисуется подстилка.
    pub fn tile_model_view(&self, center: DVec3, scale: f64) -> DMat4 {
        let view = self.rotation.transpose();
        let m = view * scale;
        let shift = view * (center * scale - self.eye);
        DMat4::from_cols(m.x_axis.extend(0.0), m.y_axis.extend(0.0), m.z_axis.extend(0.0), shift.extend(1.0))
    }

    /// Матрица для шейдера тайла, f32. None вместо projection - проекция
    /// камеры.
    pub fn tile_mvp(&self, center: DVec3, projection: Option<DMat4>, scale: f64) -> Mat4 {
        let projection = projection.unwrap_or_else(|| self.projection());
        (projection * self.tile_model_view(center, scale)).as_mat4()
    }

    /// Матрицы для набора тайлов, проекция считается один раз.
    ///
    /// scales - сжатие каждого тайла, None - все 1. Недостающие
    /// множители считаются равными 1.
    pub fn tiles_mvp(&self, centers: &[DVec3], scales: Option<&[f64]>, projection: Option<DMat4>) -> Vec<Mat4> {
        let projection = projection.unwrap_or_else(|| self.projection());
        centers
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let scale = scales.and_then(|s| s.get(i)).copied().unwrap_or(1.0);
                self.tile_mvp(*c, Some(projection), scale)
            })
            .collect()
    }

    /// Точка ECEF в пиксели окна, расчёт в f64, и признак «перед камерой».
    pub fn project(&self, point: DVec3) -> (DVec2, bool) {
        let cam = self.rotation.transpose() * (point - self.eye);
        let clip = self.projection() * cam.extend(0.0);
        let w = -cam.z;
        (self.ndc_to_pixels(DVec2::new(clip.x, clip.y) / w), w > 0.0)
    }

    pub fn ndc_to_pixels(&self, ndc: DVec2) -> DVec2 {
        DVec2::new((ndc.x + 1.0) * 0.5 * f64::from(self.width), (1.0 - ndc.y) * 0.5 * f64::from(self.height))
    }

    /// Пиксели окна, луч через центр которых встречает эллипсоид.
    ///
    /// По строкам, width × height значений. erode убирает полосу такой
    /// ширины в пикселях по краю диска. На краю многоугольники тайлов
    /// лежат внутри эллипсоида, и пиксель там законно остаётся фоном.
    pub fn earth_mask(&self, erode: u32) -> Vec<bool> {
        let (w, h) = (self.width as usize, self.height as usize);
        let t = (self.fov_y.to_radians() / 2.0).tan();
        let aspect = self.aspect();
        let scale = DVec3::new(1.0 / self.ellipsoid.a, 1.0 / self.ellipsoid.a, 1.0 / self.ellipsoid.b);
        let o = self.eye * scale;
        let qc = o.dot(o) - 1.0;
        let mut mask = Vec::with_capacity(w * h);
        for row in 0..h {
            let py = 1.0 - (row as f64 + 0.5) / h as f64 * 2.0;
            for col in 0..w {
                let px = (col as f64 + 0.5) / w as f64 * 2.0 - 1.0;
                let d = self.rotation * DVec3::new(px * t * aspect, py * t, -1.0);
                let ds = d * scale;
                let qa = ds.dot(ds);
                let qb = 2.0 * ds.dot(o);
                let disc = qb * qb - 4.0 * qa * qc;
                // Снаружи эллипсоида оба корня одного знака, перед камерой
                // они положительны, когда qb < 0.
                mask.push(disc >= 0.0 && qb < 0.0);
            }
        }
        for _ in 0..erode {
            let mut inner = mask.clone();
            for row in 0..h {
                for col in 0..w {
                    let i = row * w + col;
                    let up = row == 0 || mask[i - w];
                    let down = row + 1 == h || mask[i + w];
                    let left = col == 0 || mask[i - 1];
                    let right = col + 1 == w || mask[i + 1];
                    inner[i] = mask[i] && up && down && left && right;
                }
            }
            mask = inner;
        }
        mask
    }

    /// Луч через точку окна, начало и единичное направление в ECEF.
    pub fn ray(&self, px: f64, py: f64) -> (DVec3, DVec3) {
        let t = (self.fov_y.to_radians() / 2.0).tan();
        let nx = 2.0 * px / f64::from(self.width) - 1.0;
        let ny = 1.0 - 2.0 * py / f64::from(self.height);
        let d = self.rotation * DVec3::new(nx * t * self.aspect(), ny * t, -1.0);
        (self.eye, d.normalize_or_zero())
    }
}
