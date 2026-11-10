# Как написать аддин для PlanetX Lite

[English](README.md)

Аддин добавляет в PlanetX Lite команду. Команда берёт объекты слоя
«Моих слоёв» и отдаёт новые объекты, Lite кладёт их новым слоем. Один
и тот же аддин работает в программе для Windows, Linux и macOS и на
странице https://valery35.github.io/planetx-lite/.

Руководство по шагам собирает полный аддин «Средняя точка». Второй
образец - [`buffer`](buffer) в этой папке, буфер вокруг точки, линии
или полигона. Оба проверены песочницей Lite.

## Что такое аддин

- Модуль WebAssembly для `wasm32-unknown-unknown` и описание
  `addin.json`. Вместе они упакованы в файл `.pxaddin`, это ZIP.
- Модуль работает в песочнице wasmi. Импортов у него нет - ни файлов,
  ни сети, ни часов, ни потоков. Lite пишет запрос в его память
  и читает ответ оттуда же.
- Права аддин называет в `addin.json`. Права первой версии - `features`
  (получать объекты, выбранные пользователем) и `layers` (класть
  результат в «Мои слои»). Команде нужны оба права. Lite показывает
  права перед установкой.
- У каждого вызова есть предел шагов и памяти. Бесконечный цикл
  кончается ошибкой, программа и страница продолжают работу.
- Годится любой язык, который собирает модуль WebAssembly без
  импортов. Руководство берёт Rust и крейт `addin-api` из Lite. Крейт
  читает и пишет двоичные форматы.

## 1. Инструменты

