// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Превью строк каталога «Добавить данные», перенос `ramp_image`,
//! `tile_image`, `_over`, `MAP_COLORS` и `VECTOR_COLORS` из
//! `ui/nasamaps.py` модуля. Расчёт без интерфейса и сети.
//!
//! Превью - картинка [`SIDE`] × [`SIDE`] RGBA, непрозрачная. Под ней
//! тайл подложки [`TILE`] (Евразия, северо-восток), над ним тайл темы
//! или полоса цветов шкалы внизу. Пришедший тайл считается враждебным:
//! размер, который не сходится с байтами, даёт None, а не падение.

/// Сторона превью в пикселях, `THUMB` модуля.
pub const SIDE: usize = 104;
/// Тайл превью: z, x, y, `THUMB_TILE` модуля.
pub const TILE: (u32, u32, u32) = (1, 1, 0);
/// Фон превью без подложки.
pub const BACKGROUND: [u8; 3] = [40, 48, 60];

/// Картинка RGBA width × height, приведённая к [`SIDE`] × [`SIDE`]
/// усреднением пикселей по клеткам. Альфа сохраняется. None - размер
/// не сходится с байтами или картинка пуста.
pub fn fit(rgba: &[u8], width: usize, height: usize) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || width.checked_mul(height)?.checked_mul(4)? != rgba.len() {
        return None;
    }
    let mut out = Vec::with_capacity(SIDE * SIDE * 4);
    for dy in 0..SIDE {
        let y0 = dy * height / SIDE;
        let y1 = ((dy + 1) * height / SIDE).max(y0 + 1);
        for dx in 0..SIDE {
            let x0 = dx * width / SIDE;
            let x1 = ((dx + 1) * width / SIDE).max(x0 + 1);
            let mut sum = [0u32; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * width + x) * 4;
                    for (c, s) in sum.iter_mut().enumerate() {
                        *s += u32::from(rgba[i + c]);
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            out.extend(sum.map(|s| ((s + n / 2) / n) as u8));
        }
    }
    Some(out)
}

/// Превью из двух картинок [`SIDE`] × [`SIDE`] одна над другой, любая
/// может быть None, `_over` и `tile_image` модуля. under - подложка, цвет
/// без умножения на альфу, top - тема, цвет умножен на альфу. Под ними -
/// [`BACKGROUND`]. Картинка не того размера пропускается.
pub fn over(under: Option<&[u8]>, top: Option<&[u8]>) -> Vec<u8> {
    let size = SIDE * SIDE * 4;
    let under = under.filter(|u| u.len() == size);
    let top = top.filter(|t| t.len() == size);
    let mut out = Vec::with_capacity(size);
    for i in 0..SIDE * SIDE {
        let mut px = BACKGROUND.map(f64::from);
        if let Some(u) = under {
            let a = f64::from(u[i * 4 + 3]) / 255.0;
            for (c, v) in px.iter_mut().enumerate() {
                *v = f64::from(u[i * 4 + c]) * a + *v * (1.0 - a);
            }
        }
        if let Some(t) = top {
            let a = f64::from(t[i * 4 + 3]) / 255.0;
            for (c, v) in px.iter_mut().enumerate() {
                *v = f64::from(t[i * 4 + c]) + *v * (1.0 - a);
            }
        }
        out.extend(px.map(|v| v.round().clamp(0.0, 255.0) as u8));
        out.push(255);
    }
    out
}

/// Превью из полосы цветов шкалы поверх подложки base, `ramp_image`
/// модуля: полоса в четверть высоты внизу, цвета слева направо равными
/// долями.
pub fn ramp(colors: &[[u8; 3]], base: Option<&[u8]>) -> Vec<u8> {
    let mut out = over(base, None);
    let band = SIDE / 4;
    let count = colors.len().max(1);
    for (n, color) in colors.iter().enumerate() {
        let left = SIDE * n / count;
        let right = SIDE * (n + 1) / count;
        for y in SIDE - band..SIDE {
            for x in left..right {
                let i = (y * SIDE + x) * 4;
                out[i..i + 3].copy_from_slice(color);
            }
        }
    }
    out
}

