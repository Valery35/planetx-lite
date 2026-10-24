// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Спутники: сплайн между расчётами SGP4, рой групп, виток и след,
//! закрытие Землёй. Перенос `tests/test_satellites.py` модуля.

use std::collections::HashSet;

use data::satellites::{self, Swarm};
use data::sgp4;
use data::subsurface::{A, ecef_to_geodetic, geodetic_to_ecef};

const HEAD: &str = "OBJECT_NAME,OBJECT_ID,EPOCH,MEAN_MOTION,ECCENTRICITY,INCLINATION,\
RA_OF_ASC_NODE,ARG_OF_PERICENTER,MEAN_ANOMALY,EPHEMERIS_TYPE,\
CLASSIFICATION_TYPE,NORAD_CAT_ID,ELEMENT_SET_NO,REV_AT_EPOCH,\
BSTAR,MEAN_MOTION_DOT,MEAN_MOTION_DDOT\n";
const ISS: &str = "ISS (ZARYA),1998-067A,2026-10-05T01:07:02.318304,15.48738655,\
.00068562,51.6316,116.4131,223.9771,136.0673,0,U,25544,999,58878,\
.98342731E-4,.4924E-4,0\n";
/// Геостационарный спутник, придуманные элементы около 60° в. д.
const GEO: &str = "TEST GEO,2020-001A,2026-10-05T00:00:00.000000,1.00270000,\
.0001000,0.0500,80.0000,90.0000,150.0000,0,U,90001,999,100,\
0,0,0\n";

/// Эпоха ISS в секундах UTC от 1970 года.
fn iss_epoch() -> f64 {
    (sgp4::jday(2026, 10, 5, 1, 7, 2.318304) - sgp4::JD_1970) * 86400.0
}

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn norm(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn swarm_of(group: &str, rows: &[&str]) -> Swarm {
    let mut s = Swarm::new();
    s.set_group(group, sgp4::parse_csv(&format!("{HEAD}{}", rows.concat())));
    let mut guard = 0;
    while !s.init_some(1) && guard < 1000 {
        guard += 1;
    }
    s
}

fn swarm(rows: &[&str]) -> Swarm {
    swarm_of("stations", rows)
}

/// Положение и скорость в системе Земли прямым расчётом SGP4.
fn direct(s: &Swarm, jd: f64) -> ([f64; 3], [f64; 3]) {
    let sats = s.sats();
    let tsince: Vec<f64> = sats.iter().map(|f| (jd - sgp4::JD_EPOCH0 - f.epoch) * 1440.0).collect();
    let out = sgp4::propagate(sats, &tsince, None);
    let Some(state) = out.first() else {
        return ([f64::NAN; 3], [f64::NAN; 3]);
    };
    satellites::to_ecef(state.r, state.v, jd)
}

// Сплайн.

#[test]
fn spline_matches_model_between_samples() {
    let mut s = swarm(&[ISS]);
    let mut worst: f64 = 0.0;
    for k in (1..60).step_by(7) {
        let moment = iss_epoch() + 3600.0 + f64::from(k);
        let (got, ok) = s.positions(moment);
        let (want, _) = direct(&s, sgp4::unix_jd(moment));
        assert_eq!(ok, [true]);
        worst = worst.max(norm(sub(got[0], want)));
    }
    // Метры на шаге 60 с у низкой орбиты.
    assert!(worst < 10.0, "{worst}");
}

#[test]
fn ecef_velocity_is_derivative() {
    let s = swarm(&[ISS]);
    let jd = sgp4::unix_jd(iss_epoch() + 600.0);
    let dt = 0.5;
    let out: Vec<([f64; 3], [f64; 3])> = [-dt, 0.0, dt].iter().map(|shift| direct(&s, jd + shift / 86400.0)).collect();
    // Скорость SGP4 - не точная производная её положения, у ISS они
    // расходятся на десятые доли метра в секунду. Поправка за вращение
    // Земли - около 500 м/с, её ошибка видна сразу.
    for k in 0..3 {
        let numeric = (out[2].0[k] - out[0].0[k]) / (2.0 * dt);
        close(out[1].1[k], numeric, 0.5);
    }
}

#[test]
fn geostationary_stands_still() {
    let mut s = swarm_of("geo", &[GEO]);
    let (a, _) = s.positions(iss_epoch());
    let (b, _) = s.positions(iss_epoch() + 6.0 * 3600.0);
    assert_eq!((a.len(), b.len()), (1, 1));
    let h = ecef_to_geodetic(a[0])[2];
    // Спутник висит над одной долготой: за 6 часов сдвиг меньше 100 км
    // при высоте около 35 800 км.
    assert!(norm(sub(a[0], b[0])) < 100e3);
    close(h / 1000.0, 35786.0, 150.0);
}

// Рой.

#[test]
fn same_satellite_once() {
    let mut s = Swarm::new();
    s.set_group("stations", sgp4::parse_csv(&format!("{HEAD}{ISS}")));
    s.set_group("visual", sgp4::parse_csv(&format!("{HEAD}{ISS}{GEO}")));
    let mut guard = 0;
    while !s.init_some(1) && guard < 1000 {
        guard += 1;
    }
    assert_eq!(s.len(), 2);
    assert_eq!(s.group_of, ["stations", "visual"]);
    s.keep(HashSet::from(["visual"]));
    while !s.init_some(satellites::INIT_BUDGET) && guard < 2000 {
        guard += 1;
    }
    assert_eq!(s.len(), 2);
    assert_eq!(s.group_of, ["visual", "visual"]);
    // Цвет и размер точки - по группе.
    assert_eq!(s.colors.len(), 2);
    assert_eq!(s.sizes, [satellites::SIZE, satellites::SIZE]);
    s.drop_group("visual");
    assert!(s.init_some(1));
    assert!(s.is_empty());
}

#[test]
fn info() {
    let s = swarm(&[ISS]);
    let info = s.info(0, iss_epoch() + 1800.0);
    assert!(info.is_some());
    let Some(info) = info else { return };
    assert_eq!(info.name, "ISS (ZARYA)");
    assert_eq!(info.number, 25544);
    assert_eq!(info.group, "stations");
    assert!(info.height > 380.0 && info.height < 450.0, "{}", info.height);
    close(info.speed, 7.66, 0.05);
    close(info.age, 1800.0 / 86400.0, 1e-6);
    assert_eq!(info.error, 0);
    assert!(s.info(5, iss_epoch()).is_none());
}

#[test]
fn old_elements_not_shown() {
    let mut s = swarm(&[ISS, GEO]);
    let later = iss_epoch() + 60.0 * 86400.0;
    let (_, ok) = s.positions(later);
    assert_eq!(ok, [false, false]);
    assert_eq!(s.info(0, later).map(|i| i.error), Some(satellites::STALE));
    let (_, ok) = s.positions(iss_epoch() + 86400.0);
    assert_eq!(ok, [true, true]);
}

#[test]
fn empty() {
    let mut s = Swarm::new();
    assert!(s.init_some(satellites::INIT_BUDGET));
    let (points, ok) = s.positions(iss_epoch());
    assert!(points.is_empty() && ok.is_empty());
    assert!(s.index_of(25544).is_none());
}

// Виток и след.

#[test]
fn iss_orbit_and_track() {
    let s = swarm(&[ISS]);
    let moment = iss_epoch() + 3600.0;
    let path = satellites::path(s.sats(), 0, moment);
    assert!(path.is_some());
    let Some(path) = path else { return };
    assert_eq!(path.lat.len(), satellites::ORBIT_POINTS + 1);
    // Виток на один период почти замкнут: прецессия узла и торможение за
    // полтора часа - десятки километров.
    let first = geodetic_to_ecef(path.lat[0], path.lon[0], path.h[0]);
    let n = path.lat.len() - 1;
    let last = geodetic_to_ecef(path.lat[n], path.lon[n], path.h[n]);
    assert!(norm(sub(first, last)) < 100e3);
    let (low, high) = path.h.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), h| (a.min(*h), b.max(*h)));
    assert!(low > 380e3 && high < 450e3, "{low} {high}");
    // След не выходит за наклонение орбиты 51.6°.
    let top = path.tlat.iter().fold(0.0_f64, |a, v| a.max(v.abs()));
    assert!(top < 51.8 && top > 50.0, "{top}");
    // Середина следа - точка под спутником в сам момент.
    let motion = s.motion(0, moment);
    assert!(motion.is_some());
    let Some((point, _)) = motion else { return };
    let here = ecef_to_geodetic(point);
    let mid = satellites::ORBIT_POINTS / 2;
    close(path.tlat[mid], here[0], 1e-4);
    close(path.tlon[mid], here[1], 1e-4);
    // Первая точка витка - сам спутник.
    close(path.lat[0], here[0], 1e-4);
}

