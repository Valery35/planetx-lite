// PlanetX Lite - глобус PlanetX без QGIS.
// Copyright (C) 2026 ООО «Информ++». Лицензия Apache 2.0.
// SPDX-License-Identifier: Apache-2.0

//! Окно «Помощник»: разговор с моделью, которая управляет глобусом.
//! Перенос `ui/assistant.py` модуля, круг разговора, запросы и разбор
//! ответов - крейт `assistant`, инструменты исполняет окно глобуса.
//!
//! Подключения, адреса, модели и умолчания - как у модуля: по
//! умолчанию OpenRouter с бесплатным маршрутизатором моделей. Ключ API
//! хранится открытым текстом в настройках программы, как ключ модуля
//! с флажком «Хранить ключ без мастер-пароля», включённым по умолчанию.
//! На странице настройки лежат в хранилище браузера.
//!
//! Разговор: вопрос -> модель -> вызовы инструментов -> их ответы ->
//! модель, пока модель не ответит текстом. Круг ведёт
//! `assistant::Agent`, окно посылает запросы, отдаёт вызовы окну
//! глобуса и спрашивает пользователя - разрешить действие или
//! продолжить после многих кругов. Предложенные метки KML пишутся
//! в «Мои метки» по кнопке. «Создать метки» - один запрос без
//! инструментов, ответ - документ KML. У модуля ответ идёт потоком,
//! у Lite - целиком: сети Lite поток ответа пока не отдаёт.

use eframe::egui;
use serde_json::Value;

use assistant::{self as aic, Agent, Call, Choice, Options, Step, StepKind};

use crate::i18n::{Lang, tr, trf};

/// Подключение по умолчанию, `PROVIDER_DEFAULT` модуля.
pub const PROVIDER_DEFAULT: &str = aic::OPENROUTER;
/// Отвечать на просьбы из строки «Поиск», `SEARCH_DEFAULT` модуля.
pub const SEARCH_DEFAULT: bool = true;
/// Наибольший ответ модели, байт: ответ ждётся до долгого предела сети.
pub const ANSWER_BYTES: u64 = 16 << 20;

/// Настройки помощника: подключение, свои адреса и модели, ключи.
#[derive(Debug, Clone, PartialEq)]
pub struct AiSettings {
    pub provider: String,
    /// Свои адреса подключений: подключение, адрес.
    pub bases: Vec<(String, String)>,
    pub models: Vec<(String, String)>,
    pub keys: Vec<(String, String)>,
    pub search: bool,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self { provider: PROVIDER_DEFAULT.into(), bases: Vec::new(), models: Vec::new(), keys: Vec::new(), search: SEARCH_DEFAULT }
    }
}

fn get<'a>(list: &'a [(String, String)], provider: &str) -> Option<&'a str> {
    list.iter().find(|(p, _)| p == provider).map(|(_, v)| v.as_str()).filter(|v| !v.is_empty())
}

fn put(list: &mut Vec<(String, String)>, provider: &str, value: &str) {
    list.retain(|(p, _)| p != provider);
    if !value.is_empty() {
        list.push((provider.to_string(), value.to_string()));
    }
}

impl AiSettings {
    /// Выбранное подключение, неизвестное - по умолчанию.
    pub fn current(&self) -> &str {
        if aic::PROVIDERS.contains(&self.provider.as_str()) { &self.provider } else { PROVIDER_DEFAULT }
    }

    /// Адрес подключения: свой или по умолчанию.
    pub fn base(&self, provider: &str) -> String {
        get(&self.bases, provider).map(str::to_string).or_else(|| aic::defaults(provider).map(|d| d.0.to_string())).unwrap_or_default()
    }

    pub fn model(&self, provider: &str) -> String {
        get(&self.models, provider).map(str::to_string).or_else(|| aic::defaults(provider).map(|d| d.1.to_string())).unwrap_or_default()
    }

    pub fn key(&self, provider: &str) -> String {
        get(&self.keys, provider).unwrap_or_default().to_string()
    }

    /// Можно ли спрашивать: ключ есть или сервис на этом компьютере,
    /// `ready` модуля.
    pub fn ready(&self) -> bool {
        let p = self.current();
        !self.key(p).is_empty() || !aic::needs_key(&self.base(p))
    }

