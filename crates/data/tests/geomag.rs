// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Магнитное поле: модель IGRF-14, полюса NOAA, линии равного
//! склонения, сплайны gufm1. Перенос `tests/test_geomag.py` модуля.

use data::geomag::{self, Combined, Model, Poles, Source};

const DATA: &str = r"C:\Dev\planetx\planetx\data";

/// Модель и полюса из файлов данных модуля, если они рядом.
fn load() -> Option<(Model, Poles)> {
    let igrf = std::fs::read_to_string(format!(r"{DATA}\igrf14coeffs.txt"));
    let poles = std::fs::read_to_string(format!(r"{DATA}\magnetic_poles.json"));
    let (Ok(igrf), Ok(poles)) = (igrf, poles) else {
        eprintln!("данных модуля нет, проверка пропущена");
        return None;
    };
    let model = geomag::parse_igrf(&igrf);
    let poles = geomag::parse_poles(&poles);
    assert!(model.is_ok(), "{model:?}");
    assert!(poles.is_ok(), "{poles:?}");
    Some((model.ok()?, poles.ok()?))
}

fn near(a: f64, b: f64, delta: f64) -> bool {
    (a - b).abs() <= delta
}

/// Угол между точками (широта, долгота) на сфере, градусы.
fn angle(a: (f64, f64), b: (f64, f64)) -> f64 {
    let u = |p: (f64, f64)| {
        let (la, lo) = (p.0.to_radians(), p.1.to_radians());
        [la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin()]
    };
    let (p, q) = (u(a), u(b));
    let dot = p[0] * q[0] + p[1] * q[1] + p[2] * q[2];
    dot.min(1.0).acos().to_degrees()
}

// Поле сеткой и линии равного склонения.

#[test]
fn grid_matches_point_field() {
    let Some((model, _)) = load() else {
        return;
    };
    let coeffs = model.at(2020.0).unwrap_or_default();
    let lats = [-60.0, 0.0, 55.75];
    let lons = [-150.0, 37.62, 100.0];
    let (x, y, z) = geomag::components_grid(&coeffs, model.nmax, &lats, &lons, 0.0);
    for (i, lat) in lats.iter().enumerate() {
        for (j, lon) in lons.iter().enumerate() {
            let (px, py, pz) = geomag::components(&coeffs, model.nmax, *lat, *lon, 0.0);
            assert!(near(x.at(i, j), px, 1e-6));
            assert!(near(y.at(i, j), py, 1e-6));
            assert!(near(z.at(i, j), pz, 1e-6));
        }
    }
}

#[test]
fn isogonic_points_have_their_declination() {
    let Some((model, _)) = load() else {
        return;
    };
    let coeffs = model.at(2020.0).unwrap_or_default();
    let (lats, lons, values) = geomag::declination_grid(&coeffs, model.nmax, geomag::GRID_STEP);
    assert_eq!((lats.len(), lons.len()), (89, 181));
    let lines = geomag::isogonic_lines(&lats, &lons, &values, geomag::ISOGONIC_STEP);
    for level in [0.0, 10.0, -10.0] {
        assert!(lines.iter().any(|(l, _)| *l == level), "{level}");
    }
    let mut worst: f64 = 0.0;
    for (level, points) in &lines {
        if level.abs() > 40.0 {
            continue;
        }
        for (lat, lon) in points.iter().step_by(7) {
            let found = geomag::field(&model, *lat, *lon, 2020.0, 0.0);
            assert!(found.is_some());
            let Some(found) = found else {
                return;
            };
            if lat.abs() > 70.0 || found.horizontal < 10000.0 {
                // У полюсов Земли и у магнитных полюсов, где линии
                // сходятся, склонение меняется быстрее шага сетки.
                continue;
            }
            worst = worst.max((found.declination - level).abs());
        }
    }
    // Сетка 2°: между узлами склонение линейно с точностью до градуса.
    assert!(worst < 1.0, "{worst}");
}

// Модель.

#[test]
fn years_of_model() {
    let Some((model, _)) = load() else {
        return;
    };
    assert_eq!(model.nmax, 13);
    assert_eq!(model.first(), 1900.0);
    assert_eq!(model.last(), 2030.0);
    assert!(geomag::field(&model, 55.0, 37.0, 1899.0, 0.0).is_none());
    assert!(geomag::field(&model, 55.0, 37.0, 2031.0, 0.0).is_none());
    // g10 на 2025 год - коэффициент файла как есть.
    let g10 = |year: f64| model.at(year).and_then(|c| c.first().copied()).unwrap_or(f64::NAN);
    assert!(near(g10(2025.0), -29350.0, 1e-7));
    assert!(near(g10(2027.0), -29350.0 + 2.0 * 12.6, 1e-7));
}

