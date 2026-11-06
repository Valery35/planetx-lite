// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Распаковка bufferView glTF со сжатием EXT_meshopt_compression по
//! спецификации Khronos (приложения A и B): режимы ATTRIBUTES (поток
//! 0xa0), TRIANGLES (поток 0xe1), INDICES (поток 0xd1) и фильтры
//! OCTAHEDRAL, QUATERNION, EXPONENTIAL. Так сжимает gltfpack, им
//! готовят наборы 3D Tiles и модели для страниц.
//!
//! Вход враждебный: каждое чтение проверяет границы, длина итога -
//! count × byteStride не больше [`MAX_BYTES`], поток, который кончился
//! раньше хвоста или длиннее его, - ошибка.

use crate::models::{ModelError, error};

/// Наибольший размер распакованного bufferView, байты.
pub const MAX_BYTES: usize = 512 << 20;

/// Способ сжатия bufferView.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Attributes,
    Triangles,
    Indices,
}

impl Mode {
    pub fn of(name: &str) -> Option<Mode> {
        match name {
            "ATTRIBUTES" => Some(Mode::Attributes),
            "TRIANGLES" => Some(Mode::Triangles),
            "INDICES" => Some(Mode::Indices),
            _ => None,
        }
    }
}

/// Фильтр после распаковки атрибутов.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    None,
    Octahedral,
    Quaternion,
    Exponential,
}

impl Filter {
    pub fn of(name: &str) -> Option<Filter> {
        match name {
            "NONE" => Some(Filter::None),
            "OCTAHEDRAL" => Some(Filter::Octahedral),
            "QUATERNION" => Some(Filter::Quaternion),
            "EXPONENTIAL" => Some(Filter::Exponential),
            _ => None,
        }
    }
}

/// Распаковать поток data: count элементов по stride байтов.
pub fn decode(data: &[u8], count: usize, stride: usize, mode: Mode, filter: Filter) -> Result<Vec<u8>, ModelError> {
    let size = count.checked_mul(stride).filter(|s| *s <= MAX_BYTES).ok_or_else(|| error("meshopt: buffer too large"))?;
    let mut out = match mode {
        Mode::Attributes => {
            if stride == 0 || !stride.is_multiple_of(4) || stride > 256 {
                return Err(error("meshopt: bad byteStride"));
            }
            attributes(data, count, stride)?
        }
        Mode::Triangles | Mode::Indices => {
            if stride != 2 && stride != 4 || filter != Filter::None {
                return Err(error("meshopt: bad index layout"));
            }
            let indices = if mode == Mode::Triangles { triangles(data, count)? } else { indices(data, count)? };
            if stride == 2 {
                indices.iter().flat_map(|i| (*i as u16).to_le_bytes()).collect()
            } else {
                indices.iter().flat_map(|i| i.to_le_bytes()).collect()
            }
        }
    };
    if out.len() != size {
        return Err(error("meshopt: wrong decoded size"));
    }
    match filter {
        Filter::None => {}
        Filter::Octahedral => octahedral(&mut out, stride)?,
        Filter::Quaternion => quaternion(&mut out, stride)?,
        Filter::Exponential => exponential(&mut out, stride)?,
    }
    Ok(out)
}

/// Чтение потока с проверкой границ: байты до end.
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
    end: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, ModelError> {
        if self.at >= self.end {
            return Err(error("meshopt: stream ends early"));
        }
        let b = self.data.get(self.at).copied().ok_or_else(|| error("meshopt: stream ends early"))?;
        self.at += 1;
        Ok(b)
    }

    fn bytes(&mut self, n: usize) -> Result<&[u8], ModelError> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.end).ok_or_else(|| error("meshopt: stream ends early"))?;
        let out = self.data.get(self.at..end).ok_or_else(|| error("meshopt: stream ends early"))?;
        self.at = end;
        Ok(out)
    }

    /// Целое varint-7 (LEB128), не длиннее 5 байтов.
    fn varint(&mut self) -> Result<u32, ModelError> {
        let mut value = 0u32;
        for shift in [0, 7, 14, 21, 28] {
            let b = self.byte()?;
            value |= u32::from(b & 0x7f) << shift;
            if b < 0x80 {
                return Ok(value);
            }
        }
        Err(error("meshopt: bad varint"))
    }
}

