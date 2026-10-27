// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Сервер MCP: рукопожатие, список инструментов, вызовы, запреты,
//! ошибки протокола, снимок картинкой, ключ ретранслятора.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use assistant::Call;
use assistant::mcp::{self, Host, Output, Server};
use serde_json::{Value, json};

/// Хозяин-запись: помнит вызовы, снимок - картинка из трёх байт.
#[derive(Default)]
struct Globe {
    calls: Vec<Call>,
}

impl Host for Globe {
    fn call(&mut self, call: &Call) -> Output {
        self.calls.push(call.clone());
        match call.name.as_str() {
            "snapshot" => Output::Image(vec![1, 2, 3], "image/png", "вид у Перми".into()),
            "point_info" => Output::Error("нет точки".into()),
            _ => Output::Text(format!("сделано: {}", call.name)),
        }
    }
}

fn ask(server: &Server, host: &mut Globe, message: Value) -> Value {
    serde_json::from_str(&server.handle(host, &message.to_string()).unwrap()).unwrap()
}

#[test]
fn handshake_and_tool_list() {
    let server = Server::new(&["esri", "osm"]);
    let mut g = Globe::default();
    let r = ask(&server, &mut g, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}));
    assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(r["result"]["serverInfo"]["name"], "planetx-lite");
    assert!(r["result"]["instructions"].as_str().unwrap().contains("snapshot"));
    let r = ask(&server, &mut g, json!({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {"protocolVersion": "1999-01-01"}}));
    assert_eq!(r["result"]["protocolVersion"], mcp::VERSIONS[0]);
    // Уведомление ответа не получает.
    assert!(server.handle(&mut g, &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string()).is_none());
    let r = ask(&server, &mut g, json!({"jsonrpc": "2.0", "id": "t", "method": "tools/list"}));
    let names: Vec<&str> = r["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"fly_to") && names.contains(&"snapshot") && !names.contains(&"read_features"));
    assert!(r["result"]["tools"][0]["inputSchema"].is_object());
}

#[test]
fn calls_reach_the_globe_and_gated_ones_do_not() {
    let server = Server::new(&[]);
    let mut g = Globe::default();
    let r = ask(&server, &mut g, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "fly_to", "arguments": {"lat": 58.0, "lon": 56.2}}}));
    assert_eq!(r["result"]["content"][0]["text"], "сделано: fly_to");
    assert_eq!(r["result"]["isError"], false);
    assert_eq!(g.calls[0].args["lat"], 58.0);
    // Ошибка инструмента - ответ модели, а не ошибка протокола.
    let r = ask(&server, &mut g, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "point_info", "arguments": {}}}));
    assert_eq!(r["result"]["isError"], true);
    // Запись облёта - только пользователь.
    let r = ask(&server, &mut g, json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "run_action", "arguments": {"key": "tool:record"}}}));
    assert_eq!(r["result"]["isError"], true);
    assert_eq!(g.calls.len(), 2);
    // Инструментов модуля, которых у Lite нет, нет и здесь.
    let r = ask(&server, &mut g, json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "read_features"}}));
    assert_eq!(r["error"]["code"], -32602);
}

#[test]
fn snapshot_is_an_image() {
    let server = Server::new(&[]);
    let mut g = Globe::default();
    let msg = json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {"name": "snapshot"}});
    assert!(mcp::is_snapshot(&msg.to_string()));
    let r = ask(&server, &mut g, msg);
    assert_eq!(r["result"]["content"][0]["type"], "image");
    assert_eq!(r["result"]["content"][0]["data"], "AQID");
    assert_eq!(r["result"]["content"][0]["mimeType"], "image/png");
    assert_eq!(r["result"]["content"][1]["text"], "вид у Перми");
}

#[test]
fn protocol_errors() {
    let server = Server::new(&[]);
    let mut g = Globe::default();
    let r: Value = serde_json::from_str(&server.handle(&mut g, "{oops").unwrap()).unwrap();
    assert_eq!(r["error"]["code"], -32700);
    let r = ask(&server, &mut g, json!({"jsonrpc": "2.0", "id": 1, "method": "nope"}));
    assert_eq!(r["error"]["code"], -32601);
    let r = ask(&server, &mut g, json!([]));
    assert_eq!(r["error"]["code"], -32600);
    let r = ask(&server, &mut g, json!([{"jsonrpc": "2.0", "id": 1, "method": "ping"}, {"jsonrpc": "2.0", "method": "notifications/x"}]));
    assert_eq!(r.as_array().unwrap().len(), 1);
    assert!(mcp::expects_reply(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string()));
    assert!(!mcp::expects_reply(&json!({"jsonrpc": "2.0", "method": "notifications/x"}).to_string()));
}

#[test]
fn base64_and_attach() {
    assert_eq!(mcp::base64(b""), "");
    assert_eq!(mcp::base64(b"f"), "Zg==");
    assert_eq!(mcp::base64(b"fo"), "Zm8=");
    assert_eq!(mcp::base64(b"foobar"), "Zm9vYmFy");
    let line = mcp::attach_line("abc123");
    assert_eq!(mcp::attach_token(&line).as_deref(), Some("abc123"));
    assert!(mcp::attach_token("{\"method\": \"ping\"}").is_none());
    assert!(mcp::same_token("abc", "abc") && !mcp::same_token("abc", "abd") && !mcp::same_token("abc", "ab"));
}
