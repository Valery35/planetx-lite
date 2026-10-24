// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Горизонтали рельефа по тайлу высот и изолинии сетки. Перенос
//! `core/contours.py` модуля.
//!
//! Горизонтали рисуются в картинке тайла по высотам тайла Terrarium.
//! Сечение - шаг из ряда 1, 2, 2.5, 5 × 10ⁿ метров, чтобы горизонтали
//! стояли не реже STEP_PIXELS пикселей тайла на склоне в 45°: у уровня 15
//! средних широт - 5 м, у уровня 13 - 20 м. Каждая INDEX-я горизонталь
//! утолщённая.
//!
//! Линия сглажена: расстояние от пикселя до горизонтали в пикселях -
//! доля сечения, делённая на градиент высоты. Так толщина одинакова на
//! пологом и крутом склоне. Где горизонтали чаще FADE пикселей, они
//! бледнеют. Ниже нуля - изобаты своего цвета.
//!
//! Глубже SOURCE_LEVEL высоты Terrarium - та же съёмка, пересчитанная
//! мельче, горизонтали по ним шли ступенями и петлями. Горизонтали
//! глубже считаются по сглаженному тайлу этого уровня, пересчитанному
//! кубически (upsample). Изолинии сетки - отрезки по ячейкам методом
//! квадратов (segments), сборка в ломаные - chain.
//!
//! Цвета, толщины и пороги - выбор помощника модуля, утверждает автор.

use std::collections::HashMap;

/// Сторона тайла.
pub const SIZE: usize = 256;
const NICE: [f64; 4] = [1.0, 2.0, 2.5, 5.0];
/// Пикселей тайла на шаг сечения при уклоне 45°.
pub const STEP_PIXELS: f64 = 2.0;
/// Мельче сечение данные Terrarium не держат.
pub const MIN_STEP: f64 = 5.0;
/// Каждая пятая горизонталь утолщённая.
pub const INDEX: f64 = 5.0;
/// Полутолщина линии, пиксели.
pub const WIDTH: f64 = 0.45;
pub const INDEX_WIDTH: f64 = 0.9;
/// Пикселей между горизонталями, чаще - линии бледнее.
pub const FADE: f64 = 4.0;
/// Глубже этого уровня тайлы горизонталей не считаются, их картинка -
/// доля картинки предка.
pub const LEVEL: u32 = 17;
/// Уровень сглаженных высот съёмки.
pub const SOURCE_LEVEL: u32 = 12;
/// Рамка тайла высот из строк соседей перед пересчётом, пиксели.
pub const FRAME: usize = 3;
/// Глубины морей у Terrarium надёжны до этого уровня. Тайл, у которого
/// тайл этого уровня целиком глубже SEA_DEPTH, берёт глубины оттуда.
pub const SEA_LEVEL: u32 = 10;
pub const SEA_DEPTH: f64 = -50.0;
/// Коричневый топографических карт и изобаты.
pub const LAND: [u8; 3] = [150, 85, 30];
pub const WATER: [u8; 3] = [40, 95, 175];
pub const OPACITY: f64 = 0.85;
/// Подписи утолщённых горизонталей: соседние линии дальше LABEL_ROOM
/// высот цифр, подписи не ближе LABEL_GAP пикселей, не больше LABEL_MAX
/// на тайл, проверяется LABEL_TRIES точек, разрыв линии вокруг цифр -
/// LABEL_PAD пикселей.
pub const LABEL_ROOM: f64 = 0.7;
pub const LABEL_GAP: f64 = 110.0;
pub const LABEL_MAX: usize = 4;
pub const LABEL_TRIES: usize = 4000;
pub const LABEL_PAD: f64 = 2.0;
/// Склон положе FLAT метров на пиксель - ровное место, линий там нет.
pub const FLAT: f64 = 2e-3;
/// Слова в адресе или названии подложки, по которым она - снимок.
pub const IMAGERY_WORDS: [&str; 15] =
    ["imagery", "satellite", "sentinel", "landsat", "aerial", "ortho", "орто", "снимк", "спутник", "lyrs=s", "lyrs=y", "=sat", "/sat", "_sat", "bing"];

/// Цвета линий: на карте - коричневый, на космоснимке - жёлтый
/// и голубой.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Palette {
    Map,
    Imagery,
}

