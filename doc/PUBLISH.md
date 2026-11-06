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
- Форумы разработчиков (OSM, Rust) не принимают анонсов, тем более
  похожих на текст ИИ: ровные списки достоинств, громкие заявления,
  «feedback is welcome». Туда пишет только автор, своими словами,
  коротко, с одним живым вопросом к сообществу. Помощник даёт факты,
  но не готовый текст. Решение автора от 6 ноября 2026 года.
- Соцсетям и витринам (X, LinkedIn, Facebook, ВК, Telegram, Mastodon,
  Reddit, каталог QGIS, AlternativeTo, winget) происхождение текста
  безразлично,
  там тексты помощника публикуются как прежде.

## Площадки

| Площадка | Учётная запись | Как публиковать |
|---|---|---|
| GitHub, выпуск | Valery35 | Сборка Release по тегу, заметки из `packaging/notes` |
| Dropbox | - | `PlanetX-Lite.zip` в `Dropbox\ИИ\PlanetX Lite` |
| Сайт informpp.ru | автор | Шесть блоков из `site/pages`, публикация по слову автора |
| Каталог QGIS (модуль) | Valery35 | plugins.qgis.org |
| Telegram, @qgis_isolines | автор | Приложение Telegram, текст через буфер, пост один раз |
| ВК, vk.ru/qgismodules | автор | Пост или статья через Claude in Chrome |
| Форум OSM, community.openstreetmap.org | Valery35 | Не публиковать. Блокировку сняли 6 ноября 2026 года, но темы закрыты, писать в них нельзя |
| Форум Rust, users.rust-lang.org | автор, вход через GitHub | Не публиковать: 6 ноября 2026 года учётная запись заморожена с причиной «slop», пост о 0.22.0 удалён |
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

- Форум OSM. Пять тем и ответов о своих программах за сутки участники
  сочли рекламой, учётную запись заблокировали 5 ноября 2026 года.
  Mateusz Konieczny: программа мало связана с OSM, кроме того, что
  нагружает его серверы. Автор написал модераторам личным сообщением
  (группа moderators), 6 ноября блокировку сняли, темы остались
  закрытыми. Новичок (уровень доверия 0) пишет в личку только
  модераторам. Участнику можно написать через почту openstreetmap.org:
  openstreetmap.org/message/new/<имя>. Так автор ответил Zkir
  (UrbanEye3D) про 3D-здания 6 ноября 2026 года.
- Форум Rust. Пост о PlanetX Lite 0.22.0 удалён по жалобам, учётная
  запись заморожена с причиной «slop» - текст похож на сгенерированный
  ИИ, на форуме это запрещено. 6 ноября 2026 года.
- Discourse (форумы Rust и OSGeo). Ответ в своей теме - fetch POST
  /posts.json {topic_id, raw} с X-CSRF-Token из meta csrf-token.
- Mastodon. Предел поста - 500 знаков, подсказку тегов закрывает пробел.
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
- linux.org.ru: https://www.linux.org.ru/news/opensource/18414410 (прошла модерацию, в ленте новостей 6 ноября 2026 года)
- Telegram, LinkedIn, Facebook, X - посты 4 ноября 2026 года
- Сайт informpp.ru (главная, «Что нового»), Dropbox - обновлены
- Reddit r/QGIS, r/gis - опубликовал автор 5 ноября 2026 года, тексты в planetx/site/posts_1.40_social.md, ссылок на посты помощник не видит (reddit.com закрыт)

### PlanetX Lite 0.23.0, 5 ноября 2026 года

- GitHub: https://github.com/Valery35/planetx-lite/releases/tag/v0.23.0
- Формы ошибок и пожеланий: https://github.com/Valery35/planetx-lite/issues/new/choose
- ВК: https://vk.ru/wall-241229428_48
- Форум Rust: ответ 2 в теме 143483
- OSGeo Discourse: ответ 3 в теме 155878
- Mastodon: https://en.osm.town/@Vaery35/117560172295079513
- Telegram, LinkedIn, Facebook, X - посты 5 ноября 2026 года
- Сайт informpp.ru (шесть страниц, блок «Что нового» 9970), Dropbox - обновлены
- Форум OSM - не опубликовано, учётная запись заблокирована

### Формы ошибок и пожеланий на GitHub, 5 ноября 2026 года

Тексты - planetx/site/posts_issue_forms.md.

- Формы: https://github.com/Valery35/planetx/issues/new/choose и https://github.com/Valery35/planetx-lite/issues/new/choose
- Telegram: https://t.me/qgis_isolines/186
- ВК: https://vk.ru/wall-241229428_49
- OSGeo Discourse: ответ в теме 155878 отправлен на модерацию, к 6 ноября 2026 года в теме не появился
- Mastodon: https://en.osm.town/@Vaery35/117564382084961506
- X: https://x.com/Valery35/status/2118354934856872285
- LinkedIn: https://www.linkedin.com/feed/update/urn:li:activity:7524120941312933888/
- Facebook: пост на странице https://www.facebook.com/valery35/ 5 ноября 2026 года
- Форум OSM - не опубликовано, учётная запись Valery35 отключена (silenced), темы 149679 и 149680 скрыты
- linux.org.ru - не публикуется, объявление о формах не новость; Reddit - по желанию автора

### Просьба потестировать PlanetX 1.40.0 и Lite 0.23.0, 5 ноября 2026 года

Текст - planetx/site/posts_test_request.md. Разослал автор в Telegram, около 6 групп и личных чатов, список не записан.

### PlanetX 1.41.0, 6 ноября 2026 года

Тексты - planetx/site/posts_1.41.md и posts_1.41_social.md, кадр - planetx/site/planetx_post_1.41.jpg.

- GitHub: https://github.com/Valery35/planetx/releases/tag/v1.41.0
- Каталог QGIS: https://plugins.qgis.org/plugins/planetx/version/1.41.0/ (загружена 6 ноября 2026 года, не одобрена на 6 ноября, как и 1.40.0 от 4 ноября, итог проверок - после входа автора)
- Dropbox - архив скопирован
- Сайт informpp.ru - «Что нового» и главная, коммит fd9890b
- Telegram: https://t.me/qgis_isolines/187
- ВК: https://vk.ru/wall-241229428_50 (запись на стене, без статьи - выпуск небольшой)
- OSGeo Discourse: https://discourse.osgeo.org/t/155878/4
- LinkedIn: https://www.linkedin.com/feed/update/urn:li:activity:7524352153843478528/
- Facebook: https://www.facebook.com/valery35/posts/pfbid02tghY2a3e1zidQ64PH5oWz8Pt6CE9ekiZN5kSS5KFSp7aaGWJ2JRsqcftNJWVu6Ggl
- X: https://x.com/Valery35/status/2118585519403659441 (вышел и повтор 2118585229577441567 - первый щелчок сработал с задержкой)
- Mastodon: https://en.osm.town/@Vaery35/117567996150145928
- linux.org.ru: https://www.linux.org.ru/news/opensource/18415249 (прошла модерацию, в ленте новостей)
- Форум OSM - не опубликовано, учётная запись отключена, тема 149679 удалена
- Reddit - публикует автор, текст в posts_1.41_social.md