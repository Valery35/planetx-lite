// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Космоснимки по участку: поиск сцен, облачность над участком,
//! сочетания каналов и индексы. Перенос `core/sentinel.py` модуля,
//! окно «Космоснимки» модуля - `ui/sentinel.py`.
//!
//! Sentinel-2 - каталог STAC API Earth Search (Element 84), коллекция
//! sentinel-2-l2a: поиск по геометрии, датам и облачности сцены, ответ
//! страницами со ссылкой next. Каналы - COG на открытом хранилище AWS
//! sentinel-cogs, без ключа. У каждого канала в каталоге масштаб
//! и сдвиг: отражение = значение × scale + offset.
//!
//! Облачность над участком - по маске классов сцены SCL (20 м): доля
//! облаков, теней и перистых облаков среди пикселей участка с данными.
//!
//! Участок - многоугольник в широтах и долготах. Точка становится
//! квадратом со стороной side метров. Рамка и маска считаются в системе
//! UTM сцены (proj:epsg) рядами Крюгера.
//!
//! Landsat 4-9 - каталог STAC API Microsoft Planetary Computer,
//! коллекция landsat-c2-l2. Файлы лежат в Azure и читаются по
//! временному ключу SAS, его служба Planetary Computer выдаёт без
//! учётной записи ([`sas_url`], [`parse_token`], [`sign`]). Облака -
//! битовая маска qa_pixel. Ближний ИК Landsat - nir08 ([`band`]).
//! Тепловой канал - lwir11 у Landsat 8-9 и lwir у 4-7, продукт lst -
//! температура поверхности в °C.
//!
//! Landsat 1-5 MSS - коллекция landsat-c2-l1 того же каталога, каналы
//! green, red, nir08, nir09, пиксель 60 м. Отражение на верху атмосферы -
//! по REFLECTANCE_MULT и ADD файла mtl.json сцены ([`toa`]). Полосы
//! насыщения заменяются соседями ([`clean_mss`]).
//!
//! ASTER - [`aster`], MODIS - [`modis`].
//!
//! Массивы numpy модуля - [`Grid`], строки сверху вниз. Сырые значения
//! каналов - `Grid<f64>`, в нём точно помещаются целые uint8, uint16
//! и int16. Отражения - `Grid<f32>`, как float32 модуля. Чтение окон COG
//! по сети (GDAL в модуле) сюда не входит: функции берут уже прочитанные
//! массивы.

pub mod aster;
pub mod modis;

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

/// Поиск Sentinel-2 - STAC API Earth Search.
pub const SEARCH: &str = "https://earth-search.aws.element84.com/v1/search";
pub const COLLECTION: &str = "sentinel-2-l2a";
/// Поиск Landsat, ASTER и MODIS - STAC API Planetary Computer.
pub const LANDSAT_SEARCH: &str = "https://planetarycomputer.microsoft.com/api/stac/v1/search";
pub const LANDSAT_COLLECTION: &str = "landsat-c2-l2";
pub const LANDSAT_SAS_URL: &str =
    "https://planetarycomputer.microsoft.com/api/sas/v1/token/landsat-c2-l2";
/// Служба ключей SAS: к адресу добавляется коллекция или «учётная
/// запись/контейнер» хранилища Azure.
pub const SAS_URL: &str = "https://planetarycomputer.microsoft.com/api/sas/v1/token/";
pub const MSS_COLLECTION: &str = "landsat-c2-l1";
/// Заголовки запроса поиска STAC, как у окна модуля.
pub const SEARCH_HEADERS: [(&str, &str); 2] = [
    ("Content-Type", "application/json"),
    ("Accept", "application/geo+json"),
];
/// Запросов ключа SAS до отказа, пауза между ними, мс, и срок, после
/// которого ключ просится заново, с - раньше часа его жизни. Из окна
/// модуля.
pub const SAS_TRIES: u32 = 3;
pub const SAS_PAUSE: u64 = 2000;
pub const SAS_LIFE: u64 = 45 * 60;
/// Спутники Landsat.
pub const LANDSATS: [Mission; 2] = [Mission::Landsat, Mission::Mss];
/// Биты qa_pixel Landsat Collection 2.
pub const QA_FILL: i64 = 1;
pub const QA_CLOUDY: i64 = (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4);
pub const LANDSAT_STEPS: [f64; 3] = [30.0, 60.0, 120.0];
pub const MSS_STEPS: [f64; 2] = [60.0, 120.0];
/// Четыре канала MSS по порядку длин волн: B4-B7 у Landsat 1-3, B1-B4
/// у Landsat 4-5.
pub const MSS_BANDS: [&str; 4] = ["green", "red", "nir08", "nir09"];
pub const MSS_PRODUCTS: [&str; 4] = ["infrared", "mss_bands", "ndvi", "ndwi"];
pub const LANDSAT_ATTRIBUTION: &str =
    "Landsat {number} image courtesy of the U.S. Geological Survey";
pub const ATTRIBUTION: &str = "Contains modified Copernicus Sentinel data {year}";

/// Шкала продукта: пределы и цвета по значениям.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    pub range: (f64, f64),
    pub stops: &'static [(f64, [u8; 3])],
}

/// Температура поверхности, °C: пределы шкалы и цвета.
pub const TEMPERATURE: Scale = Scale {
    range: (-20.0, 45.0),
    stops: &[
        (-20.0, [49, 54, 149]),
        (0.0, [116, 173, 209]),
        (10.0, [224, 243, 248]),
        (20.0, [254, 224, 144]),
        (30.0, [244, 109, 67]),
        (45.0, [165, 0, 38]),
    ],
};
pub const KELVIN: f64 = 273.15;
/// K1, K2 теплового канала по номеру Landsat - из метаданных Landsat.
/// Дыры температуры закрываются формулой USGS ([`thermal_fill`]).
pub const THERMAL_K: [(&str, (f64, f64)); 5] = [
    ("4", (671.62, 1284.30)),
    ("5", (607.76, 1260.56)),
    ("7", (666.09, 1282.71)),
    ("8", (774.8853, 1321.0789)),
    ("9", (799.0284, 1329.2405)),
];
pub const FILL_BANDS: [&str; 6] = ["trad", "atran", "urad", "drad", "red", "nir08"];
/// Сцен на страницу ответа каталога.
pub const PAGE: usize = 100;
/// Сцен в поиске, дальше страницы не просятся.
pub const MAX_SCENES: usize = 400;
/// Классы SCL: 0 нет данных, 1 насыщенные и дефектные, 3 тени облаков,
/// 8 и 9 облака средней и высокой вероятности, 10 перистые облака.
pub const SCL_EMPTY: [f64; 2] = [0.0, 1.0];
pub const SCL_CLOUDY: [f64; 4] = [3.0, 8.0, 9.0, 10.0];
/// Пикселей участка в выдаче не больше MAX_PIXELS по стороне, иначе шаг
/// растёт: 10, 20, 60 м.
pub const MAX_PIXELS: f64 = 4000.0;
/// Сумма отражений, ниже которой нормированная разность не считается.
pub const MIN_SUM: f32 = 1e-3;
pub const STEPS: [f64; 3] = [10.0, 20.0, 60.0];

/// Сочетания каналов (красный, зелёный, синий канал картинки) -
/// принятые в дистанционном зондировании сочетания Sentinel-2.
pub const COMPOSITES: [(&str, [&str; 3]); 6] = [
    ("natural", ["red", "green", "blue"]),      // B4 B3 B2
    ("infrared", ["nir", "red", "green"]),      // B8 B4 B3
    ("agriculture", ["swir16", "nir", "blue"]), // B11 B8 B2
    ("swir", ["swir22", "nir08", "red"]),       // B12 B8A B4
    ("geology", ["swir22", "swir16", "blue"]),  // B12 B11 B2
    ("urban", ["swir22", "swir16", "red"]),     // B12 B11 B4
];

/// Нормированная разность (a - b) / (a + b) и её шкала.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Index {
    pub name: &'static str,
    pub bands: (&'static str, &'static str),
    pub scale: Scale,
}

pub const INDICES: [Index; 5] = [
    Index {
        name: "ndvi",
        bands: ("nir", "red"),
        scale: Scale {
            range: (-0.2, 0.9),
            stops: &[
                (-0.2, [165, 0, 38]),
                (0.0, [244, 109, 67]),
                (0.2, [254, 224, 139]),
                (0.4, [217, 239, 139]),
                (0.6, [102, 189, 99]),
                (0.9, [0, 104, 55]),
            ],
        },
    },
    Index {
        name: "ndwi",
        bands: ("green", "nir"),
        scale: Scale {
            range: (-0.6, 0.6),
            stops: &[
                (-0.6, [245, 240, 220]),
                (0.0, [198, 219, 239]),
                (0.3, [66, 146, 198]),
                (0.6, [8, 48, 107]),
            ],
        },
    },
    Index {
        name: "ndmi",
        bands: ("nir08", "swir16"),
        scale: Scale {
            range: (-0.5, 0.6),
            stops: &[
                (-0.5, [140, 81, 10]),
                (0.0, [246, 232, 195]),
                (0.3, [90, 180, 172]),
                (0.6, [1, 102, 94]),
            ],
        },
    },
    Index {
        name: "nbr",
        bands: ("nir", "swir22"),
        scale: Scale {
            range: (-0.5, 0.8),
            stops: &[
                (-0.5, [103, 0, 13]),
                (0.0, [252, 187, 161]),
                (0.3, [199, 233, 192]),
                (0.8, [0, 109, 44]),
            ],
        },
    },
    Index {
        name: "ndsi",
        bands: ("green", "swir16"),
        scale: Scale {
            range: (-0.5, 1.0),
            stops: &[
                (-0.5, [120, 90, 60]),
                (0.0, [200, 200, 200]),
                (0.4, [166, 206, 227]),
                (1.0, [255, 255, 255]),
            ],
        },
    },
];

