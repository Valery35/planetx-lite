// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Темы NASA GIBS - растры по датам: огонь, вода, газы, земля и жизнь.
//! Перенос `core/themes.py` модуля.
//!
//! Слои выбрал помощник модуля по полному списку GetCapabilities GIBS
//! EPSG:3857 «best» на 5 октября 2026 года, состав утверждает автор.
//! Тайлы - PNG с прозрачностью там, где данных нет, у ночного снимка -
//! JPEG. Дата слоя - день ряда не позже выбранного момента: ряд
//! с пропусками даёт запрос DescribeDomains, дата вне ряда отвечает 404.

/// Корень служб GIBS.
pub const ROOT: &str = "https://gibs.earthdata.nasa.gov";
/// Подпись источника и адрес.
pub const ATTRIBUTION: (&str, &str) = ("NASA GIBS", "https://earthdata.nasa.gov/gibs");
/// Непрозрачность раскраски поверх снимка.
pub const OPACITY: f64 = 0.8;
/// День ряда считается готовым через сутки, секунды.
pub const READY_LAG: f64 = 86400.0;

/// Группа темы, `GROUPS` модуля. Catalog - слой всего каталога GIBS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Fire,
    Water,
    Gases,
    Life,
    Catalog,
}

/// Приставка ключа слоя всего каталога, `GIBS_PREFIX` модуля.
pub const CATALOG_PREFIX: &str = "gibs:";
/// Ответ GetCapabilities WMTS EPSG:3857 «best» - весь каталог GIBS.
pub const CAPABILITIES: &str = "https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/1.0.0/WMTSCapabilities.xml";

/// Слой темы: ключ строки, группа, слой GIBS, расширение тайла,
/// предельный уровень, цветовая карта ("" - шкалы нет), цвета классов
/// «нет явления», которые тема делает прозрачными. intervals - ряд дат
/// слоя каталога прямо из его ответа: None - ряд даёт DescribeDomains,
/// пустой - у слоя нет времени, его тайлы - на день «default».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub key: &'static str,
    pub group: Group,
    pub layer: &'static str,
    pub ext: &'static str,
    pub level: u32,
    pub colormap: &'static str,
    pub clear: &'static [[u8; 3]],
    pub intervals: Option<&'static [Interval]>,
}

const fn theme(key: &'static str, group: Group, layer: &'static str, ext: &'static str, level: u32, colormap: &'static str) -> Theme {
    Theme { key, group, layer, ext, level, colormap, clear: &[], intervals: None }
}

