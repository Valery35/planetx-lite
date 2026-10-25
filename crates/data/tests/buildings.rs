// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! 3D-здания: разбор слоя building, обрезка, крыши, сетка, тайлы у глаза.
//! Перенос `tests/test_buildings.py` модуля.
//!
//! Данные - слой building двух тайлов OpenFreeMap уровня 14 от
//! 29 сентября 2026 года, центр Перми и Берлин-Митте, из папки тестов
//! модуля. © OpenMapTiles, © участники OpenStreetMap, ODbL. Без папки
//! проверки по тайлам пропускаются.

use std::collections::{HashMap, HashSet};

use data::buildings::{self as bd, Footprint, Mesh};
use data::mvt;
use data::subsurface::{ecef_to_geodetic, geodetic_to_ecef, surface_normal};

const DATA: &str = r"C:\Dev\planetx\planetx\tests\data";
const PERM: (u32, u32, u32) = (14, 10751, 4933);
const BERLIN: (u32, u32, u32) = (14, 8802, 5373);

type P = [f64; 2];

fn load(name: &str, key: (u32, u32, u32)) -> Option<Vec<u8>> {
    let path = format!(r"{DATA}\building_{name}_{}_{}_{}.mvt", key.0, key.1, key.2);
    let bytes = std::fs::read(path).ok();
    if bytes.is_none() {
        eprintln!("тайла модуля {name} нет, проверка пропущена");
    }
    bytes
}

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

/// Площади треугольников со знаком.
fn tri_area(points: &[P], indices: &[u32]) -> Vec<f64> {
    indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| points[i as usize]);
            bd::orient(a, b, c) * 0.5
        })
        .collect()
}

/// Кольцо против часовой стрелки в координатах тайла.
fn square(x0: f64, y0: f64, size: f64) -> Vec<P> {
    vec![[x0, y0], [x0 + size, y0], [x0 + size, y0 + size], [x0, y0 + size]]
}

fn reversed(mut ring: Vec<P>) -> Vec<P> {
    ring.reverse();
    ring
}

fn footprint(key: (u32, u32, u32), data: &[u8]) -> Option<Footprint> {
    let fp = bd::footprints(key, data);
    assert!(fp.is_ok(), "{fp:?}");
    fp.ok().flatten()
}

// Тайл со слоем building.

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

