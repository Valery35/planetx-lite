// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сцены и метки PlanetX Lite, слой L2.
//!
//! - [`when`] - время меток, перенос `core/when.py` модуля;
//! - [`lookat`] - вид метки, перенос `core/lookat.py`;
//! - [`placetree`] - дерево меток, перенос `core/placetree.py`.

pub mod lookat;
pub mod placetree;
pub mod when;