/// Темы в порядке модуля. У массы снега SMAP класс 0-0.8 кг/м²
/// покрывал серо-синим 97 % суши у Перми 2 октября 2026 года, он
/// прозрачен.
pub const THEMES: [Theme; 22] = [
    theme("smoke", Group::Fire, "OMPS_Aerosol_Index", "png", 6, "OMPS_Aerosol_Index"),
    theme("aerosol", Group::Fire, "MODIS_Combined_Value_Added_AOD", "png", 6, "MODIS_Combined_Value_Added_AOD"),
    theme("co", Group::Fire, "AIRS_L2_Carbon_Monoxide_500hPa_Volume_Mixing_Ratio_Day", "png", 6, "AIRS_Carbon_Monoxide_Volume_Mixing_Ratio"),
    theme("co_emission", Group::Fire, "MERRA2_Carbon_Monoxide_Emission_Monthly", "png", 6, "MERRA2_Carbon_Monoxide_Emission_Monthly"),
    theme("rain", Group::Water, "IMERG_Precipitation_Rate", "png", 6, "GPM_Precipitation_Rate"),
    theme("soil", Group::Water, "SMAP_L3_Passive_Enhanced_Day_Soil_Moisture", "png", 6, "SMAP_Soil_Moisture"),
    theme("snow", Group::Water, "MODIS_Terra_NDSI_Snow_Cover", "png", 8, "MODIS_NDSI_Snow_Cover"),
    theme("snow_8day", Group::Water, "MODIS_Terra_L3_Snow_Extent_8Day", "png", 8, "MODIS_L3_Snow_Extent"),
    Theme { key: "snow_mass", group: Group::Water, layer: "SMAP_L4_Snow_Mass", ext: "png", level: 6, colormap: "SMAP_Snow_Mass", clear: &[[82, 98, 106]], intervals: None },
    theme("sea_ice", Group::Water, "GHRSST_L4_MUR_Sea_Ice_Concentration", "png", 7, "GHRSST_Sea_Ice_Concentration"),
    theme("vapor", Group::Water, "MODIS_Terra_Water_Vapor_5km_Day", "png", 6, "MODIS_Water_Vapor"),
    theme("chlorophyll", Group::Water, "OCI_PACE_Chlorophyll_a", "png", 7, "MODIS_Chlorophyll"),
    theme("salinity", Group::Water, "SMAP_L3_Sea_Surface_Salinity_CAP_8Day_RunningMean", "png", 6, "SMAP_Sea_Surface_Salinity"),
    theme("flood", Group::Water, "MODIS_Combined_Flood_3-Day", "png", 9, "MODIS_Flood"),
    theme("no2", Group::Gases, "TROPOMI_L2_Nitrogen_Dioxide_Tropospheric_Column", "png", 6, "OMI_Nitrogen_Dioxide_Tropo_Column"),
    theme("so2", Group::Gases, "OMPS_SO2_Planetary_Boundary_Layer", "png", 6, "OMPS_SO2_Planetary_Boundary_Layer"),
    theme("methane", Group::Gases, "AIRS_L3_Methane_400hPa_Volume_Mixing_Ratio_Monthly_Day", "png", 6, "AIRS_Methane_Volume_Mixing_Ratio"),
    theme("co2", Group::Gases, "OCO-2_Carbon_Dioxide_Total_Column_Average", "png", 8, "OCO_Carbon_Dioxide_Total_Column_Average"),
    theme("ozone", Group::Gases, "OMI_Ozone_TOMS_Total_Column", "png", 6, "OMI_Ozone_TOMS_Total_Column"),
    theme("ndvi", Group::Life, "MODIS_Terra_L3_NDVI_16Day", "png", 9, "MODIS_L3_NDVI"),
    theme("dust", Group::Life, "AIRS_L2_Dust_Score_Day", "png", 6, "AIRS_Dust_Score"),
    theme("night", Group::Life, "VIIRS_SNPP_DayNightBand_AtSensor_M15", "jpeg", 8, ""),
];

/// Тема по ключу.
pub fn theme_of(key: &str) -> Option<&'static Theme> {
    THEMES.iter().find(|t| t.key == key)
}

impl Theme {
    /// Шаблон адреса тайла на день day (ГГГГ-ММ-ДД). Слой без времени -
    /// на день «default».
    pub fn url(&self, day: &str) -> String {
        let day = if self.intervals.is_some_and(<[Interval]>::is_empty) { "default" } else { day };
        format!("{ROOT}/wmts/epsg3857/best/{}/default/{day}/GoogleMapsCompatible_Level{}/{{z}}/{{y}}/{{x}}.{}", self.layer, self.level, self.ext)
    }

    /// День слоя каталога на момент now (секунды UTC): последний готовый
    /// день ряда, у слоя без времени - вчерашний. None - ряд даёт
    /// DescribeDomains или готовых дней нет.
    pub fn catalog_day(&self, now: f64) -> Option<String> {
        let intervals = self.intervals?;
        if intervals.is_empty() {
            return Some(Day::from_seconds(now - READY_LAG).text());
        }
        pick_day(&ready_intervals(intervals, now), None)
    }

    /// Адрес ряда дат DescribeDomains.
    pub fn domains_url(&self) -> String {
        format!("{ROOT}/wmts/epsg3857/best/1.0.0/{}/default/GoogleMapsCompatible_Level{}/all/1900-01-01--2100-01-01.xml", self.layer, self.level)
    }

