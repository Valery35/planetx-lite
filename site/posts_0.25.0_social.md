# PlanetX Lite 0.25.0 - тексты для площадок

## RU - Telegram, ВК, Facebook

PlanetX Lite 0.25.0 - открытые национальные карты, ещё три наложения маршрутов и температура поверхности, как в модуле PlanetX для QGIS 1.41 и 1.42.

В подложках - TopPlusOpen мира, basemap.de Германии, карты swisstopo и аэрофото SWISSIMAGE Швейцарии, карты и аэрофото PDOK Нидерландов. Национальная карта видна в своей стране, вокруг неё и издалека глобус показывает карту или снимки Esri. В наложениях - маршруты для горного велосипеда, горнолыжные трассы и конные маршруты Waymarked Trails. Строка «Температура поверхности» показывает температуру суши по MODIS и моря по GHRSST MUR из NASA GIBS. Подложки и наложения Lite сверяются с общим каталогом источников линейки, модуль и Lite показывают одни и те же карты.

Программа для Windows, Linux и macOS и страница в браузере - https://valery35.github.io/planetx-lite/
Выпуск - https://github.com/Valery35/planetx-lite/releases/tag/v0.25.0

## EN - OSGeo, LinkedIn

PlanetX Lite 0.25.0 - open national maps, three more trail overlays and the surface temperature, as in the PlanetX plugin for QGIS 1.41 and 1.42.

The base maps add TopPlusOpen of the world, basemap.de of Germany, swisstopo maps and SWISSIMAGE aerial photos of Switzerland, PDOK maps and aerial photos of the Netherlands. A national map is shown inside its country, around it and from afar the globe shows an Esri map or imagery. Map overlays add Waymarked Trails mountain bike routes, ski slopes and riding routes. The Surface temperature row shows the land temperature from MODIS and the sea temperature from GHRSST MUR, NASA GIBS data. The base maps and overlays of Lite are checked against the common source catalog of the PlanetX line, so the plugin and Lite show the same maps.

Desktop program for Windows, Linux and macOS and the web page - https://valery35.github.io/planetx-lite/
Release - https://github.com/Valery35/planetx-lite/releases/tag/v0.25.0

#GIS #OpenData #RustLang #WebAssembly #swisstopo

## X

PlanetX Lite 0.25.0 - a free 3D globe in Rust for desktop and the browser. National maps of Germany, Switzerland and the Netherlands, Waymarked Trails MTB, ski and riding overlays, land and sea surface temperature. https://github.com/Valery35/planetx-lite/releases/tag/v0.25.0 #GIS #RustLang

## Mastodon

PlanetX Lite 0.25.0 - a free 3D globe in Rust and wgpu for Windows, Linux, macOS and the browser.

National base maps - TopPlusOpen, basemap.de, swisstopo with SWISSIMAGE, PDOK - with an Esri map around each country. Waymarked Trails mountain bike, ski and riding overlays. Land and sea surface temperature from NASA GIBS.

https://github.com/Valery35/planetx-lite/releases/tag/v0.25.0

#GIS #OpenData #RustLang #OpenStreetMap

## Reddit r/rust (публикует автор, редактор в режиме Markdown)

Заголовок: PlanetX Lite 0.25.0 - national base maps with a world fallback and NASA surface temperature on a Rust/wgpu 3D globe

PlanetX Lite is a free (Apache 2.0) 3D globe written in Rust with egui and wgpu. The same code runs as a desktop program for Windows, Linux and macOS and as a web page through WebAssembly.

Version 0.25.0:

- **National base maps** - TopPlusOpen, basemap.de, swisstopo, SWISSIMAGE, PDOK. Each is a regional tile source: inside its bounding box and from its level the national tiles, outside them an Esri map, chosen per tile in the network crate.
- **Waymarked Trails overlays** - mountain bike, ski slopes, riding.
- **Surface temperature** - MODIS land and GHRSST MUR sea from NASA GIBS, two tile layers under one catalog row.
- **One source catalog** - a test checks every base map and overlay of Lite against the JSON catalog shared with the QGIS plugin.

- Web page: https://valery35.github.io/planetx-lite/
- Release: https://github.com/Valery35/planetx-lite/releases/tag/v0.25.0
