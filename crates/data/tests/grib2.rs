// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Свой разбор GRIB2 (`data::grib2`): синтетические сообщения и сверка
//! с terrio на полях GFS. Поля GFS - файлы `*.grib2` в папке
//! `PLANETX_GRIB_DIR` (участки файла выпуска по индексу .idx, как их
//! берёт программа), без неё сверка пропускается.

use data::grib2;

/// Сообщение GRIB2: сетка ni × nj от (lat1, lon1) с шагом step, простая
/// упаковка 8 бит, значения = reference + x, параметр (дисциплина,
/// категория, номер), битовая маска или None.
#[allow(clippy::too_many_arguments)]
fn message(ni: u32, nj: u32, lat1: f64, lon1: f64, step: f64, param: (u8, u8, u8), reference: f32, xs: &[u8], mask: Option<&[u8]>) -> Vec<u8> {
    let micro = |v: f64| -> [u8; 4] {
        let m = (v.abs() * 1e6).round() as u32;
        let raw = if v < 0.0 { m | 0x8000_0000 } else { m };
        raw.to_be_bytes()
    };
    let mut s3 = vec![0u8; 72];
    s3[4] = 3;
    s3[6..10].copy_from_slice(&(ni * nj).to_be_bytes());
    s3[30..34].copy_from_slice(&ni.to_be_bytes());
    s3[34..38].copy_from_slice(&nj.to_be_bytes());
    s3[46..50].copy_from_slice(&micro(lat1));
    s3[50..54].copy_from_slice(&micro(lon1));
    s3[63..67].copy_from_slice(&micro(step));
    s3[67..71].copy_from_slice(&micro(step));
    let mut s4 = vec![0u8; 34];
    s4[4] = 4;
    s4[9] = param.1;
    s4[10] = param.2;
    let mut s5 = vec![0u8; 21];
    s5[4] = 5;
    s5[5..9].copy_from_slice(&(xs.len() as u32).to_be_bytes());
    s5[11..15].copy_from_slice(&reference.to_be_bytes());
    s5[19] = 8;
    let s6 = match mask {
        Some(m) => [vec![0, 0, 0, 0, 6, 0], m.to_vec()].concat(),
        None => vec![0, 0, 0, 0, 6, 255],
    };
    let s7 = [vec![0, 0, 0, 0, 7], xs.to_vec()].concat();
    let mut body = Vec::new();
    for mut s in [vec![0u8; 21], s3, s4, s5, s6, s7] {
        if s.len() == 21 && s[4] == 0 {
            s[4] = 1;
        }
        let len = (s.len() as u32).to_be_bytes();
        s[0..4].copy_from_slice(&len);
        body.extend(s);
    }
    let total = (16 + body.len() + 4) as u64;
    let mut out = b"GRIB".to_vec();
    out.extend([0, 0, param.0, 2]);
    out.extend(total.to_be_bytes());
    out.extend(body);
    out.extend(b"7777");
    out
}

#[test]
fn simple_packing_and_global_shift() {
    // Сетка 4 × 2 от 0° с шагом 90°: глобальная по долготе.
    let xs = [0, 1, 2, 3, 10, 11, 12, 13];
    let bytes = message(4, 2, 45.0, 0.0, 90.0, (0, 2, 2), 100.0, &xs, None);
    let fields = grib2::decode(&bytes).unwrap_or_default();
    assert_eq!(fields.len(), 1);
    let f = &fields[0];
    assert_eq!((f.ni, f.nj, f.category, f.number), (4, 2, 2, 2));
    assert_eq!(f.values, [100.0, 101.0, 102.0, 103.0, 110.0, 111.0, 112.0, 113.0]);
    // С севера на юг, первый столбец - −180°: долготы 0, 90, 180, 270
    // встают как 180, 270, 0, 90.
    let (values, rows, cols, west, north, step) = grib2::north_up(f);
    assert_eq!((rows, cols, west, north, step), (2, 4, -180.0, 45.0, 90.0));
    assert_eq!(values, [102.0, 103.0, 100.0, 101.0, 112.0, 113.0, 110.0, 111.0]);
}

