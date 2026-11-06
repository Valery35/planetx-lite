// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Система координат облака точек по строке WKT и пересчёт её координат
//! в широту и долготу. Свой расчёт без terrio, как пересчёт UTM модуля
//! (`core/coords.py`).
//!
//! Строка WKT - версии 1 (`PROJCS`, `GEOGCS`, `COMPD_CS`) или 2
//! (`PROJCRS`, `GEOGCRS`, `COMPOUNDCRS`). Проекции - поперечная
//! Меркатора (UTM, системы штатов США, Гаусс-Крюгер), коническая
//! Ламберта с одной и двумя параллелями (Lambert-93 IGN, системы
//! штатов США), равновеликая Альберса (CONUS USGS). Эллипсоид берётся
//! из строки. Сдвиг датума не учитывается - NAD83, ETRS89, RGF93
//! отличаются от WGS84 не больше чем на 2 м, для облака на глобусе это
//! меньше точности рельефа.
//!
//! Поперечная Меркатора - ряды Крюгера шестого порядка (C. F. F. Karney,
//! Transverse Mercator with an accuracy of a few nanometers, 2011), как
//! `from_utm` модуля. Коническая Ламберта и Альберса - формулы J. P.
//! Snyder, Map Projections - A Working Manual, USGS 1987.

/// Узел WKT: имя и аргументы.
#[derive(Debug, Clone, PartialEq)]
enum Wkt {
    Node(String, Vec<Wkt>),
    Text(String),
    Number(f64),
}

impl Wkt {
    fn name(&self) -> &str {
        match self {
            Wkt::Node(n, _) => n,
            _ => "",
        }
    }

    fn args(&self) -> &[Wkt] {
        match self {
            Wkt::Node(_, a) => a,
            _ => &[],
        }
    }

    /// Первый дочерний узел с одним из имён.
    fn child(&self, names: &[&str]) -> Option<&Wkt> {
        self.args().iter().find(|a| names.iter().any(|n| a.name().eq_ignore_ascii_case(n)))
    }

    fn text(&self, i: usize) -> Option<&str> {
        match self.args().get(i) {
            Some(Wkt::Text(t)) => Some(t),
            _ => None,
        }
    }

    fn number(&self, i: usize) -> Option<f64> {
        match self.args().get(i) {
            Some(Wkt::Number(v)) => Some(*v),
            Some(Wkt::Text(t)) => t.trim().parse().ok(),
            _ => None,
        }
    }
}

/// Наибольшая глубина вложения WKT - защита стека от враждебной строки.
const MAX_DEPTH: usize = 64;

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn skip(&mut self) {
        while self.s.get(self.i).is_some_and(|c| c.is_ascii_whitespace()) {
            self.i += 1;
        }
    }

    fn value(&mut self, depth: usize) -> Result<Wkt, String> {
        if depth > MAX_DEPTH {
            return Err("WKT: слишком глубокое вложение".into());
        }
        self.skip();
        match self.s.get(self.i) {
            Some(b'"') => {
                self.i += 1;
                let mut out = Vec::new();
                loop {
                    match self.s.get(self.i) {
                        // Кавычка внутри строки WKT удваивается.
                        Some(b'"') if self.s.get(self.i + 1) == Some(&b'"') => {
                            out.push(b'"');
                            self.i += 2;
                        }
                        Some(b'"') => {
                            self.i += 1;
                            break;
                        }
                        Some(c) => {
                            out.push(*c);
                            self.i += 1;
                        }
                        None => return Err("WKT: строка не закрыта".into()),
                    }
                }
                Ok(Wkt::Text(String::from_utf8_lossy(&out).into_owned()))
            }
            Some(c) if c.is_ascii_digit() || *c == b'-' || *c == b'+' || *c == b'.' => {
                let start = self.i;
                while self.s.get(self.i).is_some_and(|c| c.is_ascii_digit() || b"+-.eE".contains(c)) {
                    self.i += 1;
                }
                let t = String::from_utf8_lossy(&self.s[start..self.i]);
                t.parse().map(Wkt::Number).map_err(|_| format!("WKT: не число {t}"))
            }
            Some(c) if c.is_ascii_alphabetic() => {
                let start = self.i;
                while self.s.get(self.i).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') {
                    self.i += 1;
                }
                let name = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
                self.skip();
                let open = match self.s.get(self.i) {
                    Some(b'[') => b']',
                    Some(b'(') => b')',
                    // Слово без скобок - перечисление, например NORTH.
                    _ => return Ok(Wkt::Text(name)),
                };
                self.i += 1;
                let mut args = Vec::new();
                loop {
                    self.skip();
                    if self.s.get(self.i) == Some(&open) {
                        self.i += 1;
                        break;
                    }
                    args.push(self.value(depth + 1)?);
                    self.skip();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(c) if *c == open => {}
                        _ => return Err("WKT: ожидалась запятая или скобка".into()),
                    }
                }
                Ok(Wkt::Node(name, args))
            }
            _ => Err("WKT: неожиданный знак".into()),
        }
    }
}

