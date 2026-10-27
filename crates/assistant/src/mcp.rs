// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сервер Model Context Protocol: глобус PlanetX Lite для любого
//! клиента MCP (Claude Code, Claude Desktop, Cursor, VS Code). Образец -
//! `crates/assistant/src/mcp.rs` RustGIS (Apache 2.0).
//!
//! Без ввода-вывода: [`Server::handle`] берёт одно сообщение JSON-RPC
//! и отдаёт ответ, его гоняют и ретранслятор программы, и тесты.
//! Инструменты исполняет хозяин - окно глобуса ([`Host`]). Модель
//! клиента управляет глобусом, который пользователь видит открытым.
//!
//! Инструменты - те же, что у помощника Lite ([`crate::lite_tools`]),
//! и снимок вида [`SNAPSHOT`]. Клиент спрашивает пользователя перед
//! вызовом, сервер добавляет свой запрет: действия, на которые помощник
//! спрашивает разрешение (окна файлов, запись облёта, «Поделиться»),
//! агенту закрыты ([`crate::needs_approval`]).

use serde_json::{Value, json};

use crate::Call;

/// Версии протокола, новые первыми, как у RustGIS.
pub const VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
/// Сообщений в пакете не больше.
pub const MAX_BATCH: usize = 16;
/// Инструмент снимка вида.
pub const SNAPSHOT: &str = "snapshot";

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// Что модель узнаёт о сервере при подключении.
pub const INSTRUCTIONS: &str = "PlanetX Lite - трёхмерный глобус: Земля, другие тела и небо, рельеф, снимки, \
метки KML, слои каталога, разрезы недр. Это глобус, открытый у пользователя, он видит, как меняется вид.\n\
- Слои, карты, окна, инструменты и демо - через list_actions и run_action, ключ берётся из свежего списка.\n\
- fly_to и search_place двигают камеру, set_layer и set_time меняют вид, point_info и quakes_summary дают данные.\n\
- Метки, пути, многоугольники и туры - документом KML через add_kml, запись в «Мои метки» подтверждает пользователь.\n\
- snapshot показывает вид картинкой: смотри его после изменения вида, прежде чем говорить, что получилось.\n\
- Не выдумывай чисел, данные бери у инструментов.";

/// Ответ инструмента: текст или картинка PNG с текстом.
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    Text(String),
    /// Картинка, её тип (image/png, image/jpeg) и подпись к ней.
    Image(Vec<u8>, &'static str, String),
    /// Ошибка - модель читает её как ответ.
    Error(String),
}

/// Хозяин сервера: окно глобуса исполняет вызовы.
pub trait Host {
    fn call(&mut self, call: &Call) -> Output;
}

/// Описание снимка вида для `tools/list`.
pub fn snapshot_tool() -> Value {
    json!({
        "name": SNAPSHOT,
        "description": "Вид глобуса картинкой, как его видит пользователь, и точка взгляда. Снимок ждёт конца перелёта и загрузки места. Смотри после изменения вида.",
        "inputSchema": {"type": "object", "properties": {}, "required": []},
        "annotations": {"readOnlyHint": true},
    })
}

/// Инструменты сервера: инструменты помощника Lite и снимок вида.
pub fn tool_list(ready_keys: &[&str]) -> Vec<Value> {
    let mut out: Vec<Value> = crate::lite_tools(ready_keys)
        .into_iter()
        .map(|t| {
            let read_only = matches!(t.name, "point_info" | "quakes_summary" | "list_actions" | "get_kml");
            json!({"name": t.name, "description": t.description, "inputSchema": t.schema, "annotations": {"readOnlyHint": read_only}})
        })
        .collect();
    out.push(snapshot_tool());
    out
}

/// Сервер: ключи готовых подложек для описаний инструментов.
#[derive(Debug, Clone, Default)]
pub struct Server {
    pub ready_keys: Vec<String>,
}

