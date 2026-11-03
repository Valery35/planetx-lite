// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Классификация по своему растру, перенос тестов `test_rasterclass`
//! модуля. Псевдослучайный шум свой: важен не ряд чисел, а то, что шум
//! не связан с классом.

use data::aef::{self, Mosaic, ZoneIndex};
use data::rasterclass::{self, Problem, Run, Source};
use terrio::{DataType, Raster, RasterConfig, RasterDataset, RasterWriteOptions};

const SIDE: usize = 60;
const LEFT: [(usize, usize); 3] = [(10, 10), (30, 12), (50, 8)];
const RIGHT: [(usize, usize); 3] = [(10, 50), (30, 48), (50, 52)];

/// Два равномерных числа в нормальное, преобразование Бокса-Мюллера.
struct Noise(u64);

impl Noise {
    fn uniform(&mut self) -> f64 {
        // SplitMix64.
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn normal(&mut self) -> f64 {
        let (a, b) = (self.uniform().max(1e-12), self.uniform());
        (-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * b).cos()
    }
}

/// Сетка с известными классами: левая половина и правая. Канал 0 -
/// высота в метрах с большим шумом, не связанным с классом, канал 1 -
/// отражение в долях единицы, класс только в нём. Клетки подряд, каналы
/// внутри клетки.
fn scene_of(seed: u64) -> (Vec<f32>, Vec<i16>) {
    let mut noise = Noise(seed);
    let (mut grid, mut truth) = (Vec::new(), Vec::new());
    for _ in 0..SIDE {
        for c in 0..SIDE {
            let class = i16::from(c >= SIDE / 2);
            let light = if class == 0 { 0.1 } else { 0.4 };
            grid.push((2000.0 + 3.0 * noise.normal()) as f32);
            grid.push((light + 0.01 * noise.normal()) as f32);
            truth.push(class);
        }
    }
    (grid, truth)
}

/// Шум 1: на нём точки сцены разделяют все клетки.
fn scene() -> (Vec<f32>, Vec<i16>) {
    scene_of(1)
}

fn points(grid: &[f32], depth: usize, cells: &[(usize, usize)]) -> Vec<Vec<f32>> {
    cells.iter().map(|&(r, c)| grid[(r * SIDE + c) * depth..(r * SIDE + c + 1) * depth].to_vec()).collect()
}

fn run_scene(grid: &[f32], depth: usize, inside: Option<&[bool]>) -> (Vec<i16>, Vec<f32>) {
    let cells: Vec<(usize, usize)> = LEFT.iter().chain(&RIGHT).copied().collect();
    rasterclass::classify(grid, depth, &points(grid, depth, &cells), &[0, 0, 0, 1, 1, 1], inside, rasterclass::KNN)
}

fn right(classes: &[i16], truth: &[i16]) -> usize {
    classes.iter().zip(truth).filter(|(a, b)| a == b).count()
}

#[test]
fn ring_samples_spread_inside_and_classify() {
    // Сторож образцов контура модуля 1.38: не больше 40 клеток поровну
    // по клеткам внутри, по одному контуру на класс классы участка верны.
    assert_eq!(rasterclass::spread_pick(5, 40), [0, 1, 2, 3, 4]);
    assert_eq!(rasterclass::spread_pick(100, 3), [0, 49, 99]);
    assert_eq!(rasterclass::spread_pick(7, 1), [0]);
    assert!(rasterclass::spread_pick(0, 40).is_empty());
    let (values, truth) = scene();
    let grid = Mosaic { x0: 0.0, y0: 0.0, step: 10.0, rows: SIDE, cols: SIDE, depth: 2, values: values.clone() };
    let block = |c0: usize, c1: usize| -> Vec<bool> { (0..SIDE * SIDE).map(|i| (20..40).contains(&(i / SIDE)) && (c0..c1).contains(&(i % SIDE))).collect() };
    let (left, right_side) = (rasterclass::ring_samples(&grid, &block(5, 20)), rasterclass::ring_samples(&grid, &block(40, 55)));
    assert_eq!((left.len(), right_side.len()), (rasterclass::RING_CELLS, rasterclass::RING_CELLS));
    // Клетки без данных образцами не идут.
    let mut holed = grid.clone();
    holed.values.iter_mut().take(2 * SIDE * 40).for_each(|v| *v = f32::NAN);
    assert!(rasterclass::ring_samples(&holed, &block(0, 10)).is_empty());
    let labels: Vec<i32> = std::iter::repeat_n(0, left.len()).chain(std::iter::repeat_n(1, right_side.len())).collect();
    let samples: Vec<Vec<f32>> = left.into_iter().chain(right_side).collect();
    let (classes, _) = rasterclass::classify(&values, 2, &samples, &labels, None, rasterclass::KNN);
    assert!(right(&classes, &truth) as f64 > 0.95 * (SIDE * SIDE) as f64);
}

/// Каналы приведены к разбросу 1: отражение решает, хотя метры больше
/// его на четыре порядка. Без приведения красный.
#[test]
fn channels_in_different_units() {
    let (grid, truth) = scene();
    let (classes, _) = run_scene(&grid, 2, None);
    assert_eq!(right(&classes, &truth), SIDE * SIDE);
}

/// На одном канале у косинуса все клетки одинаковы.
#[test]
fn single_band() {
    let (grid, truth) = scene();
    let light: Vec<f32> = grid.chunks(2).map(|c| c[1]).collect();
    let (classes, _) = run_scene(&light, 1, None);
    assert_eq!(right(&classes, &truth), SIDE * SIDE);
}

/// Координаты и абсолютные отметки: большое среднее не съедает точность.
#[test]
fn large_offset_in_float32() {
    let (mut grid, truth) = scene();
    for cell in grid.chunks_mut(2) {
        cell[0] += 1.0e6;
    }
    let (classes, _) = run_scene(&grid, 2, None);
    assert_eq!(right(&classes, &truth), SIDE * SIDE);
}

#[test]
fn control_point_cell_is_own_class() {
    let (grid, _) = scene();
    let (classes, distance) = run_scene(&grid, 2, None);
    for (label, cells) in [LEFT, RIGHT].iter().enumerate() {
        for &(r, c) in cells {
            assert_eq!(classes[r * SIDE + c], label as i16);
            assert!(distance[r * SIDE + c] < 1e-2);
        }
    }
}

#[test]
fn no_data_and_outside_have_no_class() {
    let (mut grid, truth) = scene();
    // Дыра одного канала.
    for r in 20..25 {
        for c in 20..25 {
            grid[(r * SIDE + c) * 2 + 1] = f32::NAN;
        }
    }
    let inside: Vec<bool> = (0..SIDE * SIDE).map(|i| i / SIDE >= 5).collect();
    let (classes, distance) = run_scene(&grid, 2, Some(&inside));
    for r in 20..25 {
        for c in 20..25 {
            assert_eq!(classes[r * SIDE + c], -1);
        }
    }
    assert!(classes[..5 * SIDE].iter().all(|c| *c == -1));
    assert!(distance[..5 * SIDE].iter().all(|d| d.is_nan()));
    let hit = classes.iter().zip(&truth).filter(|(c, t)| **c >= 0 && c == t).count();
    assert_eq!(hit, SIDE * SIDE - 25 - 5 * SIDE);
}

#[test]
fn point_without_data_does_not_vote() {
    let (grid, _) = scene();
    let cells: Vec<(usize, usize)> = LEFT.iter().chain(&RIGHT).copied().collect();
    let mut samples = points(&grid, 2, &cells);
    let labels = [5, 5, 5, 9, 9, 9];
    let (without, _) = rasterclass::classify(&grid, 2, &samples[1..], &labels[1..], None, rasterclass::KNN);
    samples[0][1] = f32::NAN;
    let (classes, _) = rasterclass::classify(&grid, 2, &samples, &labels, None, rasterclass::KNN);
    let mut seen = classes.clone();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen, vec![5, 9]);
    assert_eq!(classes, without);
}

