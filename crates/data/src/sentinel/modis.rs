// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Продукты MODIS в окне снимков. Перенос `core/modis.py` модуля.
//!
//! Температура поверхности, NDVI и EVI за 16 суток, снег и гари по
//! месяцам. Каталог и файлы - Microsoft Planetary Computer, коллекции
//! версии 061, ключ SAS у каждой коллекции свой, его выдаёт служба без
//! учётной записи. Файлы - COG по каналам в синусоидальной проекции,
//! тайлы 10° (h, v). Сцена окна - один день или один период одного
//! спутника, тайлы, которые задевает участок, читаются вместе
//! ([`merge_tiles`]). Выдача - в UTM середины участка ([`utm_epsg`]).
//!
//! Значения каналов (описание продуктов NASA LP DAAC и NSIDC):
//! - LST_Day_1km, LST_Night_1km - кельвины × 50, 0 - нет данных;
//! - 250m_16_days_NDVI, EVI - × 10000, -3000 - нет данных,
//!   250m_16_days_pixel_reliability - -1 нет данных, 3 облака;
//! - NDSI_Snow_Cover - 0-100 % снега, 200 и выше - флаги, 250 облака;
//! - Maximum_Snow_Extent - 25 нет снега, 100 лёд на озере, 200 снег,
//!   50 облака, 0, 1, 11, 254, 255 - нет данных;
//! - Burn_Date - день года выгорания, 0 не горело, -1 и -2 нет данных.

use serde_json::{Map, Value};

use super::{Grid, Href, Mission, Raw, Scale, Scene, integer, shares};

pub const KELVIN: f64 = 273.15;

/// Спутник окна: коллекция, шаг выдачи, м, продукты, маска качества.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spec {
    pub mission: Mission,
    pub collection: &'static str,
    pub step: f64,
    pub products: &'static [&'static str],
    pub quality: &'static str,
}

pub const MISSIONS: [Spec; 6] = [
    Spec {
        mission: Mission::ModisLst1,
        collection: "modis-11A1-061",
        step: 1000.0,
        products: &["lst_day", "lst_night"],
        quality: "LST_Day_1km",
    },
    Spec {
        mission: Mission::ModisLst8,
        collection: "modis-11A2-061",
        step: 1000.0,
        products: &["lst_day", "lst_night"],
        quality: "LST_Day_1km",
    },
    Spec {
        mission: Mission::ModisVeg,
        collection: "modis-13Q1-061",
        step: 250.0,
        products: &["modis_ndvi", "modis_evi"],
        quality: "250m_16_days_pixel_reliability",
    },
    Spec {
        mission: Mission::ModisSnow1,
        collection: "modis-10A1-061",
        step: 500.0,
        products: &["snow_cover"],
        quality: "NDSI_Snow_Cover",
    },
    Spec {
        mission: Mission::ModisSnow8,
        collection: "modis-10A2-061",
        step: 500.0,
        products: &["snow_extent"],
        quality: "Maximum_Snow_Extent",
    },
    Spec {
        mission: Mission::ModisBurn,
        collection: "modis-64A1-061",
        step: 500.0,
        products: &["burn_date"],
        quality: "Burn_Date",
    },
];

