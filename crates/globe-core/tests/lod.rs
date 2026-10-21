// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Выбор тайлов по экранной ошибке.
//!
//! Перенос `planetx/tests/test_lod.py` модуля и теста размера тайла
//! на другом теле из `test_body.py`. Не переносятся:
//! - `TestFastPath` и `TestScalarEcef.test_matches_numpy_version` - в Lite
//!   один путь обхода и одна формула ECEF, сверять нечего;
//! - `TestMemo.test_partial_reset_when_heights_arrive` - он работает
//!   с хранилищем высот, перенос вместе с `terrain` в фазе 1.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashSet;

use common::{PERM, Rng};
use globe_core::camera::Camera;
use globe_core::ellipsoid::{self as el, Ellipsoid};
use globe_core::lod::{self, Memo, Options, Selection, TileKey};
use globe_core::tiling as tl;

fn wgs84() -> Ellipsoid {
    Ellipsoid::wgs84()
}

fn everything(_: TileKey) -> bool {
    true
}

/// Повторяемое перемешивание ключа тайла.
fn key_hash(key: TileKey) -> u64 {
    let mut z = (u64::from(key.0) << 48) ^ (u64::from(key.1) << 24) ^ u64::from(key.2);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Готова примерно половина тайлов, уровни 0-2 готовы всегда.
fn half_ready(key: TileKey) -> bool {
    key.0 <= 2 || key_hash(key).is_multiple_of(2)
}

fn start_levels(key: TileKey) -> bool {
    key.0 <= 2
}

fn view(lat: f64, lon: f64, distance: f64, heading: f64, tilt: f64) -> Camera {
    Camera::look_at(wgs84(), lat, lon, distance, heading, tilt, 0.0).with_viewport(1920, 1080)
}

fn select(cam: &Camera, ready: &dyn Fn(TileKey) -> bool) -> Selection {
    lod::select(cam, ready, Options::default())
}

fn in_tile(lat: f64, lon: f64, key: TileKey) -> bool {
    tl::lonlat_to_tile(lat, lon, key.0) == (key.1, key.2)
}

fn centre_tile(cam: &Camera, selection: &Selection) -> Vec<TileKey> {
    let e = cam.ellipsoid;
    let (origin, d) = cam.ray(f64::from(cam.width) / 2.0, f64::from(cam.height) / 2.0);
    let t = e.ray_intersect(origin, d, 0.0).unwrap();
    let (lat, lon, _) = e.ecef_to_geodetic(origin + d * t);
    selection.draw.iter().copied().filter(|k| in_tile(lat, lon, *k)).collect()
}

/// Сколько тайлов списка покрывают точку под случайным пикселем.
///
/// Возвращает количества по пикселям, где луч встретил Землю в пределах
/// сетки Mercator.
fn coverage(cam: &Camera, selection: &Selection) -> Vec<usize> {
    let e = cam.ellipsoid;
    let mut rng = Rng(11);
    let mut counts = Vec::new();
    for _ in 0..400 {
        let px = rng.uniform(0.0, f64::from(cam.width));
        let py = rng.uniform(0.0, f64::from(cam.height));
        let (origin, d) = cam.ray(px, py);
        let Some(t) = e.ray_intersect(origin, d, 0.0) else {
            continue;
        };
        let (lat, lon, _) = e.ecef_to_geodetic(origin + d * t);
        if lat.abs() >= tl::MAX_LAT {
            continue;
        }
        // Заплатка покрывает точку, только если та лежит в её дыре.
        counts.push(
            selection
                .draw
                .iter()
                .zip(&selection.holes)
                .filter(|(key, hole)| in_tile(lat, lon, **key) && hole.is_none_or(|h| in_tile(lat, lon, h)))
                .count(),
        );
    }
    counts
}

fn distinct(counts: &[usize]) -> Vec<usize> {
    let mut v: Vec<usize> = counts.iter().copied().collect::<HashSet<_>>().into_iter().collect();
    v.sort_unstable();
    v
}

type CameraCase = (&'static str, (f64, f64), f64, f64, f64);

const CAMERAS: [CameraCase; 7] = [
    ("космос над Пермью", PERM, 2.0e7, 0.0, 0.0),
    ("500 м над Пермью", PERM, 500.0, 0.0, 0.0),
    ("2 км над Пермью, наклон 70°", PERM, 2000.0, 30.0, 70.0),
    ("50 км, наклон 85°", PERM, 50000.0, 200.0, 85.0),
    ("линия смены дат", (10.0, 179.9), 3000.0, 0.0, 40.0),
    ("у края Mercator", (84.0, 20.0), 20000.0, 0.0, 30.0),
    ("экватор с высоты 1000 км", (0.0, 0.0), 1.0e6, 0.0, 0.0),
];

// Уровни.

#[test]
fn space_view_is_coarse() {
    let sel = select(&view(PERM.0, PERM.1, 2.0e7, 0.0, 0.0), &everything);
    assert!(sel.draw.iter().map(|k| k.0).max().unwrap() <= 3);
}

#[test]
fn street_view_reaches_level_17_or_18() {
    let cam = view(PERM.0, PERM.1, 500.0, 0.0, 0.0);
    let found = centre_tile(&cam, &select(&cam, &everything));
    assert_eq!(found.len(), 1);
    assert!([17, 18].contains(&found[0].0), "{:?}", found[0]);
}

#[test]
fn no_more_than_400_tiles() {
    for (lat, lon) in [PERM, (0.0, 0.0), (80.0, -40.0), (-33.9, 151.2)] {
        for distance in [50.0, 300.0, 3e3, 3e4, 3e5, 3e6, 2e7, 4e7] {
            let sel = select(&view(lat, lon, distance, 0.0, 0.0), &everything);
            assert!(!sel.draw.is_empty());
            assert!(sel.draw.len() <= 400, "{lat} {lon} {distance}");
        }
    }
}

#[test]
fn max_level_is_respected() {
    let cam = view(PERM.0, PERM.1, 50.0, 0.0, 0.0);
    let sel = lod::select(&cam, &everything, Options { max_level: 15, ..Options::default() });
    assert_eq!(sel.draw.iter().map(|k| k.0).max(), Some(15));
}

// Покрытие: точка под пикселем лежит ровно в одном тайле списка.

fn check_coverage(ready: &dyn Fn(TileKey) -> bool) {
    for (name, (lat, lon), distance, heading, tilt) in CAMERAS {
        let cam = view(lat, lon, distance, heading, tilt);
        let counts = coverage(&cam, &select(&cam, ready));
        assert!(counts.len() > 50, "{name}");
        assert_eq!(distinct(&counts), [1], "{name}");
    }
}

#[test]
fn coverage_all_ready() {
    check_coverage(&everything);
}

#[test]
fn coverage_half_ready() {
    check_coverage(&half_ready);
}

#[test]
fn coverage_only_start_levels_ready() {
    check_coverage(&start_levels);
}

// Замена тайла детьми.

fn pairs(sel: &Selection) -> Vec<(TileKey, Option<TileKey>)> {
    sel.draw.iter().copied().zip(sel.holes.iter().copied()).collect()
}

#[test]
fn parent_stays_until_all_children_ready() {
    // Не готов ни тайл z11 под камерой, ни его потомки. Закрыть его
    // место нечем, поэтому родитель z10 рисуется заплаткой на месте z11,
    // а готовые братья z11 - своими потомками.
    let cam = view(PERM.0, PERM.1, 500.0, 0.0, 0.0);
    let (mx, my) = tl::lonlat_to_tile(PERM.0, PERM.1, 11);
    let missing = (11, mx, my);
    let parent = (10, mx / 2, my / 2);
    let ready = move |(z, x, y): TileKey| z < 11 || (x >> (z - 11), y >> (z - 11)) != (mx, my);
    let sel = select(&cam, &ready);
    let p = pairs(&sel);
    assert!(p.contains(&(parent, Some(missing))));
    assert!(!p.contains(&(parent, None)));
    for ((z, x, y), _) in &p {
        if *z > 11 {
            assert_ne!((x >> (z - 11), y >> (z - 11)), (mx, my), "потомок в дыре");
        }
    }
    assert!(sel.wanted(missing).is_some());
}

#[test]
fn far_missing_tile_keeps_near_detail() {
    // Камера низко с наклоном, как при облёте. Не готов дальний тайл
    // у края кадра. В модуле его предок прежде рисовался целиком вместо
    // всех потомков, и местность под камерой становилась размытой.
    let cam = view(PERM.0, PERM.1, 450.0, 130.0, 72.0);
    let full = select(&cam, &everything);
    let near = centre_tile(&cam, &full)[0];
    let mut keep: Vec<TileKey> = full.keep.iter().copied().collect();
    keep.sort_unstable();
    // Брат предка тайла под камерой на самом глубоком уровне, где он
    // виден: их общий родитель прежде рисовался целиком.
    let mut far = None;
    for level in (4..near.0).rev() {
        let shift = near.0 - level;
        let mine = (near.1 >> shift, near.2 >> shift);
        far = keep
            .iter()
            .copied()
            .find(|k| k.0 == level && (k.1, k.2) != mine && (k.1 >> 1, k.2 >> 1) == (mine.0 >> 1, mine.1 >> 1));
        if far.is_some() {
            break;
        }
    }
    let far = far.unwrap();
    let ready = move |(z, x, y): TileKey| z < far.0 || (x >> (z - far.0), y >> (z - far.0)) != (far.1, far.2);
    let sel = select(&cam, &ready);
    assert!(pairs(&sel).contains(&(near, None)));
    assert_eq!(distinct(&coverage(&cam, &sel)), [1]);
}

#[test]
fn no_tile_drawn_with_its_ancestor() {
    // Предок рядом с потомком - только заплатка на чужом месте.
    let sel = select(&view(PERM.0, PERM.1, 2000.0, 30.0, 70.0), &half_ready);
    let p = pairs(&sel);
    let whole: HashSet<TileKey> = p.iter().filter(|(_, h)| h.is_none()).map(|(k, _)| *k).collect();
    for ((z, x, y), _) in &p {
        for up in 1..=*z {
            let anc = (z - up, x >> up, y >> up);
            assert!(!whole.contains(&anc));
            for (key, ahole) in &p {
                if *key == anc
                    && let Some(h) = ahole
                    && h.0 <= *z
                {
                    let shift = z - h.0;
                    assert_ne!((x >> shift, y >> shift), (h.1, h.2));
                }
            }
        }
    }
}

// Очередь загрузки.

#[test]
fn ready_tiles_are_not_wanted() {
    let sel = select(&view(PERM.0, PERM.1, 500.0, 0.0, 0.0), &half_ready);
    assert!(!sel.want.is_empty());
    assert!(sel.want.iter().all(|(k, _)| !half_ready(*k)));
}

#[test]
fn coarser_tiles_come_first() {
    // У предка экранная ошибка не меньше. При равенстве, например когда
    // камера внутри обеих описанных сфер и ошибка бесконечна, предок
    // уходит первым как раньше попавший в очередь.
    let sel = select(&view(PERM.0, PERM.1, 500.0, 0.0, 0.0), &start_levels);
    assert!(!sel.want.is_empty());
    let order: Vec<TileKey> = sel.want.iter().map(|(k, _)| *k).collect();
    for ((z, x, y), priority) in &sel.want {
        if *z > 3 {
            let parent = (z - 1, x / 2, y / 2);
            assert!(sel.wanted(parent).unwrap() >= *priority);
            let pos = |k: TileKey| order.iter().position(|o| *o == k).unwrap();
            assert!(pos(parent) < pos((*z, *x, *y)));
        }
    }
}

#[test]
fn only_loading_front_is_wanted() {
    // У самой земли с готовыми уровнями 0-2 просятся только тайлы
    // уровня 3. В модуле прежде просились тысячи тайлов до уровня 19.
    for cam in [view(PERM.0, PERM.1, 500.0, 0.0, 0.0), view(PERM.0, PERM.1, 400.0, 0.0, 80.0)] {
        let sel = select(&cam, &start_levels);
        let levels: HashSet<u32> = sel.want.iter().map(|(k, _)| k.0).collect();
        assert_eq!(levels, HashSet::from([3]));
        assert!(sel.want.len() < 20);
    }
}

#[test]
fn priority_is_finite() {
    let sel = select(&view(PERM.0, PERM.1, 50.0, 0.0, 0.0), &|k: TileKey| k.0 <= 12);
    assert!(!sel.want.is_empty());
    for (_, priority) in &sel.want {
        assert!(*priority < lod::INFINITE_PRIORITY + 1.0);
    }
    assert!(lod::priority(f64::INFINITY, 3) > lod::priority(f64::INFINITY, 4));
}

#[test]
fn keep_covers_draw_and_want() {
    let sel = select(&view(PERM.0, PERM.1, 2000.0, 30.0, 70.0), &half_ready);
    assert!(!sel.draw.is_empty());
    assert!(sel.draw.iter().all(|k| sel.keep.contains(k)));
    assert!(sel.want.iter().all(|(k, _)| sel.keep.contains(k)));
}

// Отсечение.

#[test]
fn far_side_is_culled() {
    let e = wgs84();
    let cam = view(PERM.0, PERM.1, 1.0e6, 0.0, 0.0);
    let sel = select(&cam, &everything);
    assert!(!sel.keep.is_empty());
    let (fx, fy) = tl::lonlat_to_tile(-PERM.0, PERM.1 - 180.0, 5);
    assert!(!sel.keep.contains(&(5, fx, fy)));
    let eye_dir = cam.eye.normalize();
    for (z, x, y) in &sel.keep {
        if *z >= 5 {
            assert!(lod::tile_info(&e, *z, *x, *y).direction.dot(eye_dir) > 0.0);
        }
    }
}

#[test]
fn behind_the_camera_is_culled() {
    // Взгляд на север вдоль земли. Тайлы южнее камеры не нужны.
    let sel = select(&view(PERM.0, PERM.1, 2000.0, 0.0, 80.0), &everything);
    assert!(!sel.keep.is_empty());
    let (sx, sy) = tl::lonlat_to_tile(PERM.0 - 0.3, PERM.1, 14);
    assert!(!sel.keep.contains(&(14, sx, sy)));
}

#[test]
fn street_view_visits_few_nodes() {
    let sel = select(&view(PERM.0, PERM.1, 500.0, 0.0, 0.0), &everything);
    assert!(sel.visited > 0);
    assert!(sel.visited < 1500, "{}", sel.visited);
}

// Границы тайла.

#[test]
fn tile_rows_match_tiling() {
    let e = wgs84();
    for (z, y) in [(0u32, 0u32), (5, 7), (12, 1300), (19, 200000)] {
        let info = lod::tile_info(&e, z, 0, y);
        let n = f64::from(1u32 << z);
        let exact = e.geodetic_to_ecef(tl::lat_of_row((f64::from(y) + 0.5) / n), 0.5 / n * 360.0 - 180.0, 0.0);
        assert!((info.center - exact).abs().max_element() < 1e-6, "{z} {y}");
    }
}

#[test]
fn tile_size_follows_the_body() {
    let earth = lod::tile_info(&wgs84(), 3, 4, 3).texel;
    let moon = lod::tile_info(&Ellipsoid::new(el::MOON), 3, 4, 3).texel;
    assert!(earth > 0.0);
    assert!((moon / earth - el::MOON.a / el::EARTH.a).abs() < 5e-10);
}

// Высоты рельефа.

#[test]
fn peak_beyond_the_horizon_is_kept() {
    // Глаз в 1 км над экватором смотрит на восток вдоль земли.
    // Отсечение считает горизонт по сфере радиуса B, на экваторе это
    // 4.8° дуги с запасом. Вершина высотой 8 км видна ещё на 2.9° дальше.
    // Тайл на 7.2° без высот отсекается, с высотой 8 км остаётся.
    let cam = view(0.0, 0.0, 1000.0, 90.0, 89.0);
    // Грубые предки уровней 0-5 накрывают точку всегда.
    let covers = |sel: &Selection| sel.keep.iter().any(|k| k.0 >= 6 && in_tile(0.0, 7.2, *k));
    assert!(!covers(&select(&cam, &everything)));
    let mountains = |_: TileKey| (0.0, 8000.0);
    let sel = lod::select(&cam, &everything, Options { heights: Some(&mountains), ..Options::default() });
    assert!(covers(&sel));
}

#[test]
fn coverage_holds_with_heights() {
    let cam = view(PERM.0, PERM.1, 2000.0, 30.0, 70.0);
    let hills = |_: TileKey| (0.0, 3000.0);
    let sel = lod::select(&cam, &half_ready, Options { heights: Some(&hills), ..Options::default() });
    assert_eq!(distinct(&coverage(&cam, &sel)), [1]);
}

#[test]
fn mountains_do_not_explode_the_tree() {
    // Камера в 250 м над склоном на высоте 4 км. Склон 20 %: толщина
    // слоя высот тайла - пятая часть его ширины. Сфера тайла поднимается
    // к слою и раздувается на половину толщины. Склон ближе к камере,
    // чем равнина, и деталей нужно больше. Замер модуля 26 сентября
    // 2026 года - в 1.38 раза. С размахом всей высоты гор в радиусе было
    // в 2.4 раза. Порог - 1.5.
    let layer = |key: TileKey| {
        let width = 40_075_016.0 * 43.35f64.to_radians().cos() / f64::from(1u32 << key.0);
        let thickness = (0.2 * width).min(4000.0);
        (4000.0 - thickness, 4000.0)
    };
    let flat = select(&view(43.35, 42.44, 500.0, 0.0, 60.0), &everything);
    let cam = Camera::look_at(wgs84(), 43.35, 42.44, 500.0, 0.0, 60.0, 4000.0).with_viewport(1920, 1080);
    let hills = lod::select(&cam, &everything, Options { heights: Some(&layer), ..Options::default() });
    assert!(!flat.draw.is_empty());
    assert!((hills.draw.len() as f64) < 1.5 * flat.draw.len() as f64, "{} {}", hills.draw.len(), flat.draw.len());
}

// Память узлов не меняет выбор и сбрасывается при сдвиге камеры.

#[test]
fn same_selection_while_tiles_arrive() {
    let cam = view(PERM.0, PERM.1, 2000.0, 30.0, 70.0);
    let mut memo = Memo::new();
    let mut arrived: HashSet<TileKey> = HashSet::new();
    let full = select(&cam, &everything);
    for step in 0..6 {
        // Тайлы приходят, готовность меняется, камера стоит.
        arrived.extend(full.keep.iter().copied().filter(|k| key_hash(*k) % 6 == step));
        let is_ready = |k: TileKey| k.0 <= 2 || arrived.contains(&k);
        let values = memo.bind(&cam, lod::THRESHOLD, 0, &[]);
        let with_memo = lod::select(&cam, &is_ready, Options { memo: Some(values), ..Options::default() });
        let plain = select(&cam, &is_ready);
        assert_eq!(with_memo.draw, plain.draw, "{step}");
        assert_eq!(with_memo.holes, plain.holes, "{step}");
        assert_eq!(with_memo.want, plain.want, "{step}");
        assert_eq!(with_memo.keep, plain.keep, "{step}");
    }
}

#[test]
fn reset_on_camera_and_heights() {
    let mut memo = Memo::new();
    let cam = view(PERM.0, PERM.1, 2000.0, 30.0, 70.0);
    let values = memo.bind(&cam, lod::THRESHOLD, 0, &[]);
    lod::select(&cam, &everything, Options { memo: Some(values), ..Options::default() });
    let filled = memo.bind(&cam, lod::THRESHOLD, 0, &[]).len();
    assert!(filled > 0);
    assert!(memo.bind(&cam, lod::THRESHOLD, 1, &[]).is_empty());
    lod::select(&cam, &everything, Options { memo: Some(memo.bind(&cam, lod::THRESHOLD, 1, &[])), ..Options::default() });
    let moved = view(PERM.0, PERM.1, 2100.0, 30.0, 70.0);
    assert!(memo.bind(&moved, lod::THRESHOLD, 1, &[]).is_empty());
}

// Скорость.

#[test]
fn selection_fits_the_frame_budget() {
    // Бюджет модуля - 8 мс на выбор, здесь отладочная сборка.
    let cam = view(PERM.0, PERM.1, 2000.0, 30.0, 70.0);
    select(&cam, &everything);
    let started = std::time::Instant::now();
    for _ in 0..5 {
        select(&cam, &everything);
    }
    let spent = started.elapsed().as_secs_f64() / 5.0;
    assert!(spent < 0.008, "{:.1} мс", spent * 1000.0);
}

// Уклон и горизонтали - только вблизи глаза.

#[test]
fn far_tiles_dropped_near_kept() {
    let e = wgs84();
    let eye = e.geodetic_to_ecef(PERM.0, PERM.1, 2000.0);
    let (x0, y0) = tl::lonlat_to_tile(PERM.0, PERM.1, 12);
    let under = (12, x0, y0);
    // Около 400 км восточнее.
    let far = (12, x0 + 40, y0);
    let near = lod::near_keys(&e, &[under, far], eye, 30000.0);
    assert_eq!(near, HashSet::from([under]));
}

// Доля расстояния, а не метры: проверка при сборке.
const _: () = assert!(lod::ANALYSIS_REACH > 1.0);

// Дыра в долях тайла.

#[test]
fn hole_rect_is_the_child_share() {
    assert_eq!(lod::hole_rect((3, 5, 2), (4, 11, 4)), (0.5, 0.0, 0.5));
    assert_eq!(lod::hole_rect((3, 5, 2), (5, 22, 11)), (0.5, 0.75, 0.25));
    assert_eq!(lod::children((3, 5, 2)), [(4, 10, 4), (4, 11, 4), (4, 10, 5), (4, 11, 5)]);
}