fn zz(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// Тайл со слоем building: многоугольники - списки колец.
fn tile(polygons: &[Vec<Vec<P>>], height: u64) -> Vec<u8> {
    let mut features = Vec::new();
    for rings in polygons {
        let mut geometry = Vec::new();
        let (mut x, mut y) = (0i64, 0i64);
        for ring in rings {
            let ring: Vec<(i64, i64)> = ring.iter().map(|p| (p[0] as i64, p[1] as i64)).collect();
            geometry.extend([9, zz(ring[0].0 - x), zz(ring[0].1 - y)]);
            (x, y) = ring[0];
            geometry.push(((ring.len() as u64 - 1) << 3) | 2);
            for &(px, py) in &ring[1..] {
                geometry.extend([zz(px - x), zz(py - y)]);
                (x, y) = (px, py);
            }
            geometry.push(15);
        }
        let mut packed = Vec::new();
        for v in geometry {
            varint(v, &mut packed);
        }
        let mut feature = Vec::new();
        bytes_field(2, &[0, 0], &mut feature);
        varint_field(3, 3, &mut feature);
        bytes_field(4, &packed, &mut feature);
        bytes_field(2, &feature, &mut features);
    }
    let mut value = Vec::new();
    varint_field(5, height, &mut value);
    let mut layer = Vec::new();
    varint_field(15, 2, &mut layer);
    bytes_field(1, b"building", &mut layer);
    layer.extend_from_slice(&features);
    bytes_field(3, b"render_height", &mut layer);
    bytes_field(4, &value, &mut layer);
    varint_field(5, 4096, &mut layer);
    let mut out = Vec::new();
    bytes_field(3, &layer, &mut out);
    out
}

// Разбор.

#[test]
fn perm_tile() {
    let Some(data) = load("perm", PERM) else { return };
    let decoded = bd::decode(&data);
    assert!(decoded.is_ok());
    let Ok((buildings, extent)) = decoded else { return };
    assert_eq!(extent, 4096);
    // Проба 29 сентября 2026 года: 627 контуров и 4 двора.
    assert_eq!(buildings.len(), 627);
    assert_eq!(buildings.iter().map(|b| b.rings.len() - 1).sum::<usize>(), 4);
    for b in &buildings {
        assert!(bd::area(&b.rings[0]) > 0.0);
        for hole in &b.rings[1..] {
            assert!(bd::area(hole) < 0.0);
        }
        assert!(b.height > b.base);
    }
}

#[test]
fn heights_follow_levels() {
    let Some(data) = load("perm", PERM) else { return };
    let (buildings, _) = bd::decode(&data).unwrap_or_default();
    let heights: Vec<f64> = buildings.iter().map(|b| b.height).collect();
    // Этажи × 3.66 с округлением вверх и 5 м по умолчанию.
    for h in [4.0, 5.0, 8.0, 15.0, 19.0] {
        assert!(heights.contains(&h), "{h}");
    }
}

#[test]
fn hide_3d_is_skipped() {
    let Some(data) = load("berlin", BERLIN) else { return };
    let (buildings, _) = bd::decode(&data).unwrap_or_default();
    assert!(!buildings.is_empty());
    let names = HashSet::from([bd::LAYER]);
    let layers: HashMap<String, mvt::Layer> = mvt::decode_shapes(&data, &names, None).unwrap_or_default();
    let Some(layer) = layers.get(bd::LAYER) else {
        assert!(layers.contains_key(bd::LAYER));
        return;
    };
    let total = layer.features.len();
    assert!(layer.features.iter().any(|f| f.attrs.contains_key("hide_3d")));
    let hidden = layer.features.iter().filter(|f| f.attrs.get("hide_3d").is_some_and(bd::truthy)).count();
    assert!(hidden > 0);
    assert!(hidden < total);
}

#[test]
fn parse_color() {
    assert_eq!(bd::parse_color("#ff8000"), Some([255, 128, 0]));
    assert_eq!(bd::parse_color("#abc"), Some([170, 187, 204]));
    assert_eq!(bd::parse_color("red"), None);
    assert_eq!(bd::parse_color(""), None);
    assert_eq!(bd::parse_color("#ЖЖЖ"), None);
}

#[test]
fn overture_heights() {
    // Схема OpenMapTiles главнее, у Overture - высота, иначе этажи.
    let attrs = |pairs: &[(&str, f64)]| -> HashMap<String, mvt::Value> { pairs.iter().map(|(k, v)| (k.to_string(), mvt::Value::Float(*v))).collect() };
    assert_eq!(bd::heights(&attrs(&[("render_height", 20.0), ("render_min_height", 5.0), ("height", 99.0)])), (20.0, 5.0));
    assert_eq!(bd::heights(&attrs(&[("height", 31.5), ("min_height", 3.0)])), (31.5, 3.0));
    let (top, base) = bd::heights(&attrs(&[("num_floors", 4.0), ("min_floor", 1.0)]));
    assert!((top - 4.0 * bd::FLOOR).abs() < 1e-9 && (base - bd::FLOOR).abs() < 1e-9);
    assert_eq!(bd::heights(&HashMap::new()), (bd::DEFAULT_HEIGHT, 0.0));
}

#[test]
fn hostile_tiles_do_not_break() {
    // Обрывки и мусор - ошибка разбора или пустой тайл, не падение.
    let good = tile(&[vec![square(10.0, 10.0, 20.0)]], 10);
    for cut in 0..good.len() {
        let _ = bd::footprints((14, 0, 0), &good[..cut]);
    }
    assert!(bd::footprints((14, 0, 0), &[0xff, 0xff, 0xff]).is_err());
    assert!(matches!(bd::footprints((14, 0, 0), &[]), Ok(None)));
    // Кольцо из одной точки и нулевой площади пропускается.
    let line = tile(&[vec![vec![[5.0, 5.0], [6.0, 6.0], [7.0, 7.0]]]], 10);
    assert!(matches!(bd::footprints((14, 0, 0), &line), Ok(None)));
}

// Обрезка.

#[test]
fn inside_ring_is_kept() {
    let ring = square(10.0, 10.0, 20.0);
    assert_eq!(bd::clip(&ring, 4096.0), Some(ring));
}

#[test]
fn ring_over_edge_is_cut() {
    let out = bd::clip(&square(-30.0, 100.0, 60.0), 4096.0);
    assert!(out.is_some());
    let Some(out) = out else { return };
    let least = out.iter().fold(f64::INFINITY, |a, p| a.min(p[0]));
    assert_eq!(least, 0.0);
    close(bd::area(&out), 30.0 * 60.0, 1e-9);
}

#[test]
fn ring_outside_is_dropped() {
    assert_eq!(bd::clip(&square(-100.0, 10.0, 50.0), 4096.0), None);
}

#[test]
fn no_wall_on_tile_edge() {
    let fp = footprint((14, 0, 0), &tile(&[vec![square(-30.0, 100.0, 60.0)]], 10));
    assert!(fp.is_some());
    let Some(fp) = fp else { return };
    let on_edge = (0..fp.latlon.len()).filter(|&k| fp.latlon[k][1] == fp.latlon[fp.next[k]][1] && !fp.wall[k]).count();
    assert_eq!(fp.wall.iter().filter(|w| !**w).count(), 1);
    assert_eq!(on_edge, 1);
}

// Крыши.

fn check(rings: &[Vec<P>]) {
    let points: Vec<P> = rings.concat();
    let mut index = Vec::new();
    let mut start = 0;
    for r in rings {
        index.push((start..start + r.len()).collect::<Vec<usize>>());
        start += r.len();
    }
    let tri = bd::roof(&points, &index);
    let areas = tri_area(&points, &tri);
    assert!(areas.iter().all(|a| *a >= -1e-9));
    close(areas.iter().sum(), rings.iter().map(|r| bd::area(r)).sum(), 1e-6);
}

#[test]
fn concave() {
    check(&[vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [5.0, 4.0], [0.0, 10.0]]]);
}

