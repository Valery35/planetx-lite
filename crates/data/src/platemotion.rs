// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Скорости литосферных плит по модели NNR-MORVEL56. Перенос
//! `core/platemotion.py` модуля.
//!
//! Плита вращается вокруг своего полюса с угловой скоростью ω, скорость
//! точки на её поверхности - ω × r, в системе без суммарного вращения
//! плит (NNR). Полюса - Table 1 статьи Argus, Gordon, DeMets 2011, плита
//! точки - растр кодов PB2002 (Bird 2003), файл data/plate_motion.json
//! собирает tools/build_plate_motion.py модуля. Растр в файле - серии
//! одинаковых кодов по строкам с севера, парами чисел (код, длина),
//! [`runs`] и [`grid_of`]. Lite берёт файл из хранилища модуля по сети
//! и в себе его не несёт.
//!
//! Стрелки на глобусе - сетка узлов около ARROWS_ACROSS поперёк вида,
//! длина стрелки пропорциональна скорости, LENGTH_SHARE шага узлов на
//! REFERENCE мм/год. Величины - выбор помощника модуля, утверждает
//! автор.

use std::collections::HashMap;

/// Файл данных модуля.
pub const URL: &str = "https://raw.githubusercontent.com/Valery35/planetx/main/planetx/data/plate_motion.json";
pub const ATTRIBUTION: (&str, &str) = ("Plate motion: NNR-MORVEL56, Argus et al. 2011", "https://doi.org/10.1029/2011GC003751");
/// Средний радиус Земли, мм.
pub const RADIUS: f64 = 6371.0e6;
/// Количество стрелок поперёк вида.
pub const ARROWS_ACROSS: u32 = 7;
/// Скорость стрелки длиной LENGTH_SHARE шага узлов, мм/год.
pub const REFERENCE: f64 = 50.0;
pub const LENGTH_SHARE: f64 = 0.45;
/// Доля длины стрелки на её усики.
pub const HEAD_SHARE: f64 = 0.3;
/// Угол между древком и усиком, градусы.
pub const HEAD_ANGLE: f64 = 25.0;
/// Охват сетки стрелок, ширин вида.
pub const WINDOW: f64 = 1.0;
pub const MAX_LAT: f64 = 80.0;
/// Шаги узлов стрелок, градусы.
pub const STEPS: [f64; 10] = [30.0, 20.0, 15.0, 10.0, 5.0, 3.0, 2.0, 1.0, 0.5, 0.25];
/// Наибольшее количество ячеек растра плит - сторож враждебного файла.
const MAX_CELLS: usize = 1 << 26;

/// Полюс вращения плиты: широта, долгота, градусы, скорость °/млн лет.
pub type Pole = (f64, f64, f64);

/// Полюса плит и растр плит.
#[derive(Debug, Clone, PartialEq)]
pub struct Motion {
    /// Коды плит, код растра n - плита codes[n - 1].
    pub codes: Vec<String>,
    pub names: HashMap<String, String>,
    pub poles: HashMap<String, Pole>,
    /// Растр кодов по строкам с севера, rows × cols.
    pub grid: Vec<u8>,
    pub rows: usize,
    pub cols: usize,
    /// Шаг растра, градусы.
    pub step: f64,
}

impl Motion {
    /// Код плиты в точке или None.
    pub fn plate_at(&self, lat: f64, lon: f64) -> Option<&str> {
        if self.rows == 0 || self.cols == 0 {
            return None;
        }
        let r = ((90.0 - lat) / self.step).max(0.0).min((self.rows - 1) as f64) as usize;
        let c = (((lon + 180.0).rem_euclid(360.0)) / self.step) as usize % self.cols;
        let n = usize::from(*self.grid.get(r * self.cols + c)?);
        if n == 0 {
            return None;
        }
        self.codes.get(n - 1).map(String::as_str)
    }

