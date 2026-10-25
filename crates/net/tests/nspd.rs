// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Правила узлов: Referer и свой корневой сертификат НСПД.

use net::host_rule;

#[test]
fn rule_matches_only_its_host() {
    assert!(host_rule("https://nspd.gov.ru/api/aeggis/v3/36048/wms?BBOX=1,2,3,4").is_some());
    assert!(host_rule("https://api.nspd.gov.ru/x").is_some());
    assert!(host_rule("https://evilnspd.gov.ru/x").is_none());
    assert!(host_rule("https://example.org/?q=nspd.gov.ru").is_none());
    assert!(host_rule("nspd.gov.ru:443/x").is_some());
}

#[test]
fn root_certificate_is_pem() {
    let Some(rule) = host_rule("https://nspd.gov.ru/") else {
        return;
    };
    let text = String::from_utf8_lossy(rule.root);
    assert!(text.contains("-----BEGIN CERTIFICATE-----"));
}

/// Живой запрос к службе: `cargo test -p net --test nspd -- --ignored`.
#[cfg(not(target_arch = "wasm32"))]
#[test]
#[ignore]
fn live_tile() {
    use net::Request;
    let url = "https://nspd.gov.ru/api/aeggis/v3/36048/wms?SERVICE=WMS&REQUEST=GetMap&VERSION=1.3.0&LAYERS=36048&STYLES=&FORMAT=image/png&TRANSPARENT=true&CRS=EPSG:3857&BBOX=6259281.92,7967526.03,6259893.42,7968137.53&WIDTH=256&HEIGHT=256";
    let r = net::native::Requests::with_limit(std::sync::Arc::new(|| {}), 4 << 20);
    r.start(url);
    for _ in 0..300 {
        if let Some(answer) = r.take() {
            match answer {
                Ok(b) => eprintln!("ok {} bytes, png {}", b.len(), b.starts_with(b"\x89PNG")),
                Err(e) => eprintln!("error {e}"),
            }
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    eprintln!("no answer");
}