/// Две точки против трёх при шумном канале: без деления голосов на
/// количество точек класс с тремя точками забирал 6 % клеток.
#[test]
fn unequal_classes() {
    let (grid, truth) = scene();
    let cells: Vec<(usize, usize)> = LEFT[1..].iter().chain(&RIGHT).copied().collect();
    let (classes, _) = rasterclass::classify(&grid, 2, &points(&grid, 2, &cells), &[0, 0, 1, 1, 1], None, rasterclass::KNN);
    assert!(right(&classes, &truth) as f64 / (SIDE * SIDE) as f64 > 0.99);
}

#[test]
fn constant_channel_is_not_a_feature() {
    let (grid, truth) = scene();
    let wide: Vec<f32> = grid.chunks(2).flat_map(|c| [c[0], c[1], 7.0]).collect();
    let (classes, _) = run_scene(&wide, 3, None);
    assert_eq!(right(&classes, &truth), SIDE * SIDE);
    let (mean, spread) = rasterclass::scales(&wide, 3, None);
    assert_eq!(spread[2], 0.0);
    assert!((mean[2] - 7.0).abs() < 1e-9);
}

#[test]
fn nothing_to_compare() {
    let grid = vec![3.0f32; 2 * 4 * 4];
    let samples = vec![vec![3.0, 3.0], vec![3.0, 3.0]];
    let (classes, _) = rasterclass::classify(&grid, 2, &samples, &[0, 1], None, rasterclass::KNN);
    assert!(classes.iter().all(|c| *c == -1));
    let empty = vec![f32::NAN; 9];
    let (classes, _) = rasterclass::classify(&empty, 1, &[vec![1.0], vec![1.0]], &[0, 1], None, rasterclass::KNN);
    assert!(classes.iter().all(|c| *c == -1));
}

