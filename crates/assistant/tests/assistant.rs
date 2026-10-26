// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Запросы и ответы Anthropic Messages, OpenAI Responses и Chat, поток
//! SSE, создание меток KML, перенос `tests/test_assistant.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use assistant as ai;
use assistant::{Call, Message, Options, ToolResult};
use serde_json::{Value, json};

/// Готовые подосновы для описания add_basemap, как READY_KEYS модуля.
const READY: [&str; 2] = ["opentopomap", "esri_topo"];

fn tools() -> Vec<ai::Tool> {
    ai::tools(&READY)
}

fn with_tools(tools: &[ai::Tool]) -> Options<'_> {
    Options { tools, max_tokens: None, stream: false }
}

fn header<'a>(request: &'a ai::Request, name: &str) -> Option<&'a str> {
    request.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

// TestAnthropic

#[test]
fn anthropic_request_and_round_trip() {
    let tools = tools();
    let mut dialog = vec![Message::user("Покажи Японию")];
    let req = ai::request(ai::ANTHROPIC, "https://h/", "m", "k", "sys", &dialog, &with_tools(&tools));
    assert_eq!(req.url, "https://h/v1/messages");
    assert_eq!(header(&req, "x-api-key"), Some("k"));
    assert_eq!(req.body["messages"][0]["content"], "Покажи Японию");
    let names: BTreeSet<&str> = req.body["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ai::TOOL_NAMES.iter().copied().collect());
    let reply = ai::parse(
        ai::ANTHROPIC,
        &json!({"content": [
            {"type": "text", "text": "Лечу."},
            {"type": "tool_use", "id": "t1", "name": "fly_to", "input": {"lat": 36.0, "lon": 138.0}}],
            "stop_reason": "tool_use"}),
    );
    assert_eq!(reply.text, "Лечу.");
    assert_eq!(reply.calls, vec![Call::new("t1", "fly_to", json!({"lat": 36.0, "lon": 138.0}))]);
    dialog.push(ai::assistant_message(&reply));
    dialog.push(ai::results_message(vec![ToolResult::new("t1", "готово")]));
    let body = ai::build_anthropic("m", "sys", &dialog, &tools);
    assert_eq!(body["messages"][1]["content"][1]["type"], "tool_use");
    assert_eq!(body["messages"][2]["content"][0], json!({"type": "tool_result", "tool_use_id": "t1", "content": "готово"}));
}

#[test]
fn anthropic_error() {
    let reply = ai::parse(
        ai::ANTHROPIC,
        &json!({"type": "error", "error": {"type": "authentication_error", "message": "invalid x-api-key"}}),
    );
    assert_eq!(reply.error, "invalid x-api-key");
}

// TestResponses

#[test]
fn responses_request_and_round_trip() {
    let tools = tools();
    let mut dialog = vec![Message::user("Где глубокие землетрясения?")];
    let req = ai::request(ai::RESPONSES, "https://api.x.ai/v1", "grok", "k", "sys", &dialog, &with_tools(&tools));
    assert_eq!(req.url, "https://api.x.ai/v1/responses");
    assert_eq!(header(&req, "Authorization"), Some("Bearer k"));
    assert_eq!(req.body["instructions"], "sys");
    assert_eq!(req.body["tools"][0]["type"], "function");
    let args = serde_json::to_string(&json!({"south": 30, "north": 45, "west": 130, "east": 150})).unwrap();
    let call = json!({"type": "function_call", "call_id": "c1", "name": "quakes_summary", "arguments": args});
    let reply = ai::parse(ai::RESPONSES, &json!({"output": [call.clone()]}));
    assert_eq!(reply.calls[0].name, "quakes_summary");
    assert_eq!(reply.calls[0].args["north"], 45);
    dialog.push(ai::assistant_message(&reply));
    dialog.push(ai::results_message(vec![ToolResult::new("c1", "12 событий")]));
    let body = ai::build_responses("grok", "sys", &dialog, &tools);
    // Вызов модели возвращается как есть, за ним - ответ инструмента.
    assert_eq!(body["input"][1], call);
    assert_eq!(body["input"][2], json!({"type": "function_call_output", "call_id": "c1", "output": "12 событий"}));
}

#[test]
fn responses_text_and_bad_arguments() {
    let reply = ai::parse(
        ai::RESPONSES,
        &json!({"output": [
            {"type": "message", "content": [{"type": "output_text", "text": "Готово."}]},
            {"type": "function_call", "call_id": "c2", "name": "fly_to", "arguments": "{oops"}]}),
    );
    assert_eq!(reply.text, "Готово.");
    assert_eq!(reply.calls[0].args, json!({}));
}

#[test]
fn responses_error() {
    let reply = ai::parse(ai::RESPONSES, &json!({"error": {"message": "bad key"}}));
    assert_eq!(reply.error, "bad key");
}

// TestChat

#[test]
fn chat_request_and_round_trip() {
    let tools = tools();
    let mut dialog = vec![Message::user("Покажи Японию")];
    let req = ai::request(ai::OPENROUTER, "https://openrouter.ai/api/v1/", "m:free", "k", "sys", &dialog, &with_tools(&tools));
    assert_eq!(req.url, "https://openrouter.ai/api/v1/chat/completions");
    assert_eq!(header(&req, "Authorization"), Some("Bearer k"));
    assert_eq!(req.body["messages"][0], json!({"role": "system", "content": "sys"}));
    assert_eq!(req.body["tools"][0]["function"]["name"], "fly_to");
    let reply = ai::parse(
        ai::OPENROUTER,
        &json!({"choices": [{"message": {
            "role": "assistant", "content": null, "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "fly_to", "arguments": "{\"lat\": 36, \"lon\": 138}"}}]}}]}),
    );
    assert_eq!(reply.calls[0].args, json!({"lat": 36, "lon": 138}));
    dialog.push(ai::assistant_message(&reply));
    dialog.push(ai::results_message(vec![ToolResult::new("c1", "Перелёт.")]));
    let body = ai::build_chat("m", "sys", &dialog, &tools);
    assert_eq!(body["messages"][2]["tool_calls"][0]["id"], "c1");
    assert_eq!(body["messages"][3], json!({"role": "tool", "tool_call_id": "c1", "content": "Перелёт."}));
}

