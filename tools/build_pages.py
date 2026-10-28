# -*- coding: utf-8 -*-
# PlanetX Lite - глобус PlanetX без QGIS.
# Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
# SPDX-License-Identifier: Apache-2.0
"""Страницы PlanetX Lite для www.informpp.ru - главная и пять разделов.

    python tools/build_pages.py

Решение автора от 28 октября 2026 года - группа страниц, как у модуля
PlanetX (tools/build_pages.py модуля): своя страница planetx-lite
и подстраницы «Онлайн» (с того же дня), «Метки и туры», «Данные»,
«Небо и тела», «Что нового».
Каждая страница - самодостаточный файл HTML в site/pages для блока
«Встроить» Google Сайтов, на двух языках с переключателем, снимки
встроены строками data:. Песочница блока не разрешает переход верхнего
окна, поэтому все ссылки открываются новой вкладкой. Снимки - из
программы, site/shots. «Что нового» - описания выпусков
packaging/notes. Нужен Python с Pillow, например из состава QGIS.
"""
import base64
import html
import io
import json
import os
import re

from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "site", "pages")
SHOTS = os.path.join(ROOT, "site", "shots")
SITE = "https://www.informpp.ru/главная-страница/planetx-lite"
PRO = "https://www.informpp.ru/главная-страница/qgis-planetx"
WEB = "https://valery35.github.io/planetx-lite/"
REPO = "https://github.com/Valery35/planetx-lite"
RELEASES = REPO + "/releases/latest"
MANUAL_RU = REPO + "/blob/main/doc/manual/MANUAL.ru.md"
MANUAL_EN = REPO + "/blob/main/doc/manual/MANUAL.en.md"
# «Поддержать разработку» - раздел профиля автора на GitHub, как в README
SUPPORT_URL = ("https://github.com/Valery35#%D0%BF%D0%BE%D0%B4%D0%B4%D0%B5"
               "%D1%80%D0%B6%D0%B0%D1%82%D1%8C-%D1%80%D0%B0%D0%B7%D1%80"
               "%D0%B0%D0%B1%D0%BE%D1%82%D0%BA%D1%83")
# Страница: ключ, путь подстраницы, название по-русски и по-английски.
PAGES = (
    ("main", "", "Главная", "Overview"),
    ("online", "online", "Онлайн", "Online"),
    ("places", "places", "Метки и туры", "Places and tours"),
    ("data", "data", "Данные", "Data"),
    ("sky", "sky", "Небо и тела", "Sky and bodies"),
    ("news", "news", "Что нового", "What's new"),
)
WEB_WIDTH = 960  # пикселей, ширина снимка на странице
HALF_WIDTH = 560  # пикселей, снимок в паре или тройке
WEB_QUALITY = 70  # качество JPEG


def url(path):
    return SITE + ("/" + path if path else "")


# ---------------------------------------------------------------- тексты

T = {}  # ключ - (русский, английский)


def t(key, ru, en):
    T[key] = (ru, en)
    return key


def span(key, tag="span", cls=""):
    attr = ' class="%s"' % cls if cls else ""
    return '<%s%s data-i18n="%s"></%s>' % (tag, attr, key, tag)


def figure(src, key):
    return ('<figure><img alt="" src="%s">\n<figcaption data-i18n="%s">'
            '</figcaption></figure>' % (src, key))


def cards(prefix, items, kind="trio"):
    out = ['<div class="%s">' % kind]
    for n, (h_ru, h_en, p_ru, p_en) in enumerate(items, 1):
        h = t("%s.c%d.h" % (prefix, n), h_ru, h_en)
        p = t("%s.c%d.p" % (prefix, n), p_ru, p_en)
        out.append('<div class="card"><h3 data-i18n="%s"></h3>'
                   '<p data-i18n="%s"></p></div>' % (h, p))
    out.append("</div>")
    return "\n".join(out)


def rows(prefix, items):
    """Строки таблицы: (название ru, en, описание ru, en)."""
    out = ['<div class="tools">']
    for n, (h_ru, h_en, p_ru, p_en) in enumerate(items, 1):
        out.append('<div class="row"><b data-i18n="%s"></b><span data-i18n='
                   '"%s"></span></div>' % (t("%s.r%d.h" % (prefix, n), h_ru,
                                             h_en),
                                           t("%s.r%d.p" % (prefix, n), p_ru,
                                             p_en)))
    out.append("</div>")
    return "\n".join(out)


def section(sid, eyebrow, h2, sub, body=""):
    e = t(sid + ".eyebrow", *eyebrow)
    h = t(sid + ".h2", *h2)
    s = t(sid + ".sub", *sub) if sub else None
    return ('<section id="%s"><div class="wrap">\n'
            '<div class="eyebrow" data-i18n="%s"></div>\n'
            '<h2 data-i18n="%s"></h2>\n%s%s\n</div></section>'
            % (sid, e, h, '<p class="sub" data-i18n="%s"></p>\n' % s
               if s else "", body))


def trio(*figs):
    return '<div class="trio">%s</div>' % "".join(figs)


def pair(*figs):
    return '<div class="pair">%s</div>' % "".join(figs)


