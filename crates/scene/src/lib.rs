// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сцены и метки PlanetX Lite, слой L2.
//!
//! - [`when`] - время меток, перенос `core/when.py` модуля;
//! - [`lookat`] - вид метки, перенос `core/lookat.py`;
//! - [`placetree`] - дерево меток, перенос `core/placetree.py`;
//! - [`icons`] - значки меток, перенос `core/icons.py`;
//! - [`region`] - Region KML, скрытие дальше расстояния, перенос `core/region.py`;
//! - [`measure`] - точка по азимуту и расстоянию, перенос части `core/measure.py`;
//! - [`overlays`] - наложения картинок и рамка 3D-модели, перенос `core/overlays.py`;
//! - [`models`] - место 3D-модели и пакет её файлов, перенос части `core/models.py`;
//! - [`features`] - свои объекты глобуса, сгущение и заливка, перенос `core/features.py`;
//! - [`tour`] - тур по остановкам и записанный тур, перенос `core/tour.py`;
//! - [`xml`] - разбор XML в лёгкое дерево без сущностей DTD;
//! - [`kml`] - чтение и запись KML и KMZ, перенос `core/kml.py`;
//! - [`scene`] - файл сцены `.planetx`, перенос `core/scene.py`.

pub mod features;
pub mod icons;
pub mod kml;
pub mod lookat;
pub mod measure;
pub mod models;
pub mod overlays;
pub mod placetree;
pub mod region;
pub mod scene;
pub mod tour;
pub mod when;
pub mod xml;

mod util;

/// Цвет RGBA, каналы 0-255.
pub type Rgba = [u8; 4];
