// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Погружающиеся плиты Slab2 на разрезах. Перенос `core/slabs.py` модуля.
//!
//! Slab2 (Hayes, 2018, doi:10.5066/F7PV6JNV) - верхняя поверхность,
//! толщина и падение плиты в 27 зонах субдукции, данные USGS
//! в общественном достоянии США.
//!
//! Плита не отдельный слой: она видна на разрезе полосой с толщиной там,
//! где разрез проходит через зону, и данные зоны запрашиваются, когда
//! разрез её касается. Файлы зон лежат в хранилище planetx-terrain
//! ([`URL`], зеркала [`TERRAIN_REPOS`]), их собирает
//! `tools/build_slabs.py` модуля. Указатель зон с рамками -
//! `data/slab2_index.json` модуля, его читает [`read_index`].
//!
//! Полоса на вертикальном разрезе - от верхней поверхности плиты вниз
//! на видимую толщину: толщина по нормали к плите, делённая на косинус
//! падения, не больше [`MAX_DIP`]. На разрезе вкрест простирания это
//! настоящая толщина в вертикальном сечении, на косом разрезе - оценка.

use std::collections::BTreeMap;

use crate::npz;

/// Шаблон адреса файла зоны, `{code}` - код зоны.
pub const URL: &str = "https://raw.githubusercontent.com/Valery35/planetx-terrain/main/slab2/{code}.npz";
/// Зеркала хранилища planetx-terrain: GitHub и CDN jsDelivr с теми же
/// путями.
pub const TERRAIN_REPOS: [&str; 2] =
    ["https://raw.githubusercontent.com/Valery35/planetx-terrain/main", "https://cdn.jsdelivr.net/gh/Valery35/planetx-terrain@main"];
/// Путь файла зоны в хранилище, `{code}` - код зоны.
pub const PATH: &str = "/slab2/{code}.npz";
/// Пустой узел в файле зоны.
pub const EMPTY: f64 = -32768.0;
/// Градусы: круче видимая толщина не растёт.
pub const MAX_DIP: f64 = 75.0;
/// Подпись источника: текст и адрес.
pub const ATTRIBUTION: (&str, &str) =
    ("Slabs: USGS Slab2", "https://www.usgs.gov/data/slab2-a-comprehensive-subduction-zone-geometry-model");

/// Файл зоны или указателя не того вида.
#[derive(Debug, Clone, PartialEq)]
pub struct SlabError(pub String);

impl std::fmt::Display for SlabError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SlabError {}

/// Рамка зоны, градусы. Запад больше востока - рамка пересекает линию
/// перемены дат.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub south: f64,
    pub north: f64,
    pub west: f64,
    pub east: f64,
}

/// Зона: код USGS, долгота и широта первого узла (северо-западного), шаг
/// в градусах, глубина и толщина в км, падение в градусах - rows × cols
/// по строкам, пусто - NaN. Строки с севера на юг, столбцы с запада
/// на восток.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Slab {
    pub code: String,
    pub lon0: f64,
    pub lat0: f64,
    pub step: f64,
    pub rows: usize,
    pub cols: usize,
    pub depth: Vec<f64>,
    pub thickness: Vec<f64>,
    pub dip: Vec<f64>,
}

impl Slab {
    fn node(values: &[f64], at: Option<usize>) -> f64 {
        at.and_then(|i| values.get(i)).copied().unwrap_or(f64::NAN)
    }
}

/// Указатель зон из байтов JSON: код - рамка.
pub fn read_index(data: &[u8]) -> Result<BTreeMap<String, Frame>, SlabError> {
    let value: serde_json::Value = serde_json::from_slice(data).map_err(|e| SlabError(e.to_string()))?;
    let object = value.as_object().ok_or_else(|| SlabError("index is not an object".into()))?;
    object
        .iter()
        .map(|(code, v)| {
            let get = |key: &str| v.get(key).and_then(serde_json::Value::as_f64).ok_or_else(|| SlabError(format!("{code}: no {key}")));
            Ok((code.clone(), Frame { south: get("south")?, north: get("north")?, west: get("west")?, east: get("east")? }))
        })
        .collect()
}

