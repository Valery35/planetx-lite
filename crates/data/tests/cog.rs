// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Проверки `cog.rs` на маленьких TIFF и BigTIFF, собранных в памяти.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::io::Write;
use std::ops::Range;

use data::cog::{
    Chunks, CogError, CogHeader, Compression, Parsed, Predictor, SampleFormat, Window, bbox_window,
    level_for_factor, level_for_resolution, merge_ranges, read_window, read_window_sample,
    window_plan, window_plan_sample, window_transform,
};
use data::sentinel::Grid;

// ---------------------------------------------------------------------
// Сборка TIFF

/// Значение тега в байтах файла.
#[derive(Clone)]
struct Tag {
    tag: u16,
    typ: u16,
    count: u64,
    bytes: Vec<u8>,
}

fn put(big: bool, v: u64, size: usize, out: &mut Vec<u8>) {
    let b = v.to_be_bytes();
    let s = &b[8 - size..];
    if big {
        out.extend_from_slice(s);
    } else {
        out.extend(s.iter().rev());
    }
}

fn tag_ints(big: bool, tag: u16, typ: u16, vals: &[u64]) -> Tag {
    let size = match typ {
        3 => 2,
        4 => 4,
        16 => 8,
        _ => unreachable!(),
    };
    let mut bytes = Vec::new();
    for v in vals {
        put(big, *v, size, &mut bytes);
    }
    Tag {
        tag,
        typ,
        count: vals.len() as u64,
        bytes,
    }
}

fn tag_f64(big: bool, tag: u16, vals: &[f64]) -> Tag {
    let mut bytes = Vec::new();
    for v in vals {
        put(big, v.to_bits(), 8, &mut bytes);
    }
    Tag {
        tag,
        typ: 12,
        count: vals.len() as u64,
        bytes,
    }
}

fn tag_ascii(tag: u16, s: &str) -> Tag {
    let mut bytes = s.as_bytes().to_vec();
    bytes.push(0);
    Tag {
        tag,
        typ: 2,
        count: bytes.len() as u64,
        bytes,
    }
}

/// Снимок одного уровня.
#[derive(Clone)]
struct Img {
    width: u32,
    height: u32,
    tw: u32,
    th: u32,
    samples: u16,
    bits: u16,
    format: u16,
    compression: u16,
    predictor: u16,
    planar: u16,
    subfile: u32,
    /// Отсчёты (y * width + x) * samples + s, биты значения.
    data: Vec<u64>,
    /// Пустые тайлы: длина 0.
    sparse: Vec<usize>,
    /// Полосы вместо тайлов.
    stripped: bool,
}

#[allow(clippy::too_many_arguments)]
fn img(
    width: u32,
    height: u32,
    tile: u32,
    samples: u16,
    bits: u16,
    format: u16,
    compression: u16,
    predictor: u16,
    f: impl Fn(u32, u32, u16) -> u64,
) -> Img {
    let mut data = Vec::new();
    for y in 0..height {
        for x in 0..width {
            for s in 0..samples {
                data.push(f(x, y, s));
            }
        }
    }
    Img {
        width,
        height,
        tw: tile,
        th: tile,
        samples,
        bits,
        format,
        compression,
        predictor,
        planar: 1,
        subfile: 0,
        data,
        sparse: Vec::new(),
        stripped: false,
    }
}

fn tiles_across(i: &Img) -> u32 {
    i.width.div_ceil(i.tw)
}

fn tiles_down(i: &Img) -> u32 {
    i.height.div_ceil(i.th)
}

/// Сжатые тайлы снимка по порядку TIFF.
fn encode_tiles(i: &Img, big: bool) -> Vec<Vec<u8>> {
    let planes = if i.planar == 2 { i.samples } else { 1 };
    let spt = if i.planar == 2 {
        1
    } else {
        usize::from(i.samples)
    };
    let bps = usize::from(i.bits / 8);
    let mask = if i.bits == 64 {
        u64::MAX
    } else {
        (1u64 << i.bits) - 1
    };
    let (tw, th) = (i.tw as usize, i.th as usize);
    let row_samples = tw * spt;
    let mut out = Vec::new();
    let mut index = 0;
    for p in 0..planes {
        for ty in 0..tiles_down(i) {
            for tx in 0..tiles_across(i) {
                if i.sparse.contains(&index) {
                    out.push(Vec::new());
                    index += 1;
                    continue;
                }
                index += 1;
                let mut vals = Vec::with_capacity(tw * th * spt);
                for r in 0..th as u32 {
                    for c in 0..tw as u32 {
                        for s in 0..spt as u16 {
                            let (x, y) = (tx * i.tw + c, ty * i.th + r);
                            let s = if i.planar == 2 { p } else { s };
                            let v = if x < i.width && y < i.height {
                                i.data[((y * i.width + x) as usize) * usize::from(i.samples)
                                    + usize::from(s)]
                            } else {
                                0
                            };
                            vals.push(v);
                        }
                    }
                }
                let mut bytes = Vec::new();
                for row in vals.chunks_mut(row_samples) {
                    match i.predictor {
                        2 => {
                            for k in (spt..row.len()).rev() {
                                row[k] = row[k].wrapping_sub(row[k - spt]) & mask;
                            }
                            for v in row.iter() {
                                put(big, *v, bps, &mut bytes);
                            }
                        }
                        3 => {
                            let mut b = vec![0u8; row_samples * bps];
                            for (k, v) in row.iter().enumerate() {
                                let be = v.to_be_bytes();
                                for j in 0..bps {
                                    b[j * row_samples + k] = be[8 - bps + j];
                                }
                            }
                            for k in (spt..b.len()).rev() {
                                b[k] = b[k].wrapping_sub(b[k - spt]);
                            }
                            bytes.extend(b);
                        }
                        _ => {
                            for v in row.iter() {
                                put(big, *v, bps, &mut bytes);
                            }
                        }
                    }
                }
                out.push(compress(i.compression, &bytes));
            }
        }
    }
    out
}

