// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Данные PlanetX Lite, слой L1.
//!
//! - [`deepsky`] - объекты глубокого неба OpenNGC, перенос `core/deepsky.py`;
//! - [`ephemeris`] - Солнце, Луна и планеты на небе Земли, перенос `core/ephemeris.py`;
//! - [`hips`] - сетка HEALPix и обзоры неба HiPS, перенос `core/hips.py`;
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
//! - [`events`] - снимки чрезвычайных событий Vantor и Planet, каталоги
//!   STAC, перенос `core/events.py`;
//! - [`quakes`] - землетрясения USGS, перенос `core/quakes.py`;
//! - [`bathymetry`] - своя батиметрия, перенос `core/bathymetry.py`;
//! - [`contours`] - горизонтали рельефа и изолинии, перенос `core/contours.py`;
//! - [`linesets`], [`faults`], [`plates`] - разломы, границы плит
//!   и кабели линиями наложения;
//! - [`csv`] - таблица CSV;
//! - [`skydata`] - созвездия и светила вида неба, перенос `core/skydata.py`;
//! - [`skyview`] - взгляд из центра небесной сферы, перенос `core/skyview.py`;
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
//!   и тонкая пластина;
//! - [`topo_surface`], [`hydro_fill`], [`topo_flow`], [`topo_gauge`] -
//!   морфометрия, заполнение понижений, сток D8 и отчёт по створу,
//!   перенос ядра модуля из Isoliner;
//! - [`hydro`] - затопление, линии стока, водосборы и реки у точки,
//!   перенос `core/hydro.py`;
//! - [`relief`] - уклон, экспозиция, вершины и ямы квадрата, перенос
//!   `core/relief.py`;
//! - [`track`] - растущие треки объектов по времени, перенос `core/track.py`;
//! - [`series`] - ряд снимков участка по датам, перенос `core/series.py`;
//! - [`trend`] - карта тренда ряда снимков, наклон Тейла-Сена и тест
//!   Манна-Кендалла, перенос `core/trend.py`.

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
pub mod deepsky;
pub mod dem;
pub mod deposits;
pub mod drillholes;
pub mod ephemeris;
pub mod etomesto;
pub mod events;
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
pub mod hips;
pub mod hydro;
pub mod hydro_fill;
pub mod infra;
pub mod insolation;
pub mod iscgem;
pub mod layers;
pub mod lidarnet;
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
pub mod pmtiles;
pub mod quakes;
pub mod rastermaps;
pub mod relief;
pub mod routing;
pub mod rumsey;
pub mod satellites;
pub mod section;
pub mod sentinel;
pub mod series;
pub mod sgp4;
pub mod sheets;
pub mod skydata;
pub mod skyview;
pub mod slabs;
pub mod slope;
pub mod stars;
pub mod subsurface;
pub mod tomography;
pub mod topo_flow;
pub mod topo_gauge;
pub mod topo_surface;
pub mod track;
pub mod trains;
pub mod trend;
pub mod vessels;
pub mod viewshed;
pub mod volcanoes;
pub mod warp;
pub mod wikipedia;
