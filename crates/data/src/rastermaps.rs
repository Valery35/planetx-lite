// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Карты картинками тайлов поверх снимка, перенос `core/fields.py`,
//! `core/emag.py`, `core/glim.py`, `core/tilemaps.py` и
//! `core/tileoverlays.py` модуля.
//!
//! Три вида карт:
//! - карты значений геофизики [`FIELDS`] - тайлы planetx-terrain
//!   в записи Terrarium, код R * 256 + G + B / 256 - 32768 - значение,
//!   умноженное на scale, «нет данных» - (0, 0, 0). Тайл раскрашивается
//!   по шкале [`colors`];
//! - готовые карты [`MAPS`] - литология, покрытие земель, вода, почвы.
//!   Цвет «нет данных» становится прозрачным, альфа умножается
//!   на непрозрачность [`map_rgba`];
//! - наложения [`OVERLAYS`] - прозрачные картинки XYZ, морские знаки
//!   и маршруты, ложатся как есть.
//!
//! На глобусе одна карта значений или готовая карта, наложений - сколько
//! угодно, как у модуля. Картинки - RGBA 256 × 256 с премноженной альфой.
//! Цвета, пределы и непрозрачность - выбор помощника модуля, утверждает
//! автор.

/// Значение кода «нет данных» записи Terrarium.
pub const EMPTY: f64 = -32768.0;
/// Сторона тайла.
pub const SIZE: u32 = 256;
/// Хранилище тайлов planetx-terrain.
pub const TERRAIN: &str = "https://raw.githubusercontent.com/Valery35/planetx-terrain/main/";

/// Точка шкалы: значение и цвет.
pub type Stop = (f64, [u8; 3]);
/// Класс карты: код и цвет.
pub type Class = (&'static str, [u8; 3]);

/// Карта значений: ключ строки «Слоёв», папка planetx-terrain, глубший
/// уровень тайлов, код на единицу значения, раскраска, подписи шкалы,
/// единица, знаков после запятой, подпись источника и адрес,
/// непрозрачность.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Field {
    pub key: &'static str,
    pub folder: &'static str,
    /// Шаблон адреса тайлов.
    pub url: &'static str,
    pub max_level: u32,
    pub scale: f64,
    pub stops: &'static [Stop],
    pub ticks: &'static [f64],
    pub unit: &'static str,
    pub digits: usize,
    pub attribution: (&'static str, &'static str),
    pub opacity: f64,
}

/// Шаблон тайлов папки planetx-terrain.
macro_rules! terrain {
    ($folder:literal) => {
        concat!("https://raw.githubusercontent.com/Valery35/planetx-terrain/main/", $folder, "/{z}/{x}/{y}.png")
    };
}

/// Раскраска магнитных аномалий, нТл: синие - поле слабее нормального,
/// красные - сильнее. Пределы по 1-му и 99-му процентилям сетки.
const EMAG_STOPS: &[Stop] = &[
    (-250.0, [40, 60, 150]),
    (-120.0, [70, 130, 210]),
    (-40.0, [165, 205, 235]),
    (0.0, [245, 245, 235]),
    (40.0, [245, 205, 165]),
    (120.0, [225, 115, 70]),
    (250.0, [160, 30, 35]),
];

