// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Ядро глобуса, слой L0. Эллипсоид тела, координаты ECEF, сетка
//! тайлов и камера.
//!
//! Перенос ядра модуля PlanetX для QGIS (`planetx/core`) на Rust.
//! Слой не зависит ни от интерфейса, ни от видеокарты и собирается
//! под `wasm32-unknown-unknown`.

pub mod camera;
pub mod ellipsoid;
pub mod flight;
pub mod lod;
pub mod navigation;
pub mod tiling;

pub use glam::{DMat3, DMat4, DVec2, DVec3, DVec4, Mat4, Vec2, Vec3, Vec4};
