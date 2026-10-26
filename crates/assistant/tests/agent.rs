// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Круг разговора помощника, разрешения, бюджет ответов и руководство,
//! перенос `tests/test_assistant_agent.py` модуля.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use assistant as ai;
use assistant::{Agent, Call, Choice, DocIndex, Message, Reply, StepKind, ToolResult};
use serde_json::{Value, json};

fn reply(text: &str, calls: Vec<Call>, error: &str) -> Reply {
    Reply { text: text.to_string(), calls, raw: None, error: error.to_string() }
}

fn call(name: &str, n: u32, args: Value) -> Call {
    Call::new(&format!("c{n}"), name, args)
}

/// Руководство для проверки: разделы (название, якорь, текст), поиск
/// по началам слов запроса. Настоящий указатель модуля - manualindex.
struct Index(Vec<(&'static str, &'static str, &'static str)>);

fn words(text: &str) -> Vec<String> {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 3).map(str::to_string).collect()
}

impl DocIndex for Index {
    fn count(&self) -> usize {
        self.0.len()
    }

    fn section(&self, n: usize) -> Option<(&str, &str, &str)> {
        self.0.get(n).map(|&(t, a, x)| (t, a, x))
    }

    fn search(&self, query: &str, limit: usize) -> Vec<usize> {
        let asked = words(query);
        let mut scored: Vec<(usize, usize)> = self
            .0
            .iter()
            .enumerate()
            .map(|(n, (title, _, text))| {
                let have = words(&format!("{title} {text}"));
                (asked.iter().filter(|q| have.iter().any(|w| w.starts_with(q.as_str()))).count(), n)
            })
            .filter(|(score, _)| *score > 0)
            .collect();
        scored.sort_by_key(|a| std::cmp::Reverse(a.0));
        scored.into_iter().take(limit).map(|(_, n)| n).collect()
    }

    fn snippet(&self, n: usize, _query: &str) -> String {
        self.0.get(n).and_then(|s| s.2.split("\n\n").next()).unwrap_or("").to_string()
    }

    fn find(&self, anchor: &str) -> Option<usize> {
        self.0.iter().position(|s| s.1 == anchor)
    }
}

fn index() -> Index {
    Index(vec![
        ("Линейка", "линейка", "Линейка меряет расстояние.\n\nВторая строка.\nТретья."),
        ("Снимок вида", "снимок-вида", "Снимок вида пишет картинку."),
    ])
}

// TestDocs

#[test]
fn search_gives_anchor_and_title() {
    let text = ai::search_docs_text(&index(), "линейка расстояние", ai::DOC_HITS);
    assert!(text.starts_with("линейка - Линейка:"), "{text}");
}

#[test]
fn nothing_found() {
    assert!(ai::search_docs_text(&index(), "зебры", ai::DOC_HITS).contains("ничего"));
}

#[test]
fn read_part_and_next_offset() {
    let text = ai::read_doc_text(&index(), "#линейка", 1, 2);
    assert!(text.contains("Строки 2-3 из 4."), "{text}");
    assert!(text.contains("Вторая строка."));
    assert!(!text.contains("меряет"));
    assert!(text.contains("offset 3"));
}

#[test]
fn read_by_title_and_unknown() {
    assert!(ai::read_doc_text(&index(), "Снимок вида", 0, 0).contains("картинку"));
    assert!(ai::read_doc_text(&index(), "нет-такого", 0, 0).contains("нет"));
}

#[test]
fn read_whole_section_and_lines_like_python() {
    // splitlines Python: \r\n - один разрыв, последний разрыв строки
    // пустой строки не даёт.
    let index = Index(vec![("А", "а", "один\r\nдва\rтри\n")]);
    assert_eq!(ai::read_doc_text(&index, "а", 0, 0), "## А\nСтроки 1-3 из 3.\nодин\nдва\nтри");
    assert_eq!(ai::read_doc_text(&index, "а", 5, 0), "## А\nСтроки 5-5 из 3.\n");
}

// TestApproval

#[test]
fn gated() {
    assert!(ai::needs_approval(&call("read_features", 1, json!({"layer": "1"}))));
    assert!(ai::needs_approval(&call("list_project_layers", 1, json!({}))));
    assert!(ai::needs_approval(&call("run_action", 1, json!({"key": "tool:scene_save"}))));
    assert!(!ai::needs_approval(&call("run_action", 1, json!({"key": "extra:grid"}))));
    assert!(!ai::needs_approval(&call("fly_to", 1, json!({"lat": 1, "lon": 2}))));
}

#[test]
fn assistant_window_stays_skipped() {
    assert_eq!(ai::ACTION_SKIP, ["tool:assistant"]);
    assert!(!ai::ACTION_GATED.contains(&"tool:assistant"));
}

#[test]
fn key_ignores_argument_order() {
    let a = Call::new("x", "read_features", json!({"layer": "1", "limit": 5}));
    let b = Call::new("y", "read_features", json!({"limit": 5, "layer": "1"}));
    let c = Call::new("z", "read_features", json!({"layer": "2", "limit": 5}));
    assert_eq!(ai::approval_key(&a), ai::approval_key(&b));
    assert_ne!(ai::approval_key(&a), ai::approval_key(&c));
}

#[test]
fn key_is_python_json() {
    // json.dumps(sort_keys=True, ensure_ascii=False) модуля.
    let a = Call::new("x", "read_features", json!({"where": "имя = 'А'", "layer": "1", "limit": 5, "fields": ["a", "b"], "z": 2.0}));
    assert_eq!(ai::approval_key(&a), "read_features {\"fields\": [\"a\", \"b\"], \"layer\": \"1\", \"limit\": 5, \"where\": \"имя = 'А'\", \"z\": 2.0}");
    assert_eq!(ai::approval_key(&Call::new("x", "list_project_layers", Value::Null)), "list_project_layers {}");
}

// TestBudget

#[test]
fn bounded_keeps_head_and_tail() {
    let text = "А".repeat(50) + &"Б".repeat(50);
    let out = ai::bounded(&text, 30);
    assert!(out.starts_with(&"А".repeat(20)));
    assert!(out.ends_with(&"Б".repeat(10)));
    assert!(out.contains("пропущено 70"));
    assert_eq!(ai::bounded("коротко", 30), "коротко");
}

#[test]
fn trimmed_old_results_only() {
    let mut dialog = Vec::new();
    for n in 0..4 {
        dialog.push(Message::user("q"));
        dialog.push(ai::results_message(vec![ToolResult::new(&format!("r{n}"), &"x".repeat(100))]));
    }
    let out = ai::trimmed(&dialog, 250, 2);
    let texts: Vec<&str> = out.iter().filter(|m| !m.results.is_empty()).map(|m| m.results[0].text.as_str()).collect();
    assert_eq!(texts[..2], [ai::TRIMMED, ai::TRIMMED]);
    let long = "x".repeat(100);
    assert_eq!(texts[2..], [long.as_str(), long.as_str()]);
    // Сам разговор не меняется.
    assert_eq!(dialog[1].results[0].text, long);
    assert_eq!(ai::trimmed(&dialog, 1_000_000, 2), dialog);
}

#[test]
fn transient() {
    for text in ["Overloaded", "rate-limited upstream", "server replied: Service Unavailable", "Error 529", "Too Many Requests"] {
        assert!(ai::transient(text), "{text}");
    }
    for text in ["Connection refused", "invalid x-api-key", ""] {
        assert!(!ai::transient(text), "{text}");
    }
}

// TestAgent

#[test]
fn plain_answer() {
    let mut agent = Agent::new();
    assert_eq!(agent.ask("привет").kind, StepKind::Send);
    let step = agent.answered(reply("здравствуй", vec![], ""), "");
    assert_eq!((step.kind, step.text.as_str()), (StepKind::Done, "здравствуй"));
}

#[test]
fn runs_then_confirms_then_remembers() {
    let mut agent = Agent::new();
    agent.ask("слои");
    let gated = call("read_features", 2, json!({"layer": "1"}));
    let step = agent.answered(reply("", vec![call("fly_to", 1, json!({"lat": 1, "lon": 2})), gated.clone()], ""), "");
    assert_eq!(step.kind, StepKind::Run);
    assert!(agent.busy());
    let step = agent.result("ok");
    assert_eq!((step.kind, step.call), (StepKind::Confirm, Some(gated)));
    let step = agent.decide(Choice::AllowAlways);
    assert_eq!(step.kind, StepKind::Run);
    assert_eq!(agent.result("строки").kind, StepKind::Send);
    assert!(!agent.busy());
    // Тот же вызов в этом разговоре - без вопроса, другой - с ним.
    let step = agent.answered(reply("", vec![Call::new("c3", "read_features", json!({"layer": "1"}))], ""), "");
    assert_eq!(step.kind, StepKind::Run);
    agent.result("строки");
    let step = agent.answered(reply("", vec![call("read_features", 4, json!({"layer": "2"}))], ""), "");
    assert_eq!(step.kind, StepKind::Confirm);
}

#[test]
fn deny_answers_model() {
    let mut agent = Agent::new();
    agent.ask("слои");
    agent.answered(reply("", vec![call("list_project_layers", 1, json!({}))], ""), "");
    assert_eq!(agent.decide(Choice::Deny).kind, StepKind::Send);
    assert_eq!(agent.dialog.last().unwrap().results[0].text, ai::DENIED);
}

#[test]
fn long_result_bounded() {
    let mut agent = Agent::new();
    agent.ask("q");
    agent.answered(reply("", vec![call("fly_to", 1, json!({"lat": 1, "lon": 2}))], ""), "");
    agent.result(&"x".repeat(ai::OUTPUT_CHARS * 2));
    let text = &agent.dialog.last().unwrap().results[0].text;
    assert!(text.chars().count() < ai::OUTPUT_CHARS + 100);
}

#[test]
fn retry_then_fail_rolls_back() {
    let mut agent = Agent::new();
    agent.ask("q");
    let mut delays = Vec::new();
    for _ in 0..ai::MAX_RETRIES {
        let step = agent.answered(reply("", vec![], "Overloaded"), "");
        assert_eq!(step.kind, StepKind::Retry);
        delays.push(step.delay);
    }
    assert_eq!(delays, ai::RETRY_DELAYS);
    assert_eq!(agent.answered(reply("", vec![], "Overloaded"), "").kind, StepKind::Failed);
    assert!(agent.dialog.is_empty());
}

#[test]
fn network_error_wins_over_no_data() {
    let mut agent = Agent::new();
    agent.ask("q");
    let step = agent.answered(reply("", vec![], "no data"), "Connection refused");
    assert_eq!((step.kind, step.text.as_str()), (StepKind::Failed, "Connection refused"));
}

#[test]
fn retry_count_resets_after_answer() {
    let mut agent = Agent::new();
    agent.ask("q");
    agent.answered(reply("", vec![], "Too Many Requests"), "");
    agent.answered(reply("", vec![call("fly_to", 1, json!({"lat": 1, "lon": 2}))], ""), "");
    agent.result("ok");
    assert_eq!(agent.retries, 0);
}

#[test]
fn limit_and_resume() {
    let mut agent = Agent::new();
    agent.ask("q");
    let mut step = None;
    for n in 0..ai::MAX_ROUNDS {
        agent.answered(reply("", vec![call("fly_to", n, json!({"lat": 1, "lon": 2}))], ""), "");
        step = Some(agent.result("ok"));
    }
    assert_eq!(step.unwrap().kind, StepKind::Limit);
    assert_eq!(agent.resume().kind, StepKind::Send);
    assert_eq!(agent.rounds, 0);
}

/// Сторож: вопрос после остановки на ответах инструментов шёл
/// сообщением пользователя подряд за ответами.
#[test]
fn roles_alternate_after_cancel() {
    let mut agent = Agent::new();
    agent.ask("q");
    agent.answered(reply("", vec![call("list_project_layers", 1, json!({}))], ""), "");
    agent.cancel();
    assert_eq!(agent.dialog.last().unwrap().results[0].text, ai::CANCELLED);
    agent.ask("другое");
    let body = ai::build_anthropic("m", "s", &agent.dialog_for_request(), &[]);
    let roles: Vec<&str> = body["messages"].as_array().unwrap().iter().map(|m| m["role"].as_str().unwrap()).collect();
    assert_eq!(roles, ["user", "assistant", "user", "assistant", "user"]);
    assert_eq!(body["messages"][3]["content"][0]["text"], ai::STOPPED);
}

#[test]
fn own_gate() {
    let mut agent = Agent::with_gate(Box::new(|c: &Call| c.name == "fly_to"));
    agent.ask("q");
    let step = agent.answered(reply("", vec![call("fly_to", 1, json!({}))], ""), "");
    assert_eq!(step.kind, StepKind::Confirm);
}

#[test]
fn stray_answers_change_nothing() {
    // Ответ инструмента и решение без ждущего вызова - модуль упал бы
    // IndexError, Lite ничего не меняет.
    let mut agent = Agent::new();
    agent.ask("q");
    assert_eq!(agent.result("лишнее").kind, StepKind::Done);
    assert_eq!(agent.decide(Choice::Allow).kind, StepKind::Done);
    assert_eq!(agent.dialog.len(), 1);
}