pub const TEMPERATURE: Scale = Scale {
    range: (-30.0, 45.0),
    stops: &[
        (-30.0, [49, 54, 149]),
        (-10.0, [116, 173, 209]),
        (0.0, [224, 243, 248]),
        (15.0, [254, 224, 144]),
        (30.0, [244, 109, 67]),
        (45.0, [165, 0, 38]),
    ],
};
pub const VEGETATION: Scale = Scale {
    range: (-0.2, 0.9),
    stops: &[
        (-0.2, [165, 0, 38]),
        (0.0, [244, 109, 67]),
        (0.2, [254, 224, 139]),
        (0.4, [217, 239, 139]),
        (0.6, [102, 189, 99]),
        (0.9, [0, 104, 55]),
    ],
};
pub const SNOW: Scale = Scale {
    range: (0.0, 100.0),
    stops: &[
        (0.0, [120, 110, 90]),
        (30.0, [180, 200, 215]),
        (70.0, [120, 180, 230]),
        (100.0, [255, 255, 255]),
    ],
};
/// Классы снега за 8 суток: 0 нет снега, 1 снег, 2 лёд на озере.
pub const SNOW_CLASSES: Scale = Scale {
    range: (0.0, 2.0),
    stops: &[
        (0.0, [120, 110, 90]),
        (1.0, [255, 255, 255]),
        (2.0, [120, 200, 255]),
    ],
};
pub const BURN: Scale = Scale {
    range: (1.0, 366.0),
    stops: &[
        (1.0, [255, 255, 178]),
        (120.0, [254, 204, 92]),
        (200.0, [253, 141, 60]),
        (270.0, [240, 59, 32]),
        (366.0, [189, 0, 38]),
    ],
};

/// Продукт: канал, шкала, цвета классами (true) или переходом.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModisProduct {
    pub name: &'static str,
    pub key: &'static str,
    pub scale: Scale,
    pub classes: bool,
}

pub const PRODUCTS: [ModisProduct; 7] = [
    ModisProduct {
        name: "lst_day",
        key: "LST_Day_1km",
        scale: TEMPERATURE,
        classes: false,
    },
    ModisProduct {
        name: "lst_night",
        key: "LST_Night_1km",
        scale: TEMPERATURE,
        classes: false,
    },
    ModisProduct {
        name: "modis_ndvi",
        key: "250m_16_days_NDVI",
        scale: VEGETATION,
        classes: false,
    },
    ModisProduct {
        name: "modis_evi",
        key: "250m_16_days_EVI",
        scale: VEGETATION,
        classes: false,
    },
    ModisProduct {
        name: "snow_cover",
        key: "NDSI_Snow_Cover",
        scale: SNOW,
        classes: false,
    },
    ModisProduct {
        name: "snow_extent",
        key: "Maximum_Snow_Extent",
        scale: SNOW_CLASSES,
        classes: true,
    },
    ModisProduct {
        name: "burn_date",
        key: "Burn_Date",
        scale: BURN,
        classes: false,
    },
];

/// Архивы NASA, где хранится продукт, кроме LP DAAC.
pub const ARCHIVES: [(&str, &str); 2] = [("10A1", "NSIDC DAAC"), ("10A2", "NSIDC DAAC")];

fn spec(mission: Mission) -> Option<&'static Spec> {
    MISSIONS.iter().find(|s| s.mission == mission)
}

pub fn is_modis(mission: Mission) -> bool {
    spec(mission).is_some()
}

pub fn collection(mission: Mission) -> Option<&'static str> {
    spec(mission).map(|s| s.collection)
}

/// Продукты спутника, у чужого - пусто.
pub fn products_of(mission: Mission) -> &'static [&'static str] {
    spec(mission).map_or(&[], |s| s.products)
}

pub fn step(mission: Mission) -> Option<f64> {
    spec(mission).map(|s| s.step)
}

pub fn quality_asset(mission: Mission) -> Option<&'static str> {
    spec(mission).map(|s| s.quality)
}

/// Продукт по имени.
pub fn product(name: &str) -> Option<&'static ModisProduct> {
    PRODUCTS.iter().find(|p| p.name == name)
}

/// Спутник окна по коллекции элемента каталога или None.
pub fn mission_of(collection_id: &str) -> Option<Mission> {
    MISSIONS
        .iter()
        .find(|s| s.collection == collection_id)
        .map(|s| s.mission)
}

/// Спутник по номеру элемента: MOD - Terra, MYD - Aqua, MCD - оба.
pub fn platform(item_id: &str) -> &'static str {
    match item_id.get(..3) {
        Some("MOD") => "Terra",
        Some("MYD") => "Aqua",
        Some("MCD") => "Terra+Aqua",
        _ => "",
    }
}

