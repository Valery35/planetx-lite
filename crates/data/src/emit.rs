// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Минеральные карты EMIT: продукт NASA EMIT L2B MIN, определение
//! минералов по спектрам снимков спектрометра EMIT на МКС, пиксель 60 м.
//! Перенос `core/emit.py` и чтения `read_file`, `to_site` из `ui/emit.py`
//! модуля.
//!
//! Просьба автора от 2 ноября 2026 года, третий пункт классификации по
//! спектрам. Каталог - NASA CMR без входа, коллекции EMITL2BMIN версий 001
//! и 002, у сцены одной съёмки берётся старшая версия. Файл - netCDF-4
//! в хранилище LP DAAC lp-prod-protected, по ключу Earthdata. Данные
//! файла - полоса съёмки без геопривязки: group_1_mineral_id
//! и group_1_band_depth (группа 1 - железистые минералы, полосы
//! поглощения 0.4-1.3 мкм), group_2_* (группа 2 - глины, слюда,
//! карбонаты, сульфаты, 2.0-2.5 мкм), группа location - таблица GLT: для
//! клетки сетки широт и долгот номер строки и столбца полосы с 1, 0 - нет
//! данных. Названия минералов - группа mineral_metadata того же файла.
//! Метод определения - Tetracorder (Clark и др., 2003).
//!
//! Модуль читает файл многомерным интерфейсом GDAL, здесь - крейтом
//! netcdf-reader на Rust (MIT и Apache 2.0). Файл считается враждебным:
//! размеры массивов сверяются до выделения памяти.

use serde_json::Value;

use crate::aef::{self, ZoneIndex};

/// Поиск сцен каталога NASA CMR.
pub const SEARCH: &str = "https://cmr.earthdata.nasa.gov/search/granules.umm_json";
/// Коллекции и версии: 002 - пересчёт архива с 2025 года, у части сцен
/// её ещё нет.
pub const COLLECTIONS: [(&str, &str); 2] = [("C4079846859-LPCLOUD", "002"), ("C2408034484-LPCLOUD", "001")];
/// Первый день съёмки EMIT.
pub const FIRST_DAY: &str = "2022-08-09";
/// Сцен в ответе каталога.
pub const PAGE: usize = 200;
/// Подходящая сцена - облачность по каталогу не больше CLOUD_MAX, %,
/// участок под сценой не меньше SHARE_MIN, в списке не больше SHOWN.
/// Выбор помощника модуля 1.31, утверждает автор.
pub const CLOUD_MAX: f64 = 30.0;
pub const SHARE_MIN: f64 = 0.5;
pub const SHOWN: usize = 30;
/// Пиксель продукта, метры.
pub const STEP: f64 = 60.0;
/// Сервер входа Earthdata: ответ с него - файл без ключа.
pub const LOGIN_HOST: &str = "urs.earthdata.nasa.gov";
/// Группы минералов.
pub const GROUPS: [u32; 2] = [1, 2];
/// Точек на сторону участка при оценке покрытия сценой.
const SAMPLES: usize = 15;
/// Минералов в растре классов, остальные - «прочие».
pub const MOST: usize = 254;
/// Подпись источника.
pub const CREDIT: &str = "EMIT L2B MIN, NASA JPL, LP DAAC";
/// Значение «нет данных» файла.
const FILL: f64 = -9999.0;
/// Клеток таблицы GLT и полосы не больше.
const MAX_CELLS: usize = 64 << 20;
/// Цвета минералов по номеру - выбор помощника модуля.
const PALETTE: [[u8; 3]; 20] = [
    [230, 25, 75],
    [60, 180, 75],
    [255, 225, 25],
    [0, 130, 200],
    [245, 130, 48],
    [145, 30, 180],
    [70, 240, 240],
    [240, 50, 230],
    [210, 245, 60],
    [250, 190, 212],
    [0, 128, 128],
    [220, 190, 255],
    [170, 110, 40],
    [255, 250, 200],
    [128, 0, 0],
    [170, 255, 195],
    [128, 128, 0],
    [255, 215, 180],
    [0, 0, 128],
    [128, 128, 128],
];

