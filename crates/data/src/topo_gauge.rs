// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Отчёт по створу: морфометрия водосбора от точки замыкания и водосбор
//! линии (канава, лоток, дорога). Перенос `core/topo_gauge.py` модуля
//! (из Isoliner, `grid_isolines/topo_gauge.py`, владелец тот же).
//!
//! Соглашения те же, что в [`crate::topo_flow`]: сетка rows × cols по
//! строкам, номер узла `r * cols + c`, downstream - номер приёмника или
//! -1 у стока, размер ячейки в метрах. Перевод в километры и квадратные
//! километры делают отчёты.
//!
//! Отчёт - [`Report`]: значения по ключам REPORT_KEYS или DITCH_KEYS
//! модуля в их порядке, пустое значение - величина недоступна (ноль -
//! это измерение). Строки сводки - английский шаблон с подстановками
//! `{имя}` и значения подстановок: шаблон - ключ перевода, интерфейс
//! переводит его своим `trf`. Выбор помощника - у модуля шаблон
//! русский и переводчик передаётся в функцию.
//!
//! Сознательно не входит, как у модуля: расходы, модули стока,
//! гидравлика, снеготаяние.

use crate::topo_flow::{self, D8};

/// Ключи отчёта по створу, в порядке вывода.
pub const REPORT_KEYS: [&str; 15] = [
    // Площадь бассейна, км².
    "area_km2",
    // Средняя высота, м.
    "z_mean",
    // Наименьшая высота, м.
    "z_min",
    // Наибольшая высота, м.
    "z_max",
    // Средний уклон бассейна в единицах поданного растра.
    "slope_mean",
    // Отметка створа, м.
    "z_gauge",
    // Длина главного водотока от створа до истока, км.
    "stream_km",
    // Падение водотока, м.
    "stream_fall_m",
    // Средний уклон водотока, промилле.
    "stream_ppm",
    // Средний уклон склонов по СП 33-101, промилле.
    "sp_slope_ppm",
    // Суммарная длина горизонталей в водосборе, км.
    "sp_iso_km",
    // Средневзвешенный уклон водотока по СП, промилле.
    "sp_stream_ppm",
    // Длина речной сети внутри водосбора, км.
    "net_km",
    // Густота речной сети, км на км².
    "net_dens",
    // Ячеек в бассейне.
    "cells",
];

/// Ключи отчёта по линейному водосбору, в порядке вывода.
pub const DITCH_KEYS: [&str; 9] = [
    // Площадь водосбора, км².
    "area_km2",
    // Средняя высота, м.
    "z_mean",
    // Наименьшая высота, м.
    "z_min",
    // Наибольшая высота, м.
    "z_max",
    // Средний уклон водосбора в единицах растра.
    "slope_mean",
    // Длина трассы или контура, км.
    "trace_km",
    // Площадь затравки, км².
    "seed_km2",
    // Ячеек затравки на сетке.
    "trace_cells",
    // Ячеек в водосборе.
    "cells",
];

/// Отчёт: значения по ключам в порядке ключей. Количества ячеек -
/// тоже числа.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    entries: Vec<(&'static str, Option<f64>)>,
}

impl Report {
    /// Пустой отчёт с ключами keys.
    pub fn new(keys: &[&'static str]) -> Report {
        Report { entries: keys.iter().map(|k| (*k, None)).collect() }
    }

    /// Значение по ключу, None - нет величины или нет ключа.
    pub fn get(&self, key: &str) -> Option<f64> {
        self.entries.iter().find(|e| e.0 == key).and_then(|e| e.1)
    }

    /// Записать значение ключа. Чужой ключ не добавляется.
    pub fn set(&mut self, key: &str, value: Option<f64>) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.0 == key) {
            e.1 = value;
        }
    }

    /// Ключи и значения в порядке вывода.
    pub fn entries(&self) -> &[(&'static str, Option<f64>)] {
        &self.entries
    }
}

/// Строка сводки: английский шаблон с подстановками `{имя}` и значения
/// подстановок.
pub type TextLine = (&'static str, Vec<(&'static str, String)>);

