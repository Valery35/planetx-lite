// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Темы NASA GIBS: ряд дат, выбор дня, прозрачность. Перенос
//! `tests/test_themes.py` модуля.

use data::gibs::{self, Day};

/// Ответ DescribeDomains SMAP за 5 октября 2026 года, с пропусками.
const SMAP: &str = "<Domains><DimensionDomain><ows:Identifier>time</ows:Identifier><Domain>2026-08-01/2026-09-02/P1D,2026-09-04/2026-09-08/P1D,2026-09-10/2026-09-19/P1D,2026-09-21/2026-09-29/P1D,2026-10-01/2026-10-01/P1D</Domain><Size>5</Size></DimensionDomain></Domains>";
const MONTHLY: &str = "<Domain>1980-01-01/2026-07-01/P1M</Domain>";
const SIXTEEN: &str = "<Domain>2000-03-05/2026-08-29/P16D</Domain>";

fn stamp(year: i32, month: u32, day: u32) -> f64 {
    Day { year, month, day }.seconds()
}

#[test]
fn calendar_round_trip() {
    for (y, m, d) in [(1970, 1, 1), (2000, 2, 29), (2026, 10, 24), (1980, 12, 31)] {
        let day = Day { year: y, month: m, day: d };
        assert_eq!(Day::from_seconds(day.seconds() + 3600.0), day);
    }
    assert_eq!(stamp(1970, 1, 2), 86400.0);
    assert_eq!(Day::parse("2026-02-30"), None);
}

#[test]
fn parse() {
    let intervals = gibs::parse_domains(SMAP);
    assert_eq!(intervals.len(), 5);
    assert_eq!((intervals[0].2, intervals[0].3), (1, 'D'));
    assert!(gibs::parse_domains("<Domains/>").is_empty());
    assert_eq!(gibs::pick_day(&[], None), None);
}

#[test]
fn latest_without_moment() {
    assert_eq!(gibs::pick_day(&gibs::parse_domains(SMAP), None).as_deref(), Some("2026-10-01"));
}

#[test]
fn unprocessed_day_is_dropped() {
    // 1 октября в 12:00 UTC этот день ещё не готов: последним остаётся
    // 29 сентября, промежуток 1 октября отброшен.
    let ready = gibs::ready_intervals(&gibs::parse_domains(SMAP), stamp(2026, 10, 1) + 43200.0);
    assert_eq!(gibs::pick_day(&ready, None).as_deref(), Some("2026-09-29"));
    assert_eq!(ready.len(), 4);
    let ready = gibs::ready_intervals(&gibs::parse_domains(SIXTEEN), stamp(2026, 9, 1));
    assert_eq!(gibs::pick_day(&ready, None).as_deref(), Some("2026-08-29"));
}

#[test]
fn step_day_follows_series() {
    let intervals = gibs::parse_domains(SMAP);
    assert_eq!(gibs::step_day(&intervals, "2026-09-02", 1).as_deref(), Some("2026-09-04"));
    assert_eq!(gibs::step_day(&intervals, "2026-09-04", -1).as_deref(), Some("2026-09-02"));
    assert_eq!(gibs::step_day(&intervals, "2026-10-01", 1), None);
    assert_eq!(gibs::step_day(&intervals, "2026-09-03", 1), None);
}

#[test]
fn gap_gives_previous_day() {
    let intervals = gibs::parse_domains(SMAP);
    assert_eq!(gibs::pick_day(&intervals, Some(stamp(2026, 9, 3))).as_deref(), Some("2026-09-02"));
    assert_eq!(gibs::pick_day(&intervals, Some(stamp(2026, 9, 15))).as_deref(), Some("2026-09-15"));
}

#[test]
fn before_and_after_series() {
    let intervals = gibs::parse_domains(SMAP);
    assert_eq!(gibs::pick_day(&intervals, Some(stamp(2020, 1, 1))).as_deref(), Some("2026-08-01"));
    assert_eq!(gibs::pick_day(&intervals, Some(stamp(2027, 1, 1))).as_deref(), Some("2026-10-01"));
}

#[test]
fn monthly() {
    let intervals = gibs::parse_domains(MONTHLY);
    assert_eq!(gibs::pick_day(&intervals, Some(stamp(2010, 5, 20))).as_deref(), Some("2010-05-01"));
    assert_eq!(gibs::pick_day(&intervals, Some(stamp(2026, 9, 20))).as_deref(), Some("2026-07-01"));
}

