// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сетки подземного режима: трубка, горизонт, стенка, вырез. Перенос
//! `tests/test_subsurface.py` модуля.

use data::subsurface::{self as ss, Grid, Mask, Part, Side};

const LAT: f64 = 58.0;
const LON: f64 = 56.0;

fn close(a: f64, b: f64, eps: f64) {
    assert!((a - b).abs() <= eps, "{a} != {b}");
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn wide(n: [f32; 3]) -> [f64; 3] {
    [f64::from(n[0]), f64::from(n[1]), f64::from(n[2])]
}

/// Узлы сетки 3 × 3 через 0.001° на высоте alt, строки - широта.
fn nodes(alt: f64) -> Vec<[f64; 3]> {
    let mut lats = Vec::new();
    let mut lons = Vec::new();
    for i in 0..3 {
        for j in 0..3 {
            lats.push(LAT + i as f64 * 0.001);
            lons.push(LON + j as f64 * 0.001);
        }
    }
    ss::ecef(&lats, &lons, &[alt; 9])
}

// Трубка.

#[test]
fn tube_radius_and_closure() {
    let points = ss::ecef(&[LAT, LAT, LAT + 0.001], &[LON, LON, LON], &[100.0, -100.0, -200.0]);
    let part = ss::tube(&points, 5.0, [255, 0, 0, 255], ss::SIDES);
    assert!(part.is_some());
    let Some(part) = part else { return };
    assert_eq!(part.positions.len(), 3 * ss::SIDES);
    // Каждая вершина кольца - на радиусе от своей точки оси.
    for (ring, center) in part.positions.chunks(ss::SIDES).zip(&points) {
        for p in ring {
            close(norm(sub(*p, *center)), 5.0, 1e-6);
        }
    }
    // Две грани на сторону сечения на каждом из двух отрезков.
    assert_eq!(part.indices.len(), 2 * 2 * ss::SIDES);
    let top = part.indices.iter().flatten().copied().max().unwrap_or_default();
    assert!((top as usize) < part.positions.len());
}

#[test]
fn tube_matches_module() {
    // Вершины и треугольники трубки модуля PlanetX для QGIS.
    let points = ss::ecef(&[LAT, LAT, LAT + 0.001], &[LON, LON, LON], &[100.0, -100.0, -200.0]);
    let part = ss::tube(&points, 5.0, [1, 2, 3, 255], ss::SIDES);
    let Some(part) = part else { return };
    let want = [(0, [1894614.8015717056, 2808873.0137184714, 5385822.528306497]), (9, [
        1894553.1148758344,
        2808784.1783233825,
        5385655.718620414,
    ])];
    for (i, p) in want {
        for (got, want) in part.positions[i].iter().zip(p) {
            close(*got, want, 1e-6);
        }
    }
    assert_eq!(part.indices[..3], [[0, 1, 8], [1, 9, 8], [1, 2, 9]]);
}

// Горизонт.

#[test]
fn triangles_and_edges_match_module() {
    // Сетка 3 × 4 без узлов (1, 1) и (2, 3): порядок треугольников и рёбер
    // края модуля PlanetX для QGIS.
    let mut mask = Mask::full(3, 4, true);
    mask.valid[5] = false;
    mask.valid[11] = false;
    let tris = ss::grid_triangles(&mask);
    assert_eq!(tris, [[2, 3, 6], [3, 7, 6], [0, 1, 4], [6, 7, 10], [6, 10, 9], [4, 9, 8], [1, 2, 6]]);
    let edges = ss::boundary_edges(&mask);
    let want = [
        [2, 3],
        [3, 7],
        [0, 1],
        [4, 9],
        [1, 2],
        [1, 4],
        [7, 10],
        [10, 9],
        [9, 8],
        [4, 0],
        [9, 6],
        [8, 4],
        [6, 1],
    ];
    assert_eq!(edges, want);
}

#[test]
fn hole_cells_skipped() {
    let points = nodes(-50.0);
    let mut mask = Mask::full(3, 3, true);
    let full = ss::grid_surface(&points, &mask, &[[0, 200, 0, 255]]);
    assert!(full.is_some());
    let Some(full) = full else { return };
    assert_eq!(full.indices.len(), 8);
    // Средний узел пуст - у ячеек по треугольнику.
    mask.valid[4] = false;
    let holed = ss::grid_surface(&points, &mask, &[[0, 200, 0, 255]]);
    assert_eq!(holed.map(|p| p.indices.len()), Some(4));
    // Два узла - ни одного треугольника.
    let mut two = Mask::full(3, 3, false);
    two.valid[0] = true;
    two.valid[1] = true;
    assert!(ss::grid_surface(&points, &two, &[[0, 200, 0, 255]]).is_none());
    // Нормали смотрят вверх, от центра Земли.
    let up = ss::surface_normal(LAT, LON);
    assert!(dot(wide(full.normals[4]), up) > 0.99);
}

#[test]
fn walls_follow_outer_edges() {
    let top = nodes(-50.0);
    let bottom = nodes(-60.0);
    let mut mask = Mask::full(3, 3, true);
    let part = ss::grid_walls(&top, &bottom, &mask, [10, 20, 30, 255], &[]);
    assert!(part.is_some());
    let Some(part) = part else { return };
    // Край 2 × 2 ячеек - 8 рёбер, на ребро два треугольника.
    assert_eq!(part.indices.len(), 16);
    // Стенка вертикальна: нормаль поперёк вертикали.
    let up = ss::surface_normal(LAT, LON);
    assert!(dot(wide(part.normals[0]), up).abs() < 0.01);
    // Средний узел пуст - край идёт и по косым рёбрам вокруг него.
    mask.valid[4] = false;
    let part = ss::grid_walls(&top, &bottom, &mask, [10, 20, 30, 255], &[]);
    assert_eq!(part.map(|p| p.indices.len()), Some(2 * (8 + 4)));
}

#[test]
fn skipped_edges_have_no_wall() {
    // Ребро из skip, в любом порядке концов, стенки не получает.
    let top = nodes(-50.0);
    let bottom = nodes(-60.0);
    let mask = Mask::full(3, 3, true);
    let edges = ss::boundary_edges(&mask);
    assert_eq!(edges.len(), 8);
    let Some(&[a, b]) = edges.first() else { return };
    let part = ss::grid_walls(&top, &bottom, &mask, [10, 20, 30, 255], &[[b, a]]);
    assert_eq!(part.map(|p| p.indices.len()), Some(14));
}

#[test]
fn block_walls_darken_with_depth() {
    let surfaces = vec![nodes(-50.0), nodes(-60.0), nodes(-80.0)];
    let mask = Mask::full(3, 3, true);
    let walls = ss::block_walls(&surfaces, &mask, &[[200, 200, 200, 255], [100, 100, 100, 255]]);
    assert_eq!(walls.len(), 2);
    let Some(first) = walls.first() else { return };
    // Верх блока - цвет как есть, низ - темнее на BLOCK_FADE.
    assert_eq!(first.colors.first(), Some(&[200, 200, 200, 255]));
    let Some(last) = walls.last() else { return };
    let low = (100.0 * (1.0 - ss::BLOCK_FADE)) as u8;
    assert!(last.colors.iter().any(|c| c[0] == low));
}

// Коробка фона.

#[test]
fn box_stays_under_frame() {
    // Фон - коробка под рамкой модели, а не весь кадр: все вершины в рамке,
    // между рельефом и низом модели.
    let bounds = [59.5, 59.6, 56.7, 56.9];
    let tris = ss::backdrop_box(bounds, |la, _| vec![150.0; la.len()], -200.0, 4, ss::BACKDROP_DENSE);
    assert_eq!(tris.len(), 4 * 4 * 2 * 2 + 4 * 4 * 2);
    let mut high = f64::NEG_INFINITY;
    let mut low = f64::INFINITY;
    for p in tris.iter().flatten() {
        let [lat, lon, alt] = ss::ecef_to_geodetic(*p);
        assert!(lat > 59.5 - 1e-6 && lat < 59.6 + 1e-6, "{lat}");
        assert!(lon > 56.7 - 1e-6 && lon < 56.9 + 1e-6, "{lon}");
        high = high.max(alt);
        low = low.min(alt);
    }
    close(high, 150.0, 0.01);
    close(low, -200.0, 0.01);
}

#[test]
fn top_covers_hill_between_nodes() {
    // Узкий холм между узлами сетки коробки: верх коробки над ним не ниже
    // холма. По рельефу только в узлах верх был 100 м.
    let bounds = [59.5, 59.6, 56.7, 56.9];
    let hill = |lats: &[f64], lons: &[f64]| -> Vec<f64> {
        lats.iter()
            .zip(lons)
            .map(|(la, lo)| if (la - 59.5625).abs() < 0.004 && (lo - 56.7625).abs() < 0.004 { 400.0 } else { 100.0 })
            .collect()
    };
    let tris = ss::backdrop_box(bounds, hill, -200.0, 4, ss::BACKDROP_DENSE);
    let mut near = f64::NEG_INFINITY;
    let mut far = f64::NEG_INFINITY;
    for p in tris.iter().flatten() {
        let [lat, lon, alt] = ss::ecef_to_geodetic(*p);
        if (lat - 59.5625).abs() < 0.03 && (lon - 56.7625).abs() < 0.06 {
            near = near.max(alt);
        }
        if lat > 59.59 && lon > 56.88 {
            far = far.max(alt);
        }
    }
    assert!(near > 399.0, "{near}");
    // Вдали от холма верх прежний.
    close(far, 100.0, 0.01);
}

// Многоугольник слоя с Z: вертикальный забор разреза Isoliner3D.

#[test]
fn vertical_fence_quad() {
    let lats = [LAT, LAT + 0.001, LAT + 0.001, LAT];
    let lons = [LON, LON + 0.001, LON + 0.001, LON];
    let alts = [-60.0, -62.0, -72.0, -70.0];
    let mut xyz = ss::ecef(&lats, &lons, &alts);
    let Some(&first) = xyz.first() else { return };
    xyz.push(first);
    let part = ss::polygon3d(&xyz, [154, 105, 227, 255]);
    assert!(part.is_some());
    let Some(part) = part else { return };
    assert_eq!(part.positions.len(), 4);
    assert_eq!(part.indices.len(), 2);
    // Нормаль горизонтальна: перпендикулярна направлению вверх.
    let mut mean = [0.0; 3];
    for p in &part.positions {
        for k in 0..3 {
            mean[k] += p[k] / 4.0;
        }
    }
    let size = norm(mean);
    let up = [mean[0] / size, mean[1] / size, mean[2] / size];
    assert!(dot(wide(part.normals[0]), up).abs() < 0.05);
    // Площадь двух треугольников - площадь четырёхугольника.
    let p = &part.positions;
    let area: f64 = part
        .indices
        .iter()
        .map(|t| {
            let (a, b, c) = (p[t[0] as usize], p[t[1] as usize], p[t[2] as usize]);
            norm(cross(sub(b, a), sub(c, a))) / 2.0
        })
        .sum();
    let side = norm(sub(p[1], p[0]));
    close(area, side * 10.0, side * 0.5);
}

#[test]
fn degenerate_is_none() {
    let xyz = ss::ecef(&[LAT; 3], &[LON; 3], &[0.0; 3]);
    assert!(ss::polygon3d(&xyz, [0, 0, 0, 255]).is_none());
    assert!(ss::polygon3d(&xyz[..2], [0, 0, 0, 255]).is_none());
}

// Стенка.

#[test]
fn wall_between_planes() {
    let part = ss::wall(&[LAT; 3], &[LON, LON + 0.01, LON + 0.02], &[0.0; 3], &[-30.0; 3], [10, 20, 30, 255]);
    assert!(part.is_some());
    let Some(part) = part else { return };
    let (top, bottom) = part.positions.split_at(3);
    for (t, b) in top.iter().zip(bottom) {
        close(norm(sub(*t, *b)), 30.0, 1e-6);
    }
    assert_eq!(part.indices.len(), 4);
}

#[test]
fn crossed_surfaces_give_no_wall() {
    let part = ss::wall(&[LAT, LAT], &[LON, LON + 0.01], &[-40.0, -40.0], &[-30.0, -30.0], [1, 2, 3, 255]);
    assert!(part.is_none());
}

// Сборка сетки.

#[test]
fn merge_offsets_indices() {
    let a = ss::wall(&[LAT, LAT], &[LON, LON + 0.01], &[0.0, 0.0], &[-10.0, -10.0], [1, 2, 3, 255]);
    let b = ss::wall(&[LAT, LAT], &[LON, LON + 0.01], &[-10.0, -10.0], &[-20.0, -20.0], [4, 5, 6, 255]);
    let parts = [a, b, None];
    let mesh = ss::merge(parts.iter().flatten());
    assert!(mesh.is_some());
    let Some(mesh) = mesh else { return };
    assert_eq!(mesh.vertices.len(), 8);
    assert_eq!(mesh.indices.iter().max(), Some(&7));
    let all: Vec<[f64; 3]> = parts.iter().flatten().flat_map(|p| p.positions.clone()).collect();
    for (v, p) in mesh.vertices.iter().zip(&all) {
        for ((offset, center), want) in v.position.iter().zip(mesh.center).zip(p) {
            close(f64::from(*offset) + center, *want, 0.01);
        }
    }
    // Нормаль в вершине - в i8 с масштабом 127, четвёртый байт пустой.
    let Some(first) = mesh.vertices.first() else { return };
    assert_eq!(first.normal[3], 0);
    assert_eq!(first.color, [1, 2, 3, 255]);
}

#[test]
fn merge_drops_foreign_indices() {
    // Враждебный кусок: треугольник с номером вне своих вершин в сетку
    // не попадает.
    let part = Part {
        positions: vec![[0.0, 0.0, 6.4e6], [1.0, 0.0, 6.4e6], [0.0, 1.0, 6.4e6]],
        normals: vec![[0.0, 0.0, 1.0]; 3],
        colors: vec![[1, 1, 1, 255]; 3],
        indices: vec![[0, 1, 2], [0, 1, 3]],
    };
    let mesh = ss::merge([&part]);
    assert_eq!(mesh.map(|m| m.indices), Some(vec![0, 1, 2]));
    assert!(ss::merge(Vec::<&Part>::new()).is_none());
}

// Помощники.

#[test]
fn display_scale_and_flat() {
    assert_eq!(ss::display(-100.0, 3.0, 150.0), -300.0);
    // Рельеф выключен: глубина под поверхностью.
    assert_eq!(ss::display(-100.0, 0.0, 150.0), -250.0);
}

#[test]
fn sample_grid() {
    let grid = Grid { x0: 100.0, y0: 200.0, dx: 10.0, dy: -10.0, rows: 2, cols: 2, z: vec![10.0, 20.0, 30.0, f64::NAN] };
    let got = ss::sample_grid(&grid, &[100.0, 105.0, 105.0, 50.0], &[200.0, 200.0, 195.0, 200.0]);
    assert_eq!(got[..2], [10.0, 15.0]);
    assert!(got[2].is_nan()); // рядом пустой узел
    assert!(got[3].is_nan()); // вне растра
}

#[test]
fn stack_no_crossings() {
    let out = ss::stack(&[vec![0.0, 0.0], vec![-10.0, 5.0], vec![f64::NAN, -20.0]]);
    assert_eq!(out, vec![vec![0.0, 0.0], vec![-10.0, 0.0], vec![-10.0, -20.0]]);
}

#[test]
fn densify_keeps_vertices() {
    let out = ss::densify(&[[0.0, 0.0], [10.0, 0.0], [10.0, 5.0]], 4.0);
    assert_eq!(out.first(), Some(&[0.0, 0.0]));
    assert_eq!(out.last(), Some(&[10.0, 5.0]));
    assert!(out.contains(&[10.0, 0.0]));
    let longest = out.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).fold(0.0, f64::max);
    assert!(longest <= 4.0 + 1e-9);
}

