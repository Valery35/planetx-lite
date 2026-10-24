// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Исторические карты коллекции Дэвида Рамсея. Перенос `core/rumsey.py`
//! модуля.
//!
//! Набор Стэнфорда «David Rumsey Map Collection: Georeferenced Maps
//! Metadata» (CC0) несёт для всех 56 тысяч карт опорные точки привязки,
//! обрезку поля карты и адрес изображения IIIF, изображение по кускам
//! открыто. Изображение собирается из кусков и привязывается по опорным
//! точкам тем же способом, что у Рамсея. Каталог - planetx-terrain/rumsey,
//! его собирает `tools/build_rumsey.py` модуля: [`INDEX_PATH`] со всеми
//! картами и [`CELL_PATH`] с подробностями по клеткам.
//!
//! Изображения у Рамсея - CC BY-NC-SA 3.0: некоммерческое использование
//! с указанием источника, программа их не хранит и не передаёт, скачивает
//! пользователь.

use serde_json::Value;

/// Служба изображений IIIF по умолчанию.
pub const IIIF: &str = "https://images.iiifhosting.com/iiif/";
/// Пути указателя и клетки каталога от корня зеркала planetx-terrain.
pub const INDEX_PATH: &str = "/rumsey/index.json";
pub const CELL_PATH: &str = "/rumsey/cells/{cell}.json";
pub const ATTRIBUTION: (&str, &str) = ("Historical maps: David Rumsey Map Collection, CC BY-NC-SA 3.0", "https://www.davidrumsey.com");
/// Пикселей по большей стороне собранного изображения.
pub const MAX_SIDE: u64 = 3072;
/// Карт в списке окна.
pub const LIST_MAX: usize = 400;
/// Во сколько раз площадь рамки больше вида - в конец списка.
pub const LARGE: f64 = 40.0;
pub const TILE: u64 = 256;
/// Кусков изображения не больше: враждебная запись с огромным размером
/// не занимает память списком кусков.
pub const MAX_PIECES: usize = 4096;

/// Рамка: запад, юг, восток, север, градусы.
pub type Bbox = [f64; 4];

/// Указатель карт: номера, рамки, годы, названия, масштабы, клетки.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Catalogue {
    pub number: Vec<i64>,
    pub bbox: Vec<Bbox>,
    pub year: Vec<Option<i64>>,
    pub title: Vec<String>,
    pub scale: Vec<Option<f64>>,
    pub cell: Vec<String>,
}

impl Catalogue {
    /// Указатель из записей [номер, запад, юг, восток, север, год,
    /// название, масштаб, клетка]. Запись без номера или рамки
    /// пропускается.
    pub fn from_json(json: &Value) -> Self {
        let mut out = Self::default();
        let Some(maps) = json.get("maps").and_then(Value::as_array) else {
            return out;
        };
        for m in maps {
            let Some(m) = m.as_array() else { continue };
            let Some(number) = m.first().and_then(int) else { continue };
            let mut bbox = [0.0; 4];
            let mut ok = true;
            for (k, slot) in bbox.iter_mut().enumerate() {
                match m.get(k + 1).and_then(Value::as_f64) {
                    Some(v) => *slot = v,
                    None => ok = false,
                }
            }
            if !ok {
                continue;
            }
            out.number.push(number);
            out.bbox.push(bbox);
            out.year.push(m.get(5).and_then(int));
            out.title.push(m.get(6).and_then(Value::as_str).unwrap_or("").to_string());
            out.scale.push(m.get(7).and_then(Value::as_f64));
            out.cell.push(m.get(8).and_then(Value::as_str).unwrap_or("").to_string());
        }
        out
    }

    /// Указатель из текста index.json. Ошибка - текст не JSON.
    pub fn from_text(text: &str) -> Result<Self, String> {
        let json: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        Ok(Self::from_json(&json))
    }

    pub fn len(&self) -> usize {
        self.number.len()
    }

    pub fn is_empty(&self) -> bool {
        self.number.is_empty()
    }
}

/// Целое из числа JSON, дробное с нулевой дробной частью тоже.
fn int(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_f64().filter(|f| f.fract() == 0.0 && f.abs() < 9e15).map(|f| f as i64))
}

/// Площадь рамки в квадратных градусах с поправкой на широту.
pub fn area(bbox: &Bbox) -> f64 {
    let middle = ((bbox[1] + bbox[3]) / 2.0).to_radians();
    (bbox[2] - bbox[0]).max(1e-6) * (bbox[3] - bbox[1]).max(1e-6) * middle.cos()
}

/// Номера строк указателя - карты у вида: рамка задевает рамку вида view
/// (запад, юг, восток, север). Сначала карты с точкой взгляда (lat, lon)
/// внутри, по площади от подробных к обзорным, карты много больше вида -
/// в конце. Не больше [`LIST_MAX`].
pub fn covering(catalogue: &Catalogue, lat: f64, lon: f64, view: Bbox) -> Vec<usize> {
    let [west, south, east, north] = view;
    let view_area = area(&[west, south, if east > west { east } else { east + 360.0 }, north]);
    let mut hits: Vec<(bool, bool, f64, usize)> = Vec::new();
    for (row, b) in catalogue.bbox.iter().enumerate() {
        let lat_hit = b[3] >= south && b[1] <= north;
        let lon_hit = if west <= east { b[2] >= west && b[0] <= east } else { b[2] >= west || b[0] <= east };
        if !(lat_hit && lon_hit) {
            continue;
        }
        let inside = b[0] <= lon && b[2] >= lon && b[1] <= lat && b[3] >= lat;
        let size = area(b);
        let large = size > LARGE * view_area.max(1e-6);
        hits.push((!inside, large, size, row));
    }
    // Сортировка устойчивая, как numpy.lexsort.
    hits.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.total_cmp(&b.2)));
    hits.into_iter().take(LIST_MAX).map(|h| h.3).collect()
}

