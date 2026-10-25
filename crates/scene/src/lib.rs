// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сцены и метки PlanetX Lite, слой L2.
//!
//! - [`when`] - время меток, перенос `core/when.py` модуля;
//! - [`clock`] - ход момента шкалы времени, перенос `core/clock.py`;
//! - [`lookat`] - вид метки, перенос `core/lookat.py`;
//! - [`placetree`] - дерево меток, перенос `core/placetree.py`;
//! - [`planets`] - тела глобуса, их снимки, высоты и воздух, перенос
//!   `core/planets.py`;
//! - [`icons`] - значки меток, перенос `core/icons.py`;
//! - [`region`] - Region KML, скрытие дальше расстояния, перенос `core/region.py`;
//! - [`measure`] - единицы, профиль высот, длина по рельефу, перенос `core/measure.py`;
//! - [`geodesic`] - длина, курс, площадь на эллипсоиде, как QgsDistanceArea модуля;
//! - [`opacity`] - непрозрачность выделенных меток, перенос `core/opacity.py`;
//! - [`overlays`] - наложения картинок и рамка 3D-модели, перенос `core/overlays.py`;
//! - [`models`] - место 3D-модели, пакет её файлов и чтение сеток, перенос `core/models.py`;
//! - [`collada`], [`gltf`], [`obj`] - сетки Collada, glTF и OBJ, перенос `core/collada.py`,
//!   `core/gltf.py` и `core/obj.py`;
//! - [`animation`] - анимация узлов моделей, перенос `core/animation.py`;
//! - [`balloon`] - шаблон окна метки BalloonStyle, перенос `core/balloon.py`;
//! - [`editing`] - правка вершин на экране, перенос `core/editing.py`;
//! - [`graticule`] - координатная сетка, перенос `core/graticule.py`;
//! - [`sun`] - положение солнца и свет, перенос `core/sun.py`;
//! - [`searchbar`] - подсказки строки «Поиск», перенос `core/searchbar.py`;
//! - [`coords`] - форматы координат, перенос `core/coords.py`;
//! - [`features`] - свои объекты глобуса, сгущение и заливка, перенос `core/features.py`;
//! - [`tour`] - тур по остановкам и записанный тур, перенос `core/tour.py`;
//! - [`track`] - поза модели на gx:Track, перенос `model_pose` из `core/track.py`;
//! - [`xml`] - разбор XML в лёгкое дерево без сущностей DTD;
//! - [`kml`] - чтение и запись KML и KMZ, перенос `core/kml.py`;
//! - [`scene`] - файл сцены `.planetx`, перенос `core/scene.py`.

pub mod animation;
pub mod balloon;
pub mod clock;
pub mod collada;
pub mod coords;
pub mod editing;
pub mod features;
pub mod geodesic;
pub mod gltf;
pub mod graticule;
pub mod icons;
pub mod kml;
pub mod lookat;
pub mod measure;
pub mod models;
pub mod obj;
pub mod opacity;
pub mod overlays;
pub mod placetree;
pub mod planets;
pub mod region;
pub mod scene;
pub mod searchbar;
pub mod sun;
pub mod tour;
pub mod track;
pub mod when;
pub mod wms;
pub mod xml;

mod util;

/// Цвет RGBA, каналы 0-255.
pub type Rgba = [u8; 4];