#[test]
fn chat_arguments_written_as_python_json() {
    // Аргументы вызова идут строкой, как json.dumps модуля - с пробелом
    // после двоеточия и русскими буквами без \u.
    let reply = ai::Reply { text: String::new(), calls: vec![Call::new("c1", "search_place", json!({"query": "Пермь"}))], raw: None, error: String::new() };
    let dialog = vec![Message::user("q"), ai::assistant_message(&reply)];
    let body = ai::build_chat("m", "sys", &dialog, &[]);
    assert_eq!(body["messages"][2]["tool_calls"][0]["function"]["arguments"], "{\"query\": \"Пермь\"}");
    assert_eq!(body["messages"][2]["content"], "");
}

#[test]
fn chat_text_error_and_arguments_as_object() {
    let reply = ai::parse(
        ai::LOCAL,
        &json!({"choices": [{"message": {"content": "Готово.", "tool_calls": [{"function": {"name": "get_kml", "arguments": {}}}]}}]}),
    );
    assert_eq!(reply.text, "Готово.");
    assert_eq!(reply.calls[0].id, "call0");
    assert_eq!(ai::parse(ai::LOCAL, &json!({"error": {"message": "x"}})).error, "x");
    assert_eq!(ai::parse(ai::LOCAL, &json!({"choices": []})).error, "no choices");
}

#[test]
fn local_service_needs_no_key() {
    let req = ai::request(ai::LOCAL, "http://localhost:11434/v1", "qwen3", "", "sys", &[], &with_tools(&tools()));
    assert_eq!(header(&req, "Authorization"), None);
    assert!(!ai::needs_key("http://localhost:11434/v1"));
    assert!(!ai::needs_key("http://127.0.0.1:1234/v1"));
    assert!(!ai::needs_key("http://[::1]:11434/v1"));
    assert!(ai::needs_key("https://openrouter.ai/api/v1"));
}