    /// (код, восток, север мм/год) или None вне плит.
    pub fn velocity(&self, lat: f64, lon: f64) -> Option<(&str, f64, f64)> {
        let code = self.plate_at(lat, lon)?;
        let (east, north) = velocity(*self.poles.get(code)?, lat, lon);
        Some((code, east, north))
    }
}

/// Растр кодов в серии одинаковых значений - плоский список чисел
/// код, длина, код, длина... по строкам с севера.
pub fn runs(grid: &[u8]) -> Vec<i64> {
    let mut out = Vec::new();
    for piece in grid.chunk_by(|a, b| a == b) {
        out.push(i64::from(piece[0]));
        out.push(piece.len() as i64);
    }
    out
}

/// Растр кодов rows × cols из серий [`runs`].
pub fn grid_of(series: &[i64], rows: usize, cols: usize) -> Result<Vec<u8>, String> {
    let (pairs, rest) = series.as_chunks::<2>();
    if !rest.is_empty() {
        return Err("plate_motion: odd runs".into());
    }
    let cells = rows.checked_mul(cols).ok_or("plate_motion: grid too large")?;
    let mut total: i128 = 0;
    for pair in pairs {
        if pair[1] < 0 {
            return Err("plate_motion: negative run".into());
        }
        total += i128::from(pair[1]);
    }
    if total != cells as i128 {
        return Err("plate_motion: runs do not fill the grid".into());
    }
    let mut grid = Vec::with_capacity(cells);
    for pair in pairs {
        // Код - младший байт, как astype(uint8) модуля.
        grid.extend(std::iter::repeat_n(pair[0] as u8, pair[1] as usize));
    }
    Ok(grid)
}

/// Полюса плит и растр из текста data/plate_motion.json.
pub fn read(text: &str) -> Result<Motion, String> {
    let json: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let step = json.get("step").and_then(|s| s.as_f64()).ok_or("plate_motion: no step")?;
    if !(step.is_finite() && step > 0.0) {
        return Err("plate_motion: bad step".into());
    }
    let (rows, cols) = ((180.0 / step).round_ties_even(), (360.0 / step).round_ties_even());
    if rows * cols > MAX_CELLS as f64 {
        return Err("plate_motion: grid too large".into());
    }
    let (rows, cols) = (rows as usize, cols as usize);
    let mut codes = Vec::new();
    let mut names = HashMap::new();
    let mut poles = HashMap::new();
    for plate in json.get("plates").and_then(|p| p.as_array()).ok_or("plate_motion: no plates")? {
        let read = || -> Option<(String, String, Pole)> {
            let p = plate.as_array()?;
            Some((p.first()?.as_str()?.to_string(), p.get(1)?.as_str()?.to_string(), (p.get(2)?.as_f64()?, p.get(3)?.as_f64()?, p.get(4)?.as_f64()?)))
        };
        let (code, name, pole) = read().ok_or("plate_motion: bad plate")?;
        codes.push(code.clone());
        names.insert(code.clone(), name);
        poles.insert(code, pole);
    }
    let series = json
        .get("runs")
        .and_then(|r| r.as_array())
        .ok_or("plate_motion: no runs")?
        .iter()
        .map(|v| v.as_i64())
        .collect::<Option<Vec<i64>>>()
        .ok_or("plate_motion: bad runs")?;
    let grid = grid_of(&series, rows, cols)?;
    Ok(Motion { codes, names, poles, grid, rows, cols, step })
}