/// Строка сводки без перевода: шаблон с подставленными значениями.
pub fn render(line: &TextLine) -> String {
    let mut out = line.0.to_string();
    for (key, value) in &line.1 {
        out = out.replace(&format!("{{{key}}}"), value);
    }
    out
}

/// Необязательные входы отчётов.
#[derive(Debug, Clone, Copy, Default)]
pub struct Extras<'a> {
    /// Растр уклона: средний уклон бассейна в его единицах.
    pub slope: Option<&'a [f64]>,
    /// Узлы без данных, в статистику не идут.
    pub nodata: Option<&'a [bool]>,
    /// Сечение горизонталей для уклона склонов по СП, 0 - не считать.
    pub iso_interval: f64,
    /// Порог аккумуляции речной сети, ячеек, 0 - не считать.
    pub net_threshold: f64,
}

fn off(nodata: Option<&[bool]>, i: usize) -> bool {
    nodata.is_some_and(|m| m.get(i).copied().unwrap_or(false))
}

/// Притяжка створа: узел наибольшей аккумуляции в квадратном окне
/// радиуса radius_cells вокруг (r, c), 0 - без притяжки. Точка за краем
/// зажимается в сетку. При равных значениях - первый по обходу окна
/// по строкам.
pub fn snap_to_max_acc(acc: &[f64], rows: usize, cols: usize, r: i64, c: i64, radius_cells: i64) -> (usize, usize) {
    if rows == 0 || cols == 0 {
        return (0, 0);
    }
    let r = r.clamp(0, rows as i64 - 1) as usize;
    let c = c.clamp(0, cols as i64 - 1) as usize;
    let k = radius_cells.max(0) as usize;
    if k == 0 || acc.len() != rows * cols {
        return (r, c);
    }
    let (r0, r1) = (r.saturating_sub(k), (r.saturating_add(k)).min(rows - 1));
    let (c0, c1) = (c.saturating_sub(k), (c.saturating_add(k)).min(cols - 1));
    let mut best = (r0, c0);
    let mut top = f64::NEG_INFINITY;
    for rr in r0..=r1 {
        for cc in c0..=c1 {
            let v = acc[rr * cols + cc];
            if v > top {
                top = v;
                best = (rr, cc);
            }
        }
    }
    best
}

/// Маска водосбора от створа: true у узлов, стекающих через семя,
/// включая само семя.
pub fn basin_mask(downstream: &[i64], rows: usize, cols: usize, seed: usize) -> Vec<bool> {
    topo_flow::basins(downstream, rows, cols, Some(&[(seed, 1)]), None, None, None).iter().map(|l| *l == 1).collect()
}

/// Суммарная длина горизонталей внутри маски, метры: переходы через
/// уровень между соседними узлами, каждый - отрезок изолинии около
/// размера ячейки поперёк ребра. Нужна для среднего уклона склонов по
/// СП 33-101-2003: Iск = h·Σli / (2A).
pub fn contour_length_in_mask(z: &[f64], rows: usize, cols: usize, mask: &[bool], cellsize: f64, interval: f64, nodata: Option<&[bool]>) -> f64 {
    let n = rows.saturating_mul(cols);
    if z.len() != n || mask.len() != n || interval.is_nan() || interval <= 0.0 {
        return 0.0;
    }
    let lev: Vec<f64> = (0..n).map(|i| if mask[i] && !off(nodata, i) && z[i].is_finite() { (z[i] / interval).floor() } else { f64::NAN }).collect();
    let cross = |a: usize, b: usize| lev[a].is_finite() && lev[b].is_finite() && lev[a] != lev[b];
    let mut total = 0usize;
    for r in 0..rows {
        for c in 0..cols {
            let i = r * cols + c;
            if c + 1 < cols && cross(i, i + 1) {
                total += 1;
            }
            if r + 1 < rows && cross(i, i + cols) {
                total += 1;
            }
        }
    }
    total as f64 * cellsize
}

