// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Снимки OpenAerialMap: адрес поиска, разбор ответа, права слоя. Перенос
//! `tests/test_oam.py` модуля.

use data::oam;
use serde_json::{Value, json};

fn row() -> Value {
    json!({
        "uuid": "https://oin-hotosm-temp.s3.amazonaws.com/a/0/b.tif",
        "title": " Nanga  Kantor ",
        "provider": "Drone Pilots",
        "bbox": [120.1, -8.4, 120.2, -8.3],
        "gsd": 0.04,
        "file_size": 164181646,
        "acquisition_end": "2026-08-29T18:00:00.000Z",
        "properties": {
            "license": "CC-BY-NC 4.0",
            "thumbnail": "https://example.org/b.png",
            "tms": "http://tiles.openaerialmap.org/a/0/b/{z}/{x}/{y}"
        }
    })
}

fn one() -> oam::Image {
    let (mut images, _) = oam::parse(&json!({"results": [row()]}));
    images.remove(0)
}

#[test]
fn newest_first_in_box() {
    let url = oam::search_url([1.0, 2.0, 3.0, 4.0], 2);
    assert!(url.contains("bbox=1.00000%2C2.00000%2C3.00000%2C4.00000"), "{url}");
    assert!(url.contains("&page=2&"));
    assert!(url.contains("order_by=acquisition_end"));
    assert!(url.ends_with("sort=desc"));
}

#[test]
fn box_over_dateline_takes_all_longitudes() {
    let url = oam::search_url([179.0, 2.0, -179.0, 4.0], 1);
    assert!(url.contains("bbox=-180.00000%2C2.00000%2C180.00000"), "{url}");
}

#[test]
fn own_license_of_image_is_kept() {
    let (images, found) = oam::parse(&json!({"meta": {"license": "CC-BY 4.0", "found": 8}, "results": [row()]}));
    assert_eq!(found, 8);
    let image = &images[0];
    assert_eq!(image.license, "CC BY-NC 4.0");
    assert_eq!(image.title, "Nanga Kantor");
    assert_eq!(image.day, "2026-08-29");
    assert_eq!(image.bbox, [120.1, -8.4, 120.2, -8.3]);
}

#[test]
fn mistyped_year_gives_no_day() {
    assert_eq!(oam::day_of("0206-08-07T00:50:39.000Z"), "");
    assert_eq!(oam::day_of(""), "");
}

#[test]
fn row_without_https_file_is_skipped() {
    let mut bad = row();
    bad["uuid"] = json!("http://example.org/b.tif");
    assert!(oam::parse(&json!({"results": [bad, "x"]})).0.is_empty());
}

#[test]
fn hostile_answer_gives_nothing() {
    for answer in [json!(null), json!({"results": 5}), json!({"results": [{"uuid": "https://a", "bbox": [1, 2]}]})] {
        assert_eq!(oam::parse(&answer), (Vec::new(), 0));
    }
}

#[test]
fn rights_name_provider_and_license() {
    let image = one();
    let text = oam::rights(&image);
    assert!(text.contains("Drone Pilots") && text.contains("CC BY-NC 4.0") && text.contains("Open Imagery Network"));
    assert!(oam::is_layer(&image.tiles));
}

#[test]
fn old_tile_address_goes_over_https() {
    assert!(one().tiles.starts_with("https://tiles."));
}

#[test]
fn level_reaches_image_pixel() {
    // 4 см - предел.
    assert_eq!(oam::max_level(&one()), 22);
    let coarse = oam::Image { gsd: 1.0, bbox: [0.0, -1.0, 1.0, 1.0], ..one() };
    // 0.6 м на пиксель уровня 18.
    assert_eq!(oam::max_level(&coarse), 18);
}
