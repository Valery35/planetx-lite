// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Положения спутников по орбитальным элементам, модель SGP4/SDP4.
//! Перенос `core/sgp4.py` модуля.
//!
//! Модель - SGP4 в редакции Vallado, Crawford, Hujsak, Kelso,
//! «Revisiting Spacetrack Report #3», AIAA 2006-6753, режим improved,
//! постоянные WGS72, как у элементов NORAD. Формулы перенесены
//! из опубликованного кода авторов статьи. Начальные величины считаются
//! для каждого спутника отдельно, один раз ([`init`], [`Orbits`]).
//! Положение на момент считается для каждого спутника ([`propagate`]),
//! в том числе глубокая часть модели - Луна, Солнце и резонансы суточных
//! и полусуточных орбит.
//!
//! Резонансы интегрируются от эпохи элементов шагами по 720 мин.
//! [`Cache`] хранит последний шаг интегратора, следующий момент дальше
//! от эпохи в ту же сторону продолжает счёт с него. Шаги одни и те же,
//! поэтому результат от запоминания не зависит. Результат - положение
//! и скорость в системе TEME, км и км/с, [`teme_to_ecef`] поворачивает
//! положение в систему Земли по звёздному времени. Полярное движение
//! и разность UT1-UTC не учитываются, это до сотен метров на орбите.
//!
//! Коды ошибок - как в эталоне: 1 - эксцентриситет вне 0-1, 2 - среднее
//! движение не больше нуля, 3 - возмущённый эксцентриситет вне 0-1,
//! 4 - отрицательный фокальный параметр, 6 - спутник ниже поверхности
//! Земли. При ошибках 1-4 положение - NaN. Свой код [`TIME_ERROR`] -
//! время не число или резонансный спутник дальше [`MAX_TSINCE`] от эпохи,
//! положение тоже NaN.

use std::f64::consts::PI;

pub const TWOPI: f64 = 2.0 * PI;
pub const X2O3: f64 = 2.0 / 3.0;
pub const DEG: f64 = PI / 180.0;

// Постоянные WGS72, с ними строятся элементы NORAD.
/// Гравитационный параметр, км³/с².
pub const MU: f64 = 398600.8;
/// Радиус Земли, км.
pub const RADIUS: f64 = 6378.135;
pub const J2: f64 = 0.001082616;
pub const J3: f64 = -0.00000253881;
pub const J4: f64 = -0.00000165597;
pub const J3OJ2: f64 = J3 / J2;

/// Юлианская дата 0 января 1950 года, 0 ч - начало отсчёта эпох.
pub const JD_EPOCH0: f64 = 2433281.5;
/// Юлианская дата 1 января 1970 года, 0 ч.
pub const JD_1970: f64 = 2440587.5;
/// Период, мин, с которого орбита считается глубокой.
pub const DEEP_PERIOD: f64 = 225.0;

// Глубокая часть модели.
const ZNS: f64 = 1.19459e-5;
const ZES: f64 = 0.01675;
const ZNL: f64 = 1.5835218e-4;
const ZEL: f64 = 0.05490;
/// Вращение Земли, рад/мин. Запись числа - как в эталоне.
#[allow(clippy::excessive_precision)]
const RPTIM: f64 = 4.37526908801129966e-3;
/// Шаг интегрирования резонансов, мин.
pub const STEP: f64 = 720.0;
const STEP2: f64 = 259200.0;

/// Код ошибки - время не число или дальше [`MAX_TSINCE`] у резонансного
/// спутника. В модуле такого кода нет.
pub const TIME_ERROR: u8 = 8;
/// Наибольшее время от эпохи у резонансного спутника, мин, около 190 лет.
/// Дальше интегрирование резонанса заняло бы сотни тысяч шагов.
pub const MAX_TSINCE: f64 = 1.0e8;

/// XKE: среднее движение в единицах радиуса Земли и минутах.
pub fn xke() -> f64 {
    60.0 / (RADIUS * RADIUS * RADIUS / MU).sqrt()
}

/// Скорость в км/с на единицу модели.
pub fn vkmpersec() -> f64 {
    RADIUS * xke() / 60.0
}

/// Звёздное время Гринвича, рад, по юлианской дате.
pub fn gstime(jdut1: f64) -> f64 {
    let tut1 = (jdut1 - 2451545.0) / 36525.0;
    let temp = -6.2e-6 * tut1 * tut1 * tut1
        + 0.093104 * tut1 * tut1
        + (876600.0 * 3600.0 + 8640184.812866) * tut1
        + 67310.54841;
    (temp * DEG / 240.0).rem_euclid(TWOPI)
}

/// Юлианская дата по дате и времени UTC, как jday эталона.
pub fn jday(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: f64) -> f64 {
    let (year, month) = (year as f64, month as f64);
    367.0 * year - (7.0 * (year + ((month + 9.0) / 12.0).floor()) * 0.25).floor() + (275.0 * month / 9.0).floor()
        + day as f64
        + 1721013.5
        + ((second / 60.0 + minute as f64) / 60.0 + hour as f64) / 24.0
}

/// Юлианская дата по секундам UTC от 1970 года.
pub fn unix_jd(seconds: f64) -> f64 {
    JD_1970 + seconds / 86400.0
}

/// Средние элементы спутника в единицах SGP4.
///
/// epoch - эпоха, сутки от 0 января 1950 года, ecco - эксцентриситет,
/// углы в радианах (argpo, inclo, mo, nodeo), no - среднее движение
/// по Козаи, рад/мин.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Element {
    pub name: String,
    pub number: i64,
    pub epoch: f64,
    pub bstar: f64,
    pub ecco: f64,
    pub argpo: f64,
    pub inclo: f64,
    pub mo: f64,
    pub no: f64,
    pub nodeo: f64,
}

fn number(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok()
}

fn integer(text: &str) -> Option<i64> {
    text.trim().parse::<i64>().ok()
}

/// Сутки от 0 января 1950 года по записи ISO «2026-10-05T01:07:02.3»
/// или None.
fn iso_epoch(text: &str) -> Option<f64> {
    let mut halves = text.trim().split('T');
    let (Some(date), Some(clock), None) = (halves.next(), halves.next(), halves.next()) else {
        return None;
    };
    let date: Vec<&str> = date.split('-').collect();
    let [year, month, day] = date.as_slice() else {
        return None;
    };
    let (year, month, day) = (integer(year)?, integer(month)?, integer(day)?);
    let parts: Vec<&str> = clock.split(':').collect();
    let hour = integer(parts.first()?)?;
    let minute = integer(parts.get(1)?)?;
    let second = match parts.get(2) {
        Some(s) => number(s)?,
        None => 0.0,
    };
    Some(jday(year, month, day, hour, minute, second) - JD_EPOCH0)
}

/// Ячейки строки CSV, кавычки снимаются. Запятых внутри названий
/// у CelesTrak нет, но кавычки учитываются.
fn split_csv(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    for ch in line.chars() {
        if ch == '"' {
            quoted = !quoted;
        } else if ch == ',' && !quoted {
            cells.push(std::mem::take(&mut cell));
        } else {
            cell.push(ch);
        }
    }
    cells.push(cell);
    cells
}

