// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Слои векторного тайла и выбор надписей, перенос
//! `tests/test_places.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::io::Write;

use data::mvt::{self, Value};
use data::places::{self as pl, Kind, Place, PlaceStore};

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

fn field_varint(number: u64, value: u64) -> Vec<u8> {
    let mut out = varint(number << 3);
    out.extend(varint(value));
    out
}

fn field_bytes(number: u64, payload: &[u8]) -> Vec<u8> {
    let mut out = varint((number << 3) | 2);
    out.extend(varint(payload.len() as u64));
    out.extend_from_slice(payload);
    out
}

fn zigzag(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

#[derive(Clone)]
enum V {
    S(&'static str),
    I(u64),
}

fn value_message(v: &V) -> Vec<u8> {
    match v {
        V::S(s) => field_bytes(1, s.as_bytes()),
        V::I(i) => field_varint(5, *i),
    }
}

type Feat = (u64, u64, Vec<(&'static str, V)>, Vec<(i64, i64)>);

/// Слой MVT. features - номер, тип, атрибуты, точки.
fn layer(name: &str, features: &[Feat]) -> Vec<u8> {
    let mut keys: Vec<&str> = Vec::new();
    let mut values: Vec<Vec<u8>> = Vec::new();
    let mut body = field_varint(15, 2);
    body.extend(field_bytes(1, name.as_bytes()));
    for (fid, kind, attrs, points) in features {
        let mut tags = Vec::new();
        for (k, v) in attrs {
            if !keys.contains(k) {
                keys.push(k);
            }
            let message = value_message(v);
            if !values.contains(&message) {
                values.push(message.clone());
            }
            tags.extend(varint(keys.iter().position(|x| x == k).unwrap() as u64));
            tags.extend(varint(values.iter().position(|x| *x == message).unwrap() as u64));
        }
        let mut geometry = varint(((points.len() as u64) << 3) | 1);
        let (mut x, mut y) = (0, 0);
        for (px, py) in points {
            geometry.extend(varint(zigzag(px - x)));
            geometry.extend(varint(zigzag(py - y)));
            (x, y) = (*px, *py);
        }
        let mut feature = field_varint(1, *fid);
        feature.extend(field_bytes(2, &tags));
        feature.extend(field_varint(3, *kind));
        feature.extend(field_bytes(4, &geometry));
        body.extend(field_bytes(2, &feature));
    }
    for k in keys {
        body.extend(field_bytes(3, k.as_bytes()));
    }
    for v in values {
        body.extend(field_bytes(4, &v));
    }
    body.extend(field_varint(5, 4096));
    field_bytes(3, &body)
}

fn perm() -> Vec<(&'static str, V)> {
    vec![("class", V::S("city")), ("name", V::S("Perm")), ("name:ru", V::S("Пермь")), ("rank", V::I(4))]
}

fn tile() -> Vec<u8> {
    let mut out = layer("water", &[(1, 3, vec![("class", V::S("lake"))], vec![(5, 5)])]);
    out.extend(layer(
        "place",
        &[
            (11, 1, perm(), vec![(1000, 2000)]),
            (
                12,
                1,
                vec![("class", V::S("village")), ("name", V::S("Kultayevo")), ("rank", V::I(12))],
                vec![(1100, 2100)],
            ),
            (13, 1, vec![("class", V::S("suburb")), ("name", V::S("Motovilikha"))], vec![(1200, 2200)]),
            (14, 1, vec![("class", V::S("city")), ("name", V::S("Moscow")), ("capital", V::I(2))], vec![(10, 10)]),
            (15, 1, vec![("class", V::S("town")), ("name", V::S("Buffer"))], vec![(-40, 100)]),
            (16, 2, vec![("class", V::S("city")), ("name", V::S("Line"))], vec![(1, 1), (2, 2)]),
        ],
    ));
    out
}

fn langs(codes: &[&str]) -> Vec<String> {
    codes.iter().map(|s| s.to_string()).collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-7
}

#[test]
fn place_layer_only_points() {
    let (features, extent) = mvt::decode_layer(&tile(), "place", None).unwrap();
    assert_eq!(extent, 4096);
    assert_eq!(features.iter().map(|f| f.id).collect::<Vec<_>>(), [11, 12, 13, 14, 15]);
    assert_eq!(features[0].attrs["name:ru"], Value::Str("Пермь".into()));
    assert_eq!(features[0].attrs["rank"], Value::Int(4));
    assert_eq!(features[0].points, [(1000, 2000)]);
    assert_eq!(features[4].points, [(-40, 100)]);
}

#[test]
fn gzip() {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&tile()).unwrap();
    let packed = encoder.finish().unwrap();
    assert_eq!(mvt::decode_layer(&packed, "place", None), mvt::decode_layer(&tile(), "place", None));
}

#[test]
fn missing_layer() {
    assert_eq!(mvt::decode_layer(&layer("water", &[]), "place", None).unwrap(), (vec![], 4096));
}

#[test]
fn broken_tile() {
    let t = tile();
    assert!(mvt::decode_layer(&t[..t.len() - 3], "place", None).is_err());
}

#[test]
fn places() {
    let key = (8, 170, 76);
    let out = pl::decode_places(key, &tile(), &langs(&["ru"])).unwrap();
    // Пригород не показывается, точка за краем тайла - дубль соседа.
    assert_eq!(out.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Пермь", "Kultayevo", "Moscow"]);
    assert_eq!(out.iter().map(|p| p.kind).collect::<Vec<_>>(), [Kind::City, Kind::Village, Kind::Capital]);
    assert_eq!(out.iter().map(|p| p.rank).collect::<Vec<_>>(), [4, 12, 99]);
    let (lat, _) = pl::tile_point(8, 170, 76, 1000.0, 2000.0, 4096);
    assert!(close(out[0].lat, lat));
    let english = pl::decode_places(key, &tile(), &langs(&["en"])).unwrap();
    assert_eq!(english[0].name, "Perm");
}

#[test]
fn tile_point_matches_tiling() {
    // Границы тайла (8, 170, 76) по Web Mercator.
    let n = 256.0;
    let west = 170.0 / n * 360.0 - 180.0;
    let east = 171.0 / n * 360.0 - 180.0;
    let lat = |y: f64| (std::f64::consts::PI * (1.0 - 2.0 * y / n)).sinh().atan().to_degrees();
    let (north, south) = (lat(76.0), lat(77.0));
    let (a, b) = pl::tile_point(8, 170, 76, 0.0, 0.0, 4096);
    assert!(close(a, north) && close(b, west));
    let (a, b) = pl::tile_point(8, 170, 76, 4096.0, 4096.0, 4096);
    assert!(close(a, south) && close(b, east));
}

#[test]
fn levels() {
    let out: Vec<u32> = [0, 2, 5, 16, 19].iter().map(|z| pl::label_level(*z, pl::MAX_LEVEL)).collect();
    assert_eq!(out, [0, 0, 3, 14, 14]);
}

#[test]
fn importance() {
    let a = Place::new(1, "B", Kind::City, 5, 0.0, 0.0);
    let b = Place::new(2, "A", Kind::Capital, 9, 0.0, 0.0);
    let c = Place::new(3, "C", Kind::City, 2, 0.0, 0.0);
    let d = Place::new(4, "D", Kind::Village, 1, 0.0, 0.0);
    let mut all = vec![a.clone(), b.clone(), c.clone(), d.clone()];
    all.sort_by(|x, y| pl::importance(x).cmp(&pl::importance(y)));
    assert_eq!(all, [b, c, a, d]);
}

fn others() -> Vec<u8> {
    let mut out = layer(
        "mountain_peak",
        &[(
            21,
            1,
            vec![
                ("class", V::S("volcano")),
                ("name", V::S("Elbrus")),
                ("name:ru", V::S("Эльбрус")),
                ("ele", V::I(5642)),
                ("rank", V::I(1)),
            ],
            vec![(100, 100)],
        )],
    );
    out.extend(layer(
        "water_name",
        &[
            (22, 2, vec![("class", V::S("lake")), ("name", V::S("Kama Reservoir"))], vec![(10, 10), (20, 20), (30, 40)]),
            (23, 1, vec![("class", V::S("sea")), ("name", V::S("Caspian Sea"))], vec![(50, 50)]),
        ],
    ));
    out.extend(layer(
        "park",
        &[
            (24, 1, vec![("class", V::S("national_park")), ("name", V::S("Prielbrusye"))], vec![(60, 60)]),
            (25, 1, vec![("class", V::S("park")), ("name", V::S("City park"))], vec![(61, 61)]),
        ],
    ));
    out.extend(layer(
        "aerodrome_label",
        &[(26, 1, vec![("class", V::S("international")), ("name", V::S("Bolshoye Savino"))], vec![(70, 70)])],
    ));
    out.extend(layer(
        "transportation_name",
        &[
            (27, 2, vec![("class", V::S("trunk")), ("ref", V::S("Р-243;E 22"))], vec![(0, 0), (80, 80), (160, 160)]),
            (28, 2, vec![("class", V::S("minor")), ("ref", V::S("57К-0001"))], vec![(1, 1), (2, 2)]),
            (29, 2, vec![("class", V::S("trunk")), ("name", V::S("No ref"))], vec![(3, 3), (4, 4)]),
        ],
    ));
    out
}

fn other_places() -> std::collections::HashMap<String, Place> {
    pl::decode_places((8, 170, 76), &others(), &langs(&["ru"])).unwrap().into_iter().map(|p| (p.name.clone(), p)).collect()
}

#[test]
fn kinds_and_info() {
    let found = other_places();
    let mut names: Vec<&str> = found.keys().map(String::as_str).collect();
    names.sort_unstable();
    let mut expected = ["Эльбрус", "Kama Reservoir", "Caspian Sea", "Prielbrusye", "Bolshoye Savino", "Р-243"];
    expected.sort_unstable();
    assert_eq!(names, expected);
    assert_eq!(found["Эльбрус"].kind, Kind::Peak);
    assert_eq!(found["Эльбрус"].info, Some(5642));
    assert_eq!(found["Caspian Sea"].rank, 2);
    assert_eq!(found["Kama Reservoir"].rank, 5);
    assert_eq!(found["Prielbrusye"].kind, Kind::Park);
    assert_eq!(found["Bolshoye Savino"].kind, Kind::Airport);
    assert_eq!(found["Р-243"].kind, Kind::RoadRef);
    assert_eq!(found["Р-243"].info, None);
}

#[test]
fn line_anchor_is_middle_vertex() {
    let found = other_places();
    let (lat, lon) = pl::tile_point(8, 170, 76, 20.0, 20.0, 4096);
    assert!(close(found["Kama Reservoir"].lat, lat) && close(found["Kama Reservoir"].lon, lon));
    let (lat, _) = pl::tile_point(8, 170, 76, 80.0, 80.0, 4096);
    assert!(close(found["Р-243"].lat, lat));
}

#[test]
fn collect_by_kinds() {
    let mut store = PlaceStore::default();
    store.add((6, 42, 18), other_places().into_values().collect());
    let draw = [(8, 168, 72)];
    let kinds: HashSet<Kind> = store.collect(&draw, None).iter().map(|p| p.kind).collect();
    assert_eq!(kinds, HashSet::from([Kind::Peak, Kind::Water, Kind::Park, Kind::Airport, Kind::RoadRef]));
    let peaks = store.collect(&draw, Some(&HashSet::from([Kind::Peak])));
    assert_eq!(peaks.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Эльбрус"]);
    assert!(store.collect(&draw, Some(&HashSet::new())).is_empty());
}

#[test]
fn wanted_levels() {
    let store = PlaceStore::default();
    assert_eq!(store.wanted(&[(10, 600, 300), (10, 601, 301), (1, 1, 0)]), HashSet::from([(8, 150, 75), (0, 0, 0)]));
    assert_eq!(store.wanted(&[(19, 1 << 18, 5)]), HashSet::from([(14, 1 << 13, 0)]));
}

#[test]
fn source_falls_back_to_ancestor() {
    let mut store = PlaceStore::default();
    assert_eq!(store.source((10, 600, 300)), None);
    store.add((6, 37, 18), vec![]);
    assert_eq!(store.source((10, 600, 300)), Some((6, 37, 18)));
    store.add((8, 150, 75), vec![]);
    assert_eq!(store.source((10, 600, 300)), Some((8, 150, 75)));
}

#[test]
fn collect_prefers_finer_and_sorts() {
    let mut store = PlaceStore::default();
    let coarse = Place::new(7, "Пермь", Kind::City, 4, 58.0, 56.2);
    let fine = Place::new(7, "Пермь", Kind::City, 4, 58.01, 56.23);
    let village = Place::new(9, "Култаево", Kind::Village, 12, 57.9, 55.9);
    store.add((6, 37, 18), vec![coarse]);
    store.add((8, 150, 75), vec![village.clone(), fine.clone()]);
    let out = store.collect(&[(10, 600, 300), (7, 74, 36)], None);
    assert_eq!(*out, [fine, village]);
    let again = store.collect(&[(10, 600, 300), (7, 74, 36)], None);
    assert!(std::sync::Arc::ptr_eq(&out, &again));
}

#[test]
fn states_only_from_finer_tiles() {
    let mut store = PlaceStore::default();
    let state = Place::new(5, "Пермский край", Kind::State, 2, 59.0, 56.0);
    let country = Place::new(6, "Россия", Kind::Country, 1, 60.0, 90.0);
    store.add((2, 2, 1), vec![state.clone(), country.clone()]);
    assert_eq!(*store.collect(&[(4, 11, 5)], None), [country]);
    store.add((3, 5, 2), vec![state.clone()]);
    assert_eq!(*store.collect(&[(5, 22, 10)], None), [state]);
}

fn attrs() -> std::collections::HashMap<String, Value> {
    [("name", "Пермь"), ("name:de", "Perm"), ("name_latin", "Perm'"), ("name:zh", "彼尔姆")]
        .into_iter()
        .map(|(k, v)| (k.to_string(), Value::Str(v.to_string())))
        .collect()
}

fn name(choice: &str) -> Option<String> {
    pl::place_name(&attrs(), &pl::name_languages(choice, "ru"))
}

#[test]
fn as_interface_follows_interface() {
    assert_eq!(pl::name_languages(pl::AS_INTERFACE, "ru"), langs(&["ru"]));
    assert_eq!(pl::name_languages(pl::AS_INTERFACE, "de"), langs(&["de", "latin"]));
}

#[test]
fn unknown_interface_language_gives_english() {
    assert_eq!(pl::name_languages(pl::AS_INTERFACE, "fi"), langs(&["en", "latin"]));
}

#[test]
fn local_names() {
    assert_eq!(name(pl::LOCAL).as_deref(), Some("Пермь"));
}

#[test]
fn chosen_language() {
    assert_eq!(name("de").as_deref(), Some("Perm"));
    assert_eq!(name("zh").as_deref(), Some("彼尔姆"));
}

#[test]
fn fallback_latin_or_local() {
    assert_eq!(name("fr").as_deref(), Some("Perm'"));
    assert_eq!(name("ja").as_deref(), Some("Пермь"));
}

fn kinds() -> HashSet<Kind> {
    HashSet::from([Kind::City, Kind::Peak, Kind::Airport, Kind::RoadRef, Kind::Park])
}

#[test]
fn airports_and_road_numbers_only_below_1000_km() {
    assert!(pl::kinds_at(&kinds(), 9.9e5).is_superset(&HashSet::from([Kind::Airport, Kind::RoadRef])));
    assert_eq!(pl::kinds_at(&kinds(), 1.01e6), HashSet::from([Kind::City, Kind::Park]));
}

#[test]
fn peaks_only_below_400_km() {
    assert!(pl::kinds_at(&kinds(), 3.9e5).contains(&Kind::Peak));
    assert!(!pl::kinds_at(&kinds(), 4.1e5).contains(&Kind::Peak));
}

#[test]
fn parks_only_below_3000_km() {
    assert!(pl::kinds_at(&kinds(), 2.9e6).contains(&Kind::Park));
    assert_eq!(pl::kinds_at(&kinds(), 3.1e6), HashSet::from([Kind::City]));
}

#[test]
fn other_classes_have_no_limit() {
    let k = HashSet::from([Kind::City, Kind::Country]);
    assert_eq!(pl::kinds_at(&k, 2.0e7), k);
}

const CELL: f64 = 64.0;

#[test]
fn overlap_keeps_first() {
    let boxes = [[0.0, 0.0, 100.0, 20.0], [50.0, 10.0, 150.0, 30.0], [200.0, 0.0, 300.0, 20.0]];
    assert_eq!(pl::select_labels(&boxes, &[0, 0, 0], &HashSet::new(), CELL), [0, 2]);
}

#[test]
fn touching_boxes_do_not_clash() {
    let boxes = [[0.0, 0.0, 100.0, 20.0], [100.0, 0.0, 200.0, 20.0]];
    assert_eq!(pl::select_labels(&boxes, &[0, 0], &HashSet::new(), CELL), [0, 1]);
}

#[test]
fn previous_wins_inside_class_only() {
    let mut boxes = [[0.0, 0.0, 100.0, 20.0], [50.0, 10.0, 150.0, 30.0], [60.0, 5.0, 160.0, 25.0]];
    let tiers = [2, 4, 4];
    let previous = HashSet::from([2]);
    // Надпись 2 была видна, внутри класса 4 она идёт раньше 1.
    assert_eq!(pl::select_labels(&boxes, &tiers, &previous, CELL), [0]);
    boxes[0] = [1000.0, 0.0, 1100.0, 20.0];
    assert_eq!(pl::select_labels(&boxes, &tiers, &previous, CELL), [0, 2]);
    // Прошлая надпись класса ниже не вытесняет более важную.
    let small = [[0.0, 0.0, 10.0, 10.0], [5.0, 5.0, 15.0, 15.0]];
    assert_eq!(pl::select_labels(&small, &[1, 3], &HashSet::from([1]), CELL), [0]);
}

#[test]
fn matches_brute_force() {
    // Свой генератор вместо random модуля: последовательность другая,
    // проверка та же - сетка даёт то же, что перебор.
    let mut state = 3u64;
    let mut next = || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut boxes = Vec::new();
    for _ in 0..300 {
        let (x, y) = (next() * 1500.0, next() * 900.0);
        boxes.push([x, y, x + 20.0 + next() * 180.0, y + 10.0 + next() * 20.0]);
    }
    let tiers = vec![0; boxes.len()];
    let mut placed: Vec<usize> = Vec::new();
    for (i, [x0, y0, x1, y1]) in boxes.iter().enumerate() {
        if placed.iter().all(|&j| {
            let [a0, b0, a1, b1] = boxes[j];
            !(*x0 < a1 && a0 < *x1 && *y0 < b1 && b0 < *y1)
        }) {
            placed.push(i);
        }
    }
    assert_eq!(pl::select_labels(&boxes, &tiers, &HashSet::new(), CELL), placed);
}

#[test]
fn hostile_tile_gives_error_not_crash() {
    // Длина поля за концом данных, неизвестный тип поля, битый gzip.
    for bad in [&[0x1a, 0xff, 0x01][..], &[0x0b][..], &[0x1f, 0x8b, 0x00][..], &[0x1a, 0x80][..]] {
        assert!(pl::decode_places((0, 0, 0), bad, &[]).is_err(), "{bad:?}");
    }
}
