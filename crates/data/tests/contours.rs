// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Горизонтали тайла и изолинии сетки. Перенос `tests/test_contours.py`
//! модуля. Случайные поверхности модуля - здесь из своего генератора,
//! проверяется то же свойство.

use data::contours::{self as ct, Glyphs, Grid, Palette};

const R: f64 = 6378137.0;

/// Плоскость: высота a + b·столбец.
fn ramp(a: f64, b: f64) -> Grid {
    Grid::from_fn(256, 256, |_, c| a + b * c as f64)
}

fn alpha(rgba: &[u8], r: usize, c: usize) -> u8 {
    rgba[(r * 256 + c) * 4 + 3]
}

fn px(rgba: &[u8], r: usize, c: usize) -> [u8; 4] {
    let i = (r * 256 + c) * 4;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

/// Цифры - сплошные прямоугольники, точка и минус уже.
fn fake_glyphs() -> Glyphs {
    let (height, width) = (7, 5);
    let mut g = Glyphs { height, ..Glyphs::default() };
    for c in "0123456789".chars() {
        g.chars.insert(c, (width, vec![1.0; height * width]));
    }
    g.chars.insert('-', (3, vec![1.0; height * 3]));
    g.chars.insert('.', (2, vec![1.0; height * 2]));
    g
}

/// Псевдослучайные числа от -1 до 1.
fn noise(seed: u64, n: usize) -> Vec<f64> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((s >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
        })
        .collect()
}

#[test]
fn nice_series() {
    assert_eq!(ct::nice(4.9), 5.0);
    assert_eq!(ct::nice(21.0), 20.0);
    assert_eq!(ct::nice(2.3), 2.5);
    assert_eq!(ct::nice(8.0), 10.0);
}

#[test]
fn tile_step_by_level() {
    // Тайлы у Перми, 58° с. ш.: уровень 15 - 5 м, 13 - 20 м.
    let lat = 58f64.to_radians();
    let y15 = ((1.0 - lat.tan().asinh() / std::f64::consts::PI) / 2.0 * f64::from(1u32 << 15)) as u32;
    assert_eq!(ct::tile_step(15, y15, R), 5.0);
    assert_eq!(ct::tile_step(13, y15 >> 2, R), 20.0);
    assert_eq!(ct::tile_step(18, 80000, R), ct::MIN_STEP);
}

#[test]
fn lines_on_slope_plane() {
    // Высота растёт на 1 м за пиксель вдоль столбцов: горизонтали через
    // 10 м - вертикальные полосы через 10 пикселей.
    let rgba = ct::contour_rgba(&ramp(0.0, 1.0), 15, 10000, R, Some(10.0), None, Palette::Map, true);
    for c in (10..250).step_by(10) {
        assert!(alpha(&rgba, 128, c) > 100, "{c}");
    }
    assert_eq!(alpha(&rgba, 128, 15), 0);
    // Утолщённая горизонталь (50 м) шире обычной (40 м).
    let width = |c: usize| (c - 3..c + 4).map(|x| u32::from(alpha(&rgba, 128, x))).sum::<u32>();
    assert!(width(50) > width(40));
}

#[test]
fn flat_lake_on_level_is_not_filled() {
    let h = Grid::from_fn(256, 256, |_, c| if c >= 200 { 100.0 + (c - 200) as f64 * 0.5 } else { 100.0 });
    let rgba = ct::contour_rgba(&h, 15, 10000, R, Some(10.0), None, Palette::Map, true);
    assert!((0..256).all(|r| (0..150).all(|c| alpha(&rgba, r, c) == 0)));
}

#[test]
fn water_color_below_zero() {
    let rgba = ct::contour_rgba(&ramp(-128.0, 1.0), 15, 10000, R, Some(10.0), None, Palette::Map, true);
    let (under, over) = (px(&rgba, 100, 118), px(&rgba, 100, 148));
    assert!(under[2] > under[0]);
    assert!(over[0] > over[2]);
}