fn parse(text: &str) -> Result<Wkt, String> {
    let mut p = Parser { s: text.trim_end_matches('\0').as_bytes(), i: 0 };
    p.value(0)
}

/// Имя без регистра, пробелов, подчёркиваний и скобок - для сравнения
/// имён проекций и параметров WKT 1 и 2.
fn key(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

/// Проекция.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Method {
    /// Широта и долгота в градусах.
    Geographic,
    TransverseMercator,
    /// Коническая Ламберта, параллели φ1 и φ2 (у варианта с одной
    /// параллелью обе равны широте начала).
    Lambert,
    Albers,
    /// Web Mercator EPSG:3857: сфера радиусом большой полуоси.
    WebMercator,
}

/// Система координат облака.
#[derive(Debug, Clone, PartialEq)]
pub struct Crs {
    method: Method,
    /// Большая полуось и эксцентриситет эллипсоида.
    a: f64,
    e: f64,
    /// Широта и долгота начала, параллели в радианах.
    lat0: f64,
    lon0: f64,
    lat1: f64,
    lat2: f64,
    k0: f64,
    /// Ложные восток и север в метрах.
    fe: f64,
    fn_: f64,
    /// Метров в единице плановых координат и высоты.
    pub unit: f64,
    pub vertical_unit: f64,
    /// Код EPSG плановой системы, если он известен.
    pub epsg: Option<u32>,
}

/// Код EPSG узла: `AUTHORITY["EPSG","32631"]` или `ID["EPSG",32631]`.
fn epsg_of(node: &Wkt) -> Option<u32> {
    let id = node.child(&["AUTHORITY", "ID"])?;
    if !id.text(0)?.eq_ignore_ascii_case("EPSG") {
        return None;
    }
    let code = id.number(1)?;
    (code >= 1.0 && code < f64::from(u32::MAX)).then_some(code as u32)
}

/// Множитель единицы длины узла: `UNIT["metre",1]`, `LENGTHUNIT[...]`.
fn length_unit(node: &Wkt) -> Option<f64> {
    node.child(&["LENGTHUNIT", "UNIT"])?.number(1).filter(|v| v.is_finite() && *v > 0.0)
}

/// Радиан в единице угла узла, без неё - градус.
fn angle_unit(node: &Wkt) -> f64 {
    node.child(&["ANGLEUNIT"]).and_then(|u| u.number(1)).filter(|v| v.is_finite() && *v > 0.0).unwrap_or(1f64.to_radians())
}