#[test]
fn comb() {
    let mut teeth = Vec::new();
    for k in 0..20 {
        teeth.push([f64::from(k * 2), 10.0]);
        teeth.push([f64::from(k * 2 + 1), 1.0]);
    }
    teeth.reverse();
    teeth.pop();
    let mut ring = vec![[0.0, 0.0], [40.0, 0.0], [40.0, 10.0]];
    ring.extend(teeth);
    if bd::area(&ring) < 0.0 {
        ring.reverse();
    }
    check(&[ring]);
}

#[test]
fn courtyards() {
    let outer = square(0.0, 0.0, 100.0);
    let holes = [reversed(square(10.0, 10.0, 20.0)), reversed(square(60.0, 50.0, 30.0))];
    check(&[outer, holes[0].clone(), holes[1].clone()]);
}

#[test]
fn bridge_not_through_vertex() {
    // Здание Берлина: мост от двора прошёл через его вершину (892, 3323),
    // нарезка встала, веер лёг за контур.
    let points: Vec<P> = vec![
        [858.0, 3308.0],
        [879.0, 3287.0],
        [921.0, 3350.0],
        [889.0, 3370.0],
        [871.0, 3339.0],
        [874.0, 3338.0],
        [876.0, 3333.0],
        [888.0, 3316.0],
        [892.0, 3323.0],
        [898.0, 3318.0],
        [894.0, 3312.0],
    ];
    let outer: Vec<usize> = (0..7).collect();
    let hole: Vec<usize> = (7..11).collect();
    let ring = bd::bridge(&points, &outer, &[hole]);
    let tri = bd::cut(&points, &ring);
    let areas = tri_area(&points, &tri);
    assert!(areas.iter().all(|a| *a > -1e-9));
    close(areas.iter().sum(), bd::area(&points[..7]) + bd::area(&points[7..]), 1e-7);
}

