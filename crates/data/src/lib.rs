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
//! - [`geology`] - геологическая карта Macrostrat, перенос `core/geology.py`;
//! - [`gibs`] - темы NASA GIBS по датам, перенос `core/themes.py`;
//! - [`quakes`] - землетрясения USGS, перенос `core/quakes.py`;
//! - [`bathymetry`] - своя батиметрия, перенос `core/bathymetry.py`;
//! - [`contours`] - горизонтали рельефа и изолинии, перенос `core/contours.py`;
//! - [`linesets`], [`faults`], [`plates`] - разломы, границы плит
//!   и кабели линиями наложения;
//! - [`csv`] - таблица CSV;
//! - [`volcanoes`] и [`fires`] - вулканы GVP и пожары FIRMS;
//! - [`plants`] и [`deposits`] - электростанции WRI и месторождения USGS;
//! - [`slope`] - уклон и экспозиция, перенос `core/slope.py`;
//! - [`rastermaps`] - карты картинками тайлов поверх снимка, перенос
//!   `core/fields.py` и `core/tilemaps.py` модуля;
//! - [`routing`] - маршрут по дорогам векторных тайлов и сервис OSRM,
//!   перенос `core/routing.py` модуля;
//! - [`drillholes`] - скважины из таблиц устьев, интервалов
//!   и инклинометрии, перенос `core/drillholes.py` модуля;
//! - [`subsurface`] - сетки подземного режима, стволы, горизонты
//!   и разрезы, перенос `core/subsurface.py` модуля;
//! - [`crust`] - кора CRUST1.0 из архива модели, перенос `core/crust.py`
//!   модуля;
//! - [`tomography`] - томография мантии SEISGLOB2, перенос
//!   `core/tomography.py` модуля;
//! - [`slabs`] - погружающиеся плиты Slab2 на разрезах, перенос
//!   `core/slabs.py` модуля;
//! - [`cutaway`] - разрез Земли, сектор и оболочки PREM на гранях, перенос
//!   `core/cutaway.py` модуля.

pub mod bathymetry;
pub mod cloud;
pub mod contours;
pub mod crust;
pub mod csv;
pub mod cutaway;
pub mod dem;
pub mod deposits;
pub mod drillholes;
pub mod faults;
pub mod fires;
pub mod geocode;
pub mod geology;
pub mod geomag;
pub mod gibs;
pub mod grid;
pub mod layers;
pub mod linesets;
pub mod lines;
pub mod mvt;
mod npz;
pub mod places;
pub mod plants;
pub mod platemotion;
pub mod plates;
pub mod quakes;
pub mod rastermaps;
pub mod routing;
pub mod slabs;
pub mod slope;
pub mod stars;
pub mod subsurface;
pub mod tomography;
pub mod volcanoes;
