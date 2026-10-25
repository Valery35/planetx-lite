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
//! - [`section`] - разрез вниз вдоль линии, перенос `core/section.py`
//!   модуля;
//! - [`slabs`] - погружающиеся плиты Slab2 на разрезах, перенос
//!   `core/slabs.py` модуля;
//! - [`cutaway`] - разрез Земли, сектор и оболочки PREM на гранях, перенос
//!   `core/cutaway.py` модуля;
//! - [`flow`] - струйки ветра и течений, заливка скорости течений
//!   и поле течений ERDDAP, перенос `core/flow.py` модуля;
//! - [`netcdf`] - чтение netCDF-3 (CDF1 и CDF2) без GDAL;
//! - [`cog`] - окна Cloud Optimized GeoTIFF по кускам файла без GDAL,
//!   замена `/vsicurl/` модуля для каналов космоснимков;
//! - [`warp`] - привязка изображения по опорным точкам без GDAL, полином
//!   и тонкая пластина.

pub mod areas;
pub mod aircraft;
pub mod bathymetry;
pub mod buildings;
pub mod cadastre;
pub mod cloud;
pub mod clouds;
pub mod cog;
pub mod contours;
pub mod crust;
pub mod csv;
pub mod cutaway;
pub mod dem;
pub mod deposits;
pub mod drillholes;
pub mod etomesto;
pub mod faults;
pub mod fires;
pub mod flightsim;
pub mod flow;
pub mod geocode;
pub mod geoid;
pub mod geology;
pub mod geomag;
pub mod gibs;
pub mod grid;
pub mod infra;
pub mod insolation;
pub mod iscgem;
pub mod layers;
pub mod linesets;
pub mod lines;
pub mod menulinks;
pub mod mining;
pub mod movers;
pub mod mvt;
pub mod netcdf;
mod npz;
pub mod paleo;
pub mod places;
pub mod plants;
pub mod platemotion;
pub mod plates;
pub mod quakes;
pub mod rastermaps;
pub mod routing;
pub mod rumsey;
pub mod satellites;
pub mod section;
pub mod sentinel;
pub mod sgp4;
pub mod sheets;
pub mod slabs;
pub mod slope;
pub mod stars;
pub mod subsurface;
pub mod tomography;
pub mod trains;
pub mod vessels;
pub mod viewshed;
pub mod volcanoes;
pub mod warp;
pub mod wikipedia;
