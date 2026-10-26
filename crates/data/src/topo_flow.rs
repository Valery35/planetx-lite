// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Гидрология рельефа: направления стока D8 (Jenson-Domingue 1988),
//! аккумуляция обходом Кана, речная сеть с порядком Стралера, бассейны,
//! линии стока вниз по D8. Перенос `core/topo_flow.py` модуля
//! (из Isoliner, `grid_isolines/topo_flow.py`, владелец тот же).
//!
//! Соглашения. Сетка по строкам, строка 0 северная. Номер узла -
//! `r * cols + c`. Коды направлений ESRI: E=1, SE=2, S=4, SW=8, W=16,
//! NW=32, N=64, NE=128, сток (яма или выход с сетки) - 0. `downstream` -
//! номер приёмника или -1 у стока. Вход D8 - высоты после заполнения
//! понижений с epsilon больше нуля: на плоскостях без уклона D8
//! не определён.
//!
//! Модуль считает векторами NumPy и прыжками указателей, здесь - простые
//! циклы за один проход: ответы те же. Указатели считаются враждебными:
//! номер за решёткой - сток, круг не зацикливает ни один обход.

/// Направления D8: (dr, dc, код ESRI, длина шага в ячейках).
pub const D8: [(i64, i64, u8, f64); 8] = [
    (0, 1, 1, 1.0),
    (1, 1, 2, std::f64::consts::SQRT_2),
    (1, 0, 4, 1.0),
    (1, -1, 8, std::f64::consts::SQRT_2),
    (0, -1, 16, 1.0),
    (-1, -1, 32, std::f64::consts::SQRT_2),
    (-1, 0, 64, 1.0),
    (-1, 1, 128, std::f64::consts::SQRT_2),
];

/// Индекс направления у стока и у nodata.
pub const NODIR: i8 = -1;

/// Приёмник узла i или None: сток, номер за решёткой.
fn next(downstream: &[i64], i: usize) -> Option<usize> {
    let d = *downstream.get(i)?;
    (d >= 0 && (d as usize) < downstream.len()).then_some(d as usize)
}

/// Маска пустых узлов: своя маска или NaN. None - маска не той длины.
fn empty_mask(z: &[f64], nodata: Option<&[bool]>) -> Option<Vec<bool>> {
    match nodata {
        Some(m) if m.len() != z.len() => None,
        Some(m) => Some(m.to_vec()),
        None => Some(z.iter().map(|v| !v.is_finite()).collect()),
    }
}

/// Направления стока D8: (индекс направления 0..7 в D8 или NODIR,
/// downstream).
///
/// Сосед nodata (море, вырез) считается бесконечно низким: береговой
/// узел льёт в него, его downstream -1. Заграничье, наоборот,
/// недостижимо: узел на рамке уходит с сетки, только если у него нет
/// более низкого соседа внутри. Из двух равных уклонов берётся первый по
/// списку D8. Без маски nodata - NaN высот. Сетка не того размера даёт
/// пустые векторы.
pub fn d8_directions(z: &[f64], rows: usize, cols: usize, nodata: Option<&[bool]>) -> (Vec<i8>, Vec<i64>) {
    let Some(empty) = empty_mask(z, nodata) else {
        return (Vec::new(), Vec::new());
    };
    if rows.checked_mul(cols) != Some(z.len()) {
        return (Vec::new(), Vec::new());
    }
    let mut dir = vec![NODIR; z.len()];
    let mut down = vec![-1i64; z.len()];
    for r in 0..rows {
        for c in 0..cols {
            let i = r * cols + c;
            if empty[i] {
                continue;
            }
            let mut best = 0.0;
            let mut pick = NODIR;
            let mut target = -1i64;
            for (k, (dr, dc, _, dist)) in D8.iter().enumerate() {
                let (rr, cc) = (r as i64 + dr, c as i64 + dc);
                if rr < 0 || cc < 0 || rr >= rows as i64 || cc >= cols as i64 {
                    continue;
                }
                let j = rr as usize * cols + cc as usize;
                let slope = if empty[j] { f64::INFINITY } else { (z[i] - z[j]) / dist };
                if slope > best {
                    best = slope;
                    pick = k as i8;
                    // Приёмник в nodata равносилен выходу с сетки.
                    target = if empty[j] { -1 } else { j as i64 };
                }
            }
            dir[i] = pick;
            down[i] = target;
        }
    }
    (dir, down)
}

