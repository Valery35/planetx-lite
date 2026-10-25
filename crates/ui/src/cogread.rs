// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно канала Cloud Optimized GeoTIFF по сети, как `read_window`
//! `ui/sentinel.py` модуля через GDAL /vsicurl. Разбор - `data::cog`.
//!
//! Начало файла просится одним запросом с заголовком Range, недостающие
//! куски заголовка - следующими. Потом выбирается обзор, у которого шаг
//! не крупнее нужного, тайлы окна просятся слитыми диапазонами. Окно
//! пересчитывается на сетку участка - рамка и шаг в системе файла, как
//! `outputBounds` и `xRes` модуля: ближайший пиксель или билинейно,
//! пиксель «нет данных» в смесь не входит.

use std::collections::HashMap;
use std::ops::Range;

use data::cog::{self, Chunks, CogHeader, Parsed};
use data::sentinel::Raw;

/// Наибольший ответ куска, байт.
const ANSWER_BYTES: u64 = 64 << 20;
/// Промежуток между тайлами, который ещё сливается в один запрос, байт.
const MERGE_GAP: u64 = 65536;

/// Запрос куска: начало и запрос.
type Ask = (u64, Box<dyn net::Request>);

enum Stage {
    Header(Chunks, Vec<Ask>),
    Tiles { header: Box<CogHeader>, level: usize, window: cog::Window, plan: Vec<cog::TileRequest>, chunks: Chunks, asks: Vec<Ask> },
    Done(Raw),
    Failed(String),
}

/// Чтение окна канала.
pub struct BandRead {
    href: String,
    /// Рамка участка в системе файла: запад, юг, восток, север.
    bbox: [f64; 4],
    res: f64,
    nearest: bool,
    stage: Stage,
}

