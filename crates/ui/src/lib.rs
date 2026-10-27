// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Интерфейс, слой L6 - один код для программы и страницы.
//!
//! Программа и страница дают загрузчик тайлов своей платформы и место
//! для отчёта самопроверки, остальное общее.

pub mod ai;
pub mod app;
pub mod basemap;
pub mod buildings;
pub mod bathymetry;
pub mod card;
pub mod check;
pub mod cogread;
pub mod cogtiles;
pub mod commands;
pub mod coords_text;
pub mod credits;
pub mod currents;
pub mod cutaway;
pub mod demos;
pub mod dots;
pub mod etomesto;
pub mod events;
pub mod export;
pub mod flight;
pub mod folderprops;
pub mod fonts;
pub mod gpkg;
pub mod geophysics;
pub mod netlinks;
pub mod overlayprops;
pub mod overpass;
pub mod overture;
pub mod placeprops;
pub mod glyphs;
pub mod gps;
pub mod grid;
pub mod gridheights;
pub mod hydrocalc;
pub mod i18n;
pub mod identify;
pub mod globemenu;
pub mod input;
pub mod labels;
pub mod layers;
pub mod legend;
pub mod lidarnet;
pub mod live;
pub mod maps;
#[cfg(not(target_arch = "wasm32"))]
pub mod mcprelay;
pub mod menulinks;
pub mod mapvalue;
pub mod marks;
pub mod mining;
pub mod models;
pub mod navpad;
pub mod pictures;
pub mod places;
pub mod pointcalc;
pub mod polar;
pub mod profile;
pub mod relief;
pub mod reliefcalc;
pub mod route;
pub mod rumsey;
pub mod section;
pub mod sky;
pub mod skysurvey;
pub mod ruler;
pub mod satellites;
pub mod search;
pub mod sentinel;
pub mod series;
pub mod settings;
pub mod share;
pub mod sheets;
pub mod subsurface;
pub mod theme;
pub mod timeline;
pub mod tool;
pub mod tours;
pub mod tracks;
pub mod vector;
pub mod wiki;

pub use app::{LiteApp, Loaders, Report, ShotSink};
pub use check::Scenario;
