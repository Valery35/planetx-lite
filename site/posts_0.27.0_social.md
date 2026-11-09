# PlanetX Lite 0.27.0 - тексты для площадок

## RU - Telegram, ВК, Facebook

PlanetX Lite 0.27.0 - «Мои слои» работают как «Проект QGIS» модуля PlanetX.

У слоя есть шторка - он виден левее или правее линии, линия тянется мышью. Объекты идут за шкалой времени по полю даты, карты одной папки сменяют друг друга по годам. Горизонтали вида, изолинии своего растра, метки, космоснимки, карты тренда и карты Рамсея ложатся в «Мои слои» слоями. Строки раздела «Слои» отдают вид - землетрясения, разломы, ЛЭП со своими полями, карты картинкой. Слой-картинка сохраняется в GeoTIFF, слой с отметками Z встаёт на свои высоты.

Программа для Windows, Linux и macOS и страница в браузере - https://valery35.github.io/planetx-lite/
Выпуск - https://github.com/Valery35/planetx-lite/releases/tag/v0.27.0

## EN - OSGeo, LinkedIn

PlanetX Lite 0.27.0 - My layers work like the QGIS project of the PlanetX plugin.

A layer has a swipe side - it is seen left or right of the line, the line is dragged with the mouse. Objects follow the time slider by a date field, the maps of one folder replace each other by years. Contours of the view, isolines of an own raster, placemarks, satellite images, trend maps and Rumsey maps go to My layers as layers. Rows of the Layers section give the view - earthquakes, faults, power lines with their fields, maps as a picture. A picture layer is saved as GeoTIFF, a layer with Z values stands at its heights.

Desktop program for Windows, Linux and macOS and the web page - https://valery35.github.io/planetx-lite/
Release - https://github.com/Valery35/planetx-lite/releases/tag/v0.27.0

#GIS #RustLang #WebAssembly #OpenSource #QGIS

## X

PlanetX Lite 0.27.0 - a free 3D globe in Rust for desktop and the browser. Layer swipe, objects on the time slider, map years, contours and view objects to My layers, GeoTIFF export, Z layers in 3D. https://github.com/Valery35/planetx-lite/releases/tag/v0.27.0 #GIS #RustLang

## Mastodon

PlanetX Lite 0.27.0 - a free 3D globe in Rust and wgpu for Windows, Linux, macOS and the browser.

My layers work like a QGIS project - a layer swipe, objects on the time slider by a date field, map years. Contours of the view, isolines, placemarks and satellite images go to My layers. Rows of the globe give the view as layers. GeoTIFF export, Z layers in 3D.

https://github.com/Valery35/planetx-lite/releases/tag/v0.27.0

#GIS #RustLang #OpenData #QGIS

## Reddit r/rust (публикует автор, редактор в режиме Markdown, флер Project)

Заголовок: PlanetX Lite 0.27.0 - a layer swipe by scissor rects and one-pass contours on a Rust/wgpu 3D globe

PlanetX Lite is a free (Apache 2.0) 3D globe written in Rust with egui and wgpu. The same code runs as a desktop program for Windows, Linux and macOS and as a web page through WebAssembly.

Version 0.27.0:

- **Layer swipe** - every point cloud, picture and feature mesh carries a side, the draw call gets a scissor rect left or right of the line. No shader changes, the vector lines of a sided layer leave the shared overlay texture for meshes.
- **Contours in one pass** - a cell takes only the levels between its lowest and highest corner, so a 2048-node grid with hundreds of levels stays fast. A test checks the result against the level-by-level pass.
- **Time of objects** - a date field drives the time slider, the overlay is rebuilt only when the visible set changes.
- **GeoTIFF export** - a picture layer is written as RGBA with an alpha extra sample, read back by GDAL.

- Web page: https://valery35.github.io/planetx-lite/
- Release: https://github.com/Valery35/planetx-lite/releases/tag/v0.27.0

## Reddit r/gis (публикует автор)

Заголовок: PlanetX Lite 0.27.0 - a free 3D globe with a layer swipe, time-enabled layers and view export

PlanetX Lite 0.27.0 makes My layers work like a QGIS project - a layer swipe, objects on the time slider by a date field, map years for old maps, contours of the view and isolines of a raster as layers, earthquakes and faults of the view with their fields, GeoTIFF export of a picture layer, Z layers in 3D. Windows, Linux, macOS and a web page.

Release: https://github.com/Valery35/planetx-lite/releases/tag/v0.27.0