fn compress(compression: u16, bytes: &[u8]) -> Vec<u8> {
    match compression {
        1 => bytes.to_vec(),
        8 | 32946 => {
            let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            e.write_all(bytes).unwrap();
            e.finish().unwrap()
        }
        5 => weezl::encode::Encoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8)
            .encode(bytes)
            .unwrap(),
        50000 => {
            ruzstd::encoding::compress_to_vec(bytes, ruzstd::encoding::CompressionLevel::Fastest)
        }
        // Неподдержанное сжатие: байты как есть.
        _ => bytes.to_vec(),
    }
}

/// Теги уровня.
fn level_tags(i: &Img, big: bool, bigtiff: bool, offsets: &[u64], counts: &[u64]) -> Vec<Tag> {
    let spp = usize::from(i.samples);
    let mut t = Vec::new();
    if i.subfile != 0 {
        t.push(tag_ints(big, 254, 4, &[u64::from(i.subfile)]));
    }
    t.push(tag_ints(big, 256, 4, &[u64::from(i.width)]));
    t.push(tag_ints(big, 257, 4, &[u64::from(i.height)]));
    t.push(tag_ints(big, 258, 3, &vec![u64::from(i.bits); spp]));
    t.push(tag_ints(big, 259, 3, &[u64::from(i.compression)]));
    t.push(tag_ints(big, 262, 3, &[1]));
    t.push(tag_ints(big, 277, 3, &[u64::from(i.samples)]));
    t.push(tag_ints(big, 284, 3, &[u64::from(i.planar)]));
    if i.predictor != 1 {
        t.push(tag_ints(big, 317, 3, &[u64::from(i.predictor)]));
    }
    let typ = if bigtiff { 16 } else { 4 };
    if i.stripped {
        t.push(tag_ints(big, 273, typ, offsets));
        t.push(tag_ints(big, 278, 4, &[u64::from(i.th)]));
        t.push(tag_ints(big, 279, typ, counts));
    } else {
        t.push(tag_ints(big, 322, 3, &[u64::from(i.tw)]));
        t.push(tag_ints(big, 323, 3, &[u64::from(i.th)]));
        t.push(tag_ints(big, 324, typ, offsets));
        t.push(tag_ints(big, 325, typ, counts));
    }
    t.push(tag_ints(big, 339, 3, &vec![u64::from(i.format); spp]));
    t
}

/// IFD со смещения pos: записи, следующий IFD, внешние значения.
fn write_ifd(mut tags: Vec<Tag>, pos: u64, next: u64, big: bool, bigtiff: bool) -> Vec<u8> {
    tags.sort_by_key(|t| t.tag);
    let (cs, es, os) = if bigtiff { (8, 20, 8) } else { (2, 12, 4) };
    let n = tags.len();
    let mut out = Vec::new();
    put(big, n as u64, cs, &mut out);
    let mut ext_pos = pos + (cs + n * es + os) as u64;
    let mut ext = Vec::new();
    for t in &tags {
        put(big, u64::from(t.tag), 2, &mut out);
        put(big, u64::from(t.typ), 2, &mut out);
        put(big, t.count, os, &mut out);
        if t.bytes.len() <= os {
            let mut field = t.bytes.clone();
            field.resize(os, 0);
            out.extend(field);
        } else {
            put(big, ext_pos, os, &mut out);
            ext.extend_from_slice(&t.bytes);
            ext_pos += t.bytes.len() as u64;
            if t.bytes.len() % 2 == 1 {
                ext.push(0);
                ext_pos += 1;
            }
        }
    }
    put(big, next, os, &mut out);
    out.extend(ext);
    out
}

/// Параметры файла.
#[derive(Clone)]
struct Opts {
    big: bool,
    bigtiff: bool,
    /// IFD в конце файла, после тайлов.
    far: bool,
    /// Теги привязки и метаданных первого IFD.
    geo: Vec<Tag>,
}

fn opts(big: bool, bigtiff: bool) -> Opts {
    Opts {
        big,
        bigtiff,
        far: false,
        geo: Vec::new(),
    }
}

