// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Температура поверхности из NASA GIBS, перенос `core/temperature.py`
//! модуля.
//!
//! Суша - MODIS Terra, температура поверхности днём, сводка за 8 дней.
//! В дневном слое суши много дыр под облаками, в сводке их меньше.
//! Море - GHRSST MUR, температура поверхности моря за сутки, без дыр.
//! Оба слоя - картинки PNG уровней 0-7 с готовой раскраской GIBS
//! и прозрачностью там, где данных нет. Дата не задаётся, GIBS отдаёт
//! последнюю готовую.
//!
//! Опорные цвета шкал выписаны модулем из шкал GIBS
//! MODIS_Land_Surface_Temp и GHRSST_Sea_Surface_Temperature, v1.3,
//! 29 сентября 2026 года, 13 точек на шкалу. Шкала суши у GIBS
//! в кельвинах, здесь в °C.

use crate::rastermaps::Stop;

/// Адрес тайла суши, места {z}, {x}, {y}.
pub const LAND_URL: &str = "https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/MODIS_Terra_L3_Land_Surface_Temp_8Day_Day/default/default/GoogleMapsCompatible_Level7/{z}/{y}/{x}.png";
/// Адрес тайла моря.
pub const SEA_URL: &str = "https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/GHRSST_L4_MUR_Sea_Surface_Temperature/default/default/GoogleMapsCompatible_Level7/{z}/{y}/{x}.png";
/// Глубший уровень тайлов.
pub const MAX_LEVEL: u32 = 7;
/// Непрозрачность раскраски поверх снимка.
pub const OPACITY: f64 = 0.75;
/// Подпись источника и ссылка.
pub const ATTRIBUTION: (&str, &str) = ("NASA GIBS, MODIS, GHRSST MUR", "https://www.earthdata.nasa.gov/engage/open-data-services-software/earthdata-developer-portal/gibs-api");

/// Шкала суши, °C, от холодного к тёплому: кельвины GIBS минус 273.15.
pub const LAND_STOPS: [Stop; 13] = [
    (-72.8, [197, 0, 255]),
    (-60.2, [113, 0, 255]),
    (-47.6, [29, 0, 255]),
    (-35.6, [0, 71, 255]),
    (-23.0, [0, 179, 255]),
    (-10.4, [26, 255, 196]),
    (1.6, [98, 255, 26]),
    (14.2, [175, 255, 0]),
    (26.8, [255, 255, 0]),
    (39.4, [255, 193, 0]),
    (52.0, [255, 127, 0]),
    (64.0, [255, 67, 0]),
    (76.6, [255, 4, 0]),
];
/// Шкала моря, °C.
pub const SEA_STOPS: [Stop; 13] = [
    (0.1, [45, 0, 28]),
    (2.8, [101, 3, 93]),
    (5.3, [94, 8, 105]),
    (8.0, [30, 18, 78]),
    (10.7, [32, 57, 133]),
    (13.3, [37, 103, 185]),
    (16.0, [46, 163, 239]),
    (18.7, [32, 179, 13]),
    (21.2, [208, 239, 0]),
    (23.9, [255, 180, 0]),
    (26.6, [242, 95, 0]),
    (29.2, [202, 37, 0]),
    (31.9, [110, 3, 0]),
];
/// Подписи шкалы суши, °C.
pub const LAND_TICKS: [f64; 5] = [-60.0, -30.0, 0.0, 30.0, 60.0];
/// Подписи шкалы моря, °C.
pub const SEA_TICKS: [f64; 4] = [0.0, 10.0, 20.0, 30.0];

/// Раскраска GIBS в RGBA с премноженной альфой и непрозрачностью
/// [`OPACITY`]. Где данных нет, прозрачно. Тайл без данных - None.
pub fn overlay_rgba(tile: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(tile.len());
    let mut any = false;
    for px in tile.as_chunks::<4>().0 {
        let alpha = f64::from(px[3]) / 255.0 * OPACITY;
        any |= px[3] > 0;
        for &c in &px[..3] {
            out.push((f64::from(c) * alpha).round() as u8);
        }
        out.push((alpha * 255.0).round() as u8);
    }
    any.then_some(out)
}

/// Место значения на шкале от 0 до 1.
pub fn share(stops: &[Stop], value: f64) -> f64 {
    let (Some(low), Some(high)) = (stops.first(), stops.last()) else {
        return 0.0;
    };
    ((value - low.0) / (high.0 - low.0)).clamp(0.0, 1.0)
}

/// Цвет шкалы для значения, между опорными точками линейно.
pub fn color_at(stops: &[Stop], value: f64) -> [u8; 3] {
    let Some(first) = stops.first() else {
        return [0; 3];
    };
    if value <= first.0 {
        return first.1;
    }
    for pair in stops.windows(2) {
        let ((a, ca), (b, cb)) = (pair[0], pair[1]);
        if value <= b {
            let t = (value - a) / (b - a);
            return [0, 1, 2].map(|i| (f64::from(ca[i]) + (f64::from(cb[i]) - f64::from(ca[i])) * t).round() as u8);
        }
    }
    stops.last().map_or([0; 3], |s| s.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Слой - последний готовый день, места уровня 7.
    #[test]
    fn latest_date_and_fields() {
        assert!(SEA_URL.contains("/GHRSST_L4_MUR_Sea_Surface_Temperature/default/default/"));
        assert!(SEA_URL.ends_with("Level7/{z}/{y}/{x}.png"));
        assert!(LAND_URL.contains("/MODIS_Terra_L3_Land_Surface_Temp_8Day_Day/"));
    }

    /// Где данных нет, прозрачно, данные - с непрозрачностью 0.75.
    #[test]
    fn no_data_is_clear() {
        let out = overlay_rgba(&[64, 64, 64, 0, 255, 0, 0, 255]).unwrap_or_default();
        assert_eq!(out[3], 0);
        assert_eq!(f64::from(out[7]), (255.0 * OPACITY).round());
        assert_eq!(f64::from(out[4]), (255.0 * OPACITY).round());
        assert!(overlay_rgba(&[1, 2, 3, 0]).is_none());
    }

    #[test]
    fn land_scale_in_celsius() {
        // 200.3 K и 349.7 K у GIBS.
        assert!((LAND_STOPS[0].0 - -72.85).abs() < 0.06);
        assert!((LAND_STOPS[12].0 - 76.55).abs() < 0.06);
        for stops in [&LAND_STOPS, &SEA_STOPS] {
            assert!(stops.windows(2).all(|p| p[0].0 < p[1].0));
        }
    }

    #[test]
    fn ticks_inside_scales() {
        for (stops, ticks) in [(&LAND_STOPS[..], &LAND_TICKS[..]), (&SEA_STOPS[..], &SEA_TICKS[..])] {
            for &tick in ticks {
                let s = share(stops, tick);
                assert!((0.0..=1.0).contains(&s));
            }
        }
    }

    #[test]
    fn color_between_stops() {
        assert_eq!(color_at(&SEA_STOPS, 0.1), [45, 0, 28]);
        let warm = color_at(&LAND_STOPS, 40.0);
        assert!(warm[0] > warm[2]);
    }
}
