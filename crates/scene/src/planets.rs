// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Тела глобуса: Земля, Марс, Луна и прочие тела меню, перенос
//! `core/planets.py` модуля.
//!
//! Размеры тела лежат в [`globe_core::ellipsoid`]. Здесь - снимки тела,
//! воздух, домашняя точка и то, есть ли у тела земные функции. Марс -
//! цветная мозаика Viking MDIM2.1 (NASA, USGS) до уровня 7, копия тайлов
//! OpenPlanetaryMap в planetx-bodies-imagery. Луна - мозаика LROC WAC
//! 100 м, файл COG хранилища astrogeo-ard USGS. Снимки прочих тел - тайлы
//! JPEG из глобальных
//! мозаик USGS Astrogeology и NASA Photojournal, общественное достояние,
//! в хранилище planetx-terrain. Высоты Марса и Луны - тайлы Terrarium
//! уровней 0-5 там же.
//!
//! Воздух задаётся множителем рассеяния к земному по каналам RGB. У Марса
//! пыльный воздух рассеивает красный сильнее синего, у Луны воздуха нет.
//! Цвета подобраны модулем на снимке, их утверждает автор.

use globe_core::ellipsoid::{self, Body};

/// Viking MDIM2.1 - копия тайлов OpenPlanetaryMap в planetx-bodies-imagery,
/// PNG как есть, ряды XYZ: S3 OpenPlanetaryMap не отдаёт ответ странице
/// (CORS). Копия от 25 октября 2026 года.
pub const MARS_VIKING: &str = "https://raw.githubusercontent.com/Valery35/planetx-bodies-imagery/main/mars/{z}/{x}/{y}.png";
pub const MOON_ALBEDO: &str = "https://s3.amazonaws.com/opmbuilder/301_moon/tiles/w/hillshaded-albedo/{z}/{x}/{-y}.png";
/// Глобальная мозаика Луны LROC WAC 100 м, COG в хранилище astrogeo-ard
/// USGS, с CORS: Lite читает её окнами, без своих копий.
pub const MOON_WAC: &str = "https://astrogeo-ard.s3.us-west-2.amazonaws.com/moon/basemaps/lroc_wac_mosaic/lroc_wac_mosaic_global_303ppd_v3/lroc_wac_mosaic_global_303ppd_v3.tif";
/// Высоты тел, Terrarium, `TERRAIN_TILES` модуля, {key} - ключ тела.
pub const TERRAIN_TILES: &str = "https://raw.githubusercontent.com/Valery35/planetx-terrain/main/{key}/{z}/{x}/{y}.png";
pub const TERRAIN_LEVEL: u8 = 5;
/// Снимки прочих тел, {key} - ключ тела.
pub const IMAGERY_TILES: &str = "https://raw.githubusercontent.com/Valery35/planetx-terrain/main/imagery/{key}/{z}/{x}/{y}.jpg";
pub const USGS_MAPS: &str = "https://astrogeology.usgs.gov/search";
pub const PHOTOJOURNAL: &str = "https://science.nasa.gov/photojournal/";
/// Начальный вид прочих тел - 3.5 радиуса от точки взгляда.
pub const HOME_RADII: f64 = 3.5;

/// Снимки тела: название мозаики, адрес тайлов, предельный уровень,
/// подпись источника, ссылка. cog - адрес не шаблон тайлов, а файл COG
/// всей мозаики, clon - центральный меридиан его проекции в градусах:
/// Lite не читает параметры проекции из файла.
#[derive(Debug, Clone, PartialEq)]
pub struct Imagery {
    pub name: &'static str,
    pub url: String,
    pub max_level: u8,
    pub credit: &'static str,
    pub link: &'static str,
    pub cog: bool,
    pub clon: f64,
}

/// Высоты тела: название, адрес тайлов Terrarium, предельный уровень,
/// подпись.
#[derive(Debug, Clone, PartialEq)]
pub struct Terrain {
    pub name: &'static str,
    pub url: String,
    pub max_level: u8,
    pub credit: &'static str,
}

/// Тело глобуса. imagery None - земные подложки. air - множитель
/// рассеяния воздуха по R, G, B к земному, None - воздуха нет. home -
/// широта, долгота и расстояние начального вида. earth - есть ли земные
/// функции: поиск места, векторная основа, здания, облака, солнце, слои
/// Земли. terrain None у Земли - её высоты задаёт окно.
#[derive(Debug, Clone, PartialEq)]
pub struct Planet {
    pub body: Body,
    pub imagery: Option<Imagery>,
    pub air: Option<[f32; 3]>,
    pub home: (f64, f64, f64),
    pub earth: bool,
    pub terrain: Option<Terrain>,
}

