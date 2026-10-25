// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Объекты глубокого неба: каталог, подписи, поиск имени, перенос
//! `core/deepsky.py` модуля.
//!
//! Каталог data/deepsky.json модуля собран из OpenNGC (CC BY-SA 4.0) -
//! все объекты Мессье и яркие NGC и IC. Lite берёт его из хранилища
//! модуля, как каталог звёзд. Объект вне каталога ищет служба имён
//! Sesame CDS ([`sesame_url`], [`parse_sesame`]).

pub const URL: &str = "https://raw.githubusercontent.com/Valery35/planetx/main/planetx/data/deepsky.json";
pub const ATTRIBUTION: (&str, &str) = ("Deep sky: OpenNGC, CC BY-SA 4.0", "https://github.com/mattiaverga/OpenNGC");
pub const SESAME: &str = "https://cds.unistra.fr/cgi-bin/nph-sesame/-oI/SNV?";
pub const SIMBAD: &str = "https://simbad.cds.unistra.fr/simbad/sim-id?Ident=";

/// Объекты NGC и IC подписываются при поле зрения не шире NGC_FOV,
/// объекты Мессье - при любом. Выбор помощника модуля.
pub const NGC_FOV: f64 = 40.0;
/// Поле зрения перелёта к объекту - FIT размеров объекта в пределах
/// FOV_RANGE, градусы. Объект без размера - FOV_UNKNOWN.
pub const FIT: f64 = 4.0;
pub const FOV_RANGE: (f64, f64) = (0.5, 30.0);
pub const FOV_UNKNOWN: f64 = 2.0;

/// Объект каталога. label - M31, NGC 7000; ra, dec - градусы J2000; mag -
/// звёздная величина; size - больший поперечник в угловых минутах; kind -
/// вид OpenNGC (G, OCl, GCl, PN...); constellation - три буквы; en, ru -
/// общее название или пусто; other - обозначение NGC или IC объекта
/// Мессье или пусто.
#[derive(Debug, Clone, PartialEq)]
pub struct DeepObject {
    pub label: String,
    pub ra: f64,
    pub dec: f64,
    pub mag: Option<f64>,
    pub size: Option<f64>,
    pub kind: String,
    pub constellation: String,
    pub en: String,
    pub ru: String,
    pub other: String,
}

/// Виды OpenNGC: ключ и название по-английски, ключ перевода.
pub const KINDS: [(&str, &str); 18] = [
    ("G", "Galaxy"),
    ("GPair", "Galaxy pair"),
    ("GTrpl", "Galaxy triplet"),
    ("GGroup", "Galaxy group"),
    ("OCl", "Open cluster"),
    ("GCl", "Globular cluster"),
    ("Cl+N", "Cluster with nebula"),
    ("PN", "Planetary nebula"),
    ("Neb", "Nebula"),
    ("HII", "HII region"),
    ("EmN", "Emission nebula"),
    ("RfN", "Reflection nebula"),
    ("DrkN", "Dark nebula"),
    ("SNR", "Supernova remnant"),
    ("*Ass", "Stellar association"),
    ("**", "Double star"),
    ("*", "Star"),
    ("Other", "Other"),
];

/// Объекты каталога из байтов deepsky.json. Строка не того вида
/// пропускается.
pub fn parse(data: &[u8]) -> Result<Vec<DeepObject>, String> {
    let value: serde_json::Value = serde_json::from_slice(data).map_err(|e| e.to_string())?;
    let rows = value.get("objects").and_then(serde_json::Value::as_array).ok_or("no objects")?;
    let text = |v: &serde_json::Value| v.as_str().unwrap_or("").to_string();
    let number = |v: &serde_json::Value| v.as_f64().filter(|x| x.is_finite());
    Ok(rows
        .iter()
        .filter_map(|row| {
            let r = row.as_array().filter(|r| r.len() >= 10)?;
            let (ra, dec) = (number(&r[1])?, number(&r[2])?);
            Some(DeepObject {
                label: r[0].as_str()?.to_string(),
                ra,
                dec,
                mag: number(&r[3]),
                size: number(&r[4]).filter(|s| *s > 0.0),
                kind: text(&r[5]),
                constellation: text(&r[6]),
                en: text(&r[7]),
                ru: text(&r[8]),
                other: text(&r[9]),
            })
        })
        .collect())
}

pub fn is_messier(obj: &DeepObject) -> bool {
    obj.label.strip_prefix('M').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// Название вида по-английски, ключ перевода.
pub fn kind_name(kind: &str) -> &'static str {
    KINDS.iter().find(|(k, _)| *k == kind).map_or("Other", |(_, name)| name)
}

