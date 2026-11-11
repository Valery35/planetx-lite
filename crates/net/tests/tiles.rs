// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Номер тайла Bing и снимки в эллиптическом Меркаторе, перенос
//! `tests/test_basemap.py` модуля.

#![allow(clippy::expect_used)]

use net::{Image, elliptic_fetch, elliptic_rows, elliptic_tile, quadkey, template_url};

#[test]
fn quadkey_as_bing_tiles() {
    // Пример из описания системы тайлов Bing Maps.
    assert_eq!(quadkey((3, 3, 5)), "213");
    assert_eq!(template_url("q/{q}.jpg", (3, 3, 5)), "q/213.jpg");
    assert_eq!(quadkey((0, 0, 0)), "");
    assert!(template_url("https://ecn.t1.tiles.virtualearth.net/tiles/a{q}.jpeg?g=14000", (3, 2, 2)).contains("/a030.jpeg"));
}

#[test]
fn elliptic_rows_at_perm_shift_south() {
    // Тайл z=12 над Пермью: в EPSG:3395 он ниже на 3-4 тайла.
    let (first, last, at) = elliptic_rows(12, 1197, 256);
    assert!([1200, 1201].contains(&first), "{first}");
    assert!(last - first <= 1);
    assert!(at.windows(2).all(|w| w[1] > w[0]));
    // У экватора проекции совпадают.
    let (first, _, at) = elliptic_rows(2, 2, 256);
    assert_eq!(first, 2);
    assert!(at[0].abs() < 1.0, "{}", at[0]);
}

#[test]
fn elliptic_tile_takes_rows_from_both_parts() {
    let (z, y) = (10, 299);
    let (first, last, at) = elliptic_rows(z, y, 256);
    let images: Vec<Image> = (first..=last)
        .map(|row| {
            let rgba = (0..256u32).flat_map(|r| std::iter::repeat_n([(r / 2 + (row - first) * 128) as u8, 0, 0, 255], 256).flatten()).collect();
            Image { width: 256, height: 256, rgba }
        })
        .collect();
    let tile = elliptic_tile(z, y, &images).expect("тайл");
    assert_eq!((tile.width, tile.height, tile.rgba.len()), (256, 256, 256 * 256 * 4));
    let top = (images.len() * 256 - 1) as f64;
    for (j, a) in at.iter().enumerate() {
        let expected = a.clamp(0.0, top) / 2.0;
        let got = f64::from(tile.rgba[j * 256 * 4]);
        assert!((got - expected).abs() < 1.01, "строка {j}: {got} против {expected}");
    }
}

/// Живая проверка: тайл z=14 у Перми из снимков Яндекса с пересчётом
/// и без него против тайла Google в Web Mercator. Яркость - серым,
/// сходство - корреляция Пирсона.
/// cargo test -p net --test tiles live_yandex_elliptic -- --ignored --nocapture
#[test]
#[ignore]
#[cfg(not(target_arch = "wasm32"))]
fn live_yandex_elliptic() {
    #![allow(clippy::unwrap_used)]
    use net::Request;
    let get = |url: &str| -> net::TileBytes {
        let ask = net::native::Requests::with_limit(std::sync::Arc::new(|| {}), 4 << 20);
        ask.start(url);
        let start = std::time::Instant::now();
        loop {
            if let Some(answer) = ask.take() {
                return answer.map_err(Some);
            }
            assert!(start.elapsed().as_secs() < 30, "нет ответа {url}");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    };
    let grey = |img: &Image| -> Vec<f64> { img.rgba.chunks(4).map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2])).collect() };
    let corr = |a: &[f64], b: &[f64]| {
        let n = a.len() as f64;
        let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
        let cov: f64 = a.iter().zip(b).map(|(x, y)| (x - ma) * (y - mb)).sum();
        let (va, vb): (f64, f64) = (a.iter().map(|x| (x - ma).powi(2)).sum(), b.iter().map(|y| (y - mb).powi(2)).sum());
        cov / (va * vb).sqrt()
    };
    // Тайл с центром Перми, 58.01° с. ш., 56.25° в. д.
    let key = (14u32, 10_752u32, 4_933u32);
    let google = net::decode_tile(&get(&template_url("https://mt1.google.com/vt/lyrs=s&x={x}&y={y}&z={z}", key)).unwrap()).unwrap();
    let template = "https://core-sat.maps.yandex.net/tiles?l=sat&x={x}&y={ey}&z={z}&lang=ru_RU";
    let fixed = net::decode_tile(&elliptic_fetch(template, key, &get).unwrap()).unwrap();
    let raw = net::decode_tile(&get(&template_url(&template.replace("{ey}", "{y}"), key)).unwrap()).unwrap();
    let (with, without) = (corr(&grey(&google), &grey(&fixed)), corr(&grey(&google), &grey(&raw)));
    eprintln!("сходство с Google: с пересчётом {with:.3}, без пересчёта {without:.3}");
    assert!(with > without + 0.1, "{with} против {without}");
}

#[test]
fn elliptic_fetch_asks_one_or_two_tiles_of_column() {
    let asked = std::cell::RefCell::new(Vec::new());
    let png = {
        let img = image::RgbaImage::from_pixel(256, 256, image::Rgba([10, 20, 30, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).expect("png");
        out.into_inner()
    };
    let get = |url: &str| -> net::TileBytes {
        asked.borrow_mut().push(url.to_string());
        Ok(png.clone())
    };
    let bytes = elliptic_fetch("https://t/x={x}&y={ey}&z={z}", (12, 2900, 1197), &get).expect("тайл");
    let tile = net::decode_tile(&bytes).expect("картинка");
    assert_eq!(tile.rgba[..4], [10, 20, 30, 255]);
    let asked = asked.borrow();
    assert!(!asked.is_empty() && asked.len() <= 2, "{asked:?}");
    assert!(asked.iter().all(|u| u.starts_with("https://t/x=2900&y=120") && u.ends_with("&z=12")), "{asked:?}");
}