/// Продукты Sentinel-2: сочетания, затем индексы.
pub const PRODUCTS: [&str; 11] = [
    "natural",
    "infrared",
    "agriculture",
    "swir",
    "geology",
    "urban",
    "ndvi",
    "ndwi",
    "ndmi",
    "nbr",
    "ndsi",
];
/// Продукты Landsat 4-9: те же и температура поверхности.
pub const LANDSAT_PRODUCTS: [&str; 12] = [
    "natural",
    "infrared",
    "agriculture",
    "swir",
    "geology",
    "urban",
    "ndvi",
    "ndwi",
    "ndmi",
    "nbr",
    "ndsi",
    "lst",
];
/// «Все каналы» Sentinel-2 L2A - 12 каналов по порядку номеров.
const S2_ALL: [&str; 12] = [
    "coastal", "blue", "green", "red", "rededge1", "rededge2", "rededge3", "nir", "nir08", "nir09",
    "swir16", "swir22",
];
/// «Все каналы» Landsat - 8 с береговым coastal у 8-9, тепловой в
/// кельвинах.
const LANDSAT_ALL: [&str; 8] = [
    "coastal", "blue", "green", "red", "nir08", "swir16", "swir22", "lwir",
];
/// Каналы, которые берутся, только если они есть у сцены.
pub const OPTIONAL_BANDS: [&str; 1] = ["coastal"];
/// У Sentinel-2 «Все каналы» - сторона участка до ALL_PIXELS пикселей,
/// дальше шаг 20 м.
pub const ALL_PIXELS: f64 = 2000.0;

/// Полоса насыщения MSS - до STRIPE_ROWS строк по столбцу.
pub const STRIPE_ROWS: usize = 3;
/// Всплеск одного канала MSS: отражение выше среднего соседей сверху
/// и снизу на SPIKE, а у остальных каналов отклонение меньше доли
/// SPIKE_OTHERS от него.
pub const SPIKE: f32 = 0.05;
pub const SPIKE_OTHERS: f32 = 0.3;
/// Зона вдоль полосы: строк и столбцов от отмеченного пикселя.
pub const STRIPE_ZONE: (usize, usize) = (2, 4);
/// Отношение канала к остальным дальше этой доли от соседей - сбой.
pub const STRIPE_RATIO: f64 = 0.2;
/// Строк до пикселя сравнения, за полосой.
pub const STRIPE_REF: usize = 3;

/// Сторона участка точки, км - ширина видимой полосы глобуса в этих
/// пределах.
pub const SIDE_MIN: f64 = 1.0;
pub const SIDE_MAX: f64 = 100.0;
/// Сцена в списке видна, если данные покрывают участок не меньше этой
/// доли, %. У ASTER - почти любое касание.
pub const MIN_VALID: f64 = 50.0;
pub const MIN_VALID_ASTER: f64 = 1.0;

/// Средний радиус Земли квадрата участка, м.
const MEAN_RADIUS: f64 = 6371008.8;
/// Ячеек маски участка не больше: враждебная рамка не занимает память.
pub const MAX_CELLS: usize = 1 << 28;

/// Спутник окна снимков.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Mission {
    Sentinel2,
    Landsat,
    Mss,
    Aster,
    AsterNasa,
    ModisLst1,
    ModisLst8,
    ModisVeg,
    ModisSnow1,
    ModisSnow8,
    ModisBurn,
}

/// Спутники по порядку окна модуля.
pub const MISSIONS: [Mission; 11] = [
    Mission::Sentinel2,
    Mission::Landsat,
    Mission::Mss,
    Mission::Aster,
    Mission::AsterNasa,
    Mission::ModisLst1,
    Mission::ModisLst8,
    Mission::ModisVeg,
    Mission::ModisSnow1,
    Mission::ModisSnow8,
    Mission::ModisBurn,
];

impl Mission {
    /// Имя спутника, как ключ модуля.
    pub fn name(self) -> &'static str {
        match self {
            Self::Sentinel2 => "sentinel2",
            Self::Landsat => "landsat",
            Self::Mss => "mss",
            Self::Aster => "aster",
            Self::AsterNasa => "aster_nasa",
            Self::ModisLst1 => "modis_lst1",
            Self::ModisLst8 => "modis_lst8",
            Self::ModisVeg => "modis_veg",
            Self::ModisSnow1 => "modis_snow1",
            Self::ModisSnow8 => "modis_snow8",
            Self::ModisBurn => "modis_burn",
        }
    }

    /// Спутник по имени или None.
    pub fn from_name(name: &str) -> Option<Self> {
        MISSIONS.into_iter().find(|m| m.name() == name)
    }
}

/// Ошибка расчёта.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Точка вне широт UTM.
    OutsideUtm,
    /// EPSG сцены - не UTM WGS84 (326zz или 327zz).
    NotUtm,
    /// У сцены нет высоты солнца.
    NoSun,
    /// Солнце под горизонтом.
    SunBelowHorizon,
    /// Неизвестный канал.
    UnknownBand,
}

/// Двумерный массив, строки сверху вниз.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid<T> {
    rows: usize,
    cols: usize,
    data: Vec<T>,
}

impl<T: Copy> Grid<T> {
    /// Массив, заполненный value. Слишком большой размер - пустой массив
    /// 0 × 0.
    pub fn filled(rows: usize, cols: usize, value: T) -> Self {
        match rows.checked_mul(cols) {
            Some(n) => Self {
                rows,
                cols,
                data: vec![value; n],
            },
            None => Self {
                rows: 0,
                cols: 0,
                data: Vec::new(),
            },
        }
    }

    /// Массив из значений по строкам или None, если их не rows × cols.
    pub fn from_vec(rows: usize, cols: usize, data: Vec<T>) -> Option<Self> {
        (rows.checked_mul(cols)? == data.len()).then_some(Self { rows, cols, data })
    }

    /// Массив из строк одной длины или None.
    pub fn from_rows(list: &[Vec<T>]) -> Option<Self> {
        let cols = list.first().map_or(0, Vec::len);
        if list.iter().any(|r| r.len() != cols) {
            return None;
        }
        let data = list.iter().flatten().copied().collect();
        Self::from_vec(list.len(), cols, data)
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    /// (строки, столбцы).
    pub fn shape(&self) -> (usize, usize) {
        (self.rows, self.cols)
    }

    /// Значения по строкам.
    pub fn data(&self) -> &[T] {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut [T] {
        &mut self.data
    }

    /// Значение пикселя или None вне массива.
    pub fn get(&self, row: usize, col: usize) -> Option<T> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        self.data.get(row * self.cols + col).copied()
    }

    /// Записать пиксель. false вне массива.
    pub fn set(&mut self, row: usize, col: usize, value: T) -> bool {
        if row >= self.rows || col >= self.cols {
            return false;
        }
        match self.data.get_mut(row * self.cols + col) {
            Some(cell) => {
                *cell = value;
                true
            }
            None => false,
        }
    }

    /// Новый массив того же размера.
    pub fn map<U: Copy>(&self, f: impl Fn(T) -> U) -> Grid<U> {
        Grid {
            rows: self.rows,
            cols: self.cols,
            data: self.data.iter().map(|&v| f(v)).collect(),
        }
    }

    /// Попиксельно с другим массивом, None при разных размерах.
    pub fn zip_map<U: Copy, V: Copy>(
        &self,
        other: &Grid<U>,
        f: impl Fn(T, U) -> V,
    ) -> Option<Grid<V>> {
        if self.shape() != other.shape() {
            return None;
        }
        let data = self
            .data
            .iter()
            .zip(&other.data)
            .map(|(&a, &b)| f(a, b))
            .collect();
        Some(Grid {
            rows: self.rows,
            cols: self.cols,
            data,
        })
    }
}

impl Grid<bool> {
    /// Количество истинных пикселей.
    pub fn count(&self) -> usize {
        self.data.iter().filter(|v| **v).count()
    }

    /// Есть ли истинный пиксель.
    pub fn any(&self) -> bool {
        self.data.iter().any(|v| *v)
    }
}

/// Сырые значения канала.
pub type Raw = Grid<f64>;
/// Каналы сцены {ключ: отражение}.
pub type Bands = BTreeMap<String, Grid<f32>>;

/// Продукт: один канал (индекс, температура) или набор (сочетание,
/// все каналы).
#[derive(Debug, Clone, PartialEq)]
pub enum Product {
    Single(Grid<f32>),
    Stack(Vec<Grid<f32>>),
}

/// Адрес файла канала или адреса тайлов одного дня у MODIS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Href {
    One(String),
    Tiles(Vec<String>),
}

impl Href {
    /// Адрес или первый из тайлов.
    pub fn first(&self) -> Option<&str> {
        match self {
            Self::One(h) => Some(h),
            Self::Tiles(list) => list.first().map(String::as_str),
        }
    }

    /// Одиночный адрес или None у тайлов.
    pub fn one(&self) -> Option<&str> {
        match self {
            Self::One(h) => Some(h),
            Self::Tiles(_) => None,
        }
    }
}

/// Канал сцены: адрес, масштаб и сдвиг значения, нет данных, шаг, м.
#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub href: Href,
    pub scale: f64,
    pub offset: f64,
    pub nodata: Option<f64>,
    pub res: Option<f64>,
}

impl Asset {
    /// Канал с одним адресом.
    pub fn new(href: &str, scale: f64, offset: f64, nodata: Option<f64>, res: Option<f64>) -> Self {
        Self {
            href: Href::One(href.to_string()),
            scale,
            offset,
            nodata,
            res,
        }
    }
}

/// Каналы сцены {ключ: Asset} в порядке каталога.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Assets(Vec<(String, Asset)>);

