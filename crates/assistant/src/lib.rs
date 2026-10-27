// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! ИИ-помощник: инструменты, запросы и разбор ответов моделей, поток
//! SSE, документ KML из ответа и круг разговора. Без сети, потоков
//! и интерфейса, собирается и под wasm32.
//!
//! Перенос `core/assistant.py` модуля PlanetX для QGIS вместе с тестами
//! `tests/test_assistant.py` и `tests/test_assistant_agent.py`.
//! Подключения - Anthropic Messages, OpenAI Responses (xAI) и OpenAI
//! Chat Completions (OpenRouter, Ollama). Тексты для модели - описания
//! инструментов, системные подсказки и `KML_RULES` - совпадают с модулем
//! дословно.
//!
//! Разговор - список [`Message`]. `build_*` переводит его в тело запроса
//! подключения, `parse_*` разбирает ответ в [`Reply`]. Круг разговора
//! ведёт [`Agent`] - окно только посылает запрос, исполняет вызов
//! и спрашивает пользователя.

mod agent;
pub mod mcp;
mod provider;
mod py;
mod stream;
mod tools;

pub use agent::{Agent, CANCELLED, Choice, DENIED, Gate, STOPPED, Step, StepKind};
pub use provider::{
    Options, Request, build_anthropic, build_chat, build_responses, chat_error, needs_key, parse, parse_anthropic, parse_chat,
    parse_responses, request,
};
pub use stream::{StreamText, extract_kml, kml_progress, length_refused};
pub use tools::{TOOL_NAMES, Tool, tools};

use serde_json::Value;

// Форматы запросов. ANTHROPIC и RESPONSES - ещё и имена подключений
// Claude и Grok, так они хранятся в настройках модуля.

/// Формат и подключение Anthropic Messages.
pub const ANTHROPIC: &str = "anthropic";
/// Формат OpenAI Responses и подключение Grok (xAI).
pub const RESPONSES: &str = "responses";
/// Формат OpenAI Chat Completions: OpenRouter, Ollama и другие.
pub const CHAT: &str = "chat";
/// Подключение DeepSeek, формат Anthropic.
pub const DEEPSEEK: &str = "deepseek";
/// Подключение OpenRouter, формат Chat.
pub const OPENROUTER: &str = "openrouter";
/// Сервис на своём компьютере (Ollama), формат Chat.
pub const LOCAL: &str = "local";
/// Подключения по порядку.
pub const PROVIDERS: [&str; 5] = [ANTHROPIC, RESPONSES, DEEPSEEK, OPENROUTER, LOCAL];
/// Формат запросов подключения. DeepSeek принимает формат Anthropic
/// по адресу .../anthropic.
pub const FORMAT: [(&str, &str); 5] =
    [(ANTHROPIC, ANTHROPIC), (RESPONSES, RESPONSES), (DEEPSEEK, ANTHROPIC), (OPENROUTER, CHAT), (LOCAL, CHAT)];

/// Формат запросов подключения. Неизвестное подключение - само себе
/// формат, как `FORMAT.get(provider, provider)` модуля.
pub fn format_of(provider: &str) -> &str {
    FORMAT.iter().find(|(p, _)| *p == provider).map_or(provider, |(_, f)| f)
}

/// Подключения по умолчанию: подключение, адрес и модель. Выбор
/// помощника модуля, модели утверждает автор.
pub const DEFAULTS: [(&str, &str, &str); 5] = [
    (ANTHROPIC, "https://api.anthropic.com", "claude-sonnet-5-5"),
    (RESPONSES, "https://api.x.ai/v1", "grok-4.7"),
    (DEEPSEEK, "https://api.deepseek.com/anthropic", "deepseek-flash"),
    (OPENROUTER, "https://openrouter.ai/api/v1", "openrouter/free"),
    (LOCAL, "http://localhost:11434/v1", "qwen3"),
];

/// Адрес и модель подключения по умолчанию.
pub fn defaults(provider: &str) -> Option<(&'static str, &'static str)> {
    DEFAULTS.iter().find(|(p, _, _)| *p == provider).map(|&(_, base, model)| (base, model))
}

