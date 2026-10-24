// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Маршрут: разбор дорог тайла, граф, путь, сервис OSRM, коридор тайлов.
//! Перенос `tests/test_routing.py` модуля.

use data::routing::{self, Graph, LatLon, Mode, Route, Step};

/// Тайл центра Перми.
const KEY: (u32, u32, u32) = (14, 10751, 4933);
const E: i64 = routing::EXTENT;

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

fn varint(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn bytes_field(number: u64, data: &[u8], out: &mut Vec<u8>) {
    varint(number << 3 | 2, out);
    varint(data.len() as u64, out);
    out.extend_from_slice(data);
}

fn varint_field(number: u64, value: u64, out: &mut Vec<u8>) {
    varint(number << 3, out);
    varint(value, out);
}

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// Значение атрибута тайла.
#[derive(Clone, Copy, PartialEq)]
enum V {
    S(&'static str),
    I(i64),
}

/// Дорога тайла: вершины в единицах тайла и атрибуты.
type Road = (Vec<(i64, i64)>, Vec<(&'static str, V)>);

/// Команды MVT одной линии.
fn line_geometry(points: &[(i64, i64)]) -> Vec<u8> {
    let mut out = Vec::new();
    let Some(&(px, py)) = points.first() else {
        return out;
    };
    varint(1 << 3 | 1, &mut out);
    varint(zigzag(px), &mut out);
    varint(zigzag(py), &mut out);
    let (mut x, mut y) = (px, py);
    varint(((points.len() - 1) as u64) << 3 | 2, &mut out);
    for &(px, py) in &points[1..] {
        varint(zigzag(px - x), &mut out);
        varint(zigzag(py - y), &mut out);
        (x, y) = (px, py);
    }
    out
}

/// Тайл со слоем transportation размера extent.
fn tile_extent(roads: &[Road], extent: u64) -> Vec<u8> {
    let mut keys: Vec<&str> = Vec::new();
    let mut values: Vec<V> = Vec::new();
    let mut features = Vec::new();
    for (points, attrs) in roads {
        let mut tags = Vec::new();
        for (k, v) in attrs {
            if !keys.contains(k) {
                keys.push(k);
            }
            if !values.contains(v) {
                values.push(*v);
            }
            let ki = keys.iter().position(|x| x == k).unwrap_or_default();
            let vi = values.iter().position(|x| x == v).unwrap_or_default();
            varint(ki as u64, &mut tags);
            varint(vi as u64, &mut tags);
        }
        let mut feature = Vec::new();
        bytes_field(2, &tags, &mut feature);
        varint_field(3, 2, &mut feature);
        bytes_field(4, &line_geometry(points), &mut feature);
        bytes_field(2, &feature, &mut features);
    }
    let mut layer = Vec::new();
    varint_field(15, 2, &mut layer);
    bytes_field(1, b"transportation", &mut layer);
    layer.extend_from_slice(&features);
    for k in keys {
        bytes_field(3, k.as_bytes(), &mut layer);
    }
    for v in values {
        let mut value = Vec::new();
        match v {
            V::S(s) => bytes_field(1, s.as_bytes(), &mut value),
            V::I(i) => varint_field(6, zigzag(i), &mut value),
        }
        bytes_field(4, &value, &mut layer);
    }
    varint_field(5, extent, &mut layer);
    let mut out = Vec::new();
    bytes_field(3, &layer, &mut out);
    out
}

fn tile(roads: &[Road]) -> Vec<u8> {
    tile_extent(roads, E as u64)
}

fn road(points: &[(i64, i64)], attrs: &[(&'static str, V)]) -> Road {
    (points.to_vec(), attrs.to_vec())
}

fn minor(points: &[(i64, i64)]) -> Road {
    road(points, &[("class", V::S("minor"))])
}

fn grid_to_latlon(x: i64, y: i64) -> LatLon {
    routing::to_latlon((i64::from(KEY.1) * E + x) as f64, (i64::from(KEY.2) * E + y) as f64)
}

fn graph(roads: &[Road], mode: Mode) -> Option<Graph> {
    let lines = routing::decode(KEY, &tile(roads));
    assert!(lines.is_ok(), "{lines:?}");
    routing::build(&lines.unwrap_or_default(), mode)
}

/// Путь по дорогам roads способом mode, None - графа или пути нет.
fn path(roads: &[Road], mode: Mode, a: LatLon, b: LatLon) -> Option<Route> {
    let g = graph(roads, mode)?;
    routing::route(&g, a, b, mode)
}

// Разбор тайла.

#[test]
fn lines_attributes_and_world_grid() {
    let lines = routing::decode(
        KEY,
        &tile(&[
            road(&[(0, 0), (100, 0)], &[("class", V::S("primary")), ("oneway", V::I(1))]),
            road(&[(0, 50), (0, 200)], &[("class", V::S("path")), ("brunnel", V::S("bridge")), ("oneway", V::I(-1))]),
        ]),
    )
    .unwrap_or_default();
    assert_eq!(lines.len(), 2);
    let (first, second) = (&lines[0], &lines[1]);
    assert_eq!(first.cls.as_deref(), Some("primary"));
    assert_eq!(first.oneway, 1);
    assert_eq!(second.oneway, -1);
    assert_eq!(second.brunnel.as_deref(), Some("bridge"));
    let (x, y) = (i64::from(KEY.1) * E, i64::from(KEY.2) * E);
    assert_eq!(first.points, [(x, y), (x + 100, y)]);
}

#[test]
fn grid_round_trip() {
    let (x, y) = routing::to_grid(58.0105, 56.2294);
    let (lat, lon) = routing::to_latlon(x, y);
    close(lat, 58.0105, 5e-10);
    close(lon, 56.2294, 5e-10);
}

#[test]
fn coarse_tile_is_rescaled_to_level_14() {
    // Тайл уровня 13 с extent 512: точка (1, 2) - в 16 раз дальше
    // от угла, угол - тайл (2x, 2y) уровня 14.
    let data = tile_extent(&[minor(&[(0, 0), (1, 2)])], 512);
    let lines = routing::decode((13, 5, 7), &data).unwrap_or_default();
    assert_eq!(lines.len(), 1);
    let (x, y) = (10 * E, 14 * E);
    assert_eq!(lines[0].points, [(x, y), (x + 16, y + 32)]);
    assert!(routing::decode((15, 0, 0), &data).is_err());
}

#[test]
fn broken_tile_is_an_error() {
    let mut data = tile(&[minor(&[(0, 0), (100, 0)])]);
    data.truncate(data.len() - 5);
    assert!(routing::decode(KEY, &data).is_err());
    assert!(routing::decode(KEY, &[0xff; 3]).is_err());
    assert_eq!(routing::decode(KEY, &[]), Ok(Vec::new()));
}

// Путь.

#[test]
fn shortest_of_two_ways() {
    // Квадрат 1000 × 1000 единиц: напрямую по низу и в обход по трём
    // сторонам. Путь из левого нижнего в правый нижний угол - по низу.
    let roads = [minor(&[(0, 1000), (1000, 1000)]), minor(&[(0, 1000), (0, 0), (1000, 0), (1000, 1000)])];
    let (a, b) = (grid_to_latlon(5, 1000), grid_to_latlon(995, 1000));
    let route = path(&roads, Mode::Car, a, b);
    assert!(route.is_some());
    let Some(route) = route else { return };
    let unit = routing::unit_metres(a.0);
    close(route.length, 990.0 * unit, 0.01 * 990.0 * unit);
    close(route.seconds, route.length / (30.0 / 3.6), 1.0);
}

#[test]
fn faster_road_wins_over_shorter() {
    // Прямо по грунтовке 1000 единиц или в обход по трассе 1400.
    let roads = [
        road(&[(0, 1000), (1000, 1000)], &[("class", V::S("track"))]),
        road(&[(0, 1000), (500, 1490), (1000, 1000)], &[("class", V::S("primary"))]),
    ];
    let route = path(&roads, Mode::Car, grid_to_latlon(0, 1000), grid_to_latlon(1000, 1000));
    assert!(route.is_some());
    let Some(route) = route else { return };
    // Трасса южнее грунтовки: широты пути ниже широты грунтовки.
    let lowest = route.points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    assert!(lowest < grid_to_latlon(0, 1000).0 - 1e-4);
}

#[test]
fn car_keeps_oneway_foot_does_not() {
    let roads = [
        road(&[(1000, 1000), (0, 1000)], &[("class", V::S("minor")), ("oneway", V::I(1))]),
        minor(&[(0, 1000), (0, 0), (1000, 0), (1000, 1000)]),
    ];
    let (a, b) = (grid_to_latlon(0, 1000), grid_to_latlon(1000, 1000));
    let car = path(&roads, Mode::Car, a, b);
    let foot = path(&roads, Mode::Foot, a, b);
    assert!(car.is_some() && foot.is_some());
    let (Some(car), Some(foot)) = (car, foot) else { return };
    let unit = routing::unit_metres(a.0);
    // Машина против одностороннего не едет: в обход, 3000 единиц.
    close(car.length, 3000.0 * unit, 30.0 * unit);
    close(foot.length, 1000.0 * unit, 10.0 * unit);
    close(foot.seconds, foot.length / (5.0 / 3.6), 1.0);
}

#[test]
fn path_is_not_for_car() {
    let roads = [road(&[(0, 0), (1000, 0)], &[("class", V::S("path"))])];
    assert!(graph(&roads, Mode::Car).is_none());
    assert!(graph(&roads, Mode::Foot).is_some());
}

#[test]
fn t_junction_without_shared_vertex_is_snapped() {
    // Боковая дорога кончается в 2 единицах от главной, общей вершины
    // нет. 2 единицы у Перми - около 0.6 м, меньше SNAP.
    let roads = [road(&[(0, 0), (2000, 0)], &[("class", V::S("primary"))]), minor(&[(1000, 1000), (1000, 2)])];
    let route = path(&roads, Mode::Car, grid_to_latlon(1000, 1000), grid_to_latlon(2000, 0));
    assert!(route.is_some());
    let Some(route) = route else { return };
    let unit = routing::unit_metres(grid_to_latlon(0, 0).0);
    close(route.length, 2000.0 * unit, 20.0 * unit);
}

#[test]
fn bridge_over_road_is_not_a_junction() {
    // Мост пересекает дорогу посередине без общей вершины, и конец дороги
    // не у моста: перехода с дороги на мост нет.
    let roads = [
        minor(&[(0, 500), (1000, 500)]),
        road(&[(500, 0), (500, 1000)], &[("class", V::S("primary")), ("brunnel", V::S("bridge"))]),
    ];
    assert!(path(&roads, Mode::Car, grid_to_latlon(0, 500), grid_to_latlon(500, 0)).is_none());
}

#[test]
fn tile_seam_overlap_joins() {
    // Одна дорога, разрезанная по краю тайла с запасом: две копии
    // перекрываются, у концов копий нет общих вершин.
    let roads = [minor(&[(0, 0), (1000, 0), (1100, 0)]), minor(&[(1050, 0), (1150, 0), (2000, 0)])];
    assert!(path(&roads, Mode::Car, grid_to_latlon(10, 0), grid_to_latlon(1990, 0)).is_some());
}

#[test]
fn unreachable_point() {
    let roads = [minor(&[(0, 0), (100, 0)])];
    let far = grid_to_latlon(0, 0);
    assert!(path(&roads, Mode::Car, far, (far.0 + 1.0, far.1)).is_none());
}

#[test]
fn point_skips_island_road() {
    // Ближе к точке старта - островок дорог без выезда, как у Кремля.
    // На прежней привязке к ближайшей дороге пути нет.
    let main: Vec<(i64, i64)> = (0..4010).step_by(10).map(|x| (x, 0)).collect();
    let roads = [minor(&main), road(&[(450, 100), (550, 100), (550, 150)], &[("class", V::S("service"))])];
    assert!(path(&roads, Mode::Car, grid_to_latlon(500, 90), grid_to_latlon(3900, 0)).is_some());
}

#[test]
fn bike_mode() {
    let roads = [
        road(&[(0, 0), (1000, 0)], &[("class", V::S("trunk"))]),
        road(&[(0, 0), (0, 1000), (1000, 1000), (1000, 0)], &[("class", V::S("path"))]),
    ];
    let bike = path(&roads, Mode::Bike, grid_to_latlon(0, 0), grid_to_latlon(1000, 0));
    assert!(bike.is_some());
    let Some(bike) = bike else { return };
    let unit = routing::unit_metres(grid_to_latlon(0, 0).0);
    // По трассе велосипед не едет: в обход по тропе, 3000 единиц.
    close(bike.length, 3000.0 * unit, 30.0 * unit);
    close(bike.seconds, bike.length / (15.0 / 3.6), 1.0);
}

#[test]
fn search_in_steps_gives_the_same_route() {
    let main: Vec<(i64, i64)> = (0..4010).step_by(10).map(|x| (x, 0)).collect();
    let g = graph(&[minor(&main)], Mode::Car);
    assert!(g.is_some());
    let Some(g) = g else { return };
    let (a, b) = (grid_to_latlon(5, 0), grid_to_latlon(3995, 0));
    let whole = routing::route(&g, a, b, Mode::Car);
    assert!(whole.is_some());
    let mut search = routing::Search::new(&g, a, b, Mode::Car);
    let mut steps = 0;
    let result = loop {
        steps += 1;
        if let Step::Done(r) = search.step(&g, 50) {
            break r;
        }
    };
    // 401 узел по 50 за шаг - не меньше девяти шагов.
    assert!(steps >= 9, "{steps}");
    assert_eq!(result, whole);
    assert_eq!(search.step(&g, 50), Step::Done(whole));
}

// Сервис.

#[test]
fn url_and_parse() {
    let url = routing::service_url(routing::ROUTER_URL, Mode::Foot, (58.0, 56.2), (57.9, 56.0));
    assert!(url.starts_with("https://routing.openstreetmap.de/routed-foot/route/v1/driving/56.200000,58.000000;56.000000,57.900000?"), "{url}");
    assert!(url.contains("geometries=geojson"));
    let reply = br#"{"code": "Ok", "routes": [{"distance": 2500.5, "duration": 300.0,
        "geometry": {"type": "LineString", "coordinates": [[56.2, 58.0], [56.2, 58.0], [56.0, 57.9]]}}]}"#;
    let route = routing::parse_service(reply).unwrap_or_default();
    assert_eq!(route.points, [(58.0, 56.2), (57.9, 56.0)]);
    assert_eq!((route.length, route.seconds), (2500.5, 300.0));
    assert_eq!(routing::parse_service(br#"{"code": "NoRoute", "routes": []}"#), Err("NoRoute".to_string()));
    assert_eq!(routing::parse_service(b"null"), Err("no data".to_string()));
    assert_eq!(routing::parse_service(b"not json"), Err("no data".to_string()));
    assert_eq!(routing::parse_service(br#"{"code": "Ok", "routes": []}"#), Err("NoRoute".to_string()));
}

#[test]
fn template_check() {
    assert!(routing::router_ok(routing::ROUTER_URL));
    assert!(routing::router_ok("http://localhost:5000/route/v1/driving/{coords}"));
    assert!(!routing::router_ok("ftp://x/{coords}"));
    assert!(!routing::router_ok("https://x/route"));
    assert_eq!(Mode::from_name("bike"), Some(Mode::Bike));
    assert!(routing::MODES.iter().all(|m| Mode::from_name(m.name()) == Some(*m)));
}

// Коридор.

#[test]
fn tiles_along_line() {
    let (a, b) = ((58.0105, 56.2294), (58.0105, 56.4));
    let keys = routing::corridor(a, b, 1500.0);
    let mut xs: Vec<u32> = keys.iter().map(|k| k.1).collect();
    xs.sort_unstable();
    xs.dedup();
    let mut ys: Vec<u32> = keys.iter().map(|k| k.2).collect();
    ys.sort_unstable();
    ys.dedup();
    // Коридор - тайлы, задетые точками, и запас в тайл по краям.
    assert!(KEY.1 - 2 <= xs[0] && xs[0] <= KEY.1, "{xs:?}");
    // 0.17° долготы у Перми - около 10 км, тайл - около 1.3 км.
    assert!(xs.len() >= 8);
    assert!(ys.len() <= 5);
    assert!(keys.iter().all(|k| k.0 == 14));
    // Точка без полуширины - свой тайл и соседи, чей центр ближе
    // трёх четвертей тайла.
    let alone = routing::corridor(a, a, 0.0);
    assert!(alone.contains(&KEY) && alone.len() <= 4, "{alone:?}");
    assert!(alone.iter().all(|k| k.1.abs_diff(KEY.1) <= 1 && k.2.abs_diff(KEY.2) <= 1));
}

/// Коридор перебором всех тайлов рамки, как `corridor` модуля.
fn corridor_by_force(a: LatLon, b: LatLon, half: f64) -> Vec<(u32, u32, u32)> {
    let (ax, ay) = routing::to_grid(a.0, a.1);
    let (bx, by) = routing::to_grid(b.0, b.1);
    let e = E as f64;
    let per = routing::unit_metres(0.5 * (a.0 + b.0));
    let reach = half / per + e;
    let limit = half / per + e * 0.75;
    let tile = |v: f64| (v / e).floor() as i64;
    let mut keys = Vec::new();
    for y in tile(ay.min(by) - reach)..=tile(ay.max(by) + reach) {
        for x in tile(ax.min(bx) - reach)..=tile(ax.max(bx) + reach) {
            let (cx, cy) = ((x as f64 + 0.5) * e, (y as f64 + 0.5) * e);
            let d = (bx - ax, by - ay);
            let t = (((cx - ax) * d.0 + (cy - ay) * d.1) / (d.0 * d.0 + d.1 * d.1).max(1e-12)).clamp(0.0, 1.0);
            if (cx - ax - t * d.0).hypot(cy - ay - t * d.1) <= limit {
                keys.push((14, x as u32, y as u32));
            }
        }
    }
    for (x, y) in [(ax, ay), (bx, by)] {
        keys.push((14, tile(x) as u32, tile(y) as u32));
    }
    keys.sort_unstable();
    keys.dedup();
    keys
}

#[test]
fn corridor_matches_full_search() {
    let a = (58.0105, 56.2294);
    for (b, half) in [((58.0105, 56.4), 1500.0), ((57.9, 56.35), 3000.0), ((58.2, 56.2294), 800.0), ((58.05, 56.3), 0.0)] {
        assert_eq!(routing::corridor(a, b, half), corridor_by_force(a, b, half), "{b:?} {half}");
    }
}
