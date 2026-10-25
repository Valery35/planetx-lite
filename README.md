# PlanetX Lite

[![Support development](https://img.shields.io/badge/Support-development-2ea44f)](https://github.com/Valery35#%D0%BF%D0%BE%D0%B4%D0%B4%D0%B5%D1%80%D0%B6%D0%B0%D1%82%D1%8C-%D1%80%D0%B0%D0%B7%D1%80%D0%B0%D0%B1%D0%BE%D1%82%D0%BA%D1%83)

The [PlanetX](https://github.com/Valery35/planetx) globe without QGIS.
One Rust program for Windows, Linux and macOS, and the same program as
a web page, on phones too.

[Русский](#planetx-lite-по-русски) below.

**Web page:** https://valery35.github.io/planetx-lite/

**Downloads:** [releases](https://github.com/Valery35/planetx-lite/releases)

**User guide:** [doc/manual/MANUAL.en.md](doc/manual/MANUAL.en.md)

| System | File |
|---|---|
| Windows x86_64 | `PlanetX-Lite-<version>-windows-x86_64-setup.exe` - installer, `...-portable.zip` - no installation |
| Linux x86_64 | `PlanetX-Lite-<version>-linux-x86_64.AppImage`, `...tar.gz` |
| macOS, Apple Silicon | `PlanetX-Lite-<version>-macos-aarch64.dmg` |
| macOS, Intel | `PlanetX-Lite-<version>-macos-x86_64.dmg` |
| Web page for your own server | `PlanetX-Lite-<version>-web.zip` |

File checksums are in `SHA256SUMS` in the same release.

The program is free and open source. [Support development](https://github.com/Valery35#%D0%BF%D0%BE%D0%B4%D0%B4%D0%B5%D1%80%D0%B6%D0%B0%D1%82%D1%8C-%D1%80%D0%B0%D0%B7%D1%80%D0%B0%D0%B1%D0%BE%D1%82%D0%BA%D1%83) (Russian bank cards via SBP)

## Features

- The Earth with terrain, Esri imagery, the OpenStreetMap map,
  OpenFreeMap labels and place search.
- PlanetX `.planetx` scenes, KML and KMZ: placemarks, paths, polygons,
  overlays, tours, time slider.
- Own data: GeoTIFF and COG elevation, GeoPackage layers, LAS, LAZ and
  COPC point clouds, glTF and Collada 3D models, CSV points.
- My Places, as in PlanetX: opened scenes and files as folders, a context
  menu, rename, drag and drop. Places are kept between runs. A row, a
  folder or the whole tree saves as KML, KMZ or a `.planetx` scene.
- The Demo menu opens PlanetX demo scenes from the `demo` folder of
  [planetx-terrain](https://github.com/Valery35/planetx-terrain).
- Settings: interface language, globe label language, dark or light
  appearance, vertical exaggeration.
- 16 interface languages: English, Russian, German, Spanish, French,
  Italian, Dutch, Portuguese, Turkish, Azerbaijani, Indonesian,
  Vietnamese, Chinese, Japanese, Korean, Georgian. The catalogs are
  `crates/ui/i18n/<code>.tsv`, corrections from native speakers are
  welcome.

## First start without a signature

The release files are not signed yet.

- **Windows:** the "Windows protected your PC" window - "More info" -
  "Run anyway".
- **macOS:** after the first attempt to open the program - System
  Settings - Privacy & Security - "Open Anyway".
- **Linux:** make the AppImage executable:
  `chmod +x PlanetX-Lite-*.AppImage`.

## Code signing policy

Windows signing is being arranged with the SignPath Foundation free code
signing for open source. Once approved, this section will read "Free code
signing provided by SignPath.io, certificate by SignPath Foundation".

- Only files built from the source code of this repository by
  `.github/workflows/release.yml` are signed.
- Project roles:
  - committer - [Valery35](https://github.com/Valery35);
  - reviewer - [Valery35](https://github.com/Valery35);
  - approver - [Valery35](https://github.com/Valery35).
- Every release is approved by hand.

## Privacy

The program does not collect or send user information. It only contacts
the data sources it shows: Esri World Imagery, OpenStreetMap, OpenFreeMap
and Terrain Tiles on Amazon S3 tiles, Nominatim for place search, GitHub
for demos and, on the web page, for Chinese, Japanese and Korean fonts.
A scene or a file by a link loads from the address the user gave.
Requests carry the program's `User-Agent` with this repository's address.
User files stay on the user's computer.

## License

Apache 2.0, see [LICENSE](LICENSE). Copyright holder - Inform++ LLC.
Dependency licenses are in `THIRD-PARTY-LICENSES.html` in every release
package. Data sources and their terms are in the program's Data sources
window.

---

# PlanetX Lite по-русски

[![Поддержать СБП](https://img.shields.io/badge/%D0%9F%D0%BE%D0%B4%D0%B4%D0%B5%D1%80%D0%B6%D0%B0%D1%82%D1%8C-%D0%A1%D0%91%D0%9F-2ea44f)](https://github.com/Valery35#%D0%BF%D0%BE%D0%B4%D0%B4%D0%B5%D1%80%D0%B6%D0%B0%D1%82%D1%8C-%D1%80%D0%B0%D0%B7%D1%80%D0%B0%D0%B1%D0%BE%D1%82%D0%BA%D1%83)

Глобус [PlanetX](https://github.com/Valery35/planetx) без QGIS. Одна
программа на Rust для Windows, Linux и macOS и та же программа
страницей в браузере, на телефоне тоже.

**Страница:** https://valery35.github.io/planetx-lite/

**Загрузки:** [выпуски](https://github.com/Valery35/planetx-lite/releases)

**Руководство:** [doc/manual/MANUAL.ru.md](doc/manual/MANUAL.ru.md)

| Система | Файл |
|---|---|
| Windows x86_64 | `PlanetX-Lite-<версия>-windows-x86_64-setup.exe` - установщик, `...-portable.zip` - без установки |
| Linux x86_64 | `PlanetX-Lite-<версия>-linux-x86_64.AppImage`, `...tar.gz` |
| macOS, Apple Silicon | `PlanetX-Lite-<версия>-macos-aarch64.dmg` |
| macOS, Intel | `PlanetX-Lite-<версия>-macos-x86_64.dmg` |
| Страница для своего сервера | `PlanetX-Lite-<версия>-web.zip` |

Контрольные суммы файлов - `SHA256SUMS` в том же выпуске.

Программа бесплатная и открытая. [Поддержать разработку](https://github.com/Valery35#%D0%BF%D0%BE%D0%B4%D0%B4%D0%B5%D1%80%D0%B6%D0%B0%D1%82%D1%8C-%D1%80%D0%B0%D0%B7%D1%80%D0%B0%D0%B1%D0%BE%D1%82%D0%BA%D1%83)

## Что умеет

- Земля с рельефом, космоснимками Esri, картой OpenStreetMap
  и подписями OpenFreeMap, поиск места.
- Сцены `.planetx` модуля PlanetX, файлы KML и KMZ: метки, пути,
  многоугольники, наложения, туры, шкала времени.
- Свои данные: растры высот GeoTIFF и COG, слои GeoPackage, облака
  точек LAS, LAZ и COPC, 3D-модели glTF и Collada, точки CSV.
- «Мои метки», как в модуле: открытые сцены и файлы папками, меню
  правой кнопки, переименование, перетаскивание. Метки хранятся между
  запусками. Строка, папка или всё дерево сохраняется как KML, KMZ или
  сценой `.planetx`.
- Меню «Демо» - сцены модуля из папки `demo` хранилища
  [planetx-terrain](https://github.com/Valery35/planetx-terrain).
- Настройки: язык интерфейса, язык подписей на глобусе, тёмное или
  светлое оформление, вертикальный масштаб рельефа.
- 16 языков интерфейса: английский, русский, немецкий, испанский,
  французский, итальянский, нидерландский, португальский, турецкий,
  азербайджанский, индонезийский, вьетнамский, китайский, японский,
  корейский, грузинский. Каталоги - `crates/ui/i18n/<код>.tsv`,
  поправки носителей языка приветствуются.

## Первый запуск без подписи

Файлы выпуска пока не подписаны.

- **Windows:** окно «Система Windows защитила ваш компьютер» -
  «Подробнее» - «Выполнить в любом случае».
- **macOS:** после первой попытки открыть программу - «Системные
  настройки» - «Конфиденциальность и безопасность» - «Всё равно
  открыть».
- **Linux:** AppImage нужно сделать исполняемым:
  `chmod +x PlanetX-Lite-*.AppImage`.

## Подпись кода

Подпись Windows готовится через SignPath Foundation, бесплатную
подпись открытых проектов. Правила подписи - раздел «Code signing
policy» выше.

## Приватность

Программа не собирает и не отправляет сведения о пользователе.
Она обращается только к источникам данных, которые показывает:
тайлы Esri World Imagery, OpenStreetMap, OpenFreeMap и Terrain Tiles
на Amazon S3, поиск места - Nominatim OpenStreetMap, демо и на
странице шрифты для китайского, японского и корейского - GitHub. Сцена
или файл по ссылке грузятся с адреса, который указал пользователь.
Запросы несут заголовок `User-Agent` программы с адресом этого
хранилища. Файлы пользователя остаются на его компьютере.

## Лицензия

Apache 2.0, файл [LICENSE](LICENSE). Правообладатель - ООО «Информ++».
Лицензии зависимостей - файл `THIRD-PARTY-LICENSES.html` в каждом
пакете выпуска. Источники данных и их условия - окно «Источники
данных» программы.