/// Поля CSV CelesTrak, без которых строка не берётся.
const NEED: [&str; 10] = [
    "OBJECT_NAME",
    "NORAD_CAT_ID",
    "EPOCH",
    "MEAN_MOTION",
    "ECCENTRICITY",
    "INCLINATION",
    "RA_OF_ASC_NODE",
    "ARG_OF_PERICENTER",
    "MEAN_ANOMALY",
    "BSTAR",
];

/// Элементы из CSV CelesTrak (запись OMM). Строки без нужных полей
/// пропускаются.
pub fn parse_csv(text: &str) -> Vec<Element> {
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let Some(first) = lines.next() else {
        return Vec::new();
    };
    let head: Vec<String> = first.split(',').map(|h| h.trim().to_uppercase()).collect();
    let mut at = [0usize; 10];
    for (slot, name) in at.iter_mut().zip(NEED) {
        let Some(index) = head.iter().position(|h| h == name) else {
            return Vec::new();
        };
        *slot = index;
    }
    let mut out = Vec::new();
    for line in lines {
        let cells = split_csv(line);
        if cells.len() < head.len() {
            continue;
        }
        let Some(epoch) = iso_epoch(&cells[at[2]]) else {
            continue;
        };
        let mut values = [0.0; 7];
        let mut ok = true;
        for (value, index) in values.iter_mut().zip(&at[3..]) {
            match number(&cells[*index]) {
                Some(v) => *value = v,
                None => ok = false,
            }
        }
        let norad = number(&cells[at[1]]);
        let Some(norad) = norad.filter(|n| n.is_finite()) else {
            continue;
        };
        if !ok {
            continue;
        }
        let [mean_motion, ecc, incl, node, argp, anomaly, bstar] = values;
        out.push(Element {
            name: cells[at[0]].trim().to_string(),
            number: norad as i64,
            epoch,
            bstar,
            ecco: ecc,
            argpo: argp * DEG,
            inclo: incl * DEG,
            mo: anomaly * DEG,
            no: mean_motion / 720.0 * PI,
            nodeo: node * DEG,
        });
    }
    out
}

/// Подстрока по номерам символов, как срез строки Python.
fn chars(line: &[char], start: usize, end: usize) -> String {
    line.iter().skip(start).take(end.saturating_sub(start)).collect()
}

/// Элементы одной пары строк TLE или None, если поля не числа.
fn tle_pair(name: &str, one: &[char], two: &[char]) -> Option<Element> {
    let mut year = integer(&chars(one, 18, 20))?;
    year += if year < 57 { 2000 } else { 1900 };
    let days = number(&chars(one, 20, 32))?;
    let mantissa = number(&format!("{}.{}", chars(one, 53, 54), chars(one, 54, 59)))?;
    let exponent = i32::try_from(integer(&chars(one, 59, 61))?).ok()?;
    let ecco = number(&format!("0.{}", chars(two, 26, 33).replace(' ', "0")))?;
    let name = if name.is_empty() { chars(one, 2, 7).trim().to_string() } else { name.to_string() };
    Some(Element {
        name,
        number: integer(&chars(one, 2, 7))?,
        epoch: jday(year, 1, 1, 0, 0, 0.0) + days - 1.0 - JD_EPOCH0,
        bstar: mantissa * 10f64.powi(exponent),
        ecco,
        argpo: number(&chars(two, 34, 42))? * DEG,
        inclo: number(&chars(two, 8, 16))? * DEG,
        mo: number(&chars(two, 43, 51))? * DEG,
        no: number(&chars(two, 52, 63))? * TWOPI / 1440.0,
        nodeo: number(&chars(two, 17, 25))? * DEG,
    })
}

/// Элементы из двухстрочных элементов. Строка названия перед парой
/// необязательна. Пара с полями не числами пропускается.
pub fn parse_tle<S: AsRef<str>>(lines: &[S]) -> Vec<Element> {
    let mut out = Vec::new();
    let mut name = String::new();
    let mut first: Option<Vec<char>> = None;
    for raw in lines {
        let line: Vec<char> = raw.as_ref().trim_end().chars().collect();
        let starts = |c: char| line.len() >= 2 && line[0] == c && line[1] == ' ';
        if starts('1') && line.len() >= 64 {
            first = Some(line);
            continue;
        }
        if starts('2')
            && line.len() >= 63
            && let Some(one) = first.take()
        {
            if let Some(element) = tle_pair(&name, &one, &line) {
                out.push(element);
            }
            name.clear();
            continue;
        }
        if !line.is_empty() && line[0] != '#' {
            name = line.iter().collect::<String>().trim().to_string();
            first = None;
        }
    }
    out
}

/// Начальные величины одного спутника, поля `FIELDS` модуля.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Sat {
    pub bstar: f64,
    pub ecco: f64,
    pub argpo: f64,
    pub inclo: f64,
    pub mo: f64,
    /// Среднее движение без поправки Козаи, рад/мин.
    pub no: f64,
    pub nodeo: f64,
    pub epoch: f64,
    pub isimp: bool,
    pub deep: bool,
    /// Резонанс: 0 - нет, 1 - суточный, 2 - полусуточный.
    pub irez: u8,
    pub gsto: f64,
    pub mdot: f64,
    pub argpdot: f64,
    pub nodedot: f64,
    pub nodecf: f64,
    pub cc1: f64,
    pub cc4: f64,
    pub cc5: f64,
    pub t2cof: f64,
    pub t3cof: f64,
    pub t4cof: f64,
    pub t5cof: f64,
    pub omgcof: f64,
    pub eta: f64,
    pub xmcof: f64,
    pub delmo: f64,
    pub sinmao: f64,
    pub d2: f64,
    pub d3: f64,
    pub d4: f64,
    pub con41: f64,
    pub x1mth2: f64,
    pub x7thm1: f64,
    pub xlcof: f64,
    pub aycof: f64,
    pub d2201: f64,
    pub d2211: f64,
    pub d3210: f64,
    pub d3222: f64,
    pub d4410: f64,
    pub d4422: f64,
    pub d5220: f64,
    pub d5232: f64,
    pub d5421: f64,
    pub d5433: f64,
    pub dedt: f64,
    pub didt: f64,
    pub dmdt: f64,
    pub dnodt: f64,
    pub domdt: f64,
    pub del1: f64,
    pub del2: f64,
    pub del3: f64,
    pub xfact: f64,
    pub xlamo: f64,
    pub e3: f64,
    pub ee2: f64,
    pub peo: f64,
    pub pgho: f64,
    pub pho: f64,
    pub pinco: f64,
    pub plo: f64,
    pub se2: f64,
    pub se3: f64,
    pub sgh2: f64,
    pub sgh3: f64,
    pub sgh4: f64,
    pub sh2: f64,
    pub sh3: f64,
    pub si2: f64,
    pub si3: f64,
    pub sl2: f64,
    pub sl3: f64,
    pub sl4: f64,
    pub xgh2: f64,
    pub xgh3: f64,
    pub xgh4: f64,
    pub xh2: f64,
    pub xh3: f64,
    pub xi2: f64,
    pub xi3: f64,
    pub xl2: f64,
    pub xl3: f64,
    pub xl4: f64,
    pub zmol: f64,
    pub zmos: f64,
}