def buttons(main=True):
    t("cta.web", "Открыть в браузере", "Open in the browser")
    t("cta.release", "Скачать программу", "Download the program")
    t("cta.code", "Исходный код", "Source code")
    t("cta.manual", "Руководство", "User guide")
    t("cta.manual.url", MANUAL_RU, MANUAL_EN)
    out = ['<div class="cta">',
           '<a class="btn btn-main" target="_blank" data-i18n="cta.web" '
           'href="%s"></a>' % WEB,
           '<a class="btn btn-ghost" target="_blank" data-i18n="cta.release" '
           'href="%s"></a>' % RELEASES]
    if main:
        t("cta.pro", "PlanetX для QGIS", "PlanetX for QGIS")
        out += ['<a class="btn btn-ghost" target="_blank" data-i18n="cta.pro"'
                ' href="%s"></a>' % PRO,
                '<a class="btn btn-ghost" target="_blank" data-i18n="cta.code"'
                ' href="%s"></a>' % REPO,
                '<a class="btn btn-ghost" target="_blank" data-i18n='
                '"cta.manual" data-i18n-href="cta.manual.url" href="%s"></a>'
                % MANUAL_RU]
    out.append("</div>")
    return "\n".join(out)


# ---------------------------------------------------------------- страницы

def page_main(img):
    t("hero.eyebrow", "Программа и страница в браузере · 3D-глобус",
      "A program and a web page · 3D globe")
    t("hero.h1", "Глобус PlanetX без QGIS - на компьютере, в браузере "
      "и на телефоне", "The PlanetX globe without QGIS - on a computer, in "
      "a browser and on a phone")
    t("hero.lead",
      "PlanetX Lite - отдельная программа, QGIS ей не нужен. По слоям "
      "и инструментам она близка к модулю PlanetX для QGIS: тот же глобус, "
      "каталог слоёв, метки, туры, линейка, разрез, небо и другие тела. "
      "Сцены модуля открываются в Lite, сцены Lite - в модуле. Одна "
      "программа для Windows, Linux и macOS, та же программа работает "
      "страницей в браузере без установки.",
      "PlanetX Lite is a standalone program, it needs no QGIS. In layers "
      "and tools it is close to the PlanetX plugin for QGIS: the same "
      "globe, layer catalog, placemarks, tours, ruler, section, sky and "
      "other bodies. Plugin scenes open in Lite, Lite scenes open in the "
      "plugin. One program for Windows, Linux and macOS, the same program "
      "runs as a web page with nothing to install.")
    t("hero.shot", "Эльбрус с наклоном камеры. Снимки Esri на рельефе, "
      "подписи OpenFreeMap.", "Elbrus with a tilted camera. Esri imagery on "
      "the terrain, OpenFreeMap labels.")
    hero = ('<div class="wrap hero">\n%s\n%s\n%s\n%s\n'
            '<figure class="shot"><img alt="" src="%s">\n'
            '<figcaption data-i18n="hero.shot"></figcaption></figure>\n'
            '</div>' % (span("hero.eyebrow", "div", "eyebrow"),
                        span("hero.h1", "h1"),
                        span("hero.lead", "p", "lead"), buttons(),
                        img("hero.jpg")))
    tasks = (
        ("places", "Метки и туры", "Places and tours",
         "Метки, пути, многоугольники и круги, линейка и профиль высот, "
         "туры, запись облёта, шкала времени, сцены и ссылки на вид.",
         "Placemarks, paths, polygons and circles, the ruler and an "
         "elevation profile, tours, flight recording, the time slider, "
         "scenes and links to a view."),
        ("data", "Данные о Земле", "Earth data",
         "Горизонтали, уклоны, геология, землетрясения, вулканы, пожары, "
         "течения, энергетика, 3D-здания, слои NASA и свои файлы.",
         "Contours, slopes, geology, earthquakes, volcanoes, fires, "
         "currents, energy, 3D buildings, NASA layers and own files."),
        ("sky", "Небо и другие тела", "The sky and other bodies",
         "Марс, Луна и другие тела со снимками и рельефом. Звёздное небо "
         "с созвездиями, планетами и объектами глубокого неба.",
         "Mars, the Moon and other bodies with imagery and terrain. The "
         "starry sky with constellations, planets and deep-sky objects."),
    )
    paths = dict((k, path) for k, path, _, _ in PAGES)
    parts = []
    for key, h_ru, h_en, p_ru, p_en in tasks:
        parts.append('<a class="card link" target="_blank" href="%s">'
                     '<h3 data-i18n="%s"></h3><p data-i18n="%s"></p></a>'
                     % (url(paths[key]), t("tasks.%s.h" % key, h_ru, h_en),
                        t("tasks.%s.p" % key, p_ru, p_en)))
    tasks = section(
        "tasks", ("Что можно делать", "What you can do"),
        ("От меток и туров до звёздного неба",
         "From placemarks and tours to the starry sky"), None,
        '<div class="trio">%s</div>' % "\n".join(parts))
    idea = section(
        "idea", ("Как устроен глобус", "How the globe works"),
        ("Один код для компьютера и браузера",
         "One code for the computer and the browser"),
        ("Lite написан на Rust и рисует глобус через wgpu - Vulkan, "
         "DirectX 12, Metal или OpenGL. Страница - та же программа "
         "в WebAssembly. Мировые координаты считаются в двойной точности, "
         "в видеокарту попадают только смещения от центра тайла и от глаза, "
         "на уровне улиц картинка не дрожит. Сеть не тормозит кадр.",
         "Lite is written in Rust and draws the globe with wgpu - Vulkan, "
         "DirectX 12, Metal or OpenGL. The web page is the same program in "
         "WebAssembly. World coordinates are computed in double precision, "
         "only offsets from the tile centre and from the eye reach the "
         "graphics card, the picture does not jitter at street level. The "
         "network never holds up a frame."),
        figure(img("window.jpg"), t(
            "idea.fig", "Окно программы над Пермью с 14 км. Слева поиск, "
            "«Мои метки» с папками демо и «Слои», вверху панель значков.",
            "The program window over Perm from 14 km. Search, My Places with "
            "demo folders and Layers on the left, the icon bar on top.")) +
        cards("idea", (
            ("Подложка и рельеф", "Base map and terrain",
             "Снимки Esri World Imagery, карта OpenStreetMap или готовые "
             "подложки каталога. Рельеф Terrain Tiles, дно морей, свой "
             "растр высот GeoTIFF.",
             "Esri World Imagery, the OpenStreetMap map or ready base maps "
             "of the catalog. Terrain Tiles relief, the sea floor, an own "
             "GeoTIFF elevation raster."),
            ("Ровные подписи", "Upright labels",
             "Названия стран, городов, рек и вершин из векторной основы "
             "OpenFreeMap. Язык подписей - интерфейса, местный или один "
             "из 16.",
             "Names of countries, cities, rivers and peaks from the "
             "OpenFreeMap vector base. The label language is that of the "
             "interface, local or one of 16."),
            ("Атмосфера и свет", "Atmosphere and light",
             "Свечение воздуха, небо у горизонта, солнце на дату и время, "
             "ночная сторона с огнями городов.",
             "The glow of the air, the sky at the horizon, the sun for a "
             "date and time, the night side with city lights."))))
    phone = section(
        "phone", ("Страница в браузере", "The web page"),
        ("Без установки, на телефоне тоже",
         "Nothing to install, on a phone too"),
        ("Страница работает в Chrome, Edge, Firefox и Safari. Она ставится "
         "на главный экран телефона и работает без сети на открытой сцене. "
         "Ссылка с параметрами scene и pose открывает сцену и вид. Часть "
         "слоёв в браузере недоступна из-за его ограничений, в программе "
         "они есть.",
         "The page works in Chrome, Edge, Firefox and Safari. It installs on "
         "the home screen of a phone and works offline on an opened scene. A "
         "link with the scene and pose parameters opens a scene and a view. "
         "Some layers are not available in a browser because of its "
         "restrictions, the program has them."),
        cards("phone", (
            ("Телефон", "Phone",
             "Один палец вращает глобус, щипок приближает, два пальца "
             "поворачивают и наклоняют вид. Долгое нажатие на значок "
             "показывает подсказку.",
             "One finger turns the globe, a pinch zooms, two fingers turn "
             "and tilt the view. A long press on an icon shows a tooltip."),
            ("Ссылка на вид", "A link to a view",
             "Кнопка «Поделиться видом» собирает ссылку с камерой и сценой. "
             "На телефоне открывается окно «Поделиться» системы.",
             "The Share view button builds a link with the camera and the "
             "scene. On a phone the system Share window opens."),
            ("Свои метки", "Own places",
             "«Мои метки» страницы хранятся в браузере. Сцена сохраняется "
             "файлом и открывается программой и модулем.",
             "My Places of the page are kept in the browser. A scene saves "
             "to a file and opens in the program and in the plugin."))))
    controls = section(
        "controls", ("Управление", "Controls"),
        ("Мышь, клавиши и пальцы", "Mouse, keys and fingers"), None,
        rows("ctl", (
            ("Левая кнопка", "Left button",
             "Захват Земли. После броска глобус вращается по инерции. "
             "Двойной щелчок - перелёт ближе.",
             "Grab the Earth. After a throw the globe keeps turning. A "
             "double click flies closer."),
            ("Колесо", "Wheel",
             "Приближение к точке под курсором, точка остаётся на месте.",
             "Zoom to the point under the cursor, the point stays in "
             "place."),
            ("Средняя кнопка", "Middle button",
             "Поворот и наклон, так же Shift с левой. Ctrl с левой сдвигает "
             "вид вдоль экрана.",
             "Turn and tilt, Shift with the left button too. Ctrl with the "
             "left button shifts the view along the screen."),
            ("N, U, R", "N, U, R",
             "Север вверху, отвесный взгляд, то и другое. Стрелки вращают "
             "глобус, Page Up и Page Down приближают.",
             "North up, a straight down view, both. The arrows turn the "
             "globe, Page Up and Page Down zoom."),
            ("Ctrl+K", "Ctrl+K",
             "Строка поиска: место, координаты или действие программы по "
             "названию.",
             "The search line: a place, coordinates or a program action by "
             "name."),
            ("F11", "F11",
             "Во весь экран для показа. Панели прячутся, навигация и шкала "
             "времени остаются.",
             "Full screen for a show. The panels hide, navigation and the "
             "time slider stay."))))
    t("dl.h1", "Windows", "Windows")
    t("dl.p1", "Установщик setup.exe или переносной zip.",
      "The setup.exe installer or a portable zip.")
    t("dl.h2", "Linux", "Linux")
    t("dl.p2", "AppImage или tar.gz для x86_64.",
      "AppImage or tar.gz for x86_64.")
    t("dl.h3", "macOS", "macOS")
    t("dl.p3", "dmg для Apple Silicon и для Intel.",
      "dmg for Apple Silicon and for Intel.")
    t("dl.note",
      "Файлы не подписаны. Система предупреждает о любой программе "
      "неизвестного издателя. В Windows - «Подробнее» и «Выполнить в любом "
      "случае», в macOS - «Системные настройки», «Конфиденциальность "
      "и безопасность», «Всё равно открыть». Контрольные суммы - "
      "SHA256SUMS в выпуске.",
      "The files are not signed. The system warns about every program from "
      "an unknown publisher. On Windows choose More info and Run anyway, on "
      "macOS System Settings, Privacy &amp; Security, Open Anyway. "
      "Checksums are in SHA256SUMS of the release.")
    dl = ('<div class="trio">%s</div><p class="sub" data-i18n="dl.note"></p>'
          % "".join('<div class="card"><h3 data-i18n="dl.h%d"></h3><p '
                    'data-i18n="dl.p%d"></p></div>' % (n, n)
                    for n in (1, 2, 3)))
    download = section(
        "download", ("Загрузка", "Download"),
        ("Программа для трёх систем", "A program for three systems"),
        ("Выпуски лежат на GitHub. Программа бесплатная и открытая, "
         "лицензия Apache 2.0.",
         "Releases are on GitHub. The program is free and open source, "
         "Apache 2.0 license."), dl + buttons(main=False))
    t("pro.a", "Страница модуля PlanetX", "The PlanetX plugin page")
    pro = section(
        "pro", ("Линейка PlanetX", "The PlanetX family"),
        ("Lite и модуль для QGIS", "Lite and the QGIS plugin"),
        ("Модуль PlanetX для QGIS правит данные, считает и готовит сцены. "
         "Lite их показывает и рассказывает. Связь между ними - файлы: "
         "сцена .planetx и KML. Сцена из модуля открывается в Lite, сцена "
         "из Lite - в модуле.",
         "The PlanetX plugin for QGIS edits data, analyses it and prepares "
         "scenes. Lite shows them and tells their story. Files connect them: "
         "the .planetx scene and KML. A scene from the plugin opens in Lite, "
         "a scene from Lite opens in the plugin."),
        '<p class="sub"><a target="_blank" href="%s" data-i18n="pro.a"></a>'
        '</p>' % PRO)
    return "\n".join([hero, tasks, idea, phone, controls, download, pro])