/// Знаки запроса, которые идут как есть, остальные - %XX, как `urlencode`.
fn quote(text: &str) -> String {
    text.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { char::from(b).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Адрес поиска сцен коллекции в CMR: рамка участка (широта, долгота),
/// дни «ГГГГ-ММ-ДД», облачность, новые первыми.
pub fn cmr_url(ring: &[(f64, f64)], start: &str, end: &str, max_cloud: f64, collection: &str) -> String {
    let fold = |f: fn(f64, f64) -> f64, init: f64, pick: fn(&(f64, f64)) -> f64| ring.iter().map(pick).fold(init, f);
    let (lat0, lat1) = (fold(f64::min, f64::INFINITY, |p| p.0), fold(f64::max, f64::NEG_INFINITY, |p| p.0));
    let (lon0, lon1) = (fold(f64::min, f64::INFINITY, |p| p.1), fold(f64::max, f64::NEG_INFINITY, |p| p.1));
    let query = [
        ("collection_concept_id", collection.to_string()),
        ("bounding_box", format!("{lon0:.6},{lat0:.6},{lon1:.6},{lat1:.6}")),
        ("temporal", format!("{start}T00:00:00Z,{end}T23:59:59Z")),
        ("cloud_cover", format!("0,{max_cloud}")),
        ("sort_key", "-start_date".to_string()),
        ("page_size", PAGE.to_string()),
    ];
    let pairs: Vec<String> = query.iter().map(|(k, v)| format!("{}={}", quote(k), quote(v))).collect();
    format!("{SEARCH}?{}", pairs.join("&"))
}

/// Сцена каталога.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub id: String,
    /// «ГГГГ-ММ-ДД ЧЧ:ММ:СС».
    pub when: String,
    pub cloud: f64,
    /// Контур сцены (широта, долгота).
    pub polygon: Vec<(f64, f64)>,
    /// Адрес файла MIN.
    pub url: String,
    pub preview: String,
    pub version: String,
    /// Доля участка под контуром сцены.
    pub share: f64,
}

fn text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn list(value: Option<&Value>) -> &[Value] {
    value.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// Сцены ответа CMR.
pub fn parse_cmr(data: &Value, version: &str) -> Vec<Scene> {
    let mut out = Vec::new();
    for item in list(data.get("items")) {
        let Some(umm) = item.get("umm") else {
            continue;
        };
        let (mut url, mut preview) = (String::new(), String::new());
        for link in list(umm.get("RelatedUrls")) {
            let href = text(link.get("URL"));
            if !href.starts_with("https://") {
                continue;
            }
            let name = href.rsplit('/').next().unwrap_or("");
            let kind = link.get("Type").and_then(Value::as_str);
            if kind == Some("GET RELATED VISUALIZATION") && name.ends_with(".png") {
                preview.clone_from(&href);
            } else if kind == Some("GET DATA") && name.ends_with(".nc") && name.starts_with("EMIT_L2B_MIN_") {
                url.clone_from(&href);
            }
        }
        if url.is_empty() {
            continue;
        }
        let geometry = umm.get("SpatialExtent").and_then(|s| s.get("HorizontalSpatialDomain")).and_then(|h| h.get("Geometry"));
        let mut polygon: Vec<(f64, f64)> = list(geometry.and_then(|g| g.get("GPolygons")))
            .first()
            .map(|shape| list(shape.get("Boundary").and_then(|b| b.get("Points"))).iter().map(|p| (p.get("Latitude").and_then(Value::as_f64).unwrap_or(0.0), p.get("Longitude").and_then(Value::as_f64).unwrap_or(0.0))).collect())
            .unwrap_or_default();
        if polygon.len() > 1 && polygon.first() == polygon.last() {
            polygon.pop();
        }
        let extent = umm.get("TemporalExtent");
        let begin = extent.and_then(|e| e.get("RangeDateTime")).and_then(|r| r.get("BeginningDateTime")).or_else(|| extent.and_then(|e| e.get("SingleDateTime")));
        let when: String = text(begin).replace('T', " ").trim_end_matches('Z').chars().take(19).collect();
        out.push(Scene { id: text(umm.get("GranuleUR")), when, cloud: umm.get("CloudCover").and_then(Value::as_f64).unwrap_or(0.0), polygon, url, preview, version: version.to_string(), share: 0.0 });
    }
    out
}

/// Сцены нескольких версий: у одной съёмки (секунда начала) остаётся
/// первая по порядку lists, новые первыми.
pub fn merge(lists: Vec<Vec<Scene>>) -> Vec<Scene> {
    let mut out: Vec<Scene> = Vec::new();
    for scene in lists.into_iter().flatten() {
        if !out.iter().any(|s| s.when == scene.when) {
            out.push(scene);
        }
    }
    out.sort_by(|a, b| b.when.cmp(&a.when));
    out
}

/// Подходящие сцены по порядку списка, `suitable` модуля 1.31:
/// облачность не больше [`CLOUD_MAX`], участок под сценой не меньше
/// [`SHARE_MIN`], не больше [`SHOWN`]. Малооблачные и шире покрывающие
/// участок первыми, при равенстве - новые. Облачность без числа
/// не отсеивает сцену.
pub fn suitable(scenes: &[Scene]) -> Vec<Scene> {
    let mut kept: Vec<Scene> = scenes.iter().filter(|s| s.share >= SHARE_MIN && (s.cloud.is_nan() || s.cloud <= CLOUD_MAX)).cloned().collect();
    let cloud = |s: &Scene| if s.cloud.is_finite() { s.cloud.round() } else { 0.0 };
    kept.sort_by(|a, b| cloud(a).total_cmp(&cloud(b)).then((10.0 * b.share).round().total_cmp(&(10.0 * a.share).round())).then(b.when.cmp(&a.when)));
    kept.truncate(SHOWN);
    kept
}

/// Доля участка ring, которую накрывает контур сцены polygon: по сетке
/// точек [`SAMPLES`] × [`SAMPLES`] внутри рамки участка.
pub fn coverage(polygon: &[(f64, f64)], ring: &[(f64, f64)]) -> f64 {
    if polygon.len() < 3 || ring.len() < 3 {
        return 0.0;
    }
    let span = |pick: fn(&(f64, f64)) -> f64| ring.iter().map(pick).fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(v), b.max(v)));
    let ((lat0, lat1), (lon0, lon1)) = (span(|p| p.0), span(|p| p.1));
    let at = |lo: f64, hi: f64, i: usize| lo + (hi - lo) * i as f64 / (SAMPLES - 1) as f64;
    let points: Vec<(f64, f64)> = (0..SAMPLES).flat_map(|r| (0..SAMPLES).map(move |c| (at(lat0, lat1, r), at(lon0, lon1, c)))).collect();
    let within: Vec<(f64, f64)> = points.iter().zip(crate::site::inside(&points, ring)).filter(|(_, i)| *i).map(|(p, _)| *p).collect();
    if within.is_empty() {
        return 0.0;
    }
    crate::site::inside(&within, polygon).iter().filter(|i| **i).count() as f64 / within.len() as f64
}