/// Карты значений в порядке строк каталога, `KEYS` модуля.
pub const FIELDS: [Field; 5] = [
    // Магнитные аномалии EMAG2v3 NOAA NCEI, высота 4 км, шаг 2'.
    Field {
        key: "emag",
        folder: "emag",
        url: terrain!("emag"),
        max_level: 5,
        scale: 1.0,
        stops: EMAG_STOPS,
        ticks: &[-250.0, -120.0, 0.0, 120.0, 250.0],
        unit: "nT",
        digits: 0,
        attribution: ("EMAG2v3, NOAA NCEI", "https://www.ncei.noaa.gov/products/earth-magnetic-model-anomaly-grid-2"),
        opacity: 0.8,
    },
    // Аномалия силы тяжести в свободном воздухе XGM2019e до степени 719.
    Field {
        key: "gravity",
        folder: "gravity",
        url: terrain!("gravity"),
        max_level: 5,
        scale: 10.0,
        stops: &[
            (-200.0, [30, 40, 130]),
            (-80.0, [60, 120, 210]),
            (-20.0, [170, 210, 235]),
            (0.0, [245, 245, 240]),
            (20.0, [245, 210, 150]),
            (80.0, [230, 120, 60]),
            (200.0, [150, 30, 30]),
        ],
        ticks: &[-200.0, -100.0, 0.0, 100.0, 200.0],
        unit: "mGal",
        digits: 0,
        attribution: ("Gravity: XGM2019e, Zingerle et al. 2020, ICGEM, CC BY 4.0", "https://doi.org/10.5880/ICGEM.2019.007"),
        opacity: 0.75,
    },
    // Аномалия Буге с поправкой на рельеф ETOPO 2022.
    Field {
        key: "bouguer",
        folder: "bouguer",
        url: terrain!("bouguer"),
        max_level: 5,
        scale: 10.0,
        stops: &[
            (-400.0, [30, 40, 130]),
            (-200.0, [60, 120, 210]),
            (-60.0, [170, 210, 235]),
            (0.0, [245, 245, 240]),
            (60.0, [245, 210, 150]),
            (200.0, [230, 120, 60]),
            (400.0, [150, 30, 30]),
        ],
        ticks: &[-400.0, -200.0, 0.0, 200.0, 400.0],
        unit: "mGal",
        digits: 0,
        attribution: (
            "Bouguer anomaly: XGM2019e, Zingerle et al. 2020, ICGEM, CC BY 4.0; relief: ETOPO 2022, NOAA NCEI",
            "https://doi.org/10.5880/ICGEM.2019.007",
        ),
        opacity: 0.75,
    },
    // Возраст океанической коры, Seton и соавторы 2020, шкала GeeK2007.
    Field {
        key: "seafloor_age",
        folder: "seafloor_age",
        url: terrain!("seafloor_age"),
        max_level: 5,
        scale: 10.0,
        stops: &[
            (0.0, [230, 40, 40]),
            (20.0, [245, 150, 40]),
            (40.0, [245, 230, 70]),
            (70.0, [110, 200, 90]),
            (110.0, [60, 170, 200]),
            (160.0, [60, 90, 200]),
            (200.0, [120, 60, 180]),
            (280.0, [80, 30, 110]),
        ],
        ticks: &[0.0, 50.0, 100.0, 150.0, 200.0, 250.0],
        unit: "Ma",
        digits: 1,
        attribution: (
            "Seafloor age: Seton et al. 2020, EarthByte, CC BY 4.0",
            "https://www.earthbyte.org/a-global-dataset-of-present-day-oceanic-crustal-age-and-seafloor-spreading-parameters/",
        ),
        opacity: 0.8,
    },
    // Тепловой поток, сетка кригинга базы IHFC 2024, 0.5°.
    Field {
        key: "heatflow",
        folder: "heatflow",
        url: terrain!("heatflow"),
        max_level: 3,
        scale: 10.0,
        stops: &[
            (0.0, [40, 50, 140]),
            (40.0, [60, 120, 210]),
            (60.0, [130, 200, 220]),
            (80.0, [245, 235, 160]),
            (100.0, [245, 170, 70]),
            (150.0, [220, 70, 40]),
            (250.0, [130, 20, 30]),
        ],
        ticks: &[0.0, 50.0, 100.0, 150.0, 200.0, 250.0],
        unit: "mW/m2",
        digits: 0,
        attribution: (
            "Heat flow: IHFC Global Heat Flow Database 2024, Petrunin et al. 2026, CC BY 4.0",
            "https://doi.org/10.5880/gfz.khag.2026.004",
        ),
        opacity: 0.75,
    },
];

/// Карта значений по ключу.
pub fn field(key: &str) -> Option<&'static Field> {
    FIELDS.iter().find(|f| f.key == key)
}