def page_places(img):
    head = section(
        "places", ("Метки и туры", "Places and tours"),
        ("Рисовать, измерять и показывать", "Draw, measure and present"),
        ("«Мои метки» устроены как у модуля PlanetX: папки, меню правой "
         "кнопки, перетаскивание, свойства. Метки хранятся между запусками "
         "и сохраняются в KML, KMZ и сцену .planetx.",
         "My Places work as in the PlanetX plugin: folders, a context menu, "
         "drag and drop, properties. Places are kept between runs and save "
         "to KML, KMZ and a .planetx scene."),
        figure(img("demo.jpg"), t(
            "places.fig", "Демо «Аральское море»: снимок и карта "
            "наложениями поверх подложки.", "The Aral Sea demo: an image and "
            "a map as overlays over the base map.")) + rows("pl", (
            ("Метка, путь, многоугольник, круг", "Placemark, path, polygon, "
             "circle",
             "Точки ставятся щелчком, вершины тянутся мышью. Цвет, толщина "
             "и заливка задаются в окне.",
             "Points are set with a click, vertices are dragged. Colour, "
             "width and fill are set in the window."),
            ("Линейка", "Ruler",
             "Длина, периметр и площадь на эллипсоиде WGS84, длина по "
             "рельефу, курс. Измерение сохраняется меткой.",
             "Length, perimeter and area on the WGS84 ellipsoid, the length "
             "along the terrain, the bearing. A measurement saves as a "
             "placemark."),
            ("Профиль высот", "Elevation profile",
             "График высот вдоль пути с дном моря и водой. Курсор над "
             "графиком ставит точку на глобусе.",
             "Elevations along a path with the sea floor and water. The "
             "cursor over the graph marks the point on the globe."),
            ("Разрез вниз", "Section down",
             "Стенка разреза под путём сквозь поверхность Земли, график "
             "с легендой слоёв.",
             "A section wall under a path through the Earth's surface, a "
             "graph with a layer legend."),
            ("Туры", "Tours",
             "Тур по меткам папки, вдоль пути или записанный облёт камеры. "
             "Плашка тура даёт паузу и остановку.",
             "A tour over the placemarks of a folder, along a path or a "
             "recorded camera flight. The tour bar pauses and stops it."),
            ("Шкала времени", "Time slider",
             "Метки с датами показываются по промежутку. 3D-модели едут "
             "по gx:Track, треки слоёв GeoPackage растут по датам.",
             "Placemarks with dates show by period. 3D models move along "
             "gx:Track, GeoPackage layer tracks grow by dates."),
            ("Сцена и ссылка", "Scene and link",
             "Сцена .planetx хранит камеру, метки, туры и наложения. Ссылка "
             "на вид открывает страницу с той же камерой.",
             "A .planetx scene keeps the camera, placemarks, tours and "
             "overlays. A link to a view opens the page with the same "
             "camera."))))
    own = section(
        "own", ("Свои данные", "Own data"),
        ("Файлы открываются перетаскиванием", "Files open by drag and drop"),
        ("Файл, брошенный в окно, ложится папкой в «Мои метки». "
         "Несколько файлов открываются по порядку: сначала растр высот, "
         "потом сцена над ним.",
         "A file dropped into the window becomes a folder of My Places. "
         "Several files open in order: the elevation raster first, then "
         "the scene over it."),
        rows("own", (
            (".planetx, .kml, .kmz", ".planetx, .kml, .kmz",
             "Метки, пути, многоугольники, наложения, туры, 3D-модели "
             "glTF и Collada.",
             "Placemarks, paths, polygons, overlays, tours, glTF and "
             "Collada 3D models."),
            (".gpkg", ".gpkg",
             "Слои GeoPackage метками, растущие треки по датам.",
             "GeoPackage layers as placemarks, growing tracks by dates."),
            (".tif, .tiff", ".tif, .tiff",
             "Растр высот GeoTIFF или COG, врезанный в рельеф глобуса.",
             "A GeoTIFF or COG elevation raster set into the globe "
             "terrain."),
            (".las, .laz", ".las, .laz",
             "Облако точек, в том числе COPC, в цветах файла.",
             "A point cloud, COPC too, in the colours of the file."),
            (".csv", ".csv",
             "Точки, их читает аддин WebAssembly в песочнице.",
             "Points read by a sandboxed WebAssembly add-in."))))
    return "\n".join([head, own])