fn ask_range(href: &str, range: &Range<u64>, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Ask {
    let ask = request(ANSWER_BYTES);
    let header = format!("bytes={}-{}", range.start, range.end.saturating_sub(1));
    ask.send(href, &[("Range", header.as_str())], None);
    (range.start, ask)
}

/// Пришедшие куски - в набор. Возвращает, все ли пришли, и ошибку.
fn collect(asks: &mut Vec<Ask>, chunks: &mut Chunks) -> Result<bool, String> {
    let mut error = None;
    asks.retain(|(start, ask)| match ask.take() {
        None => true,
        Some(Ok(bytes)) => {
            chunks.add(*start, &bytes);
            false
        }
        Some(Err(e)) => {
            error = Some(e);
            false
        }
    });
    match error {
        Some(e) => Err(e),
        None => Ok(asks.is_empty()),
    }
}

impl BandRead {
    /// Начать чтение канала href по рамке bbox системы файла с шагом res.
    pub fn new(href: &str, bbox: [f64; 4], res: f64, nearest: bool, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Self {
        let ask = ask_range(href, &(0..cog::PREFIX_LEN), request);
        Self { href: href.to_string(), bbox, res, nearest, stage: Stage::Header(Chunks::new(), vec![ask]) }
    }

    /// Окно готово.
    pub fn result(&self) -> Option<&Raw> {
        match &self.stage {
            Stage::Done(r) => Some(r),
            _ => None,
        }
    }

    /// Ошибка чтения.
    pub fn error(&self) -> Option<&str> {
        match &self.stage {
            Stage::Failed(e) => Some(e),
            _ => None,
        }
    }

    /// Шаг: ответы, следующие запросы, разбор окна.
    pub fn step(&mut self, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        let stage = std::mem::replace(&mut self.stage, Stage::Failed(String::new()));
        self.stage = match stage {
            Stage::Header(mut chunks, mut asks) => match collect(&mut asks, &mut chunks) {
                Err(e) => Stage::Failed(e),
                Ok(false) => Stage::Header(chunks, asks),
                Ok(true) => match CogHeader::parse_chunks(&chunks) {
                    Err(e) => Stage::Failed(e.to_string()),
                    Ok(Parsed::NeedMore(ranges)) => {
                        let asks = cog::merge_ranges(ranges, MERGE_GAP).iter().map(|r| ask_range(&self.href, r, request)).collect();
                        Stage::Header(chunks, asks)
                    }
                    Ok(Parsed::Header(header)) => self.plan(header, request),
                },
            },
            Stage::Tiles { header, level, window, plan, mut chunks, mut asks } => match collect(&mut asks, &mut chunks) {
                Err(e) => Stage::Failed(e),
                Ok(false) => Stage::Tiles { header, level, window, plan, chunks, asks },
                Ok(true) => {
                    let ranges: HashMap<usize, Range<u64>> = plan.iter().map(|t| (t.index, t.range.clone())).collect();
                    let tiles = |i: usize| ranges.get(&i).and_then(|r| chunks.get(r.clone()));
                    match cog::read_window(&header, level, window.x0, window.y0, window.width, window.height, &tiles) {
                        Err(e) => Stage::Failed(e.to_string()),
                        Ok(native) => match cog::window_transform(&header, level, &window).and_then(|gt| self.resample(&native, gt, header.nodata)) {
                            Some(grid) => Stage::Done(grid),
                            None => Stage::Failed("window".into()),
                        },
                    }
                }
            },
            other => other,
        };
    }

    fn plan(&self, header: CogHeader, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Stage {
        let level = cog::level_for_resolution(&header, self.res);
        let Some(window) = cog::bbox_window(&header, level, self.bbox) else {
            return Stage::Failed("outside".into());
        };
        let plan = match cog::window_plan(&header, level, window.x0, window.y0, window.width, window.height) {
            Ok(p) => p,
            Err(e) => return Stage::Failed(e.to_string()),
        };
        let ranges = cog::merge_ranges(plan.iter().map(|t| t.range.clone()).collect(), MERGE_GAP);
        let asks = ranges.iter().map(|r| ask_range(&self.href, r, request)).collect();
        Stage::Tiles { header: Box::new(header), level, window, plan, chunks: Chunks::new(), asks }
    }

    /// Окно на сетку участка: строки сверху вниз, центр пикселя -
    /// (запад + (c + 0.5) шага, север - (r + 0.5) шага).
    fn resample(&self, native: &Raw, gt: [f64; 6], nodata: Option<f64>) -> Option<Raw> {
        let [west, south, east, north] = self.bbox;
        let cols = ((east - west) / self.res).round().max(1.0) as usize;
        let rows = ((north - south) / self.res).round().max(1.0) as usize;
        if gt[1] == 0.0 || gt[5] == 0.0 || cols.checked_mul(rows)? > 1 << 26 {
            return None;
        }
        let empty = nodata.unwrap_or(0.0);
        let (nr, nc) = native.shape();
        let at = |r: i64, c: i64| -> Option<f64> {
            if r < 0 || c < 0 || r as usize >= nr || c as usize >= nc {
                return None;
            }
            native.get(r as usize, c as usize).filter(|v| *v != empty && v.is_finite())
        };
        let mut out = Vec::with_capacity(rows * cols);
        for r in 0..rows {
            let y = north - (r as f64 + 0.5) * self.res;
            let fy = (y - gt[3]) / gt[5] - 0.5;
            for c in 0..cols {
                let x = west + (c as f64 + 0.5) * self.res;
                let fx = (x - gt[0]) / gt[1] - 0.5;
                let value = if self.nearest {
                    at((fy + 0.5).floor() as i64, (fx + 0.5).floor() as i64)
                } else {
                    let (r0, c0) = (fy.floor() as i64, fx.floor() as i64);
                    let (ty, tx) = (fy - fy.floor(), fx - fx.floor());
                    let mut sum = 0.0;
                    let mut weight = 0.0;
                    for (dr, dc, w) in [(0, 0, (1.0 - ty) * (1.0 - tx)), (0, 1, (1.0 - ty) * tx), (1, 0, ty * (1.0 - tx)), (1, 1, ty * tx)] {
                        if let Some(v) = at(r0 + dr, c0 + dc) {
                            sum += v * w;
                            weight += w;
                        }
                    }
                    (weight > 1e-6).then(|| sum / weight)
                };
                out.push(value.unwrap_or(empty));
            }
        }
        Raw::from_vec(rows, cols, out)
    }
}