/// Суммарная длина речной сети внутри водосбора, метры, по звеньям
/// решётки стока: диагональ весит корень из двух ячеек. Звено
/// засчитывается, когда оба его узла сетевые (аккумуляция не ниже
/// порога) и оба внутри маски. Порог 0 и меньше - None: ноль это
/// измерение, None - отсутствие измерения.
#[allow(clippy::too_many_arguments)]
pub fn network_length_in_mask(downstream: &[i64], acc: &[f64], rows: usize, cols: usize, mask: &[bool], cellsize: f64, threshold: f64, nodata: Option<&[bool]>) -> Option<f64> {
    if threshold.is_nan() || threshold <= 0.0 {
        return None;
    }
    let n = rows.saturating_mul(cols);
    if downstream.len() != n || acc.len() != n || mask.len() != n {
        return Some(0.0);
    }
    let net = |i: usize| mask[i] && !off(nodata, i) && acc[i].is_finite() && acc[i] >= threshold;
    let mut steps = 0.0;
    for (i, &d) in downstream.iter().enumerate() {
        if d < 0 || d as usize >= n || !net(i) || !net(d as usize) {
            continue;
        }
        let d = d as usize;
        let dr = (d / cols) as f64 - (i / cols) as f64;
        let dc = (d % cols) as f64 - (i % cols) as f64;
        steps += dr.hypot(dc);
    }
    Some(steps * cellsize)
}

/// Длина сети и её густота в отчёт. Порог 0 - оба поля пустые.
#[allow(clippy::too_many_arguments)]
fn fill_network(rep: &mut Report, downstream: &[i64], acc: &[f64], rows: usize, cols: usize, mask: &[bool], cellsize: f64, extras: &Extras) {
    let Some(net_m) = network_length_in_mask(downstream, acc, rows, cols, mask, cellsize, extras.net_threshold, extras.nodata) else {
        return;
    };
    let net_km = net_m / 1000.0;
    rep.set("net_km", Some(net_km));
    if let Some(area) = rep.get("area_km2").filter(|a| *a != 0.0) {
        rep.set("net_dens", Some(net_km / area));
    }
}

/// Средний уклон склонов Iск по СП 33-101-2003, промилле:
/// Iск = h·Σli / (2A). Эмпирическая величина методики СП, на плоскости
/// даёт половину тангенса.
pub fn sp_slope_ppm(iso_len_m: f64, area_m2: f64, interval: f64) -> Option<f64> {
    if area_m2.is_nan() || interval.is_nan() || area_m2 <= 0.0 || interval <= 0.0 {
        return None;
    }
    Some(interval * iso_len_m / (2.0 * area_m2) * 1000.0)
}

/// Средневзвешенный уклон водотока по СП, промилле: произведение частных
/// уклонов участков между точками перегиба профиля в степени li/L.
/// z_path - отметки от истока к створу, NaN отбрасываются. Ровные
/// участки исключаются, как у модуля.
pub fn sp_stream_ppm(z_path: &[f64], cellsize: f64, path_len_m: Option<f64>) -> Option<f64> {
    let zs: Vec<f64> = z_path.iter().copied().filter(|v| !v.is_nan()).collect();
    let n = zs.len();
    if n < 2 {
        return None;
    }
    let total = match path_len_m {
        Some(l) if l != 0.0 && !l.is_nan() => l,
        _ => cellsize * (n - 1) as f64,
    };
    if total.is_nan() || total <= 0.0 {
        return None;
    }
    // Точки перегиба: там, где меняется знак второй разности профиля.
    let mut idx = vec![0];
    if n > 2 {
        let d2: Vec<f64> = (0..n - 2).map(|i| zs[i + 2] - 2.0 * zs[i + 1] + zs[i]).collect();
        for i in 0..d2.len().saturating_sub(1) {
            if d2[i] * d2[i + 1] < 0.0 {
                idx.push(i + 1);
            }
        }
    }
    idx.push(n - 1);
    idx.sort_unstable();
    idx.dedup();
    let seg = total / (n - 1) as f64;
    let mut log_sum = 0.0;
    for pair in idx.windows(2) {
        let li = seg * (pair[1] - pair[0]) as f64;
        if li <= 0.0 {
            continue;
        }
        let ii = (zs[pair[1]] - zs[pair[0]]).abs() / li;
        if ii.is_nan() || ii <= 0.0 {
            continue;
        }
        log_sum += (li / total) * ii.ln();
    }
    if log_sum == 0.0 || !log_sum.is_finite() {
        return None;
    }
    Some(log_sum.exp() * 1000.0)
}