/// Кольца тайла после обрезки, как их кладёт footprints, и площадь
/// крыш по треугольникам: (сумма, наименьшая, ожидаемая).
fn roof_area(key: (u32, u32, u32), data: &[u8]) -> (f64, f64, f64, usize) {
    let (buildings, extent) = bd::decode(data).unwrap_or_default();
    let mut xy: Vec<P> = Vec::new();
    let mut expected = 0.0;
    for b in &buildings {
        let mut rings = Vec::new();
        for (number, r) in b.rings.iter().enumerate() {
            match bd::clip(r, f64::from(extent)) {
                Some(r) => rings.push(r),
                None if number == 0 => break,
                None => {}
            }
        }
        expected += rings.iter().map(|r| bd::area(r)).sum::<f64>();
        xy.extend(rings.concat());
    }
    let Some(fp) = footprint(key, data) else {
        return (0.0, 0.0, expected, 0);
    };
    let areas = tri_area(&xy, &fp.roof);
    let least = areas.iter().fold(f64::INFINITY, |a, v| a.min(*v));
    (areas.iter().sum(), least, expected, fp.roof.len())
}

#[test]
fn real_tiles() {
    for (name, key) in [("perm", PERM), ("berlin", BERLIN)] {
        let Some(data) = load(name, key) else { return };
        // Площадь в единицах тайла, по точкам в градусах не считается.
        let (got, least, expected, count) = roof_area(key, &data);
        close(got / expected, 1.0, 1e-6);
        // Сумма со знаком сходится и у веера за контуром, поэтому
        // вывернутых треугольников быть не должно.
        assert!(least > -1e-6, "{name} {least}");
        assert_eq!(count % 3, 0);
    }
}

#[test]
fn speed() {
    // Отсечение ушей из core.features модуля резало берлинский тайл 356 мс,
    // footprints - 57 мс, 29 сентября 2026 года.
    let Some(data) = load("berlin", BERLIN) else { return };
    let mut best = f64::INFINITY;
    for _ in 0..3 {
        let start = std::time::Instant::now();
        let fp = bd::footprints(BERLIN, &data);
        best = best.min(start.elapsed().as_secs_f64());
        assert!(matches!(fp, Ok(Some(_))));
    }
    assert!(best < 0.15, "{best}");
}

// Сетка.

fn perm_footprint() -> Option<Footprint> {
    footprint(PERM, &load("perm", PERM)?)
}

#[test]
fn counts() {
    let Some(fp) = perm_footprint() else { return };
    let m = bd::mesh(&fp, None);
    assert_eq!(m.vertices.len(), bd::vertex_count(&fp));
    assert_eq!(m.indices.len() % 3, 0);
    assert!(m.indices.iter().all(|i| (*i as usize) < m.vertices.len()));
}

#[test]
fn precision() {
    let Some(fp) = perm_footprint() else { return };
    let slope = |lats: &[f64], _lons: &[f64]| -> Vec<f64> { lats.iter().map(|la| 100.0 + (la - 58.0) * 5000.0).collect() };
    let m = bd::mesh(&fp, Some(&slope));
    let top = m.vertices.len() - fp.latlon.len();
    let lats: Vec<f64> = fp.latlon.iter().map(|p| p[0]).collect();
    let ground = slope(&lats, &[]);
    let mut low = vec![f64::INFINITY; fp.height.len()];
    for (k, o) in fp.owner.iter().enumerate() {
        low[*o] = low[*o].min(ground[k]);
    }
    let mut worst: f64 = 0.0;
    for (k, v) in m.vertices[top..].iter().enumerate() {
        let p = [0, 1, 2].map(|a| m.center[a] + f64::from(v.position[a]));
        let h = ecef_to_geodetic(p)[2];
        let o = fp.owner[k];
        worst = worst.max((h - (low[o] + fp.height[o])).abs());
    }
    assert!(worst < 0.01, "{worst}");
}

#[test]
fn mesh_ok() {
    let Some(fp) = perm_footprint() else { return };
    let m = bd::mesh(&fp, None);
    assert!(bd::mesh_ok(&m));
    let mut beyond = m.clone();
    beyond.indices[5] = u32::try_from(m.vertices.len()).unwrap_or(u32::MAX);
    assert!(!bd::mesh_ok(&beyond));
    let mut short = m.clone();
    short.indices.pop();
    assert!(!bd::mesh_ok(&short));
    let mut broken: Mesh = m.clone();
    broken.vertices[3].position[1] = f32::NAN;
    assert!(!bd::mesh_ok(&broken));
}

