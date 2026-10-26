// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Ответ потоком SSE и документ KML из ответа модели.
//!
//! Создание меток одним запросом: модель без инструментов возвращает
//! документ KML текстом. Ответ идёт потоком - длинный документ
//! не упирается в простой соединения, метки считаются по мере прихода.

use serde_json::Value;

use crate::provider::{chat_error, parse};
use crate::py::{get_true, truthy};

/// Текст ответа модели из потока событий SSE.
///
/// [`StreamText::feed`] принимает куски ответа в любом порезе, в том
/// числе посреди строки и посреди символа UTF-8. Понимает события трёх
/// форматов по их виду: Anthropic Messages (content_block_delta), OpenAI
/// Chat (choices[0].delta.content) и OpenAI Responses
/// (response.output_text.delta). Рассуждения модели пропускаются.
/// [`StreamText::finish`] закрывает поток. Ответ не потоком (ошибка HTTP
/// телом JSON или сервис без потока) разбирается целиком через
/// [`parse`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StreamText {
    pub provider: String,
    /// Пришедший текст ответа.
    pub text: String,
    /// Ошибка сервиса или пустая строка.
    pub error: String,
    /// Ответ упёрся в предел длины.
    pub cut: bool,
    /// Поток закрыт.
    pub done: bool,
    tail: Vec<u8>,
    /// Весь ответ, пока не пришло ни одного события, - для ответа
    /// не потоком.
    all: Vec<u8>,
    events: usize,
}

impl StreamText {
    pub fn new(provider: &str) -> Self {
        Self { provider: provider.to_string(), ..Self::default() }
    }

    /// Принять кусок ответа. Возвращает, вырос ли текст.
    pub fn feed(&mut self, data: &[u8]) -> bool {
        if data.is_empty() {
            return false;
        }
        if self.events == 0 {
            self.all.extend_from_slice(data);
        }
        self.tail.extend_from_slice(data);
        let before = self.text.len();
        while let Some(end) = self.tail.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.tail.drain(..=end).collect();
            let text = String::from_utf8_lossy(&line[..end]);
            self.line(text.trim());
        }
        if self.events > 0 {
            self.all = Vec::new();
        }
        self.text.len() > before
    }

    /// Поток кончился: остаток строки и ответ не потоком.
    pub fn finish(&mut self) {
        if !self.tail.is_empty() {
            let tail = std::mem::take(&mut self.tail);
            self.line(String::from_utf8_lossy(&tail).trim());
        }
        if self.events == 0 && self.error.is_empty() {
            let all = std::mem::take(&mut self.all);
            let whole = String::from_utf8_lossy(&all);
            let whole = whole.trim();
            if !whole.is_empty()
                && let Ok(data) = serde_json::from_str::<Value>(whole)
                && !data.is_null()
            {
                let reply = parse(&self.provider, &data);
                self.text = reply.text;
                self.error = reply.error;
            }
        }
        self.done = true;
    }

    fn line(&mut self, line: &str) {
        // event:, комментарий «: …», пустая строка - не данные.
        let Some(payload) = line.strip_prefix("data:") else { return };
        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" {
            return;
        }
        let Ok(event) = serde_json::from_str::<Value>(payload) else { return };
        if !event.is_object() {
            return;
        }
        self.events += 1;
        self.event(&event);
    }

    fn event(&mut self, event: &Value) {
        let kind = event.get("type").and_then(Value::as_str);
        if let Some(error) = get_true(event, "error")
            && kind != Some("response.failed")
        {
            self.error = chat_error(error);
        } else if kind == Some("content_block_delta") {
            let delta = event.get("delta");
            if delta.and_then(|d| d.get("type")).and_then(Value::as_str) == Some("text_delta") {
                self.text += delta.and_then(|d| d.get("text")).and_then(Value::as_str).unwrap_or("");
            }
        } else if kind == Some("message_delta") {
            if event.get("delta").and_then(|d| d.get("stop_reason")).and_then(Value::as_str) == Some("max_tokens") {
                self.cut = true;
            }
        } else if kind == Some("response.output_text.delta") {
            if let Some(Value::String(delta)) = event.get("delta") {
                self.text += delta;
            }
        } else if kind == Some("response.incomplete") {
            self.cut = true;
        } else if matches!(kind, Some("response.failed" | "error")) {
            let failed = event
                .get("response")
                .and_then(|r| get_true(r, "error"))
                .or_else(|| get_true(event, "error"))
                .or_else(|| get_true(event, "message"))
                .cloned()
                .unwrap_or_else(|| Value::String(kind.unwrap_or("").to_string()));
            self.error = chat_error(&failed);
        } else if let Some(choices) = get_true(event, "choices") {
            let Some(choice) = choices.get(0).filter(|c| truthy(c)) else { return };
            if let Some(Value::String(content)) = choice.get("delta").and_then(|d| d.get("content")) {
                self.text += content;
            }
            if choice.get("finish_reason").and_then(Value::as_str) == Some("length") {
                self.cut = true;
            }
        }
    }
}