    /// Адрес цветовой карты GIBS v1.3 или None - шкалы нет.
    pub fn colormap_url(&self) -> Option<String> {
        (!self.colormap.is_empty()).then(|| format!("{ROOT}/colormaps/v1.3/{}.xml", self.colormap))
    }
}

/// День календаря.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Day {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

/// Дней от 1970-01-01, алгоритм Говарда Хиннанта.
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = i64::from(y) - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// День по номеру дня от 1970-01-01.
fn civil_from_days(z: i64) -> Day {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
    Day { year, month, day }
}

fn month_days(year: i32, month: u32) -> u32 {
    let next = if month == 12 { days_from_civil(year + 1, 1, 1) } else { days_from_civil(year, month + 1, 1) };
    (next - days_from_civil(year, month, 1)) as u32
}

impl Day {
    /// День из строки ГГГГ-ММ-ДД.
    pub fn parse(text: &str) -> Option<Day> {
        let mut parts = text.splitn(3, '-');
        let year = parts.next()?.parse().ok()?;
        let month: u32 = parts.next()?.parse().ok()?;
        let day: u32 = parts.next()?.get(..2)?.parse().ok()?;
        ((1..=12).contains(&month) && day >= 1 && day <= month_days(year, month)).then_some(Day { year, month, day })
    }

    /// День момента в секундах UTC.
    pub fn from_seconds(seconds: f64) -> Day {
        civil_from_days((seconds / 86400.0).floor() as i64)
    }

    /// Начало дня в секундах UTC.
    pub fn seconds(self) -> f64 {
        days_from_civil(self.year, self.month, self.day) as f64 * 86400.0
    }

    fn number(self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }

    /// День через count шагов unit (D, M или Y). День месяца не больше
    /// длины месяца.
    fn add(self, count: i64, unit: char) -> Day {
        if unit == 'D' {
            return civil_from_days(self.number() + count);
        }
        let months = i64::from(self.month) - 1 + if unit == 'M' { count } else { 12 * count };
        let year = self.year + months.div_euclid(12) as i32;
        let month = (months.rem_euclid(12) + 1) as u32;
        Day { year, month, day: self.day.min(month_days(year, month)) }
    }