#[test]
fn walls_face_out() {
    let fp = footprint((14, 0, 0), &tile(&[vec![square(100.0, 100.0, 50.0)]], 10));
    assert!(fp.is_some());
    let Some(fp) = fp else { return };
    let m = bd::mesh(&fp, None);
    let walls = m.vertices[..16].as_chunks::<4>().0;
    let middle: Vec<[f64; 3]> = walls
        .iter()
        .map(|w| [0, 1, 2].map(|a| w.iter().map(|v| f64::from(v.position[a])).sum::<f64>() / 4.0))
        .collect();
    let center = [0, 1, 2].map(|a| middle.iter().map(|m| m[a]).sum::<f64>() / 4.0);
    for (w, mid) in walls.iter().zip(&middle) {
        let normal = [0, 1, 2].map(|a| f64::from(w[0].normal[a]));
        let out: f64 = (0..3).map(|a| (mid[a] - center[a]) * normal[a]).sum();
        assert!(out > 0.0);
    }
}

#[test]
fn bad_footprint_gives_empty_mesh() {
    let Some(mut fp) = footprint((14, 0, 0), &tile(&[vec![square(100.0, 100.0, 50.0)]], 10)) else { return };
    fp.owner[0] = 99;
    let m = bd::mesh(&fp, None);
    assert!(m.vertices.is_empty() && m.indices.is_empty());
}

// Тайлы у глаза.

/// Глаз и направление взгляда камеры, которая смотрит на точку
/// с расстояния distance, как `Camera.look_at` модуля.
fn eye(distance: f64, heading: f64, tilt: f64) -> ([f64; 3], [f64; 3]) {
    let (lat, lon) = (58.0105, 56.2294);
    let (sla, cla) = f64::to_radians(lat).sin_cos();
    let (slo, clo) = f64::to_radians(lon).sin_cos();
    let east = [-slo, clo, 0.0];
    let north = [-sla * clo, -sla * slo, cla];
    let up = surface_normal(lat, lon);
    let (psi, tau) = (heading.to_radians(), tilt.to_radians());
    let screen_up = [0, 1, 2].map(|a| north[a] * psi.cos() + east[a] * psi.sin());
    let forward = [0, 1, 2].map(|a| -up[a] * tau.cos() + screen_up[a] * tau.sin());
    let target = geodetic_to_ecef(lat, lon, 0.0);
    ([0, 1, 2].map(|a| target[a] - forward[a] * distance), forward)
}

#[test]
fn under_eye_first() {
    let (e, f) = eye(1500.0, 0.0, 0.0);
    let near = bd::near_tiles(e, f, 0.0, bd::RANGE, bd::MAX_TILES);
    assert_eq!(near.first().map(|n| n.0), Some(PERM));
    assert!(near.iter().all(|(_, d)| *d < bd::RANGE));
    assert!(near.len() <= bd::MAX_TILES);
    assert!(near.windows(2).all(|w| w[0].1 <= w[1].1));
}

#[test]
fn too_high() {
    let (e, f) = eye(bd::RANGE + 100.0, 0.0, 0.0);
    assert!(bd::near_tiles(e, f, 0.0, bd::RANGE, bd::MAX_TILES).is_empty());
}

#[test]
fn ground_counts() {
    // Над плато в 2 км камера в 7.5 км над эллипсоидом ещё видит дома.
    let (e, f) = eye(7500.0, 0.0, 0.0);
    assert!(!bd::near_tiles(e, f, 2000.0, bd::RANGE, bd::MAX_TILES).is_empty());
}

#[test]
fn behind_is_skipped() {
    let (e, f) = eye(300.0, 0.0, 80.0);
    let keys: Vec<(u32, u32, u32)> = bd::near_tiles(e, f, 0.0, bd::RANGE, 1000).into_iter().map(|(k, _)| k).collect();
    let y = PERM.2;
    let south = keys.iter().filter(|k| k.2 > y + 1).count();
    let north = keys.iter().filter(|k| k.2 + 1 < y).count();
    assert!(north > 0);
    assert_eq!(south, 0);
}

#[test]
fn hostile_eye() {
    assert!(bd::near_tiles([f64::NAN; 3], [0.0, 0.0, 1.0], 0.0, bd::RANGE, bd::MAX_TILES).is_empty());
    let (e, f) = eye(1500.0, 0.0, 0.0);
    assert!(bd::near_tiles(e, f, 0.0, 1.0e9, bd::MAX_TILES).len() <= bd::MAX_TILES);
}