fn unit(lat: f64, lon: f64) -> [f64; 3] {
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    [la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin()]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Скорость точки (восток, север) мм/год при вращении вокруг полюса
/// (широта, долгота, °/млн лет) против часовой стрелки.
pub fn velocity(pole: Pole, lat: f64, lon: f64) -> (f64, f64) {
    let (plat, plon, rate) = pole;
    let w = rate.to_radians() / 1.0e6; // рад/год
    let o = unit(plat, plon).map(|v| v * w);
    let r = unit(lat, lon).map(|v| v * RADIUS);
    let v = [o[1] * r[2] - o[2] * r[1], o[2] * r[0] - o[0] * r[2], o[0] * r[1] - o[1] * r[0]];
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    let east = [-lo.sin(), lo.cos(), 0.0];
    let north = [-la.sin() * lo.cos(), -la.sin() * lo.sin(), la.cos()];
    (dot(v, east), dot(v, north))
}

/// Скорость мм/год и азимут движения, градусы от севера.
pub fn speed_azimuth(east: f64, north: f64) -> (f64, f64) {
    (east.hypot(north), east.atan2(north).to_degrees().rem_euclid(360.0))
}

/// Шаг узлов стрелок, градусы, около ARROWS_ACROSS поперёк вида
/// шириной width_m метров.
pub fn step_for(width_m: f64) -> f64 {
    let degrees = width_m / 111_320.0 / f64::from(ARROWS_ACROSS);
    STEPS.iter().copied().find(|s| *s <= degrees).unwrap_or(STEPS[STEPS.len() - 1])
}

/// Точка в east_deg, north_deg градусах дуги от (lat, lon).
fn offset(lat: f64, lon: f64, east_deg: f64, north_deg: f64) -> (f64, f64) {
    let la = lat + north_deg;
    let lo = lon + east_deg / lat.to_radians().cos().max(0.05);
    (la, (lo + 180.0).rem_euclid(360.0) - 180.0)
}

/// Стрелка скорости: узел, скорость мм/год, азимут, градусы, ломаные
/// древка и двух усиков [(широта, долгота)].
#[derive(Debug, Clone, PartialEq)]
pub struct Arrow {
    pub lat: f64,
    pub lon: f64,
    pub speed: f64,
    pub azimuth: f64,
    pub lines: Vec<Vec<(f64, f64)>>,
}

/// Стрелки скоростей у вида с серединой (lat, lon) шириной width_m
/// метров. Узлы - сетка шага [`step_for`] в WINDOW ширинах вида, широты
/// до MAX_LAT.
pub fn arrows(motion: &Motion, lat: f64, lon: f64, width_m: f64) -> Vec<Arrow> {
    let step = step_for(width_m);
    // Охват - половина WINDOW ширин вида в каждую сторону, не дальше
    // видимого полушария.
    let reach = (width_m * WINDOW / 2.0 / 111_320.0).min(90.0);
    let lat0 = (lat / step).floor() * step;
    let lon0 = (lon / step).floor() * step;
    let count = (reach / step) as i64 + 1;
    let mut out = Vec::new();
    for i in -count..=count {
        let la = lat0 + i as f64 * step;
        if la.abs() > MAX_LAT {
            continue;
        }
        // Долготы - в охвате вида, растянутом к полюсам, но не больше
        // одного оборота.
        let stretch = 1.0 / la.to_radians().cos().max(0.2);
        let half = (reach * stretch).min(180.0);
        let across = (half / step) as i64;
        for j in -across..=across {
            if half >= 180.0 && j == across {
                continue; // -180° и 180° - одна долгота
            }
            let lo = (lon0 + j as f64 * step + 180.0).rem_euclid(360.0) - 180.0;
            let Some((_, east, north)) = motion.velocity(la, lo) else {
                continue;
            };
            let (speed, azimuth) = speed_azimuth(east, north);
            if speed < 0.5 {
                continue;
            }
            let length = step * LENGTH_SHARE * speed / REFERENCE;
            let (de, dn) = (east / speed * length, north / speed * length);
            let tip = offset(la, lo, de, dn);
            let mut lines = vec![vec![(la, lo), tip]];
            let head = length * HEAD_SHARE;
            for turn in [HEAD_ANGLE, -HEAD_ANGLE] {
                let a = (azimuth + 180.0 + turn).to_radians();
                lines.push(vec![tip, offset(tip.0, tip.1, head * a.sin(), head * a.cos())]);
            }
            out.push(Arrow { lat: la, lon: lo, speed, azimuth, lines });
        }
    }
    out
}