impl Palette {
    /// Суша, изобаты, яркость обычных и утолщённых.
    fn colors(self) -> ([u8; 3], [u8; 3], f64, f64) {
        match self {
            Palette::Map => (LAND, WATER, 1.0, 0.75),
            Palette::Imagery => ([255, 214, 0], [120, 205, 255], 0.85, 1.0),
        }
    }
}

/// Сетка значений по строкам.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<f64>,
}

impl Grid {
    pub fn new(rows: usize, cols: usize, value: f64) -> Grid {
        Grid { rows, cols, data: vec![value; rows * cols] }
    }

    pub fn from_fn(rows: usize, cols: usize, f: impl Fn(usize, usize) -> f64) -> Grid {
        let mut data = Vec::with_capacity(rows * cols);
        for r in 0..rows {
            for c in 0..cols {
                data.push(f(r, c));
            }
        }
        Grid { rows, cols, data }
    }

    pub fn at(&self, r: usize, c: usize) -> f64 {
        self.data[r * self.cols + c]
    }

    fn set(&mut self, r: usize, c: usize, v: f64) {
        self.data[r * self.cols + c] = v;
    }
}

/// Ближайшее в логарифмах число ряда 1, 2, 2.5, 5 × 10ⁿ.
pub fn nice(value: f64) -> f64 {
    if value <= 0.0 || !value.is_finite() {
        return NICE[0];
    }
    let power = 10f64.powf(value.log10().floor());
    let mut best = NICE[0] * power;
    for c in NICE.iter().map(|n| n * power).chain(std::iter::once(10.0 * power)) {
        if (c / value).ln().abs() < (best / value).ln().abs() {
            best = c;
        }
    }
    best
}

/// Сечение рельефа тайла (z, y), метры: по шагу пикселя на широте
/// середины тайла.
pub fn tile_step(z: u32, y: u32, radius: f64) -> f64 {
    let n = f64::from(1u32 << z.min(30));
    let t = (f64::from(y) + 0.5) / n;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * t)).sinh().atan();
    let pixel = 2.0 * std::f64::consts::PI * radius * lat.cos() / (SIZE as f64 * n);
    nice(pixel * STEP_PIXELS).max(MIN_STEP)
}

/// Подложка - снимок, а не карта: снимки тел или слова IMAGERY_WORDS
/// в названии или адресе.
pub fn is_imagery(name: &str, url: &str, builtin_body: bool) -> bool {
    if builtin_body {
        return true;
    }
    let text = format!("{name} {url}").to_lowercase();
    IMAGERY_WORDS.iter().any(|w| text.contains(w))
}

/// Сечение горизонталей в середине вида: камера на расстоянии distance
/// от точки взгляда на широте lat, поле зрения fov_y (рад), окно
/// height_px пикселей. Уровень тайла - тот, у которого тексель не больше
/// 1.5 пикселя экрана.
pub fn view_step(distance: f64, lat: f64, fov_y: f64, height_px: f64, radius: f64, max_level: u32) -> f64 {
    let screen = 2.0 * distance * (fov_y / 2.0).tan() / height_px.max(1.0);
    let ground = 2.0 * std::f64::consts::PI * radius * lat.to_radians().cos().max(1e-6) / SIZE as f64;
    let level = (ground / (1.5 * screen).max(1e-9)).log2().ceil().clamp(0.0, f64::from(max_level)) as u32;
    let pixel = ground / f64::from(1u32 << level.min(30));
    nice(pixel * STEP_PIXELS).max(MIN_STEP)
}

/// Высоты, сглаженные окном [1, 2, 1] / 4 по строкам и столбцам: без
/// него ступеньки целых метров Terrarium давали бусы на линиях.
pub fn smooth(h: &Grid) -> Grid {
    let (rows, cols) = (h.rows, h.cols);
    let at = |r: isize, c: isize| h.at(r.clamp(0, rows as isize - 1) as usize, c.clamp(0, cols as isize - 1) as usize);
    let mut mid = Grid::new(rows, cols, 0.0);
    for r in 0..rows as isize {
        for c in 0..cols as isize {
            mid.set(r as usize, c as usize, (at(r - 1, c) + 2.0 * at(r, c) + at(r + 1, c)) / 4.0);
        }
    }
    let m = |r: usize, c: isize| mid.at(r, c.clamp(0, cols as isize - 1) as usize);
    Grid::from_fn(rows, cols, |r, c| {
        let c = c as isize;
        (m(r, c - 1) + 2.0 * m(r, c) + m(r, c + 1)) / 4.0
    })
}