impl Planet {
    pub fn key(&self) -> &'static str {
        self.body.key
    }
}

/// Прочие тела: ключ, название мозаики, уровень, подпись, ссылка,
/// воздух, широта и долгота начальной точки. Воздух у Венеры и Титана -
/// плотная дымка.
type Other = (&'static str, &'static str, u8, &'static str, &'static str, Option<[f32; 3]>, (f64, f64));

const OTHER: [Other; 16] = [
    ("mercury", "MESSENGER MDIS MD3 color", 6, "NASA, JHU APL, Carnegie Institution, MESSENGER MDIS, USGS", USGS_MAPS, None, (30.5, -170.2)),
    ("venus", "Magellan C3-MDIR colorized topography", 5, "NASA, JPL, Magellan, USGS", USGS_MAPS, Some([1.3, 0.55, 0.1]), (65.2, 3.3)),
    ("jupiter", "Cassini PIA07782", 4, "NASA, JPL, Space Science Institute, Cassini", PHOTOJOURNAL, None, (0.0, 0.0)),
    ("io", "Galileo SSI and Voyager color merge", 5, "NASA, JPL, Galileo SSI, Voyager, USGS", USGS_MAPS, None, (0.0, 0.0)),
    ("europa", "Voyager and Galileo SSI", 6, "NASA, JPL, Galileo SSI, Voyager, USGS", USGS_MAPS, None, (0.0, 0.0)),
    ("ganymede", "Voyager and Galileo SSI color", 5, "NASA, JPL, Galileo SSI, Voyager, USGS", USGS_MAPS, None, (0.0, 0.0)),
    ("callisto", "Voyager and Galileo SSI", 6, "NASA, JPL, Galileo SSI, Voyager, USGS", USGS_MAPS, None, (0.0, 0.0)),
    ("mimas", "Cassini PIA17214", 5, "NASA, JPL-Caltech, Space Science Institute, Cassini", PHOTOJOURNAL, None, (0.0, 0.0)),
    ("enceladus", "Cassini ISS", 5, "NASA, JPL, Space Science Institute, Cassini ISS, USGS", USGS_MAPS, None, (0.0, 0.0)),
    ("tethys", "Cassini ISS", 5, "NASA, JPL, Space Science Institute, Cassini ISS, USGS", USGS_MAPS, None, (0.0, 0.0)),
    ("dione", "Cassini ISS and Voyager", 6, "NASA, JPL, Space Science Institute, Cassini ISS, USGS", USGS_MAPS, None, (0.0, 0.0)),
    ("rhea", "Cassini ISS and Voyager", 5, "NASA, JPL, Space Science Institute, DLR, Cassini ISS, USGS", USGS_MAPS, None, (0.0, 0.0)),
    ("titan", "Cassini ISS", 4, "NASA, JPL-Caltech, Space Science Institute, Cassini ISS, USGS", USGS_MAPS, Some([1.3, 0.4, 0.06]), (0.0, 0.0)),
    ("iapetus", "Cassini ISS and Voyager", 5, "NASA, JPL, Space Science Institute, Cassini ISS, USGS", USGS_MAPS, None, (0.0, 0.0)),
    ("ceres", "Dawn FC", 5, "NASA, JPL-Caltech, UCLA, MPS, DLR, IDA, Dawn FC, USGS", USGS_MAPS, None, (0.0, 0.0)),
    ("vesta", "Dawn FC HAMO", 6, "NASA, JPL-Caltech, UCLA, MPS, DLR, IDA, Dawn FC, USGS", USGS_MAPS, None, (0.0, 0.0)),
];

/// Меню «Тело»: Земля, Луна и Марс сверху, прочие тела - подменю.
/// Пустой заголовок - тела в самом меню после разделителя. Плутона,
/// Харона и Тритона нет: 24-33 % их шара аппараты не сняли. Решение
/// автора модуля от 1 октября 2026 года - в меню только тела, снятые
/// целиком.
pub const MENU: [(&str, &[&str]); 6] = [
    ("", &["earth"]),
    ("", &["moon", "mars"]),
    ("planets", &["mercury", "venus", "jupiter"]),
    ("jupiter", &["io", "europa", "ganymede", "callisto"]),
    ("saturn", &["mimas", "enceladus", "tethys", "dione", "rhea", "titan", "iapetus"]),
    ("asteroids", &["ceres", "vesta"]),
];