#[test]
fn provider_error_details_and_cut_answer() {
    let reply = ai::parse(
        ai::OPENROUTER,
        &json!({"error": {"message": "Provider returned error", "code": 429,
            "metadata": {"provider_name": "ModelRun", "raw": "rate limited upstream"}}}),
    );
    assert_eq!(reply.error, "Provider returned error (ModelRun): rate limited upstream");
    // Рассуждение съело предел, ответа нет - это ошибка, а не тишина.
    let reply = ai::parse(ai::OPENROUTER, &json!({"choices": [{"finish_reason": "length", "message": {"content": null, "reasoning": "..."}}]}));
    assert!(reply.error.contains("finish_reason length"));
}

#[test]
fn chat_error_keeps_python_text() {
    // Не строка raw - JSON с пробелами, как json.dumps, не словарь -
    // как str() Python.
    let text = ai::chat_error(&json!({"message": "m", "metadata": {"raw": {"a": [1, 2.5]}}}));
    assert_eq!(text, "m: {\"a\": [1, 2.5]}");
    assert_eq!(ai::chat_error(&json!("плохо")), "плохо");
    assert_eq!(ai::chat_error(&json!({"code": 1})), "{'code': 1}");
}

#[test]
fn openrouter_asks_for_short_reasoning() {
    let tools = tools();
    let dialog = vec![Message::user("Привет")];
    let body = ai::request(ai::OPENROUTER, "https://openrouter.ai/api/v1", "m", "k", "sys", &dialog, &with_tools(&tools)).body;
    assert_eq!(body["reasoning"], json!({"effort": "low"}));
    assert_eq!(body["max_tokens"], ai::CHAT_MAX_TOKENS);
    assert!(body.get("models").is_none());
    let body = ai::request(ai::LOCAL, "http://localhost:11434/v1", "m", "", "sys", &dialog, &with_tools(&tools)).body;
    assert!(body.get("reasoning").is_none());
}

#[test]
fn free_model_falls_back_to_free_router() {
    let tools = tools();
    let body = ai::request(ai::OPENROUTER, "https://openrouter.ai/api/v1", "qwen/qwen3.8-27b:free", "k", "sys", &[], &with_tools(&tools)).body;
    assert_eq!(body["models"], json!(["qwen/qwen3.8-27b:free", "openrouter/free"]));
    assert_eq!(ai::defaults(ai::OPENROUTER).map(|d| d.1), Some("openrouter/free"));
    let body = ai::request(ai::OPENROUTER, "https://openrouter.ai/api/v1", "openrouter/free", "k", "sys", &[], &with_tools(&tools)).body;
    assert!(body.get("models").is_none());
}

#[test]
fn deepseek_speaks_anthropic() {
    let (base, model) = ai::defaults(ai::DEEPSEEK).unwrap();
    let req = ai::request(ai::DEEPSEEK, base, model, "k", "sys", &[Message::user("Привет")], &with_tools(&tools()));
    assert_eq!(req.url, "https://api.deepseek.com/anthropic/v1/messages");
    assert_eq!(header(&req, "x-api-key"), Some("k"));
    assert_eq!(req.body["system"], "sys");
}

#[test]
fn every_provider_has_format_and_defaults() {
    for provider in ai::PROVIDERS {
        assert!(ai::FORMAT.iter().any(|(p, _)| *p == provider), "{provider}");
        assert!(ai::defaults(provider).is_some(), "{provider}");
    }
}

// TestKmlGeneration

#[test]
fn request_without_tools_and_with_limit() {
    let dialog = vec![Message::user("Путешествие Колумба")];
    let options = Options { tools: &[], max_tokens: Some(ai::KML_MAX_TOKENS), stream: false };
    for (provider, field) in [(ai::ANTHROPIC, "max_tokens"), (ai::OPENROUTER, "max_tokens"), (ai::RESPONSES, "max_output_tokens")] {
        let base = ai::defaults(provider).unwrap().0;
        let body = ai::request(provider, base, "m", "k", &ai::kml_system_text(&json!({})), &dialog, &options).body;
        assert!(body.get("tools").is_none(), "{provider}");
        assert_eq!(body[field], ai::KML_MAX_TOKENS, "{provider}");
    }
    // Обычный запрос инструменты несёт.
    let tools = tools();
    assert!(ai::request(ai::ANTHROPIC, "b", "m", "k", "s", &dialog, &with_tools(&tools)).body.get("tools").is_some());
}

