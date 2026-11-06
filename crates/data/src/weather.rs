// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Погода: прогноз модели NOAA GFS полями и полярное сияние OVATION,
//! перенос `core/weather.py` и `core/aurora.py` модуля.
//!
//! Поля - модель GFS 0.25° из открытых данных NOAA на AWS: «open to the
//! public and can be used as desired», NOAA просит указывать источник. Поле
//! берётся по диапазону байт из индекса .idx, так NOAA сама советует
//! выбирать поля. Поле на весь мир - 0.5-0.8 МБ GRIB2, сетка 1440 × 721.
//! Читает GRIB свой разбор `crate::grib2` (до 6 ноября 2026 года -
//! `terrio` 0.6), у модуля - GDAL.
//!
//! Выпуск модели - каждые 6 часов, файлы появляются через 3.5-5 часов.
//! Часы прогноза - до 120 по одному, дальше до 384 через три. Момент шкалы
//! до выпуска берётся из прежнего выпуска, архив на AWS хранится годами.
//!
//! Сияние - вероятность видимого сияния на ближайшие 30-90 минут на сетке
//! 1°, файл JSON службы NOAA SWPC обновляется каждые 5 минут.

use std::sync::Arc;

use serde_json::Value;

use crate::flow::VectorGrid;

pub const BUCKET: &str = "https://noaa-gfs-bdp-pds.s3.amazonaws.com";
/// Часов между выпусками.
pub const CYCLE: i64 = 6;
/// Часов от выпуска до первой попытки взять его файлы.
pub const DELAY: i64 = 4;
/// Выпусков назад, если свежий ещё не выложен.
pub const RUNS_BACK: i64 = 4;
/// До этого часа прогноз по одному часу.
pub const HOURLY: u32 = 120;
/// Последний час прогноза.
pub const LAST: u32 = 384;
/// Непрозрачность раскраски поля.
pub const OPACITY: f64 = 0.7;
pub const ATTRIBUTION: (&str, &str) = ("NOAA GFS", "https://registry.opendata.aws/noaa-gfs-bdp-pds/");
pub const AURORA_URL: &str = "https://services.swpc.noaa.gov/json/ovation_aurora_latest.json";
pub const AURORA_ATTRIBUTION: (&str, &str) = ("NOAA SWPC OVATION", "https://www.swpc.noaa.gov/products/aurora-30-minute-forecast");
/// Полученный прогноз сияния не просится заново раньше этого срока, с.
pub const AURORA_REFRESH: f64 = 600.0;
/// Чисел поперёк вида, окно чисел в ширинах вида вокруг точки взгляда,
/// `VALUES_ACROSS` и `VALUE_WINDOW` модуля. Сетка чисел стоит на месте,
/// пока вид не уходит на шаг, числа не прыгают при сдвиге.
pub const VALUES_ACROSS: f64 = 6.0;
pub const VALUE_WINDOW: f64 = 1.5;
/// Шаги узлов чисел, градусы, от крупного к мелкому.
pub const VALUE_STEPS: [f64; 8] = [30.0, 20.0, 10.0, 5.0, 2.0, 1.0, 0.5, 0.25];
/// Выше по широте чисел нет, узлы там сходятся.
pub const VALUE_LAT: f64 = 80.0;
const M_PER_DEGREE: f64 = 111_320.0;
/// Цветов в полосе шкалы.
const LEGEND_COLORS: usize = 64;
const AURORA_ROWS: usize = 181;
const AURORA_COLS: usize = 360;

/// Точка шкалы: значение и цвет RGBA.
pub type Stop = (f64, [u8; 4]);

/// Поле прогноза: ключ, начала строк индекса, единицы, раскраска. clear -
/// значение, ниже которого поле прозрачно, scale - множитель к значению
/// файла.
#[derive(Debug, PartialEq)]
pub struct Field {
    pub key: &'static str,
    pub lines: &'static [&'static str],
    pub units: &'static str,
    pub ramp: &'static [Stop],
    pub clear: Option<f64>,
    pub scale: f64,
}

