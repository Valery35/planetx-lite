// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Вид неба: взгляд из центра небесной сферы, перенос `core/skyview.py`
//! модуля.
//!
//! Направление взгляда - прямое восхождение и склонение J2000, север мира
//! вверху экрана. Изнутри сферы восток слева, как на звёздной карте.
//! Перетаскивание тянет небо за курсором, колесо меняет угол обзора.
//! Звёзды, Млечный путь, созвездия и планеты стоят в экваториальной
//! системе J2000, поворот на звёздное время здесь не нужен.
//!
//! Взгляд на небо задаётся и позой навигатора глобуса: широта -
//! склонение, долгота - прямое восхождение, расстояние пропорционально
//! полю зрения. Так перелёт van Wijk-Nuij и туры ведут и небо: между
//! далёкими точками поле зрения по пути расширяется.

use std::f64::consts::{PI, TAU};

/// Угол обзора по вертикали, градусы. Наименьший 0.1° - пиксель экрана
/// около пикселя самого подробного порядка обзоров неба HiPS (0.8″).
pub const FOV_MIN: f64 = 0.1;
pub const FOV_MAX: f64 = 110.0;
/// У самого полюса север вверху не задан, радианы.
pub const DEC_LIMIT: f64 = 89.5 * PI / 180.0;
/// Начальный вид: Орион и Телец, угол обзора 70°. Выбор помощника модуля.
pub const HOME: (f64, f64, f64) = (85.0, 10.0, 70.0);
/// Ширина видимой полосы на единицу расстояния у камеры глобуса с углом
/// обзора 45°: 2 tg 22.5°.
pub const SPAN: f64 = 0.828_427_124_746_190_1;
/// Радиус пересчёта позы постоянный, радиус Земли: метки и туры неба
/// одинаковы с любого тела под ним.
pub const SKY_RADIUS: f64 = 6378137.0;

type V3 = [f64; 3];

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalized(v: V3) -> V3 {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if n > 0.0 { [v[0] / n, v[1] / n, v[2] / n] } else { v }
}

/// Направление взгляда в радианах и угол обзора в градусах.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkyView {
    pub ra: f64,
    pub dec: f64,
    pub fov: f64,
}

impl Default for SkyView {
    fn default() -> Self {
        Self::new(HOME.0, HOME.1, HOME.2)
    }
}

impl SkyView {
    /// Взгляд на ra, dec в градусах с углом обзора fov.
    pub fn new(ra: f64, dec: f64, fov: f64) -> Self {
        Self { ra: ra.to_radians(), dec: dec.to_radians(), fov }
    }

    /// Взгляд на точку неба, градусы. fov None - угол обзора прежний.
    pub fn set(&mut self, ra: f64, dec: f64, fov: Option<f64>) {
        self.ra = ra.to_radians().rem_euclid(TAU);
        self.dec = dec.to_radians().clamp(-DEC_LIMIT, DEC_LIMIT);
        if let Some(fov) = fov {
            self.fov = fov.clamp(FOV_MIN, FOV_MAX);
        }
    }

    pub fn forward(&self) -> V3 {
        let cd = self.dec.cos();
        [cd * self.ra.cos(), cd * self.ra.sin(), self.dec.sin()]
    }

    /// Оси камеры - вправо, вверх, назад. Правая ось смотрит на запад.
    pub fn rotation(&self) -> [V3; 3] {
        let f = self.forward();
        let right = normalized(cross(f, [0.0, 0.0, 1.0]));
        let up = cross(right, f);
        [right, up, [-f[0], -f[1], -f[2]]]
    }

    /// Угол на пиксель кадра в середине вида.
    pub fn radians_per_pixel(&self, height: f64) -> f64 {
        2.0 * (self.fov.to_radians() / 2.0).tan() / height.max(1.0)
    }