def page_data(img):
    layers = section(
        "layers", ("Данные о Земле", "Earth data"),
        ("Слои каталога, как у модуля", "Catalog layers, as in the plugin"),
        ("Кнопка «Добавить данные» открывает каталог. Слои ложатся строками "
         "раздела «Слои», у каждой - флажок, непрозрачность и меню. "
         "Подпись источника стоит на экране.",
         "The Add data button opens the catalog. Layers become rows of the "
         "Layers section, each with a check box, opacity and a menu. The "
         "source credit is on the screen."),
        pair(figure(img("quakes.jpg", HALF_WIDTH), t(
                "layers.quakes", "Землетрясения ISC-GEM с 1904 года и "
                "границы плит у Японии, цвет - глубина очага.",
                "ISC-GEM earthquakes since 1904 and plate boundaries near "
                "Japan, the colour is the focal depth.")),
             figure(img("geology.jpg", HALF_WIDTH), t(
                "layers.geology", "Геологическая карта Macrostrat, запад "
                "США.", "The Macrostrat geological map, the western United "
                "States."))) + rows("dt", (
            ("Рельеф", "Relief",
             "Горизонтали, уклон и экспозиция, батиметрия, затопление "
             "и уровень моря, сток и водосборы.",
             "Contours, slope and aspect, bathymetry, flooding and sea "
             "level, drainage and watersheds.", ),
            ("Недра", "The subsurface",
             "Геология Macrostrat, границы плит и активные разломы, "
             "скорости плит, разрез Земли, подземный режим со скважинами.",
             "Macrostrat geology, plate boundaries and active faults, plate "
             "velocities, an Earth cutaway, an underground mode with "
             "boreholes."),
            ("События", "Events",
             "Землетрясения, вулканы, пожары, снимки событий до и после "
             "со шторкой.",
             "Earthquakes, volcanoes, fires, event imagery before and after "
             "with a swipe."),
            ("Вода и воздух", "Water and air",
             "Течения NOAA, дно морей под водой, огни городов ночью, все "
             "слои NASA GIBS по датам.",
             "NOAA currents, the sea floor under the water, city lights at "
             "night, all NASA GIBS layers by date."),
            ("Энергетика и города", "Energy and cities",
             "ЛЭП, трубопроводы, подводные кабели, электростанции, горные "
             "разработки, 3D-здания OpenFreeMap и Overture.",
             "Power lines, pipelines, submarine cables, power plants, mines, "
             "OpenFreeMap and Overture 3D buildings."),
            ("Космоснимки", "Satellite imagery",
             "Sentinel-2 и Landsat по месту и дате, службы WMS поверх "
             "подложки.",
             "Sentinel-2 and Landsat by place and date, WMS services over "
             "the base map."))))
    city = section(
        "city", ("Город в объёме", "A city in 3D"),
        ("3D-здания и снимки", "3D buildings and imagery"),
        ("Здания встают на рельеф по высотам OpenFreeMap и Overture Maps. "
         "Снимок и подписи ложатся под ними.",
         "Buildings stand on the terrain by OpenFreeMap and Overture Maps "
         "heights. Imagery and labels lie under them."),
        figure(img("buildings.jpg"), t(
            "city.fig", "Москва, 3D-здания у Кремля.",
            "Moscow, 3D buildings near the Kremlin.")))
    return "\n".join([layers, city])