/// Тайл «h21v03» по полям каталога или пусто.
pub fn tile_of(props: &Map<String, Value>) -> String {
    let (h, v) = (
        props.get("modis:horizontal-tile"),
        props.get("modis:vertical-tile"),
    );
    match (integer(h), integer(v)) {
        (Some(h), Some(v)) => format!("h{h:02}v{v:02}"),
        _ => String::new(),
    }
}

/// EPSG системы UTM середины участка (широта, долгота). None у пустого
/// кольца.
pub fn utm_epsg(ring: &[(f64, f64)]) -> Option<u32> {
    if ring.is_empty() {
        return None;
    }
    let n = ring.len() as f64;
    let lat = ring.iter().map(|p| p.0).sum::<f64>() / n;
    let lon = ring.iter().map(|p| p.1).sum::<f64>() / n;
    let cell = ((lon + 180.0) / 6.0).floor();
    if !cell.is_finite() {
        return None;
    }
    let zone = (cell as i64).rem_euclid(60) + 1;
    let base = if lat >= 0.0 { 32600 } else { 32700 };
    u32::try_from(base + zone).ok()
}

/// Каналы продуктов MODIS, неизвестные пропускаются.
pub fn needed(products: &[&str]) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for name in products {
        if let Some(p) = product(name)
            && !keys.iter().any(|k| k == p.key)
        {
            keys.push(p.key.to_string());
        }
    }
    keys
}

/// Значения продукта из сырого канала, нет данных - NaN, float32.
/// None у неизвестного продукта.
pub fn value(name: &str, raw: &Raw) -> Option<Grid<f32>> {
    let out = match name {
        "lst_day" | "lst_night" => raw.map(|v| {
            if v == 0.0 {
                f32::NAN
            } else {
                (v as f32) * 0.02f32 - KELVIN as f32
            }
        }),
        "modis_ndvi" | "modis_evi" => raw.map(|v| {
            if v <= -2000.0 {
                f32::NAN
            } else {
                (v as f32) * 1e-4f32
            }
        }),
        "snow_cover" => raw.map(|v| if v > 100.0 { f32::NAN } else { v as f32 }),
        "snow_extent" => raw.map(|v| {
            if v == 25.0 {
                0.0
            } else if v == 200.0 {
                1.0
            } else if v == 100.0 {
                2.0
            } else {
                f32::NAN
            }
        }),
        "burn_date" => raw.map(|v| {
            if v <= 0.0 || v > 366.0 {
                f32::NAN
            } else {
                v as f32
            }
        }),
        _ => return None,
    };
    Some(out)
}

/// Облачность над участком по каналу качества: (доля облаков среди
/// пикселей с данными, доля пикселей с данными), в процентах. У
/// температуры пропуск значения - облако, у гарей облаков нет. None при
/// разных размерах.
pub fn quality(mission: Mission, raw: &Raw, inside: &Grid<bool>) -> Option<(f64, f64)> {
    let (empty, cloudy) = match mission {
        Mission::ModisLst1 | Mission::ModisLst8 => (raw.map(|_| false), raw.map(|v| v == 0.0)),
        Mission::ModisVeg => (raw.map(|v| v < 0.0), raw.map(|v| v == 3.0)),
        Mission::ModisSnow1 => (
            raw.map(|v| [200.0, 201.0, 211.0, 254.0, 255.0].contains(&v)),
            raw.map(|v| v == 250.0),
        ),
        Mission::ModisSnow8 => (
            raw.map(|v| [0.0, 1.0, 11.0, 254.0, 255.0].contains(&v)),
            raw.map(|v| v == 50.0),
        ),
        _ => (raw.map(|v| v < 0.0), raw.map(|_| false)),
    };
    shares(inside, &empty, &cloudy)
}