/// Шаблон адреса тайлов карты значений.
pub fn field_url(field: &Field) -> &'static str {
    field.url
}

/// Значение в код Terrarium с множителем scale, None - «нет данных».
pub fn encode(value: Option<f64>, scale: f64) -> [u8; 3] {
    match value.filter(|v| v.is_finite()) {
        None => [0, 0, 0],
        Some(v) => {
            let code = ((v * scale).round() + 32768.0).clamp(1.0, 65535.0) as u32;
            [(code >> 8) as u8, (code & 255) as u8, 0]
        }
    }
}

/// Код Terrarium в значение, None - «нет данных».
pub fn decode(rgb: [u8; 3], scale: f64) -> Option<f64> {
    let code = f64::from(rgb[0]) * 256.0 + f64::from(rgb[1]) + f64::from(rgb[2]) / 256.0 + EMPTY;
    (code > EMPTY + 0.5).then(|| code / scale)
}

/// Цвет значения по шкале stops, линейно между точками, за пределами -
/// крайний цвет.
fn interp(stops: &[Stop], value: f64) -> [f64; 3] {
    let Some(first) = stops.first() else {
        return [0.0; 3];
    };
    if value <= first.0 {
        return first.1.map(f64::from);
    }
    for pair in stops.windows(2) {
        let ((a, ca), (b, cb)) = (pair[0], pair[1]);
        if value <= b {
            let t = if b > a { (value - a) / (b - a) } else { 0.0 };
            return [0, 1, 2].map(|k| f64::from(ca[k]) + (f64::from(cb[k]) - f64::from(ca[k])) * t);
        }
    }
    stops.last().map_or([0.0; 3], |s| s.1.map(f64::from))
}

/// Цвет значения карты с премноженной альфой непрозрачности карты,
/// «нет данных» - прозрачно.
pub fn color(field: &Field, value: Option<f64>) -> [u8; 4] {
    match value {
        None => [0, 0, 0, 0],
        Some(v) => {
            let c = interp(field.stops, v);
            let a = field.opacity;
            [(c[0] * a).round() as u8, (c[1] * a).round() as u8, (c[2] * a).round() as u8, (255.0 * a).round() as u8]
        }
    }
}

/// Тайл записи Terrarium (RGBA загрузчика) в раскраску. Тайл без единого
/// значения - None, рисовать нечего.
pub fn field_rgba(field: &Field, tile: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(tile.len());
    let mut any = false;
    for px in tile.as_chunks::<4>().0 {
        let c = color(field, decode([px[0], px[1], px[2]], field.scale));
        any |= c[3] > 0;
        out.extend_from_slice(&c);
    }
    any.then_some(out)
}

/// Значение пикселя (столбец, строка) тайла RGBA карты значений.
pub fn value_at(field: &Field, tile: &[u8], column: u32, row: u32) -> Option<f64> {
    let at = ((row.min(SIZE - 1) * SIZE + column.min(SIZE - 1)) * 4) as usize;
    let px = tile.get(at..at + 3)?;
    decode([px[0], px[1], px[2]], field.scale)
}