/// Начальные величины спутника, как sgp4init эталона.
pub fn init(e: &Element) -> Sat {
    let xke = xke();
    let (ecco, argpo, inclo, mo, bstar) = (e.ecco, e.argpo, e.inclo, e.mo, e.bstar);
    let mut s = Sat {
        bstar,
        ecco,
        argpo,
        inclo,
        mo,
        nodeo: e.nodeo,
        epoch: e.epoch,
        ..Sat::default()
    };
    let ss = 78.0 / RADIUS + 1.0;
    let qzms2t = ((120.0 - 78.0) / RADIUS).powi(4);

    // initl: среднее движение без поправки Козаи, звёздное время.
    let eccsq = ecco * ecco;
    let omeosq = 1.0 - eccsq;
    let rteosq = omeosq.sqrt();
    let cosio = inclo.cos();
    let cosio2 = cosio * cosio;
    let ak = (xke / e.no).powf(X2O3);
    let d1 = 0.75 * J2 * (3.0 * cosio2 - 1.0) / (rteosq * omeosq);
    let mut delta = d1 / (ak * ak);
    let adel = ak * (1.0 - delta * delta - delta * (1.0 / 3.0 + 134.0 * delta * delta / 81.0));
    delta = d1 / (adel * adel);
    let no = e.no / (1.0 + delta);
    let ao = (xke / no).powf(X2O3);
    let sinio = inclo.sin();
    let po = ao * omeosq;
    let con42 = 1.0 - 5.0 * cosio2;
    s.con41 = -con42 - cosio2 - cosio2;
    let posq = po * po;
    let rp = ao * (1.0 - ecco);
    s.gsto = gstime(e.epoch + JD_EPOCH0);
    s.no = no;

    s.isimp = rp < 220.0 / RADIUS + 1.0;
    let mut sfour = ss;
    let mut qzms24 = qzms2t;
    let perige = (rp - 1.0) * RADIUS;
    if perige < 156.0 {
        sfour = perige - 78.0;
        if perige < 98.0 {
            sfour = 20.0;
        }
        qzms24 = ((120.0 - sfour) / RADIUS).powi(4);
        sfour = sfour / RADIUS + 1.0;
    }
    let pinvsq = 1.0 / posq;
    let tsi = 1.0 / (ao - sfour);
    let eta = ao * ecco * tsi;
    s.eta = eta;
    let etasq = eta * eta;
    let eeta = ecco * eta;
    let psisq = (1.0 - etasq).abs();
    let coef = qzms24 * tsi.powi(4);
    let coef1 = coef / psisq.powf(3.5);
    let cc2 = coef1
        * no
        * (ao * (1.0 + 1.5 * etasq + eeta * (4.0 + etasq))
            + 0.375 * J2 * tsi / psisq * s.con41 * (8.0 + 3.0 * etasq * (8.0 + etasq)));
    let cc1 = bstar * cc2;
    s.cc1 = cc1;
    let mut cc3 = 0.0;
    if ecco > 1.0e-4 {
        cc3 = -2.0 * coef * tsi * J3OJ2 * no * sinio / ecco;
    }
    s.x1mth2 = 1.0 - cosio2;
    s.cc4 = 2.0
        * no
        * coef1
        * ao
        * omeosq
        * (eta * (2.0 + 0.5 * etasq) + ecco * (0.5 + 2.0 * etasq)
            - J2 * tsi / (ao * psisq)
                * (-3.0 * s.con41 * (1.0 - 2.0 * eeta + etasq * (1.5 - 0.5 * eeta))
                    + 0.75 * s.x1mth2 * (2.0 * etasq - eeta * (1.0 + etasq)) * (2.0 * argpo).cos()));
    s.cc5 = 2.0 * coef1 * ao * omeosq * (1.0 + 2.75 * (etasq + eeta) + eeta * etasq);
    let cosio4 = cosio2 * cosio2;
    let temp1 = 1.5 * J2 * pinvsq * no;
    let temp2 = 0.5 * temp1 * J2 * pinvsq;
    let temp3 = -0.46875 * J4 * pinvsq * pinvsq * no;
    s.mdot = no
        + 0.5 * temp1 * rteosq * s.con41
        + 0.0625 * temp2 * rteosq * (13.0 - 78.0 * cosio2 + 137.0 * cosio4);
    s.argpdot = -0.5 * temp1 * con42
        + 0.0625 * temp2 * (7.0 - 114.0 * cosio2 + 395.0 * cosio4)
        + temp3 * (3.0 - 36.0 * cosio2 + 49.0 * cosio4);
    let xhdot1 = -temp1 * cosio;
    s.nodedot = xhdot1 + (0.5 * temp2 * (4.0 - 19.0 * cosio2) + 2.0 * temp3 * (3.0 - 7.0 * cosio2)) * cosio;
    let xpidot = s.argpdot + s.nodedot;
    s.omgcof = bstar * cc3 * argpo.cos();
    if ecco > 1.0e-4 {
        s.xmcof = -X2O3 * coef * bstar / eeta;
    }
    s.nodecf = 3.5 * omeosq * xhdot1 * cc1;
    s.t2cof = 1.5 * cc1;
    let lcof_den = if (cosio + 1.0).abs() > 1.5e-12 { 1.0 + cosio } else { 1.5e-12 };
    s.xlcof = -0.25 * J3OJ2 * sinio * (3.0 + 5.0 * cosio) / lcof_den;
    s.aycof = -0.5 * J3OJ2 * sinio;
    s.delmo = (1.0 + eta * mo.cos()).powi(3);
    s.sinmao = mo.sin();
    s.x7thm1 = 7.0 * cosio2 - 1.0;

    if TWOPI / no >= DEEP_PERIOD {
        s.deep = true;
        s.isimp = true;
        let com = dscom(e.epoch, ecco, argpo, 0.0, inclo, e.nodeo, no);
        let (sun, moon, emsq) = (&com.sun, &com.moon, com.emsq);
        s.se2 = 2.0 * sun.s1 * sun.s6;
        s.se3 = 2.0 * sun.s1 * sun.s7;
        s.si2 = 2.0 * sun.s2 * sun.z12;
        s.si3 = 2.0 * sun.s2 * (sun.z13 - sun.z11);
        s.sl2 = -2.0 * sun.s3 * sun.z2;
        s.sl3 = -2.0 * sun.s3 * (sun.z3 - sun.z1);
        s.sl4 = -2.0 * sun.s3 * (-21.0 - 9.0 * emsq) * ZES;
        s.sgh2 = 2.0 * sun.s4 * sun.z32;
        s.sgh3 = 2.0 * sun.s4 * (sun.z33 - sun.z31);
        s.sgh4 = -18.0 * sun.s4 * ZES;
        s.sh2 = -2.0 * sun.s2 * sun.z22;
        s.sh3 = -2.0 * sun.s2 * (sun.z23 - sun.z21);
        s.ee2 = 2.0 * moon.s1 * moon.s6;
        s.e3 = 2.0 * moon.s1 * moon.s7;
        s.xi2 = 2.0 * moon.s2 * moon.z12;
        s.xi3 = 2.0 * moon.s2 * (moon.z13 - moon.z11);
        s.xl2 = -2.0 * moon.s3 * moon.z2;
        s.xl3 = -2.0 * moon.s3 * (moon.z3 - moon.z1);
        s.xl4 = -2.0 * moon.s3 * (-21.0 - 9.0 * emsq) * ZEL;
        s.xgh2 = 2.0 * moon.s4 * moon.z32;
        s.xgh3 = 2.0 * moon.s4 * (moon.z33 - moon.z31);
        s.xgh4 = -18.0 * moon.s4 * ZEL;
        s.xh2 = -2.0 * moon.s2 * moon.z22;
        s.xh3 = -2.0 * moon.s2 * (moon.z23 - moon.z21);
        s.zmol = com.zmol;
        s.zmos = com.zmos;
        dsinit(&mut s, &com, eccsq, xpidot);
    }

    if !s.isimp {
        let cc1sq = cc1 * cc1;
        let d2 = 4.0 * ao * tsi * cc1sq;
        let temp = d2 * tsi * cc1 / 3.0;
        let d3 = (17.0 * ao + sfour) * temp;
        let d4 = 0.5 * temp * ao * tsi * (221.0 * ao + 31.0 * sfour) * cc1;
        s.d2 = d2;
        s.d3 = d3;
        s.d4 = d4;
        s.t3cof = d2 + 2.0 * cc1sq;
        s.t4cof = 0.25 * (3.0 * d3 + cc1 * (12.0 * d2 + 10.0 * cc1sq));
        s.t5cof = 0.2 * (3.0 * d4 + 12.0 * cc1 * d3 + 6.0 * d2 * d2 + 15.0 * cc1sq * (2.0 * d2 + cc1sq));
    }
    s
}