/// Производные по строкам и столбцам, как numpy.gradient: в середине -
/// центральная разность, у края - односторонняя.
pub fn gradient(h: &Grid) -> (Grid, Grid) {
    let (rows, cols) = (h.rows, h.cols);
    let d = |a: f64, b: f64, two: bool| if two { (b - a) / 2.0 } else { b - a };
    let d_row = Grid::from_fn(rows, cols, |r, c| {
        if rows < 2 {
            0.0
        } else if r == 0 {
            d(h.at(0, c), h.at(1, c), false)
        } else if r == rows - 1 {
            d(h.at(r - 1, c), h.at(r, c), false)
        } else {
            d(h.at(r - 1, c), h.at(r + 1, c), true)
        }
    });
    let d_col = Grid::from_fn(rows, cols, |r, c| {
        if cols < 2 {
            0.0
        } else if c == 0 {
            d(h.at(r, 0), h.at(r, 1), false)
        } else if c == cols - 1 {
            d(h.at(r, c - 1), h.at(r, c), false)
        } else {
            d(h.at(r, c - 1), h.at(r, c + 1), true)
        }
    });
    (d_row, d_col)
}

/// Поля линий по сглаженным высотам h.
pub struct Fields {
    pub d_row: Grid,
    pub d_col: Grid,
    /// Расстояние до горизонтали в пикселях.
    pub gap: Grid,
    /// Расстояние между горизонталями в пикселях.
    pub spacing: Grid,
    /// Ближняя горизонталь утолщённая.
    pub index: Vec<bool>,
}

/// Поля линий по сглаженным высотам h при сечении step.
#[allow(clippy::needless_range_loop)]
pub fn line_fields(h: &Grid, step: f64) -> Fields {
    let (d_row, d_col) = gradient(h);
    let mut gap = Grid::new(h.rows, h.cols, 0.0);
    let mut spacing = Grid::new(h.rows, h.cols, 0.0);
    let mut index = vec![false; h.data.len()];
    for i in 0..h.data.len() {
        let grad = d_row.data[i].hypot(d_col.data[i]).max(1e-6);
        let level = h.data[i] / step;
        let near = level.round_ties_even();
        // Ровная гладь озера на самой отметке - не линия.
        gap.data[i] = if grad < FLAT { f64::INFINITY } else { (level - near).abs() * step / grad };
        spacing.data[i] = step / grad;
        index[i] = near.rem_euclid(INDEX) == 0.0;
    }
    Fields { d_row, d_col, gap, spacing, index }
}

/// Картинки цифр, минуса и точки: альфа от 0 до 1 по строкам.
#[derive(Debug, Clone, Default)]
pub struct Glyphs {
    pub height: usize,
    pub chars: HashMap<char, (usize, Vec<f32>)>,
}

/// Подпись отметки: целые метры без точки, дробные - с одним знаком.
pub fn label_text(value: f64) -> String {
    if (value - value.round()).abs() < 1e-6 { format!("{}", value.round() as i64) } else { format!("{value:.1}") }
}

/// Картинка подписи - альфа цифр подряд через столбец: (длина, значения
/// по строкам высоты glyphs.height).
pub fn label_strip(text: &str, glyphs: &Glyphs) -> Option<(usize, Vec<f32>)> {
    let parts: Vec<&(usize, Vec<f32>)> = text.chars().filter_map(|c| glyphs.chars.get(&c)).collect();
    if parts.is_empty() {
        return None;
    }
    let length = parts.iter().map(|p| p.0).sum::<usize>() + parts.len() - 1;
    let mut out = vec![0f32; length * glyphs.height];
    let mut x = 0;
    for (w, alpha) in parts {
        for r in 0..glyphs.height {
            for c in 0..*w {
                out[r * length + x + c] = alpha.get(r * w + c).copied().unwrap_or(0.0);
            }
        }
        x += w + 1;
    }
    Some((length, out))
}

/// Подпись на тайле: текст, место (первая строка, первый столбец,
/// строк, столбцов), цифры и разрыв по месту, отметка.
pub struct Label {
    pub text: String,
    pub r0: usize,
    pub c0: usize,
    pub rows: usize,
    pub cols: usize,
    pub ink: Vec<f32>,
    pub clear: Vec<f32>,
    pub value: f64,
}