/// Таблица GLT: номера столбца и строки полосы с 1 у клеток сетки широт
/// и долгот, 0 - нет данных, знак не учитывается.
#[derive(Debug, Clone, PartialEq)]
pub struct Glt {
    pub rows: usize,
    pub cols: usize,
    pub x: Vec<i64>,
    pub y: Vec<i64>,
}

impl Glt {
    /// Клетка полосы strip_rows × strip_cols под клеткой GLT i: номер
    /// с 0 или None.
    fn strip(&self, i: usize, strip_rows: usize, strip_cols: usize) -> Option<usize> {
        let (gx, gy) = (usize::try_from(self.x.get(i)?.unsigned_abs()).ok()?, usize::try_from(self.y.get(i)?.unsigned_abs()).ok()?);
        (gx > 0 && gy > 0 && gx <= strip_cols && gy <= strip_rows).then(|| (gy - 1) * strip_cols + gx - 1)
    }
}

/// Значения полосы strip_rows × strip_cols на сетке GLT, fill - без
/// данных.
pub fn ortho<T: Copy>(values: &[T], strip_rows: usize, strip_cols: usize, glt: &Glt, fill: T) -> Vec<T> {
    (0..glt.rows * glt.cols).map(|i| glt.strip(i, strip_rows, strip_cols).and_then(|k| values.get(k).copied()).unwrap_or(fill)).collect()
}

