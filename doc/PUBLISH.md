# Публикация выпусков

Порядок объявления выпусков PlanetX Lite и модуля PlanetX для QGIS.
Список ведётся здесь, сессии модуля читают этот же файл. Хабр и GIS-Lab
не используются, решение автора от 28 октября 2026 года.

## Правила

- Площадки делятся на четыре группы, решение автора от 7 ноября
  2026 года (прежде - все площадки на каждый выпуск, решение
  от 4 ноября).
  - Всегда, каждый выпуск: выпуск GitHub, Dropbox, сайт informpp.ru
    («Что нового» и главная), Telegram @qgis_isolines, ВК (запись
    на стене, статья - только для крупного выпуска), Mastodon, X,
    LinkedIn, Facebook, каталог QGIS (только модуль).
  - Иногда, крупный выпуск или тема, близкая площадке: OSGeo
    Discourse (ответ в своей теме), форум OSM (пишет только автор,
    когда новое касается данных OSM, без частых постов), linux.org.ru
    (только по решению автора), Reddit r/rust, r/gis, r/QGIS (пишет
    автор, reddit.com закрыт для браузеров помощника).
  - Витрины, по состоянию заявок: AlternativeTo, winget, Flathub,
    Homebrew.
  - Никогда: Хабр, GIS-Lab, форум Rust (учётная запись заморожена),
    Show HN, awesome-списки, This Week in Rust.
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
| Форум OSM, community.openstreetmap.org | Valery35 | Пишет только автор, своими словами. С 7 ноября 2026 года учётная запись без ограничений (уровень доверия 0), темы Lite 149678 и 149680 открыты для ответов, тема 149679 удалена |
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

### PlanetX 1.42.0, 7 ноября 2026 года

Тексты - planetx/site/posts_1.42.md и posts_1.42_social.md, кадр - planetx/site/planetx_post_1.42.jpg.

- GitHub: https://github.com/Valery35/planetx/releases/tag/v1.42.0
- Каталог QGIS: https://plugins.qgis.org/plugins/planetx/version/1.42.0/
- Dropbox - архив скопирован
- Сайт informpp.ru - «Что нового» и главная, коммит a517034
- Telegram: https://t.me/qgis_isolines/188
- ВК: статья https://vk.ru/@qgismodules-planetx-142-podpisi-na-globuse-izdaleka-i-sceny-dlya-vsei, запись https://vk.ru/wall-241229428_51
- Форум OSM: https://community.openstreetmap.org/t/149680/4 (ответ написал помощник модуля, правило «пишет только автор» он увидел после отправки, автору сказано)
- OSGeo Discourse: https://discourse.osgeo.org/t/155878/5
- LinkedIn и Facebook - посты в лентах автора
- X: https://x.com/Valery35/status/2119003187101171984
- Mastodon: https://en.osm.town/@Vaery35/117574519168610306
- linux.org.ru: https://www.linux.org.ru/news/opensource/18415957, ждёт модератора
- Reddit - публикует автор

### PlanetX Lite 0.24.0, 7 ноября 2026 года

Тексты - site/posts_0.24.0_social.md, описание выпуска - packaging/notes/0.24.0.md.

- GitHub: https://github.com/Valery35/planetx-lite/releases/tag/v0.24.0
- Dropbox - PlanetX-Lite.zip 0.24.0
- Сайт informpp.ru - «Что нового» (блок 10699 при странице 10596) и главная, коммит 726912c
- ВК: https://vk.ru/wall-241229428_52 (запись со снимком 3D-зданий Парижа, без статьи)
- OSGeo Discourse: https://discourse.osgeo.org/t/155878/6
- Mastodon: https://en.osm.town/@Vaery35/117574568530012193
- X: https://x.com/Valery35/status/2119006840084459540
- LinkedIn и Facebook - посты в лентах автора 7 ноября 2026 года
- linux.org.ru: https://www.linux.org.ru/news/opensource/18415962, ждёт модератора
- Telegram: https://t.me/qgis_isolines/189
- Форум OSM - пишет автор своими словами, факты переданы в чате
- Форум Rust - не публикуется, учётная запись заморожена
- Reddit r/rust - публикует автор, текст в site/posts_0.24.0_social.md

### PlanetX 1.42.1, 7 ноября 2026 года

Тексты - planetx/site/posts_1.42.1_social.md, статьи нет, выпуск малый.

- GitHub: https://github.com/Valery35/planetx/releases/tag/v1.42.1
- Каталог QGIS: https://plugins.qgis.org/plugins/planetx/version/1.42.1/
- Dropbox - архив скопирован
- Сайт - «Что нового» и главная по новым адресам /главная-страница/planetx/qgis-planetx, коммит 2452c12
- Telegram - бот выпуска отработал, пост в канале ждёт «Опубликовать» автора
- ВК: https://vk.ru/wall-241229428_53
- X: https://x.com/Valery35/status/2119088759341830218
- Mastodon: https://en.osm.town/@Vaery35/117575854269869269
- LinkedIn и Facebook - посты в лентах автора
- Площадки группы «Иногда» (OSGeo, форум OSM, linux.org.ru, Reddit) - не публиковались, выпуск малый

### PlanetX 1.43.0, 9 ноября 2026 года

Тексты - planetx/site/posts_1.43_social.md, статьи нет. По слову автора «доделывай и публикуй».