/// Где может стоять подпись: на утолщённой горизонтали, соседние линии
/// дальше LABEL_ROOM высот цифр.
pub fn label_spots(f: &Fields, height: usize) -> Vec<bool> {
    (0..f.gap.data.len()).map(|i| f.index[i] && f.gap.data[i] < 0.5 && f.spacing.data[i] >= LABEL_ROOM * height as f64).collect()
}

/// Значение strip в дробных точках (y, x), вне картинки - 0.
fn bilinear(strip: &[f32], wid: usize, hgt: usize, y: f64, x: f64) -> f64 {
    let (y, x) = (y + 1.0, x + 1.0);
    let inside = y >= 0.0 && y <= hgt as f64 + 1.0 && x >= 0.0 && x <= wid as f64 + 1.0;
    if !inside {
        return 0.0;
    }
    let y = y.clamp(0.0, hgt as f64 + 0.999);
    let x = x.clamp(0.0, wid as f64 + 0.999);
    let (y0, x0) = (y.floor() as usize, x.floor() as usize);
    let (fy, fx) = (y - y0 as f64, x - x0 as f64);
    // Картинка с полосой нулей в пиксель вокруг.
    let pad = |r: usize, c: usize| -> f64 {
        if r == 0 || c == 0 || r > hgt || c > wid { 0.0 } else { f64::from(strip[(r - 1) * wid + c - 1]) }
    };
    pad(y0, x0) * (1.0 - fy) * (1.0 - fx) + pad(y0, x0 + 1) * (1.0 - fy) * fx + pad(y0 + 1, x0) * fy * (1.0 - fx) + pad(y0 + 1, x0 + 1) * fy * fx
}

/// Подписи утолщённых горизонталей тайла. Подпись стоит на линии, где
/// соседние горизонтали дальше высоты цифр, вдоль линии, верхом
/// к подъёму. Подпись целиком внутри тайла, концы её лежат на той же
/// горизонтали. Подписи не ближе LABEL_GAP пикселей друг к другу,
/// первыми - ближние к середине тайла.
pub fn place_labels(h: &Grid, f: &Fields, step: f64, candidates: &[bool], glyphs: &Glyphs) -> Vec<Label> {
    let size = h.rows;
    let mid = (size as f64 - 1.0) / 2.0;
    let mut spots: Vec<(usize, usize)> = (0..candidates.len()).filter(|i| candidates[*i]).map(|i| (i / h.cols, i % h.cols)).collect();
    spots.sort_by(|a, b| {
        let da = (a.0 as f64 - mid).powi(2) + (a.1 as f64 - mid).powi(2);
        let db = (b.0 as f64 - mid).powi(2) + (b.1 as f64 - mid).powi(2);
        da.total_cmp(&db)
    });
    let mut placed: Vec<(usize, usize)> = Vec::new();
    let mut out = Vec::new();
    for (r, c) in spots.into_iter().take(LABEL_TRIES) {
        if placed.iter().any(|(pr, pc)| (r as f64 - *pr as f64).powi(2) + (c as f64 - *pc as f64).powi(2) < LABEL_GAP * LABEL_GAP) {
            continue;
        }
        let (gx, gy) = (f.d_col.at(r, c), f.d_row.at(r, c));
        let norm = gx.hypot(gy);
        if norm <= 0.0 {
            continue;
        }
        let value = (h.at(r, c) / step).round_ties_even() * step;
        let text = label_text(value);
        let Some(strip) = label_strip(&text, glyphs) else {
            continue;
        };
        let Some(label) = label_box(h, r, c, gx / norm, gy / norm, norm, value, &strip, glyphs.height, text) else {
            continue;
        };
        placed.push((r, c));
        out.push(label);
        if out.len() >= LABEL_MAX {
            break;
        }
    }
    out
}

