// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Интерфейс, слой L6 - один код для программы и страницы.
//!
//! Программа и страница дают загрузчик тайлов своей платформы и место
//! для отчёта самопроверки, остальное общее.

pub mod app;
pub mod basemap;
pub mod check;
pub mod credits;
pub mod i18n;
pub mod input;
pub mod labels;
pub mod search;

pub use app::{LiteApp, Loaders, Report, ShotSink};
pub use check::Scenario;
