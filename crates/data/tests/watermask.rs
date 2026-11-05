// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Вода и суша ниже уровня моря по векторной основе. Перенос
//! `tests/test_watermask.py` модуля и проверки маски прохода воды.

use std::collections::HashMap;

use data::watermask::{self as wm, Feature, WaterMask};

fn varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

fn field(number: u64, wire: u64, payload: &[u8]) -> Vec<u8> {
    let mut out = varint((number << 3) | wire);
    out.extend(varint(payload.len() as u64));
    out.extend_from_slice(payload);
    out
}

fn field_varint(number: u64, value: u64) -> Vec<u8> {
    let mut out = varint(number << 3);
    out.extend(varint(value));
    out
}

fn zigzag(n: i64) -> u64 {
    ((n << 1) ^ (n >> 63)) as u64
}

/// Команды MVT кольца: MoveTo, LineTo, ClosePath. Смещения - от курсора,
/// конца прошлого кольца.
fn ring_commands(points: &[(i64, i64)], cursor: &mut (i64, i64)) -> Vec<u64> {
    let mut out = vec![(1 << 3) | 1];
    let (px, py) = points[0];
    out.extend([zigzag(px - cursor.0), zigzag(py - cursor.1)]);
    *cursor = (px, py);
    out.push(((points.len() as u64 - 1) << 3) | 2);
    for &(px, py) in &points[1..] {
        out.extend([zigzag(px - cursor.0), zigzag(py - cursor.1)]);
        *cursor = (px, py);
    }
    out.push((1 << 3) | 7);
    out
}

/// Тайл MVT со слоем name из одного многоугольника rings.
fn tile(name: &str, rings: &[&[(i64, i64)]]) -> Vec<u8> {
    let mut geometry = Vec::new();
    let mut cursor = (0, 0);
    for ring in rings {
        geometry.extend(ring_commands(ring, &mut cursor));
    }
    let packed: Vec<u8> = geometry.into_iter().flat_map(varint).collect();
    let mut feature = field_varint(3, 3);
    feature.extend(field(4, 2, &packed));
    let mut layer = field_varint(15, 2);
    layer.extend(field(1, 2, name.as_bytes()));
    layer.extend(field(2, 2, &feature));
    layer.extend(field_varint(5, 4096));
    field(3, 2, &layer)
}

// Вода - квадрат 1000-3000 по часовой стрелке (y вниз), остров внутри -
// квадрат 1800-2200 против неё.
const WATER: [(i64, i64); 4] = [(1000, 1000), (3000, 1000), (3000, 3000), (1000, 3000)];
const ISLAND: [(i64, i64); 4] = [(1800, 1800), (1800, 2200), (2200, 2200), (2200, 1800)];

fn decode(bytes: &[u8]) -> Vec<Feature> {
    let found = wm::decode_water(bytes);
    assert!(found.is_ok(), "{found:?}");
    found.unwrap_or_default()
}

#[test]
fn decode_and_inside() {
    let features = decode(&tile("water", &[&WATER, &ISLAND]));
    assert_eq!(features.len(), 1);
    assert_eq!(features[0].len(), 2);
    // Вода, остров, суша за краем воды.
    let wet: Vec<bool> = [(0.3, 0.3), (0.5, 0.5), (0.1, 0.1)].iter().map(|(x, y)| wm::inside(&features[0], *x, *y)).collect();
    assert_eq!(wet, [true, false, false]);
}

#[test]
fn other_layer_is_not_water() {
    assert!(decode(&tile("landcover", &[&WATER])).is_empty());
}

#[test]
fn broken_tile_is_error() {
    // Длина слоя за концом данных.
    assert!(wm::decode_water(&[0x1a, 0x7f, 0x01]).is_err());
}

#[test]
fn land_needs_tile() {
    let mut mask = WaterMask::default();
    // Дно Лебединского карьера.
    let (lat, lon) = (51.2896, 37.6329);
    let (key, fx, fy) = wm::tile_of(lat, lon, wm::LEVEL);
    assert!((0.0..1.0).contains(&fx) && (0.0..1.0).contains(&fy));
    // Тайла нет - не суша, метка остаётся на уровне моря.
    assert_eq!(mask.land(&[(lat, lon)]), [false]);
    assert_eq!(mask.missing(&[(lat, lon)]), [key].into_iter().collect());
    let before = mask.generation();
    mask.add(key, Vec::new());
    assert!(mask.generation() > before);
    assert_eq!(mask.land(&[(lat, lon)]), [true]);
    assert!(mask.missing(&[(lat, lon)]).is_empty());
    // Точка в воде тайла - не суша.
    let square = decode(&tile("water", &[&[(0, 0), (4096, 0), (4096, 4096), (0, 4096)]]));
    mask.add(key, square);
    assert_eq!(mask.land(&[(lat, lon)]), [false]);
}