/// Индексы направлений в коды ESRI, сток и nodata - 0.
pub fn dir_to_esri(dir: &[i8]) -> Vec<u8> {
    dir.iter().map(|d| if *d >= 0 && (*d as usize) < D8.len() { D8[*d as usize].2 } else { 0 }).collect()
}

/// Аккумуляция: количество узлов, стекающих в узел, включая его самого.
///
/// Обход Кана: входящие степени, фронт готовых узлов, суммы передаются
/// приёмникам. Узлы nodata весят 0. Узлы круга (указатели не от
/// заполнения) остаются с частичной суммой. Сетка не того размера -
/// пустой вектор.
pub fn flow_accumulation(downstream: &[i64], rows: usize, cols: usize, nodata: Option<&[bool]>) -> Vec<f64> {
    let n = downstream.len();
    if rows.checked_mul(cols) != Some(n) || nodata.is_some_and(|m| m.len() != n) {
        return Vec::new();
    }
    let off = |i: usize| nodata.is_some_and(|m| m[i]);
    let mut acc: Vec<f64> = (0..n).map(|i| if off(i) { 0.0 } else { 1.0 }).collect();
    let mut indeg = vec![0u32; n];
    for i in 0..n {
        if let Some(d) = next(downstream, i) {
            indeg[d] += 1;
        }
    }
    let mut frontier: Vec<usize> = (0..n).filter(|i| indeg[*i] == 0 && !off(*i)).collect();
    while let Some(i) = frontier.pop() {
        if let Some(d) = next(downstream, i) {
            acc[d] += acc[i];
            indeg[d] -= 1;
            if indeg[d] == 0 {
                frontier.push(d);
            }
        }
    }
    acc
}

/// Звено речной сети.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    /// Узлы вниз по течению, последний - узел замыкающего слияния,
    /// если звено в него вливается.
    pub cells: Vec<usize>,
    /// Порядок по Стралеру.
    pub order: u32,
    /// Аккумуляция в последнем узле звена.
    pub acc_out: f64,
}

/// Речная сеть по порогу аккумуляции.
///
/// Звено идёт от истока или узла слияния до следующего слияния или до
/// выхода из сети. Порядок звеньев модуля: сначала истоки по номеру
/// узла, затем слияния. Узлы круга без истока отбрасываются.
pub fn river_network(downstream: &[i64], acc: &[f64], threshold: f64, rows: usize, cols: usize) -> Vec<Link> {
    let n = downstream.len();
    if rows.checked_mul(cols) != Some(n) || acc.len() != n {
        return Vec::new();
    }
    let mask: Vec<bool> = acc.iter().map(|a| *a >= threshold).collect();
    let net_next = |i: usize| next(downstream, i).filter(|d| mask[*d]);
    // Входящая степень в сети.
    let mut deg = vec![0u32; n];
    for (i, on) in mask.iter().enumerate() {
        if *on && let Some(d) = net_next(i) {
            deg[d] += 1;
        }
    }
    let heads = (0..n).filter(|i| mask[*i] && deg[*i] == 0);
    let juncs = (0..n).filter(|i| mask[*i] && deg[*i] >= 2);
    let mut links: Vec<Link> = Vec::new();
    for start in heads.chain(juncs) {
        let mut cells = vec![start];
        let mut cur = start;
        // Внутренние узлы звена - со степенью 1, у них один сетевой исток.
        while let Some(d) = net_next(cur) {
            if deg[d] != 1 || cells.len() > n {
                break;
            }
            cells.push(d);
            cur = d;
        }
        if let Some(d) = net_next(cur) {
            // Замыкающее слияние.
            cells.push(d);
        }
        if cells.len() < 2 {
            // Исток, сразу упирающийся в край: точка не звено.
            continue;
        }
        let last = cells[cells.len() - 1];
        links.push(Link { cells, order: 0, acc_out: acc[last] });
    }
    // Стралер: вливающиеся звенья каждого узла.
    let mut inflows: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
    for (li, link) in links.iter().enumerate() {
        inflows.entry(link.cells[link.cells.len() - 1]).or_default().push(li);
    }
    // Явный стек вместо рекурсии. 0 - новое, 1 - в работе, 2 - готово.
    let mut state = vec![0u8; links.len()];
    for root in 0..links.len() {
        if state[root] == 2 {
            continue;
        }
        let mut stack = vec![root];
        while let Some(&cur) = stack.last() {
            if state[cur] == 2 {
                stack.pop();
                continue;
            }
            let start = links[cur].cells[0];
            let ups: &[usize] = if deg[start] >= 2 { inflows.get(&start).map_or(&[], Vec::as_slice) } else { &[] };
            if state[cur] == 0 {
                state[cur] = 1;
                let todo: Vec<usize> = ups.iter().copied().filter(|u| state[*u] == 0).collect();
                if !todo.is_empty() {
                    stack.extend(todo);
                    continue;
                }
            }
            // Звено в работе среди притоков означало бы круг: такой приток
            // идёт за порядок 1, как у модуля.
            let vals: Vec<u32> = ups.iter().map(|u| if state[*u] == 2 { links[*u].order } else { 1 }).collect();
            links[cur].order = match vals.iter().max() {
                None => 1,
                Some(top) => {
                    if vals.iter().filter(|v| *v == top).count() >= 2 {
                        top + 1
                    } else {
                        *top
                    }
                }
            };
            state[cur] = 2;
            stack.pop();
        }
    }
    links
}

