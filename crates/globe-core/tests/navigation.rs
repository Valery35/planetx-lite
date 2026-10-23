// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Навигация: захват Земли, колесо, наклон, инерция.
//!
//! Перенос `planetx/tests/test_navigation.py` модуля и теста пределов
//! высоты из `test_body.py`. Рельеф с NaN проверяется функцией, которая
//! отвечает NaN. В модуле она ещё звала `terrain.mercator_share`, его
//! в Lite пока нет.
//!
//! Захват прощает отказ не у края диска, как в модуле, а по точному
//! условию недостижимости - окружность до точки захвата накрывает
//! полюс, см. [`pole_in_reach`].

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use common::{PERM, Rng};
use globe_core::camera::Camera;
use globe_core::ellipsoid::{self as el, Ellipsoid};
use globe_core::navigation::{self as nav, Navigator, Pose, Terrain};
use globe_core::{DMat3, DVec2, DVec3};

const W: u32 = 1920;
const H: u32 = 1080;
const WF: f64 = W as f64;
const HF: f64 = H as f64;
const ROLL: f64 = 1e-12;

fn wgs84() -> Ellipsoid {
    Ellipsoid::wgs84()
}

fn navigator_at(distance: f64, heading: f64, tilt: f64, where_: (f64, f64)) -> Navigator {
    let cam = Camera::new(wgs84(), DVec3::ZERO, DMat3::IDENTITY).with_viewport(W, H);
    Navigator::new(cam, Pose::new(where_.0, where_.1, distance, heading, tilt))
}

fn navigator(distance: f64, heading: f64, tilt: f64) -> Navigator {
    navigator_at(distance, heading, tilt, PERM)
}

fn terrain(f: impl Fn(f64, f64) -> f64 + Send + Sync + 'static) -> Terrain {
    Arc::new(f)
}

/// Случайный пиксель, под которым есть Земля.
fn on_earth(cam: &Camera, rng: &mut Rng) -> (f64, f64) {
    for _ in 0..10_000 {
        let px = rng.uniform(0.1, 0.9) * WF;
        let py = rng.uniform(0.1, 0.9) * HF;
        if nav::ground_under(cam, px, py, None).is_some() {
            return (px, py);
        }
    }
    panic!("под окном нет Земли");
}

fn pixel(cam: &Camera, point: DVec3) -> Option<DVec2> {
    nav::pixel_at(cam, cam.eye, &cam.rotation, point)
}

fn pixel_error(cam: &Camera, point: DVec3, px: f64, py: f64) -> f64 {
    pixel(cam, point).map_or(f64::INFINITY, |p| (p - DVec2::new(px, py)).length())
}

fn angle(a: DVec3, b: DVec3) -> f64 {
    a.cross(b).length().atan2(a.dot(b))
}

/// Окружность до точки захвата накрывает полюс.
///
/// При закреплённых азимуте и наклоне место точки grab на экране
/// задают угловое расстояние ρ от точки взгляда до grab и азимут
/// направления на grab. Чтобы grab встала под пиксель (qx, qy), точка
/// взгляда должна лежать на окружности радиуса ρ вокруг grab, где ρ -
/// угол между точкой взгляда и землёй под пикселем при текущей позе.
/// Без полюса внутри окружности азимут на grab обходит полный круг,
/// и захват достижим. С полюсом внутри азимут ходит в полосе, часть
/// направлений недостижима. Разбор 21 октября 2026 года: на выборке
/// из 726 захватов все недостижимые случаи - с накрытым полюсом.
fn pole_in_reach(cam: &Camera, pose: &Pose, grab: DVec3, qx: f64, qy: f64) -> bool {
    let look = cam.ellipsoid.geodetic_to_ecef(pose.lat, pose.lon, pose.h);
    let Some(target) = nav::ground_under(cam, qx, qy, None) else {
        return true;
    };
    let polar = angle(grab, DVec3::Z).min(angle(grab, DVec3::NEG_Z));
    polar < angle(look, target)
}

// Захват.