fn error_value(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Ответ-ошибка разбора строкой.
pub fn parse_error(message: &str) -> String {
    error_value(&Value::Null, PARSE_ERROR, message).to_string()
}

/// Ждёт ли сообщение ответа: запрос с id или пакет с запросом.
pub fn expects_reply(message: &str) -> bool {
    let wants = |v: &Value| v.get("method").is_some() && v.get("id").is_some_and(|id| !id.is_null());
    match serde_json::from_str::<Value>(message) {
        Ok(Value::Array(batch)) => batch.iter().any(wants),
        Ok(v) => wants(&v),
        Err(_) => true,
    }
}

/// Вызов инструмента снимка вида в сообщении: окно снимает вид до
/// ответа.
pub fn is_snapshot(message: &str) -> bool {
    serde_json::from_str::<Value>(message).is_ok_and(|v| v.get("method").and_then(Value::as_str) == Some("tools/call") && v.pointer("/params/name").and_then(Value::as_str) == Some(SNAPSHOT))
}

impl Server {
    pub fn new(ready_keys: &[&str]) -> Self {
        Self { ready_keys: ready_keys.iter().map(|k| k.to_string()).collect() }
    }

    /// Одно сообщение (строка JSON): ответ или None - уведомление или
    /// ответ клиента.
    pub fn handle(&self, host: &mut dyn Host, message: &str) -> Option<String> {
        let msg: Value = match serde_json::from_str(message) {
            Ok(v) => v,
            Err(e) => return Some(parse_error(&format!("not JSON: {e}"))),
        };
        let reply = match &msg {
            Value::Array(batch) if batch.is_empty() => error_value(&Value::Null, INVALID_REQUEST, "empty batch"),
            Value::Array(batch) if batch.len() > MAX_BATCH => error_value(&Value::Null, INVALID_REQUEST, &format!("a batch holds at most {MAX_BATCH} messages")),
            Value::Array(batch) => {
                let replies: Vec<Value> = batch.iter().filter_map(|m| self.handle_value(host, m)).collect();
                if replies.is_empty() {
                    return None;
                }
                Value::Array(replies)
            }
            _ => self.handle_value(host, &msg)?,
        };
        Some(reply.to_string())
    }

    fn handle_value(&self, host: &mut dyn Host, msg: &Value) -> Option<Value> {
        let Some(obj) = msg.as_object() else {
            return Some(error_value(&Value::Null, INVALID_REQUEST, "expected a JSON-RPC object"));
        };
        let id = obj.get("id").cloned();
        let Some(method) = obj.get("method").and_then(Value::as_str) else {
            return id.filter(|_| !obj.contains_key("result") && !obj.contains_key("error")).map(|id| error_value(&id, INVALID_REQUEST, "expected a method"));
        };
        // Уведомления (без id) ответа не получают.
        let id = id?;
        if !matches!(id, Value::String(_) | Value::Number(_)) {
            return Some(error_value(&Value::Null, INVALID_REQUEST, "id must be a string or a number"));
        }
        let params = obj.get("params").cloned().unwrap_or(Value::Null);
        Some(match self.call(host, method, &params) {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => error_value(&id, code, &message),
        })
    }

    fn call(&self, host: &mut dyn Host, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => Ok(initialize(params)),
            "ping" => Ok(json!({})),
            "tools/list" => {
                let keys: Vec<&str> = self.ready_keys.iter().map(String::as_str).collect();
                Ok(json!({"tools": tool_list(&keys)}))
            }
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).ok_or((INVALID_PARAMS, "name is required".to_string()))?;
                let args = match params.get("arguments") {
                    None | Some(Value::Null) => json!({}),
                    Some(v @ Value::Object(_)) => v.clone(),
                    Some(_) => return Err((INVALID_PARAMS, "arguments must be an object".into())),
                };
                let known = name == SNAPSHOT || crate::lite_tools(&[]).iter().any(|t| t.name == name);
                if !known {
                    return Err((INVALID_PARAMS, format!("unknown tool '{name}'")));
                }
                let call = Call::new("mcp", name, args);
                if crate::needs_approval(&call) {
                    return Ok(text(format!("'{name}' не выполнен: это действие пользователь делает сам в PlanetX Lite."), true));
                }
                Ok(match host.call(&call) {
                    Output::Text(t) => text(t, false),
                    Output::Error(t) => text(t, true),
                    Output::Image(data, mime, caption) => json!({"content": [{"type": "image", "data": base64(&data), "mimeType": mime}, {"type": "text", "text": caption}], "isError": false}),
                })
            }
            "resources/list" => Ok(json!({"resources": []})),
            "resources/templates/list" => Ok(json!({"resourceTemplates": []})),
            "prompts/list" => Ok(json!({"prompts": []})),
            _ => Err((METHOD_NOT_FOUND, format!("unknown method '{method}'"))),
        }
    }
}

fn text(t: String, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": t}], "isError": is_error})
}

fn initialize(params: &Value) -> Value {
    // Версия клиента, если она знакома, иначе своя новейшая.
    let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
    let version = VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}, "resources": {"listChanged": false}, "prompts": {"listChanged": false}},
        "serverInfo": {"name": "planetx-lite", "title": "PlanetX Lite", "version": env!("CARGO_PKG_VERSION")},
        "instructions": INSTRUCTIONS,
    })
}

/// Байты в base64 со знаками =, RFC 4648.
pub fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for k in 0..4 {
            if k <= chunk.len() {
                out.push(char::from(ABC[((n >> (18 - 6 * k)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Первая строка соединения с ретранслятором: ключ доступа.
pub const ATTACH: &str = "planetx/attach";

/// Строка подключения с ключом token.
pub fn attach_line(token: &str) -> String {
    json!({"jsonrpc": "2.0", "method": ATTACH, "params": {"token": token}}).to_string()
}

/// Ключ из строки подключения или None.
pub fn attach_token(line: &str) -> Option<String> {
    let v: Value = serde_json::from_str(line).ok()?;
    (v.get("method").and_then(Value::as_str) == Some(ATTACH)).then(|| v.pointer("/params/token").and_then(Value::as_str).map(str::to_string))?
}

/// Сравнение ключей без раннего выхода.
pub fn same_token(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