#[test]
fn extract_kml_from_fenced_answer() {
    let doc = "<kml xmlns=\"x\"><Document><name>А</name></Document></kml>";
    let text = format!("Вот документ:\n```xml\n<?xml version=\"1.0\"?>\n{doc}\n```\nГотово.");
    assert_eq!(ai::extract_kml(&text).as_deref(), Some(doc));
    assert_eq!(ai::extract_kml("Не могу."), None);
    assert_eq!(ai::extract_kml(""), None);
}

#[test]
fn extract_kml_closes_cut_document() {
    let text = concat!(
        "<kml xmlns=\"x\"><Document><name>А</name><Folder>",
        "<name>Ф</name><Placemark><name>1</name></Placemark>",
        "<Placemark><name>2</name><Point><coord"
    );
    let got = ai::extract_kml(text).unwrap();
    assert!(got.ends_with("<Placemark><name>1</name></Placemark></Folder></Document></kml>"), "{got}");
    // Оборван до первой целой метки - документа нет.
    assert_eq!(ai::extract_kml("<kml><Document><Placemark><na"), None);
}

#[test]
fn extract_kml_odd_answers() {
    let one = "<kml xmlns=\"x\"><Document><name>А</name></Document></kml>";
    assert_eq!(ai::extract_kml(&format!("{one}\nи ещё\n{one}")).as_deref(), Some(one));
    let got = ai::extract_kml("Вот:\n<Document><name>Б</name></Document>").unwrap();
    assert!(got.starts_with("<kml "));
    assert!(got.ends_with("</Document></kml>"));
    let cut = "<kml><Document><Folder><name>Ф</name></Folder><Folder><name>Г</name><LineString><coordinates>1,2";
    assert_eq!(ai::extract_kml(cut).as_deref(), Some("<kml><Document><Folder><name>Ф</name></Folder></Document></kml>"));
}

#[test]
fn prompt_asks_time_and_space_everywhere() {
    let text = ai::kml_system_text(&json!({}));
    let places = ai::KML_PLACES.to_string();
    for word in ["TimeStamp", "TimeSpan", "-0264", "этап", "растёт", "LineString", "Polygon", "сменяют", "styleUrl", places.as_str()] {
        assert!(text.contains(word), "{word}");
    }
    assert!(!text.contains("{places}"));
    // Разговор с инструментами получает те же правила документа.
    assert!(ai::system_text(&json!({})).contains("сменяют"));
}

#[test]
fn context_written_as_python_json() {
    let text = ai::system_text(&json!({"body": "Земля"}));
    assert!(text.ends_with("\nКонтекст вида: {\"body\": \"Земля\"}"), "{text}");
}

#[test]
fn stream_flag() {
    let dialog = vec![Message::user("Тема")];
    let options = Options { tools: &[], max_tokens: None, stream: true };
    for provider in [ai::ANTHROPIC, ai::OPENROUTER, ai::RESPONSES] {
        let body = ai::request(provider, "b", "m", "k", "s", &dialog, &options).body;
        assert_eq!(body["stream"], json!(true), "{provider}");
    }
    let tools = tools();
    assert!(ai::request(ai::ANTHROPIC, "b", "m", "k", "s", &dialog, &with_tools(&tools)).body.get("stream").is_none());
}

#[test]
fn length_refused() {
    assert!(ai::length_refused("max_tokens: 16000 > 8192, which is the maximum allowed"));
    assert!(ai::length_refused("Token limit exceeded"));
    assert!(!ai::length_refused("Provider returned error"));
    assert!(!ai::length_refused(""));
}