/// Среднее, наименьшее и наибольшее по маске: (mean, min, max). Nodata
/// и не числа исключаются, пустая выборка - None.
pub fn zonal_stats(values: &[f64], mask: &[bool], nodata: Option<&[bool]>) -> Option<(f64, f64, f64)> {
    let (mut sum, mut count, mut lo, mut hi) = (0.0, 0usize, f64::INFINITY, f64::NEG_INFINITY);
    for (i, (v, m)) in values.iter().zip(mask).enumerate() {
        if *m && !off(nodata, i) && v.is_finite() {
            sum += v;
            count += 1;
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
    }
    (count > 0).then(|| (sum / count as f64, lo, hi))
}

/// Главный водоток вверх от створа: (длина, м, путь номерами узлов от
/// створа к истоку). Шаг - в соседа, который стекает в текущий узел
/// и несёт наибольшую аккумуляцию, при равной - первый по D8. Старт за
/// решёткой - (0, пустой путь).
pub fn trace_main_stream(downstream: &[i64], acc: &[f64], rows: usize, cols: usize, start: usize, cellsize: f64) -> (f64, Vec<usize>) {
    let n = rows.saturating_mul(cols);
    if start >= n || downstream.len() != n || acc.len() != n {
        return (0.0, Vec::new());
    }
    let mut cur = start;
    let mut path = vec![cur];
    let mut total = 0.0;
    // Страховка от зацикливания.
    for _ in 0..=n {
        let (r, c) = ((cur / cols) as i64, (cur % cols) as i64);
        let mut best: Option<(usize, f64)> = None;
        let mut best_acc = f64::NEG_INFINITY;
        for (dr, dc, _, dist) in D8 {
            let (rr, cc) = (r + dr, c + dc);
            if rr < 0 || cc < 0 || rr >= rows as i64 || cc >= cols as i64 {
                continue;
            }
            let j = rr as usize * cols + cc as usize;
            if downstream[j] == cur as i64 && acc[j] > best_acc {
                best_acc = acc[j];
                best = Some((j, dist));
            }
        }
        let Some((j, dist)) = best else {
            break;
        };
        total += dist * cellsize;
        path.push(j);
        cur = j;
    }
    (total, path)
}

/// Водоток в отчёт: длина, падение, уклоны по пути от створа к истоку.
fn fill_stream(rep: &mut Report, z: &[f64], path: &[usize], length: f64, cellsize: f64) {
    if !(length > 0.0 && path.len() > 1) {
        return;
    }
    let height = |i: usize| z.get(i).copied().unwrap_or(f64::NAN);
    let (z0, z1) = (height(path[0]), height(path[path.len() - 1]));
    rep.set("stream_km", Some(length / 1000.0));
    if z0.is_finite() && z1.is_finite() {
        let fall = z1 - z0;
        rep.set("stream_fall_m", Some(fall));
        rep.set("stream_ppm", Some(fall / length * 1000.0));
    }
    // Профиль от истока к створу: методика СП идёт сверху вниз.
    let profile: Vec<f64> = path.iter().rev().map(|i| height(*i)).collect();
    rep.set("sp_stream_ppm", sp_stream_ppm(&profile, cellsize, Some(length)));
}

/// Площадь, отметки, уклон и горизонтали по маске в отчёт.
fn fill_area(rep: &mut Report, z: &[f64], rows: usize, cols: usize, mask: &[bool], cellsize: f64, extras: &Extras) {
    let cells = mask.iter().filter(|m| **m).count();
    rep.set("cells", Some(cells as f64));
    rep.set("area_km2", Some(cells as f64 * cellsize * cellsize / 1e6));
    if let Some((mean, lo, hi)) = zonal_stats(z, mask, extras.nodata) {
        rep.set("z_mean", Some(mean));
        rep.set("z_min", Some(lo));
        rep.set("z_max", Some(hi));
    }
    if let Some(slope) = extras.slope {
        rep.set("slope_mean", zonal_stats(slope, mask, extras.nodata).map(|s| s.0));
    }
    if extras.iso_interval != 0.0 && !extras.iso_interval.is_nan() {
        let iso = contour_length_in_mask(z, rows, cols, mask, cellsize, extras.iso_interval, extras.nodata);
        rep.set("sp_iso_km", Some(iso / 1000.0));
        rep.set("sp_slope_ppm", sp_slope_ppm(iso, cells as f64 * cellsize * cellsize, extras.iso_interval));
    }
}

/// Длины входов сходятся с сеткой.
fn fits(n: usize, extras: &Extras, lens: &[usize]) -> bool {
    lens.iter().all(|l| *l == n) && extras.slope.is_none_or(|s| s.len() == n) && extras.nodata.is_none_or(|m| m.len() == n)
}

/// Маска без nodata.
fn without(mut mask: Vec<bool>, nodata: Option<&[bool]>) -> Vec<bool> {
    if let Some(m) = nodata {
        for (v, e) in mask.iter_mut().zip(m) {
            *v &= !*e;
        }
    }
    mask
}

/// Морфометрия бассейна от створа seed (уже притянутого): отчёт по
/// REPORT_KEYS. z - отметки, downstream и acc - решётка стока. Входы не
/// той длины дают отчёт с нулём ячеек.
#[allow(clippy::too_many_arguments)]
pub fn gauge_report(z: &[f64], downstream: &[i64], acc: &[f64], rows: usize, cols: usize, seed: usize, cellsize: f64, extras: &Extras) -> Report {
    let mut rep = Report::new(&REPORT_KEYS);
    let n = rows.saturating_mul(cols);
    if !fits(n, extras, &[z.len(), downstream.len(), acc.len()]) || seed >= n {
        rep.set("cells", Some(0.0));
        rep.set("area_km2", Some(0.0));
        return rep;
    }
    let mask = without(basin_mask(downstream, rows, cols, seed), extras.nodata);
    fill_area(&mut rep, z, rows, cols, &mask, cellsize, extras);
    fill_network(&mut rep, downstream, acc, rows, cols, &mask, cellsize, extras);
    rep.set("z_gauge", Some(z[seed]).filter(|v| v.is_finite()));
    let (length, path) = trace_main_stream(downstream, acc, rows, cols, seed, cellsize);
    fill_stream(&mut rep, z, &path, length, cellsize);
    rep
}

/// Точка замыкания готового водосбора: узел маски с наибольшей
/// аккумуляцией, без неё - самый низкий. None - в маске нет годных
/// узлов.
pub fn outlet_in_mask(z: &[f64], mask: &[bool], acc: Option<&[f64]>, nodata: Option<&[bool]>) -> Option<usize> {
    let inside: Vec<usize> = (0..mask.len()).filter(|i| mask[*i] && !off(nodata, *i)).collect();
    if inside.is_empty() {
        return None;
    }
    if let Some(acc) = acc {
        let mut best: Option<(usize, f64)> = None;
        for &i in &inside {
            if let Some(a) = acc.get(i).copied().filter(|a| a.is_finite())
                && best.is_none_or(|b| a > b.1)
            {
                best = Some((i, a));
            }
        }
        if let Some(b) = best {
            return Some(b.0);
        }
    }
    let mut best: Option<(usize, f64)> = None;
    for &i in &inside {
        if let Some(v) = z.get(i).copied().filter(|v| v.is_finite())
            && best.is_none_or(|b| v < b.1)
        {
            best = Some((i, v));
        }
    }
    best.map(|b| b.0)
}

/// Морфометрия готового водосбора - маски извне: отчёт по REPORT_KEYS.
///
/// flow - решётка стока (downstream, acc), нужна длине сети и водотоку.
/// seed - створ, без него - outlet_in_mask. Водоток обрезается границей
/// маски. trace_stream false отключает трассировку водотока, сеть
/// считается всё равно.
#[allow(clippy::too_many_arguments)]
pub fn mask_report(z: &[f64], rows: usize, cols: usize, mask: &[bool], cellsize: f64, flow: Option<(&[i64], &[f64])>, seed: Option<usize>, extras: &Extras, trace_stream: bool) -> Report {
    let mut rep = Report::new(&REPORT_KEYS);
    let n = rows.saturating_mul(cols);
    let flow = flow.filter(|(d, a)| d.len() == n && a.len() == n);
    if !fits(n, extras, &[z.len(), mask.len()]) {
        rep.set("cells", Some(0.0));
        rep.set("area_km2", Some(0.0));
        return rep;
    }
    let m = without(mask.to_vec(), extras.nodata);
    if !m.iter().any(|v| *v) {
        rep.set("cells", Some(0.0));
        rep.set("area_km2", Some(0.0));
        return rep;
    }
    fill_area(&mut rep, z, rows, cols, &m, cellsize, extras);
    if let Some((down, acc)) = flow {
        fill_network(&mut rep, down, acc, rows, cols, &m, cellsize, extras);
    }
    if !trace_stream {
        return rep;
    }
    let seed = seed.or_else(|| outlet_in_mask(z, &m, flow.map(|f| f.1), None));
    let (Some(seed), Some((down, acc))) = (seed.filter(|s| *s < n), flow) else {
        return rep;
    };
    rep.set("z_gauge", Some(z[seed]).filter(|v| v.is_finite()));
    let (mut length, mut path) = trace_main_stream(down, acc, rows, cols, seed, cellsize);
    // Водоток обрезается границей маски: длина внутри заданного водосбора.
    if !path.is_empty() {
        let keep: Vec<usize> = path.iter().copied().take_while(|i| m[*i]).collect();
        if keep.len() > 1 {
            length = length * (keep.len() - 1) as f64 / (path.len() - 1).max(1) as f64;
            path = keep;
        } else {
            path.clear();
        }
    }
    fill_stream(&mut rep, z, &path, length, cellsize);
    rep
}

fn number(v: Option<f64>, digits: usize) -> String {
    v.map_or_else(|| "-".to_string(), |v| format!("{v:.digits$}"))
}

fn count(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_string(), |v| format!("{}", v as i64))
}