    /// Небо тянется за курсором на dx, dy пикселей кадра, y вниз.
    pub fn drag(&mut self, dx: f64, dy: f64, height: f64) {
        let step = self.radians_per_pixel(height);
        self.ra = (self.ra + dx * step / self.dec.cos().max(0.05)).rem_euclid(TAU);
        self.dec = (self.dec + dy * step).clamp(-DEC_LIMIT, DEC_LIMIT);
    }

    /// Угол обзора умножается на factor, меньше 1 - ближе.
    pub fn zoom(&mut self, factor: f64) {
        self.fov = (self.fov * factor).clamp(FOV_MIN, FOV_MAX);
    }

    /// Пиксель кадра для направления и признак точки перед взглядом.
    pub fn project(&self, direction: V3, width: f64, height: f64) -> ([f64; 2], bool) {
        let [r, u, b] = self.rotation();
        let dot = |a: V3| a[0] * direction[0] + a[1] * direction[1] + a[2] * direction[2];
        let (x, y, z) = (dot(r), dot(u), dot(b));
        let front = z < -1e-9;
        let depth = if front { -z } else { 1.0 };
        let t = (self.fov.to_radians() / 2.0).tan();
        let aspect = width / height.max(1.0);
        ([(x / depth / (t * aspect) + 1.0) * width / 2.0, (1.0 - y / depth / t) * height / 2.0], front)
    }

    /// Направление на небо под пикселем кадра, J2000.
    pub fn direction_at(&self, px: f64, py: f64, width: f64, height: f64) -> V3 {
        let t = (self.fov.to_radians() / 2.0).tan();
        let aspect = width / height.max(1.0);
        let local = [(2.0 * px / width - 1.0) * t * aspect, (1.0 - 2.0 * py / height) * t, -1.0];
        let [r, u, b] = self.rotation();
        normalized([0, 1, 2].map(|i| r[i] * local[0] + u[i] * local[1] + b[i] * local[2]))
    }
}

/// Расстояние позы для поля зрения fov, radius - радиус пересчёта.
pub fn distance_for(fov: f64, radius: f64) -> f64 {
    radius * fov.to_radians() / SPAN
}

/// Поле зрения по расстоянию позы, обратное [`distance_for`].
pub fn fov_for(distance: f64, radius: f64) -> f64 {
    (distance * SPAN / radius).to_degrees()
}

/// Широта, долгота и расстояние позы для взгляда view.
pub fn pose_of(view: &SkyView, radius: f64) -> (f64, f64, f64) {
    let ra = view.ra.to_degrees();
    let lon = if ra > 180.0 { ra - 360.0 } else { ra };
    (view.dec.to_degrees(), lon, distance_for(view.fov, radius))
}

/// Взгляд view по позе навигатора.
pub fn follow_pose(view: &mut SkyView, lat: f64, lon: f64, distance: f64, radius: f64) {
    view.set(lon.rem_euclid(360.0), lat, Some(fov_for(distance, radius)));
}

/// Прямое восхождение от 0 до 360° и склонение направления, градусы.
pub fn ra_dec_of(v: V3) -> (f64, f64) {
    crate::ephemeris::ra_dec_of(v)
}

