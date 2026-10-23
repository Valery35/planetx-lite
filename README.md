# PlanetX Lite

Глобус [PlanetX](https://github.com/Valery35/planetx) без QGIS. Одна
программа на Rust для Windows, Linux и macOS и та же программа
страницей в браузере, на телефоне тоже.

**Страница:** https://valery35.github.io/planetx-lite/

**Загрузки:** [выпуски](https://github.com/Valery35/planetx-lite/releases)

| Система | Файл |
|---|---|
| Windows x86_64 | `PlanetX-Lite-<версия>-windows-x86_64-setup.exe` - установщик, `...-portable.zip` - без установки |
| Linux x86_64 | `PlanetX-Lite-<версия>-linux-x86_64.AppImage`, `...tar.gz` |
| macOS, Apple Silicon | `PlanetX-Lite-<версия>-macos-aarch64.dmg` |
| macOS, Intel | `PlanetX-Lite-<версия>-macos-x86_64.dmg` |
| Страница для своего сервера | `PlanetX-Lite-<версия>-web.zip` |

Контрольные суммы файлов - `SHA256SUMS` в том же выпуске.

## Что умеет

- Земля с рельефом, космоснимками Esri, картой OpenStreetMap
  и подписями OpenFreeMap, поиск места.
- Сцены `.planetx` модуля PlanetX, файлы KML и KMZ: метки, пути,
  многоугольники, наложения, туры, шкала времени.
- Свои данные: растры высот GeoTIFF и COG, слои GeoPackage, облака
  точек LAS, LAZ и COPC, 3D-модели glTF и Collada, точки CSV.
- Меню «Демо» - сцены модуля из папки `demo` хранилища
  [planetx-terrain](https://github.com/Valery35/planetx-terrain).

## Первый запуск без подписи

Файлы выпуска пока не подписаны.

- **Windows:** окно «Система Windows защитила ваш компьютер» -
  «Подробнее» - «Выполнить в любом случае».
- **macOS:** после первой попытки открыть программу - «Системные
  настройки» - «Конфиденциальность и безопасность» - «Всё равно
  открыть».
- **Linux:** AppImage нужно сделать исполняемым:
  `chmod +x PlanetX-Lite-*.AppImage`.

## Code signing policy

Подпись Windows готовится через SignPath Foundation, бесплатную
подпись открытых проектов. После одобрения здесь будет строка
«Free code signing provided by SignPath.io, certificate by SignPath
Foundation».

- Подписываются только файлы, собранные из исходного кода этого
  хранилища сценарием `.github/workflows/release.yml`.
- Роли проекта:
  - автор, committer - [Valery35](https://github.com/Valery35);
  - рецензент, reviewer - [Valery35](https://github.com/Valery35);
  - утверждающий подпись, approver - [Valery35](https://github.com/Valery35).
- Каждый выпуск утверждается вручную.

## Приватность

Программа не собирает и не отправляет сведения о пользователе.
Она обращается только к источникам данных, которые показывает:
тайлы Esri World Imagery, OpenStreetMap, OpenFreeMap и Terrain Tiles
на Amazon S3, поиск места - Nominatim OpenStreetMap, демо - GitHub.
Сцена или файл по ссылке грузятся с адреса, который указал
пользователь. Запросы несут заголовок `User-Agent` программы с адресом
этого хранилища. Файлы пользователя остаются на его компьютере.

## Лицензия

Apache 2.0, файл [LICENSE](LICENSE). Правообладатель - ООО «Информ++».
Лицензии зависимостей - файл `THIRD-PARTY-LICENSES.html` в каждом
пакете выпуска. Источники данных и их условия - окно «Источники
данных» программы.

---

# PlanetX Lite (English)

The [PlanetX](https://github.com/Valery35/planetx) globe without QGIS.
One Rust program for Windows, Linux and macOS, and the same program as
a web page: https://valery35.github.io/planetx-lite/

Downloads are on the [releases](https://github.com/Valery35/planetx-lite/releases)
page, checksums in `SHA256SUMS`.

## First start without a signature

The release files are not signed yet. On Windows choose "More info" -
"Run anyway". On macOS open System Settings - Privacy & Security -
"Open Anyway". On Linux make the AppImage executable.

## Code signing policy

Windows signing is being arranged with the SignPath Foundation free
code signing for open source. Once approved, this section will read
"Free code signing provided by SignPath.io, certificate by SignPath
Foundation".

- Only files built from this repository by
  `.github/workflows/release.yml` are signed.
- Committer, reviewer and approver: [Valery35](https://github.com/Valery35).
- Every release is approved by hand.

## Privacy

The program does not collect or send user information. It only
contacts the data sources it shows: Esri World Imagery, OpenStreetMap,
OpenFreeMap and Terrain Tiles on Amazon S3 tiles, Nominatim for place
search, GitHub for demos, and addresses the user opens. Requests carry
the program's `User-Agent` with this repository's address. User files
stay on the user's computer.

## License

Apache 2.0, see [LICENSE](LICENSE). Copyright holder - Inform++ LLC.
Dependency licenses are in `THIRD-PARTY-LICENSES.html` in every package.