fn check_drags(n: &mut Navigator, count: usize, reach: f64, seed: u64) -> usize {
    let mut rng = Rng(seed);
    let (heading, tilt, distance) = (n.pose.heading, n.pose.tilt, n.pose.distance);
    let mut moved = 0;
    let mut t = 0.0;
    for _ in 0..count {
        let (px, py) = on_earth(&n.camera, &mut rng);
        assert!(n.press(px, py, t));
        let point = n.grab.unwrap();
        for _ in 0..3 {
            t += 0.016;
            let qx = (px + rng.uniform(-reach, reach)).clamp(1.0, WF - 1.0);
            let qy = (py + rng.uniform(-reach, reach)).clamp(1.0, HF - 1.0);
            if nav::ground_under(&n.camera, qx, qy, None).is_none() {
                // За краем диска точку поставить некуда.
                assert!(!n.drag(qx, qy, t));
                continue;
            }
            if pole_in_reach(&n.camera, &n.pose, point, qx, qy) {
                // Часть направлений недостижима: север в точке взгляда
                // закреплён вверху окна.
                if !n.drag(qx, qy, t) {
                    continue;
                }
            } else {
                assert!(n.drag(qx, qy, t), "{qx} {qy}");
            }
            moved += 1;
            assert!(pixel_error(&n.camera, point, qx, qy) < 0.5);
            assert!(nav::roll(&n.camera, &n.pose).abs() < ROLL);
        }
        n.release(t + 1.0);
    }
    // Захват не трогает азимут, наклон и расстояние.
    assert_eq!((n.pose.heading, n.pose.tilt, n.pose.distance), (heading, tilt, distance));
    moved
}

#[test]
fn grab_street_level() {
    let moved = check_drags(&mut navigator(500.0, 0.0, 0.0), 34, 200.0, 1);
    assert!(moved > 95, "{moved}");
}

#[test]
fn grab_tilted() {
    let moved = check_drags(&mut navigator(2000.0, 40.0, 60.0), 33, 150.0, 2);
    assert!(moved > 90, "{moved}");
}

#[test]
fn grab_from_space() {
    let moved = check_drags(&mut navigator(2.0e7, 0.0, 0.0), 33, 150.0, 3);
    assert!(moved > 50, "{moved}");
}

#[test]
fn grab_near_pole_is_refused() {
    // Поза, на которой 21 октября 2026 года отказал захват из космоса.
    // Перебор точек взгляда дал наименьшую невязку 7.3 пикселя, модуль
    // тоже отказывает. Полюс в 12.4° от точки захвата, окружность
    // радиуса 46.8° его накрывает.
    let mut n = navigator_at(2.0e7, 0.0, 0.0, (71.371_357_053_102_74, 12.856_696_042_053_812));
    let grab = DVec3::new(1_110_444.338_048_091_2, 802_221.596_664_9, 6_208_398.895_755_98);
    let (qx, qy) = (871.348_930_339_575, 279.476_141_623_671_25);
    assert!(pole_in_reach(&n.camera, &n.pose, grab, qx, qy));
    assert!(nav::solve_pin(&n.camera, &n.pose, grab, qx, qy).is_none());
    n.grab = Some(grab);
    let eye = n.camera.eye;
    assert!(!n.drag(qx, qy, 0.1));
    assert_eq!(n.camera.eye, eye);
}

#[test]
fn straight_down_keeps_heading() {
    let mut n = navigator(3000.0, 70.0, 0.0);
    assert!(n.press(WF / 2.0, HF / 2.0, 0.0));
    assert!(n.drag(WF / 2.0 + 300.0, HF / 2.0 - 200.0, 0.02));
    assert_eq!(n.pose.heading, 70.0);
}

#[test]
fn press_off_the_globe() {
    let mut n = navigator(2.0e7, 0.0, 0.0);
    assert!(!n.press(5.0, 5.0, 0.0));
    assert!(!n.drag(50.0, 50.0, 0.1));
}

#[test]
fn impossible_drag_leaves_camera() {
    // Точку у центра нельзя увести за край диска Земли.
    let mut n = navigator(2.0e7, 0.0, 0.0);
    assert!(n.press(WF / 2.0, HF / 2.0, 0.0));
    let eye = n.camera.eye;
    assert!(!n.drag(3.0, 3.0, 0.1));
    assert_eq!(n.camera.eye, eye);
}

