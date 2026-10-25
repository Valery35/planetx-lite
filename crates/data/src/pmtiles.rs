// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Архив тайлов PMTiles версии 3 частями файла, перенос `core/pmtiles.py`
//! модуля. Сеть здесь не ходит: [`Archive::locate`] говорит, какой кусок
//! файла нужен - листовой каталог или сам тайл, вызывающий просит его
//! и отдаёт листовой каталог в [`Archive::add_leaf`].
//!
//! Устройство по спецификации PMTiles v3 (github.com/protomaps/PMTiles,
//! spec/v3): заголовок 127 байт, корневой каталог, листовые каталоги,
//! тайлы. Каталог - количество записей и четыре столбца целых в varint:
//! номера тайлов разностями, длины серий, длины, смещения (0 - сразу
//! за предыдущим). Запись с серией 0 - ссылка на листовой каталог. Номер
//! тайла - сумма тайлов мельче уровня и позиция на кривой Гильберта.

use std::collections::{HashMap, VecDeque};
use std::io::Read;

/// Байт начала файла: заголовок и корневой каталог.
pub const HEAD: u64 = 16384;
/// Листовых каталогов в памяти.
const LEAVES_KEPT: usize = 64;
/// Глубина каталогов, в спецификации не больше 3.
const DEPTH: usize = 4;
/// Наибольший разжатый каталог или тайл, байт.
const UNPACKED_LIMIT: u64 = 64 << 20;

/// Файл не разбирается как поддержанный PMTiles.
#[derive(Debug, Clone, PartialEq)]
pub struct PmtilesError(pub String);

impl std::fmt::Display for PmtilesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PMTiles: {}", self.0)
    }
}

impl std::error::Error for PmtilesError {}

fn error(text: &str) -> PmtilesError {
    PmtilesError(text.to_string())
}

/// Номер тайла z/x/y в архиве.
pub fn tile_id(z: u32, x: u32, y: u32) -> u64 {
    let z = z.min(31);
    let acc = ((1u64 << (2 * z)) - 1) / 3;
    let (mut x, mut y) = (u64::from(x), u64::from(y));
    let mut d = 0u64;
    let mut s = (1u64 << z) >> 1;
    while s > 0 {
        let rx = u64::from(x & s != 0);
        let ry = u64::from(y & s != 0);
        d += s * s * ((3 * rx) ^ ry);
        if ry == 0 {
            if rx == 1 {
                x = s - 1 - (x & (s - 1));
                y = s - 1 - (y & (s - 1));
            }
            std::mem::swap(&mut x, &mut y);
        }
        s >>= 1;
    }
    acc + d
}

fn varints(data: &[u8], at: &mut usize, count: usize) -> Result<Vec<u64>, PmtilesError> {
    let mut out = Vec::with_capacity(count.min(1 << 20));
    for _ in 0..count {
        let mut value = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = *data.get(*at).ok_or_else(|| error("directory"))?;
            *at += 1;
            if shift >= 64 {
                return Err(error("varint"));
            }
            value |= u64::from(byte & 0x7F) << shift;
            if byte < 0x80 {
                break;
            }
            shift += 7;
        }
        out.push(value);
    }
    Ok(out)
}

/// Записи каталога: номера, серии, смещения и длины.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Directory {
    pub ids: Vec<u64>,
    pub runs: Vec<u64>,
    pub offsets: Vec<u64>,
    pub lengths: Vec<u64>,
}

/// Разбор разжатого каталога.
pub fn directory(data: &[u8]) -> Result<Directory, PmtilesError> {
    let mut at = 0;
    let count = varints(data, &mut at, 1)?.first().copied().unwrap_or(0);
    // Запись занимает не меньше четырёх байт.
    if count > data.len() as u64 {
        return Err(error("directory size"));
    }
    let count = count as usize;
    let deltas = varints(data, &mut at, count)?;
    let runs = varints(data, &mut at, count)?;
    let lengths = varints(data, &mut at, count)?;
    let raw = varints(data, &mut at, count)?;
    let mut ids = Vec::with_capacity(count);
    let mut offsets: Vec<u64> = Vec::with_capacity(count);
    let mut last = 0u64;
    for i in 0..count {
        last = last.checked_add(deltas[i]).ok_or_else(|| error("ids"))?;
        ids.push(last);
        let offset = match (i, raw[i]) {
            (i, 0) if i > 0 => offsets[i - 1].saturating_add(lengths[i - 1]),
            (_, r) => r.checked_sub(1).ok_or_else(|| error("offset"))?,
        };
        offsets.push(offset);
    }
    Ok(Directory { ids, runs, offsets, lengths })
}

/// (серия, смещение, длина) записи с номером target или None.
pub fn find(entries: &Directory, target: u64) -> Option<(u64, u64, u64)> {
    let n = entries.ids.partition_point(|id| *id <= target).checked_sub(1)?;
    let (id, run) = (entries.ids[n], entries.runs[n]);
    (run == 0 || target < id.saturating_add(run)).then(|| (run, entries.offsets[n], entries.lengths[n]))
}