/// Члены одного возмущающего тела - Солнца или Луны.
#[derive(Debug, Clone, Copy, Default)]
struct Terms {
    s1: f64,
    s2: f64,
    s3: f64,
    s4: f64,
    s5: f64,
    s6: f64,
    s7: f64,
    z1: f64,
    z2: f64,
    z3: f64,
    z11: f64,
    z12: f64,
    z13: f64,
    z21: f64,
    z22: f64,
    z23: f64,
    z31: f64,
    z32: f64,
    z33: f64,
}

/// Общие величины глубокой части: члены Солнца (`ss*`, `sz*` модуля)
/// и Луны (`s*`, `z*`).
struct Com {
    sun: Terms,
    moon: Terms,
    zmol: f64,
    zmos: f64,
    emsq: f64,
    sinim: f64,
    cosim: f64,
    nm: f64,
    em: f64,
}

/// Положение тела: косинусы и синусы его углов и постоянная.
struct Body {
    zcosg: f64,
    zsing: f64,
    zcosi: f64,
    zsini: f64,
    zcosh: f64,
    zsinh: f64,
    cc: f64,
}

/// Орбита спутника для членов тел.
struct Orbit {
    cosim: f64,
    sinim: f64,
    cosomm: f64,
    sinomm: f64,
    emsq: f64,
    betasq: f64,
    rtemsq: f64,
    em: f64,
    xnoi: f64,
}

fn terms(b: &Body, o: &Orbit) -> Terms {
    let (zcosg, zsing, zcosi, zsini, zcosh, zsinh) = (b.zcosg, b.zsing, b.zcosi, b.zsini, b.zcosh, b.zsinh);
    let (cosim, sinim, cosomm, sinomm, emsq) = (o.cosim, o.sinim, o.cosomm, o.sinomm, o.emsq);
    let a1 = zcosg * zcosh + zsing * zcosi * zsinh;
    let a3 = -zsing * zcosh + zcosg * zcosi * zsinh;
    let a7 = -zcosg * zsinh + zsing * zcosi * zcosh;
    let a8 = zsing * zsini;
    let a9 = zsing * zsinh + zcosg * zcosi * zcosh;
    let a10 = zcosg * zsini;
    let a2 = cosim * a7 + sinim * a8;
    let a4 = cosim * a9 + sinim * a10;
    let a5 = -sinim * a7 + cosim * a8;
    let a6 = -sinim * a9 + cosim * a10;
    let x1 = a1 * cosomm + a2 * sinomm;
    let x2 = a3 * cosomm + a4 * sinomm;
    let x3 = -a1 * sinomm + a2 * cosomm;
    let x4 = -a3 * sinomm + a4 * cosomm;
    let x5 = a5 * sinomm;
    let x6 = a6 * sinomm;
    let x7 = a5 * cosomm;
    let x8 = a6 * cosomm;
    let z31 = 12.0 * x1 * x1 - 3.0 * x3 * x3;
    let z32 = 24.0 * x1 * x2 - 6.0 * x3 * x4;
    let z33 = 12.0 * x2 * x2 - 3.0 * x4 * x4;
    let mut z1 = 3.0 * (a1 * a1 + a2 * a2) + z31 * emsq;
    let mut z2 = 6.0 * (a1 * a3 + a2 * a4) + z32 * emsq;
    let mut z3 = 3.0 * (a3 * a3 + a4 * a4) + z33 * emsq;
    let z11 = -6.0 * a1 * a5 + emsq * (-24.0 * x1 * x7 - 6.0 * x3 * x5);
    let z12 = -6.0 * (a1 * a6 + a3 * a5) + emsq * (-24.0 * (x2 * x7 + x1 * x8) - 6.0 * (x3 * x6 + x4 * x5));
    let z13 = -6.0 * a3 * a6 + emsq * (-24.0 * x2 * x8 - 6.0 * x4 * x6);
    let z21 = 6.0 * a2 * a5 + emsq * (24.0 * x1 * x5 - 6.0 * x3 * x7);
    let z22 = 6.0 * (a4 * a5 + a2 * a6) + emsq * (24.0 * (x2 * x5 + x1 * x6) - 6.0 * (x4 * x7 + x3 * x8));
    let z23 = 6.0 * a4 * a6 + emsq * (24.0 * x2 * x6 - 6.0 * x4 * x8);
    z1 = z1 + z1 + o.betasq * z31;
    z2 = z2 + z2 + o.betasq * z32;
    z3 = z3 + z3 + o.betasq * z33;
    let s3 = b.cc * o.xnoi;
    let s2 = -0.5 * s3 / o.rtemsq;
    let s4 = s3 * o.rtemsq;
    let s1 = -15.0 * o.em * s4;
    let s5 = x1 * x3 + x2 * x4;
    let s6 = x2 * x3 + x1 * x4;
    let s7 = x2 * x4 - x1 * x3;
    Terms { s1, s2, s3, s4, s5, s6, s7, z1, z2, z3, z11, z12, z13, z21, z22, z23, z31, z32, z33 }
}