/// Байт дельты из зигзага.
fn unzigzag8(v: u8) -> u8 {
    if v & 1 != 0 { !(v >> 1) } else { v >> 1 }
}

/// Целое со знаком из зигзага 32 бит.
fn unzigzag32(v: u32) -> u32 {
    if v & 1 != 0 { !(v >> 1) } else { v >> 1 }
}

/// Режим ATTRIBUTES: блоки элементов, в блоке - по байту элемента
/// дельты групп по 16, хвост - первый элемент-основа.
fn attributes(data: &[u8], count: usize, stride: usize) -> Result<Vec<u8>, ModelError> {
    match data.first() {
        Some(0xa0) => {}
        Some(h) if h & 0xf0 == 0xa0 => return Err(error(format!("meshopt: attribute codec version {} is not read", h & 0x0f))),
        _ => return Err(error("meshopt: bad attribute header")),
    }
    let tail = stride.max(32);
    if data.len() < 1 + tail {
        return Err(error("meshopt: stream too short"));
    }
    let end = data.len() - tail;
    let mut last: Vec<u8> = data.get(data.len() - stride..).ok_or_else(|| error("meshopt: stream too short"))?.to_vec();
    let mut reader = Reader { data, at: 1, end };
    let mut out = vec![0u8; count * stride];
    let block_max = ((8192 / stride) & !15).min(256);
    if block_max == 0 {
        return Err(error("meshopt: bad byteStride"));
    }
    let mut deltas = vec![0u8; block_max];
    let mut done = 0;
    while done < count {
        let elements = (count - done).min(block_max);
        let groups = elements.div_ceil(16);
        for k in 0..stride {
            let header = reader.bytes(groups.div_ceil(4))?.to_vec();
            for g in 0..groups {
                let bits = header.get(g / 4).map_or(0, |h| (h >> ((g % 4) * 2)) & 3);
                let slot = deltas.get_mut(g * 16..g * 16 + 16).ok_or_else(|| error("meshopt: bad block"))?;
                group(&mut reader, bits, slot)?;
            }
            let mut value = last.get(k).copied().unwrap_or(0);
            for j in 0..elements {
                value = value.wrapping_add(unzigzag8(deltas.get(j).copied().unwrap_or(0)));
                if let Some(o) = out.get_mut((done + j) * stride + k) {
                    *o = value;
                }
            }
            if let Some(l) = last.get_mut(k) {
                *l = value;
            }
        }
        done += elements;
    }
    if reader.at != end {
        return Err(error("meshopt: extra bytes in attribute stream"));
    }
    Ok(out)
}

/// Дельты одной группы из 16 байтов: bits 0 - нули, 1 - по 2 бита,
/// 2 - по 4 бита, 3 - байтами. Все единицы - дельта байтом после группы.
fn group(reader: &mut Reader<'_>, bits: u8, out: &mut [u8]) -> Result<(), ModelError> {
    match bits {
        0 => out.fill(0),
        3 => out.copy_from_slice(reader.bytes(16)?),
        _ => {
            let width = if bits == 1 { 2 } else { 4 };
            let packed = reader.bytes(16 * width / 8)?.to_vec();
            let sentinel = (1u8 << width) - 1;
            let per = 8 / width;
            for (i, slot) in out.iter_mut().enumerate() {
                let byte = packed.get(i / per).copied().unwrap_or(0);
                let shift = 8 - width * (i % per + 1);
                let v = (byte >> shift) & sentinel;
                *slot = if v == sentinel { reader.byte()? } else { v };
            }
        }
    }
    Ok(())
}

/// Очередь последних 16 значений: 0 - самое новое.
struct Fifo<T: Copy + Default> {
    items: [T; 16],
    at: usize,
}

impl<T: Copy + Default> Fifo<T> {
    fn new() -> Self {
        Fifo { items: [T::default(); 16], at: 0 }
    }

    fn push(&mut self, v: T) {
        self.items[self.at & 15] = v;
        self.at = self.at.wrapping_add(1);
    }

    /// Значение номер n от самого нового.
    fn get(&self, n: usize) -> T {
        self.items[self.at.wrapping_sub(1).wrapping_sub(n) & 15]
    }
}