/// Геопреобразование сетки GLT по широтам и долготам полосы, если
/// в файле нет атрибута geotransform: наименьшие квадраты по клеткам
/// с данными. Итог - (x0, dx, 0, y0, 0, dy) для углов клеток или None.
pub fn fit_transform(glt: &Glt, lats: &[f64], lons: &[f64], strip_rows: usize, strip_cols: usize) -> Option<[f64; 6]> {
    let mut cols = Vec::new();
    let (mut rows, mut lon, mut lat) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..glt.rows * glt.cols {
        if let Some(k) = glt.strip(i, strip_rows, strip_cols) {
            cols.push((i % glt.cols) as f64 + 0.5);
            rows.push((i / glt.cols) as f64 + 0.5);
            lon.push(*lons.get(k)?);
            lat.push(*lats.get(k)?);
        }
    }
    if cols.len() < 3 {
        return None;
    }
    let line = |x: &[f64], y: &[f64]| -> Option<(f64, f64)> {
        let n = x.len() as f64;
        let (mx, my) = (x.iter().sum::<f64>() / n, y.iter().sum::<f64>() / n);
        let sxx: f64 = x.iter().map(|v| (v - mx).powi(2)).sum();
        let sxy: f64 = x.iter().zip(y).map(|(a, b)| (a - mx) * (b - my)).sum();
        (sxx > 0.0).then(|| (sxy / sxx, my - sxy / sxx * mx))
    };
    let (dx, x0) = line(&cols, &lon)?;
    let (dy, y0) = line(&rows, &lat)?;
    Some([x0, dx, 0.0, y0, 0.0, dy])
}

/// Геопреобразование годно: конечные числа и ненулевой шаг.
pub fn fit_ok(transform: &[f64; 6]) -> bool {
    transform.iter().all(|v| v.is_finite()) && transform[1] != 0.0 && transform[5] != 0.0
}

/// Названия минералов групп: (группа, номер, название). Название -
/// sample_name, без него name без пути и окончания файла.
pub fn names_of(index: &[i64], group: &[i64], names: &[String], samples: Option<&[String]>) -> Vec<(i64, i64, String)> {
    let mut out = Vec::new();
    for (n, (number, kind)) in index.iter().zip(group).enumerate() {
        let mut found = samples.and_then(|s| s.get(n)).map(|s| s.trim().to_string()).unwrap_or_default();
        if found.is_empty()
            && let Some(name) = names.get(n)
        {
            let name = name.trim().replace('\\', "/");
            let last = name.rsplit('/').next().unwrap_or("").to_string();
            let tail_has_dot = last.chars().rev().take(5).any(|c| c == '.');
            found = if tail_has_dot { last.rsplit_once('.').map_or(last.clone(), |(a, _)| a.to_string()) } else { last };
        }
        if found.is_empty() {
            found = number.to_string();
        }
        out.push((*kind, *number, found));
    }
    out
}

/// Классы участка по номерам минералов ids и глубине полосы depth:
/// (номера классов, -1 без минерала, номера минералов по убыванию
/// площади). Клетка без минерала - номер меньше 0 или глубина не больше
/// 0 и не число. Больше [`MOST`] минералов - последний класс собирает
/// остальные, его номер минерала -1.
pub fn classes(ids: &[i32], depth: &[f32], mask: Option<&[bool]>) -> (Vec<i16>, Vec<i32>) {
    let good = |i: usize| ids[i] >= 0 && depth.get(i).is_some_and(|d| d.is_finite() && *d > 0.0) && mask.is_none_or(|m| m.get(i).copied().unwrap_or(false));
    let mut counts: Vec<(i32, usize)> = Vec::new();
    for i in (0..ids.len()).filter(|i| good(*i)) {
        match counts.iter_mut().find(|c| c.0 == ids[i]) {
            Some(c) => c.1 += 1,
            None => counts.push((ids[i], 1)),
        }
    }
    let mut kinds = vec![-1i16; ids.len()];
    if counts.is_empty() {
        return (kinds, Vec::new());
    }
    // Порядок модуля: np.unique - по возрастанию номера, потом устойчиво
    // по убыванию площади.
    counts.sort_by_key(|c| c.0);
    counts.sort_by_key(|c| std::cmp::Reverse(c.1));
    let mut kept: Vec<i32> = counts.iter().map(|c| c.0).collect();
    let rest = (kept.len() > MOST).then_some(MOST - 1);
    if rest.is_some() {
        kept.truncate(MOST - 1);
        kept.push(-1);
    }
    for i in (0..ids.len()).filter(|i| good(*i)) {
        let found = kept.iter().position(|m| *m == ids[i] && *m >= 0).or(rest);
        kinds[i] = found.and_then(|k| i16::try_from(k).ok()).unwrap_or(-1);
    }
    (kinds, kept)
}

/// Цвет минерала по номеру, «прочие» - серый.
pub fn color(mineral: i32) -> [u8; 3] {
    match usize::try_from(mineral) {
        Ok(m) => PALETTE[m % PALETTE.len()],
        Err(_) => [160, 160, 160],
    }
}