impl Assets {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn get(&self, key: &str) -> Option<&Asset> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, a)| a)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Записать канал: прежний ключ остаётся на своём месте.
    pub fn insert(&mut self, key: &str, asset: Asset) {
        match self.0.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = asset,
            None => self.0.push((key.to_string(), asset)),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Asset)> {
        self.0.iter().map(|(k, a)| (k.as_str(), a))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&str, &mut Asset)> {
        self.0.iter_mut().map(|(k, a)| (k.as_str(), a))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Сцена: номер, время UTC «ГГГГ-ММ-ДД ЧЧ:ММ», облачность сцены
/// в процентах, тайл MGRS или спутник с витком и рядом WRS у Landsat,
/// EPSG системы UTM, высота солнца в градусах, адрес превью, каналы,
/// спутник.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub id: String,
    pub when: String,
    pub cloud: f64,
    pub tile: String,
    pub epsg: Option<u32>,
    pub sun: Option<f64>,
    pub thumbnail: String,
    pub assets: Assets,
    pub mission: Mission,
}

/// Канал слоя-набора всей сцены: канал, ключ файла, номер канала
/// в файле с 1, подпись.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WholeBand {
    pub key: String,
    pub file: String,
    pub index: usize,
    pub label: String,
}

// Значения JSON, как их читает Python.

/// Число поля: число JSON или строка с числом, как float() Python.
pub(crate) fn number(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// Целое поля, как int() Python: дробное число отбрасывает дробь,
/// строка - только целая запись.
pub(crate) fn integer(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_i64().or_else(|| {
            n.as_f64()
                .filter(|f| f.is_finite() && f.abs() < 9.0e18)
                .map(|f| f.trunc() as i64)
        }),
        Value::String(s) => s.trim().parse().ok(),
        Value::Bool(b) => Some(i64::from(*b)),
        _ => None,
    }
}

/// Истинность значения, как bool() Python.
pub(crate) fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// Строка значения, как str() Python.
pub(crate) fn py_str(v: &Value) -> String {
    match v {
        Value::Null => "None".to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::String(s) => s.clone(),
        Value::Number(n) => {
            if n.is_f64() {
                match n.as_f64() {
                    Some(f) if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e16 => {
                        format!("{f:.1}")
                    }
                    Some(f) => format!("{f}"),
                    None => n.to_string(),
                }
            } else {
                n.to_string()
            }
        }
        other => other.to_string(),
    }
}

/// str(x or "") Python.
pub(crate) fn text(v: Option<&Value>) -> String {
    match v {
        Some(value) if truthy(Some(value)) => py_str(value),
        _ => String::new(),
    }
}

/// Объект поля или None.
pub(crate) fn object(v: Option<&Value>) -> Option<&Map<String, Value>> {
    v.and_then(Value::as_object)
}

/// Список поля или пустой.
pub(crate) fn list(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// Первые n знаков строки.
pub(crate) fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Время сцены «ГГГГ-ММ-ДД ЧЧ:ММ» из поля datetime.
fn when_of(v: Option<&Value>) -> String {
    head(&text(v).replace('T', " "), 16)
}

/// EPSG поля, как `int(epsg) if epsg else None`.
fn epsg_of(v: Option<&Value>) -> Option<u32> {
    if !truthy(v) {
        return None;
    }
    integer(v)
        .and_then(|e| u32::try_from(e).ok())
        .filter(|e| *e != 0)
}

/// Облачность поля, без неё - 100 %.
fn cloud_of(v: Option<&Value>) -> f64 {
    match v {
        None | Some(Value::Null) => 100.0,
        value => number(value).unwrap_or(100.0),
    }
}

/// Поле в первом элементе raster:bands.
fn raster_band(value: &Value) -> Option<&Map<String, Value>> {
    list(value.get("raster:bands"))
        .first()
        .and_then(Value::as_object)
}

// Спутники.

/// Продукты спутника: у Landsat ещё температура поверхности, у MODIS
/// и ASTER свои.
pub fn products_of(mission: Mission) -> Vec<&'static str> {
    if modis::is_modis(mission) {
        return modis::products_of(mission).to_vec();
    }
    if aster::is_aster(mission) {
        return aster::PRODUCTS.to_vec();
    }
    match mission {
        Mission::Mss => MSS_PRODUCTS.to_vec(),
        Mission::Landsat => LANDSAT_PRODUCTS
            .iter()
            .copied()
            .chain(["all_bands"])
            .collect(),
        _ => PRODUCTS.iter().copied().chain(["all_bands"]).collect(),
    }
}

/// Адрес поиска спутника.
pub fn search_url(mission: Mission) -> &'static str {
    match mission {
        Mission::Sentinel2 => SEARCH,
        Mission::AsterNasa => aster::NASA_SEARCH,
        _ => LANDSAT_SEARCH,
    }
}

/// Коллекция каталога спутника.
pub fn collection(mission: Mission) -> &'static str {
    match mission {
        Mission::Sentinel2 => COLLECTION,
        Mission::Landsat => LANDSAT_COLLECTION,
        Mission::Mss => MSS_COLLECTION,
        Mission::Aster => aster::COLLECTION,
        Mission::AsterNasa => aster::NASA_COLLECTION,
        m => modis::collection(m).unwrap_or(""),
    }
}

/// Маска облаков сцены: ключ ресурса и шаг чтения, м. У ASTER L1T маски
/// нет - доля данных по каналу 1.
pub fn cloud_asset(mission: Mission) -> (&'static str, f64) {
    match mission {
        Mission::Sentinel2 => ("scl", 20.0),
        Mission::Landsat => ("qa_pixel", 30.0),
        Mission::Mss => ("qa_pixel", 60.0),
        Mission::Aster | Mission::AsterNasa => ("VNIR", 90.0),
        m => (
            modis::quality_asset(m).unwrap_or(""),
            modis::step(m).unwrap_or(1000.0),
        ),
    }
}

/// Каналы «Всех каналов» спутника по порядку номеров. У ASTER и MODIS
/// пусто.
pub fn all_bands(mission: Mission) -> &'static [&'static str] {
    match mission {
        Mission::Sentinel2 => &S2_ALL,
        Mission::Landsat => &LANDSAT_ALL,
        Mission::Mss => &MSS_BANDS,
        _ => &[],
    }
}

/// Каналы естественных или ложных (MSS, ASTER) цветов слоя «Все каналы».
pub fn all_shown(mission: Mission) -> Option<[&'static str; 3]> {
    match mission {
        Mission::Sentinel2 | Mission::Landsat => Some(["red", "green", "blue"]),
        Mission::Mss => Some(["nir08", "red", "green"]),
        Mission::Aster | Mission::AsterNasa => Some(["b3n", "b2", "b1"]),
        _ => None,
    }
}

/// Сочетание по имени.
pub fn composite(name: &str) -> Option<[&'static str; 3]> {
    COMPOSITES.iter().find(|(n, _)| *n == name).map(|(_, b)| *b)
}

/// Индекс по имени.
pub fn index(name: &str) -> Option<&'static Index> {
    INDICES.iter().find(|i| i.name == name)
}

/// K1, K2 теплового канала Landsat по номеру спутника.
pub fn thermal_k(number: &str) -> Option<(f64, f64)> {
    THERMAL_K
        .iter()
        .find(|(n, _)| *n == number)
        .map(|(_, k)| *k)
}

/// Каналы файла «Все каналы» сцены по порядку номеров.
pub fn all_keys(scene: &Scene) -> Vec<String> {
    if aster::is_aster(scene.mission) {
        return aster::ORDER
            .iter()
            .filter(|k| aster::has(k, &scene.assets))
            .map(|k| k.to_string())
            .collect();
    }
    all_bands(scene.mission)
        .iter()
        .filter(|k| scene.assets.contains(k) || !OPTIONAL_BANDS.contains(k))
        .map(|k| k.to_string())
        .collect()
}

/// Адрес (или первый из тайлов) ведёт на GeoTIFF.
fn is_tif(href: &Href) -> bool {
    let first = href.first().unwrap_or("");
    let path = first.split('?').next().unwrap_or("").to_lowercase();
    path.ends_with(".tif") || path.ends_with(".tiff")
}

/// Файлы режима «Вся сцена, исходные файлы» - ключи ресурсов сцены:
/// каналы по порядку номеров, маска облаков, у Landsat ещё маска
/// насыщения и mtl.json. У ASTER Planetary Computer - файлы групп VNIR,
/// SWIR, TIR. У MODIS - все файлы GeoTIFF сцены.
pub fn whole_files(scene: &Scene) -> Vec<String> {
    if modis::is_modis(scene.mission) {
        let mut keys: Vec<String> = scene
            .assets
            .iter()
            .filter(|(_, a)| is_tif(&a.href))
            .map(|(k, _)| k.to_string())
            .collect();
        keys.sort();
        return keys;
    }
    if scene.mission == Mission::Aster {
        return ["VNIR", "SWIR", "TIR"]
            .iter()
            .filter(|f| scene.assets.contains(f))
            .map(|f| f.to_string())
            .collect();
    }
    let mut keys: Vec<String> = all_keys(scene)
        .into_iter()
        .filter(|k| scene.assets.contains(k))
        .collect();
    if aster::is_aster(scene.mission) {
        return keys;
    }
    for extra in [cloud_asset(scene.mission).0, "qa_radsat", "mtl.json"] {
        if scene.assets.contains(extra) && !keys.iter().any(|k| k == extra) {
            keys.push(extra.to_string());
        }
    }
    keys
}

/// Каналы слоя-набора всей сцены по порядку номеров. У ASTER Planetary
/// Computer в файле группы несколько каналов. У MODIS - слои продуктов
/// и канал качества.
pub fn whole_bands(scene: &Scene) -> Vec<WholeBand> {
    let mut out: Vec<WholeBand> = Vec::new();
    if modis::is_modis(scene.mission) {
        let mut keys: Vec<&str> = modis::products_of(scene.mission)
            .iter()
            .filter_map(|p| modis::product(p).map(|s| s.key))
            .collect();
        if let Some(q) = modis::quality_asset(scene.mission) {
            keys.push(q);
        }
        for key in keys {
            if scene.assets.contains(key) && out.iter().all(|b| b.key != key) {
                out.push(WholeBand {
                    key: key.to_string(),
                    file: key.to_string(),
                    index: 1,
                    label: key.to_string(),
                });
            }
        }
        return out;
    }
    for key in all_keys(scene) {
        if scene.mission == Mission::Aster {
            let Some(b) = aster::band_of(&key) else {
                continue;
            };
            if !scene.assets.contains(b.file) {
                continue;
            }
            out.push(WholeBand {
                label: band_label(scene, &key),
                key,
                file: b.file.to_string(),
                index: b.index,
            });
        } else if scene.assets.contains(&key) {
            out.push(WholeBand {
                label: band_label(scene, &key),
                file: key.clone(),
                key,
                index: 1,
            });
        }
    }
    out
}