/// Сводка по створу для экрана: две строки.
pub fn report_lines(rep: &Report) -> Vec<TextLine> {
    vec![
        (
            "Basin: {area} km², heights {low}...{high} m, mean {mean} m.",
            vec![("area", number(rep.get("area_km2"), 3)), ("low", number(rep.get("z_min"), 2)), ("high", number(rep.get("z_max"), 2)), ("mean", number(rep.get("z_mean"), 2))],
        ),
        (
            "Stream: {length} km, fall {fall} m, slope {slope} per mille, gauge height {gauge} m.",
            vec![("length", number(rep.get("stream_km"), 3)), ("fall", number(rep.get("stream_fall_m"), 2)), ("slope", number(rep.get("stream_ppm"), 1)), ("gauge", number(rep.get("z_gauge"), 2))],
        ),
    ]
}

// --- линейный водосбор: канава, лоток, дорога ----------------------------------
//
// Трасса растеризуется в ячейки, все они - семена, и водосбор - множество
// ячеек, чей путь стока приходит в любую ячейку трассы. Врезка нужна для
// другого вопроса - удержит ли канава поток - и остаётся отдельной
// необязательной операцией.

/// Шаг обхода сегментов cells_along_polyline по умолчанию, доли ячейки.
pub const STEP: f64 = 0.5;
/// Наибольшее количество шагов по одному сегменту: защита от враждебных
/// координат.
const MOST_STEPS: f64 = 4_194_304.0;