/// Зона из байтов файла slab2/<код>.npz: origin (долгота, широта, шаг),
/// depth и thickness в десятых долях км, dip в градусах, одной сетки.
pub fn load(data: &[u8], code: &str) -> Result<Slab, SlabError> {
    let arrays = npz::read_npz(data, &["origin", "depth", "thickness", "dip"]).map_err(SlabError)?;
    let [origin, depth, thickness, dip] = <[npz::Array; 4]>::try_from(arrays).map_err(|_| SlabError("slab file".into()))?;
    let &[lon0, lat0, step] = origin.values.as_slice() else {
        return Err(SlabError(format!("origin of {} values", origin.values.len())));
    };
    let &[rows, cols] = depth.shape.as_slice() else {
        return Err(SlabError(format!("depth of shape {:?}", depth.shape)));
    };
    if thickness.shape != depth.shape || dip.shape != depth.shape {
        return Err(SlabError(format!("grids {:?} {:?} {:?}", depth.shape, thickness.shape, dip.shape)));
    }
    let decode = |v: &f64| if *v == EMPTY { f64::NAN } else { v / 10.0 };
    Ok(Slab {
        code: code.to_string(),
        lon0,
        lat0,
        step,
        rows,
        cols,
        depth: depth.values.iter().map(decode).collect(),
        thickness: thickness.values.iter().map(decode).collect(),
        dip: dip.values,
    })
}

/// Долгота в пределы -180..180, как `(lon + 180) % 360 - 180`.
fn wrap(lon: f64) -> f64 {
    (lon + 180.0).rem_euclid(360.0) - 180.0
}

/// Точка в рамке зоны.
pub fn in_box(frame: &Frame, lat: f64, lon: f64) -> bool {
    let lon = wrap(lon);
    let across = if frame.west <= frame.east {
        lon >= frame.west && lon <= frame.east
    } else {
        lon >= frame.west || lon <= frame.east
    };
    lat >= frame.south && lat <= frame.north && across
}

/// Коды зон, рамку которых задевают точки разреза, по алфавиту.
pub fn touched(index: &BTreeMap<String, Frame>, lats: &[f64], lons: &[f64]) -> Vec<String> {
    index
        .iter()
        .filter(|(_, frame)| lats.iter().zip(lons).any(|(la, lo)| in_box(frame, *la, *lo)))
        .map(|(code, _)| code.clone())
        .collect()
}

/// Номер ближайшего узла сетки зоны или None - точка вне сетки.
fn node_of(slab: &Slab, lat: f64, lon: f64) -> Option<usize> {
    let i = ((slab.lat0 - lat) / slab.step).round_ties_even();
    let j = ((lon - slab.lon0).rem_euclid(360.0) / slab.step).round_ties_even();
    let fits = |v: f64, n: usize| v.is_finite() && v >= 0.0 && v < n as f64;
    if !fits(i, slab.rows) || !fits(j, slab.cols) {
        return None;
    }
    (i as usize).checked_mul(slab.cols)?.checked_add(j as usize)
}

/// Глубина плиты в точке по ближайшему узлу, км, или NaN.
pub fn depth_at(slab: &Slab, lat: f64, lon: f64) -> f64 {
    Slab::node(&slab.depth, node_of(slab, lat, lon))
}

/// Полоса плит вдоль разреза: верх и низ в км для каждой точки, NaN -
/// плиты нет. Из нескольких зон берётся первая с плитой.
pub fn band<'a>(zones: impl IntoIterator<Item = &'a Slab>, lats: &[f64], lons: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let n = lats.len().min(lons.len());
    let mut top = vec![f64::NAN; n];
    let mut bottom = vec![f64::NAN; n];
    for slab in zones {
        for k in 0..n {
            if !top[k].is_nan() {
                continue;
            }
            let at = node_of(slab, lats[k], lons[k]);
            let depth = Slab::node(&slab.depth, at);
            let thick = Slab::node(&slab.thickness, at);
            if !depth.is_finite() || !thick.is_finite() {
                continue;
            }
            // np.minimum: NaN падения остаётся NaN.
            let dip = if at.is_some() { Slab::node(&slab.dip, at) } else { 0.0 };
            let dip = if dip.is_nan() { dip } else { dip.min(MAX_DIP) };
            top[k] = depth;
            bottom[k] = depth + thick / dip.to_radians().cos();
        }
    }
    (top, bottom)
}

/// Адрес файла зоны. base - шаблон вместо [`URL`], для проверки
/// на локальной копии хранилища, пустой - [`URL`].
pub fn url_of(code: &str, base: Option<&str>) -> String {
    base.filter(|b| !b.is_empty()).unwrap_or(URL).replace("{code}", code)
}