/// Доли классов от клеток с минералом: (номер класса, доля) по убыванию.
pub fn shares(kinds: &[i16], count: usize) -> Vec<(usize, f64)> {
    let mut counts = vec![0usize; count];
    let mut total = 0usize;
    for k in kinds {
        if let Some(c) = usize::try_from(*k).ok().and_then(|k| counts.get_mut(k)) {
            *c += 1;
            total += 1;
        }
    }
    if total == 0 {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..count).filter(|n| counts[*n] > 0).collect();
    order.sort_by_key(|n| std::cmp::Reverse(counts[*n]));
    order.into_iter().map(|n| (n, counts[n] as f64 / total as f64)).collect()
}

/// Площадь клеток, квадратные километры.
pub fn area_km2(cells: usize) -> f64 {
    cells as f64 * STEP * STEP / 1e6
}

/// Сцена из файла: группа минералов на сетке GLT.
#[derive(Debug, Clone, PartialEq)]
pub struct Minerals {
    /// Номера минералов клеток GLT, -1 без минерала.
    pub ids: Vec<i32>,
    /// Глубина полосы, NaN без данных.
    pub depth: Vec<f32>,
    /// Клетка под полосой съёмки.
    pub covered: Vec<bool>,
    pub rows: usize,
    pub cols: usize,
    /// Геопреобразование сетки GLT в широтах и долготах.
    pub transform: [f64; 6],
    /// Названия минералов группы: (номер, название).
    pub names: Vec<(i32, String)>,
}

/// Почему файл не прочитан.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileProblem {
    /// Не netCDF-4 или нужного массива нет.
    Unreadable(String),
    /// Нет годного геопреобразования.
    Transform,
}

/// Массив переменной path как числа и его форма (строки, столбцы).
fn grid_of(file: &netcdf_reader::NcFile, path: &str) -> Result<(Vec<f64>, usize, usize), FileProblem> {
    let variable = file.variable(path).map_err(|e| FileProblem::Unreadable(format!("{path}: {e}")))?;
    let shape = variable.shape();
    let (rows, cols) = match shape.as_slice() {
        [r, c] => (usize::try_from(*r).unwrap_or(0), usize::try_from(*c).unwrap_or(0)),
        [n] => (1, usize::try_from(*n).unwrap_or(0)),
        _ => return Err(FileProblem::Unreadable(format!("{path}: shape"))),
    };
    if rows.saturating_mul(cols) == 0 || rows.saturating_mul(cols) > MAX_CELLS {
        return Err(FileProblem::Unreadable(format!("{path}: size")));
    }
    let values = file.read_variable_as_f64(path).map_err(|e| FileProblem::Unreadable(format!("{path}: {e}")))?;
    let values: Vec<f64> = values.iter().copied().collect();
    if values.len() != rows * cols {
        return Err(FileProblem::Unreadable(format!("{path}: size")));
    }
    Ok((values, rows, cols))
}

/// Строки или числа одномерной переменной.
fn texts_of(file: &netcdf_reader::NcFile, path: &str) -> Option<Vec<String>> {
    if let Ok(found) = file.read_variable_as_strings(path) {
        return Some(found);
    }
    let values = file.read_variable_as_f64(path).ok()?;
    Some(values.iter().map(|v| if v.fract() == 0.0 && v.abs() < 1e15 { format!("{}", *v as i64) } else { v.to_string() }).collect())
}