/// Окончание «.tif» без учёта регистра или None.
fn strip_tif(href: &str) -> Option<&str> {
    let cut = href.len().checked_sub(4)?;
    let (rest, tail) = (href.get(..cut)?, href.get(cut..)?);
    tail.eq_ignore_ascii_case(".tif").then_some(rest)
}

/// Номер канала из имени файла «…B8A.TIF», «…B04.tif» - «B8A», «B04»
/// заглавными, как BAND_FILE модуля.
fn band_file(href: &str) -> Option<String> {
    let rest = strip_tif(href)?;
    let bytes = rest.as_bytes();
    let mut end = bytes.len();
    let letter = end > 0 && bytes[end - 1].eq_ignore_ascii_case(&b'a');
    if letter {
        end -= 1;
    }
    let digits = bytes[..end]
        .iter()
        .rev()
        .take_while(|b| b.is_ascii_digit())
        .count();
    if digits == 0 || end < digits + 1 {
        return None;
    }
    let start = end - digits - 1;
    if !bytes[start].eq_ignore_ascii_case(&b'b') {
        return None;
    }
    rest.get(start..).map(str::to_uppercase)
}

/// Номер канала из имени файла «…_B5.TIF», как MTL_BAND модуля.
fn mtl_band(href: &str) -> Option<&str> {
    let rest = strip_tif(href)?;
    let bytes = rest.as_bytes();
    let digits = bytes
        .iter()
        .rev()
        .take_while(|b| b.is_ascii_digit())
        .count();
    let end = bytes.len();
    if digits == 0 || end < digits + 2 {
        return None;
    }
    let b = bytes[end - digits - 1];
    let under = bytes[end - digits - 2];
    (b.eq_ignore_ascii_case(&b'b') && under == b'_')
        .then(|| rest.get(end - digits..))
        .flatten()
}

/// Подпись канала в файле: номер из имени файла и ключ каталога,
/// «B8A nir08», у теплового - с единицей «B10 lwir, K».
pub fn band_label(scene: &Scene, key: &str) -> String {
    if aster::is_aster(scene.mission) {
        return aster::label(key);
    }
    let mut number = scene
        .assets
        .get(key)
        .and_then(|a| a.href.one())
        .and_then(band_file)
        .unwrap_or_default();
    if let Some(rest) = number.strip_prefix("B0") {
        number = format!("B{rest}");
    }
    let label = format!("{number} {key}").trim().to_string();
    if key == "lwir" { label + ", K" } else { label }
}

/// Файлы спутника читаются по ключу SAS Planetary Computer.
pub fn needs_key(mission: Mission) -> bool {
    LANDSATS.contains(&mission) || modis::is_modis(mission) || mission == Mission::Aster
}

/// Адрес службы ключа SAS. По адресу файла href - ключ его хранилища
/// Azure (учётная запись и контейнер): ключ коллекции MODIS к файлам
/// modiseuwest не подходит. Без href - ключ коллекции спутника.
pub fn sas_url(mission: Mission, href: Option<&str>) -> String {
    if let Some(href) = href.filter(|h| !h.is_empty()) {
        let after = href.split_once("://").map_or("", |(_, rest)| rest);
        let (host, path) = after.split_once('/').unwrap_or((after, ""));
        let account = host.split('.').next().unwrap_or("");
        let container = path.split('/').next().unwrap_or("");
        if host.ends_with(".blob.core.windows.net") && !container.is_empty() {
            return format!("{SAS_URL}{account}/{container}");
        }
    }
    format!("{SAS_URL}{}", collection(mission))
}

/// Ключ SAS из ответа службы ключей: поле token, иначе None.
pub fn parse_token(data: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(data).ok()?;
    let token = text(value.get("token"));
    (!token.is_empty()).then_some(token)
}

/// Ключ ресурса канала у спутника: ближний ИК Landsat - nir08.
pub fn band(key: &str, mission: Mission) -> &str {
    if LANDSATS.contains(&mission) && key == "nir" {
        "nir08"
    } else {
        key
    }
}

// Каталог.

/// Тело запроса поиска: geometry - GeoJSON, start и end - дни
/// «ГГГГ-ММ-ДД», max_cloud - облачность сцены по оценке поставщика.
pub fn search_body(
    geometry: &Value,
    start: &str,
    end: &str,
    max_cloud: f64,
    limit: usize,
    mission: Mission,
) -> Value {
    let mut body = json!({
        "collections": [collection(mission)],
        "intersects": geometry,
        "datetime": format!("{start}T00:00:00Z/{end}T23:59:59Z"),
        "limit": limit,
        "query": {"eo:cloud_cover": {"lte": max_cloud}},
        "sortby": [{"field": "properties.datetime", "direction": "desc"}],
    });
    if modis::is_modis(mission) {
        // У MODIS облачности сцены в каталоге нет.
        if let Some(map) = body.as_object_mut() {
            map.remove("query");
        }
    }
    body
}

/// Сцены страницы ответа и тело запроса следующей или None.
pub fn parse_page(data: &Value) -> (Vec<Scene>, Option<Value>) {
    let scenes = list(data.get("features"))
        .iter()
        .filter_map(parse_item)
        .collect();
    let mut following = None;
    for link in list(data.get("links")) {
        if link.get("rel").and_then(Value::as_str) == Some("next")
            && let Some(body) = link.get("body").filter(|b| b.is_object())
        {
            following = Some(body.clone());
        }
    }
    (scenes, following)
}

/// Сцена из элемента STAC или None, если нет каналов.
pub fn parse_item(feature: &Value) -> Option<Scene> {
    let feature = feature.as_object()?;
    let empty = Map::new();
    let props = object(feature.get("properties")).unwrap_or(&empty);
    let coll = feature
        .get("collection")
        .and_then(Value::as_str)
        .unwrap_or("");
    if let Some(mission) = modis::mission_of(coll) {
        return modis_item(feature, props, mission);
    }
    if coll == aster::COLLECTION {
        return aster_item(feature, props);
    }
    if text(props.get("platform")).starts_with("landsat") {
        let mission = if coll == MSS_COLLECTION {
            Mission::Mss
        } else {
            Mission::Landsat
        };
        return landsat_item(feature, props, mission);
    }
    // Смещение -1000 уже вычтено из значений, а поле offset у каналов
    // осталось.
    let applied = truthy(props.get("earthsearch:boa_offset_applied"));
    let mut assets = Assets::new();
    for (key, value) in object(feature.get("assets")).into_iter().flatten() {
        let href = text(value.get("href"));
        if !href.starts_with("http") {
            continue;
        }
        let info = raster_band(value);
        let field = |name: &str| info.and_then(|b| b.get(name));
        let offset = if applied {
            0.0
        } else {
            number(field("offset")).unwrap_or(0.0)
        };
        let asset = Asset::new(
            &href,
            number(field("scale")).unwrap_or(1.0),
            offset,
            number(field("nodata")),
            number(field("spatial_resolution")),
        );
        assets.insert(key, asset);
    }
    if !assets.contains("red") || !assets.contains("scl") {
        return None;
    }
    let epsg = match props.get("proj:epsg") {
        None | Some(Value::Null) => epsg_from_tile(props),
        value => epsg_of(value),
    };
    let thumbnail = assets
        .get("thumbnail")
        .and_then(|a| a.href.first())
        .unwrap_or("")
        .to_string();
    Some(Scene {
        id: text(feature.get("id")),
        when: when_of(props.get("datetime")),
        cloud: cloud_of(props.get("eo:cloud_cover")),
        tile: tile_of(feature, props),
        epsg,
        sun: number(props.get("view:sun_elevation")),
        thumbnail,
        assets,
        mission: Mission::Sentinel2,
    })
}

/// Сцена Landsat Planetary Computer: каналы с масштабом и сдвигом
/// из каталога, превью - картинка службы данных.
fn landsat_item(
    feature: &Map<String, Value>,
    props: &Map<String, Value>,
    mission: Mission,
) -> Option<Scene> {
    let mut assets = Assets::new();
    let mut preview = String::new();
    for (key, value) in object(feature.get("assets")).into_iter().flatten() {
        let href = text(value.get("href"));
        if !href.starts_with("https://") {
            continue;
        }
        if key == "rendered_preview" {
            preview = href;
            continue;
        }
        let info = raster_band(value);
        let field = |name: &str| info.and_then(|b| b.get(name));
        // `or 1.0` модуля: нулевой масштаб - 1.
        let scale = number(field("scale")).filter(|s| *s != 0.0).unwrap_or(1.0);
        let asset = Asset::new(
            &href,
            scale,
            number(field("offset")).unwrap_or(0.0),
            number(field("nodata")),
            number(field("spatial_resolution")),
        );
        assets.insert(key, asset);
    }
    if !assets.contains("red") || !assets.contains("qa_pixel") {
        return None;
    }
    if !assets.contains("lwir")
        && let Some(thermal) = assets.get("lwir11").cloned()
    {
        assets.insert("lwir", thermal);
    }
    let platform = text(props.get("platform"));
    let sat = platform.rsplit('-').next().unwrap_or("");
    let part = |name: &str| props.get(name).map(py_str).unwrap_or_default();
    let tile = format!(
        "L{sat} {}/{}",
        part("landsat:wrs_path"),
        part("landsat:wrs_row")
    );
    Some(Scene {
        id: text(feature.get("id")),
        when: when_of(props.get("datetime")),
        cloud: cloud_of(props.get("eo:cloud_cover")),
        tile,
        epsg: epsg_of(props.get("proj:epsg")),
        sun: number(props.get("view:sun_elevation")),
        thumbnail: preview,
        assets,
        mission,
    })
}