/// Ячейки, через которые проходит ломаная: номера в порядке обхода без
/// повторов.
///
/// vertices - точки (x, y) в координатах карты, origin_x и origin_y -
/// левый верхний угол сетки, cell - размер ячейки. Сегменты идут
/// с шагом step ячейки, пропусков на диагоналях нет. Точки вне сетки
/// отбрасываются.
pub fn cells_along_polyline(vertices: &[(f64, f64)], origin_x: f64, origin_y: f64, cell: f64, rows: usize, cols: usize, step: f64) -> Vec<usize> {
    let mut out = Vec::new();
    if vertices.is_empty() || !(cell.is_finite() && cell > 0.0) {
        return out;
    }
    let mut seen = std::collections::HashSet::new();
    let mut put = |x: f64, y: f64, out: &mut Vec<usize>| {
        let c = ((x - origin_x) / cell).floor();
        let r = ((origin_y - y) / cell).floor();
        if r >= 0.0 && c >= 0.0 && r < rows as f64 && c < cols as f64 {
            let idx = r as usize * cols + c as usize;
            if seen.insert(idx) {
                out.push(idx);
            }
        }
    };
    put(vertices[0].0, vertices[0].1, &mut out);
    for pair in vertices.windows(2) {
        let ((x1, y1), (x2, y2)) = (pair[0], pair[1]);
        let d = (x2 - x1).hypot(y2 - y1);
        let steps = (d / (cell * step)).ceil();
        let n = if steps.is_finite() && steps >= 1.0 { steps.min(MOST_STEPS) as usize } else { 1 };
        for k in 1..=n {
            let t = k as f64 / n as f64;
            put(x1 + (x2 - x1) * t, y1 + (y2 - y1) * t, &mut out);
        }
    }
    out
}