pub const TEMPERATURE: Field = Field {
    key: "temperature",
    lines: &[":TMP:2 m above ground:"],
    units: "°C",
    ramp: &[(-40.0, [60, 0, 130, 255]), (-25.0, [40, 60, 220, 255]), (-10.0, [60, 160, 240, 255]), (0.0, [230, 240, 250, 255]), (10.0, [120, 210, 90, 255]), (20.0, [250, 210, 50, 255]), (30.0, [240, 110, 30, 255]), (40.0, [180, 20, 30, 255])],
    clear: None,
    scale: 1.0,
};
/// Осадки - интенсивность в момент, кг/м²/с, в мм/ч.
pub const PRECIPITATION: Field = Field {
    key: "precipitation",
    lines: &[":PRATE:surface:"],
    units: "mm/h",
    ramp: &[(0.1, [150, 210, 255, 150]), (1.0, [40, 140, 240, 220]), (4.0, [20, 60, 200, 240]), (10.0, [150, 40, 200, 250]), (30.0, [230, 30, 120, 255])],
    clear: Some(0.1),
    scale: 3600.0,
};
/// Ветер - U и V на 10 м подряд в файле, скорость по ним.
pub const WIND: Field = Field {
    key: "wind",
    lines: &[":UGRD:10 m above ground:", ":VGRD:10 m above ground:"],
    units: "m/s",
    ramp: &[(0.0, [230, 245, 255, 0]), (3.0, [170, 220, 250, 140]), (8.0, [80, 200, 120, 210]), (14.0, [250, 210, 50, 240]), (20.0, [240, 90, 30, 250]), (30.0, [170, 20, 120, 255])],
    clear: None,
    scale: 1.0,
};
pub const CLOUDS: Field = Field {
    key: "clouds",
    lines: &[":TCDC:entire atmosphere:"],
    units: "%",
    ramp: &[(5.0, [255, 255, 255, 0]), (40.0, [240, 240, 245, 120]), (80.0, [225, 225, 235, 210]), (100.0, [210, 210, 220, 240])],
    clear: None,
    scale: 1.0,
};
/// Сияние: ниже 3 % поле прозрачно, зелёный - слабое, к красному - сильное.
pub const AURORA: Field = Field {
    key: "aurora",
    lines: &[],
    units: "%",
    ramp: &[(3.0, [60, 255, 140, 0]), (10.0, [60, 255, 140, 150]), (30.0, [150, 255, 90, 210]), (50.0, [250, 230, 60, 230]), (70.0, [250, 120, 40, 245]), (90.0, [230, 40, 90, 255])],
    clear: Some(3.0),
    scale: 1.0,
};

/// Поле на сетке широт и долгот: строки с севера на юг, west, north -
/// центр левого верхнего узла, step - шаг в градусах, run - выпуск модели
/// (секунды UTC), hour - час прогноза, vectors - U и V ветра для струек.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub values: Vec<f32>,
    pub rows: usize,
    pub cols: usize,
    pub west: f64,
    pub north: f64,
    pub step: f64,
    pub run: f64,
    pub hour: u32,
    pub vectors: Option<Arc<VectorGrid>>,
}

/// Часы прогноза GFS: до [`HOURLY`] по одному, дальше через три.
pub fn hours() -> Vec<u32> {
    (0..=HOURLY).chain((HOURLY + 3..=LAST).step_by(3)).collect()
}

/// Выпуск модели не позже момента seconds, секунды UTC.
pub fn run_time(seconds: f64) -> f64 {
    let step = (CYCLE * 3600) as f64;
    (seconds / step).floor() * step
}

/// Выпуски, которые стоит пробовать к моменту now: самый свежий из тех,
/// что уже могли выложить, и [`RUNS_BACK`] прежних.
pub fn candidate_runs(now: f64) -> Vec<f64> {
    let newest = run_time(now - (DELAY * 3600) as f64);
    (0..=RUNS_BACK).map(|n| newest - (n * CYCLE * 3600) as f64).collect()
}

/// Час прогноза выпуска run, ближайший к моменту. Момент до выпуска - час
/// 0, после последнего часа - последний.
pub fn pick_hour(run: f64, moment: f64) -> u32 {
    let offset = (moment - run) / 3600.0;
    let mut best = 0;
    let mut gap = f64::INFINITY;
    for h in hours() {
        let d = (f64::from(h) - offset).abs();
        if d < gap {
            gap = d;
            best = h;
        }
    }
    best
}

/// Выпуск для момента: прошлое - выпуск не позже момента, будущее - самый
/// свежий выложенный newest.
pub fn source_run(newest: f64, moment: f64) -> f64 {
    newest.min(run_time(moment))
}

/// Момент соседнего часа прогноза выпуска run, None - за краем ряда.
pub fn step_moment(run: f64, moment: f64, delta: i64) -> Option<f64> {
    let all = hours();
    let hour = pick_hour(run, moment);
    let index = all.iter().position(|h| *h == hour)? as i64 + delta;
    let next = usize::try_from(index).ok().and_then(|i| all.get(i))?;
    Some(run + f64::from(*next) * 3600.0)
}