fn terrain(key: &str, name: &'static str, credit: &'static str) -> Option<Terrain> {
    Some(Terrain { name, url: TERRAIN_TILES.replace("{key}", key), max_level: TERRAIN_LEVEL, credit })
}

/// Тело по ключу, неизвестный ключ - Земля.
pub fn planet_by_key(key: &str) -> Planet {
    match key {
        "mars" => Planet {
            body: ellipsoid::MARS,
            imagery: Some(Imagery {
                name: "Mars Viking MDIM2.1",
                url: MARS_VIKING.to_string(),
                max_level: 7,
                credit: "NASA, USGS, Viking MDIM2.1, OpenPlanetaryMap",
                link: "https://www.openplanetary.org/opm-basemaps/global-viking-mdim2-1-colorized-mosaic",
                cog: false,
                clon: 0.0,
            }),
            air: Some([1.2, 0.3, 0.07]),
            // Олимп.
            home: (18.65, -133.8, 1.2e7),
            earth: false,
            terrain: terrain("mars", "MOLA MEGDR", "Terrain: NASA MGS MOLA MEGDR"),
        },
        "moon" => Planet {
            body: ellipsoid::MOON,
            // LROC WAC 100 м вместо альбедо OpenPlanetaryMap 670 м - выбор
            // помощника 25 октября 2026 года, утверждает автор.
            imagery: Some(Imagery {
                name: "Moon LRO LROC WAC 100 m",
                url: MOON_WAC.to_string(),
                max_level: 9,
                credit: "NASA, GSFC, Arizona State University, LRO LROC WAC, USGS",
                link: "https://astrogeology.usgs.gov/search/map/moon_lro_lroc_wac_global_morphology_mosaic_100m",
                cog: true,
                // В файле WAC «Equirectangular, clon = 180».
                clon: 180.0,
            }),
            air: None,
            // Море Спокойствия, посадка Аполлона-11.
            home: (0.674, 23.473, 6.0e6),
            earth: false,
            terrain: terrain("moon", "LOLA GDR", "Terrain: NASA LRO LOLA GDR"),
        },
        _ => match OTHER.iter().find(|o| o.0 == key) {
            Some((key, name, level, credit, link, air, home)) => {
                let body = *ellipsoid::body_by_key(key);
                Planet {
                    body,
                    imagery: Some(Imagery { name, url: IMAGERY_TILES.replace("{key}", key), max_level: *level, credit, link, cog: false, clon: 0.0 }),
                    air: *air,
                    home: (home.0, home.1, HOME_RADII * body.a),
                    earth: false,
                    terrain: None,
                }
            }
            None => Planet { body: ellipsoid::EARTH, imagery: None, air: Some([1.0, 1.0, 1.0]), home: (58.0105, 56.2294, 2.0e7), earth: true, terrain: None },
        },
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn menu_bodies_exist() {
        // Каждый ключ меню - тело с размерами и снимками, Земля первая.
        let keys: Vec<&str> = MENU.iter().flat_map(|(_, group)| group.iter().copied()).collect();
        assert_eq!(keys[0], "earth");
        for key in keys {
            let planet = planet_by_key(key);
            assert_eq!(planet.key(), key);
            assert_eq!(ellipsoid::body_by_key(key), &planet.body);
            assert_eq!(planet.imagery.is_none(), key == "earth", "{key}");
        }
    }

    #[test]
    fn mars_and_moon_are_tms() {
        // С рядами XYZ Луна у модуля 30 сентября 2026 года вышла полосами.
        // Копия Марса - в рядах XYZ, у OpenPlanetaryMap были TMS.
        assert!(planet_by_key("mars").imagery.unwrap().url.contains("planetx-bodies-imagery"));
        // Луна - мозаика COG целиком, не шаблон тайлов.
        let moon = planet_by_key("moon").imagery.unwrap();
        assert!(moon.cog && moon.url.ends_with(".tif"));
        for key in ["mars", "moon"] {
            assert!(planet_by_key(key).terrain.is_some());
        }
    }

    #[test]
    fn unknown_key_is_earth_and_others_are_far() {
        let earth = planet_by_key("vulcan");
        assert!(earth.earth && earth.key() == "earth");
        let vesta = planet_by_key("vesta");
        assert!((vesta.home.2 - HOME_RADII * vesta.body.a).abs() < 1e-6);
        assert!(planet_by_key("moon").air.is_none());
        assert!(vesta.imagery.unwrap().url.ends_with("imagery/vesta/{z}/{x}/{y}.jpg"));
    }
}