#[test]
fn temperature_in_celsius_and_bitmap() {
    // Температура (0, 0, 0) - из кельвинов в градусы Цельсия, как GDAL.
    let t = grib2::decode(&message(2, 1, 0.0, 0.0, 1.0, (0, 0, 0), 273.15, &[0, 10], None)).unwrap_or_default();
    assert!(t[0].values.iter().zip([0.0f32, 10.0]).all(|(a, b)| (a - b).abs() < 1e-3), "{:?}", t[0].values);
    // Маска 1010: значения во 2-м и 4-м узлах нет.
    let m = grib2::decode(&message(4, 1, 0.0, 0.0, 1.0, (0, 1, 7), 0.0, &[5, 6], Some(&[0b1010_0000]))).unwrap_or_default();
    let v = &m[0].values;
    assert_eq!((v[0], v[2]), (5.0, 6.0));
    assert!(v[1].is_nan() && v[3].is_nan());
}

#[test]
fn hostile_messages_are_errors() {
    assert!(grib2::decode(b"").is_err());
    assert!(grib2::decode(b"not a grib file at all").is_err());
    let good = message(2, 1, 0.0, 0.0, 1.0, (0, 1, 7), 0.0, &[1, 2], None);
    // Обрезанный файл.
    assert!(grib2::decode(&good[..good.len() - 10]).is_err());
    // Сетка не сходится с числом значений.
    let wrong = message(3, 1, 0.0, 0.0, 1.0, (0, 1, 7), 0.0, &[1, 2], None);
    assert!(grib2::decode(&wrong).is_err());
}

/// Сверка с terrio на полях GFS: значение в значение, как GDAL их отдаёт.
#[test]
fn gfs_fields_match_terrio() {
    let Some(dir) = std::env::var_os("PLANETX_GRIB_DIR") else {
        eprintln!("PLANETX_GRIB_DIR не задана, сверка с terrio пропущена");
        return;
    };
    let mut checked = 0;
    let dir = std::path::PathBuf::from(dir);
    let mut inputs: Vec<(std::path::PathBuf, Vec<u8>)> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "grib2"))
        .map(|p| {
            let bytes = std::fs::read(&p).unwrap_or_default();
            (p, bytes)
        })
        .collect();
    // Ветер программа берёт одним участком: U и V подряд, два сообщения.
    if let (Ok(u), Ok(v)) = (std::fs::read(dir.join("ugrd.grib2")), std::fs::read(dir.join("vgrd.grib2"))) {
        inputs.push((dir.join("ugrd+vgrd"), [u, v].concat()));
    }
    for (path, bytes) in inputs {
        let fields = grib2::decode(&bytes);
        assert!(fields.is_ok(), "{}: {fields:?}", path.display());
        let fields = fields.unwrap_or_default();
        let ds = terrio::memory::open_raster_bytes(bytes, "field.grib2");
        assert!(ds.is_ok(), "{}", path.display());
        let Ok(ds) = ds else {
            continue;
        };
        let r = &ds.raster;
        let g = ds.geo_transform();
        for (band, f) in fields.iter().enumerate() {
            let (values, rows, cols, west, north, step) = grib2::north_up(f);
            assert_eq!((rows, cols), (r.rows, r.cols), "{}", path.display());
            assert!((west - (g[0] + g[1] / 2.0)).abs() < 1e-6 && (north - (g[3] + g[5] / 2.0)).abs() < 1e-6 && (step - g[1].abs()).abs() < 1e-9, "{}: {west} {north} {step} vs {g:?}", path.display());
            let mut worst = 0.0f64;
            for row in 0..rows {
                for col in 0..cols {
                    let ours = f64::from(values[row * cols + col]);
                    let theirs = r.get(band as isize, row as isize, col as isize);
                    worst = worst.max((ours - theirs).abs() / theirs.abs().max(1.0));
                }
            }
            // Допуск - округление f32 около 300 K: порядок действий у terrio
            // другой. Точность самих полей GFS - на порядки грубее.
            assert!(worst < 1e-4, "{} band {band}: relative difference {worst}", path.display());
            checked += 1;
        }
    }
    assert!(checked > 0, "в папке нет полей GRIB2");
}