/// Тайл (x, y) уровня z и пиксель (столбец, строка) точки.
pub fn pixel_of(lat: f64, lon: f64, z: u32) -> ((u32, u32), (u32, u32)) {
    let lat = lat.clamp(-85.0511, 85.0511).to_radians();
    let world = f64::from(SIZE) * 2f64.powi(z.min(30) as i32);
    let gx = ((lon + 180.0) / 360.0 * world).clamp(0.0, world - 1e-6);
    let gy = ((1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * world).clamp(0.0, world - 1e-6);
    let size = f64::from(SIZE);
    (((gx / size) as u32, (gy / size) as u32), ((gx % size) as u32, (gy % size) as u32))
}

/// Как узнать значение под щелчком у готовой карты.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reading {
    /// Класс по цвету пикселя.
    Classes(&'static [Class]),
    /// Повторяемость воды - синий канал, доля лет с водой, проценты.
    BlueShare,
    /// Запрос GetFeatureInfo службы: шаблон с {west} {south} {east}
    /// {north} и множитель ответа.
    Query(&'static str, f64),
    /// Запрос map_query Macrostrat (`crate::geology`).
    Macrostrat,
}

/// Готовая карта: ключ строки «Слоёв», шаблон тайла ({z}/{x}/{y} или
/// {bbox} WMS в EPSG:3857), глубший уровень, подпись и адрес,
/// непрозрачность, цвет «нет данных», как читать значение, шаблон тайлов
/// мельче уровня split или "" - один шаблон, единица и знаки после
/// запятой значения.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileMap {
    pub key: &'static str,
    pub url: &'static str,
    pub max_level: u32,
    pub attribution: (&'static str, &'static str),
    pub opacity: f64,
    pub clear: Option<[u8; 3]>,
    pub reading: Reading,
    pub coarse: &'static str,
    pub split: u32,
    pub unit: &'static str,
    pub digits: usize,
}

const GSW_ATTRIBUTION: (&str, &str) = ("Surface water: EC JRC/Google, Pekel et al. 2016", "https://global-surface-water.appspot.com/");
const SOIL_ATTRIBUTION: (&str, &str) = ("Soil: SoilGrids 2.0, ISRIC, CC BY 4.0", "https://soilgrids.org/");
/// Уровень тайлов почв: пиксель около 75 м у экватора, сетка SoilGrids
/// 250 м.
const SOIL_LEVEL: u32 = 11;
const WHITE: [u8; 3] = [255, 255, 255];
/// Пиксель запроса мельче сетки 250 м служба оставляет пустым.
pub const QUERY_HALF: f64 = 0.006;
/// Допуск сравнения цвета класса, сумма модулей разностей каналов.
pub const CLASS_TOLERANCE: i32 = 40;

/// Классы литологии GLiM (Hartmann, Moosdorf 2012). Вода и «нет данных»
/// прозрачны и в список не входят.
pub const GLIM_CLASSES: &[Class] = &[
    ("su", [238, 228, 150]),
    ("ss", [236, 186, 110]),
    ("sm", [205, 165, 120]),
    ("sc", [110, 180, 225]),
    ("ev", [225, 120, 200]),
    ("py", [250, 150, 95]),
    ("va", [232, 80, 72]),
    ("vi", [198, 96, 140]),
    ("vb", [150, 52, 52]),
    ("pa", [250, 165, 185]),
    ("pi", [205, 125, 175]),
    ("pb", [130, 80, 135]),
    ("mt", [140, 170, 100]),
    ("ig", [232, 242, 252]),
];

/// Классы ESA WorldCover и цвета её карты.
pub const LAND_COVER: &[Class] = &[
    ("tree", [0, 100, 0]),
    ("shrub", [255, 187, 34]),
    ("grass", [255, 255, 76]),
    ("crop", [240, 150, 255]),
    ("built", [250, 0, 0]),
    ("bare", [180, 180, 180]),
    ("snow", [240, 240, 240]),
    ("water", [0, 100, 200]),
    ("wetland", [0, 150, 160]),
    ("mangrove", [0, 207, 117]),
    ("moss", [250, 230, 160]),
];

/// Переходы воды 1984-2021 - классы карты JRC в порядке её легенды.
pub const TRANSITIONS: &[Class] = &[
    ("permanent", [0, 0, 221]),
    ("new_permanent", [34, 177, 76]),
    ("lost_permanent", [209, 45, 62]),
    ("seasonal", [153, 217, 234]),
    ("new_seasonal", [181, 230, 29]),
    ("lost_seasonal", [230, 161, 170]),
    ("seasonal_to_permanent", [255, 127, 39]),
    ("permanent_to_seasonal", [255, 201, 14]),
    ("ephemeral_permanent", [127, 127, 127]),
    ("ephemeral_seasonal", [195, 195, 195]),
];

/// Тайл почв SoilGrids: WMS maps.isric.org в EPSG:3857.
macro_rules! soil_tile {
    ($prop:literal) => {
        concat!(
            "https://maps.isric.org/mapserv?map=/map/",
            $prop,
            ".map&SERVICE=WMS&VERSION=1.1.1&STYLES=&LAYERS=",
            $prop,
            "_0-5cm_mean&REQUEST=GetMap&SRS=EPSG:3857&BBOX={bbox}&WIDTH=256&HEIGHT=256&FORMAT=image/png"
        )
    };
}

/// Значение почв в точке: середина сетки 3 × 3 пикселя вокруг неё.
macro_rules! soil_query {
    ($prop:literal) => {
        concat!(
            "https://maps.isric.org/mapserv?map=/map/",
            $prop,
            ".map&SERVICE=WMS&VERSION=1.1.1&STYLES=&LAYERS=",
            $prop,
            "_0-5cm_mean&REQUEST=GetFeatureInfo&QUERY_LAYERS=",
            $prop,
            "_0-5cm_mean&SRS=EPSG:4326&BBOX={west},{south},{east},{north}&WIDTH=3&HEIGHT=3&X=1&Y=1&INFO_FORMAT=text/html"
        )
    };
}

const fn soil(key: &'static str, url: &'static str, query: &'static str, unit: &'static str) -> TileMap {
    TileMap {
        key,
        url,
        max_level: SOIL_LEVEL,
        attribution: SOIL_ATTRIBUTION,
        opacity: 0.8,
        clear: Some(WHITE),
        reading: Reading::Query(query, 0.1),
        coarse: "",
        split: 0,
        unit,
        digits: 1,
    }
}

/// Готовые карты в порядке строк каталога, `KEYS` модуля.
pub const MAPS: [TileMap; 9] = [
    // Геологическая карта Macrostrat, тайлы 512 × 512.
    TileMap {
        key: "geology",
        url: crate::geology::TILES,
        max_level: crate::geology::MAX_LEVEL,
        attribution: crate::geology::ATTRIBUTION,
        opacity: crate::geology::OPACITY,
        clear: None,
        reading: Reading::Macrostrat,
        coarse: "",
        split: 0,
        unit: "",
        digits: 0,
    },
    TileMap {
        key: "glim",
        url: terrain!("glim"),
        max_level: 7,
        attribution: ("Lithology: GLiM, Hartmann & Moosdorf 2012, CC BY 3.0", "https://doi.org/10.1594/PANGAEA.788537"),
        opacity: 0.75,
        clear: None,
        reading: Reading::Classes(GLIM_CLASSES),
        coarse: "",
        split: 0,
        unit: "",
        digits: 0,
    },
    // Покрытие земель 10 м: служба отдаёт уровни от split, мельче - тайлы
    // planetx-terrain.
    TileMap {
        key: "worldcover",
        url: "https://planetarycomputer.microsoft.com/api/data/v1/mosaic/287434fc0e0024667464141a8cb3f231/tiles/WebMercatorQuad/{z}/{x}/{y}@1x?collection=esa-worldcover&assets=map&colormap_name=esa-worldcover&format=png",
        max_level: 14,
        attribution: ("Land cover: ESA WorldCover 2021, CC BY 4.0, Microsoft Planetary Computer", "https://esa-worldcover.org/"),
        opacity: 0.75,
        clear: None,
        reading: Reading::Classes(LAND_COVER),
        coarse: terrain!("worldcover"),
        split: 6,
        unit: "",
        digits: 0,
    },
    TileMap {
        key: "water_occurrence",
        url: "https://storage.googleapis.com/global-surface-water/tiles2021/occurrence/{z}/{x}/{y}.png",
        max_level: 13,
        attribution: GSW_ATTRIBUTION,
        opacity: 0.9,
        clear: None,
        reading: Reading::BlueShare,
        coarse: "",
        split: 0,
        unit: "%",
        digits: 0,
    },
    TileMap {
        key: "water_transitions",
        url: "https://storage.googleapis.com/global-surface-water/tiles2021/transitions/{z}/{x}/{y}.png",
        max_level: 13,
        attribution: GSW_ATTRIBUTION,
        opacity: 0.9,
        clear: None,
        reading: Reading::Classes(TRANSITIONS),
        coarse: "",
        split: 0,
        unit: "",
        digits: 0,
    },
    soil("soil_clay", soil_tile!("clay"), soil_query!("clay"), "%"),
    soil("soil_sand", soil_tile!("sand"), soil_query!("sand"), "%"),
    soil("soil_soc", soil_tile!("soc"), soil_query!("soc"), "g/kg"),
    soil("soil_ph", soil_tile!("phh2o"), soil_query!("phh2o"), "pH"),
];

/// Готовая карта по ключу.
pub fn tile_map(key: &str) -> Option<&'static TileMap> {
    MAPS.iter().find(|m| m.key == key)
}