/// Дата и час выпуска: «ГГГГММДД» и час.
fn run_day(run: f64) -> (String, u32) {
    let day = (run / 86400.0).floor();
    let d = crate::gibs::Day::from_seconds(day * 86400.0);
    (format!("{:04}{:02}{:02}", d.year, d.month, d.day), ((run - day * 86400.0) / 3600.0).round() as u32)
}

/// Путь файла часа hour выпуска run от корня зеркала.
pub fn file_path(run: f64, hour: u32) -> String {
    let (day, cycle) = run_day(run);
    format!("/gfs.{day}/{cycle:02}/atmos/gfs.t{cycle:02}z.pgrb2.0p25.f{hour:03}")
}

pub fn file_url(run: f64, hour: u32) -> String {
    format!("{BUCKET}{}", file_path(run, hour))
}

/// Диапазон байт (начало, конец включительно) строк поля часа hour
/// в индексе .idx или None. Строки поля идут подряд (U и V ветра), берётся
/// весь их участок. Час 0 - анализ «anl», прочие - «N hour fcst»,
/// осреднённые за промежуток строки не берутся. Конец None - до конца файла.
pub fn byte_range(index: &str, field: &Field, hour: u32) -> Option<(u64, Option<u64>)> {
    let kind = if hour == 0 { "anl".to_string() } else { format!("{hour} hour fcst") };
    let tail = format!(":{kind}");
    let lines: Vec<&str> = index.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = |n: usize| lines.get(n).and_then(|l| l.split(':').nth(1)).and_then(|s| s.parse::<u64>().ok());
    let mut found = Vec::new();
    for want in field.lines {
        found.push(lines.iter().position(|l| l.contains(want) && l.trim_end().trim_end_matches(':').ends_with(&tail))?);
    }
    let (first, last) = (*found.iter().min()?, *found.iter().max()?);
    Some((start(first)?, start(last + 1).map(|e| e.saturating_sub(1))))
}

/// Значения поля из каналов файла: ветер - скорость по U и V, прочее -
/// первый канал с множителем.
pub fn values_of(field: &Field, bands: &[Vec<f32>]) -> Vec<f32> {
    match bands {
        [u, v, ..] if field.key == WIND.key => u.iter().zip(v).map(|(a, b)| a.hypot(*b)).collect(),
        [first, ..] => first.iter().map(|v| (f64::from(*v) * field.scale) as f32).collect(),
        [] => Vec::new(),
    }
}

/// Поле из участка файла GRIB2, свой разбор [`crate::grib2`] вместо terrio
/// с 6 ноября 2026 года (`doc/EDITIONS.md`): температура в градусах
/// Цельсия, сетка с -180°, как отдавал terrio, сверка - тест
/// `gfs_fields_match_terrio`. Сообщение - канал: у ветра U и V. None -
/// файл не читается.
pub fn read_grib(bytes: Vec<u8>, field: &Field, run: f64, hour: u32) -> Option<Grid> {
    let messages = crate::grib2::decode(&bytes).ok()?;
    let mut bands: Vec<Vec<f32>> = Vec::new();
    let mut geo: Option<(usize, usize, f64, f64, f64)> = None;
    for m in &messages {
        let (values, rows, cols, west, north, step) = crate::grib2::north_up(m);
        match geo {
            None => geo = Some((rows, cols, west, north, step)),
            // Каналы на разных сетках не складываются.
            Some((r, c, ..)) if (r, c) != (rows, cols) => return None,
            Some(_) => {}
        }
        bands.push(values);
    }
    let (rows, cols, west, north, step) = geo?;
    let values = values_of(field, &bands);
    let vectors = match bands.as_slice() {
        [u, v, ..] if field.key == WIND.key => VectorGrid::new(u.clone(), v.clone(), rows, cols, west, north, step).map(Arc::new),
        _ => None,
    };
    Some(Grid { values, rows, cols, west, north, step, run, hour, vectors })
}

