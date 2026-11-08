# RustPlanetX 0.5.2 - тексты для площадок

## RU - Telegram, ВК, Facebook

RustPlanetX 0.5.2 - трёхмерный глобус PlanetX для RustGIS. Расширение связывает глобус с картой RustGIS так же, как модуль PlanetX связывает его с QGIS.

Слои проекта RustGIS видны в разделе «Проект RustGIS» глобуса. Флажок кладёт слой на глобус в цветах его оформления в RustGIS. Растры и тайловые слои ложатся в том виде, какой у них на карте. Меню «⋯» у слоя приближает к нему, задаёт прозрачность, подписи и выдавливание. Глобус идёт за картой, карта идёт за глобусом. Объект, выбранный на глобусе, выделяется в RustGIS. Слой глобуса уходит в проект RustGIS. Рельеф, снимки, анализ участка, туры и KML - от PlanetX Lite.

Установка в RustGIS 0.5.0 и новее - «Plugins › Plugin Manager › Settings › Install From File…», файл rustplanetx-0.5.2.rgplugin. Глобус открывает кнопка «Глобус PlanetX» на вкладке расширений. Пакет для Windows.

Выпуск - https://github.com/Valery35/rustplanetx/releases/tag/v0.5.2

## EN - LinkedIn

RustPlanetX 0.5.2 - the PlanetX 3D globe for RustGIS. The plugin links the globe with the RustGIS map, as the PlanetX plugin links it with QGIS.

The layers of the RustGIS project are listed in the RustGIS project section of the globe. A check puts a layer on the globe in the colours of its RustGIS style. Rasters and tile layers keep their look from the map. The layer menu zooms to the layer and sets its opacity, labels and extrusion. The globe follows the map, and the map follows the globe. A feature picked on the globe is selected in RustGIS. A globe layer goes to the RustGIS project. Terrain, imagery, site analysis, tours and KML come from PlanetX Lite.

Install in RustGIS 0.5.0 or newer - Plugins › Plugin Manager › Settings › Install From File…, the file rustplanetx-0.5.2.rgplugin. The PlanetX Globe button on the Plugins tab opens the globe. The package is for Windows.

Release - https://github.com/Valery35/rustplanetx/releases/tag/v0.5.2

#GIS #RustLang #RustGIS #OpenSource

## X

RustPlanetX 0.5.2 - the PlanetX 3D globe for RustGIS. Project layers in their RustGIS style, the globe and the map follow each other, a feature picked on the globe is selected in RustGIS. Free, Apache 2.0. https://github.com/Valery35/rustplanetx/releases/tag/v0.5.2 #GIS #RustLang

## Mastodon

RustPlanetX 0.5.2 - the PlanetX 3D globe for RustGIS, a free GIS in Rust.

The project layers go to the globe in their RustGIS style, rasters and tile layers too. The globe follows the map and the map follows the globe. A feature picked on the globe is selected in RustGIS. Terrain, imagery, tours and KML come from PlanetX Lite.

https://github.com/Valery35/rustplanetx/releases/tag/v0.5.2

#GIS #RustLang #OpenSource #FOSS4G

## OSGeo Discourse (тема 155878)

RustPlanetX 0.5.2 - the PlanetX 3D globe for RustGIS, a free desktop GIS written in Rust. The plugin links the globe with the RustGIS map, as the PlanetX plugin links it with QGIS.

- The layers of the RustGIS project are listed in the RustGIS project section of the globe. A check puts a layer on the globe in the colours of its RustGIS style, rasters and tile layers keep their look from the map.
- The layer menu zooms to the layer and sets its opacity, labels and extrusion.
- The globe follows the map, and the map follows the globe.
- A feature picked on the globe is selected in RustGIS, and a globe layer goes to the RustGIS project.
- Terrain, imagery, site analysis, tours and KML come from PlanetX Lite.

Install in RustGIS 0.5.0 or newer - Plugins › Plugin Manager › Settings › Install From File…, the file rustplanetx-0.5.2.rgplugin. The package is for Windows.

Release - https://github.com/Valery35/rustplanetx/releases/tag/v0.5.2

## Reddit r/rust (публикует автор, редактор в режиме Markdown)

Заголовок: RustPlanetX - a Rust/wgpu 3D globe as a process plugin for RustGIS, linked with its map over stdin/stdout

RustPlanetX is a plugin for [RustGIS](https://github.com/opengeos/RustGIS), a desktop GIS written in Rust. Its program is PlanetX Lite, a free (Apache 2.0) 3D globe in Rust with egui and wgpu.

How the link works:

- **Process plugin** - RustGIS runs `planetx-lite.exe rustgis-plugin`. With `"session": true` (opengeos/RustGIS#1553) the program stays running and talks JSON over stdin/stdout - no port, no token. A RustGIS without sessions falls back to its local MCP relay.
- **Project layers** - the globe reads the project with `info.project` and the features with `host.features`, and rereads them on `layers` events. Single symbol, graduated and unique value styles are drawn in their RustGIS colours.
- **Two-way view sync** - the globe follows the map, and the map follows the globe once it stops moving.
- **Selection** - a feature picked on the globe is selected in RustGIS.

The package is for Windows, RustGIS 0.5.0 or newer.

- Release: https://github.com/Valery35/rustplanetx/releases/tag/v0.5.2
- Source of the globe: https://github.com/Valery35/planetx-lite

## Reddit r/gis (публикует автор)

Заголовок: RustPlanetX - a free 3D globe linked with the RustGIS map, like a globe plugin for QGIS

RustPlanetX 0.5.2 adds a 3D globe to [RustGIS](https://github.com/opengeos/RustGIS), a free desktop GIS. The project layers go to the globe in their RustGIS colours, rasters and tile layers too. The globe and the map follow each other, and a feature picked on the globe is selected in RustGIS. Terrain, imagery, tours and KML come from PlanetX Lite. The package is for Windows.

Release: https://github.com/Valery35/rustplanetx/releases/tag/v0.5.2
