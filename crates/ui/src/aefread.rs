// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Блок файла AlphaEarth по сети: 64 канала Int8 одного блока обзора
//! 1024 × 1024 пикселя, `band_tile` (`core/cog.py`) модуля. Разбор
//! заголовка и тайлов - `data::cog`.
//!
//! Заголовок файла просится через [`crate::cogread::HeaderRead`], потом
//! тайлы блока всех каналов запросами HTTP Range: каналы лежат
//! плоскостями, у каждого канала свой кусок файла. Каналы распаковываются
//! по одному ([`Block::band`]), 64 плоскости в памяти не держатся.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use data::aef::{self, Piece};
use data::cog::{self, Chunks, CogHeader};

use crate::cogread::HeaderRead;

/// Наибольший ответ куска, байт.
const ANSWER_BYTES: u64 = 64 << 20;
/// Промежуток между тайлами, который ещё сливается в один запрос, байт.
const MERGE_GAP: u64 = 65536;

/// Запрос куска: начало и запрос.
type Ask = (u64, Box<dyn net::Request>);

enum Stage {
    Header(HeaderRead),
    Tiles(Box<Block>, Vec<Ask>),
    Done(Box<Block>),
    Failed(String),
}

/// Пришедший блок: заголовок, окно уровня и куски тайлов каналов.
pub struct Block {
    header: Arc<CogHeader>,
    level: usize,
    window: cog::Window,
    /// Диапазоны тайлов каждого канала по номеру тайла.
    plans: Vec<HashMap<usize, Range<u64>>>,
    chunks: Chunks,
}

impl Block {
    /// Канал sample блока, строка за строкой как в файле (с юга). None -
    /// канал не разобран.
    pub fn band(&self, sample: u16) -> Option<Vec<i8>> {
        let plan = self.plans.get(usize::from(sample))?;
        let tiles = |i: usize| plan.get(&i).and_then(|r| self.chunks.get(r.clone()));
        let w = &self.window;
        let grid = cog::read_window_sample(&self.header, self.level, w.x0, w.y0, w.width, w.height, sample, &tiles).ok()?;
        Some(grid.data().iter().map(|v| if v.is_finite() { v.clamp(-128.0, 127.0) as i8 } else { aef::NODATA }).collect())
    }

    /// Привязка куска в метрах UTM: угол блока и шаги со знаком, строк
    /// и столбцов. Значения кладёт вызывающий.
    pub fn piece<T>(&self, values: Vec<T>) -> Option<Piece<T>> {
        let gt = cog::window_transform(&self.header, self.level, &self.window)?;
        Some(Piece { x0: gt[0], y0: gt[3], dx: gt[1], dy: gt[5], rows: self.window.height as usize, cols: self.window.width as usize, values })
    }

    /// Пикселей блока.
    pub fn pixels(&self) -> usize {
        self.window.width as usize * self.window.height as usize
    }

    /// Каналов файла.
    pub fn samples(&self) -> u16 {
        self.header.level(self.level).map_or(0, |l| l.samples)
    }
}

/// Чтение блока (столбец, строка) обзора level файла href.
pub struct BlockRead {
    href: String,
    level: usize,
    col: u32,
    row: u32,
    stage: Stage,
}

fn ask_range(href: &str, range: &Range<u64>, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Ask {
    let ask = request(ANSWER_BYTES);
    let header = format!("bytes={}-{}", range.start, range.end.saturating_sub(1));
    ask.send(href, &[("Range", header.as_str())], None);
    (range.start, ask)
}

impl BlockRead {
    pub fn new(href: &str, level: usize, col: u32, row: u32, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Self {
        Self { href: href.to_string(), level, col, row, stage: Stage::Header(HeaderRead::new(href, request)) }
    }

    /// Блок готов.
    pub fn block(&self) -> Option<&Block> {
        match &self.stage {
            Stage::Done(b) => Some(b),
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

    /// Шаг: ответы и следующие запросы.
    pub fn step(&mut self, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        let stage = std::mem::replace(&mut self.stage, Stage::Failed(String::new()));
        self.stage = match stage {
            Stage::Header(mut read) => {
                read.step(request);
                match read.result() {
                    None => Stage::Header(read),
                    Some(Err(e)) => Stage::Failed(e.clone()),
                    Some(Ok(header)) => self.plan(header.clone(), request),
                }
            }
            Stage::Tiles(mut block, mut asks) => {
                let mut error = None;
                asks.retain(|(start, ask)| match ask.take() {
                    None => true,
                    Some(Ok(bytes)) => {
                        block.chunks.add(*start, &bytes);
                        false
                    }
                    Some(Err(e)) => {
                        error = Some(e);
                        false
                    }
                });
                match error {
                    Some(e) => Stage::Failed(e),
                    None if asks.is_empty() => Stage::Done(block),
                    None => Stage::Tiles(block, asks),
                }
            }
            other => other,
        };
    }

    /// Тайлы блока всех каналов: окно блока на уровне и запросы.
    fn plan(&self, header: Arc<CogHeader>, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Stage {
        let Some(level) = header.level(self.level).cloned() else {
            return Stage::Failed("level".into());
        };
        let (x0, y0) = (self.col.saturating_mul(level.tile_width), self.row.saturating_mul(level.tile_height));
        if x0 >= level.width || y0 >= level.height {
            return Stage::Failed("outside".into());
        }
        let window = cog::Window { x0, y0, width: level.tile_width.min(level.width - x0), height: level.tile_height.min(level.height - y0) };
        let mut plans = Vec::with_capacity(usize::from(level.samples));
        let mut ranges = Vec::new();
        for sample in 0..level.samples {
            match cog::window_plan_sample(&header, self.level, window.x0, window.y0, window.width, window.height, sample) {
                Ok(plan) => {
                    ranges.extend(plan.iter().map(|t| t.range.clone()));
                    plans.push(plan.into_iter().map(|t| (t.index, t.range)).collect());
                }
                Err(e) => return Stage::Failed(e.to_string()),
            }
        }
        let asks = cog::merge_ranges(ranges, MERGE_GAP).iter().map(|r| ask_range(&self.href, r, request)).collect();
        Stage::Tiles(Box::new(Block { header, level: self.level, window, plans, chunks: Chunks::new() }), asks)
    }
}
