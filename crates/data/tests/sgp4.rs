// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Модель SGP4/SDP4. Перенос `tests/test_sgp4.py` модуля.
//!
//! Ожидаемые положения - из контрольного набора к статье Vallado,
//! Crawford, Hujsak, Kelso, «Revisiting Spacetrack Report #3», AIAA
//! 2006-6753 (SGP4-VER.TLE и tcppver.out), км в системе TEME. Набор
//! охватывает околоземные орбиты с торможением и низким перигеем,
//! полусуточный и суточный резонансы, отрицательное время, поправку
//! Лиддейна и счёт на 3.5 года от эпохи.

use data::sgp4::{self, Cache, Orbits};

type Row = (f64, [f64; 3]);
type Case = (&'static str, &'static str, [Row; 3]);

const CASES: [Case; 9] = [
    (
        "1 00005U 58002B   00179.78495062  .00000023  00000-0  28098-4 0  4753",
        "2 00005  34.2682 348.7242 1859667 331.7664  19.3264 10.82419157413667",
        [
            (0.0, [7022.46529266, -1400.08296755, 0.03995155]),
            (2160.0, [190.19796988, 7746.96653614, 5110.00675412]),
            (4320.0, [-9060.47373569, 4658.70952502, 813.68673153]),
        ],
    ),
    (
        "1 06251U 62025E   06176.82412014  .00008885  00000-0  12808-3 0  3985",
        "2 06251  58.0579  54.0425 0030035 139.1568 221.1854 15.56387291  6774",
        [
            (0.0, [3988.31022699, 5498.96657235, 0.90055879]),
            (1440.0, [-2777.14682335, -5663.16031708, -2462.54889123]),
            (2880.0, [1159.27802897, 5056.60175495, 4353.49418579]),
        ],
    ),
    (
        "1 08195U 75081A   06176.33215444  .00000099  00000-0  11873-3 0   813",
        "2 08195  64.1586 279.0717 6877146 264.7651  20.2257  2.00491383225656",
        [
            (0.0, [2349.8948335, -14785.93811562, 0.02119378]),
            (1440.0, [2890.80638268, -15446.439523, 948.77010176]),
            (2880.0, [3417.20931586, -16038.79510665, 1894.74934058]),
        ],
    ),
    (
        "1 09998U 74033F   05148.79417928 -.00000112  00000-0  00000+0 0  4480",
        "2 09998   9.4958 313.1750 0270971 327.5225  30.8097  1.16186785 45878",
        [
            (0.0, [25532.98947267, -27244.26327953, -1.11572421]),
            (-1080.0, [37732.454386, 288.18821054, 4643.87587495]),
            (-720.0, [-8535.81598158, 38171.79073851, 3331.00311285]),
        ],
    ),
    (
        "1 14128U 83058A   06176.02844893 -.00000158  00000-0  10000-3 0  9627",
        "2 14128  11.4384  35.2134 0011562  26.4582 333.5652  0.98870114 46093",
        [
            (0.0, [34747.57932696, 24502.37114079, -1.32832986]),
            (1440.0, [36366.59147396, 22023.5424572, -601.47121821]),
            (2880.0, [37802.25393045, 19433.57330019, -1198.66634226]),
        ],
    ),
    (
        "1 20413U 83020D   05363.79166667  .00000000  00000-0  00000+0 0  7041",
        "2 20413  12.3514 187.4253 7864447 196.3027 356.5478  0.24690082  7978",
        [
            (0.0, [25123.29290741, -13225.49966286, 3249.40351869]),
            (1844170.0, [-17163.94050833, -48981.47771614, 7620.3708488]),
            (1844340.0, [5091.5554638, -5030.01134361, -1222.14210549]),
        ],
    ),
    (
        "1 23599U 95029B   06171.76535463  .00085586  12891-6  12956-2 0  2905",
        "2 23599   6.9327   0.2849 5782022 274.4436  25.2425  4.47796565123555",
        [
            (0.0, [9892.63794341, 35.76144969, -1.08228838]),
            (360.0, [11376.23941678, 12858.97121366, 1563.40660172]),
            (720.0, [7140.41945884, 20539.25485336, 2501.21469368]),
        ],
    ),
    (
        "1 28350U 04020A   06167.21788666  .16154492  76267-5  18678-3 0  8894",
        "2 28350  64.9977 345.6130 0024870 260.7578  99.9590 16.47856722116490",
        [
            (0.0, [6333.08123128, -1580.82852326, 90.6935572]),
            (720.0, [-446.42460916, 2932.28872588, 5759.19389757]),
            (1440.0, [-4527.90871828, -723.29199041, -4527.44608319]),
        ],
    ),
    (
        "1 28872U 05037B   05333.02012661  .25992681  00000-0  24476-3 0  1534",
        "2 28872  96.4736 157.9986 0303955 244.0492 110.6523 16.46015938 10708",
        [
            (0.0, [-6131.82730456, 2446.52815528, -253.64211033]),
            (25.0, [896.73799533, 447.12357305, 6607.22400507]),
            (50.0, [5548.43325922, -2480.16469245, -1979.24314527]),
        ],
    ),
];

