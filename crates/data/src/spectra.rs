// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Минералы по спектру: спектр пикселя снимка сравнивается с эталонами
//! спектральной библиотеки USGS Spectral Library Version 7. Перенос
//! `core/spectra.py` модуля.
//!
//! Библиотека - `assets/spectra.json` из `tools/build_spectra.py` модуля,
//! общественное достояние США. У каждого эталона значения, пересчитанные
//! USGS на каналы ASTER, Landsat 8 и Sentinel-2. Каналы растра узнаются
//! по длине волны из подписи канала: «B3N 0.82 um» файлов ASTER, имена
//! каналов «red», «swir16» файлов Sentinel-2 и Landsat, число с единицей
//! нм или мкм. Канал сопоставляется с ближайшим каналом датчика не дальше
//! [`TOLERANCE`] доли длины волны, тепловые каналы не участвуют.
//!
//! Мера - угол между спектрами (Spectral Angle Mapper, Kruse и др. 1993):
//! arccos(a·b / |a||b|). Угол не зависит от общей яркости, поэтому склон
//! в тени и на солнце даёт один ответ. Клетка получает эталон
//! с наименьшим углом, если он не больше порога, иначе остаётся без
//! класса. У страницы потоков нет, поэтому классы считаются шагами
//! [`Classify::step`].

use std::sync::OnceLock;

/// Доля длины волны, в которой каналы считаются одним.
pub const TOLERANCE: f64 = 0.06;
/// Совпавших каналов не меньше.
pub const MIN_BANDS: usize = 3;
/// Порог угла по умолчанию, градусы.
pub const LIMIT: f64 = 6.0;
/// Группы эталонов в порядке списка окна.
pub const GROUPS: [&str; 6] = ["minerals", "soils", "vegetation", "coatings", "artificial", "liquids"];
/// Длины волн, мкм, по именам каналов файлов модуля.
const KEYS: [(&str, f64); 12] = [
    ("coastal", 0.443),
    ("blue", 0.49),
    ("green", 0.56),
    ("red", 0.665),
    ("rededge1", 0.705),
    ("rededge2", 0.74),
    ("rededge3", 0.783),
    ("nir", 0.842),
    ("nir08", 0.865),
    ("nir09", 0.945),
    ("swir16", 1.61),
    ("swir22", 2.19),
];
/// Номера каналов датчиков в порядке библиотеки: подписи «B8A», «Band 3N».
const NUMBERS: [(&str, &[&str]); 3] = [
    ("aster", &["1", "2", "3N", "4", "5", "6", "7", "8", "9"]),
    ("landsat8", &["1", "2", "3", "4", "5", "6", "7"]),
    ("sentinel2", &["1", "2", "3", "4", "5", "6", "7", "8", "8A", "9", "10", "11", "12"]),
];
/// Текст библиотеки.
const DATA: &str = include_str!("../assets/spectra.json");

/// Датчик библиотеки: ключ, название, длины волн каналов, мкм.
#[derive(Debug, Clone, PartialEq)]
pub struct Sensor {
    pub key: String,
    pub name: String,
    pub um: Vec<f64>,
}

/// Эталон: номер, название, группа, прибор, вид и значения по датчикам
/// в порядке [`Library::sensors`], None - у датчика значений нет.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: usize,
    pub name: String,
    pub group: String,
    pub instrument: String,
    pub kind: String,
    pub values: Vec<Option<Vec<f64>>>,
}

/// Библиотека эталонов.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Library {
    pub sensors: Vec<Sensor>,
    pub spectra: Vec<Entry>,
    /// Подпись источника.
    pub source: String,
}

/// Встроенная библиотека, разбирается один раз.
pub fn library() -> &'static Library {
    static LIBRARY: OnceLock<Library> = OnceLock::new();
    LIBRARY.get_or_init(|| Library::parse(DATA).unwrap_or_default())
}