/// Расчёт по частям - тот же, что целиком: окно считает его за кадры.
#[test]
fn parts_equal_whole() {
    let (grid, _) = scene();
    let (whole, _) = run_scene(&grid, 2, None);
    let cells: Vec<(usize, usize)> = LEFT.iter().chain(&RIGHT).copied().collect();
    let mut run = Run::new(&grid, 2, &points(&grid, 2, &cells), &[0, 0, 0, 1, 1, 1], None, rasterclass::KNN);
    let mut seen = vec![run.done()];
    while !run.step(&grid, 97) {
        seen.push(run.done());
    }
    assert_eq!(run.finish().0, whole);
    assert_eq!(seen[..2], [0, 97]);
}

#[test]
fn auto_pixel() {
    assert_eq!(rasterclass::auto_pixel(10.0, 3000.0), 10.0);
    assert!((rasterclass::auto_pixel(0.05, 8000.0) - 10.0).abs() < 1e-9);
}

#[test]
fn too_big() {
    assert!(!rasterclass::too_big(1000, 1000, 64));
    assert!(rasterclass::too_big(1000, 1000, 400));
    assert!(rasterclass::too_big(usize::MAX, usize::MAX, usize::MAX));
    // Большой участок: один канал, но клеток больше предела.
    assert!(!rasterclass::too_big(4000, 4000, 1));
    assert!(rasterclass::too_big(4001, 4000, 1));
    // «Авто» укладывается в предел на участке любого размера.
    let side = 1000000.0;
    let cells = (side / rasterclass::auto_pixel(1.0, side)) as usize;
    assert!(!rasterclass::too_big(cells, cells, 8));
}

#[test]
fn covered() {
    let grid = [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0];
    let mut holed = grid;
    holed[1] = f32::NAN;
    let inside = [true, true, true, false];
    assert!((rasterclass::covered(&holed, 2, &inside) - 2.0 / 3.0).abs() < 1e-9);
}

