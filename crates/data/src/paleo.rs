// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Палеогеография: карты Земли в прошлом по PaleoDEM PALEOMAP. Перенос
//! `core/paleo.py` модуля.
//!
//! Источник - Scotese & Wright 2018, PALEOMAP Paleodigital Elevation
//! Models, сетки высот и глубин 0.1° на возрасты AGES, CC BY 4.0. Выбран
//! автором 5 октября 2026 года вместо масок суши службы GPlates. Карта
//! возраста - тайлы JPEG Web Mercator с раскраской высот и отмывкой, их
//! нарезает tools/build_paleo.py модуля в хранилище planetx-terrain.
//! Окно ставит карту возраста подложкой. Здесь расчёт: возрасты, адрес
//! тайлов, название периода, раскраска и отмывка сетки.

/// Охват набора, млн лет.
pub const MAX_AGE: u32 = 540;
/// Возрасты карт набора 6 минут, млн лет: 109 сеток через 5 млн лет.
pub const AGES: [u32; (MAX_AGE / 5 + 1) as usize] = {
    let mut out = [0u32; (MAX_AGE / 5 + 1) as usize];
    let mut i = 0;
    while i < out.len() {
        out[i] = i as u32 * 5;
        i += 1;
    }
    out
};
/// Уровень тайлов, пиксель около 0.09° у экватора.
pub const MAX_LEVEL: u32 = 4;
/// Адрес тайла: поля `{age}`, `{z}`, `{x}`, `{y}`.
pub const TILE_URL: &str = "https://raw.githubusercontent.com/Valery35/planetx-terrain/main/paleo/paleomap/{age}/{z}/{x}/{y}.jpg";
pub const ATTRIBUTION: (&str, &str) = ("Paleogeography: PALEOMAP PaleoDEM, Scotese & Wright 2018, CC BY 4.0", "https://doi.org/10.5281/zenodo.5460860");
/// Нижние границы периодов, млн лет, шкала ICS, и ключи периодов.
pub const PERIODS: [(f64, &str); 12] = [
    (2.58, "quaternary"),
    (23.03, "neogene"),
    (66.0, "paleogene"),
    (145.0, "cretaceous"),
    (201.4, "jurassic"),
    (251.9, "triassic"),
    (298.9, "permian"),
    (358.9, "carboniferous"),
    (419.2, "devonian"),
    (443.8, "silurian"),
    (485.4, "ordovician"),
    (538.8, "cambrian"),
];
pub const PRECAMBRIAN: &str = "precambrian";
/// Раскраска высот: отметка в метрах и цвет. Ниже нуля - море, от
/// светлого шельфа к тёмной глубине, выше - суша от низменности к снегу
/// вершин. Цвета подобрал помощник модуля, утверждает автор.
pub const COLOR_STOPS: [(f64, [u8; 3]); 12] = [
    (-9000.0, [8, 28, 66]),
    (-6000.0, [16, 48, 102]),
    (-4000.0, [28, 72, 136]),
    (-2000.0, [48, 108, 168]),
    (-200.0, [92, 160, 200]),
    (-0.5, [138, 196, 222]),
    (0.0, [96, 140, 82]),
    (300.0, [132, 162, 96]),
    (1000.0, [186, 176, 118]),
    (2000.0, [166, 132, 94]),
    (3500.0, [146, 118, 108]),
    (5000.0, [236, 236, 236]),
];
/// Свет с северо-запада, градусы.
pub const SUN_AZIMUTH: f64 = 315.0;
/// Высота солнца над горизонтом, градусы.
pub const SUN_ALTITUDE: f64 = 45.0;
/// Подъём отмывки: на сетке 11 км уклоны малы, без подъёма горы плоские.
/// У моря подъём меньше, дно не спорит с сушей.
pub const LAND_RELIEF: f64 = 30.0;
pub const SEA_RELIEF: f64 = 8.0;
/// Средний радиус Земли, м.
pub const RADIUS: f64 = 6371008.8;

/// Адрес тайла карты возраста age.
pub fn tile_url(age: u32, z: u32, x: u32, y: u32) -> String {
    TILE_URL.replace("{age}", &age.to_string()).replace("{z}", &z.to_string()).replace("{x}", &x.to_string()).replace("{y}", &y.to_string())
}

