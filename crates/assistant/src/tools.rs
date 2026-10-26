// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Инструменты помощника: имя, описание для модели и схема аргументов
//! JSON Schema. Описания - для модели, на русском, дословно как в модуле.

use serde_json::{Value, json};

/// Инструмент для модели.
#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub name: &'static str,
    pub description: String,
    pub schema: Value,
}

/// Имена инструментов в порядке [`tools`].
pub const TOOL_NAMES: [&str; 19] = [
    "fly_to",
    "search_place",
    "set_body",
    "set_layer",
    "set_time",
    "earth_cutaway",
    "section_down",
    "point_info",
    "quakes_summary",
    "add_kml",
    "list_actions",
    "run_action",
    "set_opacity",
    "add_basemap",
    "get_kml",
    "search_docs",
    "read_doc",
    "list_project_layers",
    "read_features",
];

fn obj(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required})
}

fn tool(name: &'static str, description: &str, schema: Value) -> Tool {
    Tool { name, description: description.to_string(), schema }
}

/// Инструменты помощника. `ready_keys` - ключи готовых подложек
/// для описания add_basemap, в Lite это `ui::basemap::READY`, в модуле -
/// `core.basemap.READY_KEYS`.
pub fn tools(ready_keys: &[&str]) -> Vec<Tool> {
    let number = json!({"type": "number"});
    let string = json!({"type": "string"});
    let boolean = json!({"type": "boolean"});
    let points = json!({"type": "array", "items": {"type": "array", "items": number, "minItems": 2, "maxItems": 2},
        "description": "Точки [широта, долгота] в градусах."});
    vec![
        tool(
            "fly_to",
            concat!("Перелёт к точке. distance_km - расстояние камеры до точки ", "взгляда, по умолчанию 500 км."),
            obj(json!({"lat": number, "lon": number, "distance_km": number}), &["lat", "lon"]),
        ),
        tool("search_place", "Найти место по названию и подлететь к нему.", obj(json!({"query": string}), &["query"])),
        tool(
            "set_body",
            concat!("Показать тело: earth, mars, moon, sky или другое тело ", "из списка body_keys контекста."),
            obj(json!({"body": string}), &["body"]),
        ),
        tool(
            "set_layer",
            concat!("Включить или выключить строку раздела «Слои» по ключу ", "из списка layer_keys контекста."),
            obj(json!({"key": string, "on": boolean}), &["key", "on"]),
        ),
        tool(
            "set_time",
            concat!("Открыть шкалу времени на промежуток дат ISO 8601 или ", "закрыть её (start и end пустые)."),
            obj(json!({"start": string, "end": string}), &[]),
        ),
        tool(
            "earth_cutaway",
            concat!(
                "Вынуть из Земли сектор с гранями коры, мантии, ",
                "ядра и плит: долготы west, east и широты south, north в градусах. ",
                "off=true убирает сектор."
            ),
            obj(json!({"west": number, "east": number, "south": number, "north": number, "off": boolean}), &[]),
        ),
        tool(
            "section_down",
            concat!("Разрез Земли вниз вдоль пути по точкам до глубины ", "depth_km (100, 300, 700, 2891 или 6371)."),
            obj(json!({"points": points, "depth_km": number}), &["points"]),
        ),
        tool(
            "point_info",
            concat!("Сведения о точке: высота или глубина моря, Мохо, ", "толщина коры и осадков (CRUST1.0), плита Slab2 под точкой."),
            obj(json!({"lat": number, "lon": number}), &["lat", "lon"]),
        ),
        tool(
            "quakes_summary",
            concat!(
                "Землетрясения за 30 суток (сводка USGS, M4.5+) ",
                "в рамке south, north, west, east: количество, самые сильные ",
                "и самые глубокие."
            ),
            obj(json!({"south": number, "north": number, "west": number, "east": number}), &["south", "north", "west", "east"]),
        ),
        tool(
            "add_kml",
            concat!(
                "Предложить пользователю документ KML 2.2 для «Моих ",
                "меток»: Placemark с Point, LineString, Polygon, Folder, Style, ",
                "LookAt, TimeStamp и TimeSpan, gx:Tour. Координаты KML - долгота, ",
                "широта, высота. Пользователь видит состав документа и подтверждает ",
                "запись сам, документ ложится новой папкой с именем документа."
            ),
            obj(json!({"kml": string}), &["kml"]),
        ),
        tool(
            "list_actions",
            concat!(
                "Список действий модуля: слои и строки раздела ",
                "«Слои», карты каталога, окна и инструменты, демо, тела. Строка - ",
                "ключ, название и что сделает действие (включить, выключить, ",
                "показать карту, открыть окно). Состав зависит от тела и того, что ",
                "включено, поэтому список берётся перед run_action."
            ),
            obj(json!({}), &[]),
        ),
        tool(
            "run_action",
            concat!("Выполнить действие по ключу из list_actions. ", "Действие строки слоя переключает её - включённая выключится."),
            obj(json!({"key": string}), &["key"]),
        ),
        tool(
            "set_opacity",
            concat!("Непрозрачность включённого слоя от 0 до 1 по ключу ", "строки (как в layers_on) или map для карты каталога."),
            obj(json!({"key": string, "value": number}), &["key", "value"]),
        ),
        Tool {
            name: "add_basemap",
            description: format!(
                "{}{}{}",
                "Поставить подложку глобуса: ready - ключ готового источника (",
                ready_keys.join(", "),
                concat!(
                    "), или url - шаблон XYZ ",
                    "с {z}, {x}, {y} ({-y} - TMS, {q} - квадрант Bing) и name - его ",
                    "название. Подложку подтверждает пользователь, у сервисов, чьи ",
                    "условия не разрешают такой доступ, окно его предупреждает."
                )
            ),
            schema: obj(json!({"ready": string, "url": string, "name": string}), &[]),
        },
        tool(
            "get_kml",
            concat!(
                "Получить KML: вид камеры (LookAt) и метки, пути, ",
                "многоугольники и папки, которые пользователь выделил в «Моих ",
                "метках». Невыделенные метки недоступны. Так помощник читает ",
                "и правит метки пользователя: правка возвращается через add_kml."
            ),
            obj(json!({}), &[]),
        ),
        tool(
            "search_docs",
            concat!(
                "Найти разделы руководства PlanetX по словам ",
                "вопроса. Ответ - якорь, название и абзац раздела. Как сделать что-то ",
                "в модуле, где пункт меню, что значит настройка - сначала сюда."
            ),
            obj(json!({"query": string}), &["query"]),
        ),
        tool(
            "read_doc",
            concat!(
                "Прочитать раздел руководства по якорю из search_docs: ",
                "строки с offset, не больше limit (по умолчанию и не больше 120)."
            ),
            obj(json!({"anchor": string, "offset": number, "limit": number}), &["anchor"]),
        ),
        tool(
            "list_project_layers",
            concat!(
                "Слои проекта QGIS: номер, название, вид, ",
                "система координат, количество объектов, поля. Каждый вызов ",
                "разрешает пользователь."
            ),
            obj(json!({}), &[]),
        ),
        tool(
            "read_features",
            concat!(
                "Прочитать строки векторного слоя проекта по номеру ",
                "из list_project_layers: where - выражение QGIS отбора, fields - ",
                "поля, limit - не больше 200 строк, geometry - добавить точку ",
                "середины объекта. Каждый вызов разрешает пользователь."
            ),
            obj(
                json!({"layer": string, "where": string, "fields": {"type": "array", "items": string}, "limit": number, "geometry": boolean}),
                &["layer"],
            ),
        ),
    ]
}