#[test]
fn densify_bad_step_keeps_line() {
    // Шаг 0 или не число - ломаная без новых точек, а не бесконечный цикл.
    let line = [[0.0, 0.0], [10.0, 0.0]];
    assert_eq!(ss::densify(&line, 0.0), line.to_vec());
    assert_eq!(ss::densify(&line, f64::NAN), line.to_vec());
}

#[test]
fn fence_and_floor() {
    let surfaces = ss::stack(&[vec![0.0, 0.0], vec![-10.0, -10.0], vec![-30.0, -30.0]]);
    let parts = ss::fence(&[LAT, LAT], &[LON, LON + 0.01], &surfaces, &[[1, 2, 3, 255], [4, 5, 6, 255]]);
    assert_eq!(parts.iter().flatten().count(), 2);
    let ring = [[LAT, LON], [LAT, LON + 0.01], [LAT + 0.01, LON + 0.01], [LAT + 0.01, LON]];
    let floor = ss::floor_part(&ring, -30.0, [9, 9, 9, 255]);
    assert_eq!(floor.map(|p| p.indices.len()), Some(2));
}

// Тоннель под рельефом: глубина по доле длины линии.

#[test]
fn along_is_share_of_length() {
    let share = ss::along(&[0.0, 0.0, 0.0], &[0.0, 0.001, 0.003]);
    assert_eq!(share.len(), 3);
    for (s, want) in share.iter().zip([0.0, 1.0 / 3.0, 1.0]) {
        close(*s, want, 1e-8);
    }
    assert_eq!(ss::along(&[1.0], &[2.0]), vec![0.0]);
}