/// Канал k цвета высоты z, как `np.interp` по COLOR_STOPS.
fn channel(z: f64, k: usize) -> f64 {
    let stops = &COLOR_STOPS;
    if z <= stops[0].0 {
        return f64::from(stops[0].1[k]);
    }
    for pair in stops.windows(2) {
        let ((a, ca), (b, cb)) = (pair[0], pair[1]);
        if z <= b {
            let t = (z - a) / (b - a);
            return f64::from(ca[k]) + (f64::from(cb[k]) - f64::from(ca[k])) * t;
        }
    }
    if z.is_nan() {
        return f64::NAN;
    }
    f64::from(stops[stops.len() - 1].1[k])
}

/// Цвет высот по COLOR_STOPS. Высота не число - чёрный.
pub fn colors(heights: &[f64]) -> Vec<[u8; 3]> {
    heights.iter().map(|&z| [0, 1, 2].map(|k| channel(z, k).round_ties_even().clamp(0.0, 255.0) as u8)).collect()
}

/// Отмывка сетки высот шириной width через step градусов, север
/// в строке 0: множитель яркости, на равнине около 1. None - длина
/// не кратна ширине.
pub fn hillshade(heights: &[f64], width: usize, step: f64) -> Option<Vec<f64>> {
    if width == 0 || !heights.len().is_multiple_of(width) {
        return None;
    }
    let rows = heights.len() / width;
    let dy = step.to_radians() * RADIUS;
    let (azimuth, altitude) = (SUN_AZIMUTH.to_radians(), SUN_ALTITUDE.to_radians());
    let sun = [azimuth.sin() * altitude.cos(), azimuth.cos() * altitude.cos(), altitude.sin()];
    let at = |r: usize, c: usize| heights[r * width + c];
    // Наклон к северу строки r: края по широте повторяют соседа.
    let north_of = |r: usize, c: usize| {
        if rows < 3 {
            return 0.0;
        }
        let r = r.clamp(1, rows - 2);
        (at(r - 1, c) - at(r + 1, c)) / (2.0 * dy)
    };
    let mut out = Vec::with_capacity(heights.len());
    for r in 0..rows {
        let lat = 90.0 - (r as f64 + 0.5) * 180.0 / rows as f64;
        let dx = lat.to_radians().cos().max(0.01) * dy;
        for c in 0..width {
            // Долгота замкнута.
            let (east_c, west_c) = ((c + 1) % width, (c + width - 1) % width);
            let z = at(r, c);
            let relief = if z < 0.0 { SEA_RELIEF } else { LAND_RELIEF };
            let east = (at(r, east_c) - at(r, west_c)) / (2.0 * dx) * relief;
            let north = north_of(r, c) * relief;
            let length = (east * east + north * north + 1.0).sqrt();
            let light = (-east * sun[0] - north * sun[1] + sun[2]) / length;
            out.push((light / sun[2]).clamp(0.35, 1.4));
        }
    }
    Some(out)
}

/// Карта возраста: цвет высот с отмывкой. None - длина не кратна ширине.
pub fn picture(heights: &[f64], width: usize, step: f64) -> Option<Vec<[u8; 3]>> {
    let shade = hillshade(heights, width, step)?;
    Some(colors(heights).into_iter().zip(shade).map(|(rgb, s)| rgb.map(|v| (f64::from(v) * s).round_ties_even().clamp(0.0, 255.0) as u8)).collect())
}

/// Ключ периода для возраста age млн лет.
pub fn period(age: f64) -> &'static str {
    PERIODS.iter().find(|(base, _)| age < *base).map_or(PRECAMBRIAN, |p| p.1)
}

/// Ближайший к age возраст набора AGES.
pub fn nearest(age: f64) -> u32 {
    nearest_in(age, &AGES)
}

/// Ближайший к age возраст из ages, из двух равных - младший. Пустой
/// набор - AGES.
pub fn nearest_in(age: f64, ages: &[u32]) -> u32 {
    let ages = if ages.is_empty() { &AGES[..] } else { ages };
    let key = |v: u32| ((f64::from(v) - age).abs(), v);
    ages.iter().copied().min_by(|a, b| {
        let (ka, kb) = (key(*a), key(*b));
        ka.0.total_cmp(&kb.0).then(ka.1.cmp(&kb.1))
    })
    .unwrap_or(0)
}