fn build(imgs: &[Img], o: &Opts) -> Vec<u8> {
    let (big, bigtiff) = (o.big, o.bigtiff);
    let tiles: Vec<Vec<Vec<u8>>> = imgs.iter().map(|i| encode_tiles(i, big)).collect();
    let hdr_len: u64 = if bigtiff { 16 } else { 8 };
    let tags_of = |k: usize, offsets: &[u64], counts: &[u64]| {
        let mut t = level_tags(&imgs[k], big, bigtiff, offsets, counts);
        if k == 0 {
            t.extend(o.geo.iter().cloned());
        }
        t
    };
    // Длины IFD не зависят от значений смещений.
    let sizes: Vec<u64> = (0..imgs.len())
        .map(|k| {
            let zeros = vec![0u64; tiles[k].len()];
            write_ifd(tags_of(k, &zeros, &zeros), 0, 0, big, bigtiff).len() as u64
        })
        .collect();
    let tiles_len: u64 = tiles.iter().flatten().map(|t| t.len() as u64).sum();
    let (ifd_start, tiles_start) = if o.far {
        let start = hdr_len + tiles_len;
        (start + start % 2, hdr_len)
    } else {
        (hdr_len, hdr_len + sizes.iter().sum::<u64>())
    };
    let mut at = tiles_start;
    let mut placed = Vec::new();
    for t in &tiles {
        let mut offsets = Vec::new();
        let mut counts = Vec::new();
        for b in t {
            if b.is_empty() {
                offsets.push(0);
                counts.push(0);
            } else {
                offsets.push(at);
                counts.push(b.len() as u64);
                at += b.len() as u64;
            }
        }
        placed.push((offsets, counts));
    }
    let mut ifd_pos = Vec::new();
    let mut p = ifd_start;
    for s in &sizes {
        ifd_pos.push(p);
        p += s;
    }
    let mut file = Vec::new();
    if big {
        file.extend_from_slice(b"MM");
    } else {
        file.extend_from_slice(b"II");
    }
    if bigtiff {
        put(big, 43, 2, &mut file);
        put(big, 8, 2, &mut file);
        put(big, 0, 2, &mut file);
        put(big, ifd_start, 8, &mut file);
    } else {
        put(big, 42, 2, &mut file);
        put(big, ifd_start, 4, &mut file);
    }
    let mut ifds = Vec::new();
    for k in 0..imgs.len() {
        let next = ifd_pos.get(k + 1).copied().unwrap_or(0);
        let (offsets, counts) = &placed[k];
        ifds.extend(write_ifd(
            tags_of(k, offsets, counts),
            ifd_pos[k],
            next,
            big,
            bigtiff,
        ));
    }
    let tile_bytes: Vec<u8> = tiles.iter().flatten().flatten().copied().collect();
    if o.far {
        file.extend(tile_bytes);
        while (file.len() as u64) < ifd_start {
            file.push(0);
        }
        file.extend(ifds);
    } else {
        file.extend(ifds);
        file.extend(tile_bytes);
    }
    file
}

// ---------------------------------------------------------------------
// Чтение

fn header(file: &[u8]) -> CogHeader {
    match CogHeader::parse(file).unwrap() {
        Parsed::Header(h) => h,
        Parsed::NeedMore(r) => panic!("нужны ещё куски {r:?}"),
    }
}

fn slice<'a>(file: &'a [u8], r: &Range<u64>) -> Option<&'a [u8]> {
    file.get(usize::try_from(r.start).ok()?..usize::try_from(r.end).ok()?)
}

#[allow(clippy::too_many_arguments)]
fn read(
    h: &CogHeader,
    file: &[u8],
    level: usize,
    x0: u32,
    y0: u32,
    w: u32,
    hh: u32,
    sample: u16,
) -> Result<Grid<f64>, CogError> {
    let plan = window_plan_sample(h, level, x0, y0, w, hh, sample)?;
    let map: HashMap<usize, Range<u64>> = plan.into_iter().map(|t| (t.index, t.range)).collect();
    read_window_sample(h, level, x0, y0, w, hh, sample, &|i| {
        map.get(&i).and_then(|r| slice(file, r))
    })
}

fn check(grid: &Grid<f64>, x0: u32, y0: u32, f: impl Fn(u32, u32) -> f64) {
    for r in 0..grid.rows() {
        for c in 0..grid.cols() {
            let (x, y) = (x0 + c as u32, y0 + r as u32);
            assert_eq!(grid.get(r, c), Some(f(x, y)), "пиксель x {x}, y {y}");
        }
    }
}

fn u16_value(x: u32, y: u32) -> u64 {
    u64::from((y * 1000 + x * 7 + 13) % 65536)
}

fn i16_value(x: u32, y: u32) -> i64 {
    i64::from(x) * 37 - i64::from(y) * 101 - 500
}

fn f32_value(x: u32, y: u32) -> f32 {
    x as f32 * 0.5 - y as f32 * 1.25 + 0.1
}

// ---------------------------------------------------------------------
// Значения окон