    /// Строки настроек «ключ=значение».
    pub fn to_text(&self) -> String {
        let mut out = format!("ai={}\nai_search={}\n", self.current(), self.search);
        for (name, list) in [("ai_base", &self.bases), ("ai_model", &self.models), ("ai_key", &self.keys)] {
            for (p, v) in list {
                out.push_str(&format!("{name}={p}\t{v}\n"));
            }
        }
        out
    }

    /// Строка настроек: true - строка помощника.
    pub fn read_line(&mut self, key: &str, value: &str) -> bool {
        let pair = || value.split_once('\t').filter(|(p, v)| aic::PROVIDERS.contains(p) && !v.contains(['\n', '\r']));
        match key {
            "ai" if aic::PROVIDERS.contains(&value) => self.provider = value.to_string(),
            "ai_search" => self.search = value != "false",
            "ai_base" => {
                if let Some((p, v)) = pair().filter(|(_, v)| v.starts_with("https://") || v.starts_with("http://")) {
                    put(&mut self.bases, p, v);
                }
            }
            "ai_model" => {
                if let Some((p, v)) = pair() {
                    put(&mut self.models, p, v);
                }
            }
            "ai_key" => {
                if let Some((p, v)) = pair() {
                    put(&mut self.keys, p, v);
                }
            }
            _ => return false,
        }
        true
    }
}

/// Название подключения в списке, `service_names` модуля.
pub fn service_name(lang: Lang, provider: &str) -> String {
    match provider {
        aic::ANTHROPIC => "Claude (Anthropic)".into(),
        aic::RESPONSES => "Grok (xAI)".into(),
        aic::DEEPSEEK => "DeepSeek".into(),
        aic::OPENROUTER => tr(lang, "OpenRouter (has free models)").into(),
        aic::LOCAL => tr(lang, "Own service, for example Ollama").into(),
        other => other.into(),
    }
}

/// Кто сказал реплику разговора.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    User,
    Model,
    Tool,
    Note,
}

/// Чего ждёт запрос: ответа в разговоре или документа KML.
enum Asked {
    Talk,
    /// Создание меток: описание и предел длины ответа.
    Make(String, u64),
}

/// Окно помощника и разговор.
pub struct Ai {
    pub open: bool,
    pub settings_open: bool,
    agent: Agent,
    pub lines: Vec<(Who, String)>,
    input: String,
    key_input: String,
    /// Идущий запрос, его подключение и чего он ждёт.
    asking: Option<(Box<dyn net::Request>, String, Asked)>,
    /// Повтор запроса после временного отказа, время окна.
    retry_at: Option<f64>,
    /// Вызов, который ждёт разрешения пользователя.
    confirm: Option<Call>,
    /// Круги кончились, разговор ждёт «Продолжить».
    more: bool,
    /// Вызов, который исполняет окно глобуса.
    pub run: Option<Call>,
    /// Предложенный документ: текст предложения и KML.
    proposal: Option<(String, String)>,
    /// Документ к записи в «Мои метки»: KML и признак созданного одним
    /// запросом - у него шкала времени открывается на даты меток.
    pub write: Option<(String, bool)>,
    /// Последний ответ модели для строки «Поиск».
    pub answer: Option<String>,
    /// Часы окна прошлого шага, секунды.
    now: f64,
}

impl Default for Ai {
    fn default() -> Self {
        Self::new()
    }
}

impl Ai {
    pub fn new() -> Self {
        Self {
            open: false,
            settings_open: false,
            agent: Agent::new(),
            lines: Vec::new(),
            input: String::new(),
            key_input: String::new(),
            asking: None,
            retry_at: None,
            confirm: None,
            more: false,
            run: None,
            proposal: None,
            write: None,
            answer: None,
            now: 0.0,
        }
    }

    /// Ответ модели ждётся.
    pub fn busy(&self) -> bool {
        self.asking.is_some() || self.retry_at.is_some()
    }

    /// Разговор ждёт разрешения или «Продолжить».
    pub fn waiting(&self) -> bool {
        self.confirm.is_some() || self.more
    }