#[test]
fn fine_tile_takes_part_of_water_tile() {
    // Тайл уровня 14 - четверть стороны тайла воды уровня 12.
    let parts = wm::mask_parts((14, 4 * 100 + 1, 4 * 200 + 3));
    assert_eq!(parts, [((12, 100, 200), [0.25, 0.75, 0.25], [0.0, 0.0, 1.0])]);
    // Вода в правой половине тайла воды: у этой четверти (x 0.25-0.5)
    // её нет, у четверти x 0.75-1 - вся.
    let half: Vec<Feature> = vec![vec![vec![[0.5, 0.0], [1.0, 0.0], [1.0, 1.0], [0.5, 1.0]]]];
    let tiles: HashMap<(u32, u32, u32), Vec<Feature>> = HashMap::from([((12, 100, 200), half)]);
    let rings = wm::mask_rings((14, 401, 803), &tiles).unwrap_or_default();
    assert_eq!(rings.len(), 1);
    assert!(!wm::inside(&rings[0], 0.5, 0.5));
    let rings = wm::mask_rings((14, 403, 803), &tiles).unwrap_or_default();
    assert!(wm::inside(&rings[0], 0.5, 0.5));
}

/// Отличие от модуля: у модуля тайл грубее уровня 12 собирал маску
/// из 4^(12 - z) тайлов воды уровня 12 и мельче уровня 10 маски не имел.
/// На виде с 2500 км тайлы уровней 4-6, и Прикаспийская низменность
/// заливалась водой. Здесь тайл грубее уровня 12 берёт свой тайл воды
/// того же уровня: слой water OpenMapTiles есть на всех уровнях.
#[test]
fn coarse_tile_takes_own_water_tile() {
    assert_eq!(wm::mask_parts((11, 10, 20)), [((11, 10, 20), [0.0, 0.0, 1.0], [0.0, 0.0, 1.0])]);
    assert_eq!(wm::mask_parts((5, 20, 11)), [((5, 20, 11), [0.0, 0.0, 1.0], [0.0, 0.0, 1.0])]);
    assert!(wm::mask_parts((wm::MASK_FROM - 1, 0, 0)).is_empty());
    let mut tiles: HashMap<(u32, u32, u32), Vec<Feature>> = HashMap::new();
    assert!(wm::mask_rings((11, 10, 20), &tiles).is_none());
    tiles.insert((11, 10, 20), Vec::new());
    assert_eq!(wm::mask_rings((11, 10, 20), &tiles), Some(Vec::new()));
}

/// Красный пикселя маски: первая строка - северный край.
fn red(image: &[u8], side: usize, x: usize, y: usize) -> u8 {
    image[(y * side + x) * 4]
}

#[test]
fn mask_image_fills_water_even_odd() {
    let features = decode(&tile("water", &[&WATER, &ISLAND]));
    let side = 64;
    let image = wm::mask_image(&features, side);
    assert_eq!(image.len(), side * side * 4);
    // Вода у левого верхнего угла квадрата, остров в середине, суша
    // в углу тайла.
    assert_eq!(red(&image, side, 20, 20), 255);
    assert_eq!(red(&image, side, 32, 32), 0);
    assert_eq!(red(&image, side, 2, 2), 0);
    assert_eq!(red(&image, side, 45, 45), 255);
    // Непрозрачна вся картинка.
    assert!(image.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
}

#[test]
fn mask_image_joins_features() {
    // Два объекта воды внахлёст: пересечение - вода, а не дыра чёт-нечет.
    let a: Feature = vec![vec![[0.0, 0.0], [0.6, 0.0], [0.6, 1.0], [0.0, 1.0]]];
    let b: Feature = vec![vec![[0.4, 0.0], [1.0, 0.0], [1.0, 1.0], [0.4, 1.0]]];
    let side = 32;
    let image = wm::mask_image(&[a, b], side);
    assert!(image.as_chunks::<4>().0.iter().all(|p| p[0] == 255));
    // Без воды - вся суша.
    assert!(wm::mask_image(&[], side).as_chunks::<4>().0.iter().all(|p| p[0] == 0 && p[3] == 255));
}

#[test]
fn mask_image_survives_hostile_rings() {
    // Пустые кольца, одна точка, нечисла и точки за краем тайла.
    let features: Vec<Feature> = vec![vec![Vec::new(), vec![[0.5, 0.5]], vec![[f64::NAN, 0.0], [1.0, f64::INFINITY], [0.0, 1.0]], vec![[-5.0, -5.0], [5.0, -5.0], [5.0, 5.0], [-5.0, 5.0]]]];
    let side = 16;
    let image = wm::mask_image(&features, side);
    assert_eq!(image.len(), side * side * 4);
    // Квадрат за краем закрывает весь тайл.
    assert_eq!(red(&image, side, 8, 8), 255);
    assert!(wm::mask_image(&[], 0).is_empty());
}