- Rust (https://rustup.rs) и цель WebAssembly:

  ```
  rustup target add wasm32-unknown-unknown
  ```

- Python 3 для скрипта упаковки из каталога, любая сборка без
  дополнительных модулей.
- PlanetX Lite 0.28.0 или новее для пробы аддина.

## 2. Новый крейт

```
cargo new --lib mean-point
```

`Cargo.toml`:

```toml
[package]
name = "mean-point"
version = "0.1.0"
edition = "2024"
license = "Apache-2.0"

[lib]
crate-type = ["cdylib", "rlib"]   # cdylib - модуль, rlib - для тестов

[dependencies]
addin-api = { git = "https://github.com/Valery35/planetx-lite", tag = "v0.28.0" }
serde_json = "1"

[dev-dependencies]
# Песочница Lite для теста собранного модуля.
addin-host = { git = "https://github.com/Valery35/planetx-lite", tag = "v0.28.0" }

[profile.release]
opt-level = "s"   # модуль меньше
lto = true
panic = "abort"   # паника кончает вызов ошибкой
```

## 3. Описание `addin.json`

```json
{
  "id": "mean-point",
  "version": "0.1.0",
  "author": "Your Name",
  "license": "Apache-2.0",
  "title": {"ru": "Средняя точка", "en": "Mean point"},
  "description": {"ru": "Точка в среднем вершин объекта.", "en": "A point at the mean of the vertices of an object."},
  "permissions": ["features", "layers"],
  "commands": [{
    "id": "mean",
    "title": {"ru": "Средняя точка…", "en": "Mean point…"},
    "places": ["layer"],
    "kinds": ["point", "line", "polygon"],
    "params": [
      {"id": "height", "type": "number", "unit": "m", "title": {"ru": "Высота", "en": "Height"}, "default": 0, "min": -500, "max": 9000},
      {"id": "label", "type": "bool", "title": {"ru": "Название объекта", "en": "Object name"}, "default": true}
    ]
  }]
}
```

| Поле | Смысл |
|---|---|
| `id` | Ключ аддина - латиница, цифры, `-`, `_`, `.`, до 64 знаков. Один id - один аддин в каталоге |
| `version` | Версия вида `старшая.младшая.правка`. Версия выше в каталоге предлагается обновлением |
| `author`, `license` | Видны в окне «Аддины» |
| `title`, `description` | Тексты на русском и английском. Без одного языка показывается другой |
| `permissions` | `features` и `layers`. Неизвестное право - отказ в установке |
| `commands` | Команды, до 64 |

Команда:

| Поле | Смысл |
|---|---|
| `id` | Ключ команды, он приходит в модуль с запросом |
| `title` | Пункт меню слоя. Пункт открывает окно параметров с кнопкой «Выполнить», поэтому `…` в конце уместно |
| `places` | Место пункта. `layer` и `object` пока ставят его в меню слоя «Моих слоёв», `object` оставлен для меню объекта следующих версий |
| `kinds` | Геометрии, которые команда принимает - `point`, `line`, `polygon`. Пусто - все |
| `params` | Параметры, до 64. Окно параметров рисует сама Lite |

Параметры:

| `type` | Поля | Значение в запросе |
|---|---|---|
| `number` | `unit` (`m`, `km`, `deg` и другие), `default`, `min`, `max` | число в пределах `min`..`max` |
| `text` | `default` | строка |
| `bool` | `default` | `true` или `false` |
| `choice` | `options` - список `{"id", "title"}`, `default` - id варианта | id варианта |

У каждого параметра есть `id` и `title`. Запрос несёт значения одним
объектом JSON, `{"height": 0, "label": true}`. Первый числовой параметр
с единицей входит в название слоя результата, например «Дороги ·
Буфер 100 м».

## 4. Модуль `src/lib.rs`

```rust
//! Аддин «Средняя точка»: точка в среднем вершин каждого объекта.

use std::sync::Mutex;

use addin_api::{ABI_VERSION, Feature, Kind, decode_request, encode_features};

/// Описание - тот же addin.json, что лежит в пакете.
const MANIFEST: &str = include_str!("../addin.json");

static INPUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());
static OUTPUT: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// Положить байты в выход, ответ - их длина.
fn put(bytes: Vec<u8>) -> i32 {
    let len = i32::try_from(bytes.len()).unwrap_or(i32::MAX);
    if let Ok(mut out) = OUTPUT.lock() {
        *out = bytes;
    }
    len
}

#[unsafe(no_mangle)]
pub extern "C" fn pxa_abi() -> i32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn pxa_manifest() -> i32 {
    put(MANIFEST.as_bytes().to_vec())
}

#[unsafe(no_mangle)]
pub extern "C" fn pxa_input(len: i32) -> i32 {
    let (Ok(n), Ok(mut input)) = (usize::try_from(len), INPUT.lock()) else {
        return 0;
    };
    *input = vec![0; n.max(1)];
    input.as_ptr() as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn pxa_output() -> i32 {
    OUTPUT.lock().map_or(0, |out| out.as_ptr() as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn pxa_command(len: i32) -> i32 {
    let request = match INPUT.lock() {
        Ok(input) => decode_request(input.get(..usize::try_from(len).unwrap_or(0)).unwrap_or(&[])),
        Err(_) => Err("нет входа".to_string()),
    };
    match request.and_then(|(command, params, features)| run(&command, &params, &features)) {
        Ok(features) => put(encode_features(&features)),
        Err(e) => -put(e.into_bytes()).max(1),
    }
}

/// Сама команда - обычный Rust, тесты идут без WebAssembly.
pub fn run(command: &str, params: &str, features: &[Feature]) -> Result<Vec<Feature>, String> {
    if command != "mean" {
        return Err(format!("нет команды {command}"));
    }
    let p: serde_json::Value = serde_json::from_str(params).map_err(|e| e.to_string())?;
    let height = p.get("height").and_then(serde_json::Value::as_f64).unwrap_or(0.0);
    let label = p.get("label").and_then(serde_json::Value::as_bool).unwrap_or(true);
    let mut out = Vec::new();
    for f in features {
        let v: Vec<&[f64; 3]> = f.parts.iter().flatten().collect();
        if v.is_empty() {
            continue;
        }
        let n = v.len() as f64;
        let lat = v.iter().map(|x| x[0]).sum::<f64>() / n;
        let lon = v.iter().map(|x| x[1]).sum::<f64>() / n;
        let name = if label { f.name.clone() } else { String::new() };
        out.push(Feature { kind: Kind::Point, parts: vec![vec![[lat, lon, height]]], name });
    }
    Ok(out)
}
```

Простое среднее долгот неверно для объекта через 180-й меридиан.
Образец `buffer` показывает, как это обойти.

### Экспорты

| Экспорт | Что ждёт Lite |
|---|---|
| `memory` | Память модуля, Rust экспортирует её сам |
| `pxa_abi() -> i32` | Версия интерфейса, `2` для команд |
| `pxa_manifest() -> i32` | Кладёт текст `addin.json` в выход и отвечает его длиной. Id и версия должны совпасть с `addin.json` пакета, права - не выходить за права пакета |
| `pxa_input(len) -> i32` | Даёт место под `len` байт запроса и отвечает его адресом, `0` - места нет |
| `pxa_command(len) -> i32` | Читает запрос длиной `len`, кладёт результат в выход. Ответ - длина результата или длина текста ошибки со знаком минус |
| `pxa_output() -> i32` | Адрес выхода |

Порядок вызова - `pxa_input`, Lite пишет запрос, `pxa_command`,
`pxa_output`, Lite читает ответ.

### Объекты

`addin_api::Feature` - один объект:

- `kind` - `Kind::Point`, `Kind::Line` или `Kind::Polygon`.
- `parts` - части геометрии, вершина - `[широта, долгота, высота]`,
  градусы WGS84 и метры.
- `name` - название объекта, до 1024 байт.

Части точки - отдельные вершины, части линии - ломаные. Первая часть
полигона - внешнее кольцо, остальные части - его дыры. Кольцо не
повторяет первую вершину в конце. Мультиполигон слоя приходит
несколькими объектами-полигонами с одним названием.

Объекты запроса - выделенные объекты слоя, без выделения - все его
объекты. Результат может нести любые виды и любое количество объектов.
Названия результата Lite хранит в поле `name`. Высоты результата пока
не используются.

`addin_api` проверяет каждый запрос и каждый результат. Обрезанные
данные, вершина вне широты ±90 и не конечное число - ошибка.

## 5. Тесты

Тесту `run` WebAssembly не нужен:

```rust
#[cfg(test)]
mod tests {
    use addin_api::{Feature, Kind};

    #[test]
    fn mean_of_a_square() {
        let square = Feature { kind: Kind::Polygon, parts: vec![vec![[0.0, 0.0, 0.0], [0.0, 2.0, 0.0], [2.0, 2.0, 0.0], [2.0, 0.0, 0.0]]], name: "A".into() };
        let out = super::run("mean", r#"{"height":10,"label":true}"#, &[square]).unwrap();
        assert_eq!(out[0].parts[0][0], [1.0, 1.0, 10.0]);
        assert_eq!(out[0].name, "A");
    }
}
```

Собранный модуль проходит ту же песочницу и те же проверки, что
в Lite, `tests/sandbox.rs`:

```rust
use addin_api::{Feature, Kind};
use addin_host::{Addin, Package};

#[test]
fn module_runs_in_the_sandbox() {
    let bytes = std::fs::read("target/wasm32-unknown-unknown/release/mean_point.wasm").unwrap();
    // Те же проверки, что при установке - id, версия, права, команды.
    let package = Package::from_parts(include_str!("../addin.json"), bytes.clone()).unwrap();
    assert_eq!(package.manifest.id, "mean-point");
    let mut addin = Addin::load(&bytes).unwrap();
    let line = Feature { kind: Kind::Line, parts: vec![vec![[55.0, 37.0, 0.0], [56.0, 38.0, 0.0]]], name: "L".into() };
    let params = serde_json::json!({"height": 0, "label": true});
    let out = addin.run_command("mean", &params, &[line]).unwrap();
    assert_eq!(out[0].parts[0][0], [55.5, 37.5, 0.0]);
}
```

```
cargo build --release --target wasm32-unknown-unknown
cargo test
```

Модуль - `target/wasm32-unknown-unknown/release/mean_point.wasm`,
около 90 КБ.

## 6. Пакет

Скрипт `pack.py` каталога
[planetx-lite-addins](https://github.com/Valery35/planetx-lite-addins)
упаковывает описание и модуль:

```
python scripts/pack.py path/to/addin.json path/to/mean_point.wasm packages/
```

Скрипт пишет `packages/mean-point-0.1.0.pxaddin` и выводит его SHA-256.
Пакет при каждом запуске одинаков до байта.

## 7. Проба в Lite

1. Пакет открывается кнопкой «Открыть», броском на окно или через
   Поиск ▸ **Аддины** ▸ **Из файла…**. Страница Lite принимает пакет
   так же.
2. Окно показывает название, автора, лицензию и права, с предупреждением,
   что каталог аддин не проверял. Кнопка **Установить**.
3. В «Мои слои» открывается слой - GeoPackage, Shapefile, GeoJSON, GPX,
   MapInfo, FlatGeobuf или DXF.
4. В меню **⋯** слоя появляется пункт **Средняя точка…**. Окно
   параметров показывает слой и количество объектов. Кнопка
   **Выполнить**.
5. Результат - новый слой «<слой> · Средняя точка 0 м». Ошибка аддина
   видна в строке состояния.

Программа хранит установленные аддины в папке своих данных, страница -
в хранилище браузера. Вкладка **Установленные** выключает, включает
и удаляет аддин.

## 8. Публикация в каталоге

Каталог -
[planetx-lite-addins](https://github.com/Valery35/planetx-lite-addins).
Lite читает его по адресу
https://valery35.github.io/planetx-lite-addins/registry.json.

1. Пакет `.pxaddin` выходит выпуском в своём хранилище автора.
2. Запрос на слияние в каталог добавляет запись `registry/<id>.json`:

   ```json
   {
     "id": "mean-point",
     "version": "0.1.0",
     "package": "https://github.com/you/mean-point/releases/download/v0.1.0/mean-point-0.1.0.pxaddin",
     "sha256": "…",
     "source": "https://github.com/you/mean-point"
   }
   ```

3. Сборка каталога скачивает пакет и сверяет его SHA-256, id и версию
   с записью, проверяет права. После слияния пакет копируется
   на GitHub Pages.
4. Обновление - новый выпуск и запрос на слияние с новыми `version`,
   `package` и `sha256`. Lite предлагает его на вкладке
   **Установленные**.

Пакет из каталога ставится, только если его SHA-256 совпадает
с каталогом. Аддин, снятый в `blocklist.json` (`id` или `id@version`),
Lite выключает.

## Пределы

| Что | Предел |
|---|---|
| Модуль | 32 МБ |
| `addin.json` | 256 КБ |
| Память модуля | 512 МБ |
| Запрос | 64 МБ |
| Параметры JSON | 64 КБ |
| Имя команды | 256 байт |
| Название объекта | 1024 байта |
| Текст описания | 2000 знаков |
| Команды, параметры, варианты | по 64 |
| Шаги одной команды | 20 миллиардов и 200 тысяч на байт запроса |

## Советы

- Параметры стоит проверять и в `run`. Окно держит `min` и `max`, но
  запрос - данные извне.
- Ошибка - короткий текст для пользователя, без стека.
- Паника кончает вызов ошибкой, Lite загружает модуль заново. Лучше
  вернуть ошибку.
- Команда должна быть быстрой. Вызов идёт в кадре Lite, долгая команда
  держит окно.
- Без новых прав аддину не прочесть ни файлы, ни сеть. Права сверх
  `features` и `layers` придут с новыми версиями Lite.

## Интерфейс в байтах

Для языка без `addin-api`. Все числа - little-endian.

Запрос:

```
"PXQ1"
u16 длина + UTF-8   - id команды
u32 длина + UTF-8   - параметры, JSON
объекты
```

Объекты (запрос и результат):

```
"PXG1"
u32 количество
  u8 вид             - 1 точка, 2 линия, 3 полигон
  u16 длина + UTF-8  - название
  u32 количество частей
    u32 количество вершин
      f64 широта, f64 долгота, f64 высота
```

После объектов байтов нет. ABI 1 (`pxa_read`, файл в точки) описан
в документации [`addin-api`](../crates/addin-api/src/lib.rs).