/// Цвета шкалы слоя без своего тайла, `MAP_COLORS` модуля. Цвета
/// звёзд, солнца, разреза Земли и спутников - выбор помощника: у модуля
/// для них свои картинки.
pub fn colors(key: &str) -> Option<&'static [[u8; 3]]> {
    Some(match key {
        "grid" => &[[250, 220, 60], [230, 230, 230]],
        "buildings" | "overture" => &[[200, 190, 175], [150, 140, 130]],
        "contours" => &[[150, 100, 60], [250, 220, 60]],
        "cadastre" => &[[230, 90, 80], [250, 225, 220]],
        "infra_osm" => &[[220, 60, 60], [240, 150, 40], [90, 90, 200]],
        "cables" => &[[60, 200, 200], [170, 110, 220]],
        "magpoles" => &[[235, 70, 60], [70, 130, 235]],
        "magdecl" => &[[215, 70, 60], [40, 170, 70], [60, 110, 215]],
        "seafloor_age" => &[[230, 40, 40], [245, 230, 70], [60, 90, 200]],
        "platemotion" => &[[255, 225, 60], [60, 60, 60]],
        "volcanoes" => &[[230, 40, 30], [245, 140, 40], [240, 210, 70]],
        "faults" => &[[225, 45, 40], [40, 120, 230], [30, 170, 70]],
        "geology" => &[[251, 167, 148], [128, 197, 101], [191, 128, 196]],
        "gravity" => &[[30, 40, 130], [245, 245, 240], [150, 30, 30]],
        "bouguer" => &[[60, 120, 210], [245, 245, 240], [230, 120, 60]],
        "heatflow" => &[[40, 50, 140], [245, 235, 160], [220, 70, 40]],
        "emag" => &[[40, 60, 150], [245, 245, 235], [160, 30, 35]],
        "geoid" => &[[60, 90, 170], [120, 170, 120], [200, 150, 80]],
        "air" => &[[10, 15, 40], [90, 150, 230], [200, 225, 250]],
        "glim" => &[[238, 228, 150], [110, 180, 225], [232, 80, 72], [140, 170, 100]],
        "water_occurrence" => &[[255, 120, 120], [130, 60, 200], [0, 0, 255]],
        "water_transitions" => &[[0, 0, 221], [34, 177, 76], [209, 45, 62], [153, 217, 234]],
        "soil_clay" | "soil_sand" => &[[242, 218, 193], [180, 123, 65], [118, 70, 21]],
        "soil_soc" => &[[251, 249, 249], [187, 163, 156], [118, 70, 57]],
        "soil_ph" => &[[241, 238, 45], [66, 181, 50], [15, 204, 217]],
        "worldcover" => &[[0, 100, 0], [240, 150, 255], [250, 0, 0], [0, 100, 200]],
        "sea_marks" => &[[230, 40, 40], [40, 160, 60], [250, 220, 40]],
        "hiking" => &[[200, 30, 30], [240, 240, 240]],
        "cycling" => &[[120, 60, 200], [240, 240, 240]],
        "wiki" => &[[45, 110, 205], [245, 245, 245], [205, 228, 255]],
        "aef_change" => &[[0, 0, 0], [70, 60, 10], [255, 245, 30]],
        "ftw_2025" | "ftw_2024" => &[[254, 195, 121], [243, 250, 187], [51, 160, 44]],
        "mining" => &[[235, 125, 35], [120, 60, 20]],
        "stars" => &[[10, 12, 30], [255, 255, 255], [180, 200, 255]],
        "sun" => &[[255, 220, 120], [20, 25, 50]],
        "satellites" => &[[255, 255, 255], [90, 200, 255], [255, 120, 80]],
        "cutaway" => &[[200, 120, 60], [230, 80, 40], [250, 200, 60]],
        "paleo" => &[[60, 110, 170], [120, 160, 90], [190, 160, 110]],
        "plates" => &[[230, 60, 50], [250, 200, 60], [60, 60, 60]],
        _ => return None,
    })
}

/// Цвета превью групп векторной основы, `VECTOR_COLORS` модуля: группа n
/// по кругу из трёх.
pub fn vector_colors(n: usize) -> &'static [[u8; 3]] {
    const GROUPS: [&[[u8; 3]]; 3] = [&[[250, 220, 60], [240, 240, 240]], &[[255, 200, 70], [250, 240, 180], [90, 90, 90]], &[[60, 120, 220], [90, 170, 90], [150, 110, 70]]];
    GROUPS[n % GROUPS.len()]
}
