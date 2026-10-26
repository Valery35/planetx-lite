// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Тела запросов и разбор ответов трёх форматов:
//!
//! - Anthropic Messages - POST {base}/v1/messages, заголовки x-api-key
//!   и anthropic-version;
//! - OpenAI Responses - POST {base}/responses, заголовок Authorization:
//!   Bearer, так работает xAI;
//! - OpenAI Chat Completions - POST {base}/chat/completions, OpenRouter
//!   и Ollama.
//!
//! Ответ сервиса считается враждебным: поле не того вида пропускается.
//! Поля тел идут в том же порядке, что у модуля.

use serde_json::{Map, Value, json};

use crate::py::{dumps, get_true, py_str};
use crate::{
    ANTHROPIC, ANTHROPIC_VERSION, CHAT, CHAT_MAX_TOKENS, Call, LOCAL_HOSTS, MAX_TOKENS, Message, OPENROUTER,
    OPENROUTER_FREE, OPENROUTER_REASONING_EFFORT, Reply, Role, Tool, format_of,
};

/// Запрос к сервису модели: адрес, заголовки и тело JSON. Отправляет
/// его окно.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

/// Состав запроса. Пустой `tools` - запрос без инструментов, ответ
/// только текстом. `max_tokens` - свой предел длины ответа. `stream` -
/// ответ потоком событий SSE, его разбирает [`crate::StreamText`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Options<'a> {
    pub tools: &'a [Tool],
    pub max_tokens: Option<u64>,
    pub stream: bool,
}

fn empty() -> Value {
    Value::Object(Map::new())
}

fn set(body: &mut Value, key: &str, value: Value) {
    if let Value::Object(map) = body {
        map.insert(key.to_string(), value);
    }
}

/// Тело запроса Messages API.
pub fn build_anthropic(model: &str, system: &str, dialog: &[Message], tools: &[Tool]) -> Value {
    let mut messages = Vec::new();
    for m in dialog {
        if m.role == Role::User {
            let content = if m.has_results() {
                Value::Array(
                    m.results.iter().map(|r| json!({"type": "tool_result", "tool_use_id": r.id, "content": r.text})).collect(),
                )
            } else {
                json!(m.text)
            };
            messages.push(json!({"role": "user", "content": content}));
        } else {
            let mut content = Vec::new();
            if !m.text.is_empty() {
                content.push(json!({"type": "text", "text": m.text}));
            }
            for c in &m.calls {
                content.push(json!({"type": "tool_use", "id": c.id, "name": c.name, "input": c.args}));
            }
            messages.push(json!({"role": "assistant", "content": content}));
        }
    }
    let tools: Vec<Value> =
        tools.iter().map(|t| json!({"name": t.name, "description": t.description, "input_schema": t.schema})).collect();
    json!({"model": model, "max_tokens": MAX_TOKENS, "system": system, "tools": tools, "messages": messages})
}

/// Текст ошибки `str(error.get("message") or error)` модуля.
fn error_text(error: &Value) -> String {
    py_str(get_true(error, "message").unwrap_or(error))
}

/// Ответ Messages API в [`Reply`].
pub fn parse_anthropic(data: &Value) -> Reply {
    if !data.is_object() {
        return Reply::failed("no data");
    }
    if data.get("type").and_then(Value::as_str) == Some("error") {
        let error = get_true(data, "error").cloned().unwrap_or_else(empty);
        return Reply::failed(&error_text(&error));
    }
    let mut texts = Vec::new();
    let mut calls = Vec::new();
    for block in get_true(data, "content").and_then(Value::as_array).into_iter().flatten() {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => texts.push(block.get("text").and_then(Value::as_str).unwrap_or("")),
            Some("tool_use") => calls.push(Call::new(
                block.get("id").and_then(Value::as_str).unwrap_or(""),
                block.get("name").and_then(Value::as_str).unwrap_or(""),
                get_true(block, "input").cloned().unwrap_or_else(empty),
            )),
            _ => {}
        }
    }
    Reply { text: join_texts(&texts), calls, raw: None, error: String::new() }
}

/// `"\n".join(t for t in texts if t)` модуля.
fn join_texts(texts: &[&str]) -> String {
    texts.iter().filter(|t| !t.is_empty()).copied().collect::<Vec<_>>().join("\n")
}