// Колесо.

#[test]
fn point_under_cursor_stays() {
    let mut rng = Rng(4);
    for (distance, tilt) in [(2.0e7, 0.0), (5000.0, 0.0), (3000.0, 60.0)] {
        let mut n = navigator(distance, 20.0, tilt);
        for _ in 0..10 {
            let (px, py) = on_earth(&n.camera, &mut rng);
            let point = nav::ground_under(&n.camera, px, py, None).unwrap();
            let d0 = n.pose.distance;
            let pose = nav::zoom(&n.camera, &n.pose, px, py, 0.8);
            assert!(pose.is_some(), "{distance} {tilt}");
            n.set_pose(pose.unwrap());
            assert!(pixel_error(&n.camera, point, px, py) < 0.5);
            assert!((n.pose.distance - 0.8 * d0).abs() < 1e-6 * d0);
        }
    }
}

#[test]
fn minimum_altitude() {
    let mut n = navigator(300.0, 0.0, 0.0);
    for _ in 0..20 {
        let pose = nav::zoom(&n.camera, &n.pose, WF / 2.0, HF / 2.0, 0.5).unwrap();
        n.set_pose(pose);
    }
    assert!((nav::altitude(&wgs84(), n.camera.eye) - nav::MIN_ALTITUDE).abs() < 0.5);
}

#[test]
fn smooth_zoom_reaches_the_total() {
    let mut n = navigator(10000.0, 0.0, 0.0);
    n.wheel(WF / 2.0, HF / 2.0, 0.8, 0.0);
    n.wheel(WF / 2.0, HF / 2.0, 0.8, 0.0);
    let mut t = 0.0;
    while n.step(t) {
        t += 0.016;
        assert!(t < 5.0);
    }
    assert!((n.pose.distance / 10000.0 - 0.64).abs() < 0.001, "{}", n.pose.distance);
}

#[test]
fn turn_does_not_cancel_zoom() {
    let mut n = navigator(10000.0, 0.0, 0.0);
    n.wheel(WF / 2.0, HF / 2.0, 0.8, 0.0);
    n.turn(10.0, 20.0);
    let mut t = 0.0;
    while n.step(t) {
        t += 0.016;
        assert!(t < 5.0);
    }
    assert!((n.pose.distance / 10000.0 - 0.8).abs() < 0.001, "{}", n.pose.distance);
    assert_eq!((n.pose.heading, n.pose.tilt), (10.0, 20.0));
}

// Поворот и наклон.

#[test]
fn centre_stays_and_no_roll() {
    let mut n = navigator(3000.0, 10.0, 30.0);
    let centre = nav::ground_under(&n.camera, WF / 2.0, HF / 2.0, None).unwrap();
    for (dh, dt) in [(15.0, 0.0), (0.0, 20.0), (-40.0, -10.0)] {
        n.turn(dh, dt);
        assert!(pixel_error(&n.camera, centre, WF / 2.0, HF / 2.0) < 1e-6);
        assert!(nav::roll(&n.camera, &n.pose).abs() < ROLL);
    }
}

#[test]
fn tilt_limits() {
    let mut n = navigator(3000.0, 0.0, 30.0);
    n.turn(0.0, 200.0);
    assert_eq!(n.pose.tilt, nav::MAX_TILT);
    n.turn(0.0, -300.0);
    assert_eq!(n.pose.tilt, 0.0);
}

#[test]
fn tilt_keeps_camera_above_minimum() {
    let mut n = navigator(60.0, 0.0, 0.0);
    n.turn(0.0, 85.0);
    assert!(nav::altitude(&wgs84(), n.camera.eye) >= nav::MIN_ALTITUDE - 0.01);
    assert!(n.pose.tilt > 0.0);
}

// Рельеф.

/// Холмы до 3 км с периодом около 0.2°.
fn hills_height(lat: f64, lon: f64) -> f64 {
    1500.0 + 1500.0 * (lat * 31.0).sin() * (lon * 23.0).cos()
}

fn hills() -> Terrain {
    terrain(hills_height)
}

fn terrain_navigator(distance: f64, tilt: f64, relief: Terrain) -> Navigator {
    let mut n = navigator(distance, 30.0, tilt);
    n.set_terrain(Some(relief));
    n
}