/// Значение поля в точке, билинейно, долгота по кругу.
pub fn sample(grid: &Grid, lat: f64, lon: f64) -> f64 {
    let (rows, cols) = (grid.rows, grid.cols);
    if rows < 2 || cols == 0 {
        return f64::NAN;
    }
    let y = ((grid.north - lat) / grid.step).clamp(0.0, rows as f64 - 1.0);
    let x = ((lon - grid.west) / grid.step).rem_euclid(cols as f64);
    let y0 = (y.floor() as usize).min(rows - 2);
    let x0 = (x.floor() as usize) % cols;
    let x1 = (x0 + 1) % cols;
    let (fy, fx) = (y - y0 as f64, x - x.floor());
    let at = |r: usize, c: usize| f64::from(grid.values.get(r * cols + c).copied().unwrap_or(f32::NAN));
    let top = at(y0, x0) * (1.0 - fx) + at(y0, x1) * fx;
    let bottom = at(y0 + 1, x0) * (1.0 - fx) + at(y0 + 1, x1) * fx;
    top * (1.0 - fy) + bottom * fy
}

/// Шаг узлов чисел в градусах для видимой полосы width метров. Мельче шага
/// сетки GFS 0.25° не бывает.
pub fn value_step(width: f64) -> f64 {
    let wanted = width / VALUES_ACROSS / M_PER_DEGREE;
    VALUE_STEPS.iter().rev().copied().find(|s| *s >= wanted).unwrap_or(VALUE_STEPS[0])
}

/// Узлы чисел вокруг точки взгляда: (широта, долгота).
pub fn value_points(lat: f64, lon: f64, width: f64) -> Vec<(f64, f64)> {
    let step = value_step(width);
    let half = VALUE_WINDOW * width / 2.0 / M_PER_DEGREE;
    let south = (lat - half).max(-VALUE_LAT);
    let north = (lat + half).min(VALUE_LAT);
    let span = half / lat.to_radians().cos().max(0.05);
    let (first, last) = if span >= 180.0 {
        ((-180.0 / step).ceil() as i64, ((180.0 - 1e-9) / step).floor() as i64)
    } else {
        (((lon - span) / step).ceil() as i64, ((lon + span) / step).floor() as i64)
    };
    let mut out = Vec::new();
    for row in (south / step).ceil() as i64..=(north / step).floor() as i64 {
        for col in first..=last {
            out.push((row as f64 * step, (col as f64 * step + 180.0).rem_euclid(360.0) - 180.0));
        }
    }
    out
}

/// Числа поля у точки взгляда: (широта, долгота, значение). Значения ниже
/// порога прозрачности не подписываются.
pub fn value_marks(grid: &Grid, field: &Field, lat: f64, lon: f64, width: f64) -> Vec<(f64, f64, f64)> {
    value_points(lat, lon, width)
        .into_iter()
        .filter_map(|(la, lo)| {
            let value = sample(grid, la, lo);
            (value.is_finite() && field.clear.is_none_or(|c| value >= c)).then_some((la, lo, value))
        })
        .collect()
}

/// Число поля на глобусе: «7°», «1.2», «5», «80%». Единицы стоят в шкале.
pub fn value_text(field: &Field, value: f64) -> String {
    let whole = format!("{value:.0}");
    let whole = if whole == "-0" { "0".to_string() } else { whole };
    if field.key == TEMPERATURE.key {
        format!("{whole}°")
    } else if field.key == PRECIPITATION.key {
        format!("{value:.1}")
    } else if field.key == CLOUDS.key || field.key == AURORA.key {
        format!("{whole}%")
    } else {
        whole
    }
}

/// Шкала поля для панели шкал: полоса цветов с подписями у точек раскраски.
pub fn legend(field: &Field) -> Option<crate::gibs::Colormap> {
    let (lo, hi) = (field.ramp.first()?.0, field.ramp.last()?.0);
    let colors = (0..LEGEND_COLORS)
        .map(|n| {
            let c = colorize(field, lo + (hi - lo) * n as f64 / (LEGEND_COLORS - 1) as f64);
            [c[0], c[1], c[2]]
        })
        .collect();
    let labels = field.ramp.iter().map(|(s, _)| ((s - lo) / (hi - lo), s.to_string())).collect();
    Some(crate::gibs::Colormap { kind: "continuous".to_string(), units: field.units.to_string(), colors, labels, names: Vec::new() })
}

/// Срок поля: момент UTC, на который дан прогноз, «ГГГГ-ММ-ДД ЧЧ:ММ».
pub fn valid_text(grid: &Grid) -> String {
    let moment = grid.run + f64::from(grid.hour) * 3600.0;
    let day = (moment / 86400.0).floor();
    let minutes = ((moment - day * 86400.0) / 60.0).floor() as u32;
    format!("{} {:02}:{:02}", crate::gibs::Day::from_seconds(day * 86400.0).text(), minutes / 60, minutes % 60)
}