impl Library {
    /// Библиотека из текста JSON. None - текст не того устройства.
    pub fn parse(text: &str) -> Option<Self> {
        let root: serde_json::Value = serde_json::from_str(text).ok()?;
        let numbers = |v: &serde_json::Value| -> Option<Vec<f64>> { v.as_array()?.iter().map(serde_json::Value::as_f64).collect() };
        let mut sensors = Vec::new();
        for (key, item) in root.get("sensors")?.as_object()? {
            sensors.push(Sensor { key: key.clone(), name: item.get("name")?.as_str()?.to_string(), um: numbers(item.get("um")?)? });
        }
        let text_of = |v: &serde_json::Value, key: &str| v.get(key).and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
        let mut spectra = Vec::new();
        for item in root.get("spectra")?.as_array()? {
            let id = usize::try_from(item.get("id")?.as_u64()?).ok()?;
            let values = sensors.iter().map(|s| item.get(&s.key).and_then(numbers).filter(|v| v.len() == s.um.len())).collect();
            spectra.push(Entry { id, name: text_of(item, "name"), group: text_of(item, "group"), instrument: text_of(item, "instrument"), kind: text_of(item, "kind"), values });
        }
        Some(Self { sensors, spectra, source: text_of(&root, "source") })
    }

    /// Номер датчика по ключу.
    pub fn sensor(&self, key: &str) -> Option<usize> {
        self.sensors.iter().position(|s| s.key == key)
    }

    /// Эталон по номеру.
    pub fn entry(&self, id: usize) -> Option<&Entry> {
        self.spectra.get(id).filter(|e| e.id == id).or_else(|| self.spectra.iter().find(|e| e.id == id))
    }

    /// Эталоны ids на каналах bands датчика sensor: номера тех, у кого
    /// значения этого датчика есть, и их значения.
    pub fn values(&self, sensor: usize, ids: &[usize], bands: &[usize]) -> (Vec<usize>, Vec<Vec<f64>>) {
        let mut kept = Vec::new();
        let mut out = Vec::new();
        for &id in ids {
            if let Some(values) = self.entry(id).and_then(|e| e.values.get(sensor)?.as_ref()) {
                kept.push(id);
                out.push(bands.iter().map(|&j| values.get(j).copied().unwrap_or(f64::NAN)).collect());
            }
        }
        (kept, out)
    }
}

/// Длина волны канала, мкм, по подписи или None. Тепловой канал (подпись
/// с «, K» или длина больше 5 мкм) - None.
pub fn wavelength(label: &str) -> Option<f64> {
    if label.trim_end().ends_with(", K") {
        return None;
    }
    if let Some(value) = number_with_unit(label) {
        return (value > 0.3 && value < 5.0).then_some(value);
    }
    words(label).into_iter().find_map(|w| KEYS.iter().find(|(k, _)| w.eq_ignore_ascii_case(k)).map(|(_, v)| *v))
}

/// Первое число с единицей нм или мкм в тексте, мкм.
fn number_with_unit(text: &str) -> Option<f64> {
    let chars: Vec<char> = text.chars().collect();
    for start in 0..chars.len() {
        if !chars[start].is_ascii_digit() {
            continue;
        }
        let mut end = start;
        while chars.get(end).is_some_and(char::is_ascii_digit) {
            end += 1;
        }
        if matches!(chars.get(end), Some('.' | ',')) && chars.get(end + 1).is_some_and(char::is_ascii_digit) {
            end += 1;
            while chars.get(end).is_some_and(char::is_ascii_digit) {
                end += 1;
            }
        }
        let mut at = end;
        while chars.get(at).is_some_and(|c| c.is_whitespace()) {
            at += 1;
        }
        let rest: String = chars.get(at..).unwrap_or_default().iter().take(3).collect::<String>().to_lowercase();
        let scale = if rest.starts_with("nm") || rest.starts_with("нм") {
            1000.0
        } else if rest.starts_with("um") || rest.starts_with("µm") || rest.starts_with("мкм") {
            1.0
        } else {
            continue;
        };
        let digits: String = chars.get(start..end).unwrap_or_default().iter().map(|c| if *c == ',' { '.' } else { *c }).collect();
        return digits.parse::<f64>().ok().map(|v| v / scale);
    }
    None
}

/// Слова вида «буквы, потом цифры» подписи: «B4», «red», «swir16».
fn words(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let start = i;
        while chars.get(i).is_some_and(char::is_ascii_alphabetic) {
            i += 1;
        }
        while chars.get(i).is_some_and(char::is_ascii_digit) {
            i += 1;
        }
        out.push(chars[start..i].iter().collect());
    }
    out
}

/// Датчик и пары (канал растра, канал датчика).
pub type Pairs = (usize, Vec<(usize, usize)>);