#[test]
fn air_pose_is_not_put_on_terrain() {
    // Вид из кабины: точка взгляда в 800 м над рельефом не садится.
    let mut n = navigator(5000.0, 30.0, 60.0);
    n.set_pose(Pose::new(10.0, 20.0, 1000.0, 0.0, 60.0).with_h(900.0).with_terrain(Some(terrain(|_, _| 100.0))));
    n.air = true;
    assert!(!n.keep_clear());
    assert_eq!(n.pose.h, 900.0);
    n.show(Pose::new(10.0, 20.0, 1000.0, 0.0, 60.0));
    assert!(!n.air);
    assert_eq!(n.pose.h, 100.0);
}

#[test]
fn new_pose_without_terrain_keeps_navigator_terrain() {
    // Поза без рельефа (смена тела) садится на рельеф навигатора: точка
    // взгляда во впадине на -2600 м, а не на нуле.
    let mut n = navigator(5000.0, 30.0, 60.0);
    n.set_terrain(Some(terrain(|_, _| -2600.0)));
    n.set_pose(Pose::new(18.4, 77.5, 4000.0, 0.0, 75.0));
    assert_eq!(n.pose.h, -2600.0);
    assert!(n.pose.terrain.is_some());
    n.show(Pose::new(18.5, 77.5, 3000.0, 0.0, 75.0));
    assert_eq!(n.pose.h, -2600.0);
    n.set_terrain(None);
    n.set_pose(Pose::new(18.4, 77.5, 4000.0, 0.0, 75.0));
    assert_eq!(n.pose.h, 0.0);
}

#[test]
fn ground_under_lies_on_terrain() {
    let e = wgs84();
    let n = terrain_navigator(8000.0, 40.0, hills());
    let mut rng = Rng(8);
    let relief = hills();
    for _ in 0..20 {
        let (px, py) = on_earth(&n.camera, &mut rng);
        let point = nav::ground_under(&n.camera, px, py, Some(&relief)).unwrap();
        let (lat, lon, h) = e.ecef_to_geodetic(point);
        assert!((h - hills_height(lat, lon)).abs() < 0.5, "{px} {py}");
    }
}

#[test]
fn drag_over_hills_keeps_the_point() {
    let mut n = terrain_navigator(5000.0, 30.0, hills());
    let mut rng = Rng(9);
    let mut t = 0.0;
    let mut moved = 0;
    for _ in 0..20 {
        let (px, py) = on_earth(&n.camera, &mut rng);
        n.press(px, py, t);
        let point = n.grab;
        let qx = px + rng.uniform(-100.0, 100.0);
        let qy = py + rng.uniform(-100.0, 100.0);
        t += 0.02;
        if n.drag(qx, qy, t) {
            moved += 1;
            assert!(pixel_error(&n.camera, point.unwrap(), qx, qy) < 0.5);
        }
        n.release(t + 1.0);
    }
    assert!(moved > 0);
}

#[test]
fn wheel_keeps_clear_of_terrain() {
    let e = wgs84();
    let mut n = terrain_navigator(3000.0, 0.0, hills());
    for _ in 0..30 {
        if let Some(pose) = nav::zoom(&n.camera, &n.pose, WF / 2.0, HF / 2.0, 0.5) {
            n.set_pose(pose);
        }
    }
    let c = nav::clearance(&e, n.camera.eye, Some(&hills()));
    assert!(c >= nav::MIN_ALTITUDE - 0.5, "{c}");
    assert!(c < 200.0, "{c}");
}

#[test]
fn tilt_keeps_clear_of_terrain() {
    let e = wgs84();
    let mut n = terrain_navigator(300.0, 0.0, hills());
    n.turn(0.0, 85.0);
    assert!(nav::clearance(&e, n.camera.eye, Some(&hills())) >= nav::MIN_ALTITUDE - 0.5);
}