/// Тайл готовой карты в слой вида: цвет «нет данных» прозрачен, альфа
/// умножена на непрозрачность, цвет премножен. Тайл без единого пикселя
/// карты - None.
pub fn map_rgba(map: &TileMap, tile: &[u8], opacity: f64) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(tile.len());
    let mut any = false;
    for px in tile.as_chunks::<4>().0 {
        let mut alpha = f64::from(px[3]) / 255.0 * opacity;
        if let Some(clear) = map.clear
            && (0..3).all(|k| (i16::from(px[k]) - i16::from(clear[k])).abs() <= 2)
        {
            alpha = 0.0;
        }
        any |= alpha > 0.0;
        for &c in &px[..3] {
            out.push((f64::from(c) * alpha).round() as u8);
        }
        out.push((alpha * 255.0).round() as u8);
    }
    any.then_some(out)
}

/// Код класса, цвет которого ближе всего к rgb, или None.
pub fn class_of(classes: &[Class], rgb: [u8; 3], tolerance: i32) -> Option<&'static str> {
    let mut best = None;
    let mut score = tolerance;
    for (code, color) in classes {
        let d: i32 = (0..3).map(|k| (i32::from(rgb[k]) - i32::from(color[k])).abs()).sum();
        if d < score {
            best = Some(*code);
            score = d;
        }
    }
    best
}

