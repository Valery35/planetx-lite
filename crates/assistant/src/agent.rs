// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Круг разговора с моделью без сети и интерфейса, по образцу Agent
//! RustGIS. Окно посылает запросы, исполняет вызовы и спрашивает
//! пользователя, а что делать дальше, решает [`Agent`]: каждый метод
//! возвращает следующий [`Step`].

use std::collections::{HashSet, VecDeque};

use crate::{
    Call, DIALOG_CHARS, KEEP_RESULTS, MAX_RETRIES, MAX_ROUNDS, Message, OUTPUT_CHARS, RETRY_DELAYS, Reply, Role, ToolResult,
    approval_key, assistant_message, bounded, needs_approval, results_message, transient, trimmed,
};

/// Что окно делает дальше.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    /// Послать запрос с [`Agent::dialog_for_request`].
    Send,
    /// Исполнить `step.call` и отдать ответ [`Agent::result`].
    Run,
    /// Спросить пользователя о `step.call`, ответ - [`Agent::decide`].
    Confirm,
    /// Подождать `step.delay` с и послать запрос снова.
    Retry,
    /// Круги кончились, [`Agent::resume`] по «Продолжить».
    Limit,
    /// Модель ответила текстом `step.text`.
    Done,
    /// Модель не ответила, причина - `step.text`.
    Failed,
}

/// Шаг круга разговора: вид, вызов, текст и пауза в секундах.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub kind: StepKind,
    pub call: Option<Call>,
    pub text: String,
    pub delay: f64,
}

impl Step {
    fn new(kind: StepKind) -> Self {
        Self { kind, call: None, text: String::new(), delay: 0.0 }
    }

    fn with_call(kind: StepKind, call: Call) -> Self {
        Self { call: Some(call), ..Self::new(kind) }
    }

    fn with_text(kind: StepKind, text: &str) -> Self {
        Self { text: text.to_string(), ..Self::new(kind) }
    }
}

/// Ответ пользователя на вопрос о вызове.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// Этот раз.
    Allow,
    /// Этот же вызов до конца разговора.
    AllowAlways,
    Deny,
}

/// Ответ инструмента после отказа пользователя.
pub const DENIED: &str = "Пользователь отказал в этом действии. Не повторяй его, ответь без него.";
/// Ответ ждущего вызова после остановки разговора.
pub const CANCELLED: &str = "Действие отменено: пользователь остановил разговор.";
/// Реплика модели между остановленными ответами инструментов и новым
/// вопросом.
pub const STOPPED: &str = "(разговор остановлен пользователем)";

/// Нужно ли разрешение пользователя на вызов.
pub type Gate = Box<dyn Fn(&Call) -> bool>;

/// Круг разговора. Разрешения «в этом разговоре» живут, пока жив Agent,
/// новый разговор - новый Agent.
pub struct Agent {
    pub dialog: Vec<Message>,
    pub rounds: u32,
    pub retries: u32,
    /// Ключи [`approval_key`] вызовов, разрешённых до конца разговора.
    pub allowed: HashSet<String>,
    gated: Gate,
    calls: VecDeque<Call>,
    results: Vec<ToolResult>,
}

impl Default for Agent {
    fn default() -> Self {
        Self::new()
    }
}

impl Agent {
    /// Разговор с разрешениями [`needs_approval`].
    pub fn new() -> Self {
        Self::with_gate(Box::new(needs_approval))
    }

    /// Разговор со своим правилом разрешений.
    pub fn with_gate(gated: Gate) -> Self {
        Self { dialog: Vec::new(), rounds: 0, retries: 0, allowed: HashSet::new(), gated, calls: VecDeque::new(), results: Vec::new() }
    }

    /// Ждут ли вызовы своей очереди или ответа пользователя.
    pub fn busy(&self) -> bool {
        !self.calls.is_empty()
    }

    /// Вопрос пользователя. Если прежний круг остановлен на ответах
    /// инструментов, между ними и вопросом встаёт короткая реплика
    /// модели: сообщения пользователя подряд принимают не все форматы.
    pub fn ask(&mut self, text: &str) -> Step {
        if self.dialog.last().is_some_and(Message::has_results) {
            self.dialog.push(Message { role: Role::Assistant, text: STOPPED.to_string(), ..Message::default() });
        }
        self.dialog.push(Message::user(text));
        self.rounds = 0;
        self.retries = 0;
        Step::new(StepKind::Send)
    }