#[test]
fn nearest_terrain_on_steep_slope() {
    // Склон 45° к северу, глаз в 150 м над ним по отвесу. Ближайшая
    // точка склона в 106 м. В модуле ближняя плоскость от зазора 150 м
    // срезала склон перед камерой в низком полёте у Эльбруса.
    let e = wgs84();
    let (lat0, lon0) = (43.3, 42.4);
    let slope = terrain(move |lat, _| 3000.0 + (lat - lat0) * nav::M_PER_DEGREE);
    let eye = e.geodetic_to_ecef(lat0, lon0, 3150.0);
    let truth = 150.0 / 2f64.sqrt();
    let found = nav::nearest_terrain(&e, eye, Some(&slope));
    assert!(found <= truth * 1.02, "{found}");
    assert!(found > truth * 0.9, "{found}");
    let flat = nav::nearest_terrain(&e, eye, Some(&terrain(|_, _| 3000.0)));
    assert!((flat - 150.0).abs() < 0.5);
    assert!((nav::nearest_terrain(&e, eye, None) - 3150.0).abs() < 0.5);
    // Высоко над рельефом кольца не опрашиваются, оценка осторожная.
    let high = e.geodetic_to_ecef(lat0, lon0, 100000.0);
    let far = nav::nearest_terrain(&e, high, Some(&slope));
    assert!((far - (97000.0 - globe_core::camera::MAX_TERRAIN)).abs() < 0.5);
}

#[test]
fn nearest_terrain_survives_nan_height() {
    // Рельеф ответил NaN. В модуле NaN уходил в радиусы колец опроса,
    // и кадр падал, 17 октября 2026 года.
    let e = wgs84();
    let eye = e.geodetic_to_ecef(58.0, 56.2, 3000.0);
    let found = nav::nearest_terrain(&e, eye, Some(&terrain(|_, _| f64::NAN)));
    assert!(found.is_finite());
}

#[test]
fn rising_terrain_lifts_the_camera() {
    let e = wgs84();
    let level = Arc::new(AtomicU64::new(0f64.to_bits()));
    let source = level.clone();
    let mut n = terrain_navigator(200.0, 0.0, terrain(move |_, _| f64::from_bits(source.load(Ordering::Relaxed))));
    level.store(4000f64.to_bits(), Ordering::Relaxed);
    assert!(n.keep_clear());
    assert!(nav::clearance(&e, n.camera.eye, Some(&terrain(|_, _| 4000.0))) >= nav::MIN_ALTITUDE - 0.5);
    assert!((n.pose.h - 4000.0).abs() < 1e-7);
    assert!(!n.keep_clear());
}

// Взгляд по сторонам и сдвиг стрелками, как в Google Earth.

#[test]
fn look_keeps_eye() {
    let e = wgs84();
    for (distance, tilt) in [(3000.0, 40.0), (2.0e5, 10.0), (500.0, 70.0)] {
        let pose = Pose::new(PERM.0, PERM.1, distance, 30.0, tilt);
        let (eye0, _) = pose.eye_rotation(&e);
        let out = nav::look(&e, &pose, 20.0, 5.0).unwrap();
        let (eye1, _) = out.eye_rotation(&e);
        assert!((eye1 - eye0).length() < 1e-3 * distance, "{distance} {tilt}");
        assert!((out.heading - 50.0).abs() < 1e-7);
        assert!((out.tilt - (tilt + 5.0)).abs() < 1e-7);
    }
}

#[test]
fn look_into_sky_is_refused() {
    let pose = Pose::new(PERM.0, PERM.1, 3000.0, 0.0, 80.0);
    assert!(nav::look(&wgs84(), &pose, 0.0, 10.0).is_none());
}

#[test]
fn pan_moves_along_heading() {
    let north = nav::pan(&Pose::new(PERM.0, PERM.1, 1000.0, 0.0, 0.0), 1000.0, 0.0);
    assert!(((north.lat - PERM.0) * nav::M_PER_DEGREE - 1000.0).abs() < 1.0);
    assert!((north.lon - PERM.1).abs() < 5e-10);
    let east = nav::pan(&Pose::new(PERM.0, PERM.1, 1000.0, 90.0, 0.0), 1000.0, 0.0);
    assert!(east.lon > PERM.1);
    let right = nav::pan(&Pose::new(PERM.0, PERM.1, 1000.0, 0.0, 0.0), 0.0, 1000.0);
    assert!(right.lon > PERM.1);
}