#[test]
fn ramp_goes_from_portal_to_depth() {
    let ground = [600.0, 610.0, 620.0];
    let share = [0.0, 0.5, 1.0];
    for ((g, s), want) in ground.iter().zip(share).zip([600.0, 604.0, 608.0]) {
        close(ss::tunnel_z(*g, 0.0, 12.0, s), want, 1e-9);
        close(ss::tunnel_z(*g, 12.0, 12.0, s), g - 12.0, 1e-9);
    }
}

// Стенка разреза с картинкой вдоль линии.

#[test]
fn image_wall_spans_line_and_height() {
    let lats = [58.0, 58.0, 58.0];
    let lons = [56.0, 56.005, 56.02];
    let wall = ss::image_wall(&lats, &lons, &[100.0], &[-300.0]);
    assert!(wall.is_some());
    let Some(wall) = wall else { return };
    assert_eq!(wall.vertices.len(), 6);
    assert_eq!(wall.indices.len(), 12);
    // Картинка по длине: u в точках - доля длины, v - верх 0, низ 1.
    for (v, u) in wall.vertices.iter().zip([0.0, 0.25, 1.0]) {
        close(f64::from(v.uv[0]), u, 1e-6);
        close(f64::from(v.uv[1]), 0.0, 1e-9);
    }
    for v in &wall.vertices[3..] {
        close(f64::from(v.uv[1]), 1.0, 1e-9);
    }
    let point = |i: usize| {
        let p = wall.vertices[i].position;
        [f64::from(p[0]) + wall.center[0], f64::from(p[1]) + wall.center[1], f64::from(p[2]) + wall.center[2]]
    };
    close(norm(sub(point(0), point(3))), 400.0, 0.05);
}

