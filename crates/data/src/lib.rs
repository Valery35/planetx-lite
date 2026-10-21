// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Данные PlanetX Lite, слой L1.
//!
//! - [`mvt`] - разбор векторного тайла Mapbox Vector Tile 2;
//! - [`places`] - пункты для надписей на глобусе, перенос
//!   `core/places.py` модуля;
//! - [`geocode`] - поиск места через Nominatim, перенос
//!   `core/geocode.py` модуля.

pub mod geocode;
pub mod mvt;
pub mod places;