/// Сцена ASTER L1T: файлы VNIR, SWIR и TIR, превью - картинка службы
/// данных, тайл - «ASTER».
fn aster_item(feature: &Map<String, Value>, props: &Map<String, Value>) -> Option<Scene> {
    let mut assets = Assets::new();
    let mut preview = String::new();
    for (key, value) in object(feature.get("assets")).into_iter().flatten() {
        let href = text(value.get("href"));
        if !href.starts_with("https://") {
            continue;
        }
        if key == "rendered_preview" {
            preview = href;
        } else if ["VNIR", "SWIR", "TIR"].contains(&key.as_str()) {
            assets.insert(key, Asset::new(&href, 1.0, 0.0, Some(0.0), None));
        }
    }
    if !assets.contains("VNIR") {
        return None;
    }
    let tile = if assets.contains("SWIR") {
        "ASTER"
    } else {
        "ASTER VNIR"
    };
    Some(Scene {
        id: text(feature.get("id")),
        when: when_of(props.get("datetime")),
        cloud: cloud_of(props.get("eo:cloud_cover")),
        tile: tile.to_string(),
        epsg: epsg_of(props.get("proj:epsg")),
        sun: number(props.get("view:sun_elevation")),
        thumbnail: preview,
        assets,
        mission: Mission::Aster,
    })
}

/// Сцены ASTER NASA из ответа CMR ([`aster::parse_cmr`]): каналы -
/// отдельные файлы под ключами каналов, тайл - «ASTER» или «ASTER VNIR
/// TIR» без коротковолновых.
pub fn cmr_scenes(data: &Value) -> Vec<Scene> {
    aster::parse_cmr(data)
        .into_iter()
        .map(|item| {
            let mut assets = Assets::new();
            for (k, h) in &item.files {
                assets.insert(k, Asset::new(h, 1.0, 0.0, Some(0.0), None));
            }
            let tile = if assets.contains("b4") {
                "ASTER"
            } else {
                "ASTER VNIR TIR"
            };
            Scene {
                id: item.id,
                when: item.when,
                cloud: item.cloud,
                tile: tile.to_string(),
                epsg: item.epsg,
                sun: Some(item.sun),
                thumbnail: item.preview,
                assets,
                mission: Mission::AsterNasa,
            }
        })
        .collect()
}

/// Сцена MODIS: день или начало периода, спутник и тайл, каналы -
/// списки адресов ([`modis::merge_tiles`] сводит тайлы одного дня).
fn modis_item(
    feature: &Map<String, Value>,
    props: &Map<String, Value>,
    mission: Mission,
) -> Option<Scene> {
    let mut assets = Assets::new();
    for (key, value) in object(feature.get("assets")).into_iter().flatten() {
        let href = text(value.get("href"));
        let kind = value
            .get("type")
            .map(py_str)
            .unwrap_or_else(|| "None".to_string());
        if href.starts_with("https://") && kind.contains("tiff") {
            let asset = Asset {
                href: Href::Tiles(vec![href]),
                scale: 1.0,
                offset: 0.0,
                nodata: None,
                res: None,
            };
            assets.insert(key, asset);
        }
    }
    if !assets.contains(modis::quality_asset(mission)?) {
        return None;
    }
    let id = text(feature.get("id"));
    let when = head(&text(props.get("start_datetime")), 10);
    let tile = format!("{} {}", modis::platform(&id), modis::tile_of(props));
    Some(Scene {
        id,
        when,
        cloud: 0.0,
        tile,
        epsg: None,
        sun: None,
        thumbnail: String::new(),
        assets,
        mission,
    })
}

/// Тайл MGRS из полей mgrs или из номера сцены.
fn tile_of(feature: &Map<String, Value>, props: &Map<String, Value>) -> String {
    let (zone, band, square) = (
        props.get("mgrs:utm_zone"),
        props.get("mgrs:latitude_band"),
        props.get("mgrs:grid_square"),
    );
    if truthy(zone)
        && truthy(band)
        && truthy(square)
        && let Some(z) = integer(zone)
    {
        return format!("{z:02}{}{}", text(band), text(square));
    }
    let id = text(feature.get("id"));
    id.split('_').nth(1).unwrap_or("").to_string()
}

/// EPSG по зоне и полосе MGRS.
fn epsg_from_tile(props: &Map<String, Value>) -> Option<u32> {
    let (zone, band) = (props.get("mgrs:utm_zone"), props.get("mgrs:latitude_band"));
    if !truthy(zone) || !truthy(band) {
        return None;
    }
    let base = if text(band).to_uppercase().as_str() >= "N" {
        32600
    } else {
        32700
    };
    u32::try_from(base + integer(zone)?)
        .ok()
        .filter(|e| *e != 0)
}

/// Адрес файла Planetary Computer с ключом SAS.
pub fn sign_one(href: &str, sas: &str) -> String {
    if sas.is_empty() {
        return href.to_string();
    }
    let joint = if href.contains('?') { '&' } else { '?' };
    format!("{href}{joint}{sas}")
}

/// Адрес с ключом SAS, у тайлов - каждый.
pub fn sign(href: &Href, sas: &str) -> Href {
    match href {
        Href::One(h) => Href::One(sign_one(h, sas)),
        Href::Tiles(list) => Href::Tiles(list.iter().map(|h| sign_one(h, sas)).collect()),
    }
}

/// Заголовок запроса файлов ASTER NASA с ключом Earthdata пользователя.
pub fn earthdata_header(token: &str) -> (&'static str, String) {
    ("Authorization", format!("Bearer {token}"))
}

// Landsat MSS.

/// Бит насыщения канала в qa_radsat: номер канала из имени файла минус
/// один (B4 у Landsat 1-3 - бит 3, B1 у Landsat 4-5 - бит 0).
pub fn radsat_bit(asset: &Asset) -> Option<u32> {
    let digits = mtl_band(asset.href.one()?)?;
    digits.parse::<u32>().ok()?.checked_sub(1)
}

/// Бит bit сырого значения.
fn bit_set(value: f64, bit: u32) -> bool {
    (value as i64).checked_shr(bit).unwrap_or(0) & 1 != 0
}

/// Насыщенные пиксели канала - среднее соседних целых пикселей окна
/// 3 × 3. Полосы на снимках MSS - строки сбойных насыщенных пикселей
/// шириной 1-2 пикселя. Пиксель без целых соседей остаётся. Возвращает
/// (массив, заменено), None при разных размерах.
pub fn fill_saturated(values: &Grid<f32>, radsat: &Raw, bit: u32) -> Option<(Grid<f32>, usize)> {
    let flagged = radsat.map(|v| bit_set(v, bit));
    fill_mask(values, &stripe_rows(&flagged), STRIPE_ROWS + 1)
}

/// Маска полос: отмеченные пиксели в отрезках столбца не длиннее
/// STRIPE_ROWS и по пикселю над и под таким отрезком.
pub fn stripe_rows(flagged: &Grid<bool>) -> Grid<bool> {
    let (rows, cols) = flagged.shape();
    let at = |r: usize, c: usize| flagged.data[r * cols + c];
    let mut up = vec![0usize; rows * cols];
    let mut down = vec![0usize; rows * cols];
    for r in 0..rows {
        for c in 0..cols {
            let prev = if r > 0 { up[(r - 1) * cols + c] } else { 0 };
            up[r * cols + c] = if at(r, c) { prev + 1 } else { 0 };
        }
    }
    for r in (0..rows).rev() {
        for c in 0..cols {
            let prev = if r + 1 < rows {
                down[(r + 1) * cols + c]
            } else {
                0
            };
            down[r * cols + c] = if at(r, c) { prev + 1 } else { 0 };
        }
    }
    let thin: Vec<bool> = (0..rows * cols)
        .map(|i| flagged.data[i] && up[i] + down[i] <= STRIPE_ROWS + 1)
        .collect();
    let mut out = thin.clone();
    for r in 0..rows {
        for c in 0..cols {
            let i = r * cols + c;
            if (r > 0 && thin[i - cols]) || (r + 1 < rows && thin[i + cols]) {
                out[i] = true;
            }
        }
    }
    Grid {
        rows,
        cols,
        data: out,
    }
}

/// NaN - 0, бесконечность - наибольшее число, как nan_to_num numpy.
fn nan_to_num32(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else if v.is_infinite() {
        if v > 0.0 { f32::MAX } else { f32::MIN }
    } else {
        v
    }
}

fn nan_to_num64(v: f64) -> f64 {
    if v.is_nan() {
        0.0
    } else if v.is_infinite() {
        if v > 0.0 { f64::MAX } else { f64::MIN }
    } else {
        v
    }
}

/// Всплески каждого канала {ключ: маска} по соседям сверху и снизу, см.
/// [`SPIKE`]. Крайние строки не проверяются. None, если канала нет,
/// каналов меньше двух или размеры разные.
pub fn spikes(bands: &Bands, keys: &[&str]) -> Option<BTreeMap<String, Grid<bool>>> {
    if keys.len() < 2 {
        return None;
    }
    let mut dev: Vec<Grid<f32>> = Vec::new();
    let mut shape = None;
    for key in keys {
        let v = bands.get(*key)?;
        if *shape.get_or_insert(v.shape()) != v.shape() {
            return None;
        }
        let (rows, cols) = v.shape();
        let mut d = Grid::filled(rows, cols, 0.0f32);
        for r in 1..rows.saturating_sub(1) {
            for c in 0..cols {
                let i = r * cols + c;
                let value = v.data[i] - (v.data[i - cols] + v.data[i + cols]) / 2.0;
                d.data[i] = nan_to_num32(value);
            }
        }
        dev.push(d);
    }
    let mut out = BTreeMap::new();
    for (n, key) in keys.iter().enumerate() {
        let own = &dev[n];
        let data = (0..own.data.len())
            .map(|i| {
                let others = dev
                    .iter()
                    .enumerate()
                    .filter(|(m, _)| *m != n)
                    .map(|(_, d)| d.data[i].abs())
                    .fold(f32::NEG_INFINITY, f32::max);
                own.data[i] > SPIKE && others < SPIKE_OTHERS * own.data[i]
            })
            .collect();
        out.insert(
            key.to_string(),
            Grid {
                rows: own.rows,
                cols: own.cols,
                data,
            },
        );
    }
    Some(out)
}