/// Пары (канал растра, канал датчика) по длинам волн waves каналов
/// растра, None - неизвестна. sensor - датчик явно, без него - тот,
/// у кого совпало больше каналов, при равенстве - у кого количество
/// каналов ближе к растру (Landsat и Sentinel-2 делят многие длины волн),
/// потом - у кого каналы ближе. Итог - (датчик, пары) или None.
pub fn by_waves(library: &Library, waves: &[Option<f64>], sensor: Option<usize>) -> Option<Pairs> {
    let known = waves.iter().filter(|w| w.is_some()).count() as i64;
    let keys: Vec<usize> = sensor.map_or_else(|| (0..library.sensors.len()).collect(), |s| vec![s]);
    let mut best: Option<((usize, i64, f64), Pairs)> = None;
    for key in keys {
        let Some(um) = library.sensors.get(key).map(|s| &s.um) else {
            continue;
        };
        let (mut pairs, mut used, mut miss) = (Vec::new(), Vec::new(), 0.0);
        for (n, wave) in waves.iter().enumerate() {
            let Some(wave) = wave else {
                continue;
            };
            let Some((near, gap)) = um.iter().enumerate().map(|(j, u)| (j, (u - wave).abs() / wave)).min_by(|a, b| a.1.total_cmp(&b.1)) else {
                continue;
            };
            if gap <= TOLERANCE && !used.contains(&near) {
                pairs.push((n, near));
                used.push(near);
                miss += gap;
            }
        }
        let score = (pairs.len(), -(um.len() as i64 - known).abs(), -miss);
        let better = best.as_ref().is_none_or(|(s, _)| (score.0, score.1).cmp(&(s.0, s.1)).then(score.2.total_cmp(&s.2)).is_gt());
        if better {
            best = Some((score, (key, pairs)));
        }
    }
    best.map(|(_, found)| found).filter(|(_, pairs)| pairs.len() >= MIN_BANDS)
}

/// Номер канала датчика из подписи вида «B4», «B8A», «Band 3N», «b_07»:
/// граница слова, B, необязательное AND, пробел или подчёркивание, нули,
/// цифры, необязательные A или N, граница слова. Заглавными.
fn band_number(label: &str) -> Option<String> {
    let chars: Vec<char> = label.chars().collect();
    let word = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric() || *c == '_');
    for start in 0..chars.len() {
        if !chars[start].eq_ignore_ascii_case(&'b') || (start > 0 && word(chars.get(start - 1))) {
            continue;
        }
        let after_b = start + 1;
        let and: String = chars.get(after_b..after_b + 3).unwrap_or_default().iter().collect();
        for at in [and.eq_ignore_ascii_case("and").then_some(after_b + 3), Some(after_b)].into_iter().flatten() {
            for at in [matches!(chars.get(at), Some(' ' | '_')).then_some(at + 1), Some(at)].into_iter().flatten() {
                let mut digits_end = at;
                while chars.get(digits_end).is_some_and(char::is_ascii_digit) {
                    digits_end += 1;
                }
                if digits_end == at {
                    continue;
                }
                let raw: String = chars[at..digits_end].iter().collect();
                let trimmed = raw.trim_start_matches('0');
                let digits = if trimmed.is_empty() { "0" } else { trimmed };
                let letter = chars.get(digits_end).filter(|c| matches!(c.to_ascii_uppercase(), 'A' | 'N'));
                if let Some(l) = letter
                    && !word(chars.get(digits_end + 1))
                {
                    return Some(format!("{digits}{}", l.to_ascii_uppercase()));
                }
                if !word(chars.get(digits_end)) {
                    return Some(digits.to_string());
                }
            }
        }
    }
    None
}

/// Пары каналов по номерам в подписях для датчика sensor, иначе по
/// порядку каналов.
fn by_names(library: &Library, sensor: usize, labels: &[String]) -> Vec<(usize, usize)> {
    let key = library.sensors.get(sensor).map_or("", |s| s.key.as_str());
    let numbers: &[&str] = NUMBERS.iter().find(|(k, _)| *k == key).map_or(&[], |(_, n)| n);
    let size = library.sensors.get(sensor).map_or(0, |s| s.um.len());
    let (mut pairs, mut used) = (Vec::new(), Vec::new());
    for (n, label) in labels.iter().enumerate() {
        if let Some(j) = band_number(label).and_then(|b| numbers.iter().position(|x| *x == b))
            && !used.contains(&j)
        {
            pairs.push((n, j));
            used.push(j);
        }
    }
    if pairs.len() >= MIN_BANDS {
        return pairs;
    }
    (0..labels.len().min(size)).map(|n| (n, n)).collect()
}

