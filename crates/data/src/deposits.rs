// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Месторождения полезных ископаемых USGS. Перенос `core/deposits.py`
//! модуля.
//!
//! Три набора USGS:
//! - MRDS, Mineral Resources Data System - 304 632 записи месторождений,
//!   рудников, проявлений, архив 25 МБ, таблица 137 МБ;
//! - Major mineral deposits of the world, OFR 2005-1294 - 3 168 крупных
//!   месторождений мира;
//! - Mineral operations outside the United States (minfac) - 6 478
//!   рудников и заводов мира вне США на 2006-2010 годы.
//!
//! Архивы скачиваются при включении строки. Цвет точки - группа
//! первого полезного ископаемого, размер - крупное месторождение мира,
//! рудник мира или стадия освоения MRDS. Поля записи лежат одной
//! строкой с разделителем, как у модуля: записей в MRDS сотни тысяч.

pub const MRDS_URL: &str = "https://mrdata.usgs.gov/mrds/mrds-csv.zip";
pub const MAJOR_URL: &str = "https://mrdata.usgs.gov/major-deposits/ofr20051294-csv.zip";
pub const MINFAC_URL: &str = "https://mrdata.usgs.gov/mineral-operations/minfac-csv.zip";
pub const ATTRIBUTION: (&str, &str) = ("Deposits: USGS MRDS, OFR 2005-1294, mineral operations", "https://mrdata.usgs.gov/");
pub const MRDS_PAGE: &str = "https://mrdata.usgs.gov/mrds/show-mrds.php?dep_id=";

/// Группы полезных ископаемых: ключ, слова в названии, цвет. Порядок -
/// порядок проверки. Состав и цвета - выбор помощника модуля.
pub const GROUPS: [(&str, &[&str], [u8; 3]); 7] = [
    ("precious", &["gold", "silver", "platinum", "palladium", "pge", "rhodium", "iridium", "osmium", "ruthenium"], [240, 200, 40]),
    ("base", &["copper", "lead", "zinc", "nickel", "cobalt", "molybdenum", "tin", "tungsten", "mercury", "antimony", "bismuth", "cadmium", "arsenic"], [230, 110, 40]),
    ("iron", &["iron", "manganese", "chromium", "chromite", "vanadium", "titanium"], [170, 60, 60]),
    (
        "rare",
        &[
            "lithium", "rare earth", "ree", "niobium", "tantalum", "beryllium", "zirconium", "cesium", "gallium", "germanium", "indium", "tellurium", "graphite", "aluminum", "bauxite",
            "scandium", "yttrium", "rubidium", "hafnium",
        ],
        [170, 90, 220],
    ),
    ("energy", &["coal", "uranium", "thorium", "lignite", "oil", "gas", "peat", "anthracite", "geothermal", "helium"], [60, 200, 90]),
    ("gems", &["diamond", "gem", "emerald", "sapphire", "ruby", "opal", "turquoise", "garnet", "jade", "amber"], [60, 200, 230]),
    (
        "industrial",
        &[
            "sand", "gravel", "stone", "clay", "limestone", "gypsum", "salt", "potash", "phosphate", "barite", "fluor", "feldspar", "mica", "talc", "kaolin", "perlite", "pumice",
            "diatomite", "sulfur", "boron", "borate", "dolomite", "marble", "granite", "silica", "zeolite", "vermiculite", "asbestos", "magnesite", "magnesium", "bentonite", "cement",
            "lime", "quartz", "sodium", "potassium", "iodine", "bromine", "nitrate", "slate", "shale", "pegmatite", "wollastonite", "kyanite", "andalusite", "olivine", "strontium",
            "celestite", "calcium", "trona", "halite", "volcanic", "corundum", "emery", "fuller", "pigment", "aggregate", "montmorillonite", "abrasive", "phosphor", "asphalt", "soda",
            "sylvite", "pyrophyllite", "nepheline", "staurolite",
        ],
        [175, 175, 175],
    ),
];
/// Номер группы «прочее» и её цвет.
pub const OTHER: usize = GROUPS.len();
pub const OTHER_COLOR: [u8; 3] = [235, 235, 235];
/// Стадия освоения MRDS и размер точки, логических пикселей.
pub const STATUSES: [&str; 6] = ["Producer", "Past Producer", "Plant", "Prospect", "Occurrence", "Unknown"];
pub const STATUS_SIZES: [f64; 6] = [4.5, 4.0, 4.0, 3.0, 2.5, 2.5];
/// Крупное месторождение мира заметно крупнее точек MRDS, просьба
/// автора модуля от 11 октября 2026 года.
pub const MAJOR_SIZE: f64 = 10.0;
/// Рудник или завод мира, minfac.
pub const FACILITY_SIZE: f64 = 7.5;
/// Разделитель полей записи.
const SEPARATOR: char = '\u{1f}';