/// Узлы своего компьютера - сервису на них ключ не нужен.
pub const LOCAL_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];
/// Заголовок anthropic-version.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Предел длины ответа, токенов.
pub const MAX_TOKENS: u64 = 2048;
/// Предел формата Chat: модели OpenRouter и Ollama часто рассуждают
/// перед ответом, рассуждение входит в предел.
pub const CHAT_MAX_TOKENS: u64 = 8192;
/// OpenRouter: короткое рассуждение, параметр reasoning.effort.
pub const OPENROUTER_REASONING_EFFORT: &str = "low";
/// Запасной путь бесплатной модели OpenRouter: при отказе OpenRouter
/// берёт следующую модель списка models.
pub const OPENROUTER_FREE: &str = "openrouter/free";
/// Кругов вызовов инструментов подряд, потом разговор ждёт
/// «Продолжить».
pub const MAX_ROUNDS: u32 = 16;
/// Повторы запроса при перегрузке сервиса, пределе частоты и сбое
/// сервера.
pub const MAX_RETRIES: u32 = 3;
/// Пауза перед каждым повтором, с.
pub const RETRY_DELAYS: [f64; 3] = [2.0, 5.0, 10.0];
/// Ответ инструмента не длиннее, знаков. Длинный - начало и конец.
pub const OUTPUT_CHARS: usize = 40000;
/// Ответы инструментов в разговоре вместе не длиннее, знаков.
pub const DIALOG_CHARS: usize = 120000;
/// Ответы последних кругов, которые не сокращаются.
pub const KEEP_RESULTS: usize = 2;
/// Пометка на месте сокращённого ответа инструмента.
pub const TRIMMED: &str = "(ответ сокращён: разговор длинный, при нужде вызови снова)";

/// Первые слова просьбы помощнику в строке поиска.
pub const REQUEST_WORDS: &[&str] = &[
    "покажи", "показать", "где", "что", "как", "какой", "какая", "какие", "какое", "сколько", "почему", "зачем", "когда",
    "включи", "выключи", "убери", "сделай", "построй", "поставь", "открой", "найди", "лети", "перелети", "переключи",
    "отметь", "добавь", "расскажи", "объясни", "дай", "нарисуй", "проложи", "сравни", "show", "where", "what", "how",
    "which", "why", "when", "turn", "switch", "make", "build", "put", "open", "find", "fly", "mark", "add", "tell",
    "explain", "give", "draw", "compare", "list",
];
/// Слов в запросе и больше - просьба, названия мест короче.
pub const REQUEST_LENGTH: usize = 6;

/// Похож ли запрос строки поиска на просьбу помощнику: вопросительный
/// знак в конце, первое слово из `REQUEST_WORDS` или длинный запрос.
pub fn is_request(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }
    if text.ends_with('?') {
        return true;
    }
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    let first = words.first().map_or("", |w| w.trim_matches(|c| ",.:;!".contains(c)));
    REQUEST_WORDS.contains(&first) || words.len() >= REQUEST_LENGTH
}

/// Вызов инструмента моделью: номер вызова, имя, аргументы.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub id: String,
    pub name: String,
    pub args: Value,
}

impl Call {
    pub fn new(id: &str, name: &str, args: Value) -> Self {
        Self { id: id.to_string(), name: name.to_string(), args }
    }
}

/// Ответ инструмента на вызов `id`, `Result` модуля.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    pub id: String,
    pub text: String,
}

impl ToolResult {
    pub fn new(id: &str, text: &str) -> Self {
        Self { id: id.to_string(), text: text.to_string() }
    }
}

/// Разобранный ответ модели: текст, вызовы инструментов, исходные
/// элементы ответа для следующего запроса (формат Responses требует
/// вернуть их как есть, у других форматов `None`), ошибка или пустая
/// строка.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Reply {
    pub text: String,
    pub calls: Vec<Call>,
    pub raw: Option<Vec<Value>>,
    pub error: String,
}