#[test]
fn label_text() {
    assert_eq!(ct::label_text(250.0), "250");
    assert_eq!(ct::label_text(-1000.0), "-1000");
    assert_eq!(ct::label_text(112.5), "112.5");
}

#[test]
fn index_line_labelled_across_slope() {
    // Подъём на 0.5 м за пиксель, сечение 5 м: горизонтали через 10
    // пикселей, утолщённые (25 м) - через 50.
    let h = ramp(0.0, 0.5);
    let glyphs = fake_glyphs();
    let bare = ct::contour_rgba(&h, 15, 10000, R, Some(5.0), None, Palette::Map, true);
    let rgba = ct::contour_rgba(&h, 15, 10000, R, Some(5.0), Some(&glyphs), Palette::Map, true);
    let smooth = ct::smooth(&h);
    let f = ct::line_fields(&smooth, 5.0);
    let labels = ct::place_labels(&smooth, &f, 5.0, &ct::label_spots(&f, 7), &glyphs);
    assert!(!labels.is_empty());
    let changed = (0..256 * 256).filter(|i| (i32::from(rgba[i * 4 + 3]) - i32::from(bare[i * 4 + 3])).abs() > 60).count();
    assert!(changed > 30);
    for l in &labels {
        assert_eq!(l.value % 25.0, 0.0);
        let mut cols = Vec::new();
        let mut rows = Vec::new();
        for lr in 0..l.rows {
            for lc in 0..l.cols {
                if l.ink[lr * l.cols + lc] > 0.5 {
                    rows.push(l.r0 + lr);
                    cols.push(l.c0 + lc);
                }
            }
        }
        cols.sort_unstable();
        let median = cols[cols.len() / 2] as f64;
        // Подпись на утолщённой линии: столбец отметки value.
        assert!((median - l.value / 0.5).abs() < 3.0);
        // Текст вдоль линии - по строкам, поперёк - высота цифр.
        assert!(cols[cols.len() - 1] - cols[0] <= 8);
        let length = ct::label_strip(&l.text, &glyphs).map_or(0, |s| s.0);
        let (lo, hi) = (rows.iter().min().copied().unwrap_or(0), rows.iter().max().copied().unwrap_or(0));
        assert!(hi - lo + 2 >= length);
        assert_eq!(l.text, ct::label_text(l.value));
    }
}

#[test]
fn no_labels_where_lines_crowd() {
    let h = ramp(0.0, 1.0);
    let bare = ct::contour_rgba(&h, 15, 10000, R, Some(2.0), None, Palette::Map, true);
    let rgba = ct::contour_rgba(&h, 15, 10000, R, Some(2.0), Some(&fake_glyphs()), Palette::Map, true);
    assert_eq!(bare, rgba);
}

#[test]
fn plane_is_exact_inside() {
    // Плоскость на предке уровня 12 - та же плоскость на внуке 14.
    let parent = Grid::from_fn(256, 256, |r, c| 100.0 + 2.0 * c as f64 + 3.0 * r as f64);
    let child = ct::upsample(&parent, (12, 10, 20), (14, 41, 82), None, false);
    for (r, c) in [(0, 0), (17, 200), (255, 255), (128, 3)] {
        let pos = |i: usize| (i as f64 + 0.5) / 4.0 - 0.5;
        let want = 100.0 + 2.0 * (64.0 + pos(c)) + 3.0 * (128.0 + pos(r));
        assert!((child.at(r, c) - want).abs() < 1e-9, "{r} {c}");
    }
}

#[test]
fn no_kinks_between_parent_pixels() {
    // Кубический пересчёт гладкий: вторая разность у параболы постоянна.
    let parent = Grid::from_fn(256, 256, |_, c| 0.01 * (c as f64 - 128.0).powi(2));
    let child = ct::upsample(&parent, (12, 0, 0), (15, 3, 3), None, false);
    let row: Vec<f64> = (0..256).map(|c| child.at(100, c)).collect();
    let second: Vec<f64> = row.windows(3).map(|w| w[2] - 2.0 * w[1] + w[0]).collect();
    let (lo, hi) = second.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(*v), b.max(*v)));
    assert!(hi - lo < 1e-6);
}

