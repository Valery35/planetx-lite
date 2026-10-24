// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Номенклатура листов карт в точке: общемировые разграфки. Перенос
//! `core/sheets.py` модуля.
//!
//! Всё считается формулой по широте и долготе, данных не нужно.
//!
//! - Международная карта мира 1:1 000 000 (IMW): лист 4° по широте
//!   и 6° по долготе. Пояс - буква A-V от экватора, перед ней N или S
//!   полушария, выше 88° - Z. Колонна - номер 1-60 от 180° на восток.
//! - Российская номенклатура (ГОСТ Р 51607, советская система) до
//!   1:200 000. 1:1 000 000 - O-40, между 60° и 76° листы сдвоены по
//!   долготе (P-39,40), между 76° и 88° счетверены (T-37,38,39,40).
//!   1:500 000 - четверть листа, буквы А-Г, севернее 60° сдвоены
//!   (R-39-А,Б). 1:200 000 - 36 листов 40′ × 1°, римские I-XXXVI,
//!   между 60° и 76° сдвоены, между 76° и 88° строены
//!   (U-40-XXXI,XXXII,XXXIII). Южное полушарие помечается «(Ю. П.)».
//!   По той же номенклатуре нумеруются листы Госгеолкарты-1000 и 200.
//! - NATO, JOG 1:250 000 (серия 1501): лист IMW делится на 16 листов
//!   1° × 1.5°, номера 1-16 с северо-западного угла по рядам. Между 60°
//!   и 68° лист IMW сдвоен (NQ 33,34) и делится на 16 листов 1° × 3°,
//!   между 68° и 72° - на 12 листов 1° × 4°. Правила сверены модулем по
//!   листам NH 42-5 (Кветта), NM 11-3 (Банф), NQ 33,34-1…4 (Будё,
//!   Сулитьельма, Кируна, Виттанги) и NR 33,34-11 (Нарвик). Севернее 72°
//!   разграфка не найдена, номер не даётся.
//! - MGRS: зона, полоса и квадрат 100 км. Расчёт UTM - перенос нужной
//!   части `core/coords.py` модуля: ряды Крюгера шестого порядка
//!   (C. F. F. Karney, 2011), зоны Норвегии и Шпицбергена по правилам
//!   UTM, полярные области UPS не поддерживаются.

/// Пояса по 4° от экватора.
pub const LETTERS: &str = "ABCDEFGHIJKLMNOPQRSTUV";
/// Листы 1:500 000, кириллица.
pub const QUARTERS: [&str; 4] = ["А", "Б", "В", "Г"];
/// Пометка южного полушария по умолчанию.
pub const SOUTH_MARK: &str = " (Ю. П.)";
/// Севернее разграфка JOG не найдена.
pub const JOG_LIMIT: f64 = 72.0;