/// Режим TRIANGLES: код на треугольник, дополнительные байты, хвост -
/// таблица codeaux из 16 байтов.
fn triangles(data: &[u8], count: usize) -> Result<Vec<u32>, ModelError> {
    if !count.is_multiple_of(3) {
        return Err(error("meshopt: triangle count"));
    }
    match data.first() {
        Some(0xe1) => {}
        Some(h) if h & 0xf0 == 0xe0 => return Err(error(format!("meshopt: index codec version {} is not read", h & 0x0f))),
        _ => return Err(error("meshopt: bad triangle header")),
    }
    let tris = count / 3;
    if data.len() < 1 + tris + 16 {
        return Err(error("meshopt: stream too short"));
    }
    let end = data.len() - 16;
    let codes = data.get(1..1 + tris).ok_or_else(|| error("meshopt: stream too short"))?;
    let aux = data.get(end..).ok_or_else(|| error("meshopt: stream too short"))?;
    let mut reader = Reader { data, at: 1 + tris, end };
    let mut edges: Fifo<(u32, u32)> = Fifo::new();
    let mut verts: Fifo<u32> = Fifo::new();
    let (mut next, mut last) = (0u32, 0u32);
    let mut out = Vec::with_capacity(count);
    let explicit = |reader: &mut Reader<'_>, last: &mut u32| -> Result<u32, ModelError> {
        *last = last.wrapping_add(unzigzag32(reader.varint()?));
        Ok(*last)
    };
    for &code in codes {
        if code < 0xf0 {
            let (a, b) = edges.get(usize::from(code >> 4));
            let fec = code & 15;
            let c = if fec < 13 {
                let c = if fec == 0 { next } else { verts.get(usize::from(fec)) };
                if fec == 0 {
                    next = next.wrapping_add(1);
                    verts.push(c);
                }
                c
            } else {
                let c = match fec {
                    13 => last.wrapping_sub(1),
                    14 => last.wrapping_add(1),
                    _ => explicit(&mut reader, &mut last)?,
                };
                last = c;
                verts.push(c);
                c
            };
            out.extend([a, b, c]);
            edges.push((c, b));
            edges.push((a, c));
        } else {
            let (aux_byte, a) = if code < 0xfe {
                let aux_byte = aux.get(usize::from(code & 15)).copied().unwrap_or(0);
                let a = next;
                next = next.wrapping_add(1);
                (aux_byte, a)
            } else {
                let aux_byte = reader.byte()?;
                if aux_byte == 0 {
                    next = 0;
                }
                let a = if code == 0xfe {
                    let a = next;
                    next = next.wrapping_add(1);
                    a
                } else {
                    explicit(&mut reader, &mut last)?
                };
                (aux_byte, a)
            };
            let (feb, fec) = (aux_byte >> 4, aux_byte & 15);
            let mut corner = |f: u8, reader: &mut Reader<'_>| -> Result<(u32, bool), ModelError> {
                Ok(match f {
                    0 => {
                        let v = next;
                        next = next.wrapping_add(1);
                        (v, true)
                    }
                    15 if code >= 0xfe => (explicit(reader, &mut last)?, true),
                    _ => (verts.get(usize::from(f) - 1), false),
                })
            };
            let (b, push_b) = corner(feb, &mut reader)?;
            let (c, push_c) = corner(fec, &mut reader)?;
            out.extend([a, b, c]);
            verts.push(a);
            if push_b {
                verts.push(b);
            }
            if push_c {
                verts.push(c);
            }
            edges.push((b, a));
            edges.push((c, b));
            edges.push((a, c));
        }
    }
    if reader.at != end {
        return Err(error("meshopt: extra bytes in triangle stream"));
    }
    Ok(out)
}

/// Режим INDICES: дельты от одной из двух основ, хвост - 4 байта.
fn indices(data: &[u8], count: usize) -> Result<Vec<u32>, ModelError> {
    match data.first() {
        Some(0xd1) => {}
        Some(h) if h & 0xf0 == 0xd0 => return Err(error(format!("meshopt: index sequence codec version {} is not read", h & 0x0f))),
        _ => return Err(error("meshopt: bad index header")),
    }
    if data.len() < 1 + 4 {
        return Err(error("meshopt: stream too short"));
    }
    let end = data.len() - 4;
    let mut reader = Reader { data, at: 1, end };
    let mut last = [0u32; 2];
    let mut out = Vec::with_capacity(count.min(MAX_BYTES / 4));
    for _ in 0..count {
        let v = reader.varint()?;
        let base = (v & 1) as usize;
        let delta = if v & 2 != 0 { !(v >> 2) } else { v >> 2 };
        last[base] = last[base].wrapping_add(delta);
        out.push(last[base]);
    }
    if reader.at != end {
        return Err(error("meshopt: extra bytes in index stream"));
    }
    Ok(out)
}

