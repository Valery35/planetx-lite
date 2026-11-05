// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! RustPlanetX - глобус PlanetX для RustGIS, как модуль PlanetX для QGIS,
//! `doc/EDITIONS.md`. Общий с PlanetX Lite запуск и ключи командной
//! строки - `desktop`. Связь с открытым RustGIS есть всегда, её
//! ретранслятор MCP включается в RustGIS. Расширение RustGIS запускает
//! `rustplanetx rustgis-plugin`.
//!
//! Сборка release под Windows - программа окна без консоли.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    desktop::main(ui::Edition::RustPlanetX)
}