/// Полосы MSS: bands - отражения, radsat - qa_radsat, bits - бит
/// насыщения каждого канала. Насыщенные узкие полосы с соседними
/// строками, пиксели зоны полосы с отношением канала к остальным не как
/// у соседей, затем всплески одного канала заменяются соседями.
/// Возвращает новые каналы и количество заменённых пикселей, None при
/// разных размерах.
pub fn clean_mss(
    bands: &Bands,
    radsat: &Raw,
    bits: &BTreeMap<String, u32>,
) -> Option<(Bands, usize)> {
    let mut out = bands.clone();
    let shape = radsat.shape();
    if out.values().any(|g| g.shape() != shape) {
        return None;
    }
    let mut thin: BTreeMap<&str, Grid<bool>> = BTreeMap::new();
    for (key, bit) in bits {
        if out.contains_key(key) {
            thin.insert(key, stripe_rows(&radsat.map(|v| bit_set(v, *bit))));
        }
    }
    let mut stripe = Grid::filled(shape.0, shape.1, false);
    for m in thin.values() {
        for (s, t) in stripe.data.iter_mut().zip(&m.data) {
            *s |= *t;
        }
    }
    let zone = dilate(&stripe, STRIPE_ZONE.0, STRIPE_ZONE.1);
    let keys: Vec<&str> = MSS_BANDS
        .iter()
        .copied()
        .filter(|k| out.contains_key(*k))
        .collect();
    let mut done = 0;
    for key in &keys {
        let mut bad = thin
            .get(key)
            .cloned()
            .unwrap_or_else(|| Grid::filled(shape.0, shape.1, false));
        if keys.len() > 1 && zone.any() {
            let off = ratio_off(&out, key, &keys)?;
            for ((b, z), o) in bad.data.iter_mut().zip(&zone.data).zip(&off.data) {
                *b |= *z && *o;
            }
        }
        let values = out.get(*key)?;
        let (filled, count) = fill_mask(values, &bad, STRIPE_ROWS + STRIPE_ZONE.0 + 1)?;
        out.insert(key.to_string(), filled);
        done += count;
    }
    if keys.len() == MSS_BANDS.len() {
        for (key, bad) in spikes(&out, &MSS_BANDS)? {
            let values = out.get(&key)?;
            let (filled, count) = fill_mask(values, &bad, 1)?;
            out.insert(key, filled);
            done += count;
        }
    }
    Some((out, done))
}

/// Пиксели, где отношение канала key к среднему остальных отличается
/// больше STRIPE_RATIO в одну сторону и от пикселя на STRIPE_REF строк
/// выше, и от пикселя на столько же ниже.
fn ratio_off(bands: &Bands, key: &str, keys: &[&str]) -> Option<Grid<bool>> {
    let own = bands.get(key)?;
    let (rows, cols) = own.shape();
    let others: Vec<&Grid<f32>> = keys
        .iter()
        .filter(|k| **k != key)
        .map(|k| bands.get(*k))
        .collect::<Option<_>>()?;
    let count = others.len() as f32;
    let ratio: Vec<f64> = (0..own.data.len())
        .map(|i| {
            let mean = others.iter().map(|g| g.data[i]).sum::<f32>() / count;
            f64::from(own.data[i]) / f64::from(mean)
        })
        .collect();
    let n = STRIPE_REF;
    let mut off = Grid::filled(rows, cols, false);
    if rows <= 2 * n {
        return Some(off);
    }
    for r in n..rows - n {
        for c in 0..cols {
            let mid = ratio[r * cols + c];
            let up = mid / ratio[(r - n) * cols + c] - 1.0;
            let down = mid / ratio[(r + n) * cols + c] - 1.0;
            let same = nan_to_num64(up * down) > 0.0;
            off.data[r * cols + c] = same
                && nan_to_num64(up).abs() > STRIPE_RATIO
                && nan_to_num64(down).abs() > STRIPE_RATIO;
        }
    }
    Some(off)
}

/// Маска, расширенная на rows строк и cols столбцов.
fn dilate(mask: &Grid<bool>, rows: usize, cols: usize) -> Grid<bool> {
    let (h, w) = mask.shape();
    let mut out = mask.clone();
    for r in 0..h {
        for c in 0..w {
            if !mask.data[r * w + c] {
                continue;
            }
            for y in r.saturating_sub(rows)..(r + rows + 1).min(h) {
                for x in c.saturating_sub(cols)..(c + cols + 1).min(w) {
                    out.data[y * w + x] = true;
                }
            }
        }
    }
    out
}

/// Пиксели bad - среднее соседних целых пикселей окна 3 × 3, без целых
/// соседей пиксель остаётся до следующего прохода, заполнение идёт от
/// края маски внутрь не больше passes проходов. Возвращает (массив,
/// заменено), None при разных размерах.
pub fn fill_mask(
    values: &Grid<f32>,
    bad: &Grid<bool>,
    passes: usize,
) -> Option<(Grid<f32>, usize)> {
    let mut bad = bad.zip_map(values, |b, v| b && v.is_finite())?;
    let mut out = values.clone();
    let mut done = 0;
    for _ in 0..passes {
        if !bad.any() {
            break;
        }
        let fixed = fill_once(&mut out, &bad);
        let count = fixed.count();
        if count == 0 {
            break;
        }
        for (b, f) in bad.data.iter_mut().zip(&fixed.data) {
            *b &= !*f;
        }
        done += count;
    }
    Some((out, done))
}

/// Один проход fill_mask: заполненные пиксели.
fn fill_once(values: &mut Grid<f32>, bad: &Grid<bool>) -> Grid<bool> {
    let (rows, cols) = values.shape();
    let good: Vec<bool> = values
        .data
        .iter()
        .zip(&bad.data)
        .map(|(v, b)| !*b && v.is_finite())
        .collect();
    let mut fix = Grid::filled(rows, cols, false);
    let mut fill = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            if !bad.data[r * cols + c] {
                continue;
            }
            let (mut total, mut count) = (0.0f64, 0u32);
            for y in r.saturating_sub(1)..(r + 2).min(rows) {
                for x in c.saturating_sub(1)..(c + 2).min(cols) {
                    let j = y * cols + x;
                    if (y, x) != (r, c) && good[j] {
                        total += f64::from(values.data[j]);
                        count += 1;
                    }
                }
            }
            if count > 0 {
                fix.data[r * cols + c] = true;
                fill.push((r * cols + c, (total / f64::from(count)) as f32));
            }
        }
    }
    for (i, v) in fill {
        values.data[i] = v;
    }
    fix
}

/// Сцена MSS с масштабом каналов в отражение на верху атмосферы:
/// (REFLECTANCE_MULT × значение + REFLECTANCE_ADD) / sin(высоты солнца),
/// формула USGS. mtl - разобранный mtl.json сцены. Номер канала берётся
/// из имени файла (…_B5.TIF).
pub fn toa(scene: &Scene, mtl: &Value) -> Result<Scene, Error> {
    let meta = mtl.get("LANDSAT_METADATA_FILE").filter(|v| truthy(Some(v)));
    let rescale = meta.and_then(|m| m.get("LEVEL1_RADIOMETRIC_RESCALING"));
    let image = meta
        .and_then(|m| m.get("IMAGE_ATTRIBUTES"))
        .and_then(Value::as_object);
    let sun = match image.and_then(|i| i.get("SUN_ELEVATION")) {
        Some(v) => number(Some(v)),
        None => scene.sun,
    };
    let sun = sun.ok_or(Error::NoSun)?;
    let sine = sun.to_radians().sin();
    if sine.is_nan() || sine <= 0.0 {
        return Err(Error::SunBelowHorizon);
    }
    let mut out = scene.clone();
    for (_, asset) in out.assets.iter_mut() {
        let Some(digits) = asset.href.one().and_then(mtl_band) else {
            continue;
        };
        let field = |name: &str| number(rescale.and_then(|r| r.get(format!("{name}{digits}"))));
        let (Some(mult), Some(add)) = (
            field("REFLECTANCE_MULT_BAND_"),
            field("REFLECTANCE_ADD_BAND_"),
        ) else {
            continue;
        };
        asset.scale = mult / sine;
        asset.offset = add / sine;
    }
    Ok(out)
}

// Участок.

/// Сторона участка точки в км по ширине видимой полосы width метров,
/// с шагом 0.1 км.
pub fn side_for_view(width: f64) -> f64 {
    let side = (width / 1000.0).clamp(SIDE_MIN, SIDE_MAX);
    (side * 10.0).round() / 10.0
}

/// Квадрат со стороной side метров вокруг точки: кольцо (широта,
/// долгота) без повтора первой точки.
pub fn square(lat: f64, lon: f64, side: f64) -> Vec<(f64, f64)> {
    let half_lat = (side / 2.0 / MEAN_RADIUS).to_degrees();
    let half_lon = half_lat / lat.to_radians().cos().max(1e-6);
    vec![
        (lat - half_lat, lon - half_lon),
        (lat - half_lat, lon + half_lon),
        (lat + half_lat, lon + half_lon),
        (lat + half_lat, lon - half_lon),
    ]
}

/// Многоугольник GeoJSON из кольца (широта, долгота).
pub fn geojson(ring: &[(f64, f64)]) -> Value {
    let mut points: Vec<[f64; 2]> = ring.iter().map(|(lat, lon)| [*lon, *lat]).collect();
    if let (Some(first), Some(last)) = (points.first().copied(), points.last().copied())
        && first != last
    {
        points.push(first);
    }
    json!({"type": "Polygon", "coordinates": [points]})
}