/// Цвет значения по раскраске поля, без премножения.
pub fn colorize(field: &Field, value: f64) -> [u8; 4] {
    if !value.is_finite() || field.clear.is_some_and(|c| value < c) {
        return [0; 4];
    }
    let (Some(first), Some(last)) = (field.ramp.first(), field.ramp.last()) else {
        return [0; 4];
    };
    if value <= first.0 {
        return first.1;
    }
    if value >= last.0 {
        return last.1;
    }
    for pair in field.ramp.windows(2) {
        if let [(a, ca), (b, cb)] = pair
            && value <= *b
        {
            let t = (value - a) / (b - a);
            return [0, 1, 2, 3].map(|i| (f64::from(ca[i]) + (f64::from(cb[i]) - f64::from(ca[i])) * t).round() as u8);
        }
    }
    last.1
}

/// Картинка тайла (z, x, y), 256 × 256 RGBA с премноженной альфой, None -
/// тайл пуст.
pub fn tile_rgba(field: &Field, grid: &Grid, key: (u32, u32, u32)) -> Option<Vec<u8>> {
    let (z, x, y) = key;
    let n = f64::from(1u32 << z.min(30));
    let mut out = vec![0u8; 256 * 256 * 4];
    let mut any = false;
    for r in 0..256usize {
        let lat = (std::f64::consts::PI * (1.0 - 2.0 * (f64::from(y) + (r as f64 + 0.5) / 256.0) / n)).sinh().atan().to_degrees();
        for c in 0..256usize {
            let lon = (f64::from(x) + (c as f64 + 0.5) / 256.0) / n * 360.0 - 180.0;
            let p = colorize(field, sample(grid, lat, lon));
            let alpha = f64::from(p[3]) * OPACITY;
            if alpha > 0.0 {
                any = true;
                let dim = |v: u8| (f64::from(v) * alpha / 255.0).round() as u8;
                out[(r * 256 + c) * 4..(r * 256 + c) * 4 + 4].copy_from_slice(&[dim(p[0]), dim(p[1]), dim(p[2]), alpha.round() as u8]);
            }
        }
    }
    any.then_some(out)
}

/// Секунды UTC из записи «2026-10-12T03:15:00Z» или None.
pub fn forecast_seconds(text: &str) -> Option<f64> {
    let text = text.trim().trim_end_matches('Z').replace(' ', "T");
    let (date, time) = text.split_once('T')?;
    let mut part = date.split('-').map(str::parse::<u32>);
    let (y, m, d) = (part.next()?.ok()?, part.next()?.ok()?, part.next()?.ok()?);
    let mut clock = time.split(':').map(str::parse::<u32>);
    let (h, min) = (clock.next()?.ok()?, clock.next()?.ok()?);
    let s = clock.next().and_then(Result::ok).unwrap_or(0);
    let day = crate::gibs::Day { year: i32::try_from(y).ok()?, month: m, day: d };
    Some(day.seconds() + f64::from(h * 3600 + min * 60 + s))
}

/// Поле сияния из файла OVATION или None. Строки - с севера на юг, столбцы -
/// долготы от 0°. run - момент прогноза, hour 0.
pub fn parse_aurora(data: &[u8]) -> Option<Grid> {
    let doc: Value = serde_json::from_slice(data).ok()?;
    let triples = doc.get("coordinates")?.as_array().filter(|a| !a.is_empty())?;
    let run = doc.get("Forecast Time").and_then(Value::as_str).and_then(forecast_seconds).or_else(|| doc.get("Observation Time").and_then(Value::as_str).and_then(forecast_seconds))?;
    let mut values = vec![0.0f32; AURORA_ROWS * AURORA_COLS];
    for t in triples {
        let (Some(lon), Some(lat), Some(v)) = (t.get(0).and_then(Value::as_f64), t.get(1).and_then(Value::as_f64), t.get(2).and_then(Value::as_f64)) else {
            return None;
        };
        let (lon, lat) = (lon.round() as i64, lat.round() as i64);
        if (-90..=90).contains(&lat) && v.is_finite() {
            values[(90 - lat) as usize * AURORA_COLS + lon.rem_euclid(AURORA_COLS as i64) as usize] = v as f32;
        }
    }
    Some(Grid { values, rows: AURORA_ROWS, cols: AURORA_COLS, west: 0.0, north: 90.0, step: 1.0, run, hour: 0, vectors: None })
}