fn max_diff(a: &DMat3, b: &DMat3) -> f64 {
    (*a - *b).to_cols_array().iter().fold(0.0, |m, v| m.max(v.abs()))
}

#[test]
fn shift_moves_view_along_screen() {
    // Ctrl с левой кнопкой: точка взгляда идёт за курсором, направление
    // взгляда прежнее, глаз на том же расстоянии.
    let e = wgs84();
    for (distance, heading, tilt, dx, dy) in [
        (1.5e7, 330.0, 25.0, 300.0, -250.0),
        (2.0e5, 20.0, 50.0, -120.0, 80.0),
        (3000.0, 0.0, 0.0, 40.0, 40.0),
    ] {
        let mut n = navigator(distance, heading, tilt);
        let before = n.pose.clone();
        let (target, _) = Pose::new(before.lat, before.lon, 0.0, 0.0, 0.0).eye_rotation(&e);
        let rotation = n.camera.rotation;
        let eye = n.camera.eye;
        assert!(n.shift_by(dx, dy), "{distance}");
        n.pose.apply(&mut n.camera);
        // Кадр не поворачивается: оси камеры прежние, глаз ушёл
        // в плоскости экрана.
        assert!(max_diff(&n.camera.rotation, &rotation) < 1e-9, "{distance}");
        assert!((n.camera.eye - eye).dot(rotation.z_axis).abs() < 1e-3, "{distance}");
        assert!(pixel_error(&n.camera, target, WF / 2.0 + dx, HF / 2.0 + dy) < 1e-3, "{distance}");
        // Следующий сдвиг от позы с креном тоже не поворачивает кадр.
        assert!(n.shift_by(-dx, -dy), "{distance}");
        n.pose.apply(&mut n.camera);
        assert!(max_diff(&n.camera.rotation, &rotation) < 1e-9, "{distance}");
    }
}

#[test]
fn shift_off_the_globe_is_refused() {
    let mut n = navigator(2.0e7, 0.0, 0.0);
    let before = (n.pose.lat, n.pose.lon);
    assert!(!n.shift_by(20.0 * WF, 0.0));
    assert_eq!((n.pose.lat, n.pose.lon), before);
}

#[test]
fn zoom_now_keeps_point() {
    let mut n = navigator(2.0e5, 20.0, 30.0);
    let point = nav::ground_under(&n.camera, 1200.0, 400.0, None).unwrap();
    assert!(n.zoom_now(1200.0, 400.0, 0.5));
    n.pose.apply(&mut n.camera);
    assert!(pixel_error(&n.camera, point, 1200.0, 400.0) < 0.01);
    assert!((n.pose.distance - 1.0e5).abs() < 1.0);
}

// Инерция.

/// Бросок: три шага перетаскивания и отпускание, потом кадры.
fn flick(n: &mut Navigator, fps: f64, until: f64) -> (bool, u32) {
    n.press(WF / 2.0, HF / 2.0, 0.0);
    for i in 1..4 {
        let k = f64::from(i);
        n.drag(WF / 2.0 + 30.0 * k, HF / 2.0 + 10.0 * k, 0.016 * k);
    }
    let started = n.release(0.05);
    let mut t = 0.05;
    let mut frames = 0;
    while n.step(t) && t < until {
        frames += 1;
        t = 0.05 + f64::from(frames) / fps;
    }
    (started, frames)
}

#[test]
fn path_does_not_depend_on_frame_rate() {
    let mut slow = navigator(5.0e5, 0.0, 0.0);
    let mut fast = navigator(5.0e5, 0.0, 0.0);
    let (started, frames_slow) = flick(&mut slow, 30.0, 3.0);
    assert!(started);
    let (_, frames_fast) = flick(&mut fast, 144.0, 3.0);
    assert!(frames_fast > frames_slow);
    // Последний кадр обоих прогонов - после остановки инерции, поза
    // в нём - конец пути.
    assert!((slow.camera.eye - fast.camera.eye).length() < 1e-6);
}

