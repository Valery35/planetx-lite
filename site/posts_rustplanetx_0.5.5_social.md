# RustPlanetX 0.5.5 - тексты для площадок

Выпуск собирает 0.5.3-0.5.5: PlanetX Lite 0.26.0 внутри, вид в макет RustGIS, глобус для помощника RustGIS, оформление расчётов.

## RU - Telegram, ВК, Facebook

RustPlanetX 0.5.5 - трёхмерный глобус PlanetX для RustGIS. Помощник ИИ RustGIS теперь управляет глобусом.

Просьба «покажи этот участок в 3D» или «посмотри на гору с юга» превращается в перелёт глобуса с нужным наклоном и азимутом. Просьба «добавь 3D-вид в макет» кладёт вид глобуса картинкой в макет RustGIS. Для этого у расширения две новые команды, «Вид в 3D» и «Вид в макет», и навык для помощника. Команды есть и на ленте RustGIS.

Расчёты анализа участка ложатся в проект RustGIS в тех же цветах, что на глобусе: видимость, инсоляция, тени, уклон и экспозиция, затопление, земляные работы, классы и минералы. Рядом с каждым файлом GeoTIFF лежит стиль QML, его берёт и QGIS.

Внутри - PlanetX Lite 0.26.0: данные JAXA Earth по месяцам и слои сцен I3S.

Установка в RustGIS 0.5.0 и новее - «Plugins › Plugin Manager › Settings › Install From File…», файл rustplanetx-0.5.5.rgplugin. Пакет для Windows.

Выпуск - https://github.com/Valery35/rustplanetx/releases/tag/v0.5.5

## EN - LinkedIn

RustPlanetX 0.5.5 - the PlanetX 3D globe for RustGIS. The RustGIS AI Assistant now drives the globe.

A request like "show this area in 3D" or "look at the mountain from the south" becomes a globe flight with the right tilt and heading. "Add a 3D view to the layout" puts the globe's view in a RustGIS page layout as a picture. The plugin has two new commands for this, 3D View and View to Layout, and a skill for the assistant. The commands are on the RustGIS ribbon too.

Site analysis results go to the RustGIS project in the colours of the globe: viewshed, insolation, shadows, slope and aspect, flooding, earthwork, classes and minerals. Each GeoTIFF comes with a QML style that QGIS reads as well.

Inside is PlanetX Lite 0.26.0 with JAXA Earth monthly data and I3S scene layers.

Install in RustGIS 0.5.0 or newer - Plugins › Plugin Manager › Settings › Install From File…, the file rustplanetx-0.5.5.rgplugin. The package is for Windows.

Release - https://github.com/Valery35/rustplanetx/releases/tag/v0.5.5

#GIS #RustLang #RustGIS #OpenSource #AI

## X

RustPlanetX 0.5.5 - the RustGIS AI Assistant drives the PlanetX 3D globe: "show this area in 3D" flies the globe, "add a 3D view to the layout" puts it in a page layout. Free, Apache 2.0. https://github.com/Valery35/rustplanetx/releases/tag/v0.5.5 #GIS #RustLang

## Mastodon

RustPlanetX 0.5.5 - the PlanetX 3D globe for RustGIS, a free GIS in Rust.

The RustGIS AI Assistant now drives the globe: a flight to a place with a tilt and heading, and the globe's view as a picture in a page layout. Site analysis results go to the RustGIS project in the colours of the globe, with QML styles QGIS reads too.

https://github.com/Valery35/rustplanetx/releases/tag/v0.5.5

#GIS #RustLang #OpenSource #FOSS4G

## OSGeo Discourse (тема 155878)

RustPlanetX 0.5.5 - the PlanetX 3D globe for RustGIS, a free desktop GIS written in Rust.

- The RustGIS AI Assistant drives the globe. The plugin has two commands, 3D View (a flight to the map's view, a point or an extent with a tilt and heading) and View to Layout (the globe's view as a picture in a page layout), and a skill telling the assistant when to use them.
- Site analysis results go to the RustGIS project in the colours of the globe. Each GeoTIFF comes with a QML style, which QGIS reads as well.
- Inside is PlanetX Lite 0.26.0 with JAXA Earth monthly data and I3S scene layers.

Install in RustGIS 0.5.0 or newer - Plugins › Plugin Manager › Settings › Install From File…, the file rustplanetx-0.5.5.rgplugin. The package is for Windows.

Release - https://github.com/Valery35/rustplanetx/releases/tag/v0.5.5

## Reddit r/rust (публикует автор, флер Project, редактор в режиме Markdown)

Заголовок: RustPlanetX - a Rust/wgpu 3D globe plugin for RustGIS that the RustGIS AI assistant can drive

RustPlanetX is a plugin for [RustGIS](https://github.com/opengeos/RustGIS), a desktop GIS written in Rust. Its program is PlanetX Lite, a free (Apache 2.0) 3D globe in Rust with egui and wgpu.

What is in 0.5.5:

- **Commands for the assistant** - the plugin declares `fly` and `layout` commands and ships an Agent Skills `SKILL.md`. The RustGIS assistant reads the skill and calls the commands: a flight with a tilt and heading, or the globe's view into a page layout (`layout.addElement` with a picture).
- **Two transports** - with `"session": true` (opengeos/RustGIS#1553) a command arrives as a JSON line on the running globe's stdin. A RustGIS without sessions starts the program per command; it hands the command to the open globe through a folder and waits for the answer.
- **Styled results** - analysis rasters carry their colour scale, written as a QGIS QML style and applied in RustGIS with `style.importQgis`.

The package is for Windows, RustGIS 0.5.0 or newer.

- Release: https://github.com/Valery35/rustplanetx/releases/tag/v0.5.5
- Source of the globe: https://github.com/Valery35/planetx-lite