#[test]
fn time_span_and_bc_dates_read() {
    let text = concat!(
        "<kml xmlns=\"http://www.opengis.net/kml/2.2\"><Document>",
        "<name>Пунические войны</name><Placemark><name>Первая ",
        "война</name><TimeSpan><begin>-0263</begin><end>-0240",
        "</end></TimeSpan><Point><coordinates>10.3,36.8,0",
        "</coordinates></Point></Placemark><Placemark><name>Канны",
        "</name><TimeStamp><when>-0215-08-02</when></TimeStamp>",
        "<Point><coordinates>16.13,41.3,0</coordinates></Point>",
        "</Placemark></Document></kml>"
    );
    let tree = scene::kml::read_kml(ai::extract_kml(text).unwrap().as_bytes(), "").unwrap();
    let times: Vec<_> = tree.places().iter().map(|p| p.time.clone()).collect();
    assert_eq!(times[0], Some(("-0263".to_string(), "-0240".to_string())));
    assert_eq!(times[1], Some(("-0215-08-02".to_string(), "-0215-08-02".to_string())));
}

#[test]
fn cut_document_reads_as_kml() {
    let text = concat!(
        "<kml xmlns=\"http://www.opengis.net/kml/2.2\"><Document>",
        "<name>Колумб</name><Folder><name>Первое</name><Placemark>",
        "<name>Палос</name><Point><coordinates>-6.89,37.23,0",
        "</coordinates></Point></Placemark><Placemark><name>Сан"
    );
    let tree = scene::kml::read_kml(ai::extract_kml(text).unwrap().as_bytes(), "").unwrap();
    let names: Vec<&str> = tree.places().iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Палос"]);
}

// TestStreamText

/// Поток SSE из событий: объект JSON - строка data, строка - как есть.
fn sse(events: &[Value]) -> Vec<u8> {
    let mut lines = Vec::new();
    for event in events {
        match event {
            Value::String(s) => lines.push(s.clone()),
            other => lines.push(format!("data: {}", serde_json::to_string(other).unwrap())),
        }
        lines.push(String::new());
    }
    (lines.join("\n") + "\n").into_bytes()
}

fn feed_in_pieces(provider: &str, data: &[u8], size: usize) -> (ai::StreamText, usize) {
    let mut stream = ai::StreamText::new(provider);
    let mut grew = 0;
    for piece in data.chunks(size) {
        grew += usize::from(stream.feed(piece));
    }
    stream.finish();
    (stream, grew)
}

#[test]
fn chat_deltas_in_any_cut() {
    let data = sse(&[
        json!(": OPENROUTER PROCESSING"),
        json!({"choices": [{"delta": {"reasoning": "думаю"}}]}),
        json!({"choices": [{"delta": {"content": "<kml><Docu"}}]}),
        json!({"choices": [{"delta": {"content": "ment>Колумб"}}]}),
        json!({"choices": [{"delta": {}, "finish_reason": "stop"}]}),
        json!("data: [DONE]"),
    ]);
    // Порез по 1, 3 и 7 байт рвёт строки и символы UTF-8.
    for size in [1, 3, 7, 4096] {
        let (stream, grew) = feed_in_pieces(ai::OPENROUTER, &data, size);
        assert_eq!(stream.text, "<kml><Document>Колумб", "{size}");
        assert_eq!(stream.error, "");
        assert!(!stream.cut);
        assert!(stream.done);
        assert!(grew >= 1);
    }
}

#[test]
fn chat_cut_and_error() {
    let (stream, _) = feed_in_pieces(ai::LOCAL, &sse(&[json!({"choices": [{"delta": {"content": "<kml>"}, "finish_reason": "length"}]})]), 5);
    assert!(stream.cut);
    let (stream, _) = feed_in_pieces(
        ai::OPENROUTER,
        &sse(&[
            json!({"choices": [{"delta": {"content": "<kml>"}}]}),
            json!({"error": {"message": "Provider returned error", "metadata": {"provider_name": "ModelRun", "raw": "overloaded"}}}),
        ]),
        9,
    );
    assert_eq!(stream.error, "Provider returned error (ModelRun): overloaded");
    // Пришедший до ошибки текст остаётся.
    assert_eq!(stream.text, "<kml>");
}

