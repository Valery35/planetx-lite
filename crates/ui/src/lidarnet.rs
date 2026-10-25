// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Облака точек лидара по сети у точки, перенос `ui/lidarnet.py` модуля:
//! каталоги IGN LiDAR HD и USGS 3DEP (`data::lidarnet`), файлы COPC
//! частями запросами Range (`data::cloud::SparseCloud`). Облако ложится
//! на глобус тем же путём, что облако с диска.
//!
//! Каталоги спрашиваются оба сразу, точку покрывает один из них. Файлам
//! 3DEP нужен ключ SAS службы Planetary Computer, он просится один раз
//! на поиск. Сервер IGN 25 октября 2026 года ограничивал частоту
//! запросов одним в секунду: к нему запросы идут по одному не чаще
//! [`IGN_GAP`], отказ повторяется через [`RETRY`] секунд, как у модуля.

use std::collections::VecDeque;

use data::cloud::{Cloud, SparseCloud};
use data::lidarnet::{self, Tile};

/// Секунд между запросами к серверу IGN.
const IGN_GAP: f64 = 1.1;
/// Секунд до повтора куска после отказа, как у модуля.
const RETRY: f64 = 10.0;
/// Отказов одного файла подряд, после которых он бросается.
const FAILURES: u32 = 6;
/// Кусков одного файла 3DEP в работе сразу.
const PARALLEL: usize = 4;
/// Предел ответа куска файла, байт.
const PART_BYTES: u64 = 256 << 20;
/// Предел ответа каталога, байт.
const DOC_BYTES: u64 = 8 << 20;

/// Чтение одного файла.
struct Read {
    tile: Tile,
    url: String,
    sparse: SparseCloud,
    queue: VecDeque<(u64, u64)>,
    asks: Vec<(u64, u64, Box<dyn net::Request>)>,
    failures: u32,
    wait_until: f64,
}

/// Что пришло: облако плитки или текст для строки сообщений.
pub enum Arrived {
    Cloud(Tile, Cloud),
    Message(String),
    /// У точки нет плиток ни в одном каталоге: ошибки каталогов или "".
    Nothing(String),
}

/// Поиск и чтение облаков у точки.
pub struct Lidar {
    point: (f64, f64),
    ign: Option<Box<dyn net::Request>>,
    pc: Option<Box<dyn net::Request>>,
    sas: Option<Box<dyn net::Request>>,
    tiles: Vec<Tile>,
    errors: Vec<String>,
    chosen: Vec<Tile>,
    reads: Vec<Read>,
    ign_last: f64,
    /// Ответы обоих каталогов пришли.
    searched: bool,
}