/// Пиксели подписи в точке (r, c): направление подъёма (ux_up, uy_up)
/// в столбцах и строках. None, если подпись не помещается в тайл или
/// концы её уходят с горизонтали.
#[allow(clippy::too_many_arguments)]
fn label_box(h: &Grid, r: usize, c: usize, ux_up: f64, uy_up: f64, grad: f64, value: f64, strip: &(usize, Vec<f32>), height: usize, text: String) -> Option<Label> {
    let size = h.rows as f64;
    let length = strip.0 as f64;
    let hh = height as f64;
    // Ось текста вдоль линии, «вниз» текста - под уклон.
    let (ux, uy) = (-uy_up, ux_up);
    let (vx, vy) = (-ux_up, -uy_up);
    let half = length / 2.0 + LABEL_PAD;
    let reach = half.hypot(hh / 2.0 + LABEL_PAD);
    let (rf, cf) = (r as f64, c as f64);
    if !(reach + 1.0 <= rf && rf <= size - 2.0 - reach && reach + 1.0 <= cf && cf <= size - 2.0 - reach) {
        return None;
    }
    // Концы подписи на той же горизонтали не дальше трети высоты цифр.
    for sign in [-1.0, 1.0] {
        let er = (rf + sign * uy * length / 2.0).round_ties_even() as usize;
        let ec = (cf + sign * ux * length / 2.0).round_ties_even() as usize;
        if (h.at(er.min(h.rows - 1), ec.min(h.cols - 1)) - value).abs() / grad.max(1e-6) > hh / 3.0 {
            return None;
        }
    }
    let (r0, r1) = ((rf - reach) as usize, (rf + reach) as usize + 1);
    let (c0, c1) = ((cf - reach) as usize, (cf + reach) as usize + 1);
    let (rows, cols) = (r1 - r0, c1 - c0);
    let mut ink = Vec::with_capacity(rows * cols);
    let mut clear = Vec::with_capacity(rows * cols);
    for gr in r0..r1 {
        for gc in c0..c1 {
            let (dx, dy) = (gc as f64 - cf, gr as f64 - rf);
            let along = dx * ux + dy * uy;
            let across = dx * vx + dy * vy;
            let lx = along + length / 2.0 - 0.5;
            let ly = across + hh / 2.0 - 0.5;
            clear.push(((half - along.abs()).min(hh / 2.0 + 1.0 - across.abs()) + 0.5).clamp(0.0, 1.0) as f32);
            ink.push(bilinear(&strip.1, strip.0, height, ly, lx) as f32);
        }
    }
    Some(Label { text, r0, c0, rows, cols, ink, clear, value })
}

/// Картинка горизонталей тайла (z, y): RGBA с премноженной альфой.
/// heights - высоты тайла по строкам. glyphs - цифры подписей утолщённых
/// горизонталей, без них подписей нет. sea=false - у тела нет моря,
/// линии ниже нуля цвета суши.
// Параллельные массивы по номеру пикселя, как массивы NumPy модуля.
#[allow(clippy::too_many_arguments, clippy::needless_range_loop)]
pub fn contour_rgba(heights: &Grid, z: u32, y: u32, radius: f64, step: Option<f64>, glyphs: Option<&Glyphs>, palette: Palette, sea: bool) -> Vec<u8> {
    let h = smooth(heights);
    let step = step.unwrap_or_else(|| tile_step(z, y, radius));
    let f = line_fields(&h, step);
    let (land, water, shade, index_shade) = palette.colors();
    let water = if sea { water } else { land };
    let n = h.data.len();
    let mut alpha = vec![0f64; n];
    let mut rgb = vec![[0f64; 3]; n];
    for i in 0..n {
        let width = if f.index[i] { INDEX_WIDTH } else { WIDTH };
        let a = (width + 0.5 - f.gap.data[i]).clamp(0.0, 1.0) * (f.spacing.data[i] / FADE).clamp(0.0, 1.0);
        alpha[i] = f64::from((a * OPACITY) as f32);
        let base = if h.data[i] < 0.0 { water } else { land };
        let k = if f.index[i] { index_shade } else { shade };
        rgb[i] = base.map(|v| f64::from(v) * k);
    }
    if let Some(g) = glyphs {
        let spots = label_spots(&f, g.height);
        for label in place_labels(&h, &f, step, &spots, g) {
            let color = (if label.value < 0.0 { water } else { land }).map(|v| f64::from(v) * index_shade);
            for lr in 0..label.rows {
                for lc in 0..label.cols {
                    let (r, c) = (label.r0 + lr, label.c0 + lc);
                    if r >= h.rows || c >= h.cols {
                        continue;
                    }
                    let i = r * h.cols + c;
                    let j = lr * label.cols + lc;
                    // Разрыв линии под подписью, потом цифры цветом линии.
                    alpha[i] *= 1.0 - f64::from(label.clear[j]);
                    rgb[i] = color;
                    alpha[i] = alpha[i].max(f64::from(label.ink[j]) * OPACITY);
                }
            }
        }
    }
    let mut out = Vec::with_capacity(n * 4);
    for i in 0..n {
        for k in 0..3 {
            out.push((rgb[i][k] * alpha[i]).round_ties_even().clamp(0.0, 255.0) as u8);
        }
        out.push((alpha[i] * 255.0).round_ties_even().clamp(0.0, 255.0) as u8);
    }
    out
}