#[test]
fn field_points_down_in_north() {
    let Some((model, _)) = load() else {
        return;
    };
    let north = geomag::field(&model, 55.75, 37.62, 2020.0, 0.0);
    let south = geomag::field(&model, -33.87, 151.21, 2020.0, 0.0);
    assert!(north.is_some() && south.is_some());
    let (Some(north), Some(south)) = (north, south) else {
        return;
    };
    assert!(north.inclination > 60.0);
    assert!(south.inclination < -60.0);
    assert!(20000.0 < north.total && north.total < 70000.0);
}

#[test]
fn declination_east_where_known() {
    // Склонение восточное в Боулдере (около +8° в 2020 году) и
    // в Москве (около +11.5°). Сторож знака восточной составляющей:
    // полюс по нулю горизонтальной составляющей знака не видит.
    let Some((model, _)) = load() else {
        return;
    };
    let decl = |lat: f64, lon: f64| geomag::field(&model, lat, lon, 2020.0, 0.0).map_or(f64::NAN, |f| f.declination);
    let boulder = decl(40.0, -105.25);
    let moscow = decl(55.75, 37.62);
    assert!(7.5 < boulder && boulder < 9.0, "{boulder}");
    assert!(10.5 < moscow && moscow < 12.5, "{moscow}");
}

#[test]
fn igrf_pole_matches_noaa_table() {
    // Сторож расчёта поля: точка нулевой горизонтальной составляющей
    // по IGRF-14 - полюс таблицы NOAA. На прежних ошибках знаков
    // и пересчёта широты поиск уходит на сотни километров.
    let Some((model, poles)) = load() else {
        return;
    };
    let cases: [(&str, &[f64]); 2] = [("south", &[1950.0, 2000.0, 2020.0]), ("north", &[2020.0])];
    for (key, years) in cases {
        let track = if key == "north" { &poles.north } else { &poles.south };
        for year in years {
            let table = geomag::pole_at(track, *year);
            assert!(table.is_some(), "{key} {year}");
            let found = table.and_then(|t| geomag::dip_pole(&model, *year, t));
            assert!(found.is_some(), "{key} {year}");
            let (Some(table), Some(found)) = (table, found) else {
                return;
            };
            assert!((found.0 - table.0).abs() < 0.05, "{key} {year}");
            let f = geomag::field(&model, found.0, found.1, *year, 0.0).map_or(f64::NAN, |f| f.horizontal);
            assert!(f < 1.0, "{key} {year} {f}");
        }
    }
}

// Полюса.

#[test]
fn table_years() {
    let Some((_, poles)) = load() else {
        return;
    };
    for track in [&poles.north, &poles.south] {
        assert_eq!(track.first().map(|p| p.0), Some(1590.0));
        assert_eq!(track.last().map(|p| p.0), Some(2025.0));
        assert!(track.iter().all(|p| (-180.0..=180.0).contains(&p.2)));
    }
}

#[test]
fn between_years_on_arc() {
    let Some((_, poles)) = load() else {
        return;
    };
    let track = &poles.north;
    let at = |year: f64| geomag::pole_at(track, year).unwrap_or((f64::NAN, f64::NAN));
    let (a, b, mid) = (at(1900.0), at(1901.0), at(1900.5));
    assert!(a.0.min(b.0) - 0.01 <= mid.0 && mid.0 <= a.0.max(b.0) + 0.01);
}

#[test]
fn after_table_from_igrf() {
    let Some((model, poles)) = load() else {
        return;
    };
    let track = &poles.north;
    let late = geomag::pole(&model, track, 2028.0);
    assert!(late.is_some());
    let (Some(late), Some(last)) = (late, track.last()) else {
        return;
    };
    // Полюс идёт к Сибири - за три года сдвиг меньше 300 км.
    assert!(angle(late, (last.1, last.2)) < 3.0);
    assert!(geomag::pole(&model, track, 1500.0).is_none());
}

#[test]
fn trail_ends_at_year() {
    let Some((_, poles)) = load() else {
        return;
    };
    assert_eq!(geomag::trail(&poles.south, 1600.0, None).len(), 11);
}

#[test]
fn trail_starts_at_left_handle() {
    // Левый бегунок обрезает начало следа - замечание пользователя
    // модуля от 14 октября 2026 года.
    let Some((_, poles)) = load() else {
        return;
    };
    let track = &poles.north;
    let whole = geomag::trail(track, 2000.0, None);
    let cut = geomag::trail(track, 2000.0, Some(1900.0));
    assert!(cut.len() < whole.len());
    assert_eq!(cut.first().copied(), geomag::pole_at(track, 1900.0));
    assert_eq!(cut.last(), whole.last());
    // Левый бегунок раньше таблицы - след целиком.
    assert_eq!(geomag::trail(track, 2000.0, Some(1500.0)), whole);
}