    /// Разговор для запроса - с сокращёнными старыми ответами.
    pub fn dialog_for_request(&self) -> Vec<Message> {
        trimmed(&self.dialog, DIALOG_CHARS, KEEP_RESULTS)
    }

    /// Ответ модели: [`Reply`] и ошибка сети, если тела ответа нет -
    /// тогда причина в ней, а не в «no data». Временный отказ
    /// повторяется не больше `MAX_RETRIES` раз.
    pub fn answered(&mut self, reply: Reply, error: &str) -> Step {
        let reason = if error.is_empty() { reply.error.as_str() } else { error };
        if !reason.is_empty() {
            if transient(reason) && self.retries < MAX_RETRIES {
                let n = usize::try_from(self.retries).unwrap_or(usize::MAX).min(RETRY_DELAYS.len() - 1);
                let delay = RETRY_DELAYS.get(n).copied().unwrap_or(0.0);
                self.retries += 1;
                return Step { delay, ..Step::with_text(StepKind::Retry, reason) };
            }
            let step = Step::with_text(StepKind::Failed, reason);
            self.rollback();
            return step;
        }
        self.retries = 0;
        self.dialog.push(assistant_message(&reply));
        if reply.calls.is_empty() {
            return Step::with_text(StepKind::Done, &reply.text);
        }
        self.calls = reply.calls.into_iter().collect();
        self.results = Vec::new();
        self.next()
    }

    /// Ответ инструмента на вызов, который был в шаге `Run`. Без ждущего
    /// вызова ничего не меняет и даёт `Done`.
    pub fn result(&mut self, text: &str) -> Step {
        let Some(call) = self.calls.pop_front() else {
            return Step::new(StepKind::Done);
        };
        self.results.push(ToolResult { id: call.id, text: bounded(text, OUTPUT_CHARS) });
        self.next()
    }

    /// Ответ пользователя на шаг `Confirm`. Без ждущего вызова ничего
    /// не меняет и даёт `Done`.
    pub fn decide(&mut self, choice: Choice) -> Step {
        let Some(call) = self.calls.front().cloned() else {
            return Step::new(StepKind::Done);
        };
        match choice {
            Choice::Deny => self.result(DENIED),
            Choice::AllowAlways => {
                self.allowed.insert(approval_key(&call));
                Step::with_call(StepKind::Run, call)
            }
            Choice::Allow => Step::with_call(StepKind::Run, call),
        }
    }

    /// «Продолжить» после `Limit`: ещё `MAX_ROUNDS` кругов.
    pub fn resume(&mut self) -> Step {
        self.rounds = 0;
        Step::new(StepKind::Send)
    }

    /// Остановить разговор: ждущие вызовы получают ответ «отменено»,
    /// иначе следующий запрос был бы неполным.
    pub fn cancel(&mut self) -> Step {
        if !self.calls.is_empty() {
            let mut results = std::mem::take(&mut self.results);
            results.extend(self.calls.drain(..).map(|c| ToolResult { id: c.id, text: CANCELLED.to_string() }));
            self.dialog.push(results_message(results));
        }
        Step::new(StepKind::Done)
    }

    fn next(&mut self) -> Step {
        if let Some(call) = self.calls.front() {
            if (self.gated)(call) && !self.allowed.contains(&approval_key(call)) {
                return Step::with_call(StepKind::Confirm, call.clone());
            }
            return Step::with_call(StepKind::Run, call.clone());
        }
        let results = std::mem::take(&mut self.results);
        self.dialog.push(results_message(results));
        self.rounds += 1;
        if self.rounds >= MAX_ROUNDS {
            return Step::new(StepKind::Limit);
        }
        Step::new(StepKind::Send)
    }

    /// Неудачный вопрос не остаётся в разговоре: разговор откатывается
    /// к состоянию до последнего вопроса пользователя вместе с начатыми
    /// вызовами инструментов.
    fn rollback(&mut self) {
        if let Some(n) = self.dialog.iter().rposition(|m| m.role == Role::User && !m.has_results()) {
            self.dialog.truncate(n);
        }
    }
}