- Коммиты a73faa4 (окно «Данные JAXA»), 954bcce (выпуск), 6919532 (страница «Что нового»)
- GitHub: https://github.com/Valery35/planetx/releases/tag/v1.43.0
- Каталог QGIS: https://plugins.qgis.org/plugins/planetx/version/1.43.0/ (загружена, ждёт проверки и одобрения)
- Dropbox - planetx-1.43.0.zip в Dropbox\ИИ\PlanetX
- Сайт informpp.ru - «Что нового» и главная модуля, коммит 6919532, опубликовано. Блок «Что нового» 1154 × 7320 короче страницы (20706), прокрутка внутри блока была и до выпуска
- Telegram: https://t.me/qgis_isolines/195 (бот выпуска)
- ВК: https://vk.ru/wall-241229428_57 (запись без снимка)
- Mastodon: https://en.osm.town/@Vaery35/117587763411210814
- X: https://x.com/Valery35/status/2119851293954941257
- LinkedIn и Facebook - посты в лентах автора 9 ноября 2026 года
- OSGeo Discourse: https://discourse.osgeo.org/t/155878/10 (по слову автора «Да, в OSGeo»)
- Форум OSM, linux.org.ru, Reddit - не публиковались, ждут слова автора
- Лента страницы линейки planetx (planetx-scene/site/feed.json) - не обновлялась

### PlanetX Lite 0.25.0, 8 ноября 2026 года

Тексты - site/posts_0.25.0_social.md, описание выпуска - packaging/notes/0.25.0.md. По слову автора «публикуй всюду».

- GitHub: https://github.com/Valery35/planetx-lite/releases/tag/v0.25.0
- Dropbox - PlanetX-Lite.zip 0.25.0
- Сайт informpp.ru - «Что нового» (блок 11308 при странице 11203) и главная, коммит bb27ca5
- ВК: https://vk.ru/wall-241229428_54 (запись со снимком Швейцарии в карте swisstopo, без статьи)
- OSGeo Discourse: https://discourse.osgeo.org/t/155878/7
- Mastodon: https://en.osm.town/@Vaery35/117581306652913116
- X: https://x.com/Valery35/status/2119438056155447707
- LinkedIn и Facebook - посты в лентах автора 8 ноября 2026 года
- Telegram: https://t.me/qgis_isolines/191
- linux.org.ru - не отправлялось, только по решению автора, новость о 0.24.0 ещё у модератора
- Форум OSM и Reddit r/rust - пишет автор, текст для Reddit в файле постов

### PlanetX Lite 0.26.0, 9 ноября 2026 года

Тексты - site/posts_0.26.0_social.md, описание выпуска - packaging/notes/0.26.0.md. По слову автора «публикуй всюду», пока он спал.

- Коммит выпуска 2913ee6, страница в браузере обновлена (Pages)
- GitHub: https://github.com/Valery35/planetx-lite/releases/tag/v0.26.0 (метка отправлена утром 9 ноября по слову автора «да, выпускай»)
- Dropbox - PlanetX-Lite.zip 0.26.0, собран из файлов выпуска для Windows и страницы
- Сайт informpp.ru - «Что нового» (блок 11824 при странице 11716) и главная, коммит 2913ee6, опубликовано
- Сайт informpp.ru - страница линейки `главная-страница/planetx`, в ленте Lite 0.26.0 (planetx-scene 1959981), опубликовано
- Telegram: https://t.me/qgis_isolines/193
- ВК: https://vk.ru/wall-241229428_56 (запись без снимка)
- OSGeo Discourse: https://discourse.osgeo.org/t/155878/9
- Mastodon: https://en.osm.town/@Vaery35/117583787610816530
- X: https://x.com/Valery35/status/2119596824768004597
- LinkedIn и Facebook - посты в лентах автора 9 ноября 2026 года
- Reddit - пишет автор, тексты в файле постов. Пост RustPlanetX 0.5.2 в r/rust снят автомодератором по правилу 7, повторить с флером Project

### RustPlanetX 0.5.2, 8 ноября 2026 года

Тексты - site/posts_rustplanetx_0.5.2_social.md. Первые публикации RustPlanetX. Сначала по слову автора вышел пост в Telegram, затем выпуск разослан по площадкам группы «всегда».

- GitHub: https://github.com/Valery35/rustplanetx/releases/tag/v0.5.2
- Каталог расширений RustGIS - заявка opengeos/rustgis-plugins#47 на 0.5.2, ждёт сопровождающих
- Dropbox - RustPlanetX.rgplugin 0.5.2
- Telegram: https://t.me/qgis_isolines/192
- Mastodon: https://en.osm.town/@Vaery35/117582299254472511
- X: https://x.com/Valery35/status/2119501622321848723
- ВК: https://vk.ru/wall-241229428_55 (запись без снимка, с карточкой выпуска GitHub)
- LinkedIn и Facebook - посты в лентах автора 8 ноября 2026 года
- Сайт informpp.ru - страница линейки `главная-страница/planetx`, в ленте обновлений RustPlanetX 0.5.2, Lite 0.25.0 и модуль 1.42.1 (planetx-scene, site/feed.json). Своей страницы у RustPlanetX нет, главная не менялась
- OSGeo Discourse: https://discourse.osgeo.org/t/155878/8 (9 ноября 2026 года, по слову автора «всюду, где имеет смысл»)
- Reddit r/rust и r/gis - публикует автор, тексты в файле постов
- Форум OSM - не публикуется, учётная запись отключена, данных OSM выпуск не касается
- linux.org.ru - не публикуется, пакет только для Windows
- Форум Rust - не публикуется, учётная запись заморожена