/// Подпись на небе: название на языке, иначе обозначение. У прочих
/// языков, кроме русского, - английское название, как у модуля.
pub fn title(obj: &DeepObject, russian: bool) -> &str {
    let name = if russian { &obj.ru } else { &obj.en };
    if name.is_empty() { &obj.label } else { name }
}

/// Имена, по которым объект находится строкой «Поиск»: обозначение слитно
/// и через пробел, NGC или IC объекта Мессье, общие названия.
pub fn names(obj: &DeepObject) -> Vec<String> {
    let mut out = vec![obj.label.clone(), obj.label.replace(' ', "")];
    if is_messier(obj) {
        out.push(format!("M {}", &obj.label[1..]));
    }
    if !obj.other.is_empty() {
        out.push(obj.other.clone());
        out.push(obj.other.replace(' ', ""));
    }
    out.extend([&obj.ru, &obj.en].into_iter().filter(|n| !n.is_empty()).cloned());
    let mut seen = std::collections::HashSet::new();
    out.retain(|n| seen.insert(n.to_lowercase()));
    out
}

/// Подписывается ли объект при поле зрения fov, градусы.
pub fn shown(obj: &DeepObject, fov: f64) -> bool {
    is_messier(obj) || fov <= NGC_FOV
}

/// Старшинство подписи - меньше раньше: звёзды 10 + блеск, созвездия
/// 20 + ранг, Мессье 15 + блеск, прочие 30 + блеск.
pub fn rank(obj: &DeepObject) -> f64 {
    (if is_messier(obj) { 15.0 } else { 30.0 }) + obj.mag.unwrap_or(12.0)
}

/// Поле зрения перелёта к объекту, градусы.
pub fn fov_for(obj: &DeepObject) -> f64 {
    obj.size.map_or(FOV_UNKNOWN, |s| (FIT * s / 60.0).clamp(FOV_RANGE.0, FOV_RANGE.1))
}

/// Половина поперечника объекта в пикселях кадра, без размера - 0.
pub fn radius_pixels(obj: &DeepObject, radians_per_pixel: f64) -> f64 {
    match obj.size {
        Some(s) if radians_per_pixel > 0.0 => (s / 120.0).to_radians() / radians_per_pixel,
        _ => 0.0,
    }
}

/// Прямое восхождение в часах, минутах и секундах: 0h 42m 44s.
pub fn ra_text(ra: f64) -> String {
    let seconds = ((ra.rem_euclid(360.0) / 15.0 * 3600.0).round() as i64).rem_euclid(86400);
    format!("{}h {:02}m {:02}s", seconds / 3600, seconds / 60 % 60, seconds % 60)
}

/// Склонение в градусах, минутах и секундах: +41° 16′ 07″.
pub fn dec_text(dec: f64) -> String {
    let seconds = (dec.abs() * 3600.0).round() as i64;
    format!("{}{}° {:02}′ {:02}″", if dec < 0.0 { "-" } else { "+" }, seconds / 3600, seconds / 60 % 60, seconds % 60)
}