/// Значение готовой карты под щелчком по пикселю.
#[derive(Debug, Clone, PartialEq)]
pub enum MapValue {
    Class(&'static str),
    Percent(u32),
}

/// Значение под щелчком по пикселю (r, g, b, a) тайла: класс,
/// повторяемость воды или None - пусто или значение даёт только запрос.
pub fn value_of(map: &TileMap, pixel: [u8; 4]) -> Option<MapValue> {
    if pixel[3] == 0 {
        return None;
    }
    match map.reading {
        Reading::Classes(classes) => class_of(classes, [pixel[0], pixel[1], pixel[2]], CLASS_TOLERANCE).map(MapValue::Class),
        Reading::BlueShare => Some(MapValue::Percent((f64::from(pixel[2]) / 255.0 * 100.0).round() as u32)),
        Reading::Query(..) | Reading::Macrostrat => None,
    }
}

/// Шаблон адреса тайла уровня z: мельче уровня split - coarse.
pub fn map_template(map: &TileMap, z: u32) -> &'static str {
    if !map.coarse.is_empty() && z < map.split { map.coarse } else { map.url }
}

/// Адрес GetFeatureInfo значения в точке или None.
pub fn query_url(map: &TileMap, lat: f64, lon: f64) -> Option<String> {
    let Reading::Query(template, _) = map.reading else {
        return None;
    };
    Some(
        template
            .replace("{west}", &format!("{:.5}", lon - QUERY_HALF))
            .replace("{south}", &format!("{:.5}", lat - QUERY_HALF))
            .replace("{east}", &format!("{:.5}", lon + QUERY_HALF))
            .replace("{north}", &format!("{:.5}", lat + QUERY_HALF)),
    )
}