def page_sky(img):
    bodies = section(
        "bodies", ("Другие тела", "Other bodies"),
        ("Марс, Луна и ещё 14 тел", "Mars, the Moon and 14 more bodies"),
        ("Значок «Тело» заменяет Землю другим телом по мозаикам USGS и NASA, "
         "у Марса и Луны - с рельефом. Навигация, метки, туры и линейка "
         "работают на любом теле. Земные слои на время другого тела "
         "приостанавливаются.",
         "The Body icon replaces the Earth with another body from USGS and "
         "NASA mosaics, Mars and the Moon with terrain. Navigation, places, "
         "tours and the ruler work on any body. Earth layers pause while "
         "another body is shown."),
        pair(figure(img("mars.jpg", HALF_WIDTH), t(
                "bodies.mars", "Марс, вулканы Фарсиды и долины Маринер, "
                "мозаика Viking.", "Mars, the Tharsis volcanoes and Valles "
                "Marineris, Viking mosaic.")),
             figure(img("moon.jpg", HALF_WIDTH), t(
                "bodies.moon", "Видимая сторона Луны, мозаика LROC WAC.",
                "The near side of the Moon, LROC WAC mosaic."))))
    sky = section(
        "sky", ("Звёздное небо", "The starry sky"),
        ("Небо из центра небесной сферы",
         "The sky from the centre of the celestial sphere"),
        ("Пункт «Небо» показывает звёзды, созвездия, Солнце, Луну и планеты "
         "на выбранный момент. Ближе 50° поля зрения встаёт подробная карта "
         "неба NASA, ближе 6° - снимки обзора DSS. Метки галактик, "
         "туманностей и скоплений каталогов Messier, NGC и IC открывают "
         "карточку объекта.",
         "The Sky item shows stars, constellations, the Sun, the Moon and the "
         "planets at the chosen moment. Closer than a 50° field of view the "
         "detailed NASA sky map appears, closer than 6° - the DSS survey "
         "images. Labels of galaxies, nebulae and clusters of the Messier, "
         "NGC and IC catalogs open the object card."),
        figure(img("sky.jpg"), t(
            "sky.fig", "Орион с туманностями и скоплениями каталогов NGC "
            "и IC.", "Orion with nebulae and clusters of the NGC and IC "
            "catalogs.")))
    return "\n".join([bodies, sky])


