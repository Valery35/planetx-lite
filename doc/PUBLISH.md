# Публикация выпусков

Порядок объявления выпусков PlanetX Lite и модуля PlanetX для QGIS.
Список ведётся здесь, сессии модуля читают этот же файл. Хабр и GIS-Lab
не используются, решение автора от 28 октября 2026 года.

## Правила

- Выпуск объявляется на всех площадках списка, решение автора
  от 4 ноября 2026 года.
- Учётные записи создаёт и подтверждает автор. Доступ OAuth выдаётся
  только по отдельному согласию автора.
- Помощник публикует через Claude in Chrome в окне с группой вкладок
  Claude. Вход в другом окне Chrome помощнику не виден.
- Каждая публикация проверяется после отправки - запись открывается
  и сверяется. Ссылка на неё заносится в журнал ниже.
- Тексты на английском - для международных площадок, на русском -
  для Telegram, ВК, Facebook и раздела «Россия» форума OSM.

## Площадки

| Площадка | Учётная запись | Как публиковать |
|---|---|---|
| GitHub, выпуск | Valery35 | Сборка Release по тегу, заметки из `packaging/notes` |
| Dropbox | - | `PlanetX-Lite.zip` в `Dropbox\ИИ\PlanetX Lite` |
| Сайт informpp.ru | автор | Шесть блоков из `site/pages`, публикация по слову автора |
| Каталог QGIS (модуль) | Valery35 | plugins.qgis.org |
| Telegram, @qgis_isolines | автор | Приложение Telegram, текст через буфер, пост один раз |
| ВК, vk.ru/qgismodules | автор | Пост или статья через Claude in Chrome |
| Форум OSM, community.openstreetmap.org | Valery35 | General talk по-английски, «Россия (Russia)» по-русски |
| Форум Rust, users.rust-lang.org | автор, вход через GitHub | Раздел Announcements, только Lite |
| OSGeo Discourse, discourse.osgeo.org | автор, вход через GitHub | Раздел General (id 4) |
| LinkedIn | автор | Пост на английском с хештегами, без продвижения |
| Facebook | автор | Пост на русском на странице автора |
| X, @Valery35 | автор | До 280 знаков, ссылка считается за 23 |
| Mastodon, en.osm.town | Vaery35 | Пост на английском, с 1.40.0 |
| linux.org.ru | автор | Раздел «Новости», новость проходит модератора |
| Reddit r/gis, r/QGIS, r/rust | автор | Только автор, сайт закрыт для браузеров помощника |
| AlternativeTo | автор | Карточка подана 4 ноября 2026 года |
| winget | Valery35 | Пакет `InformPP.PlanetXLite`, заявка в microsoft/winget-pkgs |

Особенности площадок:

- Форум OSM. Новичку разрешены две ссылки в посте. Категория
  выбирается до ввода заголовка, иначе её название попадает
  в заголовок.
- OSGeo Discourse. В главный раздел QGIS темы не ставятся,
  его подразделы - языковые и региональные.
- This Week in Rust берёт проекты только из r/rust.
- Show HN закрыт для новых учётных записей Hacker News.
- awesome-списки и Homebrew принимают проекты со звёздами на GitHub.

## Журнал публикаций

### PlanetX Lite 0.22.0, 4 ноября 2026 года

- GitHub: https://github.com/Valery35/planetx-lite/releases/tag/v0.22.0
- ВК: https://vk.com/wall-241229428_44
- Форум OSM, Lite: https://community.openstreetmap.org/t/planetx-lite-free-3d-globe-for-desktop-and-the-browser-opens-kml-geopackage-shapefile-and-geojson/149678
- Форум OSM, модуль и Lite: https://community.openstreetmap.org/t/planetx-for-qgis-and-planetx-lite-free-3d-globes-with-osm-data-3d-buildings-routes-kml-and-tours/149679
- Форум OSM, модуль и Lite по-русски: https://community.openstreetmap.org/t/planetx-qgis-planetx-lite-3d-osm-3d-kml/149680
- Форум Rust: https://users.rust-lang.org/t/planetx-lite-0-22-0-a-3d-globe-in-rust-and-wgpu-for-desktop-and-webassembly-now-with-gis-file-layers/143483
- OSGeo Discourse: https://discourse.osgeo.org/t/planetx-a-free-3d-globe-plugin-for-qgis-and-planetx-lite-the-same-globe-without-qgis/155878
- winget: https://github.com/microsoft/winget-pkgs/pull/470778
- Telegram, LinkedIn, Facebook, X - посты 4 ноября 2026 года
- Сайт informpp.ru, Dropbox, AlternativeTo - обновлены

### PlanetX для QGIS 1.40.0, 4-5 ноября 2026 года

- GitHub: https://github.com/Valery35/planetx/releases/tag/v1.40.0
- Каталог QGIS: https://plugins.qgis.org/plugins/planetx/version/1.40.0/
- ВК, статья: https://vk.ru/@qgismodules-planetx-140-analiz-uchastka-v-qgis-chto-uzhe-est-i-chto
- ВК, пост с опросом: https://vk.ru/wall-241229428_45
- Форум OSM: ответы 2 в темах 149679 и 149680
- OSGeo Discourse: ответ 2 в теме 155878
- Mastodon: https://en.osm.town/@Vaery35/117559681201596064
- linux.org.ru: https://www.linux.org.ru/news/opensource/18414410 (ждёт модератора)
- Telegram, LinkedIn, Facebook, X - посты 4 ноября 2026 года
- Сайт informpp.ru (главная, «Что нового»), Dropbox - обновлены
- Reddit r/QGIS, r/gis - тексты в planetx/site/posts_1.40_social.md, публикует автор