/// Длина ломаной в единицах карты.
pub fn polyline_length(vertices: &[(f64, f64)]) -> f64 {
    vertices.windows(2).map(|p| (p[1].0 - p[0].0).hypot(p[1].1 - p[0].1)).sum()
}

/// Ячейки внутренности полигона плюс ячейки его контура: номера без
/// повторов.
///
/// rings - внешние кольца, дырки не передаются: затравка контура
/// (карьера) - сплошная внутренность. Внутренность - строчной
/// развёрткой по центрам ячеек (полуоткрытое правило пересечения),
/// контур - cells_along_polyline, чтобы узкие части не выпадали. Кольцо
/// замыкается само.
pub fn cells_in_polygon(rings: &[Vec<(f64, f64)>], origin_x: f64, origin_y: f64, cell: f64, rows: usize, cols: usize) -> Vec<usize> {
    let mut out = Vec::new();
    if !(cell.is_finite() && cell > 0.0) || rows == 0 || cols == 0 {
        return out;
    }
    let mut seen = std::collections::HashSet::new();
    for ring in rings {
        let mut pts = ring.clone();
        if pts.len() < 3 {
            continue;
        }
        if pts[0] != pts[pts.len() - 1] {
            pts.push(pts[0]);
        }
        let top = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
        let bottom = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
        let r0 = ((origin_y - top) / cell).floor().max(0.0);
        let r1 = ((origin_y - bottom) / cell).floor().min(rows as f64 - 1.0);
        if r0.is_finite() && r1.is_finite() && r0 <= r1 {
            for r in r0 as usize..=r1 as usize {
                let yc = origin_y - (r as f64 + 0.5) * cell;
                let mut xs: Vec<f64> = Vec::new();
                for p in pts.windows(2) {
                    let ((x1, y1), (x2, y2)) = (p[0], p[1]);
                    if (y1 <= yc && yc < y2) || (y2 <= yc && yc < y1) {
                        xs.push(x1 + (x2 - x1) * (yc - y1) / (y2 - y1));
                    }
                }
                xs.sort_by(f64::total_cmp);
                for k in (0..xs.len().saturating_sub(1)).step_by(2) {
                    let c0 = ((xs[k] - origin_x) / cell - 0.5).ceil().max(0.0);
                    let c1 = ((xs[k + 1] - origin_x) / cell - 0.5).floor().min(cols as f64 - 1.0);
                    if !(c0.is_finite() && c1.is_finite()) || c0 > c1 {
                        continue;
                    }
                    for c in c0 as usize..=c1 as usize {
                        if seen.insert(r * cols + c) {
                            out.push(r * cols + c);
                        }
                    }
                }
            }
        }
        for idx in cells_along_polyline(&pts, origin_x, origin_y, cell, rows, cols, STEP) {
            if seen.insert(idx) {
                out.push(idx);
            }
        }
    }
    out
}

/// Водосбор набора узлов: true у всех, чей путь стока приходит в любое
/// из семян. Семена входят в водосбор.
pub fn catchment_mask(downstream: &[i64], rows: usize, cols: usize, seeds: &[usize]) -> Vec<bool> {
    let n = rows.saturating_mul(cols);
    if seeds.is_empty() {
        return vec![false; n];
    }
    let pairs: Vec<(usize, i32)> = seeds.iter().map(|i| (*i, 1)).collect();
    topo_flow::basins(downstream, rows, cols, Some(&pairs), None, None, None).iter().map(|l| *l == 1).collect()
}