    /// Строка ГГГГ-ММ-ДД.
    pub fn text(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// Промежуток ряда: начало, конец, шаг, единица шага D, M или Y.
pub type Interval = (Day, Day, i64, char);

/// Ряд дат слоя из ответа DescribeDomains. Шаги меньше суток не
/// разбираются. Ответ считается враждебным.
pub fn parse_domains(data: &str) -> Vec<Interval> {
    let Some(start) = data.find("<Domain>") else {
        return Vec::new();
    };
    let body = &data[start + 8..];
    let Some(end) = body.find("</Domain>") else {
        return Vec::new();
    };
    body[..end]
        .split(',')
        .filter_map(|part| {
            let mut it = part.trim().split('/');
            let first = Day::parse(it.next()?)?;
            let last = Day::parse(it.next()?)?;
            let period = it.next()?.strip_prefix('P')?;
            let unit = period.chars().last()?;
            let count: i64 = period[..period.len() - 1].parse().ok()?;
            (matches!(unit, 'D' | 'M' | 'Y') && count > 0).then_some((first, last, count, unit))
        })
        .collect()
}

/// Промежуток «начало/конец/PnU» с шагом в днях, месяцах или годах.
fn parse_interval(part: &str) -> Option<Interval> {
    let mut it = part.trim().split('/');
    let (a, b) = (it.next()?, it.next()?);
    // Моменты со временем суток - ряд с шагом меньше суток.
    if a.len() != 10 || b.len() != 10 {
        return None;
    }
    let (first, last) = (Day::parse(a)?, Day::parse(b)?);
    let period = it.next()?.strip_prefix('P')?;
    let unit = period.chars().last()?;
    let count: i64 = period[..period.len() - unit.len_utf8()].parse().ok()?;
    (matches!(unit, 'D' | 'M' | 'Y') && count > 0).then_some((first, last, count, unit))
}

/// Растровый слой всего каталога GIBS: имя, название из каталога,
/// расширение тайла, предельный уровень, цветовая карта ("" - нет),
/// ряд дат (пустой - слой без времени).
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogLayer {
    pub layer: String,
    pub title: String,
    pub ext: &'static str,
    pub level: u32,
    pub colormap: String,
    pub intervals: Vec<Interval>,
}

/// Текст между open и close первого вхождения в body.
fn between<'a>(body: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let at = body.find(open)? + open.len();
    let end = body[at..].find(close)?;
    Some(&body[at..at + end])
}

/// Растровые слои GIBS из ответа GetCapabilities WMTS по названию,
/// `parse_capabilities` модуля. Слой с Dimension Time без суточного
/// ряда (шаги в минутах и часах) и векторные тайлы пропускаются. Ответ
/// считается враждебным.
pub fn parse_capabilities(data: &str) -> Vec<CatalogLayer> {
    let mut out = Vec::new();
    let mut rest = data;
    while let Some(open) = rest.find("<Layer>") {
        let tail = &rest[open + 7..];
        let Some(close) = tail.find("</Layer>") else {
            break;
        };
        let body = &tail[..close];
        rest = &tail[close + 8..];
        let Some(ident) = between(body, "<ows:Identifier>", "</ows:Identifier>").map(str::trim).filter(|s| !s.is_empty() && !s.contains(['/', '?', '#'])) else {
            continue;
        };
        let ext = if body.contains("<Format>image/png</Format>") {
            "png"
        } else if body.contains("<Format>image/jpeg</Format>") {
            "jpg"
        } else {
            continue;
        };
        let Some(level) = between(body, "GoogleMapsCompatible_Level", "</TileMatrixSet>").and_then(|l| l.trim().parse::<u32>().ok()).filter(|l| *l <= 22) else {
            continue;
        };
        let title = body.find("<ows:Title").and_then(|t| between(&body[t..], ">", "</ows:Title>")).map(unescape).unwrap_or_else(|| ident.to_string());
        let colormap = between(body, "/colormaps/v1.3/", ".xml").filter(|c| !c.contains(['"', '\'', '/', '<'])).unwrap_or("").to_string();
        let mut intervals = Vec::new();
        if body.contains("<Dimension>") {
            let mut scan = body;
            while let Some(v) = between(scan, "<Value>", "</Value>") {
                intervals.extend(v.split(',').filter_map(parse_interval));
                scan = &scan[scan.find("</Value>").map_or(scan.len(), |e| e + 8)..];
            }
            if intervals.is_empty() {
                continue;
            }
        }
        out.push(CatalogLayer { layer: ident.to_string(), title: title.trim().to_string(), ext, level, colormap, intervals });
    }
    out.sort_by_key(|l| l.title.to_lowercase());
    out
}

/// Слои, в названии или имени которых есть все слова text без учёта
/// регистра, `search` модуля.
pub fn search<'a>(layers: &'a [CatalogLayer], text: &str) -> Vec<&'a CatalogLayer> {
    let words: Vec<String> = text.to_lowercase().split_whitespace().map(String::from).collect();
    layers.iter().filter(|l| fits_words(&format!("{} {}", l.title, l.layer), &words)).collect()
}

/// В тексте есть все слова words (слова в нижнем регистре).
pub fn fits_words(text: &str, words: &[String]) -> bool {
    let text = text.to_lowercase();
    words.iter().all(|w| text.contains(w.as_str()))
}