/// Номера строк указателя с text в названии или годе по всему каталогу,
/// не больше limit. Пустой текст - пустой список.
pub fn search(catalogue: &Catalogue, text: &str, limit: usize) -> Vec<usize> {
    let text = text.to_lowercase();
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (row, (title, year)) in catalogue.title.iter().zip(&catalogue.year).enumerate() {
        if found.len() >= limit {
            break;
        }
        // Год 0 и без года, как `year or ""` модуля, - пусто.
        let year = year.filter(|y| *y != 0).map(|y| y.to_string()).unwrap_or_default();
        let line = format!("{year} {title}").to_lowercase();
        if words.iter().all(|w| line.contains(w)) {
            found.push(row);
        }
    }
    found
}

/// Расстояние камеры, с которого рамка карты видна целиком, метры.
/// fov_deg - угол зрения, у модуля по умолчанию 40°.
pub fn fly_distance(bbox: &Bbox, fov_deg: f64) -> f64 {
    let [west, south, east, north] = *bbox;
    let middle = ((south + north) / 2.0).to_radians();
    let width = (east - west) * 111320.0 * middle.cos().max(0.05);
    let height = (north - south) * 110540.0;
    let size = width.max(height).max(500.0);
    0.6 * size / (fov_deg.to_radians() / 2.0).tan()
}

/// Адрес службы IIIF изображения из записи каталога.
pub fn image_url(image: &str) -> String {
    if image.starts_with("http") { image.to_string() } else { format!("{IIIF}{image}") }
}

/// Кусок изображения: область в полном размере и место в собранном.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Piece {
    pub x: u64,
    pub y: u64,
    pub w: u64,
    pub h: u64,
    pub ox: u64,
    pub oy: u64,
    pub ow: u64,
    pub oh: u64,
}

/// План сборки: уменьшение, размер собранного изображения и куски.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TilePlan {
    pub factor: u64,
    pub width: u64,
    pub height: u64,
    pub pieces: Vec<Piece>,
}

/// Уменьшение и куски изображения width × height. factors - степени
/// уменьшения службы IIIF, нули пропускаются. Берётся наименьшее
/// уменьшение, при котором большая сторона не больше max_side. None -
/// кусков вышло бы больше [`MAX_PIECES`].
pub fn tile_plan(width: u64, height: u64, factors: &[u64], max_side: u64, tile: u64) -> Option<TilePlan> {
    let mut factors: Vec<u64> = factors.iter().copied().filter(|f| *f > 0).collect();
    factors.sort_unstable();
    if factors.is_empty() {
        factors.push(1);
    }
    let mut factor = factors.last().copied().unwrap_or(1);
    for &f in &factors {
        if width.max(height) as f64 / f as f64 <= max_side as f64 {
            factor = f;
            break;
        }
    }
    let step = tile.max(1).saturating_mul(factor);
    let count = width.div_ceil(step).saturating_mul(height.div_ceil(step));
    if count > MAX_PIECES as u64 {
        return None;
    }
    let mut pieces = Vec::new();
    for y in (0..height).step_by(usize::try_from(step).ok()?) {
        for x in (0..width).step_by(usize::try_from(step).ok()?) {
            let (w, h) = (step.min(width - x), step.min(height - y));
            pieces.push(Piece { x, y, w, h, ox: x / factor, oy: y / factor, ow: w.div_ceil(factor), oh: h.div_ceil(factor) });
        }
    }
    Some(TilePlan { factor, width: width.div_ceil(factor), height: height.div_ceil(factor), pieces })
}

/// Адрес куска IIIF Image API 2: область и ширина в пикселях.
pub fn piece_url(service: &str, piece: &Piece) -> String {
    format!("{service}/{},{},{},{}/{},/0/default.jpg", piece.x, piece.y, piece.w, piece.h, piece.ow)
}

/// Способ привязки по опорным точкам.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Warp {
    /// Тонкая пластина.
    Tps,
    /// Полином указанной степени.
    Polynomial(u32),
}

/// Способ привязки по способу Рамсея method и количеству точек count:
/// tps - тонкая пластина (от трёх точек), polynomial - полином второй
/// степени (третьей при 10 и больше точках), прочее - первой.
pub fn warp_options(method: &str, count: usize) -> Warp {
    if method == "tps" && count >= 3 {
        return Warp::Tps;
    }
    if method == "polynomial" {
        return Warp::Polynomial(if count >= 10 { 3 } else { 2 });
    }
    Warp::Polynomial(1)
}

/// Опорные точки (x, y, долгота, широта) в пикселях собранного
/// изображения с уменьшением factor.
pub fn scaled_points(points: &[(f64, f64, f64, f64)], factor: f64) -> Vec<(f64, f64, f64, f64)> {
    points.iter().map(|p| (p.0 / factor, p.1 / factor, p.2, p.3)).collect()
}

/// Год карты строкой, годы до нашей эры - «N до н. э.».
pub fn year_text(year: Option<i64>) -> String {
    match year {
        None => String::new(),
        Some(y) if y > 0 => y.to_string(),
        Some(y) => format!("{} до н. э.", 1i64.saturating_sub(y)),
    }
}