/// Общие величины глубокой части - солнечные и лунные члены.
fn dscom(epoch: f64, ep: f64, argpp: f64, tc: f64, inclp: f64, nodep: f64, np: f64) -> Com {
    const C1SS: f64 = 2.9864797e-6;
    const C1L: f64 = 4.7968065e-7;
    const ZSINIS: f64 = 0.39785416;
    const ZCOSIS: f64 = 0.91744867;
    const ZCOSGS: f64 = 0.1945905;
    const ZSINGS: f64 = -0.98088458;
    let nm = np;
    let em = ep;
    let (snodm, cnodm) = nodep.sin_cos();
    let (sinomm, cosomm) = argpp.sin_cos();
    let (sinim, cosim) = inclp.sin_cos();
    let emsq = em * em;
    let betasq = 1.0 - emsq;
    let rtemsq = betasq.sqrt();
    let day = epoch + 18261.5 + tc / 1440.0;
    let xnodce = (4.5236020 - 9.2422029e-4 * day).rem_euclid(TWOPI);
    let (stem, ctem) = xnodce.sin_cos();
    let zcosil = 0.91375164 - 0.03568096 * ctem;
    let zsinil = (1.0 - zcosil * zcosil).sqrt();
    let zsinhl = 0.089683511 * stem / zsinil;
    let zcoshl = (1.0 - zsinhl * zsinhl).sqrt();
    let gam = 5.8351514 + 0.0019443680 * day;
    let zx = 0.39785416 * stem / zsinil;
    let zy = zcoshl * ctem + 0.91744867 * zsinhl * stem;
    let zx = gam + zx.atan2(zy) - xnodce;
    let (zsingl, zcosgl) = zx.sin_cos();
    let orbit = Orbit { cosim, sinim, cosomm, sinomm, emsq, betasq, rtemsq, em, xnoi: 1.0 / nm };
    let sun = terms(
        &Body { zcosg: ZCOSGS, zsing: ZSINGS, zcosi: ZCOSIS, zsini: ZSINIS, zcosh: cnodm, zsinh: snodm, cc: C1SS },
        &orbit,
    );
    let moon = terms(
        &Body {
            zcosg: zcosgl,
            zsing: zsingl,
            zcosi: zcosil,
            zsini: zsinil,
            zcosh: zcoshl * cnodm + zsinhl * snodm,
            zsinh: snodm * zcoshl - cnodm * zsinhl,
            cc: C1L,
        },
        &orbit,
    );
    Com {
        sun,
        moon,
        zmol: (4.7199672 + 0.22997150 * day - gam).rem_euclid(TWOPI),
        zmos: (6.2565837 + 0.017201977 * day).rem_euclid(TWOPI),
        emsq,
        sinim,
        cosim,
        nm,
        em,
    }
}

/// Вековые члены Луны и Солнца и резонансы суточных и полусуточных
/// орбит, поля пишутся в s.
fn dsinit(s: &mut Sat, c: &Com, eccsq: f64, xpidot: f64) {
    const Q22: f64 = 1.7891679e-6;
    const Q31: f64 = 2.1460748e-6;
    const Q33: f64 = 2.2123015e-7;
    const ROOT22: f64 = 1.7891679e-6;
    const ROOT44: f64 = 7.3636953e-9;
    const ROOT54: f64 = 2.1765803e-9;
    const ROOT32: f64 = 3.7393792e-7;
    const ROOT52: f64 = 1.1428639e-7;
    let (nm, em, emsq, sinim, cosim) = (c.nm, c.em, c.emsq, c.sinim, c.cosim);
    let (sun, moon) = (&c.sun, &c.moon);
    let inclm = s.inclo;
    let mut irez = 0;
    if 0.0034906585 < nm && nm < 0.0052359877 {
        irez = 1;
    }
    if (8.26e-3..=9.24e-3).contains(&nm) && em >= 0.5 {
        irez = 2;
    }
    s.irez = irez;

    let ses = sun.s1 * ZNS * sun.s5;
    let sis = sun.s2 * ZNS * (sun.z11 + sun.z13);
    let sls = -ZNS * sun.s3 * (sun.z1 + sun.z3 - 14.0 - 6.0 * emsq);
    let sghs = sun.s4 * ZNS * (sun.z31 + sun.z33 - 6.0);
    let mut shs = -ZNS * sun.s2 * (sun.z21 + sun.z23);
    let flat = inclm < 5.2359877e-2 || inclm > PI - 5.2359877e-2;
    if flat {
        shs = 0.0;
    }
    if sinim != 0.0 {
        shs /= sinim;
    }
    let sgs = sghs - cosim * shs;
    s.dedt = ses + moon.s1 * ZNL * moon.s5;
    s.didt = sis + moon.s2 * ZNL * (moon.z11 + moon.z13);
    s.dmdt = sls - ZNL * moon.s3 * (moon.z1 + moon.z3 - 14.0 - 6.0 * emsq);
    let sghl = moon.s4 * ZNL * (moon.z31 + moon.z33 - 6.0);
    let mut shll = -ZNL * moon.s2 * (moon.z21 + moon.z23);
    if flat {
        shll = 0.0;
    }
    let mut domdt = sgs + sghl;
    let mut dnodt = shs;
    if sinim != 0.0 {
        domdt -= cosim / sinim * shll;
        dnodt += shll / sinim;
    }
    s.domdt = domdt;
    s.dnodt = dnodt;
    if irez == 0 {
        return;
    }
    let theta = s.gsto.rem_euclid(TWOPI);
    let aonv = (nm / xke()).powf(X2O3);
    let (mo, nodeo, argpo, no) = (s.mo, s.nodeo, s.argpo, s.no);
    if irez == 2 {
        let cosisq = cosim * cosim;
        let em = s.ecco;
        let emsq = eccsq;
        let eoc = em * emsq;
        let g201 = -0.306 - (em - 0.64) * 0.440;
        let (g211, g310, g322, g410, g422, g520);
        if em <= 0.65 {
            g211 = 3.616 - 13.2470 * em + 16.2900 * emsq;
            g310 = -19.302 + 117.3900 * em - 228.4190 * emsq + 156.5910 * eoc;
            g322 = -18.9068 + 109.7927 * em - 214.6334 * emsq + 146.5816 * eoc;
            g410 = -41.122 + 242.6940 * em - 471.0940 * emsq + 313.9530 * eoc;
            g422 = -146.407 + 841.8800 * em - 1629.014 * emsq + 1083.4350 * eoc;
            g520 = -532.114 + 3017.977 * em - 5740.032 * emsq + 3708.2760 * eoc;
        } else {
            g211 = -72.099 + 331.819 * em - 508.738 * emsq + 266.724 * eoc;
            g310 = -346.844 + 1582.851 * em - 2415.925 * emsq + 1246.113 * eoc;
            g322 = -342.585 + 1554.908 * em - 2366.899 * emsq + 1215.972 * eoc;
            g410 = -1052.797 + 4758.686 * em - 7193.992 * emsq + 3651.957 * eoc;
            g422 = -3581.690 + 16178.110 * em - 24462.770 * emsq + 12422.520 * eoc;
            g520 = if em > 0.715 {
                -5149.66 + 29936.92 * em - 54087.36 * emsq + 31324.56 * eoc
            } else {
                1464.74 - 4664.75 * em + 3763.64 * emsq
            };
        }
        let (g533, g521, g532);
        if em < 0.7 {
            g533 = -919.22770 + 4988.6100 * em - 9064.7700 * emsq + 5542.21 * eoc;
            g521 = -822.71072 + 4568.6173 * em - 8491.4146 * emsq + 5337.524 * eoc;
            g532 = -853.66600 + 4690.2500 * em - 8624.7700 * emsq + 5341.4 * eoc;
        } else {
            g533 = -37995.780 + 161616.52 * em - 229838.20 * emsq + 109377.94 * eoc;
            g521 = -51752.104 + 218913.95 * em - 309468.16 * emsq + 146349.42 * eoc;
            g532 = -40023.880 + 170470.89 * em - 242699.48 * emsq + 115605.82 * eoc;
        }
        let sini2 = sinim * sinim;
        let f220 = 0.75 * (1.0 + 2.0 * cosim + cosisq);
        let f221 = 1.5 * sini2;
        let f321 = 1.875 * sinim * (1.0 - 2.0 * cosim - 3.0 * cosisq);
        let f322 = -1.875 * sinim * (1.0 + 2.0 * cosim - 3.0 * cosisq);
        let f441 = 35.0 * sini2 * f220;
        let f442 = 39.3750 * sini2 * sini2;
        let f522 = 9.84375
            * sinim
            * (sini2 * (1.0 - 2.0 * cosim - 5.0 * cosisq) + 0.33333333 * (-2.0 + 4.0 * cosim + 6.0 * cosisq));
        let f523 = sinim
            * (4.92187512 * sini2 * (-2.0 - 4.0 * cosim + 10.0 * cosisq)
                + 6.56250012 * (1.0 + 2.0 * cosim - 3.0 * cosisq));
        let f542 = 29.53125 * sinim * (2.0 - 8.0 * cosim + cosisq * (-12.0 + 8.0 * cosim + 10.0 * cosisq));
        let f543 = 29.53125 * sinim * (-2.0 - 8.0 * cosim + cosisq * (12.0 + 8.0 * cosim - 10.0 * cosisq));
        let xno2 = nm * nm;
        let ainv2 = aonv * aonv;
        let mut temp1 = 3.0 * xno2 * ainv2;
        let mut temp = temp1 * ROOT22;
        s.d2201 = temp * f220 * g201;
        s.d2211 = temp * f221 * g211;
        temp1 *= aonv;
        temp = temp1 * ROOT32;
        s.d3210 = temp * f321 * g310;
        s.d3222 = temp * f322 * g322;
        temp1 *= aonv;
        temp = 2.0 * temp1 * ROOT44;
        s.d4410 = temp * f441 * g410;
        s.d4422 = temp * f442 * g422;
        temp1 *= aonv;
        temp = temp1 * ROOT52;
        s.d5220 = temp * f522 * g520;
        s.d5232 = temp * f523 * g532;
        temp = 2.0 * temp1 * ROOT54;
        s.d5421 = temp * f542 * g521;
        s.d5433 = temp * f543 * g533;
        s.xlamo = (mo + nodeo + nodeo - theta - theta).rem_euclid(TWOPI);
        s.xfact = s.mdot + s.dmdt + 2.0 * (s.nodedot + s.dnodt - RPTIM) - no;
    } else {
        let g200 = 1.0 + emsq * (-2.5 + 0.8125 * emsq);
        let g310 = 1.0 + 2.0 * emsq;
        let g300 = 1.0 + emsq * (-6.0 + 6.60937 * emsq);
        let f220 = 0.75 * (1.0 + cosim) * (1.0 + cosim);
        let f311 = 0.9375 * sinim * sinim * (1.0 + 3.0 * cosim) - 0.75 * (1.0 + cosim);
        let f330 = 1.875 * (1.0 + cosim).powi(3);
        let del1 = 3.0 * nm * nm * aonv * aonv;
        s.del2 = 2.0 * del1 * f220 * g200 * Q22;
        s.del3 = 3.0 * del1 * f330 * g300 * Q33 * aonv;
        s.del1 = del1 * f311 * g310 * Q31 * aonv;
        s.xlamo = (mo + nodeo + argpo - theta).rem_euclid(TWOPI);
        s.xfact = s.mdot + xpidot - RPTIM + s.dmdt + s.domdt + s.dnodt - no;
    }
}