/// Тайл (уровень, x, y).
pub type Key = (u32, u32, u32);

/// Предок тайла key на уровне level.
pub fn ancestor(key: Key, level: u32) -> Key {
    let (z, x, y) = key;
    let d = z.saturating_sub(level);
    (z - d, x >> d, y >> d)
}

/// Наибольшая высота тайла высот tile_key (предка key) под тайлом key
/// с полосой в пиксель вокруг.
pub fn footprint_high(heights: &Grid, tile_key: Key, key: Key) -> f64 {
    let scale = f64::from(1u32 << (key.0 - tile_key.0).min(30));
    let size = SIZE as f64 / scale;
    let c0 = (f64::from(key.1) - f64::from(tile_key.1) * scale) * size;
    let r0 = (f64::from(key.2) - f64::from(tile_key.2) * scale) * size;
    let lo_c = (c0.floor() as i64 - 1).max(0) as usize;
    let lo_r = (r0.floor() as i64 - 1).max(0) as usize;
    let hi_c = ((c0 + size).ceil() as usize + 1).min(SIZE);
    let hi_r = ((r0 + size).ceil() as usize + 1).min(SIZE);
    let mut high = f64::NEG_INFINITY;
    for r in lo_r..hi_r.min(heights.rows) {
        for c in lo_c..hi_c.min(heights.cols) {
            high = high.max(heights.at(r, c));
        }
    }
    high
}

/// Уровень тайла высот, по которому строятся горизонтали тайла key.
/// sea_high - наибольшая высота тайла уровня SEA_LEVEL над key или None:
/// целиком глубже SEA_DEPTH - открытое море, глубины с SEA_LEVEL, иначе
/// land_level. Не глубже самого key.
pub fn source_level(key: Key, sea_high: Option<f64>, land_level: u32) -> u32 {
    if sea_high.is_some_and(|h| h < SEA_DEPTH) {
        return key.0.min(SEA_LEVEL);
    }
    key.0.min(land_level)
}

/// Кубическая кривая Катмулла-Рома через четыре значения.
fn cubic(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    p1 + 0.5 * t * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)))
}

/// Тайл высот с рамкой FRAME пикселей: строки соседей, где они есть,
/// иначе нечётное отражение (линейное продолжение).
fn framed(heights: &Grid, source: Key, neighbor: Option<&dyn Fn(Key) -> Option<Grid>>) -> Grid {
    let n = SIZE as isize;
    let f = FRAME as isize;
    let reflect = |i: isize, at: &dyn Fn(isize) -> f64| -> f64 {
        if i < 0 {
            2.0 * at(0) - at(-i)
        } else if i >= n {
            2.0 * at(n - 1) - at(2 * (n - 1) - i)
        } else {
            at(i)
        }
    };
    let side = SIZE + 2 * FRAME;
    let mut big = Grid::new(side, side, 0.0);
    for r in -f..n + f {
        for c in -f..n + f {
            // Сначала по столбцам внутри строки, потом по строкам.
            let row_value = |rr: isize| reflect(c, &|cc: isize| heights.at(rr as usize, cc as usize));
            let v = reflect(r, &row_value);
            big.set((r + f) as usize, (c + f) as usize, v);
        }
    }
    let Some(neighbor) = neighbor else {
        return big;
    };
    let (z, x, y) = source;
    let count = 1i64 << z.min(30);
    for dy in -1i64..=1 {
        if !(0..count).contains(&(i64::from(y) + dy)) {
            continue;
        }
        for dx in -1i64..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let key = (z, (i64::from(x) + dx).rem_euclid(count) as u32, (i64::from(y) + dy) as u32);
            let Some(other) = neighbor(key) else {
                continue;
            };
            // Части рамки: строки и столбцы большого тайла и соседа.
            let span = |d: i64| -> (std::ops::Range<usize>, usize) {
                match d {
                    -1 => (0..FRAME, SIZE - FRAME),
                    0 => (FRAME..FRAME + SIZE, 0),
                    _ => (FRAME + SIZE..side, 0),
                }
            };
            let (rows, src_r) = span(dy);
            let (cols, src_c) = span(dx);
            for (i, br) in rows.clone().enumerate() {
                for (j, bc) in cols.clone().enumerate() {
                    let (sr, sc) = (src_r + i, src_c + j);
                    if sr < other.rows && sc < other.cols {
                        big.set(br, bc, other.at(sr, sc));
                    }
                }
            }
        }
    }
    big
}

