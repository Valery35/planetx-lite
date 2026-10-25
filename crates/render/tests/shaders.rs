// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Шейдеры разбираются и проходят проверку naga без видеокарты, как их
//! собирает `render`: у глобуса и неба впереди стоит air.wgsl. Ошибка
//! в шейдере иначе видна только при запуске программы.

#![allow(clippy::panic)]

const SHADERS: [(&str, &str); 13] = [
    ("globe", concat!(include_str!("../src/air.wgsl"), include_str!("../src/globe.wgsl"))),
    ("sky", concat!(include_str!("../src/air.wgsl"), include_str!("../src/sky.wgsl"))),
    ("holes", include_str!("../src/holes.wgsl")),
    ("hips", include_str!("../src/hips.wgsl")),
    ("features", include_str!("../src/features.wgsl")),
    ("models", include_str!("../src/models.wgsl")),
    ("points", include_str!("../src/points.wgsl")),
    ("skylines", include_str!("../src/skylines.wgsl")),
    ("skymap", include_str!("../src/skymap.wgsl")),
    ("pictures", include_str!("../src/pictures.wgsl")),
    ("stars", include_str!("../src/stars.wgsl")),
    ("subsurface", include_str!("../src/subsurface.wgsl")),
    ("air", include_str!("../src/air.wgsl")),
];

#[test]
fn shaders_validate() {
    for (name, source) in SHADERS {
        let module = match naga::front::wgsl::parse_str(source) {
            Ok(m) => m,
            Err(e) => panic!("{name}: {}", e.emit_to_string(source)),
        };
        let mut validator = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::default());
        if let Err(e) = validator.validate(&module) {
            panic!("{name}: {e:?}");
        }
    }
}