/// Состояние интегратора резонансов одного спутника: время последнего
/// шага от эпохи, мин, и величины на нём.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Integrator {
    pub atime: f64,
    pub xli: f64,
    pub xni: f64,
}

impl Integrator {
    /// Интегратор в эпохе спутника.
    pub fn new(s: &Sat) -> Integrator {
        Integrator { atime: 0.0, xli: s.xlamo, xni: s.no }
    }
}

/// Состояние интеграторов резонансов спутников между вызовами
/// [`propagate`], по одному на спутник.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cache {
    pub states: Vec<Integrator>,
}

impl Cache {
    /// Интеграторы спутников sats в их эпохах.
    pub fn new(sats: &[Sat]) -> Cache {
        Cache { states: sats.iter().map(Integrator::new).collect() }
    }
}

/// Положение r, км, и скорость v, км/с, в TEME и код ошибки.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct State {
    pub r: [f64; 3],
    pub v: [f64; 3],
    pub error: u8,
}

impl State {
    fn lost(error: u8) -> State {
        State { r: [f64::NAN; 3], v: [f64::NAN; 3], error }
    }
}

/// Начальные величины спутников и расчёт положений.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Orbits {
    pub elements: Vec<Element>,
    /// Начальные величины уже посчитанных спутников, по порядку элементов.
    pub sats: Vec<Sat>,
    pub ready: bool,
    cache: Option<Cache>,
}

impl Orbits {
    pub fn new(elements: Vec<Element>) -> Orbits {
        let ready = elements.is_empty();
        Orbits { elements, sats: Vec::new(), ready, cache: None }
    }