/// Бассейны: метка каждого узла по его конечному стоку.
///
/// seeds - (номер узла, метка больше 0), точки замыкания: семя рвёт путь,
/// всё, что стекает через него, получает его метку. Без seeds (или с
/// пустыми, как у модуля) метятся устья - узлы, чей сток уходит с сетки,
/// а при заданных acc и threshold - только устья с аккумуляцией не ниже
/// порога, метки 1, 2... по номерам узлов. Остальное получает 0, nodata
/// тоже. Узлы круга без стока получают 0.
pub fn basins(downstream: &[i64], rows: usize, cols: usize, seeds: Option<&[(usize, i32)]>, nodata: Option<&[bool]>, acc: Option<&[f64]>, threshold: Option<f64>) -> Vec<i32> {
    let n = downstream.len();
    if rows.checked_mul(cols) != Some(n) || nodata.is_some_and(|m| m.len() != n) {
        return Vec::new();
    }
    let mut label = vec![0i32; n];
    let mut terminal: Vec<bool> = (0..n).map(|i| next(downstream, i).is_none()).collect();
    match seeds.filter(|s| !s.is_empty()) {
        Some(seeds) => {
            for (i, lab) in seeds {
                if *i < n {
                    label[*i] = *lab;
                    // Семя терминально.
                    terminal[*i] = true;
                }
            }
        }
        None => {
            let mut next_label = 1;
            for i in 0..n {
                let outlet = terminal[i] && !nodata.is_some_and(|m| m[i]);
                let big = match (acc, threshold) {
                    (Some(a), Some(t)) => a.get(i).is_some_and(|v| *v >= t),
                    _ => true,
                };
                if outlet && big {
                    label[i] = next_label;
                    next_label += 1;
                }
            }
        }
    }
    // Метка конечного стока: ход вниз до терминала или уже решённого узла.
    // 0 - не решён, 1 - на текущем пути, 2 - решён.
    let mut state = vec![0u8; n];
    let mut out = vec![0i32; n];
    let mut path = Vec::new();
    for s in 0..n {
        if state[s] == 2 {
            continue;
        }
        path.clear();
        let mut cur = s;
        let result = loop {
            if state[cur] == 2 {
                break out[cur];
            }
            if state[cur] == 1 {
                // Круг: стока нет.
                break 0;
            }
            state[cur] = 1;
            path.push(cur);
            if terminal[cur] {
                break label[cur];
            }
            match next(downstream, cur) {
                Some(d) => cur = d,
                None => break label[cur],
            }
        };
        for &p in &path {
            out[p] = result;
            state[p] = 2;
        }
    }
    if let Some(m) = nodata {
        for i in 0..n {
            if m[i] {
                out[i] = 0;
            }
        }
    }
    out
}

// --- линии стока: ход вниз по D8 от заданных узлов ----------------------------