/// Номер группы по строке полезных ископаемых: первое найденное слово
/// по порядку GROUPS сперва в первом ископаемом, потом во всей строке.
pub fn group_of(commodities: &str) -> usize {
    let text = commodities.to_lowercase();
    let first = text.split(',').next().unwrap_or("").trim().to_string();
    for probe in [first.as_str(), text.as_str()] {
        if probe.is_empty() {
            continue;
        }
        if let Some(n) = GROUPS.iter().position(|(_, words, _)| words.iter().any(|w| probe.contains(w))) {
            return n;
        }
    }
    OTHER
}

/// Цвет группы.
pub fn color(group: usize) -> [u8; 3] {
    GROUPS.get(group).map_or(OTHER_COLOR, |g| g.2)
}

/// Месторождение: место, группа, стадия (номер STATUSES), крупное
/// месторождение мира, рудник или завод мира и поля записи.
#[derive(Debug, Clone, PartialEq)]
pub struct Deposit {
    pub lat: f64,
    pub lon: f64,
    pub group: usize,
    pub status: usize,
    pub major: bool,
    pub facility: bool,
    text: Box<str>,
}

impl Deposit {
    /// Поля записи: название, страна, полезные ископаемые, тип, стадия
    /// или модель, номер записи MRDS (пусто у прочих наборов), оператор,
    /// мощность и точность места - A примерная, F точная (у рудников
    /// и заводов мира).
    pub fn fields(&self) -> Vec<&str> {
        self.text.split(SEPARATOR).collect()
    }

    /// Размер точки.
    pub fn size(&self) -> f64 {
        if self.major {
            MAJOR_SIZE
        } else if self.facility {
            FACILITY_SIZE
        } else {
            STATUS_SIZES.get(self.status).copied().unwrap_or(STATUS_SIZES[STATUSES.len() - 1])
        }
    }
}

#[derive(Default)]
struct Builder {
    out: Vec<Deposit>,
}

impl Builder {
    fn add(&mut self, at: (Option<f64>, Option<f64>), commodities: &str, kind: (bool, usize, bool), fields: &[String]) {
        let (Some(lat), Some(lon)) = at else {
            return;
        };
        if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
            return;
        }
        let (major, status, facility) = kind;
        let text: Vec<String> = fields.iter().map(|f| f.replace(SEPARATOR, " ")).collect();
        self.out.push(Deposit { lat, lon, group: group_of(commodities), status, major, facility, text: text.join(&SEPARATOR.to_string()).into_boxed_str() });
    }
}

/// Наборы в один список: крупные месторождения мира, рудники и заводы
/// мира, потом MRDS. Аргументы - байты архивов или None.
pub fn parse(mrds: Option<&[u8]>, major: Option<&[u8]>, minfac: Option<&[u8]>) -> Result<Vec<Deposit>, String> {
    use crate::csv::{cell_number, text, text_1252};
    let mut out = Builder::default();
    let unknown = STATUSES.len() - 1;
    if let Some(data) = major {
        crate::csv::zip_table(data, "deposit.csv", |col, cells| {
            let t = |name: &str| text(cells, col(name));
            let model = if t("model_name").is_empty() { t("type_detail") } else { t("model_name") };
            let commodity = t("commodity");
            out.add(
                (cell_number(cells, col("latitude")), cell_number(cells, col("longitude"))),
                &commodity,
                (true, unknown, false),
                &[t("dep_name"), t("country"), commodity.clone(), t("dep_type"), model, String::new()],
            );
        })?;
    }
    if let Some(data) = minfac {
        // Файл в cp1252 - знак градуса в полях координат текстом.
        crate::csv::zip_table(data, "minfac.csv", |col, cells| {
            let t = |name: &str| text_1252(cells, col(name));
            let capacity = [t("capacity"), t("units")].into_iter().filter(|c| !c.is_empty()).collect::<Vec<_>>().join(" ");
            let name = if t("fac_name").is_empty() { t("location") } else { t("fac_name") };
            let operator = if t("op_comp").is_empty() { t("maininvest") } else { t("op_comp") };
            let capacity = if capacity.is_empty() { String::new() } else { format!("{capacity} ({})", t("year")) };
            let commodity = t("commodity");
            out.add(
                (cell_number(cells, col("latitude")), cell_number(cells, col("longitude"))),
                &commodity,
                (false, unknown, true),
                &[name, t("country"), commodity.clone(), t("fac_type"), t("status"), String::new(), operator, capacity, t("precision")],
            );
        })?;
    }
    if let Some(data) = mrds {
        crate::csv::zip_table(data, "mrds.csv", |col, cells| {
            let t = |name: &str| text(cells, col(name));
            let commodities = [t("commod1"), t("commod2"), t("commod3")].into_iter().filter(|c| !c.is_empty()).collect::<Vec<_>>().join(", ");
            let stage = t("dev_stat");
            let status = STATUSES.iter().position(|s| s.eq_ignore_ascii_case(&stage)).unwrap_or(unknown);
            out.add(
                (cell_number(cells, col("latitude")), cell_number(cells, col("longitude"))),
                &commodities,
                (false, status, false),
                &[t("site_name"), t("country"), commodities.clone(), t("dep_type"), stage, t("dep_id")],
            );
        })?;
    }
    Ok(out.out)
}
