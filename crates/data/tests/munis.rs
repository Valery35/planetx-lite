// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Границы муниципальных образований СберИндекса: разбор, линии, точка.

use std::io::Write;

use data::munis;

/// Квадрат A 50..51 с.ш., 30..31 в.д. с дырой, квадрат B 31..32 в.д.
/// и кусок Чукотки C, разрезанный по шву 180°.
const UNITS: &str = r#"{"units":[
 {"id":"m_a","name":"А м.р.","kind":"муниципальный район","oktmo":"01234","region":"Край","center":"с. А","from":2018,"lat":50.5,"lon":30.5,
  "rings":[[50,30,51,30,51,31,50,31,50,30],[50.4,30.4,50.6,30.4,50.6,30.6,50.4,30.6,50.4,30.4]]},
 {"id":"m_b","name":"Б г.о.","kind":"городской округ","oktmo":"01235","from":2025,"lat":50.5,"lon":31.5,
  "rings":[[50,31,51,31,51,32,50,32,50,31],[1,2],["x"]]},
 {"id":"m_c","name":"В м.о.","lat":66.5,"lon":179.5,
  "rings":[[66,179,67,179,67,179.9999,66,179.9999,66,179]]},
 {"id":"m_d","name":"Без колец","lat":10,"lon":10,"rings":[]}
]}"#;

#[test]
fn point_finds_its_unit_and_seam_is_not_drawn() {
    let m = munis::parse(UNITS.as_bytes()).unwrap_or_default();
    // Без колец муниципалитет отброшен, кривое кольцо - тоже.
    assert_eq!(m.units.len(), 3);
    assert_eq!(m.units[1].rings.len(), 1);
    assert_eq!(m.unit_at(50.2, 30.2).map(|u| u.id.as_str()), Some("m_a"));
    // В дыре квадрата A - ничего.
    assert!(m.unit_at(50.5, 30.5).is_none());
    assert_eq!(m.unit_at(50.5, 31.8).map(|u| u.id.as_str()), Some("m_b"));
    assert_eq!(m.unit_at(66.5, 179.5).map(|u| u.id.as_str()), Some("m_c"));
    assert!(m.unit_at(52.0, 31.0).is_none());
    assert_eq!(m.units[0].oktmo, "01234");
    // Кольцо C без отрезка по шву - два куска по обе стороны разреза.
    let lines = m.lines();
    assert_eq!(lines.len(), 5);
    assert_eq!(lines[3].points, vec![(66.0, 179.0), (67.0, 179.0), (67.0, 179.9999)]);
    assert_eq!(lines[4].points, vec![(66.0, 179.9999), (66.0, 179.0)]);
}

#[test]
fn gzip_file_reads_like_plain() {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = gz.write_all(UNITS.as_bytes());
    let packed = gz.finish().unwrap_or_default();
    assert_eq!(munis::parse(&packed), munis::parse(UNITS.as_bytes()));
}

/// Настоящий файл слоя из `PLANETX_MUNIS`, без переменной проверка
/// пропускается.
#[test]
fn real_file_finds_perm() {
    let Some(path) = std::env::var_os("PLANETX_MUNIS") else {
        return;
    };
    let bytes = std::fs::read(path).unwrap_or_default();
    let m = munis::parse(&bytes);
    assert!(m.is_ok(), "{:?}", m.as_ref().err());
    let m = m.unwrap_or_default();
    assert_eq!(m.units.len(), 2570);
    let perm = m.unit_at(58.0105, 56.2502);
    assert_eq!(perm.map(|u| u.name.as_str()), Some("Пермский г.о."), "{perm:?}");
    assert!(m.lines().len() > 2570);
}

#[test]
fn broken_file_is_an_error() {
    assert!(munis::parse(b"").is_err());
    assert!(munis::parse(b"{\"units\":[]}").is_err());
    assert!(munis::parse(&[0x1f, 0x8b, 1, 2, 3]).is_err());
}
