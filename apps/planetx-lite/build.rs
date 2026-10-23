// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Под Windows в exe встраиваются значок и сведения о программе:
//! название, версия, правообладатель. Их показывает Проводник, их же
//! сверяет подпись кода.

fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=../../packaging/planetx.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../packaging/planetx.ico")
            .set("ProductName", "PlanetX Lite")
            .set("FileDescription", "PlanetX Lite")
            .set("CompanyName", "Inform++ LLC")
            .set("LegalCopyright", "Copyright (C) 2026 Inform++ LLC. Apache-2.0")
            .set("OriginalFilename", "planetx-lite.exe");
        // Без ресурсов программа работает, сборка не останавливается.
        if let Err(e) = res.compile() {
            println!("cargo:warning=ресурсы exe не собраны: {e}");
        }
    }
}