    /// Ответ на вопрос, заданный после from реплик: первая реплика модели,
    /// пусто - ответ ждётся, иначе последняя строка программы о сбое.
    pub fn answer_since(&self, from: usize) -> String {
        let after = self.lines.get(from..).unwrap_or_default();
        if let Some((_, text)) = after.iter().find(|(w, _)| *w == Who::Model) {
            return text.clone();
        }
        if self.busy() || self.run.is_some() {
            return String::new();
        }
        after.iter().rev().find(|(w, _)| *w == Who::Note).map(|(_, t)| t.clone()).unwrap_or_default()
    }

    pub fn say(&mut self, who: Who, text: &str) {
        if who == Who::Model {
            self.answer = Some(text.to_string());
        }
        self.lines.push((who, text.to_string()));
    }

    fn no_key(&mut self, lang: Lang) {
        self.say(Who::Note, tr(lang, "No API key. It is entered in the window «Assistant settings»."));
        self.open = true;
        self.settings_open = true;
    }

    /// Вопрос модели из поля окна или строки «Поиск», `ask` модуля.
    pub fn ask(&mut self, lang: Lang, text: &str, settings: &AiSettings, context: &Value, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> bool {
        let text = text.trim();
        if text.is_empty() || self.busy() {
            return false;
        }
        if !settings.ready() {
            self.no_key(lang);
            return false;
        }
        if self.waiting() {
            self.stop();
        }
        self.say(Who::User, text);
        let step = self.agent.ask(text);
        self.follow(lang, step, settings, context, request);
        true
    }

    /// Метки по описанию одним запросом, `generate` модуля.
    pub fn generate(&mut self, lang: Lang, text: &str, settings: &AiSettings, context: &Value, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> bool {
        let text = text.trim();
        if text.is_empty() || self.busy() || self.waiting() {
            return false;
        }
        if !settings.ready() {
            self.no_key(lang);
            return false;
        }
        self.say(Who::User, text);
        self.make(text, aic::KML_MAX_TOKENS, settings, context, request);
        true
    }

    fn make(&mut self, text: &str, tokens: u64, settings: &AiSettings, context: &Value, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        let provider = settings.current().to_string();
        let dialog = [aic::Message::user(text)];
        let options = Options { tools: &[], max_tokens: Some(tokens), stream: false };
        let ask = aic::request(&provider, &settings.base(&provider), &settings.model(&provider), &settings.key(&provider), &aic::kml_system_text(context), &dialog, &options);
        self.post(ask, provider, Asked::Make(text.to_string(), tokens), request);
    }

    /// Послать запрос подключения.
    fn post(&mut self, ask: aic::Request, provider: String, asked: Asked, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        let mut headers = ask.headers.clone();
        // Запрос к Anthropic со страницы - с заголовком прямого доступа
        // из браузера, документация Anthropic.
        if cfg!(target_arch = "wasm32") && aic::format_of(&provider) == aic::ANTHROPIC {
            headers.push(("anthropic-dangerous-direct-browser-access".into(), "true".into()));
        }
        let pairs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let mut r = request(ANSWER_BYTES);
        r.keep_error_bodies();
        r.send(&ask.url, &pairs, Some(ask.body.to_string().as_bytes()));
        self.asking = Some((r, provider, asked));
    }

    /// Запрос разговора, `_ask` модуля.
    fn send(&mut self, lang: Lang, settings: &AiSettings, context: &Value, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        let provider = settings.current().to_string();
        let base = settings.base(&provider);
        let key = settings.key(&provider);
        if key.is_empty() && aic::needs_key(&base) {
            self.no_key(lang);
            return;
        }
        let tools = aic::lite_tools(&crate::basemap::READY.map(|r| r.key));
        let dialog = self.agent.dialog_for_request();
        let options = Options { tools: &tools, max_tokens: None, stream: false };
        let ask = aic::request(&provider, &base, &settings.model(&provider), &key, &aic::lite_system_text(context), &dialog, &options);
        self.post(ask, provider, Asked::Talk, request);
    }

    /// Следующий шаг круга разговора, `_step` модуля. Вызов уходит
    /// окну глобуса через [`Ai::run`].
    fn follow(&mut self, lang: Lang, step: Step, settings: &AiSettings, context: &Value, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        match step.kind {
            StepKind::Run => self.run = step.call,
            StepKind::Send => self.send(lang, settings, context, request),
            StepKind::Retry => {
                self.say(Who::Note, &trf(lang, "The service is busy ({error}), a repeat in {seconds} s.", &[("error", &step.text), ("seconds", &format!("{}", step.delay))]));
                self.retry_at = Some(self.now + step.delay);
            }
            StepKind::Confirm => {
                if let Some(call) = step.call {
                    let text = trf(lang, "The assistant asks to run {name} {args}.", &[("name", &call.name), ("args", &call.args.to_string())]);
                    self.say(Who::Note, &text);
                    self.confirm = Some(call);
                    self.open = true;
                }
            }
            StepKind::Limit => {
                self.more = true;
                self.say(Who::Note, tr(lang, "The assistant has done many actions in a row and waits for «Continue»."));
                self.open = true;
            }
            StepKind::Failed => self.say(Who::Note, &trf(lang, "The model did not answer: {error}", &[("error", &step.text)])),
            StepKind::Done => {}
        }
    }

    /// Ответ инструмента на вызов из [`Ai::run`].
    pub fn result(&mut self, lang: Lang, text: &str, settings: &AiSettings, context: &Value, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        self.say(Who::Tool, text);
        let step = self.agent.result(text);
        self.follow(lang, step, settings, context, request);
    }

    /// Остановить разговор: вопрос о разрешении, «Продолжить» и повтор
    /// снимаются, `stop_talk` модуля.
    pub fn stop(&mut self) {
        self.retry_at = None;
        self.confirm = None;
        self.more = false;
        self.run = None;
        let _ = self.agent.cancel();
    }

    /// Шаг: ответ модели, повтор после паузы.
    pub fn step(&mut self, lang: Lang, now: f64, settings: &AiSettings, context: &Value, request: &dyn Fn(u64) -> Box<dyn net::Request>) {
        self.now = now;
        if self.retry_at.is_some_and(|t| now >= t) {
            self.retry_at = None;
            self.send(lang, settings, context, request);
        }
        let Some(answer) = self.asking.as_ref().and_then(|(r, _, _)| r.take()) else {
            return;
        };
        let Some((_, provider, asked)) = self.asking.take() else {
            return;
        };
        // Тело ответа разбирается и при ошибке HTTP: причину отказа
        // сервис пишет в нём.
        let (data, error) = match answer {
            Ok(bytes) => (serde_json::from_slice::<Value>(&bytes).ok(), String::new()),
            Err(e) => {
                let body = e.split_once(": ").and_then(|(_, b)| serde_json::from_str::<Value>(b).ok());
                (body, e)
            }
        };
        let reply = data.as_ref().map_or_else(|| aic::Reply::failed("no data"), |d| aic::parse(&provider, d));
        match asked {
            Asked::Talk => {
                if reply.error.is_empty() && !reply.text.is_empty() {
                    self.say(Who::Model, &reply.text);
                }
                let reason = if !reply.error.is_empty() { String::new() } else if data.is_none() { error.clone() } else { String::new() };
                let local = !aic::needs_key(&settings.base(&provider));
                let step = self.agent.answered(reply, &reason);
                let failed = step.kind == StepKind::Failed;
                self.follow(lang, step, settings, context, request);
                if failed && local {
                    self.say(Who::Note, tr(lang, "The service on this computer does not answer. Check that it is running, for example Ollama, and that the address in the assistant settings is right."));
                }
            }
            Asked::Make(text, tokens) => {
                let reason = if reply.error.is_empty() { error } else { reply.error.clone() };
                if !reason.is_empty() && tokens > aic::KML_SAFE_TOKENS && aic::length_refused(&reason) {
                    self.say(Who::Note, tr(lang, "The service did not accept a long answer, the request is repeated with a smaller length limit."));
                    self.make(&text, aic::KML_SAFE_TOKENS, settings, context, request);
                    return;
                }
                let Some(kml) = aic::extract_kml(&reply.text) else {
                    if reason.is_empty() {
                        self.say(Who::Note, tr(lang, "The model did not return a KML document."));
                    } else {
                        self.say(Who::Note, &trf(lang, "The model did not answer: {error}", &[("error", &reason)]));
                    }
                    return;
                };
                if !reply.text.contains("</kml>") {
                    self.say(Who::Note, tr(lang, "The answer of the model hit the length limit. The placemarks that came whole are taken."));
                }
                // В разговоре остаётся короткий след, документ в запросы
                // разговора не идёт.
                let _ = self.agent.ask(&text);
                self.agent.dialog.push(aic::assistant_message(&aic::Reply { text: "Метки созданы документом KML.".into(), calls: Vec::new(), raw: None, error: String::new() }));
                self.write = Some((kml, true));
            }
        }
    }

    /// Показать предложенный документ, `propose` модуля.
    pub fn propose(&mut self, text: String, kml: String) {
        self.proposal = Some((text, kml));
    }

    /// Текст предложенного документа, None - предложения нет.
    pub fn proposal_text(&self) -> Option<&str> {
        self.proposal.as_ref().map(|(t, _)| t.as_str())
    }

    /// Записать предложенный документ, «Записать в «Мои метки»».
    pub fn accept_proposal(&mut self) {
        if let Some((_, kml)) = self.proposal.take() {
            self.write = Some((kml, false));
        }
    }

    /// Окно «Помощник» и окно «Настройки помощника».
    pub fn show(&mut self, ctx: &egui::Context, lang: Lang, settings: &mut AiSettings, context: &Value, request: &dyn Fn(u64) -> Box<dyn net::Request>) -> bool {
        let changed = self.settings_window(ctx, lang, settings);
        if !self.open {
            return changed;
        }
        let mut open = true;
        let mut ask = false;
        let mut make = false;
        let mut decided: Option<Choice> = None;
        let mut resume = false;
        let mut stop = false;
        let mut accept = false;
        let mut reject = false;
        egui::Window::new(tr(lang, "Assistant")).open(&mut open).default_size([560.0, 520.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let p = settings.current().to_string();
                let mut text = trf(lang, "Service: {service}, model {model}.", &[("service", &service_name(lang, &p)), ("model", &settings.model(&p))]);
                if !settings.ready() {
                    text.push(' ');
                    text.push_str(tr(lang, "No API key."));
                }
                ui.label(text);
                if ui.button(tr(lang, "Settings…")).on_hover_text(tr(lang, "Service, model and API key of the assistant.")).clicked() {
                    self.settings_open = true;
                }
            });
            ui.separator();
            let height = (ui.available_height() - 120.0).max(160.0);
            egui::ScrollArea::vertical().max_height(height).stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                if self.lines.is_empty() {
                    ui.weak(tr(lang, "Here goes the conversation: your questions, the actions of the assistant on the globe and its answers. A question is asked in the field below or in the Search line."));
                }
                for (who, text) in &self.lines {
                    let (name, colour) = match who {
                        Who::User => (tr(lang, "You"), egui::Color32::from_rgb(90, 150, 230)),
                        Who::Model => (tr(lang, "Assistant"), ui.visuals().text_color()),
                        Who::Tool => (tr(lang, "Action"), egui::Color32::GRAY),
                        Who::Note => (tr(lang, "PlanetX Lite"), egui::Color32::from_rgb(200, 130, 40)),
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(format!("{name}:")).strong().color(colour));
                        ui.label(egui::RichText::new(text).color(colour));
                    });
                }
                if self.busy() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.weak(tr(lang, "Waiting for the answer…"));
                    });
                }
            });
            if let Some((text, _)) = &self.proposal {
                ui.horizontal_wrapped(|ui| {
                    ui.label(text);
                    accept = ui.button(tr(lang, "Write to My Places")).clicked();
                    reject = ui.button(tr(lang, "Cancel")).clicked();
                });
            }
            if self.confirm.is_some() {
                ui.horizontal_wrapped(|ui| {
                    if ui.button(tr(lang, "Allow")).on_hover_text(tr(lang, "Run this action once.")).clicked() {
                        decided = Some(Choice::Allow);
                    }
                    if ui.button(tr(lang, "Allow in this conversation")).on_hover_text(tr(lang, "Exactly the same request is not asked again until a new conversation.")).clicked() {
                        decided = Some(Choice::AllowAlways);
                    }
                    if ui.button(tr(lang, "Deny")).on_hover_text(tr(lang, "The assistant gets a refusal and answers without this action.")).clicked() {
                        decided = Some(Choice::Deny);
                    }
                });
            }
            if self.more {
                ui.horizontal_wrapped(|ui| {
                    ui.label(tr(lang, "The assistant has done many actions in a row."));
                    resume = ui.button(tr(lang, "Continue")).on_hover_text(tr(lang, "The assistant continues where it stopped.")).clicked();
                    stop = ui.button(tr(lang, "Stop")).clicked();
                });
            }
            ui.horizontal(|ui| {
                let edit = ui.add(egui::TextEdit::singleline(&mut self.input).hint_text(tr(lang, "For example: show a section through the Japan Trench")).desired_width(ui.available_width() - 200.0));
                ask = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let free = !self.busy();
                ask |= ui.add_enabled(free, egui::Button::new(tr(lang, "Ask"))).clicked();
                make = ui
                    .add_enabled(free && !self.waiting(), egui::Button::new(tr(lang, "Create placemarks")))
                    .on_hover_text(tr(lang, "Placemarks, paths and polygons by the description in the field, for example «the voyage of Columbus». The model answers with one KML document, it is written at once as a new folder in My Places."))
                    .clicked();
            });
            ui.add(egui::Label::new(egui::RichText::new(tr(lang, "The question, the view point and the turned on rows of the Layers section go to the server of the chosen model. My Places go only selected.")).small()).wrap());
        });
        if ask {
            let text = self.input.clone();
            if self.ask(lang, &text, settings, context, request) {
                self.input.clear();
            }
        }
        if make {
            let text = self.input.clone();
            if self.generate(lang, &text, settings, context, request) {
                self.input.clear();
            }
        }
        if let Some(choice) = decided
            && self.confirm.take().is_some()
        {
            if choice == Choice::Deny {
                self.say(Who::Note, tr(lang, "Denied."));
            }
            let step = self.agent.decide(choice);
            self.follow(lang, step, settings, context, request);
        }
        if resume {
            self.more = false;
            let step = self.agent.resume();
            self.follow(lang, step, settings, context, request);
        }
        if stop {
            self.stop();
        }
        if accept && let Some((_, kml)) = self.proposal.take() {
            self.write = Some((kml, false));
        }
        if reject {
            self.proposal = None;
        }
        if !open {
            self.open = false;
            if self.waiting() || self.retry_at.is_some() {
                self.stop();
            }
        }
        changed
    }

    /// Окно «Настройки помощника», `AssistantSettings` модуля. true -
    /// настройки сменились.
    fn settings_window(&mut self, ctx: &egui::Context, lang: Lang, settings: &mut AiSettings) -> bool {
        if !self.settings_open {
            return false;
        }
        let before = settings.clone();
        let mut open = true;
        egui::Window::new(tr(lang, "Assistant settings")).open(&mut open).default_width(520.0).show(ctx, |ui| {
            let p = settings.current().to_string();
            egui::Grid::new("planetx-ai-settings").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label(tr(lang, "Service"));
                let mut chosen = p.clone();
                egui::ComboBox::from_id_salt("planetx-ai-provider").selected_text(service_name(lang, &p)).show_ui(ui, |ui| {
                    for q in aic::PROVIDERS {
                        ui.selectable_value(&mut chosen, q.to_string(), service_name(lang, q));
                    }
                })
                .response
                .on_hover_text(tr(lang, "Through which service the assistant works. Each service has its own key, address and model. OpenRouter models with «:free» in the name are free with a limit of requests a day. The own service works in the OpenAI Chat format, on this computer no key is needed."));
                if chosen != p {
                    settings.provider = chosen;
                    self.key_input.clear();
                }
                ui.end_row();
                let p = settings.current().to_string();
                ui.label(tr(lang, "Address"));
                let mut base = settings.base(&p);
                if ui.add(egui::TextEdit::singleline(&mut base).desired_width(360.0)).on_hover_text(tr(lang, "The address of the service. For Anthropic, xAI, DeepSeek and OpenRouter it does not need a change. For an own service - the address of its OpenAI format entry, for example http://localhost:11434/v1 for Ollama.")).changed() {
                    let default = aic::defaults(&p).map(|d| d.0).unwrap_or_default();
                    put(&mut settings.bases, &p, if base.trim() == default { "" } else { base.trim() });
                }
                ui.end_row();
                ui.label(tr(lang, "Model"));
                let mut model = settings.model(&p);
                if ui.add(egui::TextEdit::singleline(&mut model).desired_width(360.0)).on_hover_text(tr(lang, "The name of the model at the chosen service. The model must be able to call tools, otherwise the assistant only answers with text.")).changed() {
                    let default = aic::defaults(&p).map(|d| d.1).unwrap_or_default();
                    put(&mut settings.models, &p, if model.trim() == default { "" } else { model.trim() });
                }
                ui.end_row();
                ui.label(tr(lang, "Key"));
                ui.horizontal(|ui| {
                    let hint = if !settings.key(&p).is_empty() {
                        tr(lang, "the key is saved")
                    } else if !aic::needs_key(&settings.base(&p)) {
                        tr(lang, "no key is needed")
                    } else {
                        tr(lang, "enter the API key")
                    };
                    ui.add(egui::TextEdit::singleline(&mut self.key_input).password(true).hint_text(hint).desired_width(260.0))
                        .on_hover_text(tr(lang, "The API key of the chosen service. It is kept as plain text in the settings of the program and goes only to the address of the service."));
                    if ui.button(tr(lang, "Save key")).clicked() && !self.key_input.trim().is_empty() {
                        put(&mut settings.keys, &p, self.key_input.trim());
                        self.key_input.clear();
                    }
                    if !settings.key(&p).is_empty() && ui.button(tr(lang, "Delete key")).clicked() {
                        put(&mut settings.keys, &p, "");
                    }
                });
                ui.end_row();
            });
            ui.checkbox(&mut settings.search, tr(lang, "Answer requests from the Search line"))
                .on_hover_text(tr(lang, "A request in words in the Search line goes to the assistant, the answer appears under the line. Without the tick the line finds only places and coordinates."));
        });
        if !open {
            self.settings_open = false;
        }
        *settings != before
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    type Answer = Result<Vec<u8>, String>;
    /// Посланный запрос: адрес, заголовки, тело.
    type Sent = (String, Vec<(String, String)>, Value);

    /// Поддельный сервер модели: ответы по очереди, посланные тела.
    #[derive(Clone, Default)]
    struct Fake {
        answers: Rc<RefCell<VecDeque<Answer>>>,
        sent: Rc<RefCell<Vec<Sent>>>,
        ready: Rc<RefCell<Option<Answer>>>,
        stats: Rc<net::Stats>,
    }

    impl net::Request for Fake {
        fn start(&self, url: &str) {
            self.send(url, &[], None);
        }
        fn send(&self, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>) {
            let body = body.and_then(|b| serde_json::from_slice(b).ok()).unwrap_or(Value::Null);
            self.sent.borrow_mut().push((url.to_string(), headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), body));
            *self.ready.borrow_mut() = self.answers.borrow_mut().pop_front();
        }
        fn take(&self) -> Option<Result<Vec<u8>, String>> {
            self.ready.borrow_mut().take()
        }
        fn stats(&self) -> &net::Stats {
            &self.stats
        }
    }

    fn chat(message: Value) -> Result<Vec<u8>, String> {
        Ok(serde_json::json!({"choices": [{"message": message, "finish_reason": "stop"}]}).to_string().into_bytes())
    }

    /// Круг разговора: вызов инструмента, его ответ, текст модели,
    /// как шаги `assistant_*` модуля с поддельным сервером.
    #[test]
    fn conversation_runs_a_tool_and_answers() {
        let fake = Fake::default();
        fake.answers.borrow_mut().extend([
            chat(serde_json::json!({"content": null, "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "fly_to", "arguments": "{\"lat\": 58.0, \"lon\": 56.2}"}}]})),
            chat(serde_json::json!({"content": "Пермь на экране."})),
        ]);
        let request = |_: u64| -> Box<dyn net::Request> { Box::new(fake.clone()) };
        let mut s = AiSettings::default();
        put(&mut s.keys, aic::OPENROUTER, "sk-or-1");
        let context = serde_json::json!({"body": "earth"});
        let mut ai = Ai::new();
        assert!(ai.ask(Lang::RU, "покажи Пермь", &s, &context, &request));
        ai.step(Lang::RU, 1.0, &s, &context, &request);
        let call = ai.run.take().unwrap();
        assert_eq!(call.name, "fly_to");
        ai.result(Lang::RU, "Перелёт к 58.000, 56.200.", &s, &context, &request);
        ai.step(Lang::RU, 2.0, &s, &context, &request);
        assert_eq!(ai.answer.as_deref(), Some("Пермь на экране."));
        assert!(!ai.busy());
        let sent = fake.sent.borrow();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].0, "https://openrouter.ai/api/v1/chat/completions");
        assert!(sent[0].1.contains(&("Authorization".into(), "Bearer sk-or-1".into())));
        let tools: Vec<&str> = sent[0].2["tools"].as_array().unwrap().iter().filter_map(|t| t["function"]["name"].as_str()).collect();
        assert!(tools.contains(&"fly_to") && !tools.contains(&"read_features"));
        assert!(sent[0].2["messages"][0]["content"].as_str().unwrap().contains("PlanetX Lite"));
        // Второй запрос несёт ответ инструмента.
        assert!(sent[1].2.to_string().contains("Перелёт к 58.000"));
    }

    /// Без ключа вопрос не уходит, открываются настройки.
    #[test]
    fn no_key_opens_settings() {
        let fake = Fake::default();
        let request = |_: u64| -> Box<dyn net::Request> { Box::new(fake.clone()) };
        let mut ai = Ai::new();
        assert!(!ai.ask(Lang::RU, "где я", &AiSettings::default(), &Value::Null, &request));
        assert!(ai.settings_open && fake.sent.borrow().is_empty());
    }

    /// Метки одним запросом: ответ - документ KML к записи, ошибка HTTP
    /// с телом - причина из тела.
    #[test]
    fn generate_returns_kml_and_errors_carry_reason() {
        let fake = Fake::default();
        let kml = "<kml xmlns=\"http://www.opengis.net/kml/2.2\"><Document><name>Колумб</name><Placemark><name>Палос</name><Point><coordinates>-6.89,37.23</coordinates></Point></Placemark></Document></kml>";
        fake.answers.borrow_mut().extend([chat(serde_json::json!({"content": format!("```xml\n{kml}\n```")})), Err("HTTP 401: {\"error\": {\"message\": \"No auth credentials found\", \"code\": 401}}".into())]);
        let request = |_: u64| -> Box<dyn net::Request> { Box::new(fake.clone()) };
        let mut s = AiSettings::default();
        put(&mut s.keys, aic::OPENROUTER, "sk-or-1");
        let mut ai = Ai::new();
        assert!(ai.generate(Lang::RU, "путешествие Колумба", &s, &Value::Null, &request));
        assert!(fake.sent.borrow()[0].2.get("tools").is_none());
        ai.step(Lang::RU, 1.0, &s, &Value::Null, &request);
        let (doc, made) = ai.write.take().unwrap();
        assert!(made && doc.starts_with("<kml") && doc.contains("Палос"));
        assert!(ai.ask(Lang::RU, "что здесь?", &s, &Value::Null, &request));
        ai.step(Lang::RU, 2.0, &s, &Value::Null, &request);
        assert!(ai.lines.last().unwrap().1.contains("No auth credentials found"), "{:?}", ai.lines);
    }

    #[test]
    fn settings_round_trip_and_defaults() {
        let mut s = AiSettings::default();
        assert_eq!(s.current(), aic::OPENROUTER);
        assert_eq!(s.model(aic::OPENROUTER), "openrouter/free");
        assert!(!s.ready());
        put(&mut s.keys, aic::OPENROUTER, "sk-or-1");
        put(&mut s.models, aic::ANTHROPIC, "claude-opus-5-5");
        s.search = false;
        assert!(s.ready());
        let mut back = AiSettings::default();
        for line in s.to_text().lines() {
            let (k, v) = line.split_once('=').unwrap();
            assert!(back.read_line(k, v));
        }
        assert_eq!(back, s);
        // Своему сервису на этом компьютере ключ не нужен.
        back.provider = aic::LOCAL.into();
        assert!(back.ready());
        // Чужие и испорченные строки не берутся.
        assert!(!back.read_line("foo", "bar"));
        back.read_line("ai_base", "openrouter\tftp://x");
        assert_eq!(back.base(aic::OPENROUTER), "https://openrouter.ai/api/v1");
        back.read_line("ai", "evil");
        assert_eq!(back.current(), aic::LOCAL);
    }
}