/// Датчик и пары каналов растра с подписями labels: сначала по длинам
/// волн, у явного датчика без длин волн - по номерам в подписях или по
/// порядку. None - не узнан.
pub fn pairs_for(library: &Library, labels: &[String], sensor: Option<usize>) -> Option<Pairs> {
    let waves: Vec<Option<f64>> = labels.iter().map(|l| wavelength(l)).collect();
    if let Some(found) = by_waves(library, &waves, sensor) {
        return Some(found);
    }
    let sensor = sensor?;
    Some((sensor, by_names(library, sensor, labels)))
}

/// Угол, градусы, между спектром и эталоном. NaN - у спектра нет данных
/// или он нулевой.
pub fn angle(spectrum: &[f64], reference: &[f64]) -> f64 {
    let (mut dot, mut a, mut b) = (0.0, 0.0, 0.0);
    for (s, r) in spectrum.iter().zip(reference) {
        dot += s * r;
        a += s * s;
        b += r * r;
    }
    if !(a.is_finite() && a > 0.0 && b > 0.0) {
        return f64::NAN;
    }
    (dot / (a.sqrt() * b.sqrt())).clamp(-1.0, 1.0).acos().to_degrees()
}

/// Ближайший эталон спектра и угол до него, `classify` модуля для одной
/// клетки: None - нет данных, номер None - дальше порога.
pub fn nearest_one(spectrum: &[f64], refs: &[Vec<f64>], limit: f64) -> Option<(Option<usize>, f64)> {
    let mut best: Option<(usize, f64)> = None;
    for (n, r) in refs.iter().enumerate() {
        let a = angle(spectrum, r);
        if !a.is_finite() {
            return None;
        }
        if best.is_none_or(|(_, b)| a < b) {
            best = Some((n, a));
        }
    }
    let (n, a) = best?;
    Some(((a <= limit).then_some(n), a))
}

/// Ближайшие к спектру эталоны: (номер в refs, угол) по углу, не больше
/// count.
pub fn nearest(spectrum: &[f64], refs: &[Vec<f64>], count: usize) -> Vec<(usize, f64)> {
    let mut found: Vec<(usize, f64)> = refs.iter().enumerate().map(|(n, r)| (n, angle(spectrum, r))).filter(|(_, a)| a.is_finite()).collect();
    found.sort_by(|a, b| a.1.total_cmp(&b.1));
    found.truncate(count);
    found
}

/// Эталон в масштабе спектра: наименьшие квадраты одним множителем, так
/// на графике видна форма, а не яркость.
pub fn scaled(reference: &[f64], spectrum: &[f64]) -> Vec<f64> {
    let (mut num, mut den) = (0.0, 0.0);
    for (r, s) in reference.iter().zip(spectrum) {
        if s.is_finite() {
            num += r * s;
            den += r * r;
        }
    }
    if den <= 0.0 {
        return reference.to_vec();
    }
    reference.iter().map(|r| r * num / den).collect()
}