/// Эталон не может вычислить: фокальный параметр меньше нуля, код 4.
const FAILING: [&str; 2] = [
    "1 33333U 05037B   05333.02012661  .25992681  00000-0  24476-3 0  1534",
    "2 33333  96.4736 157.9986 9950000 244.0492 110.6523  4.00004038 10708",
];

const CSV: &str = "OBJECT_NAME,OBJECT_ID,EPOCH,MEAN_MOTION,ECCENTRICITY,INCLINATION,\
RA_OF_ASC_NODE,ARG_OF_PERICENTER,MEAN_ANOMALY,EPHEMERIS_TYPE,\
CLASSIFICATION_TYPE,NORAD_CAT_ID,ELEMENT_SET_NO,REV_AT_EPOCH,BSTAR,\
MEAN_MOTION_DOT,MEAN_MOTION_DDOT\n\
ISS (ZARYA),1998-067A,2026-10-05T01:07:02.318304,15.48738655,\
.00068562,51.6316,116.4131,223.9771,136.0673,0,U,25544,999,58878,\
.98342731E-4,.4924E-4,0\n";

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

fn close3(a: [f64; 3], b: [f64; 3], eps: f64) {
    for k in 0..3 {
        assert!((a[k] - b[k]).abs() <= eps, "{a:?} != {b:?}");
    }
}

fn norm(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn orbits(lines: &[&str]) -> Orbits {
    let mut orb = Orbits::new(sgp4::parse_tle(lines));
    orb.init_some(None);
    orb
}

// Контрольный набор Vallado.

#[test]
fn positions_match_reference() {
    for (one, two, rows) in CASES {
        let orb = orbits(&[one, two]);
        assert_eq!(orb.sats.len(), 1, "{one}");
        for (tsince, expected) in rows {
            let out = sgp4::propagate(&orb.sats, &[tsince], None);
            assert_eq!(out.len(), 1);
            let Some(state) = out.first() else { return };
            assert_eq!(state.error, 0, "{one} {tsince}");
            close3(state.r, expected, 1e-6);
        }
    }
}

#[test]
fn all_satellites_in_one_call() {
    // Все спутники сразу дают то же, что по одному.
    let lines: Vec<&str> = CASES.iter().flat_map(|(one, two, _)| [*one, *two]).collect();
    let mut orb = Orbits::new(sgp4::parse_tle(&lines));
    orb.init_some(Some(4));
    assert!(!orb.ready);
    orb.init_some(None);
    assert!(orb.ready);
    let tsince: Vec<f64> = CASES.iter().map(|(_, _, rows)| rows[1].0).collect();
    let out = sgp4::propagate(&orb.sats, &tsince, None);
    assert_eq!(out.len(), CASES.len());
    for (state, (_, _, rows)) in out.iter().zip(CASES) {
        close3(state.r, rows[1].1, 1e-6);
        assert_eq!(state.error, 0);
    }
}

#[test]
fn resonance_state_does_not_change_result() {
    // Запомненное состояние интегратора даёт тот же результат, что счёт
    // от эпохи, и вперёд, и после шага назад.
    let orb = orbits(&[CASES[5].0, CASES[5].1]);
    let mut cache = Cache::new(&orb.sats);
    for tsince in [1844170.0, 1844340.0, 10000.0, 1844340.0] {
        let a = sgp4::propagate(&orb.sats, &[tsince], Some(&mut cache));
        let b = sgp4::propagate(&orb.sats, &[tsince], None);
        assert_eq!(a, b);
    }
    // 20413 не резонансный, проверка того же на полусуточном резонансе
    // 08195.
    let orb = orbits(&[CASES[2].0, CASES[2].1]);
    let mut cache = Cache::new(&orb.sats);
    for tsince in [2880.0, 1440.0, 30000.0, -1440.0, 2880.0] {
        let a = sgp4::propagate(&orb.sats, &[tsince], Some(&mut cache));
        let b = sgp4::propagate(&orb.sats, &[tsince], None);
        assert_eq!(a, b);
    }
}

#[test]
fn error_code_and_nan() {
    let orb = orbits(&FAILING);
    let a = sgp4::propagate(&orb.sats, &[20.0], None);
    let b = sgp4::propagate(&orb.sats, &[25.0], None);
    assert_eq!((a.len(), b.len()), (1, 1));
    let (Some(a), Some(b)) = (a.first(), b.first()) else { return };
    assert_eq!((a.error, b.error), (0, 4));
    assert!(a.r.iter().all(|v| v.is_finite()));
    assert!(b.r.iter().all(|v| v.is_nan()));
    assert!(b.v.iter().all(|v| v.is_nan()));
}

// Входные данные.

#[test]
fn csv_and_tle_give_same_elements() {
    let el = sgp4::parse_csv(CSV);
    assert_eq!(el.len(), 1);
    let Some(e) = el.first() else { return };
    assert_eq!(e.name, "ISS (ZARYA)");
    assert_eq!(e.number, 25544);
    close(e.inclo, 51.6316_f64.to_radians(), 1e-7);
    close(e.no, 15.48738655 * 2.0 * std::f64::consts::PI / 1440.0, 1e-12);
    // 5 октября 2026 года 01:07:02.318 от 0 января 1950 года.
    let jd = sgp4::jday(2026, 10, 5, 1, 7, 2.318304);
    close(e.epoch, jd - sgp4::JD_EPOCH0, 1e-7);
}

#[test]
fn bad_rows_skipped() {
    let el = sgp4::parse_csv(&format!("{CSV}BROKEN,1998-067A,нет даты,15.4\n"));
    assert_eq!(el.len(), 1);
    assert!(sgp4::parse_csv("").is_empty());
    assert!(sgp4::parse_csv("A,B\n1,2\n").is_empty());
}

#[test]
fn iss_altitude() {
    let el = sgp4::parse_csv(CSV);
    assert_eq!(el.len(), 1);
    let Some(epoch) = el.first().map(|e| e.epoch) else { return };
    let mut orb = Orbits::new(el);
    let jd = sgp4::JD_EPOCH0 + epoch + 0.25;
    let out = orb.at(jd);
    assert_eq!(out.len(), 1);
    let Some(state) = out.first() else { return };
    let height = norm(state.r) - sgp4::RADIUS;
    assert!(height > 380.0 && height < 450.0, "{height}");
    close(norm(state.v), 7.66, 0.05);
}

#[test]
fn hostile_tle_lines_do_not_break() {
    // Строки с буквами вместо чисел и не ASCII пропускаются.
    let bad = [
        "1 0000AU 58002B   00179.78495062  .00000023  00000-0  28098-4 0  4753",
        "2 0000A  34.2682 348.7242 1859667 331.7664  19.3264 10.82419157413667",
        "1 ЖЖЖЖЖU 58002B   00179.78495062  .00000023  00000-0  28098-4 0  4753",
        "2 ЖЖЖЖЖ  34.2682 348.7242 1859667 331.7664  19.3264 10.82419157413667",
    ];
    assert!(sgp4::parse_tle(&bad).is_empty());
    // Название перед парой берётся, без него - номер.
    let named = sgp4::parse_tle(&["VANGUARD 1", CASES[0].0, CASES[0].1, CASES[1].0, CASES[1].1]);
    let names: Vec<&str> = named.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["VANGUARD 1", "06251"]);
}