#[test]
fn year_of_seconds() {
    // 1 января - целый год, средним годом выходило 2019.997.
    assert!(near(geomag::year_of(0.0), 1970.0, 1e-7));
    assert_eq!(geomag::year_of(1_577_836_800.0), 2020.0);
    assert!(near(geomag::year_of(-2_208_988_800.0), 1900.0, 1e-7));
    assert!(near(geomag::year_of(1_593_561_600.0), 2020.0 + 182.0 / 366.0, 1e-7));
}

// Без данных модуля: разбор, сплайны gufm1, выбор модели по году.

const IGRF: &str = "# заголовок
c/s deg ord IGRF IGRF SV
g/h n m 2000.0 2005.0 2005-10
g  1  0 -100.0 -90.0 2.0
g  1  1 10.0 20.0 0.0
h  1  1 5.0 5.0 1.0
";

#[test]
fn parse_small_igrf() {
    let model = geomag::parse_igrf(IGRF);
    assert!(model.is_ok(), "{model:?}");
    let Ok(model) = model else {
        return;
    };
    assert_eq!((model.nmax, model.first(), model.last()), (1, 2000.0, 2010.0));
    assert_eq!(model.at(2002.5), Some(vec![-95.0, 15.0, 5.0]));
    assert_eq!(model.at(2007.0), Some(vec![-86.0, 20.0, 7.0]));
    assert_eq!(model.at(1999.0), None);
    assert!(geomag::parse_igrf("g 1 0 1 2").is_err());
    // Нет коэффициента h11.
    assert!(geomag::parse_igrf("g/h n m 2000.0 SV\ng 1 0 1 2\ng 1 1 1 2\n").is_err());
    assert!(geomag::parse_poles(r#"{"north": [[1, 2]], "south": []}"#).is_err());
}

#[test]
fn bspline_sums_to_one() {
    let knots = [0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 3.0, 3.0, 3.0];
    for t in [0.0, 0.5, 1.0, 2.25, 3.0] {
        let b = geomag::bspline(t, &knots, 4);
        assert_eq!(b.len(), knots.len() - 4);
        let sum: f64 = b.iter().sum();
        assert!(near(sum, 1.0, 1e-12), "{t} {sum}");
        assert!(b.iter().all(|v| *v >= 0.0));
    }
    // Вне узлов сплайнов нет.
    assert!(geomag::bspline(4.0, &knots, 4).iter().all(|v| *v == 0.0));
    assert!(geomag::bspline(1.0, &[], 4).is_empty());
}

#[test]
fn gufm_before_igrf() {
    // nmax 1, четыре сплайна порядка 4 - восемь узлов, коэффициенты
    // постоянные во времени.
    let text = "gufm1\n1 4\n1590 1590 1590 1590 1990 1990 1990 1990\n-30000 -2000 5000\n-30000 -2000 5000\n-30000 -2000 5000\n-30000 -2000 5000\n";
    let gufm = geomag::parse_gufm(text);
    assert!(gufm.is_ok(), "{gufm:?}");
    let Ok(gufm) = gufm else {
        return;
    };
    // Количество чисел не совпадает с nmax и nspl.
    assert!(geomag::parse_gufm("gufm1\n1 2\n1 2 3\n").is_err());
    assert!(geomag::parse_gufm("gufm1\n").is_err());
    let igrf = geomag::parse_igrf(IGRF).unwrap_or(Model { epochs: Vec::new(), coeffs: Vec::new(), secular: Vec::new(), nmax: 0 });
    let both = Combined { igrf, gufm: Some(gufm) };
    assert_eq!(both.first(), geomag::GUFM_FIRST);
    assert_eq!(both.last(), 2010.0);
    let c = both.at(1700.0).unwrap_or_default();
    assert_eq!(c.len(), 3);
    assert!(near(c[0], -30000.0, 1e-9), "{c:?}");
    assert_eq!(both.at(2002.5), Some(vec![-95.0, 15.0, 5.0]));
    // Между концом gufm1 и началом IGRF - gufm1 вне своих лет, поля нет.
    assert_eq!(both.at(1995.0), None);
    assert_eq!(both.at(1500.0), None);
    assert!(geomag::field(&both, 55.0, 37.0, 1700.0, 0.0).is_some_and(|f| f.inclination > 0.0));
}