/// Последний день ряда start..end с шагом не позже day.
fn floor(interval: Interval, day: Day) -> Day {
    let (start, end, count, unit) = interval;
    let found = if unit == 'D' {
        let steps = (day.number() - start.number()).div_euclid(count);
        start.add(steps * count, 'D')
    } else {
        let mut months = i64::from(day.year - start.year) * 12 + i64::from(day.month) - i64::from(start.month);
        if unit == 'Y' {
            months -= months.rem_euclid(12);
        }
        let step = count * if unit == 'Y' { 12 } else { 1 };
        let found = start.add(months.div_euclid(step) * step, 'M');
        if found > day { start.add((months.div_euclid(step) - 1) * step, 'M') } else { found }
    };
    found.min(end)
}

/// Ряд без дней, которые ещё не обработаны: каталог GIBS объявляет
/// день, когда он только начался, тайлы такого дня пусты. Последним
/// остаётся день не позже now - READY_LAG.
pub fn ready_intervals(intervals: &[Interval], now: f64) -> Vec<Interval> {
    let cap = Day::from_seconds(now - READY_LAG);
    intervals
        .iter()
        .filter(|i| i.0 <= cap)
        .map(|&(start, end, count, unit)| if end > cap { (start, floor((start, end, count, unit), cap), count, unit) } else { (start, end, count, unit) })
        .collect()
}

/// День ряда для момента moment (секунды UTC): последний день ряда не
/// позже момента, раньше начала ряда - первый, без момента - последний.
pub fn pick_day(intervals: &[Interval], moment: Option<f64>) -> Option<String> {
    let mut ordered = intervals.to_vec();
    ordered.sort();
    let last = ordered.last()?;
    let Some(moment) = moment else {
        return Some(last.1.text());
    };
    let day = Day::from_seconds(moment);
    let mut best = None;
    for interval in &ordered {
        if interval.0 > day {
            break;
        }
        best = Some(floor(*interval, day));
    }
    Some(best.unwrap_or(ordered[0].0).text())
}

/// Все дни ряда по порядку: шаги ползунка темы.
pub fn days(intervals: &[Interval]) -> Vec<String> {
    let mut out: Vec<Day> = Vec::new();
    for &(start, end, count, unit) in intervals {
        let mut day = start;
        let mut steps = 0i64;
        while day <= end && out.len() < 100_000 {
            out.push(day);
            steps += 1;
            day = start.add(steps * count, unit);
        }
    }
    out.sort();
    out.dedup();
    out.into_iter().map(Day::text).collect()
}

/// Соседний день ряда: delta -1 или 1. None - за краем ряда или дня нет
/// в ряду.
pub fn step_day(intervals: &[Interval], day: &str, delta: i64) -> Option<String> {
    let ordered = days(intervals);
    let at = ordered.iter().position(|d| d == day)? as i64 + delta;
    usize::try_from(at).ok().and_then(|i| ordered.get(i).cloned())
}

/// Охват ряда: первый и последний день в секундах UTC.
pub fn span(intervals: &[Interval]) -> Option<(f64, f64)> {
    let first = intervals.iter().map(|i| i.0).min()?;
    let last = intervals.iter().map(|i| i.1).max()?;
    Some((first.seconds(), last.seconds()))
}

