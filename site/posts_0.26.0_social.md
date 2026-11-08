# PlanetX Lite 0.26.0 - тексты для площадок

## RU - Telegram, ВК, Facebook

PlanetX Lite 0.26.0 - спутниковые данные JAXA Earth по месяцам и слои сцен I3S сервисов ArcGIS по ссылке.

В окне «Добавить данные» - семь строк JAXA Earth. Это температура поверхности суши и моря, вегетационный индекс NDVI, хлорофилл-а в море, осадки GSMaP, влажность почвы и оптическая толща аэрозоля. Каждая строка - среднее за месяц по всему миру на сетке 0.1° со своей шкалой. Месяц задаёт шкала времени. Слой сцены I3S открывается по ссылке на сервис ArcGIS SceneServer из строки поиска. Здания и сетки местности слоёв 3D Object и Integrated Mesh подгружаются по мере приближения, как наборы 3D Tiles.

Программа для Windows, Linux и macOS и страница в браузере - https://valery35.github.io/planetx-lite/
Выпуск - https://github.com/Valery35/planetx-lite/releases/tag/v0.26.0

## EN - OSGeo, LinkedIn

PlanetX Lite 0.26.0 - JAXA Earth satellite data by month and I3S scene layers of ArcGIS by a link.

The Add data window gets seven JAXA Earth rows. They are land and sea surface temperature, the NDVI vegetation index, chlorophyll-a in the sea, GSMaP precipitation, soil moisture and aerosol optical depth. Each row is a monthly mean for the whole world on a 0.1° grid with its own scale. The month is set by the time slider. An I3S scene layer opens by a link to an ArcGIS SceneServer service from the search line. Buildings and meshes of 3D Object and Integrated Mesh layers load as the camera comes closer, like 3D Tiles sets.

Desktop program for Windows, Linux and macOS and the web page - https://valery35.github.io/planetx-lite/
Release - https://github.com/Valery35/planetx-lite/releases/tag/v0.26.0

#GIS #RemoteSensing #RustLang #WebAssembly #I3S

## X

PlanetX Lite 0.26.0 - a free 3D globe in Rust for desktop and the browser. JAXA Earth monthly data - land and sea temperature, NDVI, chlorophyll, rain, soil moisture, aerosol. I3S scene layers of ArcGIS by a link. https://github.com/Valery35/planetx-lite/releases/tag/v0.26.0 #GIS #RustLang

## Mastodon

PlanetX Lite 0.26.0 - a free 3D globe in Rust and wgpu for Windows, Linux, macOS and the browser.

Seven JAXA Earth rows - land and sea surface temperature, NDVI, chlorophyll-a, GSMaP precipitation, soil moisture, aerosol optical depth. Monthly means for the world, the month is set by the time slider. I3S scene layers of ArcGIS SceneServer open by a link.

https://github.com/Valery35/planetx-lite/releases/tag/v0.26.0

#GIS #RemoteSensing #RustLang #OpenData

## Reddit r/rust (публикует автор, редактор в режиме Markdown, флер Project)

Заголовок: PlanetX Lite 0.26.0 - I3S scene layers with Draco and JAXA Earth COG data on a Rust/wgpu 3D globe

PlanetX Lite is a free (Apache 2.0) 3D globe written in Rust with egui and wgpu. The same code runs as a desktop program for Windows, Linux and macOS and as a web page through WebAssembly.

Version 0.26.0:

- **I3S scene layers** - a link to an ArcGIS `SceneServer` opens its layer. Node pages are read on demand and grafted into the same tree as 3D Tiles, so selection by screen error and loading are shared. Geometry is Draco (the `draco-core` crate), vertex offsets are scaled degrees from the node centre.
- **JAXA Earth** - seven monthly datasets from the JAXA STAC catalog. Both hemispheres of level 0 are read as COG windows with HTTP range requests and painted as a 0.1° world grid over the maps.
- **No eviction loop** - ready children of a shown tile are no longer evicted while they wait for their siblings.

- Web page: https://valery35.github.io/planetx-lite/
- Release: https://github.com/Valery35/planetx-lite/releases/tag/v0.26.0

## Reddit r/gis (публикует автор)

Заголовок: PlanetX Lite 0.26.0 - a free 3D globe with JAXA Earth monthly data and ArcGIS I3S scene layers

PlanetX Lite 0.26.0 adds seven JAXA Earth rows - land and sea surface temperature, NDVI, chlorophyll-a, GSMaP precipitation, soil moisture and aerosol optical depth, monthly means on a 0.1° grid. An I3S scene layer of an ArcGIS SceneServer opens by a link, buildings load as the camera comes closer. Windows, Linux, macOS and a web page.

Release: https://github.com/Valery35/planetx-lite/releases/tag/v0.26.0