impl Lidar {
    /// Поиск плиток у точки в обоих каталогах.
    pub fn search(lat: f64, lon: f64, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Self {
        let bbox = lidarnet::box_around(lat, lon, lidarnet::SIDE);
        let ign = request(DOC_BYTES);
        ign.start(&lidarnet::ign_url(bbox));
        let pc = request(DOC_BYTES);
        let body = lidarnet::pc_body(bbox);
        pc.send(lidarnet::PC_SEARCH, &[("Content-Type", "application/json")], Some(body.as_bytes()));
        Self { point: (lat, lon), ign: Some(ign), pc: Some(pc), sas: None, tiles: Vec::new(), errors: Vec::new(), chosen: Vec::new(), reads: Vec::new(), ign_last: f64::MIN, searched: false }
    }

    pub fn busy(&self) -> bool {
        self.ign.is_some() || self.pc.is_some() || self.sas.is_some() || !self.reads.is_empty()
    }

    /// Шаг: ответы каталогов, ключ, куски файлов. now - секунды.
    pub fn step(&mut self, now: f64, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> Vec<Arrived> {
        let mut out = Vec::new();
        for (slot, parse) in [(&mut self.ign, lidarnet::parse_ign as fn(&[u8]) -> Result<Vec<Tile>, String>), (&mut self.pc, lidarnet::parse_pc)] {
            if let Some(ask) = slot
                && let Some(answer) = ask.take()
            {
                *slot = None;
                match answer.and_then(|b| parse(&b)) {
                    Ok(found) => self.tiles.extend(found),
                    Err(e) => self.errors.push(e),
                }
            }
        }
        if self.ign.is_none() && self.pc.is_none() && self.chosen.is_empty() && self.sas.is_none() && self.reads.is_empty() && !self.tiles.is_empty() {
            self.chosen = lidarnet::choose(&self.tiles, self.point.0, self.point.1, lidarnet::MAX_TILES);
            self.tiles.clear();
            if self.chosen.iter().any(|t| lidarnet::needs_key(&t.url)) {
                let ask = request(DOC_BYTES);
                ask.start(lidarnet::PC_SAS);
                self.sas = Some(ask);
            } else {
                self.start_reads("");
            }
        }
        if self.ign.is_none() && self.pc.is_none() && !self.searched {
            self.searched = true;
            if self.tiles.is_empty() && self.reads.is_empty() && self.sas.is_none() {
                out.push(Arrived::Nothing(self.errors.iter().take(2).cloned().collect::<Vec<_>>().join("; ")));
            }
        }
        if let Some(ask) = &self.sas
            && let Some(answer) = ask.take()
        {
            self.sas = None;
            let token = answer.ok().and_then(|b| lidarnet::parse_token(&b)).unwrap_or_default();
            self.start_reads(&token);
        }
        self.step_reads(now, request, &mut out);
        out
    }

    fn start_reads(&mut self, token: &str) {
        for tile in std::mem::take(&mut self.chosen) {
            let url = lidarnet::with_key(&tile.url, token);
            let name = format!("{} {}", tile.name, tile.date).trim().to_string();
            self.reads.push(Read { sparse: SparseCloud::new(&name, None, crate::app::CLOUD_POINTS as usize), url, tile, queue: VecDeque::new(), asks: Vec::new(), failures: 0, wait_until: 0.0 });
        }
    }

    fn step_reads(&mut self, now: f64, request: &dyn Fn(u64) -> Box<dyn net::Request>, out: &mut Vec<Arrived>) {
        let mut finished = Vec::new();
        for (n, read) in self.reads.iter_mut().enumerate() {
            // Пришедшие куски.
            let mut failed = false;
            read.asks.retain(|(start, length, ask)| match ask.take() {
                None => true,
                Some(Ok(bytes)) => {
                    read.sparse.insert(*start, &bytes);
                    false
                }
                Some(Err(_)) => {
                    read.queue.push_back((*start, *length));
                    failed = true;
                    false
                }
            });
            if failed {
                read.failures += 1;
                read.wait_until = now + RETRY;
                if read.failures > FAILURES {
                    out.push(Arrived::Message(format!("{}: {}", read.tile.name, "HTTP")));
                    finished.push(n);
                    continue;
                }
            }
            if !read.asks.is_empty() || !read.queue.is_empty() {
                // Ждут куски этого круга.
            } else {
                match read.sparse.read() {
                    Ok(Some(cloud)) => {
                        out.push(Arrived::Cloud(read.tile.clone(), cloud));
                        finished.push(n);
                        continue;
                    }
                    Ok(None) => read.queue.extend(read.sparse.missing()),
                    Err(e) => {
                        out.push(Arrived::Message(format!("{}: {e}", read.tile.name)));
                        finished.push(n);
                        continue;
                    }
                }
            }
            // Новые запросы: к IGN по одному и не чаще IGN_GAP.
            let ign = read.tile.source == lidarnet::IGN;
            while now >= read.wait_until && !read.queue.is_empty() {
                if ign && (!read.asks.is_empty() || now - self.ign_last < IGN_GAP) {
                    break;
                }
                if !ign && read.asks.len() >= PARALLEL {
                    break;
                }
                let Some((start, length)) = read.queue.pop_front() else {
                    break;
                };
                let ask = request(PART_BYTES.min(length.saturating_add(1 << 16)));
                let range = format!("bytes={}-{}", start, (start + length).saturating_sub(1));
                ask.send(&read.url, &[("Range", range.as_str())], None);
                read.asks.push((start, length, ask));
                if ign {
                    self.ign_last = now;
                }
            }
        }
        for n in finished.into_iter().rev() {
            self.reads.remove(n);
        }
    }
}