/// Поперечный Меркатор UTM рядами Крюгера шестого порядка, как
/// `to_utm` в `core/coords.py` модуля: восток и север или None вне
/// широт UTM.
fn to_utm(lat: f64, lon: f64, zone: u32) -> Option<(f64, f64)> {
    const A: f64 = 6378137.0;
    const F: f64 = 1.0 / 298.257223563;
    const K0: f64 = 0.9996;
    if !(-80.0..=84.0).contains(&lat) || !lon.is_finite() {
        return None;
    }
    let n = F / (2.0 - F);
    let e = (F * (2.0 - F)).sqrt();
    let (n2, n3, n4, n5, n6) = (n.powi(2), n.powi(3), n.powi(4), n.powi(5), n.powi(6));
    let ar = A / (1.0 + n) * (1.0 + n2 / 4.0 + n4 / 64.0 + n6 / 256.0);
    let alpha = [
        n / 2.0 - 2.0 * n2 / 3.0 + 5.0 * n3 / 16.0 + 41.0 * n4 / 180.0 - 127.0 * n5 / 288.0
            + 7891.0 * n6 / 37800.0,
        13.0 * n2 / 48.0 - 3.0 * n3 / 5.0 + 557.0 * n4 / 1440.0 + 281.0 * n5 / 630.0
            - 1983433.0 * n6 / 1935360.0,
        61.0 * n3 / 240.0 - 103.0 * n4 / 140.0 + 15061.0 * n5 / 26880.0 + 167603.0 * n6 / 181440.0,
        49561.0 * n4 / 161280.0 - 179.0 * n5 / 168.0 + 6601661.0 * n6 / 7257600.0,
        34729.0 * n5 / 80640.0 - 3418889.0 * n6 / 1995840.0,
        212378941.0 * n6 / 319334400.0,
    ];
    let central = (f64::from(zone) - 1.0) * 6.0 - 180.0 + 3.0;
    let dlon = ((lon - central + 180.0).rem_euclid(360.0) - 180.0).to_radians();
    let tau = lat.to_radians().tan();
    let sigma = (e * (e * tau / 1.0_f64.hypot(tau)).atanh()).sinh();
    let tp = tau * 1.0_f64.hypot(sigma) - sigma * 1.0_f64.hypot(tau);
    let xi_p = tp.atan2(dlon.cos());
    let eta_p = (dlon.sin() / tp.hypot(dlon.cos())).asinh();
    let (mut xi, mut eta) = (xi_p, eta_p);
    for (j, a) in (1..).zip(alpha) {
        let k = 2.0 * f64::from(j);
        xi += a * (k * xi_p).sin() * (k * eta_p).cosh();
        eta += a * (k * xi_p).cos() * (k * eta_p).sinh();
    }
    let east = 500000.0 + K0 * ar * eta;
    let mut north = K0 * ar * xi;
    if lat < 0.0 {
        north += 10000000.0;
    }
    Some((east, north))
}

/// Кольцо участка в метрах UTM системы epsg (326zz или 327zz).
pub fn ring_utm(ring: &[(f64, f64)], epsg: u32) -> Result<Vec<(f64, f64)>, Error> {
    let zone = epsg % 100;
    let south = match epsg / 100 {
        326 => false,
        327 => true,
        _ => return Err(Error::NotUtm),
    };
    if !(1..=60).contains(&zone) {
        return Err(Error::NotUtm);
    }
    let mut out = Vec::with_capacity(ring.len());
    for (lat, lon) in ring {
        let (east, mut north) = to_utm(*lat, *lon, zone).ok_or(Error::OutsideUtm)?;
        // to_utm ставит ложный север по полушарию точки, участок у
        // экватора может пересечь его: система сцены одна.
        if south && *lat >= 0.0 {
            north += 10000000.0;
        } else if !south && *lat < 0.0 {
            north -= 10000000.0;
        }
        out.push((east, north));
    }
    Ok(out)
}

/// Рамка (xmin, ymin, xmax, ymax), выровненная на шаг res. None у пустого
/// кольца.
pub fn bounds(xy: &[(f64, f64)], res: f64) -> Option<[f64; 4]> {
    if xy.is_empty() {
        return None;
    }
    let fold = |f: fn(f64, f64) -> f64, start: f64, pick: fn(&(f64, f64)) -> f64| {
        xy.iter().map(pick).fold(start, f)
    };
    let (x0, x1) = (
        fold(f64::min, f64::INFINITY, |p| p.0),
        fold(f64::max, f64::NEG_INFINITY, |p| p.0),
    );
    let (y0, y1) = (
        fold(f64::min, f64::INFINITY, |p| p.1),
        fold(f64::max, f64::NEG_INFINITY, |p| p.1),
    );
    Some([
        (x0 / res).floor() * res,
        (y0 / res).floor() * res,
        (x1 / res).ceil() * res,
        (y1 / res).ceil() * res,
    ])
}

/// Шаг выдачи: 10 м у Sentinel-2 и 30 м у Landsat, если сторона участка
/// не больше MAX_PIXELS пикселей, иначе крупнее. С «Всеми каналами»
/// Sentinel-2 и ASTER предел - ALL_PIXELS.
pub fn step_for(xy: &[(f64, f64)], mission: Mission, products: &[&str]) -> f64 {
    if let Some(step) = modis::step(mission) {
        return step;
    }
    let steps: &[f64] = match mission {
        Mission::Landsat => &LANDSAT_STEPS,
        Mission::Mss => &MSS_STEPS,
        Mission::Aster | Mission::AsterNasa => &aster::STEPS,
        _ => &STEPS,
    };
    let all = (mission == Mission::Sentinel2 || aster::is_aster(mission))
        && products.contains(&"all_bands");
    let limit = if all { ALL_PIXELS } else { MAX_PIXELS };
    let first = steps.first().copied().unwrap_or(10.0);
    let Some(bx) = bounds(xy, 1.0) else {
        return first;
    };
    let side = (bx[2] - bx[0]).max(bx[3] - bx[1]);
    steps
        .iter()
        .copied()
        .find(|res| side / res <= limit)
        .unwrap_or(steps.last().copied().unwrap_or(first))
}

/// Пиксели рамки bx с шагом res внутри кольца xy, первая строка - север.
/// None, если размер не число или больше [`MAX_CELLS`].
pub fn mask(xy: &[(f64, f64)], bx: [f64; 4], res: f64) -> Option<Grid<bool>> {
    let size = |span: f64| -> Option<usize> {
        let n = (span / res).round();
        (n.is_finite() && n >= 0.0 && n <= MAX_CELLS as f64).then_some(n as usize)
    };
    let (cols, rows) = (size(bx[2] - bx[0])?, size(bx[3] - bx[1])?);
    if rows.checked_mul(cols)? > MAX_CELLS {
        return None;
    }
    let mut inside = Grid::filled(rows, cols, false);
    let n = xy.len();
    let mut cuts: Vec<f64> = Vec::new();
    for r in 0..rows {
        let y = bx[3] - (r as f64 + 0.5) * res;
        cuts.clear();
        for i in 0..n {
            let (x1, y1) = xy[i];
            let (x2, y2) = xy[(i + 1) % n];
            if y1 == y2 || (y1 > y) == (y2 > y) {
                continue;
            }
            cuts.push(x1 + (y - y1) * (x2 - x1) / (y2 - y1));
        }
        for c in 0..cols {
            let x = bx[0] + (c as f64 + 0.5) * res;
            // Чётность количества пересечений правее центра пикселя.
            let right = cuts.iter().filter(|at| x < **at).count();
            inside.data[r * cols + c] = right % 2 == 1;
        }
    }
    Some(inside)
}

// Облачность над участком.

/// Доли (облаков среди пикселей с данными, пикселей с данными), %.
pub(crate) fn shares(
    inside: &Grid<bool>,
    empty: &Grid<bool>,
    cloudy: &Grid<bool>,
) -> Option<(f64, f64)> {
    if inside.shape() != empty.shape() || inside.shape() != cloudy.shape() {
        return None;
    }
    let area = inside.count();
    let mut count = 0usize;
    let mut cloud = 0usize;
    for i in 0..inside.data.len() {
        if inside.data[i] && !empty.data[i] {
            count += 1;
            if cloudy.data[i] {
                cloud += 1;
            }
        }
    }
    if area == 0 || count == 0 {
        return Some((100.0, 0.0));
    }
    Some((
        100.0 * cloud as f64 / count as f64,
        100.0 * count as f64 / area as f64,
    ))
}

/// Облачность над участком по маске SCL: (доля облаков и теней среди
/// пикселей с данными, доля пикселей с данными), в процентах. None при
/// разных размерах.
pub fn cloud_share(scl: &Raw, inside: &Grid<bool>) -> Option<(f64, f64)> {
    let empty = scl.map(|v| SCL_EMPTY.contains(&v));
    let cloudy = scl.map(|v| SCL_CLOUDY.contains(&v));
    shares(inside, &empty, &cloudy)
}

/// Пиксели облаков и теней по qa_pixel Landsat: (облака, нет данных).
pub fn qa_cloudy(qa: &Raw) -> (Grid<bool>, Grid<bool>) {
    let empty = qa.map(|v| (v as i64) & QA_FILL != 0);
    let cloudy = qa.map(|v| (v as i64) & QA_CLOUDY != 0 && (v as i64) & QA_FILL == 0);
    (cloudy, empty)
}

/// Облачность над участком по qa_pixel Landsat, как [`cloud_share`].
pub fn cloud_share_qa(qa: &Raw, inside: &Grid<bool>) -> Option<(f64, f64)> {
    let (cloudy, empty) = qa_cloudy(qa);
    shares(inside, &empty, &cloudy)
}

/// Наименьшая доля данных над участком, при которой сцена видна.
pub fn min_valid(mission: Mission) -> f64 {
    if aster::is_aster(mission) {
        MIN_VALID_ASTER
    } else {
        MIN_VALID
    }
}