#[test]
fn footprint_high_reads_own_window() {
    let h = Grid::from_fn(256, 256, |_, c| if c >= 200 { 100.0 } else { -5000.0 });
    assert_eq!(ct::footprint_high(&h, (10, 0, 0), (14, 0, 0)), -5000.0);
    assert_eq!(ct::footprint_high(&h, (10, 0, 0), (14, 15, 0)), 100.0);
}

#[test]
fn seam_between_parents_is_small() {
    let surface = |c: usize| 0.001 * (c as f64 - 200.0).powi(2);
    let left = Grid::from_fn(256, 256, |_, c| surface(c));
    let right = Grid::from_fn(256, 256, |_, c| surface(c + 256));
    let a = ct::upsample(&left, (10, 0, 0), (15, 31, 0), None, false);
    let b = ct::upsample(&right, (10, 1, 0), (15, 32, 0), None, false);
    let step = (0..256).map(|r| (b.at(r, 0) - a.at(r, 255)).abs()).fold(0.0, f64::max);
    let slope = (0..255).map(|c| (a.at(0, c + 1) - a.at(0, c)).abs()).fold(0.0, f64::max);
    assert!(step < 2.0 * slope);
}

#[test]
fn neighbour_rows_close_the_seam() {
    // Шероховатая поверхность на двух предках одна над другой: без
    // соседей шов на краю, с их строками - как шаг внутри тайла.
    let n = noise(3, 512 * 256);
    let mut surface = vec![0.0; 512 * 256];
    for c in 0..256 {
        let mut acc = 0.0;
        for r in 0..512 {
            acc += n[r * 256 + c] * 3.0;
            surface[r * 256 + c] = acc;
        }
    }
    let upper = Grid::from_fn(256, 256, |r, c| surface[r * 256 + c]);
    let lower = Grid::from_fn(256, 256, |r, c| surface[(r + 256) * 256 + c]);
    let tiles = |k: ct::Key| match k {
        (10, 0, 0) => Some(upper.clone()),
        (10, 0, 1) => Some(lower.clone()),
        _ => None,
    };
    let seam = |neighbor: Option<&dyn Fn(ct::Key) -> Option<Grid>>| {
        let a = ct::upsample(&upper, (10, 0, 0), (14, 5, 15), neighbor, true);
        let b = ct::upsample(&lower, (10, 0, 1), (14, 5, 16), neighbor, true);
        let inner = (0..255).flat_map(|r| (0..256).map(move |c| (r, c))).map(|(r, c)| (a.at(r + 1, c) - a.at(r, c)).abs()).sum::<f64>() / (255.0 * 256.0);
        (0..256).map(|c| (b.at(0, c) - a.at(255, c)).abs()).sum::<f64>() / 256.0 / inner
    };
    assert!(seam(Some(&tiles)) < 3.0);
    assert!(seam(None) > 6.0);
}

#[test]
fn open_sea_takes_depths_from_sea_level() {
    assert_eq!(ct::source_level((15, 1, 1), Some(-5483.0), ct::SOURCE_LEVEL), ct::SEA_LEVEL);
    assert_eq!(ct::source_level((15, 1, 1), Some(120.0), ct::SOURCE_LEVEL), ct::SOURCE_LEVEL);
    assert_eq!(ct::source_level((15, 1, 1), None, ct::SOURCE_LEVEL), ct::SOURCE_LEVEL);
    assert_eq!(ct::source_level((15, 1, 1), Some(-10.0), ct::SOURCE_LEVEL), ct::SOURCE_LEVEL);
    assert_eq!(ct::source_level((8, 1, 1), Some(-5000.0), ct::SOURCE_LEVEL), 8);
}