/// Кубическая выборка сетки big с рамкой pad вокруг данных: ys и xs -
/// строки и столбцы данных, не дальше полупикселя за краем.
fn sample(big: &Grid, pad: usize, ys: &[f64], xs: &[f64]) -> Grid {
    let rows = (big.rows - 2 * pad) as f64;
    let cols = (big.cols - 2 * pad) as f64;
    let xs: Vec<(usize, f64)> = xs
        .iter()
        .map(|x| {
            let v = x.clamp(-0.5, cols - 0.5) + pad as f64;
            (v.floor() as usize, v - v.floor())
        })
        .collect();
    let ys: Vec<(usize, f64)> = ys
        .iter()
        .map(|y| {
            let v = y.clamp(-0.5, rows - 0.5) + pad as f64;
            (v.floor() as usize, v - v.floor())
        })
        .collect();
    Grid::from_fn(ys.len(), xs.len(), |i, j| {
        let (y0, ty) = ys[i];
        let (x0, tx) = xs[j];
        let row = |r: usize| cubic(big.at(r, x0 - 1), big.at(r, x0), big.at(r, x0 + 1), big.at(r, x0 + 2), tx);
        cubic(row(y0 - 1), row(y0), row(y0 + 1), row(y0 + 2), ty)
    })
}

/// Высоты тайла key по высотам тайла-предка source: кубический пересчёт
/// без изломов по сетке предка. neighbor - высоты соседнего тайла уровня
/// source или None, их строки встают за край. smoothed - сначала
/// сгладить предка вместе с рамкой.
pub fn upsample(heights: &Grid, source: Key, key: Key, neighbor: Option<&dyn Fn(Key) -> Option<Grid>>, smoothed: bool) -> Grid {
    let mut big = framed(heights, source, neighbor);
    if smoothed {
        big = smooth(&big);
    }
    let scale = f64::from(1u32 << (key.0 - source.0).min(30));
    let pos: Vec<f64> = (0..SIZE).map(|i| (i as f64 + 0.5) / scale - 0.5).collect();
    let off_x = (f64::from(key.1) - f64::from(source.1) * scale) * SIZE as f64 / scale;
    let off_y = (f64::from(key.2) - f64::from(source.2) * scale) * SIZE as f64 / scale;
    let xs: Vec<f64> = pos.iter().map(|p| off_x + p).collect();
    let ys: Vec<f64> = pos.iter().map(|p| off_y + p).collect();
    sample(&big, FRAME, &ys, &xs)
}

/// Шаг утолщённых горизонталей при сечении step.
pub fn index_step(step: f64) -> f64 {
    step * INDEX
}

/// Отметки изолиний через step в пределах значений, без NaN.
pub fn levels(values: &[f64], step: f64) -> Vec<f64> {
    let finite = values.iter().copied().filter(|v| v.is_finite());
    let (lo, hi) = finite.fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(v), b.max(v)));
    if !lo.is_finite() || step <= 0.0 {
        return Vec::new();
    }
    let (lo, hi) = ((lo / step).ceil() * step, (hi / step).floor() * step);
    if hi < lo {
        return Vec::new();
    }
    let count = ((hi - lo) / step).round() as usize;
    (0..=count).map(|i| lo + i as f64 * step).collect()
}

/// Шаг изолиний сетки: около lines линий на размах значений.
pub fn grid_step(values: &[f64], lines: f64) -> f64 {
    let finite = values.iter().copied().filter(|v| v.is_finite());
    let (lo, hi) = finite.fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(v), b.max(v)));
    if !lo.is_finite() {
        return 0.0;
    }
    let span = hi - lo;
    if span > 0.0 { nice(span / lines) } else { 0.0 }
}

/// Отрезок изолинии: две точки (строка, столбец) в долях узлов сетки.
pub type Segment = [[f64; 2]; 2];