// Точки модели на разрезе вдоль линии.

fn line() -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let lons = (0..101).map(|i| 56.0 + 0.1 * i as f64 / 100.0).collect();
    let lats = vec![58.0; 101];
    let distance = (0..101).map(|i| 5000.0 * i as f64 / 100.0).collect();
    (lats, lons, distance)
}

#[test]
fn along_and_offset() {
    let (lats, lons, distance) = line();
    // Точка над серединой линии, в 0.001° к северу (около 111 м).
    let got = ss::to_line(&lats, &lons, &distance, &[58.001], &[56.05]);
    assert_eq!(got.len(), 1);
    let Some(&(along, gap)) = got.first() else { return };
    close(along, 2500.0, 1e-9);
    close(gap, 111.2, 0.5);
}

// Разрез модели: пласт между кровлей и подошвой своим цветом, промежуток
// под подошвой и над первой кровлей - порода между пластами. На прежней
// раскраске промежуток под подошвой шёл цветом пласта.

#[test]
fn gap_under_bottom_is_host() {
    let colors = [("В_top", [1u8; 3]), ("В_bottom", [1; 3]), ("Г_top", [2; 3]), ("Г_bottom", [2; 3])];
    let fill_of = |c: &str| colors.iter().find(|(k, _)| *k == c).map(|(_, v)| *v).unwrap_or_default();
    let fills = ss::section_fills(&["", "В_top", "В_bottom", "Г_top", "Г_bottom"], fill_of, &[9, 9, 9]);
    let want: Vec<(String, [u8; 3])> = vec![
        (ss::HOST.into(), [9; 3]),
        ("В_top".into(), [1; 3]),
        (ss::HOST.into(), [9; 3]),
        ("Г_top".into(), [2; 3]),
        (ss::HOST.into(), [9; 3]),
    ];
    assert_eq!(fills, want);
}