/// Шум 7 здесь и в `core/rasterclass.py` модуля дал одно и то же
/// количество верных клеток: сверка 1 ноября 2026 года тем же рядом чисел.
#[test]
fn same_cells_as_the_module() {
    let (grid, truth) = scene_of(7);
    let (classes, _) = run_scene(&grid, 2, None);
    assert_eq!(right(&classes, &truth), 3563);
}

const NORTH: f64 = 58.0;
const EAST: f64 = 56.2;
/// Растр EPSG:3857 вокруг точки: шаг и сторона, метры проекции.
const PIXEL: f64 = 30.0;
const CELLS: usize = 100;

fn mercator(lat: f64, lon: f64) -> (f64, f64) {
    let r = 6378137.0;
    (r * lon.to_radians(), r * (std::f64::consts::FRAC_PI_4 + lat.to_radians() / 2.0).tan().ln())
}

/// Левый нижний угол растра, точка - посередине.
fn corner() -> (f64, f64) {
    let (x, y) = mercator(NORTH, EAST);
    (x - PIXEL * CELLS as f64 / 2.0, y - PIXEL * CELLS as f64 / 2.0)
}

fn raster(bands: usize, kind: DataType, epsg: Option<u32>, value: impl Fn(usize, usize, usize) -> f64, nodata: Option<f64>) -> RasterDataset {
    let (x_min, y_min) = corner();
    let config = RasterConfig { cols: CELLS, rows: CELLS, bands, x_min, y_min, cell_size: PIXEL, data_type: kind, nodata: nodata.unwrap_or(-9999.0), ..Default::default() };
    let mut r = Raster::new(config);
    r.crs.epsg = epsg;
    for band in 0..bands {
        for row in 0..CELLS {
            for col in 0..CELLS {
                assert!(r.set(band as isize, row as isize, col as isize, value(band, row, col)).is_ok());
            }
        }
    }
    RasterDataset::new(r, nodata.is_some())
}

fn written(ds: &RasterDataset, options: &RasterWriteOptions) -> Vec<u8> {
    let bytes = terrio::memory::write_raster_bytes(ds, "own.tif", options);
    assert!(bytes.is_ok(), "{:?}", bytes.err());
    bytes.unwrap_or_default()
}

fn zone() -> ZoneIndex {
    let index = ZoneIndex::bare(&aef::zone_of(EAST, NORTH));
    assert!(index.is_some());
    index.unwrap_or(ZoneIndex { zone: String::new(), number: 1, south: false, cells: Vec::new() })
}

/// Участок вокруг точки, градусы: запад, юг, восток, север.
const BOX: (f64, f64, f64, f64) = (EAST - 0.024, NORTH - 0.012, EAST + 0.024, NORTH + 0.012);

fn warped(bytes: &[u8], step: f64) -> Option<(Source, Mosaic<f32>)> {
    let source = Source::read(bytes, "own.tif");
    assert!(source.is_ok(), "{:?}", source.as_ref().err());
    let source = source.ok()?;
    let warp = source.warp(&zone(), BOX, step);
    assert!(warp.is_ok());
    let mut warp = warp.ok()?;
    while !source.warp_rows(&mut warp, 7) {}
    Some((source, warp.grid))
}

