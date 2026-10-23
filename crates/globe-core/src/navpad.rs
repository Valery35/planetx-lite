// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Экранные органы навигации в духе Google Earth, перенос
//! `core/navpad.py` модуля.
//!
//! Органы стоят в правом верхнем углу вида - кольцо компаса с джойстиком
//! взгляда внутри, джойстик сдвига и ползунок высоты с кнопками плюс
//! и минус. Здесь углы кольца, отклонение джойстика, связь положения
//! ползунка с расстоянием и шаги движения за время dt. Отрисовка и мышь -
//! `ui/src/navpad.rs`. Скорости - умолчания модуля. Органы как в Google
//! Earth - решение автора от 29 сентября 2026 года.

/// Градусов в секунду при полном отклонении джойстика взгляда.
pub const LOOK_RATE: f64 = 30.0;
/// Ширин видимой полосы в секунду при полном отклонении джойстика сдвига.
pub const MOVE_RATE: f64 = 0.5;
/// Ширина полосы для скорости сдвига не больше радиуса тела: с 20 000 км
/// полоса в 23 000 км уводила камеру на полюс за полсекунды (модуль,
/// 17 октября 2026 года).
pub const MOVE_WIDTH_RADII: f64 = 1.0;
/// Плюс и минус - вдвое за секунду.
pub const ZOOM_RATE: f64 = std::f64::consts::LN_2;
/// Метров, расстояние у верхнего края ползунка.
pub const NEAR: f64 = 100.0;
/// Метров, у нижнего края, у Земли и тел меньше неё.
pub const FAR: f64 = 2.0e7;
/// У тел крупнее Земли - в радиусах.
pub const FAR_RADII: f64 = FAR / crate::ellipsoid::EARTH.a;
/// Доля яркости контура вдали от курсора. Органы видны всегда - решение
/// автора от 30 сентября 2026 года.
pub const FAINT: f64 = 0.3;

/// Угол точки (x, y) от центра кольца по часовой стрелке от верха,
/// в градусах. Ось y экрана направлена вниз.
pub fn ring_angle(x: f64, y: f64) -> f64 {
    x.atan2(-y).to_degrees().rem_euclid(360.0)
}

/// Кратчайший поворот от a0 к a1, от -180 до 180 градусов.
pub fn angle_delta(a0: f64, a1: f64) -> f64 {
    (a1 - a0 + 180.0).rem_euclid(360.0) - 180.0
}

/// Угол буквы N на кольце: север сдвигается против поворота вида.
pub fn north_angle(heading: f64) -> f64 {
    (-heading).rem_euclid(360.0)
}

/// Поворот вида, когда кольцо тянут от угла a0 к углу a1. Кольцо идёт за
/// курсором вместе с севером, азимут вида меняется обратно.
pub fn ring_turn(a0: f64, a1: f64) -> f64 {
    -angle_delta(a0, a1)
}

/// Отклонение джойстика от -1 до 1 по осям, длина не больше 1.
pub fn stick(x: f64, y: f64, radius: f64) -> (f64, f64) {
    let (sx, sy) = (x / radius, y / radius);
    let length = sx.hypot(sy);
    if length > 1.0 { (sx / length, sy / length) } else { (sx, sy) }
}

/// Поворот взгляда (азимут, наклон) за dt. Вверх - взгляд выше.
pub fn look_step(sx: f64, sy: f64, dt: f64) -> (f64, f64) {
    (sx * LOOK_RATE * dt, -sy * LOOK_RATE * dt)
}

/// Сдвиг (вперёд, вправо) в метрах за dt. width - ширина видимой полосы,
/// скорость от неё, поэтому сдвиг одинаков на экране. radius - большая
/// полуось тела.
pub fn move_step(sx: f64, sy: f64, dt: f64, width: f64, radius: f64) -> (f64, f64) {
    let step = MOVE_RATE * width.min(MOVE_WIDTH_RADII * radius) * dt;
    (-sy * step, sx * step)
}

/// Множитель расстояния за dt. direction 1 - ближе, -1 - дальше.
pub fn zoom_factor(direction: f64, dt: f64) -> f64 {
    (-direction * ZOOM_RATE * dt).exp()
}

/// Расстояние у нижнего края ползунка для тела с полуосью radius.
pub fn slider_far(radius: f64) -> f64 {
    FAR.max(FAR_RADII * radius)
}

/// Место бегунка от 0 (верх, близко) до 1 (низ, далеко). Шкала
/// логарифмическая, как высота в Google Earth.
pub fn slider_share(distance: f64, near: f64, far: f64) -> f64 {
    let distance = distance.clamp(near, far);
    (distance / near).ln() / (far / near).ln()
}

/// Расстояние по месту бегунка.
pub fn slider_distance(share: f64, near: f64, far: f64) -> f64 {
    near * (far / near).powf(share.clamp(0.0, 1.0))
}