/// Эталоны по словам названия и группе: все слова должны быть в названии
/// или приборе.
pub fn search<'a>(library: &'a Library, text: &str, group: Option<&str>) -> Vec<&'a Entry> {
    let words: Vec<String> = text.to_lowercase().split_whitespace().map(str::to_string).collect();
    library
        .spectra
        .iter()
        .filter(|e| group.is_none_or(|g| e.group == g))
        .filter(|e| {
            let hay = format!("{} {}", e.name, e.instrument).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

/// Классы сетки шагами. Сетка - значения клеток по depth подряд, NaN без
/// данных, как у [`crate::rasterclass`].
pub struct Classify {
    depth: usize,
    /// Каналы сетки, идущие в спектр, по порядку каналов эталонов.
    bands: Vec<usize>,
    refs: Vec<Vec<f64>>,
    limit: f64,
    inside: Option<Vec<bool>>,
    next: usize,
    total: usize,
    /// Номер эталона клетки, -1 без класса.
    pub kinds: Vec<i16>,
    /// Угол до ближайшего эталона, NaN без данных.
    pub angles: Vec<f32>,
}

impl Classify {
    /// Расчёт по сетке из cells клеток. inside - клетки участка.
    pub fn new(cells: usize, depth: usize, bands: Vec<usize>, refs: Vec<Vec<f64>>, limit: f64, inside: Option<Vec<bool>>) -> Self {
        Self { depth, bands, refs, limit, inside, next: 0, total: cells, kinds: vec![-1; cells], angles: vec![f32::NAN; cells] }
    }

    /// Готовых клеток.
    pub fn done(&self) -> usize {
        self.next
    }

    /// Всего клеток.
    pub fn total(&self) -> usize {
        self.total
    }

    /// Следующие count клеток сетки grid. true - готовы все.
    pub fn step(&mut self, grid: &[f32], count: usize) -> bool {
        let end = self.next.saturating_add(count).min(self.total);
        let mut spectrum = vec![0.0; self.bands.len()];
        for cell in self.next..end {
            if self.inside.as_ref().is_some_and(|m| !m.get(cell).copied().unwrap_or(false)) {
                continue;
            }
            let start = cell * self.depth;
            let mut good = true;
            for (slot, band) in spectrum.iter_mut().zip(&self.bands) {
                let v = grid.get(start + band).copied().map_or(f64::NAN, f64::from);
                good &= v.is_finite();
                *slot = v;
            }
            if !good {
                continue;
            }
            if let Some((kind, a)) = nearest_one(&spectrum, &self.refs, self.limit) {
                self.angles[cell] = a as f32;
                self.kinds[cell] = kind.and_then(|k| i16::try_from(k).ok()).unwrap_or(-1);
            }
        }
        self.next = end;
        self.next >= self.total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    fn sensor(key: &str) -> usize {
        library().sensor(key).unwrap_or(usize::MAX)
    }

    #[test]
    fn sensors_and_groups() {
        let lib = library();
        assert_eq!(lib.sensors.get(sensor("aster")).map(|s| s.um.len()), Some(9));
        assert_eq!(lib.sensors.get(sensor("landsat8")).map(|s| s.um.len()), Some(7));
        assert_eq!(lib.sensors.get(sensor("sentinel2")).map(|s| s.um.len()), Some(13));
        assert!(lib.spectra.len() > 1600);
        assert!(lib.spectra.iter().all(|s| GROUPS.contains(&s.group.as_str())));
        assert!(lib.spectra.iter().any(|s| s.group == "minerals"));
        assert!(lib.source.contains("USGS"));
    }

    #[test]
    fn kaolinite_dips_at_aster_band_6() {
        // Каолинит поглощает у 2.2 мкм: канал 6 ASTER ниже канала 4.
        let lib = library();
        let found = search(lib, "kaolinite cm3", None);
        let values = found.first().and_then(|e| e.values.get(sensor("aster"))?.clone()).unwrap_or_default();
        assert!(values.len() == 9 && values[5] < values[3] * 0.8);
    }

    #[test]
    fn aster_file_labels_match_aster() {
        let labels = strings(&["B1 0.56 um", "B2 0.66 um", "B3N 0.82 um", "B4 1.65 um", "B5 2.165 um", "B6 2.205 um", "B7 2.26 um", "B8 2.33 um", "B9 2.395 um", "B10 8.3 um, K", "B11 8.65 um, K"]);
        let found = pairs_for(library(), &labels, None);
        assert_eq!(found, Some((sensor("aster"), (0..9).map(|n| (n, n)).collect())));
    }

    #[test]
    fn sentinel_and_landsat_names() {
        let s2 = strings(&["B1 coastal", "B2 blue", "B3 green", "B4 red", "B5 rededge1", "B6 rededge2", "B7 rededge3", "B8 nir", "B8A nir08", "B9 nir09", "B11 swir16", "B12 swir22"]);
        let (key, pairs) = pairs_for(library(), &s2, None).unwrap_or_default();
        assert_eq!(key, sensor("sentinel2"));
        assert_eq!(pairs.len(), 12);
        // B11 - 12-й канал библиотеки.
        assert!(pairs.contains(&(10, 11)));
        let landsat = strings(&["B1 coastal", "B2 blue", "B3 green", "B4 red", "B5 nir08", "B6 swir16", "B7 swir22", "B10 lwir, K"]);
        assert_eq!(pairs_for(library(), &landsat, None), Some((sensor("landsat8"), (0..7).map(|n| (n, n)).collect())));
    }

    #[test]
    fn unknown_labels_need_sensor() {
        let labels = strings(&["Band 1", "Band 2", "Band 3", "Band 4"]);
        assert_eq!(pairs_for(library(), &labels, None), None);
        assert_eq!(pairs_for(library(), &labels, Some(sensor("landsat8"))), Some((sensor("landsat8"), vec![(0, 0), (1, 1), (2, 2), (3, 3)])));
        let s2 = strings(&["B2", "B3", "B4", "B8A", "B11"]);
        assert_eq!(pairs_for(library(), &s2, Some(sensor("sentinel2"))).map(|f| f.1), Some(vec![(0, 1), (1, 2), (2, 3), (3, 8), (4, 11)]));
    }

    #[test]
    fn band_numbers_follow_the_word() {
        assert_eq!(band_number("Band 3N"), Some("3N".into()));
        assert_eq!(band_number("b_07"), Some("7".into()));
        assert_eq!(band_number("B0"), Some("0".into()));
        assert_eq!(band_number("B8A"), Some("8A".into()));
        assert_eq!(band_number("B8AX"), None);
        assert_eq!(band_number("AB4"), None);
    }

    #[test]
    fn wavelength_units() {
        assert!(wavelength("665 nm").is_some_and(|w| (w - 0.665).abs() < 1e-12));
        assert!(wavelength("B4 1.65 um").is_some_and(|w| (w - 1.65).abs() < 1e-12));
        assert!(wavelength("B4 1,65 мкм").is_some_and(|w| (w - 1.65).abs() < 1e-12));
        assert_eq!(wavelength("B13 10.6 um, K"), None);
        assert_eq!(wavelength("B13 10.6 um"), None);
        assert_eq!(wavelength("Band 1"), None);
        assert_eq!(wavelength("swir16"), Some(1.61));
    }

    #[test]
    fn brightness_does_not_change_angle() {
        let r = [0.2, 0.4, 0.3, 0.1];
        assert!(angle(&[0.1, 0.2, 0.15, 0.05], &r) < 1e-3);
        assert!(angle(&[0.0; 4], &r).is_nan());
    }

    #[test]
    fn classify_picks_nearest_within_limit() {
        let refs = vec![vec![1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0]];
        let grid: Vec<f32> = [[1.0, 0.05, 0.0], [0.05, 1.0, 0.0], [1.0, 1.0, 1.0], [f32::NAN; 3]].concat();
        let mut run = Classify::new(4, 3, vec![0, 1, 2], refs, 6.0, None);
        assert!(!run.step(&grid, 2));
        assert!(run.step(&grid, 10));
        assert_eq!(run.kinds, vec![0, 1, -1, -1]);
        assert!((run.angles[0] - 2.862).abs() < 0.01);
        assert!(run.angles[3].is_nan());
        // Клетка вне участка класса не получает.
        let mut masked = Classify::new(4, 3, vec![0, 1, 2], vec![vec![1.0, 0.0, 0.0]], 6.0, Some(vec![false, true, true, true]));
        masked.step(&grid, 4);
        assert_eq!(masked.kinds[0], -1);
        assert!(masked.angles[0].is_nan());
    }

    #[test]
    fn nearest_and_scaled() {
        let refs = vec![vec![1.0, 2.0, 3.0], vec![3.0, 2.0, 1.0]];
        let spectrum = [0.5, 1.0, 1.6];
        let found = nearest(&spectrum, &refs, 2);
        assert_eq!(found.first().map(|f| f.0), Some(0));
        assert!(found[0].1 < found[1].1);
        let fitted = scaled(&refs[0], &spectrum);
        assert!((fitted[1] - 1.0).abs() < 0.1);
    }

    #[test]
    fn values_take_the_bands_of_the_raster() {
        let lib = library();
        let id = search(lib, "kaolinite cm3", None).first().map_or(0, |e| e.id);
        let (ids, refs) = lib.values(sensor("aster"), &[id], &[3, 5]);
        assert_eq!(ids, vec![id]);
        assert!(refs[0][1] < refs[0][0] * 0.8);
        assert!(Library::parse("{}").is_none());
    }
}
