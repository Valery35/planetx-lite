// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Данные PlanetX Lite, слой L1.
//!
//! - [`lines`] - линии векторной основы в картинку тайла, стиль
//!   `net/overlay.py` модуля;
//! - [`mvt`] - разбор векторного тайла Mapbox Vector Tile 2;
//! - [`places`] - пункты для надписей на глобусе, перенос
//!   `core/places.py` модуля;
//! - [`geocode`] - поиск места через Nominatim, перенос
//!   `core/geocode.py` модуля;
//! - [`grid`] - свой растр GeoTIFF или COG, через terrio;
//! - [`dem`] - растр высот для врезки своего рельефа;
//! - [`layers`] - слои GeoPackage, через terrio;
//! - [`gibs`] - темы NASA GIBS по датам, перенос `core/themes.py`;
//! - [`rastermaps`] - карты картинками тайлов поверх снимка, перенос
//!   `core/fields.py` и `core/tilemaps.py` модуля.

pub mod cloud;
pub mod dem;
pub mod geocode;
pub mod gibs;
pub mod grid;
pub mod layers;
pub mod lines;
pub mod mvt;
pub mod places;
pub mod rastermaps;