#[test]
fn uint16_deflate_predictor2_classic_le() {
    let i = img(70, 50, 16, 1, 16, 1, 8, 2, |x, y, _| u16_value(x, y));
    let file = build(&[i], &opts(false, false));
    let h = header(&file);
    assert!(!h.bigtiff && !h.big_endian);
    let lvl = &h.levels[0];
    assert_eq!(
        (lvl.width, lvl.height, lvl.tile_width, lvl.tile_height),
        (70, 50, 16, 16)
    );
    assert_eq!(lvl.compression, Compression::Deflate);
    assert_eq!(lvl.predictor, Predictor::Horizontal);
    assert_eq!(lvl.format, SampleFormat::Uint);
    // Окно через границы тайлов по обеим осям.
    let g = read(&h, &file, 0, 10, 12, 40, 30, 0).unwrap();
    assert_eq!(g.shape(), (30, 40));
    check(&g, 10, 12, |x, y| u16_value(x, y) as f64);
    // Весь снимок с неполными крайними тайлами.
    let g = read(&h, &file, 0, 0, 0, 70, 50, 0).unwrap();
    check(&g, 0, 0, |x, y| u16_value(x, y) as f64);
    // Один пиксель в углу.
    let g = read(&h, &file, 0, 69, 49, 1, 1, 0).unwrap();
    check(&g, 69, 49, |x, y| u16_value(x, y) as f64);
    // План: тайлы 0..=2 по x, 0..=2 по y при 5 тайлах в строке.
    let plan = window_plan(&h, 0, 10, 12, 40, 30).unwrap();
    let idx: Vec<usize> = plan.iter().map(|t| t.index).collect();
    assert_eq!(idx, vec![0, 1, 2, 3, 5, 6, 7, 8, 10, 11, 12, 13]);
    for t in &plan {
        assert_eq!(t.range.start, lvl.tile_offsets[t.index]);
        assert_eq!(t.range.end - t.range.start, lvl.tile_byte_counts[t.index]);
    }
}

#[test]
fn int16_none_bigtiff_be() {
    let i = img(45, 37, 16, 1, 16, 2, 1, 1, |x, y, _| {
        (i16_value(x, y) as u64) & 0xFFFF
    });
    let file = build(&[i], &opts(true, true));
    let h = header(&file);
    assert!(h.bigtiff && h.big_endian);
    assert_eq!(h.levels[0].format, SampleFormat::Int);
    let g = read(&h, &file, 0, 5, 3, 30, 30, 0).unwrap();
    check(&g, 5, 3, |x, y| i16_value(x, y) as f64);
    assert!(g.data().iter().any(|v| *v < 0.0));
}

#[test]
fn int16_deflate_predictor2_classic_be() {
    let i = img(40, 20, 16, 1, 16, 2, 8, 2, |x, y, _| {
        (i16_value(x, y) as u64) & 0xFFFF
    });
    let file = build(&[i], &opts(true, false));
    let h = header(&file);
    let g = read(&h, &file, 0, 0, 0, 40, 20, 0).unwrap();
    check(&g, 0, 0, |x, y| i16_value(x, y) as f64);
}

#[test]
fn uint8_three_samples_chunky() {
    let f = |x: u32, y: u32, s: u16| u64::from((x * 7 + y * 13 + u32::from(s) * 50) % 256);
    for (big, bigtiff, compression, predictor) in [
        (true, false, 8, 2),
        (false, true, 1, 1),
        (false, false, 32946, 2),
    ] {
        let i = img(33, 21, 16, 3, 8, 1, compression, predictor, f);
        let file = build(&[i], &opts(big, bigtiff));
        let h = header(&file);
        assert_eq!(h.levels[0].samples, 3);
        for s in 0..3 {
            let g = read(&h, &file, 0, 3, 2, 28, 17, s).unwrap();
            check(&g, 3, 2, |x, y| f(x, y, s) as f64);
        }
        assert_eq!(
            read(&h, &file, 0, 0, 0, 1, 1, 3).unwrap_err(),
            CogError::Window
        );
    }
}

#[test]
fn uint16_planar_separate() {
    let f = |x: u32, y: u32, s: u16| u16_value(x, y) + u64::from(s) * 3000;
    let mut i = img(40, 30, 16, 2, 16, 1, 8, 2, f);
    i.planar = 2;
    let file = build(&[i], &opts(false, true));
    let h = header(&file);
    for s in 0..2 {
        let plan = window_plan_sample(&h, 0, 10, 10, 20, 10, s).unwrap();
        // 3 × 2 тайлов в канале, канал 1 начинается с тайла 6.
        assert!(plan.iter().all(|t| (t.index >= 6) == (s == 1)));
        let g = read(&h, &file, 0, 10, 10, 20, 10, s).unwrap();
        check(&g, 10, 10, |x, y| f(x, y, s) as f64);
    }
}

#[test]
fn float32_none_and_predictor3() {
    let bits = |x: u32, y: u32, _s: u16| u64::from(f32_value(x, y).to_bits());
    for (big, bigtiff, compression, predictor) in [
        (false, true, 1, 1),
        (true, false, 8, 3),
        (false, false, 8, 3),
        (false, false, 8, 1),
    ] {
        let i = img(37, 29, 16, 1, 32, 3, compression, predictor, bits);
        let file = build(&[i], &opts(big, bigtiff));
        let h = header(&file);
        assert_eq!(h.levels[0].format, SampleFormat::Float);
        let g = read(&h, &file, 0, 7, 9, 25, 20, 0).unwrap();
        check(&g, 7, 9, |x, y| f64::from(f32_value(x, y)));
    }
}