#[test]
fn anthropic_events() {
    let data = sse(&[
        json!("event: message_start"),
        json!({"type": "message_start", "message": {"id": "m"}}),
        json!("event: ping"),
        json!({"type": "ping"}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "думаю"}}),
        json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "<kml>"}}),
        json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "</kml>"}}),
        json!({"type": "message_delta", "delta": {"stop_reason": "max_tokens"}}),
        json!({"type": "message_stop"}),
    ]);
    let (stream, _) = feed_in_pieces(ai::ANTHROPIC, &data, 11);
    assert_eq!(stream.text, "<kml></kml>");
    assert!(stream.cut);
    let (stream, _) = feed_in_pieces(ai::DEEPSEEK, &sse(&[json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}})]), 6);
    assert_eq!(stream.error, "Overloaded");
}

#[test]
fn responses_events() {
    let data = sse(&[
        json!({"type": "response.created", "response": {}}),
        json!({"type": "response.output_text.delta", "delta": "<kml>"}),
        json!({"type": "response.output_text.delta", "delta": "</kml>"}),
        json!({"type": "response.completed", "response": {}}),
    ]);
    let (stream, _) = feed_in_pieces(ai::RESPONSES, &data, 13);
    assert_eq!(stream.text, "<kml></kml>");
    assert!(!stream.cut);
    let (stream, _) = feed_in_pieces(
        ai::RESPONSES,
        &sse(&[json!({"type": "response.output_text.delta", "delta": "<kml>"}), json!({"type": "response.incomplete", "response": {}})]),
        13,
    );
    assert!(stream.cut);
    let (stream, _) = feed_in_pieces(ai::RESPONSES, &sse(&[json!({"type": "response.failed", "response": {"error": {"message": "bad key"}}})]), 13);
    assert_eq!(stream.error, "bad key");
}

#[test]
fn plain_json_answer_instead_of_stream() {
    // Ошибка HTTP приходит телом JSON, не потоком.
    let refused = serde_json::to_vec(&json!({"error": {"message": "max_tokens: 16000 > 8192"}})).unwrap();
    let (stream, _) = feed_in_pieces(ai::OPENROUTER, &refused, 10);
    assert_eq!(stream.error, "max_tokens: 16000 > 8192");
    assert!(ai::length_refused(&stream.error));
    // Сервис без потока отвечает обычным телом - текст берётся.
    let whole = serde_json::to_vec(&json!({"choices": [{"message": {"content": "<kml></kml>"}}]})).unwrap();
    let (stream, _) = feed_in_pieces(ai::LOCAL, &whole, 10);
    assert_eq!(stream.text, "<kml></kml>");
    assert_eq!(stream.error, "");
    let whole = serde_json::to_vec(&json!({"type": "error", "error": {"message": "invalid x-api-key"}})).unwrap();
    let (stream, _) = feed_in_pieces(ai::ANTHROPIC, &whole, 10);
    assert_eq!(stream.error, "invalid x-api-key");
}

#[test]
fn progress_counts_whole_placemarks() {
    let text = "<kml><Document><Placemark><name>1</name></Placemark><Placemark><name>2</name></Placemark><Placemark><na";
    assert_eq!(ai::kml_progress(text), 2);
    assert_eq!(ai::kml_progress(""), 0);
}

// TestIsRequest

#[test]
fn places_and_coordinates_are_not_requests() {
    for text in ["Пермь", "Нью-Йорк", "Красная площадь", "Mount Everest", "58.0105, 56.2294", "ул. Ленина 1, Пермь", ""] {
        assert!(!ai::is_request(text), "{text}");
    }
}

#[test]
fn requests() {
    for text in [
        "покажи разрез через Японский жёлоб",
        "Где самые глубокие землетрясения у Японии",
        "что под Березниками?",
        "Show Mars",
        "fly, please, to the deepest point of the ocean now",
        "Эльбрус?",
    ] {
        assert!(ai::is_request(text), "{text}");
    }
}

// TestHelpers