/// Фильтр OCTAHEDRAL: октаэдрические x, y и единица K бит - в нормаль
/// целыми со знаком.
fn octahedral(out: &mut [u8], stride: usize) -> Result<(), ModelError> {
    match stride {
        4 => {
            for e in out.as_chunks_mut::<4>().0.iter_mut() {
                let v = [e[0] as i8, e[1] as i8, e[2] as i8].map(f32::from);
                let n = unpack_octahedral(v, 127.0);
                for (o, x) in e.iter_mut().zip(n) {
                    *o = (x as i8) as u8;
                }
            }
        }
        8 => {
            for e in out.as_chunks_mut::<8>().0.iter_mut() {
                let get = |i: usize| f32::from(i16::from_le_bytes([e[i * 2], e[i * 2 + 1]]));
                let n = unpack_octahedral([get(0), get(1), get(2)], 32767.0);
                for (i, x) in n.into_iter().enumerate() {
                    let b = (x as i16).to_le_bytes();
                    e[i * 2] = b[0];
                    e[i * 2 + 1] = b[1];
                }
            }
        }
        _ => return Err(error("meshopt: octahedral filter needs byteStride 4 or 8")),
    }
    Ok(())
}

fn unpack_octahedral(v: [f32; 3], max: f32) -> [f32; 3] {
    let one = if v[2] == 0.0 { 1.0 } else { v[2] };
    let (mut x, mut y) = (v[0] / one, v[1] / one);
    let z = 1.0 - x.abs() - y.abs();
    let t = z.min(0.0);
    x -= t.copysign(x);
    y -= t.copysign(y);
    let len = (x * x + y * y + z * z).sqrt();
    let len = if len > 0.0 { len } else { 1.0 };
    [x, y, z].map(|c| (c / len * max).round())
}

/// Фильтр QUATERNION: три составляющие и номер наибольшей - в четыре
/// 16-битных.
fn quaternion(out: &mut [u8], stride: usize) -> Result<(), ModelError> {
    if stride != 8 {
        return Err(error("meshopt: quaternion filter needs byteStride 8"));
    }
    let range = 1.0 / 2f32.sqrt();
    for e in out.as_chunks_mut::<8>().0.iter_mut() {
        let get = |i: usize| i16::from_le_bytes([e[i * 2], e[i * 2 + 1]]);
        let tag = get(3);
        let one = f32::from(tag | 3);
        let [x, y, z] = [0, 1, 2].map(|i| f32::from(get(i)) / one * range);
        let w = (1.0 - x * x - y * y - z * z).max(0.0).sqrt();
        let max = (tag & 3) as usize;
        let mut q = [0i16; 4];
        for (k, value) in [(1, x), (2, y), (3, z), (0, w)] {
            q[(max + k) % 4] = (value * 32767.0).round() as i16;
        }
        for (i, v) in q.into_iter().enumerate() {
            let b = v.to_le_bytes();
            e[i * 2] = b[0];
            e[i * 2 + 1] = b[1];
        }
    }
    Ok(())
}

/// Фильтр EXPONENTIAL: 8 бит порядка и 24 бита мантиссы - в float32.
fn exponential(out: &mut [u8], stride: usize) -> Result<(), ModelError> {
    if !stride.is_multiple_of(4) {
        return Err(error("meshopt: exponential filter needs byteStride divisible by 4"));
    }
    for e in out.as_chunks_mut::<4>().0.iter_mut() {
        let v = i32::from_le_bytes([e[0], e[1], e[2], e[3]]);
        let exp = (v >> 24).clamp(-100, 100);
        let mantissa = (v << 8) >> 8;
        let f = 2f32.powi(exp) * mantissa as f32;
        *e = f.to_le_bytes();
    }
    Ok(())
}