impl Reply {
    /// Ответ с ошибкой и без текста.
    pub fn failed(error: &str) -> Self {
        Self { error: error.to_string(), ..Self::default() }
    }
}

/// Кто говорит в разговоре.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Role {
    #[default]
    User,
    Assistant,
}

/// Сообщение разговора в нейтральном виде, словарь
/// `{"role", "text", "calls", "results", "raw"}` модуля. Сообщение
/// пользователя - вопрос `text` или ответы инструментов `results`,
/// сообщение модели - текст, вызовы и исходные элементы ответа.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Message {
    pub role: Role,
    pub text: String,
    pub calls: Vec<Call>,
    pub results: Vec<ToolResult>,
    pub raw: Option<Vec<Value>>,
}

impl Message {
    /// Вопрос пользователя.
    pub fn user(text: &str) -> Self {
        Self { role: Role::User, text: text.to_string(), ..Self::default() }
    }

    /// Есть ли ответы инструментов - `m.get("results")` модуля.
    fn has_results(&self) -> bool {
        !self.results.is_empty()
    }
}

/// Сообщение модели для разговора.
pub fn assistant_message(reply: &Reply) -> Message {
    Message { role: Role::Assistant, text: reply.text.clone(), calls: reply.calls.clone(), results: Vec::new(), raw: reply.raw.clone() }
}

/// Ответы инструментов - следующее сообщение пользователя.
pub fn results_message(results: Vec<ToolResult>) -> Message {
    Message { role: Role::User, results, ..Message::default() }
}

/// Действия, которые помощник не выполняет: разговор с помощником уже
/// открыт.
pub const ACTION_SKIP: [&str; 1] = ["tool:assistant"];
/// Действия с окнами выбора файла и записью облёта - только
/// по разрешению пользователя.
pub const ACTION_GATED: [&str; 5] = ["tool:record", "tool:scene_save", "tool:scene_open", "tool:share", "tool:crust"];
/// Инструменты, каждый вызов которых разрешает пользователь.
pub const GATED_TOOLS: [&str; 2] = ["list_project_layers", "read_features"];
/// Строк слоя за один read_features, не больше.
pub const FEATURE_LIMIT: usize = 200;
/// Строк раздела руководства за один read_doc.
pub const DOC_LINES: usize = 120;
/// Разделов в ответе search_docs.
pub const DOC_HITS: usize = 5;

/// Действие строки «Поиск»: ключ, название и что сделает действие.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Action {
    pub key: String,
    pub text: String,
    pub note: String,
}

/// Действия строки «Поиск», доступные помощнику.
pub fn assistant_actions(actions: &[Action]) -> Vec<Action> {
    actions.iter().filter(|a| !ACTION_SKIP.contains(&a.key.as_str())).cloned().collect()
}

/// Нужно ли разрешение пользователя на вызов.
pub fn needs_approval(call: &Call) -> bool {
    if GATED_TOOLS.contains(&call.name.as_str()) {
        return true;
    }
    call.name == "run_action" && call.args.get("key").is_some_and(|key| ACTION_GATED.contains(&py::py_str(key).as_str()))
}

/// Ключ разрешения «в этом разговоре»: имя и аргументы JSON
/// с упорядоченными ключами, как `json.dumps(sort_keys=True,
/// ensure_ascii=False)` модуля. Разрешение действует только на тот же
/// самый вызов.
pub fn approval_key(call: &Call) -> String {
    let empty = Value::Object(serde_json::Map::new());
    let args = if py::truthy(&call.args) { &call.args } else { &empty };
    format!("{} {}", call.name, py::dumps(args, true))
}

/// Ответ инструмента в пределе длины `limit` знаков: начало и конец,
/// между ними пометка, сколько знаков пропущено.
pub fn bounded(text: &str, limit: usize) -> String {
    let len = py::chars(text);
    if len <= limit {
        return text.to_string();
    }
    let head = limit * 2 / 3;
    let tail = limit - head;
    let start: String = text.chars().take(head).collect();
    // text[-0:] Python - весь текст.
    let end: String = if tail == 0 { text.to_string() } else { text.chars().skip(len - tail).collect() };
    format!("{start}\n…(пропущено {} знаков)…\n{end}", len - head - tail)
}