/// Запись направления в астрономической нотации J2000: α в часах,
/// минутах и секундах, δ в градусах и минутах.
pub fn ra_dec_text(v: V3) -> String {
    let (ra, dec) = ra_dec_of(v);
    let seconds = ((ra / 15.0 * 3600.0).round() as i64).rem_euclid(86400);
    let arcmin = (dec.abs() * 60.0).round() as i64;
    format!("α {:02}h {:02}m {:02}s  δ {}{:02}° {:02}′", seconds / 3600, seconds / 60 % 60, seconds % 60, if dec < 0.0 { "-" } else { "+" }, arcmin / 60, arcmin % 60)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::stars::sky_direction;

    fn dir(ra: f64, dec: f64) -> V3 {
        sky_direction(ra.to_radians(), dec.to_radians())
    }

    #[test]
    fn axes_are_orthonormal() {
        let r = SkyView::new(123.0, -40.0, 60.0).rotation();
        for i in 0..3 {
            for j in 0..3 {
                let d: f64 = (0..3).map(|k| r[i][k] * r[j][k]).sum();
                assert!((d - if i == j { 1.0 } else { 0.0 }).abs() < 1e-12);
            }
        }
        // Правая тройка: right × up = back.
        let c = cross(r[0], r[1]);
        assert!((0..3).all(|k| (c[k] - r[2][k]).abs() < 1e-12));
    }

    #[test]
    fn centre_and_north_up() {
        let view = SkyView::new(83.0, 5.0, 40.0);
        let ([x, y], _) = view.project(dir(83.0, 5.0), 1600.0, 900.0);
        assert!((x - 800.0).abs() < 1e-6 && (y - 450.0).abs() < 1e-6);
        let ([_, ny], _) = view.project(dir(83.0, 15.0), 1600.0, 900.0);
        let ([ex, _], _) = view.project(dir(93.0, 5.0), 1600.0, 900.0);
        assert!(ny < 450.0, "север выше середины");
        assert!(ex < 800.0, "восток слева, изнутри сферы");
    }

    #[test]
    fn behind_is_masked() {
        let view = SkyView::new(0.0, 0.0, 60.0);
        assert!(!view.project(dir(180.0, 0.0), 800.0, 600.0).1);
    }

    #[test]
    fn pixel_round_trip() {
        let view = SkyView::new(250.0, 60.0, 80.0);
        let v = view.direction_at(300.0, 200.0, 1200.0, 800.0);
        let ([x, y], _) = view.project(v, 1200.0, 800.0);
        assert!((x - 300.0).abs() < 1e-6 && (y - 200.0).abs() < 1e-6);
    }

    #[test]
    fn drag_follows_cursor() {
        // Точка под курсором после перетаскивания уходит вместе с ним.
        let mut view = SkyView::new(40.0, 20.0, 30.0);
        let v = view.direction_at(600.0, 400.0, 1200.0, 800.0);
        view.drag(50.0, -30.0, 800.0);
        let ([x, y], _) = view.project(v, 1200.0, 800.0);
        assert!((x - 650.0).abs() < 1.5 && (y - 370.0).abs() < 1.5, "{x} {y}");
    }

    #[test]
    fn pose_round_trip() {
        // Взгляд - поза навигатора - взгляд, в том числе RA за 180°.
        for (ra, dec, fov) in [(83.0, 5.0, 70.0), (279.2, 38.8, 30.0), (350.0, -60.0, 2.0)] {
            let view = SkyView::new(ra, dec, fov);
            let (lat, lon, distance) = pose_of(&view, 1737400.0);
            assert!((-180.0..=180.0).contains(&lon));
            let mut back = SkyView::default();
            follow_pose(&mut back, lat, lon, distance, 1737400.0);
            assert!((back.ra.to_degrees() - ra).abs() < 1e-6);
            assert!((back.dec.to_degrees() - dec).abs() < 1e-6);
            assert!((back.fov - fov).abs() < 1e-6);
        }
    }

    #[test]
    fn limits() {
        let mut view = SkyView::new(0.0, 0.0, 60.0);
        view.zoom(1e-3);
        assert_eq!(view.fov, FOV_MIN);
        view.zoom(1e6);
        assert_eq!(view.fov, FOV_MAX);
        view.drag(0.0, 1e7, 800.0);
        assert!((view.dec - DEC_LIMIT).abs() < 1e-12);
    }

    #[test]
    fn span_and_text() {
        assert!((SPAN - 2.0 * (22.5f64).to_radians().tan()).abs() < 1e-15);
        assert_eq!(ra_dec_text(dir(83.0, -5.5)), "α 05h 32m 00s  δ -05° 30′");
    }
}