#[test]
fn table_beds_keep_their_colors() {
    // Кровли таблицы пластов без окончаний: каждый промежуток - свой пласт,
    // верхний - пласт таблицы над первой кровлей.
    let fills = ss::section_fills(&["Q", "П1", "П2"], |c| [c.chars().count() as u8; 3], &[9, 9, 9]);
    let keys: Vec<&str> = fills.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["Q", "П1", "П2"]);
    assert_eq!(ss::line_color([200, 100, 20]), [110, 55, 11]);
}

#[test]
fn bed_key_sides() {
    assert_eq!(ss::bed_key("В_top"), ("В".to_string(), Some(Side::Top)));
    assert_eq!(ss::bed_key("В подошва"), ("В".to_string(), Some(Side::Bottom)));
    assert_eq!(ss::bed_key("ПКС"), ("ПКС".to_string(), None));
}

// Стенка «3D-забора» и линия с отметками на разрезе.

#[test]
fn fence_wall_along_line() {
    // Четырёхугольник пласта между 1000 и 1500 м линии, отметки кровли 100
    // и 90 м, подошвы 60 и 50 м.
    let (lats, lons, distance) = line();
    let part = [[58.0, 56.02, 100.0], [58.0, 56.03, 90.0], [58.0, 56.03, 50.0], [58.0, 56.02, 60.0]];
    let got = ss::fence_part(&part, &lats, &lons, &distance, 200.0);
    assert!(got.is_some());
    let Some((d, z)) = got else { return };
    assert_eq!(d, [1000.0, 1500.0, 1500.0, 1000.0]);
    assert_eq!(z, [100.0, 90.0, 50.0, 60.0]);
}

