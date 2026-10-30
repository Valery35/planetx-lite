// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Запись GeoTIFF результата анализа: файл читается обратно terrio,
//! как свой растр Lite, привязка и значения те же.

use data::geotiff::{self, BYTE_NODATA, LatLonGrid, NODATA};

fn grid() -> LatLonGrid {
    LatLonGrid { north: 58.01, west: 56.24, dlat: 0.001, dlon: 0.002, cols: 5, rows: 3 }
}

fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}

#[test]
fn float_raster_reads_back() {
    let mut values: Vec<f64> = (0..15).map(f64::from).collect();
    values[7] = f64::NAN;
    let bytes = geotiff::write_float(&grid(), &values);
    assert!(bytes.is_some());
    let Some(bytes) = bytes else { return };
    let ds = terrio::memory::open_raster_bytes(bytes, "result.tif");
    assert!(ds.is_ok());
    let Ok(ds) = ds else { return };
    assert_eq!((ds.raster.cols, ds.raster.rows), (5, 3));
    // Угол растра - на полшага к северо-западу от первого узла.
    let t = ds.geo_transform();
    close(t[0], 56.24 - 0.001);
    close(t[1], 0.002);
    close(t[3], 58.01 + 0.0005);
    close(t[5], -0.001);
    assert_eq!(ds.crs().authority().map(|(_, c)| c), Some(4326));
    assert_eq!(ds.nodata(), Some(NODATA));
    close(ds.raster.get(0, 0, 0), 0.0);
    close(ds.raster.get(0, 2, 4), 14.0);
    close(ds.raster.get(0, 1, 2), NODATA);
}

#[test]
fn byte_raster_reads_back() {
    let values: Vec<u8> = (0..15).map(|v| if v == 3 { BYTE_NODATA } else { v }).collect();
    let bytes = geotiff::write_byte(&grid(), &values);
    assert!(bytes.is_some());
    let Some(bytes) = bytes else { return };
    let ds = terrio::memory::open_raster_bytes(bytes, "result.tif");
    assert!(ds.is_ok());
    let Ok(ds) = ds else { return };
    assert_eq!(ds.nodata(), Some(f64::from(BYTE_NODATA)));
    close(ds.raster.get(0, 1, 0), 5.0);
    close(ds.raster.get(0, 0, 3), f64::from(BYTE_NODATA));
}

#[test]
fn mercator_raster_reads_back() {
    let g = data::geotiff::MercatorGrid { x0: 6_261_000.0, y0: 7_960_000.0, step: 30.0, cols: 4, rows: 2 };
    let bytes = geotiff::write_float_mercator(&g, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, f64::NAN]);
    assert!(bytes.is_some());
    let Some(bytes) = bytes else { return };
    let ds = terrio::memory::open_raster_bytes(bytes, "slope.tif");
    assert!(ds.is_ok());
    let Ok(ds) = ds else { return };
    assert_eq!(ds.crs().authority().map(|(_, c)| c), Some(3857));
    let t = ds.geo_transform();
    close(t[0], 6_261_000.0 - 15.0);
    close(t[3], 7_960_000.0 + 15.0);
    close(t[1], 30.0);
    close(ds.raster.get(0, 1, 3), NODATA);
    // Зона UTM 40N файлов AlphaEarth.
    let bytes = geotiff::write_byte_projected(&g, 32640, &[0, 1, 2, 3, 4, 5, 6, 255]);
    let ds = bytes.and_then(|b| terrio::memory::open_raster_bytes(b, "change.tif").ok());
    assert!(ds.is_some_and(|d| d.crs().authority().map(|(_, c)| c) == Some(32640) && d.nodata() == Some(255.0)));
}

#[test]
fn bad_grid_gives_nothing() {
    let mut g = grid();
    assert!(geotiff::write_float(&g, &[0.0; 14]).is_none());
    g.dlat = 0.0;
    assert!(geotiff::write_float(&g, &[0.0; 15]).is_none());
    g.dlat = f64::NAN;
    assert!(geotiff::write_byte(&g, &[0; 15]).is_none());
    g = grid();
    g.cols = 0;
    assert!(geotiff::write_byte(&g, &[]).is_none());
}