/// Тело запроса формата OpenAI Responses. Элементы ответа модели (raw)
/// возвращаются в input как есть, за ними - ответы инструментов.
pub fn build_responses(model: &str, system: &str, dialog: &[Message], tools: &[Tool]) -> Value {
    let mut items = Vec::new();
    for m in dialog {
        if m.role == Role::User {
            if m.has_results() {
                items.extend(m.results.iter().map(|r| json!({"type": "function_call_output", "call_id": r.id, "output": r.text})));
            } else {
                items.push(json!({"role": "user", "content": m.text}));
            }
        } else if let Some(raw) = m.raw.as_ref().filter(|raw| !raw.is_empty()) {
            items.extend(raw.iter().cloned());
        } else if !m.text.is_empty() {
            items.push(json!({"role": "assistant", "content": m.text}));
        }
    }
    let tools: Vec<Value> = tools
        .iter()
        .map(|t| json!({"type": "function", "name": t.name, "description": t.description, "parameters": t.schema}))
        .collect();
    json!({"model": model, "instructions": system, "input": items, "tools": tools})
}

/// Аргументы вызова из строки JSON. Испорченная строка - пустые
/// аргументы.
fn arguments(text: &str) -> Value {
    let text = if text.is_empty() { "{}" } else { text };
    serde_json::from_str(text).unwrap_or_else(|_| empty())
}

/// Ответ формата Responses в [`Reply`].
pub fn parse_responses(data: &Value) -> Reply {
    if !data.is_object() {
        return Reply::failed("no data");
    }
    if let Some(error) = get_true(data, "error") {
        return Reply::failed(&if error.is_object() { error_text(error) } else { py_str(error) });
    }
    let mut texts = Vec::new();
    let mut calls = Vec::new();
    let mut raw = Vec::new();
    for item in get_true(data, "output").and_then(Value::as_array).into_iter().flatten() {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                raw.push(item.clone());
                for part in get_true(item, "content").and_then(Value::as_array).into_iter().flatten() {
                    if matches!(part.get("type").and_then(Value::as_str), Some("output_text" | "text")) {
                        texts.push(part.get("text").and_then(Value::as_str).unwrap_or(""));
                    }
                }
            }
            Some("function_call") => {
                raw.push(item.clone());
                let args = match get_true(item, "arguments") {
                    Some(Value::String(s)) => arguments(s),
                    Some(other) if other.is_object() => other.clone(),
                    _ => empty(),
                };
                calls.push(Call::new(
                    item.get("call_id").and_then(Value::as_str).unwrap_or(""),
                    item.get("name").and_then(Value::as_str).unwrap_or(""),
                    args,
                ));
            }
            _ => {}
        }
    }
    Reply { text: join_texts(&texts), calls, raw: Some(raw), error: String::new() }
}

/// Тело запроса формата OpenAI Chat Completions. Аргументы вызовов -
/// строка `json.dumps(ensure_ascii=False)`, как у модуля.
pub fn build_chat(model: &str, system: &str, dialog: &[Message], tools: &[Tool]) -> Value {
    let mut messages = vec![json!({"role": "system", "content": system})];
    for m in dialog {
        if m.role == Role::User {
            if m.has_results() {
                messages.extend(m.results.iter().map(|r| json!({"role": "tool", "tool_call_id": r.id, "content": r.text})));
            } else {
                messages.push(json!({"role": "user", "content": m.text}));
            }
            continue;
        }
        let mut message = json!({"role": "assistant", "content": m.text});
        if !m.calls.is_empty() {
            let calls: Vec<Value> = m
                .calls
                .iter()
                .map(|c| json!({"id": c.id, "type": "function", "function": {"name": c.name, "arguments": dumps(&c.args, false)}}))
                .collect();
            set(&mut message, "tool_calls", Value::Array(calls));
        }
        messages.push(message);
    }
    let tools: Vec<Value> = tools
        .iter()
        .map(|t| json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.schema}}))
        .collect();
    json!({"model": model, "max_tokens": CHAT_MAX_TOKENS, "messages": messages, "tools": tools})
}

/// Ответ формата Chat Completions в [`Reply`].
pub fn parse_chat(data: &Value) -> Reply {
    if !data.is_object() {
        return Reply::failed("no data");
    }
    if let Some(error) = get_true(data, "error") {
        return Reply::failed(&chat_error(error));
    }
    let Some(choice) = get_true(data, "choices").and_then(|c| c.get(0)) else {
        return Reply::failed("no choices");
    };
    let message = get_true(choice, "message").cloned().unwrap_or_else(empty);
    if choice.get("finish_reason").and_then(Value::as_str) == Some("length")
        && get_true(&message, "content").is_none()
        && get_true(&message, "tool_calls").is_none()
    {
        return Reply::failed("finish_reason length: ответ модели обрезан пределом длины");
    }
    let text = match message.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter(|p| p.is_object())
            .map(|p| p.get("text").and_then(Value::as_str).unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    let mut calls = Vec::new();
    for (n, call) in get_true(&message, "tool_calls").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let function = get_true(call, "function").cloned().unwrap_or_else(empty);
        let args = match get_true(&function, "arguments") {
            Some(Value::String(s)) => arguments(s),
            Some(other) => other.clone(),
            None => empty(),
        };
        let id = get_true(call, "id").map_or_else(|| format!("call{n}"), py_str);
        calls.push(Call::new(&id, function.get("name").and_then(Value::as_str).unwrap_or(""), args));
    }
    Reply { text: text.trim().to_string(), calls, raw: None, error: String::new() }
}