#[test]
fn far_part_is_skipped() {
    // Вершина в 0.002° к северу - около 220 м от линии.
    let (lats, lons, distance) = line();
    let part = [[58.0, 56.02, 100.0], [58.002, 56.03, 90.0]];
    assert!(ss::fence_part(&part, &lats, &lons, &distance, 200.0).is_none());
    assert!(ss::fence_part(&part, &lats, &lons, &distance, 500.0).is_some());
}

// Ось ствола или тоннеля под курсором для окна «Объекты».

#[test]
fn nearest_segment_and_share() {
    let pixels = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]];
    let found = ss::nearest_on_screen(&pixels, &[true; 3], 10.0, 7.0);
    assert!(found.is_some());
    let Some((gap, i, t)) = found else { return };
    assert_eq!(i, 1);
    close(gap, 0.0, 1e-7);
    close(t, 0.7, 1e-7);
    let found = ss::nearest_on_screen(&pixels, &[true; 3], 4.0, 3.0);
    let Some((gap, i, t)) = found else { return };
    assert_eq!((i, (t * 100.0).round() / 100.0), (0, 0.4));
    close(gap, 3.0, 1e-7);
}

#[test]
fn segment_behind_camera_is_skipped() {
    let pixels = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]];
    let found = ss::nearest_on_screen(&pixels, &[true, true, false], 10.0, 7.0);
    assert_eq!(found.map(|f| f.1), Some(0));
    assert!(ss::nearest_on_screen(&pixels, &[false; 3], 0.0, 0.0).is_none());
}

// Вырез.

#[test]
fn inside_square() {
    let ring = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
    let got = ss::inside(&[0.5, 1.5, 0.5], &[0.5, 0.5, -0.1], &ring);
    assert_eq!(got, [true, false, false]);
}
