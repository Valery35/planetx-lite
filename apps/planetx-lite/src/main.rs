// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Настольная программа PlanetX Lite. Запуск и ключи командной строки -
//! общие с RustPlanetX, крейт `desktop`: `--check`, `--pose`, `--shot`,
//! файлы и ссылки, `mcp`, `--mcp`, `--rustgis`, `rustgis-plugin`.
//!
//! Сборка release под Windows - программа окна без консоли. Отчёт
//! самопроверки тогда виден, когда вывод перенаправлен в файл.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    desktop::main(ui::Edition::Lite)
}