/// Отрезки изолинии level по сетке z, NaN - пусто. Ячейка с пустым углом
/// пропускается. Седло делится по среднему значению ячейки.
pub fn segments(z: &Grid, level: f64) -> Vec<Segment> {
    let mut two = Vec::new();
    let mut saddles = Vec::new();
    for r in 0..z.rows.saturating_sub(1) {
        for c in 0..z.cols.saturating_sub(1) {
            let (va, vb, vc, vd) = (z.at(r, c), z.at(r, c + 1), z.at(r + 1, c), z.at(r + 1, c + 1));
            if ![va, vb, vc, vd].iter().all(|v| v.is_finite()) {
                continue;
            }
            // Рёбра ячейки: верх a-b, право b-d, низ c-d, лево a-c.
            let edges = [(va, vb, 0.0, 0.0, 0.0, 1.0), (vb, vd, 0.0, 1.0, 1.0, 1.0), (vc, vd, 1.0, 0.0, 1.0, 1.0), (va, vc, 0.0, 0.0, 1.0, 0.0)];
            let mut points = [[0.0; 2]; 4];
            let mut hits = [false; 4];
            for (k, (v0, v1, r0, c0, r1, c1)) in edges.into_iter().enumerate() {
                // Узел ровно на уровне считается выше: линия не пропадает.
                let cross = (v0 >= level) != (v1 >= level);
                let t = if cross { (level - v0) / if v1 != v0 { v1 - v0 } else { 1.0 } } else { 0.0 };
                points[k] = [r as f64 + r0 + (r1 - r0) * t, c as f64 + c0 + (c1 - c0) * t];
                hits[k] = cross;
            }
            match hits.iter().filter(|h| **h).count() {
                2 => {
                    let found: Vec<[f64; 2]> = (0..4).filter(|k| hits[*k]).map(|k| points[k]).collect();
                    two.push([found[0], found[1]]);
                }
                4 => {
                    let mid = (va + vb + vc + vd) / 4.0;
                    let high = (mid > level) == (va > level);
                    if high {
                        saddles.push(([points[0], points[1]], [points[2], points[3]]));
                    } else {
                        saddles.push(([points[0], points[3]], [points[1], points[2]]));
                    }
                }
                _ => {}
            }
        }
    }
    // Порядок как у модуля: сначала ячейки с двумя пересечениями, потом
    // первые и вторые пары сёдел.
    let (first, second): (Vec<Segment>, Vec<Segment>) = saddles.into_iter().unzip();
    two.extend(first);
    two.extend(second);
    two
}

/// Отрезки в ломаные. Концы соседних ячеек на общем ребре совпадают,
/// они узнаются по округлению до digits знаков.
pub fn chain(pieces: &[Segment], digits: i32) -> Vec<Vec<[f64; 2]>> {
    let scale = 10f64.powi(digits);
    let key = |p: [f64; 2]| ((p[0] * scale).round() as i64, (p[1] * scale).round() as i64);
    // Узел ровно на уровне даёт отрезки нулевой длины.
    let pieces: Vec<Segment> = pieces.iter().copied().filter(|s| key(s[0]) != key(s[1])).collect();
    if pieces.is_empty() {
        return Vec::new();
    }
    let keys: Vec<(i64, i64)> = pieces.iter().flat_map(|s| [key(s[0]), key(s[1])]).collect();
    let mut ends: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, k) in keys.iter().enumerate() {
        ends.entry(*k).or_default().push(i);
    }
    let mut used = vec![false; pieces.len()];
    let point = |end: usize| pieces[end / 2][end % 2];
    let walk = |end: usize, used: &mut Vec<bool>| -> Vec<[f64; 2]> {
        let mut out = Vec::new();
        let mut end = end;
        loop {
            let other = end ^ 1;
            out.push(point(other));
            let next = ends.get(&keys[other]).and_then(|list| list.iter().copied().find(|e| *e != other && !used[e / 2]));
            let Some(next) = next else {
                return out;
            };
            end = next;
            used[end / 2] = true;
        }
    };
    let mut lines = Vec::new();
    for start in 0..pieces.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let forward = walk(2 * start, &mut used);
        let mut backward = walk(2 * start + 1, &mut used);
        backward.reverse();
        backward.extend(forward);
        lines.push(backward);
    }
    lines
}

/// Высоты тайла Terrarium из RGBA: R·256 + G + B/256 - 32768 метров.
pub fn terrarium(rgba: &[u8]) -> Grid {
    let data = rgba.as_chunks::<4>().0.iter().map(|p| f64::from(p[0]) * 256.0 + f64::from(p[1]) + f64::from(p[2]) / 256.0 - 32768.0).collect();
    Grid { rows: SIZE, cols: SIZE, data }
}