/// Файл EMIT L2B MIN из байтов: группа number на сетке GLT, `read_file`
/// модуля.
pub fn read_file(bytes: &[u8], number: u32) -> Result<Minerals, FileProblem> {
    let file = netcdf_reader::NcFile::from_bytes(bytes).map_err(|e| FileProblem::Unreadable(e.to_string()))?;
    let (ids, strip_rows, strip_cols) = grid_of(&file, &format!("group_{number}_mineral_id"))?;
    let (depth, depth_rows, depth_cols) = grid_of(&file, &format!("group_{number}_band_depth"))?;
    if (depth_rows, depth_cols) != (strip_rows, strip_cols) {
        return Err(FileProblem::Unreadable("band_depth: shape".into()));
    }
    let (gx, rows, cols) = grid_of(&file, "location/glt_x")?;
    let (gy, y_rows, y_cols) = grid_of(&file, "location/glt_y")?;
    if (y_rows, y_cols) != (rows, cols) {
        return Err(FileProblem::Unreadable("glt: shape".into()));
    }
    let glt = Glt { rows, cols, x: gx.iter().map(|v| *v as i64).collect(), y: gy.iter().map(|v| *v as i64).collect() };
    let attribute: Option<Vec<f64>> = file.global_attribute("geotransform").ok().and_then(|a| a.value.as_f64_vec());
    let transform = match attribute.as_deref() {
        Some(&[a, b, c, d, e, f]) => Some([a, b, c, d, e, f]),
        _ => {
            let lats = grid_of(&file, "location/lat").ok().map(|g| g.0);
            let lons = grid_of(&file, "location/lon").ok().map(|g| g.0);
            lats.zip(lons).and_then(|(la, lo)| fit_transform(&glt, &la, &lo, strip_rows, strip_cols))
        }
    };
    let Some(transform) = transform.filter(fit_ok) else {
        return Err(FileProblem::Transform);
    };
    let index: Vec<i64> = texts_of(&file, "mineral_metadata/index").unwrap_or_default().iter().filter_map(|t| t.trim().parse::<f64>().ok()).map(|v| v as i64).collect();
    let group: Vec<i64> = texts_of(&file, "mineral_metadata/group").unwrap_or_default().iter().filter_map(|t| t.trim().parse::<f64>().ok()).map(|v| v as i64).collect();
    let files = texts_of(&file, "mineral_metadata/name").unwrap_or_default();
    let samples = texts_of(&file, "mineral_metadata/sample_name");
    let names = names_of(&index, &group, &files, samples.as_deref()).into_iter().filter(|(g, _, _)| *g == i64::from(number)).filter_map(|(_, n, t)| Some((i32::try_from(n).ok()?, t))).collect();
    let ids: Vec<i32> = ids.iter().map(|v| if *v == FILL || !v.is_finite() || *v < 0.0 { -1 } else { *v as i32 }).collect();
    let depth: Vec<f32> = depth.iter().map(|v| if *v <= FILL + 1.0 || !v.is_finite() { f32::NAN } else { *v as f32 }).collect();
    let covered = ortho(&vec![true; strip_rows * strip_cols], strip_rows, strip_cols, &glt, false);
    Ok(Minerals { ids: ortho(&ids, strip_rows, strip_cols, &glt, -1), depth: ortho(&depth, strip_rows, strip_cols, &glt, f32::NAN), covered, rows, cols, transform, names })
}

/// Минералы на сетке участка.
#[derive(Debug, Clone, PartialEq)]
pub struct SiteGrid {
    pub x0: f64,
    pub y0: f64,
    pub step: f64,
    pub rows: usize,
    pub cols: usize,
    pub ids: Vec<i32>,
    pub depth: Vec<f32>,
    /// Клетка под полосой съёмки.
    pub seen: Vec<bool>,
}

/// Номера, глубина и покрытие сценой на сетке участка bbox в зоне UTM
/// указателя index с шагом [`STEP`], ближайшим соседом, `to_site` модуля.
pub fn to_site(m: &Minerals, index: &ZoneIndex, bbox: (f64, f64, f64, f64)) -> SiteGrid {
    let (x0, y0, cols, rows) = aef::grid_frame(index, bbox, STEP);
    let t = &m.transform;
    let mut out = SiteGrid { x0, y0, step: STEP, rows, cols, ids: vec![-1; rows * cols], depth: vec![f32::NAN; rows * cols], seen: vec![false; rows * cols] };
    for r in 0..rows {
        for c in 0..cols {
            let (lat, lon) = aef::from_utm(index.number, index.south, x0 + (c as f64 + 0.5) * STEP, y0 - (r as f64 + 0.5) * STEP);
            let (gc, gr) = (((lon - t[0]) / t[1]).floor(), ((lat - t[3]) / t[5]).floor());
            if !(gc >= 0.0 && gr >= 0.0 && gc < m.cols as f64 && gr < m.rows as f64) {
                continue;
            }
            let k = gr as usize * m.cols + gc as usize;
            let i = r * cols + c;
            out.seen[i] = m.covered.get(k).copied().unwrap_or(false);
            if let Some(id) = m.ids.get(k).copied().filter(|id| *id >= 0) {
                out.ids[i] = id;
                out.depth[i] = m.depth.get(k).copied().unwrap_or(f32::NAN);
            }
        }
    }
    out
}