/// Текст ошибки формата Chat. OpenRouter кладёт имя провайдера модели
/// и его ответ в metadata, «Provider returned error» без них ничего
/// не говорит.
pub fn chat_error(error: &Value) -> String {
    if !error.is_object() {
        return py_str(error);
    }
    let mut text = error_text(error);
    if let Some(meta) = get_true(error, "metadata").filter(|m| m.is_object()) {
        if let Some(name) = get_true(meta, "provider_name") {
            text += &format!(" ({})", py_str(name));
        }
        if let Some(raw) = get_true(meta, "raw") {
            let raw = match raw {
                Value::String(s) => s.clone(),
                other => dumps(other, false),
            };
            text += ": ";
            text.extend(raw.chars().take(300));
        }
    }
    text
}

/// Нужен ли ключ: сервису на своём компьютере ключ не нужен.
pub fn needs_key(base: &str) -> bool {
    let rest = base.split_once("://").map_or(base, |(_, rest)| rest);
    let host = rest.split('/').next().unwrap_or("").to_lowercase();
    let host = if host.starts_with('[') {
        format!("{}]", host.split(']').next().unwrap_or(""))
    } else {
        host.split(':').next().unwrap_or("").to_string()
    };
    !LOCAL_HOSTS.contains(&host.as_str())
}

/// Адрес, заголовки и тело запроса для подключения `provider`.
pub fn request(provider: &str, base: &str, model: &str, key: &str, system: &str, dialog: &[Message], options: &Options) -> Request {
    let base = base.trim_end_matches('/');
    let kind = format_of(provider);
    let mut headers = vec![("content-type".to_string(), "application/json".to_string())];
    let (url, mut body, limit) = if kind == ANTHROPIC {
        headers.push(("x-api-key".to_string(), key.to_string()));
        headers.push(("anthropic-version".to_string(), ANTHROPIC_VERSION.to_string()));
        (format!("{base}/v1/messages"), build_anthropic(model, system, dialog, options.tools), "max_tokens")
    } else {
        if !key.is_empty() {
            headers.push(("Authorization".to_string(), format!("Bearer {key}")));
        }
        if kind == CHAT {
            let mut body = build_chat(model, system, dialog, options.tools);
            if provider == OPENROUTER {
                set(&mut body, "reasoning", json!({"effort": OPENROUTER_REASONING_EFFORT}));
                if model.ends_with(":free") {
                    set(&mut body, "models", json!([model, OPENROUTER_FREE]));
                }
            }
            (format!("{base}/chat/completions"), body, "max_tokens")
        } else {
            (format!("{base}/responses"), build_responses(model, system, dialog, options.tools), "max_output_tokens")
        }
    };
    if options.tools.is_empty()
        && let Value::Object(map) = &mut body
    {
        map.remove("tools");
    }
    if let Some(max) = options.max_tokens.filter(|&m| m > 0) {
        set(&mut body, limit, json!(max));
    }
    if options.stream {
        set(&mut body, "stream", json!(true));
    }
    Request { url, headers, body }
}

/// Ответ подключения `provider` в [`Reply`].
pub fn parse(provider: &str, data: &Value) -> Reply {
    match format_of(provider) {
        ANTHROPIC => parse_anthropic(data),
        CHAT => parse_chat(data),
        _ => parse_responses(data),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_answers_give_no_panic() {
        for data in [json!(null), json!([1]), json!("x"), json!({"content": "x"}), json!({"output": [1, {"type": "message", "content": 5}]})] {
            for provider in [ANTHROPIC, crate::RESPONSES, crate::LOCAL] {
                let _ = parse(provider, &data);
            }
        }
        assert_eq!(parse(ANTHROPIC, &json!([1])).error, "no data");
        assert_eq!(parse_chat(&json!({"choices": {"a": 1}})).error, "no choices");
    }
}