def releases():
    """Выпуски по packaging/notes: [(номер, английский, русский)],
    новые первыми."""
    folder = os.path.join(ROOT, "packaging", "notes")
    out = []
    for name in os.listdir(folder):
        m = re.match(r"(\d+)\.(\d+)\.(\d+)\.md$", name)
        if not m:
            continue
        with open(os.path.join(folder, name), encoding="utf-8") as fh:
            en, _, ru = fh.read().partition("\n---\n")
        out.append((tuple(int(v) for v in m.groups()), en.strip(),
                    ru.strip()))
    out.sort(reverse=True)
    return out


def markdown(text):
    """Описание выпуска в HTML: абзацы и списки «- », код в `...`."""
    parts = []
    for block in re.split(r"\n\s*\n", text):
        lines = block.strip().split("\n")
        if lines[0].startswith("- "):
            items = re.split(r"\n(?=- )", block.strip())
            body = "".join("<li>%s</li>" % " ".join(i[2:].split())
                           for i in items)
            parts.append("<ul>%s</ul>" % body)
        else:
            parts.append("<p>%s</p>" % " ".join(block.split()))
    out = html.escape("\n".join(parts), quote=False)
    out = re.sub(r"&lt;(/?(?:p|ul|li))&gt;", r"<\1>", out)
    out = re.sub(r"`([^`]+)`", r"<code>\1</code>", out)
    return re.sub(r"(https://\S+?)([.,]?)(?=[\s<])",
                  r'<a target="_blank" href="\1">\1</a>\2', out)


def page_news(img):
    blocks = []
    for number, en, ru in releases():
        k = "_".join(str(v) for v in number)
        body = t("news.%s" % k, markdown(ru), markdown(en))
        blocks.append('<div class="card release"><h3>PlanetX Lite %s</h3>'
                      '<div data-i18n="%s"></div></div>'
                      % (".".join(str(v) for v in number), body))
    t("news.all", "Все выпуски и файлы - на GitHub.",
      "All releases and files are on GitHub.")
    blocks.append('<p class="sub"><a target="_blank" href="%s/releases" '
                  'data-i18n="news.all"></a></p>' % REPO)
    return section("news", ("Что нового", "What's new"),
                   ("Изменения по выпускам", "Changes by release"), None,
                   "\n".join(blocks) + buttons(main=False))


def page_online(img):
    """Глобус страницы Lite в рамке, просьба автора от 28 октября
    2026 года. Рамка пускает полный экран, GPS «Моего места», буфер
    обмена и «Поделиться»."""
    t("online.full", "Открыть во весь экран", "Open full screen")
    t("online.note",
      "Глобус в рамке - та же страница, что по ссылке. Колесо внутри рамки "
      "приближает глобус, страницу сайта листает полоса справа. «Мои "
      "метки» хранятся в браузере.",
      "The globe in the frame is the same page as by the link. The wheel "
      "inside the frame zooms the globe, the scroll bar on the right "
      "scrolls the site page. My Places are kept in the browser.")
    frame = ('<div class="online"><iframe src="%s" title="PlanetX Lite" '
             'allow="fullscreen; geolocation; clipboard-write; web-share" '
             'allowfullscreen loading="lazy"></iframe></div>\n'
             '<p class="sub" data-i18n="online.note"></p>\n'
             '<div class="cta"><a class="btn btn-main" target="_blank" '
             'data-i18n="online.full" href="%s"></a></div>' % (WEB, WEB))
    return section(
        "online", ("Онлайн", "Online"),
        ("Глобус прямо на этой странице", "The globe right on this page"),
        ("Без установки: перетаскивание вращает Землю, кнопка «Демо» "
         "открывает готовые сцены.",
         "Nothing to install: dragging turns the Earth, the Demo button "
         "opens ready scenes."), frame)


BUILDERS = {"main": page_main, "online": page_online, "places": page_places,
            "data": page_data, "sky": page_sky, "news": page_news}