#[test]
fn sixteen_days() {
    let intervals = gibs::parse_domains(SIXTEEN);
    assert_eq!(gibs::pick_day(&intervals, Some(stamp(2000, 3, 25))).as_deref(), Some("2000-03-21"));
}

#[test]
fn days_skip_gaps() {
    let found = gibs::days(&gibs::parse_domains(SMAP));
    assert_eq!(found.first().map(String::as_str), Some("2026-08-01"));
    assert_eq!(found.last().map(String::as_str), Some("2026-10-01"));
    assert!(!found.iter().any(|d| d == "2026-09-03" || d == "2026-09-30"));
    let monthly = gibs::days(&gibs::parse_domains(MONTHLY));
    assert_eq!(&monthly[..2], ["1980-01-01", "1980-02-01"]);
    assert_eq!(monthly.len(), 46 * 12 + 7);
}

#[test]
fn span() {
    assert_eq!(gibs::span(&gibs::parse_domains(SMAP)), Some((stamp(2026, 8, 1), stamp(2026, 10, 1))));
    assert_eq!(gibs::span(&[]), None);
}

#[test]
fn keys_unique_and_url() {
    let mut keys: Vec<&str> = gibs::THEMES.iter().map(|t| t.key).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), gibs::THEMES.len());
    let rain = gibs::theme_of("rain").map(|t| t.url("2026-10-03")).unwrap_or_default();
    assert!(rain.contains("/IMERG_Precipitation_Rate/default/2026-10-03/GoogleMapsCompatible_Level6/{z}/{y}/{x}.png"), "{rain}");
}

#[test]
fn overlay_clears_no_snow_class() {
    let clear = gibs::theme_of("snow_mass").map(|t| t.clear).unwrap_or(&[]);
    let out = gibs::overlay_rgba(&[82, 98, 106, 255, 89, 94, 111, 255], 1.0, clear).unwrap_or_default();
    assert_eq!(out[3], 0);
    assert_eq!(out[7], 255);
}

#[test]
fn overlay_keeps_no_data_clear() {
    let out = gibs::overlay_rgba(&[200, 100, 50, 255, 9, 9, 9, 0], 0.5, &[]).unwrap_or_default();
    assert_eq!(&out[4..], &[0, 0, 0, 0]);
    assert_eq!(&out[..4], &[100, 50, 25, 128]);
}

/// Цветовая карта по образцу GPM_Precipitation_Rate.
const COLORMAP: &str = r#"<ColorMaps>
<ColorMap title="Classifications"><Entries/>
<Legend type="classification">
<LegendEntry rgb="128,128,128" tooltip="No Data" id="0"/>
<LegendEntry rgb="191,191,191" tooltip="Insignificant" id="1"/>
</Legend></ColorMap>
<ColorMap title="Rain Rate" units="mm/hr"><Entries/>
<Legend type="continuous" minLabel="0.1" maxLabel="&#8805; 53.0">
<LegendEntry rgb="0,118,78" tooltip="0.1" id="2"/>
<LegendEntry rgb="0,148,36" tooltip="0.2" showTick="true" showLabel="true"
 label="0.2" id="3"/>
<LegendEntry rgb="195,228,0" tooltip="1" showTick="true" id="4"/>
<LegendEntry rgb="255,0,0" tooltip="53" id="5"/>
</Legend></ColorMap></ColorMaps>"#;

#[test]
fn colormap() {
    let found = gibs::parse_colormap(COLORMAP).unwrap_or_default();
    assert_eq!(found.kind, "continuous");
    assert_eq!(found.units, "mm/hr");
    assert_eq!(found.colors.len(), 4);
    assert_eq!(found.labels, [(0.375, "0.2".to_string())]);
}

#[test]
fn colormap_without_labels_uses_ends() {
    let data = COLORMAP.replace("showLabel=\"true\"", "");
    let labels = gibs::parse_colormap(&data).unwrap_or_default().labels;
    assert_eq!(labels, [(0.0, "0.1".to_string()), (1.0, "≥ 53.0".to_string())]);
    assert_eq!(gibs::parse_colormap("<ColorMaps/>"), None);
}