impl Crs {
    /// Система из строки WKT версии 1 или 2.
    pub fn from_wkt(text: &str) -> Result<Self, String> {
        let root = parse(text)?;
        let compound = ["COMPD_CS", "COMPOUNDCRS"].iter().any(|n| root.name().eq_ignore_ascii_case(n));
        let (horizontal, vertical) = if compound {
            let h = root.child(&["PROJCS", "PROJCRS", "GEOGCS", "GEOGCRS", "GEODCRS", "GEOGRAPHICCRS", "PROJECTEDCRS"]);
            let v = root.child(&["VERT_CS", "VERTCRS", "VERTICALCRS"]);
            (h.ok_or("WKT: в составной системе нет плановой")?, v)
        } else {
            (&root, None)
        };
        let mut crs = Self::horizontal(horizontal)?;
        crs.vertical_unit = vertical.and_then(|v| length_unit(v).or_else(|| v.child(&["CS"]).and_then(length_unit))).unwrap_or(crs.unit);
        Ok(crs)
    }

    fn horizontal(node: &Wkt) -> Result<Self, String> {
        let name = node.name().to_ascii_uppercase();
        let projected = matches!(name.as_str(), "PROJCS" | "PROJCRS" | "PROJECTEDCRS");
        let geographic = matches!(name.as_str(), "GEOGCS" | "GEOGCRS" | "GEODCRS" | "GEOGRAPHICCRS");
        if !projected && !geographic {
            return Err(format!("WKT: система {name} не поддерживается"));
        }
        let geog = if projected { node.child(&["GEOGCS", "BASEGEOGCRS", "BASEGEODCRS"]).ok_or("WKT: нет базовой системы")? } else { node };
        let datum = geog.child(&["DATUM", "ENSEMBLE"]).unwrap_or(geog);
        let ellipsoid = datum.child(&["SPHEROID", "ELLIPSOID"]).or_else(|| geog.child(&["SPHEROID", "ELLIPSOID"])).ok_or("WKT: нет эллипсоида")?;
        let a = ellipsoid.number(1).ok_or("WKT: нет полуоси")? * length_unit(ellipsoid).unwrap_or(1.0);
        let inv_f = ellipsoid.number(2).ok_or("WKT: нет сжатия")?;
        if !(a.is_finite() && a > 1.0 && inv_f.is_finite() && inv_f >= 0.0) {
            return Err("WKT: неверный эллипсоид".into());
        }
        let f = if inv_f == 0.0 { 0.0 } else { 1.0 / inv_f };
        let e = (f * (2.0 - f)).sqrt();
        let mut crs = Self {
            method: Method::Geographic,
            a,
            e,
            lat0: 0.0,
            lon0: 0.0,
            lat1: 0.0,
            lat2: 0.0,
            k0: 1.0,
            fe: 0.0,
            fn_: 0.0,
            unit: 1.0,
            vertical_unit: 1.0,
            epsg: epsg_of(node),
        };
        if geographic {
            return Ok(crs);
        }
        // Единица плановых координат: у WKT 1 - UNIT узла, у WKT 2 -
        // LENGTHUNIT узла или его системы осей CS.
        let unit = node
            .args()
            .iter()
            .rev()
            .find(|a| a.name().eq_ignore_ascii_case("UNIT") || a.name().eq_ignore_ascii_case("LENGTHUNIT"))
            .and_then(|u| u.number(1))
            .or_else(|| node.child(&["CS"]).and_then(length_unit))
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(1.0);
        crs.unit = unit;
        let (method, params): (String, Vec<&Wkt>) = match node.child(&["PROJECTION"]) {
            Some(p) => (p.text(0).unwrap_or_default().to_string(), node.args().iter().filter(|a| a.name().eq_ignore_ascii_case("PARAMETER")).collect()),
            None => {
                let conv = node.child(&["CONVERSION", "DERIVINGCONVERSION"]).ok_or("WKT: нет проекции")?;
                let m = conv.child(&["METHOD", "PROJECTION"]).and_then(|m| m.text(0)).unwrap_or_default().to_string();
                (m, conv.args().iter().filter(|a| a.name().eq_ignore_ascii_case("PARAMETER")).collect())
            }
        };
        let wkt2 = node.child(&["PROJECTION"]).is_none();
        // Значение параметра: углы - в радианах, длины - в метрах.
        let param = |names: &[&str], angle: bool| -> Option<f64> {
            let p = params.iter().find(|p| p.text(0).is_some_and(|n| names.contains(&key(n).as_str())))?;
            let v = p.number(1)?;
            let v = if angle {
                if wkt2 { v * angle_unit(p) } else { v.to_radians() }
            } else if wkt2 {
                v * length_unit(p).unwrap_or(1.0)
            } else {
                v * unit
            };
            v.is_finite().then_some(v)
        };
        let lat0 = param(&["latitudeoforigin", "latitudeofnaturalorigin", "latitudeoffalseorigin", "latitudeofcenter"], true);
        let lon0 = param(&["centralmeridian", "longitudeofnaturalorigin", "longitudeoffalseorigin", "longitudeofcenter", "longitudeoforigin"], true);
        let lat1 = param(&["standardparallel1", "latitudeof1ststandardparallel"], true);
        let lat2 = param(&["standardparallel2", "latitudeof2ndstandardparallel"], true);
        crs.lat0 = lat0.unwrap_or(0.0);
        crs.lon0 = lon0.unwrap_or(0.0);
        crs.k0 = param(&["scalefactor", "scalefactoratnaturalorigin"], false).map(|v| if wkt2 { v } else { v / unit }).unwrap_or(1.0);
        crs.fe = param(&["falseeasting", "eastingatfalseorigin"], false).unwrap_or(0.0);
        crs.fn_ = param(&["falsenorthing", "northingatfalseorigin"], false).unwrap_or(0.0);
        let m = key(&method);
        if m.contains("transversemercator") || m.contains("gausskruger") {
            crs.method = Method::TransverseMercator;
        } else if m.contains("lambertconformalconic") || m.contains("lambertconicconformal") {
            crs.method = Method::Lambert;
            match (lat1, lat2) {
                (Some(l1), Some(l2)) => (crs.lat1, crs.lat2) = (l1, l2),
                // Одна параллель: широта начала, масштаб на ней k0.
                _ => (crs.lat1, crs.lat2) = (crs.lat0, crs.lat0),
            }
        } else if m.contains("pseudomercator") || m.contains("mercatorauxiliarysphere") {
            crs.method = Method::WebMercator;
        } else if m.contains("albers") {
            crs.method = Method::Albers;
            crs.lat1 = lat1.ok_or("WKT: у проекции Альберса нет параллели")?;
            crs.lat2 = lat2.unwrap_or(crs.lat1);
        } else {
            return Err(format!("проекция {method} не поддерживается"));
        }
        if !(crs.k0.is_finite() && crs.k0 > 0.0) {
            return Err("WKT: неверный масштаб".into());
        }
        Ok(crs)
    }