STYLE = """
@import url('https://fonts.googleapis.com/css2?family=Bitter:wght@500;600;700\\
&family=Golos+Text:wght@400;500;600&display=swap');
:root{
  --paper:#EEF2F6; --paper-2:#E2E8EF; --ink:#131C26; --ink-soft:#4A5866;
  --teal:#1D5FA8; --teal-deep:#12457D; --amber:#C2622C;
  --line:rgba(19,28,38,.14); --r:14px; --maxw:1080px;
  --display:'Bitter',Georgia,serif;
  --body:'Golos Text','Segoe UI',system-ui,sans-serif;
}
*{box-sizing:border-box}
html{scroll-behavior:smooth}
body{margin:0;background:var(--paper);color:var(--ink);
  font-family:var(--body);font-size:17px;line-height:1.6;
  -webkit-font-smoothing:antialiased}
.wrap{max-width:var(--maxw);margin:0 auto;padding:0 24px}
a{color:inherit}
h1,h2,h3{font-family:var(--display);font-weight:700;line-height:1.1;margin:0;
  letter-spacing:-.01em}
p{margin:0}
code{font-size:.92em}
.eyebrow{font-size:13px;font-weight:600;letter-spacing:.16em;
  text-transform:uppercase;color:var(--teal-deep);margin-bottom:18px;
  display:inline-flex;gap:10px;align-items:center}
.eyebrow::before{content:"";width:26px;height:2px;background:var(--amber);
  display:inline-block}
.hdr{position:sticky;top:0;z-index:20;background:rgba(238,242,246,.86);
  backdrop-filter:blur(8px);border-bottom:1px solid var(--line)}
.hdr .wrap{display:flex;align-items:center;justify-content:space-between;
  height:62px;gap:18px}
.brand{display:flex;align-items:baseline;gap:10px;
  font-family:var(--display);font-weight:700;text-decoration:none;
  font-size:20px}
.brand span{font-family:var(--body);font-weight:500;font-size:13px;
  color:var(--ink-soft)}
.right{display:flex;align-items:center;gap:22px}
.nav{display:flex;gap:22px;font-size:15px}
.nav a{text-decoration:none;color:var(--ink-soft)}
.nav a:hover{color:var(--teal-deep)}
.nav a.on{color:var(--teal-deep);font-weight:600}
@media(max-width:860px){.nav{display:none}}
.lang{display:flex;border:1px solid var(--line);border-radius:10px;
  overflow:hidden;font-size:13px;font-weight:600}
.lang button{border:0;background:transparent;padding:6px 11px;cursor:pointer;
  font:inherit;color:var(--ink-soft)}
.lang button.on{background:var(--teal);color:#fff}
.hero{padding:74px 0 48px}
.hero h1{font-size:clamp(34px,5vw,56px);max-width:20ch}
.hero .lead{margin-top:22px;font-size:20px;max-width:64ch;
  color:var(--ink-soft)}
.cta{margin-top:34px;display:flex;gap:14px;flex-wrap:wrap}
.btn{display:inline-block;padding:13px 22px;border-radius:var(--r);
  text-decoration:none;font-weight:600;font-size:16px}
.btn-main{background:var(--teal);color:#fff}
.btn-main:hover{background:var(--teal-deep)}
.btn-ghost{border:1px solid var(--line);color:var(--ink)}
.btn-ghost:hover{border-color:var(--teal)}
section{padding:56px 0;border-top:1px solid var(--line)}
section h2{font-size:clamp(26px,3.3vw,38px);max-width:24ch}
section .sub{margin-top:16px;max-width:70ch;color:var(--ink-soft)}
section .sub a{color:var(--teal-deep)}
.pair{display:grid;grid-template-columns:1fr 1fr;gap:28px;margin-top:34px}
.trio{display:grid;grid-template-columns:repeat(3,1fr);gap:24px;
  margin-top:34px}
@media(max-width:860px){.pair,.trio{grid-template-columns:1fr}}
.pair figure,.trio figure{margin:0}
.card{background:var(--paper-2);border:1px solid var(--line);
  border-radius:var(--r);padding:22px 24px}
.card h3{font-size:19px;margin-bottom:10px}
.card p{color:var(--ink-soft);font-size:16px}
.card p + p{margin-top:10px}
.card a{color:var(--teal-deep)}
.card.link{display:block;text-decoration:none;color:inherit}
.card.link:hover{border-color:var(--teal)}
.card.link h3{color:var(--teal-deep)}
figure{margin:30px 0 0}
figure img{width:100%;height:auto;display:block;border-radius:var(--r);
  border:1px solid var(--line);background:#000}
.shot{margin-top:38px}
figcaption{margin-top:10px;font-size:14px;color:var(--ink-soft)}
.tools{margin-top:34px;border:1px solid var(--line);border-radius:var(--r);
  overflow:hidden;background:#fff}
.row{display:grid;grid-template-columns:minmax(220px,1fr) 1.4fr;gap:14px;
  padding:12px 20px;border-bottom:1px solid var(--line);font-size:15px}
.row:last-child{border-bottom:0}
.row span{color:var(--ink-soft)}
@media(max-width:860px){.row{grid-template-columns:1fr;gap:4px}}
.online{margin-top:30px;height:min(78vh,760px);border-radius:var(--r);
  overflow:hidden;border:1px solid var(--line);background:#000}
.online iframe{width:100%;height:100%;border:0;display:block}
.release{margin-top:20px}
.release ul{margin:10px 0;padding-left:20px;color:var(--ink-soft);
  font-size:16px}
.release li + li{margin-top:6px}
.release p{color:var(--ink-soft);font-size:16px;margin-top:10px}
.ftr{padding:40px 0 60px;border-top:1px solid var(--line);
  color:var(--ink-soft);font-size:15px}
.ftr a{color:var(--teal-deep)}
"""