#[test]
fn float32_predictor3_two_samples() {
    let f = |x: u32, y: u32, s: u16| u64::from((f32_value(x, y) * (1.0 + f32::from(s))).to_bits());
    let i = img(20, 18, 16, 2, 32, 3, 8, 3, f);
    let file = build(&[i], &opts(true, true));
    let h = header(&file);
    for s in 0..2 {
        let g = read(&h, &file, 0, 0, 0, 20, 18, s).unwrap();
        check(&g, 0, 0, |x, y| {
            f64::from(f32_value(x, y) * (1.0 + f32::from(s)))
        });
    }
}

#[test]
fn lzw_and_zstd() {
    for (compression, predictor, big) in [
        (5, 2, false),
        (5, 1, true),
        (50000, 2, true),
        (50000, 1, false),
    ] {
        let i = img(50, 34, 16, 1, 16, 1, compression, predictor, |x, y, _| {
            u16_value(x, y)
        });
        let file = build(&[i], &opts(big, false));
        let h = header(&file);
        let g = read(&h, &file, 0, 9, 9, 41, 25, 0).unwrap();
        check(&g, 9, 9, |x, y| u16_value(x, y) as f64);
    }
}

#[test]
fn unsupported_compression_is_error() {
    let i = img(16, 16, 16, 1, 16, 1, 7, 1, |x, y, _| u16_value(x, y));
    let file = build(&[i], &opts(false, false));
    assert_eq!(
        CogHeader::parse(&file).unwrap_err(),
        CogError::Unsupported("сжатие")
    );
}

#[test]
fn stripped_is_error() {
    let mut i = img(16, 32, 16, 1, 16, 1, 1, 1, |x, y, _| u16_value(x, y));
    i.stripped = true;
    let file = build(&[i], &opts(false, false));
    assert_eq!(CogHeader::parse(&file).unwrap_err(), CogError::Stripped);
}

#[test]
fn sparse_tile_is_nodata() {
    let mut i = img(48, 16, 16, 1, 16, 1, 8, 2, |x, y, _| u16_value(x, y));
    i.sparse = vec![1];
    let mut o = opts(false, false);
    o.geo.push(tag_ascii(42113, "7"));
    let file = build(&[i], &o);
    let h = header(&file);
    assert_eq!(h.nodata, Some(7.0));
    let plan = window_plan(&h, 0, 0, 0, 48, 16).unwrap();
    assert_eq!(plan.iter().map(|t| t.index).collect::<Vec<_>>(), vec![0, 2]);
    let g = read(&h, &file, 0, 0, 0, 48, 16, 0).unwrap();
    check(&g, 0, 0, |x, y| {
        if (16..32).contains(&x) {
            7.0
        } else {
            u16_value(x, y) as f64
        }
    });
}

// ---------------------------------------------------------------------
// Привязка

fn utm_geo(big: bool) -> Vec<Tag> {
    vec![
        tag_f64(big, 33550, &[10.0, 10.0, 0.0]),
        tag_f64(big, 33922, &[0.0, 0.0, 0.0, 500000.0, 4600000.0, 0.0]),
        tag_ints(
            big,
            34735,
            3,
            &[1, 1, 0, 3, 1024, 0, 1, 1, 1025, 0, 1, 1, 3072, 0, 1, 32637],
        ),
        tag_ascii(42113, "0"),
        tag_ascii(
            42112,
            "<GDALMetadata>\n  <Item name=\"OFFSET\" sample=\"0\" role=\"offset\">-0.1</Item>\n  \
             <Item name=\"SCALE\" sample=\"0\" role=\"scale\">0.0001</Item>\n  \
             <Item name=\"SCALE\" sample=\"1\" role=\"scale\">5</Item>\n</GDALMetadata>",
        ),
    ]
}

#[test]
fn geotransform_epsg_nodata_metadata() {
    for (big, bigtiff) in [(false, false), (true, true)] {
        let i = img(70, 50, 16, 1, 16, 1, 8, 2, |x, y, _| u16_value(x, y));
        let mut o = opts(big, bigtiff);
        o.geo = utm_geo(big);
        let file = build(&[i], &o);
        let h = header(&file);
        assert_eq!(
            h.geotransform,
            Some([500000.0, 10.0, 0.0, 4600000.0, 0.0, -10.0])
        );
        assert_eq!(h.epsg, Some(32637));
        assert!(!h.pixel_is_point);
        assert_eq!(h.nodata, Some(0.0));
        assert_eq!(h.scale, Some(0.0001));
        assert_eq!(h.offset, Some(-0.1));

        // Рамка по краям пикселей - точное окно.
        let w = bbox_window(&h, 0, [500100.0, 4599700.0, 500250.0, 4599900.0]).unwrap();
        assert_eq!(
            w,
            Window {
                x0: 10,
                y0: 10,
                width: 15,
                height: 20
            }
        );
        // Рамка внутри пикселей захватывает их целиком.
        let w = bbox_window(&h, 0, [500105.0, 4599705.0, 500245.0, 4599895.0]).unwrap();
        assert_eq!((w.x0, w.y0, w.width, w.height), (10, 10, 15, 20));
        // Рамка за краем обрезается по снимку.
        let w = bbox_window(&h, 0, [499000.0, 4599000.0, 500055.0, 4601000.0]).unwrap();
        assert_eq!((w.x0, w.y0, w.width, w.height), (0, 0, 6, 50));
        // Рамка вне снимка.
        assert_eq!(bbox_window(&h, 0, [400000.0, 0.0, 400100.0, 100.0]), None);
        let gt = window_transform(
            &h,
            0,
            &Window {
                x0: 10,
                y0: 10,
                width: 15,
                height: 20,
            },
        );
        assert_eq!(gt, Some([500100.0, 10.0, 0.0, 4599900.0, 0.0, -10.0]));
    }
}