    /// Широта и долгота в градусах точки (x, y) в единицах системы.
    /// None - точка вне области проекции.
    pub fn to_lat_lon(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let (x, y) = (x * self.unit - self.fe, y * self.unit - self.fn_);
        let (lat, lon) = match self.method {
            // Широта и долгота - в градусах, единица длины не участвует.
            Method::Geographic => ((y / self.unit).to_radians(), (x / self.unit).to_radians()),
            Method::TransverseMercator => self.tm_inverse(x, y),
            Method::Lambert => self.lcc_inverse(x, y)?,
            Method::Albers => self.albers_inverse(x, y)?,
            Method::WebMercator => (std::f64::consts::FRAC_PI_2 - 2.0 * (-y / self.a).exp().atan(), x / self.a),
        };
        let lon = (self.lon0 + lon).to_degrees();
        let lon = (lon + 180.0).rem_euclid(360.0) - 180.0;
        let lat = lat.to_degrees();
        (lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0).then_some((lat, lon))
    }

    /// Точка (x, y) в единицах системы для широты и долготы в градусах.
    /// None - точка вне области проекции.
    pub fn from_lat_lon(&self, lat: f64, lon: f64) -> Option<(f64, f64)> {
        if !(lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0) {
            return None;
        }
        if self.method == Method::Geographic {
            return Some((lon, lat));
        }
        let phi = lat.to_radians();
        // Разность долгот в пределах ±π.
        let dlon = (lon.to_radians() - self.lon0 + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
        let (x, y) = match self.method {
            Method::Geographic => return None,
            Method::TransverseMercator => self.tm_forward(phi, dlon),
            Method::Lambert => self.lcc_forward(phi, dlon)?,
            Method::Albers => self.albers_forward(phi, dlon)?,
            Method::WebMercator => {
                // Полюс в Web Mercator уходит в бесконечность.
                if lat.abs() >= 89.999 {
                    return None;
                }
                (self.a * dlon, self.a * (std::f64::consts::FRAC_PI_4 + phi / 2.0).tan().ln())
            }
        };
        let (x, y) = ((x + self.fe) / self.unit, (y + self.fn_) / self.unit);
        (x.is_finite() && y.is_finite()).then_some((x, y))
    }

    /// Географическая система - координаты в градусах.
    pub fn is_geographic(&self) -> bool {
        self.method == Method::Geographic
    }

    /// Ряды Крюгера: n, a·(1 + n²/4 + ...)/(1 + n), α и β.
    fn kruger(&self) -> (f64, [f64; 6], [f64; 6]) {
        let f = 1.0 - (1.0 - self.e * self.e).sqrt();
        let n = f / (2.0 - f);
        let (n2, n3, n4, n5, n6) = (n * n, n.powi(3), n.powi(4), n.powi(5), n.powi(6));
        let ar = self.a / (1.0 + n) * (1.0 + n2 / 4.0 + n4 / 64.0 + n6 / 256.0);
        let alpha = [
            n / 2.0 - 2.0 * n2 / 3.0 + 5.0 * n3 / 16.0 + 41.0 * n4 / 180.0 - 127.0 * n5 / 288.0 + 7891.0 * n6 / 37800.0,
            13.0 * n2 / 48.0 - 3.0 * n3 / 5.0 + 557.0 * n4 / 1440.0 + 281.0 * n5 / 630.0 - 1983433.0 * n6 / 1935360.0,
            61.0 * n3 / 240.0 - 103.0 * n4 / 140.0 + 15061.0 * n5 / 26880.0 + 167603.0 * n6 / 181440.0,
            49561.0 * n4 / 161280.0 - 179.0 * n5 / 168.0 + 6601661.0 * n6 / 7257600.0,
            34729.0 * n5 / 80640.0 - 3418889.0 * n6 / 1995840.0,
            212378941.0 * n6 / 319334400.0,
        ];
        let beta = [
            n / 2.0 - 2.0 * n2 / 3.0 + 37.0 * n3 / 96.0 - n4 / 360.0 - 81.0 * n5 / 512.0 + 96199.0 * n6 / 604800.0,
            n2 / 48.0 + n3 / 15.0 - 437.0 * n4 / 1440.0 + 46.0 * n5 / 105.0 - 1118711.0 * n6 / 3870720.0,
            17.0 * n3 / 480.0 - 37.0 * n4 / 840.0 - 209.0 * n5 / 4480.0 + 5569.0 * n6 / 90720.0,
            4397.0 * n4 / 161280.0 - 11.0 * n5 / 504.0 - 830251.0 * n6 / 7257600.0,
            4583.0 * n5 / 161280.0 - 108847.0 * n6 / 3991680.0,
            20648693.0 * n6 / 638668800.0,
        ];
        (ar, alpha, beta)
    }

    fn tau_prime(&self, tau: f64) -> f64 {
        let e = self.e;
        let sigma = (e * (e * tau / 1f64.hypot(tau)).atanh()).sinh();
        tau * 1f64.hypot(sigma) - sigma * 1f64.hypot(tau)
    }

    /// Север начала проекции на сфере ξ: дуга меридиана до широты начала.
    fn xi0(&self, alpha: &[f64; 6]) -> f64 {
        let tp = self.tau_prime(self.lat0.tan());
        let xi_p = tp.atan();
        let mut xi = xi_p;
        for (j, a) in alpha.iter().enumerate() {
            xi += a * (2.0 * (j as f64 + 1.0) * xi_p).sin();
        }
        xi
    }

    /// Поперечная Меркатора вперёд: метры от начала без ложных сдвигов.
    fn tm_forward(&self, phi: f64, dlon: f64) -> (f64, f64) {
        let (ar, alpha, _) = self.kruger();
        let tp = self.tau_prime(phi.tan());
        let (xi_p, eta_p) = (tp.atan2(dlon.cos()), (dlon.sin() / tp.hypot(dlon.cos())).asinh());
        let (mut xi, mut eta) = (xi_p, eta_p);
        for (j, a) in alpha.iter().enumerate() {
            let k = 2.0 * (j as f64 + 1.0);
            xi += a * (k * xi_p).sin() * (k * eta_p).cosh();
            eta += a * (k * xi_p).cos() * (k * eta_p).sinh();
        }
        (self.k0 * ar * eta, self.k0 * ar * (xi - self.xi0(&alpha)))
    }

    /// Поперечная Меркатора обратно: широта и разность долгот в радианах.
    fn tm_inverse(&self, x: f64, y: f64) -> (f64, f64) {
        let (ar, alpha, beta) = self.kruger();
        let xi = y / (self.k0 * ar) + self.xi0(&alpha);
        let eta = x / (self.k0 * ar);
        let (mut xi_p, mut eta_p) = (xi, eta);
        for (j, b) in beta.iter().enumerate() {
            let k = 2.0 * (j as f64 + 1.0);
            xi_p -= b * (k * xi).sin() * (k * eta).cosh();
            eta_p -= b * (k * xi).cos() * (k * eta).sinh();
        }
        let tp = xi_p.sin() / eta_p.sinh().hypot(xi_p.cos());
        let dlon = eta_p.sinh().atan2(xi_p.cos());
        let e2 = self.e * self.e;
        let mut tau = tp;
        for _ in 0..10 {
            let tpi = self.tau_prime(tau);
            let delta = (tp - tpi) / 1f64.hypot(tpi) * (1.0 + (1.0 - e2) * tau * tau) / ((1.0 - e2) * 1f64.hypot(tau));
            tau += delta;
            if delta.abs() < 1e-14 {
                break;
            }
        }
        (tau.atan(), dlon)
    }

    /// m Снайдера (14-15): cos φ / √(1 − e² sin² φ).
    fn m(&self, phi: f64) -> f64 {
        phi.cos() / (1.0 - (self.e * phi.sin()).powi(2)).sqrt()
    }

    /// t Снайдера (15-9).
    fn t(&self, phi: f64) -> f64 {
        let es = self.e * phi.sin();
        (std::f64::consts::FRAC_PI_4 - phi / 2.0).tan() / ((1.0 - es) / (1.0 + es)).powf(self.e / 2.0)
    }

    /// Широта по t (7-9), итерациями.
    fn phi_from_t(&self, t: f64) -> Option<f64> {
        let mut phi = std::f64::consts::FRAC_PI_2 - 2.0 * t.atan();
        for _ in 0..15 {
            let es = self.e * phi.sin();
            let next = std::f64::consts::FRAC_PI_2 - 2.0 * (t * ((1.0 - es) / (1.0 + es)).powf(self.e / 2.0)).atan();
            if (next - phi).abs() < 1e-12 {
                return Some(next);
            }
            phi = next;
        }
        phi.is_finite().then_some(phi)
    }

    /// Постоянные конической Ламберта: n, F и ρ0 (15-8 ... 15-10).
    fn lcc_constants(&self) -> Option<(f64, f64, f64)> {
        let (m1, m2) = (self.m(self.lat1), self.m(self.lat2));
        let (t0, t1, t2) = (self.t(self.lat0), self.t(self.lat1), self.t(self.lat2));
        let n = if (self.lat1 - self.lat2).abs() < 1e-10 { self.lat1.sin() } else { (m1.ln() - m2.ln()) / (t1.ln() - t2.ln()) };
        if n.abs() < 1e-12 {
            return None;
        }
        let f = m1 / (n * t1.powf(n));
        Some((n, f, self.a * f * t0.powf(n) * self.k0))
    }

    /// Коническая Ламберта вперёд (15-1 ... 15-7).
    fn lcc_forward(&self, phi: f64, dlon: f64) -> Option<(f64, f64)> {
        let (n, f, rho0) = self.lcc_constants()?;
        let rho = self.a * self.k0 * f * self.t(phi).powf(n);
        let theta = n * dlon;
        Some((rho * theta.sin(), rho0 - rho * theta.cos()))
    }

    /// Коническая Ламберта обратно (15-1 ... 15-11).
    fn lcc_inverse(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let (n, f, rho0) = self.lcc_constants()?;
        let dy = rho0 - y;
        let rho = n.signum() * x.hypot(dy);
        let theta = if n > 0.0 { x.atan2(dy) } else { (-x).atan2(-dy) };
        if rho == 0.0 {
            return Some((std::f64::consts::FRAC_PI_2 * n.signum(), 0.0));
        }
        let t = (rho / (self.a * self.k0 * f)).powf(1.0 / n);
        Some((self.phi_from_t(t)?, theta / n))
    }

    /// q Снайдера (3-12).
    fn q(&self, phi: f64) -> f64 {
        let e = self.e;
        let s = phi.sin();
        if e < 1e-12 {
            return 2.0 * s;
        }
        (1.0 - e * e) * (s / (1.0 - (e * s).powi(2)) - (1.0 / (2.0 * e)) * ((1.0 - e * s) / (1.0 + e * s)).ln())
    }

    /// Постоянные равновеликой Альберса: n, C и ρ0 (14-12 ... 14-14).
    fn albers_constants(&self) -> Option<(f64, f64, f64)> {
        let (m1, m2) = (self.m(self.lat1), self.m(self.lat2));
        let (q0, q1, q2) = (self.q(self.lat0), self.q(self.lat1), self.q(self.lat2));
        let n = if (self.lat1 - self.lat2).abs() < 1e-10 { self.lat1.sin() } else { (m1 * m1 - m2 * m2) / (q2 - q1) };
        if n.abs() < 1e-12 {
            return None;
        }
        let c = m1 * m1 + n * q1;
        Some((n, c, self.a * (c - n * q0).max(0.0).sqrt() / n))
    }

    /// Равновеликая Альберса вперёд (14-1 ... 14-4).
    fn albers_forward(&self, phi: f64, dlon: f64) -> Option<(f64, f64)> {
        let (n, c, rho0) = self.albers_constants()?;
        let rho = self.a * (c - n * self.q(phi)).max(0.0).sqrt() / n;
        let theta = n * dlon;
        Some((rho * theta.sin(), rho0 - rho * theta.cos()))
    }

    /// Равновеликая Альберса обратно (14-1 ... 14-21).
    fn albers_inverse(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let e = self.e;
        let (n, c, rho0) = self.albers_constants()?;
        let dy = rho0 - y;
        let rho = x.hypot(dy);
        let theta = if n > 0.0 { x.atan2(dy) } else { (-x).atan2(-dy) };
        let q = (c - (rho * n / self.a).powi(2)) / n;
        // Широта по q итерациями (3-16).
        let mut phi = (q / 2.0).clamp(-1.0, 1.0).asin();
        for _ in 0..15 {
            let s = phi.sin();
            let es2 = (e * s).powi(2);
            let next = phi
                + (1.0 - es2).powi(2) / (2.0 * phi.cos())
                    * (q / (1.0 - e * e) - s / (1.0 - es2) + (1.0 / (2.0 * e.max(1e-12))) * ((1.0 - e * s) / (1.0 + e * s)).ln());
            if (next - phi).abs() < 1e-12 {
                phi = next;
                break;
            }
            phi = next;
        }
        phi.is_finite().then_some((phi, theta / n))
    }
}

/// Система по коду EPSG без строки WKT - для старых файлов LAS с ключами
/// GeoTIFF и для сборки без terrio ([`crate::crs`]). Известны UTM
/// на WGS84, NAD83 и ETRS89, Lambert-93, Web Mercator и широта
/// с долготой.
pub fn from_epsg(code: u32) -> Option<Crs> {
    const WGS84: (f64, f64) = (6_378_137.0, 298.257_223_563);
    const GRS80: (f64, f64) = (6_378_137.0, 298.257_222_101);
    let tm = |ell: (f64, f64), zone: u32, south: bool| {
        let lon0 = (f64::from(zone) - 1.0) * 6.0 - 180.0 + 3.0;
        format!(
            "PROJCS[\"UTM\",GEOGCS[\"g\",DATUM[\"d\",SPHEROID[\"s\",{},{}]]],PROJECTION[\"Transverse_Mercator\"],PARAMETER[\"latitude_of_origin\",0],PARAMETER[\"central_meridian\",{lon0}],PARAMETER[\"scale_factor\",0.9996],PARAMETER[\"false_easting\",500000],PARAMETER[\"false_northing\",{}],UNIT[\"metre\",1],AUTHORITY[\"EPSG\",\"{code}\"]]",
            ell.0,
            ell.1,
            if south { 10_000_000 } else { 0 }
        )
    };
    let wkt = match code {
        4326 | 4269 | 4258 | 4171 => format!("GEOGCS[\"g\",DATUM[\"d\",SPHEROID[\"s\",{},{}]],AUTHORITY[\"EPSG\",\"{code}\"]]", WGS84.0, WGS84.1),
        // Web Mercator и его старые коды.
        3857 | 3785 | 900_913 | 102_100 => format!(
            "PROJCS[\"WGS 84 / Pseudo-Mercator\",GEOGCS[\"g\",DATUM[\"d\",SPHEROID[\"s\",{},{}]]],PROJECTION[\"Popular_Visualisation_Pseudo_Mercator\"],PARAMETER[\"central_meridian\",0],PARAMETER[\"false_easting\",0],PARAMETER[\"false_northing\",0],UNIT[\"metre\",1],AUTHORITY[\"EPSG\",\"3857\"]]",
            WGS84.0, WGS84.1
        ),
        32601..=32660 => tm(WGS84, code - 32600, false),
        32701..=32760 => tm(WGS84, code - 32700, true),
        // NAD83, NAD83(CSRS), NAD83(2011), ETRS89, GDA94, GDA2020.
        26901..=26923 => tm(GRS80, code - 26900, false),
        3154..=3160 => tm(GRS80, code - 3147, false),
        2955..=2957 => tm(GRS80, code - 2944, false),
        2958..=2962 => tm(GRS80, code - 2941, false),
        3761 => tm(GRS80, 22, false),
        6328 | 6329 => tm(GRS80, code - 6269, false),
        6330..=6348 => tm(GRS80, code - 6329, false),
        25828..=25838 => tm(GRS80, code - 25800, false),
        28348..=28358 => tm(GRS80, code - 28300, true),
        7846..=7859 => tm(GRS80, code - 7800, true),
        2154 => format!(
            "PROJCS[\"RGF93 / Lambert-93\",GEOGCS[\"g\",DATUM[\"d\",SPHEROID[\"s\",{},{}]]],PROJECTION[\"Lambert_Conformal_Conic_2SP\"],PARAMETER[\"standard_parallel_1\",49],PARAMETER[\"standard_parallel_2\",44],PARAMETER[\"latitude_of_origin\",46.5],PARAMETER[\"central_meridian\",3],PARAMETER[\"false_easting\",700000],PARAMETER[\"false_northing\",6600000],UNIT[\"metre\",1],AUTHORITY[\"EPSG\",\"2154\"]]",
            GRS80.0, GRS80.1
        ),
        _ => return None,
    };
    Crs::from_wkt(&wkt).ok()
}