/// Значения - столбец и строка растра. Клетка сетки UTM должна читать
/// то, что лежит под ней в системе растра: без пересчёта систем красный.
#[test]
fn raster_lands_on_the_utm_grid() {
    let hole = |row: usize, col: usize| (40..45).contains(&row) && (40..45).contains(&col);
    let value = |band: usize, row: usize, col: usize| {
        if band == 1 && hole(row, col) {
            -1.0
        } else if band == 0 {
            col as f64 + 0.5
        } else {
            row as f64 + 0.5
        }
    };
    let bytes = written(&raster(2, DataType::F32, Some(3857), value, Some(-1.0)), &RasterWriteOptions::default());
    let Some((source, grid)) = warped(&bytes, 10.0) else { return };
    assert_eq!(source.bands(), 2);
    let index = zone();
    let (x_min, y_min) = corner();
    let top = y_min + PIXEL * CELLS as f64;
    let (mut seen, mut empty) = (0, 0);
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            let (lat, lon) = aef::from_utm(index.number, index.south, grid.x0 + (c as f64 + 0.5) * grid.step, grid.y0 - (r as f64 + 0.5) * grid.step);
            let (x, y) = mercator(lat, lon);
            let (col, row) = ((x - x_min) / PIXEL, (top - y) / PIXEL);
            let cell = grid.at(r, c).unwrap_or(&[]);
            // Клетки у края растра и у дыры не берутся.
            let far = col > 2.0 && col < 98.0 && row > 2.0 && row < 98.0 && !(38.0..47.0).contains(&col) | !(38.0..47.0).contains(&row);
            if far {
                assert!((f64::from(cell[0]) - col).abs() < 1e-2, "{} {col}", cell[0]);
                assert!((f64::from(cell[1]) - row).abs() < 1e-2, "{} {row}", cell[1]);
                seen += 1;
            }
            if !(-1.0..101.0).contains(&col) || !(-1.0..101.0).contains(&row) {
                assert!(cell[0].is_nan());
                empty += 1;
            }
        }
    }
    assert!(seen > 1000, "{seen}");
    assert!(empty > 0);
    // Дыра только в канале 1: канал 0 в ней читается, клетка без данных.
    let (x, y) = (x_min + 42.5 * PIXEL, top - 42.5 * PIXEL);
    let (lat, lon) = ((2.0 * (y / 6378137.0).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees(), (x / 6378137.0).to_degrees());
    let (ux, uy) = index.project(lat, lon);
    let cell = grid.cell_of(ux, uy).and_then(|(r, c)| grid.at(r, c)).unwrap_or(&[]);
    assert!(cell[0].is_finite() && cell[1].is_nan());
    // Пиксель 30 м проекции на этой широте - около 16 м земли.
    let pixel = source.pixel(&index, NORTH, EAST);
    assert!((pixel - 30.0 * 58f64.to_radians().cos()).abs() < 0.5, "{pixel}");
}

fn alpha_file(bigtiff: bool) -> Vec<u8> {
    let value = |band: usize, _: usize, col: usize| match band {
        3 if col < CELLS / 2 => 0.0,
        3 => 255.0,
        _ => (col + band) as f64,
    };
    let options = RasterWriteOptions { bigtiff, creation_options: vec![("PHOTOMETRIC".to_string(), "RGB".to_string()), ("ALPHA".to_string(), "YES".to_string())], ..Default::default() };
    written(&raster(4, DataType::U8, Some(3857), value, None), &options)
}