/// Значение из ответа GetFeatureInfo в единицах карты или None - в точке
/// нет данных. Ответ считается враждебным.
pub fn parse_query(map: &TileMap, data: &[u8]) -> Option<f64> {
    let Reading::Query(_, scale) = map.reading else {
        return None;
    };
    let text = String::from_utf8_lossy(data);
    let start = text.find("id=\"value\">")? + 11;
    let rest = text[start..].trim_start();
    let end = rest.find('<')?;
    let value: f64 = rest[..end].trim().parse().ok()?;
    let value = value * scale;
    (value.is_finite() && value >= 0.0).then_some(value)
}

/// Наложение: ключ строки «Слоёв», шаблон адреса XYZ, предельный
/// уровень, подпись, ссылка на условия.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Overlay {
    pub key: &'static str,
    pub url: &'static str,
    pub max_level: u32,
    pub attribution: &'static str,
    pub terms: &'static str,
}

const WAYMARKED_TERMS: &str = "https://hiking.waymarkedtrails.org/#help-legal";

/// Наложения, `OVERLAYS` модуля. Адреса проверены модулем 18 октября
/// 2026 года.
pub const OVERLAYS: [Overlay; 3] = [
    Overlay {
        key: "sea_marks",
        url: "https://tiles.openseamap.org/seamark/{z}/{x}/{y}.png",
        max_level: 18,
        attribution: "© OpenSeaMap contributors",
        terms: "https://openseamap.org/",
    },
    Overlay {
        key: "hiking",
        url: "https://tile.waymarkedtrails.org/hiking/{z}/{x}/{y}.png",
        max_level: 18,
        attribution: "© OpenStreetMap contributors, © waymarkedtrails.org (CC-BY-SA)",
        terms: WAYMARKED_TERMS,
    },
    Overlay {
        key: "cycling",
        url: "https://tile.waymarkedtrails.org/cycling/{z}/{x}/{y}.png",
        max_level: 18,
        attribution: "© OpenStreetMap contributors, © waymarkedtrails.org (CC-BY-SA)",
        terms: WAYMARKED_TERMS,
    },
];

/// Картинка наложения в слой вида: альфа картинки, цвет премножен.
/// Пустой тайл - None.
pub fn overlay_rgba(tile: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(tile.len());
    let mut any = false;
    for px in tile.as_chunks::<4>().0 {
        let a = u16::from(px[3]);
        any |= a > 0;
        for &c in &px[..3] {
            out.push(((u16::from(c) * a + 127) / 255) as u8);
        }
        out.push(px[3]);
    }
    any.then_some(out)
}

/// Слой картинки наложения: тайл RGBA 256 × 256 с премноженной альфой
/// и доля в нём - глубина depth от тайла глобуса и номер клетки
/// (столбец, строка) этой доли.
pub struct Piece<'a> {
    pub rgba: &'a [u8],
    pub depth: u32,
    pub cell: (u32, u32),
}

/// Сложить слои снизу вверх в одну картинку тайла 256 × 256 с
/// премноженной альфой. Доля предка растягивается на весь тайл. Пустой
/// итог - None.
pub fn compose(pieces: &[Piece<'_>]) -> Option<Vec<u8>> {
    let mut canvas = tiny_skia::Pixmap::new(SIZE, SIZE)?;
    let mut any = false;
    for piece in pieces {
        let Some(size) = tiny_skia::IntSize::from_wh(SIZE, SIZE) else {
            continue;
        };
        if piece.rgba.len() != (SIZE * SIZE * 4) as usize {
            continue;
        }
        let Some(source) = tiny_skia::PixmapRef::from_bytes(piece.rgba, size.width(), size.height()) else {
            continue;
        };
        let depth = piece.depth.min(20);
        let scale = 2f32.powi(depth as i32);
        let side = SIZE as f32;
        let transform = tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, -(piece.cell.0 as f32) * side, -(piece.cell.1 as f32) * side);
        let quality = if depth == 0 { tiny_skia::FilterQuality::Nearest } else { tiny_skia::FilterQuality::Bilinear };
        let paint = tiny_skia::PixmapPaint { quality, ..tiny_skia::PixmapPaint::default() };
        canvas.draw_pixmap(0, 0, source, &paint, transform, None);
        any = true;
    }
    any.then(|| canvas.take())
}