/// Ход вниз по течению от каждого стартового узла: номера узлов от старта
/// вниз.
///
/// Путь обрывается в четырёх случаях: сток (ниже некуда), выход за край,
/// приход в узел stop (водоём или водоток, куда трасса вливается)
/// и попадание в уже пройденный узел seen, когда seen передан. С seen
/// трассы сливаются в дерево, каждый узел проходится один раз, seen
/// пополняется путями. Старт за решёткой даёт пустой путь. max_steps
/// ограничивает ход, без него - количество узлов плюс один. Маски
/// короче решётки читаются как false за своим концом.
pub fn trace_downhill(downstream: &[i64], starts: &[i64], rows: usize, cols: usize, stop: Option<&[bool]>, mut seen: Option<&mut [bool]>, max_steps: Option<usize>) -> Vec<Vec<usize>> {
    let n = rows.saturating_mul(cols).min(downstream.len());
    let limit = max_steps.filter(|m| *m > 0).unwrap_or(n + 1);
    let in_stop = |i: usize| stop.is_some_and(|s| s.get(i).copied().unwrap_or(false));
    let mut out = Vec::with_capacity(starts.len());
    for &s0 in starts {
        if s0 < 0 || s0 as usize >= n {
            out.push(Vec::new());
            continue;
        }
        let s0 = s0 as usize;
        let mut cur = s0;
        let mut path = vec![cur];
        let mut local = std::collections::HashSet::from([cur]);
        let in_seen = |seen: &Option<&mut [bool]>, i: usize| seen.as_ref().is_some_and(|s| s.get(i).copied().unwrap_or(false));
        while path.len() < limit {
            if cur != s0 && (in_seen(&seen, cur) || in_stop(cur)) {
                break;
            }
            let Some(nxt) = next(downstream, cur).filter(|d| *d < n) else {
                // Сток или выход за край.
                break;
            };
            if !local.insert(nxt) {
                // Петля: указатели пришли не от заполнения.
                break;
            }
            path.push(nxt);
            cur = nxt;
            if in_stop(cur) || in_seen(&seen, cur) {
                break;
            }
        }
        if let Some(s) = seen.as_deref_mut() {
            for &p in &path {
                if let Some(v) = s.get_mut(p) {
                    *v = true;
                }
            }
        }
        out.push(path);
    }
    out
}

/// Длина, перепад и средний уклон трассы.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathMetrics {
    /// Длина по осям ячейки, м.
    pub length: f64,
    /// Перепад от старта к концу, м. Отрицательный - сетка не заполнена.
    pub drop: f64,
    /// Средний уклон, м на м.
    pub slope: f64,
    pub z_start: f64,
    pub z_end: f64,
    /// Узлов в трассе.
    pub cells: usize,
}

/// Длина, перепад и средний уклон трассы path по высотам z. cell_y -
/// шаг по строкам у неквадратной ячейки, без него - cell_x. Номер за
/// решёткой читается как NaN высоты.
pub fn path_metrics(path: &[usize], z: &[f64], rows: usize, cols: usize, cell_x: f64, cell_y: Option<f64>) -> PathMetrics {
    let height = |i: usize| if i < rows.saturating_mul(cols) { z.get(i).copied().unwrap_or(f64::NAN) } else { f64::NAN };
    if path.len() < 2 {
        let z0 = path.first().map_or(f64::NAN, |i| height(*i));
        return PathMetrics { length: 0.0, drop: 0.0, slope: 0.0, z_start: z0, z_end: z0, cells: path.len() };
    }
    let length = steps(path, cols, cell_x, cell_y.unwrap_or(cell_x)).last().copied().unwrap_or(0.0);
    let (z0, z1) = (height(path[0]), height(path[path.len() - 1]));
    let drop = z0 - z1;
    PathMetrics { length, drop, slope: if length > 0.0 { drop / length } else { 0.0 }, z_start: z0, z_end: z1, cells: path.len() }
}

/// Накопленная длина вдоль пути от его начала, м: первый элемент 0.
fn steps(path: &[usize], cols: usize, cx: f64, cy: f64) -> Vec<f64> {
    let cols = cols.max(1);
    let mut s = Vec::with_capacity(path.len());
    let mut total = 0.0;
    for (k, &i) in path.iter().enumerate() {
        if k > 0 {
            let p = path[k - 1];
            let dr = (i / cols) as f64 - (p / cols) as f64;
            let dc = (i % cols) as f64 - (p % cols) as f64;
            total += (dc * cx).hypot(dr * cy);
        }
        s.push(total);
    }
    s
}

/// Чем закончилась трасса.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathReason {
    /// Трассы нет.
    Empty,
    /// Пришла в приёмник stop.
    Receiver,
    /// Край листа: за краем рельеф кончился.
    Edge,
    /// Сток: ниже некуда.
    Sink,
    /// Слилась с пройденной трассой seen.
    Merge,
    /// Оборвалась по пределу шагов.
    Break,
}