SCRIPT = """
function apply(lang){
  var d = TEXTS[lang] || TEXTS.ru;
  document.documentElement.lang = lang;
  document.querySelectorAll('[data-i18n]').forEach(function(el){
    var value = d[el.getAttribute('data-i18n')];
    if (value !== undefined) el.innerHTML = value;
  });
  document.querySelectorAll('[data-i18n-href]').forEach(function(el){
    var value = d[el.getAttribute('data-i18n-href')];
    if (value !== undefined) el.setAttribute('href', value);
  });
  document.querySelectorAll('.lang button').forEach(function(b){
    b.classList.toggle('on', b.getAttribute('data-lang') === lang);
  });
  try { localStorage.setItem('planetx-lang', lang); } catch (e) {}
}
document.querySelectorAll('.lang button').forEach(function(b){
  b.addEventListener('click', function(){ apply(b.getAttribute('data-lang')); });
});
// По умолчанию русский: страница живёт на русском сайте.
var saved = null;
try { saved = localStorage.getItem('planetx-lang'); } catch (e) {}
apply(saved || 'ru');
"""


def page(key, img, version):
    T.clear()
    body = BUILDERS[key](img)
    title_ru = dict((k, ru) for k, _, ru, _ in PAGES)[key]
    title_en = dict((k, en) for k, _, _, en in PAGES)[key]
    t("title", "PlanetX Lite - %s · Информ++" % title_ru,
      "PlanetX Lite - %s · Inform++" % title_en)
    t("brand.sub", "глобус без QGIS", "globe without QGIS")
    nav = []
    for k, path, ru, en in PAGES:
        nk = t("nav.%s" % k, ru, en)
        nav.append('<a target="_blank" href="%s" data-i18n="%s"%s></a>'
                   % (url(path), nk, ' class="on"' if k == key else ""))
    t("ftr.line1", "Лицензия Apache 2.0", "Apache 2.0 license")
    t("ftr.line2", "ООО «Информ++»", "Inform++ LLC")
    t("ftr.line3", "Программа бесплатная и открытая.",
      "The program is free and open source.")
    t("ftr.support", "Поддержать разработку", "Support development")
    t("ftr.pro", "Модуль PlanetX для QGIS", "The PlanetX plugin for QGIS")
    texts = {"ru": {k: v[0] for k, v in T.items()},
             "en": {k: v[1] for k, v in T.items()}}
    return """<!-- PlanetX Lite - страница «%s» для www.informpp.ru.   -->
<!-- Собирается tools/build_pages.py, править лучше его.        -->
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title data-i18n="title"></title>
<style>%s</style>
<header class="hdr"><div class="wrap">
<a class="brand" target="_blank" href="%s">PlanetX Lite
<span data-i18n="brand.sub"></span></a>
<div class="right"><nav class="nav">%s</nav>
<div class="lang"><button type="button" data-lang="ru">RU</button>
<button type="button" data-lang="en">EN</button></div></div>
</div></header>
%s
<footer class="ftr"><div class="wrap">
PlanetX Lite %s &middot; <span data-i18n="ftr.line1"></span><br>
<span data-i18n="ftr.line2"></span>,
<a target="_blank" href="https://www.informpp.ru/">www.informpp.ru</a><br>
<span data-i18n="ftr.line3"></span>
<a target="_blank" href="%s" data-i18n="ftr.support"></a><br>
<a target="_blank" href="%s" data-i18n="ftr.pro"></a>
</div></footer>
<script>
var TEXTS = %s;
%s</script>
""" % (title_ru, STYLE, url(""), "\n".join(nav), body, version, SUPPORT_URL,
       PRO, json.dumps(texts, ensure_ascii=False), SCRIPT)


def version():
    """Версия рабочего пространства из Cargo.toml."""
    with open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8") as fh:
        text = fh.read()
    m = re.search(r'\[workspace\.package\][^\[]*?version\s*=\s*"([^"]+)"',
                  text)
    return m.group(1) if m else ""


def shot(name, width):
    """Снимок site/shots в строке data - JPEG не шире width."""
    with Image.open(os.path.join(SHOTS, name)) as picture:
        picture = picture.convert("RGB")
        if picture.width > width:
            height = round(picture.height * width / picture.width)
            picture = picture.resize((width, height), Image.LANCZOS)
        out = io.BytesIO()
        picture.save(out, "JPEG", quality=WEB_QUALITY, optimize=True,
                     progressive=True)
    return "data:image/jpeg;base64," + base64.b64encode(
        out.getvalue()).decode("ascii")


def main():
    os.makedirs(OUT, exist_ok=True)
    cache = {}

    def img(name, width=WEB_WIDTH):
        if (name, width) not in cache:
            cache[name, width] = shot(name, width)
        return cache[name, width]

    number = version()
    for key, path, _, _ in PAGES:
        text = page(key, img, number)
        target = os.path.join(OUT, "planetx_lite_%s.html" % key)
        with open(target, "wb") as fh:
            fh.write(text.encode("utf-8"))
        print("%-7s %6.0f КБ  %s" % (key, len(text.encode("utf-8")) / 1024,
                                    url(path)))


if __name__ == "__main__":
    main()