/// Строка окна листов: система («imw», «russian», «jog», «mgrs»),
/// знаменатель масштаба или None, номер листа или None - листа в этой
/// разграфке нет.
pub type Row = (&'static str, Option<u32>, Option<String>);

/// Римская запись числа 1-39.
pub fn roman(number: u32) -> String {
    let mut number = number;
    let mut out = String::new();
    for (value, sign) in [(10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I")] {
        while number >= value {
            out.push_str(sign);
            number -= value;
        }
    }
    out
}

/// Долгота в пределах -180..180.
fn wrap_lon(lon: f64) -> f64 {
    (lon + 180.0).rem_euclid(360.0) - 180.0
}

/// Лист 1:1 000 000 под точкой: буква, колонна, место точки от
/// северо-западного угла в градусах.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cell {
    /// Южное полушарие.
    pub south: bool,
    /// Выше 88°, лист Z.
    pub polar: bool,
    /// Буква пояса, Z у полюса.
    pub letter: char,
    /// Широта от экватора, градусы.
    pub band: f64,
    /// Колонна 1-60.
    pub column: u32,
    /// Градусов от северного края листа вниз.
    pub down: f64,
    /// Градусов от западного края листа вправо.
    pub right: f64,
}

impl Cell {
    pub fn new(lat: f64, lon: f64) -> Self {
        let lat = lat.clamp(-90.0, 90.0);
        let lon = wrap_lon(lon);
        let south = lat < 0.0;
        // Нечисло и бесконечность при приведении дают 0 или предел типа.
        let index = (lat.abs() / 4.0).floor() as usize;
        let letter = LETTERS.as_bytes().get(index).map(|b| char::from(*b));
        let polar = letter.is_none();
        let column = (((lon + 180.0) / 6.0).floor() as i64 + 1).clamp(1, 60) as u32;
        let index = index.min(LETTERS.len()) as f64;
        let north = if south { -4.0 * index } else { 4.0 * (index + 1.0) };
        let west = f64::from(column - 1) * 6.0 - 180.0;
        Self {
            south,
            polar,
            letter: letter.unwrap_or('Z'),
            band: lat.abs(),
            column,
            down: (north - lat).clamp(0.0, 4.0 - 1e-9),
            right: (lon - west).clamp(0.0, 6.0 - 1e-9),
        }
    }

    /// Номера колонн сдвоенного или счетверённого листа.
    pub fn group(&self, size: u32) -> Vec<u32> {
        let size = size.max(1);
        let start = self.column - (self.column - 1) % size;
        (start..start + size).collect()
    }
}

fn join(numbers: &[u32]) -> String {
    numbers.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
}

/// Лист Международной карты мира 1:1 000 000: «NO 40».
pub fn imw(lat: f64, lon: f64) -> String {
    let cell = Cell::new(lat, lon);
    let prefix = format!("{}{}", if cell.south { 'S' } else { 'N' }, cell.letter);
    if cell.polar { prefix } else { format!("{prefix} {}", cell.column) }
}

fn million(cell: &Cell) -> String {
    let columns = if cell.band >= 76.0 {
        cell.group(4)
    } else if cell.band >= 60.0 {
        cell.group(2)
    } else {
        vec![cell.column]
    };
    format!("{}-{}", cell.letter, join(&columns))
}

/// Номенклатура листа масштаба 1 000 000, 500 000 или 200 000. Выше 88° -
/// лист Z, листов крупнее него нет (None). Другой масштаб - None.
pub fn russian(lat: f64, lon: f64, scale: u32, south_mark: &str) -> Option<String> {
    let cell = Cell::new(lat, lon);
    let mark = if cell.south { south_mark } else { "" };
    if cell.polar {
        return (scale == 1000000).then(|| format!("Z{mark}"));
    }
    let base = format!("{}-{}", cell.letter, cell.column);
    match scale {
        1000000 => Some(format!("{}{mark}", million(&cell))),
        500000 => {
            let row = ((cell.down / 2.0).floor() as usize).min(1);
            if cell.band >= 60.0 {
                let pair = QUARTERS.get(2 * row..2 * row + 2)?.join(",");
                return Some(format!("{base}-{pair}{mark}"));
            }
            let col = ((cell.right / 3.0).floor() as usize).min(1);
            Some(format!("{base}-{}{mark}", QUARTERS.get(2 * row + col)?))
        }
        200000 => {
            let row = ((cell.down * 1.5).floor() as u32).min(5);
            let col = (cell.right.floor() as u32).min(5);
            let size = if cell.band >= 76.0 {
                3
            } else if cell.band >= 60.0 {
                2
            } else {
                1
            };
            let start = col - col % size;
            let numbers: Vec<String> = (start..start + size).map(|c| roman(row * 6 + c + 1)).collect();
            Some(format!("{base}-{}{mark}", numbers.join(",")))
        }
        _ => None,
    }
}

/// Лист JOG 1:250 000: «NO 40-6», «NQ 33,34-3». Севернее 72° - None.
pub fn jog(lat: f64, lon: f64) -> Option<String> {
    let cell = Cell::new(lat, lon);
    if cell.band >= JOG_LIMIT {
        return None;
    }
    let prefix = format!("{}{}", if cell.south { 'S' } else { 'N' }, cell.letter);
    let row = (cell.down.floor() as u32).min(3);
    if cell.band < 60.0 {
        let sheet = row * 4 + ((cell.right / 1.5).floor() as u32).min(3) + 1;
        return Some(format!("{prefix} {}-{sheet}", cell.column));
    }
    let columns = cell.group(2);
    let first = columns.first().copied().unwrap_or(cell.column);
    let right = cell.right + f64::from(cell.column - first) * 6.0;
    let sheet = if cell.band < 68.0 { row * 4 + ((right / 3.0).floor() as u32).min(3) + 1 } else { row * 3 + ((right / 4.0).floor() as u32).min(2) + 1 };
    Some(format!("{prefix} {}-{sheet}", join(&columns)))
}

/// Зона, полоса и квадрат 100 км MGRS: «40V DK». Вне UTM - None.
pub fn mgrs_square(lat: f64, lon: f64) -> Option<String> {
    let (zone, band, easting, northing) = to_utm(lat, lon)?;
    let columns = COLUMNS.get(((zone - 1) % 3) as usize)?.as_bytes();
    let column = columns.get(((easting / 100000.0).floor() as i64 - 1).rem_euclid(8) as usize)?;
    let shift = if zone % 2 == 0 { 5 } else { 0 };
    let row = ROWS.as_bytes().get(((northing / 100000.0).floor() as i64 + shift).rem_euclid(20) as usize)?;
    Some(format!("{zone}{band} {}{}", char::from(*column), char::from(*row)))
}

/// Строки окна: система, знаменатель масштаба или None, номер или None.
pub fn sheets(lat: f64, lon: f64, south_mark: &str) -> Vec<Row> {
    vec![
        ("imw", Some(1000000), Some(imw(lat, lon))),
        ("russian", Some(1000000), russian(lat, lon, 1000000, south_mark)),
        ("russian", Some(500000), russian(lat, lon, 500000, south_mark)),
        ("russian", Some(200000), russian(lat, lon, 200000, south_mark)),
        ("jog", Some(250000), jog(lat, lon)),
        ("mgrs", None, mgrs_square(lat, lon)),
    ]
}

// UTM, часть `core/coords.py` модуля.

/// Большая полуось и сжатие WGS84.
const A: f64 = 6378137.0;
const F: f64 = 1.0 / 298.257223563;
const K0: f64 = 0.9996;
const FALSE_EASTING: f64 = 500000.0;
/// Южное полушарие.
const FALSE_NORTHING: f64 = 10000000.0;
/// Полосы широты по 8°, X - 12°.
const BANDS: &str = "CDEFGHJKLMNPQRSTUVWX";
/// 100 км по востоку и по северу.
const COLUMNS: [&str; 3] = ["ABCDEFGH", "JKLMNPQR", "STUVWXYZ"];
const ROWS: &str = "ABCDEFGHJKLMNPQRSTUV";

/// Номер зоны UTM с исключениями Норвегии и Шпицбергена.
fn utm_zone(lat: f64, lon: f64) -> u32 {
    let lon = wrap_lon(lon);
    let zone = (((lon + 180.0) / 6.0).floor() as i64 + 1).clamp(1, 60) as u32;
    if (56.0..64.0).contains(&lat) && (3.0..12.0).contains(&lon) {
        return 32;
    }
    if (72.0..=84.0).contains(&lat) && (0.0..42.0).contains(&lon) {
        return if lon < 9.0 {
            31
        } else if lon < 21.0 {
            33
        } else if lon < 33.0 {
            35
        } else {
            37
        };
    }
    zone
}

/// Буква полосы широты или None вне UTM.
fn band(lat: f64) -> Option<char> {
    if !(-80.0..=84.0).contains(&lat) {
        return None;
    }
    let index = (((lat + 80.0) / 8.0).floor() as usize).min(BANDS.len() - 1);
    BANDS.as_bytes().get(index).map(|b| char::from(*b))
}

fn tau_prime(tau: f64, e: f64) -> f64 {
    let sigma = (e * (e * tau / 1.0f64.hypot(tau)).atanh()).sinh();
    tau * 1.0f64.hypot(sigma) - sigma * 1.0f64.hypot(tau)
}

/// Зона, полоса, восток и север в метрах или None вне UTM.
fn to_utm(lat: f64, lon: f64) -> Option<(u32, char, f64, f64)> {
    let letter = band(lat)?;
    if !lon.is_finite() {
        return None;
    }
    let n = F / (2.0 - F);
    let e = (F * (2.0 - F)).sqrt();
    let ar = A / (1.0 + n) * (1.0 + n.powi(2) / 4.0 + n.powi(4) / 64.0 + n.powi(6) / 256.0);
    let (n2, n3, n4, n5, n6) = (n.powi(2), n.powi(3), n.powi(4), n.powi(5), n.powi(6));
    let alpha = [
        n / 2.0 - 2.0 * n2 / 3.0 + 5.0 * n3 / 16.0 + 41.0 * n4 / 180.0 - 127.0 * n5 / 288.0 + 7891.0 * n6 / 37800.0,
        13.0 * n2 / 48.0 - 3.0 * n3 / 5.0 + 557.0 * n4 / 1440.0 + 281.0 * n5 / 630.0 - 1983433.0 * n6 / 1935360.0,
        61.0 * n3 / 240.0 - 103.0 * n4 / 140.0 + 15061.0 * n5 / 26880.0 + 167603.0 * n6 / 181440.0,
        49561.0 * n4 / 161280.0 - 179.0 * n5 / 168.0 + 6601661.0 * n6 / 7257600.0,
        34729.0 * n5 / 80640.0 - 3418889.0 * n6 / 1995840.0,
        212378941.0 * n6 / 319334400.0,
    ];
    let zone = utm_zone(lat, lon);
    let central = f64::from(zone - 1) * 6.0 - 180.0 + 3.0;
    let dlon = ((lon - central + 180.0).rem_euclid(360.0) - 180.0).to_radians();
    let tp = tau_prime(lat.to_radians().tan(), e);
    let xi_p = tp.atan2(dlon.cos());
    let eta_p = (dlon.sin() / tp.hypot(dlon.cos())).asinh();
    let (mut xi, mut eta) = (xi_p, eta_p);
    for (j, a) in alpha.iter().enumerate() {
        let k = 2.0 * (j + 1) as f64;
        xi += a * (k * xi_p).sin() * (k * eta_p).cosh();
        eta += a * (k * xi_p).cos() * (k * eta_p).sinh();
    }
    let easting = FALSE_EASTING + K0 * ar * eta;
    let mut northing = K0 * ar * xi;
    if lat < 0.0 {
        northing += FALSE_NORTHING;
    }
    Some((zone, letter, easting, northing))
}