/// Тайл темы в RGBA с премноженной альфой и непрозрачностью opacity.
/// Где данных нет и где цвет из clear, прозрачно. Пустой тайл - None.
pub fn overlay_rgba(rgba: &[u8], opacity: f64, clear: &[[u8; 3]]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(rgba.len());
    let mut any = false;
    for px in rgba.as_chunks::<4>().0 {
        let mut alpha = f64::from(px[3]) / 255.0 * opacity;
        if clear.iter().any(|c| c[..] == px[..3]) {
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

/// Шкала слоя из цветовой карты GIBS v1.3.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Colormap {
    /// «continuous» и «discrete» - полоса цветов, «classification» -
    /// классы.
    pub kind: String,
    pub units: String,
    /// Цвета по порядку.
    pub colors: Vec<[u8; 3]>,
    /// Подписи полосы: доля от 0 до 1 и текст.
    pub labels: Vec<(f64, String)>,
    /// Подписи классов.
    pub names: Vec<String>,
}

/// Атрибуты тега: пары имя - значение.
fn attributes(tag: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = tag;
    while let Some(eq) = rest.find("=\"") {
        let name: String = rest[..eq].chars().rev().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect::<Vec<_>>().into_iter().rev().collect();
        let after = &rest[eq + 2..];
        let Some(end) = after.find('"') else {
            break;
        };
        out.push((name, unescape(&after[..end])));
        rest = &after[end + 1..];
    }
    out
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

/// Ссылки на знаки XML: &amp; &lt; &gt; &quot; &apos; и &#N;.
fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let Some(end) = tail.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let name = &tail[1..end];
        let c = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => name.strip_prefix('#').and_then(|n| match n.strip_prefix('x') {
                Some(h) => u32::from_str_radix(h, 16).ok(),
                None => n.parse().ok(),
            }).and_then(char::from_u32),
        };
        match c {
            Some(c) => out.push(c),
            None => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Шкала слоя из цветовой карты GIBS v1.3. Карта «нет данных»
/// и прозрачные классы пропускаются, из нескольких карт берётся та, где
/// цветов больше. None - шкалы в ответе нет. Ответ считается враждебным.
pub fn parse_colormap(data: &str) -> Option<Colormap> {
    let mut best: Option<Colormap> = None;
    let mut rest = data;
    while let Some(open) = rest.find("<ColorMap") {
        let after = &rest[open + 9..];
        if !after.starts_with([' ', '>', '\n', '\t', '\r']) {
            rest = after;
            continue;
        }
        let Some(head_end) = after.find('>') else {
            break;
        };
        let head = attributes(&after[..head_end]);
        let Some(close) = after.find("</ColorMap>") else {
            break;
        };
        let body = &after[head_end + 1..close];
        rest = &after[close + 11..];
        let Some(legend_at) = body.find("<Legend") else {
            continue;
        };
        let legend_tail = &body[legend_at + 7..];
        let Some(legend_end) = legend_tail.find('>') else {
            continue;
        };
        let legend = attributes(&legend_tail[..legend_end]);
        let kind = attr(&legend, "type").unwrap_or("continuous").to_string();
        let mut entries: Vec<Vec<(String, String)>> = Vec::new();
        let mut scan = legend_tail;
        while let Some(e) = scan.find("<LegendEntry") {
            let tail = &scan[e + 12..];
            let Some(end) = tail.find("/>") else {
                break;
            };
            let a = attributes(&tail[..end]);
            if attr(&a, "rgb").is_some() && attr(&a, "tooltip") != Some("No Data") {
                entries.push(a);
            }
            scan = &tail[end + 2..];
        }
        if entries.is_empty() {
            continue;
        }
        let colors: Vec<[u8; 3]> = entries
            .iter()
            .map(|a| {
                let mut c = [0u8; 3];
                for (k, v) in attr(a, "rgb").unwrap_or("").split(',').take(3).enumerate() {
                    c[k] = v.trim().parse().unwrap_or(0);
                }
                c
            })
            .collect();
        let count = entries.len() as f64;
        let mut found = Colormap { kind, units: attr(&head, "units").unwrap_or("").to_string(), colors, labels: Vec::new(), names: Vec::new() };
        for (n, a) in entries.iter().enumerate() {
            let label = attr(a, "label").filter(|l| !l.is_empty());
            if attr(a, "showLabel") == Some("true")
                && let Some(l) = label
            {
                found.labels.push(((n as f64 + 0.5) / count, l.to_string()));
            }
            found.names.push(label.or(attr(a, "tooltip")).unwrap_or("").to_string());
        }
        if found.kind != "classification" && found.labels.is_empty() {
            found.labels = vec![(0.0, attr(&legend, "minLabel").unwrap_or("").to_string()), (1.0, attr(&legend, "maxLabel").unwrap_or("").to_string())];
        }
        if best.as_ref().is_none_or(|b| found.colors.len() > b.colors.len()) {
            best = Some(found);
        }
    }
    best
}