/// Кодирование для адреса: всё, кроме букв, цифр и -_.~, - %XX.
fn quote(text: &str) -> String {
    text.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~/".contains(&b) { char::from(b).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// Запрос службы имён Sesame CDS: Simbad, NED, VizieR.
pub fn sesame_url(name: &str) -> String {
    format!("{SESAME}{}", quote(name.trim()))
}

pub fn simbad_url(obj: &DeepObject) -> String {
    format!("{SIMBAD}{}", quote(if obj.other.is_empty() { &obj.label } else { &obj.other }))
}

/// Число в начале строки: знак, цифры, точка.
fn leading_number(text: &str) -> Option<(f64, &str)> {
    let text = text.trim_start();
    let end = text.char_indices().find(|(i, c)| !(c.is_ascii_digit() || *c == '.' || (*i == 0 && (*c == '+' || *c == '-')))).map_or(text.len(), |(i, _)| i);
    Some((text[..end].parse().ok()?, &text[end..]))
}

/// Ответ Sesame в (RA°, Dec°, имя) первого найденного или None.
pub fn parse_sesame(text: &str) -> Option<(f64, f64, String)> {
    let line = text.lines().find_map(|l| l.strip_prefix("%J").filter(|rest| rest.starts_with(char::is_whitespace)))?;
    let (ra, rest) = leading_number(line)?;
    let (dec, _) = leading_number(rest)?;
    if !((0.0..=360.0).contains(&ra) && (-90.0..=90.0).contains(&dec)) {
        return None;
    }
    let name = text.lines().find_map(|l| l.strip_prefix("%I.0").filter(|r| r.starts_with(char::is_whitespace))).map_or(String::new(), |n| n.split_whitespace().collect::<Vec<_>>().join(" "));
    Some((ra, dec, name))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// Каталог из рабочей копии модуля или None, если её нет.
    fn catalogue() -> Option<Vec<DeepObject>> {
        let bytes = std::fs::read(r"C:\Dev\planetx\planetx\data\deepsky.json").ok()?;
        Some(parse(&bytes).unwrap())
    }

    fn by_label<'a>(all: &'a [DeepObject], label: &str) -> &'a DeepObject {
        all.iter().find(|o| o.label == label).unwrap()
    }

    const SAMPLE: &str = r#"{"objects":[["M31",10.68471,41.26875,3.44,177.83,"G","And","Andromeda Galaxy","Галактика Андромеды","NGC 224"],
        ["M40",185.55208,58.08306,null,null,"**","UMa","","",""],
        ["NGC 7000",314.69375,44.32,4.0,120.0,"HII","Cyg","North America Nebula","Туманность Северная Америка",""],
        ["bad"]]}"#;

    #[test]
    fn messier_and_famous_objects() {
        let Some(all) = catalogue() else {
            eprintln!("каталога модуля нет, проверка пропущена");
            return;
        };
        assert_eq!(all.iter().filter(|o| is_messier(o)).count(), 109);
        let m31 = by_label(&all, "M31");
        assert!((m31.ra - 10.685).abs() < 0.01 && (m31.dec - 41.269).abs() < 0.01);
        assert_eq!(m31.other, "NGC 224");
        assert_eq!(m31.ru, "Галактика Андромеды");
        assert_eq!(by_label(&all, "M42").ru, "Туманность Ориона");
        assert!(all.iter().any(|o| o.label == "NGC 7000"));
        // Звёзд без номера Мессье в каталоге нет.
        assert!(all.iter().filter(|o| o.kind == "*" || o.kind == "**").all(is_messier));
    }

    #[test]
    fn names_and_title() {
        let all = parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(all.len(), 3);
        let m31 = by_label(&all, "M31");
        let names = names(m31);
        for name in ["M31", "M 31", "NGC 224", "NGC224", "Галактика Андромеды", "Andromeda Galaxy"] {
            assert!(names.iter().any(|n| n == name), "{name}");
        }
        assert_eq!(title(m31, false), "Andromeda Galaxy");
        assert_eq!(title(by_label(&all, "M40"), true), "M40");
        assert_eq!(kind_name("GCl"), "Globular cluster");
        assert_eq!(kind_name("xyz"), "Other");
    }

    #[test]
    fn ngc_only_when_close() {
        let all = parse(SAMPLE.as_bytes()).unwrap();
        let (m31, ngc) = (by_label(&all, "M31"), by_label(&all, "NGC 7000"));
        assert!(shown(m31, 100.0));
        assert!(!shown(ngc, NGC_FOV + 1.0));
        assert!(shown(ngc, NGC_FOV));
        assert!(rank(m31) < rank(ngc));
    }

    #[test]
    fn fov_and_radius() {
        let all = parse(SAMPLE.as_bytes()).unwrap();
        let m31 = by_label(&all, "M31");
        let size = m31.size.unwrap();
        assert!((fov_for(m31) - FIT * size / 60.0).abs() < 1e-12);
        assert_eq!(fov_for(by_label(&all, "M40")), FOV_UNKNOWN);
        let r = radius_pixels(m31, 0.01f64.to_radians());
        assert!((r - size / 120.0 / 0.01).abs() < 1e-3);
    }

    #[test]
    fn coordinates_text() {
        assert_eq!(ra_text(10.68470833), "0h 42m 44s");
        assert_eq!(dec_text(-5.3876), "-5° 23′ 15″");
    }

    #[test]
    fn sesame() {
        let answer = "# M31\t#Q12499889\n#=Sc=Simbad (CDS, via client/server):    1   290ms\n%@ @1575544\n%C.0 AGN\n%J 10.68470833 +41.26875000 = 00 42 44.330  +41 16 07.50\n%I.0 M  31\n%I NGC 224\n";
        let (ra, dec, name) = parse_sesame(answer).unwrap();
        assert!((ra - 10.68470833).abs() < 1e-9 && (dec - 41.26875).abs() < 1e-9);
        assert_eq!(name, "M 31");
        assert!(parse_sesame("# xyz\n#! *** Nothing found").is_none());
        assert!(parse_sesame("").is_none());
        assert_eq!(sesame_url(" Orion Nebula"), format!("{SESAME}Orion%20Nebula"));
    }
}