/// Канал прозрачности - маска, а не признак: левая половина без данных.
#[test]
fn alpha_is_a_mask() {
    for bigtiff in [false, true] {
        let Some((source, grid)) = warped(&alpha_file(bigtiff), 10.0) else { return };
        assert_eq!(source.bands(), 3);
        assert_eq!(grid.depth, 3);
        let index = zone();
        let (x_min, y_min) = corner();
        let at = |col: f64| {
            let (x, y) = (x_min + col * PIXEL, y_min + PIXEL * CELLS as f64 / 2.0);
            let (lat, lon) = ((2.0 * (y / 6378137.0).exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees(), (x / 6378137.0).to_degrees());
            let (ux, uy) = index.project(lat, lon);
            grid.cell_of(ux, uy).and_then(|(r, c)| grid.at(r, c)).map(<[f32]>::to_vec).unwrap_or_default()
        };
        assert!(at(20.0).iter().all(|v| v.is_nan()));
        assert!(at(80.0).iter().all(|v| v.is_finite()));
    }
}

/// Многоканальный файл не идёт в рельеф: каналы считаются по заголовку.
/// Серый с прозрачностью - один признак, рельеф как раньше.
#[test]
fn feature_bands_by_the_header() {
    let value = |b: usize, _: usize, c: usize| (b + c) as f64;
    let plain = RasterWriteOptions::default();
    assert_eq!(rasterclass::feature_bands(&written(&raster(1, DataType::F32, Some(3857), value, None), &plain)), Some(1));
    assert_eq!(rasterclass::feature_bands(&written(&raster(4, DataType::F32, Some(3857), value, None), &plain)), Some(4));
    assert_eq!(rasterclass::feature_bands(&alpha_file(false)), Some(3));
    assert_eq!(rasterclass::feature_bands(&alpha_file(true)), Some(3));
    assert_eq!(rasterclass::feature_bands(b"not a tiff"), None);
    assert_eq!(rasterclass::feature_bands(&[]), None);
    let gray = RasterWriteOptions { creation_options: vec![("ALPHA".to_string(), "YES".to_string())], ..Default::default() };
    assert_eq!(rasterclass::feature_bands(&written(&raster(2, DataType::U8, Some(3857), value, None), &gray)), Some(1));
}

/// Углы растра - вокруг его середины.
#[test]
fn corners_surround_the_raster() {
    let bytes = written(&raster(2, DataType::F32, Some(3857), |b, _, c| (b + c) as f64, None), &RasterWriteOptions::default());
    let corners = Source::read(&bytes, "own.tif").ok().and_then(|s| s.corners());
    assert!(corners.is_some());
    let Some([bottom_left, _, top_right, _]) = corners else { return };
    assert!(bottom_left.0 < NORTH && NORTH < top_right.0);
    assert!(bottom_left.1 < EAST && EAST < top_right.1);
}

/// Четыре канала без тега прозрачности - четыре признака.
#[test]
fn four_bands_without_alpha_are_features() {
    let bytes = written(&raster(4, DataType::F32, Some(3857), |b, _, c| (b + c) as f64, None), &RasterWriteOptions::default());
    let Some((source, _)) = warped(&bytes, 20.0) else { return };
    assert_eq!(source.bands(), 4);
}

#[test]
fn raster_without_a_system_is_refused() {
    let bytes = written(&raster(1, DataType::F32, None, |_, r, c| (r + c) as f64, None), &RasterWriteOptions::default());
    assert_eq!(Source::read(&bytes, "own.tif").err(), Some(Problem::NoCrs));
}

/// Файл считается враждебным: обрезанный и пустой не роняют чтение.
#[test]
fn broken_file_is_refused() {
    assert_eq!(Source::read(b"not a tiff", "own.tif").err(), Some(Problem::Unreadable));
    assert_eq!(Source::read(b"II*\0", "own.tif").err(), Some(Problem::Unreadable));
    let bytes = alpha_file(false);
    for cut in [8, 64, 300, bytes.len() / 2] {
        let _ = Source::read(&bytes[..cut.min(bytes.len())], "own.tif");
    }
}

/// Сетка участка не больше предела - раньше выделения памяти.
#[test]
fn huge_grid_is_refused_before_memory() {
    let bytes = written(&raster(1, DataType::F32, Some(3857), |_, r, c| (r + c) as f64, None), &RasterWriteOptions::default());
    let source = Source::read(&bytes, "own.tif");
    assert!(source.is_ok());
    let Ok(source) = source else { return };
    assert_eq!(source.warp(&zone(), BOX, 0.001).err(), Some(Problem::Big));
}

#[test]
fn points_on_cells_with_data() {
    let bytes = written(&raster(1, DataType::F32, Some(3857), |_, r, c| (r + c) as f64, Some(-1.0)), &RasterWriteOptions::default());
    let Some((_, grid)) = warped(&bytes, 10.0) else { return };
    let points = [(0, (NORTH, EAST)), (1, (NORTH + 0.001, EAST)), (1, (NORTH + 0.009, EAST + 0.011))];
    let (values, labels, outside) = rasterclass::samples(&grid, &zone(), &points);
    assert_eq!((values.len(), labels, outside), (2, vec![0, 1], 1));
}