/// Разговор для запроса: ответы инструментов старше последних `keep`
/// сообщений с ответами заменяются пометкой `TRIMMED`, пока их длина
/// больше `limit`. Сам разговор не меняется - возвращается копия.
pub fn trimmed(dialog: &[Message], limit: usize, keep: usize) -> Vec<Message> {
    let marks: Vec<usize> = dialog.iter().enumerate().filter(|(_, m)| m.has_results()).map(|(n, _)| n).collect();
    let mut total: usize = dialog.iter().flat_map(|m| m.results.iter()).map(|r| py::chars(&r.text)).sum();
    let mut out = dialog.to_vec();
    if total <= limit {
        return out;
    }
    let old = if keep == 0 { &marks[..] } else { &marks[..marks.len().saturating_sub(keep)] };
    let mark = py::chars(TRIMMED);
    for &n in old {
        if total <= limit {
            break;
        }
        let Some(message) = out.get_mut(n) else { continue };
        for result in &mut message.results {
            let len = py::chars(&result.text);
            if total > limit && len > mark {
                total -= len - mark;
                result.text = TRIMMED.to_string();
            }
        }
    }
    out
}

/// Признаки отказа, после которого запрос повторяется: перегрузка
/// сервиса (Anthropic overloaded_error, код 529), предел частоты (429,
/// OpenRouter «rate-limited upstream»), сбой сервера 5xx, срок ответа.
/// Отказ соединения не повторяется: свой сервис не запущен.
pub const TRANSIENT: [&str; 18] = [
    "overload", "rate limit", "rate-limit", "ratelimit", "too many requests", "temporarily", "unavailable", "bad gateway",
    "gateway timeout", "internal server error", "timed out", "timeout", "try again", "529", "503", "502", "504", "429",
];

/// Временный ли отказ - стоит ли повторить тот же запрос.
pub fn transient(error: &str) -> bool {
    let text = error.to_lowercase();
    TRANSIENT.iter().any(|word| text.contains(word))
}

/// Указатель руководства для search_docs и read_doc. Раздел - название,
/// якорь и текст Markdown, номер раздела - от нуля.
pub trait DocIndex {
    /// Количество разделов.
    fn count(&self) -> usize;
    /// Раздел `n`: название, якорь и текст.
    fn section(&self, n: usize) -> Option<(&str, &str, &str)>;
    /// Номера разделов к запросу, лучшие первыми, не больше `limit`.
    fn search(&self, query: &str, limit: usize) -> Vec<usize>;
    /// Абзац раздела `n`, где больше всего слов запроса.
    fn snippet(&self, n: usize, query: &str) -> String;
    /// Номер раздела по якорю.
    fn find(&self, anchor: &str) -> Option<usize>;
}

/// Ответ search_docs: строки «якорь - название: абзац».
pub fn search_docs_text(index: &dyn DocIndex, query: &str, limit: usize) -> String {
    let found = index.search(query, limit);
    if found.is_empty() {
        return "В руководстве ничего не нашлось. Попробуй другие слова.".to_string();
    }
    let mut lines = Vec::new();
    for n in found {
        if let Some((title, anchor, _)) = index.section(n) {
            lines.push(format!("{anchor} - {title}: {}", index.snippet(n, query)));
        }
    }
    lines.join("\n")
}