#[test]
fn model_transformation_pixel_is_point_geographic() {
    let i = img(16, 16, 16, 1, 16, 1, 1, 1, |x, y, _| u16_value(x, y));
    let mut o = opts(false, true);
    let m = [
        0.5, 0.0, 0.0, 30.0, //
        0.0, -0.25, 0.0, 60.0, //
        0.0, 0.0, 0.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ];
    o.geo = vec![
        tag_f64(false, 34264, &m),
        tag_ints(
            false,
            34735,
            3,
            &[1, 1, 0, 3, 1024, 0, 1, 2, 1025, 0, 1, 2, 2048, 0, 1, 4326],
        ),
    ];
    let file = build(&[i], &o);
    let h = header(&file);
    assert!(h.pixel_is_point);
    assert_eq!(h.epsg, Some(4326));
    assert_eq!(h.geotransform, Some([29.75, 0.5, 0.0, 60.125, 0.0, -0.25]));
    assert_eq!(h.nodata, None);
    assert_eq!(h.scale, None);
}

#[test]
fn user_defined_projection_has_no_epsg() {
    let i = img(16, 16, 16, 1, 16, 1, 1, 1, |x, y, _| u16_value(x, y));
    let mut o = opts(false, false);
    o.geo = vec![
        tag_f64(false, 33550, &[463.3127, 463.3127, 0.0]),
        tag_f64(
            false,
            33922,
            &[0.0, 0.0, 0.0, -20015109.354, 10007554.677, 0.0],
        ),
        tag_ints(
            false,
            34735,
            3,
            &[1, 1, 0, 2, 1024, 0, 1, 1, 3072, 0, 1, 32767],
        ),
    ];
    let file = build(&[i], &o);
    let h = header(&file);
    assert_eq!(h.epsg, None);
    assert!(h.geotransform.is_some());
}

// ---------------------------------------------------------------------
// Обзоры

fn level_value(level: u32) -> impl Fn(u32, u32, u16) -> u64 {
    move |x, y, _| u64::from(level * 20000 + y * 300 + x)
}

/// Полное 64 × 48, обзоры 32 × 24 и 16 × 12, между ними маска.
fn pyramid(o: &Opts) -> Vec<u8> {
    let full = img(64, 48, 16, 1, 16, 1, 8, 2, level_value(0));
    let mut ov1 = img(32, 24, 16, 1, 16, 1, 8, 2, level_value(1));
    ov1.subfile = 1;
    let mut mask = img(64, 48, 16, 1, 8, 1, 8, 1, |_, _, _| 255);
    mask.subfile = 4;
    let mut ov2 = img(16, 12, 16, 1, 16, 1, 8, 2, level_value(2));
    ov2.subfile = 1;
    build(&[full, ov1, mask, ov2], o)
}

#[test]
fn overview_levels_and_selection() {
    let mut o = opts(false, false);
    o.geo = utm_geo(false);
    let file = pyramid(&o);
    let h = header(&file);
    assert_eq!(h.levels.len(), 3);
    assert_eq!(
        h.levels
            .iter()
            .map(|l| (l.width, l.height))
            .collect::<Vec<_>>(),
        vec![(64, 48), (32, 24), (16, 12)]
    );
    assert_eq!(h.level_resolution(0), Some((10.0, 10.0)));
    assert_eq!(h.level_resolution(1), Some((20.0, 20.0)));
    assert_eq!(h.level_resolution(2), Some((40.0, 40.0)));
    assert_eq!(
        h.level_transform(2),
        Some([500000.0, 40.0, 0.0, 4600000.0, 0.0, -40.0])
    );
    for (res, level) in [
        (1.0, 0),
        (10.0, 0),
        (19.99, 0),
        (20.0, 1),
        (39.9, 1),
        (40.0, 2),
        (1000.0, 2),
        (f64::NAN, 0),
        (-5.0, 0),
    ] {
        assert_eq!(level_for_resolution(&h, res), level, "шаг {res}");
    }
    assert_eq!(level_for_factor(&h, 2.0), 1);
    assert_eq!(level_for_factor(&h, 3.9), 1);
    assert_eq!(level_for_factor(&h, 4.0), 2);
    // Окна обзоров - свои значения.
    let g = read(&h, &file, 1, 5, 3, 20, 20, 0).unwrap();
    check(&g, 5, 3, |x, y| level_value(1)(x, y, 0) as f64);
    let g = read(&h, &file, 2, 0, 0, 16, 12, 0).unwrap();
    check(&g, 0, 0, |x, y| level_value(2)(x, y, 0) as f64);
    // Рамка на обзоре 1: 20 м пиксель.
    let w = bbox_window(&h, 1, [500100.0, 4599700.0, 500250.0, 4599900.0]).unwrap();
    assert_eq!((w.x0, w.y0, w.width, w.height), (5, 5, 8, 10));
    assert_eq!(
        read(&h, &file, 3, 0, 0, 1, 1, 0).unwrap_err(),
        CogError::NoLevel
    );
}

