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
//! Читает GRIB `terrio` 0.6, у модуля - GDAL.
//!
//! Выпуск модели - каждые 6 часов, файлы появляются через 3.5-5 часов.
//! Часы прогноза - до 120 по одному, дальше до 384 через три. Момент шкалы
//! до выпуска берётся из прежнего выпуска, архив на AWS хранится годами.
//!
//! Сияние - вероятность видимого сияния на ближайшие 30-90 минут на сетке
//! 1°, файл JSON службы NOAA SWPC обновляется каждые 5 минут.

use serde_json::Value;

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
/// (секунды UTC), hour - час прогноза.
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

/// Поле из участка файла GRIB2. Температуру terrio отдаёт в градусах
/// Цельсия, как GDAL, сетка начинается с -180°. None - файл не читается.
pub fn read_grib(bytes: Vec<u8>, field: &Field, run: f64, hour: u32) -> Option<Grid> {
    let ds = terrio::memory::open_raster_bytes(bytes, "field.grib2").ok()?;
    let r = &ds.raster;
    let (cols, rows) = (r.cols, r.rows);
    let bands: Vec<Vec<f32>> = (0..r.bands.max(1))
        .map(|b| {
            let mut out = Vec::with_capacity(cols * rows);
            for row in 0..rows as isize {
                for col in 0..cols as isize {
                    out.push(r.get(b as isize, row, col) as f32);
                }
            }
            out
        })
        .collect();
    let g = ds.geo_transform();
    Some(Grid { values: values_of(field, &bands), rows, cols, west: g[0] + g[1] / 2.0, north: g[3] + g[5] / 2.0, step: g[1].abs(), run, hour })
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
    Some(Grid { values, rows: AURORA_ROWS, cols: AURORA_COLS, west: 0.0, north: 90.0, step: 1.0, run, hour: 0 })
}