/// Ответ read_doc: строки раздела с `offset`, не больше `limit`, и где
/// они в разделе. `limit` 0 - `DOC_LINES`, больше `DOC_LINES` не бывает.
/// Якорь можно дать с # или названием раздела.
pub fn read_doc_text(index: &dyn DocIndex, anchor: &str, offset: i64, limit: i64) -> String {
    let anchor = anchor.trim().trim_start_matches('#');
    let wanted = anchor.to_lowercase();
    let found = index.find(anchor).or_else(|| {
        (0..index.count()).find(|&k| index.section(k).is_some_and(|(title, _, _)| title.to_lowercase() == wanted))
    });
    let Some((title, _, text)) = found.and_then(|n| index.section(n)) else {
        return format!("Раздела «{anchor}» нет, якорь берётся из search_docs.");
    };
    let lines = py::splitlines(text);
    let offset = usize::try_from(offset.max(0)).unwrap_or(usize::MAX);
    let limit = if limit == 0 { DOC_LINES } else { usize::try_from(limit.clamp(1, DOC_LINES as i64)).unwrap_or(DOC_LINES) };
    let part: &[&str] = lines.get(offset..).map_or(&[], |rest| &rest[..limit.min(rest.len())]);
    let first = if part.is_empty() { offset } else { offset + 1 };
    let head = format!("## {title}\nСтроки {first}-{} из {}.", offset + part.len(), lines.len());
    let more = if offset.saturating_add(limit) >= lines.len() {
        String::new()
    } else {
        format!("\nДальше - read_doc с offset {}.", offset + limit)
    };
    format!("{head}\n{}{more}", part.join("\n"))
}

/// Ответ list_actions: строка «ключ - название: что сделает».
pub fn actions_text(actions: &[Action]) -> String {
    if actions.is_empty() {
        return "Действий нет.".to_string();
    }
    let lines: Vec<String> = actions
        .iter()
        .map(|a| if a.note.is_empty() { format!("{} - {}", a.key, a.text) } else { format!("{} - {}: {}", a.key, a.text, a.note) })
        .collect();
    lines.join("\n")
}

/// Создание меток одним запросом: предел длины ответа, токенов.
pub const KML_MAX_TOKENS: u64 = 16000;
/// Запасной предел: сервис отказал из-за длины ответа - повтор с ним.
pub const KML_SAFE_TOKENS: u64 = 8192;
/// Меток в документе, не больше.
pub const KML_PLACES: usize = 100;

/// Правила документа KML для модели. `{places}` заменяется на
/// `KML_PLACES` - готовый текст даёт [`kml_rules`].
pub const KML_RULES: &str = concat!(
    "корень <kml xmlns=\"http://www.opengis.net/kml/2.2\">, в нём ",
    "<Document> с <name> на языке пользователя.\n",
    "Пространство - всё, что у темы есть на местности:\n",
    "- место или событие - <Placemark> с <Point>;\n",
    "- маршрут, поход, плавание, линия фронта, граница, река, дорога - ",
    "<LineString>. Линия идёт по настоящему пути с промежуточными ",
    "точками вдоль берегов, рек и дорог, а не прямой между концами;\n",
    "- государство, территория, район боёв, зона, ареал, водоём - ",
    "<Polygon> с <outerBoundaryIs><LinearRing>, контур по настоящим ",
    "очертаниям, от 12 до 60 вершин.\n",
    "Каждая геометрия лежит внутри своего <Placemark> с <name>. Класть ",
    "<LineString> или <Polygon> прямо в <Folder> нельзя.\n",
    "Координаты - пары «долгота,широта» через пробел, десятичные ",
    "градусы с 3-4 знаками, восточная долгота и северная широта ",
    "положительные. Внутри пары только запятая, без пробела, вот так - ",
    "<coordinates>-6.93,37.18 -13.85,28.68 -74.67,24.08</coordinates>. ",
    "Координаты настоящие, не выдуманные.\n",
    "Время - у каждой метки, для которой известна хотя бы дата или год. ",
    "Без времени метка остаётся, только если его у неё нет вовсе:\n",
    "- событие в один день - <TimeStamp><when>ГГГГ-ММ-ДД</when>",
    "</TimeStamp>;\n",
    "- всё, что длилось - осада, поход, война, правление, жизнь города ",
    "или государства - <TimeSpan><begin>…</begin><end>…</end>",
    "</TimeSpan>;\n",
    "- годится неполная дата ГГГГ или ГГГГ-ММ. Год не короче четырёх ",
    "цифр, год до нашей эры - со знаком минус, как в XML Schema: 264 год ",
    "до н. э. это -0264, 30 000 лет назад - -28000. Описание и название ",
    "без HTML, знак & пиши как &amp;;\n",
    "- путь разбивай на этапы. Каждый этап - своя <LineString> со своим ",
    "<TimeSpan> от выхода до прихода: на шкале времени такая линия ",
    "растёт от начала к концу. Остановки пути - точки с датами;\n",
    "- изменение территории или линии фронта во времени - несколько ",
    "<Polygon> или <LineString> одного объекта с промежутками ",
    "<TimeSpan> подряд: на шкале времени они сменяют друг друга;\n",
    "- общее время этапа можно поставить <TimeSpan> у <Folder>, метки ",
    "без своего времени берут его.\n",
    "Оформление: общие стили объявляй один раз в <Document> - <Style ",
    "id=\"…\"> с <LineStyle>, <PolyStyle>, <IconStyle>, цвет в записи ",
    "aabbggrr - и ссылайся на них <styleUrl>#id</styleUrl>. Заливка ",
    "многоугольника полупрозрачная, альфа от 40 до 80. Разные стороны, ",
    "этапы и виды объектов - разные цвета.\n",
    "У каждой метки <name> и <description> в одно-два предложения, ",
    "в начале описания дата словами. Части темы раскладывай по ",
    "<Folder>, внутри - по порядку дат.\n",
    "Объём: столько меток, сколько нужно, чтобы показать тему полно, ",
    "не больше {places}. Покрой всю тему - все этапы, весь промежуток ",
    "времени и всю территорию, а не только начало. Если места не ",
    "хватает, сокращай описания, а не геометрию и даты."
);