// ---------------------------------------------------------------------
// Дочитывание кусков

/// Разбор по кускам: начало first байт, дальше запрошенное. Количество
/// кругов запросов и заголовок.
fn rounds(file: &[u8], first: usize, len_known: bool) -> (usize, CogHeader, u64) {
    let mut chunks = Chunks::from_prefix(&file[..first.min(file.len())]);
    if len_known {
        chunks.set_file_len(file.len() as u64);
    }
    for round in 0..10 {
        match CogHeader::parse_chunks(&chunks).unwrap() {
            Parsed::Header(h) => return (round, h, chunks.covered()),
            Parsed::NeedMore(ranges) => {
                assert!(!ranges.is_empty());
                let mut grew = false;
                for r in ranges {
                    assert!(r.start < r.end, "пустой диапазон {r:?}");
                    let end = r.end.min(file.len() as u64);
                    grew |= chunks.add(r.start, &file[r.start as usize..end as usize]);
                }
                assert!(grew, "круг {round} не добавил байт");
            }
        }
    }
    panic!("заголовок не разобран за 10 кругов");
}

#[test]
fn need_more_flow() {
    for (big, bigtiff) in [(false, false), (true, true), (false, true)] {
        let full = img(300, 200, 16, 1, 16, 1, 8, 2, |x, y, _| {
            u64::from((x * 3 + y * 11) % 65536)
        });
        let mut ov = img(150, 100, 16, 1, 16, 1, 8, 2, level_value(1));
        ov.subfile = 1;
        let mut o = opts(big, bigtiff);
        o.geo = utm_geo(big);
        o.far = true;
        let file = build(&[full, ov], &o);
        let whole = header(&file);
        for len_known in [false, true] {
            // С 16 байт: IFD0, затем его массивы и IFD1, затем массивы IFD1.
            let (n, h, covered) = rounds(&file, 16, len_known);
            assert_eq!(h, whole);
            assert!((1..=4).contains(&n), "кругов {n}");
            assert!(covered < file.len() as u64 / 2, "прочитано {covered}");
        }
        // Пустое начало: первый круг - начало файла.
        assert_eq!(
            CogHeader::parse(&[]).unwrap(),
            Parsed::NeedMore(std::iter::once(0..data::cog::PREFIX_LEN).collect())
        );
    }
    // IFD в начале файла: хватает начала файла.
    let i = img(300, 200, 16, 1, 16, 1, 8, 2, |x, y, _| u64::from(x + y));
    let file = build(&[i], &opts(false, false));
    let prefix = &file[..(data::cog::PREFIX_LEN as usize).min(file.len())];
    assert!(matches!(
        CogHeader::parse(prefix).unwrap(),
        Parsed::Header(_)
    ));
}

#[test]
fn tiles_through_chunks() {
    let f = |x: u32, y: u32, _| u16_value(x, y);
    let i = img(70, 50, 16, 1, 16, 1, 8, 2, f);
    let file = build(&[i], &opts(false, true));
    let h = header(&file);
    let plan = window_plan(&h, 0, 20, 20, 40, 25).unwrap();
    let merged = merge_ranges(plan.iter().map(|t| t.range.clone()).collect(), 0);
    // Тайлы строки лежат подряд, запросов меньше, чем тайлов.
    assert!(merged.len() < plan.len());
    let mut chunks = Chunks::new();
    for r in &merged {
        assert!(chunks.add(r.start, &file[r.start as usize..r.end as usize]));
    }
    let map: HashMap<usize, Range<u64>> = plan.iter().map(|t| (t.index, t.range.clone())).collect();
    let g = read_window(&h, 0, 20, 20, 40, 25, &|i| {
        map.get(&i).and_then(|r| chunks.get(r.clone()))
    })
    .unwrap();
    check(&g, 20, 20, |x, y| u16_value(x, y) as f64);
    // Нет байт тайла - ошибка с его номером.
    let first = plan[0].index;
    let err = read_window(&h, 0, 20, 20, 40, 25, &|i| {
        if i == first {
            None
        } else {
            map.get(&i).and_then(|r| chunks.get(r.clone()))
        }
    })
    .unwrap_err();
    assert_eq!(err, CogError::MissingTile(first));
    // Испорченные байты тайла - ошибка распаковки.
    let junk = vec![0xAB_u8; 4096];
    let err = read_window(&h, 0, 20, 20, 40, 25, &|_| Some(junk.as_slice())).unwrap_err();
    assert!(matches!(err, CogError::Decode(_)));
    // Окно за снимком.
    assert_eq!(
        window_plan(&h, 0, 60, 0, 11, 1).unwrap_err(),
        CogError::Window
    );
    assert_eq!(
        window_plan(&h, 0, 0, 0, 0, 1).unwrap_err(),
        CogError::Window
    );
}