/// Врезка трассы: копия рельефа, опущенная на depth в узлах трассы.
/// Узлы nodata и номера за сеткой не трогаются.
pub fn burn_trace(z: &[f64], seeds: &[usize], depth: f64, nodata: Option<&[bool]>) -> Vec<f64> {
    let mut out = z.to_vec();
    if depth.is_nan() || depth <= 0.0 {
        return out;
    }
    for &i in seeds {
        if !off(nodata, i) && let Some(v) = out.get_mut(i) {
            *v -= depth;
        }
    }
    out
}

/// Врезка жёлобом: дно каждой трассы опускается монотонно в сторону
/// стока (приём Ивана Иванова).
///
/// runs - трассы, узлы в порядке вдоль линии. Сток - к нижнему концу
/// врезанного профиля. Дно - z минус depth, затем бегущий минимум
/// с обязательным падением slope·cell на узел. Выше «рельеф минус depth»
/// дно не поднимается. Узлы nodata и номера за сеткой пропускаются.
pub fn burn_trace_sloped(z: &[f64], runs: &[Vec<usize>], depth: f64, slope: f64, cell: f64, nodata: Option<&[bool]>) -> Vec<f64> {
    let mut out = z.to_vec();
    if depth.is_nan() || depth <= 0.0 || runs.is_empty() {
        return out;
    }
    let step = slope * cell;
    for run in runs {
        let idx: Vec<usize> = run.iter().copied().filter(|i| *i < z.len() && !off(nodata, *i)).collect();
        if idx.is_empty() {
            continue;
        }
        let mut prof: Vec<f64> = idx.iter().map(|i| out[*i] - depth).collect();
        // Сток к началу трассы - профиль переворачивается на время расчёта.
        let back = idx.len() > 1 && prof[0] < prof[prof.len() - 1];
        if back {
            prof.reverse();
        }
        for k in 1..prof.len() {
            prof[k] = prof[k].min(prof[k - 1] - step);
        }
        if back {
            prof.reverse();
        }
        for (i, v) in idx.iter().zip(prof) {
            out[*i] = v;
        }
    }
    out
}

/// Морфометрия водосбора линии: отчёт по DITCH_KEYS. trace_len - длина
/// трассы, м. Трассировки водотока нет: вода приходит в трассу со всей
/// площади.
#[allow(clippy::too_many_arguments)]
pub fn ditch_report(z: &[f64], downstream: &[i64], rows: usize, cols: usize, seeds: &[usize], cellsize: f64, trace_len: Option<f64>, extras: &Extras) -> Report {
    let mut rep = Report::new(&DITCH_KEYS);
    let n = rows.saturating_mul(cols);
    rep.set("trace_cells", Some(seeds.len() as f64));
    rep.set("seed_km2", Some(seeds.len() as f64 * cellsize * cellsize / 1e6));
    if let Some(len) = trace_len {
        rep.set("trace_km", Some(len / 1000.0));
    }
    if !fits(n, extras, &[z.len(), downstream.len()]) {
        rep.set("cells", Some(0.0));
        rep.set("area_km2", Some(0.0));
        return rep;
    }
    let mask = without(catchment_mask(downstream, rows, cols, seeds), extras.nodata);
    let area_only = Extras { iso_interval: 0.0, ..*extras };
    fill_area(&mut rep, z, rows, cols, &mask, cellsize, &area_only);
    rep
}

/// Сводка по линейному водосбору для экрана: две строки.
pub fn ditch_report_lines(rep: &Report) -> Vec<TextLine> {
    vec![
        (
            "Catchment: {area} km², heights {low}...{high} m, mean {mean} m.",
            vec![("area", number(rep.get("area_km2"), 3)), ("low", number(rep.get("z_min"), 2)), ("high", number(rep.get("z_max"), 2)), ("mean", number(rep.get("z_mean"), 2))],
        ),
        ("Trace: {length} km, trace cells {trace}, catchment cells {cells}.", vec![("length", number(rep.get("trace_km"), 3)), ("trace", count(rep.get("trace_cells"))), ("cells", count(rep.get("cells")))]),
    ]
}