/// Разжать каталог или тайл по коду сжатия заголовка.
pub fn unpack(data: &[u8], kind: u8) -> Result<Vec<u8>, PmtilesError> {
    match kind {
        0 | 1 => Ok(data.to_vec()),
        2 => {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(data).take(UNPACKED_LIMIT).read_to_end(&mut out).map_err(|e| error(&e.to_string()))?;
            Ok(out)
        }
        k => Err(PmtilesError(format!("compression {k}"))),
    }
}

/// Что нужно для тайла.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locate {
    /// Тайла в архиве нет.
    Missing,
    /// Тайл - кусок файла: начало и длина, сжат по [`Archive::compression`].
    Tile(u64, u64),
    /// Нужен листовой каталог: начало и длина куска файла.
    Leaf(u64, u64),
}

/// Архив PMTiles: заголовок, корень и прочитанные листовые каталоги.
#[derive(Debug, Clone)]
pub struct Archive {
    pub leaf_at: u64,
    pub data_at: u64,
    pub internal: u8,
    pub compression: u8,
    pub kind: u8,
    pub min_zoom: u32,
    pub max_zoom: u32,
    root: Directory,
    leaves: HashMap<(u64, u64), Directory>,
    order: VecDeque<(u64, u64)>,
}

fn u64_at(head: &[u8], at: usize) -> u64 {
    head.get(at..at + 8).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes).unwrap_or(0)
}

impl Archive {
    /// Архив по началу файла. Корень, которого нет в начале, - Err.
    pub fn open(head: &[u8]) -> Result<Self, PmtilesError> {
        if head.len() < 127 || &head[..7] != b"PMTiles" || head[7] != 3 {
            return Err(error("not PMTiles v3"));
        }
        let (root_at, root_len) = (u64_at(head, 8), u64_at(head, 16));
        let end = root_at.checked_add(root_len).ok_or_else(|| error("root"))?;
        if end > head.len() as u64 {
            return Err(error("root outside the head"));
        }
        let internal = head[97];
        let root = directory(&unpack(&head[root_at as usize..end as usize], internal)?)?;
        Ok(Self {
            leaf_at: u64_at(head, 40),
            data_at: u64_at(head, 56),
            internal,
            compression: head[98],
            kind: head[99],
            min_zoom: u32::from(head[100]),
            max_zoom: u32::from(head[101]),
            root,
            leaves: HashMap::new(),
            order: VecDeque::new(),
        })
    }

    /// Что нужно для тайла z/x/y.
    pub fn locate(&self, z: u32, x: u32, y: u32) -> Locate {
        if z < self.min_zoom || z > self.max_zoom {
            return Locate::Missing;
        }
        let target = tile_id(z, x, y);
        let mut entries = &self.root;
        for _ in 0..DEPTH {
            let Some((run, offset, length)) = find(entries, target) else {
                return Locate::Missing;
            };
            if run > 0 {
                return Locate::Tile(self.data_at.saturating_add(offset), length);
            }
            match self.leaves.get(&(offset, length)) {
                Some(leaf) => entries = leaf,
                None => return Locate::Leaf(self.leaf_at.saturating_add(offset), length),
            }
        }
        Locate::Missing
    }

    /// Пришёл листовой каталог: начало куска файла из [`Locate::Leaf`]
    /// и его байты.
    pub fn add_leaf(&mut self, start: u64, length: u64, bytes: &[u8]) -> Result<(), PmtilesError> {
        let key = (start.saturating_sub(self.leaf_at), length);
        let leaf = directory(&unpack(bytes, self.internal)?)?;
        self.leaves.insert(key, leaf);
        self.order.retain(|k| *k != key);
        self.order.push_back(key);
        while self.order.len() > LEAVES_KEPT {
            if let Some(old) = self.order.pop_front() {
                self.leaves.remove(&old);
            }
        }
        Ok(())
    }