/// Чем закончилась трасса path.
pub fn path_reason(path: &[usize], downstream: &[i64], rows: usize, cols: usize, stop: Option<&[bool]>, seen: Option<&[bool]>) -> PathReason {
    let Some(&last) = path.last() else {
        return PathReason::Empty;
    };
    let flag = |m: Option<&[bool]>, i: usize| m.is_some_and(|m| m.get(i).copied().unwrap_or(false));
    if flag(stop, last) {
        return PathReason::Receiver;
    }
    let n = rows.saturating_mul(cols);
    let cols = cols.max(1);
    let (lr, lc) = (last / cols, last % cols);
    let on_edge = lr == 0 || lr + 1 == rows || lc == 0 || lc + 1 == cols;
    let nxt = downstream.get(last).copied().unwrap_or(-1);
    if nxt < 0 {
        // У рамки сток и уход за край неотличимы по указателю, но для
        // отчёта это разное: за краем рельеф просто кончился.
        return if on_edge { PathReason::Edge } else { PathReason::Sink };
    }
    let nxt = nxt as usize;
    if nxt >= n {
        return PathReason::Edge;
    }
    if flag(seen, nxt) {
        return PathReason::Merge;
    }
    if path.len() > 1 && nxt == path[path.len() - 2] {
        return PathReason::Sink;
    }
    PathReason::Break
}

/// Обрезать трассу там, где рельеф выполаживается: (путь, обрезано ли).
///
/// Уклон меряется осреднённо по последним window метрам пути. Начало
/// окна - последний узел, от которого до текущего не меньше окна. Режется
/// по началу окна: оттуда пошло выполаживание и поток растекается.
#[allow(clippy::too_many_arguments)]
pub fn cut_on_flattening(path: &[usize], z: &[f64], rows: usize, cols: usize, cell_x: f64, min_slope: f64, window: f64, cell_y: Option<f64>) -> (Vec<usize>, bool) {
    if path.len() < 3 || min_slope.is_nan() || min_slope <= 0.0 || window.is_nan() || window <= 0.0 {
        return (path.to_vec(), false);
    }
    let n = rows.saturating_mul(cols);
    let height = |i: usize| if i < n { z.get(i).copied().unwrap_or(f64::NAN) } else { f64::NAN };
    let s = steps(path, cols, cell_x, cell_y.unwrap_or(cell_x));
    let zz: Vec<f64> = path.iter().map(|i| height(*i)).collect();
    if s[s.len() - 1] <= window {
        return (path.to_vec(), false);
    }
    let mut j0 = 0;
    for j in 1..path.len() {
        while j0 + 1 < j && s[j] - s[j0 + 1] >= window {
            j0 += 1;
        }
        if s[j] - s[j0] < window {
            continue;
        }
        let drop = zz[j0] - zz[j];
        if drop / (s[j] - s[j0]) < min_slope {
            return (path[..=j0].to_vec(), true);
        }
    }
    (path.to_vec(), false)
}

/// Самый длинный участок трассы круче порога: (длина, м, начало и конец
/// в пути). Средний уклон участка не ниже min_slope.
///
/// Участок держит порог, когда z_i - z_j >= thr (s_j - s_i), то есть
/// g(i) >= g(j) для g(k) = z_k + thr s_k: самая широкая пара
/// с невозрастающим g.
pub fn steep_run(path: &[usize], z: &[f64], rows: usize, cols: usize, cell_x: f64, min_slope: f64, cell_y: Option<f64>) -> (f64, usize, usize) {
    if path.len() < 2 || min_slope.is_nan() || min_slope <= 0.0 {
        return (0.0, 0, 0);
    }
    let n = rows.saturating_mul(cols);
    let s = steps(path, cols, cell_x, cell_y.unwrap_or(cell_x));
    let g: Vec<f64> = path.iter().zip(&s).map(|(i, sk)| if *i < n { z.get(*i).copied().unwrap_or(f64::NAN) } else { f64::NAN } + min_slope * sk).collect();
    // Кандидаты на начало: индексы, где g строго растёт.
    let mut stack = vec![0];
    for k in 1..g.len() {
        if stack.last().is_some_and(|t| g[k] > g[*t]) {
            stack.push(k);
        }
    }
    let (mut best, mut bi, mut bj) = (0.0, 0, 0);
    for j in (1..g.len()).rev() {
        while let Some(&i) = stack.last() {
            if g[i] < g[j] || g[i].is_nan() || g[j].is_nan() {
                break;
            }
            stack.pop();
            if s[j] - s[i] > best {
                (best, bi, bj) = (s[j] - s[i], i, j);
            }
        }
        if stack.is_empty() {
            break;
        }
    }
    (best, bi, bj)
}

/// Уклон в м на м из градусов: пороги задают углом.
pub fn slope_from_degrees(deg: f64) -> f64 {
    deg.to_radians().tan()
}
