// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Интерфейс, слой L6 - один код для программы и страницы.
//!
//! Программа и страница дают загрузчик тайлов своей платформы и место
//! для отчёта самопроверки, остальное общее.

pub mod app;
pub mod basemap;
pub mod card;
pub mod check;
pub mod commands;
pub mod credits;
pub mod demos;
pub mod export;
pub mod fonts;
pub mod gpkg;
pub mod grid;
pub mod i18n;
pub mod input;
pub mod labels;
pub mod marks;
pub mod models;
pub mod navpad;
pub mod pictures;
pub mod places;
pub mod profile;
pub mod relief;
pub mod ruler;
pub mod search;
pub mod settings;
pub mod share;
pub mod theme;
pub mod timeline;
pub mod tool;
pub mod tours;

pub use app::{LiteApp, Loaders, Report, ShotSink};
pub use check::Scenario;