/// Пиксели без облаков и пропусков по каналу качества.
pub fn clear(mission: Mission, raw: &Raw) -> Grid<bool> {
    match mission {
        Mission::ModisVeg => raw.map(|v| v >= 0.0),
        Mission::ModisSnow1 => raw.map(|v| v <= 100.0),
        Mission::ModisSnow8 => raw.map(|v| [25.0, 100.0, 200.0].contains(&v)),
        _ => raw.map(|_| true),
    }
}

/// Подпись слоя: «MOD11A1 v061, NASA EOSDIS LP DAAC».
pub fn credit(item_id: &str) -> String {
    let short = item_id.split('.').next().unwrap_or("");
    let code: String = short.chars().skip(3).collect();
    let archive = ARCHIVES
        .iter()
        .find(|(k, _)| *k == code)
        .map_or("LP DAAC", |(_, a)| *a);
    format!("{short} v061, NASA EOSDIS {archive}")
}

/// Линейная интерполяция по точкам, за краями - крайние значения, как
/// interp numpy.
fn interp(x: f64, xs: &[f64], fs: &[f64]) -> f64 {
    let (Some(&x0), Some(&f0)) = (xs.first(), fs.first()) else {
        return 0.0;
    };
    if x <= x0 {
        return f0;
    }
    for i in 1..xs.len().min(fs.len()) {
        if x <= xs[i] {
            let (a, b) = (xs[i - 1], xs[i]);
            if b == a {
                return fs[i];
            }
            return fs[i - 1] + (x - a) * (fs[i] - fs[i - 1]) / (b - a);
        }
    }
    fs.last().copied().unwrap_or(f0)
}

/// Картинка - три канала uint8 по шкале продукта, нет данных - серый.
pub fn colorize(values: &Grid<f32>, scale: &Scale) -> [Grid<u8>; 3] {
    let (lo, hi) = scale.range;
    let xs: Vec<f64> = scale.stops.iter().map(|s| f64::from(s.0 as f32)).collect();
    let channel = |c: usize| {
        let fs: Vec<f64> = scale.stops.iter().map(|s| f64::from(s.1[c])).collect();
        values.map(|v| {
            if !v.is_finite() {
                return 90u8;
            }
            let v = f64::from(v).clamp(lo, hi);
            interp(v, &xs, &fs).clamp(0.0, 255.0) as u8
        })
    };
    [channel(0), channel(1), channel(2)]
}

/// Сцены одного дня или периода одного спутника - одна сцена, адреса
/// каналов всех тайлов вместе. Порядок - от новых к старым.
pub fn merge_tiles(scenes: &[Scene]) -> Vec<Scene> {
    let mut groups: Vec<((String, String), Scene)> = Vec::new();
    for s in scenes {
        let key = (
            s.when.clone(),
            s.tile.split(' ').next().unwrap_or("").to_string(),
        );
        let Some((_, first)) = groups.iter_mut().find(|(k, _)| *k == key) else {
            groups.push((key, s.clone()));
            continue;
        };
        for (name, asset) in s.assets.iter() {
            let Some(target) = first.assets.get(name).cloned() else {
                continue;
            };
            let mut list = hrefs(&target.href);
            list.extend(hrefs(&asset.href));
            let mut joined = target;
            joined.href = Href::Tiles(list);
            first.assets.insert(name, joined);
        }
        let last = s.tile.rsplit(' ').next().unwrap_or("");
        first.tile = format!("{},{last}", first.tile);
    }
    let mut merged: Vec<Scene> = groups.into_iter().map(|(_, s)| s).collect();
    merged.sort_by(|a, b| b.when.cmp(&a.when));
    merged
}

/// Все адреса канала.
fn hrefs(href: &Href) -> Vec<String> {
    match href {
        Href::One(h) => vec![h.clone()],
        Href::Tiles(list) => list.clone(),
    }
}