#[test]
fn chunks_merge() {
    let mut c = Chunks::new();
    assert!(c.add(10, &[1, 2, 3]));
    assert!(c.add(13, &[4, 5]));
    assert!(!c.add(11, &[2, 3]));
    assert!(c.add(0, &[9; 4]));
    assert_eq!(c.get(10..15), Some(&[1, 2, 3, 4, 5][..]));
    assert_eq!(c.get(0..4), Some(&[9; 4][..]));
    assert_eq!(c.get(3..11), None);
    assert!(c.add(4, &[7; 6]));
    assert_eq!(c.get(3..11), Some(&[9, 7, 7, 7, 7, 7, 7, 1][..]));
    assert_eq!(c.covered(), 15);
    assert_eq!(
        merge_ranges(vec![20..30, 0..5, 4..8, 31..40, 100..101], 1),
        vec![0..8, 20..40, 100..101]
    );
}

// ---------------------------------------------------------------------
// Враждебный вход

/// Разбор и чтение всего уровня 0 не паникуют.
fn survive(file: &[u8], len_known: bool) {
    let mut chunks = Chunks::from_prefix(file);
    if len_known {
        chunks.set_file_len(file.len() as u64);
    }
    if let Ok(Parsed::Header(h)) = CogHeader::parse_chunks(&chunks) {
        for level in 0..h.levels.len() {
            let l = &h.levels[level];
            let (w, hh) = (l.width.min(4096), l.height.min(4096));
            let _ = read(&h, file, level, 0, 0, w, hh, 0);
            let _ = bbox_window(&h, level, [0.0, 0.0, 1e6, 1e7]);
        }
        let _ = level_for_resolution(&h, 15.0);
    }
}

fn samples() -> Vec<Vec<u8>> {
    let mut o = opts(true, true);
    o.geo = utm_geo(true);
    o.far = true;
    let mut lzw = img(40, 20, 16, 1, 16, 1, 5, 2, |x, y, _| u16_value(x, y));
    lzw.subfile = 0;
    vec![
        pyramid(&opts(false, false)),
        build(
            &[img(33, 17, 16, 2, 32, 3, 8, 3, |x, y, _| {
                u64::from(f32_value(x, y).to_bits())
            })],
            &o,
        ),
        build(&[lzw], &opts(false, false)),
        build(
            &[img(20, 20, 16, 1, 16, 1, 50000, 1, |x, y, _| {
                u16_value(x, y)
            })],
            &opts(true, false),
        ),
    ]
}

#[test]
fn truncated_never_panics() {
    for file in samples() {
        for len in 0..file.len() {
            let cut = &file[..len];
            for len_known in [false, true] {
                let mut chunks = Chunks::from_prefix(cut);
                if len_known {
                    chunks.set_file_len(len as u64);
                }
                let parsed = CogHeader::parse_chunks(&chunks);
                if len_known && len < 8 {
                    assert!(parsed.is_err(), "длина {len}");
                }
                survive(cut, len_known);
            }
        }
    }
}

#[test]
fn corrupt_never_panics() {
    for file in samples() {
        let n = file.len().min(1500);
        for i in 0..n {
            for v in [0x00, 0xFF, file[i] ^ 0x55, file[i].wrapping_add(1)] {
                let mut bad = file.clone();
                bad[i] = v;
                survive(&bad, true);
                survive(&bad, false);
            }
        }
    }
    // Случайные байты после подписи TIFF и BigTIFF.
    let mut seed: u64 = 0x1234_5678_9abc_def0;
    for k in 0..400 {
        let len = 16 + (k * 37) % 3000;
        let mut bad = Vec::with_capacity(len);
        bad.extend_from_slice(match k % 4 {
            0 => b"II*\0",
            1 => b"MM\0*",
            2 => b"II+\0",
            _ => b"MM\0+",
        });
        if k % 4 >= 2 {
            bad.extend_from_slice(if k % 4 == 2 {
                &[8, 0, 0, 0]
            } else {
                &[0, 8, 0, 0]
            });
        }
        while bad.len() < len {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            bad.push((seed >> 33) as u8);
        }
        survive(&bad, true);
        survive(&bad, false);
    }
    assert_eq!(
        CogHeader::parse(b"GIF89a\0\0\0\0").unwrap_err(),
        CogError::NotTiff
    );
    assert_eq!(
        CogHeader::parse(b"II\x2a\x00\x08\x00\x00\x00\x00\x00").unwrap_err(),
        CogError::Corrupt("количество записей IFD")
    );
}

#[test]
fn ifd_loop_is_error() {
    // IFD, ссылающийся на себя.
    let i = img(16, 16, 16, 1, 16, 1, 1, 1, |x, y, _| u16_value(x, y));
    let mut file = build(&[i], &opts(false, false));
    let n = u16::from_le_bytes([file[8], file[9]]) as usize;
    let next = 8 + 2 + n * 12;
    file[next..next + 4].copy_from_slice(&8u32.to_le_bytes());
    assert_eq!(
        CogHeader::parse(&file).unwrap_err(),
        CogError::Corrupt("кольцо IFD")
    );
}