/// Правила документа KML с `KML_PLACES` на месте `{places}`.
pub fn kml_rules() -> String {
    KML_RULES.replace("{places}", &KML_PLACES.to_string())
}

/// Системная подсказка разговора: роль помощника, правила KML
/// и контекст вида. `context` собирает окно - точка взгляда, тело,
/// строки «Слоёв».
pub fn system_text(context: &Value) -> String {
    format!(
        "{}{}\nКонтекст вида: {}",
        concat!(
            "Ты - помощник трёхмерного глобуса PlanetX внутри QGIS. ",
            "Отвечай кратко, на языке пользователя. Действуй через ",
            "инструменты, координаты - широта и долгота в градусах. ",
            "Слои, карты, окна, инструменты и демо - через list_actions ",
            "и run_action, ключ берётся из свежего списка. Если ",
            "для ответа нужны данные - вызови point_info или ",
            "quakes_summary, не выдумывай чисел. Вопрос о том, как ",
            "работает модуль, где пункт меню или что делает настройка - ",
            "ищи в руководстве через search_docs и read_doc, не ",
            "угадывай. Слои проекта QGIS - list_project_layers ",
            "и read_features, их чтение разрешает пользователь, отказ ",
            "не повторяй. Метки, пути, ",
            "многоугольники, папки и туры - только документом KML через ",
            "add_kml, пользователь подтверждает запись сам. Правила ",
            "документа KML: "
        ),
        kml_rules(),
        py::dumps(context, false)
    )
}

/// Инструменты модуля, которых у Lite нет: руководства и проекта
/// QGIS у Lite нет.
pub const NOT_IN_LITE: [&str; 4] = ["search_docs", "read_doc", "list_project_layers", "read_features"];

/// Инструменты Lite: [`tools`] без [`NOT_IN_LITE`].
pub fn lite_tools(ready_keys: &[&str]) -> Vec<Tool> {
    tools(ready_keys).into_iter().filter(|t| !NOT_IN_LITE.contains(&t.name)).collect()
}