#[test]
fn body_without_sea_has_no_isobaths() {
    let h = Grid::from_fn(256, 256, |r, _| -3000.0 + r as f64 * 20.0);
    let blue = |rgba: &[u8]| (0..140 * 256).filter(|i| rgba[i * 4 + 3] > 60 && rgba[i * 4 + 2] > rgba[i * 4]).count();
    let sea = ct::contour_rgba(&h, 5, 10, 1737400.0, Some(200.0), None, Palette::Map, true);
    let dry = ct::contour_rgba(&h, 5, 10, 1737400.0, Some(200.0), None, Palette::Map, false);
    assert!(blue(&sea) > 0);
    assert_eq!(blue(&dry), 0);
}

#[test]
fn imagery_detection() {
    assert!(ct::is_imagery("Esri World Imagery - пример", "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}", false));
    assert!(ct::is_imagery("Google", "https://mt1.google.com/vt/lyrs=s&x={x}&y={y}&z={z}", false));
    assert!(ct::is_imagery("Марс", "", true));
    assert!(!ct::is_imagery("OpenStreetMap", "https://tile.openstreetmap.org/{z}/{x}/{y}.png", false));
    assert!(!ct::is_imagery("Topo", "https://server.arcgisonline.com/ArcGIS/rest/services/World_Topo_Map/MapServer/tile/{z}/{y}/{x}", false));
}

#[test]
fn yellow_on_imagery() {
    let rgba = ct::contour_rgba(&ramp(0.0, 1.0), 15, 10000, R, Some(10.0), None, Palette::Imagery, true);
    for p in rgba.as_chunks::<4>().0 {
        if p[3] > 150 {
            assert!(f64::from(p[0]) > f64::from(p[2]) + 60.0 && f64::from(p[1]) > f64::from(p[2]) + 60.0);
        }
    }
}

#[test]
fn view_step_grows_with_distance() {
    let near = ct::view_step(1600.0, 58.0, 30f64.to_radians(), 900.0, R, ct::LEVEL);
    let far = ct::view_step(50000.0, 58.0, 30f64.to_radians(), 900.0, R, ct::LEVEL);
    assert_eq!(near, ct::MIN_STEP);
    assert!(far > near);
}

#[test]
fn circle_and_open_line_join() {
    let cone = Grid::from_fn(21, 21, |r, c| (r as f64 - 10.0).hypot(c as f64 - 10.0));
    let lines = ct::chain(&ct::segments(&cone, 5.0), 6);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].iter().all(|p| ((p[0] - 10.0).hypot(p[1] - 10.0) - 5.0).abs() < 0.3));
    let ramp = Grid::from_fn(21, 21, |r, c| r as f64 + 0.5 * c as f64);
    let lines = ct::chain(&ct::segments(&ramp, 7.3), 6);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].len() > 10);
    assert!(ct::chain(&[], 6).is_empty());
}

#[test]
fn cone_levels_and_nan() {
    let z = Grid::from_fn(21, 21, |r, c| 100.0 - (r as f64 - 10.0).hypot(c as f64 - 10.0) * 5.0);
    let segs = ct::segments(&z, 77.0);
    assert!(segs.len() > 10);
    assert!(segs.iter().flatten().all(|p| ((p[0] - 10.0).hypot(p[1] - 10.0) - 4.6).abs() < 0.3));
    let z = Grid { rows: 2, cols: 2, data: vec![0.0, 10.0, 0.0, f64::NAN] };
    assert!(ct::segments(&z, 5.0).is_empty());
}

#[test]
fn grid_step_and_levels() {
    let z = [-306.0, 69.0, f64::NAN, 0.0];
    let step = ct::grid_step(&z, 12.0);
    assert_eq!(step, 25.0);
    let lv = ct::levels(&z, step);
    assert_eq!((lv[0], lv[lv.len() - 1]), (-300.0, 50.0));
}