#[test]
fn absurd_time_does_not_hang() {
    // Резонансный спутник на время за пределами счёта - потерян, а не
    // тысячи лет интегрирования.
    // 08195 - полусуточный резонанс.
    let orb = orbits(&[CASES[2].0, CASES[2].1]);
    assert!(orb.sats.first().is_some_and(|s| s.irez == 2));
    let sats = vec![orb.sats[0]; 3];
    let out = sgp4::propagate(&sats, &[1.0e300, f64::NAN, f64::INFINITY], None);
    assert_eq!(out.len(), 3);
    for state in out {
        assert_eq!(state.error, sgp4::TIME_ERROR);
        assert!(state.r.iter().all(|v| v.is_nan()));
    }
    // Суточный резонанс 14128 тоже.
    let orb = orbits(&[CASES[4].0, CASES[4].1]);
    assert!(orb.sats.first().is_some_and(|s| s.irez == 1));
}

// Системы координат.

#[test]
fn sidereal_time_at_j2000() {
    // Среднее звёздное время 1 января 2000 года, 12 ч UT1 - 280.46061837°.
    close(sgp4::gstime(2451545.0).to_degrees(), 280.46061837, 1e-6);
}

#[test]
fn ecef_rotation() {
    let jd = 2451545.0;
    let g = sgp4::gstime(jd);
    // Точка TEME в направлении Гринвича ложится на ось x Земли.
    let r = [g.cos() * 7000.0, g.sin() * 7000.0, 10.0];
    close3(sgp4::teme_to_ecef(r, jd), [7000.0, 0.0, 10.0], 1e-9);
}

#[test]
fn unix_jd() {
    assert_eq!(sgp4::unix_jd(0.0), 2440587.5);
    assert_eq!(sgp4::unix_jd(86400.0), 2440588.5);
}