    /// Разжатый тайл.
    pub fn tile_bytes(&self, bytes: &[u8]) -> Result<Vec<u8>, PmtilesError> {
        unpack(bytes, self.compression)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::type_complexity)]
    use super::*;
    use std::io::Write;

    fn varint(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (value & 0x7F) as u8;
            value >>= 7;
            if value > 0 {
                out.push(byte | 0x80);
            } else {
                out.push(byte);
                return out;
            }
        }
    }

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let _ = e.write_all(data);
        e.finish().unwrap_or_default()
    }

    /// Каталог: записи (номер, серия, смещение, длина) по номерам.
    fn encode(entries: &[(u64, u64, u64, u64)]) -> Vec<u8> {
        let mut out = varint(entries.len() as u64);
        let mut last = 0;
        for e in entries {
            out.extend(varint(e.0 - last));
            last = e.0;
        }
        for e in entries {
            out.extend(varint(e.1));
        }
        for e in entries {
            out.extend(varint(e.3));
        }
        let mut previous: Option<(u64, u64)> = None;
        for e in entries {
            match previous {
                Some((o, l)) if e.2 == o + l => out.extend(varint(0)),
                _ => out.extend(varint(e.2 + 1)),
            }
            previous = Some((e.2, e.3));
        }
        gzip(&out)
    }

    /// Архив из тайлов. leaf_after - номера тайлов больше него уходят
    /// в листовой каталог.
    fn build(tiles: &[((u32, u32, u32), &[u8])], leaf_after: Option<u64>) -> Vec<u8> {
        let mut sorted: Vec<_> = tiles.to_vec();
        sorted.sort_by_key(|(k, _)| tile_id(k.0, k.1, k.2));
        let mut data = Vec::new();
        let mut rows = Vec::new();
        for (k, blob) in &sorted {
            let blob = gzip(blob);
            rows.push((tile_id(k.0, k.1, k.2), 1, data.len() as u64, blob.len() as u64));
            data.extend(blob);
        }
        let mut leaves = Vec::new();
        if let Some(after) = leaf_after {
            let near: Vec<_> = rows.iter().copied().filter(|r| r.0 <= after).collect();
            let far: Vec<_> = rows.iter().copied().filter(|r| r.0 > after).collect();
            leaves = encode(&far);
            rows = near;
            rows.push((far[0].0, 0, 0, leaves.len() as u64));
        }
        let root = encode(&rows);
        let root_at = 127u64;
        let leaf_at = root_at + root.len() as u64;
        let data_at = leaf_at + leaves.len() as u64;
        let mut head = b"PMTiles".to_vec();
        head.push(3);
        let n = tiles.len() as u64;
        for v in [root_at, root.len() as u64, 0, 0, leaf_at, leaves.len() as u64, data_at, data.len() as u64, n, n, n] {
            head.extend(v.to_le_bytes());
        }
        head.extend([1, 2, 2, 1, 0, 14]);
        head.resize(127, 0);
        [head, root, leaves, data].concat()
    }

    /// Тайл по архиву, как читал бы вызывающий: листы, потом тайл.
    fn read(archive: &mut Archive, file: &[u8], key: (u32, u32, u32), calls: &mut usize) -> Option<Vec<u8>> {
        loop {
            match archive.locate(key.0, key.1, key.2) {
                Locate::Missing => return None,
                Locate::Tile(at, len) => {
                    *calls += 1;
                    return archive.tile_bytes(&file[at as usize..(at + len) as usize]).ok();
                }
                Locate::Leaf(at, len) => {
                    *calls += 1;
                    archive.add_leaf(at, len, &file[at as usize..(at + len) as usize]).ok()?;
                }
            }
        }
    }

    #[test]
    fn tile_ids_of_spec() {
        assert_eq!(tile_id(0, 0, 0), 0);
        assert_eq!([(0, 0), (0, 1), (1, 1), (1, 0)].map(|(x, y)| tile_id(1, x, y)), [1, 2, 3, 4]);
        assert_eq!(tile_id(2, 0, 0), 5);
        // Номера уровня 14 идут подряд после всех мельче.
        assert_eq!(tile_id(14, 0, 0), (4u64.pow(14) - 1) / 3);
    }

    #[test]
    fn tiles_from_root_and_leaf() {
        let tiles: [((u32, u32, u32), &[u8]); 4] = [((0, 0, 0), b"world"), ((14, 10500, 5100), b"perm"), ((14, 10501, 5100), b"east"), ((5, 20, 9), b"five")];
        for leaf in [None, Some(tile_id(5, 20, 9))] {
            let file = build(&tiles, leaf);
            let mut archive = Archive::open(&file).expect("архив");
            assert_eq!(archive.max_zoom, 14);
            let mut calls = 0;
            for (key, blob) in tiles {
                assert_eq!(read(&mut archive, &file, key, &mut calls).as_deref(), Some(blob));
            }
            assert_eq!(read(&mut archive, &file, (14, 10502, 5100), &mut calls), None);
            assert_eq!(read(&mut archive, &file, (15, 0, 0), &mut calls), None);
        }
    }

    #[test]
    fn leaf_read_once() {
        let blobs: Vec<Vec<u8>> = (0..5).map(|i| format!("t{i}").into_bytes()).collect();
        let tiles: Vec<((u32, u32, u32), &[u8])> = (0..5).map(|i| ((14, 10500 + i as u32, 5100), blobs[i].as_slice())).collect();
        // Все тайлы - в листовом каталоге.
        let first = tiles.iter().map(|(k, _)| tile_id(k.0, k.1, k.2)).min().unwrap_or(1);
        let file = build(&tiles, Some(first - 1));
        let mut archive = Archive::open(&file).expect("архив");
        let mut calls = 0;
        read(&mut archive, &file, (14, 10503, 5100), &mut calls);
        let before = calls;
        read(&mut archive, &file, (14, 10504, 5100), &mut calls);
        assert_eq!(calls - before, 1); // только тайл
    }

    #[test]
    fn not_pmtiles() {
        let mut gif = b"GIF89a".to_vec();
        gif.resize(206, 0);
        assert!(Archive::open(&gif).is_err());
    }
}