/// Системная подсказка Lite: как [`system_text`], без руководства
/// и слоёв проекта QGIS.
pub fn lite_system_text(context: &Value) -> String {
    format!(
        "{}{}\nКонтекст вида: {}",
        concat!(
            "Ты - помощник трёхмерного глобуса PlanetX Lite. ",
            "Отвечай кратко, на языке пользователя. Действуй через ",
            "инструменты, координаты - широта и долгота в градусах. ",
            "Слои, карты, окна, инструменты и демо - через list_actions ",
            "и run_action, ключ берётся из свежего списка. Если ",
            "для ответа нужны данные - вызови point_info или ",
            "quakes_summary, не выдумывай чисел. Действия, которые ",
            "пользователь запретил, не повторяй. Метки, пути, ",
            "многоугольники, папки и туры - только документом KML через ",
            "add_kml, пользователь подтверждает запись сам. Правила ",
            "документа KML: "
        ),
        kml_rules(),
        py::dumps(context, false)
    )
}

/// Системная подсказка создания меток: в ответе только KML.
pub fn kml_system_text(context: &Value) -> String {
    format!(
        "{}{}\nКонтекст вида: {}",
        concat!(
            "Ты создаёшь метки для трёхмерного глобуса со шкалой времени. ",
            "В ответе - только один документ KML 2.2, без пояснений и без ",
            "разметки Markdown. Правила документа: "
        ),
        kml_rules(),
        py::dumps(context, false)
    )
}

/// Документ KML с видом камеры: LookAt, расстояние в метрах.
pub fn look_at_kml(lat: f64, lon: f64, distance: f64, heading: f64, tilt: f64) -> String {
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<kml xmlns=\"http://www.opengis.net/kml/2.2\"><Document>",
            "<LookAt><longitude>{:.6}</longitude>",
            "<latitude>{:.6}</latitude><altitude>0</altitude>",
            "<heading>{:.2}</heading><tilt>{:.2}</tilt>",
            "<range>{:.0}</range>",
            "<altitudeMode>relativeToGround</altitudeMode></LookAt>",
            "</Document></kml>"
        ),
        lon, lat, heading, tilt, distance
    )
}

/// Точка в рамке (юг, север, запад, восток), рамка через линию перемены
/// дат - запад больше востока.
pub fn in_box(lat: f64, lon: f64, bbox: (f64, f64, f64, f64)) -> bool {
    let (south, north, west, east) = bbox;
    let lon = (lon + 180.0).rem_euclid(360.0) - 180.0;
    let across = if west <= east { west <= lon && lon <= east } else { lon >= west || lon <= east };
    south <= lat && lat <= north && across
}

/// Землетрясение для сводки: широта, долгота, глубина очага в км,
/// магнитуда, название места.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Quake {
    pub lat: f64,
    pub lon: f64,
    pub depth: f64,
    pub mag: f64,
    pub place: String,
}

/// Сводка землетрясений в рамке для модели: количество, `top` самых
/// сильных и самых глубоких.
pub fn quakes_text(events: &[Quake], bbox: (f64, f64, f64, f64), top: usize) -> String {
    let inside: Vec<&Quake> = events.iter().filter(|q| in_box(q.lat, q.lon, bbox)).collect();
    if inside.is_empty() {
        return "В рамке землетрясений M4.5+ за 30 суток нет.".to_string();
    }
    let line = |q: &&Quake| format!("M{:.1}, глубина {:.0} км, {:.2}, {:.2}, {}", q.mag, q.depth, q.lat, q.lon, q.place);
    let mut strongest = inside.clone();
    strongest.sort_by(|a, b| b.mag.partial_cmp(&a.mag).unwrap_or(std::cmp::Ordering::Equal));
    let mut deepest = inside.clone();
    deepest.sort_by(|a, b| b.depth.partial_cmp(&a.depth).unwrap_or(std::cmp::Ordering::Equal));
    let mut lines = vec![format!("Событий в рамке: {}.", inside.len()), "Самые сильные:".to_string()];
    lines.extend(strongest.iter().take(top).map(line));
    lines.push("Самые глубокие:".to_string());
    lines.extend(deepest.iter().take(top).map(line));
    lines.join("\n")
}