#[test]
fn inertia_moves_on_and_stops() {
    let mut n = navigator(5.0e5, 0.0, 0.0);
    n.press(WF / 2.0, HF / 2.0, 0.0);
    for i in 1..4 {
        let k = f64::from(i);
        n.drag(WF / 2.0 + 30.0 * k, HF / 2.0, 0.016 * k);
    }
    let eye = n.camera.eye;
    assert!(n.release(0.05));
    n.step(0.3);
    assert!((n.camera.eye - eye).length() > 1.0);
    let mut t = 0.3;
    while n.step(t) {
        t += 0.016;
        assert!(t < 10.0);
    }
    let stop = -nav::INERTIA_TAU * nav::INERTIA_STOP.ln();
    assert!(t < 0.05 + stop + 0.05, "{t}");
}

#[test]
fn pause_before_release_kills_inertia() {
    let mut n = navigator(5.0e5, 0.0, 0.0);
    n.press(WF / 2.0, HF / 2.0, 0.0);
    for i in 1..4 {
        let k = f64::from(i);
        n.drag(WF / 2.0 + 30.0 * k, HF / 2.0, 0.016 * k);
    }
    assert!(!n.release(0.048 + 0.061));
    assert!(!n.step(0.2));
}

#[test]
fn press_stops_inertia() {
    let mut n = navigator(5.0e5, 0.0, 0.0);
    n.press(WF / 2.0, HF / 2.0, 0.0);
    n.drag(WF / 2.0 + 40.0, HF / 2.0, 0.016);
    n.release(0.02);
    assert!(n.has_inertia());
    n.press(WF / 2.0, HF / 2.0, 0.03);
    assert!(!n.has_inertia());
}

// Пределы высоты у тел, перенос из test_body.py.

#[test]
fn limits_grow_with_large_bodies() {
    // Юпитер целиком в кадре: предел высоты в радиусах.
    let mars = Ellipsoid::new(el::MARS);
    assert_eq!(nav::max_altitude(&mars), nav::MAX_ALTITUDE);
    let jupiter = Ellipsoid::new(el::JUPITER);
    assert!(nav::max_altitude(&jupiter) > 5.0 * jupiter.a);
}


/// Гора 4 км с подошвой около 2 км.
fn mountain_height(lat: f64, lon: f64) -> f64 {
    let (dy, dx) = ((lat - 46.0) / 0.02, (lon - 7.0) / 0.03);
    4000.0 * (-(dx * dx + dy * dy)).exp()
}

#[test]
fn zoom_into_mountain_does_not_jump() {
    // Замечание автора 23 октября 2026 года, то же в модуле: приближение
    // колесом к склону горы переносило камеру далеко. Начало подбора
    // строилось лучом из придвинутой камеры, её глаз бывал внутри горы.
    let e = wgs84();
    let relief = terrain(mountain_height);
    for (tilt, distance) in [(70.0, 9000.0), (80.0, 6000.0), (60.0, 12000.0), (85.0, 4000.0)] {
        let mut n = navigator_at(distance, 0.0, tilt, (45.93, 7.0));
        n.set_terrain(Some(relief.clone()));
        // Пиксель на склоне в средней колонке экрана.
        let Some(py) = (1..134).map(|i| HF - f64::from(i) * 8.0).find(|&py| {
            nav::ground_under(&n.camera, WF / 2.0, py, Some(&relief))
                .is_some_and(|p| e.ecef_to_geodetic(p).2 > 1500.0)
        }) else {
            panic!("склона нет на экране, наклон {tilt}");
        };
        let point = nav::ground_under(&n.camera, WF / 2.0, py, Some(&relief)).unwrap();
        for step in 0..60 {
            let eye = n.camera.eye;
            let Some(pose) = nav::zoom(&n.camera, &n.pose, WF / 2.0, py, 0.85) else {
                break;
            };
            n.set_pose(pose);
            let moved = (n.camera.eye - eye).length();
            let reach = (point - eye).length();
            assert!(moved <= reach, "наклон {tilt}, шаг {step}: глаз ушёл на {moved:.0} м, до точки {reach:.0} м");
            let err = pixel_error(&n.camera, point, WF / 2.0, py);
            assert!(err < 1.0, "наклон {tilt}, шаг {step}: невязка {err}, глаз ушёл на {moved:.0} м, до точки {reach:.0} м, поза {:?}", n.pose);
        }
    }
}