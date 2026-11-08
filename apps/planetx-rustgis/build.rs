// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Под Windows в exe встраиваются значок и сведения о программе:
//! название, версия, правообладатель. Их показывает Проводник, их же
//! сверяет подпись кода.
//!
//! Стек главного потока под Windows - 8 МБ, как у Linux и macOS. По
//! умолчанию у Windows 1 МБ, его не хватало сборке debug: замыкание
//! запуска окна держит копии `LiteApp` (около 60 КБ) от каждого
//! `with_…`, окно падало с переполнением стека.

/// Стек главного потока, байт.
const STACK: u32 = 8 << 20;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
            Ok("msvc") => println!("cargo:rustc-link-arg-bins=/STACK:{STACK}"),
            Ok("gnu") => println!("cargo:rustc-link-arg-bins=-Wl,--stack,{STACK}"),
            _ => {}
        }
    }
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=../../packaging/planetx.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../packaging/planetx.ico")
            .set("ProductName", "RustPlanetX")
            .set("FileDescription", "RustPlanetX")
            .set("CompanyName", "Inform++ LLC")
            .set("LegalCopyright", "Copyright (C) 2026 Inform++ LLC. Apache-2.0")
            .set("OriginalFilename", "rustplanetx.exe");
        // Без ресурсов программа работает, сборка не останавливается.
        if let Err(e) = res.compile() {
            println!("cargo:warning=ресурсы exe не собраны: {e}");
        }
    }
}