/// Похожа ли ошибка сервиса на отказ из-за предела длины ответа: тогда
/// запрос повторяется с `KML_SAFE_TOKENS`.
pub fn length_refused(error: &str) -> bool {
    let text = error.to_lowercase();
    text.contains("token") && (text.contains("max") || text.contains("limit") || text.contains("exceed"))
}

/// Сколько меток уже пришло целиком в потоке ответа.
pub fn kml_progress(text: &str) -> usize {
    text.matches("</Placemark>").count()
}

const KML_ROOT: &str = "<kml xmlns=\"http://www.opengis.net/kml/2.2\">";

/// Идёт ли за именем тега пробел или `>` - `[\s>]` выражений модуля.
fn tag_end(rest: &str) -> bool {
    rest.chars().next().is_some_and(|c| c.is_whitespace() || c == '>')
}

/// Первое начало тега `<name` с пробелом или `>` за именем.
fn find_open(text: &str, name: &str) -> Option<usize> {
    let pattern = format!("<{name}");
    let mut from = 0;
    while let Some(pos) = text.get(from..).and_then(|rest| rest.find(&pattern)) {
        let at = from + pos;
        if text.get(at + pattern.len()..).is_some_and(tag_end) {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

/// Документ KML из ответа модели или `None`. Снимает текст вокруг
/// документа. Оборванный документ (ответ упёрся в предел длины)
/// обрезается по последней целой метке и закрывается.
pub fn extract_kml(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    let owned;
    let (text, root) = match find_open(text, "kml") {
        Some(root) => (text, root),
        None => {
            // Документ без корня <kml>: сразу <Document> или метки.
            let bare = ["Document", "Folder", "Placemark"].iter().filter_map(|name| find_open(text, name)).min()?;
            owned = format!("{KML_ROOT}{}", &text[bare..]);
            (owned.as_str(), 0)
        }
    };
    let body = &text[root..];
    if let Some(end) = body.find("</kml>") {
        // Первый документ, если их в ответе два.
        return Some(body[..end + "</kml>".len()].to_string());
    }
    // Оборванный документ: до последнего целого элемента, открытые
    // Folder и Document закрываются по стеку.
    let cut = ["Placemark", "Folder", "Document"]
        .iter()
        .filter_map(|tag| body.rfind(&format!("</{tag}>")).map(|pos| pos + tag.len() + 3))
        .max()?;
    let body = &body[..cut];
    let mut stack: Vec<&str> = Vec::new();
    for (at, _) in body.match_indices('<') {
        let rest = &body[at + 1..];
        let (closing, rest) = match rest.strip_prefix('/') {
            Some(rest) => (true, rest),
            None => (false, rest),
        };
        let Some(name) = ["Folder", "Document"].into_iter().find(|name| rest.strip_prefix(name).is_some_and(tag_end)) else {
            continue;
        };
        if !closing {
            stack.push(name);
        } else if stack.last() == Some(&name) {
            stack.pop();
        }
    }
    let mut out = body.to_string();
    for name in stack.iter().rev() {
        out += &format!("</{name}>");
    }
    out += "</kml>";
    Some(out)
}