#[test]
fn look_at_kml_reads_back() {
    let text = ai::look_at_kml(38.5, 142.0, 500000.0, 30.0, 60.0);
    assert!(text.contains("<range>500000</range>"));
    assert!(text.contains("<longitude>142.000000</longitude><latitude>38.500000</latitude>"));
    assert!(scene::kml::read_kml(text.as_bytes(), "вид").is_ok());
}

#[test]
fn quakes_text() {
    let quake = |lat, lon, depth, mag, place: &str| ai::Quake { lat, lon, depth, mag, place: place.to_string() };
    let events = [quake(38.0, 142.0, 30.0, 6.1, "A"), quake(40.0, 179.5, 600.0, 5.0, "B"), quake(0.0, 0.0, 10.0, 7.0, "C")];
    let text = ai::quakes_text(&events, (30.0, 45.0, 140.0, -170.0), 3);
    assert!(text.contains("Событий в рамке: 2."));
    assert!(!text.contains(", C"));
    assert!(text.find("M6.1").unwrap() < text.find("Самые глубокие").unwrap());
    assert!(text.contains("M6.1, глубина 30 км, 38.00, 142.00, A"));
    assert!(ai::quakes_text(&events, (-10.0, -5.0, 0.0, 1.0), 3).contains("нет"));
}

#[test]
fn actions_for_assistant() {
    let item = |key: &str, text: &str, note: &str| ai::Action { key: key.to_string(), text: text.to_string(), note: note.to_string() };
    // Окна выбора файла и запись облёта с 25 октября 2026 года
    // доступны по разрешению пользователя, разговор - нет.
    let actions = [
        item("layer:grid", "Координатная сетка", "включить"),
        item("tool:assistant", "Помощник", "открыть окно"),
        item("map:weather_aurora", "Полярное сияние", ""),
    ];
    let allowed = ai::assistant_actions(&actions);
    let keys: Vec<&str> = allowed.iter().map(|a| a.key.as_str()).collect();
    assert_eq!(keys, ["layer:grid", "map:weather_aurora"]);
    let text = ai::actions_text(&allowed);
    assert_eq!(text.lines().collect::<Vec<_>>(), ["layer:grid - Координатная сетка: включить", "map:weather_aurora - Полярное сияние"]);
    assert_eq!(ai::actions_text(&[]), "Действий нет.");
    for name in ["list_actions", "run_action", "set_opacity"] {
        assert!(ai::TOOL_NAMES.contains(&name));
    }
    assert!(ai::system_text(&json!({})).contains("run_action"));
}

#[test]
fn tools_have_schemas() {
    assert!(ai::TOOL_NAMES.contains(&"add_kml"));
    assert!(ai::TOOL_NAMES.contains(&"get_kml"));
    let tools = tools();
    assert_eq!(tools.len(), ai::TOOL_NAMES.len());
    for (tool, name) in tools.iter().zip(ai::TOOL_NAMES) {
        assert_eq!(tool.name, name);
        assert_eq!(tool.schema["type"], "object", "{name}");
        assert!(!tool.description.is_empty(), "{name}");
    }
}

#[test]
fn basemap_description_names_ready_keys() {
    let tools = tools();
    let basemap = tools.iter().find(|t| t.name == "add_basemap").unwrap();
    assert!(basemap.description.contains("ready - ключ готового источника (opentopomap, esri_topo), или url"));
}

/// У Lite нет руководства и проекта QGIS: этих инструментов и слов о них
/// в подсказке нет.
#[test]
fn lite_has_no_manual_and_no_project() {
    let names: Vec<&str> = assistant::lite_tools(&["esri"]).iter().map(|t| t.name).collect();
    assert!(names.contains(&"fly_to") && names.contains(&"add_kml"));
    assert!(!names.iter().any(|n| assistant::NOT_IN_LITE.contains(n)));
    let text = assistant::lite_system_text(&serde_json::json!({"body": "earth"}));
    assert!(text.contains("PlanetX Lite") && !text.contains("QGIS") && !text.contains("search_docs"));
    assert!(text.ends_with("{\"body\": \"earth\"}"));
}