#[test]
fn old_elements_no_path() {
    let s = swarm(&[ISS]);
    let later = iss_epoch() + 60.0 * 86400.0;
    assert!(satellites::path(s.sats(), 0, later).is_none());
    assert!(s.motion(0, later).is_none());
    assert!(satellites::path(s.sats(), 3, iss_epoch()).is_none());
}

#[test]
fn heading() {
    let p = [A, 0.0, 0.0];
    close(satellites::heading(p, [0.0, 1.0, 0.0]), 90.0, 1e-9);
    close(satellites::heading(p, [0.0, 0.0, 1.0]), 0.0, 1e-9);
    close(satellites::heading(p, [0.0, -1.0, 0.0]), 270.0, 1e-9);
}

#[test]
fn index_of() {
    let s = swarm(&[ISS, GEO]);
    assert_eq!(s.index_of(90001), Some(1));
    assert_eq!(s.index_of(12345), None);
}

// Закрытие Землёй.

#[test]
fn far_side_hidden_near_side_seen() {
    let eye = [3.0 * A, 0.0, 0.0];
    let points = [
        [1.2 * A, 0.0, 0.0],  // над точкой под глазом
        [-1.2 * A, 0.0, 0.0], // за Землёй
        [0.0, 1.5 * A, 0.0],  // сбоку, виден
        [-0.2 * A, 1.5 * A, 0.0],
    ];
    assert_eq!(satellites::hidden(eye, &points), [false, true, false, false]);
}

#[test]
fn eye_inside_sees_nothing_hidden() {
    assert_eq!(satellites::hidden([0.0; 3], &[[2.0 * A, 0.0, 0.0]]), [false]);
}

#[test]
fn feed_and_groups() {
    assert_eq!(satellites::feed("stations"), "https://celestrak.org/NORAD/elements/gp.php?GROUP=stations&FORMAT=csv");
    assert_eq!(satellites::color("gnss"), [102, 204, 255]);
    assert_eq!(satellites::color("неизвестная"), [255, 255, 255]);
    assert_eq!(satellites::size("starlink"), 3.0);
    assert!(satellites::DEFAULT_GROUPS.iter().all(|g| satellites::GROUPS.iter().any(|(k, _)| k == g)));
}
