// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Вулканы GVP и пожары FIRMS: разбор, периоды, цвета и размеры.
//! Перенос `tests/test_volcanoes.py` и `tests/test_fires.py` модуля.

use data::{csv, fires, volcanoes};

const VOLCANOES: &str = "FID,Volcano_Number,Volcano_Name,Volcanic_Landform,Primary_Volcano_Type,Last_Eruption_Year,Country,Region,Subregion,Geological_Summary,Latitude,Longitude,Elevation,Tectonic_Setting,Geologic_Epoch,Evidence_Category
1,300250,Klyuchevskoy,Composite,Stratovolcano,2025,Russia,Kamchatka,x,\"Long, text\",56.056,160.642,4754,Subduction zone,Holocene,Eruption Observed
2,211040,Vesuvius,Composite,Stratovolcano,1944,Italy,Europe,x,s,40.821,14.426,1281,Subduction zone,Holocene,Eruption Observed
3,1,Old,Composite,Caldera,-4350,Japan,x,x,s,35.0,139.0,500,Subduction zone,Holocene,Eruption Dated
4,2,Unknown,Composite,Shield,,Chile,x,x,s,-20.0,-68.0,,Subduction zone,Holocene,Evidence Credible
5,3,Broken,Composite,Shield,,Chile,x,x,s,,,,Rift,Holocene,x
";

const FIRES: &str = "latitude,longitude,bright_ti4,scan,track,acq_date,acq_time,satellite,confidence,version,bright_ti5,frp,daynight
42.93438,18.87675,367,0.37,0.58,2026-10-04,0001,N20,high,2.0NRT,286.87,7.38,N
-10.5,-55.25,330.1,0.4,0.6,2026-10-04,1742,N20,nominal,2.0NRT,290.1,152.4,D
bad,row,1,1,1,2026-10-04,0001,N20,low,2.0NRT,1,1,N
1.0,2.0,300,0.4,0.6,2026-10-04,1200,N20,low,2.0NRT,290,nan,D
";

#[test]
fn csv_quotes() {
    let rows = csv::rows("a,\"b, c\",\"d \"\"e\"\"\"\r\n\n1,2\n");
    assert_eq!(rows, [vec!["a", "b, c", "d \"e\""], vec!["1", "2"]]);
}

#[test]
fn volcanoes_parse_and_periods() {
    let found = volcanoes::parse(VOLCANOES.as_bytes()).unwrap_or_default();
    assert_eq!(found.len(), 4);
    let periods: Vec<usize> = found.iter().map(|v| volcanoes::period_of(v.last)).collect();
    assert_eq!(periods, [0, 0, 2, 3]);
    assert_eq!(found[3].last, None);
    assert_eq!(found[2].last, Some(-4350.0));
    assert_eq!((found[0].number.as_str(), found[0].name.as_str(), found[0].kind.as_str()), ("300250", "Klyuchevskoy", "Stratovolcano"));
    assert!(volcanoes::size(found[0].elevation) > volcanoes::size(found[1].elevation));
    assert_eq!(volcanoes::size(found[3].elevation), volcanoes::SIZE_MIN);
    assert!(volcanoes::parse(b"a,b\n1,2\n").is_err());
}

#[test]
fn volcano_period_bounds() {
    assert_eq!(volcanoes::period_of(Some(1900.0)), 0);
    assert_eq!(volcanoes::period_of(Some(1899.0)), 1);
    assert_eq!(volcanoes::period_of(Some(1500.0)), 1);
    assert_eq!(volcanoes::period_of(Some(-9000.0)), 2);
    assert_eq!(volcanoes::period_of(Some(f64::NAN)), 3);
}

#[test]
fn fires_rows_and_fields() {
    let data = fires::parse(FIRES);
    assert_eq!(data.len(), 2);
    assert!((data[1].lat + 10.5).abs() < 1e-9);
    assert!((data[1].frp - 152.4).abs() < 1e-9);
    assert_eq!((data[0].confidence.as_str(), data[1].confidence.as_str()), ("high", "nominal"));
    assert_eq!((data[0].night, data[1].night), (true, false));
    let day = data::gibs::Day { year: 2026, month: 10, day: 4 }.seconds();
    assert_eq!(data[1].time, Some(day + 17.0 * 3600.0 + 42.0 * 60.0));
    assert!(fires::parse("").is_empty());
    assert!(fires::parse("a,b\n1,2\n").is_empty());
}

#[test]
fn fire_colors_and_sizes() {
    for (frp, color) in fires::FRP_STOPS {
        assert_eq!(fires::color(frp), color);
    }
    assert_eq!(fires::color(5000.0), fires::FRP_STOPS[3].1);
    assert_eq!(fires::color(0.1), fires::FRP_STOPS[0].1);
    let sizes = [0.5, 1.0, 10.0, 100.0, 1e6].map(fires::size);
    assert_eq!(sizes[0], fires::MIN_SIZE);
    assert!(sizes.windows(2).all(|w| w[1] >= w[0]));
    assert_eq!(sizes[4], fires::MAX_SIZE);
}
