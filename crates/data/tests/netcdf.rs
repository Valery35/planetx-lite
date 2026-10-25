// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Чтение netCDF-3 и поле течений ERDDAP. Файлы собираются в памяти
//! по спецификации netCDF-3 classic и 64-bit offset.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use data::flow;
use data::netcdf::{Format, NcFile, NcType};

/// Значения атрибута или переменной.
#[derive(Clone)]
enum Val {
    Text(&'static str),
    I16(Vec<i16>),
    F32(Vec<f32>),
    F64(Vec<f64>),
}

impl Val {
    fn code(&self) -> u32 {
        match self {
            Val::Text(_) => 2,
            Val::I16(_) => 3,
            Val::F32(_) => 5,
            Val::F64(_) => 6,
        }
    }

    fn len(&self) -> usize {
        match self {
            Val::Text(t) => t.len(),
            Val::I16(v) => v.len(),
            Val::F32(v) => v.len(),
            Val::F64(v) => v.len(),
        }
    }

    fn bytes(&self) -> Vec<u8> {
        match self {
            Val::Text(t) => t.as_bytes().to_vec(),
            Val::I16(v) => v.iter().flat_map(|x| x.to_be_bytes()).collect(),
            Val::F32(v) => v.iter().flat_map(|x| x.to_be_bytes()).collect(),
            Val::F64(v) => v.iter().flat_map(|x| x.to_be_bytes()).collect(),
        }
    }
}

struct Var {
    name: &'static str,
    dims: Vec<u32>,
    attrs: Vec<(&'static str, Val)>,
    data: Val,
    record: bool,
}

fn var(name: &'static str, dims: &[u32], attrs: Vec<(&'static str, Val)>, data: Val) -> Var {
    Var { name, dims: dims.to_vec(), attrs, data, record: false }
}

fn pad(buf: &mut Vec<u8>) {
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
}

fn put_name(buf: &mut Vec<u8>, name: &str) {
    buf.extend((name.len() as u32).to_be_bytes());
    buf.extend(name.as_bytes());
    pad(buf);
}

fn put_attrs(buf: &mut Vec<u8>, attrs: &[(&str, Val)]) {
    if attrs.is_empty() {
        buf.extend([0u8; 8]);
        return;
    }
    buf.extend(0x0Cu32.to_be_bytes());
    buf.extend((attrs.len() as u32).to_be_bytes());
    for (name, value) in attrs {
        put_name(buf, name);
        buf.extend(value.code().to_be_bytes());
        buf.extend((value.len() as u32).to_be_bytes());
        buf.extend(value.bytes());
        pad(buf);
    }
}

/// Файл netCDF-3 версии version (1 или 2). Переменные записей - после
/// обычных, записи чередуются, numrecs - количество записей.
fn build(version: u8, numrecs: u32, dims: &[(&str, u32)], gatts: &[(&str, Val)], vars: &[Var]) -> Vec<u8> {
    let records: Vec<usize> = (0..vars.len()).filter(|&i| vars[i].record).collect();
    let slab = |v: &Var| v.data.bytes().len() / if v.record { numrecs.max(1) as usize } else { 1 };
    let padded = |n: usize| n.div_ceil(4) * 4;
    let single = records.len() == 1;
    let header = |begins: &[u64]| {
        let mut buf = b"CDF".to_vec();
        buf.push(version);
        buf.extend(numrecs.to_be_bytes());
        if dims.is_empty() {
            buf.extend([0u8; 8]);
        } else {
            buf.extend(0x0Au32.to_be_bytes());
            buf.extend((dims.len() as u32).to_be_bytes());
            for (name, len) in dims {
                put_name(&mut buf, name);
                buf.extend(len.to_be_bytes());
            }
        }
        put_attrs(&mut buf, gatts);
        buf.extend(0x0Bu32.to_be_bytes());
        buf.extend((vars.len() as u32).to_be_bytes());
        for (v, begin) in vars.iter().zip(begins) {
            put_name(&mut buf, v.name);
            buf.extend((v.dims.len() as u32).to_be_bytes());
            for d in &v.dims {
                buf.extend(d.to_be_bytes());
            }
            put_attrs(&mut buf, &v.attrs);
            buf.extend(v.data.code().to_be_bytes());
            buf.extend((padded(slab(v)) as u32).to_be_bytes());
            if version == 1 {
                buf.extend((*begin as u32).to_be_bytes());
            } else {
                buf.extend(begin.to_be_bytes());
            }
        }
        buf
    };
    let mut begins = vec![0u64; vars.len()];
    let mut offset = header(&begins).len() as u64;
    for (i, v) in vars.iter().enumerate().filter(|(_, v)| !v.record) {
        begins[i] = offset;
        offset += padded(slab(v)) as u64;
    }
    for &i in &records {
        begins[i] = offset;
        offset += if single { slab(&vars[i]) } else { padded(slab(&vars[i])) } as u64;
    }
    let mut buf = header(&begins);
    for v in vars.iter().filter(|v| !v.record) {
        buf.extend(v.data.bytes());
        pad(&mut buf);
    }
    for r in 0..numrecs as usize {
        for &i in &records {
            let n = slab(&vars[i]);
            buf.extend(&vars[i].data.bytes()[r * n..(r + 1) * n]);
            if !single {
                pad(&mut buf);
            }
        }
    }
    buf
}

fn packed_file(version: u8) -> Vec<u8> {
    build(
        version,
        0,
        &[("lat", 2), ("lon", 3)],
        &[("title", Val::Text("test\0")), ("version", Val::F64(vec![1.5, 2.0]))],
        &[
            var(
                "temp",
                &[0, 1],
                vec![
                    ("scale_factor", Val::F32(vec![0.5])),
                    ("add_offset", Val::F64(vec![10.0])),
                    ("_FillValue", Val::I16(vec![-1])),
                    ("units", Val::Text("degC")),
                ],
                Val::I16(vec![0, 2, -1, 4, -6, 8]),
            ),
            var("lat", &[0], vec![], Val::F64(vec![-1.0, 1.0])),
        ],
    )
}

#[test]
fn reads_classic_packed_variable() {
    let bytes = packed_file(1);
    let file = NcFile::parse(&bytes).unwrap();
    assert_eq!(file.format, Format::Classic);
    assert_eq!(file.dims.len(), 2);
    assert_eq!(file.dimension("lon").unwrap().len, 3);
    assert_eq!(file.attribute("title").unwrap().text(), Some("test"));
    assert_eq!(file.attribute("version").unwrap().numbers(), &[1.5, 2.0]);
    let temp = file.variable("temp").unwrap();
    assert_eq!(temp.kind, NcType::Short);
    assert_eq!(file.shape(temp), vec![2, 3]);
    assert_eq!(temp.attr("units").unwrap().text(), Some("degC"));
    assert_eq!(file.read_raw("temp").unwrap(), vec![0.0, 2.0, -1.0, 4.0, -6.0, 8.0]);
    let got = file.read_f64("temp").unwrap();
    assert_eq!(got[..2], [10.0, 11.0]);
    assert!(got[2].is_nan());
    assert_eq!(got[3..], [12.0, 7.0, 14.0]);
    assert_eq!(file.read_f64("lat").unwrap(), vec![-1.0, 1.0]);
    assert!(file.read_f64("nothing").is_err());
}

#[test]
fn reads_offset64_with_missing_values() {
    let bytes = packed_file(2);
    let file = NcFile::parse(&bytes).unwrap();
    assert_eq!(file.format, Format::Offset64);
    assert_eq!(file.read_f64("lat").unwrap(), vec![-1.0, 1.0]);

    let bytes = build(
        2,
        0,
        &[("x", 5)],
        &[],
        &[var(
            "f",
            &[0],
            vec![("missing_value", Val::F32(vec![-999.0, -998.0]))],
            Val::F32(vec![1.0, -999.0, f32::NAN, -998.0, 9.969_21e36]),
        )],
    );
    let got = NcFile::parse(&bytes).unwrap().read_f64("f").unwrap();
    assert_eq!(got[0], 1.0);
    // Заполнитель по умолчанию у float без _FillValue - тоже пусто.
    assert!(got[1..].iter().all(|x| x.is_nan()), "{got:?}");
}

fn record_file(numrecs: u32, two: bool) -> Vec<u8> {
    let mut vars = vec![
        var("x", &[1], vec![], Val::F64(vec![5.0, 6.0])),
        Var {
            name: "time",
            dims: vec![0],
            attrs: vec![],
            data: Val::F64((0..numrecs).map(f64::from).collect()),
            record: true,
        },
    ];
    if two {
        vars.push(Var {
            name: "level",
            dims: vec![0, 1],
            attrs: vec![],
            data: Val::I16((0..numrecs as i16 * 2).collect()),
            record: true,
        });
        vars.push(Var { name: "flag", dims: vec![0], attrs: vec![], data: Val::I16(vec![7; numrecs as usize]), record: true });
    }
    build(1, numrecs, &[("time", 0), ("x", 2)], &[], &vars)
}

#[test]
fn reads_record_variables() {
    let bytes = record_file(3, false);
    let file = NcFile::parse(&bytes).unwrap();
    assert_eq!(file.numrecs, 3);
    assert!(file.dimension("time").unwrap().unlimited);
    assert_eq!(file.read_f64("time").unwrap(), vec![0.0, 1.0, 2.0]);
    assert_eq!(file.read_f64("x").unwrap(), vec![5.0, 6.0]);

    let bytes = record_file(3, true);
    let file = NcFile::parse(&bytes).unwrap();
    let level = file.variable("level").unwrap();
    assert!(level.record);
    assert_eq!(file.shape(level), vec![3, 2]);
    assert_eq!(file.read_f64("level").unwrap(), vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
    assert_eq!(file.read_f64("flag").unwrap(), vec![7.0; 3]);
    assert_eq!(file.read_f64("time").unwrap(), vec![0.0, 1.0, 2.0]);

    // Количество записей потоком - по длине файла.
    let mut streaming = bytes.clone();
    streaming[4..8].copy_from_slice(&[0xFF; 4]);
    let file = NcFile::parse(&streaming).unwrap();
    assert_eq!(file.numrecs, 3);
    assert_eq!(file.read_f64("flag").unwrap(), vec![7.0; 3]);
}

#[test]
fn rejects_other_formats() {
    assert!(NcFile::parse(b"").is_err());
    assert!(NcFile::parse(b"\x89HDF\r\n\x1a\n").unwrap_err().0.contains("HDF5"));
    assert!(NcFile::parse(b"CDF\x05\0\0\0\0").is_err());
    assert!(NcFile::parse(b"PK\x03\x04").is_err());
}

#[test]
fn hostile_input_gives_errors() {
    let good = packed_file(1);
    // Каждое обрезание файла - ошибка разбора или чтения, без паники.
    for n in 0..good.len() {
        let cut = &good[..n];
        if let Ok(file) = NcFile::parse(cut) {
            assert!(file.read_f64("temp").is_err() || file.read_f64("lat").is_err(), "{n}");
        }
    }
    // Порча байтов заголовка и данных - без паники.
    let mut state = 0x1234_5678_9abc_def0u64;
    for _ in 0..20000 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let mut bad = good.clone();
        let at = (state % bad.len() as u64) as usize;
        bad[at] = (state >> 32) as u8;
        if let Ok(file) = NcFile::parse(&bad) {
            for v in &file.vars {
                let _ = file.read_f64(&v.name);
            }
        }
        let _ = flow::read_currents(&bad);
    }
    // Огромное количество измерений и смещение за концом файла.
    let mut huge = good.clone();
    huge[12..16].copy_from_slice(&[0xFF; 4]);
    assert!(NcFile::parse(&huge).is_err());
    let file = NcFile::parse(&good).unwrap();
    let begin = file.variable("temp").unwrap().begin as usize;
    let at = good.windows(4).position(|w| w == (begin as u32).to_be_bytes()).unwrap();
    let mut far = good.clone();
    far[at..at + 4].copy_from_slice(&0x7FFF_FFF0u32.to_be_bytes());
    assert!(NcFile::parse(&far).unwrap().read_f64("temp").is_err());
}

/// Выборка ERDDAP в малом: время 1, широты с юга на север, долготы
/// с запада на восток, шаг 1°.
fn currents_file(with_day: bool, lat_step: f64) -> Vec<u8> {
    let nan = f32::NAN;
    let fill = || vec![("_FillValue", Val::F32(vec![nan])), ("units", Val::Text("m s-1"))];
    let gatts: Vec<(&str, Val)> =
        if with_day { vec![("time_coverage_end", Val::Text("2026-10-20T00:00:00Z"))] } else { vec![] };
    build(
        1,
        0,
        &[("time", 1), ("latitude", 3), ("longitude", 4)],
        &gatts,
        &[
            var(
                "time",
                &[0],
                vec![("units", Val::Text("seconds since 1970-01-01T00:00:00Z"))],
                Val::F64(vec![1_792_324_800.0]),
            ),
            var("latitude", &[1], vec![], Val::F64(vec![-lat_step, 0.0, lat_step])),
            var("longitude", &[2], vec![], Val::F64(vec![-1.5, -0.5, 0.5, 1.5])),
            var(
                "ugos",
                &[0, 1, 2],
                fill(),
                Val::F32(vec![1.0, 1.0, 1.0, 1.0, 2.0, 2.0, nan, 2.0, 3.0, 3.0, 3.0, 3.0]),
            ),
            var("vgos", &[0, 1, 2], fill(), Val::F32(vec![-0.5; 12])),
        ],
    )
}

#[test]
fn reads_erddap_currents() {
    let got = flow::read_currents(&currents_file(true, 1.0)).unwrap();
    assert_eq!(got.day, "2026-10-20");
    assert_eq!(got.lats, vec![1.0, 0.0, -1.0]);
    assert_eq!(got.lons, vec![-1.5, -0.5, 0.5, 1.5]);
    let g = &got.grid;
    assert_eq!((g.rows, g.cols, g.west, g.north, g.step), (3, 4, -1.5, 1.0, 1.0));
    // Строки с севера на юг: первая строка - северная широта 1°.
    assert_eq!(g.u[..4], [3.0; 4]);
    assert_eq!(g.u[8..], [1.0; 4]);
    assert!(g.u[6].is_nan());
    let (u, v) = g.sample(1.0, -1.5);
    assert_eq!((u, v), (3.0, -0.5));
    assert!(g.sample(0.0, 0.5).0.is_nan());

    // Без атрибута день - по переменной time.
    let got = flow::read_currents(&currents_file(false, 1.0)).unwrap();
    assert_eq!(got.day, "2026-10-18");
}

#[test]
fn rejects_bad_currents() {
    assert!(flow::read_currents(&currents_file(true, 2.0)).is_err());
    assert!(flow::read_currents(&packed_file(1)).is_err());
    assert!(flow::read_currents(b"<html>Error</html>").is_err());
}
