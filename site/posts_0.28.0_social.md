# PlanetX Lite 0.28.0 - тексты для площадок

## RU - Telegram, ВК, Facebook

PlanetX Lite 0.28.0 - аддины. Модуль WebAssembly в песочнице добавляет свою команду в меню слоя «Моих слоёв».

Окно «Аддины» ставит, обновляет и удаляет аддины каталога planetx-lite-addins. Пакет ставится, только если его SHA-256 совпадает с каталогом. Перед установкой видны автор, лицензия и права. У аддина нет файлов, сети и часов - он получает объекты слоя и отдаёт новый слой. Первый аддин - «Буфер» вокруг точки, линии и полигона, верный у полюсов и через 180-й меридиан.

Аддин пишется на Rust с крейтом addin-api, окно параметров Lite рисует сама. Руководство с готовым примером, тестами и публикацией в каталоге - https://github.com/Valery35/planetx-lite/blob/main/addins/README.ru.md

Программа для Windows, Linux и macOS и страница в браузере - https://valery35.github.io/planetx-lite/
Выпуск - https://github.com/Valery35/planetx-lite/releases/tag/v0.28.0

## EN - OSGeo, LinkedIn

PlanetX Lite 0.28.0 - add-ins. A WebAssembly module in a sandbox adds its own command to the menu of a layer of My layers.

The Add-ins window installs, updates and removes add-ins of the planetx-lite-addins catalogue. A package is installed only when its SHA-256 equals the catalogue. The author, the license and the rights are shown before installation. An add-in has no files, no network and no clock - it receives the objects of a layer and gives a new layer. The first add-in is Buffer around a point, a line or a polygon, right at the poles and across the 180th meridian.

An add-in is written in Rust with the addin-api crate, Lite draws the window of parameters itself. The guide with a complete example, tests and publication in the catalogue - https://github.com/Valery35/planetx-lite/blob/main/addins/README.md

Desktop program for Windows, Linux and macOS and the web page - https://valery35.github.io/planetx-lite/
Release - https://github.com/Valery35/planetx-lite/releases/tag/v0.28.0

#GIS #RustLang #WebAssembly #OpenSource #QGIS

## X

PlanetX Lite 0.28.0 - add-ins for a free 3D globe in Rust. A WebAssembly module in a sandbox adds a command to a layer menu, the same add-in runs on desktop and in the browser. Guide: https://github.com/Valery35/planetx-lite/blob/main/addins/README.md #GIS #RustLang #WebAssembly

## Mastodon

PlanetX Lite 0.28.0 - a free 3D globe in Rust and wgpu for Windows, Linux, macOS and the browser - gets add-ins.

An add-in is a WebAssembly module in a wasmi sandbox, without files, network or clock. It adds a command to the menu of a layer, receives the objects and gives a new layer. A catalogue with SHA-256 checks, the first add-in is Buffer.

Write your own: https://github.com/Valery35/planetx-lite/blob/main/addins/README.md

#GIS #RustLang #WebAssembly #OpenData

## Reddit r/rust (публикует автор, редактор в режиме Markdown, флер Project)

Заголовок: PlanetX Lite 0.28.0 - WebAssembly add-ins in a wasmi sandbox, the same module on desktop and in the browser

PlanetX Lite is a free (Apache 2.0) 3D globe written in Rust with egui and wgpu. The same code runs as a desktop program and as a web page through WebAssembly. Version 0.28.0 adds add-ins.

- **The module has no imports.** No WASI, no host functions - the host writes a request into the memory of the module and reads the answer. Five exports: `pxa_abi`, `pxa_manifest`, `pxa_input`, `pxa_command`, `pxa_output`.
- **wasmi in both places.** The desktop program and the page run the module in the same interpreter with fuel and a memory limit, so an add-in behaves the same way everywhere. A test checks a digest of the result.
- **A hostile-input format.** Requests and results are a small binary format of the `addin-api` crate, the readers trust no length or count. A test flips every byte of a request.
- **Packages and a catalogue.** A `.pxaddin` is a ZIP with `addin.json` and the module. The module must report the same id and version and may not ask for more rights than the package states. The catalogue is a GitHub repository with a workflow that checks SHA-256 and publishes `registry.json` on Pages.
- **The first add-in** is a buffer built with `geo` in a local azimuthal equidistant projection, so it is right at the poles and across the antimeridian. 563 KB of WebAssembly.

A guide with a complete add-in in about 90 lines, a sandbox test and the byte format: https://github.com/Valery35/planetx-lite/blob/main/addins/README.md

- Web page: https://valery35.github.io/planetx-lite/
- Release: https://github.com/Valery35/planetx-lite/releases/tag/v0.28.0
- Catalogue: https://github.com/Valery35/planetx-lite-addins

## Reddit r/gis (публикует автор)

Заголовок: PlanetX Lite 0.28.0 - a free 3D globe gets add-ins, the first one is a geodesically right buffer

PlanetX Lite 0.28.0 adds add-ins - WebAssembly modules in a sandbox that add commands to the menu of a layer. A command takes the selected objects of a GeoPackage, Shapefile or GeoJSON layer and gives a new layer. The first add-in of the catalogue is Buffer, right at the poles and across the 180th meridian. The same add-in works in the desktop program and on the web page. A guide shows how to write one in Rust.

Release: https://github.com/Valery35/planetx-lite/releases/tag/v0.28.0
Guide: https://github.com/Valery35/planetx-lite/blob/main/addins/README.md