    pub fn len(&self) -> usize {
        self.elements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Посчитать начальные величины следующих спутников. budget -
    /// количество спутников за вызов, None - все. Возвращает true, когда
    /// посчитаны все.
    pub fn init_some(&mut self, budget: Option<usize>) -> bool {
        let count = self.elements.len();
        let done = self.sats.len();
        let stop = budget.map_or(count, |b| count.min(done.saturating_add(b)));
        for e in self.elements.get(done..stop).unwrap_or_default() {
            self.sats.push(init(e));
        }
        self.ready = self.sats.len() >= count;
        self.ready
    }

    /// Положения и скорости в TEME на юлианскую дату jd.
    pub fn at(&mut self, jd: f64) -> Vec<State> {
        if !self.ready {
            self.init_some(None);
        }
        let tsince: Vec<f64> = self.sats.iter().map(|s| (jd - JD_EPOCH0 - s.epoch) * 1440.0).collect();
        let cache = self.cache.get_or_insert_with(|| Cache::new(&self.sats));
        propagate(&self.sats, &tsince, Some(cache))
    }
}

/// SGP4 для спутников sats на время tsince минут от эпохи каждого.
/// cache - интеграторы тех же спутников или None. Лишние значения
/// длинного списка не берутся.
pub fn propagate(sats: &[Sat], tsince: &[f64], cache: Option<&mut Cache>) -> Vec<State> {
    match cache {
        Some(cache) => {
            if cache.states.len() != sats.len() {
                *cache = Cache::new(sats);
            }
            sats.iter().zip(tsince).zip(&mut cache.states).map(|((s, t), i)| propagate_one(s, *t, i)).collect()
        }
        None => sats.iter().zip(tsince).map(|(s, t)| propagate_one(s, *t, &mut Integrator::new(s))).collect(),
    }
}

/// SGP4 одного спутника на время t минут от эпохи.
pub fn propagate_one(s: &Sat, t: f64, integrator: &mut Integrator) -> State {
    if !t.is_finite() || (s.deep && s.irez != 0 && t.abs() > MAX_TSINCE) {
        return State::lost(TIME_ERROR);
    }
    let xke = xke();
    let no = s.no;
    let xmdf = s.mo + s.mdot * t;
    let argpdf = s.argpo + s.argpdot * t;
    let nodedf = s.nodeo + s.nodedot * t;
    let t2 = t * t;
    let mut nodem = nodedf + s.nodecf * t2;
    let mut tempa = 1.0 - s.cc1 * t;
    let mut tempe = s.bstar * s.cc4 * t;
    let mut templ = s.t2cof * t2;
    let mut mm = xmdf;
    let mut argpm = argpdf;
    if !s.isimp {
        let delomg = s.omgcof * t;
        let delmtemp = 1.0 + s.eta * xmdf.cos();
        let delm = s.xmcof * (delmtemp * delmtemp * delmtemp - s.delmo);
        let temp = delomg + delm;
        mm = xmdf + temp;
        argpm = argpdf - temp;
        let t3 = t2 * t;
        let t4 = t3 * t;
        tempa = tempa - s.d2 * t2 - s.d3 * t3 - s.d4 * t4;
        tempe += s.bstar * s.cc5 * (mm.sin() - s.sinmao);
        templ += s.t3cof * t3 + t4 * (s.t4cof + t * s.t5cof);
    }
    let mut nm = no;
    let mut em = s.ecco;
    let mut inclm = s.inclo;
    if s.deep {
        (em, argpm, inclm, mm, nodem, nm) = dspace(s, t, em, argpm, inclm, mm, nodem, integrator);
    }

    let bad_n = nm <= 0.0;
    let am = (xke / nm).powf(X2O3) * tempa * tempa;
    nm = xke / am.powf(1.5);
    em -= tempe;
    let bad_e = em >= 1.0 || em < -0.001;
    if em < 1.0e-6 {
        em = 1.0e-6;
    }
    mm += no * templ;
    let xlm = mm + argpm + nodem;
    nodem %= TWOPI;
    argpm = argpm.rem_euclid(TWOPI);
    let xlm = xlm.rem_euclid(TWOPI);
    mm = (xlm - argpm - nodem).rem_euclid(TWOPI);

    let mut ep = em;
    let mut xincp = inclm;
    let mut argpp = argpm;
    let mut nodep = nodem;
    let mut mp = mm;
    let mut sinip = inclm.sin();
    let mut cosip = inclm.cos();
    let mut aycof = s.aycof;
    let mut xlcof = s.xlcof;
    let mut con41 = s.con41;
    let mut x1mth2 = s.x1mth2;
    let mut x7thm1 = s.x7thm1;
    let mut bad_p = false;
    if s.deep {
        let (e, mut i, mut n, mut a, m) = dpper(s, t, ep, xincp, nodep, argpp, mp);
        if i < 0.0 {
            i = -i;
            n += PI;
            a -= PI;
        }
        (ep, xincp, nodep, argpp, mp) = (e, i, n, a, m);
        bad_p = e < 0.0 || e > 1.0;
        let (si, ci) = i.sin_cos();
        sinip = si;
        cosip = ci;
        aycof = -0.5 * J3OJ2 * si;
        let den = if (ci + 1.0).abs() > 1.5e-12 { 1.0 + ci } else { 1.5e-12 };
        xlcof = -0.25 * J3OJ2 * si * (3.0 + 5.0 * ci) / den;
        let cosisq = ci * ci;
        con41 = 3.0 * cosisq - 1.0;
        x1mth2 = 1.0 - cosisq;
        x7thm1 = 7.0 * cosisq - 1.0;
    }

    let axnl = ep * argpp.cos();
    let temp = 1.0 / (am * (1.0 - ep * ep));
    let aynl = ep * argpp.sin() + temp * aycof;
    let xl = mp + argpp + nodep + temp * xlcof * axnl;

    // Уравнение Кеплера: не больше 10 шагов, до сходимости, как в эталоне.
    let u = (xl - nodep).rem_euclid(TWOPI);
    let mut eo1 = u;
    let mut tem5: f64 = 9999.9;
    let mut sineo1 = 0.0;
    let mut coseo1 = 0.0;
    for _ in 0..10 {
        let active = tem5.abs() >= 1.0e-12;
        if !active {
            break;
        }
        let (sn, cs) = eo1.sin_cos();
        let step = ((u - aynl * cs + axnl * sn - eo1) / (1.0 - cs * axnl - sn * aynl)).clamp(-0.95, 0.95);
        sineo1 = sn;
        coseo1 = cs;
        tem5 = step;
        eo1 += step;
    }

    let ecose = axnl * coseo1 + aynl * sineo1;
    let esine = axnl * sineo1 - aynl * coseo1;
    let el2 = axnl * axnl + aynl * aynl;
    let pl = am * (1.0 - el2);
    let bad_l = pl < 0.0;
    let rl = am * (1.0 - ecose);
    let rdotl = am.sqrt() * esine / rl;
    let rvdotl = pl.sqrt() / rl;
    let betal = (1.0 - el2).sqrt();
    let temp = esine / (1.0 + betal);
    let sinu = am / rl * (sineo1 - aynl - axnl * temp);
    let cosu = am / rl * (coseo1 - axnl + aynl * temp);
    let mut su = sinu.atan2(cosu);
    let sin2u = (cosu + cosu) * sinu;
    let cos2u = 1.0 - 2.0 * sinu * sinu;
    let temp = 1.0 / pl;
    let temp1 = 0.5 * J2 * temp;
    let temp2 = temp1 * temp;
    let mrt = rl * (1.0 - 1.5 * temp2 * betal * con41) + 0.5 * temp1 * x1mth2 * cos2u;
    su -= 0.25 * temp2 * x7thm1 * sin2u;
    let xnode = nodep + 1.5 * temp2 * cosip * sin2u;
    let xinc = xincp + 1.5 * temp2 * cosip * sinip * cos2u;
    let mvt = rdotl - nm * temp1 * x1mth2 * sin2u / xke;
    let rvdot = rvdotl + nm * temp1 * (x1mth2 * cos2u + 1.5 * con41) / xke;
    let (sinsu, cossu) = su.sin_cos();
    let (snod, cnod) = xnode.sin_cos();
    let (sini, cosi) = xinc.sin_cos();
    let xmx = -snod * cosi;
    let xmy = cnod * cosi;
    let ux = xmx * sinsu + cnod * cossu;
    let uy = xmy * sinsu + snod * cossu;
    let uz = sini * sinsu;
    let vx = xmx * cossu - cnod * sinsu;
    let vy = xmy * cossu - snod * sinsu;
    let vz = sini * cossu;
    let mr = mrt * RADIUS;
    let k = vkmpersec();

    // Коды ошибок в порядке проверки эталона: первая найденная главнее.
    let error = if bad_n {
        2
    } else if bad_e {
        1
    } else if bad_p {
        3
    } else if bad_l {
        4
    } else if mrt < 1.0 {
        6
    } else {
        0
    };
    if (1..6).contains(&error) {
        return State::lost(error);
    }
    State {
        r: [mr * ux, mr * uy, mr * uz],
        v: [(mvt * ux + rvdot * vx) * k, (mvt * uy + rvdot * vy) * k, (mvt * uz + rvdot * vz) * k],
        error,
    }
}

/// Долгопериодические члены Луны и Солнца на время t:
/// (ep, inclp, nodep, argpp, mp).
fn dpper(s: &Sat, t: f64, ep: f64, inclp: f64, nodep: f64, argpp: f64, mp: f64) -> (f64, f64, f64, f64, f64) {
    let zm = s.zmos + ZNS * t;
    let zf = zm + 2.0 * ZES * zm.sin();
    let sinzf = zf.sin();
    let f2 = 0.5 * sinzf * sinzf - 0.25;
    let f3 = -0.5 * sinzf * zf.cos();
    let ses = s.se2 * f2 + s.se3 * f3;
    let sis = s.si2 * f2 + s.si3 * f3;
    let sls = s.sl2 * f2 + s.sl3 * f3 + s.sl4 * sinzf;
    let sghs = s.sgh2 * f2 + s.sgh3 * f3 + s.sgh4 * sinzf;
    let shs = s.sh2 * f2 + s.sh3 * f3;
    let zm = s.zmol + ZNL * t;
    let zf = zm + 2.0 * ZEL * zm.sin();
    let sinzf = zf.sin();
    let f2 = 0.5 * sinzf * sinzf - 0.25;
    let f3 = -0.5 * sinzf * zf.cos();
    let sel = s.ee2 * f2 + s.e3 * f3;
    let sil = s.xi2 * f2 + s.xi3 * f3;
    let sll = s.xl2 * f2 + s.xl3 * f3 + s.xl4 * sinzf;
    let sghl = s.xgh2 * f2 + s.xgh3 * f3 + s.xgh4 * sinzf;
    let shll = s.xh2 * f2 + s.xh3 * f3;
    let pe = ses + sel - s.peo;
    let pinc = sis + sil - s.pinco;
    let pl = sls + sll - s.plo;
    let pgh = sghs + sghl - s.pgho;
    let ph = shs + shll - s.pho;
    let inclp = inclp + pinc;
    let ep = ep + pe;
    let (sinip, cosip) = inclp.sin_cos();

    // Наклон от 0.2 рад - члены напрямую, иначе поправка Лиддейна.
    if inclp >= 0.2 {
        let ph = ph / sinip;
        return (ep, inclp, nodep + ph, argpp + pgh - cosip * ph, mp + pl);
    }
    let (sinop, cosop) = nodep.sin_cos();
    let alfdp = sinip * sinop + ph * cosop + pinc * cosip * sinop;
    let betdp = sinip * cosop - ph * sinop + pinc * cosip * cosop;
    let node = nodep % TWOPI;
    let xls = mp + argpp + pl + pgh + (cosip - pinc * sinip) * node;
    let xnoh = node;
    let mut node = alfdp.atan2(betdp);
    if (xnoh - node).abs() > PI {
        if node < xnoh {
            node += TWOPI;
        } else {
            node -= TWOPI;
        }
    }
    let m = mp + pl;
    (ep, inclp, node, xls - m - cosip * node, m)
}

/// Вековые члены Луны и Солнца и резонансы на время t:
/// (em, argpm, inclm, mm, nodem, nm). Состояние интегратора резонансов
/// берётся из integrator и пишется туда же.
#[allow(clippy::too_many_arguments)]
fn dspace(
    s: &Sat,
    t: f64,
    em: f64,
    argpm: f64,
    inclm: f64,
    mm: f64,
    nodem: f64,
    integrator: &mut Integrator,
) -> (f64, f64, f64, f64, f64, f64) {
    const FASX2: f64 = 0.13130908;
    const FASX4: f64 = 2.8843198;
    const FASX6: f64 = 0.37448087;
    const G22: f64 = 5.7686396;
    const G32: f64 = 0.95240898;
    const G44: f64 = 1.8014998;
    const G52: f64 = 1.0508330;
    const G54: f64 = 4.4108898;
    let theta = (s.gsto + t * RPTIM).rem_euclid(TWOPI);
    let em = em + s.dedt * t;
    let inclm = inclm + s.didt * t;
    let argpm = argpm + s.domdt * t;
    let nodem = nodem + s.dnodt * t;
    let mm = mm + s.dmdt * t;
    if s.irez == 0 {
        return (em, argpm, inclm, mm, nodem, s.no);
    }
    let half = s.irez == 2;
    // Счёт от эпохи, если запомненный шаг по другую сторону от эпохи или
    // дальше от неё, чем новый момент.
    let Integrator { mut atime, mut xli, mut xni } = *integrator;
    if atime == 0.0 || t * atime <= 0.0 || t.abs() < atime.abs() {
        atime = 0.0;
        xni = s.no;
        xli = s.xlamo;
    }
    let delt = if t > 0.0 { STEP } else { -STEP };
    let (ft, xndt, xldot, xnddt) = loop {
        let (xndt, xnddt) = if half {
            let xomi = s.argpo + s.argpdot * atime;
            let x2omi = xomi + xomi;
            let x2li = xli + xli;
            let xndt = s.d2201 * (x2omi + xli - G22).sin()
                + s.d2211 * (xli - G22).sin()
                + s.d3210 * (xomi + xli - G32).sin()
                + s.d3222 * (-xomi + xli - G32).sin()
                + s.d4410 * (x2omi + x2li - G44).sin()
                + s.d4422 * (x2li - G44).sin()
                + s.d5220 * (xomi + xli - G52).sin()
                + s.d5232 * (-xomi + xli - G52).sin()
                + s.d5421 * (xomi + x2li - G54).sin()
                + s.d5433 * (-xomi + x2li - G54).sin();
            let xnddt = s.d2201 * (x2omi + xli - G22).cos()
                + s.d2211 * (xli - G22).cos()
                + s.d3210 * (xomi + xli - G32).cos()
                + s.d3222 * (-xomi + xli - G32).cos()
                + s.d5220 * (xomi + xli - G52).cos()
                + s.d5232 * (-xomi + xli - G52).cos()
                + 2.0
                    * (s.d4410 * (x2omi + x2li - G44).cos()
                        + s.d4422 * (x2li - G44).cos()
                        + s.d5421 * (xomi + x2li - G54).cos()
                        + s.d5433 * (-xomi + x2li - G54).cos());
            (xndt, xnddt)
        } else {
            let xndt = s.del1 * (xli - FASX2).sin()
                + s.del2 * (2.0 * (xli - FASX4)).sin()
                + s.del3 * (3.0 * (xli - FASX6)).sin();
            let xnddt = s.del1 * (xli - FASX2).cos()
                + 2.0 * s.del2 * (2.0 * (xli - FASX4)).cos()
                + 3.0 * s.del3 * (3.0 * (xli - FASX6)).cos();
            (xndt, xnddt)
        };
        let xldot = xni + s.xfact;
        let xnddt = xnddt * xldot;
        if (t - atime).abs() >= STEP {
            xli += xldot * delt + xndt * STEP2;
            xni += xndt * delt + xnddt * STEP2;
            atime += delt;
        } else {
            break (t - atime, xndt, xldot, xnddt);
        }
    };
    *integrator = Integrator { atime, xli, xni };
    let nm = xni + xndt * ft + xnddt * ft * ft * 0.5;
    let xl = xli + xldot * ft + xndt * ft * ft * 0.5;
    let mm = if s.irez != 1 { xl - 2.0 * nodem + 2.0 * theta } else { xl - nodem - argpm + theta };
    (em, argpm, inclm, mm, nodem, nm)
}

/// Положение TEME в систему, связанную с Землёй, поворотом на звёздное
/// время Гринвича.
pub fn teme_to_ecef(r: [f64; 3], jd: f64) -> [f64; 3] {
    let (s, c) = gstime(jd).sin_cos();
    [c * r[0] + s * r[1], -s * r[0] + c * r[1], r[2]]
}