/// Облачность над участком по маске сцены любого спутника.
pub fn clouds_of(scene: &Scene, raw: &Raw, inside: &Grid<bool>) -> Option<(f64, f64)> {
    if modis::is_modis(scene.mission) {
        return modis::quality(scene.mission, raw, inside);
    }
    if aster::is_aster(scene.mission) {
        // Маски облаков нет - облачность сцены, доля данных по каналу 1.
        let empty = raw.map(|v| v == 0.0);
        let none = raw.map(|_| false);
        let (_, valid) = shares(inside, &empty, &none)?;
        return Some(if valid == 0.0 {
            (100.0, 0.0)
        } else {
            (scene.cloud, valid)
        });
    }
    if LANDSATS.contains(&scene.mission) {
        return cloud_share_qa(raw, inside);
    }
    cloud_share(raw, inside)
}

/// Пиксели без облаков, теней и пропусков по маске сцены.
pub fn clear_mask(scene: &Scene, raw: &Raw) -> Grid<bool> {
    if modis::is_modis(scene.mission) {
        return modis::clear(scene.mission, raw);
    }
    if aster::is_aster(scene.mission) {
        return raw.map(|v| v != 0.0);
    }
    if LANDSATS.contains(&scene.mission) {
        return raw.map(|v| {
            let q = v as i64;
            q & QA_FILL == 0 && q & QA_CLOUDY == 0
        });
    }
    raw.map(|v| !SCL_CLOUDY.contains(&v) && !SCL_EMPTY.contains(&v))
}

// Каналы и выдача.

/// Каналы сцены для продуктов: ключи ресурсов STAC. fill - дыры
/// температуры Landsat закрываются формулой ([`thermal_fill`]).
/// Неизвестные продукты пропускаются.
pub fn needed(products: &[&str], mission: Mission, fill: bool) -> Vec<String> {
    if modis::is_modis(mission) {
        return modis::needed(products);
    }
    if aster::is_aster(mission) {
        return aster::needed(products, None);
    }
    let mut keys: Vec<String> = Vec::new();
    for product in products {
        let bands: Vec<&str> = match *product {
            "lst" => {
                let mut b = vec!["lwir"];
                if fill {
                    b.extend(FILL_BANDS);
                }
                b
            }
            "mss_bands" => MSS_BANDS.to_vec(),
            "all_bands" => all_bands(mission).to_vec(),
            name => match (composite(name), index(name)) {
                (Some(c), _) => c.to_vec(),
                (None, Some(i)) => vec![i.bands.0, i.bands.1],
                (None, None) => Vec::new(),
            },
        };
        for name in bands {
            let key = band(name, mission);
            if !keys.iter().any(|k| k == key) {
                keys.push(key.to_string());
            }
        }
    }
    keys
}

/// Отражение из значений канала, нет данных - NaN, float32.
pub fn reflectance(dn: &Raw, asset: &Asset) -> Grid<f32> {
    let (scale, offset) = (asset.scale as f32, asset.offset as f32);
    let nodata = asset.nodata.unwrap_or(0.0);
    dn.map(|v| {
        if v == nodata {
            f32::NAN
        } else {
            (v as f32) * scale + offset
        }
    })
}

/// (a - b) / (a + b) в пределах -1..1. Где сумма не больше MIN_SUM,
/// индекс не определён - NaN. None при разных размерах.
pub fn normalized_difference(a: &Grid<f32>, b: &Grid<f32>) -> Option<Grid<f32>> {
    a.zip_map(b, |a, b| {
        let total = a + b;
        if total.is_nan() || total <= MIN_SUM {
            return f32::NAN;
        }
        ((a - b) / total).clamp(-1.0, 1.0)
    })
}

/// Излучательная способность по NDVI: вода, почва, смесь почвы
/// и растительности по доле покрытия, сплошная растительность.
pub fn emissivity(ndvi: &Grid<f32>) -> Grid<f32> {
    ndvi.map(|v| {
        let v = f64::from(v);
        let cover = ((v - 0.2) / 0.3).clamp(0.0, 1.0).powi(2);
        let out = if v > 0.5 {
            0.99
        } else if v < 0.0 {
            0.991
        } else if v < 0.2 {
            0.973
        } else {
            0.986 + 0.004 * cover
        };
        out as f32
    })
}

/// Температура поверхности, К: значения USGS, а их дыры - по формуле
/// (см. [`THERMAL_K`]). bands - каналы с масштабом каталога, number -
/// номер Landsat. Возвращает (кельвины, доля пикселей по формуле). None,
/// если канала нет или размеры разные.
pub fn thermal_fill(bands: &Bands, number: &str) -> Option<(Grid<f32>, f64)> {
    let lwir = bands.get("lwir")?;
    let (k1, k2) = thermal_k(number).or_else(|| thermal_k("8"))?;
    let ndvi = normalized_difference(bands.get("nir08")?, bands.get("red")?)?;
    let eps = emissivity(&ndvi);
    let (tau, trad, urad, drad) = (
        bands.get("atran")?,
        bands.get("trad")?,
        bands.get("urad")?,
        bands.get("drad")?,
    );
    let shape = lwir.shape();
    if [
        eps.shape(),
        tau.shape(),
        trad.shape(),
        urad.shape(),
        drad.shape(),
    ]
    .iter()
    .any(|s| *s != shape)
    {
        return None;
    }
    let mut kelvin = lwir.clone();
    let mut holes = 0usize;
    for i in 0..kelvin.data.len() {
        let (t, e) = (f64::from(tau.data[i]), f64::from(eps.data[i]));
        let radiance = (f64::from(trad.data[i])
            - f64::from(urad.data[i])
            - t * (1.0 - e) * f64::from(drad.data[i]))
            / (t * e);
        let formula = if radiance > 0.0 {
            k2 / (k1 / radiance + 1.0).ln()
        } else {
            f64::NAN
        };
        if !kelvin.data[i].is_finite() && formula.is_finite() {
            kelvin.data[i] = formula as f32;
            holes += 1;
        }
    }
    let known = kelvin.data.iter().filter(|v| v.is_finite()).count();
    let share = if known > 0 {
        holes as f64 / known as f64
    } else {
        0.0
    };
    Some((kelvin, share))
}

/// Массив продукта: сочетание и все каналы - набор, индекс и температура -
/// один канал. bands - отражения, у теплового канала - кельвины. None,
/// если продукт неизвестен или канала нет.
pub fn product(name: &str, bands: &Bands, mission: Mission) -> Option<Product> {
    let pick = |key: &str| bands.get(band(key, mission)).cloned();
    match name {
        "lst" => Some(Product::Single(
            bands.get("lwir")?.map(|v| v - KELVIN as f32),
        )),
        "mss_bands" => Some(Product::Stack(
            MSS_BANDS
                .iter()
                .map(|k| bands.get(*k).cloned())
                .collect::<Option<_>>()?,
        )),
        "all_bands" => {
            let keys = all_bands(mission);
            if keys.is_empty() {
                return None;
            }
            Some(Product::Stack(
                keys.iter().filter_map(|k| bands.get(*k).cloned()).collect(),
            ))
        }
        _ => {
            if let Some(c) = composite(name) {
                return Some(Product::Stack(
                    c.iter().map(|k| pick(k)).collect::<Option<_>>()?,
                ));
            }
            let i = index(name)?;
            let (a, b) = (pick(i.bands.0)?, pick(i.bands.1)?);
            Some(Product::Single(normalized_difference(&a, &b)?))
        }
    }
}

/// Пределы растяжения канала по процентилям low и high значений без NaN,
/// линейная интерполяция, как percentile numpy.
pub fn stretch(values: &Grid<f32>, low: f64, high: f64) -> (f64, f64) {
    let mut finite: Vec<f64> = values
        .data
        .iter()
        .filter(|v| v.is_finite())
        .map(|v| f64::from(*v))
        .collect();
    if finite.is_empty() {
        return (0.0, 0.3);
    }
    finite.sort_by(f64::total_cmp);
    let lo = percentile(&finite, low);
    let mut hi = percentile(&finite, high);
    if hi <= lo {
        hi = lo + 1e-3;
    }
    (lo, hi)
}

/// Процентиль p упорядоченного непустого списка.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    let last = sorted.len().saturating_sub(1);
    let at = (p.clamp(0.0, 100.0) / 100.0) * last as f64;
    let below = (at.floor() as usize).min(last);
    let above = (below + 1).min(last);
    let (a, b) = (sorted[below], sorted[above]);
    a + (b - a) * (at - below as f64)
}

/// Имя файла продукта: «S2_20260929_40VDK_ndvi»,
/// «L9_20250923_166-020_ndvi», у MSS «LM2_19750601_181-019_ndvi».
pub fn file_name(s: &Scene, name: &str) -> String {
    let day = head(&s.when, 10).replace('-', "");
    if modis::is_modis(s.mission) {
        let sat = s.tile.split(' ').next().unwrap_or("").replace('+', "");
        return format!("MODIS_{sat}_{day}_{name}");
    }
    if aster::is_aster(s.mission) {
        return aster::file_name(&head(&s.when, 10), name);
    }
    if LANDSATS.contains(&s.mission) {
        let (sat, place) = s.tile.split_once(' ').unwrap_or((&s.tile, ""));
        let sat = if s.mission == Mission::Mss {
            format!("LM{}", sat.chars().skip(1).collect::<String>())
        } else {
            sat.to_string()
        };
        return format!("{sat}_{day}_{}_{name}", place.replace('/', "-"));
    }
    format!("S2_{day}_{}_{name}", s.tile)
}

/// Подпись снимка: изменённые данные Copernicus с годом или снимок
/// Landsat от USGS, у MODIS - продукт и архив NASA.
pub fn credit(s: &Scene) -> String {
    if modis::is_modis(s.mission) {
        return modis::credit(&s.id);
    }
    if aster::is_aster(s.mission) {
        return aster::CREDIT.to_string();
    }
    if LANDSATS.contains(&s.mission) {
        let number: String = s
            .tile
            .split(' ')
            .next()
            .unwrap_or("")
            .chars()
            .skip(1)
            .collect();
        return LANDSAT_ATTRIBUTION.replace("{number}", &number);
    }
    ATTRIBUTION.replace("{year}", &head(&s.when, 4))
}
